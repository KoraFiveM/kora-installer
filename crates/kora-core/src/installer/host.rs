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
//! Windows: runs FXServer as a child and waits. TODO: report to the Service Control Manager
//! (needs the `windows-service` crate) so `sc stop` can forward a graceful stop; untested here.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::registry::{Registry, ServerFile};
use super::{InstallError, Result};
use crate::artifacts::{ArtifactStore, Platform};
use crate::txadmin;

/// Resolve a registry name or an explicit server folder (containing `kora-server.json`) to a folder.
fn resolve_dir(name_or_dir: &str) -> Result<PathBuf> {
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
        let mut child = cmd.spawn().map_err(|e| InstallError(format!("could not start FXServer: {e}")))?;
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
    #[cfg(not(unix))]
    {
        let status = cmd.status().map_err(|e| InstallError(format!("could not start FXServer: {e}")))?;
        Ok(status.code().unwrap_or(1))
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
    use super::exit_code_for;

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
