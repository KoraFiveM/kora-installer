//! What kind of address is this? Pure functions, so the NAT / carrier-grade NAT decisions are
//! tested on every system without a network.

use std::net::{IpAddr, Ipv4Addr};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddrClass {
    Unspecified,
    Loopback,
    /// 10/8, 172.16/12, 192.168/16 (and IPv6 unique-local fc00::/7).
    Private,
    /// 169.254/16 (and fe80::/10).
    LinkLocal,
    /// 100.64.0.0/10 (RFC 6598): an internet provider's shared address space. Port forwarding on
    /// the home router cannot work when the router's own "internet" address is in here.
    Cgnat,
    /// Documentation, benchmarking, multicast, reserved: never a real public address.
    Special,
    Public,
}

pub fn is_cgnat_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 100 && (o[1] & 0b1100_0000) == 64
}

pub fn classify(ip: IpAddr) -> AddrClass {
    match ip.to_canonical() {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            if v4.is_unspecified() {
                AddrClass::Unspecified
            } else if v4.is_loopback() {
                AddrClass::Loopback
            } else if v4.is_private() {
                AddrClass::Private
            } else if v4.is_link_local() {
                AddrClass::LinkLocal
            } else if is_cgnat_v4(v4) {
                AddrClass::Cgnat
            } else if v4.is_multicast()
                || v4.is_broadcast()
                || o[0] == 0
                || o[0] >= 240
                || (o[0] == 192 && o[1] == 0 && o[2] == 0)
                || (o[0] == 192 && o[1] == 0 && o[2] == 2)
                || (o[0] == 198 && o[1] == 51 && o[2] == 100)
                || (o[0] == 203 && o[1] == 0 && o[2] == 113)
                || (o[0] == 198 && (o[1] & 0b1111_1110) == 18)
            {
                AddrClass::Special
            } else {
                AddrClass::Public
            }
        }
        IpAddr::V6(v6) => {
            let s = v6.segments();
            if v6.is_unspecified() {
                AddrClass::Unspecified
            } else if v6.is_loopback() {
                AddrClass::Loopback
            } else if (s[0] & 0xfe00) == 0xfc00 {
                AddrClass::Private
            } else if (s[0] & 0xffc0) == 0xfe80 {
                AddrClass::LinkLocal
            } else if v6.is_multicast() || (s[0] == 0x2001 && s[1] == 0x0db8) {
                AddrClass::Special
            } else {
                AddrClass::Public
            }
        }
    }
}

/// Could a program on this computer have a reason to talk to this address on the local network?
/// (Router discovery only ever talks to these.)
pub fn is_local_network(ip: IpAddr) -> bool {
    matches!(classify(ip), AddrClass::Private | AddrClass::LinkLocal | AddrClass::Cgnat | AddrClass::Loopback)
}

/// Parse a public address from text an echo service sent: one address, nothing else.
pub fn parse_ip_text(text: &str) -> Option<IpAddr> {
    let t = text.trim();
    if t.is_empty() || t.len() > 45 {
        return None;
    }
    t.parse::<IpAddr>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(s: &str) -> AddrClass {
        classify(s.parse().unwrap())
    }

    #[test]
    fn classifies_the_ranges_that_matter() {
        assert_eq!(c("192.168.1.20"), AddrClass::Private);
        assert_eq!(c("10.0.0.5"), AddrClass::Private);
        assert_eq!(c("172.16.0.1"), AddrClass::Private);
        assert_eq!(c("172.32.0.1"), AddrClass::Public);
        assert_eq!(c("127.0.0.1"), AddrClass::Loopback);
        assert_eq!(c("169.254.3.4"), AddrClass::LinkLocal);
        assert_eq!(c("8.8.8.8"), AddrClass::Public);
        assert_eq!(c("192.0.2.1"), AddrClass::Special);
        assert_eq!(c("0.0.0.0"), AddrClass::Unspecified);
        assert_eq!(c("::1"), AddrClass::Loopback);
        assert_eq!(c("fd00::1"), AddrClass::Private);
        assert_eq!(c("2606:4700::1111"), AddrClass::Public);
        assert_eq!(c("::ffff:192.168.0.9"), AddrClass::Private);
    }

    #[test]
    fn cgnat_is_exactly_100_64_slash_10() {
        assert_eq!(c("100.64.0.0"), AddrClass::Cgnat);
        assert_eq!(c("100.100.100.100"), AddrClass::Cgnat);
        assert_eq!(c("100.127.255.255"), AddrClass::Cgnat);
        assert_eq!(c("100.63.255.255"), AddrClass::Public);
        assert_eq!(c("100.128.0.0"), AddrClass::Public);
    }

    #[test]
    fn echo_text_must_be_one_address() {
        assert_eq!(parse_ip_text("203.0.113.9\n"), Some("203.0.113.9".parse().unwrap()));
        assert_eq!(parse_ip_text("<html>203.0.113.9</html>"), None);
        assert_eq!(parse_ip_text(""), None);
        assert_eq!(parse_ip_text("1.2.3.4 and more"), None);
    }
}
