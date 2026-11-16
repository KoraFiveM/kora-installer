//! The opt-in background panel: `kora panel --install-service` keeps `kora panel --no-open --foreground --port <p>`
//! running so monitoring and alerts work while nobody has the panel open.
//!
//! * Linux: a systemd **user** unit (`~/.config/systemd/user/kora-panel.service`), no administrator
//!   rights needed. `loginctl enable-linger` is tried so it also runs while nobody is signed in.
//! * Windows: a hidden script in the user's Startup folder (runs at every sign-in, no administrator
//!   rights needed; a scheduled task would need them). Started straight away too.
//!
//! The panel needs the same sign-in token every time: it is kept in `<kora home>/panel-token`
//! (owner-only). `kora panel --print-link` reads it back. All steps are a data [`Plan`], so the unit and
//! script text is tested on every OS without touching the machine.

use std::path::{Path, PathBuf};

use crate::installer::registry::kora_home;
use crate::service::{
    apply_plan, CommandRunner, Plan, Platform, ServiceError, ServiceResult, SystemStep,
};

pub const UNIT_NAME: &str = "kora-panel.service";
const TOKEN_FILE: &str = "panel-token";
const INFO_FILE: &str = "panel-service.json";
const PID_FILE: &str = "panel.pid";
const RUNNING_FILE: &str = "panel-running.json";

pub fn token_path(home: &Path) -> PathBuf {
    home.join(TOKEN_FILE)
}
pub fn info_path(home: &Path) -> PathBuf {
    home.join(INFO_FILE)
}
pub fn pid_path(home: &Path) -> PathBuf {
    home.join(PID_FILE)
}
pub fn running_path(home: &Path) -> PathBuf {
    home.join(RUNNING_FILE)
}

/// A panel started with `kora panel` (in a window or by the background service), so a second
/// `kora panel` can show its link instead of failing to listen on the same port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningPanel {
    pub port: u16,
    pub pid: u32,
    pub token: String,
}

impl RunningPanel {
    pub fn link(&self) -> String {
        format!("http://127.0.0.1:{}/#token={}", self.port, self.token)
    }
}

/// Remember a running panel (owner-only: the record holds its sign-in token).
pub fn record_running(home: &Path, p: &RunningPanel) -> Result<(), String> {
    let text = serde_json::json!({"port": p.port, "pid": p.pid, "token": p.token}).to_string();
    super::write_private(&running_path(home), &text)
}

/// The recorded running panel, if any (it may have stopped since: check it answers).
pub fn load_running(home: &Path) -> Option<RunningPanel> {
    let v: serde_json::Value =
        serde_json::from_slice(&std::fs::read(running_path(home)).ok()?).ok()?;
    let token = v["token"].as_str()?.to_string();
    (!token.is_empty() && token.bytes().all(|b| b.is_ascii_hexdigit())).then_some(())?;
    Some(RunningPanel {
        port: u16::try_from(v["port"].as_u64()?).ok()?,
        pid: u32::try_from(v["pid"].as_u64()?).ok()?,
        token,
    })
}

/// Forget the running panel, but only if the record is still ours (`pid`).
pub fn clear_running(home: &Path, pid: u32) {
    if load_running(home).is_none_or(|r| r.pid == pid) {
        let _ = std::fs::remove_file(running_path(home));
    }
}

/// Is the background service set up here (unit file or start-up script present)?
pub fn service_installed(platform: Platform) -> bool {
    match platform {
        Platform::Linux => unit_path().exists(),
        Platform::Windows => startup_script_path().exists(),
    }
}

/// The saved sign-in token, when a service has been installed.
pub fn load_token(home: &Path) -> Option<String> {
    let t = std::fs::read_to_string(token_path(home)).ok()?;
    let t = t.trim();
    (t.len() >= 32 && t.bytes().all(|b| b.is_ascii_hexdigit())).then(|| t.to_string())
}

/// Keep the saved token, or make one (random, 256 bits) and save it owner-only.
pub fn ensure_token(home: &Path) -> Result<String, String> {
    if let Some(t) = load_token(home) {
        return Ok(t);
    }
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).map_err(|e| format!("Could not make a sign-in token: {e}"))?;
    let t = hex::encode(b);
    super::write_private(&token_path(home), &t)
        .map_err(|e| format!("Could not save the sign-in token: {e}"))?;
    Ok(t)
}

fn saved_info(home: &Path) -> Option<serde_json::Value> {
    serde_json::from_slice(&std::fs::read(info_path(home)).ok()?).ok()
}

/// The port the installed service was set up with.
pub fn saved_port(home: &Path) -> Option<u16> {
    u16::try_from(saved_info(home)?["port"].as_u64()?).ok()
}

/// Was the installed service set up to listen on Tailscale too (`kora panel --tailscale`)?
pub fn saved_tailscale(home: &Path) -> bool {
    saved_info(home).is_some_and(|v| v["tailscale"].as_bool() == Some(true))
}

/// Make a new sign-in token (the old link stops working once the panel restarts with it).
pub fn rotate_token(home: &Path) -> Result<String, String> {
    let _ = std::fs::remove_file(token_path(home));
    ensure_token(home)
}

/// The sign-in link of the installed service (needs no running process: the token is on disk).
pub fn link(home: &Path) -> Result<String, String> {
    let (Some(token), Some(port)) = (load_token(home), saved_port(home)) else {
        return Err("The Kora Panel background service is not installed. Install it with `kora panel --install-service`.".into());
    };
    Ok(format!("http://127.0.0.1:{port}/#token={token}"))
}

fn user_home() -> PathBuf {
    PathBuf::from(
        std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .unwrap_or_default(),
    )
}

/// Where the systemd user unit lives.
pub fn unit_path() -> PathBuf {
    match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        Some(x) => PathBuf::from(x),
        None => user_home().join(".config"),
    }
    .join("systemd")
    .join("user")
    .join(UNIT_NAME)
}

/// Where the Windows start-up script lives.
pub fn startup_script_path() -> PathBuf {
    let appdata = std::env::var_os("APPDATA").map_or_else(
        || user_home().join("AppData").join("Roaming"),
        PathBuf::from,
    );
    appdata
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs")
        .join("Startup")
        .join("Kora Panel.vbs")
}

fn systemd_quote(s: &str) -> String {
    format!(
        "\"{}\"",
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
    )
}

/// The panel's command line in the service (after the program).
fn service_args(port: u16, tailscale: bool) -> String {
    // `--listen-tailscale` only listens; `--tailscale` would also redo the set-up at every start.
    let ts = if tailscale { " --listen-tailscale" } else { "" };
    format!("panel --no-open --foreground --port {port}{ts}")
}

/// The systemd user unit. `kora_home` is passed on only when it was customised.
pub fn systemd_unit(exe: &Path, port: u16, tailscale: bool, kora_home_env: Option<&Path>) -> String {
    let env = kora_home_env
        .map(|h| {
            format!(
                "Environment={}\n",
                systemd_quote(&format!("KORA_HOME={}", h.display()))
            )
        })
        .unwrap_or_default();
    format!(
        "[Unit]\nDescription=Kora Panel (monitoring and alerts)\nAfter=network.target\n\n[Service]\nType=simple\n{env}ExecStart={} {}\nRestart=on-failure\nRestartSec=5\n\n[Install]\nWantedBy=default.target\n",
        systemd_quote(&exe.display().to_string()),
        service_args(port, tailscale)
    )
}

/// The Windows script: starts the panel with no console window.
pub fn startup_script(exe: &Path, port: u16, tailscale: bool) -> String {
    let exe = exe.display().to_string().replace('"', "");
    let args = service_args(port, tailscale);
    format!("' Starts Kora Panel in the background at sign-in. Remove it with: kora panel --remove-service\r\nCreateObject(\"WScript.Shell\").Run \"\"\"{exe}\"\" {args}\", 0, False\r\n")
}

pub struct InstallSpec<'a> {
    pub exe: &'a Path,
    pub port: u16,
    /// Also listen on the Tailscale addresses (`kora panel --tailscale`).
    pub tailscale: bool,
    /// Windows: the running background panel to stop first. There is no service manager to restart it,
    /// and a new panel cannot take the port while the old one holds it (so a new token or setting
    /// would silently not apply). Only set when that process is known to be our panel.
    pub stop_pid: Option<u32>,
    /// Set when `KORA_HOME` is customised, so the service sees the same home.
    pub kora_home_env: Option<&'a Path>,
}

/// Everything installing the service does, in order. All user level: no administrator rights.
pub fn install_plan(spec: &InstallSpec<'_>, platform: Platform) -> Plan {
    match platform {
        Platform::Linux => {
            let unit = unit_path();
            Plan::new(vec![
                SystemStep::write(
                    "Write the Kora Panel user service",
                    &unit.display().to_string(),
                    systemd_unit(spec.exe, spec.port, spec.tailscale, spec.kora_home_env),
                    0o644,
                )
                .user_level(),
                SystemStep::run(
                    "Reload the user services",
                    "systemctl",
                    &["--user", "daemon-reload"],
                )
                .user_level(),
                SystemStep::run(
                    "Start the panel at every sign-in",
                    "systemctl",
                    &["--user", "enable", UNIT_NAME],
                )
                .user_level(),
                // Restart (not just start) so a changed port takes effect when it was already running.
                SystemStep::run(
                    "Start the panel now",
                    "systemctl",
                    &["--user", "restart", UNIT_NAME],
                )
                .user_level(),
                // So it also runs while nobody is signed in. Not every system allows it: that is fine.
                SystemStep::run(
                    "Keep it running when you are signed out",
                    "loginctl",
                    &["enable-linger"],
                )
                .user_level()
                .tolerant(),
            ])
        }
        Platform::Windows => {
            let script = startup_script_path().display().to_string();
            let mut steps = Vec::new();
            if let Some(pid) = spec.stop_pid {
                steps.push(
                    SystemStep::run_owned(
                        "Stop the running panel",
                        "taskkill.exe",
                        vec!["/PID".into(), pid.to_string(), "/F".into()],
                    )
                    .user_level()
                    .tolerant(),
                );
            }
            steps.extend([
                SystemStep::write(
                    "Write the Kora Panel start-up script",
                    &script,
                    startup_script(spec.exe, spec.port, spec.tailscale),
                    0o644,
                )
                .user_level(),
                SystemStep::run("Start the panel now", "wscript.exe", &["//nologo", &script])
                    .user_level(),
            ]);
            Plan::new(steps)
        }
    }
}

/// Everything removing it does. `running_pid` is the panel's own process on Windows (no service manager there).
pub fn remove_plan(platform: Platform, running_pid: Option<u32>) -> Plan {
    match platform {
        Platform::Linux => Plan::new(vec![
            SystemStep::run(
                "Stop the panel service",
                "systemctl",
                &["--user", "disable", "--now", UNIT_NAME],
            )
            .user_level()
            .tolerant(),
            SystemStep::remove(
                "Remove the Kora Panel user service",
                &unit_path().display().to_string(),
            )
            .user_level(),
            SystemStep::run(
                "Reload the user services",
                "systemctl",
                &["--user", "daemon-reload"],
            )
            .user_level()
            .tolerant(),
        ]),
        Platform::Windows => {
            let mut steps = Vec::new();
            if let Some(pid) = running_pid {
                steps.push(
                    SystemStep::run_owned(
                        "Stop the running panel",
                        "taskkill.exe",
                        vec!["/PID".into(), pid.to_string(), "/F".into()],
                    )
                    .user_level()
                    .tolerant(),
                );
            }
            steps.push(
                SystemStep::remove(
                    "Remove the Kora Panel start-up script",
                    &startup_script_path().display().to_string(),
                )
                .user_level(),
            );
            Plan::new(steps)
        }
    }
}

/// Install: keep or make the token, remember the port, then run the plan. Returns the sign-in link.
pub fn install(
    home: &Path,
    spec: &InstallSpec<'_>,
    runner: &dyn CommandRunner,
    platform: Platform,
) -> ServiceResult<String> {
    ensure_token(home).map_err(ServiceError::Other)?;
    let info = serde_json::json!({"port": spec.port, "tailscale": spec.tailscale}).to_string();
    std::fs::create_dir_all(home)?;
    std::fs::write(info_path(home), info)?;
    apply_plan(&install_plan(spec, platform), runner)?;
    link(home).map_err(ServiceError::Other)
}

/// Remove the service and the saved token and port.
pub fn remove(home: &Path, runner: &dyn CommandRunner, platform: Platform) -> ServiceResult<()> {
    let pid = std::fs::read_to_string(pid_path(home))
        .ok()
        .and_then(|t| t.trim().parse().ok());
    apply_plan(&remove_plan(platform, pid), runner)?;
    for f in [
        token_path(home),
        info_path(home),
        pid_path(home),
        running_path(home),
    ] {
        let _ = std::fs::remove_file(f);
    }
    Ok(())
}

/// The real Kora home (for callers that have no override).
pub fn home() -> PathBuf {
    kora_home()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::testing::FakeRunner;

    #[test]
    fn the_unit_runs_the_panel_without_a_browser_and_quotes_paths() {
        let u = systemd_unit(
            Path::new("/opt/my kora/kora"),
            47200,
            false,
            Some(Path::new("/data/kora home")),
        );
        assert!(
            u.contains("ExecStart=\"/opt/my kora/kora\" panel --no-open --foreground --port 47200\n"),
            "{u}"
        );
        assert!(
            u.contains("Environment=\"KORA_HOME=/data/kora home\"")
                && u.contains("WantedBy=default.target")
                && u.contains("Restart=on-failure")
        );
        assert!(!systemd_unit(Path::new("/k"), 1, false, None).contains("Environment="));
        assert!(systemd_unit(Path::new("/k"), 1, true, None).contains("ExecStart=\"/k\" panel --no-open --foreground --port 1 --listen-tailscale\n"));
    }

    #[test]
    fn the_windows_script_is_hidden_and_quotes_the_path() {
        let s = startup_script(Path::new(r"C:\Program Files\Kora\kora.exe"), 47120, false);
        assert!(startup_script(Path::new("k.exe"), 1, true).contains("--port 1 --listen-tailscale\", 0, False"));
        assert!(
            s.contains(
                r#"Run """C:\Program Files\Kora\kora.exe"" panel --no-open --foreground --port 47120", 0, False"#
            ),
            "{s}"
        );
    }

    #[test]
    fn plans_need_no_administrator_rights() {
        let spec = InstallSpec {
            exe: Path::new("/k/kora"),
            port: 47120,
            tailscale: false,
            stop_pid: None,
            kora_home_env: None,
        };
        for p in [Platform::Linux, Platform::Windows] {
            assert!(!install_plan(&spec, p).needs_admin(), "{p:?}");
            assert!(!remove_plan(p, Some(7)).needs_admin(), "{p:?}");
        }
        let lines: Vec<String> = install_plan(&spec, Platform::Linux)
            .steps
            .iter()
            .filter_map(SystemStep::command_line)
            .collect();
        assert!(
            lines
                .iter()
                .any(|l| l.contains("systemctl --user enable kora-panel.service")),
            "{lines:?}"
        );
        assert!(remove_plan(Platform::Windows, Some(7))
            .steps
            .iter()
            .any(|s| s.command_line().is_some_and(|l| l.contains("/PID 7"))));
        // Windows: a running panel is stopped first, so the new token or settings take effect.
        let restart = InstallSpec { exe: Path::new("k.exe"), port: 1, tailscale: false, stop_pid: Some(42), kora_home_env: None };
        let lines: Vec<String> = install_plan(&restart, Platform::Windows).steps.iter().filter_map(SystemStep::command_line).collect();
        assert!(lines[0].contains("taskkill.exe /PID 42 /F") && lines[1].contains("wscript.exe"), "{lines:?}");
        assert!(!remove_plan(Platform::Windows, None)
            .steps
            .iter()
            .any(|s| s.command_line().is_some()));
    }

    #[test]
    fn the_token_is_made_once_kept_private_and_gives_the_link() {
        let home = tempfile::tempdir().unwrap();
        assert!(link(home.path()).is_err());
        let t1 = ensure_token(home.path()).unwrap();
        assert_eq!(t1.len(), 64);
        assert_eq!(
            ensure_token(home.path()).unwrap(),
            t1,
            "a second install keeps the token"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(token_path(home.path()))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o077,
                0
            );
        }
        // A link needs the port too.
        assert!(link(home.path()).is_err());
        std::fs::write(info_path(home.path()), r#"{"port":47999}"#).unwrap();
        assert_eq!(
            link(home.path()).unwrap(),
            format!("http://127.0.0.1:47999/#token={t1}")
        );
    }

    #[test]
    fn install_and_remove_run_the_plan_through_the_runner() {
        let home = tempfile::tempdir().unwrap();
        let rn = FakeRunner::default();
        let spec = InstallSpec {
            exe: Path::new("/k/kora"),
            port: 47123,
            tailscale: true,
            stop_pid: None,
            kora_home_env: None,
        };
        let link = install(home.path(), &spec, &rn, Platform::Linux).unwrap();
        assert!(saved_tailscale(home.path()));
        let old = load_token(home.path()).unwrap();
        assert_ne!(rotate_token(home.path()).unwrap(), old, "a new token");
        assert!(link.starts_with("http://127.0.0.1:47123/#token="));
        assert!(
            rn.calls()
                .iter()
                .any(|c| c.contains("restart kora-panel.service")),
            "{:?}",
            rn.calls()
        );
        remove(home.path(), &rn, Platform::Linux).unwrap();
        assert!(load_token(home.path()).is_none() && saved_port(home.path()).is_none());
    }

    #[test]
    fn a_running_panel_is_recorded_privately_and_cleared_only_by_itself() {
        let home = tempfile::tempdir().unwrap();
        assert!(load_running(home.path()).is_none());
        let p = RunningPanel {
            port: 47121,
            pid: 42,
            token: "ab".repeat(32),
        };
        record_running(home.path(), &p).unwrap();
        assert_eq!(load_running(home.path()), Some(p.clone()));
        assert_eq!(
            p.link(),
            format!("http://127.0.0.1:47121/#token={}", "ab".repeat(32))
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(running_path(home.path()))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0);
        }
        clear_running(home.path(), 7);
        assert!(
            load_running(home.path()).is_some(),
            "another panel's record is kept"
        );
        clear_running(home.path(), 42);
        assert!(load_running(home.path()).is_none());
        std::fs::write(
            running_path(home.path()),
            r#"{"port":1,"pid":2,"token":"not hex!"}"#,
        )
        .unwrap();
        assert!(load_running(home.path()).is_none());
    }
}
