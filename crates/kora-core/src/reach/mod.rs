//! "Can players reach my server?": a guided checklist, one line per layer.
//!
//! 1. **running**: the game server listens on its game port (TCP and UDP: FiveM needs both), and not
//!    only on this computer's own loopback address.
//! 2. **firewall**: this computer's firewall lets it in (what Kora itself opened is known; what it
//!    cannot see is said so).
//! 3. **router**: whether a router (NAT) sits in between, whether it is the kind that cannot forward
//!    at all (carrier-grade NAT, 100.64.0.0/10), and whether the port is forwarded to this computer.
//! 4. **internet**: whether the server is reachable from outside. Only an outside vantage point can
//!    say. See [`probe`] for the two optional lookups and why neither is a proof; without a sound
//!    answer the line says "unknown" and guides the manual test.
//!
//! Everything is gathered into [`Facts`] by a [`ReachEnv`] (so tests never touch a network) and turned
//! into the report by the pure function [`assess`]. [`forward`] is the optional automatic port
//! forwarding, [`settings`] what the person chose.

pub mod addr;
pub mod forward;
pub mod fw;
pub mod probe;
pub mod settings;

use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::installer::registry::ServerEntry;
use addr::{classify, AddrClass};
use probe::{Listing, Lookup};
use settings::{Consent, ForwardStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ReachState {
    Ok,
    /// Evidence that it works, short of proof.
    Likely,
    Problem,
    Unknown,
}

/// Short name used inside this module and by the CLI.
pub type State = ReachState;

/// A button the panel can show on a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum FixId {
    StartServer,
    OpenFirewall,
    EnableForwarding,
    AllowCfxList,
    AllowIpEcho,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Fix {
    pub id: FixId,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum LayerId {
    Running,
    Firewall,
    Router,
    Internet,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Layer {
    pub id: LayerId,
    pub state: State,
    pub title: String,
    pub detail: String,
    /// What to do by hand, one step per line, with the exact values to enter.
    pub steps: Vec<String>,
    pub fixes: Vec<Fix>,
}

/// The addresses to give people, each for who it works for.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Connect {
    /// `ip:port` on the same network.
    pub lan: Option<String>,
    /// `ip:port` from the internet, once the public address is known (and not behind carrier-grade NAT).
    pub public: Option<String>,
    /// `https://cfx.re/join/<code>` when the server has a join code.
    pub cfx: Option<String>,
    /// What to type in FiveM's console (F8) for the best address.
    pub command: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ForwardingView {
    pub enabled: bool,
    pub status: Option<ForwardStatus>,
}

/// What was (and was not) contacted outside this computer, for the "what did you do" line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Contacted {
    pub what: String,
    pub learned: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum PublicSource {
    Router,
    Cfx,
    IpEcho,
}

/// An outside lookup the person may allow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ConsentKind {
    CfxList,
    IpEcho,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ReachReport {
    pub server: String,
    pub game_port: u16,
    pub layers: Vec<Layer>,
    /// `ok`, `problem` or `unknown`: the worst of the layers, in one word.
    pub verdict: State,
    pub headline: String,
    pub lan_ip: Option<String>,
    pub public_ip: Option<String>,
    pub public_source: Option<PublicSource>,
    pub behind_nat: Option<bool>,
    pub cgnat: bool,
    pub join_code: Option<String>,
    pub connect: Connect,
    pub forwarding: ForwardingView,
    pub contacted: Vec<Contacted>,
    /// Outside lookups that would help but were not allowed: `cfx_list`, `ip_echo`.
    pub needs_consent: Vec<ConsentKind>,
    pub checked_unix: u64,
    /// The saved choices about outside lookups (the check itself may have used wider ones for one time).
    pub consent: Consent,
}

// ------------------------------------------------------------------ facts

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirewallFacts {
    /// `ufw`, `firewalld`, `Windows Firewall`; `None` when no active firewall was found.
    pub kind: Option<&'static str>,
    /// Kora opened this server's ports itself (its install record says so).
    pub opened_by_kora: bool,
    /// Could Kora have seen a firewall at all (administrator rights, or one was found)?
    pub readable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouterFacts {
    /// Protocols that answered: `UPnP`, `PCP`, `NAT-PMP`.
    pub protocols: Vec<&'static str>,
    /// The router's own report of its internet-side address.
    pub external_ip: Option<Ipv4Addr>,
}

#[derive(Debug, Clone)]
pub struct Facts {
    pub server: String,
    pub game_port: u16,
    pub running: bool,
    pub tcp_local: bool,
    pub tcp_lan: Option<bool>,
    pub udp_held: bool,
    pub lan: Option<IpAddr>,
    pub firewall: FirewallFacts,
    pub router: RouterFacts,
    pub forwarding: ForwardingView,
    pub join_code: Option<String>,
    pub consent: Consent,
    /// `None`: not asked (no consent, or no join code).
    pub cfx: Option<Lookup<Listing>>,
    pub echo: Option<Lookup<IpAddr>>,
    pub now: u64,
}

/// Everything [`gather`] needs from the machine.
pub trait ReachEnv {
    fn running(&self, e: &ServerEntry) -> bool;
    fn tcp_connect(&self, ip: IpAddr, port: u16) -> bool;
    /// Does a program hold UDP `port` on this computer?
    fn udp_held(&self, port: u16) -> bool;
    fn lan_ip(&self) -> Option<IpAddr>;
    fn firewall(&self, e: &ServerEntry) -> FirewallFacts;
    fn router(&self) -> RouterFacts;
    /// FXServer's recent output (for the join code).
    fn log_text(&self, e: &ServerEntry) -> String;
    fn saved_join_code(&self, e: &ServerEntry) -> Option<String>;
    fn save_join_code(&self, e: &ServerEntry, code: &str);
    fn forwarding(&self, e: &ServerEntry) -> ForwardingView;
    fn consent(&self) -> Consent;
    fn cfx(&self) -> &dyn probe::CfxList;
    fn echo(&self) -> &dyn probe::IpEcho;
    fn now(&self) -> u64;
}

/// Look at everything. The outside lookups run only when the person allowed them.
pub fn gather(env: &dyn ReachEnv, e: &ServerEntry, consent_override: Option<Consent>) -> Facts {
    let port = e.ports.game;
    let consent = consent_override.unwrap_or_else(|| env.consent());
    let lan = env.lan_ip();
    let router = env.router();
    let running = env.running(e);
    let tcp_local = env.tcp_connect(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let tcp_lan = lan.filter(|_| tcp_local).map(|ip| env.tcp_connect(ip, port));
    let udp_held = env.udp_held(port);
    let join_code = probe::join_code_from_log(&env.log_text(e)).or_else(|| env.saved_join_code(e));
    if let Some(c) = &join_code {
        if env.saved_join_code(e).as_deref() != Some(c) {
            env.save_join_code(e, c);
        }
    }
    let cfx = match (&join_code, consent.cfx_list) {
        (Some(c), true) => Some(env.cfx().listing(c)),
        _ => None,
    };
    let known = known_public(router.external_ip, cfx.as_ref());
    let echo = (known.is_none() && consent.ip_echo).then(|| env.echo().public_ip());
    Facts {
        server: e.name.clone(),
        game_port: port,
        running,
        tcp_local,
        tcp_lan,
        udp_held,
        lan,
        firewall: env.firewall(e),
        router,
        forwarding: env.forwarding(e),
        join_code,
        consent,
        cfx,
        echo,
        now: env.now(),
    }
}

/// The public address and where it came from, best source first.
fn known_public(router_ext: Option<Ipv4Addr>, cfx: Option<&Lookup<Listing>>) -> Option<(IpAddr, PublicSource)> {
    if let Some(ip) = router_ext.filter(|ip| classify(IpAddr::V4(*ip)) == AddrClass::Public) {
        return Some((IpAddr::V4(ip), PublicSource::Router));
    }
    if let Some(Lookup::Found(l)) = cfx {
        if let Some(ip) = l.endpoints.iter().filter_map(|e| e.parse::<SocketAddr>().ok()).map(|s| s.ip()).find(|ip| classify(*ip) == AddrClass::Public) {
            return Some((ip, PublicSource::Cfx));
        }
    }
    None
}

// ------------------------------------------------------------------ the checklist

fn layer(id: LayerId, state: State, title: impl Into<String>, detail: impl Into<String>) -> Layer {
    Layer { id, state, title: title.into(), detail: detail.into(), steps: Vec::new(), fixes: Vec::new() }
}

fn running_layer(f: &Facts) -> Layer {
    let p = f.game_port;
    let mut l = if !f.running {
        let mut l = layer(LayerId::Running, ReachState::Problem, "The server is not running", "Nobody can join a server that is switched off.");
        l.fixes.push(Fix { id: FixId::StartServer, label: "Start the server".into() });
        l
    } else if !f.tcp_local {
        layer(
            LayerId::Running,
            ReachState::Problem,
            format!("The server is not answering on port {p} yet"),
            "It may still be starting (give it a minute and check again), or it could not use this port: look at the server's log for \"port is busy\" or a licence message.",
        )
    } else if !f.udp_held {
        layer(
            LayerId::Running,
            ReachState::Problem,
            format!("Nothing is listening on UDP port {p}"),
            "FiveM needs both TCP and UDP on the game port. The web part answers but the game part does not: check the server's log, and that the server.cfg lines \"endpoint_add_udp\" and \"endpoint_add_tcp\" use the same port.",
        )
    } else if f.tcp_lan == Some(false) {
        let mut l = layer(
            LayerId::Running,
            ReachState::Problem,
            "The server only listens on this computer itself",
            format!("It answers on 127.0.0.1 but not on this computer's network address, so no other device can join. In server.cfg the endpoint_add_tcp and endpoint_add_udp lines should say \"0.0.0.0:{p}\"."),
        );
        l.steps.push(format!("Open server.cfg and set: endpoint_add_tcp \"0.0.0.0:{p}\" and endpoint_add_udp \"0.0.0.0:{p}\""));
        l.steps.push("Restart the server.".into());
        l
    } else {
        layer(LayerId::Running, ReachState::Ok, format!("The server is running and listening on port {p} (TCP and UDP)"), "")
    };
    if l.detail.is_empty() {
        l.detail = "This computer accepts connections on the game port.".into();
    }
    l
}

fn firewall_layer(f: &Facts) -> Layer {
    let p = f.game_port;
    let fw = &f.firewall;
    let open_fix = Fix { id: FixId::OpenFirewall, label: format!("Open port {p} in the firewall") };
    match (fw.kind, fw.opened_by_kora) {
        (Some(k), true) => layer(LayerId::Firewall, ReachState::Ok, format!("The firewall ({k}) lets port {p} in"), "Kora opened this server's port when it was installed."),
        (Some(k), false) if k != "Windows Firewall" => {
            let mut l = layer(
                LayerId::Firewall,
                ReachState::Problem,
                format!("The firewall ({k}) is on and Kora did not open port {p}"),
                format!("Unless you opened port {p} yourself (TCP and UDP), players are blocked here."),
            );
            l.fixes.push(open_fix);
            l.steps.push(if k == "ufw" {
                format!("sudo ufw allow {p}/tcp && sudo ufw allow {p}/udp")
            } else {
                format!("sudo firewall-cmd --permanent --add-port={p}/tcp --add-port={p}/udp && sudo firewall-cmd --reload")
            });
            l
        }
        (Some(_), false) => {
            // Windows Firewall also asks the first time a program listens, so a rule may exist that Kora never made.
            let mut l = layer(
                LayerId::Firewall,
                ReachState::Unknown,
                format!("Windows Firewall: Kora did not open port {p}"),
                "Windows may have asked you when the server first started and you may have allowed it. Kora cannot read your other firewall rules. If friends cannot join, open the port here.",
            );
            l.fixes.push(open_fix);
            l
        }
        (None, _) if fw.readable => layer(LayerId::Firewall, ReachState::Ok, "No firewall is blocking this computer", "No active firewall (ufw, firewalld, Windows Firewall) was found."),
        (None, _) => {
            let mut l = layer(
                LayerId::Firewall,
                ReachState::Unknown,
                "Kora cannot see this computer's firewall",
                "Reading the firewall needs administrator rights. If you use one (ufw, firewalld, or your hosting provider's), port 30120-style game ports need TCP and UDP allowed."
                    .replace("30120-style game ports", &format!("port {p}")),
            );
            l.fixes.push(open_fix);
            l
        }
    }
}

/// The router-side explanation of carrier-grade NAT, shared by two lines.
fn cgnat_help(lan: Option<&str>, port: u16) -> Vec<String> {
    let _ = (lan, port);
    vec![
        "Your internet provider shares one public address between many customers (carrier-grade NAT). Port forwarding on your own router cannot work behind it.".into(),
        "Ask your provider for a public IPv4 address (many do this free or for a small fee: say you host a game server), or".into(),
        "Run the server at a hosting provider or a cheap VPS instead, or".into(),
        "Let friends join over a private network such as Tailscale or ZeroTier (everyone installs it, then they connect to the server's private address).".into(),
    ]
}

fn nat_view(f: &Facts) -> (Option<bool>, bool, Option<(IpAddr, PublicSource)>) {
    let mut public = known_public(f.router.external_ip, f.cfx.as_ref());
    if public.is_none() {
        if let Some(Lookup::Found(ip)) = &f.echo {
            if classify(*ip) == AddrClass::Public {
                public = Some((*ip, PublicSource::IpEcho));
            }
        }
    }
    // The router says its own internet-side address is a shared (100.64/10) or private one: another NAT in front.
    let router_wan_class = f.router.external_ip.map(|ip| classify(IpAddr::V4(ip)));
    let cgnat = matches!(router_wan_class, Some(AddrClass::Cgnat | AddrClass::Private));
    let lan_class = f.lan.map(classify);
    let behind = match (lan_class, public) {
        (_, _) if cgnat => Some(true),
        (Some(AddrClass::Public), Some((ip, _))) => Some(Some(ip) != f.lan),
        (Some(AddrClass::Public), None) => Some(false),
        (Some(AddrClass::Private | AddrClass::Cgnat), Some((ip, _))) => Some(Some(ip) != f.lan),
        (Some(AddrClass::Private | AddrClass::Cgnat), None) => Some(true),
        _ => None,
    };
    (behind, cgnat, public)
}

fn forward_steps(lan: Option<&str>, port: u16) -> Vec<String> {
    let lan = lan.unwrap_or("this computer's network address");
    vec![
        "Open your router's settings page (often http://192.168.0.1 or http://192.168.1.1; the address and password are usually on a sticker on the router).".into(),
        "Find \"Port forwarding\" (sometimes \"Virtual server\", \"NAT\" or \"Applications\").".into(),
        format!("Add a rule: protocol TCP and UDP (both; if there is only one choice make two rules), outside/external port {port}, inside/internal port {port}, destination address {lan}."),
        "Save, then press \"Check again\" here.".into(),
        "Tip: give this computer a fixed address in the router (\"DHCP reservation\"), or the rule stops working when its address changes.".into(),
    ]
}

fn router_layer(f: &Facts, behind: Option<bool>, cgnat: bool, public: Option<(IpAddr, PublicSource)>) -> Layer {
    let p = f.game_port;
    let lan = f.lan.map(|i| i.to_string());
    if cgnat {
        let mut l = layer(
            LayerId::Router,
            ReachState::Problem,
            "Your internet provider shares your public address",
            "Players on the internet cannot reach a server behind carrier-grade NAT, whatever you set in the router.",
        );
        l.steps = cgnat_help(lan.as_deref(), p);
        return l;
    }
    match behind {
        Some(false) => layer(
            LayerId::Router,
            ReachState::Ok,
            "No router in the way",
            format!("This computer has a public address ({}) itself, so nothing needs forwarding.", f.lan.map(|i| i.to_string()).unwrap_or_default()),
        ),
        Some(true) => {
            let found = !f.router.protocols.is_empty();
            let via = public.map(|(ip, _)| format!(" Your public address is {ip}.")).unwrap_or_default();
            match (&f.forwarding.status, f.forwarding.enabled) {
                (Some(s), true) if s.ok => layer(LayerId::Router, ReachState::Ok, format!("Port {p} is forwarded by your router"), format!("{}{via}", s.message)),
                (Some(s), true) => {
                    let mut l = layer(LayerId::Router, ReachState::Problem, "Automatic port forwarding did not work", format!("{}{via}", s.message));
                    l.steps = forward_steps(lan.as_deref(), p);
                    l
                }
                (None, true) => layer(LayerId::Router, ReachState::Unknown, "Automatic port forwarding is on, waiting for the server", "It is set up while the server is running."),
                (_, false) => {
                    let mut l = layer(
                        LayerId::Router,
                        ReachState::Unknown,
                        format!("Your router must forward port {p}"),
                        format!("This computer is behind a router, so players on the internet only get in if the router sends port {p} (TCP and UDP) here. Kora cannot see the router's rules.{via}"),
                    );
                    if found {
                        l.fixes.push(Fix { id: FixId::EnableForwarding, label: format!("Forward port {p} for me ({})", f.router.protocols.join(" / ")) });
                    }
                    l.steps = forward_steps(lan.as_deref(), p);
                    l
                }
            }
        }
        None => layer(LayerId::Router, ReachState::Unknown, "Kora could not find this computer's network address", "Without it the router cannot be checked. Connect to your network and check again."),
    }
}

fn internet_layer(f: &Facts, cgnat: bool, public: Option<(IpAddr, PublicSource)>, connect: &Connect) -> (Layer, Vec<ConsentKind>) {
    let p = f.game_port;
    let mut need = Vec::new();
    let test = match (&connect.public, &connect.cfx) {
        (Some(a), _) => format!("Ask a friend who is NOT on your home network to start FiveM, press F8 and type: connect {a}"),
        (None, Some(c)) => format!("Ask a friend who is NOT on your home network to open {c}"),
        _ => format!("Ask a friend who is NOT on your home network to connect to your public address, port {p}. Find your public address by searching \"what is my IP\" from this computer."),
    };
    if cgnat {
        let mut l = layer(LayerId::Internet, ReachState::Problem, "Not reachable from the internet", "Your provider's shared address blocks incoming connections (see the line above).");
        l.steps.push(test);
        return (l, need);
    }
    if f.consent.cfx_list && f.cfx.is_none() && f.join_code.is_none() && f.running {
        // asked for but no code to ask with
    }
    let mut l = match &f.cfx {
        Some(Lookup::Found(listing)) => {
            let mismatch = match public {
                Some((ip, _)) => !listing.endpoints.is_empty() && !listing.endpoints.iter().any(|e| e.parse::<SocketAddr>().is_ok_and(|s| s.ip() == ip)),
                None => false,
            };
            let mut l = layer(
                LayerId::Internet,
                ReachState::Likely,
                "Cfx.re lists your server",
                format!(
                    "Cfx.re's server list shows it as cfx.re/join/{}{}. That is a good sign, but only a real player joining proves that the game's UDP port works too.",
                    listing.code,
                    listing.endpoints.first().map(|e| format!(" (address {e}{})", listing.last_seen.as_ref().map(|t| format!(", last seen {t}")).unwrap_or_default())).unwrap_or_default()
                ),
            );
            if mismatch {
                l.state = ReachState::Unknown;
                l.title = "Cfx.re lists your server under a different address".into();
                l.detail.push_str(" The address Cfx.re has is not the one your router reports: check which address players should use.");
            }
            l
        }
        Some(Lookup::Absent) => {
            layer(LayerId::Internet, ReachState::Unknown, "Cfx.re does not list your server (yet)", "A new server can take a few minutes to appear. This does not prove players cannot connect.")
        }
        Some(Lookup::Unavailable(why)) => layer(LayerId::Internet, ReachState::Unknown, "Could not ask Cfx.re", format!("{why}. This does not say anything about your server.")),
        None => {
            if f.join_code.is_some() && !f.consent.cfx_list {
                need.push(ConsentKind::CfxList);
            }
            let mut l = layer(LayerId::Internet, ReachState::Unknown, "Kora cannot test this from the inside", "Only a computer outside your network can tell whether it gets through.");
            if need.contains(&ConsentKind::CfxList) {
                l.fixes.push(Fix { id: FixId::AllowCfxList, label: "Look my server up on Cfx.re's server list".into() });
                l.detail
                    .push_str(" Kora can look your server up on Cfx.re's public server list, which already knows it (it learns nothing new). It shows whether you are listed and at which address.");
            } else if f.join_code.is_none() {
                l.detail.push_str(" Once the server has started and announced itself to Cfx.re, Kora can read its join code from the server log and look it up there.");
            }
            l
        }
    };
    if public.is_none() && !f.consent.ip_echo && f.router.external_ip.is_none() {
        need.push(ConsentKind::IpEcho);
        l.fixes.push(Fix { id: FixId::AllowIpEcho, label: "Find my public address (asks api.ipify.org)".into() });
    }
    if let Some(Lookup::Unavailable(why)) = &f.echo {
        l.detail.push_str(&format!(" The public address could not be looked up: {why}."));
    }
    l.steps.push(test);
    (l, need)
}

/// The checklist for `f`. Pure.
pub fn assess(f: &Facts) -> ReachReport {
    let p = f.game_port;
    let (behind, cgnat, public) = nat_view(f);
    let lan_s = f.lan.map(|i| format!("{i}:{p}"));
    let public_s = public.filter(|_| !cgnat).map(|(ip, _)| match ip {
        IpAddr::V4(_) => format!("{ip}:{p}"),
        IpAddr::V6(_) => format!("[{ip}]:{p}"),
    });
    let cfx = f.join_code.as_ref().map(|c| format!("https://cfx.re/join/{c}"));
    let best = public_s.clone().or_else(|| lan_s.clone());
    let connect = Connect { lan: lan_s, public: public_s, cfx, command: best.map(|a| format!("connect {a}")) };
    let router = router_layer(f, behind, cgnat, public);
    let (internet, needs_consent) = internet_layer(f, cgnat, public, &connect);
    let layers = vec![running_layer(f), firewall_layer(f), router, internet];
    let verdict = if layers.iter().any(|l| l.state == ReachState::Problem) {
        ReachState::Problem
    } else if layers.iter().all(|l| matches!(l.state, ReachState::Ok)) {
        ReachState::Ok
    } else if layers.iter().any(|l| l.state == ReachState::Unknown) {
        ReachState::Unknown
    } else {
        ReachState::Likely
    };
    let headline = match layers.iter().find(|l| l.state == ReachState::Problem) {
        Some(l) => format!("Something is in the way: {}.", l.title.trim_end_matches('.')),
        None if verdict == ReachState::Ok => "Everything Kora can check looks right.".to_string(),
        None => "Nothing is wrong that Kora can see, but it cannot prove players can get in: finish with the test below.".to_string(),
    };
    let mut contacted = Vec::new();
    if f.cfx.is_some() {
        contacted.push(Contacted { what: "Cfx.re's public server list (frontend.cfx-services.net)".into(), learned: "your server's join code (it already knows it)".into() });
    }
    if f.echo.is_some() {
        contacted.push(Contacted { what: "api.ipify.org".into(), learned: "this connection's public address".into() });
    }
    ReachReport {
        server: f.server.clone(),
        game_port: p,
        layers,
        verdict,
        headline,
        lan_ip: f.lan.map(|i| i.to_string()),
        public_ip: if cgnat { None } else { public.map(|(ip, _)| ip.to_string()) },
        public_source: if cgnat { None } else { public.map(|(_, s)| s) },
        behind_nat: behind,
        cgnat,
        join_code: f.join_code.clone(),
        connect,
        forwarding: f.forwarding.clone(),
        contacted,
        needs_consent,
        checked_unix: f.now,
        consent: f.consent,
    }
}

/// [`gather`] then [`assess`].
pub fn check(env: &dyn ReachEnv, e: &ServerEntry, consent_override: Option<Consent>) -> ReachReport {
    let mut report = assess(&gather(env, e, consent_override));
    // A container server's port is published by its engine; the firewall line says what that means.
    if let Some(rec) = crate::container::record::ContainerRecord::load(&e.dir) {
        crate::container::reach::adjust(&mut report, &rec.engine());
    }
    report
}

/// Turn automatic port forwarding on or off for a server. Off also takes the mapping away at once
/// (the keepers would do it within half a minute anyway).
pub fn set_forwarding(e: &ServerEntry, on: bool) -> Result<ForwardingView, String> {
    let mut s = settings::load_server(&e.dir);
    if s.forward == on {
        return Ok(ForwardingView { enabled: on, status: s.status });
    }
    s.forward = on;
    if !on {
        s.status = None;
    }
    settings::save_server(&e.dir, &s)?;
    if !on {
        forward::release_port(&forward::Discover::default(), e.ports.game);
    }
    Ok(ForwardingView { enabled: on, status: s.status })
}

// ------------------------------------------------------------------ the real machine

pub struct SystemReach {
    cfx: probe::SystemCfxList,
    echo: probe::SystemIpEcho,
}

impl Default for SystemReach {
    fn default() -> Self {
        Self { cfx: probe::SystemCfxList, echo: probe::SystemIpEcho }
    }
}

fn read_log_text(dir: &Path) -> String {
    let mut sources: Vec<std::path::PathBuf> = crate::api::logs::discover(dir).into_iter().map(|s| s.path).collect();
    if let Some(c) = crate::console::source(dir) {
        if !sources.contains(&c) {
            sources.push(c);
        }
    }
    let mut text = String::new();
    for p in sources {
        if let Ok(lines) = crate::api::logs::tail(&p, 3000) {
            text.push_str(&lines.join("\n"));
            text.push('\n');
        }
    }
    text
}

impl ReachEnv for SystemReach {
    fn running(&self, e: &ServerEntry) -> bool {
        let spec = crate::launch::Spec { name: e.name.clone(), dir: e.dir.clone(), game_port: e.ports.game, txadmin_port: e.ports.txadmin };
        e.dir.is_dir() && crate::launch::status_of(&crate::launch::SystemEnv::default(), &spec).running
    }
    fn tcp_connect(&self, ip: IpAddr, port: u16) -> bool {
        TcpStream::connect_timeout(&SocketAddr::new(ip, port), Duration::from_millis(800)).is_ok()
    }
    fn udp_held(&self, port: u16) -> bool {
        // Binding fails when another program holds the port (what the install's port check does).
        std::net::UdpSocket::bind(("0.0.0.0", port)).is_err()
    }
    fn lan_ip(&self) -> Option<IpAddr> {
        crate::launch::lan_address()
    }
    fn firewall(&self, e: &ServerEntry) -> FirewallFacts {
        use crate::firewall::{detect, FirewallKind};
        use crate::service::{Platform, SystemRunner};
        let kind = detect(Platform::current(), &SystemRunner).map(|k| match k {
            FirewallKind::Ufw => "ufw",
            FirewallKind::Firewalld => "firewalld",
            FirewallKind::WindowsFirewall => "Windows Firewall",
        });
        let opened_by_kora = crate::installer::load_created_from(&e.dir).is_some_and(|c| c.firewall);
        FirewallFacts { kind, opened_by_kora, readable: kind.is_some() || crate::elevate::is_elevated() || cfg!(windows) }
    }
    fn router(&self) -> RouterFacts {
        let mut protocols = Vec::new();
        let mut external_ip = None;
        for mut m in forward::discover(&forward::Discover::default()) {
            protocols.push(m.name());
            if external_ip.is_none() {
                external_ip = m.external_ip().ok();
            }
        }
        RouterFacts { protocols, external_ip }
    }
    fn log_text(&self, e: &ServerEntry) -> String {
        read_log_text(&e.dir)
    }
    fn saved_join_code(&self, e: &ServerEntry) -> Option<String> {
        settings::load_server(&e.dir).join_code
    }
    fn save_join_code(&self, e: &ServerEntry, code: &str) {
        let mut s = settings::load_server(&e.dir);
        s.join_code = Some(code.to_string());
        let _ = settings::save_server(&e.dir, &s);
    }
    fn forwarding(&self, e: &ServerEntry) -> ForwardingView {
        let s = settings::load_server(&e.dir);
        ForwardingView { enabled: s.forward, status: s.status }
    }
    fn consent(&self) -> Consent {
        settings::load_consent()
    }
    fn cfx(&self) -> &dyn probe::CfxList {
        &self.cfx
    }
    fn echo(&self) -> &dyn probe::IpEcho {
        &self.echo
    }
    fn now(&self) -> u64 {
        crate::monitor::now_unix()
    }
}

/// A plausible set of facts for tests of the front ends: running, behind a router with UPnP, nothing known outside.
#[cfg(any(test, feature = "fake"))]
pub fn sample_facts(server: &str, port: u16) -> Facts {
    Facts {
        server: server.into(),
        game_port: port,
        running: true,
        tcp_local: true,
        tcp_lan: Some(true),
        udp_held: true,
        lan: Some("192.168.1.50".parse().expect("literal")),
        firewall: FirewallFacts { kind: None, opened_by_kora: false, readable: true },
        router: RouterFacts { protocols: vec!["UPnP"], external_ip: Some("84.12.5.6".parse().expect("literal")) },
        forwarding: ForwardingView { enabled: false, status: None },
        join_code: None,
        consent: Consent::default(),
        cfx: None,
        echo: None,
        now: 1,
    }
}

#[cfg(test)]
mod tests;
