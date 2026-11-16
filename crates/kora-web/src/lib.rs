//! Kora Panel HTTP server: security middleware, sessions, jobs/SSE, embedded UI.
mod error;
pub mod handlers;
mod jobs;
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
        }
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
        match self.registry.spawn(op, server, f) {
            Ok(id) => Ok((StatusCode::ACCEPTED, Json(json!({"job": id.0}))).into_response()),
            Err(c) => Err(ApiError::conflict(c.to_string())),
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

#[derive(Deserialize)]
struct SessionReq {
    /// Local: the launch token. Remote: a user token, or absent to sign in by Tailscale identity.
    #[serde(default)]
    token: Option<String>,
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

pub(crate) async fn session_create(
    State(st): State<AppState>,
    Extension(caller): Extension<remote::Caller>,
    Json(req): Json<SessionReq>,
) -> Result<Response, ApiError> {
    let token = req.token.as_deref().map(str::trim).filter(|t| !t.is_empty());
    let (id, identity) = if caller.remote {
        remote_sign_in(&st, &caller, token).await?
    } else {
        if st.limiter.locked() {
            return Err(ApiError::rate_limited(
                "too many failed attempts, try again later",
            ));
        }
        if !token.is_some_and(|t| security::ct_eq(t, &st.token)) {
            st.limiter.failure();
            return Err(ApiError::unauthorized("invalid token"));
        }
        st.limiter.success();
        (st.sessions.create(), remote::Identity::local())
    };
    let cookie = format!(
        "{}={id}; HttpOnly; SameSite=Strict; Path=/",
        security::COOKIE
    );
    let mut r = Json(json!({"ok": true, "identity": identity})).into_response();
    r.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).expect("ascii cookie"),
    );
    Ok(r)
}

/// `POST /session/logout`: forget this session and clear the cookie. 204.
pub(crate) async fn session_logout(State(st): State<AppState>, req_headers: axum::http::HeaderMap) -> Response {
    for v in req_headers.get_all(header::COOKIE) {
        for part in v.to_str().unwrap_or("").split(';') {
            if let Some((k, id)) = part.trim().split_once('=') {
                if k == security::COOKIE {
                    st.sessions.remove(id);
                }
            }
        }
    }
    let mut r = StatusCode::NO_CONTENT.into_response();
    r.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&format!("{}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0", security::COOKIE)).expect("ascii cookie"),
    );
    r
}

/// Sign in from another computer: a per-user token, or (no token) the Tailscale identity of the peer.
/// The launch token is not accepted here. Failed tokens are counted on their own limiter.
async fn remote_sign_in(
    st: &AppState,
    caller: &remote::Caller,
    token: Option<&str>,
) -> Result<(String, remote::Identity), ApiError> {
    let r = &st.remote;
    if r.limiter.locked() {
        return Err(ApiError::rate_limited("too many failed attempts, try again later"));
    }
    let identity = match token {
        Some(t) => {
            let found = r.identity_for_token(t);
            if found.is_none() {
                r.limiter.failure();
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
    r.limiter.success();
    let id = st.sessions.create_for(identity.provider, &identity.name, true, identity.role);
    Ok((id, identity))
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

fn is_headless() -> bool {
    if std::env::var_os("SSH_CONNECTION").is_some() {
        return true;
    }
    cfg!(unix)
        && !cfg!(target_os = "macos")
        && std::env::var_os("DISPLAY").is_none()
        && std::env::var_os("WAYLAND_DISPLAY").is_none()
}

fn open_browser(url: &str) -> bool {
    use std::process::{Command, Stdio};
    let mut c = if cfg!(windows) {
        let mut c = Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    c.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
}

/// Run the panel until Ctrl+C. Blocking; builds its own tokio runtime.
pub fn run(opts: PanelOptions) -> Result<(), PanelError> {
    let path = opts.remote_config.clone().unwrap_or_else(remote::config::default_path);
    let rem = remote::Remote::from_file(path)
        .map_err(PanelError::Remote)?
        .forced(opts.remote)
        .with_whois(Arc::new(remote::SystemWhois));
    let non_loopback = !opts.bind.to_canonical().is_loopback();
    if non_loopback && rem.enabled() {
        rem.learn_self_names();
    }
    let warnings = remote::bind_plan(opts.bind, rem.enabled(), &rem.config(), &rem.extra_hosts()).map_err(PanelError::Remote)?;
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
            if is_headless() {
                println!("No local desktop session detected; open the URL on your machine with:");
                println!("  ssh -L {0}:127.0.0.1:{0} user@host", addr.port());
            } else if opts.open_browser && !open_browser(&url) {
                println!("Could not open a browser; open the URL above manually.");
            }
        }
        println!("Press Ctrl+C to stop.");
        axum::serve(
            listener,
            router(state).into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
        Ok(())
    })
}
