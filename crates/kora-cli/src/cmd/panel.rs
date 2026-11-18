//! `kora panel`: the options, and starting the panel in this window or handing over to the background panel.

use crate::cmd::panel_background::{install_background_panel, panel_restart, panel_setup};
use crate::cmd::panel_remote::{panel_remote_actions, panel_user, PanelUserCmd, RemoteActionsCmd};
use crate::cmd::panel_running::{one_time_link, print_running, running_panel};
use crate::ui::ask_yes_no;
use std::io::IsTerminal;

use clap::{Args, Subcommand};

#[derive(Args, Debug)]
pub(crate) struct PanelArgs {
    /// Port to listen on. Default: 47120, or the next free one (up to 47140) when something else uses it.
    #[arg(long)]
    pub(crate) port: Option<u16>,
    /// Address to listen on. The default, 127.0.0.1, is this computer only. Anything else needs remote access (--remote or saved settings) and uses HTTPS once there is a certificate (`kora panel tls`); without one it is plain HTTP: use it only on a private network such as Tailscale or WireGuard.
    #[arg(long, default_value = "127.0.0.1")]
    pub(crate) bind: std::net::IpAddr,
    /// Allow remote access for this run (needed to listen on an address other than 127.0.0.1 unless it is already switched on in the settings).
    #[arg(long)]
    pub(crate) remote: bool,
    /// Open the panel to your own devices over Tailscale: it keeps running in the background, also listens on this computer's Tailscale address, and you sign in with your Tailscale account (no link, no tunnel). Run it once; with --foreground it runs in this window instead.
    #[arg(long, conflicts_with_all = ["remove_service", "print_link", "setup", "new_token"])]
    pub(crate) tailscale: bool,
    /// With --tailscale: the Tailscale account (name@example.com) that becomes the panel's admin. Without it Kora uses the account you are connected from, else the one that added this computer to Tailscale.
    #[arg(long, value_name = "LOGIN", requires = "tailscale")]
    pub(crate) admin: Option<String>,
    /// Also listen on this computer's Tailscale addresses, with the saved settings (the background service uses this; `--tailscale` sets it up).
    #[arg(long, hide = true, conflicts_with = "tailscale")]
    pub(crate) listen_tailscale: bool,
    /// Let --bind be a public address or every interface (0.0.0.0). Needs HTTPS (`kora panel tls`) or a trusted reverse proxy (`kora panel proxy trust`); --tailscale or an SSH tunnel is safer.
    #[arg(long)]
    pub(crate) allow_public: bool,
    /// Make a new sign-in token, so links shown before stop working (restarts the background panel).
    #[arg(long, conflicts_with_all = ["install_service", "remove_service", "print_link", "setup", "foreground"])]
    pub(crate) new_token: bool,
    /// Do not open a browser; print the address and sign-in link instead (automatic over SSH or without a desktop).
    #[arg(long)]
    pub(crate) no_open: bool,
    /// Run the panel in this window and stop it when the window closes: do not offer or use the background panel.
    #[arg(long, conflicts_with_all = ["install_service", "remove_service", "print_link", "setup"])]
    pub(crate) foreground: bool,
    /// Make sure the background panel is set up and running, then open it in your browser and print the sign-in link (the install scripts run this). Never reads standard input; on a server with Tailscale it asks once on the terminal whether to use it (KORA_TAILSCALE=1 or 0 answers in advance).
    #[arg(long, conflicts_with_all = ["install_service", "remove_service", "print_link"])]
    pub(crate) setup: bool,
    /// Keep the panel running in the background so monitoring and alerts work while it is closed (starts at sign-in; no administrator rights needed).
    #[arg(long, conflicts_with_all = ["remove_service", "print_link"])]
    pub(crate) install_service: bool,
    /// Stop and remove the background panel installed with --install-service.
    #[arg(long, conflicts_with_all = ["install_service", "print_link"])]
    pub(crate) remove_service: bool,
    /// Print the sign-in link of the background panel.
    #[arg(long, conflicts_with_all = ["install_service", "remove_service"])]
    pub(crate) print_link: bool,
    #[command(subcommand)]
    pub(crate) cmd: Option<PanelCmd>,
}

#[derive(Subcommand, Debug)]
pub(crate) enum PanelCmd {
    /// Manage the people who may sign in to the panel remotely (each has a token and a role).
    User {
        #[command(subcommand)]
        cmd: PanelUserCmd,
    },
    /// Manage who may sign in with their Tailscale account (set it up with `kora panel --tailscale`).
    Tailscale {
        #[command(subcommand)]
        cmd: crate::cmd::panel_tailscale::TailscaleCmd,
    },
    /// HTTPS for the panel: use your own certificate, Tailscale's, or one made on this computer.
    Tls {
        #[command(subcommand)]
        cmd: crate::cmd::panel_tls::TlsCmd,
    },
    /// Trust a reverse proxy (Caddy, nginx) in front of the panel, for a public name with a real certificate.
    Proxy {
        #[command(subcommand)]
        cmd: crate::cmd::panel_tls::ProxyCmd,
    },
    /// Restart the background panel (the one that keeps running after you close the window). The sign-in link stays the same; open pages sign in again.
    Restart,
    /// Let people who sign in from another computer with a token do the risky things too: delete and unregister servers, uninstall Kora, send console commands, change schedules and update Kora. Off by default. (Token administrators can already install servers; Tailscale administrators can do all of this without the switch.)
    RemoteActions {
        #[command(subcommand)]
        cmd: RemoteActionsCmd,
    },
    /// Remote sign-in settings that are not about one person.
    Remote {
        #[command(subcommand)]
        cmd: crate::cmd::panel_remote::RemoteCmd,
    },
}

/// Printed on Linux when `kora panel` is run as root: many rented servers only have the root account.
/// `adduser` and the `sudo` group are right for Debian and Ubuntu, the systems Kora supports. Enabling
/// linger starts the new user's service manager, so the background panel can be set up after `su`.
pub(crate) const ROOT_ONLY_HELP: &[&str] = &[
    "",
    "If this server only has the root account, create a user for Kora first (it asks you to choose a password):",
    "  adduser kora",
    "  usermod -aG sudo kora",
    "  loginctl enable-linger kora",
    "  su - kora",
    "Then, as that user, run:",
    "  kora panel --setup",
];

pub(crate) fn panel(a: &PanelArgs) -> i32 {
    panel_with(a, None)
}

/// [`panel`], opening the browser on `landing` (`/install`) when it runs in this window.
pub(crate) fn panel_with(a: &PanelArgs, landing: Option<&str>) -> i32 {
    use kora_core::monitor::panel_service as svc;
    // SECURITY.md: the panel runs as the installing user, never as administrator/root. A root panel
    // would keep its token and state in root's home and run every action with full rights.
    if kora_core::elevate::is_elevated() && std::env::var_os("KORA_ALLOW_ROOT_PANEL").is_none_or(|v| v != "1") {
        eprintln!("error: Kora Panel does not run as administrator/root: it keeps your servers and your sign-in safe. Run `kora` from your normal account, without sudo. It asks for administrator rights only when an action needs them.");
        if cfg!(target_os = "linux") {
            for line in ROOT_ONLY_HELP {
                eprintln!("{line}");
            }
        }
        return 1;
    }
    let home = svc::home();
    match &a.cmd {
        Some(PanelCmd::User { cmd }) => return panel_user(cmd),
        Some(PanelCmd::Tailscale { cmd }) => return crate::cmd::panel_tailscale::command(cmd, &home),
        Some(PanelCmd::Tls { cmd }) => return crate::cmd::panel_tls::tls(cmd),
        Some(PanelCmd::Proxy { cmd }) => return crate::cmd::panel_tls::proxy(cmd),
        Some(PanelCmd::Restart) => return panel_restart(&home),
        Some(PanelCmd::RemoteActions { cmd }) => return panel_remote_actions(*cmd, &kora_web::remote::config::default_path()),
        Some(PanelCmd::Remote { cmd }) => return crate::cmd::panel_remote::panel_remote(cmd, &kora_web::remote::config::default_path()),
        None => {}
    }
    if a.new_token {
        return crate::cmd::panel_tailscale::new_token(&home);
    }
    if a.tailscale {
        // Set up who may sign in, then keep it running in the background (or here with --foreground).
        let ts = match crate::cmd::panel_tailscale::configure(a.admin.as_deref()) {
            Ok(ts) => ts,
            Err(e) => {
                eprintln!("error: {e}");
                return 1;
            }
        };
        if !a.foreground {
            return crate::cmd::panel_tailscale::start_background(a, &home, &ts);
        }
    }
    if a.print_link {
        return match svc::link(&home) {
            Ok(link) => {
                let port = svc::saved_port(&home).unwrap_or(kora_web::DEFAULT_PORT);
                // A panel that is still starting gets a moment, so the link can carry a one-time code.
                let up = kora_web::launch::wait_for_panel(port, std::time::Duration::from_secs(5), std::time::Duration::from_millis(250), &kora_web::launch::SystemPorts);
                println!("{}", one_time_link(&link, port));
                if !up {
                    eprintln!("Note: nothing is answering on port {port} right now. The background panel may still be starting; check it with `systemctl --user status kora-panel` (Linux).");
                } else {
                    eprintln!("The link works once, within 10 minutes. Run this again for a new one.");
                }
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        };
    }
    if a.setup {
        return panel_setup(a, &home);
    }
    let ports = kora_web::launch::SystemPorts;
    let platform = kora_core::service::Platform::current();
    if !a.install_service && !a.remove_service && !a.foreground && may_reuse_running(a) {
        let running = running_panel(&home, &ports, platform);
        let tty = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
        match panel_start_mode(headless_session(), tty, running.is_some()) {
            PanelStart::ShowRunning => {
                if let Some(r) = running {
                    print_running(&r);
                    return 0;
                }
            }
            PanelStart::OfferBackground => {
                if ask_yes_no("This computer has no screen, so Kora Panel would stop when you close this window.\nKeep it running in the background instead? [Y/n] ") {
                    return install_background_panel(a, &home);
                }
            }
            PanelStart::Foreground => {}
        }
    }
    if a.install_service || a.remove_service {
        let runner = kora_core::service::SystemRunner;
        if a.remove_service {
            return match svc::remove(&home, &runner, platform) {
                Ok(()) => {
                    println!("The background panel has been stopped and removed. Monitoring and alerts now only run while `kora panel` is open.");
                    0
                }
                Err(e) => {
                    eprintln!("error: could not remove the background panel: {e}");
                    1
                }
            };
        }
        return install_background_panel(a, &home);
    }
    // Only on this computer can we tell which ports are free; another address keeps the asked-for port.
    let port = match (a.port, a.bind.is_loopback()) {
        (Some(p), _) => p,
        (None, false) => kora_web::DEFAULT_PORT,
        (None, true) => match kora_web::launch::pick_port(kora_web::DEFAULT_PORT, false, &ports) {
            Some(p) => {
                if p != kora_web::DEFAULT_PORT {
                    println!("Port {} is used by another program, so Kora Panel uses port {p}.", kora_web::DEFAULT_PORT);
                }
                p
            }
            None => {
                eprintln!("error: ports {} to {} are all in use. Choose one with --port.", kora_web::DEFAULT_PORT, kora_web::launch::FALLBACK_PORTS.end());
                return 1;
            }
        },
    };
    // A saved token (made by --install-service) keeps the sign-in link the same every time.
    let token = svc::load_token(&home).unwrap_or_else(kora_web::security::random_hex);
    if !a.bind.is_loopback() {
        println!(
            "This panel runs in this window only and stops when you close it.{}",
            if a.tailscale { "" } else { " With Tailscale, `kora panel --tailscale` keeps one running in the background instead." }
        );
    }
    let opts = kora_web::PanelOptions {
        bind: a.bind,
        port,
        open_browser: !a.no_open,
        remote: a.remote,
        token: Some(token.clone()),
        tailscale: a.tailscale || a.listen_tailscale,
        allow_public: a.allow_public,
        landing: landing.map(str::to_string),
        ..Default::default()
    };
    let pid = std::process::id();
    let pid_file = svc::pid_path(&home);
    let _ = std::fs::create_dir_all(&home);
    let _ = std::fs::write(&pid_file, pid.to_string());
    // So a second `kora panel` shows this one's link instead of failing to listen on the same port.
    if a.bind.is_loopback() {
        let _ = svc::record_running(&home, &svc::RunningPanel { port, pid, token });
    }
    let result = kora_web::run(opts);
    let _ = std::fs::remove_file(&pid_file);
    svc::clear_running(&home, pid);
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// May `kora panel` answer with the link of a panel that already runs? Not when it was asked for a
/// different address (`--bind`, `--remote`, Tailscale): that is a different panel, and answering with the
/// 127.0.0.1 link silently ignored those options.
fn may_reuse_running(a: &PanelArgs) -> bool {
    a.bind.is_loopback() && !a.remote && !a.tailscale && !a.listen_tailscale
}

/// What `kora panel` does when started without options.
#[derive(Debug, PartialEq, Eq)]
enum PanelStart {
    /// A panel is already up: show its link instead of starting a second one.
    ShowRunning,
    /// A server with no screen and nobody to keep the window open: offer the background panel.
    OfferBackground,
    /// Run in this window.
    Foreground,
}

fn panel_start_mode(headless: bool, interactive: bool, already_running: bool) -> PanelStart {
    if already_running {
        PanelStart::ShowRunning
    } else if headless && interactive {
        PanelStart::OfferBackground
    } else {
        PanelStart::Foreground
    }
}

/// Running as root (Linux)?
pub(crate) fn launch_uid_is_root() -> bool {
    kora_web::launch::current_uid() == Some(0)
}

/// No desktop to open a browser on: an SSH session, or no display on Linux.
pub(crate) fn headless_session() -> bool {
    kora_web::launch::is_headless()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Cli;
    use crate::Command;
    use clap::Parser;

    #[test]
    fn the_panel_offers_the_background_service_only_where_nobody_keeps_a_window_open() {
        // (headless, interactive, background panel already running)
        assert_eq!(panel_start_mode(true, true, true), PanelStart::ShowRunning, "never start a second one");
        assert_eq!(panel_start_mode(false, true, true), PanelStart::ShowRunning);
        assert_eq!(panel_start_mode(true, true, false), PanelStart::OfferBackground, "a server with no screen");
        assert_eq!(panel_start_mode(true, false, false), PanelStart::Foreground, "scripts and pipes are never asked");
        assert_eq!(panel_start_mode(false, true, false), PanelStart::Foreground, "a desktop opens the browser and runs here");
        assert!(Cli::try_parse_from(["kora", "panel", "--foreground"]).is_ok());
        assert!(Cli::try_parse_from(["kora", "panel", "--foreground", "--install-service"]).is_err());
    }

    #[test]
    fn setup_is_a_plain_flag_and_the_port_is_only_set_when_asked() {
        let panel = |a: &[&str]| {
            let mut v = vec!["kora", "panel"];
            v.extend_from_slice(a);
            match Cli::try_parse_from(v).map(|c| c.command) {
                Ok(Some(Command::Panel(p))) => Ok(p),
                Ok(_) => unreachable!(),
                Err(e) => Err(e),
            }
        };
        let p = panel(&["--setup"]).unwrap();
        assert!(p.setup && p.port.is_none(), "no --port means: pick a free one");
        assert_eq!(panel(&["--setup", "--port", "47200"]).unwrap().port, Some(47200));
        assert!(panel(&["--setup", "--no-open"]).is_ok());
        // A panel for another address is never answered with the link of the one on 127.0.0.1.
        assert!(may_reuse_running(&panel(&[]).unwrap()));
        assert!(!may_reuse_running(&panel(&["--bind", "100.64.0.5", "--remote"]).unwrap()));
        assert!(!may_reuse_running(&panel(&["--remote"]).unwrap()));
        assert!(!may_reuse_running(&panel(&["--tailscale"]).unwrap()));
        assert!(!may_reuse_running(&panel(&["--listen-tailscale"]).unwrap()));
        assert!(panel(&["--admin", "a@b"]).is_err(), "--admin goes with --tailscale");
        assert!(panel(&["--tailscale", "--admin", "a@b"]).is_ok());
        assert!(panel(&["--tailscale", "--foreground"]).is_ok());
        assert!(panel(&["--tailscale", "--listen-tailscale"]).is_err());
        assert!(panel(&["--tailscale", "--setup"]).is_err());
        for clash in ["--foreground", "--install-service", "--remove-service", "--print-link"] {
            assert!(panel(&["--setup", clash]).is_err(), "{clash}");
        }
    }

    #[test]
    fn root_only_help_gives_the_commands_to_continue() {
        let text = ROOT_ONLY_HELP.join("\n");
        for want in ["adduser kora", "usermod -aG sudo kora", "loginctl enable-linger kora", "su - kora", "kora panel --setup"] {
            assert!(text.contains(want), "{want}");
        }
    }
}
