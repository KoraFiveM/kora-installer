//! UPnP Internet Gateway Device: SSDP discovery, the device description, and the three SOAP actions
//! Kora needs (`GetExternalIPAddress`, `AddPortMapping`, `DeletePortMapping`). Written here rather
//! than pulled in as a dependency: the exchange is small, and every byte from the router is parsed
//! strictly (see the threat model in [`super`]).

use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use std::time::{Duration, Instant};

use super::{Discover, Granted, MapError, PortMapper, Proto, Wanted};
use crate::reach::addr::{classify, is_local_network, AddrClass};

const MAX_DATAGRAM: usize = 2048;
const MAX_REPLIES: usize = 16;
const MAX_DESCRIPTION: u64 = 96 * 1024;
const MAX_SOAP_REPLY: u64 = 32 * 1024;
const SEARCH_TARGETS: [&str; 2] = ["urn:schemas-upnp-org:device:InternetGatewayDevice:1", "urn:schemas-upnp-org:device:InternetGatewayDevice:2"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SsdpReply {
    pub from: Ipv4Addr,
    pub host: Ipv4Addr,
    pub port: u16,
    pub path: String,
}

/// One `M-SEARCH` answer: `HTTP/1.1 200 OK` with a `LOCATION: http://<ip>:<port>/<path>` that names the
/// very machine that sent the datagram. Anything else (another host, a name instead of an address,
/// a public address, userinfo, https) is dropped: it would make Kora send a request wherever an
/// attacker on the network says.
pub fn parse_ssdp_reply(text: &str, from: IpAddr, allow_loopback: bool) -> Option<SsdpReply> {
    let mut lines = text.split("\r\n");
    let first = lines.next()?;
    if !first.starts_with("HTTP/1.1 200") && !first.starts_with("HTTP/1.0 200") {
        return None;
    }
    let mut location = None;
    for l in lines {
        if let Some((k, v)) = l.split_once(':') {
            if k.trim().eq_ignore_ascii_case("location") {
                location = Some(v.trim().to_string());
            }
        }
    }
    let loc = location?;
    let rest = loc.strip_prefix("http://")?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    if authority.contains('@') || path.contains(['\r', '\n', ' ']) || path.len() > 512 {
        return None;
    }
    let (host_s, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().ok().filter(|p| *p != 0)?),
        None => (authority, 80),
    };
    let host: Ipv4Addr = host_s.parse().ok()?;
    let IpAddr::V4(from4) = from.to_canonical() else { return None };
    if host != from4 {
        return None;
    }
    let ok = if host.is_loopback() { allow_loopback } else { is_local_network(host.into()) };
    ok.then(|| SsdpReply { from: from4, host, port, path: path.to_string() })
}

fn msearch(target: &str) -> String {
    format!("M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\nST: {target}\r\n\r\n")
}

/// Ask the network for routers; distinct replies only.
pub fn ssdp_search(d: &Discover) -> Vec<SsdpReply> {
    let Ok(sock) = UdpSocket::bind("0.0.0.0:0") else { return Vec::new() };
    let _ = sock.set_multicast_ttl_v4(2);
    for t in SEARCH_TARGETS {
        let _ = sock.send_to(msearch(t).as_bytes(), d.ssdp);
    }
    let deadline = Instant::now() + d.timeout;
    let mut out: Vec<SsdpReply> = Vec::new();
    let mut buf = [0u8; MAX_DATAGRAM];
    while Instant::now() < deadline && out.len() < MAX_REPLIES {
        let left = deadline.saturating_duration_since(Instant::now()).max(Duration::from_millis(10));
        let _ = sock.set_read_timeout(Some(left));
        match sock.recv_from(&mut buf) {
            Ok((n, from)) => {
                if let Some(r) = std::str::from_utf8(&buf[..n]).ok().and_then(|t| parse_ssdp_reply(t, from.ip(), d.allow_loopback)) {
                    if !out.contains(&r) {
                        out.push(r);
                    }
                }
            }
            Err(_) => break,
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service {
    pub service_type: String,
    pub control_url: String,
}

fn text_of<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&format!("</{name}>"))? + start;
    Some(xml[start..end].trim())
}

/// The WAN connection services of a device description (`WANIPConnection` 1 or 2, `WANPPPConnection` 1),
/// with their control URLs as written.
pub fn parse_services(xml: &str) -> Vec<Service> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(i) = rest.find("<service>") {
        rest = &rest[i + "<service>".len()..];
        let Some(end) = rest.find("</service>") else { break };
        let block = &rest[..end];
        rest = &rest[end..];
        let (Some(t), Some(c)) = (text_of(block, "serviceType"), text_of(block, "controlURL")) else { continue };
        let wan = ["WANIPConnection:1", "WANIPConnection:2", "WANPPPConnection:1"].iter().any(|s| t.ends_with(s)) && t.starts_with("urn:schemas-upnp-org:service:");
        if wan && c.len() <= 512 && !c.contains(['\r', '\n', ' ']) {
            out.push(Service { service_type: t.to_string(), control_url: c.to_string() });
        }
    }
    // The newer service first.
    out.sort_by_key(|s| std::cmp::Reverse(s.service_type.ends_with(":2")));
    out
}

/// Where a control URL points: an absolute path on the router, or a full `http://` address of the
/// SAME router. `None` for anything else.
pub fn resolve_control(reply: &SsdpReply, control: &str) -> Option<String> {
    if control.starts_with('/') && !control.starts_with("//") {
        return Some(control.to_string());
    }
    if let Some(rest) = control.strip_prefix("http://") {
        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };
        let (h, p) = authority.rsplit_once(':').map_or((authority, Some(80)), |(h, p)| (h, p.parse::<u16>().ok()));
        if h.parse::<Ipv4Addr>().ok()? == reply.host && p? == reply.port {
            return Some(path.to_string());
        }
        return None;
    }
    if !control.contains(':') && !control.starts_with('/') {
        // relative to the description's folder
        let base = reply.path.rsplit_once('/').map_or("", |(b, _)| b);
        return Some(format!("{base}/{control}"));
    }
    None
}

fn escape(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '"' => "&quot;".to_string(),
            '\'' => "&apos;".to_string(),
            c => c.to_string(),
        })
        .collect()
}

pub fn soap_body(service: &str, action: &str, args: &[(&str, String)]) -> String {
    let mut inner = String::new();
    for (k, v) in args {
        inner.push_str(&format!("<{k}>{}</{k}>", escape(v)));
    }
    format!("<?xml version=\"1.0\"?>\r\n<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body><u:{action} xmlns:u=\"{service}\">{inner}</u:{action}></s:Body></s:Envelope>")
}

/// `errorCode` and `errorDescription` of a SOAP fault.
pub fn parse_fault(xml: &str) -> Option<(u32, String)> {
    let code = text_of(xml, "errorCode")?.parse().ok()?;
    let desc = text_of(xml, "errorDescription").unwrap_or("").chars().filter(|c| !c.is_control()).take(80).collect();
    Some((code, desc))
}

/// What a UPnP error code means to a person.
pub fn explain_fault(code: u32, desc: &str) -> String {
    match code {
        718 => "another device on your network already forwards that port (remove that rule in the router and try again)".into(),
        724 | 726 | 727 | 729 => "the router only allows a different kind of rule for this port".into(),
        725 => "the router only accepts rules that never expire".into(),
        606 | 501 | 401 => "the router does not let programs change its port forwarding (UPnP is switched off or read-only in its settings)".into(),
        402 | 600 => "the router did not understand the request".into(),
        _ if desc.is_empty() => format!("error {code}"),
        _ => format!("error {code}: {desc}"),
    }
}

pub struct Upnp {
    gateway: Ipv4Addr,
    port: u16,
    control_path: String,
    service: String,
    timeout: Duration,
}

impl Upnp {
    fn agent(&self) -> ureq::Agent {
        ureq::AgentBuilder::new().redirects(0).timeout(self.timeout).timeout_connect(self.timeout).user_agent("kora").build()
    }

    fn call(&self, action: &str, args: &[(&str, String)]) -> Result<String, MapError> {
        let url = format!("http://{}:{}{}", self.gateway, self.port, self.control_path);
        let body = soap_body(&self.service, action, args);
        let res = self.agent().post(&url).set("Content-Type", "text/xml; charset=\"utf-8\"").set("SOAPAction", &format!("\"{}#{action}\"", self.service)).send_string(&body);
        match res {
            Ok(r) => read_capped(r.into_reader(), MAX_SOAP_REPLY),
            Err(ureq::Error::Status(_, r)) => {
                let text = read_capped(r.into_reader(), MAX_SOAP_REPLY).unwrap_or_default();
                match parse_fault(&text) {
                    Some((code, desc)) => Err(MapError::Refused(explain_fault(code, &desc))),
                    None => Err(MapError::Protocol("an error without a reason".into())),
                }
            }
            Err(ureq::Error::Transport(t)) => Err(if matches!(t.kind(), ureq::ErrorKind::Io) && t.to_string().contains("timed out") { MapError::Timeout } else { MapError::Io(t.to_string()) }),
        }
    }

    fn add_once(&self, w: &Wanted, proto: Proto, lease: u32) -> Result<(), MapError> {
        self.call(
            "AddPortMapping",
            &[
                ("NewRemoteHost", String::new()),
                ("NewExternalPort", w.port().to_string()),
                ("NewProtocol", proto.upper().to_string()),
                ("NewInternalPort", w.port().to_string()),
                ("NewInternalClient", w.client.to_string()),
                ("NewEnabled", "1".into()),
                ("NewPortMappingDescription", w.description.clone()),
                ("NewLeaseDuration", lease.to_string()),
            ],
        )
        .map(|_| ())
    }
}

fn read_capped(r: impl Read, cap: u64) -> Result<String, MapError> {
    let mut s = String::new();
    r.take(cap).read_to_string(&mut s).map_err(|e| MapError::Io(e.to_string()))?;
    Ok(s)
}

impl PortMapper for Upnp {
    fn name(&self) -> &'static str {
        "UPnP"
    }
    fn gateway(&self) -> Ipv4Addr {
        self.gateway
    }
    fn external_ip(&mut self) -> Result<Ipv4Addr, MapError> {
        let xml = self.call("GetExternalIPAddress", &[])?;
        let t = text_of(&xml, "NewExternalIPAddress").ok_or_else(|| MapError::Protocol("no address in the answer".into()))?;
        t.parse::<Ipv4Addr>().map_err(|_| MapError::Protocol("the address is not valid".into()))
    }
    fn add(&mut self, w: &Wanted, proto: Proto, lease: u32) -> Result<Granted, MapError> {
        match self.add_once(w, proto, lease) {
            Ok(()) => Ok(Granted { external_port: w.port(), lease }),
            // 725: only permanent rules. Fine, they are removed by us when the server stops.
            Err(MapError::Refused(m)) if m.contains("never expire") && lease != 0 => {
                self.add_once(w, proto, 0)?;
                Ok(Granted { external_port: w.port(), lease: 0 })
            }
            Err(e) => Err(e),
        }
    }
    fn remove(&mut self, w: &Wanted, proto: Proto) -> Result<(), MapError> {
        let r = self.call("DeletePortMapping", &[("NewRemoteHost", String::new()), ("NewExternalPort", w.port().to_string()), ("NewProtocol", proto.upper().to_string())]);
        match r {
            // 714 NoSuchEntryInArray: already gone.
            Err(MapError::Refused(m)) if m.starts_with("error 714") || m.contains("714") => Ok(()),
            other => other.map(|_| ()),
        }
    }
}

/// Fetch and read the description of one answering router.
fn open(reply: &SsdpReply, d: &Discover) -> Option<Upnp> {
    let agent = ureq::AgentBuilder::new().redirects(0).timeout(d.timeout).timeout_connect(d.timeout).user_agent("kora").build();
    let url = format!("http://{}:{}{}", reply.host, reply.port, reply.path);
    let r = agent.get(&url).call().ok()?;
    let xml = read_capped(r.into_reader(), MAX_DESCRIPTION).ok()?;
    for s in parse_services(&xml) {
        if let Some(path) = resolve_control(reply, &s.control_url) {
            return Some(Upnp { gateway: reply.host, port: reply.port, control_path: path, service: s.service_type, timeout: d.timeout });
        }
    }
    None
}

pub fn discover(d: &Discover) -> Vec<Upnp> {
    let mut out: Vec<Upnp> = Vec::new();
    for r in ssdp_search(d) {
        if classify(IpAddr::V4(r.host)) == AddrClass::Public {
            continue;
        }
        if let Some(u) = open(&r, d) {
            if !out.iter().any(|o| o.gateway == u.gateway && o.port == u.port && o.control_path == u.control_path) {
                out.push(u);
            }
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod fake {
    //! A router on this computer: answers SSDP on one UDP socket, serves its description and SOAP on
    //! one TCP socket. Records what it was asked.
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener, UdpSocket};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    pub struct State {
        pub mappings: Vec<(String, u16, String, String, u32)>,
        pub external_ip: String,
        /// `Some(code)`: answer AddPortMapping with this fault.
        pub fault: Option<u32>,
        /// Reject non-zero leases with 725.
        pub permanent_only: bool,
        pub requests: Vec<String>,
        /// Absolute control URL pointing elsewhere.
        pub evil_control: bool,
    }

    pub struct FakeIgd {
        pub ssdp: SocketAddr,
        pub state: Arc<Mutex<State>>,
    }

    fn tag<'a>(body: &'a str, name: &str) -> &'a str {
        let open = format!("<{name}>");
        let s = body.find(&open).map(|i| i + open.len()).unwrap_or(0);
        let e = body[s..].find(&format!("</{name}>")).map(|i| i + s).unwrap_or(s);
        &body[s..e]
    }

    pub fn start() -> FakeIgd {
        let tcp = TcpListener::bind("127.0.0.1:0").unwrap();
        let http = tcp.local_addr().unwrap();
        let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
        let ssdp = udp.local_addr().unwrap();
        let state = Arc::new(Mutex::new(State { external_ip: "203.0.113.7".into(), ..Default::default() }));
        std::thread::spawn(move || {
            let mut buf = [0u8; 1024];
            while let Ok((_, from)) = udp.recv_from(&mut buf) {
                let reply = format!("HTTP/1.1 200 OK\r\nCACHE-CONTROL: max-age=100\r\nLOCATION: http://{http}/rootDesc.xml\r\nST: urn:schemas-upnp-org:device:InternetGatewayDevice:1\r\n\r\n");
                let _ = udp.send_to(reply.as_bytes(), from);
            }
        });
        let st = state.clone();
        std::thread::spawn(move || {
            for conn in tcp.incoming() {
                let Ok(mut c) = conn else { break };
                let st = st.clone();
                std::thread::spawn(move || {
                    let mut data = Vec::new();
                    let mut buf = [0u8; 2048];
                    let _ = c.set_read_timeout(Some(std::time::Duration::from_millis(500)));
                    loop {
                        let n = c.read(&mut buf).unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        data.extend_from_slice(&buf[..n]);
                        let t = String::from_utf8_lossy(&data).to_string();
                        if let Some(i) = t.find("\r\n\r\n") {
                            let len = t.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                            if data.len() >= i + 4 + len {
                                break;
                            }
                        }
                    }
                    let text = String::from_utf8_lossy(&data).to_string();
                    let first = text.lines().next().unwrap_or("").to_string();
                    let body = text.split_once("\r\n\r\n").map(|x| x.1).unwrap_or("").to_string();
                    let mut s = st.lock().unwrap();
                    s.requests.push(first.clone());
                    let (status, out) = if first.starts_with("GET /rootDesc.xml") {
                        let control = if s.evil_control { "http://10.9.9.9:80/ctl" } else { "/ctl/IPConn" };
                        (200, format!("<?xml version=\"1.0\"?><root><device><serviceList><service><serviceType>urn:schemas-upnp-org:service:WANIPConnection:1</serviceType><controlURL>{control}</controlURL></service></serviceList></device></root>"))
                    } else if first.starts_with("POST /ctl/IPConn") {
                        let action = text.lines().find_map(|l| l.strip_prefix("soapaction: ").or_else(|| l.strip_prefix("SOAPAction: "))).unwrap_or("").to_string();
                        if action.contains("GetExternalIPAddress") {
                            (
                                200,
                                format!(
                                    "<s:Envelope><s:Body><u:GetExternalIPAddressResponse><NewExternalIPAddress>{}</NewExternalIPAddress></u:GetExternalIPAddressResponse></s:Body></s:Envelope>",
                                    s.external_ip
                                ),
                            )
                        } else if action.contains("AddPortMapping") {
                            let lease: u32 = tag(&body, "NewLeaseDuration").parse().unwrap_or(0);
                            if let Some(code) = s.fault {
                                (500, format!("<s:Envelope><s:Body><s:Fault><detail><UPnPError><errorCode>{code}</errorCode><errorDescription>nope</errorDescription></UPnPError></detail></s:Fault></s:Body></s:Envelope>"))
                            } else if s.permanent_only && lease != 0 {
                                (500, "<s:Envelope><s:Body><s:Fault><detail><UPnPError><errorCode>725</errorCode><errorDescription>OnlyPermanentLeasesSupported</errorDescription></UPnPError></detail></s:Fault></s:Body></s:Envelope>".to_string())
                            } else {
                                let rec = (
                                    tag(&body, "NewProtocol").to_string(),
                                    tag(&body, "NewExternalPort").parse().unwrap_or(0),
                                    tag(&body, "NewInternalClient").to_string(),
                                    tag(&body, "NewPortMappingDescription").to_string(),
                                    lease,
                                );
                                s.mappings.retain(|m| !(m.0 == rec.0 && m.1 == rec.1));
                                s.mappings.push(rec);
                                (200, "<s:Envelope><s:Body><u:AddPortMappingResponse/></s:Body></s:Envelope>".to_string())
                            }
                        } else if action.contains("DeletePortMapping") {
                            let (p, port) = (tag(&body, "NewProtocol").to_string(), tag(&body, "NewExternalPort").parse::<u16>().unwrap_or(0));
                            let before = s.mappings.len();
                            s.mappings.retain(|m| !(m.0 == p && m.1 == port));
                            if s.mappings.len() == before {
                                (500, "<s:Envelope><s:Body><s:Fault><detail><UPnPError><errorCode>714</errorCode><errorDescription>NoSuchEntryInArray</errorDescription></UPnPError></detail></s:Fault></s:Body></s:Envelope>".to_string())
                            } else {
                                (200, "<s:Envelope><s:Body><u:DeletePortMappingResponse/></s:Body></s:Envelope>".to_string())
                            }
                        } else {
                            (500, String::new())
                        }
                    } else {
                        (404, String::new())
                    };
                    let reply = format!("HTTP/1.1 {status} X\r\nContent-Type: text/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}", out.len());
                    let _ = c.write_all(reply.as_bytes());
                });
            }
        });
        FakeIgd { ssdp, state }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(f: &fake::FakeIgd) -> Discover {
        Discover { ssdp: f.ssdp, timeout: Duration::from_millis(600), allow_loopback: true, ..Discover::default() }
    }
    fn wanted() -> Wanted {
        Wanted::new(30120, "192.168.1.50".parse().unwrap(), "main").unwrap()
    }

    #[test]
    fn ssdp_reply_must_name_the_sender() {
        let from: IpAddr = "192.168.1.1".parse().unwrap();
        let ok = "HTTP/1.1 200 OK\r\nLOCATION: http://192.168.1.1:5000/desc.xml\r\nST: x\r\n\r\n";
        assert_eq!(parse_ssdp_reply(ok, from, false).unwrap().port, 5000);
        for bad in [
            "HTTP/1.1 200 OK\r\nLOCATION: http://192.168.1.99:5000/desc.xml\r\n\r\n",
            "HTTP/1.1 200 OK\r\nLOCATION: http://8.8.8.8:80/desc.xml\r\n\r\n",
            "HTTP/1.1 200 OK\r\nLOCATION: https://192.168.1.1/desc.xml\r\n\r\n",
            "HTTP/1.1 200 OK\r\nLOCATION: http://evil@192.168.1.1/desc.xml\r\n\r\n",
            "HTTP/1.1 200 OK\r\nLOCATION: http://router.local/desc.xml\r\n\r\n",
            "HTTP/1.1 200 OK\r\nLOCATION: http://192.168.1.1:0/desc.xml\r\n\r\n",
            "HTTP/1.1 404 Not Found\r\nLOCATION: http://192.168.1.1/d\r\n\r\n",
            "NOTIFY * HTTP/1.1\r\n\r\n",
        ] {
            assert!(parse_ssdp_reply(bad, from, false).is_none(), "{bad}");
        }
        // Loopback only when asked (tests).
        let lo = "HTTP/1.1 200 OK\r\nLOCATION: http://127.0.0.1:5000/d\r\n\r\n";
        assert!(parse_ssdp_reply(lo, "127.0.0.1".parse().unwrap(), false).is_none());
        assert!(parse_ssdp_reply(lo, "127.0.0.1".parse().unwrap(), true).is_some());
    }

    #[test]
    fn control_urls_stay_on_the_router() {
        let r = SsdpReply { from: "192.168.1.1".parse().unwrap(), host: "192.168.1.1".parse().unwrap(), port: 5000, path: "/a/desc.xml".into() };
        assert_eq!(resolve_control(&r, "/ctl"), Some("/ctl".into()));
        assert_eq!(resolve_control(&r, "ctl"), Some("/a/ctl".into()));
        assert_eq!(resolve_control(&r, "http://192.168.1.1:5000/ctl"), Some("/ctl".into()));
        assert_eq!(resolve_control(&r, "http://192.168.1.2:5000/ctl"), None);
        assert_eq!(resolve_control(&r, "http://192.168.1.1:5001/ctl"), None);
        assert_eq!(resolve_control(&r, "//evil/ctl"), None);
        assert_eq!(resolve_control(&r, "https://192.168.1.1/ctl"), None);
    }

    #[test]
    fn description_services() {
        let xml = "<root><service><serviceType>urn:schemas-upnp-org:service:Layer3Forwarding:1</serviceType><controlURL>/x</controlURL></service><service><serviceType>urn:schemas-upnp-org:service:WANPPPConnection:1</serviceType><controlURL>/ppp</controlURL></service><service><serviceType>urn:schemas-upnp-org:service:WANIPConnection:2</serviceType><controlURL>/ip2</controlURL></service></root>";
        let s = parse_services(xml);
        assert_eq!(s.iter().map(|s| s.control_url.as_str()).collect::<Vec<_>>(), ["/ip2", "/ppp"]);
        assert!(parse_services("<service><serviceType>urn:evil:WANIPConnection:1</serviceType><controlURL>/x</controlURL></service>").is_empty());
    }

    #[test]
    fn soap_escapes_values() {
        let b = soap_body("urn:s", "AddPortMapping", &[("NewPortMappingDescription", "a<b>&\"'".into())]);
        assert!(b.contains("a&lt;b&gt;&amp;&quot;&apos;"));
        assert!(!b.contains("a<b>"));
    }

    #[test]
    fn maps_and_removes_both_protocols_against_a_fake_router() {
        let f = fake::start();
        let mut found = discover(&d(&f));
        assert_eq!(found.len(), 1);
        let m = &mut found[0];
        assert_eq!(m.external_ip().unwrap(), "203.0.113.7".parse::<Ipv4Addr>().unwrap());
        let w = wanted();
        for p in [Proto::Tcp, Proto::Udp] {
            assert_eq!(m.add(&w, p, 3600).unwrap(), Granted { external_port: 30120, lease: 3600 });
        }
        {
            let s = f.state.lock().unwrap();
            assert_eq!(s.mappings.len(), 2);
            assert!(s.mappings.iter().all(|m| m.1 == 30120 && m.2 == "192.168.1.50" && m.3 == "Kora main game" && m.4 == 3600));
        }
        // Renewing is the same request and does not duplicate.
        m.add(&w, Proto::Tcp, 3600).unwrap();
        assert_eq!(f.state.lock().unwrap().mappings.len(), 2);
        m.remove(&w, Proto::Tcp).unwrap();
        m.remove(&w, Proto::Udp).unwrap();
        assert!(f.state.lock().unwrap().mappings.is_empty());
        // Removing what is not there is fine.
        m.remove(&w, Proto::Udp).unwrap();
    }

    #[test]
    fn a_router_that_only_does_permanent_rules_is_handled() {
        let f = fake::start();
        f.state.lock().unwrap().permanent_only = true;
        let mut found = discover(&d(&f));
        let g = found[0].add(&wanted(), Proto::Tcp, 3600).unwrap();
        assert_eq!(g.lease, 0);
    }

    #[test]
    fn a_refusal_is_explained() {
        let f = fake::start();
        f.state.lock().unwrap().fault = Some(718);
        let mut found = discover(&d(&f));
        let e = found[0].add(&wanted(), Proto::Tcp, 3600).unwrap_err();
        assert!(e.to_string().contains("already forwards that port"), "{e}");
        f.state.lock().unwrap().fault = Some(501);
        let e = found[0].add(&wanted(), Proto::Tcp, 3600).unwrap_err();
        assert!(e.to_string().contains("does not let programs"), "{e}");
    }

    #[test]
    fn a_description_that_points_elsewhere_is_ignored() {
        let f = fake::start();
        f.state.lock().unwrap().evil_control = true;
        assert!(discover(&d(&f)).is_empty());
        // and nothing was sent to the other address: only the description was fetched.
        assert!(f.state.lock().unwrap().requests.iter().all(|r| r.starts_with("GET /rootDesc.xml")));
    }

    #[test]
    fn nobody_answering_is_not_an_error_just_nothing() {
        let silent = UdpSocket::bind("127.0.0.1:0").unwrap();
        let d = Discover { ssdp: silent.local_addr().unwrap(), timeout: Duration::from_millis(200), allow_loopback: true, ..Discover::default() };
        assert!(discover(&d).is_empty());
    }
}
