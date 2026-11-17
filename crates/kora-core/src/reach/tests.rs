use super::*;
use std::cell::RefCell;

fn facts() -> Facts {
    Facts {
        server: "main".into(),
        game_port: 30120,
        running: true,
        tcp_local: true,
        tcp_lan: Some(true),
        udp_held: true,
        lan: Some("192.168.1.50".parse().unwrap()),
        firewall: FirewallFacts { kind: None, opened_by_kora: false, readable: true },
        router: RouterFacts { protocols: vec![], external_ip: None },
        forwarding: ForwardingView { enabled: false, status: None },
        join_code: None,
        consent: Consent::default(),
        cfx: None,
        echo: None,
        now: 7,
    }
}

fn layer_of<'a>(r: &'a ReachReport, id: &str) -> &'a Layer {
    r.layers.iter().find(|l| l.id == id).unwrap()
}

fn listing(ep: &str) -> Lookup<Listing> {
    Lookup::Found(Listing { code: "abc123".into(), endpoints: vec![ep.into()], hostname: None, clients: Some(0) })
}

#[test]
fn stopped_server_offers_to_start_it() {
    let mut f = facts();
    f.running = false;
    f.tcp_local = false;
    let r = assess(&f);
    let l = layer_of(&r, "running");
    assert_eq!(l.state, State::Problem);
    assert_eq!(l.fixes[0].id, "start_server");
    assert_eq!(r.verdict, State::Problem);
    assert!(r.headline.starts_with("Something is in the way"));
}

#[test]
fn tcp_without_udp_and_loopback_only_are_told_apart() {
    let mut f = facts();
    f.udp_held = false;
    assert!(layer_of(&assess(&f), "running").title.contains("UDP"));
    let mut f = facts();
    f.tcp_lan = Some(false);
    let l = layer_of(&assess(&f), "running").clone();
    assert!(l.title.contains("only listens on this computer"));
    assert!(l.steps[0].contains("0.0.0.0:30120"));
    let mut f = facts();
    f.tcp_local = false;
    assert!(layer_of(&assess(&f), "running").title.contains("not answering"));
}

#[test]
fn firewall_cases() {
    let mut f = facts();
    f.firewall = FirewallFacts { kind: Some("ufw"), opened_by_kora: true, readable: true };
    assert_eq!(layer_of(&assess(&f), "firewall").state, State::Ok);
    f.firewall = FirewallFacts { kind: Some("ufw"), opened_by_kora: false, readable: true };
    let r = assess(&f);
    let l = layer_of(&r, "firewall");
    assert_eq!(l.state, State::Problem);
    assert_eq!(l.fixes[0].id, "open_firewall");
    assert!(l.steps[0].contains("ufw allow 30120/tcp") && l.steps[0].contains("30120/udp"));
    f.firewall = FirewallFacts { kind: Some("Windows Firewall"), opened_by_kora: false, readable: true };
    assert_eq!(layer_of(&assess(&f), "firewall").state, State::Unknown);
    f.firewall = FirewallFacts { kind: None, opened_by_kora: false, readable: false };
    assert_eq!(layer_of(&assess(&f), "firewall").state, State::Unknown);
    f.firewall = FirewallFacts { kind: None, opened_by_kora: false, readable: true };
    assert_eq!(layer_of(&assess(&f), "firewall").state, State::Ok);
}

#[test]
fn carrier_grade_nat_is_a_problem_with_alternatives_and_no_public_address() {
    let mut f = facts();
    f.router.external_ip = Some("100.72.3.4".parse().unwrap());
    f.router.protocols = vec!["UPnP"];
    let r = assess(&f);
    assert!(r.cgnat);
    assert_eq!(r.behind_nat, Some(true));
    let l = layer_of(&r, "router");
    assert_eq!(l.state, State::Problem);
    let text = l.steps.join(" ");
    assert!(text.contains("carrier-grade NAT") && text.contains("VPS") && text.contains("Tailscale"), "{text}");
    // No forwarding button: it cannot work. No public address is offered either.
    assert!(l.fixes.is_empty());
    assert_eq!(r.public_ip, None);
    assert_eq!(r.connect.public, None);
    assert_eq!(layer_of(&r, "internet").state, State::Problem);
}

#[test]
fn double_nat_with_a_private_wan_address_is_treated_the_same() {
    let mut f = facts();
    f.router.external_ip = Some("10.1.2.3".parse().unwrap());
    assert!(assess(&f).cgnat);
}

#[test]
fn behind_a_router_without_forwarding_it_is_unknown_with_exact_values() {
    let mut f = facts();
    f.router.external_ip = Some("84.12.5.6".parse().unwrap());
    f.router.protocols = vec!["UPnP"];
    let r = assess(&f);
    assert_eq!(r.behind_nat, Some(true));
    assert_eq!(r.public_ip.as_deref(), Some("84.12.5.6"));
    assert_eq!(r.public_source, Some("router"));
    let l = layer_of(&r, "router");
    assert_eq!(l.state, State::Unknown);
    assert_eq!(l.fixes[0].id, "enable_forwarding");
    let text = l.steps.join("\n");
    assert!(text.contains("TCP and UDP") && text.contains("30120") && text.contains("192.168.1.50"), "{text}");
    assert_eq!(r.connect.lan.as_deref(), Some("192.168.1.50:30120"));
    assert_eq!(r.connect.public.as_deref(), Some("84.12.5.6:30120"));
    assert_eq!(r.connect.command.as_deref(), Some("connect 84.12.5.6:30120"));
}

#[test]
fn no_forwarding_button_when_no_router_protocol_answered() {
    let r = assess(&facts());
    assert!(layer_of(&r, "router").fixes.is_empty());
}

#[test]
fn forwarding_results_show_in_the_router_line() {
    let mut f = facts();
    f.router.external_ip = Some("84.12.5.6".parse().unwrap());
    f.forwarding = ForwardingView {
        enabled: true,
        status: Some(ForwardStatus {
            at: 1,
            ok: true,
            via: Some("UPnP".into()),
            message: "Your router (UPnP) forwards port 30120 (TCP and UDP) while the server runs.".into(),
            external_ip: None,
            permanent: false,
        }),
    };
    assert_eq!(layer_of(&assess(&f), "router").state, State::Ok);
    f.forwarding.status = Some(ForwardStatus { at: 1, ok: false, via: None, message: "The router refused: read-only".into(), external_ip: None, permanent: false });
    let r = assess(&f);
    assert_eq!(layer_of(&r, "router").state, State::Problem);
    assert!(layer_of(&r, "router").detail.contains("read-only"));
    f.forwarding.status = None;
    assert_eq!(layer_of(&assess(&f), "router").state, State::Unknown);
}

#[test]
fn a_public_address_on_this_computer_needs_no_forwarding() {
    let mut f = facts();
    f.lan = Some("84.12.5.40".parse().unwrap());
    let r = assess(&f);
    assert_eq!(r.behind_nat, Some(false));
    assert_eq!(layer_of(&r, "router").state, State::Ok);
    assert_eq!(r.connect.lan.as_deref(), Some("84.12.5.40:30120"));
}

#[test]
fn cfx_listing_is_likely_not_ok_and_gives_the_public_address() {
    let mut f = facts();
    f.join_code = Some("abc123".into());
    f.consent.cfx_list = true;
    f.cfx = Some(listing("198.51.100.9:30120"));
    // 198.51.100.0/24 is documentation space: not a usable public address, so use a real-looking one.
    f.cfx = Some(listing("93.184.216.34:30120"));
    let r = assess(&f);
    let l = layer_of(&r, "internet");
    assert_eq!(l.state, State::Likely);
    assert!(l.detail.contains("only a real player joining proves"));
    assert_eq!(r.public_ip.as_deref(), Some("93.184.216.34"));
    assert_eq!(r.public_source, Some("cfx"));
    assert_eq!(r.connect.cfx.as_deref(), Some("https://cfx.re/join/abc123"));
    assert_eq!(r.contacted.len(), 1);
    assert!(r.contacted[0].what.contains("Cfx.re"));
    // Never a plain "ok" overall from the list alone.
    assert_ne!(r.verdict, State::Ok);
}

#[test]
fn a_cfx_address_that_differs_from_the_routers_is_called_out() {
    let mut f = facts();
    f.join_code = Some("abc123".into());
    f.consent.cfx_list = true;
    f.router.external_ip = Some("84.12.5.6".parse().unwrap());
    f.cfx = Some(listing("93.184.216.34:30120"));
    let l = layer_of(&assess(&f), "internet").clone();
    assert_eq!(l.state, State::Unknown);
    assert!(l.title.contains("different address"));
}

#[test]
fn without_consent_nothing_is_asked_and_the_buttons_say_what_would_happen() {
    let mut f = facts();
    f.join_code = Some("abc123".into());
    let r = assess(&f);
    assert!(r.contacted.is_empty());
    assert_eq!(r.needs_consent, ["cfx_list", "ip_echo"]);
    let l = layer_of(&r, "internet");
    assert_eq!(l.state, State::Unknown);
    assert!(l.fixes.iter().any(|x| x.id == "allow_cfx_list"));
    assert!(l.fixes.iter().any(|x| x.id == "allow_ip_echo" && x.label.contains("api.ipify.org")));
    // The manual test is always there.
    assert!(l.steps.iter().any(|s| s.contains("friend")));
}

#[test]
fn absent_and_unavailable_lookups_are_unknown_not_problems() {
    let mut f = facts();
    f.join_code = Some("abc123".into());
    f.consent.cfx_list = true;
    f.cfx = Some(Lookup::Absent);
    assert_eq!(layer_of(&assess(&f), "internet").state, State::Unknown);
    f.cfx = Some(Lookup::Unavailable("offline".into()));
    let r = assess(&f);
    assert_eq!(layer_of(&r, "internet").state, State::Unknown);
    assert_eq!(r.verdict, State::Unknown);
}

#[test]
fn echo_gives_the_public_address_only_when_nothing_better_is_known() {
    let mut f = facts();
    f.consent.ip_echo = true;
    f.echo = Some(Lookup::Found("93.184.216.34".parse().unwrap()));
    let r = assess(&f);
    assert_eq!(r.public_source, Some("ip_echo"));
    assert_eq!(r.contacted[0].what, "api.ipify.org");
    assert!(r.contacted[0].learned.contains("public address"));
}

// ---- gather: who is asked, and when

struct FakeEnv {
    consent: Consent,
    log: String,
    saved: RefCell<Option<String>>,
    router_ext: Option<Ipv4Addr>,
}
struct Env(FakeEnv);
impl ReachEnv for Env {
    fn running(&self, _: &ServerEntry) -> bool {
        true
    }
    fn tcp_connect(&self, _: IpAddr, _: u16) -> bool {
        true
    }
    fn udp_held(&self, _: u16) -> bool {
        true
    }
    fn lan_ip(&self) -> Option<IpAddr> {
        Some("192.168.1.50".parse().unwrap())
    }
    fn firewall(&self, _: &ServerEntry) -> FirewallFacts {
        FirewallFacts { kind: None, opened_by_kora: false, readable: true }
    }
    fn router(&self) -> RouterFacts {
        RouterFacts { protocols: vec![], external_ip: self.0.router_ext }
    }
    fn log_text(&self, _: &ServerEntry) -> String {
        self.0.log.clone()
    }
    fn saved_join_code(&self, _: &ServerEntry) -> Option<String> {
        self.0.saved.borrow().clone()
    }
    fn save_join_code(&self, _: &ServerEntry, c: &str) {
        *self.0.saved.borrow_mut() = Some(c.into());
    }
    fn forwarding(&self, _: &ServerEntry) -> ForwardingView {
        ForwardingView { enabled: false, status: None }
    }
    fn consent(&self) -> Consent {
        self.0.consent
    }
    fn cfx(&self) -> &dyn probe::CfxList {
        &CFX
    }
    fn echo(&self) -> &dyn probe::IpEcho {
        &ECHO
    }
    fn now(&self) -> u64 {
        1
    }
}

// Thread-local recorders (the trait hands out `&dyn`, which cannot borrow the test's own fields).
thread_local! {
    static CFX_CALLS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static ECHO_CALLS: RefCell<u32> = const { RefCell::new(0) };
}
struct CfxStatic;
struct EchoStatic;
static CFX: CfxStatic = CfxStatic;
static ECHO: EchoStatic = EchoStatic;
impl probe::CfxList for CfxStatic {
    fn listing(&self, code: &str) -> Lookup<Listing> {
        CFX_CALLS.with(|c| c.borrow_mut().push(code.into()));
        Lookup::Absent
    }
}
impl probe::IpEcho for EchoStatic {
    fn public_ip(&self) -> Lookup<IpAddr> {
        ECHO_CALLS.with(|c| *c.borrow_mut() += 1);
        Lookup::Found("93.184.216.34".parse().unwrap())
    }
}

fn entry() -> ServerEntry {
    ServerEntry { name: "main".into(), dir: "/nope".into(), mode: crate::installer::Mode::Production, ports: crate::installer::registry::PortsRecord { game: 30120, txadmin: 40120 }, created_unix: 0 }
}

fn env(consent: Consent, log: &str, router_ext: Option<Ipv4Addr>) -> Env {
    CFX_CALLS.with(|c| c.borrow_mut().clear());
    ECHO_CALLS.with(|c| *c.borrow_mut() = 0);
    Env(FakeEnv { consent, log: log.into(), saved: RefCell::default(), router_ext })
}

const NUCLEUS: &str = "Authenticated with cfx.re Nucleus: https://host-abc123.users.cfx.re/";

#[test]
fn nothing_outside_is_contacted_without_consent() {
    let e = env(Consent::default(), NUCLEUS, None);
    let r = check(&e, &entry(), None);
    assert!(CFX_CALLS.with(|c| c.borrow().is_empty()));
    assert_eq!(ECHO_CALLS.with(|c| *c.borrow()), 0);
    assert!(r.contacted.is_empty());
    assert_eq!(r.join_code.as_deref(), Some("abc123"), "the code is read from the log, which is local");
    assert_eq!(e.0.saved.borrow().as_deref(), Some("abc123"), "and remembered");
}

#[test]
fn consent_allows_exactly_the_lookups_it_names() {
    let e = env(Consent { cfx_list: true, ip_echo: false }, NUCLEUS, None);
    let _ = check(&e, &entry(), None);
    assert_eq!(CFX_CALLS.with(|c| c.borrow().clone()), ["abc123"]);
    assert_eq!(ECHO_CALLS.with(|c| *c.borrow()), 0);
    let e = env(Consent { cfx_list: false, ip_echo: true }, NUCLEUS, None);
    let _ = check(&e, &entry(), None);
    assert!(CFX_CALLS.with(|c| c.borrow().is_empty()));
    assert_eq!(ECHO_CALLS.with(|c| *c.borrow()), 1);
}

#[test]
fn the_echo_service_is_skipped_when_the_router_already_told_us() {
    let e = env(Consent { cfx_list: false, ip_echo: true }, "", Some("84.12.5.6".parse().unwrap()));
    let _ = check(&e, &entry(), None);
    assert_eq!(ECHO_CALLS.with(|c| *c.borrow()), 0);
}

#[test]
fn a_one_off_consent_does_not_need_saving() {
    let e = env(Consent::default(), NUCLEUS, None);
    let r = check(&e, &entry(), Some(Consent { cfx_list: true, ip_echo: false }));
    assert_eq!(CFX_CALLS.with(|c| c.borrow().len()), 1);
    assert_eq!(r.contacted.len(), 1);
}
