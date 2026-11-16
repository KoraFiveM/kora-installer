//! Kora Panel HTTP server: security middleware, sessions, jobs/SSE, embedded UI.
mod error;
mod jobs;
pub mod ops_placeholder;
pub mod security;
mod static_ui;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{middleware, Json, Router};
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
    #[error("server error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
pub struct PanelOptions {
    pub bind: IpAddr,
    pub port: u16,
    pub open_browser: bool,
    pub token: Option<String>,
}

impl Default for PanelOptions {
    fn default() -> Self {
        Self {
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: DEFAULT_PORT,
            open_browser: true,
            token: None,
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub token: Arc<String>,
    pub allowed_hosts: Arc<Vec<String>>,
    pub sessions: Arc<security::Sessions>,
    pub limiter: Arc<security::Limiter>,
    pub jobs: Arc<dyn Jobs>,
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
        }
    }
    pub fn with_limiter(mut self, max: u32, lockout: Duration) -> Self {
        self.limiter = Arc::new(security::Limiter::new(max, lockout));
        self
    }
}

#[derive(Deserialize)]
struct SessionReq {
    token: String,
}

async fn meta() -> Json<Value> {
    let platform = if cfg!(windows) { "windows" } else { "linux" };
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "api": 1,
        "platform": platform,
        "ops": ops_placeholder::OPS,
        "node": hostname(),
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

async fn session_create(
    State(st): State<AppState>,
    Json(req): Json<SessionReq>,
) -> Result<Response, ApiError> {
    if st.limiter.locked() {
        return Err(ApiError::rate_limited(
            "too many failed attempts, try again later",
        ));
    }
    if !security::ct_eq(&req.token, &st.token) {
        st.limiter.failure();
        return Err(ApiError::unauthorized("invalid token"));
    }
    st.limiter.success();
    let id = st.sessions.create();
    let cookie = format!(
        "{}={id}; HttpOnly; SameSite=Strict; Path=/",
        security::COOKIE
    );
    let mut r = Json(json!({"ok": true})).into_response();
    r.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).expect("ascii cookie"),
    );
    Ok(r)
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
    let api = Router::new()
        .route("/meta", get(meta))
        .route("/session", post(session_create))
        .route("/jobs/:id", get(jobs::job_get))
        .route("/jobs/:id/events", get(jobs::job_events))
        .fallback(|| async { ApiError::not_found("unknown API route") });
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
        let state = AppState::new(token.clone(), addr, Arc::new(NoJobs));
        let url = format!("http://{addr}/#token={token}");
        println!("Kora Panel: {url}");
        if opts.open_browser {
            if is_headless() {
                println!("No local desktop session detected; open the URL on your machine with:");
                println!("  ssh -L {0}:127.0.0.1:{0} user@host", addr.port());
            } else if !open_browser(&url) {
                println!("Could not open a browser; open the URL above manually.");
            }
        }
        println!("Press Ctrl+C to stop.");
        axum::serve(listener, router(state))
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await?;
        Ok(())
    })
}
