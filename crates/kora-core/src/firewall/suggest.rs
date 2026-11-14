//! Suggest the next free port. A port is "free" when nothing listens on it (TCP, and for the
//! game port also UDP) and no OTHER Kora server in the registry has claimed it, even when that
//! server is stopped. On Windows a bind test also fails inside the Hyper-V / WinNAT reserved
//! ranges (permission denied), which is exactly what we want: such a port cannot be used.

use super::{check_port, PortRequest, PortStatus};
use crate::installer::{slug, Registry, ServerEntry};
use std::path::Path;
use std::process::Command;

/// Lowest port Kora allows.
pub const MIN_PORT: u16 = 1024;
/// How many candidates are looked at before giving up.
pub const SEARCH_LIMIT: usize = 500;

/// Answers "can this port be used right now?" (fake-able in tests).
pub trait PortProber {
    fn is_free(&self, port: u16) -> bool;
}

/// Real check by trying to bind. `udp` = also test UDP (game port).
#[derive(Debug, Clone, Copy)]
pub struct SystemProber {
    pub udp: bool,
}

impl PortProber for SystemProber {
    fn is_free(&self, port: u16) -> bool {
        port >= MIN_PORT && check_port(port, self.udp) == PortStatus::Free
    }
}

/// The next port above `start` that is free and not in `avoid`. Never goes above 65535 (no
/// wrapping), never below 1024, and looks at most [`SEARCH_LIMIT`] candidates.
pub fn suggest_free_port(start: u16, avoid: &[u16], prober: &dyn PortProber) -> Option<u16> {
    let first = (u32::from(start) + 1).max(u32::from(MIN_PORT));
    (first..=u32::from(u16::MAX))
        .take(SEARCH_LIMIT)
        .map(|p| p as u16)
        .find(|p| !avoid.contains(p) && prober.is_free(*p))
}

/// Is this port usable for a new server (free and not in `avoid`)?
pub fn port_ok(port: u16, avoid: &[u16], prober: &dyn PortProber) -> bool {
    port >= MIN_PORT && !avoid.contains(&port) && prober.is_free(port)
}

/// Ports claimed by servers other than the one being installed (matched by folder or name).
pub fn registered_ports(servers: &[ServerEntry], own_name: &str, own_dir: Option<&Path>) -> Vec<u16> {
    servers
        .iter()
        .filter(|s| slug(&s.name) != slug(own_name) && Some(s.dir.as_path()) != own_dir)
        .flat_map(|s| [s.ports.game, s.ports.txadmin])
        .collect()
}

/// Registry servers now (empty when the registry can't be read).
pub fn registered_servers() -> Vec<ServerEntry> {
    Registry::load().map(|r| r.servers).unwrap_or_default()
}

/// Pick usable ports starting from the requested ones: unchanged when fine, otherwise the next
/// free ones. `reg` = ports of other Kora servers. `None` when no free port could be found.
pub fn resolve_ports(game: u16, tx: u16, reg: &[u16], game_probe: &dyn PortProber, tx_probe: &dyn PortProber) -> Option<(u16, u16)> {
    let avoid_for_game: Vec<u16> = reg.iter().copied().chain([tx]).collect();
    let g = if port_ok(game, &avoid_for_game, game_probe) { game } else { suggest_free_port(game, &avoid_for_game, game_probe)? };
    let avoid_for_tx: Vec<u16> = reg.iter().copied().chain([g]).collect();
    let t = if port_ok(tx, &avoid_for_tx, tx_probe) { tx } else { suggest_free_port(tx, &avoid_for_tx, tx_probe)? };
    Some((g, t))
}

/// Plain-language advice for a failed port check: which port, what uses it (when it can be found
/// cheaply) and the next free ports, plus how to apply them. Falls back to the plain conflict texts.
pub fn port_advice(req: &PortRequest, conflicts: &[String]) -> String {
    let reg = registered_ports(&registered_servers(), &req.server, None);
    let (gp, tp) = (SystemProber { udp: true }, SystemProber { udp: false });
    let Some((g, t)) = resolve_ports(req.game_port, req.txadmin_port, &reg, &gp, &tp) else {
        return conflicts.join(" ");
    };
    let mut parts = Vec::new();
    for (label, port, now) in [("game", req.game_port, g), ("txAdmin", req.txadmin_port, t)] {
        if port == now {
            continue;
        }
        let who = describe_port_user(port).map(|w| format!(" ({w})")).unwrap_or_default();
        parts.push(format!("Port {port} ({label}) is already used by another program{who}. The next free port is {now}."));
    }
    if parts.is_empty() {
        return conflicts.join(" ");
    }
    parts.push(format!("Run the command again with --game-port {g} --txadmin-port {t}, or add --auto-ports to let Kora pick free ports."));
    parts.join(" ")
}

/// Name of the program using `port` (best effort, `None` when unknown or not cheap to find).
pub fn describe_port_user(port: u16) -> Option<String> {
    if cfg!(windows) {
        let out = Command::new("netstat").args(["-ano"]).output().ok()?;
        let pid = parse_netstat(&String::from_utf8_lossy(&out.stdout), port)?;
        let out = Command::new("tasklist").args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"]).output().ok()?;
        parse_tasklist(&String::from_utf8_lossy(&out.stdout))
    } else {
        let out = Command::new("ss").args(["-H", "-ltnup"]).output().ok()?;
        parse_ss(&String::from_utf8_lossy(&out.stdout), port)
    }
}

fn local_port(addr: &str) -> Option<u16> {
    addr.rsplit_once(':')?.1.parse().ok()
}

/// `ss -H -ltnup` output -> process name listening on `port` (`users:(("fxserver",pid=12,fd=3))`).
pub fn parse_ss(out: &str, port: u16) -> Option<String> {
    out.lines().find_map(|l| {
        let cols: Vec<&str> = l.split_whitespace().collect();
        (local_port(cols.get(4)?)? == port).then_some(())?;
        let rest = l.split("users:((\"").nth(1)?;
        Some(rest.split('"').next()?.to_string())
    })
}

/// `netstat -ano` output -> PID bound to `port` (listening TCP or UDP).
pub fn parse_netstat(out: &str, port: u16) -> Option<u32> {
    out.lines().find_map(|l| {
        let cols: Vec<&str> = l.split_whitespace().collect();
        let proto = cols.first()?.to_ascii_uppercase();
        if !proto.starts_with("TCP") && !proto.starts_with("UDP") {
            return None;
        }
        (local_port(cols.get(1)?)? == port).then_some(())?;
        cols.last()?.parse().ok()
    })
}

/// `tasklist /FO CSV /NH` output -> image name of the first row.
pub fn parse_tasklist(out: &str) -> Option<String> {
    let first = out.lines().next()?.trim();
    let name = first.strip_prefix('"')?.split('"').next()?;
    (!name.is_empty()).then(|| name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::installer::{registry::PortsRecord, Mode};
    use std::collections::HashSet;

    struct Taken(HashSet<u16>);
    impl PortProber for Taken {
        fn is_free(&self, p: u16) -> bool {
            !self.0.contains(&p)
        }
    }
    fn taken(v: &[u16]) -> Taken {
        Taken(v.iter().copied().collect())
    }

    #[test]
    fn next_higher_free_port_skipping_taken_and_avoided() {
        assert_eq!(suggest_free_port(30120, &[], &taken(&[])), Some(30121));
        assert_eq!(suggest_free_port(30120, &[], &taken(&[30121, 30122])), Some(30123));
        assert_eq!(suggest_free_port(30120, &[30121], &taken(&[30122])), Some(30123), "avoid list is skipped too");
        assert_eq!(suggest_free_port(30120, &[], &taken(&[30119])), Some(30121), "only higher ports are suggested");
    }

    #[test]
    fn stays_inside_the_allowed_range_and_never_wraps() {
        assert_eq!(suggest_free_port(65535, &[], &taken(&[])), None);
        assert_eq!(suggest_free_port(65534, &[], &taken(&[])), Some(65535));
        assert_eq!(suggest_free_port(65534, &[], &taken(&[65535])), None, "no wrap to low ports");
        assert_eq!(suggest_free_port(10, &[], &taken(&[])), Some(1024), "never below 1024");
        assert_eq!(suggest_free_port(0, &[], &taken(&[1024])), Some(1025));
    }

    #[test]
    fn gives_up_after_about_500_candidates() {
        let busy: Vec<u16> = (30121..=30700).collect();
        assert_eq!(suggest_free_port(30120, &[], &taken(&busy)), None);
        let busy: Vec<u16> = (30121..30600).collect();
        assert_eq!(suggest_free_port(30120, &[], &taken(&busy)), Some(30600));
    }

    fn entry(name: &str, dir: &str, g: u16, t: u16) -> ServerEntry {
        ServerEntry { name: name.into(), dir: dir.into(), mode: Mode::Production, ports: PortsRecord { game: g, txadmin: t }, created_unix: 0 }
    }

    #[test]
    fn other_kora_servers_keep_their_ports_even_when_stopped() {
        let servers = vec![entry("Alpha", "/a", 30120, 40120), entry("Beta", "/b", 30121, 40121)];
        assert_eq!(registered_ports(&servers, "New", Some(Path::new("/n"))), vec![30120, 40120, 30121, 40121]);
        assert_eq!(registered_ports(&servers, "alpha", None), vec![30121, 40121], "the server being installed is not 'other'");
        assert_eq!(registered_ports(&servers, "New", Some(Path::new("/b"))), vec![30120, 40120]);
        let reg = registered_ports(&servers, "New", None);
        assert_eq!(resolve_ports(30120, 40120, &reg, &taken(&[]), &taken(&[])), Some((30122, 40122)));
    }

    #[test]
    fn resolve_keeps_good_ports_and_moves_taken_ones() {
        assert_eq!(resolve_ports(30120, 40120, &[], &taken(&[]), &taken(&[])), Some((30120, 40120)));
        assert_eq!(resolve_ports(30120, 40120, &[], &taken(&[30120]), &taken(&[])), Some((30121, 40120)));
        assert_eq!(resolve_ports(30120, 40120, &[], &taken(&[]), &taken(&[40120, 40121])), Some((30120, 40122)));
        // the two ports never collide
        assert_eq!(resolve_ports(30120, 30121, &[], &taken(&[30120]), &taken(&[])), Some((30122, 30121)));
        assert_eq!(resolve_ports(65535, 40120, &[], &taken(&[65535]), &taken(&[])), None, "exhausted");
    }

    #[test]
    fn advice_names_the_port_and_the_next_free_one() {
        let l = std::net::TcpListener::bind(("0.0.0.0", 0)).unwrap();
        let p = l.local_addr().unwrap().port();
        if p >= 65000 {
            return; // no room above; not worth being flaky
        }
        let req = PortRequest { server: "kora-advice-test".into(), game_port: p, txadmin_port: 40999, expose_txadmin: false };
        let text = port_advice(&req, &["plain".into()]);
        assert!(text.contains(&format!("Port {p} (game)")) && text.contains("--game-port") && text.contains("--auto-ports"), "{text}");
        assert!(!text.contains(&format!("--game-port {p} ")), "{text}");
        drop(l);
    }

    #[test]
    fn process_lookup_parsers() {
        let ss = "tcp LISTEN 0 511 0.0.0.0:30120 0.0.0.0:* users:((\"FXServer\",pid=4242,fd=7))\nudp UNCONN 0 0 *:5353 *:*\n";
        assert_eq!(parse_ss(ss, 30120).as_deref(), Some("FXServer"));
        assert_eq!(parse_ss(ss, 30121), None);
        let ns = "  TCP    0.0.0.0:30120    0.0.0.0:0    LISTENING    1234\n  UDP    0.0.0.0:30120    *:*    1234\n  TCP [::]:3306 [::]:0 LISTENING 99\n";
        assert_eq!(parse_netstat(ns, 30120), Some(1234));
        assert_eq!(parse_netstat(ns, 3306), Some(99));
        assert_eq!(parse_netstat(ns, 1), None);
        assert_eq!(parse_tasklist("\"FXServer.exe\",\"1234\",\"Console\",\"1\",\"50,000 K\"\r\n").as_deref(), Some("FXServer.exe"));
        assert_eq!(parse_tasklist("INFO: No tasks are running"), None);
    }
}
