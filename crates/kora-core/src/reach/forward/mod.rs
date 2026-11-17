//! Asking the home router to forward the game port (UPnP IGD, NAT-PMP, PCP).
//!
//! # Threat model
//!
//! A router that a program can reconfigure is a risk: anything on the local network that can talk
//! to the router can do the same, and a mapping stays open to the whole internet. So:
//!
//! * It is **off** unless the person turned it on for one server.
//! * Only that server's own game port is ever mapped, TCP and UDP, to this computer's own local
//!   address and the same port number. txAdmin's port, the panel's port and every other port are
//!   not accepted by [`Wanted::new`] (it takes the game port only, and refuses privileged ports).
//! * Everything the router (or whoever answers for it) sends back is untrusted: replies are parsed
//!   strictly with size limits; the router's description document must come from the very address
//!   that answered the discovery; control URLs on any other host are refused (no request forgery
//!   towards other machines); only private/link-local/shared-space addresses are talked to.
//! * Every mapping has a lease (the router forgets it by itself), is renewed while the server runs
//!   ([`keeper`]), and is removed when the server stops.
//! * Nothing goes to the internet from here: every packet stays on the local network.

pub mod gateway;
pub mod keeper;
pub mod natpmp;
pub mod pcp;
pub mod upnp;

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

/// Seconds a mapping lives before the router drops it; renewed at half of it.
pub const LEASE_SECS: u32 = 3600;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Proto {
    Tcp,
    Udp,
}

impl Proto {
    pub fn upper(self) -> &'static str {
        match self {
            Proto::Tcp => "TCP",
            Proto::Udp => "UDP",
        }
    }
}

/// Why asking the router did not work, in words for a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapError {
    /// No router answered: it does not offer this, or it is switched off.
    NotFound,
    /// The router answered and said no (the text says why, as far as it told).
    Refused(String),
    /// The router did not answer in time.
    Timeout,
    /// The router answered something that does not follow the protocol.
    Protocol(String),
    /// Our own network call failed.
    Io(String),
}

impl std::fmt::Display for MapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MapError::NotFound => f.write_str("No router answered. It may not support automatic port forwarding (UPnP, NAT-PMP) or it is turned off in the router's settings."),
            MapError::Refused(m) => write!(f, "The router refused: {m}"),
            MapError::Timeout => f.write_str("The router did not answer in time."),
            MapError::Protocol(m) => write!(f, "The router sent an answer Kora could not understand ({m})."),
            MapError::Io(m) => write!(f, "Could not talk to the router: {m}"),
        }
    }
}

impl std::error::Error for MapError {}

/// What one mapping request asks for. The only way to make one is [`Wanted::new`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted {
    port: u16,
    pub client: Ipv4Addr,
    pub description: String,
}

impl Wanted {
    /// `port`: the server's game port. `client`: this computer's address on the network.
    pub fn new(port: u16, client: Ipv4Addr, server: &str) -> Result<Self, MapError> {
        if port < 1024 {
            return Err(MapError::Refused(format!("port {port} is in the range Kora never forwards (use 1024 or higher)")));
        }
        if !super::addr::is_local_network(client.into()) || client.is_loopback() {
            return Err(MapError::Refused("this computer's network address could not be found".into()));
        }
        let clean: String = server.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ' ')).take(32).collect();
        Ok(Self { port, client, description: format!("Kora {clean} game") })
    }
    pub fn port(&self) -> u16 {
        self.port
    }
}

/// What the router granted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Granted {
    pub external_port: u16,
    /// Seconds the router will keep it; 0 for "until removed" (a router that only does permanent rules).
    pub lease: u32,
}

/// One way of reaching the router.
pub trait PortMapper: Send {
    /// `"UPnP"`, `"NAT-PMP"`, `"PCP"`.
    fn name(&self) -> &'static str;
    /// The router's own address on the local network.
    fn gateway(&self) -> Ipv4Addr;
    /// The router's address on the internet side, as the router itself reports it.
    fn external_ip(&mut self) -> Result<Ipv4Addr, MapError>;
    /// Forward `proto` `wanted.port()` (same number outside and inside) to `wanted.client`.
    fn add(&mut self, wanted: &Wanted, proto: Proto, lease: u32) -> Result<Granted, MapError>;
    /// Remove it again. A mapping that is not there is not an error.
    fn remove(&mut self, wanted: &Wanted, proto: Proto) -> Result<(), MapError>;
}

/// How to look for a router; the defaults are the real network.
#[derive(Debug, Clone)]
pub struct Discover {
    /// Where the UPnP search goes (`239.255.255.250:1900`).
    pub ssdp: SocketAddr,
    /// The router's address for NAT-PMP / PCP; `None` asks the operating system for the default gateway.
    pub gateway: Option<Ipv4Addr>,
    /// UDP port of NAT-PMP / PCP (5351).
    pub pmp_port: u16,
    pub timeout: Duration,
    /// Tests talk to fake routers on this computer.
    pub allow_loopback: bool,
}

impl Default for Discover {
    fn default() -> Self {
        Self { ssdp: SocketAddr::from(([239, 255, 255, 250], 1900)), gateway: None, pmp_port: 5351, timeout: Duration::from_millis(2500), allow_loopback: false }
    }
}

/// Every router protocol that answers, best first (UPnP, then PCP, then NAT-PMP).
pub fn discover(d: &Discover) -> Vec<Box<dyn PortMapper>> {
    let mut out: Vec<Box<dyn PortMapper>> = Vec::new();
    for u in upnp::discover(d) {
        out.push(Box::new(u));
    }
    let gw = d.gateway.or_else(gateway::default_gateway_v4).or_else(|| out.first().map(|m| m.gateway()));
    if let Some(gw) = gw {
        if let Some(p) = pcp::probe(gw, d) {
            out.push(Box::new(p));
        }
        if let Some(n) = natpmp::probe(gw, d) {
            out.push(Box::new(n));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_game_port_on_this_network_can_be_asked_for() {
        assert!(Wanted::new(30120, "192.168.1.5".parse().unwrap(), "main").is_ok());
        assert!(matches!(Wanted::new(80, "192.168.1.5".parse().unwrap(), "main"), Err(MapError::Refused(_))));
        assert!(matches!(Wanted::new(22, "192.168.1.5".parse().unwrap(), "main"), Err(MapError::Refused(_))));
        // A public or loopback "client" would send somebody else's traffic somewhere else.
        assert!(Wanted::new(30120, "8.8.8.8".parse().unwrap(), "main").is_err());
        assert!(Wanted::new(30120, "127.0.0.1".parse().unwrap(), "main").is_err());
    }

    #[test]
    fn description_is_plain() {
        let w = Wanted::new(30120, "10.0.0.2".parse().unwrap(), "my <server>\"&").unwrap();
        assert_eq!(w.description, "Kora my server game");
    }
}
