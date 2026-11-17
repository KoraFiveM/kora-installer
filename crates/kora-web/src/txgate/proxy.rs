//! The gateway's request handler: checks who asks, then streams the request to txAdmin and its answer
//! back (WebSocket upgrades included). See the module documentation of [`super`] for the design and the
//! threat model.

use std::net::SocketAddr;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode, Uri};
use axum::response::Response;
use axum::Router;
use hyper_util::rt::TokioIo;
use kora_core::api::Op;

use super::rewrite::{
    self, connection_tokens, cookies_for_txadmin, gate_cookie_name, is_websocket_upgrade, location_for_browser, request_header_dropped, response_header_dropped, set_cookie_for_browser,
};
use super::{page, GateSession};
use crate::remote::{self, roles};
use crate::security::{self, cookie_values};
use crate::AppState;

/// Most the request body of one proxied request may be (txAdmin's own limit for JSON is far lower; this
/// is for uploads such as a backup).
const BODY_LIMIT: usize = 64 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// txAdmin's socket.io long polls hold a request for about 25 s; anything slower is stuck.
const HEADER_TIMEOUT: Duration = Duration::from_secs(60);
/// Most WebSockets open through one gateway at once.
const WS_MAX: usize = 32;

#[derive(Clone)]
pub struct Ctx {
    pub st: AppState,
    /// The port this listener really has (the key of the route and of every cookie name).
    pub port: u16,
}

pub fn router(ctx: Ctx) -> Router {
    Router::new().fallback(handle).with_state(ctx)
}

static WS_OPEN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

struct WsPermit;
impl WsPermit {
    fn take() -> Option<Self> {
        use std::sync::atomic::Ordering::SeqCst;
        (WS_OPEN.fetch_add(1, SeqCst) < WS_MAX).then_some(Self).or_else(|| {
            WS_OPEN.fetch_sub(1, SeqCst);
            None
        })
    }
}
impl Drop for WsPermit {
    fn drop(&mut self) {
        WS_OPEN.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

fn deny(status: StatusCode, title: &str, text: &str) -> Response {
    page(status, title, text)
}

fn sign_in_again() -> Response {
    deny(StatusCode::UNAUTHORIZED, "Open txAdmin from the Kora panel", "This address only works after you open txAdmin from the Kora panel (the \"Open txAdmin\" button on the server). That link works once and for a short time, so go back to the panel and use the button again.")
}

/// Is this `Host` one the panel answers to, with this gateway's port?
fn host_allowed(st: &AppState, host: &str, port: u16) -> bool {
    let Some((name, p)) = (if host.starts_with('[') { host.find(']').and_then(|i| host[i + 1..].strip_prefix(':').map(|p| (&host[..=i], p))) } else { host.rsplit_once(':') }) else {
        return false;
    };
    p.parse::<u16>().ok() == Some(port) && !name.is_empty() && st.host_ok(&format!("{name}:{}", st.addr.port()))
}

/// The browser-facing origin of a request (`http://<host>`), for comparing with `Origin`.
fn origin_matches(origin: &HeaderValue, host: &str) -> bool {
    origin.to_str().ok().and_then(|o| o.strip_prefix("http://")).is_some_and(|h| h.eq_ignore_ascii_case(host))
}

/// May this gateway session still do this? The panel session must be alive and its role must still allow
/// txAdmin (a user removed or demoted, a switch turned off, a sign-out: all end it).
fn session_allowed(st: &AppState, gs: &GateSession) -> bool {
    let Some(info) = st.sessions.get(&gs.panel_session) else { return false };
    if info.remote != gs.remote {
        return false;
    }
    if !gs.remote {
        return info.provider == remote::Provider::Local;
    }
    match st.remote.resolve(info.provider, &info.name) {
        Some(i) if i.role == info.role => roles::remote_allowed(Op::TxAdminTicket, i.role, i.provider.via(), st.remote.config().allow_remote_destructive),
        _ => false,
    }
}

/// The peer rules the panel's guard applies to remote callers.
fn peer_allowed(st: &AppState, peer: Option<SocketAddr>) -> Result<bool, Response> {
    let remote = peer.is_some_and(|a| remote::is_remote_peer(a.ip()));
    if remote && !st.remote.enabled() {
        return Err(deny(StatusCode::FORBIDDEN, "Remote access is off", "Remote access is turned off on the computer that runs Kora."));
    }
    if remote && peer.is_some_and(|p| st.remote.is_self_ip(p.ip())) {
        return Err(deny(StatusCode::FORBIDDEN, "Not from this computer's own network address", security::SELF_ADDRESS_REFUSED));
    }
    Ok(remote)
}

async fn handle(State(ctx): State<Ctx>, peer: Option<ConnectInfo<SocketAddr>>, req: Request) -> Response {
    let st = &ctx.st;
    let h = req.headers();
    if security::PROXY_HEADERS.iter().any(|p| h.contains_key(*p)) {
        return deny(StatusCode::FORBIDDEN, "Not through a proxy", security::PROXY_REFUSED);
    }
    let host = h.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    if !host_allowed(st, &host, ctx.port) {
        return deny(StatusCode::FORBIDDEN, "Address not allowed", "This address is not one the Kora panel answers to.");
    }
    // Only origin-form targets (`/path?query`): an absolute-form target names another authority, and
    // the gateway never chooses where to connect from the request.
    if req.uri().authority().is_some() || req.uri().scheme().is_some() || !req.uri().path().starts_with('/') {
        return deny(StatusCode::BAD_REQUEST, "Bad request", "Unsupported request address.");
    }
    let remote = match peer_allowed(st, peer.map(|p| p.0)) {
        Ok(r) => r,
        Err(r) => return r,
    };
    if req.uri().path() == "/__kora/enter" {
        return enter(&ctx, req, remote).await;
    }
    // Who is this? The gateway cookie (every one of that name: another service on this host may set one).
    let cookie_name = gate_cookie_name(ctx.port);
    let Some((sid, gs)) = cookie_values(req.headers(), &cookie_name).into_iter().find_map(|id| st.txgate.session(&id).map(|s| (id, s))) else {
        return sign_in_again();
    };
    if gs.remote != remote || !session_allowed(st, &gs) {
        st.txgate.forget_session(&sid);
        return sign_in_again();
    }
    let Some(route) = st.txgate.fresh_route(st, ctx.port).await else {
        return deny(StatusCode::NOT_FOUND, "This server is gone", "The server this address belonged to was removed or its txAdmin port changed. Open txAdmin from the Kora panel again.");
    };
    if route.server != gs.server {
        return sign_in_again();
    }
    forward(&ctx, req, &host, route.txadmin_port, sid).await
}

/// `/__kora/enter?t=<ticket>`: turn a ticket into a gateway cookie and send the person to txAdmin.
async fn enter(ctx: &Ctx, req: Request, remote: bool) -> Response {
    if req.method() != Method::GET {
        return deny(StatusCode::METHOD_NOT_ALLOWED, "Not allowed", "Use the \"Open txAdmin\" button in the Kora panel.");
    }
    let ticket = req.uri().query().and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("t="))).unwrap_or("").to_string();
    let st = &ctx.st;
    let Some(t) = st.txgate.take_ticket(&ticket) else { return sign_in_again() };
    // The ticket was made for this port and for this kind of visitor, and its panel session is still alive.
    if t.gateway != ctx.port || t.remote != remote {
        return sign_in_again();
    }
    let now = std::time::Instant::now();
    let gs = GateSession { server: t.server.clone(), panel_session: t.panel_session.clone(), remote, created: now, last: now };
    if !session_allowed(st, &gs) {
        return sign_in_again();
    }
    let Some(route) = st.txgate.route(ctx.port) else { return sign_in_again() };
    if route.server != t.server {
        return sign_in_again();
    }
    let id = st.txgate.open_session(&t);
    let cookie = format!("{}={id}; HttpOnly; SameSite=Lax; Path=/", gate_cookie_name(ctx.port));
    let mut r = Response::new(Body::empty());
    *r.status_mut() = StatusCode::SEE_OTHER;
    r.headers_mut().insert(header::LOCATION, HeaderValue::from_static("/"));
    r.headers_mut().insert(header::SET_COOKIE, HeaderValue::from_str(&cookie).expect("ascii"));
    r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r.headers_mut().insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    r
}

fn bad_gateway(text: &str) -> Response {
    deny(StatusCode::BAD_GATEWAY, "txAdmin is not answering", text)
}

async fn forward(ctx: &Ctx, mut req: Request, host: &str, txadmin_port: u16, sid: String) -> Response {
    let upgrade = is_websocket_upgrade(req.headers());
    if rewrite::asks_for_upgrade(req.headers()) && !upgrade {
        return deny(StatusCode::BAD_REQUEST, "Bad request", "Only WebSocket upgrades are supported.");
    }
    let safe = matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    let origin = req.headers().get(header::ORIGIN).cloned();
    if let Some(o) = &origin {
        if !origin_matches(o, host) && (!safe || upgrade) {
            return deny(StatusCode::FORBIDDEN, "Origin not allowed", "This request came from another web page.");
        }
    } else if upgrade {
        // Browsers always send an Origin with a WebSocket handshake; without one this is not a page of txAdmin.
        return deny(StatusCode::FORBIDDEN, "Origin not allowed", "WebSocket connections must come from txAdmin's own page.");
    }
    let permit = if upgrade {
        match WsPermit::take() {
            Some(p) => Some(p),
            None => return deny(StatusCode::SERVICE_UNAVAILABLE, "Too many live views", "Close some txAdmin tabs and try again."),
        }
    } else {
        None
    };
    let client_upgrade = upgrade.then(|| hyper::upgrade::on(&mut req));
    let (parts, body) = req.into_parts();
    // Request line: origin-form path and query only.
    let pq = parts.uri.path_and_query().map(|p| p.as_str().to_string()).unwrap_or_else(|| "/".into());
    let Ok(uri) = pq.parse::<Uri>() else { return deny(StatusCode::BAD_REQUEST, "Bad request", "Unsupported request address.") };
    let listed = connection_tokens(&parts.headers);
    let mut up = hyper::Request::builder().method(parts.method.clone()).uri(uri).version(hyper::Version::HTTP_11);
    {
        let hdrs = up.headers_mut().expect("builder is fine");
        for (k, v) in &parts.headers {
            let n = k.as_str();
            if request_header_dropped(n) || listed.iter().any(|t| t == n) {
                continue;
            }
            hdrs.append(k.clone(), v.clone());
        }
        if let Some(c) = cookies_for_txadmin(&parts.headers, ctx.port) {
            hdrs.insert(header::COOKIE, c);
        }
        if upgrade {
            hdrs.insert(header::CONNECTION, HeaderValue::from_static("upgrade"));
            hdrs.insert(header::UPGRADE, HeaderValue::from_static("websocket"));
        }
    }
    let body = Body::new(http_body_util::Limited::new(body, BODY_LIMIT));
    let Ok(up) = up.body(body) else { return deny(StatusCode::BAD_REQUEST, "Bad request", "Unsupported request.") };

    let stream = match tokio::time::timeout(CONNECT_TIMEOUT, tokio::net::TcpStream::connect(SocketAddr::from(([127, 0, 0, 1], txadmin_port)))).await {
        Ok(Ok(s)) => s,
        _ => return bad_gateway(&format!("Kora could not reach txAdmin on port {txadmin_port} of this computer. Is the server running? It may still be starting: wait a minute and reload.")),
    };
    let _ = stream.set_nodelay(true);
    let Ok((mut sender, conn)) = hyper::client::conn::http1::handshake::<_, Body>(TokioIo::new(stream)).await else {
        return bad_gateway("txAdmin did not speak HTTP.");
    };
    tokio::spawn(async move {
        let _ = conn.with_upgrades().await;
    });
    let mut resp = match tokio::time::timeout(HEADER_TIMEOUT, sender.send_request(up)).await {
        Ok(Ok(r)) => r,
        Ok(Err(_)) => return bad_gateway("txAdmin closed the connection."),
        Err(_) => return bad_gateway("txAdmin did not answer in time."),
    };

    let status = resp.status();
    let mut out = Response::builder().status(status);
    {
        let oh = out.headers_mut().expect("builder is fine");
        let listed_resp = connection_tokens(resp.headers());
        for (k, v) in resp.headers() {
            let n = k.as_str();
            if (response_header_dropped(n) && !(upgrade && status == StatusCode::SWITCHING_PROTOCOLS && matches!(n, "connection" | "upgrade"))) || listed_resp.iter().any(|t| t == n) {
                continue;
            }
            if k == header::LOCATION {
                if let Ok(s) = v.to_str() {
                    if let Ok(nv) = HeaderValue::from_str(&location_for_browser(s, txadmin_port, host)) {
                        oh.append(k.clone(), nv);
                    }
                }
                continue;
            }
            oh.append(k.clone(), v.clone());
        }
        for v in resp.headers().get_all(header::SET_COOKIE) {
            if let Some(c) = v.to_str().ok().and_then(|s| set_cookie_for_browser(s, ctx.port)).and_then(|s| HeaderValue::from_str(&s).ok()) {
                oh.append(header::SET_COOKIE, c);
            }
        }
        if upgrade && status == StatusCode::SWITCHING_PROTOCOLS {
            oh.insert(header::CONNECTION, HeaderValue::from_static("upgrade"));
            oh.insert(header::UPGRADE, HeaderValue::from_static("websocket"));
        }
    }
    if upgrade {
        if status != StatusCode::SWITCHING_PROTOCOLS {
            // txAdmin said no: pass that on as an ordinary answer.
            return out.body(Body::new(resp.into_body())).unwrap_or_else(|_| bad_gateway("Unusable answer."));
        }
        let (Some(client_up), upstream_up) = (client_upgrade, hyper::upgrade::on(&mut resp)) else {
            return bad_gateway("The connection could not be upgraded.");
        };
        let st = ctx.st.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let (Ok(c), Ok(u)) = (client_up.await, upstream_up.await) else { return };
            let (mut c, mut u) = (TokioIo::new(c), TokioIo::new(u));
            let copy = tokio::io::copy_bidirectional(&mut c, &mut u);
            tokio::pin!(copy);
            let mut tick = tokio::time::interval(st.txgate.ws_recheck);
            loop {
                tokio::select! {
                    _ = &mut copy => break,
                    _ = tick.tick() => {
                        // Signed out, or no longer allowed: close the live console.
                        let ok = st.txgate.session(&sid).is_some_and(|gs| session_allowed(&st, &gs));
                        if !ok {
                            break;
                        }
                    }
                }
            }
        });
        return out.body(Body::empty()).unwrap_or_else(|_| bad_gateway("Unusable answer."));
    }
    out.body(Body::new(resp.into_body())).unwrap_or_else(|_| bad_gateway("Unusable answer."))
}
