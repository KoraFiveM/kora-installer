//! HTTPS rules in the guard (cookie `Secure`, HSTS, the scheme of `Origin`) and the trusted reverse proxy.
//! Requests go through `router()`; the connection's TLS state is what the connection loop would add.
use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, Request, Response};
use kora_core::api::fake::FakeBackend;
use kora_web::remote::proxy::{Honour, ProxyConfig, SECRET_HEADER};
use kora_web::remote::{Remote, RemoteConfig, Role};
use kora_web::tls::serve::Transport;
use kora_web::{router, AppState};
use serde_json::{json, Value};
use tower::ServiceExt;

const BOUND: &str = "192.168.1.5:47120";
const LAUNCH: &str = "launchtoken";

struct W {
    app: axum::Router,
    token: String,
    secret: String,
}

fn world(proxy: bool, with_hsts_cert: bool) -> W {
    kora_core::installer::registry::use_test_home();
    let mut cfg = RemoteConfig { enabled: true, allowed_hosts: vec!["panel.example.com".into()], ..Default::default() };
    let token = cfg.add_user("adam", Role::Admin).unwrap();
    let mut secret = String::new();
    if proxy {
        let (p, s) = ProxyConfig::new(vec!["127.0.0.1".parse().unwrap()], vec![Honour::For, Honour::Proto]);
        cfg.trusted_proxy = Some(p);
        secret = s;
    }
    let fake = FakeBackend::sample();
    let st = AppState::system(LAUNCH.into(), BOUND.parse().unwrap()).with_backend(Arc::new(move || Box::new(fake.clone()))).with_remote(Arc::new(Remote::new(None, cfg)));
    if with_hsts_cert {
        // A certificate "the person provided": HSTS follows it.
        let ca = kora_web::tls::selfsigned::new_ca("t").unwrap();
        let leaf = kora_web::tls::selfsigned::issue_leaf(&ca, &["192.168.1.5".into()]).unwrap();
        let chain = kora_web::tls::certs::parse_chain(leaf.cert_pem.as_bytes()).unwrap();
        let sum = kora_web::tls::certs::summarise(&chain[0]).unwrap();
        let ck = kora_web::tls::certs::certified_key(chain, &kora_web::tls::certs::parse_key(leaf.key_pem.as_bytes()).unwrap()).unwrap();
        st.tls.install(
            ck,
            kora_web::tls::CertInfo {
                source: kora_web::tls::CertSource::Provided,
                names: sum.names,
                not_before: sum.not_before,
                not_after: sum.not_after,
                fingerprint: sum.fingerprint,
                ca_fingerprint: None,
            },
        );
    }
    W { app: router(st), token, secret }
}

struct R {
    method: &'static str,
    uri: &'static str,
    host: &'static str,
    peer: &'static str,
    secure: bool,
    headers: Vec<(&'static str, String)>,
    body: Option<Value>,
}
fn req(method: &'static str, uri: &'static str) -> R {
    R { method, uri, host: BOUND, peer: "192.168.1.20:51000", secure: false, headers: vec![], body: None }
}
impl R {
    fn host(mut self, h: &'static str) -> Self {
        self.host = h;
        self
    }
    fn peer(mut self, p: &'static str) -> Self {
        self.peer = p;
        self
    }
    fn tls(mut self) -> Self {
        self.secure = true;
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
    async fn send(self, app: &axum::Router) -> Response<Body> {
        let mut b = Request::builder().method(self.method).uri(self.uri).header(header::HOST, self.host);
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
        r.extensions_mut().insert(ConnectInfo(self.peer.parse::<SocketAddr>().unwrap()));
        r.extensions_mut().insert(Transport { secure: self.secure });
        app.clone().oneshot(r).await.unwrap()
    }
}
async fn json_of(r: Response<Body>) -> (u16, Value) {
    let s = r.status().as_u16();
    let b = axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap();
    (s, serde_json::from_slice(&b).unwrap_or(Value::Null))
}

fn sign_in(w: &W) -> R {
    req("POST", "/api/v1/session").h("x-kora", "1").json(json!({"token": w.token}))
}

#[tokio::test]
async fn a_secure_connection_gets_a_secure_cookie_and_a_plain_one_does_not() {
    let w = world(false, false);
    let r = sign_in(&w).send(&w.app).await;
    let c = r.headers().get(header::SET_COOKIE).unwrap().to_str().unwrap().to_string();
    assert!(!c.contains("Secure") && c.contains("HttpOnly") && c.contains("SameSite=Strict"), "{c}");
    let r = sign_in(&w).tls().host(BOUND).send(&w.app).await;
    let c = r.headers().get(header::SET_COOKIE).unwrap().to_str().unwrap().to_string();
    assert!(c.contains("; Secure"), "{c}");
    assert!(r.headers().get(header::STRICT_TRANSPORT_SECURITY).is_none(), "no certificate that earns HSTS");
}

#[tokio::test]
async fn hsts_only_over_tls_with_a_provided_certificate() {
    let w = world(false, true);
    let r = req("GET", "/api/v1/meta").tls().send(&w.app).await;
    assert_eq!(r.headers().get(header::STRICT_TRANSPORT_SECURITY).unwrap(), "max-age=31536000");
    let r = req("GET", "/api/v1/meta").send(&w.app).await;
    assert!(r.headers().get(header::STRICT_TRANSPORT_SECURITY).is_none(), "never over plain HTTP");
}

#[tokio::test]
async fn the_origin_scheme_must_match_the_connection() {
    let w = world(false, false);
    let post = |origin: &str, tls: bool| {
        let mut r = sign_in(&w).h("origin", origin);
        r.secure = tls;
        r
    };
    assert_eq!(post("https://192.168.1.5:47120", true).send(&w.app).await.status(), 200);
    assert_eq!(post("http://192.168.1.5:47120", true).send(&w.app).await.status(), 403, "an http page cannot post to the https panel");
    assert_eq!(post("http://192.168.1.5:47120", false).send(&w.app).await.status(), 200);
    assert_eq!(post("https://192.168.1.5:47120", false).send(&w.app).await.status(), 403);
    assert_eq!(post("https://evil.example", true).send(&w.app).await.status(), 403);
}

#[tokio::test]
async fn without_a_trusted_proxy_forwarded_headers_are_still_refused() {
    let w = world(false, false);
    let r = req("GET", "/api/v1/meta").peer("127.0.0.1:5000").h("x-forwarded-for", "203.0.113.9").send(&w.app).await;
    assert_eq!(r.status(), 403);
}

#[tokio::test]
async fn a_trusted_proxy_needs_its_address_and_its_secret() {
    let w = world(true, false);
    let proxied = |r: R| r.peer("127.0.0.1:5000").host("panel.example.com").h("x-forwarded-for", "203.0.113.9").h("x-forwarded-proto", "https");
    // Right address, right secret: the visitor is remote and signs in with a token.
    let r = proxied(sign_in(&w)).h(SECRET_HEADER, &w.secret).send(&w.app).await;
    let c = r.headers().get(header::SET_COOKIE).unwrap().to_str().unwrap().to_string();
    assert_eq!(r.status(), 200);
    assert!(c.contains("; Secure"), "X-Forwarded-Proto https counts as secure: {c}");
    // No secret (a forgetful proxy, or a local program): refused like any forwarded request.
    assert_eq!(proxied(sign_in(&w)).send(&w.app).await.status(), 403);
    // Wrong secret: refused, with a message that says how to fix it.
    let (s, j) = json_of(proxied(sign_in(&w)).h(SECRET_HEADER, "nope").send(&w.app).await).await;
    assert_eq!(s, 403);
    assert!(j["error"]["message"].as_str().unwrap().contains("secret"), "{j}");
    // Right secret from another address: not a proxy at all.
    let r = req("GET", "/api/v1/meta").peer("192.168.1.99:5000").h(SECRET_HEADER, &w.secret).h("x-forwarded-for", "1.2.3.4").send(&w.app).await;
    assert_eq!(r.status(), 403);
    // A Host the settings do not list is refused through the proxy as everywhere.
    let r = proxied(sign_in(&w)).host("evil.example.com").h(SECRET_HEADER, &w.secret).send(&w.app).await;
    assert_eq!(r.status(), 403);
}

#[tokio::test]
async fn a_proxied_visitor_is_never_the_local_owner() {
    let w = world(true, false);
    // The launch token (for the computer itself) does not work for a visitor that came through the proxy,
    // even though the proxy connects from 127.0.0.1 and names a loopback client address.
    let r = req("POST", "/api/v1/session")
        .peer("127.0.0.1:5000")
        .host("panel.example.com")
        .h("x-kora", "1")
        .h("x-forwarded-for", "127.0.0.2")
        .h(SECRET_HEADER, &w.secret)
        .json(json!({"token": LAUNCH}))
        .send(&w.app)
        .await;
    assert_eq!(r.status(), 401, "an invalid user token: the launch token is not a remote credential");
    // And the local-only link maker is out of reach.
    let r = req("POST", "/api/v1/session/link")
        .peer("127.0.0.1:5000")
        .host("panel.example.com")
        .h("x-kora", "1")
        .h("x-forwarded-for", "127.0.0.2")
        .h(SECRET_HEADER, &w.secret)
        .h("authorization", &format!("Bearer {LAUNCH}"))
        .send(&w.app)
        .await;
    assert_ne!(r.status(), 200);
}

#[tokio::test]
async fn lockouts_follow_the_forwarded_address() {
    let w = world(true, false);
    let attempt = |client: &'static str, token: &str| {
        req("POST", "/api/v1/session").peer("127.0.0.1:5000").host("panel.example.com").h("x-kora", "1").h("x-forwarded-for", client).h(SECRET_HEADER, &w.secret).json(json!({"token": token}))
    };
    for _ in 0..10 {
        assert_eq!(attempt("203.0.113.9", "wrong").send(&w.app).await.status(), 401);
    }
    // That address is locked out, even with the right token.
    assert_eq!(attempt("203.0.113.9", &w.token).send(&w.app).await.status(), 429);
    // Another visitor behind the same proxy is not.
    assert_eq!(attempt("198.51.100.7", &w.token).send(&w.app).await.status(), 200);
}
