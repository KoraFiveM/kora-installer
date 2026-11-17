//! Local security middleware (plan 2.3): Host/Origin/CSRF checks, auth, headers. Also the pieces the
//! guard and the session routes share: sign-in link codes, session secrets, sign-in limiters and the cap
//! on live streams.
use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use crate::remote::{self, roles, Caller, Identity, Provider, Role};
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use kora_core::api::Op;
use kora_core::problem::{codes, Action, Problem};

use crate::error::ApiError;
use crate::AppState;

pub const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
img-src 'self' data:; font-src 'self'; connect-src 'self'; frame-ancestors 'none'; \
base-uri 'none'; form-action 'self'";
/// The session cookie's name without the port. The real name is [`cookie_name`].
pub const COOKIE: &str = "kora_session";
/// Header carrying the per-session secret (the UI keeps it in `localStorage`, so it is per port).
pub const SESSION_HEADER: &str = "x-kora-session";
/// Query parameter carrying the same secret where a header cannot be set (`EventSource`, GET only).
pub const SESSION_PARAM: &str = "ks";

/// Browsers send cookies by host, not by port, so a plain name would also go to txAdmin and to anything
/// else on this host (a FiveM resource can log the headers on the game port). The port in the name keeps
/// two panels from overwriting each other; the session secret makes a leaked cookie useless on its own.
pub fn cookie_name(port: u16) -> String {
    format!("{COOKIE}_{port}")
}

pub fn random_hex() -> String {
    random_hex_bytes(32)
}

fn random_hex_bytes(n: usize) -> String {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).expect("system RNG unavailable");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    // Any length difference counts (a cast would drop all but the low byte: 1 and 257 would match).
    let mut diff = u8::from(a.len() != b.len());
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
        Self { max, lockout, st: Mutex::new((0, None)) }
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

/// How long a sign-in link code works.
pub const LINK_CODE_TTL: Duration = Duration::from_secs(10 * 60);
/// Codes kept at once; minting another drops the oldest.
pub const LINK_CODES_MAX: usize = 32;

/// One-time codes for sign-in links (`/#code=...`), so the long-lived launch token never ends up in an
/// address bar, the browser history or a screenshot. Each code works once, for [`LINK_CODE_TTL`].
pub struct LinkCodes {
    codes: Mutex<VecDeque<(String, Instant)>>,
    clock: Clock,
}

impl Default for LinkCodes {
    fn default() -> Self {
        Self::with_clock(Arc::new(Instant::now))
    }
}

impl LinkCodes {
    pub fn with_clock(clock: Clock) -> Self {
        Self { codes: Mutex::new(VecDeque::new()), clock }
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<(String, Instant)>> {
        self.codes.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    /// A new code (128 random bits, hex).
    pub fn mint(&self) -> String {
        let code = random_hex_bytes(16);
        let now = (self.clock)();
        let mut g = self.lock();
        g.retain(|(_, until)| now < *until);
        while g.len() >= LINK_CODES_MAX {
            g.pop_front();
        }
        g.push_back((code.clone(), now + LINK_CODE_TTL));
        code
    }
    /// Use up `code`: true once, if it was minted here and has not expired.
    pub fn consume(&self, code: &str) -> bool {
        let now = (self.clock)();
        let mut g = self.lock();
        g.retain(|(_, until)| now < *until);
        match g.iter().position(|(c, _)| ct_eq(c, code)) {
            Some(i) => {
                g.remove(i);
                true
            }
            None => false,
        }
    }
}

/// Failed sign-ins counted per source address (IPv6 per /64, one network), so a stranger who keeps
/// guessing locks out only themselves. The table is bounded: expired entries go first, then the oldest
/// unlocked one. A locked address is never forgotten to make room: when every place holds one, addresses
/// not seen before are refused until a lockout ends (otherwise many addresses could unlock one).
pub struct IpLimiter {
    max: u32,
    window: Duration,
    lockout: Duration,
    cap: usize,
    map: Mutex<HashMap<IpAddr, IpEntry>>,
    clock: Clock,
}

#[derive(Clone, Copy)]
struct IpEntry {
    fails: u32,
    since: Instant,
    locked_until: Option<Instant>,
}

impl IpEntry {
    fn stale(&self, now: Instant, window: Duration) -> bool {
        match self.locked_until {
            Some(t) => now >= t,
            None => now.saturating_duration_since(self.since) >= window,
        }
    }
}

/// The address a failure is counted against: IPv4 as is, IPv6 cut to its /64.
pub fn limiter_key(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V6(v6) => {
            let s = v6.segments();
            IpAddr::V6(std::net::Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
        }
        v4 => v4,
    }
}

impl IpLimiter {
    /// `max` failures within `window` lock that address out for `lockout`; at most `cap` addresses kept.
    pub fn new(max: u32, window: Duration, lockout: Duration, cap: usize) -> Self {
        Self::with_clock(max, window, lockout, cap, Arc::new(Instant::now))
    }
    pub fn with_clock(max: u32, window: Duration, lockout: Duration, cap: usize, clock: Clock) -> Self {
        Self { max, window, lockout, cap: cap.max(1), map: Mutex::new(HashMap::new()), clock }
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<IpAddr, IpEntry>> {
        self.map.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    pub fn locked(&self, ip: IpAddr) -> bool {
        let key = limiter_key(ip);
        let now = (self.clock)();
        let mut m = self.lock();
        match m.get(&key) {
            Some(e) => e.locked_until.is_some_and(|t| now < t),
            None => !self.make_room(&mut m, now),
        }
    }
    /// Room for one more address: drop expired entries, then the oldest unlocked one. False when every
    /// place holds a live lockout.
    fn make_room(&self, m: &mut HashMap<IpAddr, IpEntry>, now: Instant) -> bool {
        if m.len() < self.cap {
            return true;
        }
        m.retain(|_, e| !e.stale(now, self.window));
        if m.len() < self.cap {
            return true;
        }
        let oldest_unlocked = m.iter().filter(|(_, e)| e.locked_until.is_none()).min_by_key(|(_, e)| e.since).map(|(k, _)| *k);
        match oldest_unlocked {
            Some(k) => {
                m.remove(&k);
                true
            }
            None => false,
        }
    }
    pub fn failure(&self, ip: IpAddr) {
        let key = limiter_key(ip);
        let now = (self.clock)();
        let mut m = self.lock();
        if !m.contains_key(&key) && !self.make_room(&mut m, now) {
            // Full of locked addresses: this one is refused anyway (see `locked`).
            return;
        }
        let e = m.entry(key).or_insert(IpEntry { fails: 0, since: now, locked_until: None });
        if e.stale(now, self.window) {
            *e = IpEntry { fails: 0, since: now, locked_until: None };
        }
        e.fails += 1;
        if e.fails >= self.max {
            e.locked_until = Some(now + self.lockout);
        }
    }
    pub fn success(&self, ip: IpAddr) {
        self.lock().remove(&limiter_key(ip));
    }
    /// Addresses being tracked (tests).
    pub fn tracked(&self) -> usize {
        self.lock().len()
    }
}

/// Live streams (SSE) a remote identity may hold open at once.
pub const STREAMS_PER_IDENTITY: usize = 8;
/// Live streams all remote callers together may hold open.
pub const STREAMS_TOTAL: usize = 64;

/// Counts open remote streams. A [`StreamPermit`] is held by the stream and given back when the
/// browser goes away (the response body is dropped).
pub struct StreamLimits {
    per: usize,
    total: usize,
    open: Arc<Mutex<(usize, HashMap<String, usize>)>>,
}

impl Default for StreamLimits {
    fn default() -> Self {
        Self::new(STREAMS_PER_IDENTITY, STREAMS_TOTAL)
    }
}

impl StreamLimits {
    pub fn new(per: usize, total: usize) -> Self {
        Self { per, total, open: Arc::default() }
    }
    pub fn acquire(&self, who: &str) -> Result<StreamPermit, ApiError> {
        let mut g = self.open.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if g.0 >= self.total {
            return Err(ApiError::rate_limited("Too many live views are open on this panel right now. Close some panel tabs, or try again in a minute."));
        }
        let mine = g.1.get(who).copied().unwrap_or(0);
        if mine >= self.per {
            return Err(ApiError::rate_limited(format!("You already have {mine} live views open (consoles, job progress, server status). Close some panel tabs, then try again.")));
        }
        g.0 += 1;
        *g.1.entry(who.to_string()).or_insert(0) += 1;
        Ok(StreamPermit { open: self.open.clone(), who: who.to_string() })
    }
    /// Streams open now, in total (tests).
    pub fn open(&self) -> usize {
        self.open.lock().unwrap_or_else(std::sync::PoisonError::into_inner).0
    }
}

pub struct StreamPermit {
    open: Arc<Mutex<(usize, HashMap<String, usize>)>>,
    who: String,
}

impl Drop for StreamPermit {
    fn drop(&mut self) {
        let mut g = self.open.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        g.0 = g.0.saturating_sub(1);
        if let Some(n) = g.1.get_mut(&self.who) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                g.1.remove(&self.who);
            }
        }
    }
}

/// A stream that holds a [`StreamPermit`] (if any) for as long as it is alive.
pub struct Limited<S> {
    inner: S,
    _permit: Option<StreamPermit>,
}

impl<S> Limited<S> {
    pub fn new(inner: S, permit: Option<StreamPermit>) -> Self {
        Self { inner, _permit: permit }
    }
}

impl<S: tokio_stream::Stream + Unpin> tokio_stream::Stream for Limited<S> {
    type Item = S::Item;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<S::Item>> {
        Pin::new(&mut self.inner).poll_next(cx)
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
#[derive(Clone)]
pub struct SessionInfo {
    pub provider: Provider,
    pub name: String,
    /// Created for a remote peer. A cookie never works from the other kind of peer.
    pub remote: bool,
    /// The role when the session was made (remote sessions).
    pub role: Role,
    /// Must come with the cookie on every request ([`SESSION_HEADER`] or [`SESSION_PARAM`]).
    secret: String,
    created: Instant,
    last_seen: Instant,
}

impl std::fmt::Debug for SessionInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionInfo").field("provider", &self.provider).field("name", &self.name).field("remote", &self.remote).field("role", &self.role).finish_non_exhaustive()
    }
}

/// A new session: the cookie value and the secret the page sends back with it.
#[derive(Clone)]
pub struct NewSession {
    pub id: String,
    pub secret: String,
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
        Self { map: Mutex::new(HashMap::new()), clock }
    }
    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<String, SessionInfo>> {
        self.map.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    fn expired(&self, i: &SessionInfo, now: Instant) -> bool {
        now.saturating_duration_since(i.created) >= SESSION_MAX || now.saturating_duration_since(i.last_seen) >= SESSION_IDLE
    }
    /// A local (launch token) session.
    pub fn create(&self) -> NewSession {
        self.create_for(Provider::Local, "local", false, Role::Admin)
    }
    pub fn create_for(&self, provider: Provider, name: &str, remote: bool, role: Role) -> NewSession {
        let id = random_hex();
        let secret = random_hex();
        let now = (self.clock)();
        let mut m = self.map();
        // Keep the table from growing with sessions nobody will use again.
        m.retain(|_, i| !self.expired(i, now));
        m.insert(id.clone(), SessionInfo { provider, name: name.to_string(), remote, role, secret: secret.clone(), created: now, last_seen: now });
        NewSession { id, secret }
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
    /// Like [`get`](Self::get), but only when `secret` is the session's own (constant-time compare). A
    /// wrong or missing secret changes nothing, so a stolen cookie can neither use nor end the session.
    pub fn get_with_secret(&self, id: &str, secret: Option<&str>) -> Option<SessionInfo> {
        let secret = secret?;
        let ok = self.map().get(id).is_some_and(|i| ct_eq(secret, &i.secret));
        if !ok {
            return None;
        }
        self.get(id)
    }
    pub fn remove(&self, id: &str) {
        self.map().remove(id);
    }
    pub fn valid(&self, id: &str) -> bool {
        self.get(id).is_some()
    }
}

/// Every value of the cookie `name`. Another service on this host (any port) can set a cookie with the
/// same name that the browser sends first, so the caller must try them all, not just the first.
pub(crate) fn cookie_values(h: &HeaderMap, name: &str) -> Vec<String> {
    h.get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|part| part.trim().split_once('='))
        .filter(|(k, _)| *k == name)
        .map(|(_, v)| v.to_string())
        .collect()
}

/// The routes an `EventSource` opens. It cannot set headers, so only these take the secret as `?ks=`;
/// everywhere else a secret in the address (history, logs) is not accepted.
fn is_stream(path: &str) -> bool {
    matches!(roles::op_for("GET", path), Some(kora_core::api::Op::Events | kora_core::api::Op::JobEvents | kora_core::api::Op::ConsoleStream))
}

/// The session secret of this request: the header, or for a stream (an `EventSource`) the `ks` parameter.
fn session_secret<'a>(h: &'a HeaderMap, method: &Method, path: &str, query: Option<&'a str>) -> Option<&'a str> {
    if let Some(v) = h.get(SESSION_HEADER).and_then(|v| v.to_str().ok()) {
        return Some(v.trim());
    }
    if method != Method::GET || !is_stream(path) {
        return None;
    }
    query?.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == SESSION_PARAM).then_some(v)
    })
}

/// The session this request is signed in with: one of its cookies, with the matching secret in the header,
/// made for this kind of peer. A cookie alone (or a wrong secret) names no session.
pub(crate) fn current_session(st: &AppState, h: &HeaderMap, remote: bool) -> Option<String> {
    let secret = h.get(SESSION_HEADER)?.to_str().ok()?.trim();
    cookie_values(h, &cookie_name(st.addr.port())).into_iter().find(|id| st.sessions.get_with_secret(id, Some(secret)).is_some_and(|i| i.remote == remote))
}

/// Headers a reverse proxy (or `tailscale serve`) adds. Behind one, every visitor reaches the panel from
/// 127.0.0.1 and would be treated as the owner, so such requests are refused outright.
pub const PROXY_HEADERS: &[&str] =
    &["x-forwarded-for", "x-forwarded-host", "x-forwarded-proto", "forwarded", "x-real-ip", "cf-connecting-ip", "true-client-ip", "tailscale-user-login", "tailscale-user-name"];

pub const PROXY_REFUSED: &str = "Kora Panel must not be put behind a proxy or `tailscale serve`: they make every visitor look like this computer. \
To reach the panel from your own devices, run `kora panel --tailscale`, or use an SSH tunnel.";

/// For a request from this machine's own Tailscale address (see [`check`]).
pub const SELF_ADDRESS_REFUSED: &str = "Open the panel from another device, or run `kora panel` on this computer for a sign-in link.";

fn bearer(h: &HeaderMap) -> Option<&str> {
    let v = h.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, tok) = v.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then_some(tok.trim())
}

fn is_public(method: &Method, path: &str) -> bool {
    (method == Method::GET && path == "/api/v1/meta") || (method == Method::POST && path == "/api/v1/session")
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
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(CSP));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    if api {
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    resp
}

/// The refusal for a token session asking for a destructive operation while `remote actions` is off:
/// why, exactly how to turn it on, and the other ways to do it.
fn actions_off(op: Op) -> ApiError {
    let what = match op {
        Op::ServerDelete => "Deleting a server",
        Op::ServerUnregister => "Removing a server from Kora's list",
        Op::UninstallRun | Op::AppUninstall => "Uninstalling Kora",
        Op::ConsoleSend => "Sending commands to the server console",
        Op::SelfUpdateApply => "Updating Kora",
        _ => "Changing the schedule",
    };
    let p = Problem::new(
        codes::REMOTE_ACTIONS_OFF,
        format!("{what} is turned off for sign-ins made with a token. A token is a secret in a link, and anyone who gets hold of it could use it, so this stays off until someone on the computer that runs Kora turns it on."),
    )
    .with_details(format!(
        "To turn it on, run this on the computer that runs Kora (over SSH is fine): {}\n\
         Or do it another way: sign in through Tailscale as an administrator, or open the panel on the computer that runs Kora (an SSH tunnel counts).",
        REMOTE_ACTIONS_ON
    ))
    .with_action(Action::show_command(&["kora", "panel", "remote-actions", "on"], false));
    ApiError::problem(StatusCode::FORBIDDEN, p)
}

/// The command that turns the token-session destructive actions on.
pub const REMOTE_ACTIONS_ON: &str = "kora panel remote-actions on";

/// What a remote session with `id` may do with this request (plan section 1). Loopback never gets here.
fn authorise(st: &AppState, method: &Method, path: &str, id: &Identity) -> Result<(), ApiError> {
    let safe = method == Method::GET || method == Method::HEAD;
    let Some(op) = roles::op_for(method.as_str(), path) else {
        // No such route: a read falls through to the ordinary 404, anything else is refused.
        return if safe { Ok(()) } else { Err(ApiError::forbidden("This is not available over remote access.")) };
    };
    if roles::local_only(op) {
        return Err(ApiError::forbidden("This can only be done on the computer that runs Kora, not over remote access."));
    }
    let need = roles::min_role(op);
    if id.role < need {
        return Err(ApiError::forbidden(format!("Your account has {} access, so it cannot do this. Ask an administrator for {need} access.", id.role)));
    }
    if roles::needs_switch(op, id.provider.via()) && !st.remote.config().allow_remote_destructive {
        return Err(actions_off(op));
    }
    Ok(())
}

fn check(st: &AppState, req: &Request) -> Result<Caller, ApiError> {
    let h = req.headers();
    if PROXY_HEADERS.iter().any(|p| h.contains_key(*p)) {
        return Err(ApiError::forbidden(PROXY_REFUSED));
    }
    let peer = req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0);
    // No peer information (the router called directly, as the tests do) counts as local.
    let remote = peer.is_some_and(|a| remote::is_remote_peer(a.ip()));
    // DNS rebinding defence: every route, including static files.
    let host = h.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("");
    if !st.host_ok(host) {
        return Err(ApiError::forbidden("host not allowed"));
    }
    let method = req.method();
    let path = req.uri().path();
    // A program on this computer reaching the panel through the machine's own Tailscale address looks
    // remote, and Tailscale names the machine's owner for it: any local account would become an admin.
    // Only the page's first question (`/meta`) is answered, so it can explain.
    let api = path == "/api" || path.starts_with("/api/");
    let meta = method == Method::GET && path == "/api/v1/meta";
    if remote && api && !meta && peer.is_some_and(|p| st.remote.is_self_ip(p.ip())) {
        return Err(ApiError::forbidden(SELF_ADDRESS_REFUSED));
    }
    if remote && !st.remote.enabled() {
        return Err(ApiError::forbidden("Remote access is turned off on this computer."));
    }
    let safe = method == Method::GET || method == Method::HEAD;
    // The launch token is for the computer itself. Remote callers present a per-user token.
    let bearer_identity = match (bearer(h), peer) {
        (Some(t), Some(p)) if remote => {
            // Guessing tokens on ordinary requests is counted like guessing them at sign-in.
            let ip = p.ip();
            if st.remote.limiter.locked(ip) {
                return Err(ApiError::rate_limited("too many failed attempts, try again later"));
            }
            let found = st.remote.identity_for_token(t);
            if found.is_none() {
                st.remote.limiter.failure(ip);
            }
            found
        }
        (Some(t), _) if !remote => ct_eq(t, &st.token).then(Identity::local),
        _ => None,
    };
    let bearer_ok = bearer_identity.is_some();
    if !safe {
        if let Some(o) = h.get(header::ORIGIN) {
            let o = o.to_str().unwrap_or("");
            let ok = o.get(..7).is_some_and(|p| p.eq_ignore_ascii_case("http://")) && st.host_ok(&o[7..]);
            if !ok {
                return Err(ApiError::forbidden("origin not allowed"));
            }
        }
        if !bearer_ok && h.get("x-kora").and_then(|v| v.to_str().ok()) != Some("1") {
            return Err(ApiError::forbidden("missing X-Kora header"));
        }
    }
    let mut caller = Caller { remote, peer, identity: None };
    if path == "/api/v1" || path.starts_with("/api/v1/") {
        let session_identity = || {
            let secret = session_secret(h, method, path, req.uri().query());
            // Every cookie of that name: a planted one sent first must not hide the real one.
            cookie_values(h, &cookie_name(st.addr.port())).into_iter().find_map(|id| {
                let info = st.sessions.get_with_secret(&id, secret)?;
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
            })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn clock() -> (Clock, Arc<Mutex<Duration>>) {
        let base = Instant::now();
        let off = Arc::new(Mutex::new(Duration::ZERO));
        let o = off.clone();
        (Arc::new(move || base + *o.lock().unwrap()), off)
    }

    #[test]
    fn link_codes_work_once_and_expire() {
        let (c, off) = clock();
        let codes = LinkCodes::with_clock(c);
        let a = codes.mint();
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|b| b.is_ascii_hexdigit()));
        assert!(!codes.consume("nope") && !codes.consume(""));
        assert!(codes.consume(&a), "a fresh code works");
        assert!(!codes.consume(&a), "but only once");
        let b = codes.mint();
        *off.lock().unwrap() += LINK_CODE_TTL;
        assert!(!codes.consume(&b), "expired after ten minutes");
        let c2 = codes.mint();
        *off.lock().unwrap() += LINK_CODE_TTL - Duration::from_secs(1);
        assert!(codes.consume(&c2), "still fine just before");
    }

    #[test]
    fn link_codes_keep_only_the_newest() {
        let codes = LinkCodes::default();
        let first = codes.mint();
        let rest: Vec<String> = (0..LINK_CODES_MAX).map(|_| codes.mint()).collect();
        assert!(!codes.consume(&first), "the oldest was dropped");
        assert!(rest.iter().all(|c| codes.consume(c)));
    }

    #[test]
    fn ip_limiter_counts_each_address_alone() {
        let (c, off) = clock();
        let l = IpLimiter::with_clock(3, Duration::from_secs(60), Duration::from_secs(60), 100, c);
        let a: IpAddr = "100.64.0.1".parse().unwrap();
        let b: IpAddr = "100.64.0.2".parse().unwrap();
        for _ in 0..3 {
            assert!(!l.locked(a));
            l.failure(a);
        }
        assert!(l.locked(a), "locked after three");
        assert!(!l.locked(b), "another address is not");
        *off.lock().unwrap() += Duration::from_secs(60);
        assert!(!l.locked(a), "the lockout ends");
        // Failures spread out over more than the window never add up.
        for _ in 0..5 {
            l.failure(b);
            l.failure(b);
            *off.lock().unwrap() += Duration::from_secs(61);
        }
        assert!(!l.locked(b));
        l.failure(b);
        l.success(b);
        l.failure(b);
        l.failure(b);
        assert!(!l.locked(b), "success resets");
    }

    #[test]
    fn ipv6_is_counted_per_64_and_mapped_v4_as_v4() {
        let l = IpLimiter::new(2, Duration::from_secs(60), Duration::from_secs(60), 100);
        l.failure("fd7a:115c:a1e0:ab12::1".parse().unwrap());
        l.failure("fd7a:115c:a1e0:ab12:ffff::2".parse().unwrap());
        assert!(l.locked("fd7a:115c:a1e0:ab12::99".parse().unwrap()));
        assert!(!l.locked("fd7a:115c:a1e0:ab13::1".parse().unwrap()));
        l.failure("::ffff:10.0.0.1".parse().unwrap());
        l.failure("10.0.0.1".parse().unwrap());
        assert!(l.locked("10.0.0.1".parse().unwrap()));
    }

    #[test]
    fn ip_limiter_is_bounded() {
        let l = IpLimiter::new(2, Duration::from_secs(60), Duration::from_secs(60), 16);
        for i in 0..200u32 {
            l.failure(IpAddr::from(i.to_be_bytes()));
            assert!(l.tracked() <= 16);
        }
        let newest = IpAddr::from(199u32.to_be_bytes());
        l.failure(newest);
        assert!(l.locked(newest), "the newest is kept");
    }

    #[test]
    fn stream_limits_per_identity_and_overall() {
        let s = StreamLimits::new(2, 3);
        let a1 = s.acquire("a").unwrap();
        let _a2 = s.acquire("a").unwrap();
        let e = s.acquire("a").err().unwrap();
        assert_eq!(e.status, axum::http::StatusCode::TOO_MANY_REQUESTS);
        assert!(e.message.contains("Close some panel tabs"), "{}", e.message);
        let _b1 = s.acquire("b").unwrap();
        assert!(s.acquire("c").is_err(), "the overall cap");
        drop(a1);
        assert_eq!(s.open(), 2);
        assert!(s.acquire("a").is_ok(), "a closed stream frees its place");
    }

    #[test]
    fn session_secret_is_required_and_compared() {
        let s = Sessions::default();
        let n = s.create();
        assert!(s.get_with_secret(&n.id, None).is_none());
        assert!(s.get_with_secret(&n.id, Some("wrong")).is_none());
        assert!(s.get_with_secret(&n.id, Some(&n.secret)).is_some());
        assert_ne!(n.id, n.secret);
        assert!(!format!("{:?}", s.get(&n.id).unwrap()).contains(&n.secret));
    }

    #[test]
    fn secret_from_header_or_query_on_streams_only() {
        let mut h = HeaderMap::new();
        let ev = "/api/v1/events";
        assert_eq!(session_secret(&h, &Method::GET, ev, Some("a=1&ks=abc")), Some("abc"));
        for p in ["/api/v1/jobs/7/events", "/api/v1/servers/prod/console/stream"] {
            assert_eq!(session_secret(&h, &Method::GET, p, Some("ks=abc")), Some("abc"), "{p}");
        }
        // Not on other reads, not on HEAD, not on writes.
        for p in ["/api/v1/servers", "/api/v1/servers/prod", "/api/v1/remote", "/api/v1/meta"] {
            assert_eq!(session_secret(&h, &Method::GET, p, Some("ks=abc")), None, "{p}");
        }
        assert_eq!(session_secret(&h, &Method::HEAD, ev, Some("ks=abc")), None);
        assert_eq!(session_secret(&h, &Method::POST, ev, Some("ks=abc")), None);
        assert_eq!(session_secret(&h, &Method::GET, ev, None), None);
        h.insert(SESSION_HEADER, HeaderValue::from_static("hdr"));
        assert_eq!(session_secret(&h, &Method::POST, "/api/v1/servers", Some("ks=abc")), Some("hdr"));
    }

    #[test]
    fn ct_eq_compares_whole_lengths() {
        let long = format!("x{}", "\0".repeat(256));
        assert!(!ct_eq("x", &long), "lengths 1 and 257 differ only above the low byte");
        assert!(!ct_eq(&long, "x"));
        assert!(!ct_eq("", &"\0".repeat(256)));
        assert!(ct_eq("abc", "abc") && ct_eq("", ""));
        assert!(!ct_eq("abc", "abd") && !ct_eq("abc", "ab"));
    }

    #[test]
    fn every_cookie_with_the_name_is_read() {
        let mut h = HeaderMap::new();
        h.append(header::COOKIE, HeaderValue::from_static("kora_session_1=planted; other=1; kora_session_1=real"));
        h.append(header::COOKIE, HeaderValue::from_static("kora_session_1=third"));
        assert_eq!(cookie_values(&h, "kora_session_1"), ["planted", "real", "third"]);
        assert!(cookie_values(&h, "kora_session_2").is_empty());
    }

    #[test]
    fn a_full_limiter_never_unlocks_anyone() {
        let (c, off) = clock();
        let l = IpLimiter::with_clock(1, Duration::from_secs(60), Duration::from_secs(60), 4, c);
        let ip = |i: u32| IpAddr::from(i.to_be_bytes());
        for i in 0..4 {
            l.failure(ip(i));
        }
        assert!((0..4).all(|i| l.locked(ip(i))));
        // Every place holds a lockout: a new address neither pushes one out nor gets in.
        for i in 10..20 {
            assert!(l.locked(ip(i)), "unseen addresses wait");
            l.failure(ip(i));
        }
        assert!((0..4).all(|i| l.locked(ip(i))), "nobody was unlocked to make room");
        assert_eq!(l.tracked(), 4);
        // Once the lockouts end, new addresses are fine again.
        *off.lock().unwrap() += Duration::from_secs(60);
        assert!(!l.locked(ip(10)));
    }

    #[test]
    fn a_full_limiter_drops_unlocked_addresses_first() {
        let l = IpLimiter::new(2, Duration::from_secs(60), Duration::from_secs(60), 3);
        let ip = |i: u32| IpAddr::from(i.to_be_bytes());
        l.failure(ip(1));
        l.failure(ip(1)); // locked
        l.failure(ip(2)); // one failure, unlocked
        l.failure(ip(3));
        l.failure(ip(3)); // locked
        l.failure(ip(4)); // takes the place of 2
        assert!(l.locked(ip(1)) && l.locked(ip(3)));
        assert_eq!(l.tracked(), 3);
        l.failure(ip(4));
        assert!(l.locked(ip(4)), "the new address is counted");
    }
}
