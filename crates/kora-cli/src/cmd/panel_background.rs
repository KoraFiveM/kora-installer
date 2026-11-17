//! The background panel (the per-user service that keeps the panel running): install, restart and `kora panel --setup`.

use crate::cmd::panel::{headless_session, PanelArgs};
use crate::cmd::panel_running::{background_port, one_time_link, running_panel, Running};
use std::path::PathBuf;

use clap::Parser;
use kora_core::firewall::ports;
use kora_core::launch;

/// Install and start the background panel (systemd user service on Linux, a Startup entry on Windows).
pub(crate) fn install_background_panel(a: &PanelArgs, home: &std::path::Path) -> i32 {
    let port = match background_port(a, home, &kora_web::launch::SystemPorts) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    match start_background_service(home, port, svc_tailscale(home)) {
        Ok(link) => {
            let ports = kora_web::launch::SystemPorts;
            let link = if kora_web::launch::wait_for_panel(port, SETUP_WAIT, std::time::Duration::from_millis(250), &ports) { one_time_link(&link, port) } else { link };
            println!("The Kora Panel now runs in the background (starts at sign-in) and keeps watching your servers.");
            println!("Sign in with: {link}");
            if headless_session() {
                println!("On a server with no screen, open a tunnel from your own computer first:  {}", kora_web::launch::ssh_hint(port, &kora_web::launch::real_env));
            }
            println!("Show the link again any time with `kora panel`; remove it with `kora panel --remove-service`.");
            println!("The link contains your sign-in token: keep it private.");
            0
        }
        Err(e) => {
            eprintln!("error: could not set up the background panel: {e}");
            1
        }
    }
}

/// Does the installed background panel also listen on Tailscale? Kept when it is set up again.
pub(crate) fn svc_tailscale(home: &std::path::Path) -> bool {
    kora_core::monitor::panel_service::saved_tailscale(home)
}

/// Write and start the background service on `port`. Returns the sign-in link.
pub(crate) fn start_background_service(home: &std::path::Path, port: u16, tailscale: bool) -> Result<String, String> {
    use kora_core::monitor::panel_service as svc;
    let runner = kora_core::service::SystemRunner;
    let platform = kora_core::service::Platform::current();
    let exe = std::env::current_exe().map_err(|e| format!("cannot find the kora program to run in the background: {e}"))?;
    let home_env = std::env::var_os("KORA_HOME").map(std::path::PathBuf::from);
    // Windows has no service manager to restart the panel: stop the running one (only when it really is
    // our panel answering on its saved port), else the new one cannot take the port.
    let stop_pid = windows_panel_pid(home, platform);
    let spec = svc::InstallSpec { exe: &exe, port, tailscale, stop_pid, kora_home_env: home_env.as_deref() };
    svc::install(home, &spec, &runner, platform).map_err(|e| e.to_string())
}

/// Windows: the process of the running background panel, only when it really is our panel answering
/// on its saved port. `None` on Linux, where systemd stops it.
fn windows_panel_pid(home: &std::path::Path, platform: kora_core::service::Platform) -> Option<u32> {
    use kora_core::monitor::panel_service as svc;
    (platform == kora_core::service::Platform::Windows)
        .then(|| svc::saved_port(home))
        .flatten()
        .filter(|p| kora_web::launch::PortCheck::is_kora_panel(&kora_web::launch::SystemPorts, *p))
        .and_then(|_| std::fs::read_to_string(svc::pid_path(home)).ok())
        .and_then(|t| t.trim().parse().ok())
}

/// What `kora panel restart` can do.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PanelRestart {
    /// The background panel is set up: restart it (this also starts it when it had stopped).
    Background,
    /// Only a `kora panel` in a window runs: it has to be stopped there.
    Window,
    /// Nothing to restart.
    NotRunning,
}

fn panel_restart_mode(background_installed: bool, window_running: bool) -> PanelRestart {
    if background_installed {
        PanelRestart::Background
    } else if window_running {
        PanelRestart::Window
    } else {
        PanelRestart::NotRunning
    }
}

/// `kora panel restart`: restart the background panel and print where to sign in. Token, port and
/// settings stay as they are.
pub(crate) fn panel_restart(home: &std::path::Path) -> i32 {
    use kora_core::monitor::panel_service as svc;
    use kora_core::service::Platform;
    let ports = kora_web::launch::SystemPorts;
    let platform = Platform::current();
    let installed = svc::service_installed(platform);
    let window = !installed && running_panel(home, &ports, platform).is_some();
    match panel_restart_mode(installed, window) {
        PanelRestart::Background => {}
        PanelRestart::Window => {
            eprintln!("error: Kora Panel runs in a window here, not in the background, so it cannot be restarted from another window. Stop it there (Ctrl+C) and run `kora panel` again.");
            eprintln!("(`kora panel --install-service` keeps one running in the background instead.)");
            return 1;
        }
        PanelRestart::NotRunning => {
            eprintln!("error: Kora Panel is not running, so there is nothing to restart. Start it with `kora panel` (or keep it running in the background with `kora panel --install-service`).");
            return 1;
        }
    }
    if platform == Platform::Linux && !user_services_available() {
        eprintln!("error: this session cannot reach the background panel (common after sudo or su). Sign in as the user who runs it (not with sudo) and run `kora panel restart` again.");
        return 1;
    }
    let port = svc::saved_port(home).unwrap_or(kora_web::DEFAULT_PORT);
    if let Err(e) = svc::restart(&kora_core::service::SystemRunner, platform, windows_panel_pid(home, platform)) {
        eprintln!("error: could not restart the background panel: {e}");
        return 1;
    }
    if !kora_web::launch::wait_for_panel(port, SETUP_WAIT, std::time::Duration::from_millis(250), &ports) {
        eprintln!("Kora Panel was restarted, but did not answer within {} seconds.", SETUP_WAIT.as_secs());
        if platform == Platform::Linux {
            eprintln!("See what happened with:  systemctl --user status kora-panel");
        }
        return 1;
    }
    println!("Kora Panel restarted. Pages that were open sign in again.");
    if let Ok(link) = svc::link(home) {
        println!("Sign in with: {}", one_time_link(&link, port));
    }
    if headless_session() {
        println!("On a server with no screen, open a tunnel from your own computer first:  {}", kora_web::launch::ssh_hint(port, &kora_web::launch::real_env));
    }
    crate::cmd::panel_tailscale::print_tailnet_address(home, port);
    0
}

/// Can this session use systemd user services? Under `sudo` or `su` there is often no user service
/// manager to talk to. Never waits more than a few seconds.
pub(crate) fn user_services_available() -> bool {
    use kora_web::launch::{run_and_wait, OpenerCommand, OpenerExit};
    // `sudo` drops XDG_RUNTIME_DIR; systemctl needs it to find the user service manager.
    if std::env::var_os("XDG_RUNTIME_DIR").is_none() {
        if let Some(uid) = kora_web::launch::current_uid() {
            let dir = format!("/run/user/{uid}");
            if std::path::Path::new(&dir).is_dir() {
                std::env::set_var("XDG_RUNTIME_DIR", dir);
            }
        }
    }
    let check = OpenerCommand { program: "systemctl".into(), args: vec!["--user".into(), "show-environment".into()] };
    matches!(run_and_wait(&check, std::time::Duration::from_secs(5)), Ok(OpenerExit::Exited(Some(0))))
}

/// How long `--setup` waits for the background panel to answer.
pub(crate) const SETUP_WAIT: std::time::Duration = std::time::Duration::from_secs(15);

/// `kora panel --setup`: never asks anything (stdin may be a pipe). Reuses a running panel, or sets up
/// and starts the background one; then opens the browser where there is a screen and prints the link.
pub(crate) fn panel_setup(a: &PanelArgs, home: &std::path::Path) -> i32 {
    use kora_web::launch;
    let ports = launch::SystemPorts;
    let platform = kora_core::service::Platform::current();
    let headless = headless_session();
    let (port, link) = match running_panel(home, &ports, platform) {
        Some(r) => {
            match r {
                Running::Background { .. } => println!("Kora Panel is already running in the background."),
                Running::Window { .. } => {
                    println!("Kora Panel is already running in another window (it stops when that window closes; `kora panel --install-service` keeps one running in the background).")
                }
            }
            let (p, l) = r.port_and_link();
            (p, l.to_string())
        }
        None => {
            if platform == kora_core::service::Platform::Linux && !user_services_available() {
                print_setup_fallback("this session cannot run background services (common after sudo or su, or in a container)");
                return 1;
            }
            let port = match background_port(a, home, &ports) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("error: {e}");
                    return 1;
                }
            };
            println!("Starting Kora Panel in the background (it also starts by itself after a restart) ...");
            let link = match start_background_service(home, port, svc_tailscale(home)) {
                Ok(l) => l,
                Err(e) => {
                    print_setup_fallback(&format!("the background panel could not be set up: {e}"));
                    return 1;
                }
            };
            if !launch::wait_for_panel(port, SETUP_WAIT, std::time::Duration::from_millis(250), &ports) {
                eprintln!("Kora Panel did not answer within {} seconds.", SETUP_WAIT.as_secs());
                if platform == kora_core::service::Platform::Linux {
                    eprintln!("See what happened with:  systemctl --user status kora-panel");
                }
                eprintln!("When it is up, sign in with: {link}");
                return 1;
            }
            (port, link)
        }
    };
    let link = one_time_link(&link, port);
    let opened = (!headless && !a.no_open).then(|| launch::open_browser(&link));
    for line in launch::ready_message(&link, port, headless, opened.as_ref(), &launch::real_env) {
        println!("{line}");
    }
    crate::cmd::panel_tailscale::after_setup(a, home, headless);
    0
}

fn print_setup_fallback(why: &str) {
    eprintln!("Kora Panel could not start in the background: {why}.");
    eprintln!("Start it yourself instead; it runs until you close the window:");
    eprintln!("  kora panel --foreground");
    eprintln!("To keep it running in the background, sign in as the user who should run it (not with sudo) and run:");
    eprintln!("  kora panel --setup");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::panel::PanelCmd;
    use crate::Cli;
    use crate::Command;
    use clap::Parser;

    #[test]
    fn panel_restart_restarts_only_the_background_panel() {
        let cmd = |a: &[&str]| match Cli::try_parse_from(a).map(|c| c.command) {
            Ok(Some(Command::Panel(p))) => Ok(p.cmd),
            Ok(_) => panic!("not the panel command"),
            Err(e) => Err(e.to_string()),
        };
        assert!(matches!(cmd(&["kora", "panel", "restart"]), Ok(Some(PanelCmd::Restart))));
        assert!(cmd(&["kora", "panel", "restart", "now"]).is_err(), "it takes no arguments");
        assert!(matches!(cmd(&["kora", "panel"]), Ok(None)));
        // (background panel set up, a `kora panel` window running)
        assert_eq!(panel_restart_mode(true, false), PanelRestart::Background);
        assert_eq!(panel_restart_mode(true, true), PanelRestart::Background, "the background panel is the one to restart");
        assert_eq!(panel_restart_mode(false, true), PanelRestart::Window, "a window is stopped by hand");
        assert_eq!(panel_restart_mode(false, false), PanelRestart::NotRunning);
    }
}
