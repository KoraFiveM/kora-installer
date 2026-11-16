//! Local security middleware (plan 2.3): Host/Origin/CSRF checks, auth, headers.
use std::collections::HashSet;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::error::ApiError;
use crate::AppState;

pub const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
img-src 'self' data:; font-src 'self'; connect-src 'self'; frame-ancestors 'none'; \
base-uri 'none'; form-action 'self'";
pub const COOKIE: &str = "kora_session";

pub fn random_hex() -> String {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).expect("system RNG unavailable");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut diff = (a.len() ^ b.len()) as u8;
    for i in 0..a.len().max(b.len()) {
        diff |= a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0);
    }
    diff == 0
}

/// Failed-attempt counter with lockout.
pub struct Limiter {
    max: u32,
    lockout: Duration,
    st: Mutex<(u32, Option<Instant>)>,
}

impl Limiter {
    pub fn new(max: u32, lockout: Duration) -> Self {
        Self {
            max,
            lockout,
            st: Mutex::new((0, None)),
        }
    }
    pub fn locked(&self) -> bool {
        let mut g = self.st.lock().unwrap();
        match g.1 {
            Some(t) if Instant::now() < t => true,
            Some(_) => {
                *g = (0, None);
                false
            }
            None => false,
        }
    }
    pub fn failure(&self) {
        let mut g = self.st.lock().unwrap();
        g.0 += 1;
        if g.0 >= self.max {
            g.1 = Some(Instant::now() + self.lockout);
        }
    }
    pub fn success(&self) {
        *self.st.lock().unwrap() = (0, None);
    }
}

#[derive(Default)]
pub struct Sessions(Mutex<HashSet<String>>);
impl Sessions {
    pub fn create(&self) -> String {
        let id = random_hex();
        self.0.lock().unwrap().insert(id.clone());
        id
    }
    pub fn valid(&self, id: &str) -> bool {
        self.0.lock().unwrap().contains(id)
    }
}

fn cookie_value(h: &HeaderMap, name: &str) -> Option<String> {
    for v in h.get_all(header::COOKIE) {
        for part in v.to_str().ok()?.split(';') {
            if let Some((k, val)) = part.trim().split_once('=') {
                if k == name {
                    return Some(val.to_string());
                }
            }
        }
    }
    None
}

fn bearer(h: &HeaderMap) -> Option<&str> {
    let v = h.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, tok) = v.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then_some(tok.trim())
}

fn is_public(method: &Method, path: &str) -> bool {
    (method == Method::GET && path == "/api/v1/meta")
        || (method == Method::POST && path == "/api/v1/session")
}

pub async fn guard(State(st): State<AppState>, req: Request, next: Next) -> Response {
    let api = req.uri().path() == "/api" || req.uri().path().starts_with("/api/");
    let mut resp = match check(&st, &req) {
        Ok(()) => next.run(req).await,
        Err(e) => e.into_response(),
    };
    let h = resp.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    if api {
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    resp
}

fn check(st: &AppState, req: &Request) -> Result<(), ApiError> {
    let h = req.headers();
    // DNS rebinding defence: every route, including static files.
    let host = h
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !st
        .allowed_hosts
        .iter()
        .any(|a| a.eq_ignore_ascii_case(host))
    {
        return Err(ApiError::forbidden("host not allowed"));
    }
    let method = req.method();
    let path = req.uri().path();
    let safe = method == Method::GET || method == Method::HEAD;
    let bearer_ok = bearer(h).is_some_and(|t| ct_eq(t, &st.token));
    if !safe {
        if let Some(o) = h.get(header::ORIGIN) {
            let o = o.to_str().unwrap_or("");
            let ok = st
                .allowed_hosts
                .iter()
                .any(|a| o.eq_ignore_ascii_case(&format!("http://{a}")));
            if !ok {
                return Err(ApiError::forbidden("origin not allowed"));
            }
        }
        if !bearer_ok && h.get("x-kora").and_then(|v| v.to_str().ok()) != Some("1") {
            return Err(ApiError::forbidden("missing X-Kora header"));
        }
    }
    if path == "/api/v1" || path.starts_with("/api/v1/") {
        if is_public(method, path) {
            return Ok(());
        }
        let session_ok = cookie_value(h, COOKIE).is_some_and(|c| st.sessions.valid(&c));
        if !bearer_ok && !session_ok {
            return Err(ApiError::unauthorized("missing or invalid session"));
        }
    }
    Ok(())
}
