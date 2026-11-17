//! txAdmin through the panel: a gateway that signs people in with the panel's own session and then
//! proxies them to the server's txAdmin on this computer's loopback address.
//!
//! # Why a port of its own, not `/txadmin/<server>/` on the panel's port
//!
//! txAdmin's web interface cannot run under a path prefix. Verified against txAdmin v8.1.2
//! (commit 1db5eb0, `tabarra/txAdmin`): the panel's JavaScript reaches txAdmin with absolute paths
//! (`panel/src/lib/utils.ts` opens socket.io with `path: '/socket.io'`; `AccountDialog.tsx`,
//! `Login.tsx` and others call `/auth/...`, `/adminManager/...`), its router is rooted at `/`
//! (`panel/src/main.tsx` only knows the one special prefix `/WebPipe`, used by the in-game menu
//! with its own token), and nothing in `core/` rewrites those paths. Under `/txadmin/<server>/` the
//! page would load and then talk to the PANEL's `/auth/...` and `/socket.io`. Rewriting a minified
//! bundle on the way through would break with every txAdmin release. So the gateway gives each
//! server's txAdmin an address of its own on the panel's side:
//!
//! * one TCP port per server, [`gateway_port`] of txAdmin's own port (40120 -> 50120), bound on the
//!   very interfaces the panel is bound on (never wider), so "reachable through the panel" and
//!   "reachable at all" are the same people;
//! * people get there with [`txadmin_ticket`]: a one-time, one-minute ticket made by an authenticated
//!   panel request (session cookie AND the per-session secret header, the role check and the remote
//!   gate of `security.rs`), bound to that panel session;
//! * `/__kora/enter?t=<ticket>` on the gateway port turns the ticket into a gateway cookie that names
//!   the panel session. Every request re-checks that the panel session still exists and that the role
//!   still allows it, so signing out of the panel (or removing a user) ends txAdmin access too;
//! * after that every request is proxied to `127.0.0.1:<txAdmin port of that server>`, streaming,
//!   including WebSocket upgrades (txAdmin's live console is socket.io), see [`proxy`].
//!
//! # Threat model
//!
//! txAdmin is full control of the game server, including running code (resources, `exec`), so opening
//! it needs the **admin** role and counts as a destructive action: a remote session signed in with a
//! user token needs `kora panel remote-actions on`, exactly as console input does (`roles.rs`).
//!
//! * No session, no ticket: nothing is proxied (401/403 page). The panel's cookies and its secret
//!   header are never sent to txAdmin; txAdmin's cookies are renamed per gateway ([`rewrite`]) so two
//!   servers never share a session and nothing txAdmin sets can replace the panel's cookies.
//! * Never an open proxy: the target is `127.0.0.1` and the txAdmin port the registry holds for the
//!   server the gateway port belongs to; the request's own authority (absolute-form targets, `Host`)
//!   is never used to choose where to connect.
//! * `x-txadmin-token`/`x-txadmin-identifiers` (txAdmin treats such a request as FXServer's own, no
//!   login needed), `Authorization`, proxy headers and hop-by-hop headers never reach txAdmin.
//!   Requests that carry proxy headers (`X-Forwarded-For`...) are refused like the panel refuses them.
//! * `Host` must be a name the panel answers to, with the gateway port; WebSocket handshakes must
//!   carry an `Origin` equal to that host; other unsafe requests with an `Origin` must match it too.
//! * txAdmin sees every visitor as 127.0.0.1: its own per-address rate limit for failed sign-ins is
//!   shared and its flood protection does not apply. The panel session in front of it is what keeps
//!   strangers out.
//!
//! # Sign-in
//!
//! txAdmin offers no token or single sign-on for its web interface (only a password and the
//! Cfx.re account; `core/modules/WebServer/router.ts`), so Kora does not try to log in for the person:
//! they land on txAdmin's own login page. The ticket answer carries the account name Kora created at
//! install (never the password) so the panel can say which one to use and where the password is saved.

pub mod proxy;
pub mod rewrite;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use kora_core::api::Op;

use crate::remote::{roles, Provider};
use crate::{blocking, security, ApiError, AppState};

/// A ticket works once, for this long.
pub const TICKET_TTL: Duration = Duration::from_secs(60);
/// Tickets kept at once; making another drops the oldest.
pub const TICKETS_MAX: usize = 64;
/// A gateway session with no request for this long ends.
pub const GATE_IDLE: Duration = Duration::from_secs(8 * 60 * 60);
/// A gateway session ends this long after it began, however busy.
pub const GATE_MAX: Duration = Duration::from_secs(24 * 60 * 60);
/// How long the registry lookup behind a gateway port is trusted.
const ROUTE_TTL: Duration = Duration::from_secs(10);
/// Most gateway sessions kept at once.
const SESSIONS_MAX: usize = 256;

/// The gateway port of a server whose txAdmin listens on `txadmin`: ten thousand above it (40120 ->
/// 50120), or ten thousand below when that would leave the port range. A fixed rule, so a firewall
/// rule or an SSH tunnel written once stays right.
pub fn gateway_port(txadmin: u16) -> Option<u16> {
    txadmin.checked_add(10_000).or_else(|| txadmin.checked_sub(10_000)).filter(|p| *p >= 1024)
}

#[derive(Clone)]
struct Ticket {
    server: String,
    panel_session: String,
    remote: bool,
    gateway: u16,
    expires: Instant,
}

#[derive(Clone)]
pub(crate) struct GateSession {
    pub server: String,
    pub panel_session: String,
    pub remote: bool,
    created: Instant,
    last: Instant,
}

#[derive(Clone)]
pub(crate) struct Route {
    pub server: String,
    pub txadmin_port: u16,
    checked: Instant,
}

type PortFn = Arc<dyn Fn(u16) -> Option<u16> + Send + Sync>;

#[derive(Default)]
struct Inner {
    tickets: Vec<(String, Ticket)>,
    sessions: HashMap<String, GateSession>,
    routes: HashMap<u16, Route>,
    /// Interfaces the panel is bound on (the gateway listens on exactly these).
    ips: Vec<IpAddr>,
    listeners: HashMap<(IpAddr, u16), tokio::task::AbortHandle>,
}

pub struct Gate {
    inner: Mutex<Inner>,
    port_fn: PortFn,
    /// Where "now" comes from (tests step over the timeouts).
    clock: Arc<dyn Fn() -> Instant + Send + Sync>,
    /// How often an open WebSocket re-checks that the person may still use txAdmin.
    pub(crate) ws_recheck: Duration,
}

impl Default for Gate {
    fn default() -> Self {
        Self { inner: Mutex::default(), port_fn: Arc::new(gateway_port), clock: Arc::new(Instant::now), ws_recheck: Duration::from_secs(20) }
    }
}

impl Gate {
    /// Choose gateway ports another way (tests bind port 0).
    pub fn with_port_fn(mut self, f: impl Fn(u16) -> Option<u16> + Send + Sync + 'static) -> Self {
        self.port_fn = Arc::new(f);
        self
    }
    pub fn with_ws_recheck(mut self, d: Duration) -> Self {
        self.ws_recheck = d;
        self
    }
    pub fn with_clock(mut self, c: Arc<dyn Fn() -> Instant + Send + Sync>) -> Self {
        self.clock = c;
        self
    }
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    fn now(&self) -> Instant {
        (self.clock)()
    }

    /// The panel listens on one more address (Tailscale): gateways already open follow it.
    pub fn add_ip(&self, st: &AppState, ip: IpAddr) {
        let ports: Vec<u16> = {
            let mut g = self.lock();
            if !g.ips.contains(&ip) {
                g.ips.push(ip);
            }
            g.routes.keys().copied().collect()
        };
        for p in ports {
            let (gate, st) = (st.txgate.clone(), st.clone());
            tokio::spawn(async move {
                let _ = gate.listen_on(&st, ip, p).await;
            });
        }
    }

    fn ips(&self, st: &AppState) -> Vec<IpAddr> {
        let mut g = self.lock();
        if !g.ips.contains(&st.addr.ip()) {
            g.ips.insert(0, st.addr.ip());
        }
        g.ips.clone()
    }

    fn mint_ticket(&self, t: Ticket) -> String {
        let id = security::random_hex();
        let now = self.now();
        let mut g = self.lock();
        g.tickets.retain(|(_, t)| now < t.expires);
        while g.tickets.len() >= TICKETS_MAX {
            g.tickets.remove(0);
        }
        g.tickets.push((id.clone(), t));
        id
    }

    fn take_ticket(&self, id: &str) -> Option<Ticket> {
        let now = self.now();
        let mut g = self.lock();
        g.tickets.retain(|(_, t)| now < t.expires);
        let i = g.tickets.iter().position(|(c, _)| security::ct_eq(c, id))?;
        Some(g.tickets.remove(i).1)
    }

    fn open_session(&self, t: &Ticket) -> String {
        let id = security::random_hex();
        let now = self.now();
        let mut g = self.lock();
        g.sessions.retain(|_, s| now.saturating_duration_since(s.last) < GATE_IDLE && now.saturating_duration_since(s.created) < GATE_MAX);
        if g.sessions.len() >= SESSIONS_MAX {
            if let Some(oldest) = g.sessions.iter().min_by_key(|(_, s)| s.last).map(|(k, _)| k.clone()) {
                g.sessions.remove(&oldest);
            }
        }
        g.sessions.insert(id.clone(), GateSession { server: t.server.clone(), panel_session: t.panel_session.clone(), remote: t.remote, created: now, last: now });
        id
    }

    pub(crate) fn session(&self, id: &str) -> Option<GateSession> {
        let now = self.now();
        let mut g = self.lock();
        let s = g.sessions.get_mut(id)?;
        if now.saturating_duration_since(s.last) >= GATE_IDLE || now.saturating_duration_since(s.created) >= GATE_MAX {
            g.sessions.remove(id);
            return None;
        }
        s.last = now;
        Some(s.clone())
    }

    pub(crate) fn forget_session(&self, id: &str) {
        self.lock().sessions.remove(id);
    }

    pub(crate) fn route(&self, gateway: u16) -> Option<Route> {
        self.lock().routes.get(&gateway).cloned()
    }

    fn set_route(&self, gateway: u16, server: &str, txadmin_port: u16) -> Result<(), String> {
        let mut g = self.lock();
        if let Some(r) = g.routes.get(&gateway) {
            if r.server != server {
                return Err(format!("Port {gateway} is already the txAdmin address of the server '{}'.", r.server));
            }
        }
        g.routes.insert(gateway, Route { server: server.to_string(), txadmin_port, checked: self.now() });
        Ok(())
    }

    /// Re-read the registry when the cached answer is old: a server that was removed, or whose txAdmin
    /// port changed, stops being reachable here. `None`: the gateway port no longer belongs to anyone.
    pub(crate) async fn fresh_route(&self, st: &AppState, gateway: u16) -> Option<Route> {
        let r = self.route(gateway)?;
        if self.now().saturating_duration_since(r.checked) < ROUTE_TTL {
            return Some(r);
        }
        let (st2, name) = (st.clone(), r.server.clone());
        let entry = blocking(move || st2.entry(&name)).await.ok();
        let port_fn = self.port_fn.clone();
        match entry {
            Some(e) if port_fn(e.ports.txadmin).is_some() && e.ports.txadmin == r.txadmin_port => {
                let mut g = self.lock();
                if let Some(x) = g.routes.get_mut(&gateway) {
                    x.checked = self.now();
                }
                Some(r)
            }
            _ => {
                let mut g = self.lock();
                g.routes.remove(&gateway);
                // Sessions of that server end with its route.
                g.sessions.retain(|_, s| s.server != r.server);
                None
            }
        }
    }

    /// Listen for `gateway` on `ip`. Already listening is fine.
    async fn listen_on(self: &Arc<Self>, st: &AppState, ip: IpAddr, gateway: u16) -> Result<u16, String> {
        if let Some(_h) = self.lock().listeners.get(&(ip, gateway)) {
            return Ok(gateway);
        }
        let listener = tokio::net::TcpListener::bind(SocketAddr::new(ip, gateway))
            .await
            .map_err(|e| format!("Kora could not listen on {ip} port {gateway} for txAdmin ({e}). Is something else using that port?"))?;
        let actual = listener.local_addr().map_err(|e| e.to_string())?.port();
        let app = proxy::router(proxy::Ctx { st: st.clone(), port: actual });
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await;
        });
        self.lock().listeners.insert((ip, actual), handle.abort_handle());
        Ok(actual)
    }
}

/// `Host` of the request without its port: what the person typed to reach the panel (an address or a name).
fn host_name(host: &str) -> &str {
    if host.starts_with('[') {
        return host.find(']').map_or(host, |i| &host[..=i]);
    }
    host.rsplit_once(':').map_or(host, |(n, _)| n)
}

/// The username Kora made the txAdmin master account with at install, from the folder's private note.
/// Only that one line: the password in the same file never leaves it.
fn saved_txadmin_username(dir: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("secrets").join("txadmin-admin.txt")).ok()?;
    let name = text.lines().find_map(|l| l.strip_prefix("Username:"))?.trim().to_string();
    (!name.is_empty() && name.len() <= 64 && name.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))).then_some(name)
}

/// `POST /servers/{s}/txadmin/ticket`: open txAdmin through the panel. Answers
/// `{url, port, expires_in, username?}`; the page opens `url` in a new tab.
pub async fn txadmin_ticket(
    State(st): State<AppState>,
    Extension(caller): Extension<crate::remote::Caller>,
    headers: HeaderMap,
    Path(s): Path<String>,
) -> Result<Json<kora_core::api::wire::TxAdminTicket>, ApiError> {
    let name = s.clone();
    let st2 = st.clone();
    let e = blocking(move || st2.entry(&name)).await?;
    // The session that proves who asks: cookie and secret header together. A bearer token alone names
    // no browser session that a gateway session could follow.
    let Some(panel_session) = security::current_session(&st, &headers, caller.remote) else {
        return Err(ApiError::forbidden("Open txAdmin from the Kora panel in your browser (signed in), not from a script."));
    };
    let Some(identity) = caller.identity.clone() else { return Err(ApiError::unauthorized("missing or invalid session")) };
    // Defence in depth: the guard already did this for remote callers.
    if caller.remote && !roles::remote_allowed(Op::TxAdminTicket, identity.role, identity.provider.via(), st.remote.config().allow_remote_destructive) {
        return Err(ApiError::forbidden("Your account may not open txAdmin."));
    }
    if !caller.remote && identity.provider != Provider::Local {
        return Err(ApiError::forbidden("Open txAdmin from the Kora panel."));
    }
    let Some(gw_wanted) = (st.txgate.port_fn)(e.ports.txadmin) else {
        return Err(ApiError::bad_request("This server's txAdmin port cannot get a panel address. Change the txAdmin port to one between 1024 and 55000."));
    };
    // Another server's gateway already owns this port: say so before opening anything (0 = "any", tests).
    if gw_wanted != 0 {
        if let Some(r) = st.txgate.route(gw_wanted) {
            if r.server != e.name {
                return Err(ApiError::conflict(format!("Port {gw_wanted} is already the txAdmin address of the server '{}'.", r.server)));
            }
        }
    }
    let mut actual = None;
    let mut first_err = None;
    for ip in st.txgate.ips(&st) {
        match st.txgate.listen_on(&st, ip, gw_wanted).await {
            Ok(p) => actual = Some(p),
            Err(m) => first_err = first_err.or(Some(m)),
        }
    }
    let Some(port) = actual else {
        return Err(ApiError::conflict(first_err.unwrap_or_else(|| "Kora could not open txAdmin's address.".into())));
    };
    // The route is keyed by the port that is really open.
    st.txgate.set_route(port, &e.name, e.ports.txadmin).map_err(ApiError::conflict)?;
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("127.0.0.1");
    let ticket = st.txgate.mint_ticket(Ticket { server: e.name.clone(), panel_session, remote: caller.remote, gateway: port, expires: st.txgate.now() + TICKET_TTL });
    let dir = e.dir.clone();
    let username = blocking(move || Ok(saved_txadmin_username(&dir))).await.ok().flatten();
    Ok(Json(kora_core::api::wire::TxAdminTicket { url: format!("http://{}:{port}/__kora/enter?t={ticket}", host_name(host)), port, expires_in: TICKET_TTL.as_secs(), username }))
}

/// Static pages of the gateway itself (never txAdmin's).
pub(crate) fn page(status: StatusCode, title: &str, text: &str) -> Response {
    let body = format!("<!doctype html><meta charset=utf-8><title>{title}</title><body style=\"font:16px system-ui;max-width:34rem;margin:4rem auto;padding:0 1rem\"><h1>{title}</h1><p>{text}</p>");
    let mut r = (status, body).into_response();
    r.headers_mut().insert(header::CONTENT_TYPE, axum::http::HeaderValue::from_static("text/html; charset=utf-8"));
    r.headers_mut().insert(header::CACHE_CONTROL, axum::http::HeaderValue::from_static("no-store"));
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_ports_follow_a_fixed_rule() {
        assert_eq!(gateway_port(40120), Some(50120));
        assert_eq!(gateway_port(55535), Some(65535));
        assert_eq!(gateway_port(55536), Some(45536));
        assert_eq!(gateway_port(65000), Some(55000));
        assert_eq!(gateway_port(1500), Some(11500));
        assert_eq!(gateway_port(1000), Some(11000));
        // Never below the privileged range.
        assert_eq!(gateway_port(65535), Some(55535));
    }

    #[test]
    fn host_names_lose_only_the_port() {
        assert_eq!(host_name("192.168.1.5:47120"), "192.168.1.5");
        assert_eq!(host_name("my-pc.tail1234.ts.net:47120"), "my-pc.tail1234.ts.net");
        assert_eq!(host_name("[fd7a::1]:47120"), "[fd7a::1]");
        assert_eq!(host_name("localhost"), "localhost");
    }

    #[test]
    fn the_saved_username_is_read_and_the_password_is_not() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("secrets")).unwrap();
        std::fs::write(d.path().join("secrets/txadmin-admin.txt"), "txAdmin master account\nURL: x\nUsername: kora-admin\nPassword: hunter2hunter2\n").unwrap();
        assert_eq!(saved_txadmin_username(d.path()).as_deref(), Some("kora-admin"));
        std::fs::write(d.path().join("secrets/txadmin-admin.txt"), "Username: bad name; rm\n").unwrap();
        assert_eq!(saved_txadmin_username(d.path()), None);
    }
}
