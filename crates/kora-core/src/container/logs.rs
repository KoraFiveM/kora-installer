//! The console copy: the container's output, where the Console and Logs tabs read it.
//!
//! A service-mode server runs under `kora service-host`, which copies FXServer's output to
//! `<server>/.kora/console.log` (see `installer::host::ConsoleTee`). A container's output belongs to the
//! engine, so a small follower does the same: `kora container-logs <folder>` runs `<engine> logs --follow`
//! and feeds it to the very same `ConsoleTee`. The follower is started (detached, in its own session) when
//! Kora starts the container, and started again by the monitor's next look when it is found missing (the
//! container was restarted by the engine, or after a reboot). It ends by itself when the container stops.
//! Only one runs per folder: its process id is in `.kora/container-logs.pid` and is checked, by command line,
//! before a new one starts. Nothing here ever reads a secret: the log is what FXServer printed.

use std::path::{Path, PathBuf};

#[cfg(unix)]
use super::engine;
#[cfg(unix)]
use super::record::ContainerRecord;

pub fn pid_path(dir: &Path) -> PathBuf {
    dir.join(".kora").join("container-logs.pid")
}

/// Is a follower for this folder running? (Its pid file names a live process whose command line says so.)
#[cfg(unix)]
pub fn follower_running(dir: &Path) -> bool {
    let Some(pid) = std::fs::read_to_string(pid_path(dir)).ok().and_then(|t| t.trim().parse::<u32>().ok()) else { return false };
    let Ok(cmd) = std::fs::read(format!("/proc/{pid}/cmdline")) else { return false };
    let text = String::from_utf8_lossy(&cmd);
    text.split('\0').any(|a| a == "container-logs") && text.split('\0').any(|a| Path::new(a) == dir)
}

#[cfg(not(unix))]
pub fn follower_running(_dir: &Path) -> bool {
    false
}

/// What `--since` gets: the container's own start time when known, else ten minutes back.
pub fn since_of(started_at: &str) -> String {
    let t = started_at.trim();
    let shaped = t.len() >= 19 && t.as_bytes()[4] == b'-' && !t.starts_with("0001-");
    if !shaped {
        return "10m".to_string();
    }
    // Docker prints RFC 3339 (`2026-11-17T21:46:01.5Z`). Podman prints Go's own form (`2026-11-17 21:46:01.594366672 +0000 UTC`),
    // which `podman logs --since` rejects ("unable to interpret time value"): turn that into RFC 3339 (UTC only).
    if t.as_bytes()[10] == b'T' {
        return t.to_string();
    }
    let mut parts = t.split_whitespace();
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(date), Some(time), Some("+0000"), Some("UTC")) => format!("{date}T{time}Z"),
        _ => "10m".to_string(),
    }
}

/// Run the follower in this process until the container's log ends. Returns the exit code.
#[cfg(unix)]
pub fn run_follower(dir: &Path) -> i32 {
    use std::process::Stdio;
    let Some(rec) = ContainerRecord::load(dir) else {
        eprintln!("kora: {} is not a container-mode server", dir.display());
        return 1;
    };
    let engine = rec.engine();
    let started = engine
        .run(&super::lifecycle::system(), &["inspect".into(), "--format".into(), "{{.State.StartedAt}}".into(), rec.container.clone()])
        .ok()
        .filter(|o| o.success())
        .map(|o| o.stdout)
        .unwrap_or_default();
    let args = engine::logs_follow_args(&rec.container, &since_of(&started));
    let _ = std::fs::create_dir_all(dir.join(".kora"));
    let _ = std::fs::write(pid_path(dir), format!("{}\n", std::process::id()));
    let mut child = match crate::sysexe::command(engine.program()).args(&args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("kora: cannot run {}: {e}", engine.program());
            let _ = std::fs::remove_file(pid_path(dir));
            return 1;
        }
    };
    let out = child.stdout.take().map(|o| Box::new(o) as Box<dyn std::io::Read + Send>);
    let err = child.stderr.take().map(|e| Box::new(e) as Box<dyn std::io::Read + Send>);
    let _tee = crate::installer::host::ConsoleTee::attach(dir, out, err, false, None);
    let code = child.wait().ok().and_then(|s| s.code()).unwrap_or(1);
    // Let the pump threads write what is left.
    std::thread::sleep(std::time::Duration::from_millis(300));
    let _ = std::fs::remove_file(pid_path(dir));
    code
}

#[cfg(not(unix))]
pub fn run_follower(_dir: &Path) -> i32 {
    1
}

/// Start a detached follower unless one runs. Never fails the caller: a missing console copy must not stop a
/// server from starting. Tests (and any test binary) never start one.
#[cfg(unix)]
pub fn ensure_follower(dir: &Path) {
    use std::os::unix::process::CommandExt;
    if follower_running(dir) || ContainerRecord::load(dir).is_none() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let stem = exe.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    if crate::installer::registry::is_test_binary_name(&stem) || std::env::var_os("KORA_NO_LOG_FOLLOWER").is_some() {
        return;
    }
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("container-logs").arg(dir).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    // SAFETY: setsid is async-signal-safe; it only detaches the child from our session so it outlives the caller.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    if let Ok(child) = cmd.spawn() {
        // Not waited for; reap it when it ends so it never lingers as a zombie of a long-running panel.
        std::thread::spawn(move || {
            let mut c = child;
            let _ = c.wait();
        });
    }
}

#[cfg(not(unix))]
pub fn ensure_follower(_dir: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_follower_reads_from_the_containers_own_start() {
        assert_eq!(since_of("2025-11-17T19:56:01.123456789Z\n"), "2025-11-17T19:56:01.123456789Z");
        assert_eq!(since_of("0001-01-01T00:00:00Z"), "10m");
        assert_eq!(since_of(""), "10m");
        assert_eq!(since_of("<no value>"), "10m");
        assert_eq!(since_of("2026-11-17 21:46:01.594366672 +0000 UTC"), "2026-11-17T21:46:01.594366672Z", "podman prints Go time");
        assert_eq!(since_of("2026-11-17 21:46:01 +0100 CET"), "10m");
    }

    #[test]
    fn no_pid_file_means_no_follower() {
        let d = tempfile::tempdir().unwrap();
        assert!(!follower_running(d.path()));
        std::fs::create_dir_all(d.path().join(".kora")).unwrap();
        std::fs::write(pid_path(d.path()), "999999999\n").unwrap();
        assert!(!follower_running(d.path()), "a stale id is not a follower");
        std::fs::write(pid_path(d.path()), format!("{}\n", std::process::id())).unwrap();
        assert!(!follower_running(d.path()), "a live process that is not a follower for this folder is not one either");
    }

    #[test]
    fn a_test_process_never_starts_a_follower() {
        let d = tempfile::tempdir().unwrap();
        ensure_follower(d.path());
        assert!(!pid_path(d.path()).exists());
    }
}
