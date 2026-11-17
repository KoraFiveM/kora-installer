//! Remote sign-in end to end through `router()`: enrolment links, passkeys (a software passkey stands in for the
//! browser), one-time codes with a password, recovery codes, API tokens, the fresh check, devices, resets and the
//! negative cases (reused links, replayed assertions and codes, other names, token downgrade).
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, Request};
use kora_core::api::fake::FakeBackend;
use kora_web::remote::auth::{db, totp};
use kora_web::remote::{Remote, RemoteConfig, Role};
use kora_web::tls::serve::Transport;
use kora_web::{router, AppState};
use serde_json::{json, Value};
use tower::ServiceExt;
use url::Url;
use webauthn_authenticator_rs::softpasskey::SoftPasskey;
use webauthn_authenticator_rs::WebauthnAuthenticator;
use webauthn_rs::prelude::{CreationChallengeResponse, RequestChallengeResponse};

const BOUND: &str = "192.168.1.5:47120";
const HOST: &str = "box.lan:47120";
const LAUNCH: &str = "launchtoken";
const REMOTE: &str = "192.168.1.20:51000";
const LOOPBACK: &str = "127.0.0.1:51000";

struct W {
    app: axum::Router,
    remote: Arc<Remote>,
    clock: Arc<Mutex<Duration>>,
    _dir: tempfile::TempDir,
}

fn world_with(cfg: RemoteConfig) -> W {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = cfg;
    cfg.enabled = true;
    cfg.allowed_hosts = vec!["box.lan".into(), "other.lan".into()];
    let path = dir.path().join("panel.json");
    cfg.save(&path).unwrap();
    let remote = Arc::new(Remote::from_file(path).unwrap());
    let fake = FakeBackend::sample();
    kora_core::installer::registry::use_test_home();
    let base = Instant::now();
    let off = Arc::new(Mutex::new(Duration::ZERO));
    let o = off.clone();
    let st = AppState::system(LAUNCH.into(), BOUND.parse().unwrap())
        .with_backend(Arc::new(move || Box::new(fake.clone())))
        .with_remote(remote.clone())
        .with_session_clock(Arc::new(move || base + *o.lock().unwrap()));
    W { app: router(st), remote, clock: off, _dir: dir }
}

fn world() -> W {
    world_with(RemoteConfig::default())
}

struct R {
    method: &'static str,
    uri: String,
    host: String,
    peer: &'static str,
    secure: bool,
    headers: Vec<(&'static str, String)>,
    body: Option<Value>,
}
fn req(method: &'static str, uri: &str) -> R {
    R { method, uri: uri.into(), host: HOST.into(), peer: REMOTE, secure: true, headers: vec![("x-kora", "1".into())], body: None }
}
impl R {
    fn host(mut self, h: &str) -> Self {
        self.host = h.into();
        self
    }
    fn peer(mut self, p: &'static str) -> Self {
        self.peer = p;
        self
    }
    fn plain(mut self) -> Self {
        self.secure = false;
        self
    }
    fn h(mut self, k: &'static str, v: &str) -> Self {
        self.headers.push((k, v.into()));
        self
    }
    fn bearer(self, t: &str) -> Self {
        self.h("authorization", &format!("Bearer {t}"))
    }
    fn session(self, s: &Sess) -> Self {
        self.h("cookie", &s.cookie).h("x-kora-session", &s.secret)
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
        r.extensions_mut().insert(ConnectInfo(self.peer.parse::<SocketAddr>().unwrap()));
        r.extensions_mut().insert(Transport { secure: self.secure });
        let resp = app.clone().oneshot(r).await.unwrap();
        let status = resp.status().as_u16();
        let cookie = resp.headers().get(header::SET_COOKIE).map(|c| c.to_str().unwrap().to_string());
        let bytes = axum::body::to_bytes(resp.into_body(), 1 << 22).await.unwrap();
        Resp { status, json: serde_json::from_slice(&bytes).unwrap_or(Value::Null), set_cookie: cookie }
    }
}

struct Resp {
    status: u16,
    json: Value,
    set_cookie: Option<String>,
}
impl Resp {
    fn msg(&self) -> String {
        self.json["error"]["message"].as_str().unwrap_or("").to_string()
    }
    fn code(&self) -> String {
        self.json["error"]["code"].as_str().unwrap_or("").to_string()
    }
}

#[derive(Clone)]
struct Sess {
    cookie: String,
    secret: String,
}
fn sess(r: &Resp) -> Sess {
    let c = r.set_cookie.as_ref().expect("a cookie");
    Sess { cookie: c.split(';').next().unwrap().to_string(), secret: r.json["secret"].as_str().expect("a secret").to_string() }
}

/// An administrator on the computer makes a user; returns the enrolment link.
async fn invite(w: &W, name: &str, role: &str) -> String {
    let local = req("POST", "/api/v1/session").peer(LOOPBACK).plain().host(BOUND).json(json!({"token": LAUNCH})).send(&w.app).await;
    let s = sess(&local);
    let r = req("POST", "/api/v1/users").peer(LOOPBACK).plain().host(BOUND).session(&s).json(json!({"name": name, "role": role})).send(&w.app).await;
    assert_eq!(r.status, 201, "{:?}", r.json);
    r.json["link"].as_str().unwrap().to_string()
}

async fn local_session(w: &W) -> Sess {
    sess(&req("POST", "/api/v1/session").peer(LOOPBACK).plain().host(BOUND).json(json!({"token": LAUNCH})).send(&w.app).await)
}

fn origin() -> Url {
    Url::parse("https://box.lan:47120").unwrap()
}

/// Enrol a software passkey with `link`; returns the new session and the authenticator.
async fn enrol_passkey(w: &W, link: &str) -> (Sess, SoftPasskey) {
    let mut auth = SoftPasskey::new(true);
    let begin = req("POST", "/api/v1/auth/enrol").json(json!({"step": "passkey_begin", "link": link})).send(&w.app).await;
    assert_eq!(begin.status, 200, "{:?}", begin.json);
    let ch: CreationChallengeResponse = serde_json::from_value(begin.json["options"].clone()).unwrap();
    let cred = auth.do_registration(origin(), ch).unwrap();
    let done = req("POST", "/api/v1/auth/enrol").json(json!({"step": "passkey_finish", "link": link, "handle": begin.json["handle"], "label": "Laptop", "credential": cred})).send(&w.app).await;
    assert_eq!(done.status, 200, "{:?}", done.json);
    assert_eq!(done.json["identity"]["provider"], "passkey");
    assert!(done.set_cookie.as_ref().unwrap().contains("; Secure"), "the cookie is Secure over TLS");
    (sess(&done), auth)
}

async fn login_passkey(w: &W, auth: &mut SoftPasskey, user: &str) -> Resp {
    let begin = req("POST", "/api/v1/auth/login").json(json!({"step": "passkey_begin", "user": user})).send(&w.app).await;
    if begin.status != 200 {
        return begin;
    }
    let ch: RequestChallengeResponse = serde_json::from_value(begin.json["options"].clone()).unwrap();
    let cred = auth.do_authentication(origin(), ch).unwrap();
    req("POST", "/api/v1/auth/login").json(json!({"step": "passkey_finish", "handle": begin.json["handle"], "credential": cred})).send(&w.app).await
}

fn b32_decode(s: &str) -> Vec<u8> {
    let (mut buf, mut bits, mut out) = (0u32, 0u32, Vec::new());
    for c in s.bytes() {
        let v = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567".iter().position(|x| *x == c).unwrap() as u32;
        buf = (buf << 5) | v;
        bits += 5;
        if bits >= 8 {
            out.push((buf >> (bits - 8)) as u8);
            bits -= 8;
        }
    }
    out
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

/// Set up one-time codes with `link`; returns (session, secret bytes, recovery codes).
async fn enrol_code(w: &W, link: &str, password: &str) -> (Sess, Vec<u8>, Vec<String>) {
    let begin = req("POST", "/api/v1/auth/enrol").json(json!({"step": "code_begin", "link": link})).send(&w.app).await;
    assert_eq!(begin.status, 200, "{:?}", begin.json);
    assert!(begin.json["qr_svg"].as_str().unwrap().contains("<svg"));
    assert!(begin.json["otpauth"].as_str().unwrap().starts_with("otpauth://totp/"));
    let secret = b32_decode(begin.json["secret"].as_str().unwrap());
    let code = totp::code_at(&secret, totp::step_of(now()));
    let done = req("POST", "/api/v1/auth/enrol").json(json!({"step": "code_finish", "link": link, "password": password, "code": code})).send(&w.app).await;
    assert_eq!(done.status, 200, "{:?}", done.json);
    let codes: Vec<String> = done.json["recovery_codes"].as_array().unwrap().iter().map(|c| c.as_str().unwrap().to_string()).collect();
    (sess(&done), secret, codes)
}

const PW: &str = "plum otter lantern quietly";

#[tokio::test]
async fn a_user_is_invited_enrols_a_passkey_and_signs_in_with_it() {
    let w = world();
    let link = invite(&w, "sam", "admin").await;
    // The link says who it is for, without being used up.
    let who = req("POST", "/api/v1/auth/enrol").json(json!({"step": "check", "link": link})).send(&w.app).await;
    assert_eq!((who.status, who.json["user"].as_str(), who.json["role"].as_str(), who.json["passkey"].as_bool()), (200, Some("sam"), Some("admin"), Some(true)));
    let (s, mut auth) = enrol_passkey(&w, &link).await;
    // The new session works at once.
    let me = req("GET", "/api/v1/auth/account").session(&s).send(&w.app).await;
    assert_eq!(me.status, 200, "{:?}", me.json);
    assert_eq!((me.json["user"].as_str(), me.json["strong"].as_bool(), me.json["passkeys"].as_array().unwrap().len()), (Some("sam"), Some(true), 1));
    assert_eq!(me.json["passkeys"][0]["rp_id"], "box.lan");
    assert!(!me.json.to_string().contains("credential_id") && !me.json.to_string().contains("passkey_json"), "no credential material in the answer");
    // Signing in with the passkey from a new browser.
    let r = login_passkey(&w, &mut auth, "SAM").await;
    assert_eq!(r.status, 200, "{:?}", r.json);
    assert_eq!((r.json["identity"]["provider"].as_str(), r.json["identity"]["role"].as_str()), (Some("passkey"), Some("admin")));
    let s2 = sess(&r);
    assert_eq!(req("GET", "/api/v1/servers").session(&s2).send(&w.app).await.status, 200);
    // Roles are unchanged: this admin still cannot do the local-only things, nor the destructive ones with the switch off.
    assert_eq!(req("GET", "/api/v1/users").session(&s2).send(&w.app).await.status, 403);
    let del = req("DELETE", "/api/v1/servers/prod").session(&s2).json(json!({"confirm": "prod"})).send(&w.app).await;
    assert_eq!((del.status, del.code().as_str()), (403, "remote.actions_off"));
}

#[tokio::test]
async fn an_enrolment_link_works_once() {
    let w = world();
    let link = invite(&w, "sam", "viewer").await;
    let (_s, _auth) = enrol_passkey(&w, &link).await;
    // Used: every step refuses it.
    for step in [json!({"step": "check", "link": link}), json!({"step": "passkey_begin", "link": link}), json!({"step": "code_begin", "link": link})] {
        let r = req("POST", "/api/v1/auth/enrol").json(step).send(&w.app).await;
        assert_eq!(r.status, 401, "{:?}", r.json);
        assert!(r.msg().contains("expired or was already used"));
    }
    // A made-up link is refused the same way.
    assert_eq!(req("POST", "/api/v1/auth/enrol").json(json!({"step": "check", "link": "0".repeat(32)})).send(&w.app).await.status, 401);
}

#[tokio::test]
async fn a_link_started_twice_cannot_finish_twice() {
    let w = world();
    let link = invite(&w, "sam", "viewer").await;
    let mut a = SoftPasskey::new(true);
    let mut b = SoftPasskey::new(true);
    let begin = |_| async { req("POST", "/api/v1/auth/enrol").json(json!({"step": "passkey_begin", "link": link})).send(&w.app).await };
    let (ra, rb) = (begin(0).await, begin(1).await);
    let ca = a.do_registration(origin(), serde_json::from_value::<CreationChallengeResponse>(ra.json["options"].clone()).unwrap()).unwrap();
    let cb = b.do_registration(origin(), serde_json::from_value::<CreationChallengeResponse>(rb.json["options"].clone()).unwrap()).unwrap();
    let fin = |h: &Value, c| req("POST", "/api/v1/auth/enrol").json(json!({"step": "passkey_finish", "link": link, "handle": h, "label": "x", "credential": c})).send(&w.app);
    assert_eq!(fin(&ra.json["handle"], ca).await.status, 200);
    assert_eq!(fin(&rb.json["handle"], cb).await.status, 401, "the second one is too late");
}

#[tokio::test]
async fn a_replayed_assertion_and_other_names_do_not_sign_in() {
    let w = world();
    let (_s, mut auth) = enrol_passkey(&w, &invite(&w, "sam", "operator").await).await;
    let begin = req("POST", "/api/v1/auth/login").json(json!({"step": "passkey_begin", "user": "sam"})).send(&w.app).await;
    let ch: RequestChallengeResponse = serde_json::from_value(begin.json["options"].clone()).unwrap();
    let cred = auth.do_authentication(origin(), ch).unwrap();
    let finish = |h: Value| req("POST", "/api/v1/auth/login").json(json!({"step": "passkey_finish", "handle": h, "credential": cred.clone()})).send(&w.app);
    assert_eq!(finish(begin.json["handle"].clone()).await.status, 200);
    // The recorded answer again, to the same handle or to a new challenge: refused.
    assert_eq!(finish(begin.json["handle"].clone()).await.status, 401);
    let again = req("POST", "/api/v1/auth/login").json(json!({"step": "passkey_begin", "user": "sam"})).send(&w.app).await;
    assert_eq!(finish(again.json["handle"].clone()).await.status, 401);
    // A name nobody has and a user without a passkey give the same answer.
    let nobody = req("POST", "/api/v1/auth/login").json(json!({"step": "passkey_begin", "user": "nobody"})).send(&w.app).await;
    let (n, m) = (nobody.status, nobody.msg());
    assert_eq!(n, 401);
    let link = invite(&w, "kim", "viewer").await;
    let _ = link;
    let nokey = req("POST", "/api/v1/auth/login").json(json!({"step": "passkey_begin", "user": "kim"})).send(&w.app).await;
    assert_eq!((nokey.status, nokey.msg()), (n, m));
}

#[tokio::test]
async fn passkeys_are_for_the_name_they_were_registered_on_and_never_for_an_address_or_plain_http() {
    let w = world();
    let (_s, _auth) = enrol_passkey(&w, &invite(&w, "sam", "operator").await).await;
    // Another allowed name: the passkey belongs to box.lan.
    let r = req("POST", "/api/v1/auth/login").host("other.lan:47120").json(json!({"step": "passkey_begin", "user": "sam"})).send(&w.app).await;
    assert_eq!(r.status, 401);
    assert!(r.msg().contains("no passkey for that name on this address"), "{}", r.msg());
    // A bare address cannot be an ID, and the page is told why (and to use one-time codes).
    let o = req("GET", "/api/v1/auth/options").host(BOUND).send(&w.app).await;
    assert_eq!(o.json["passkey"], false);
    assert!(o.json["passkey_problem"].as_str().unwrap().contains("one-time code"), "{}", o.json);
    assert_eq!(o.json["code"], true);
    // Plain HTTP is not a secure context.
    let o = req("GET", "/api/v1/auth/options").plain().send(&w.app).await;
    assert_eq!(o.json["passkey"], false);
    assert!(o.json["passkey_problem"].as_str().unwrap().contains("https://"));
    let o = req("GET", "/api/v1/auth/options").send(&w.app).await;
    assert_eq!((o.json["passkey"].as_bool(), o.json["token"].as_bool(), o.json["tailscale"].as_bool()), (Some(true), Some(true), Some(false)));
    // And these routes are not for the computer itself.
    let r = req("POST", "/api/v1/auth/login").peer(LOOPBACK).plain().host(BOUND).json(json!({"step": "passkey_begin", "user": "sam"})).send(&w.app).await;
    assert_eq!(r.status, 400);
}

#[tokio::test]
async fn one_time_codes_with_a_password_recovery_codes_and_replay_protection() {
    let w = world();
    let link = invite(&w, "amy", "operator").await;
    // A weak password is refused before the link is used up.
    let begin = req("POST", "/api/v1/auth/enrol").json(json!({"step": "code_begin", "link": link})).send(&w.app).await;
    let secret = b32_decode(begin.json["secret"].as_str().unwrap());
    let code = totp::code_at(&secret, totp::step_of(now()));
    let weak = req("POST", "/api/v1/auth/enrol").json(json!({"step": "code_finish", "link": link, "password": "password1234", "code": code})).send(&w.app).await;
    assert_eq!(weak.status, 400);
    assert!(weak.msg().contains("first ones"), "{}", weak.msg());
    // A wrong code is refused too.
    let wrong = req("POST", "/api/v1/auth/enrol").json(json!({"step": "code_finish", "link": link, "password": PW, "code": "000000"})).send(&w.app).await;
    assert_eq!(wrong.status, 400);
    let (s, secret, recovery) = enrol_code(&w, &link, PW).await;
    assert_eq!(recovery.len(), 10);
    assert_eq!(req("GET", "/api/v1/servers").session(&s).send(&w.app).await.status, 200);
    // The code that proved the set-up is spent: signing in with it again is refused (a code is single use).
    let step = totp::step_of(now());
    let login = |code: String, pw: &str| req("POST", "/api/v1/auth/login").json(json!({"step": "code", "user": "amy", "password": pw, "code": code})).send(&w.app);
    assert_eq!(login(totp::code_at(&secret, step), PW).await.status, 401, "the set-up code cannot be replayed");
    // The next step's code signs in, once.
    let next = totp::code_at(&secret, step + 1);
    let ok = login(next.clone(), PW).await;
    assert_eq!(ok.status, 200, "{:?}", ok.json);
    assert_eq!(ok.json["identity"]["provider"], "code");
    assert_eq!(login(next, PW).await.status, 401, "replay of an accepted code");
    // A wrong password with a valid code does not sign in.
    assert_eq!(login(totp::code_at(&secret, step + 1), "not the password at all").await.status, 401);
    // Same answer for an unknown name.
    let unknown = req("POST", "/api/v1/auth/login").json(json!({"step": "code", "user": "nobody", "password": PW, "code": "123456"})).send(&w.app).await;
    assert_eq!(unknown.status, 401);
    assert_eq!(unknown.msg(), ok_msg_of_wrong(&w).await);
    // A recovery code works once.
    let r = login(recovery[0].clone(), PW).await;
    assert_eq!(r.status, 200, "{:?}", r.json);
    assert!(r.json["notice"].as_str().unwrap().contains("recovery code (9 left)"));
    assert_eq!(login(recovery[0].clone(), PW).await.status, 401);
    // The store holds no secret in clear.
    let store = w.remote.auth_store().unwrap();
    let st = db::state(&store, "amy").unwrap();
    assert!(st.totp && st.password && st.recovery_left == 9);
}

async fn ok_msg_of_wrong(w: &W) -> String {
    req("POST", "/api/v1/auth/login").json(json!({"step": "code", "user": "amy", "password": "wrong wrong wrong wrong", "code": "123456"})).send(&w.app).await.msg()
}

#[tokio::test]
async fn an_account_locks_after_repeated_wrong_passwords_but_the_passkey_still_works() {
    let w = world();
    let (_s, mut auth) = enrol_passkey(&w, &invite(&w, "sam", "operator").await).await;
    // Give sam one-time codes as well (as a signed-in user would).
    let s = sess(&login_passkey(&w, &mut auth, "sam").await);
    let begin = req("POST", "/api/v1/auth/account").session(&s).json(json!({"step": "code_begin"})).send(&w.app).await;
    assert_eq!(begin.status, 200, "{:?}", begin.json);
    let secret = b32_decode(begin.json["secret"].as_str().unwrap());
    let done = req("POST", "/api/v1/auth/account").session(&s).json(json!({"step": "code_finish", "password": PW, "code": totp::code_at(&secret, totp::step_of(now()))})).send(&w.app).await;
    assert_eq!(done.status, 200, "{:?}", done.json);
    // Five wrong passwords from different addresses (so the address limiter does not interfere) lock the account.
    for i in 0..5 {
        let peer: &'static str = Box::leak(format!("192.168.1.{}:5000", 30 + i).into_boxed_str());
        let r = req("POST", "/api/v1/auth/login").peer(peer).json(json!({"step": "code", "user": "sam", "password": "wrong wrong wrong wrong", "code": "123456"})).send(&w.app).await;
        assert_eq!(r.status, 401);
    }
    let late = req("POST", "/api/v1/auth/login")
        .peer("192.168.1.99:5000")
        .json(json!({"step": "code", "user": "sam", "password": PW, "code": totp::code_at(&secret, totp::step_of(now()) + 1)}))
        .send(&w.app)
        .await;
    assert_eq!(late.status, 401, "locked: even the right password and code are refused for now");
    // The passkey is not affected: a stranger cannot lock the owner out of their own device.
    assert_eq!(login_passkey(&w, &mut auth, "sam").await.status, 200);
}

#[tokio::test]
async fn failed_attempts_from_one_address_lock_that_address_out() {
    let w = world();
    for _ in 0..10 {
        let r = req("POST", "/api/v1/auth/login").json(json!({"step": "code", "user": "nobody", "password": "x", "code": "1"})).send(&w.app).await;
        assert_eq!(r.status, 401);
    }
    let r = req("POST", "/api/v1/auth/login").json(json!({"step": "code", "user": "nobody", "password": "x", "code": "1"})).send(&w.app).await;
    assert_eq!(r.status, 429);
    // Another address is not locked.
    let r = req("POST", "/api/v1/auth/login").peer("192.168.1.77:1").json(json!({"step": "code", "user": "nobody", "password": "x", "code": "1"})).send(&w.app).await;
    assert_eq!(r.status, 401);
}

fn remote_actions_on(w: &W) {
    w.remote
        .update(|c| {
            c.allow_remote_destructive = true;
            Ok(())
        })
        .unwrap();
}

#[tokio::test]
async fn sensitive_actions_ask_for_a_fresh_passkey_check() {
    let w = world();
    remote_actions_on(&w);
    let (s, mut auth) = enrol_passkey(&w, &invite(&w, "sam", "admin").await).await;
    // Just signed in: counts as checked.
    let ok = req("POST", "/api/v1/servers/prod/console").session(&s).json(json!({"command": "say hi"})).send(&w.app).await;
    assert_ne!(ok.code(), "remote.reauth_required", "{:?}", ok.json);
    // Eleven minutes later it no longer does.
    *w.clock.lock().unwrap() += Duration::from_secs(11 * 60);
    let r = req("POST", "/api/v1/servers/prod/console").session(&s).json(json!({"command": "say hi"})).send(&w.app).await;
    assert_eq!((r.status, r.code().as_str()), (403, "remote.reauth_required"), "{:?}", r.json);
    // Ordinary work is not interrupted.
    assert_eq!(req("GET", "/api/v1/servers").session(&s).send(&w.app).await.status, 200);
    // Checking with the passkey makes it work again.
    let begin = req("POST", "/api/v1/auth/reauth").session(&s).json(json!({"step": "passkey_begin"})).send(&w.app).await;
    assert_eq!(begin.status, 200, "{:?}", begin.json);
    let ch: RequestChallengeResponse = serde_json::from_value(begin.json["options"].clone()).unwrap();
    let cred = auth.do_authentication(origin(), ch).unwrap();
    let fin = req("POST", "/api/v1/auth/reauth").session(&s).json(json!({"step": "passkey_finish", "handle": begin.json["handle"], "credential": cred})).send(&w.app).await;
    assert_eq!((fin.status, fin.json["valid_for"].as_u64()), (200, Some(600)), "{:?}", fin.json);
    let r = req("POST", "/api/v1/servers/prod/console").session(&s).json(json!({"command": "say hi"})).send(&w.app).await;
    assert_ne!(r.code(), "remote.reauth_required", "{:?}", r.json);
}

#[tokio::test]
async fn a_code_check_also_makes_a_session_fresh_and_a_wrong_user_does_not() {
    let w = world();
    remote_actions_on(&w);
    let (s, secret, _) = enrol_code(&w, &invite(&w, "amy", "admin").await, PW).await;
    *w.clock.lock().unwrap() += Duration::from_secs(11 * 60);
    let blocked = req("POST", "/api/v1/servers/prod/console").session(&s).json(json!({"command": "say hi"})).send(&w.app).await;
    assert_eq!(blocked.code(), "remote.reauth_required");
    let step = totp::step_of(now());
    let bad = req("POST", "/api/v1/auth/reauth").session(&s).json(json!({"step": "code", "password": "wrong wrong wrong wrong", "code": totp::code_at(&secret, step + 1)})).send(&w.app).await;
    assert_eq!(bad.status, 401);
    let good = req("POST", "/api/v1/auth/reauth").session(&s).json(json!({"step": "code", "password": PW, "code": totp::code_at(&secret, step + 1)})).send(&w.app).await;
    assert_eq!(good.status, 200, "{:?}", good.json);
    let r = req("POST", "/api/v1/servers/prod/console").session(&s).json(json!({"command": "say hi"})).send(&w.app).await;
    assert_ne!(r.code(), "remote.reauth_required");
}

#[tokio::test]
async fn changing_your_sign_in_needs_a_fresh_check_and_the_last_credential_stays() {
    let w = world();
    let (s, _auth) = enrol_passkey(&w, &invite(&w, "sam", "viewer").await).await;
    let acct = req("GET", "/api/v1/auth/account").session(&s).send(&w.app).await;
    let id = acct.json["passkeys"][0]["id"].as_i64().unwrap();
    // It is your only way in.
    let r = req("POST", "/api/v1/auth/account").session(&s).json(json!({"step": "passkey_remove", "id": id})).send(&w.app).await;
    assert_eq!(r.status, 409, "{:?}", r.json);
    assert!(r.msg().contains("only way to sign in"));
    // Later, without a recent check, it asks first.
    *w.clock.lock().unwrap() += Duration::from_secs(11 * 60);
    let r = req("POST", "/api/v1/auth/account").session(&s).json(json!({"step": "recovery_new"})).send(&w.app).await;
    assert_eq!(r.code(), "remote.reauth_required");
}

#[tokio::test]
async fn devices_are_listed_and_can_be_signed_out() {
    let w = world();
    let (s1, mut auth) = enrol_passkey(&w, &invite(&w, "sam", "viewer").await).await;
    let s2 = sess(&login_passkey(&w, &mut auth, "sam").await);
    let list = req("GET", "/api/v1/auth/account").session(&s1).send(&w.app).await;
    let devices = list.json["devices"].as_array().unwrap();
    assert_eq!(devices.len(), 2);
    assert_eq!(devices.iter().filter(|d| d["current"] == true).count(), 1);
    assert!(!list.json.to_string().contains(&s2.secret) && !list.json.to_string().contains(&s2.cookie[s2.cookie.find('=').unwrap() + 1..]), "never a cookie or secret");
    let other = devices.iter().find(|d| d["current"] == false).unwrap()["handle"].as_str().unwrap().to_string();
    let r = req("POST", "/api/v1/auth/account").session(&s1).json(json!({"step": "device_revoke", "handle": other})).send(&w.app).await;
    assert_eq!(r.status, 200, "{:?}", r.json);
    assert_eq!(req("GET", "/api/v1/servers").session(&s2).send(&w.app).await.status, 401, "signed out at once");
    assert_eq!(req("GET", "/api/v1/servers").session(&s1).send(&w.app).await.status, 200);
}

#[tokio::test]
async fn an_administrator_reset_ends_sessions_and_gives_a_new_link() {
    let w = world();
    let (s, _auth) = enrol_passkey(&w, &invite(&w, "sam", "operator").await).await;
    let local = local_session(&w).await;
    let creds = req("GET", "/api/v1/users/sam/credentials").peer(LOOPBACK).plain().host(BOUND).session(&local).send(&w.app).await;
    assert_eq!((creds.status, creds.json["passkeys"].as_array().map(|a| a.len())), (200, Some(1)), "{:?}", creds.json);
    assert_eq!(creds.json["signed_in_devices"], 1);
    let reset = req("POST", "/api/v1/users/sam/reset").peer(LOOPBACK).plain().host(BOUND).session(&local).json(json!({"what": "all"})).send(&w.app).await;
    assert_eq!(reset.status, 200, "{:?}", reset.json);
    let link = reset.json["link"].as_str().unwrap().to_string();
    assert_eq!(req("GET", "/api/v1/servers").session(&s).send(&w.app).await.status, 401, "the lost device is signed out");
    let creds = req("GET", "/api/v1/users/sam/credentials").peer(LOOPBACK).plain().host(BOUND).session(&local).send(&w.app).await;
    assert_eq!(creds.json["passkeys"].as_array().unwrap().len(), 0);
    // The new link works once and enrols the replacement device.
    let (s2, _) = enrol_passkey(&w, &link).await;
    assert_eq!(req("GET", "/api/v1/servers").session(&s2).send(&w.app).await.status, 200);
    // These are the computer's own controls: a remote administrator does not reach them.
    let (admin, _) = enrol_passkey(&w, &invite(&w, "boss", "admin").await).await;
    assert_eq!(req("POST", "/api/v1/users/sam/reset").session(&admin).json(json!({"what": "all"})).send(&w.app).await.status, 403);
    assert_eq!(req("GET", "/api/v1/api-tokens").session(&admin).send(&w.app).await.status, 403);
}

#[tokio::test]
async fn removing_a_user_removes_their_credentials_and_links() {
    let w = world();
    let (_s, _auth) = enrol_passkey(&w, &invite(&w, "sam", "viewer").await).await;
    let again = invite(&w, "kim", "viewer").await;
    let local = local_session(&w).await;
    for u in ["sam", "kim"] {
        assert_eq!(req("DELETE", &format!("/api/v1/users/{u}")).peer(LOOPBACK).plain().host(BOUND).session(&local).send(&w.app).await.status, 204);
    }
    let store = w.remote.auth_store().unwrap();
    assert!(db::state(&store, "sam").unwrap().passkeys.is_empty());
    assert!(db::peek_link(&store, &again).unwrap().is_none(), "the link of a removed user is gone");
}

#[tokio::test]
async fn api_tokens_are_for_scripts_only() {
    let w = world();
    let local = local_session(&w).await;
    let made = req("POST", "/api/v1/api-tokens").peer(LOOPBACK).plain().host(BOUND).session(&local).json(json!({"name": "backup", "role": "operator", "days": 30})).send(&w.app).await;
    assert_eq!(made.status, 201, "{:?}", made.json);
    let token = made.json["token"].as_str().unwrap().to_string();
    // It works as a bearer header, with its role.
    let r = req("GET", "/api/v1/servers").bearer(&token).send(&w.app).await;
    assert_eq!(r.status, 200);
    let me = req("GET", "/api/v1/meta").bearer(&token).send(&w.app).await;
    assert_eq!((me.json["identity"]["provider"].as_str(), me.json["identity"]["role"].as_str(), me.json["identity"]["name"].as_str()), (Some("api_token"), Some("operator"), Some("backup")));
    // Never in an address: not as a query parameter, not as the stream parameter.
    let r = req("GET", &format!("/api/v1/servers?token={token}")).send(&w.app).await;
    assert_eq!(r.status, 401);
    let r = req("GET", &format!("/api/v1/events?ks={token}")).send(&w.app).await;
    assert_eq!(r.status, 401);
    // Not a sign-in: it makes no session.
    let r = req("POST", "/api/v1/session").json(json!({"token": token})).send(&w.app).await;
    assert_eq!(r.status, 401);
    // Its role is its limit.
    let r = req("DELETE", "/api/v1/servers/prod").bearer(&token).json(json!({"confirm": "prod"})).send(&w.app).await;
    assert_eq!(r.status, 403);
    // The listing never shows the secret; revoking ends it at once.
    let list = req("GET", "/api/v1/api-tokens").peer(LOOPBACK).plain().host(BOUND).session(&local).send(&w.app).await;
    assert!(!list.json.to_string().contains(&token));
    let id = list.json["tokens"][0]["id"].as_i64().unwrap();
    assert_eq!(req("DELETE", &format!("/api/v1/api-tokens/{id}")).peer(LOOPBACK).plain().host(BOUND).session(&local).send(&w.app).await.status, 204);
    assert_eq!(req("GET", "/api/v1/servers").bearer(&token).send(&w.app).await.status, 401);
    // An admin token is still held back by the remote-actions switch for destructive operations.
    let admin = req("POST", "/api/v1/api-tokens").peer(LOOPBACK).plain().host(BOUND).session(&local).json(json!({"name": "ops", "role": "admin"})).send(&w.app).await.json["token"]
        .as_str()
        .unwrap()
        .to_string();
    let r = req("DELETE", "/api/v1/servers/prod").bearer(&admin).json(json!({"confirm": "prod"})).send(&w.app).await;
    assert_eq!((r.status, r.code().as_str()), (403, "remote.actions_off"));
}

#[tokio::test]
async fn the_old_token_link_keeps_working_until_it_is_replaced() {
    let mut cfg = RemoteConfig::default();
    let tok = cfg.add_user("old", Role::Admin).unwrap();
    let w = world_with(cfg);
    // Nothing breaks on upgrade: the token signs in, and the page is told to suggest something better.
    let r = req("POST", "/api/v1/session").json(json!({"token": tok})).send(&w.app).await;
    assert_eq!(r.status, 200);
    let s = sess(&r);
    let meta = req("GET", "/api/v1/meta").session(&s).send(&w.app).await;
    assert_eq!(meta.json["auth_upgrade"], true);
    let acct = req("GET", "/api/v1/auth/account").session(&s).send(&w.app).await;
    assert_eq!((acct.json["upgrade_suggested"].as_bool(), acct.json["legacy_token"].as_bool(), acct.json["strong"].as_bool()), (Some(true), Some(true), Some(false)));
    // The user adds a passkey from this session (the first credential needs no fresh check).
    let mut auth = SoftPasskey::new(true);
    let begin = req("POST", "/api/v1/auth/account").session(&s).json(json!({"step": "passkey_begin"})).send(&w.app).await;
    assert_eq!(begin.status, 200, "{:?}", begin.json);
    let cred = auth.do_registration(origin(), serde_json::from_value::<CreationChallengeResponse>(begin.json["options"].clone()).unwrap()).unwrap();
    let fin = req("POST", "/api/v1/auth/account").session(&s).json(json!({"step": "passkey_finish", "handle": begin.json["handle"], "label": "Phone", "credential": cred})).send(&w.app).await;
    assert_eq!(fin.status, 200, "{:?}", fin.json);
    // Now the token link is not accepted for signing in any more: no downgrade.
    let r = req("POST", "/api/v1/session").json(json!({"token": tok})).send(&w.app).await;
    assert_eq!(r.status, 403);
    assert!(r.msg().contains("passkey or one-time codes"), "{}", r.msg());
    // ...but it still works as a header for an old script, until revoked.
    assert_eq!(req("GET", "/api/v1/servers").bearer(&tok).send(&w.app).await.status, 200);
}

#[tokio::test]
async fn an_administrator_can_require_strong_sign_in_for_everybody() {
    let mut cfg = RemoteConfig::default();
    let tok = cfg.add_user("old", Role::Viewer).unwrap();
    let w = world_with(cfg);
    let local = local_session(&w).await;
    let r = req("PUT", "/api/v1/remote").peer(LOOPBACK).plain().host(BOUND).session(&local).json(json!({"require_strong_auth": true})).send(&w.app).await;
    assert_eq!((r.status, r.json["require_strong_auth"].as_bool()), (200, Some(true)), "{:?}", r.json);
    let r = req("POST", "/api/v1/session").json(json!({"token": tok})).send(&w.app).await;
    assert_eq!(r.status, 403);
    assert!(r.msg().contains("requires a passkey"), "{}", r.msg());
    let o = req("GET", "/api/v1/auth/options").send(&w.app).await;
    assert_eq!((o.json["token"].as_bool(), o.json["strong_required"].as_bool()), (Some(false), Some(true)));
    // Scripts with the same token are unaffected (they are not interactive).
    assert_eq!(req("GET", "/api/v1/servers").bearer(&tok).send(&w.app).await.status, 200);
}
