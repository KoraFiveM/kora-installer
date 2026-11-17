//! txAdmin through the panel: a fake txAdmin (HTTP with redirects, cookies and chunked answers, and a
//! WebSocket echo) on loopback behind the gateway, signed in the way the page does it, plus the negative
//! cases: no session, wrong role, another server's gateway, absolute-form and `Host` tricks, header
//! smuggling, WebSocket origin, and a panel sign-out ending txAdmin access.
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, Request};
use kora_core::api::fake::{entry, FakeBackend};
use kora_core::installer::Mode;
use kora_web::remote::{Remote, RemoteConfig, Role};
use kora_web::txgate::Gate;
use kora_web::{router, AppState};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tower::ServiceExt;

const PANEL: &str = "127.0.0.1:47120";
const LAUNCH: &str = "launchtoken";

// ------------------------------------------------------------------ a fake txAdmin

#[derive(Default)]
struct Tx {
    /// Requests that reached it (not counting the sockets).
    hits: AtomicUsize,
    last_head: Mutex<String>,
}

/// Serves on an ephemeral loopback port; returns the port.
async fn fake_txadmin(label: &'static str) -> (u16, Arc<Tx>) {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    let tx = Arc::new(Tx::default());
    let tx2 = tx.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = l.accept().await else { break };
            let tx = tx2.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                let head_end = loop {
                    let n = s.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break i + 4;
                    }
                };
                let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                tx.hits.fetch_add(1, Ordering::SeqCst);
                *tx.last_head.lock().unwrap() = head.clone();
                let first = head.lines().next().unwrap_or("").to_string();
                let path = first.split_whitespace().nth(1).unwrap_or("/").to_string();
                let lower = head.to_ascii_lowercase();
                let clen: usize = lower.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse().ok()).unwrap_or(0);
                while buf.len() < head_end + clen {
                    let n = s.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                }
                let body_in = String::from_utf8_lossy(&buf[head_end..]).to_string();
                if lower.contains("upgrade: websocket") && path.starts_with("/socket.io") {
                    let _ = s.write_all(b"HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Accept: x\r\n\r\n").await;
                    let _ = s.write_all(format!("hello-from-{label}").as_bytes()).await;
                    loop {
                        let n = s.read(&mut tmp).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        let up: Vec<u8> = tmp[..n].iter().map(u8::to_ascii_uppercase).collect();
                        if s.write_all(&up).await.is_err() {
                            break;
                        }
                    }
                    return;
                }
                let reply = match path.as_str() {
                    "/" => format!(
                        "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{port}/login\r\nSet-Cookie: tx:abc=sess-{label}; Path=/; Domain=evil.example; Secure; HttpOnly; SameSite=None\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    ),
                    "/headers" => {
                        let body = head.clone();
                        format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                    }
                    "/post" => {
                        let body = format!("got {} bytes: {body_in}", body_in.len());
                        format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                    }
                    "/chunks" => {
                        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\nfirst\r\n").await;
                        let _ = s.flush().await;
                        tokio::time::sleep(Duration::from_millis(400)).await;
                        let _ = s.write_all(b"6\r\nsecond\r\n0\r\n\r\n").await;
                        return;
                    }
                    "/who" => {
                        let body = format!("txadmin {label}");
                        format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                    }
                    "/login" => {
                        let body = "login page";
                        format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                    }
                    _ => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string(),
                };
                let _ = s.write_all(reply.as_bytes()).await;
            });
        }
    });
    (port, tx)
}

// ------------------------------------------------------------------ the panel

struct World {
    app: axum::Router,
    st: AppState,
    prod_tx: Arc<Tx>,
    dev_tx: Arc<Tx>,
    prod_port: u16,
}

async fn world() -> World {
    let (prod_port, prod_tx) = fake_txadmin("prod").await;
    let (dev_port, dev_tx) = fake_txadmin("dev").await;
    let fake = FakeBackend::new();
    {
        let mut s = fake.state();
        s.servers = vec![entry("prod", Mode::Production, 30120, prod_port), entry("dev", Mode::Development, 30130, dev_port)];
    }
    let mut st = AppState::system(LAUNCH.into(), PANEL.parse().unwrap()).with_backend(Arc::new(move || Box::new(fake.clone())));
    // Gateway ports are chosen by the operating system here.
    st.txgate = Arc::new(Gate::default().with_port_fn(|_| Some(0)).with_ws_recheck(Duration::from_millis(150)));
    World { app: router(st.clone()), st, prod_tx, dev_tx, prod_port }
}

struct Session {
    cookie: String,
    secret: String,
}

async fn sign_in(app: &axum::Router) -> Session {
    let r = app
        .clone()
        .oneshot(Request::builder().method("POST").uri("/api/v1/session").header(header::HOST, PANEL).header("x-kora", "1").header(header::CONTENT_TYPE, "application/json").body(Body::from(json!({"token": LAUNCH}).to_string())).unwrap())
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let cookie = r.headers().get_all(header::SET_COOKIE).iter().filter_map(|v| v.to_str().ok()).find(|c| c.starts_with("kora_session_47120=")).unwrap().split(';').next().unwrap().to_string();
    let b = axum::body::to_bytes(r.into_body(), 1 << 16).await.unwrap();
    let j: Value = serde_json::from_slice(&b).unwrap();
    Session { cookie, secret: j["secret"].as_str().unwrap().to_string() }
}

async fn ticket(w: &World, s: &Session, server: &str) -> (u16, Value) {
    let r = w
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/servers/{server}/txadmin/ticket"))
                .header(header::HOST, PANEL)
                .header(header::COOKIE, &s.cookie)
                .header("x-kora-session", &s.secret)
                .header("x-kora", "1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = r.status().as_u16();
    let b = axum::body::to_bytes(r.into_body(), 1 << 16).await.unwrap();
    (status, serde_json::from_slice(&b).unwrap_or(Value::Null))
}

// ------------------------------------------------------------------ a raw HTTP client

struct Answer {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Answer {
    fn header(&self, n: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.as_str())
    }
    fn all(&self, n: &str) -> Vec<&str> {
        self.headers.iter().filter(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.as_str()).collect()
    }
}

async fn raw(port: u16, request: &str) -> Answer {
    let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    s.write_all(request.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut buf)).await;
    parse(&buf)
}

fn parse(buf: &[u8]) -> Answer {
    let text = String::from_utf8_lossy(buf).to_string();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let mut lines = head.lines();
    let status = lines.next().and_then(|l| l.split_whitespace().nth(1)).and_then(|c| c.parse().ok()).unwrap_or(0);
    let headers = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect();
    Answer { status, headers, body: body.to_string() }
}

fn get(path: &str, port: u16, extra: &str) -> String {
    format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n{extra}\r\n")
}

fn gate_port(url: &str) -> u16 {
    url.split("://").nth(1).unwrap().split('/').next().unwrap().rsplit(':').next().unwrap().parse().unwrap()
}

fn enter_path(url: &str) -> String {
    format!("/{}", url.split("://").nth(1).unwrap().split_once('/').unwrap().1)
}

/// Open txAdmin for `server` and return the gateway port and the gateway cookie (`name=value`).
async fn open(w: &World, s: &Session, server: &str) -> (u16, String) {
    let (st, j) = ticket(w, s, server).await;
    assert_eq!(st, 200, "{j}");
    let url = j["url"].as_str().unwrap().to_string();
    let port = gate_port(&url);
    let a = raw(port, &get(&enter_path(&url), port, "")).await;
    assert_eq!(a.status, 303, "{}", a.body);
    assert_eq!(a.header("location"), Some("/"));
    let c = a.header("set-cookie").unwrap();
    assert!(c.contains("HttpOnly") && c.contains("SameSite=Lax") && c.contains("Path=/"), "{c}");
    (port, c.split(';').next().unwrap().to_string())
}

// ------------------------------------------------------------------ tests

#[tokio::test]
async fn ticket_enter_then_proxied_with_cookies_and_redirects_rewritten() {
    let w = world().await;
    let s = sign_in(&w.app).await;
    let (port, gate) = open(&w, &s, "prod").await;
    assert_eq!(w.prod_tx.hits.load(Ordering::SeqCst), 0, "entering reaches no txAdmin");

    // txAdmin's redirect to its own address becomes a path; its cookie is renamed and scoped.
    let a = raw(port, &get("/", port, &format!("Cookie: {gate}\r\n"))).await;
    assert_eq!(a.status, 302);
    assert_eq!(a.header("location"), Some("/login"));
    let sc = a.all("set-cookie");
    assert_eq!(sc.len(), 1, "{sc:?}");
    assert_eq!(sc[0], format!("kora-tx-{port}-tx:abc=sess-prod; Path=/; SameSite=Lax; HttpOnly"));

    // The browser sends it back with the prefix; txAdmin sees its own name again and nothing else.
    let ck = format!("Cookie: {gate}; kora_session_47120={}; kora-tx-{port}-tx:abc=sess-prod; other=1\r\n", s.secret);
    let a = raw(port, &get("/headers", port, &ck)).await;
    assert_eq!(a.status, 200);
    let seen = a.body.to_ascii_lowercase();
    assert!(seen.contains("cookie: tx:abc=sess-prod\r\n"), "{seen}");
    assert!(!seen.contains("kora_txgate") && !seen.contains("kora_session") && !seen.contains(&s.secret) && !seen.contains("other=1"), "{seen}");
    // The ticket worked once.
    let (_, j) = ticket(&w, &s, "prod").await;
    let url = j["url"].as_str().unwrap().to_string();
    let again = raw(port, &get(&enter_path(&url), port, "")).await;
    assert_eq!(again.status, 303);
    let reuse = raw(port, &get(&enter_path(&url), port, "")).await;
    assert_eq!(reuse.status, 401, "a ticket is single use");
}

#[tokio::test]
async fn bodies_stream_both_ways() {
    let w = world().await;
    let s = sign_in(&w.app).await;
    let (port, gate) = open(&w, &s, "prod").await;
    let req = format!("POST /post HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nCookie: {gate}\r\nContent-Length: 11\r\nConnection: close\r\n\r\nhello world");
    let a = raw(port, &req).await;
    assert_eq!(a.status, 200);
    assert_eq!(a.body, "got 11 bytes: hello world");

    // A chunked answer reaches the browser as it is made, not after txAdmin is done.
    let mut c = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    c.write_all(get("/chunks", port, &format!("Cookie: {gate}\r\n")).as_bytes()).await.unwrap();
    let t0 = Instant::now();
    let mut seen = Vec::new();
    let mut tmp = [0u8; 1024];
    let mut first_at = None;
    loop {
        let n = tokio::time::timeout(Duration::from_secs(5), c.read(&mut tmp)).await.unwrap().unwrap();
        if n == 0 {
            break;
        }
        seen.extend_from_slice(&tmp[..n]);
        if first_at.is_none() && String::from_utf8_lossy(&seen).contains("first") {
            first_at = Some(t0.elapsed());
        }
    }
    let first_at = first_at.expect("first chunk arrived");
    assert!(first_at < Duration::from_millis(300), "the first chunk waited for the whole answer: {first_at:?}");
    assert!(String::from_utf8_lossy(&seen).contains("second"));
}

#[tokio::test]
async fn nothing_without_a_session_reaches_txadmin() {
    let w = world().await;
    let s = sign_in(&w.app).await;
    let (port, gate) = open(&w, &s, "prod").await;
    for extra in ["", "Cookie: nothing=1\r\n", "Cookie: kora_txgate_1=abc\r\n"] {
        let a = raw(port, &get("/who", port, extra)).await;
        assert_eq!(a.status, 401, "{extra}");
    }
    // a forged gateway cookie with the right name
    let a = raw(port, &get("/who", port, &format!("Cookie: kora_txgate_{port}=deadbeef\r\n"))).await;
    assert_eq!(a.status, 401);
    assert_eq!(w.prod_tx.hits.load(Ordering::SeqCst), 0);
    // while the real one works
    assert_eq!(raw(port, &get("/who", port, &format!("Cookie: {gate}\r\n"))).await.body, "txadmin prod");
}

#[tokio::test]
async fn the_ticket_needs_a_signed_in_browser_session_and_the_admin_role() {
    let w = world().await;
    // The panel's own guard: no session at all.
    let r = w.app.clone().oneshot(Request::builder().method("POST").uri("/api/v1/servers/prod/txadmin/ticket").header(header::HOST, PANEL).header("x-kora", "1").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(r.status(), 401);
    // A cookie without its secret header is no session either.
    let s = sign_in(&w.app).await;
    let r = w.app.clone().oneshot(Request::builder().method("POST").uri("/api/v1/servers/prod/txadmin/ticket").header(header::HOST, PANEL).header(header::COOKIE, &s.cookie).header("x-kora", "1").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(r.status(), 401);
    // The launch token as a bearer is a valid caller but has no browser session to follow.
    let r = w.app.clone().oneshot(Request::builder().method("POST").uri("/api/v1/servers/prod/txadmin/ticket").header(header::HOST, PANEL).header(header::AUTHORIZATION, format!("Bearer {LAUNCH}")).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(r.status(), 403);
    // Unknown server.
    let (st, _) = ticket(&w, &s, "nope").await;
    assert_eq!(st, 404);
}

#[tokio::test]
async fn operators_viewers_and_token_admins_without_the_switch_are_refused() {
    let mut cfg = RemoteConfig { enabled: true, ..Default::default() };
    let tokens = [("vera", Role::Viewer), ("olly", Role::Operator), ("adam", Role::Admin)].map(|(n, r)| (n, cfg.add_user(n, r).unwrap()));
    let (prod_port, _tx) = fake_txadmin("prod").await;
    let fake = FakeBackend::new();
    fake.state().servers = vec![entry("prod", Mode::Production, 30120, prod_port)];
    let st = AppState::system(LAUNCH.into(), "100.64.0.5:47120".parse().unwrap()).with_backend(Arc::new(move || Box::new(fake.clone()))).with_remote(Arc::new(Remote::new(None, cfg)));
    let app = router(st);
    let peer: SocketAddr = "100.64.0.20:51000".parse().unwrap();
    for (name, tok) in tokens {
        let mut r = Request::builder()
            .method("POST")
            .uri("/api/v1/session")
            .header(header::HOST, "100.64.0.5:47120")
            .header("x-kora", "1")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({"token": tok}).to_string()))
            .unwrap();
        r.extensions_mut().insert(ConnectInfo(peer));
        let resp = app.clone().oneshot(r).await.unwrap();
        assert_eq!(resp.status(), 200, "{name}");
        let cookie = resp.headers().get_all(header::SET_COOKIE).iter().filter_map(|v| v.to_str().ok()).find(|c| c.starts_with("kora_session_47120=")).unwrap().split(';').next().unwrap().to_string();
        let j: Value = serde_json::from_slice(&axum::body::to_bytes(resp.into_body(), 1 << 16).await.unwrap()).unwrap();
        let secret = j["secret"].as_str().unwrap().to_string();
        let mut r = Request::builder()
            .method("POST")
            .uri("/api/v1/servers/prod/txadmin/ticket")
            .header(header::HOST, "100.64.0.5:47120")
            .header(header::COOKIE, cookie)
            .header("x-kora-session", secret)
            .header("x-kora", "1")
            .body(Body::empty())
            .unwrap();
        r.extensions_mut().insert(ConnectInfo(peer));
        let resp = app.clone().oneshot(r).await.unwrap();
        // Viewer and operator: not allowed. Admin on a token: not until the switch is on.
        assert_eq!(resp.status(), 403, "{name}");
        let text = String::from_utf8_lossy(&axum::body::to_bytes(resp.into_body(), 1 << 16).await.unwrap()).to_string();
        if name == "adam" {
            assert!(text.contains("remote_actions_off") || text.contains("turned off"), "{text}");
        }
    }
}

#[tokio::test]
async fn another_servers_gateway_and_foreign_targets_are_not_reachable() {
    let w = world().await;
    let s = sign_in(&w.app).await;
    let (prod_gw, prod_cookie) = open(&w, &s, "prod").await;
    let (dev_gw, dev_cookie) = open(&w, &s, "dev").await;
    assert_ne!(prod_gw, dev_gw);
    assert_eq!(raw(prod_gw, &get("/who", prod_gw, &format!("Cookie: {prod_cookie}\r\n"))).await.body, "txadmin prod");
    assert_eq!(raw(dev_gw, &get("/who", dev_gw, &format!("Cookie: {dev_cookie}\r\n"))).await.body, "txadmin dev");

    // prod's cookie value under dev's cookie name: the session belongs to another server.
    let prod_id = prod_cookie.split_once('=').unwrap().1;
    let a = raw(dev_gw, &get("/who", dev_gw, &format!("Cookie: kora_txgate_{dev_gw}={prod_id}\r\n"))).await;
    assert_eq!(a.status, 401);
    // prod's ticket on dev's port: refused.
    let (_, j) = ticket(&w, &s, "prod").await;
    let url = j["url"].as_str().unwrap();
    let a = raw(dev_gw, &get(&enter_path(url), dev_gw, "")).await;
    assert_eq!(a.status, 401);

    // Absolute-form target naming the OTHER txAdmin: refused, and dev's txAdmin is not touched.
    let before = w.dev_tx.hits.load(Ordering::SeqCst);
    let evil = format!("GET http://127.0.0.1:{}/who HTTP/1.1\r\nHost: 127.0.0.1:{prod_gw}\r\nCookie: {prod_cookie}\r\nConnection: close\r\n\r\n", 1);
    let a = raw(prod_gw, &evil).await;
    assert_eq!(a.status, 400);
    assert_eq!(w.dev_tx.hits.load(Ordering::SeqCst), before);
    // A `Host` for somewhere else.
    let a = raw(prod_gw, &format!("GET /who HTTP/1.1\r\nHost: evil.example:{prod_gw}\r\nCookie: {prod_cookie}\r\nConnection: close\r\n\r\n")).await;
    assert_eq!(a.status, 403);
    let a = raw(prod_gw, &format!("GET /who HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nCookie: {prod_cookie}\r\nConnection: close\r\n\r\n", prod_gw + 1)).await;
    assert_eq!(a.status, 403);
    // A path that tries to climb stays a path on prod's txAdmin (it answers 404, nothing else is reached).
    let a = raw(prod_gw, &get("/../../who", prod_gw, &format!("Cookie: {prod_cookie}\r\n"))).await;
    assert!(a.status == 404 || a.status == 200 && a.body == "txadmin prod", "{} {}", a.status, a.body);
    assert_eq!(w.dev_tx.hits.load(Ordering::SeqCst), before);
    let _ = w.prod_port;
}

#[tokio::test]
async fn smuggling_attempts_are_dropped_or_refused() {
    let w = world().await;
    let s = sign_in(&w.app).await;
    let (port, gate) = open(&w, &s, "prod").await;
    // Headers txAdmin trusts or the panel refuses: the first group never arrives, the second is refused.
    let extra = format!(
        "Cookie: {gate}\r\nX-TxAdmin-Token: guess\r\nX-TxAdmin-Identifiers: license:1\r\nX-TxAdmin-EnvToken: x\r\nAuthorization: Bearer {LAUNCH}\r\nX-Kora-Session: {}\r\nX-Kora: 1\r\nConnection: close, X-Listed\r\nX-Listed: me\r\nX-TxAdmin-CsrfToken: keep\r\n",
        s.secret
    );
    let a = raw(port, &format!("GET /headers HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{extra}\r\n")).await;
    assert_eq!(a.status, 200, "{}", a.body);
    let seen = a.body.to_ascii_lowercase();
    for gone in ["x-txadmin-token", "x-txadmin-identifiers", "x-txadmin-envtoken", "authorization", "x-kora", "x-listed", &s.secret, LAUNCH] {
        assert!(!seen.contains(&gone.to_ascii_lowercase()), "{gone} reached txAdmin: {seen}");
    }
    assert!(seen.contains("x-txadmin-csrftoken: keep"), "the web page's own header passes: {seen}");
    for h in ["X-Forwarded-For: 1.2.3.4", "Forwarded: for=1.2.3.4", "X-Real-IP: 1.2.3.4", "Tailscale-User-Login: x@y"] {
        let a = raw(port, &get("/who", port, &format!("Cookie: {gate}\r\n{h}\r\n"))).await;
        assert_eq!(a.status, 403, "{h}");
    }
    let hits = w.prod_tx.hits.load(Ordering::SeqCst);
    // Both a length and chunked framing, or a doubled length: the parser refuses, nothing is forwarded.
    let a = raw(port, &format!("POST /post HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nCookie: {gate}\r\nContent-Length: 5\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n0\r\n\r\nGET /who HTTP/1.1\r\n\r\n")).await;
    assert!(a.status == 400 || a.status == 0, "{}", a.status);
    let a = raw(port, &format!("POST /post HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nCookie: {gate}\r\nContent-Length: 5\r\nContent-Length: 6\r\nConnection: close\r\n\r\nhello!")).await;
    assert!(a.status == 400 || a.status == 0, "{}", a.status);
    // A space before the colon (a classic) and a bare line feed.
    let a = raw(port, &format!("GET /who HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nCookie: {gate}\r\nX-Evil : 1\r\nConnection: close\r\n\r\n")).await;
    assert!(a.status == 400 || a.status == 0, "{}", a.status);
    assert_eq!(w.prod_tx.hits.load(Ordering::SeqCst), hits, "no smuggled request got through");
    // An unsupported upgrade is not passed on either.
    let a = raw(port, &get("/who", port, &format!("Cookie: {gate}\r\nConnection: Upgrade\r\nUpgrade: h2c\r\n"))).await;
    assert_eq!(a.status, 400);
    // A cross-site POST (Origin of another page) is refused.
    let a = raw(port, &format!("POST /post HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nCookie: {gate}\r\nOrigin: http://evil.example\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")).await;
    assert_eq!(a.status, 403);
}

#[tokio::test]
async fn websocket_echo_origin_check_and_sign_out() {
    let w = world().await;
    let s = sign_in(&w.app).await;
    let (port, gate) = open(&w, &s, "prod").await;
    let handshake = |origin: Option<&str>, cookie: &str| {
        let o = origin.map(|o| format!("Origin: {o}\r\n")).unwrap_or_default();
        format!("GET /socket.io/?EIO=4&transport=websocket HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n{o}{cookie}\r\n")
    };
    // Wrong origin, no origin, no session: all refused before txAdmin.
    let hits = w.prod_tx.hits.load(Ordering::SeqCst);
    assert_eq!(raw(port, &handshake(Some("http://evil.example"), &format!("Cookie: {gate}\r\n"))).await.status, 403);
    assert_eq!(raw(port, &handshake(None, &format!("Cookie: {gate}\r\n"))).await.status, 403);
    assert_eq!(raw(port, &handshake(Some(&format!("http://127.0.0.1:{port}")), "")).await.status, 401);
    assert_eq!(w.prod_tx.hits.load(Ordering::SeqCst), hits);

    // The real one: 101, then bytes both ways.
    let mut c = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    c.write_all(handshake(Some(&format!("http://127.0.0.1:{port}")), &format!("Cookie: {gate}\r\n")).as_bytes()).await.unwrap();
    let mut seen = Vec::new();
    let mut tmp = [0u8; 1024];
    while !String::from_utf8_lossy(&seen).contains("hello-from-prod") {
        let n = tokio::time::timeout(Duration::from_secs(5), c.read(&mut tmp)).await.expect("handshake answer").unwrap();
        assert!(n > 0, "closed early: {}", String::from_utf8_lossy(&seen));
        seen.extend_from_slice(&tmp[..n]);
    }
    let text = String::from_utf8_lossy(&seen).to_string();
    assert!(text.starts_with("HTTP/1.1 101"), "{text}");
    assert!(text.to_ascii_lowercase().contains("upgrade: websocket"), "{text}");
    c.write_all(b"ping from the browser").await.unwrap();
    let mut echoed = Vec::new();
    while !String::from_utf8_lossy(&echoed).contains("PING FROM THE BROWSER") {
        let n = tokio::time::timeout(Duration::from_secs(5), c.read(&mut tmp)).await.expect("echo").unwrap();
        assert!(n > 0);
        echoed.extend_from_slice(&tmp[..n]);
    }

    // Signing out of the panel closes the live connection and ends the access.
    let r = w.app.clone().oneshot(Request::builder().method("POST").uri("/api/v1/session/logout").header(header::HOST, PANEL).header(header::COOKIE, &s.cookie).header("x-kora-session", &s.secret).header("x-kora", "1").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(r.status(), 204);
    let n = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match c.read(&mut tmp).await {
                Ok(0) | Err(_) => break,
                Ok(_) => continue,
            }
        }
    })
    .await;
    assert!(n.is_ok(), "the live connection was not closed after sign-out");
    assert_eq!(raw(port, &get("/who", port, &format!("Cookie: {gate}\r\n"))).await.status, 401);
}

#[tokio::test]
async fn a_server_that_is_down_gets_a_plain_page_not_a_hang() {
    let w = world().await;
    let s = sign_in(&w.app).await;
    // Port 1 on loopback: nothing listens.
    {
        let fake = FakeBackend::new();
        fake.state().servers = vec![entry("prod", Mode::Production, 30120, 1)];
        let mut st = AppState::system(LAUNCH.into(), PANEL.parse().unwrap()).with_backend(Arc::new(move || Box::new(fake.clone())));
        st.txgate = Arc::new(Gate::default().with_port_fn(|_| Some(0)));
        let app = router(st);
        let s2 = sign_in(&app).await;
        let w2 = World { app, st: w.st.clone(), prod_tx: w.prod_tx.clone(), dev_tx: w.dev_tx.clone(), prod_port: 1 };
        let (port, gate) = open(&w2, &s2, "prod").await;
        let a = raw(port, &get("/", port, &format!("Cookie: {gate}\r\n"))).await;
        assert_eq!(a.status, 502);
        assert!(a.body.contains("Is the server running"), "{}", a.body);
    }
    let _ = s;
}
