//! Starting a server and proving it came up: `kora start`, the installer's last step (`launch`)
//! and the "Start server" button all use [`start`].
//!
//! * A registered OS service is started through the service module.
//! * Otherwise FXServer is started as a **detached background process** (new session on Linux,
//!   hidden console and new process group on Windows) whose output goes to `<server>/.kora/server.log`
//!   and whose pid is kept in `<server>/.kora/server.pid`. It outlives the installer window.
//! * Then we wait (about two minutes) until txAdmin answers HTTP on its port and report the game
//!   port. A rejected licence key, a crash or a port clash is reported in plain words with the last
//!   log lines, never as success.
//!
//! Everything that touches the machine sits behind [`LaunchEnv`], so the logic is tested with a fake.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::service::ServiceState;

/// How long to wait for txAdmin to answer.
pub const TXADMIN_TIMEOUT: Duration = Duration::from_secs(120);
/// After txAdmin is up: how long to watch for the game port (or a licence rejection).
pub const GAME_GRACE: Duration = Duration::from_secs(40);
/// A process that ends within this long of starting "crashed on start".
pub const CRASH_WINDOW: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(1000);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spec {
    pub name: String,
    pub dir: PathBuf,
    pub game_port: u16,
    pub txadmin_port: u16,
}

impl Spec {
    pub fn meta(&self) -> PathBuf {
        self.dir.join(".kora")
    }
    pub fn log_path(&self) -> PathBuf {
        self.meta().join("server.log")
    }
    pub fn pid_path(&self) -> PathBuf {
        self.meta().join("server.pid")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Service,
    Detached,
    /// It was already running: nothing was started.
    AlreadyRunning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    LicenceRejected,
    /// The process ended (within the crash window or later).
    Crashed,
    PortInUse,
    /// txAdmin did not answer in time.
    Timeout,
    /// Could not start at all (missing build, service refused...).
    StartFailed,
}

#[derive(Debug, Clone)]
pub struct Report {
    pub method: Method,
    pub pid: Option<u32>,
    pub txadmin_up: bool,
    pub game_listening: bool,
    pub problem: Option<Problem>,
    /// Plain-language one-liner (success or failure).
    pub message: String,
    /// Last log lines, filled on a problem.
    pub log_tail: Vec<String>,
}

impl Report {
    /// The server is up and healthy.
    pub fn running(&self) -> bool {
        self.problem.is_none() && self.txadmin_up
    }
}

/// Everything the launch logic needs from the machine.
pub trait LaunchEnv {
    fn service_state(&self, spec: &Spec) -> ServiceState;
    fn service_start(&self, spec: &Spec) -> Result<(), crate::problem::Problem>;
    /// Start FXServer detached, output to `log`; returns its pid.
    fn spawn_detached(&self, spec: &Spec, log: &Path) -> Result<u32, crate::problem::Problem>;
    /// Is `pid` alive and (as far as we can tell) this server's FXServer?
    fn pid_alive(&self, spec: &Spec, pid: u32) -> bool;
    /// Does txAdmin answer HTTP on `port`?
    fn txadmin_answers(&self, port: u16) -> bool;
    fn port_listening(&self, port: u16) -> bool;
    fn log_len(&self, path: &Path) -> u64;
    /// Log text from byte `from` on.
    fn read_log(&self, path: &Path, from: u64) -> String;
    fn read_pid_file(&self, spec: &Spec) -> Option<u32>;
    fn write_pid_file(&self, spec: &Spec, pid: u32);
    fn remove_pid_file(&self, spec: &Spec);
    /// End `pid` and everything it started; true if the signal/command worked.
    fn kill_tree(&self, pid: u32) -> bool;
    fn sleep(&self, d: Duration);
    /// Time since an arbitrary fixed point (monotonic).
    fn now(&self) -> Duration;
}

/// Does this log text say the licence key was rejected?
pub fn licence_rejected(log: &str) -> bool {
    let l = log.to_ascii_lowercase();
    [
        "could not authenticate server license key",
        "could not authenticate server licence key",
        "invalid license",
        "invalid licence",
        "license key authentication failed",
        "server license key authentication failed",
        "invalid server license key",
        "sv_licensekey is invalid",
    ]
    .iter()
    .any(|k| l.contains(k))
}

/// Does the log say a port could not be bound?
pub fn port_clash(log: &str) -> bool {
    let l = log.to_ascii_lowercase();
    ["address already in use", "eaddrinuse", "only one usage of each socket address", "failed to bind", "could not bind"].iter().any(|k| l.contains(k))
}

/// What to say after a bind error in the log. Names the port only when a line with the error
/// carries one of this server's two ports.
fn port_clash_message(log: &str, spec: &Spec) -> String {
    let named = log.lines().filter(|l| port_clash(l)).find_map(|l| {
        let mut nums = l.split(|c: char| !c.is_ascii_digit()).filter_map(|n| n.parse::<u16>().ok());
        nums.find(|n| *n == spec.game_port || *n == spec.txadmin_port)
    });
    match named {
        Some(port) => format!("The server stopped because port {port} is already used by another program. Change the port in the panel's Ports tab, or stop the other program."),
        None => format!(
            "The server stopped because one of its ports ({} for players, {} for txAdmin) is already used by another program. Change the port in the panel's Ports tab, or stop the other program.",
            spec.game_port, spec.txadmin_port
        ),
    }
}

/// Remove terminal colour codes (`ESC [ ... letter`) from a log line.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\u{1b}' {
            if it.peek() == Some(&'[') {
                it.next();
                for d in it.by_ref() {
                    if d.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub fn last_lines(text: &str, n: usize) -> Vec<String> {
    let v: Vec<String> = text.lines().map(|l| strip_ansi(l).trim_end().to_string()).filter(|l| !l.trim().is_empty()).collect();
    v[v.len().saturating_sub(n)..].to_vec()
}

/// Is a server of this folder alive (pid file, then service)? Cleans up a stale pid file.
pub fn running_pid(env: &dyn LaunchEnv, spec: &Spec) -> Option<u32> {
    let pid = env.read_pid_file(spec)?;
    if env.pid_alive(spec, pid) {
        Some(pid)
    } else {
        env.remove_pid_file(spec);
        None
    }
}

pub const LICENCE_MESSAGE: &str = "Your licence key was rejected. Check it at portal.cfx.re, then run kora start";

fn done(method: Method, pid: Option<u32>, txadmin_up: bool, game_listening: bool, problem: Option<Problem>, message: impl Into<String>, tail: Vec<String>) -> Report {
    Report { method, pid, txadmin_up, game_listening, problem, message: message.into(), log_tail: tail }
}

/// Start the server (unless it already runs) and wait until it answers. `progress` gets short
/// status lines ("Starting txAdmin...", "Waiting for the game server...").
pub fn start(env: &dyn LaunchEnv, spec: &Spec, progress: &mut dyn FnMut(&str, f32)) -> Report {
    start_with(env, spec, progress, TXADMIN_TIMEOUT, GAME_GRACE)
}

pub fn start_with(env: &dyn LaunchEnv, spec: &Spec, progress: &mut dyn FnMut(&str, f32), tx_timeout: Duration, grace: Duration) -> Report {
    let log = spec.log_path();
    let svc = env.service_state(spec);
    let has_service = svc != ServiceState::NotInstalled;

    // ---- already running? no second instance -------------------------------------------------
    let pid0 = running_pid(env, spec);
    let log_before = env.log_len(&log);
    if pid0.is_some() || svc == ServiceState::Running {
        let up_now = env.txadmin_answers(spec.txadmin_port);
        let game = env.port_listening(spec.game_port);
        let rejected = !game && licence_rejected(&env.read_log(&log, 0));
        if !rejected {
            let msg = if up_now { "Already running." } else { "Already running (txAdmin is still starting)." };
            if up_now {
                return done(Method::AlreadyRunning, pid0, true, game, None, msg, vec![]);
            }
            // Alive but txAdmin not answering yet: wait for it like after a fresh start.
            return wait_ready(env, spec, progress, Method::AlreadyRunning, pid0, log_before, tx_timeout, grace);
        }
        // A previous start ran into a rejected licence: restart it (the key may be fixed now).
        progress("Restarting the server...", 0.0);
        if has_service {
            let _ = env.service_start(spec);
        } else if let Some(p) = pid0 {
            let _ = stop_pid(env, spec, p);
        }
    } else if port_busy_by_other(env, spec) {
        return done(
            Method::Detached,
            None,
            false,
            false,
            Some(Problem::PortInUse),
            format!("Port {} (game) or {} (txAdmin) is already used by another program. Close it or pick other ports.", spec.game_port, spec.txadmin_port),
            vec![],
        );
    }

    // ---- start ------------------------------------------------------------------------------
    progress("Starting txAdmin...", 0.0);
    let log_start = env.log_len(&log);
    let (method, pid) = if has_service {
        if svc != ServiceState::Running {
            if let Err(e) = env.service_start(spec) {
                return done(Method::Service, None, false, false, Some(Problem::StartFailed), format!("Could not start the background service: {e}"), vec![]);
            }
        }
        (Method::Service, None)
    } else {
        match env.spawn_detached(spec, &log) {
            Ok(pid) => {
                env.write_pid_file(spec, pid);
                (Method::Detached, Some(pid))
            }
            Err(e) => return done(Method::Detached, None, false, false, Some(Problem::StartFailed), format!("Could not start FXServer: {e}"), vec![]),
        }
    };
    wait_ready(env, spec, progress, method, pid, log_start, tx_timeout, grace)
}

/// Another program (not this server) holds one of our ports.
fn port_busy_by_other(env: &dyn LaunchEnv, spec: &Spec) -> bool {
    env.port_listening(spec.game_port) || env.port_listening(spec.txadmin_port) || env.txadmin_answers(spec.txadmin_port)
}

#[allow(clippy::too_many_arguments)]
fn wait_ready(env: &dyn LaunchEnv, spec: &Spec, progress: &mut dyn FnMut(&str, f32), method: Method, pid: Option<u32>, log_from: u64, tx_timeout: Duration, grace: Duration) -> Report {
    let log = spec.log_path();
    let t0 = env.now();
    let mut tx_up_at: Option<Duration> = None;
    loop {
        let el = env.now().saturating_sub(t0);
        let text = env.read_log(&log, log_from);
        let tail = || last_lines(&text, 12);
        let alive = match (method, pid) {
            (Method::Service, _) => env.service_state(spec) == ServiceState::Running,
            (_, Some(p)) => env.pid_alive(spec, p),
            _ => true,
        };
        let up = tx_up_at.is_some() || env.txadmin_answers(spec.txadmin_port);
        if up && tx_up_at.is_none() {
            tx_up_at = Some(el);
        }
        let game = env.port_listening(spec.game_port);
        if licence_rejected(&text) {
            return done(method, pid, up, game, Some(Problem::LicenceRejected), LICENCE_MESSAGE, tail());
        }
        if !alive {
            if method != Method::Service {
                env.remove_pid_file(spec);
            }
            let (p, m) = if port_clash(&text) {
                (Problem::PortInUse, port_clash_message(&text, spec))
            } else if el <= CRASH_WINDOW {
                (Problem::Crashed, "The server stopped right after starting. The last lines of its log are below; the full log is in the panel's Logs tab.".to_string())
            } else {
                (Problem::Crashed, "The server stopped while starting. The last lines of its log are below; the full log is in the panel's Logs tab.".to_string())
            };
            return done(method, pid, up, game, Some(p), m, tail());
        }
        if let Some(at) = tx_up_at {
            if game {
                return done(method, pid, true, true, None, "Your server is running.", vec![]);
            }
            if el.saturating_sub(at) >= grace {
                return done(method, pid, true, false, None, "txAdmin is running; the game server is not listening yet (txAdmin is still starting it).", vec![]);
            }
            progress("Waiting for the game server...", 0.7 + 0.3 * (el.saturating_sub(at).as_secs_f32() / grace.as_secs_f32().max(1.0)));
        } else {
            if el >= tx_timeout {
                return done(method, pid, false, game, Some(Problem::Timeout), "txAdmin did not answer in time. The last log lines are below.", tail());
            }
            progress("Starting txAdmin...", 0.7 * (el.as_secs_f32() / tx_timeout.as_secs_f32().max(1.0)));
        }
        env.sleep(POLL);
    }
}

/// Stop a detached server (and everything it started). Returns true if something was stopped.
pub fn stop_pid(env: &dyn LaunchEnv, spec: &Spec, pid: u32) -> bool {
    let ok = env.kill_tree(pid);
    for _ in 0..30 {
        if !env.pid_alive(spec, pid) {
            break;
        }
        env.sleep(Duration::from_millis(500));
    }
    env.remove_pid_file(spec);
    ok
}

/// Stop whatever Kora started detached for this folder. `Ok(false)` = nothing was running.
pub fn stop_detached(env: &dyn LaunchEnv, spec: &Spec) -> bool {
    match running_pid(env, spec) {
        Some(p) => stop_pid(env, spec, p),
        None => false,
    }
}

/// `localhost:<game>`, or `<lan address>:<game>` when the machine's address is cheaply known.
pub fn connect_address(game_port: u16) -> String {
    match lan_address() {
        Some(ip) => format!("{ip}:{game_port}"),
        None => format!("localhost:{game_port}"),
    }
}

/// The address other computers on the network would use (no packet is sent).
pub fn lan_address() -> Option<std::net::IpAddr> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

/// Display form of a command line with Windows-style quoting (log + tests).
pub fn command_line_text(program: &str, args: &[String]) -> String {
    let q = |s: &str| if s.is_empty() || s.contains([' ', '\t', '"']) { format!("\"{}\"", s.replace('"', "\\\"")) } else { s.to_string() };
    std::iter::once(program).chain(args.iter().map(String::as_str)).map(q).collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------------------------------------
// Real environment
// ---------------------------------------------------------------------------------------------

pub struct SystemEnv {
    t0: std::time::Instant,
}

impl Default for SystemEnv {
    fn default() -> Self {
        Self { t0: std::time::Instant::now() }
    }
}

/// Windows: CreateProcess hands the child every *inheritable* handle of ours, including the pipe a
/// caller (PowerShell, a script, the elevated relaunch) gave us as stdout. A long-lived server that
/// inherited it would keep that pipe open and the caller would wait for its end of output forever.
/// So while we spawn it our own standard handles are made non-inheritable (the child's redirected
/// log handles are duplicated explicitly by the standard library and are not affected).
#[cfg(windows)]
mod winh {
    use std::ffi::c_void;
    type Handle = *mut c_void;
    const HANDLE_FLAG_INHERIT: u32 = 1;
    extern "system" {
        fn GetStdHandle(n: u32) -> Handle;
        fn GetHandleInformation(h: Handle, flags: *mut u32) -> i32;
        fn SetHandleInformation(h: Handle, mask: u32, flags: u32) -> i32;
    }

    pub struct NoInherit(Vec<Handle>);

    impl NoInherit {
        pub fn std_handles() -> Self {
            let mut changed = vec![];
            // STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE
            for n in [-10i32 as u32, -11i32 as u32, -12i32 as u32] {
                // SAFETY: plain Win32 calls on our own standard handles; failures are ignored.
                unsafe {
                    let h = GetStdHandle(n);
                    let mut flags = 0u32;
                    if !h.is_null() && h as isize != -1 && GetHandleInformation(h, &mut flags) != 0 && flags & HANDLE_FLAG_INHERIT != 0 && SetHandleInformation(h, HANDLE_FLAG_INHERIT, 0) != 0 {
                        changed.push(h);
                    }
                }
            }
            NoInherit(changed)
        }
    }

    impl Drop for NoInherit {
        fn drop(&mut self) {
            for &h in &self.0 {
                // SAFETY: restores the flag we cleared on a handle that is still ours.
                unsafe {
                    SetHandleInformation(h, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT);
                }
            }
        }
    }
}

mod sys {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    /// Children this process spawned, so a finished one is reaped (not a zombie that looks alive).
    pub fn children() -> &'static Mutex<HashMap<u32, std::process::Child>> {
        static C: OnceLock<Mutex<HashMap<u32, std::process::Child>>> = OnceLock::new();
        C.get_or_init(Default::default)
    }

    pub fn reaped(pid: u32) -> Option<bool> {
        let mut m = children().lock().unwrap();
        let c = m.get_mut(&pid)?;
        let ended = c.try_wait().ok().flatten().is_some();
        if ended {
            m.remove(&pid);
        }
        Some(ended)
    }

    #[cfg(unix)]
    pub fn alive(pid: u32, dir: &std::path::Path) -> bool {
        if let Some(ended) = reaped(pid) {
            return !ended;
        }
        // SAFETY: signal 0 only checks that the process exists.
        let ok = unsafe { libc::kill(pid as i32, 0) } == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
        if !ok {
            return false;
        }
        if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            // "pid (comm) S ...": a zombie is not alive.
            if stat.rsplit(')').next().map(|r| r.trim_start().starts_with('Z')).unwrap_or(false) {
                return false;
            }
        }
        // Guard against pid reuse: our processes carry the server's data path in their environment.
        match std::fs::read(format!("/proc/{pid}/environ")) {
            Ok(env) => {
                let want = format!("TXHOST_DATA_PATH={}", dir.join("txData").display());
                String::from_utf8_lossy(&env).split('\0').any(|e| e == want)
            }
            Err(_) => true,
        }
    }

    #[cfg(windows)]
    pub fn alive(pid: u32, _dir: &std::path::Path) -> bool {
        if let Some(ended) = reaped(pid) {
            return !ended;
        }
        use std::os::windows::process::CommandExt;
        crate::sysexe::command("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .creation_flags(0x0800_0000)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_ascii_lowercase().contains("fxserver"))
            .unwrap_or(false)
    }

    #[cfg(unix)]
    pub fn kill_tree(pid: u32) -> bool {
        // The detached server is a session/group leader: signal the whole group.
        // SAFETY: plain kill(2) with a group id we created.
        let term = unsafe { libc::kill(-(pid as i32), libc::SIGTERM) } == 0 || unsafe { libc::kill(pid as i32, libc::SIGTERM) } == 0;
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(250));
            if reaped(pid) == Some(true) || unsafe { libc::kill(pid as i32, 0) } != 0 {
                return term;
            }
        }
        // SAFETY: as above.
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
            libc::kill(pid as i32, libc::SIGKILL);
        }
        reaped(pid);
        term
    }

    #[cfg(windows)]
    pub fn kill_tree(pid: u32) -> bool {
        use std::os::windows::process::CommandExt;
        let ok = crate::sysexe::command("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).creation_flags(0x0800_0000).output().map(|o| o.status.success()).unwrap_or(false);
        reaped(pid);
        ok
    }

    #[cfg(not(any(unix, windows)))]
    pub fn alive(_: u32, _: &std::path::Path) -> bool {
        false
    }
    #[cfg(not(any(unix, windows)))]
    pub fn kill_tree(_: u32) -> bool {
        false
    }
}

impl LaunchEnv for SystemEnv {
    fn service_state(&self, spec: &Spec) -> ServiceState {
        crate::installer::host::service_state(&spec.name).unwrap_or(ServiceState::NotInstalled)
    }

    fn service_start(&self, spec: &Spec) -> Result<(), crate::problem::Problem> {
        crate::installer::host::service_control(&spec.name, crate::service::ControlAction::Start).map_err(|e| e.to_string())
    }

    fn spawn_detached(&self, spec: &Spec, log: &Path) -> Result<u32, crate::problem::Problem> {
        let mut cmd = crate::installer::host::build_command(&spec.dir.to_string_lossy())?;
        std::fs::create_dir_all(spec.meta()).map_err(|e| format!("cannot create {}: {e}", spec.meta().display()))?;
        if log.is_file() {
            let _ = std::fs::rename(log, log.with_extension("log.old"));
        }
        // Never through a link planted in the server folder (this can run as root).
        // Read-write, not append: Windows cannot empty (`set_len`) a file opened for appending only.
        let out =
            crate::fsutil::open_no_follow(log, crate::fsutil::Access::ReadWrite, Some(0o644)).and_then(|f| f.set_len(0).map(|_| f)).map_err(|e| format!("cannot write {}: {e}", log.display()))?;
        let err = out.try_clone().map_err(|e| e.to_string())?;
        cmd.stdin(std::process::Stdio::null()).stdout(out).stderr(err);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // SAFETY: setsid is async-signal-safe; it only detaches the child from our terminal/session.
            unsafe {
                cmd.pre_exec(|| {
                    libc::setsid();
                    Ok(())
                });
            }
        }
        #[cfg(windows)]
        let _no_inherit = winh::NoInherit::std_handles();
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW: no console window (a DETACHED_PROCESS would make
            // txAdmin's own child processes open new ones), and Ctrl+C / closing the installer's console
            // does not reach the server.
            cmd.creation_flags(0x0000_0200 | 0x0800_0000);
        }
        let child = cmd.spawn().map_err(|e| e.to_string())?;
        let pid = child.id();
        sys::children().lock().unwrap().insert(pid, child);
        Ok(pid)
    }

    fn pid_alive(&self, spec: &Spec, pid: u32) -> bool {
        sys::alive(pid, &spec.dir)
    }

    fn txadmin_answers(&self, port: u16) -> bool {
        // Any HTTP answer below 500 (txAdmin redirects its root to the login page).
        let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(3)).redirects(0).build();
        match agent.get(&format!("http://127.0.0.1:{port}/")).call() {
            Ok(r) => r.status() < 500,
            Err(ureq::Error::Status(c, _)) => c < 500,
            Err(_) => false,
        }
    }

    fn port_listening(&self, port: u16) -> bool {
        use std::net::{SocketAddr, TcpStream};
        TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_millis(400)).is_ok()
    }

    fn log_len(&self, path: &Path) -> u64 {
        std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
    }

    fn read_log(&self, path: &Path, from: u64) -> String {
        use std::io::{Read, Seek, SeekFrom};
        let Ok(mut f) = std::fs::File::open(path) else { return String::new() };
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        let from = if from > len { 0 } else { from };
        let _ = f.seek(SeekFrom::Start(from));
        let mut b = Vec::new();
        let _ = f.take(2 * 1024 * 1024).read_to_end(&mut b);
        String::from_utf8_lossy(&b).into_owned()
    }

    fn read_pid_file(&self, spec: &Spec) -> Option<u32> {
        std::fs::read_to_string(spec.pid_path()).ok()?.trim().parse().ok()
    }

    fn write_pid_file(&self, spec: &Spec, pid: u32) {
        let _ = crate::fsutil::write_no_follow(&spec.pid_path(), format!("{pid}\n").as_bytes(), 0o644);
    }

    fn remove_pid_file(&self, spec: &Spec) {
        let _ = std::fs::remove_file(spec.pid_path());
    }

    fn kill_tree(&self, pid: u32) -> bool {
        sys::kill_tree(pid)
    }

    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }

    fn now(&self) -> Duration {
        self.t0.elapsed()
    }
}

/// Resolve a registered name or folder to a [`Spec`].
pub fn spec_for(name_or_dir: &str) -> Result<Spec, crate::problem::Problem> {
    let dir = crate::installer::host::resolve_dir(name_or_dir)?;
    let sf = crate::installer::ServerFile::load(&dir).map_err(|e| format!("cannot read kora-server.json: {e}"))?;
    Ok(Spec { name: sf.name.clone(), dir, game_port: sf.ports.game, txadmin_port: sf.ports.txadmin })
}

/// `kora start` for a registered server or folder (service or detached).
pub fn start_named(name_or_dir: &str, progress: &mut dyn FnMut(&str, f32)) -> Result<Report, crate::problem::Problem> {
    let spec = spec_for(name_or_dir)?;
    Ok(start(&SystemEnv::default(), &spec, progress))
}

/// What `kora status` shows for a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub running: bool,
    pub txadmin_up: bool,
    pub game_listening: bool,
}

pub fn status_of(env: &dyn LaunchEnv, spec: &Spec) -> Status {
    let tx = env.txadmin_answers(spec.txadmin_port);
    let game = env.port_listening(spec.game_port);
    let alive = running_pid(env, spec).is_some() || env.service_state(spec) == ServiceState::Running;
    Status { running: alive || tx || game, txadmin_up: tx, game_listening: game }
}

/// Stop a detached server by name (the service case is handled by `host::service_control`).
pub fn stop_named_detached(name_or_dir: &str) -> Result<bool, crate::problem::Problem> {
    let spec = spec_for(name_or_dir)?;
    Ok(stop_detached(&SystemEnv::default(), &spec))
}

#[cfg(test)]
mod tests;
