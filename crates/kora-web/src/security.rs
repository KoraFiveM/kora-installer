//! Local security middleware (plan 2.3): Host/Origin/CSRF checks, auth, headers.
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::remote::{self, roles, Caller, Identity, Provider, Role};
use axum::extract::{ConnectInfo, Request, State};
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

/// A session with no request for this long is dropped.
pub const SESSION_IDLE: Duration = Duration::from_secs(12 * 60 * 60);
/// A session is dropped this long after sign-in, however busy it is.
pub const SESSION_MAX: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Where "now" comes from. Tests replace it to step over the timeouts.
pub type Clock = Arc<dyn Fn() -> Instant + Send + Sync>;

/// What a session cookie stands for. The role is looked up again on every request; for remote
/// sessions the role at sign-in is kept too, and the session ends if it no longer matches.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub provider: Provider,
    pub name: String,
    /// Created for a remote peer. A cookie never works from the other kind of peer.
    pub remote: bool,
    /// The role when the session was made (remote sessions).
    pub role: Role,
    created: Instant,
    last_seen: Instant,
}

pub struct Sessions {
    map: Mutex<HashMap<String, SessionInfo>>,
    clock: Clock,
}

impl Default for Sessions {
    fn default() -> Self {
        Self::with_clock(Arc::new(Instant::now))
    }
}

impl Sessions {
    pub fn with_clock(clock: Clock) -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
            clock,
        }
    }
    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<String, SessionInfo>> {
        self.map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    fn expired(&self, i: &SessionInfo, now: Instant) -> bool {
        now.saturating_duration_since(i.created) >= SESSION_MAX
            || now.saturating_duration_since(i.last_seen) >= SESSION_IDLE
    }
    /// A local (launch token) session.
    pub fn create(&self) -> String {
        self.create_for(Provider::Local, "local", false, Role::Admin)
    }
    pub fn create_for(&self, provider: Provider, name: &str, remote: bool, role: Role) -> String {
        let id = random_hex();
        let now = (self.clock)();
        let mut m = self.map();
        // Keep the table from growing with sessions nobody will use again.
        m.retain(|_, i| !self.expired(i, now));
        m.insert(
            id.clone(),
            SessionInfo {
                provider,
                name: name.to_string(),
                remote,
                role,
                created: now,
                last_seen: now,
            },
        );
        id
    }
    /// The session, if it exists and has not expired; a use counts as activity (idle timer restarts).
    pub fn get(&self, id: &str) -> Option<SessionInfo> {
        let now = (self.clock)();
        let mut m = self.map();
        let info = m.get_mut(id)?;
        if self.expired(info, now) {
            m.remove(id);
            return None;
        }
        info.last_seen = now;
        Some(info.clone())
    }
    pub fn remove(&self, id: &str) {
        self.map().remove(id);
    }
    pub fn valid(&self, id: &str) -> bool {
        self.get(id).is_some()
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

pub async fn guard(State(st): State<AppState>, mut req: Request, next: Next) -> Response {
    let api = req.uri().path() == "/api" || req.uri().path().starts_with("/api/");
    let mut resp = match check(&st, &req) {
        Ok(caller) => {
            req.extensions_mut().insert(caller);
            next.run(req).await
        }
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

/// What a remote session with `id` may do with this request (plan section 1). Loopback never gets here.
fn authorise(st: &AppState, method: &Method, path: &str, id: &Identity) -> Result<(), ApiError> {
    let safe = method == Method::GET || method == Method::HEAD;
    let Some(op) = roles::op_for(method.as_str(), path) else {
        // No such route: a read falls through to the ordinary 404, anything else is refused.
        return if safe {
            Ok(())
        } else {
            Err(ApiError::forbidden(
                "This is not available over remote access.",
            ))
        };
    };
    if roles::local_only(op) {
        return Err(ApiError::forbidden(
            "This can only be done on the computer that runs Kora, not over remote access.",
        ));
    }
    let need = roles::min_role(op);
    if id.role < need {
        return Err(ApiError::forbidden(format!(
            "Your account has {} access, so it cannot do this. Ask an administrator for {need} access.",
            id.role
        )));
    }
    if roles::destructive(op) && !st.remote.config().allow_remote_destructive {
        return Err(ApiError::forbidden(
            "This action is turned off for remote access. An administrator can turn it on in Settings, on the computer that runs Kora.",
        ));
    }
    Ok(())
}

fn check(st: &AppState, req: &Request) -> Result<Caller, ApiError> {
    let h = req.headers();
    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0);
    // No peer information (the router called directly, as the tests do) counts as local.
    let remote = peer.is_some_and(|a| remote::is_remote_peer(a.ip()));
    // DNS rebinding defence: every route, including static files.
    let host = h
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !st.host_ok(host) {
        return Err(ApiError::forbidden("host not allowed"));
    }
    if remote && !st.remote.enabled() {
        return Err(ApiError::forbidden(
            "Remote access is turned off on this computer.",
        ));
    }
    let method = req.method();
    let path = req.uri().path();
    let safe = method == Method::GET || method == Method::HEAD;
    // The launch token is for the computer itself. Remote callers present a per-user token.
    let bearer_identity = bearer(h).and_then(|t| {
        if remote {
            st.remote.identity_for_token(t)
        } else {
            ct_eq(t, &st.token).then(Identity::local)
        }
    });
    let bearer_ok = bearer_identity.is_some();
    if !safe {
        if let Some(o) = h.get(header::ORIGIN) {
            let o = o.to_str().unwrap_or("");
            let ok = o
                .get(..7)
                .is_some_and(|p| p.eq_ignore_ascii_case("http://"))
                && st.host_ok(&o[7..]);
            if !ok {
                return Err(ApiError::forbidden("origin not allowed"));
            }
        }
        if !bearer_ok && h.get("x-kora").and_then(|v| v.to_str().ok()) != Some("1") {
            return Err(ApiError::forbidden("missing X-Kora header"));
        }
    }
    let mut caller = Caller {
        remote,
        peer,
        identity: None,
    };
    if path == "/api/v1" || path.starts_with("/api/v1/") {
        let session_identity = || {
            let id = cookie_value(h, COOKIE)?;
            let info = st.sessions.get(&id)?;
            if info.remote != remote {
                return None;
            }
            // A removed user, or a role that changed since sign-in, ends the session.
            match st.remote.resolve(info.provider, &info.name) {
                Some(i) if i.role == info.role => Some(i),
                _ => {
                    st.sessions.remove(&id);
                    None
                }
            }
        };
        caller.identity = bearer_identity.or_else(session_identity);
        if is_public(method, path) {
            return Ok(caller);
        }
        let Some(id) = caller.identity.clone() else {
            return Err(ApiError::unauthorized("missing or invalid session"));
        };
        if remote {
            authorise(st, method, path, &id)?;
        }
    }
    Ok(caller)
}
