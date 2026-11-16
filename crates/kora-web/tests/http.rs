use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request, Response, StatusCode};
use kora_web::{router, AppState, MemoryJobs};
use serde_json::{json, Value};
use tower::ServiceExt;

const ADDR: &str = "127.0.0.1:47120";
const TOKEN: &str = "testtoken";

fn setup() -> (axum::Router, Arc<MemoryJobs>) {
    let jobs = Arc::new(MemoryJobs::new());
    let st = AppState::new(
        TOKEN.into(),
        ADDR.parse::<SocketAddr>().unwrap(),
        jobs.clone(),
    );
    (router(st), jobs)
}

fn req(method: &str, uri: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, ADDR)
}

async fn body(r: Response<Body>) -> Vec<u8> {
    axum::body::to_bytes(r.into_body(), 1 << 22)
        .await
        .unwrap()
        .to_vec()
}
async fn json_of(r: Response<Body>) -> Value {
    serde_json::from_slice(&body(r).await).unwrap()
}

/// A browser session: the cookie (`name=value`) and the secret the page sends with it.
struct Session {
    cookie: String,
    secret: String,
}

async fn sign_in_with(app: &axum::Router, body: Value) -> Response<Body> {
    app.clone()
        .oneshot(
            req("POST", "/api/v1/session")
                .header("x-kora", "1")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn session_of(r: Response<Body>) -> Session {
    assert_eq!(r.status(), 200);
    let cookies: Vec<String> = r
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|c| c.to_str().unwrap().to_string())
        .collect();
    let c = &cookies[0];
    assert!(c.starts_with("kora_session_47120="), "{c}");
    assert!(c.contains("HttpOnly") && c.contains("SameSite=Strict") && c.contains("Path=/"));
    assert!(!c.contains("Secure") && !c.contains("Max-Age"), "a browser-session cookie");
    // The old shared name is cleared.
    assert!(cookies.iter().any(|c| c.starts_with("kora_session=;") && c.contains("Max-Age=0")));
    let cookie = c.split(';').next().unwrap().to_string();
    let j = json_of(r).await;
    let secret = j["secret"].as_str().unwrap().to_string();
    assert_eq!(secret.len(), 64);
    assert!(!cookie.contains(&secret));
    Session { cookie, secret }
}

async fn login(app: &axum::Router) -> Session {
    session_of(sign_in_with(app, json!({"token": TOKEN})).await).await
}

/// A GET with the session cookie and, if given, the secret header.
async fn get_with(app: &axum::Router, uri: &str, cookie: &str, secret: Option<&str>) -> Response<Body> {
    let mut b = req("GET", uri).header(header::COOKIE, cookie);
    if let Some(s) = secret {
        b = b.header("x-kora-session", s);
    }
    app.clone().oneshot(b.body(Body::empty()).unwrap()).await.unwrap()
}

async fn bad_login(app: &axum::Router, tok: &str) -> StatusCode {
    app.clone()
        .oneshot(
            req("POST", "/api/v1/session")
                .header("x-kora", "1")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"token": tok}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn meta_without_auth() {
    let (app, _) = setup();
    let r = app
        .oneshot(req("GET", "/api/v1/meta").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()[header::CACHE_CONTROL], "no-store");
    let j = json_of(r).await;
    assert_eq!(j["api"], 1);
    assert_eq!(j["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(j["platform"], if cfg!(windows) { "windows" } else { "linux" });
    assert!(j["ops"].as_array().unwrap().len() > 10);
    assert!(j["node"].is_string());
}

#[tokio::test]
async fn api_requires_auth() {
    let (app, _) = setup();
    let r = app
        .oneshot(req("GET", "/api/v1/jobs/x").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    let j = json_of(r).await;
    assert_eq!(j["error"]["code"], "unauthorized");
}

#[tokio::test]
async fn session_then_authorised() {
    let (app, jobs) = setup();
    jobs.create("j1");
    let s = login(&app).await;
    let r = get_with(&app, "/api/v1/jobs/j1", &s.cookie, Some(&s.secret)).await;
    assert_eq!(r.status(), 200);
    assert_eq!(json_of(r).await["state"], "running");
}

#[tokio::test]
async fn a_cookie_alone_is_useless() {
    let (app, jobs) = setup();
    jobs.create("j1");
    let s = login(&app).await;
    // What txAdmin or a FiveM resource on the same host could see: the cookie, not the secret.
    assert_eq!(get_with(&app, "/api/v1/jobs/j1", &s.cookie, None).await.status(), 401);
    assert_eq!(get_with(&app, "/api/v1/jobs/j1", &s.cookie, Some("0000")).await.status(), 401);
    let other = login(&app).await;
    assert_eq!(get_with(&app, "/api/v1/jobs/j1", &s.cookie, Some(&other.secret)).await.status(), 401, "another session's secret");
    // The old cookie name is not read at all.
    let old = s.cookie.replace("kora_session_47120=", "kora_session=");
    assert_eq!(get_with(&app, "/api/v1/jobs/j1", &old, Some(&s.secret)).await.status(), 401);
    // Nor a cookie meant for a panel on another port.
    let port = s.cookie.replace("kora_session_47120=", "kora_session_47121=");
    assert_eq!(get_with(&app, "/api/v1/jobs/j1", &port, Some(&s.secret)).await.status(), 401);
    // The failed tries did not end the session.
    assert_eq!(get_with(&app, "/api/v1/jobs/j1", &s.cookie, Some(&s.secret)).await.status(), 200);
    // A stream cannot set headers: `?ks=` works for reads...
    let uri = format!("/api/v1/jobs/j1?ks={}", s.secret);
    assert_eq!(get_with(&app, &uri, &s.cookie, None).await.status(), 200);
    assert_eq!(get_with(&app, "/api/v1/jobs/j1?ks=nope", &s.cookie, None).await.status(), 401);
    // ...but never for a change.
    let r = app
        .clone()
        .oneshot(
            req("POST", &format!("/api/v1/session/logout?ks={}", s.secret))
                .header(header::COOKIE, &s.cookie)
                .header("x-kora", "1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    // With the header, logout works and the session is gone.
    let r = app
        .clone()
        .oneshot(
            req("POST", "/api/v1/session/logout")
                .header(header::COOKIE, &s.cookie)
                .header("x-kora-session", &s.secret)
                .header("x-kora", "1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let cleared = r.headers()[header::SET_COOKIE].to_str().unwrap();
    assert!(cleared.starts_with("kora_session_47120=;") && cleared.contains("Max-Age=0"), "{cleared}");
    assert_eq!(get_with(&app, "/api/v1/jobs/j1", &s.cookie, Some(&s.secret)).await.status(), 401);
}

async fn mint(app: &axum::Router, auth: (&str, String)) -> Response<Body> {
    app.clone()
        .oneshot(
            req("POST", "/api/v1/session/link")
                .header(auth.0, auth.1)
                .header("x-kora", "1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn sign_in_links_use_one_time_codes() {
    let (app, jobs) = setup();
    jobs.create("j1");
    // Not signed in: no code.
    let r = app
        .clone()
        .oneshot(req("POST", "/api/v1/session/link").header("x-kora", "1").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    // The CLI asks with the launch token.
    let r = mint(&app, ("authorization", format!("Bearer {TOKEN}"))).await;
    assert_eq!(r.status(), 200);
    let j = json_of(r).await;
    assert_eq!(j["expires_in"], 600);
    let code = j["code"].as_str().unwrap().to_string();
    assert_eq!(code.len(), 32);
    // Opening the link signs in, once.
    let s = session_of(sign_in_with(&app, json!({"code": code})).await).await;
    assert_eq!(get_with(&app, "/api/v1/jobs/j1", &s.cookie, Some(&s.secret)).await.status(), 200);
    let r = sign_in_with(&app, json!({"code": code})).await;
    assert_eq!(r.status(), 401);
    assert!(json_of(r).await["error"]["message"].as_str().unwrap().contains("already used"));
    // The code is not a token, and the token is not a code.
    assert_eq!(sign_in_with(&app, json!({"token": code})).await.status(), 401);
    assert_eq!(sign_in_with(&app, json!({"code": TOKEN})).await.status(), 401);
    // A local browser session can make one too (with its secret).
    let r = app
        .clone()
        .oneshot(
            req("POST", "/api/v1/session/link")
                .header(header::COOKIE, &s.cookie)
                .header("x-kora-session", &s.secret)
                .header("x-kora", "1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    // The paste box still takes the launch token.
    login(&app).await;
}

#[tokio::test]
async fn bad_codes_count_towards_the_lockout() {
    let (app, _) = setup();
    for _ in 0..10 {
        assert_eq!(sign_in_with(&app, json!({"code": "guess"})).await.status(), 401);
    }
    assert_eq!(sign_in_with(&app, json!({"token": TOKEN})).await.status(), 429);
}

#[tokio::test]
async fn mint_link_code_works_in_process() {
    let st = AppState::new(TOKEN.into(), ADDR.parse().unwrap(), Arc::new(MemoryJobs::new()));
    let code = st.mint_link_code();
    let app = router(st);
    session_of(sign_in_with(&app, json!({"code": code})).await).await;
}

#[tokio::test]
async fn requests_through_a_proxy_are_refused() {
    let (app, _) = setup();
    for h in [
        "x-forwarded-for",
        "x-forwarded-host",
        "x-forwarded-proto",
        "forwarded",
        "x-real-ip",
        "cf-connecting-ip",
        "true-client-ip",
        "tailscale-user-login",
        "tailscale-user-name",
    ] {
        for uri in ["/api/v1/meta", "/", "/api/v1/servers"] {
            let r = app
                .clone()
                .oneshot(
                    req("GET", uri)
                        .header(h, "1.2.3.4")
                        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(r.status(), 403, "{h} on {uri}");
            let m = json_of(r).await["error"]["message"].as_str().unwrap().to_string();
            assert!(m.contains("proxy") && m.contains("kora panel --tailscale"), "{m}");
        }
    }
    // Without them, all is normal.
    let r = app
        .oneshot(req("GET", "/api/v1/meta").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
}

#[tokio::test]
async fn busy_answers_do_not_give_away_the_running_job() {
    let st = AppState::system(TOKEN.into(), ADDR.parse().unwrap());
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let first = st
        .spawn_job(kora_core::api::Op::ServerControl, Some("a".into()), move |_| {
            let _ = rx.recv_timeout(Duration::from_secs(5));
            Ok(Value::Null)
        })
        .unwrap();
    let id = json_of(first).await["job"].as_str().unwrap().to_string();
    let e = st
        .spawn_job(kora_core::api::Op::UpdateApply, Some("a".into()), |_| Ok(Value::Null))
        .err()
        .unwrap();
    assert_eq!(e.status, StatusCode::CONFLICT);
    assert!(e.message.contains("'a'") && !e.message.contains(&id), "{}", e.message);
    let _ = tx.send(());
}

#[tokio::test]
async fn forged_cookie_rejected() {
    let (app, _) = setup();
    let r = app
        .oneshot(
            req("GET", "/api/v1/jobs/j1")
                .header(header::COOKIE, "kora_session_47120=nope")
                .header("x-kora-session", "nope")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
}

#[tokio::test]
async fn wrong_token_and_lockout() {
    let (app, _) = setup();
    for _ in 0..9 {
        assert_eq!(bad_login(&app, "bad").await, 401);
    }
    assert_eq!(bad_login(&app, "bad").await, 401); // 10th failure triggers lockout
    assert_eq!(bad_login(&app, "bad").await, 429);
    assert_eq!(bad_login(&app, TOKEN).await, 429); // even the right token is refused while locked
}

#[tokio::test]
async fn lockout_expires() {
    let jobs = Arc::new(MemoryJobs::new());
    let st = AppState::new(TOKEN.into(), ADDR.parse().unwrap(), jobs)
        .with_limiter(2, Duration::from_millis(100));
    let app = router(st);
    bad_login(&app, "x").await;
    bad_login(&app, "x").await;
    assert_eq!(bad_login(&app, TOKEN).await, 429);
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(bad_login(&app, TOKEN).await, 200);
}

#[tokio::test]
async fn bad_host_forbidden_everywhere() {
    let (app, _) = setup();
    for uri in ["/api/v1/meta", "/", "/some/page"] {
        let r = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header(header::HOST, "evil.example:47120")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), 403, "{uri}");
        assert!(r.headers().contains_key(header::CONTENT_SECURITY_POLICY));
    }
    let r = app
        .clone()
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(r.status(), 403); // no Host header at all
    let r = app
        .oneshot(
            Request::builder()
                .uri("/")
                .header(header::HOST, "localhost:47120")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
}

#[tokio::test]
async fn post_needs_x_kora() {
    let (app, _) = setup();
    let r = app
        .oneshot(
            req("POST", "/api/v1/session")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"token": TOKEN}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
}

#[tokio::test]
async fn bad_origin_forbidden_good_origin_ok() {
    let (app, _) = setup();
    let mk = |origin: &str| {
        req("POST", "/api/v1/session")
            .header("x-kora", "1")
            .header(header::ORIGIN, origin.to_string())
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({"token": TOKEN}).to_string()))
            .unwrap()
    };
    assert_eq!(
        app.clone()
            .oneshot(mk("http://evil.example"))
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(app.clone().oneshot(mk("null")).await.unwrap().status(), 403);
    assert_eq!(
        app.oneshot(mk("http://127.0.0.1:47120"))
            .await
            .unwrap()
            .status(),
        200
    );
}

#[tokio::test]
async fn bearer_works_without_x_kora() {
    let (app, jobs) = setup();
    jobs.create("j1");
    let r = app
        .clone()
        .oneshot(
            req("GET", "/api/v1/jobs/j1")
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    // a POST with bearer and no X-Kora passes the CSRF gate (route itself is unknown -> 404, not 403)
    let r = app
        .clone()
        .oneshot(
            req("POST", "/api/v1/nothing")
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    // wrong bearer is refused
    let r = app
        .oneshot(
            req("GET", "/api/v1/jobs/j1")
                .header(header::AUTHORIZATION, "Bearer wrong")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
}

#[tokio::test]
async fn security_headers_everywhere() {
    let (app, _) = setup();
    for uri in ["/", "/api/v1/meta", "/api/v1/jobs/x"] {
        let r = app
            .clone()
            .oneshot(req("GET", uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let h = r.headers();
        assert_eq!(
            h[header::CONTENT_SECURITY_POLICY],
            kora_web::security::CSP,
            "{uri}"
        );
        assert!(h[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .contains("frame-ancestors 'none'"));
        assert_eq!(h[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert_eq!(h[header::REFERRER_POLICY], "no-referrer");
        if uri.starts_with("/api") {
            assert_eq!(h[header::CACHE_CONTROL], "no-store", "{uri}");
        }
    }
}

#[tokio::test]
async fn body_limit() {
    let (app, _) = setup();
    let big = format!("{{\"token\":\"{}\"}}", "a".repeat(2 * 1024 * 1024));
    let r = app
        .oneshot(
            req("POST", "/api/v1/session")
                .header("x-kora", "1")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(big))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), 413);
}

#[tokio::test]
async fn sse_stream_end_to_end() {
    let (app, jobs) = setup();
    jobs.create("j1");
    jobs.push("j1", json!({"step": 1}));
    let s = login(&app).await;
    // As the page's EventSource asks: the secret in `?ks=`.
    let uri = format!("/api/v1/jobs/j1/events?ks={}", s.secret);
    let r = get_with(&app, &uri, &s.cookie, None).await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()[header::CONTENT_TYPE], "text/event-stream");
    let j2 = jobs.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        j2.push("j1", json!({"step": 2}));
        j2.finish("j1", json!({"ok": true}));
    });
    let text = String::from_utf8(body(r).await).unwrap();
    let expected = "event: progress\ndata: {\"step\":1}\n\nevent: progress\ndata: {\"step\":2}\n\nevent: done\ndata: {\"ok\":true}\n\n";
    assert_eq!(text, expected);
}

#[tokio::test]
async fn sse_failed_and_finished_job_replay() {
    let (app, jobs) = setup();
    jobs.create("f");
    jobs.push("f", json!("a"));
    jobs.fail("f", "boom");
    let r = app
        .clone()
        .oneshot(
            req("GET", "/api/v1/jobs/f/events")
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let text = String::from_utf8(body(r).await).unwrap();
    assert_eq!(
        text,
        "event: progress\ndata: \"a\"\n\nevent: failed\ndata: {\"error\":\"boom\"}\n\n"
    );
    let r = app
        .oneshot(
            req("GET", "/api/v1/jobs/missing/events")
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
}

#[tokio::test]
async fn static_fallback_page_and_spa() {
    let (app, _) = setup();
    let r = app
        .clone()
        .oneshot(req("GET", "/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert!(r.headers()[header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .starts_with("text/html"));
    let t = String::from_utf8(body(r).await).unwrap();
    // Without web/dist this is the built-in page; with it, index.html. Either way: no inline script.
    assert!(!t.contains("<script>"));
    let r = app
        .clone()
        .oneshot(req("GET", "/servers/abc").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert!(r.headers()[header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .starts_with("text/html"));
}

#[tokio::test]
async fn spa_does_not_swallow_api() {
    let (app, _) = setup();
    let s = login(&app).await;
    let r = get_with(&app, "/api/v1/unknown", &s.cookie, Some(&s.secret)).await;
    assert_eq!(r.status(), 404);
    assert_eq!(json_of(r).await["error"]["code"], "not_found");
    let r = app
        .oneshot(req("GET", "/api/other").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    assert_eq!(json_of(r).await["error"]["code"], "not_found");
}

#[tokio::test]
async fn real_listener_roundtrip() {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let st = AppState::new(TOKEN.into(), addr, Arc::new(MemoryJobs::new()));
    tokio::spawn(async move { axum::serve(l, router(st)).await.unwrap() });
    let out = tokio::task::spawn_blocking(move || {
        use std::io::{Read, Write};
        let mut s = std::net::TcpStream::connect(addr).unwrap();
        s.write_all(
            format!("GET /api/v1/meta HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    })
    .await
    .unwrap();
    assert!(out.starts_with("HTTP/1.1 200"), "{out}");
}

#[test]
fn port_in_use_is_a_clear_error() {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let e = kora_web::run(kora_web::PanelOptions {
        port,
        open_browser: false,
        ..Default::default()
    })
    .unwrap_err();
    assert!(matches!(e, kora_web::PanelError::Bind { .. }), "{e}");
}
