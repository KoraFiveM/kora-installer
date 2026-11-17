//! NAT-PMP (RFC 6886): two tiny UDP exchanges with the router on port 5351. Replies are only
//! accepted from the router's own address and with exactly the expected length and opcode.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use super::{Discover, Granted, MapError, PortMapper, Proto, Wanted};

pub struct NatPmp {
    gateway: Ipv4Addr,
    port: u16,
    timeout: Duration,
}

pub fn external_address_request() -> [u8; 2] {
    [0, 0]
}

pub fn map_request(proto: Proto, internal: u16, external: u16, lifetime: u32) -> [u8; 12] {
    let mut b = [0u8; 12];
    b[1] = match proto {
        Proto::Udp => 1,
        Proto::Tcp => 2,
    };
    b[4..6].copy_from_slice(&internal.to_be_bytes());
    b[6..8].copy_from_slice(&external.to_be_bytes());
    b[8..12].copy_from_slice(&lifetime.to_be_bytes());
    b
}

pub fn result_text(code: u16) -> &'static str {
    match code {
        1 => "the router does not support this version of the protocol",
        2 => "the router has port forwarding switched off or refuses it",
        3 => "the router's network is not working yet",
        4 => "the router is out of room for more forwarded ports",
        5 => "the router does not support that protocol",
        _ => "the router gave an unknown error",
    }
}

/// `Ok(address)` of an external-address reply (12 bytes, opcode 128).
pub fn parse_external_address(b: &[u8]) -> Result<Ipv4Addr, MapError> {
    if b.len() != 12 || b[0] != 0 || b[1] != 128 {
        return Err(MapError::Protocol("unexpected NAT-PMP reply".into()));
    }
    let code = u16::from_be_bytes([b[2], b[3]]);
    if code != 0 {
        return Err(MapError::Refused(result_text(code).into()));
    }
    Ok(Ipv4Addr::new(b[8], b[9], b[10], b[11]))
}

/// `Ok((granted external port, lifetime))` of a mapping reply (16 bytes, opcode 129/130).
pub fn parse_map_reply(b: &[u8], proto: Proto, internal: u16) -> Result<(u16, u32), MapError> {
    let want = match proto {
        Proto::Udp => 129,
        Proto::Tcp => 130,
    };
    if b.len() != 16 || b[0] != 0 || b[1] != want {
        return Err(MapError::Protocol("unexpected NAT-PMP reply".into()));
    }
    let code = u16::from_be_bytes([b[2], b[3]]);
    if code != 0 {
        return Err(MapError::Refused(result_text(code).into()));
    }
    if u16::from_be_bytes([b[8], b[9]]) != internal {
        return Err(MapError::Protocol("the reply is for another port".into()));
    }
    Ok((u16::from_be_bytes([b[10], b[11]]), u32::from_be_bytes([b[12], b[13], b[14], b[15]])))
}

/// Send `req` to `to` and wait for a datagram from `to` (others are ignored), resending a few times.
pub(crate) fn exchange(to: SocketAddr, req: &[u8], total: Duration, max_len: usize) -> Result<Vec<u8>, MapError> {
    let sock = UdpSocket::bind("0.0.0.0:0").map_err(|e| MapError::Io(e.to_string()))?;
    let deadline = Instant::now() + total;
    let mut wait = Duration::from_millis(250);
    let mut buf = vec![0u8; max_len + 1];
    while Instant::now() < deadline {
        sock.send_to(req, to).map_err(|e| MapError::Io(e.to_string()))?;
        let until = (Instant::now() + wait).min(deadline);
        loop {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            let _ = sock.set_read_timeout(Some(left));
            match sock.recv_from(&mut buf) {
                Ok((n, from)) if from == to && n <= max_len => return Ok(buf[..n].to_vec()),
                Ok(_) => continue,
                Err(_) => break,
            }
        }
        wait *= 2;
    }
    Err(MapError::Timeout)
}

/// Does `gateway` speak NAT-PMP? (Asks for the external address.)
pub fn probe(gateway: Ipv4Addr, d: &Discover) -> Option<NatPmp> {
    if !d.allow_loopback && !crate::reach::addr::is_local_network(gateway.into()) {
        return None;
    }
    let n = NatPmp { gateway, port: d.pmp_port, timeout: d.timeout };
    let reply = exchange(SocketAddr::from((gateway, d.pmp_port)), &external_address_request(), d.timeout.min(Duration::from_millis(1200)), 64).ok()?;
    parse_external_address(&reply).ok().map(|_| n)
}

impl PortMapper for NatPmp {
    fn name(&self) -> &'static str {
        "NAT-PMP"
    }
    fn gateway(&self) -> Ipv4Addr {
        self.gateway
    }
    fn external_ip(&mut self) -> Result<Ipv4Addr, MapError> {
        let r = exchange(SocketAddr::from((self.gateway, self.port)), &external_address_request(), self.timeout, 64)?;
        parse_external_address(&r)
    }
    fn add(&mut self, w: &Wanted, proto: Proto, lease: u32) -> Result<Granted, MapError> {
        let r = exchange(SocketAddr::from((self.gateway, self.port)), &map_request(proto, w.port(), w.port(), lease), self.timeout, 64)?;
        let (ext, life) = parse_map_reply(&r, proto, w.port())?;
        if ext != w.port() {
            // The router gave a different outside port: players would need that number. Undo it.
            let _ = exchange(SocketAddr::from((self.gateway, self.port)), &map_request(proto, w.port(), 0, 0), self.timeout, 64);
            return Err(MapError::Refused(format!("the router offered outside port {ext} instead of {}, which would not work for players", w.port())));
        }
        Ok(Granted { external_port: ext, lease: life })
    }
    fn remove(&mut self, w: &Wanted, proto: Proto) -> Result<(), MapError> {
        // Lifetime 0 and external port 0 removes the mapping.
        let r = exchange(SocketAddr::from((self.gateway, self.port)), &map_request(proto, w.port(), 0, 0), self.timeout, 64)?;
        parse_map_reply(&r, proto, w.port()).map(|_| ())
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    pub struct State {
        pub mappings: Vec<(u8, u16, u32)>,
        pub result: u16,
        /// Give this outside port instead of the one asked for.
        pub offer_port: Option<u16>,
    }

    /// A NAT-PMP router on loopback; returns its port.
    pub fn start() -> (u16, Arc<Mutex<State>>) {
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = sock.local_addr().unwrap().port();
        let st = Arc::new(Mutex::new(State::default()));
        let s2 = st.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 64];
            while let Ok((n, from)) = sock.recv_from(&mut buf) {
                let mut s = s2.lock().unwrap();
                let code = s.result.to_be_bytes();
                if n == 2 {
                    let mut r = vec![0, 128, code[0], code[1], 0, 0, 0, 1];
                    r.extend_from_slice(&[203, 0, 113, 8]);
                    let _ = sock.send_to(&r, from);
                } else if n == 12 {
                    let op = buf[1];
                    let internal = u16::from_be_bytes([buf[4], buf[5]]);
                    let ext = u16::from_be_bytes([buf[6], buf[7]]);
                    let life = u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]);
                    if life == 0 {
                        s.mappings.retain(|m| !(m.0 == op && m.1 == internal));
                    } else if s.result == 0 {
                        s.mappings.retain(|m| !(m.0 == op && m.1 == internal));
                        s.mappings.push((op, internal, life));
                    }
                    let granted = s.offer_port.unwrap_or(if ext == 0 { internal } else { ext });
                    let mut r = vec![0, 128 + op, code[0], code[1], 0, 0, 0, 1];
                    r.extend_from_slice(&internal.to_be_bytes());
                    r.extend_from_slice(&granted.to_be_bytes());
                    r.extend_from_slice(&life.to_be_bytes());
                    let _ = sock.send_to(&r, from);
                }
            }
        });
        (port, st)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(port: u16) -> Discover {
        Discover { pmp_port: port, timeout: Duration::from_millis(800), allow_loopback: true, ..Discover::default() }
    }

    #[test]
    fn encodes_the_requests() {
        assert_eq!(map_request(Proto::Tcp, 30120, 30120, 3600), [0, 2, 0, 0, 0x75, 0xA8, 0x75, 0xA8, 0, 0, 0x0E, 0x10]);
        assert_eq!(map_request(Proto::Udp, 1, 2, 0)[1], 1);
    }

    #[test]
    fn replies_are_checked_strictly() {
        assert!(parse_external_address(&[0, 128, 0, 0, 0, 0, 0, 1, 1, 2, 3, 4]).is_ok());
        assert!(parse_external_address(&[0, 128, 0, 0, 0, 0, 0, 1, 1, 2, 3]).is_err());
        assert!(parse_external_address(&[1, 128, 0, 0, 0, 0, 0, 1, 1, 2, 3, 4]).is_err());
        assert!(matches!(parse_external_address(&[0, 128, 0, 2, 0, 0, 0, 1, 0, 0, 0, 0]), Err(MapError::Refused(_))));
        let mut m = vec![0, 130, 0, 0, 0, 0, 0, 1, 0x75, 0xA8, 0x75, 0xA8, 0, 0, 0x0E, 0x10];
        assert_eq!(parse_map_reply(&m, Proto::Tcp, 30120).unwrap(), (30120, 3600));
        assert!(parse_map_reply(&m, Proto::Udp, 30120).is_err());
        assert!(parse_map_reply(&m, Proto::Tcp, 30121).is_err());
        m.push(0);
        assert!(parse_map_reply(&m, Proto::Tcp, 30120).is_err());
    }

    #[test]
    fn maps_and_removes_against_a_fake_router() {
        let (port, st) = fake::start();
        let mut n = probe(Ipv4Addr::LOCALHOST, &d(port)).expect("router answers");
        assert_eq!(n.external_ip().unwrap(), "203.0.113.8".parse::<Ipv4Addr>().unwrap());
        let w = Wanted::new(30120, "192.168.1.50".parse().unwrap(), "main").unwrap();
        for p in [Proto::Tcp, Proto::Udp] {
            assert_eq!(n.add(&w, p, 3600).unwrap(), Granted { external_port: 30120, lease: 3600 });
        }
        assert_eq!(st.lock().unwrap().mappings.len(), 2);
        n.remove(&w, Proto::Tcp).unwrap();
        n.remove(&w, Proto::Udp).unwrap();
        assert!(st.lock().unwrap().mappings.is_empty());
    }

    #[test]
    fn a_different_outside_port_is_undone() {
        let (port, st) = fake::start();
        let mut n = probe(Ipv4Addr::LOCALHOST, &d(port)).unwrap();
        st.lock().unwrap().offer_port = Some(40000);
        let w = Wanted::new(30120, "192.168.1.50".parse().unwrap(), "main").unwrap();
        let e = n.add(&w, Proto::Tcp, 3600).unwrap_err();
        assert!(e.to_string().contains("40000"), "{e}");
        assert!(st.lock().unwrap().mappings.is_empty());
    }

    #[test]
    fn refusal_and_silence() {
        let (port, st) = fake::start();
        let mut n = probe(Ipv4Addr::LOCALHOST, &d(port)).unwrap();
        st.lock().unwrap().result = 2;
        let w = Wanted::new(30120, "192.168.1.50".parse().unwrap(), "main").unwrap();
        assert!(matches!(n.add(&w, Proto::Tcp, 3600), Err(MapError::Refused(_))));
        let silent = UdpSocket::bind("127.0.0.1:0").unwrap();
        let dd = Discover { pmp_port: silent.local_addr().unwrap().port(), timeout: Duration::from_millis(300), allow_loopback: true, ..Discover::default() };
        assert!(probe(Ipv4Addr::LOCALHOST, &dd).is_none());
    }
}
