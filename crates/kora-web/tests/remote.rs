//! Remote access: identity providers, roles, remote-destructive switch, Host checks, config file.
//! Everything runs through `router()` with a fake backend and a fake tailscale `whois`; no real
//! tailscaled, no real files outside a temp dir.
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, Request, Response};
use kora_core::api::fake::FakeBackend;
use kora_web::remote::{Remote, RemoteConfig, Role, Whois};
use kora_web::{router, AppState};
use serde_json::{json, Value};
use tower::ServiceExt;

const BOUND: &str = "100.64.0.5:47120";
const LAUNCH: &str = "launchtoken";
const REMOTE_PEER: &str = "100.64.0.20:51000";
const LOOPBACK_PEER: &str = "127.0.0.1:51000";

/// A fake tailscaled: answers `whois` from a table and records what it was asked.
#[derive(Default)]
struct FakeWhois {
    logins: Mutex<Vec<(String, String)>>,
    asked: Mutex<Vec<SocketAddr>>,
    names: Vec<String>,
}
impl FakeWhois {
    fn with(ip: &str, login: &str) -> Arc<Self> {
        let w = Self::default();
        w.logins.lock().unwrap().push((ip.into(), login.into()));
        Arc::new(w)
    }
}
impl Whois for FakeWhois {
    fn whois(&self, peer: SocketAddr) -> Result<String, String> {
        self.asked.lock().unwrap().push(peer);
        self.logins.lock().unwrap().iter().find(|(ip, _)| *ip == peer.ip().to_string()).map(|(_, l)| l.clone()).ok_or_else(|| "Tailscale does not know this address.".to_string())
    }
    fn self_names(&self) -> Vec<String> {
        self.names.clone()
    }
}

struct World {
    app: axum::Router,
    remote: Arc<Remote>,
}

fn world_with(remote: Remote) -> World {
    let remote = Arc::new(remote);
    let fake = FakeBackend::sample();
    let st = AppState::system(LAUNCH.into(), BOUND.parse().unwrap()).with_backend(Arc::new(move || Box::new(fake.clone()))).with_remote(remote.clone());
    World { app: router(st), remote }
}

fn enabled_cfg() -> RemoteConfig {
    RemoteConfig { enabled: true, ..Default::default() }
}

/// Remote access on, one token user per role. Returns their tokens.
fn with_users(cfg: RemoteConfig) -> (World, [String; 3]) {
    let mut cfg = cfg;
    let v = cfg.add_user("vera", Role::Viewer).unwrap();
    let o = cfg.add_user("olly", Role::Operator).unwrap();
    let a = cfg.add_user("adam", Role::Admin).unwrap();
    (world_with(Remote::new(None, cfg)), [v, o, a])
}

struct Req {
    method: &'static str,
    uri: String,
    host: String,
    peer: Option<&'static str>,
    headers: Vec<(&'static str, String)>,
    body: Option<Value>,
}

fn req(method: &'static str, uri: &str) -> Req {
    Req { method, uri: uri.into(), host: BOUND.into(), peer: Some(REMOTE_PEER), headers: vec![], body: None }
}
impl Req {
    fn host(mut self, h: &str) -> Self {
        self.host = h.into();
        self
    }
    fn peer(mut self, p: &'static str) -> Self {
        self.peer = Some(p);
        self
    }
    fn no_peer(mut self) -> Self {
        self.peer = None;
        self
    }
    fn bearer(mut self, t: &str) -> Self {
        self.headers.push(("authorization", format!("Bearer {t}")));
        self
    }
    /// A session as the helpers return it, `cookie|secret`: sends both, as the page does.
    fn cookie(mut self, c: &str) -> Self {
        let (cookie, secret) = c.split_once('|').unwrap_or((c, ""));
        self.headers.push(("cookie", cookie.into()));
        if !secret.is_empty() {
            self.headers.push(("x-kora-session", secret.into()));
        }
        self
    }
    fn h(mut self, k: &'static str, v: &str) -> Self {
        self.headers.push((k, v.into()));
        self
    }
    fn json(mut self, v: Value) -> Self {
        self.body = Some(v);
        self
    }
    async fn send(self, app: &axum::Router) -> Resp {
        let mut b = Request::builder().method(self.method).uri(&self.uri).header(header::HOST, &self.host);
        for (k, v) in &self.headers {
            b = b.header(*k, v);
        }
        let body = match &self.body {
            Some(v) => {
                b = b.header(header::CONTENT_TYPE, "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        let mut r = b.body(body).unwrap();
        if let Some(p) = self.peer {
            r.extensions_mut().insert(ConnectInfo(p.parse::<SocketAddr>().unwrap()));
        }
        Resp::new(app.clone().oneshot(r).await.unwrap()).await
    }
}

struct Resp {
    status: u16,
    json: Value,
    /// The first `Set-Cookie` (`name=value`), with `|secret` added when the answer carries one.
    cookie: Option<String>,
}
impl Resp {
    async fn new(r: Response<Body>) -> Self {
        let status = r.status().as_u16();
        let cookie = r.headers().get(header::SET_COOKIE).map(|c| c.to_str().unwrap().split(';').next().unwrap().to_string());
        let is_stream = r.headers().get(header::CONTENT_TYPE).is_some_and(|v| v.to_str().unwrap_or("").starts_with("text/event-stream"));
        let json = if is_stream {
            Value::Null
        } else {
            let bytes = axum::body::to_bytes(r.into_body(), 1 << 22).await.unwrap();
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        let cookie = match (cookie, json["secret"].as_str()) {
            (Some(c), Some(sec)) => Some(format!("{c}|{sec}")),
            (c, _) => c,
        };
        Self { status, json, cookie }
    }
    fn msg(&self) -> String {
        self.json["error"]["message"].as_str().unwrap_or("").to_string()
    }
}

/// POST /session as a remote peer with a user token; returns the cookie.
async fn sign_in(app: &axum::Router, token: &str) -> String {
    let r = req("POST", "/api/v1/session").h("x-kora", "1").json(json!({"token": token})).send(app).await;
    assert_eq!(r.status, 200, "{:?}", r.json);
    r.cookie.expect("a session cookie")
}

// ---------- loopback is unchanged ----------

#[tokio::test]
async fn loopback_still_needs_the_launch_token_and_is_admin() {
    let (w, [_, _, adam]) = with_users(enabled_cfg());
    // From the computer itself: the launch token works, in every form.
    let r = req("GET", "/api/v1/servers").host(BOUND).peer(LOOPBACK_PEER).bearer(LAUNCH).send(&w.app).await;
    assert_eq!(r.status, 200);
    let r = req("GET", "/api/v1/servers").peer(LOOPBACK_PEER).send(&w.app).await;
    assert_eq!(r.status, 401);
    // A remote user's token is not a launch token.
    let r = req("GET", "/api/v1/servers").peer(LOOPBACK_PEER).bearer(&adam).send(&w.app).await;
    assert_eq!(r.status, 401);
    // The local session is admin: it can do local-only and destructive things.
    let r = req("POST", "/api/v1/session").peer(LOOPBACK_PEER).h("x-kora", "1").json(json!({"token": LAUNCH})).send(&w.app).await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json["identity"], json!({"name": "local", "provider": "local", "role": "admin"}));
    let cookie = r.cookie.unwrap();
    let r = req("GET", "/api/v1/users").peer(LOOPBACK_PEER).cookie(&cookie).send(&w.app).await;
    assert_eq!(r.status, 200);
    let r = req("GET", "/api/v1/meta").peer(LOOPBACK_PEER).cookie(&cookie).send(&w.app).await;
    assert_eq!(r.json["identity"]["role"], "admin");
    assert_eq!(r.json["remote"], false);
}

#[tokio::test]
async fn the_launch_token_does_not_work_from_another_computer() {
    let (w, _) = with_users(enabled_cfg());
    let r = req("GET", "/api/v1/servers").bearer(LAUNCH).send(&w.app).await;
    assert_eq!(r.status, 401);
    let r = req("POST", "/api/v1/session").h("x-kora", "1").json(json!({"token": LAUNCH})).send(&w.app).await;
    assert_eq!(r.status, 401);
}

#[tokio::test]
async fn a_cookie_only_works_from_the_kind_of_peer_it_was_made_for() {
    let (w, [_, _, adam]) = with_users(enabled_cfg());
    let remote_cookie = sign_in(&w.app, &adam).await;
    let r = req("GET", "/api/v1/servers").peer(LOOPBACK_PEER).cookie(&remote_cookie).send(&w.app).await;
    assert_eq!(r.status, 401);
    let r = req("POST", "/api/v1/session").peer(LOOPBACK_PEER).h("x-kora", "1").json(json!({"token": LAUNCH})).send(&w.app).await;
    let local_cookie = r.cookie.unwrap();
    let r = req("GET", "/api/v1/servers").cookie(&local_cookie).send(&w.app).await;
    assert_eq!(r.status, 401, "a local cookie must not work from a remote peer");
}

#[tokio::test]
async fn remote_peers_are_refused_while_remote_access_is_off() {
    let w = world_with(Remote::memory());
    for uri in ["/api/v1/meta", "/api/v1/servers", "/"] {
        let r = req("GET", uri).send(&w.app).await;
        assert_eq!(r.status, 403, "{uri}");
        assert!(r.msg().contains("turned off"), "{}", r.msg());
    }
    // The local one is fine.
    let r = req("GET", "/api/v1/meta").no_peer().send(&w.app).await;
    assert_eq!(r.status, 200);
}

// ---------- Host / Origin ----------

#[tokio::test]
async fn host_checks_for_a_remote_bind() {
    let mut cfg = enabled_cfg();
    cfg.allowed_hosts = vec!["box.example.com".into()];
    let w = world_with(Remote::new(None, cfg));
    w.remote.set_dynamic_hosts(vec!["box.tail1234.ts.net".into(), "box".into()]);
    let get = |h: &'static str| {
        let app = w.app.clone();
        async move { req("GET", "/api/v1/meta").host(h).send(&app).await.status }
    };
    assert_eq!(get(BOUND).await, 200, "the bound address");
    assert_eq!(get("box.example.com:47120").await, 200, "a name from the settings");
    assert_eq!(get("BOX.tail1234.ts.net:47120").await, 200, "the MagicDNS name, any case");
    assert_eq!(get("box:47120").await, 200, "the short MagicDNS name");
    assert_eq!(get("box.example.com").await, 403, "a configured name still needs the panel's port");
    assert_eq!(get("box.example.com:9").await, 403);
    assert_eq!(get("evil.example.org:47120").await, 403);
    assert_eq!(get("127.0.0.1:47120").await, 403, "this bind is not loopback");
    assert_eq!(get("localhost:47120").await, 403);
}

#[tokio::test]
async fn configured_names_are_ignored_while_remote_access_is_off() {
    let cfg = RemoteConfig { allowed_hosts: vec!["box.example.com".into()], ..Default::default() };
    let w = world_with(Remote::new(None, cfg));
    let r = req("GET", "/api/v1/meta").host("box.example.com:47120").no_peer().send(&w.app).await;
    assert_eq!(r.status, 403);
}

#[tokio::test]
async fn csrf_and_origin_still_apply_to_remote_sessions() {
    let (w, [_, _, adam]) = with_users(enabled_cfg());
    let cookie = sign_in(&w.app, &adam).await;
    let body = json!({"action": "restart"});
    let r = req("POST", "/api/v1/servers/prod/control").cookie(&cookie).json(body.clone()).send(&w.app).await;
    assert_eq!((r.status, r.msg().as_str()), (403, "missing X-Kora header"));
    let r = req("POST", "/api/v1/servers/prod/control").cookie(&cookie).h("x-kora", "1").h("origin", "http://evil.example.org").json(body.clone()).send(&w.app).await;
    assert_eq!((r.status, r.msg().as_str()), (403, "origin not allowed"));
    let r = req("POST", "/api/v1/servers/prod/control").cookie(&cookie).h("x-kora", "1").h("origin", &format!("http://{BOUND}")).json(body).send(&w.app).await;
    assert_eq!(r.status, 202);
}

// ---------- token provider ----------

#[tokio::test]
async fn token_sign_in_and_meta_identity() {
    let (w, [vera, _, _]) = with_users(enabled_cfg());
    let cookie = sign_in(&w.app, &vera).await;
    assert!(cookie.starts_with("kora_session_47120="));
    let r = req("GET", "/api/v1/meta").cookie(&cookie).send(&w.app).await;
    assert_eq!(r.json["identity"], json!({"name": "vera", "provider": "token", "role": "viewer"}));
    assert_eq!(r.json["remote"], true);
    // Not signed in: meta still answers, with no identity.
    let r = req("GET", "/api/v1/meta").send(&w.app).await;
    assert_eq!(r.status, 200);
    assert!(r.json["identity"].is_null());
    // A bearer token works for scripts too.
    let r = req("GET", "/api/v1/servers").bearer(&vera).send(&w.app).await;
    assert_eq!(r.status, 200);
}

#[tokio::test]
async fn bad_tokens_are_refused_and_lock_out_remote_sign_in_only() {
    let (w, [_, _, adam]) = with_users(enabled_cfg());
    for _ in 0..10 {
        let r = req("POST", "/api/v1/session").h("x-kora", "1").json(json!({"token": "wrong"})).send(&w.app).await;
        assert_eq!(r.status, 401);
    }
    let r = req("POST", "/api/v1/session").h("x-kora", "1").json(json!({"token": adam})).send(&w.app).await;
    assert_eq!(r.status, 429, "locked out after repeated failures");
    // The owner on the computer itself is not locked out by strangers.
    let r = req("POST", "/api/v1/session").peer(LOOPBACK_PEER).h("x-kora", "1").json(json!({"token": LAUNCH})).send(&w.app).await;
    assert_eq!(r.status, 200);
}

#[tokio::test]
async fn removing_a_user_or_changing_their_role_takes_effect_at_once() {
    let (w, [_, olly, _]) = with_users(enabled_cfg());
    let cookie = sign_in(&w.app, &olly).await;
    let ok = |uri: &'static str| {
        let (app, c) = (w.app.clone(), cookie.clone());
        async move { req("GET", uri).cookie(&c).send(&app).await.status }
    };
    assert_eq!(ok("/api/v1/servers/prod/logs").await, 200);
    w.remote
        .update(|c| {
            c.users.iter_mut().find(|u| u.name == "olly").unwrap().role = Role::Viewer;
            Ok(())
        })
        .unwrap();
    // A changed role ends the session: they must sign in again to get the new role.
    assert_eq!(ok("/api/v1/servers/prod/logs").await, 401);
    assert_eq!(ok("/api/v1/servers").await, 401);
    let cookie2 = sign_in(&w.app, &olly).await;
    w.remote.update(|c| Ok(c.remove_user("olly"))).unwrap();
    assert_eq!(req("GET", "/api/v1/servers").cookie(&cookie2).send(&w.app).await.status, 401);
}

// ---------- tailscale provider ----------

fn tailscale_world(login: &str, map: &[(&str, Role)]) -> (World, Arc<FakeWhois>) {
    let mut cfg = enabled_cfg();
    cfg.tailscale = true;
    for (l, r) in map {
        cfg.tailscale_users.insert(l.to_string(), *r);
    }
    let who = FakeWhois::with("100.64.0.20", login);
    (world_with(Remote::new(None, cfg).with_whois(who.clone())), who)
}

#[tokio::test]
async fn tailscale_identity_comes_from_whois_only() {
    let (w, who) = tailscale_world("Alice@Example.com", &[("alice@example.com", Role::Operator), ("boss@example.com", Role::Admin)]);
    // The request claims to be the boss with the headers tailscale serve would add: refused outright.
    let r = req("POST", "/api/v1/session").h("x-kora", "1").h("tailscale-user-login", "boss@example.com").json(json!({})).send(&w.app).await;
    assert_eq!(r.status, 403, "{:?}", r.json);
    assert!(r.msg().contains("tailscale serve"), "{}", r.msg());
    assert!(who.asked.lock().unwrap().is_empty());
    // Other headers that only look like identity are ignored: the identity comes from whois.
    let r = req("POST", "/api/v1/session").h("x-kora", "1").h("x-forwarded-user", "boss@example.com").json(json!({})).send(&w.app).await;
    assert_eq!(r.status, 200, "{:?}", r.json);
    assert_eq!(r.json["identity"], json!({"name": "Alice@Example.com", "provider": "tailscale", "role": "operator"}));
    assert_eq!(who.asked.lock().unwrap().as_slice(), &[REMOTE_PEER.parse::<SocketAddr>().unwrap()], "whois is asked about the peer address");
    let cookie = r.cookie.unwrap();
    // Operator: may restart, may not read users or delete.
    let r = req("POST", "/api/v1/servers/prod/control").cookie(&cookie).h("x-kora", "1").json(json!({"action": "restart"})).send(&w.app).await;
    assert_eq!(r.status, 202);
    let r = req("DELETE", "/api/v1/servers/prod/registration").cookie(&cookie).h("x-kora", "1").send(&w.app).await;
    assert_eq!(r.status, 403);
}

#[tokio::test]
async fn tailscale_identity_headers_alone_never_sign_anyone_in() {
    let (w, _) = tailscale_world("nobody@example.com", &[("boss@example.com", Role::Admin)]);
    for h in ["tailscale-user-login", "tailscale-user-name"] {
        let r = req("GET", "/api/v1/servers").h(h, "boss@example.com").send(&w.app).await;
        assert_eq!(r.status, 403, "{h}");
    }
    // whois says this peer is someone who is not listed: refused, with their login named.
    let r = req("POST", "/api/v1/session").h("x-kora", "1").json(json!({})).send(&w.app).await;
    assert_eq!(r.status, 401);
    assert!(r.msg().contains("nobody@example.com"), "{}", r.msg());
}

#[tokio::test]
async fn tailscale_off_unknown_peer_or_whois_failure_are_refused() {
    // Provider not enabled.
    let mut cfg = enabled_cfg();
    cfg.tailscale_users.insert("alice@example.com".into(), Role::Admin);
    let who = FakeWhois::with("100.64.0.20", "alice@example.com");
    let w = world_with(Remote::new(None, cfg).with_whois(who));
    let r = req("POST", "/api/v1/session").h("x-kora", "1").json(json!({})).send(&w.app).await;
    assert_eq!(r.status, 401);
    // Enabled, but tailscaled does not know the peer.
    let (w, _) = tailscale_world("alice@example.com", &[("alice@example.com", Role::Admin)]);
    let r = req("POST", "/api/v1/session").peer("10.9.9.9:1234").h("x-kora", "1").json(json!({})).send(&w.app).await;
    assert_eq!(r.status, 401);
    // Turning the provider off ends existing sessions.
    let r = req("POST", "/api/v1/session").h("x-kora", "1").json(json!({})).send(&w.app).await;
    let cookie = r.cookie.unwrap();
    w.remote
        .update(|c| {
            c.tailscale = false;
            Ok(())
        })
        .unwrap();
    let r = req("GET", "/api/v1/servers").cookie(&cookie).send(&w.app).await;
    assert_eq!(r.status, 401);
}

// ---------- roles ----------

async fn status(w: &World, tok: &str, method: &'static str, uri: &str, body: Option<Value>) -> u16 {
    let mut r = req(method, uri).bearer(tok);
    if let Some(b) = body {
        r = r.json(b);
    }
    r.send(&w.app).await.status
}

#[tokio::test]
async fn every_role_boundary_for_a_sample_of_operations() {
    let (w, [vera, olly, adam]) = with_users(enabled_cfg());
    let none = || None;
    // (method, uri, body, lowest role that gets past the guard)
    let table: Vec<(&str, &str, Option<Value>, Role)> = vec![
        ("GET", "/api/v1/servers", none(), Role::Viewer),
        ("GET", "/api/v1/servers/prod", none(), Role::Viewer),
        ("GET", "/api/v1/system", none(), Role::Viewer),
        ("GET", "/api/v1/catalog", none(), Role::Viewer),
        ("GET", "/api/v1/servers/prod/logs", none(), Role::Operator),
        ("GET", "/api/v1/servers/prod/logs/server.log", none(), Role::Operator),
        ("POST", "/api/v1/servers/prod/control", Some(json!({"action": "restart"})), Role::Operator),
        ("POST", "/api/v1/servers/prod/ensure", Some(json!({"name": "x", "restart": false})), Role::Operator),
        ("POST", "/api/v1/servers/prod/update/preview", Some(json!({"scope": "all"})), Role::Operator),
        ("POST", "/api/v1/servers/prod/webhook/test", Some(json!({})), Role::Operator),
        ("PUT", "/api/v1/servers/prod/ports", Some(json!({"game": 30120, "txadmin": 40120})), Role::Admin),
        ("PUT", "/api/v1/servers/prod/webhook", Some(json!({"url": "https://example.com/x"})), Role::Admin),
        ("POST", "/api/v1/servers/prod/resources", Some(json!({"ids": []})), Role::Admin),
        ("POST", "/api/v1/installs/plan", Some(json!({})), Role::Admin),
        ("PUT", "/api/v1/servers/prod/console/enabled", Some(json!({"enabled": true})), Role::Admin),
    ];
    for (method, uri, body, min) in table {
        for (role, tok) in [(Role::Viewer, &vera), (Role::Operator, &olly), (Role::Admin, &adam)] {
            let s = status(&w, tok, method, uri, body.clone()).await;
            if role >= min {
                assert_ne!(s, 403, "{role} should get past the guard for {method} {uri}");
                assert_ne!(s, 401, "{method} {uri}");
            } else {
                assert_eq!(s, 403, "{role} must be refused for {method} {uri}");
            }
        }
    }
}

#[tokio::test]
async fn refusals_explain_themselves() {
    let (w, [vera, _, _]) = with_users(enabled_cfg());
    let r = req("POST", "/api/v1/servers/prod/control").bearer(&vera).json(json!({"action": "stop"})).send(&w.app).await;
    assert_eq!(r.status, 403);
    assert_eq!(r.json["error"]["code"], "forbidden");
    assert!(r.msg().contains("viewer") && r.msg().contains("operator"), "{}", r.msg());
    // The fake backend was not touched by a refused request.
}

#[tokio::test]
async fn a_viewer_cannot_read_the_console_or_logs() {
    let (w, [vera, olly, _]) = with_users(enabled_cfg());
    assert_eq!(status(&w, &vera, "GET", "/api/v1/servers/prod/console/stream", None).await, 403);
    assert_eq!(status(&w, &vera, "GET", "/api/v1/servers/prod/logs", None).await, 403);
    assert_eq!(status(&w, &olly, "GET", "/api/v1/servers/prod/logs", None).await, 200);
}

#[tokio::test]
async fn unknown_non_get_routes_are_refused_for_remote_callers() {
    let (w, [_, _, adam]) = with_users(enabled_cfg());
    assert_eq!(status(&w, &adam, "POST", "/api/v1/nothing-here", Some(json!({}))).await, 403);
    assert_eq!(status(&w, &adam, "GET", "/api/v1/nothing-here", None).await, 404);
}

// ---------- remote destructive ----------

const DESTRUCTIVE: &[(&str, &str)] = &[
    ("DELETE", "/api/v1/servers/dev"),
    ("DELETE", "/api/v1/servers/dev/registration"),
    ("POST", "/api/v1/uninstall"),
    ("DELETE", "/api/v1/app"),
    ("POST", "/api/v1/servers/prod/console"),
    ("POST", "/api/v1/self-update"),
    ("PUT", "/api/v1/servers/prod/schedule"),
    ("DELETE", "/api/v1/servers/prod/schedule"),
];

/// Installing a new server (start, and resume a paused install).
const INSTALL: &[(&str, &str)] = &[("POST", "/api/v1/installs"), ("POST", "/api/v1/installs/j1/resume")];

/// What the remote guard says when it refuses an action: a guided problem about the switch.
fn is_switch_refusal(r: &Resp) -> bool {
    r.status == 403 && r.json["error"]["code"] == "remote.actions_off"
}

#[tokio::test]
async fn destructive_actions_are_refused_for_token_admins_while_remote_actions_are_off() {
    let (w, [_, _, adam]) = with_users(enabled_cfg());
    for (m, uri) in DESTRUCTIVE {
        let r = req(m, uri).bearer(&adam).json(json!({})).send(&w.app).await;
        assert!(is_switch_refusal(&r), "{m} {uri}: {} {:?}", r.status, r.json);
        assert!(r.msg().contains("turned off for sign-ins made with a token"), "{m} {uri}: {}", r.msg());
        // The problem says exactly how to turn it on, and the other ways to do it.
        let e = &r.json["error"];
        assert!(e["details"].as_str().unwrap().contains("kora panel remote-actions on"), "{e}");
        assert!(e["details"].as_str().unwrap().contains("Tailscale"), "{e}");
        assert_eq!(e["actions"][0]["kind"], "show_command", "{e}");
        assert_eq!(e["actions"][0]["command"], json!(["kora", "panel", "remote-actions", "on"]));
    }
}

#[tokio::test]
async fn token_admins_can_install_by_default_but_other_token_roles_cannot() {
    let (w, [vera, olly, adam]) = with_users(enabled_cfg());
    for (m, uri) in INSTALL {
        let r = req(m, uri).bearer(&adam).json(json!({})).send(&w.app).await;
        // The handler may refuse the empty request itself; the remote guard must not.
        assert!(!is_switch_refusal(&r), "{m} {uri}: {:?}", r.json);
        assert_ne!(r.status, 401, "{m} {uri}");
        assert!(!r.msg().contains("not available over remote"), "{m} {uri}: {}", r.msg());
        for tok in [&vera, &olly] {
            let r = req(m, uri).bearer(tok).json(json!({})).send(&w.app).await;
            assert_eq!(r.status, 403, "{m} {uri}");
            assert!(r.msg().contains("access, so it cannot do this"), "{m} {uri}: {}", r.msg());
        }
    }
}

#[tokio::test]
async fn tailscale_admins_get_install_and_destructive_actions_with_no_switch() {
    let (w, _) = tailscale_world("boss@example.com", &[("boss@example.com", Role::Admin)]);
    assert!(!w.remote.config().allow_remote_destructive, "the switch is off");
    let r = req("POST", "/api/v1/session").h("x-kora", "1").json(json!({})).send(&w.app).await;
    assert_eq!(r.status, 200, "{:?}", r.json);
    let cookie = r.cookie.unwrap();
    // The typed-name confirmation still applies.
    let r = req("DELETE", "/api/v1/servers/dev").cookie(&cookie).h("x-kora", "1").json(json!({"confirm_name": "wrong"})).send(&w.app).await;
    assert_eq!(r.status, 400);
    assert!(r.msg().contains("does not match"), "{}", r.msg());
    for (m, uri) in DESTRUCTIVE.iter().chain(INSTALL) {
        let r = req(m, uri).cookie(&cookie).h("x-kora", "1").json(json!({})).send(&w.app).await;
        assert!(!is_switch_refusal(&r), "{m} {uri}: {:?}", r.json);
        assert_ne!(r.status, 401, "{m} {uri}");
    }
    // The meta flag the UI reads says the destructive actions are available.
    let r = req("GET", "/api/v1/meta").cookie(&cookie).send(&w.app).await;
    assert_eq!(r.json["allow_remote_destructive"], true);
}

#[tokio::test]
async fn tailscale_roles_below_admin_are_unchanged() {
    let (w, _) = tailscale_world("op@example.com", &[("op@example.com", Role::Operator)]);
    let r = req("POST", "/api/v1/session").h("x-kora", "1").json(json!({})).send(&w.app).await;
    let cookie = r.cookie.unwrap();
    for (m, uri) in DESTRUCTIVE.iter().chain(INSTALL) {
        let r = req(m, uri).cookie(&cookie).h("x-kora", "1").json(json!({})).send(&w.app).await;
        assert_eq!(r.status, 403, "{m} {uri}");
        assert!(r.msg().contains("access, so it cannot do this"), "{m} {uri}: {}", r.msg());
    }
}

#[tokio::test]
async fn tailscale_admins_still_cannot_use_local_only_operations() {
    let (w, _) = tailscale_world("boss@example.com", &[("boss@example.com", Role::Admin)]);
    let r = req("POST", "/api/v1/session").h("x-kora", "1").json(json!({})).send(&w.app).await;
    let cookie = r.cookie.unwrap();
    for (m, uri) in [("GET", "/api/v1/remote"), ("GET", "/api/v1/users"), ("POST", "/api/v1/servers/prod/open-folder"), ("POST", "/api/v1/session/link")] {
        let r = req(m, uri).cookie(&cookie).h("x-kora", "1").json(json!({})).send(&w.app).await;
        assert_eq!(r.status, 403, "{m} {uri}");
        assert!(r.msg().contains("only be done on the computer"), "{m} {uri}: {}", r.msg());
    }
}

#[tokio::test]
async fn destructive_actions_pass_the_guard_when_enabled_and_keep_their_own_confirmation() {
    let mut cfg = enabled_cfg();
    cfg.allow_remote_destructive = true;
    let (w, [_, olly, adam]) = with_users(cfg);
    // Typed-name confirmation is still enforced by the handler.
    let r = req("DELETE", "/api/v1/servers/dev").bearer(&adam).json(json!({"confirm_name": "wrong"})).send(&w.app).await;
    assert_eq!(r.status, 400);
    assert!(r.msg().contains("does not match"), "{}", r.msg());
    for (m, uri) in DESTRUCTIVE.iter().chain(INSTALL) {
        let r = req(m, uri).bearer(&adam).json(json!({})).send(&w.app).await;
        // A handler may still answer 403 itself (needs_elevation when this process is not root,
        // as on a CI runner): what matters is that the remote guard did not refuse it.
        assert!(!is_switch_refusal(&r), "{m} {uri} should reach the handler: {}", r.msg());
        assert_ne!(r.status, 401, "{m} {uri}");
    }
    // The switch does not lift the role: an operator is still not an admin.
    assert_eq!(status(&w, &olly, "DELETE", "/api/v1/servers/dev", Some(json!({"confirm_name": "dev"}))).await, 403);
}

#[tokio::test]
async fn local_only_operations_are_never_available_remotely() {
    let mut cfg = enabled_cfg();
    cfg.allow_remote_destructive = true;
    let (w, [_, _, adam]) = with_users(cfg);
    let cases = [
        ("GET", "/api/v1/remote", None),
        ("PUT", "/api/v1/remote", Some(json!({"allow_remote_destructive": true}))),
        ("GET", "/api/v1/users", None),
        ("POST", "/api/v1/users", Some(json!({"name": "mallory", "role": "admin"}))),
        ("DELETE", "/api/v1/users/vera", None),
        ("POST", "/api/v1/servers/prod/open-folder", Some(json!({}))),
        // The hub works with tokens of other panels stored here.
        ("GET", "/api/v1/fleet", None),
        ("GET", "/api/v1/nodes", None),
        ("GET", "/api/v1/nodes/n1/servers", None),
        ("POST", "/api/v1/nodes/n1/servers/prod/control", Some(json!({"action": "stop"}))),
        ("POST", "/api/v1/session/link", Some(json!({}))),
    ];
    for (m, uri, body) in cases {
        let s = status(&w, &adam, m, uri, body).await;
        assert_eq!(s, 403, "{m} {uri}");
    }
    assert_eq!(w.remote.config().users.len(), 3, "no user was added or removed");
}

// ---------- settings and users API (local) ----------

async fn local_cookie(w: &World) -> String {
    let r = req("POST", "/api/v1/session").peer(LOOPBACK_PEER).h("x-kora", "1").json(json!({"token": LAUNCH})).send(&w.app).await;
    r.cookie.unwrap()
}

#[tokio::test]
async fn users_api_shows_the_token_once_and_never_the_hash() {
    let w = world_with(Remote::new(None, enabled_cfg()));
    let c = local_cookie(&w).await;
    let r = req("POST", "/api/v1/users").peer(LOOPBACK_PEER).cookie(&c).h("x-kora", "1").json(json!({"name": "pat", "role": "operator"})).send(&w.app).await;
    assert_eq!(r.status, 201);
    let token = r.json["token"].as_str().unwrap().to_string();
    assert!(token.len() >= 32);
    assert_eq!((r.json["name"].as_str(), r.json["role"].as_str()), (Some("pat"), Some("operator")));
    // Duplicate and bad names.
    let r = req("POST", "/api/v1/users").peer(LOOPBACK_PEER).cookie(&c).h("x-kora", "1").json(json!({"name": "PAT", "role": "viewer"})).send(&w.app).await;
    assert_eq!(r.status, 409);
    let r = req("POST", "/api/v1/users").peer(LOOPBACK_PEER).cookie(&c).h("x-kora", "1").json(json!({"name": "a b", "role": "viewer"})).send(&w.app).await;
    assert_eq!(r.status, 400);
    let r = req("POST", "/api/v1/users").peer(LOOPBACK_PEER).cookie(&c).h("x-kora", "1").json(json!({"name": "x", "role": "root"})).send(&w.app).await;
    assert!(r.status == 400 || r.status == 422, "{}", r.status);
    // List and settings: no token, no hash, no salt.
    let list = req("GET", "/api/v1/users").peer(LOOPBACK_PEER).cookie(&c).send(&w.app).await;
    assert_eq!(list.json["users"][0]["name"], "pat");
    let get = req("GET", "/api/v1/remote").peer(LOOPBACK_PEER).cookie(&c).send(&w.app).await;
    for text in [list.json.to_string(), get.json.to_string()] {
        assert!(!text.contains(&token) && !text.contains("hash") && !text.contains("salt"), "{text}");
    }
    // The new token signs in from another computer, then removal ends the session.
    let cookie = sign_in(&w.app, &token).await;
    assert_eq!(req("GET", "/api/v1/servers").cookie(&cookie).send(&w.app).await.status, 200);
    let r = req("DELETE", "/api/v1/users/pat").peer(LOOPBACK_PEER).cookie(&c).h("x-kora", "1").send(&w.app).await;
    assert_eq!(r.status, 204);
    assert_eq!(req("GET", "/api/v1/servers").cookie(&cookie).send(&w.app).await.status, 401);
    let r = req("DELETE", "/api/v1/users/pat").peer(LOOPBACK_PEER).cookie(&c).h("x-kora", "1").send(&w.app).await;
    assert_eq!(r.status, 404);
}

#[tokio::test]
async fn remote_set_updates_settings_and_checks_input() {
    let w = world_with(Remote::new(None, enabled_cfg()));
    let c = local_cookie(&w).await;
    let put = |b: Value| {
        let (app, c) = (w.app.clone(), c.clone());
        async move { req("PUT", "/api/v1/remote").peer(LOOPBACK_PEER).cookie(&c).h("x-kora", "1").json(b).send(&app).await }
    };
    let r = put(json!({
        "tailscale": true,
        "tailscale_users": {"Alice@Example.com": "operator"},
        "allowed_hosts": ["box.example.com", "BOX.example.com", " 100.64.0.9 "],
        "allow_remote_destructive": true
    }))
    .await;
    assert_eq!(r.status, 200, "{:?}", r.json);
    assert_eq!(r.json["tailscale"], true);
    assert_eq!(r.json["tailscale_users"], json!({"alice@example.com": "operator"}));
    assert_eq!(r.json["allowed_hosts"], json!(["box.example.com", "100.64.0.9"]));
    assert_eq!(r.json["allow_remote_destructive"], true);
    assert_eq!(r.json["bind"], "100.64.0.5");
    // Left-out fields stay as they are.
    let r = put(json!({"allow_remote_destructive": false})).await;
    assert_eq!(r.json["tailscale"], true);
    assert_eq!(r.json["allow_remote_destructive"], false);
    // Bad input.
    assert_eq!(put(json!({"allowed_hosts": ["http://evil"]})).await.status, 400);
    assert_eq!(put(json!({"tailscale_users": {"a b": "admin"}})).await.status, 400);
    assert!(put(json!({"tailscale_users": {"a@b": "root"}})).await.status >= 400);
    assert!(put(json!({"surprise": 1})).await.status >= 400);
    // A new name is honoured at once.
    let r = req("GET", "/api/v1/meta").host("box.example.com:47120").send(&w.app).await;
    assert_eq!(r.status, 200);
}

#[tokio::test]
async fn turning_remote_access_off_closes_the_door_but_not_for_local() {
    let (w, [_, _, adam]) = with_users(enabled_cfg());
    let c = local_cookie(&w).await;
    let r = req("PUT", "/api/v1/remote").peer(LOOPBACK_PEER).cookie(&c).h("x-kora", "1").json(json!({"enabled": false})).send(&w.app).await;
    assert_eq!(r.status, 200);
    assert_eq!(req("GET", "/api/v1/servers").bearer(&adam).send(&w.app).await.status, 403);
    assert_eq!(req("GET", "/api/v1/servers").peer(LOOPBACK_PEER).cookie(&c).send(&w.app).await.status, 200);
}

#[tokio::test]
async fn the_remote_flag_turns_it_on_for_one_run_without_saving() {
    let w = world_with(Remote::new(None, RemoteConfig::default()).forced(true));
    let r = req("GET", "/api/v1/meta").send(&w.app).await;
    assert_eq!(r.status, 200);
    let c = local_cookie(&w).await;
    let r = req("GET", "/api/v1/remote").peer(LOOPBACK_PEER).cookie(&c).send(&w.app).await;
    assert_eq!((r.json["enabled"].clone(), r.json["enabled_by_flag"].clone()), (json!(false), json!(true)));
}

#[tokio::test]
async fn settings_say_where_tailnet_devices_open_the_panel() {
    let w = world_with(Remote::new(None, enabled_cfg()));
    let c = local_cookie(&w).await;
    let get = || req("GET", "/api/v1/remote").peer(LOOPBACK_PEER).cookie(&c).send(&w.app);
    assert_eq!(get().await.json["tailnet_url"], Value::Null, "not listening on Tailscale");
    w.remote.set_tailnet_url(Some("http://box.tail1.ts.net:47120".into()));
    // Once it listens there, the MagicDNS name passes the Host check too.
    w.remote.set_dynamic_hosts(vec!["box.tail1.ts.net".into()]);
    assert_eq!(get().await.json["tailnet_url"], json!("http://box.tail1.ts.net:47120"));
    let r = req("GET", "/api/v1/meta").peer(REMOTE_PEER).host("box.tail1.ts.net:47120").send(&w.app).await;
    assert_eq!(r.status, 200);
}

// ---------- the config file ----------

#[cfg(unix)]
#[tokio::test]
async fn settings_are_saved_to_a_private_file() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("panel.json");
    let w = world_with(Remote::from_file(path.clone()).unwrap().forced(true));
    let c = local_cookie(&w).await;
    let r = req("POST", "/api/v1/users").peer(LOOPBACK_PEER).cookie(&c).h("x-kora", "1").json(json!({"name": "pat", "role": "admin"})).send(&w.app).await;
    let token = r.json["token"].as_str().unwrap().to_string();
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains(&token), "the token must not be on disk");
    assert!(text.contains("\"hash\""));
    // A fresh panel reading the file knows the user.
    let again = Remote::from_file(path).unwrap();
    assert_eq!(again.identity_for_token(&token).unwrap().role, Role::Admin);
}

// ---------- /meta for the UI ----------

#[tokio::test]
async fn meta_tells_a_remote_user_whether_destructive_actions_are_on() {
    let mut cfg = enabled_cfg();
    cfg.allow_remote_destructive = true;
    let (w, [_, _, adam]) = with_users(cfg);
    let r = req("GET", "/api/v1/meta").bearer(&adam).send(&w.app).await;
    assert_eq!(r.json["allow_remote_destructive"], true);
    // With the switch off a token session sees false (a Tailscale admin sees true: see its own test).
    let (w2, [_, _, adam2]) = with_users(enabled_cfg());
    let r = req("GET", "/api/v1/meta").bearer(&adam2).send(&w2.app).await;
    assert_eq!(r.json["allow_remote_destructive"], false);
    let r = req("GET", "/api/v1/meta").send(&w.app).await;
    assert_eq!(r.json["allow_remote_destructive"], false, "not shown to someone who is not signed in");
}

// ---------- session expiry and logout ----------

/// A clock the test can move forward.
fn stepping_clock() -> (kora_web::security::Clock, Arc<Mutex<Duration>>) {
    let base = Instant::now();
    let offset = Arc::new(Mutex::new(Duration::ZERO));
    let o = offset.clone();
    (Arc::new(move || base + *o.lock().unwrap()), offset)
}

fn hours(h: u64) -> Duration {
    Duration::from_secs(h * 3600)
}

fn timed_world() -> (World, Arc<Mutex<Duration>>, String) {
    let mut cfg = enabled_cfg();
    let tok = cfg.add_user("adam", Role::Admin).unwrap();
    let remote = Arc::new(Remote::new(None, cfg));
    let fake = FakeBackend::sample();
    let (clock, offset) = stepping_clock();
    let st = AppState::system(LAUNCH.into(), BOUND.parse().unwrap()).with_backend(Arc::new(move || Box::new(fake.clone()))).with_remote(remote.clone()).with_session_clock(clock);
    (World { app: router(st), remote }, offset, tok)
}

async fn local_login(w: &World) -> String {
    local_cookie(w).await
}

#[tokio::test]
async fn sessions_expire_after_twelve_idle_hours_and_use_keeps_them_alive() {
    let (w, clock, tok) = timed_world();
    let remote = sign_in(&w.app, &tok).await;
    let local = local_login(&w).await;
    let get = |cookie: String, peer: &'static str| {
        let app = w.app.clone();
        async move { req("GET", "/api/v1/servers").peer(peer).cookie(&cookie).send(&app).await.status }
    };
    // Activity every 11 hours keeps both alive well past 12 hours in total.
    for _ in 0..3 {
        *clock.lock().unwrap() += hours(11);
        assert_eq!(get(remote.clone(), REMOTE_PEER).await, 200);
        assert_eq!(get(local.clone(), LOOPBACK_PEER).await, 200);
    }
    // Then 12 hours of silence ends them (401, which shows the UI's "session expired" screen).
    *clock.lock().unwrap() += hours(12);
    assert_eq!(get(remote.clone(), REMOTE_PEER).await, 401);
    assert_eq!(get(local.clone(), LOOPBACK_PEER).await, 401);
    // Signing in again works.
    let again = sign_in(&w.app, &tok).await;
    assert_eq!(get(again, REMOTE_PEER).await, 200);
}

#[tokio::test]
async fn sessions_end_after_seven_days_however_busy() {
    let (w, clock, tok) = timed_world();
    let remote = sign_in(&w.app, &tok).await;
    let local = local_login(&w).await;
    let get = |cookie: String, peer: &'static str| {
        let app = w.app.clone();
        async move { req("GET", "/api/v1/servers").peer(peer).cookie(&cookie).send(&app).await.status }
    };
    // Busy every 6 hours for just under 7 days: still fine.
    let mut t = 0;
    while t + 6 < 7 * 24 {
        t += 6;
        *clock.lock().unwrap() = hours(t);
        assert_eq!(get(remote.clone(), REMOTE_PEER).await, 200, "at {t} h");
        assert_eq!(get(local.clone(), LOOPBACK_PEER).await, 200, "at {t} h");
    }
    *clock.lock().unwrap() = hours(7 * 24);
    assert_eq!(get(remote, REMOTE_PEER).await, 401);
    assert_eq!(get(local, LOOPBACK_PEER).await, 401);
}

#[tokio::test]
async fn logout_ends_the_session_and_clears_the_cookie() {
    let (w, [vera, _, _]) = with_users(enabled_cfg());
    let cookie = sign_in(&w.app, &vera).await;
    let r = req("POST", "/api/v1/session/logout").cookie(&cookie).h("x-kora", "1").send(&w.app).await;
    assert_eq!(r.status, 204);
    assert!(r.cookie.is_some_and(|c| c == "kora_session_47120="), "the cookie is cleared");
    assert_eq!(req("GET", "/api/v1/servers").cookie(&cookie).send(&w.app).await.status, 401);
    // Not signed in: refused like any other route.
    let r = req("POST", "/api/v1/session/logout").h("x-kora", "1").send(&w.app).await;
    assert_eq!(r.status, 401);
}

// ---------- hardening: links, lockout per address, jobs, stream caps ----------

#[tokio::test]
async fn sign_in_links_are_for_this_computer_only() {
    let (w, [_, _, adam]) = with_users(enabled_cfg());
    // A remote admin cannot make one, by token or by session.
    assert_eq!(status(&w, &adam, "POST", "/api/v1/session/link", None).await, 403);
    let cookie = sign_in(&w.app, &adam).await;
    let r = req("POST", "/api/v1/session/link").cookie(&cookie).h("x-kora", "1").send(&w.app).await;
    assert_eq!(r.status, 403);
    // A code made on this computer does not work from another one, and is not used up by trying.
    let r = req("POST", "/api/v1/session/link").peer(LOOPBACK_PEER).bearer(LAUNCH).send(&w.app).await;
    assert_eq!(r.status, 200);
    let code = r.json["code"].as_str().unwrap().to_string();
    let r = req("POST", "/api/v1/session").h("x-kora", "1").json(json!({"code": code})).send(&w.app).await;
    assert_eq!(r.status, 403, "{:?}", r.json);
    assert!(r.msg().contains("computer that runs Kora"), "{}", r.msg());
    let r = req("POST", "/api/v1/session").peer(LOOPBACK_PEER).h("x-kora", "1").json(json!({"code": code})).send(&w.app).await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json["identity"]["provider"], "local");
}

#[tokio::test]
async fn the_remote_lockout_is_per_address() {
    let (w, [_, _, adam]) = with_users(enabled_cfg());
    for _ in 0..10 {
        let r = req("POST", "/api/v1/session").h("x-kora", "1").json(json!({"token": "wrong"})).send(&w.app).await;
        assert_eq!(r.status, 401);
    }
    let attempt = |peer: &'static str| {
        let app = w.app.clone();
        let adam = adam.clone();
        async move { req("POST", "/api/v1/session").peer(peer).h("x-kora", "1").json(json!({"token": adam})).send(&app).await.status }
    };
    assert_eq!(attempt(REMOTE_PEER).await, 429, "the guesser is locked out");
    assert_eq!(attempt("100.64.0.21:40000").await, 200, "someone else is not");
    // IPv6: one /64 is one network.
    for _ in 0..10 {
        req("POST", "/api/v1/session").peer("[fd7a:115c:a1e0:1::5]:5000").h("x-kora", "1").json(json!({"token": "wrong"})).send(&w.app).await;
    }
    assert_eq!(attempt("[fd7a:115c:a1e0:1::6]:5000").await, 429);
    assert_eq!(attempt("[fd7a:115c:a1e0:2::6]:5000").await, 200);
}

#[tokio::test]
async fn remote_callers_only_follow_jobs_their_role_allows() {
    let (w, [vera, olly, adam]) = with_users(enabled_cfg());
    let r = req("POST", "/api/v1/servers/prod/control").bearer(&olly).json(json!({"action": "restart"})).send(&w.app).await;
    assert_eq!(r.status, 202);
    let id = r.json["job"].as_str().unwrap().to_string();
    let get = |tok: String, uri: String| {
        let app = w.app.clone();
        async move { req("GET", &uri).bearer(&tok).send(&app).await.status }
    };
    let job = format!("/api/v1/jobs/{id}");
    let events = format!("/api/v1/jobs/{id}/events");
    assert_eq!(get(vera.clone(), job.clone()).await, 403, "a viewer cannot restart, so cannot follow a restart");
    assert_eq!(get(vera.clone(), events.clone()).await, 403);
    assert_eq!(get(olly.clone(), job.clone()).await, 200);
    assert_eq!(get(olly, events).await, 200);
    assert_eq!(get(adam, job.clone()).await, 200);
    // This computer can follow anything; unknown ids are still 404.
    let r = req("GET", &job).peer(LOOPBACK_PEER).bearer(LAUNCH).send(&w.app).await;
    assert_eq!(r.status, 200);
    assert_eq!(get(vera, "/api/v1/jobs/nope".into()).await, 404);
}

/// Open a live stream as `tok` from `peer` and keep it open (the response is the stream).
async fn open_stream(w: &World, tok: &str, peer: &'static str) -> Response<Body> {
    let mut r = Request::builder().method("GET").uri("/api/v1/events").header(header::HOST, BOUND).header(header::AUTHORIZATION, format!("Bearer {tok}")).body(Body::empty()).unwrap();
    r.extensions_mut().insert(ConnectInfo(peer.parse::<SocketAddr>().unwrap()));
    w.app.clone().oneshot(r).await.unwrap()
}

#[tokio::test]
async fn live_streams_are_capped_per_remote_account() {
    let (w, [vera, olly, _]) = with_users(enabled_cfg());
    let mut open = Vec::new();
    for _ in 0..kora_web::security::STREAMS_PER_IDENTITY {
        let r = open_stream(&w, &vera, REMOTE_PEER).await;
        assert_eq!(r.status(), 200);
        open.push(r);
    }
    let r = open_stream(&w, &vera, REMOTE_PEER).await;
    assert_eq!(r.status(), 429);
    let j: Value = serde_json::from_slice(&axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap()).unwrap();
    assert!(j["error"]["message"].as_str().unwrap().contains("Close some panel tabs"));
    // Someone else is not affected, nor is this computer.
    assert_eq!(open_stream(&w, &olly, "100.64.0.21:40000").await.status(), 200);
    for _ in 0..12 {
        let r = open_stream(&w, LAUNCH, LOOPBACK_PEER).await;
        assert_eq!(r.status(), 200);
        open.push(r);
    }
    // Closing a tab frees its place.
    open.remove(0);
    assert_eq!(open_stream(&w, &vera, REMOTE_PEER).await.status(), 200);
}

/// A program on this computer that connects to the machine's own Tailscale address looks remote, and
/// whois names the machine's owner for it. It must get nothing beyond `/meta`.
#[tokio::test]
async fn the_machines_own_tailscale_address_never_signs_in() {
    const SELF_PEER: &str = "100.64.0.5:40000";
    let mut cfg = enabled_cfg();
    cfg.tailscale = true;
    cfg.tailscale_users.insert("boss@example.com".into(), Role::Admin);
    cfg.tailscale_users.insert("alice@example.com".into(), Role::Operator);
    let admin = cfg.add_user("adam", Role::Admin).unwrap();
    let who = FakeWhois::with("100.64.0.5", "boss@example.com");
    who.logins.lock().unwrap().push(("100.64.0.20".into(), "alice@example.com".into()));
    let w = world_with(Remote::new(None, cfg).with_whois(who.clone()));
    w.remote.set_self_ips(vec!["100.64.0.5".parse().unwrap(), "fd7a:115c:a1e0::5".parse().unwrap()]);
    assert!(w.remote.is_self_ip("::ffff:100.64.0.5".parse().unwrap()), "compared in canonical form");
    assert!(!w.remote.is_self_ip("100.64.0.20".parse().unwrap()));

    let r = req("POST", "/api/v1/session").peer(SELF_PEER).h("x-kora", "1").json(json!({})).send(&w.app).await;
    assert_eq!(r.status, 403, "{:?}", r.json);
    assert!(r.msg().contains("another device") && r.msg().contains("kora panel"), "{}", r.msg());
    assert!(r.cookie.is_none());
    assert!(who.asked.lock().unwrap().is_empty(), "Tailscale is not even asked");
    // A user token through that address is refused too.
    let r = req("GET", "/api/v1/servers").peer(SELF_PEER).bearer(&admin).send(&w.app).await;
    assert_eq!(r.status, 403);
    let r = req("POST", "/api/v1/session").peer(SELF_PEER).h("x-kora", "1").json(json!({"token": admin})).send(&w.app).await;
    assert_eq!(r.status, 403);
    // The page can still load and explain.
    let r = req("GET", "/api/v1/meta").peer(SELF_PEER).send(&w.app).await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json["identity"], Value::Null);

    // Another device on the tailnet still signs in as before.
    let r = req("POST", "/api/v1/session").h("x-kora", "1").json(json!({})).send(&w.app).await;
    assert_eq!(r.status, 200, "{:?}", r.json);
    assert_eq!(r.json["identity"]["name"], "alice@example.com");
    let r = req("GET", "/api/v1/servers").bearer(&admin).send(&w.app).await;
    assert_eq!(r.status, 200);
}

/// Guessing user tokens on ordinary requests (`Authorization: Bearer`) is counted per address, like at
/// sign-in: after ten misses that address is refused, even with a right token; others are not affected.
#[tokio::test]
async fn bearer_guesses_lock_out_only_that_address() {
    let (w, [_, _, admin]) = with_users(enabled_cfg());
    let r = req("GET", "/api/v1/servers").bearer(&admin).send(&w.app).await;
    assert_eq!(r.status, 200, "normal use");
    for i in 0..10 {
        let r = req("GET", "/api/v1/servers").bearer(&format!("guess{i}")).send(&w.app).await;
        assert_eq!(r.status, 401, "try {i}");
    }
    let r = req("GET", "/api/v1/servers").bearer(&admin).send(&w.app).await;
    assert_eq!(r.status, 429, "locked, even with the right token");
    let r = req("POST", "/api/v1/session").h("x-kora", "1").json(json!({"token": admin})).send(&w.app).await;
    assert_eq!(r.status, 429, "sign-in shares the count");
    let r = req("GET", "/api/v1/servers").peer("100.64.0.21:5000").bearer(&admin).send(&w.app).await;
    assert_eq!(r.status, 200, "another address is fine");
    assert!(w.remote.limiter.locked(REMOTE_PEER.parse::<SocketAddr>().unwrap().ip()));
}

#[tokio::test]
async fn panel_restart_is_local_and_only_for_the_service() {
    use kora_web::handlers::system::seam;
    let (w, [_, _, adam]) = with_users(enabled_cfg());
    // A remote admin cannot restart the panel.
    assert_eq!(status(&w, &adam, "POST", "/api/v1/panel/restart", None).await, 403);
    // Not the background service: 409 with a plain message.
    seam::set_can_restart(Some(false));
    let r = req("POST", "/api/v1/panel/restart").peer(LOOPBACK_PEER).bearer(LAUNCH).send(&w.app).await;
    assert_eq!(r.status, 409, "{:?}", r.json);
    assert!(r.msg().contains("background service"), "{}", r.msg());
    let r = req("GET", "/api/v1/system").peer(LOOPBACK_PEER).bearer(LAUNCH).send(&w.app).await;
    assert_eq!(r.json["can_restart"], json!(false));
    // The service: 202, and a restart is asked for.
    seam::set_can_restart(Some(true));
    let before = seam::restarts();
    let r = req("POST", "/api/v1/panel/restart").peer(LOOPBACK_PEER).bearer(LAUNCH).send(&w.app).await;
    assert_eq!(r.status, 202, "{:?}", r.json);
    assert_eq!(r.json, json!({"restarting": true}));
    assert_eq!(seam::restarts(), before + 1);
    let r = req("GET", "/api/v1/system").peer(LOOPBACK_PEER).bearer(LAUNCH).send(&w.app).await;
    assert_eq!(r.json["can_restart"], json!(true));
    seam::set_can_restart(None);
}
