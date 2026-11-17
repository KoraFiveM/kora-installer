//! The computer's default gateway (the router), for NAT-PMP / PCP. Parsers are pure.

use std::net::Ipv4Addr;

/// `/proc/net/route`: the line whose destination and mask are 0 and whose flags say "up, gateway".
/// Addresses are little-endian hex.
pub fn parse_proc_route(text: &str) -> Option<Ipv4Addr> {
    let mut best: Option<(u32, Ipv4Addr)> = None;
    for line in text.lines().skip(1) {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 8 {
            continue;
        }
        let (Ok(dest), Ok(flags), Ok(mask), Ok(gw)) = (u32::from_str_radix(f[1], 16), u32::from_str_radix(f[3], 16), u32::from_str_radix(f[7], 16), u32::from_str_radix(f[2], 16)) else {
            continue;
        };
        if dest != 0 || mask != 0 || flags & 0x3 != 0x3 || gw == 0 {
            continue;
        }
        let metric = f.get(6).and_then(|m| m.parse::<u32>().ok()).unwrap_or(0);
        let ip = Ipv4Addr::from(gw.swap_bytes());
        if best.is_none_or(|(m, _)| metric < m) {
            best = Some((metric, ip));
        }
    }
    best.map(|(_, ip)| ip)
}

/// `route print -4 0.0.0.0` on Windows: the row `0.0.0.0  0.0.0.0  <gateway>  <interface>  <metric>`.
pub fn parse_route_print(text: &str) -> Option<Ipv4Addr> {
    let mut best: Option<(u32, Ipv4Addr)> = None;
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 5 || f[0] != "0.0.0.0" || f[1] != "0.0.0.0" {
            continue;
        }
        let Ok(gw) = f[2].parse::<Ipv4Addr>() else { continue };
        let metric = f[4].parse::<u32>().unwrap_or(u32::MAX);
        if gw.is_unspecified() {
            continue;
        }
        if best.is_none_or(|(m, _)| metric < m) {
            best = Some((metric, gw));
        }
    }
    best.map(|(_, ip)| ip)
}

#[cfg(target_os = "linux")]
pub fn default_gateway_v4() -> Option<Ipv4Addr> {
    parse_proc_route(&std::fs::read_to_string("/proc/net/route").ok()?)
}

#[cfg(windows)]
pub fn default_gateway_v4() -> Option<Ipv4Addr> {
    let out = crate::sysexe::command("route").args(["print", "-4", "0.0.0.0"]).output().ok()?;
    parse_route_print(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn default_gateway_v4() -> Option<Ipv4Addr> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_route_table() {
        let t = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
                 eth0\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\n\
                 eth0\t0001A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0\n";
        assert_eq!(parse_proc_route(t), Some("192.168.1.1".parse().unwrap()));
        assert_eq!(parse_proc_route("Iface\n"), None);
        // A route that is down (flag 2 missing) is not a gateway.
        let down = "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\n eth0\t00000000\t0101A8C0\t0001\t0\t0\t100\t00000000\n";
        assert_eq!(parse_proc_route(down), None);
    }

    #[test]
    fn windows_route_print() {
        let t = "IPv4 Route Table\n===\nActive Routes:\nNetwork Destination        Netmask          Gateway       Interface  Metric\n          0.0.0.0          0.0.0.0      192.168.0.1     192.168.0.23     25\n        127.0.0.0        255.0.0.0         On-link         127.0.0.1    331\n";
        assert_eq!(parse_route_print(t), Some("192.168.0.1".parse().unwrap()));
        assert_eq!(parse_route_print("nothing"), None);
    }
}
