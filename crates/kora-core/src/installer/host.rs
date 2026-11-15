//! `kora service-host <name-or-folder>`: what the service plans (systemd unit, Windows service) and
//! any supervisor run. Starts FXServer (txAdmin mode) for a server, given its registry name or its
//! folder (the folder form needs no registry, so it works for the service account).
//!
//! Unix: FXServer runs as a **supervised child**. SIGTERM/SIGINT are forwarded to it; if a stop was
//! requested and FXServer then dies with SIGSEGV (observed with artifact 38621: txAdmin logs "Got
//! SIGTERM, shutting down..." and the process segfaults in its own teardown, reported by systemd as
//! `code=dumped`, which neither `SuccessExitStatus=` nor `LimitCORE=0` can turn into success), we
//! exit 0 so `systemctl stop` leaves the unit `inactive`. Any crash that was not requested keeps its
//! failure code (128+signal) so `Restart=on-failure` still works.
//! Windows: when started by the Service Control Manager (SCM) it registers with it, reports
//! START_PENDING/RUNNING/STOP_PENDING/STOPPED, and on `sc stop`/shutdown ends FXServer and reports a
//! clean stop (a plain console program would be killed by the SCM after 30 s with error 1053). Started
//! from a terminal (error 1063 from the dispatcher) it just runs FXServer in the foreground.
//! FXServer is ended with TerminateProcess: a service has no console to send Ctrl+C to.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::registry::{Registry, ServerFile};
use super::{InstallError, Result};
use crate::artifacts::{ArtifactStore, Platform};
use crate::txadmin;

/// Resolve a registry name or an explicit server folder (containing `kora-server.json`) to a folder.
pub(crate) fn resolve_dir(name_or_dir: &str) -> Result<PathBuf> {
    let p = Path::new(name_or_dir);
    if p.is_absolute() && p.join("kora-server.json").is_file() {
        return Ok(p.to_path_buf());
    }
    let reg = Registry::load().map_err(|e| InstallError(format!("cannot read the server registry: {e}")))?;
    let entry = reg.find(name_or_dir).ok_or_else(|| InstallError(format!("no registered server named '{name_or_dir}'")))?;
    Ok(entry.dir.clone())
}

/// The command that starts a server (exposed for tests and `kora status`).
pub fn build_command(name_or_dir: &str) -> Result<Command> {
    let dir = resolve_dir(name_or_dir)?;
    let sf = ServerFile::load(&dir).map_err(|e| InstallError(format!("cannot read kora-server.json: {e}")))?;
    let store = ArtifactStore::new(dir.join("artifacts"));
    let build_dir: PathBuf = store
        .current_dir()
        .map_err(|e| InstallError(e.to_string()))?
        .ok_or_else(|| InstallError("this server has no FXServer build installed".into()))?;
    let exe = build_dir.join(Platform::current().executable());
    if !exe.is_file() {
        return Err(InstallError(format!("{} is missing", exe.display())));
    }
    let mut c = Command::new(exe);
    c.current_dir(&dir);
    for (k, v) in txadmin::txadmin_env(&dir.join("txData"), sf.ports.txadmin) {
        c.env(k, v);
    }
    Ok(c)
}

/// Copies FXServer's output to `<server>/.kora/console.log` (what the manager's Console screen shows),
/// to `extra` (the service log on Windows) and, when `echo` is set, to our own stdout (journal / terminal).
/// The pump threads end by themselves when FXServer closes its output.
pub struct ConsoleTee;

/// What the pump threads share: the log file, bytes written to it, its path, and the optional second copy.
struct TeeSink {
    file: Option<std::fs::File>,
    written: u64,
    path: PathBuf,
    extra: Option<std::fs::File>,
}

impl ConsoleTee {
    /// At most this much is kept in `console.log`; then it is moved to `console.log.old` and restarted.
    const MAX_BYTES: u64 = 20 * 1024 * 1024;

    pub fn start(dir: &Path, child: &mut std::process::Child, echo: bool, extra: Option<std::fs::File>) -> Self {
        let out = child.stdout.take().map(|o| Box::new(o) as Box<dyn std::io::Read + Send>);
        let err = child.stderr.take().map(|e| Box::new(e) as Box<dyn std::io::Read + Send>);
        Self::attach(dir, out, err, echo, extra)
    }

    /// Like [`start`](Self::start) for any two output streams.
    pub fn attach(dir: &Path, out: Option<Box<dyn std::io::Read + Send>>, err: Option<Box<dyn std::io::Read + Send>>, echo: bool, extra: Option<std::fs::File>) -> Self {
        use std::sync::{Arc, Mutex};
        let meta = dir.join(".kora");
        let _ = std::fs::create_dir_all(&meta);
        let path = meta.join("console.log");
        let _ = std::fs::rename(&path, meta.join("console.log.old"));
        let file = std::fs::File::create(&path).ok();
        let sink = Arc::new(Mutex::new(TeeSink { file, written: 0, path, extra }));
        let pump = |mut r: Box<dyn std::io::Read + Send>, sink: Arc<Mutex<TeeSink>>| {
            std::thread::spawn(move || {
                use std::io::{Read, Write};
                let mut buf = [0u8; 8192];
                loop {
                    let n = match r.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    if echo {
                        let mut o = std::io::stdout().lock();
                        let _ = o.write_all(&buf[..n]);
                        let _ = o.flush();
                    }
                    let mut g = sink.lock().unwrap();
                    if g.written > Self::MAX_BYTES {
                        let _ = std::fs::rename(&g.path, g.path.with_extension("log.old"));
                        g.file = std::fs::File::create(&g.path).ok();
                        g.written = 0;
                    }
                    if let Some(f) = g.file.as_mut() {
                        let _ = f.write_all(&buf[..n]);
                    }
                    if let Some(f) = g.extra.as_mut() {
                        let _ = f.write_all(&buf[..n]);
                    }
                    g.written += n as u64;
                }
            });
        };
        if let Some(o) = out {
            pump(o, Arc::clone(&sink));
        }
        if let Some(e) = err {
            pump(e, sink);
        }
        ConsoleTee
    }
}

/// Exit code `service-host` reports for FXServer's wait status.
/// `stop_requested`: we received SIGTERM/SIGINT before it ended. Pure, so it is unit-tested.
pub fn exit_code_for(code: Option<i32>, signal: Option<i32>, stop_requested: bool) -> i32 {
    const SIGINT: i32 = 2;
    const SIGSEGV: i32 = 11;
    const SIGTERM: i32 = 15;
    match (code, signal) {
        (Some(c), _) => c,
        (None, Some(s)) if stop_requested && matches!(s, SIGINT | SIGSEGV | SIGTERM) => 0,
        (None, Some(s)) => 128 + s,
        (None, None) => 1,
    }
}

/// Run the server; returns the exit code.
pub fn service_host(name_or_dir: &str) -> Result<i32> {
    // Windows: connect to the service manager FIRST. Anything that fails before that (missing build,
    // unreadable file) would exit without answering the manager, which reports a bare "1053" with no reason.
    // `scm::run` builds the command itself, after reporting "starting", and writes any error to the log.
    #[cfg(windows)]
    {
        scm::run_as_service_or_console(name_or_dir)
    }
    #[cfg(unix)]
    let mut cmd = build_command(name_or_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        let stop = Arc::new(AtomicBool::new(false));
        for sig in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT] {
            signal_hook::flag::register(sig, Arc::clone(&stop)).map_err(|e| InstallError(format!("cannot install signal handlers: {e}")))?;
        }
        let dir = resolve_dir(name_or_dir)?;
        cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| InstallError(format!("could not start FXServer: {e}")))?;
        let _console = ConsoleTee::start(&dir, &mut child, true, None);
        let pid = child.id() as i32;
        let mut forwarded = false;
        loop {
            if let Some(st) = child.try_wait().map_err(|e| InstallError(format!("waiting for FXServer failed: {e}")))? {
                let stop_requested = stop.load(Ordering::SeqCst);
                if stop_requested && st.signal() == Some(11) {
                    eprintln!("kora: FXServer segfaulted while shutting down after a stop request (known artifact behaviour); treating the stop as clean");
                }
                return Ok(exit_code_for(st.code(), st.signal(), stop_requested));
            }
            if stop.load(Ordering::SeqCst) && !forwarded {
                forwarded = true;
                // SAFETY: plain kill(2) on our own child's pid.
                unsafe {
                    libc::kill(pid, libc::SIGTERM);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let status = build_command(name_or_dir)?.status().map_err(|e| InstallError(format!("could not start FXServer: {e}")))?;
        Ok(status.code().unwrap_or(1))
    }
}

/// Exit code the Windows service reports. A stop that Windows asked for is always clean (0);
/// otherwise FXServer's own code, with 1 for "no code" so the restart-on-failure rule still applies.
pub fn windows_service_exit(code: Option<i32>, stop_requested: bool) -> u32 {
    if stop_requested {
        0
    } else {
        code.map_or(1, |c| c as u32)
    }
}

#[cfg(windows)]
mod scm {
    use super::*;
    use std::ffi::OsString;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{mpsc, OnceLock};
    use std::time::Duration;
    use windows_service::service::{ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType};
    use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
    use windows_service::{define_windows_service, service_dispatcher};

    /// The name is ignored for an own-process service.
    const NAME: &str = "kora";
    /// ERROR_FAILED_SERVICE_CONTROLLER_CONNECT: not started by the SCM.
    const NOT_A_SERVICE: i32 = 1063;
    static TARGET: OnceLock<String> = OnceLock::new();
    static EXIT: AtomicU32 = AtomicU32::new(1);

    define_windows_service!(ffi_service_main, service_main);

    /// `<server folder>/.kora/service-host.log`: FXServer's output and the reason it ended (a service has no console).
    fn open_service_log(target: &str) -> Option<std::fs::File> {
        let dir = resolve_dir(target).ok()?;
        std::fs::create_dir_all(dir.join(".kora")).ok()?;
        std::fs::OpenOptions::new().create(true).append(true).open(dir.join(".kora").join("service-host.log")).ok()
    }

    fn log_line(log: &Option<std::fs::File>, line: &str) {
        use std::io::Write;
        if let Some(mut f) = log.as_ref() {
            let _ = writeln!(f, "{line}");
        }
    }

    fn service_main(_args: Vec<OsString>) {
        let code = match run() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("kora service-host: {e}");
                1
            }
        };
        EXIT.store(code, Ordering::SeqCst);
    }

    pub fn run_as_service_or_console(target: &str) -> Result<i32> {
        let _ = TARGET.set(target.to_string());
        match service_dispatcher::start(NAME, ffi_service_main) {
            Ok(()) => Ok(EXIT.load(Ordering::SeqCst) as i32),
            Err(windows_service::Error::Winapi(e)) if e.raw_os_error() == Some(NOT_A_SERVICE) => {
                let mut cmd = build_command(target)?;
                let st = cmd.status().map_err(|e| InstallError(format!("could not start FXServer: {e}")))?;
                Ok(st.code().unwrap_or(1))
            }
            Err(e) => Err(InstallError(format!("cannot talk to the Windows service manager: {e}"))),
        }
    }

    fn run() -> std::result::Result<u32, String> {
        let target = TARGET.get().cloned().unwrap_or_default();
        let (tx, rx) = mpsc::channel::<()>();
        let handler = move |ev: ServiceControl| match ev {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                let _ = tx.send(());
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        };
        let h = service_control_handler::register(NAME, handler).map_err(|e| e.to_string())?;
        let report = |state: ServiceState, accept: ServiceControlAccept, wait: Duration, code: u32, checkpoint: u32| {
            let _ = h.set_service_status(ServiceStatus {
                service_type: ServiceType::OWN_PROCESS,
                current_state: state,
                controls_accepted: accept,
                exit_code: ServiceExitCode::Win32(code),
                checkpoint,
                wait_hint: wait,
                process_id: None,
            });
        };
        report(ServiceState::StartPending, ServiceControlAccept::empty(), Duration::from_secs(15), 0, 1);
        let log = open_service_log(&target);
        log_line(&log, &format!("kora service-host: starting FXServer for {target}"));
        let started = std::time::Instant::now();
        let mut child = match build_command(&target).map_err(|e| e.to_string()).and_then(|mut c| {
            c.stdin(std::process::Stdio::null());
            c.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
            c.spawn().map_err(|e| format!("could not start FXServer: {e}"))
        }) {
            Ok(c) => c,
            Err(e) => {
                log_line(&log, &format!("kora service-host: {e}"));
                report(ServiceState::Stopped, ServiceControlAccept::empty(), Duration::ZERO, 1, 0);
                return Err(e);
            }
        };
        let _console = resolve_dir(&target).ok().map(|d| super::ConsoleTee::start(&d, &mut child, false, log.as_ref().and_then(|f| f.try_clone().ok())));
        report(ServiceState::Running, ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN, Duration::ZERO, 0, 0);
        let (code, stop) = loop {
            if let Ok(Some(st)) = child.try_wait() {
                break (st.code(), false);
            }
            if rx.recv_timeout(Duration::from_millis(500)).is_ok() {
                report(ServiceState::StopPending, ServiceControlAccept::empty(), Duration::from_secs(30), 0, 1);
                let _ = child.kill();
                let st = child.wait().ok();
                break (st.and_then(|s| s.code()), true);
            }
        };
        let exit = windows_service_exit(code, stop);
        log_line(&log, &format!("kora service-host: FXServer ended after {}s (exit code {code:?}, stop requested: {stop}); reporting {exit}", started.elapsed().as_secs()));
        report(ServiceState::Stopped, ServiceControlAccept::empty(), Duration::ZERO, exit, 0);
        Ok(exit)
    }
}

/// Name of the OS service Kora registered for a registered server (`kora-<slug>`).
pub fn service_name_of(name: &str) -> Result<String> {
    let reg = Registry::load().map_err(|e| InstallError(format!("cannot read the server registry: {e}")))?;
    let entry = reg.find(name).ok_or_else(|| InstallError(format!("no registered server named '{name}'")))?;
    Ok(format!("kora-{}", super::slug(&entry.name)))
}

fn service_for(name: &str) -> Result<crate::service::ManagedServer> {
    let reg = Registry::load().map_err(|e| InstallError(format!("cannot read the server registry: {e}")))?;
    let entry = reg.find(name).ok_or_else(|| InstallError(format!("no registered server named '{name}'")))?;
    // Control verbs only use the service name; the other fields satisfy validation.
    Ok(crate::service::ManagedServer {
        name: super::slug(&entry.name),
        dir: entry.dir.to_string_lossy().into_owned(),
        start_command: vec!["kora".into()],
        user: "kora".into(),
        env_file: None,
        kora_exe: std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| "kora".into()),
        windows_run_as: None,
    })
}

/// Current OS service state of a registered server (`NotInstalled` when it has no service).
pub fn service_state(name: &str) -> Result<crate::service::ServiceState> {
    let server = service_for(name)?;
    let rn = crate::service::SystemRunner;
    crate::service::ServiceManager::new(crate::service::Platform::current(), &rn)
        .status(&server)
        .map_err(|e| InstallError(e.to_string()))
}

/// start / stop / restart the server's OS service.
pub fn service_control(name: &str, action: crate::service::ControlAction) -> Result<()> {
    use crate::service::ServiceState;
    if service_state(name)? == ServiceState::NotInstalled {
        return Err(InstallError(format!(
            "'{name}' has no background service. Install with --service (as administrator), or run `kora service-host {name}` in a terminal."
        )));
    }
    let server = service_for(name)?;
    let rn = crate::service::SystemRunner;
    let m = crate::service::ServiceManager::new(crate::service::Platform::current(), &rn);
    match action {
        crate::service::ControlAction::Start => m.start(&server),
        crate::service::ControlAction::Stop => m.stop(&server),
        crate::service::ControlAction::Restart => m.restart(&server),
    }
    .map_err(|e| InstallError(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::{exit_code_for, windows_service_exit};

    #[test]
    fn the_console_tee_copies_server_output_to_console_log() {
        // In-memory streams: no child process, so no SIGCHLD to interrupt other tests' socket reads.
        let d = tempfile::tempdir().unwrap();
        let out: Box<dyn std::io::Read + Send> = Box::new(std::io::Cursor::new(b"hello from fxserver\n".to_vec()));
        let err: Box<dyn std::io::Read + Send> = Box::new(std::io::Cursor::new(b"oops\n".to_vec()));
        let _t = super::ConsoleTee::attach(d.path(), Some(out), Some(err), false, None);
        let log = d.path().join(".kora").join("console.log");
        for _ in 0..50 {
            if std::fs::read_to_string(&log).is_ok_and(|t| t.contains("hello") && t.contains("oops")) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        panic!("console.log: {:?}", std::fs::read_to_string(&log));
    }

    #[test]
    fn windows_service_reports_a_requested_stop_as_clean_and_a_crash_as_failure() {
        assert_eq!(windows_service_exit(Some(1), true), 0, "TerminateProcess code is not a failure after sc stop");
        assert_eq!(windows_service_exit(None, true), 0);
        assert_eq!(windows_service_exit(Some(0xC0000005u32 as i32), false), 0xC0000005, "an access violation keeps its code so the restart rule runs");
        assert_eq!(windows_service_exit(None, false), 1);
    }

    #[test]
    fn stop_with_segv_is_clean_but_crash_is_not() {
        assert_eq!(exit_code_for(None, Some(11), true), 0, "segv after SIGTERM = clean stop");
        assert_eq!(exit_code_for(None, Some(15), true), 0);
        assert_eq!(exit_code_for(None, Some(11), false), 139, "unrequested segv stays a failure");
        assert_eq!(exit_code_for(None, Some(9), true), 137, "SIGKILL is never hidden");
        assert_eq!(exit_code_for(Some(3), None, true), 3);
        assert_eq!(exit_code_for(Some(0), None, false), 0);
    }
}
