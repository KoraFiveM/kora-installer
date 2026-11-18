//! Finding the panel that is already running (background service or window) and the port a background panel should use.

use crate::cmd::panel::{headless_session, PanelArgs};

/// A Kora Panel of ours that is already answering.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Running {
    /// The background service (`--install-service` / `--setup`).
    Background { port: u16, link: String },
    /// A `kora panel` running in another window.
    Window { port: u16, link: String },
}

impl Running {
    pub(crate) fn port_and_link(&self) -> (u16, &str) {
        match self {
            Running::Background { port, link } | Running::Window { port, link } => (*port, link),
        }
    }
}

/// Find a running panel: the background service on its saved port, else the one recorded by a
/// `kora panel` window (whose process must still exist).
pub(crate) fn running_panel(home: &std::path::Path, ports: &dyn kora_web::launch::PortCheck, platform: kora_core::service::Platform) -> Option<Running> {
    use kora_core::monitor::panel_service as svc;
    find_running(svc::service_installed(platform).then(|| svc::link(home).ok().zip(svc::saved_port(home))).flatten(), svc::load_running(home).filter(|r| process_alive(r.pid)), ports)
}

/// The decision behind [`running_panel`], without the disk: `service` is the installed service's
/// (link, port), `window` the recorded `kora panel` window.
fn find_running(service: Option<(String, u16)>, window: Option<kora_core::monitor::panel_service::RunningPanel>, ports: &dyn kora_web::launch::PortCheck) -> Option<Running> {
    if let Some((link, port)) = service {
        if ports.is_kora_panel(port) {
            return Some(Running::Background { port, link });
        }
    }
    let w = window?;
    ports.is_kora_panel(w.port).then(|| Running::Window { port: w.port, link: w.link() })
}

fn process_alive(pid: u32) -> bool {
    if cfg!(target_os = "linux") {
        std::path::Path::new("/proc").join(pid.to_string()).exists()
    } else {
        true
    }
}

/// Swap the token in a `#token=` link for a one-time code from the panel on `port`, so the long-lived
/// token does not end up in a browser's history. Unchanged when the panel cannot give one (an older
/// version, or not answering).
pub(crate) fn one_time_link(link: &str, port: u16) -> String {
    match link.split_once("#token=") {
        Some((_, token)) => kora_web::launch::fresh_link(port, token),
        None => link.to_string(),
    }
}

pub(crate) fn print_running(r: &Running) {
    let (port, link) = r.port_and_link();
    let link = &one_time_link(link, port);
    match r {
        Running::Background { .. } => println!("Kora Panel is already running in the background."),
        Running::Window { .. } => println!("Kora Panel is already running in another window."),
    }
    println!("Sign in with: {link}");
    if headless_session() {
        println!("On a server with no screen, open a tunnel from your own computer first:  {}", kora_web::launch::ssh_hint(port, &kora_web::launch::real_env));
    }
    if matches!(r, Running::Background { .. }) {
        crate::cmd::panel_tailscale::print_tailnet_address(&kora_core::monitor::panel_service::home(), port);
    }
    match r {
        Running::Background { .. } => println!("(`kora panel --remove-service` stops and removes it; `kora panel --foreground` runs a separate one in this window.)"),
        Running::Window { .. } => println!("(It stops when that window closes. `kora panel --install-service` keeps one running in the background instead.)"),
    }
}

/// The port for a new background panel: the asked-for one, else the saved one, else the default; when
/// not asked for explicitly and it is taken, the next free one.
pub(crate) fn background_port(a: &PanelArgs, home: &std::path::Path, ports: &dyn kora_web::launch::PortCheck) -> Result<u16, String> {
    background_port_for(a.port, home, ports)
}

/// [`background_port`] with the asked-for port given directly.
pub(crate) fn background_port_for(requested: Option<u16>, home: &std::path::Path, ports: &dyn kora_web::launch::PortCheck) -> Result<u16, String> {
    use kora_core::monitor::panel_service as svc;
    let preferred = requested.or_else(|| svc::saved_port(home)).unwrap_or(kora_web::DEFAULT_PORT);
    // Our own background panel on its saved port is about to be restarted, not "another program":
    // keep the port, so links, bookmarks, tunnels and firewall rules stay right.
    if svc::saved_port(home) == Some(preferred) && ports.is_kora_panel(preferred) {
        return Ok(preferred);
    }
    match kora_web::launch::pick_port(preferred, requested.is_some(), ports) {
        Some(p) => {
            if p != preferred {
                println!("Port {preferred} is used by another program, so Kora Panel uses port {p}.");
            }
            Ok(p)
        }
        None if requested.is_some() => Err(format!("port {preferred} is already in use. Choose another with --port.")),
        None => Err(format!("ports {} to {} are all in use. Choose one with --port.", kora_web::DEFAULT_PORT, kora_web::launch::FALLBACK_PORTS.end())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Ports(Vec<u16>);
    impl kora_web::launch::PortCheck for Ports {
        fn is_free(&self, p: u16) -> bool {
            !self.0.contains(&p)
        }
        fn is_kora_panel(&self, p: u16) -> bool {
            self.0.contains(&p)
        }
    }

    #[test]
    fn a_running_panel_is_found_whether_in_the_background_or_in_a_window() {
        use kora_core::monitor::panel_service::RunningPanel;
        let window = RunningPanel { port: 47121, pid: 9, token: "cd".into() };
        let service = Some(("http://127.0.0.1:47120/#token=ab".to_string(), 47120));
        assert_eq!(
            find_running(service.clone(), Some(window.clone()), &Ports(vec![47120, 47121])),
            Some(Running::Background { port: 47120, link: "http://127.0.0.1:47120/#token=ab".into() }),
            "the background service wins"
        );
        assert_eq!(
            find_running(service.clone(), Some(window.clone()), &Ports(vec![47121])),
            Some(Running::Window { port: 47121, link: "http://127.0.0.1:47121/#token=cd".into() }),
            "a service that is not answering is not running"
        );
        assert_eq!(find_running(service, Some(window), &Ports(vec![])), None, "stale records are ignored");
        assert_eq!(find_running(None, None, &Ports(vec![47120])), None, "someone else's panel has no link we know");
    }

    #[test]
    fn the_background_port_prefers_the_asked_then_saved_then_default_port() {
        let home = tempfile::tempdir().unwrap();
        let args = |port: Option<u16>| PanelArgs {
            port,
            bind: "127.0.0.1".parse().unwrap(),
            remote: false,
            tailscale: false,
            admin: None,
            listen_tailscale: false,
            allow_public: false,
            new_token: false,
            no_open: true,
            foreground: false,
            background: false,
            setup: true,
            install_service: false,
            remove_service: false,
            print_link: false,
            cmd: None,
        };
        assert_eq!(background_port(&args(None), home.path(), &Ports(vec![])), Ok(47120));
        assert_eq!(background_port(&args(None), home.path(), &Ports(vec![47120, 47121])), Ok(47122));
        assert!(background_port(&args(Some(47120)), home.path(), &Ports(vec![47120])).unwrap_err().contains("47120 is already in use"));
        std::fs::write(kora_core::monitor::panel_service::info_path(home.path()), r#"{"port":47130}"#).unwrap();
        assert_eq!(background_port(&args(None), home.path(), &Ports(vec![])), Ok(47130), "the saved port is kept");
        // `Ports` says 47130 is busy AND a Kora panel: our own panel, restarted on the same port.
        assert_eq!(background_port(&args(None), home.path(), &Ports(vec![47130])), Ok(47130), "our own running panel keeps its port");
        struct Other;
        impl kora_web::launch::PortCheck for Other {
            fn is_free(&self, p: u16) -> bool {
                p != 47130
            }
            fn is_kora_panel(&self, _: u16) -> bool {
                false
            }
        }
        assert_eq!(background_port(&args(None), home.path(), &Other), Ok(47120), "someone else's program on the saved port");
    }
}
