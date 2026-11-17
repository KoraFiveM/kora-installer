//! Bare `kora`, `kora manager` and a flagless `kora install`: make sure the panel is running and open it.
//!
//! The web panel is the only interface. This never asks for administrator rights. Whatever the situation,
//! the person ends up with the panel open, or with a plain sentence saying what to do next:
//!
//! * the panel already runs: open it;
//! * it does not: start it in the background (so monitoring and alerts keep working), then open it; where
//!   no background service is possible, run it in this window;
//! * no screen (SSH): print the sign-in link, the `ssh -L` tunnel command and the Tailscale option;
//! * started from an administrator/root prompt: open a running panel, run Kora again as the normal user
//!   (Linux `sudo`), or explain (the panel never runs with administrator rights).

use std::ffi::OsString;

use kora_core::elevate;
use kora_core::monitor::panel_service as svc;
use kora_web::launch::{self, PortCheck, SystemPorts};

use super::{
    background_port_for, headless_session, one_time_link, panel_tailscale, panel_with, running_panel, start_background_service, svc_tailscale,
    user_services_available, Running, ROOT_ONLY_HELP, SETUP_WAIT,
};

/// Which page of the panel opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Landing {
    /// The start page (the server list; with no servers yet, the install wizard).
    Home,
    /// The install wizard (`kora install` with no options).
    Install,
}

impl Landing {
    pub fn page(self) -> &'static str {
        match self {
            Landing::Home => "/",
            Landing::Install => "/install",
        }
    }

    /// The words that start this again as another user (`kora`, `kora install`).
    fn rerun_args(self) -> Vec<OsString> {
        match self {
            Landing::Home => vec![],
            Landing::Install => vec!["install".into()],
        }
    }
}

/// What the decision depends on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Facts {
    /// Running with administrator/root rights (and `KORA_ALLOW_ROOT_PANEL` is not set).
    pub elevated: bool,
    /// A panel of ours already answers.
    pub running: bool,
    /// Linux under `sudo`: the normal account behind it.
    pub sudo_user: Option<String>,
    /// A background service can be set up here (systemd user services work, or Windows).
    pub background_possible: bool,
    /// Windows: the Startup script of the background panel exists, so Explorer can start it as the normal user.
    pub startup_script: bool,
    pub windows: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Plan {
    /// Open the panel that runs.
    OpenRunning,
    /// Set up and start the background panel, then open it.
    StartBackground,
    /// No background service here: run the panel in this window.
    RunHere,
    /// Linux `sudo`: run Kora again as this normal user.
    RunAsUser(String),
    /// Windows, from an administrator window: have Explorer start the panel as the normal user.
    StartAsNormalUser,
    /// Administrator/root, nothing to open and no way to hand over: say what to do.
    ExplainElevated,
}

pub fn decide(f: &Facts) -> Plan {
    if f.elevated {
        if let Some(user) = &f.sudo_user {
            return Plan::RunAsUser(user.clone());
        }
        return if f.running {
            Plan::OpenRunning
        } else if f.windows && f.startup_script {
            Plan::StartAsNormalUser
        } else {
            Plan::ExplainElevated
        };
    }
    if f.running {
        Plan::OpenRunning
    } else if f.background_possible {
        Plan::StartBackground
    } else {
        Plan::RunHere
    }
}

/// The sentences for [`Plan::ExplainElevated`].
pub fn elevated_explanation(windows: bool) -> Vec<String> {
    if windows {
        vec![
            "This window is running as administrator, and Kora Panel does not run that way: it keeps your servers and your sign-in safe.".into(),
            "Close this window and open Kora from the Start menu or the desktop shortcut, or run `kora` in a normal PowerShell window.".into(),
            "Nothing was changed.".into(),
        ]
    } else {
        let mut v = vec![
            "Kora Panel does not run as administrator/root: it keeps your servers and your sign-in safe. Run `kora` from your normal account, without sudo.".to_string(),
            "It asks for administrator rights only when an action needs them.".to_string(),
        ];
        v.extend(ROOT_ONLY_HELP.iter().map(|l| (*l).to_string()));
        v
    }
}

/// `runuser` arguments that run `exe args` as the normal user `name` (Linux, from `sudo`). The user's own
/// home and runtime folder are set, so their background panel and settings are the ones used; the desktop
/// variables are kept so the browser can open on their screen.
pub fn runuser_command(name: &str, uid: u32, home: &std::path::Path, exe: &std::path::Path, args: &[OsString], env: &dyn Fn(&str) -> Option<String>) -> Vec<OsString> {
    let mut v: Vec<OsString> = vec!["-u".into(), name.into(), "--".into(), "env".into()];
    let mut set = |k: &str, val: String| v.push(format!("{k}={val}").into());
    set("HOME", home.display().to_string());
    set("USER", name.to_string());
    set("LOGNAME", name.to_string());
    set("XDG_RUNTIME_DIR", format!("/run/user/{uid}"));
    set("DBUS_SESSION_BUS_ADDRESS", format!("unix:path=/run/user/{uid}/bus"));
    for k in ["DISPLAY", "WAYLAND_DISPLAY", "XAUTHORITY", "XDG_CURRENT_DESKTOP", "XDG_SESSION_TYPE", "KORA_HOME", "SSH_CONNECTION", "NO_COLOR", "TERM"] {
        if let Some(val) = env(k) {
            set(k, val);
        }
    }
    v.push(exe.as_os_str().to_os_string());
    v.extend(args.iter().cloned());
    v
}

/// What gets printed once the panel is up (pure, so the wording is tested).
pub fn ready_lines(link: &str, port: u16, headless: bool, opened: Option<&Result<(), String>>, env: &dyn Fn(&str) -> Option<String>, tailscale_here: bool) -> Vec<String> {
    if let Some(Ok(())) = opened {
        return vec!["Kora Panel is open in your browser.".to_string()];
    }
    let mut out = launch::ready_message(link, port, headless, opened, env);
    if headless {
        out.push(String::new());
        if tailscale_here {
            out.push("Or skip the tunnel: Tailscale is running here, and `kora panel --tailscale` lets you open the panel from your own devices.".into());
        } else {
            out.push("Or skip the tunnel with Tailscale: install it here and on your own devices, then run `kora panel --tailscale`.".into());
        }
    }
    out
}

/// Where the panel is, once it is up: print and open.
fn present(landing: Landing, port: u16, link: &str, no_open: bool) {
    let headless = headless_session();
    let link = launch::with_landing(&one_time_link(link, port), landing.page());
    let opened = (!headless && !no_open).then(|| launch::open_browser(&link));
    let tailscale_here = headless && panel_tailscale::tailscale_running();
    for line in ready_lines(&link, port, headless, opened.as_ref(), &launch::real_env, tailscale_here) {
        println!("{line}");
    }
    if headless {
        panel_tailscale::print_tailnet_address(&svc::home(), port);
    }
}

/// The real-world facts.
fn facts(home: &std::path::Path, ports: &dyn PortCheck, platform: kora_core::service::Platform) -> (Facts, Option<Running>) {
    let windows = platform == kora_core::service::Platform::Windows;
    let running = running_panel(home, ports, platform);
    let elevated = elevate::is_elevated() && std::env::var_os("KORA_ALLOW_ROOT_PANEL").is_none_or(|v| v != "1");
    let sudo_user = if cfg!(unix) { kora_core::userhome::sudo_account().map(|a| a.name) } else { None };
    // Only worth asking where it can matter (it can take a moment).
    let background_possible = !elevated && running.is_none() && (windows || user_services_available());
    let f = Facts { elevated, running: running.is_some(), sudo_user, background_possible, startup_script: windows && svc::startup_script_path().is_file(), windows };
    (f, running)
}

/// Open the panel (see the module documentation). Returns the exit code.
pub fn open_panel(landing: Landing, no_open: bool) -> i32 {
    let home = svc::home();
    let ports = SystemPorts;
    let platform = kora_core::service::Platform::current();
    let (f, running) = facts(&home, &ports, platform);
    match decide(&f) {
        Plan::OpenRunning => match running {
            Some(r) => {
                let (port, link) = r.port_and_link();
                present(landing, port, link, no_open);
                0
            }
            None => 1,
        },
        Plan::StartBackground => match start_and_wait(&home, &ports) {
            Ok((port, link)) => {
                present(landing, port, &link, no_open);
                0
            }
            Err(why) => {
                // No dead end: the panel still opens, in this window.
                println!("Kora could not set up the background panel ({why}).");
                println!("Running Kora Panel in this window instead: close the window to stop it.");
                panel_with(&foreground_args(no_open), Some(landing.page()))
            }
        },
        Plan::RunHere => {
            println!("Running Kora Panel in this window: close the window to stop it.");
            panel_with(&foreground_args(no_open), Some(landing.page()))
        }
        Plan::RunAsUser(name) => run_as_user(&name, landing),
        Plan::StartAsNormalUser => start_as_normal_user(landing, no_open, &home, &ports),
        Plan::ExplainElevated => {
            for l in elevated_explanation(f.windows) {
                eprintln!("{l}");
            }
            1
        }
    }
}

fn foreground_args(no_open: bool) -> super::PanelArgs {
    let mut argv = vec!["kora", "panel", "--foreground"];
    if no_open {
        argv.push("--no-open");
    }
    match <super::Cli as clap::Parser>::try_parse_from(argv).map(|c| c.command) {
        Ok(Some(super::Command::Panel(a))) => a,
        _ => unreachable!("`kora panel --foreground` always parses"),
    }
}

/// Set up and start the background panel, wait for it, return its port and sign-in link.
fn start_and_wait(home: &std::path::Path, ports: &dyn PortCheck) -> Result<(u16, String), String> {
    let port = background_port_for(None, home, ports)?;
    println!("Starting Kora Panel in the background (it also starts by itself when you sign in) ...");
    let link = start_background_service(home, port, svc_tailscale(home))?;
    if !launch::wait_for_panel(port, SETUP_WAIT, std::time::Duration::from_millis(250), ports) {
        return Err(format!("it did not answer within {} seconds", SETUP_WAIT.as_secs()));
    }
    Ok((port, link))
}

/// Linux `sudo kora`: do it as the person who typed `sudo`.
fn run_as_user(name: &str, landing: Landing) -> i32 {
    let (Some(acct), Ok(exe)) = (kora_core::userhome::sudo_account(), std::env::current_exe()) else {
        for l in elevated_explanation(false) {
            eprintln!("{l}");
        }
        return 1;
    };
    println!("Kora Panel does not run as administrator, so Kora is opening it as {name} instead.");
    let args = runuser_command(name, acct.uid, &acct.home, &exe, &landing.rerun_args(), &launch::real_env);
    match std::process::Command::new("runuser").args(args).status() {
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => {
            eprintln!("Could not switch to {name} ({e}). Run `kora` from your normal account, without sudo.");
            1
        }
    }
}

/// Windows, from an administrator window: Explorer starts the Startup script with the normal user's rights.
fn start_as_normal_user(landing: Landing, no_open: bool, home: &std::path::Path, ports: &dyn PortCheck) -> i32 {
    let port = svc::saved_port(home).unwrap_or(kora_web::DEFAULT_PORT);
    println!("Kora Panel does not run as administrator, so Kora is starting it as your normal user.");
    let started = std::process::Command::new("explorer.exe").arg(svc::startup_script_path()).spawn().is_ok();
    if started && launch::wait_for_panel(port, SETUP_WAIT, std::time::Duration::from_millis(250), ports) {
        if let Ok(link) = svc::link(home) {
            present(landing, port, &link, no_open);
            return 0;
        }
    }
    for l in elevated_explanation(true) {
        eprintln!("{l}");
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Facts {
        Facts { elevated: false, running: false, sudo_user: None, background_possible: true, startup_script: false, windows: false }
    }

    #[test]
    fn a_normal_user_gets_the_panel_running_or_started_and_never_an_elevation() {
        assert_eq!(decide(&Facts { running: true, ..base() }), Plan::OpenRunning);
        assert_eq!(decide(&base()), Plan::StartBackground);
        assert_eq!(decide(&Facts { background_possible: false, ..base() }), Plan::RunHere, "no dead end where there are no background services");
        assert_eq!(decide(&Facts { running: true, background_possible: false, ..base() }), Plan::OpenRunning);
    }

    #[test]
    fn sudo_hands_over_to_the_person_who_typed_it() {
        let sudo = Facts { elevated: true, sudo_user: Some("sam".into()), ..base() };
        assert_eq!(decide(&sudo), Plan::RunAsUser("sam".into()));
        assert_eq!(decide(&Facts { running: true, ..sudo }), Plan::RunAsUser("sam".into()), "root's view of the panel is not theirs");
    }

    #[test]
    fn an_administrator_window_opens_a_running_panel_or_explains() {
        let admin = Facts { elevated: true, ..base() };
        assert_eq!(decide(&Facts { running: true, ..admin.clone() }), Plan::OpenRunning, "opening a page needs no rights");
        assert_eq!(decide(&admin), Plan::ExplainElevated, "plain root on Linux: say what to do");
        let win = Facts { windows: true, ..admin };
        assert_eq!(decide(&win), Plan::ExplainElevated);
        assert_eq!(decide(&Facts { startup_script: true, ..win }), Plan::StartAsNormalUser, "Explorer starts it with the normal user's rights");
    }

    #[test]
    fn an_elevated_session_never_starts_the_panel_itself() {
        for running in [false, true] {
            for sudo in [None, Some("sam".to_string())] {
                for windows in [false, true] {
                    for script in [false, true] {
                        let p = decide(&Facts { elevated: true, running, sudo_user: sudo.clone(), background_possible: true, startup_script: script, windows });
                        assert!(!matches!(p, Plan::StartBackground | Plan::RunHere), "{p:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_explanations_say_what_to_do_next() {
        let win = elevated_explanation(true).join(" ");
        assert!(win.contains("Start menu") && win.contains("normal PowerShell") && win.contains("Nothing was changed"), "{win}");
        let lin = elevated_explanation(false).join("\n");
        assert!(lin.contains("without sudo") && lin.contains("adduser kora"), "{lin}");
    }

    #[test]
    fn landing_pages() {
        assert_eq!((Landing::Home.page(), Landing::Install.page()), ("/", "/install"));
        assert!(Landing::Home.rerun_args().is_empty());
        assert_eq!(Landing::Install.rerun_args(), [OsString::from("install")]);
    }

    #[test]
    fn runuser_runs_kora_as_that_user_with_their_home_and_screen() {
        let env = |k: &str| match k {
            "DISPLAY" => Some(":0".to_string()),
            "KORA_HOME" => Some("/srv/kora-home".to_string()),
            "HOME" => Some("/root".to_string()),
            "SECRET" => Some("no".to_string()),
            _ => None,
        };
        let a: Vec<String> = runuser_command("sam", 1000, std::path::Path::new("/home/sam"), std::path::Path::new("/usr/local/bin/kora"), &[OsString::from("install")], &env)
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        assert_eq!(a[..4], ["-u", "sam", "--", "env"]);
        for want in ["HOME=/home/sam", "USER=sam", "XDG_RUNTIME_DIR=/run/user/1000", "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus", "DISPLAY=:0", "KORA_HOME=/srv/kora-home"] {
            assert!(a.iter().any(|x| x == want), "missing {want}: {a:?}");
        }
        assert!(!a.iter().any(|x| x == "HOME=/root" || x.starts_with("SECRET")), "root's HOME and unknown variables are not passed on: {a:?}");
        assert_eq!(a[a.len() - 2..], ["/usr/local/bin/kora", "install"], "the program, then its words, last");
    }

    #[test]
    fn once_open_the_message_is_short_and_without_a_screen_it_has_the_tunnel_and_tailscale() {
        let env = |_: &str| None::<String>;
        let link = "http://127.0.0.1:47120/#code=abcdefghijklmnop";
        let ok: Result<(), String> = Ok(());
        assert_eq!(ready_lines(link, 47120, false, Some(&ok), &env, false), ["Kora Panel is open in your browser."]);
        let failed: Result<(), String> = Err("no browser".into());
        let text = ready_lines(link, 47120, false, Some(&failed), &env, false).join("\n");
        assert!(text.contains("Could not open a browser") && text.contains(link), "{text}");
        let ssh = |k: &str| (k == "SSH_CONNECTION").then(|| "10.0.0.5 5000 10.0.0.9 22".to_string());
        let text = ready_lines(link, 47120, true, None, &ssh, false).join("\n");
        assert!(text.contains(link) && text.contains("ssh -L 47120:127.0.0.1:47120"), "{text}");
        assert!(text.contains("install it here") && text.contains("kora panel --tailscale"), "{text}");
        let text = ready_lines(link, 47120, true, None, &ssh, true).join("\n");
        assert!(text.contains("Tailscale is running here") && text.contains("kora panel --tailscale"), "{text}");
    }
}
