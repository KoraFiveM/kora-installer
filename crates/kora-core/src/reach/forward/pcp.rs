//! PCP (RFC 6887), the MAP opcode only: the successor of NAT-PMP on the same port. Replies are
//! checked against the request's nonce, protocol and port.

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use super::natpmp::exchange;
use super::{Discover, Granted, MapError, PortMapper, Proto, Wanted};

pub struct Pcp {
    gateway: Ipv4Addr,
    port: u16,
    timeout: Duration,
    /// The address mapped to (also the "client" in every request).
    client: Option<Ipv4Addr>,
    external: Option<Ipv4Addr>,
}

fn v4_mapped(ip: Ipv4Addr) -> [u8; 16] {
    ip.to_ipv6_mapped().octets()
}

fn proto_number(p: Proto) -> u8 {
    match p {
        Proto::Tcp => 6,
        Proto::Udp => 17,
    }
}

pub fn map_request(client: Ipv4Addr, proto: Proto, port: u16, lifetime: u32, nonce: [u8; 12]) -> [u8; 60] {
    let mut b = [0u8; 60];
    b[0] = 2; // version
    b[1] = 1; // request, MAP
    b[4..8].copy_from_slice(&lifetime.to_be_bytes());
    b[8..24].copy_from_slice(&v4_mapped(client));
    b[24..36].copy_from_slice(&nonce);
    b[36] = proto_number(proto);
    b[40..42].copy_from_slice(&port.to_be_bytes());
    b[42..44].copy_from_slice(&port.to_be_bytes());
    // suggested external address: any (all zero)
    b
}

pub fn result_text(code: u8) -> &'static str {
    match code {
        2 => "the router does not allow this program to forward ports",
        3 => "the router's network is not working yet",
        4 => "the router is not configured for this",
        8 => "the router is out of room for more forwarded ports",
        11 => "the router does not support that protocol",
        12 => "the router limits how many ports one computer may forward",
        13 => "the router refused the address",
        14 | 15 => "the router cannot give this outside port",
        _ => "the router refused",
    }
}

/// `(external address, external port, lifetime)` of a MAP reply for `nonce`.
pub fn parse_map_reply(b: &[u8], proto: Proto, port: u16, nonce: [u8; 12]) -> Result<(Ipv4Addr, u16, u32), MapError> {
    if b.len() != 60 || b[0] != 2 || b[1] != 0x81 {
        return Err(MapError::Protocol("unexpected PCP reply".into()));
    }
    if b[3] != 0 {
        return Err(MapError::Refused(result_text(b[3]).into()));
    }
    if b[24..36] != nonce || b[36] != proto_number(proto) || u16::from_be_bytes([b[40], b[41]]) != port {
        return Err(MapError::Protocol("the reply is for another request".into()));
    }
    let lifetime = u32::from_be_bytes([b[4], b[5], b[6], b[7]]);
    let ext_port = u16::from_be_bytes([b[42], b[43]]);
    let mut o = [0u8; 16];
    o.copy_from_slice(&b[44..60]);
    let ext = Ipv6Addr::from(o).to_ipv4_mapped().ok_or_else(|| MapError::Protocol("the outside address is not an IPv4 address".into()))?;
    Ok((ext, ext_port, lifetime))
}

fn nonce() -> [u8; 12] {
    let mut n = [0u8; 12];
    let _ = getrandom::fill(&mut n);
    n
}

/// This computer's address towards `gateway` (no packet is sent).
fn client_towards(gateway: Ipv4Addr, port: u16) -> Option<Ipv4Addr> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect(SocketAddr::from((gateway, port))).ok()?;
    match s.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(v4) => Some(v4),
        _ => None,
    }
}

/// Does `gateway` speak PCP? A MAP request for a throw-away port with lifetime 0 (a removal of
/// nothing) is answered by every PCP server and changes nothing.
pub fn probe(gateway: Ipv4Addr, d: &Discover) -> Option<Pcp> {
    if !d.allow_loopback && !crate::reach::addr::is_local_network(gateway.into()) {
        return None;
    }
    let client = client_towards(gateway, d.pmp_port)?;
    let n = nonce();
    let req = map_request(client, Proto::Udp, 9, 0, n);
    let reply = exchange(SocketAddr::from((gateway, d.pmp_port)), &req, d.timeout.min(Duration::from_millis(1200)), 1100).ok()?;
    // A NAT-PMP-only router answers a version-2 request with an "unsupported version" error of its own.
    if reply.len() == 60 && reply[0] == 2 && reply[1] == 0x81 && reply[3] != 1 {
        return Some(Pcp { gateway, port: d.pmp_port, timeout: d.timeout, client: Some(client), external: None });
    }
    None
}

impl PortMapper for Pcp {
    fn name(&self) -> &'static str {
        "PCP"
    }
    fn gateway(&self) -> Ipv4Addr {
        self.gateway
    }
    fn external_ip(&mut self) -> Result<Ipv4Addr, MapError> {
        // PCP reports the address together with a mapping; one is only learnt by making one.
        self.external.ok_or(MapError::NotFound)
    }
    fn add(&mut self, w: &Wanted, proto: Proto, lease: u32) -> Result<Granted, MapError> {
        let client = self.client.unwrap_or(w.client);
        let n = nonce();
        let r = exchange(SocketAddr::from((self.gateway, self.port)), &map_request(client, proto, w.port(), lease, n), self.timeout, 1100)?;
        let (ext_ip, ext_port, life) = parse_map_reply(&r, proto, w.port(), n)?;
        self.external = Some(ext_ip);
        if ext_port != w.port() {
            let n2 = nonce();
            let _ = exchange(SocketAddr::from((self.gateway, self.port)), &map_request(client, proto, w.port(), 0, n2), self.timeout, 1100);
            return Err(MapError::Refused(format!("the router offered outside port {ext_port} instead of {}, which would not work for players", w.port())));
        }
        Ok(Granted { external_port: ext_port, lease: life })
    }
    fn remove(&mut self, w: &Wanted, proto: Proto) -> Result<(), MapError> {
        let client = self.client.unwrap_or(w.client);
        let n = nonce();
        let r = exchange(SocketAddr::from((self.gateway, self.port)), &map_request(client, proto, w.port(), 0, n), self.timeout, 1100)?;
        parse_map_reply(&r, proto, w.port(), n).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::UdpSocket;
    use std::sync::{Arc, Mutex};

    /// A PCP router on loopback: records `(protocol, port, lifetime)` mappings.
    type Maps = Arc<Mutex<Vec<(u8, u16, u32)>>>;
    fn fake() -> (u16, Maps) {
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = sock.local_addr().unwrap().port();
        let st: Maps = Arc::default();
        let s2 = st.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 128];
            while let Ok((n, from)) = sock.recv_from(&mut buf) {
                if n != 60 || buf[0] != 2 {
                    continue;
                }
                let life = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
                let proto = buf[36];
                let internal = u16::from_be_bytes([buf[40], buf[41]]);
                {
                    let mut m = s2.lock().unwrap();
                    m.retain(|x| !(x.0 == proto && x.1 == internal));
                    if life > 0 {
                        m.push((proto, internal, life));
                    }
                }
                let mut r = buf[..60].to_vec();
                r[1] = 0x81;
                r[3] = 0;
                r[42..44].copy_from_slice(&internal.to_be_bytes());
                r[44..60].copy_from_slice(&Ipv4Addr::new(203, 0, 113, 9).to_ipv6_mapped().octets());
                let _ = sock.send_to(&r, from);
            }
        });
        (port, st)
    }

    #[test]
    fn request_layout() {
        let r = map_request("192.168.1.5".parse().unwrap(), Proto::Tcp, 30120, 3600, [7; 12]);
        assert_eq!((r[0], r[1]), (2, 1));
        assert_eq!(&r[4..8], &3600u32.to_be_bytes());
        assert_eq!(&r[20..24], &[192, 168, 1, 5]);
        assert_eq!(&r[24..36], &[7; 12]);
        assert_eq!(r[36], 6);
        assert_eq!(&r[40..42], &30120u16.to_be_bytes());
    }

    #[test]
    fn replies_for_other_requests_are_refused() {
        let req = map_request("192.168.1.5".parse().unwrap(), Proto::Tcp, 30120, 3600, [7; 12]);
        let mut rep = req.to_vec();
        rep[1] = 0x81;
        rep[44..60].copy_from_slice(&Ipv4Addr::new(1, 2, 3, 4).to_ipv6_mapped().octets());
        assert!(parse_map_reply(&rep, Proto::Tcp, 30120, [7; 12]).is_ok());
        assert!(parse_map_reply(&rep, Proto::Tcp, 30120, [8; 12]).is_err());
        assert!(parse_map_reply(&rep, Proto::Udp, 30120, [7; 12]).is_err());
        assert!(parse_map_reply(&rep[..59], Proto::Tcp, 30120, [7; 12]).is_err());
        rep[3] = 2;
        assert!(matches!(parse_map_reply(&rep, Proto::Tcp, 30120, [7; 12]), Err(MapError::Refused(_))));
    }

    #[test]
    fn maps_and_removes_against_a_fake_router() {
        let (port, st) = fake();
        let d = Discover { pmp_port: port, timeout: Duration::from_millis(800), allow_loopback: true, ..Discover::default() };
        let mut p = probe(Ipv4Addr::LOCALHOST, &d).expect("answers");
        // The probe itself left nothing behind.
        assert!(st.lock().unwrap().is_empty());
        let w = Wanted::new(30120, "192.168.1.50".parse().unwrap(), "main").unwrap();
        for pr in [Proto::Tcp, Proto::Udp] {
            assert_eq!(p.add(&w, pr, 3600).unwrap(), Granted { external_port: 30120, lease: 3600 });
        }
        assert_eq!(st.lock().unwrap().len(), 2);
        assert_eq!(p.external_ip().unwrap(), "203.0.113.9".parse::<Ipv4Addr>().unwrap());
        p.remove(&w, Proto::Tcp).unwrap();
        p.remove(&w, Proto::Udp).unwrap();
        assert!(st.lock().unwrap().is_empty());
    }
}
