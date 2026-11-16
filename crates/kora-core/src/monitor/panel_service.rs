//! The opt-in background panel: `kora panel --install-service` keeps `kora panel --no-open --port <p>`
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

pub fn token_path(home: &Path) -> PathBuf {
    home.join(TOKEN_FILE)
}
pub fn info_path(home: &Path) -> PathBuf {
    home.join(INFO_FILE)
}
pub fn pid_path(home: &Path) -> PathBuf {
    home.join(PID_FILE)
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

/// The port the installed service was set up with.
pub fn saved_port(home: &Path) -> Option<u16> {
    let v: serde_json::Value =
        serde_json::from_slice(&std::fs::read(info_path(home)).ok()?).ok()?;
    u16::try_from(v["port"].as_u64()?).ok()
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

/// The systemd user unit. `kora_home` is passed on only when it was customised.
pub fn systemd_unit(exe: &Path, port: u16, kora_home_env: Option<&Path>) -> String {
    let env = kora_home_env
        .map(|h| {
            format!(
                "Environment={}\n",
                systemd_quote(&format!("KORA_HOME={}", h.display()))
            )
        })
        .unwrap_or_default();
    format!(
        "[Unit]\nDescription=Kora Panel (monitoring and alerts)\nAfter=network.target\n\n[Service]\nType=simple\n{env}ExecStart={} panel --no-open --port {port}\nRestart=on-failure\nRestartSec=5\n\n[Install]\nWantedBy=default.target\n",
        systemd_quote(&exe.display().to_string())
    )
}

/// The Windows script: starts the panel with no console window.
pub fn startup_script(exe: &Path, port: u16) -> String {
    let exe = exe.display().to_string().replace('"', "");
    format!("' Starts Kora Panel in the background at sign-in. Remove it with: kora panel --remove-service\r\nCreateObject(\"WScript.Shell\").Run \"\"\"{exe}\"\" panel --no-open --port {port}\", 0, False\r\n")
}

pub struct InstallSpec<'a> {
    pub exe: &'a Path,
    pub port: u16,
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
                    systemd_unit(spec.exe, spec.port, spec.kora_home_env),
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
                    "Start the panel now and at every sign-in",
                    "systemctl",
                    &["--user", "enable", "--now", UNIT_NAME],
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
            Plan::new(vec![
                SystemStep::write(
                    "Write the Kora Panel start-up script",
                    &script,
                    startup_script(spec.exe, spec.port),
                    0o644,
                )
                .user_level(),
                SystemStep::run("Start the panel now", "wscript.exe", &["//nologo", &script])
                    .user_level(),
            ])
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
    let info = serde_json::json!({"port": spec.port}).to_string();
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
    for f in [token_path(home), info_path(home), pid_path(home)] {
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
            Some(Path::new("/data/kora home")),
        );
        assert!(
            u.contains("ExecStart=\"/opt/my kora/kora\" panel --no-open --port 47200\n"),
            "{u}"
        );
        assert!(
            u.contains("Environment=\"KORA_HOME=/data/kora home\"")
                && u.contains("WantedBy=default.target")
                && u.contains("Restart=on-failure")
        );
        assert!(!systemd_unit(Path::new("/k"), 1, None).contains("Environment="));
    }

    #[test]
    fn the_windows_script_is_hidden_and_quotes_the_path() {
        let s = startup_script(Path::new(r"C:\Program Files\Kora\kora.exe"), 47120);
        assert!(
            s.contains(
                r#"Run """C:\Program Files\Kora\kora.exe"" panel --no-open --port 47120", 0, False"#
            ),
            "{s}"
        );
    }

    #[test]
    fn plans_need_no_administrator_rights() {
        let spec = InstallSpec {
            exe: Path::new("/k/kora"),
            port: 47120,
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
                .any(|l| l.contains("systemctl --user enable --now kora-panel.service")),
            "{lines:?}"
        );
        assert!(remove_plan(Platform::Windows, Some(7))
            .steps
            .iter()
            .any(|s| s.command_line().is_some_and(|l| l.contains("/PID 7"))));
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
            kora_home_env: None,
        };
        let link = install(home.path(), &spec, &rn, Platform::Linux).unwrap();
        assert!(link.starts_with("http://127.0.0.1:47123/#token="));
        assert!(
            rn.calls()
                .iter()
                .any(|c| c.contains("enable --now kora-panel.service")),
            "{:?}",
            rn.calls()
        );
        remove(home.path(), &rn, Platform::Linux).unwrap();
        assert!(load_token(home.path()).is_none() && saved_port(home.path()).is_none());
    }
}
