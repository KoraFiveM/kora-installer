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
use kora_core::api::wire;
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

pub async fn system_info(State(st): State<AppState>) -> Result<Json<wire::SystemInfo>, ApiError> {
    let port = st.addr.port();
    let loopback = st.allowed_hosts.iter().any(|h| h.starts_with("127.") || h.starts_with("localhost:") || h.starts_with("[::1]"));
    blocking(move || {
        let elevated = kora_core::elevate::is_elevated();
        let headless = headless();
        Ok(Json(wire::SystemInfo {
            version: env!("CARGO_PKG_VERSION").into(),
            platform: if cfg!(windows) { wire::PlatformName::Windows } else { wire::PlatformName::Linux },
            elevated,
            // The UI reads `admin`; `elevated` is the same answer under the plan's name.
            admin: elevated,
            helper_installed: crate::elevated::helper_installed(),
            // Windows: the panel asks Windows for permission itself (one prompt per install or action).
            admin_prompt: crate::elevated::worker_supported() && !elevated,
            headless,
            local: loopback && !headless,
            kora_home: kora_core::installer::registry::kora_home().display().to_string(),
            can_restart: can_restart(port),
        }))
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
