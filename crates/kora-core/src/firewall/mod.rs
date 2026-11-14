//! Firewall layer: ufw / firewalld / Windows Firewall plans, detection,
//! port-in-use checks and router port-forward notes. Plans are plain data
//! (see [`crate::service::Plan`]) and are only applied with consent.

use crate::service::{CommandRunner, Plan, Platform, ServiceError, ServiceResult, SystemStep};
use std::net::{TcpListener, UdpSocket};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirewallKind {
    Ufw,
    Firewalld,
    WindowsFirewall,
}

/// Which ports to open for a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortRequest {
    pub server: String,
    /// Game port, opened TCP+UDP.
    pub game_port: u16,
    pub txadmin_port: u16,
    /// txAdmin (TCP) is only opened when this is explicitly true.
    pub expose_txadmin: bool,
}

impl PortRequest {
    pub fn validate(&self) -> ServiceResult<()> {
        if self.server.is_empty()
            || !self.server.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            return Err(ServiceError::Invalid("server name has unsupported characters".into()));
        }
        for (label, p) in [("game", self.game_port), ("txAdmin", self.txadmin_port)] {
            if p < 1024 {
                return Err(ServiceError::Invalid(format!(
                    "{label} port {p} is not allowed: choose a port between 1024 and 65535"
                )));
            }
        }
        if self.game_port == self.txadmin_port {
            return Err(ServiceError::Invalid("the game port and txAdmin port must be different".into()));
        }
        Ok(())
    }

    /// (label, port, protocol) tuples that this request opens.
    fn rules(&self) -> Vec<(&'static str, u16, &'static str)> {
        let mut v = vec![("game", self.game_port, "tcp"), ("game", self.game_port, "udp")];
        if self.expose_txadmin {
            v.push(("txAdmin", self.txadmin_port, "tcp"));
        }
        v
    }
}

fn rule_name(server: &str, label: &str, port: u16, proto: &str) -> String {
    format!("Kora {server} {label} {port} {}", proto.to_uppercase())
}

fn plan(req: &PortRequest, kind: FirewallKind, open: bool) -> ServiceResult<Plan> {
    req.validate()?;
    let verb = if open { "Open" } else { "Close" };
    let mut steps = Vec::new();
    for (label, port, proto) in req.rules() {
        let what = format!("{verb} {proto_u} port {port} ({label}) for '{}'", req.server, proto_u = proto.to_uppercase());
        let step = match kind {
            FirewallKind::Ufw if open => SystemStep::run(
                format!("{what} in the Ubuntu/Debian firewall (ufw)"),
                "ufw",
                &["allow", &format!("{port}/{proto}"), "comment", &format!("Kora {} {label}", req.server)],
            ),
            FirewallKind::Ufw => SystemStep::run(
                format!("{what} in the Ubuntu/Debian firewall (ufw)"),
                "ufw",
                &["delete", "allow", &format!("{port}/{proto}")],
            )
            .tolerant(),
            FirewallKind::Firewalld => {
                let flag = if open { "--add-port" } else { "--remove-port" };
                let s = SystemStep::run(
                    format!("{what} in firewalld"),
                    "firewall-cmd",
                    &["--permanent", &format!("{flag}={port}/{proto}")],
                );
                if open { s } else { s.tolerant() }
            }
            FirewallKind::WindowsFirewall => {
                let name = rule_name(&req.server, label, port, proto);
                if open {
                    SystemStep::run_owned(
                        format!("{what} in Windows Firewall"),
                        "netsh",
                        vec![
                            "advfirewall".into(), "firewall".into(), "add".into(), "rule".into(),
                            format!("name={name}"), "dir=in".into(), "action=allow".into(),
                            format!("protocol={}", proto.to_uppercase()), format!("localport={port}"),
                        ],
                    )
                } else {
                    SystemStep::run_owned(
                        format!("{what} in Windows Firewall"),
                        "netsh",
                        vec!["advfirewall".into(), "firewall".into(), "delete".into(), "rule".into(), format!("name={name}")],
                    )
                    .tolerant()
                }
            }
        };
        steps.push(step);
    }
    if kind == FirewallKind::Firewalld {
        steps.push(SystemStep::run("Apply the firewalld changes", "firewall-cmd", &["--reload"]));
    }
    Ok(Plan::new(steps))
}

/// Plan that opens the requested ports. txAdmin is never included unless `expose_txadmin`.
pub fn open_plan(req: &PortRequest, kind: FirewallKind) -> ServiceResult<Plan> {
    plan(req, kind, true)
}

/// Plan that removes exactly the rules [`open_plan`] adds (including txAdmin, if it was opened).
pub fn close_plan(req: &PortRequest, kind: FirewallKind) -> ServiceResult<Plan> {
    let mut all = req.clone();
    all.expose_txadmin = true; // removal is tolerant: missing rules are fine
    plan(&all, kind, false)
}

/// Detect the active firewall for `platform`; `None` if none is active.
pub fn detect(platform: Platform, runner: &dyn CommandRunner) -> Option<FirewallKind> {
    let ok = |prog: &str, args: &[&str]| {
        let a: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        runner.run(prog, &a).ok().filter(|o| o.success())
    };
    match platform {
        Platform::Linux => {
            if ok("ufw", &["status"]).is_some_and(|o| o.stdout.contains("Status: active")) {
                Some(FirewallKind::Ufw)
            } else if ok("firewall-cmd", &["--state"]).is_some_and(|o| o.stdout.trim() == "running") {
                Some(FirewallKind::Firewalld)
            } else {
                None
            }
        }
        // If netsh works at all, add the rules: they are harmless while the firewall is off, and
        // matching the word "ON" in the output missed every non-English Windows ("EIN", "ACTIVADO"...).
        Platform::Windows => ok("netsh", &["advfirewall", "show", "allprofiles", "state"]).map(|_| FirewallKind::WindowsFirewall),
    }
}

/// Result of a port availability check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortStatus {
    Free,
    InUse { tcp: bool, udp: bool },
}

/// Check by trying to bind. `udp` should be true for the game port.
pub fn check_port(port: u16, check_udp: bool) -> PortStatus {
    let tcp = TcpListener::bind(("0.0.0.0", port)).is_err();
    let udp = check_udp && UdpSocket::bind(("0.0.0.0", port)).is_err();
    if tcp || udp {
        PortStatus::InUse { tcp, udp }
    } else {
        PortStatus::Free
    }
}

fn port_in_use_text(port: u16, label: &str, windows: bool) -> String {
    let mut m = format!("Port {port} ({label}) is already used by another program. Pick a different {label} port.");
    if windows {
        m.push_str(" (Windows can also reserve a port for Hyper-V or WSL; if nothing else is using it, run `netsh interface ipv4 show excludedportrange protocol=tcp` to see the reserved ranges.)");
    }
    m
}

/// Conflict messages (plain language) for a request; empty if all is free.
pub fn port_conflicts(req: &PortRequest) -> ServiceResult<Vec<String>> {
    req.validate()?;
    let mut out = Vec::new();
    if let PortStatus::InUse { .. } = check_port(req.game_port, true) {
        out.push(port_in_use_text(req.game_port, "game", cfg!(windows)));
    }
    if let PortStatus::InUse { .. } = check_port(req.txadmin_port, false) {
        out.push(port_in_use_text(req.txadmin_port, "txAdmin", cfg!(windows)));
    }
    Ok(out)
}

/// Plain-language router note. Kora never changes the router (no UPnP).
pub fn router_note(req: &PortRequest) -> String {
    let mut s = format!(
        "To let players outside your home network join, your router must forward port {} (both TCP and UDP) \
         to this computer. Open your router's settings page (often 192.168.0.1 or 192.168.1.1), find \
         \"Port forwarding\", and add that rule pointing at this computer's local address. \
         Kora does not change your router for you.",
        req.game_port
    );
    if req.expose_txadmin {
        s.push_str(&format!(
            " You chose to expose txAdmin: also forward TCP port {}. Anyone who can reach it can try to log in, so keep it off the internet unless you need it.",
            req.txadmin_port
        ));
    } else {
        s.push_str(&format!(
            " Do not forward port {}: it is the txAdmin web panel and is meant for this computer or your local network only.",
            req.txadmin_port
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::testing::FakeRunner;
    use crate::service::{apply_plan, Action};

    fn req(expose: bool) -> PortRequest {
        PortRequest { server: "main".into(), game_port: 30120, txadmin_port: 40120, expose_txadmin: expose }
    }

    fn lines(p: &Plan) -> Vec<String> {
        p.steps.iter().filter_map(|s| s.command_line()).collect()
    }

    #[test]
    fn port_validation() {
        let mut r = req(false);
        r.game_port = 80;
        assert!(r.validate().is_err());
        r = req(false);
        r.txadmin_port = 30120;
        assert!(r.validate().is_err());
        r = req(false);
        r.game_port = 0;
        assert!(open_plan(&r, FirewallKind::Ufw).is_err());
        assert!(req(false).validate().is_ok());
    }

    #[test]
    fn ufw_open_and_close() {
        let p = open_plan(&req(false), FirewallKind::Ufw).unwrap();
        assert_eq!(lines(&p), vec!["ufw allow 30120/tcp comment Kora main game", "ufw allow 30120/udp comment Kora main game"]);
        assert!(p.needs_admin());
        let c = close_plan(&req(false), FirewallKind::Ufw).unwrap();
        assert!(lines(&c).contains(&"ufw delete allow 40120/tcp".to_string()));
        assert!(c.steps.iter().all(|s| s.ignore_failure));
    }

    #[test]
    fn txadmin_not_opened_by_default() {
        for k in [FirewallKind::Ufw, FirewallKind::Firewalld, FirewallKind::WindowsFirewall] {
            let p = open_plan(&req(false), k).unwrap();
            assert!(!lines(&p).iter().any(|l| l.contains("40120")), "{k:?}");
            let p = open_plan(&req(true), k).unwrap();
            assert!(lines(&p).iter().any(|l| l.contains("40120") && !l.contains("udp") && !l.contains("UDP")), "{k:?}");
        }
    }

    #[test]
    fn firewalld_plan() {
        let p = open_plan(&req(false), FirewallKind::Firewalld).unwrap();
        assert_eq!(
            lines(&p),
            vec![
                "firewall-cmd --permanent --add-port=30120/tcp",
                "firewall-cmd --permanent --add-port=30120/udp",
                "firewall-cmd --reload"
            ]
        );
        assert!(lines(&close_plan(&req(false), FirewallKind::Firewalld).unwrap())[0].contains("--remove-port=30120/tcp"));
    }

    #[test]
    fn netsh_plan() {
        let p = open_plan(&req(false), FirewallKind::WindowsFirewall).unwrap();
        let Action::Run { program, args } = &p.steps[0].action else { panic!() };
        assert_eq!(program, "netsh");
        assert_eq!(
            args,
            &["advfirewall", "firewall", "add", "rule", "name=Kora main game 30120 TCP", "dir=in", "action=allow", "protocol=TCP", "localport=30120"]
        );
        let c = close_plan(&req(false), FirewallKind::WindowsFirewall).unwrap();
        assert!(lines(&c)[1].contains("delete rule name=Kora main game 30120 UDP"));
    }

    #[test]
    fn step_descriptions_plain() {
        let p = open_plan(&req(false), FirewallKind::Ufw).unwrap();
        assert_eq!(p.descriptions()[0], "Open TCP port 30120 (game) for 'main' in the Ubuntu/Debian firewall (ufw)");
    }

    #[test]
    fn detection() {
        let r = FakeRunner::default();
        r.respond("ufw status", 0, "Status: active\n");
        assert_eq!(detect(Platform::Linux, &r), Some(FirewallKind::Ufw));
        let r = FakeRunner::default();
        r.respond("ufw status", 0, "Status: inactive\n");
        r.respond("firewall-cmd --state", 0, "running\n");
        assert_eq!(detect(Platform::Linux, &r), Some(FirewallKind::Firewalld));
        let r = FakeRunner::default();
        r.respond("ufw status", 1, "");
        r.respond("firewall-cmd", 252, "not running");
        assert_eq!(detect(Platform::Linux, &r), None);
        let r = FakeRunner::default();
        r.respond("netsh", 0, "Domain Profile Settings:\nState ON\n");
        assert_eq!(detect(Platform::Windows, &r), Some(FirewallKind::WindowsFirewall));
        let r = FakeRunner::default();
        r.respond("netsh", 0, "Domänenprofil-Einstellungen:\nStatus EIN\n");
        assert_eq!(detect(Platform::Windows, &r), Some(FirewallKind::WindowsFirewall), "localized Windows");
        let r = FakeRunner::default();
        r.respond("netsh", 1, "");
        assert_eq!(detect(Platform::Windows, &r), None);
    }

    #[test]
    fn windows_port_hint_mentions_reserved_ranges() {
        assert!(port_in_use_text(30120, "game", true).contains("Hyper-V"));
        assert!(!port_in_use_text(30120, "game", false).contains("Hyper-V"));
    }

    #[test]
    fn apply_through_fake_runner() {
        let r = FakeRunner::default();
        apply_plan(&open_plan(&req(false), FirewallKind::Ufw).unwrap(), &r).unwrap();
        assert_eq!(r.calls().len(), 2);
    }

    #[test]
    fn port_in_use_detected() {
        let l = TcpListener::bind("0.0.0.0:0").unwrap();
        let port = l.local_addr().unwrap().port();
        assert!(matches!(check_port(port, false), PortStatus::InUse { tcp: true, .. }));
        drop(l);
        let u = UdpSocket::bind("0.0.0.0:0").unwrap();
        let up = u.local_addr().unwrap().port();
        assert!(matches!(check_port(up, true), PortStatus::InUse { udp: true, .. }));
    }

    #[test]
    fn router_note_text() {
        let n = router_note(&req(false));
        assert!(n.contains("30120") && n.contains("TCP and UDP"));
        assert!(n.contains("Do not forward port 40120"));
        assert!(router_note(&req(true)).contains("also forward TCP port 40120"));
    }
}
