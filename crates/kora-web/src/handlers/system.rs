//! System information and the live server-status event stream.
//!
//! Each `pub async fn` is mounted by `crate::routes` for the [`Op`] of the same name; the HTTP
//! method and path come from `Op::route`, so only the handler's extractors live here.
#![allow(clippy::unused_async)]

use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::{Extension, Json};
use serde_json::{json, Value};
use tokio::sync::broadcast;
use tokio_stream::wrappers::ReceiverStream;

use super::servers::list_json;
use crate::remote::Caller;
use crate::security::Limited;
use crate::{blocking, ApiError, AppState, BackendFactory};

/// How often the event stream looks at the servers again.
const POLL: Duration = Duration::from_secs(3);

/// Is this machine without a desktop session (so "open folder" cannot work)?
pub(crate) fn headless() -> bool {
    if let Some(v) = seam::headless_override() {
        return v;
    }
    if std::env::var_os("SSH_CONNECTION").is_some() {
        return true;
    }
    cfg!(unix) && !cfg!(target_os = "macos") && std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none()
}

/// A test seam: lets tests pretend to be headless or not, and choose what "elevated" means.
pub mod seam {
    use std::sync::Mutex;

    static HEADLESS: Mutex<Option<bool>> = Mutex::new(None);

    /// Tests only: force the headless answer (`None` = detect for real).
    pub fn set_headless(v: Option<bool>) {
        *HEADLESS.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = v;
    }
    static CAN_RESTART: Mutex<Option<bool>> = Mutex::new(None);
    static RESTARTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    /// Tests only: force the "can restart" answer. While set, a restart only counts (no command runs).
    pub fn set_can_restart(v: Option<bool>) {
        *CAN_RESTART.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = v;
    }
    /// Tests only: how many restarts were asked for while [`set_can_restart`] was set.
    pub fn restarts() -> usize {
        RESTARTS.load(std::sync::atomic::Ordering::SeqCst)
    }
    pub(super) fn can_restart_override() -> Option<bool> {
        *CAN_RESTART.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    pub(super) fn count_restart() {
        RESTARTS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    pub(super) fn headless_override() -> Option<bool> {
        *HEADLESS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Is this panel the background service, so it can restart itself (after a self-update)?
fn can_restart(port: u16) -> bool {
    use kora_core::monitor::panel_service as svc;
    if let Some(v) = seam::can_restart_override() {
        return v;
    }
    let platform = kora_core::service::Platform::current();
    svc::can_restart(platform, &svc::restart_facts(&svc::home(), platform, port))
}

/// `POST /panel/restart`: 202 `{"restarting": true}`, then the panel restarts (local admin only).
/// Sign-in sessions live in memory, so the page signs in again afterwards.
pub async fn panel_restart(State(st): State<AppState>, Extension(caller): Extension<Caller>) -> Result<(axum::http::StatusCode, Json<Value>), ApiError> {
    if caller.remote {
        return Err(ApiError::forbidden("The panel can only be restarted from the computer that runs Kora."));
    }
    let port = st.addr.port();
    if !blocking(move || Ok(can_restart(port))).await? {
        return Err(ApiError::conflict("This panel is not the background service, so it cannot restart itself. Stop it and run `kora panel` again."));
    }
    if seam::can_restart_override().is_some() {
        seam::count_restart();
    } else {
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            restart_now(port);
        });
    }
    Ok((axum::http::StatusCode::ACCEPTED, Json(json!({"restarting": true}))))
}

/// Start the restart (Linux: systemd restarts the unit, and so us; Windows: start a new panel, exit).
fn restart_now(port: u16) {
    use kora_core::monitor::panel_service as svc;
    let platform = kora_core::service::Platform::current();
    let home = svc::home();
    let exe = std::env::current_exe().unwrap_or_default();
    let (program, args) = svc::restart_command(platform, &exe, port, svc::saved_tailscale(&home));
    let mut cmd = std::process::Command::new(program);
    cmd.args(args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW | DETACHED_PROCESS
        cmd.creation_flags(0x0800_0000 | 0x0000_0008);
    }
    match cmd.spawn() {
        Ok(_) if platform == kora_core::service::Platform::Windows => {
            // The new panel waits for the port (see `bind_retrying`).
            std::process::exit(0);
        }
        Ok(_) => {}
        Err(e) => eprintln!("Could not restart the panel: {e}"),
    }
}

pub async fn system_info(State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    let port = st.addr.port();
    let loopback = st.allowed_hosts.iter().any(|h| h.starts_with("127.") || h.starts_with("localhost:") || h.starts_with("[::1]"));
    blocking(move || {
        let elevated = kora_core::elevate::is_elevated();
        let headless = headless();
        Ok(Json(json!({
            "version": env!("CARGO_PKG_VERSION"),
            "platform": if cfg!(windows) { "windows" } else { "linux" },
            "elevated": elevated,
            // The UI reads `admin`; `elevated` is the same answer under the plan's name.
            "admin": elevated,
            // Kept for older pages: the administrator service is there and serves this account.
            "helper_installed": crate::elevated::helper_installed(),
            // Kora's administrator service for this account: ready (no prompt, locally or remotely),
            // missing (a per-user install: Settings offers to install it) or not_allowed.
            "admin_service": crate::elevated::service_state(),
            // The old Windows worker only: the panel asks Windows for permission itself.
            "admin_prompt": crate::elevated::prompts() && crate::elevated::worker_supported() && !elevated,
            "headless": headless,
            "local": loopback && !headless,
            "kora_home": kora_core::installer::registry::kora_home(),
            "can_restart": can_restart(port),
        })))
    })
    .await
}

fn servers_event(v: &Value) -> Event {
    Event::default().event("servers").data(v.to_string())
}

/// The single poller behind every `GET /events` stream: however many streams are open, the servers are
/// read once per [`POLL`]. It starts with the first stream and stops when the last one has gone.
pub struct EventsHub {
    poll: Duration,
    slot: Mutex<Slot>,
}

#[derive(Default)]
struct Slot {
    tx: Option<broadcast::Sender<Value>>,
    /// The latest list, sent first to a stream that joins a running poller.
    last: Option<Value>,
}

impl Default for EventsHub {
    fn default() -> Self {
        Self::new(POLL)
    }
}

impl EventsHub {
    pub fn new(poll: Duration) -> Self {
        Self { poll, slot: Mutex::default() }
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, Slot> {
        self.slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    /// Join the poller (starting it if needed): the latest list, if any, and the later changes.
    pub fn subscribe(self: &Arc<Self>, make: BackendFactory) -> (Option<Value>, broadcast::Receiver<Value>) {
        let mut g = self.lock();
        if let Some(tx) = &g.tx {
            return (g.last.clone(), tx.subscribe());
        }
        let (tx, rx) = broadcast::channel(16);
        g.tx = Some(tx.clone());
        g.last = None;
        tokio::spawn(self.clone().run(tx, make));
        (None, rx)
    }
    /// Is a poller running (tests)?
    pub fn running(&self) -> bool {
        self.lock().tx.is_some()
    }
    /// Stop when nobody listens. Decided under the lock that `subscribe` takes, so a stream that joins
    /// at that moment either gets this poller or starts a new one.
    fn keep_going(&self, tx: &broadcast::Sender<Value>) -> bool {
        let mut g = self.lock();
        if tx.receiver_count() == 0 {
            if g.tx.as_ref().is_some_and(|t| t.same_channel(tx)) {
                g.tx = None;
                g.last = None;
            }
            return false;
        }
        true
    }
    async fn run(self: Arc<Self>, tx: broadcast::Sender<Value>, make: BackendFactory) {
        let mut last: Option<Value> = None;
        while self.keep_going(&tx) {
            let m = make.clone();
            if let Ok(v) = blocking(move || list_json(&mut *m())).await {
                if last.as_ref() != Some(&v) {
                    self.lock().last = Some(v.clone());
                    let _ = tx.send(v.clone());
                    last = Some(v);
                }
            }
            tokio::time::sleep(self.poll).await;
        }
    }
}

pub async fn events(State(st): State<AppState>, Extension(caller): Extension<Caller>) -> Result<Sse<Limited<ReceiverStream<Result<Event, Infallible>>>>, ApiError> {
    let permit = st.stream_permit(&caller)?;
    let (first, mut sub) = st.events.subscribe(st.backend.clone());
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(16);
    tokio::spawn(async move {
        if let Some(v) = first {
            if tx.send(Ok(servers_event(&v))).await.is_err() {
                return;
            }
        }
        loop {
            tokio::select! {
                r = sub.recv() => match r {
                    Ok(v) => {
                        if tx.send(Ok(servers_event(&v))).await.is_err() {
                            return;
                        }
                    }
                    // Fell behind: the next change carries the whole list anyway.
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return,
                },
                () = tx.closed() => return,
            }
        }
    });
    Ok(Sse::new(Limited::new(ReceiverStream::new(rx), permit)).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn one_poller_for_every_stream_and_it_stops_when_they_go() {
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let fake = kora_core::api::fake::FakeBackend::sample();
        let make: BackendFactory = Arc::new(move || {
            c.fetch_add(1, Ordering::SeqCst);
            Box::new(fake.clone())
        });
        let hub = Arc::new(EventsHub::new(Duration::from_millis(200)));
        let (_, mut a) = hub.subscribe(make.clone());
        let first = a.recv().await.unwrap();
        let (latest, b) = hub.subscribe(make.clone());
        assert_eq!(latest.as_ref(), Some(&first), "a new stream gets the current list at once");
        let (_, c2) = hub.subscribe(make.clone());
        tokio::time::sleep(Duration::from_millis(500)).await;
        let n = calls.load(Ordering::SeqCst);
        assert!((2..=4).contains(&n), "one read per poll for three streams, got {n}");
        drop((a, b, c2));
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(!hub.running(), "stopped with nobody listening");
        let after = calls.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(calls.load(Ordering::SeqCst), after);
        // And it starts again for the next stream.
        let (_, mut d) = hub.subscribe(make);
        assert!(d.recv().await.is_ok());
    }
}

/// `POST /system/admin-service/install` → `202 {job}` on Windows, where the installer is launched and
/// asks an administrator for approval on the computer that runs Kora (the one place a prompt remains: it
/// is the install of the service itself). Elsewhere `403 needs_elevation` with the command to run in
/// an administrator terminal (`kora service install`). Local callers only (see `remote::roles`).
pub async fn admin_service_install(State(st): State<AppState>) -> Result<axum::response::Response, ApiError> {
    use kora_core::problem::{codes, Action, Problem};
    if crate::elevated::worker_supported() {
        return Err(ApiError::conflict("Kora's administrator service is already installed and turned on for you."));
    }
    let command = vec!["kora".to_string(), "service".into(), "install".into()];
    if !cfg!(windows) || kora_core::elevate::is_elevated() {
        let p = Problem::new(codes::NEEDS_ELEVATION, "Installing Kora's administrator service needs administrator rights. Run this command as administrator, then come back here:")
            .with_action(Action::ShowCommand { command, elevated: true });
        return Err(ApiError::problem(axum::http::StatusCode::FORBIDDEN, p));
    }
    st.spawn_job(kora_core::api::Op::AdminServiceInstall, None, move |h| {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let mut child = std::process::Command::new(exe)
            .args(["--no-elevate", "service", "install"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("Kora could not start the installer: {e}"))?;
        let err = child.stderr.take();
        let t = std::thread::spawn(move || {
            use std::io::Read;
            let mut s = String::new();
            if let Some(mut e) = err {
                let _ = e.read_to_string(&mut s);
            }
            s
        });
        if let Some(out) = child.stdout.take() {
            use std::io::BufRead;
            for line in std::io::BufReader::new(out).lines().map_while(Result::ok) {
                h.event(json!({"type": "status", "message": line}));
            }
        }
        let status = child.wait().map_err(|e| e.to_string())?;
        let stderr = t.join().unwrap_or_default();
        if !status.success() {
            let why = stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("the installer stopped").trim_start_matches("error: ").to_string();
            return Err(format!("Kora's administrator service was not installed: {why}"));
        }
        Ok(json!({"installed": crate::elevated::worker_supported()}))
    })
}
