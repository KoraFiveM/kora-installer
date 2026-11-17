//! Port conflict checking with a recommendation of the next higher free port. A port is free
//! when it can be bound (TCP, and UDP for the game port), is not reserved by Windows
//! (Hyper-V/WinNAT excluded ranges) and is not claimed by another Kora server of this machine.

use crate::installer::{slug, Registry, ServerEntry};
use std::net::{Ipv6Addr, TcpListener, UdpSocket};
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

/// Lowest port Kora allows.
pub const MIN_PORT: u16 = 1024;
/// How many candidates are looked at before giving up.
pub const SEARCH_LIMIT: usize = 500;

/// Answers "can this port be used right now?" (fake-able in tests).
pub trait PortProber {
    fn tcp_free(&self, port: u16) -> bool;
    fn udp_free(&self, port: u16) -> bool;
}

/// Real check: bind tests on 0.0.0.0 (and `[::]` where IPv6 exists) plus the Windows reserved ranges.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemProber;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortStatus {
    Free,
    /// In use (or reserved, or claimed by another Kora server); `suggestion` = next free port.
    InUse {
        suggestion: Option<u16>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortReport {
    pub game: PortStatus,
    pub txadmin: PortStatus,
}

impl PortReport {
    pub fn all_free(&self) -> bool {
        self.game == PortStatus::Free && self.txadmin == PortStatus::Free
    }
}

/// Ports that cannot be bound for sure: only an "address in use" or "permission denied" answer
/// counts on IPv6; a machine without IPv6 simply skips that test.
fn v6_blocked(e: &std::io::Error) -> bool {
    matches!(e.kind(), std::io::ErrorKind::AddrInUse | std::io::ErrorKind::PermissionDenied)
}

impl PortProber for SystemProber {
    fn tcp_free(&self, port: u16) -> bool {
        if port < MIN_PORT || windows_excluded(port) {
            return false;
        }
        // Windows lets a wildcard bind succeed while another program holds only the loopback address
        // (an existing MariaDB does exactly that), so the loopback addresses are probed too.
        for host in ["0.0.0.0", "127.0.0.1"] {
            if TcpListener::bind((host, port)).is_err() {
                return false;
            }
        }
        [Ipv6Addr::UNSPECIFIED, Ipv6Addr::LOCALHOST].iter().all(|a| !matches!(TcpListener::bind((*a, port)), Err(e) if v6_blocked(&e)))
    }

    fn udp_free(&self, port: u16) -> bool {
        if port < MIN_PORT {
            return false;
        }
        for host in ["0.0.0.0", "127.0.0.1"] {
            if UdpSocket::bind((host, port)).is_err() {
                return false;
            }
        }
        [Ipv6Addr::UNSPECIFIED, Ipv6Addr::LOCALHOST].iter().all(|a| !matches!(UdpSocket::bind((*a, port)), Err(e) if v6_blocked(&e)))
    }
}

/// Parse `netsh int ipv4 show excludedportrange protocol=tcp` output into (start, end) ranges.
pub fn parse_excluded_ranges(out: &str) -> Vec<(u16, u16)> {
    out.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let a: u16 = it.next()?.parse().ok()?;
            let b: u16 = it.next()?.parse().ok()?;
            (a <= b).then_some((a, b))
        })
        .collect()
}

fn windows_excluded(port: u16) -> bool {
    static RANGES: OnceLock<Vec<(u16, u16)>> = OnceLock::new();
    if !cfg!(windows) {
        return false;
    }
    let r = RANGES.get_or_init(|| {
        crate::sysexe::command("netsh")
            .args(["int", "ipv4", "show", "excludedportrange", "protocol=tcp"])
            .output()
            .map(|o| parse_excluded_ranges(&String::from_utf8_lossy(&o.stdout)))
            .unwrap_or_default() // netsh missing: fail open to the bind test
    });
    r.iter().any(|&(a, b)| (a..=b).contains(&port))
}

fn free_for(prober: &dyn PortProber, port: u16, need_udp: bool) -> bool {
    port >= MIN_PORT && prober.tcp_free(port) && (!need_udp || prober.udp_free(port))
}

/// Game and txAdmin ports of the OTHER Kora servers in the registry (also when stopped).
pub fn registered_ports() -> Vec<u16> {
    registered_ports_except("", None)
}

/// Same, leaving out the server being installed (matched by name or folder).
pub fn registered_ports_except(own_name: &str, own_dir: Option<&Path>) -> Vec<u16> {
    let servers = Registry::load().map(|r| r.servers).unwrap_or_default();
    ports_of(&servers, own_name, own_dir)
}

fn ports_of(servers: &[ServerEntry], own_name: &str, own_dir: Option<&Path>) -> Vec<u16> {
    servers
        .iter()
        .filter(|s| (own_name.is_empty() || slug(&s.name) != slug(own_name)) && (own_dir.is_none() || Some(s.dir.as_path()) != own_dir))
        .flat_map(|s| [s.ports.game, s.ports.txadmin])
        .collect()
}

/// Smallest port above `start` that is free, not in `avoid`, within 1024..=65535 (never wraps),
/// looking at most [`SEARCH_LIMIT`] candidates. `need_udp`: the game port (TCP and UDP).
pub fn suggest_free_port(start: u16, avoid: &[u16], need_udp: bool, prober: &dyn PortProber) -> Option<u16> {
    let first = (u32::from(start) + 1).max(u32::from(MIN_PORT));
    (first..=u32::from(u16::MAX)).take(SEARCH_LIMIT).map(|p| p as u16).find(|p| !avoid.contains(p) && free_for(prober, *p, need_udp))
}

/// Check both ports; suggestions never collide with each other or with `avoid`.
pub fn check_ports(game: u16, txadmin: u16, avoid: &[u16], prober: &dyn PortProber) -> PortReport {
    let with = |extra: u16| -> Vec<u16> { avoid.iter().copied().chain([extra]).collect() };
    let game_ok = !avoid.contains(&game) && game != txadmin && free_for(prober, game, true);
    let game_status = if game_ok { PortStatus::Free } else { PortStatus::InUse { suggestion: suggest_free_port(game, &with(txadmin), true, prober) } };
    // The game port txAdmin has to stay clear of: the one that will really be used.
    let game_final = match game_status {
        PortStatus::Free => Some(game),
        PortStatus::InUse { suggestion } => suggestion,
    };
    let tx_avoid: Vec<u16> = avoid.iter().copied().chain(game_final).chain([game]).collect();
    let tx_ok = !avoid.contains(&txadmin) && txadmin != game && free_for(prober, txadmin, false);
    let tx_status = if tx_ok { PortStatus::Free } else { PortStatus::InUse { suggestion: suggest_free_port(txadmin, &tx_avoid, false, prober) } };
    PortReport { game: game_status, txadmin: tx_status }
}

/// The ports to use: the requested ones when free, otherwise the suggestions. `None` = exhausted.
pub fn effective_ports(game: u16, txadmin: u16, avoid: &[u16], prober: &dyn PortProber) -> Option<(u16, u16)> {
    let r = check_ports(game, txadmin, avoid, prober);
    let pick = |s: PortStatus, req: u16| match s {
        PortStatus::Free => Some(req),
        PortStatus::InUse { suggestion } => suggestion,
    };
    Some((pick(r.game, game)?, pick(r.txadmin, txadmin)?))
}

/// Plain-language sentences for each conflicting port (name, what uses it when cheaply known,
/// next free port). Empty when both are fine.
pub fn conflict_messages(game: u16, txadmin: u16, report: &PortReport, avoid: &[u16]) -> Vec<String> {
    let mut v = Vec::new();
    for (label, port, st) in [("game", game, report.game), ("txAdmin", txadmin, report.txadmin)] {
        let PortStatus::InUse { suggestion } = st else { continue };
        let why = if avoid.contains(&port) {
            " (another Kora server on this computer uses it)".to_string()
        } else if port == if label == "game" { txadmin } else { game } {
            " (the game and txAdmin ports must differ)".to_string()
        } else {
            describe_port_user(port).map(|w| format!(" (used by {w})")).unwrap_or_default()
        };
        let next = suggestion.map(|s| format!(" The next free port is {s}.")).unwrap_or_else(|| " No free port was found nearby.".into());
        v.push(format!("Port {port} ({label}) is already in use{why}.{next}"));
    }
    v
}

/// Everything wrong with the ports of an install (other Kora servers of this computer count as
/// taken), as plain sentences naming the port, who uses it and the next free port. Empty = fine.
pub fn install_conflicts(name: &str, dir: &Path, game: u16, txadmin: u16, prober: &dyn PortProber) -> Vec<String> {
    let avoid = registered_ports_except(name, Some(dir));
    let report = check_ports(game, txadmin, &avoid, prober);
    conflict_messages(game, txadmin, &report, &avoid)
}

/// Ports to use for an install and what changed: `(game, txadmin; `None` = no free port.
pub fn install_effective(name: &str, dir: &Path, game: u16, txadmin: u16, prober: &dyn PortProber) -> Option<(u16, u16)> {
    effective_ports(game, txadmin, &registered_ports_except(name, Some(dir)), prober)
}

/// Name of the program using `port` (best effort; `None` when unknown or the lookup fails).
pub fn describe_port_user(port: u16) -> Option<String> {
    if cfg!(windows) {
        let out = crate::sysexe::command("netstat").args(["-ano"]).output().ok()?;
        let pid = parse_netstat(&String::from_utf8_lossy(&out.stdout), port)?;
        let out = crate::sysexe::command("tasklist").args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"]).output().ok()?;
        parse_tasklist(&String::from_utf8_lossy(&out.stdout))
    } else {
        let out = Command::new("ss").args(["-H", "-ltnup"]).output().ok()?;
        parse_ss(&String::from_utf8_lossy(&out.stdout), port)
    }
}

fn local_port(addr: &str) -> Option<u16> {
    addr.rsplit_once(':')?.1.parse().ok()
}

/// `ss -H -ltnup` output -> process name using `port`.
pub fn parse_ss(out: &str, port: u16) -> Option<String> {
    out.lines().find_map(|l| {
        let cols: Vec<&str> = l.split_whitespace().collect();
        (local_port(cols.get(4)?)? == port).then_some(())?;
        let rest = l.split("users:((\"").nth(1)?;
        Some(rest.split('"').next()?.to_string())
    })
}

/// `netstat -ano` output -> PID bound to `port`.
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
    let name = out.lines().next()?.trim().strip_prefix('"')?.split('"').next()?;
    (!name.is_empty()).then(|| name.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_loopback_only_listener_makes_the_port_not_free() {
        // Windows allows a wildcard bind next to a loopback-only listener; an existing MariaDB looks like this.
        let held = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = held.local_addr().unwrap().port();
        assert!(!SystemProber.tcp_free(port), "port {port} is held on 127.0.0.1");
        drop(held);
    }

    use super::*;
    use crate::installer::{registry::PortsRecord, Mode};
    use std::collections::HashSet;

    #[derive(Default)]
    struct Fake {
        tcp: HashSet<u16>,
        udp: HashSet<u16>,
    }
    impl PortProber for Fake {
        fn tcp_free(&self, p: u16) -> bool {
            !self.tcp.contains(&p)
        }
        fn udp_free(&self, p: u16) -> bool {
            !self.udp.contains(&p)
        }
    }
    fn tcp(v: &[u16]) -> Fake {
        Fake { tcp: v.iter().copied().collect(), ..Default::default() }
    }

    #[test]
    fn next_higher_free_port_skipping_taken_and_avoided() {
        assert_eq!(suggest_free_port(30120, &[], true, &tcp(&[])), Some(30121));
        assert_eq!(suggest_free_port(30120, &[], true, &tcp(&[30121, 30122])), Some(30123));
        assert_eq!(suggest_free_port(30120, &[30121], true, &tcp(&[30122])), Some(30123));
        assert_eq!(suggest_free_port(30120, &[], true, &tcp(&[30119])), Some(30121));
    }

    #[test]
    fn range_limits_and_exhaustion() {
        assert_eq!(suggest_free_port(65535, &[], false, &tcp(&[])), None);
        assert_eq!(suggest_free_port(65534, &[], false, &tcp(&[])), Some(65535));
        assert_eq!(suggest_free_port(65534, &[], false, &tcp(&[65535])), None);
        assert_eq!(suggest_free_port(10, &[], false, &tcp(&[])), Some(1024));
        let busy: Vec<u16> = (30121..=30700).collect();
        assert_eq!(suggest_free_port(30120, &[], false, &tcp(&busy)), None);
        let busy: Vec<u16> = (30121..30600).collect();
        assert_eq!(suggest_free_port(30120, &[], false, &tcp(&busy)), Some(30600));
    }

    #[test]
    fn udp_matters_only_for_the_game_port() {
        let f = Fake { udp: [30121].into_iter().collect(), ..Default::default() };
        assert_eq!(suggest_free_port(30120, &[], true, &f), Some(30122));
        assert_eq!(suggest_free_port(30120, &[], false, &f), Some(30121));
        assert_eq!(check_ports(30121, 40120, &[], &f).game, PortStatus::InUse { suggestion: Some(30122) });
        assert_eq!(check_ports(40120, 30121, &[], &f).txadmin, PortStatus::Free);
    }

    #[test]
    fn game_and_txadmin_never_collide() {
        let r = check_ports(30120, 30121, &[], &tcp(&[30120, 30121]));
        assert_eq!((r.game, r.txadmin), (PortStatus::InUse { suggestion: Some(30122) }, PortStatus::InUse { suggestion: Some(30123) }));
        // game moves onto the txAdmin port's neighbour; txAdmin busy -> must not take the game's pick
        let r = check_ports(30120, 30122, &[], &tcp(&[30120, 30122]));
        assert_eq!((r.game, r.txadmin), (PortStatus::InUse { suggestion: Some(30121) }, PortStatus::InUse { suggestion: Some(30123) }));
        let r = check_ports(30120, 30120, &[], &tcp(&[]));
        assert_eq!(r.game, PortStatus::InUse { suggestion: Some(30121) });
        assert_eq!(r.txadmin, PortStatus::InUse { suggestion: Some(30122) });
    }
    #[test]
    fn avoided_ports_count_as_taken_and_are_not_suggested() {
        let r = check_ports(30120, 40120, &[30120, 30121], &tcp(&[]));
        assert_eq!(r.game, PortStatus::InUse { suggestion: Some(30122) });
        assert_eq!(r.txadmin, PortStatus::Free);
        assert!(check_ports(30120, 40120, &[], &tcp(&[])).all_free());
        assert_eq!(effective_ports(30120, 40120, &[30120], &tcp(&[40120])), Some((30121, 40121)));
        assert_eq!(effective_ports(65535, 40120, &[], &tcp(&[65535])), None);
    }

    fn entry(name: &str, dir: &str, g: u16, t: u16) -> ServerEntry {
        ServerEntry { source: None, versions: None, name: name.into(), dir: dir.into(), mode: Mode::Production, ports: PortsRecord { game: g, txadmin: t }, created_unix: 0 }
    }

    #[test]
    fn registry_ports_of_other_servers() {
        let s = vec![entry("Alpha", "/a", 30120, 40120), entry("Beta", "/b", 30121, 40121)];
        assert_eq!(ports_of(&s, "", None), vec![30120, 40120, 30121, 40121]);
        assert_eq!(ports_of(&s, "alpha", None), vec![30121, 40121]);
        assert_eq!(ports_of(&s, "New", Some(Path::new("/b"))), vec![30120, 40120]);
    }

    #[test]
    fn messages_name_port_and_next_free() {
        let f = tcp(&[30120]);
        let r = check_ports(30120, 40120, &[], &f);
        let m = conflict_messages(30120, 40120, &r, &[]);
        assert_eq!(m.len(), 1);
        assert!(m[0].starts_with("Port 30120 (game) is already in use") && m[0].contains("next free port is 30121"), "{m:?}");
    }

    #[test]
    fn parsers() {
        let ss = "tcp LISTEN 0 511 0.0.0.0:30120 0.0.0.0:* users:((\"FXServer\",pid=4242,fd=7))\n";
        assert_eq!(parse_ss(ss, 30120).as_deref(), Some("FXServer"));
        assert_eq!(parse_ss(ss, 1), None);
        let ns = "  TCP    0.0.0.0:30120    0.0.0.0:0    LISTENING    1234\n";
        assert_eq!(parse_netstat(ns, 30120), Some(1234));
        assert_eq!(parse_tasklist("\"FXServer.exe\",\"1234\"\r\n").as_deref(), Some("FXServer.exe"));
        assert_eq!(parse_tasklist("INFO: No tasks"), None);
        let nets = "Start Port    End Port\n----------    --------\n      1914        2013\n     50000       50059     *\n";
        assert_eq!(parse_excluded_ranges(nets), vec![(1914, 2013), (50000, 50059)]);
    }

    #[test]
    fn system_prober_sees_a_real_listener() {
        let l = TcpListener::bind(("0.0.0.0", 0)).unwrap();
        let p = l.local_addr().unwrap().port();
        assert!(!SystemProber.tcp_free(p));
        drop(l);
        assert!(!SystemProber.tcp_free(80), "below 1024 is never free");
    }
}
