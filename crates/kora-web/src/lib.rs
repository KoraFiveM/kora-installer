//! Kora Panel HTTP server: security middleware, sessions, jobs/SSE, embedded UI.
mod error;
pub mod elevated;
pub mod handlers;
mod jobs;
pub mod launch;
pub mod remote;
pub mod routes;
pub mod security;
mod static_ui;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{DefaultBodyLimit, Request, State};
use axum::Extension;
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{middleware, Json, Router};
use kora_core::api::{Backend, JobRegistry, Op};
use kora_core::installer::registry::ServerEntry;
use serde::Deserialize;
use serde_json::{json, Value};

pub use error::{blocking, ApiError};
pub use jobs::{JobMessage, Jobs, MemoryJobs, NoJobs};

pub const DEFAULT_PORT: u16 = 47120;
pub const BODY_LIMIT: usize = 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum PanelError {
    #[error("cannot listen on {addr}: {source} (is another panel running? choose --port)")]
    Bind {
        addr: SocketAddr,
        source: std::io::Error,
    },
    /// Remote access is not set up for this bind, or its settings file cannot be read.
    #[error("{0}")]
    Remote(String),
    #[error("server error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
pub struct PanelOptions {
    pub bind: IpAddr,
    pub port: u16,
    pub open_browser: bool,
    pub token: Option<String>,
    /// `--remote`: allow remote access for this run (a non-loopback `bind` needs this or saved settings).
    pub remote: bool,
    /// Where the remote access settings live; `None` is `<kora home>/panel.json`.
    pub remote_config: Option<PathBuf>,
    /// `--tailscale`: besides `bind` (loopback), also listen on this machine's Tailscale addresses.
    /// They are looked up when the panel starts (and again until Tailscale is up), never stored, so
    /// a changed address or a slow Tailscale at boot needs nothing from the user. Only while remote
    /// access is on in the settings: the settings decide who gets in.
    pub tailscale: bool,
    /// `--allow-public`: let `bind` be a public address or every interface (plain HTTP on the internet).
    pub allow_public: bool,
}

impl Default for PanelOptions {
    fn default() -> Self {
        Self {
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: DEFAULT_PORT,
            open_browser: true,
            token: None,
            remote: false,
            remote_config: None,
            tailscale: false,
            allow_public: false,
        }
    }
}

/// Makes a [`Backend`] for one request (the real one is `SystemBackend`; tests use a fake).
pub type BackendFactory = Arc<dyn Fn() -> Box<dyn Backend + Send> + Send + Sync>;

#[derive(Clone)]
pub struct AppState {
    pub token: Arc<String>,
    pub allowed_hosts: Arc<Vec<String>>,
    pub sessions: Arc<security::Sessions>,
    pub limiter: Arc<security::Limiter>,
    /// What `GET /jobs/{id}` and its SSE stream read.
    pub jobs: Arc<dyn Jobs>,
    /// Where handlers start jobs. In `run()` this is also `jobs`.
    pub registry: JobRegistry,
    pub backend: BackendFactory,
    /// The bound address (its port is part of every allowed `Host`).
    pub addr: SocketAddr,
    /// Optional remote access: settings, identity providers, roles. Off unless configured.
    pub remote: Arc<remote::Remote>,
    /// One-time codes for sign-in links (`/#code=...`).
    pub link_codes: Arc<security::LinkCodes>,
    /// Open live streams of remote callers.
    pub streams: Arc<security::StreamLimits>,
    /// The one poller behind every `GET /events` stream.
    pub events: Arc<handlers::system::EventsHub>,
    /// The last `GET /fleet` answer, for a few seconds.
    pub fleet_cache: Arc<handlers::hub::FleetCache>,
}

impl AppState {
    /// `addr` is the bound socket address; it defines the allowed `Host` values.
    pub fn new(token: String, addr: SocketAddr, jobs: Arc<dyn Jobs>) -> Self {
        let mut hosts = vec![addr.to_string()];
        if addr.ip().is_loopback() {
            hosts.push(format!("localhost:{}", addr.port()));
        }
        Self {
            token: Arc::new(token),
            allowed_hosts: Arc::new(hosts),
            sessions: Arc::default(),
            limiter: Arc::new(security::Limiter::new(10, Duration::from_secs(60))),
            jobs,
            registry: JobRegistry::new(),
            backend: Arc::new(|| Box::new(kora_core::api::SystemBackend)),
            addr,
            // Memory only by default: nothing on disk is read, so tests and embedders stay local.
            remote: Arc::new(remote::Remote::memory()),
            link_codes: Arc::default(),
            streams: Arc::default(),
            events: Arc::default(),
            fleet_cache: Arc::default(),
        }
    }
    /// A one-time code for a sign-in link: `http://127.0.0.1:<port>/#code=<code>`. Works once, for ten
    /// minutes, from this computer only.
    pub fn mint_link_code(&self) -> String {
        self.link_codes.mint()
    }
    /// A place for one more live stream: always for this computer; for a remote caller only while it
    /// (and everyone together) is under the caps.
    pub fn stream_permit(&self, caller: &remote::Caller) -> Result<Option<security::StreamPermit>, ApiError> {
        if !caller.remote {
            return Ok(None);
        }
        let who = caller
            .identity
            .as_ref()
            .map(|i| format!("{:?}:{}", i.provider, i.name))
            .unwrap_or_default();
        self.streams.acquire(&who).map(Some)
    }
    pub fn with_remote(mut self, r: Arc<remote::Remote>) -> Self {
        self.remote = r;
        self
    }
    /// Is `host` (a `Host` header, or the host part of an `Origin`) one this panel answers to? The bound
    /// address and `localhost` always; names from the remote settings only while remote access is on.
    pub fn host_ok(&self, host: &str) -> bool {
        if self.allowed_hosts.iter().any(|a| a.eq_ignore_ascii_case(host)) {
            return true;
        }
        self.remote.enabled() && remote::extra_host_allowed(host, &self.remote.extra_hosts(), self.addr.port())
    }
    /// The real setup: `SystemBackend` and one registry that both starts and serves jobs.
    pub fn system(token: String, addr: SocketAddr) -> Self {
        let registry = JobRegistry::new();
        let mut st = Self::new(token, addr, Arc::new(registry.clone()));
        st.registry = registry;
        st
    }
    pub fn with_backend(mut self, f: BackendFactory) -> Self {
        self.backend = f;
        self
    }
    /// Use `registry` for starting jobs and for `GET /jobs/...`.
    pub fn with_registry(mut self, registry: JobRegistry) -> Self {
        self.jobs = Arc::new(registry.clone());
        self.registry = registry;
        self
    }
    pub fn backend(&self) -> Box<dyn Backend + Send> {
        (self.backend)()
    }
    /// Find a registered server by name (blocking: call inside [`blocking`]).
    pub fn entry(&self, name: &str) -> Result<ServerEntry, ApiError> {
        let list = self.backend().list().map_err(ApiError::internal)?;
        list.into_iter()
            .find(|e| e.name == name)
            .ok_or_else(|| ApiError::not_found(format!("There is no server named '{name}'.")))
    }
    /// Start a job for `op`; `202 {"job": id}`, or `409 conflict` when the server is busy.
    pub fn spawn_job<F>(&self, op: Op, server: Option<String>, f: F) -> Result<Response, ApiError>
    where
        F: FnOnce(&kora_core::api::JobHandle) -> Result<Value, String> + Send + 'static,
    {
        // A job changes what the fleet shows: forget the cached list when it starts and when it ends.
        self.fleet_cache.clear();
        let cache = self.fleet_cache.clone();
        let body = move |h: &kora_core::api::JobHandle| {
            let r = f(h);
            cache.clear();
            r
        };
        match self.registry.spawn(op, server, body) {
            Ok(id) => Ok((StatusCode::ACCEPTED, Json(json!({"job": id.0}))).into_response()),
            Err(c) => Err(ApiError::conflict(conflict_message(&c))),
        }
    }
    /// Replace where session expiry reads the time (tests).
    pub fn with_session_clock(mut self, clock: security::Clock) -> Self {
        self.sessions = Arc::new(security::Sessions::with_clock(clock));
        self
    }
    pub fn with_limiter(mut self, max: u32, lockout: Duration) -> Self {
        self.limiter = Arc::new(security::Limiter::new(max, lockout));
        self
    }
}

/// "Busy" without the running job's id: the id is all it takes to follow that job, and the UI never
/// needed it here.
pub(crate) fn conflict_message(c: &kora_core::api::jobs::Conflict) -> String {
    format!(
        "Another operation is already running for '{}'. Wait for it to finish, then try again.",
        c.server
    )
}

#[derive(Deserialize)]
struct SessionReq {
    /// Local: the launch token. Remote: a user token, or absent to sign in by Tailscale identity.
    #[serde(default)]
    token: Option<String>,
    /// Local only: a one-time code from a sign-in link (`/#code=...`).
    #[serde(default)]
    code: Option<String>,
}

/// `GET /meta`. `identity` is `null` unless the request carries a valid session or bearer token;
/// then it is `{name, provider: local|tailscale|token, role: viewer|operator|admin}` and `remote` says
/// whether the request came from another computer. The UI reads these to hide what the role cannot do.
pub(crate) async fn meta(Extension(caller): Extension<remote::Caller>, State(st): State<AppState>) -> Json<Value> {
    let platform = if cfg!(windows) { "windows" } else { "linux" };
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "api": 1,
        "platform": platform,
        "ops": Op::ALL.iter().map(|o| o.name()).collect::<Vec<_>>(),
        "node": hostname(),
        "identity": caller.identity,
        "remote": caller.remote,
        "allow_remote_destructive": caller.remote && caller.identity.is_some() && st.remote.config().allow_remote_destructive,
    }))
}

fn hostname() -> String {
    for k in ["HOSTNAME", "COMPUTERNAME"] {
        if let Ok(v) = std::env::var(k) {
            if !v.is_empty() {
                return v;
            }
        }
    }
    std::fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_string())
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

/// `POST /session`. Local: `{"token"}` (the launch token, as pasted) or `{"code"}` (from a sign-in link).
/// Remote: `{"token"}` (a user token) or `{}` (Tailscale identity). Answers
/// `{"ok": true, "identity", "secret"}` and sets the session cookie; the page must send `secret` back with
/// every request (header `X-Kora-Session`, or `?ks=` on a stream).
pub(crate) async fn session_create(
    State(st): State<AppState>,
    Extension(caller): Extension<remote::Caller>,
    Json(req): Json<SessionReq>,
) -> Result<Response, ApiError> {
    let token = req.token.as_deref().map(str::trim).filter(|t| !t.is_empty());
    let code = req.code.as_deref().map(str::trim).filter(|t| !t.is_empty());
    let (session, identity) = if caller.remote {
        if code.is_some() {
            return Err(ApiError::forbidden(
                "Sign-in links only work on the computer that runs Kora. Sign in with your user token or your Tailscale account.",
            ));
        }
        remote_sign_in(&st, &caller, token).await?
    } else {
        if st.limiter.locked() {
            return Err(ApiError::rate_limited(
                "too many failed attempts, try again later",
            ));
        }
        let ok = match code {
            Some(c) => st.link_codes.consume(c),
            None => token.is_some_and(|t| security::ct_eq(t, &st.token)),
        };
        if !ok {
            st.limiter.failure();
            return Err(ApiError::unauthorized(if code.is_some() {
                "That sign-in link has expired or was already used. Run `kora panel` again for a new one."
            } else {
                "invalid token"
            }));
        }
        st.limiter.success();
        (st.sessions.create(), remote::Identity::local())
    };
    let cookie = format!(
        "{}={}; HttpOnly; SameSite=Strict; Path=/",
        security::cookie_name(st.addr.port()),
        session.id
    );
    let mut r = Json(json!({"ok": true, "identity": identity, "secret": session.secret})).into_response();
    r.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).expect("ascii cookie"),
    );
    // Older versions used one name for every port, sent to every service on this host: drop it.
    r.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&format!("{}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0", security::COOKIE)).expect("ascii cookie"),
    );
    Ok(r)
}

/// `POST /session/link`: a one-time sign-in code, `{"code", "expires_in": 600}`. Only for a caller on this
/// computer that is already signed in (the launch token as a bearer token, or a local session).
pub(crate) async fn session_link(
    State(st): State<AppState>,
    Extension(caller): Extension<remote::Caller>,
) -> Result<Json<Value>, ApiError> {
    let local = !caller.remote
        && caller
            .identity
            .as_ref()
            .is_some_and(|i| i.provider == remote::Provider::Local);
    if !local {
        return Err(ApiError::forbidden(
            "Sign-in links can only be made on the computer that runs Kora.",
        ));
    }
    Ok(Json(json!({
        "code": st.mint_link_code(),
        "expires_in": security::LINK_CODE_TTL.as_secs(),
    })))
}

/// `POST /session/logout`: forget this session and clear the cookie. 204.
pub(crate) async fn session_logout(State(st): State<AppState>, req_headers: axum::http::HeaderMap) -> Response {
    let name = security::cookie_name(st.addr.port());
    if let Some(id) = security::cookie_value(&req_headers, &name) {
        st.sessions.remove(&id);
    }
    let mut r = StatusCode::NO_CONTENT.into_response();
    r.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&format!("{name}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0")).expect("ascii cookie"),
    );
    r
}

/// Sign in from another computer: a per-user token, or (no token) the Tailscale identity of the peer.
/// The launch token is not accepted here. Failed tokens are counted on their own limiter.
async fn remote_sign_in(
    st: &AppState,
    caller: &remote::Caller,
    token: Option<&str>,
) -> Result<(security::NewSession, remote::Identity), ApiError> {
    let r = &st.remote;
    // Counted per source address: one person guessing does not lock out everyone else.
    let ip = caller.peer.map_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED), |p| p.ip());
    if r.limiter.locked(ip) {
        return Err(ApiError::rate_limited("too many failed attempts, try again later"));
    }
    let identity = match token {
        Some(t) => {
            let found = r.identity_for_token(t);
            if found.is_none() {
                r.limiter.failure(ip);
                return Err(ApiError::unauthorized("invalid token"));
            }
            found
        }
        None => {
            if !r.config().tailscale {
                return Err(ApiError::unauthorized("Sign in with your user token."));
            }
            let peer = caller.peer.ok_or_else(|| ApiError::unauthorized("Could not tell where you are connecting from."))?;
            let remote = st.remote.clone();
            let login = blocking(move || remote.whois(peer).map_err(ApiError::unauthorized)).await?;
            let found = r.identity_for_tailscale(&login);
            if found.is_none() {
                return Err(ApiError::unauthorized(format!(
                    "The Tailscale account {login} is not allowed on this panel. Ask an administrator to add it."
                )));
            }
            found
        }
    };
    let identity = identity.expect("checked above");
    r.limiter.success(ip);
    let session = st.sessions.create_for(identity.provider, &identity.name, true, identity.role);
    Ok((session, identity))
}

async fn fallback(req: Request) -> Response {
    let path = req.uri().path().to_string();
    if path == "/api" || path.starts_with("/api/") {
        return ApiError::not_found("unknown API route").into_response();
    }
    match *req.method() {
        Method::GET | Method::HEAD => static_ui::serve(&path),
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

pub fn router(state: AppState) -> Router {
    let api = routes::api_routes().fallback(|| async { ApiError::not_found("unknown API route") });
    Router::new()
        .nest("/api/v1", api)
        .fallback(fallback)
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            security::guard,
        ))
        .with_state(state)
}

/// Listen on this machine's Tailscale addresses as well (`--tailscale`), on the panel's port.
///
/// Tailscale may start after the panel (at boot) or change its addresses, so this asks it again:
/// every few seconds until it answers, then every minute for new addresses. Every listener serves the
/// same router: the guard sees the peer, so tailnet visitors are remote callers (Tailscale identity or a
/// user token, their role, no destructive actions unless allowed), never local ones.
async fn serve_tailnet(state: AppState, port: u16) {
    let mut bound: Vec<IpAddr> = Vec::new();
    let mut reported: Vec<IpAddr> = Vec::new();
    let mut announced = false;
    loop {
        let rem = state.remote.clone();
        let status = tokio::task::spawn_blocking(move || rem.tailscale_status())
            .await
            .ok()
            .flatten()
            .filter(|s| s.running);
        if let Some(s) = status {
            // The names a browser on the tailnet uses (MagicDNS name, short name, addresses).
            state.remote.set_dynamic_hosts(s.host_names());
            for ip in s.ips.iter().copied().filter(|ip| remote::is_tailnet_ip(*ip)) {
                if bound.contains(&ip) {
                    continue;
                }
                match tokio::net::TcpListener::bind(SocketAddr::new(ip, port)).await {
                    Ok(l) => {
                        bound.push(ip);
                        let app = router(state.clone()).into_make_service_with_connect_info::<SocketAddr>();
                        tokio::spawn(async move {
                            let _ = axum::serve(l, app).await;
                        });
                    }
                    Err(e) if !reported.contains(&ip) => {
                        reported.push(ip);
                        eprintln!("Could not listen on the Tailscale address {ip} port {port} ({e}). Kora keeps trying.");
                    }
                    Err(_) => {}
                }
            }
            if !bound.is_empty() {
                state.remote.set_tailnet_url(s.url(port));
            }
            if !announced && !bound.is_empty() {
                announced = true;
                if let Some(u) = s.url(port) {
                    println!("Your Tailscale devices can open the panel at {u}");
                }
            }
        }
        let wait = if bound.is_empty() { 5 } else { 60 };
        tokio::time::sleep(Duration::from_secs(wait)).await;
    }
}

/// Run the panel until Ctrl+C. Blocking; builds its own tokio runtime.
pub fn run(opts: PanelOptions) -> Result<(), PanelError> {
    let path = opts.remote_config.clone().unwrap_or_else(remote::config::default_path);
    let rem = remote::Remote::from_file(path)
        .map_err(PanelError::Remote)?
        .forced(opts.remote)
        .with_whois(Arc::new(remote::SystemWhois));
    let non_loopback = !opts.bind.to_canonical().is_loopback();
    if opts.tailscale && non_loopback {
        return Err(PanelError::Remote(
            "--tailscale already listens on this computer and on its Tailscale addresses: leave out --bind.".into(),
        ));
    }
    if non_loopback && rem.enabled() {
        rem.learn_self_names();
    }
    let warnings = remote::bind_plan(opts.bind, rem.enabled(), opts.allow_public, &rem.config(), &rem.extra_hosts())
        .map_err(PanelError::Remote)?;
    let rem = Arc::new(rem);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let addr = SocketAddr::new(opts.bind, opts.port);
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|source| PanelError::Bind { addr, source })?;
        let addr = listener.local_addr()?;
        let token = opts.token.clone().unwrap_or_else(security::random_hex);
        let state = AppState::system(token.clone(), addr).with_remote(rem);
        for w in &warnings {
            eprintln!("{w}");
        }
        if non_loopback {
            println!("Kora Panel: http://{addr}/");
            println!("Sign in with a user token (kora panel user add <name> --role <role>) or, if set up, your Tailscale account.");
        } else {
            let url = format!("http://{addr}/#token={token}");
            println!("Kora Panel: {url}");
            // The tunnel hint is for people on a machine without a desktop, whether or not we would have
            // tried to open a browser (`--no-open` must not hide it).
            if launch::is_headless() {
                println!("No screen here: on your own computer, run this first, then open the link there:");
                println!("  {}", launch::ssh_hint(addr.port(), &launch::real_env));
            } else if opts.open_browser {
                // Waits for the opener's exit status, so keep it off the server's threads.
                let url = url.clone();
                std::thread::spawn(move || {
                    if let Err(e) = launch::open_browser(&url) {
                        println!("Could not open a browser ({e}); open the link above yourself.");
                    }
                });
            }
        }
        if opts.tailscale {
            if state.remote.enabled() {
                tokio::spawn(serve_tailnet(state.clone(), addr.port()));
            } else {
                println!("Remote access is turned off in the settings, so the panel only listens on this computer. Turn it on with `kora panel --tailscale`.");
            }
        }
        println!("Press Ctrl+C to stop.");
        // Sample the servers and raise alerts for as long as the panel runs.
        let monitor = handlers::monitor::start_system_monitor(state.backend.clone());
        let served = axum::serve(
            listener,
            router(state).into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await;
        monitor.stop();
        served?;
        Ok(())
    })
}
