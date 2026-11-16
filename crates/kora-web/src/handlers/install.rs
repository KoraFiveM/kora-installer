//! The install wizard: defaults, checks, artifacts, plan, the install job and its log.
//!
//! Each `pub async fn` is mounted by `crate::routes` for the [`Op`] of the same name; the HTTP
//! method and path come from `Op::route`, so only the handler's extractors live here. The logic
//! is in `kora_core::api::install`; everything that touches the machine goes through
//! [`InstallEnv`], which tests replace with [`set_env`].
#![allow(clippy::unused_async)]

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::Json;
use kora_core::api::install::{
    self as core, InstallEnv, SystemInstallEnv, WireDbAdmin, WireInstall,
};
use kora_core::api::Op;
use kora_core::installer::{slug, Mode};
use serde_json::{json, Value};

use crate::{blocking, ApiError, AppState};

static ENV: RwLock<Option<Arc<dyn InstallEnv>>> = RwLock::new(None);

/// Replace the machine seam (tests). `None` goes back to the real machine.
pub fn set_env(e: Option<Arc<dyn InstallEnv>>) {
    *ENV.write().unwrap_or_else(|p| p.into_inner()) = e;
}

/// The machine seam in use.
pub fn env() -> Arc<dyn InstallEnv> {
    ENV.read()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
        .unwrap_or_else(|| Arc::new(SystemInstallEnv))
}

fn parse<T: for<'de> serde::Deserialize<'de>>(v: Value) -> Result<T, ApiError> {
    serde_json::from_value(v).map_err(|_| {
        ApiError::bad_request("That request was not understood. Reload the page and try again.")
    })
}

fn port_of(v: &Value, key: &str, label: &str) -> Result<u16, ApiError> {
    v.get(key)
        .and_then(Value::as_u64)
        .and_then(|n| u16::try_from(n).ok())
        .ok_or_else(|| {
            ApiError::bad_request(format!(
                "The {label} port must be a number between 1024 and 65535."
            ))
        })
}

/// `GET /artifacts` → `{recommended, items: [{build, url}]}` (builds known to be broken are left out).
pub async fn artifacts_list() -> Result<Json<Value>, ApiError> {
    let e = env();
    let list = blocking(move || {
        e.artifacts().map_err(|m| {
            ApiError::new(StatusCode::BAD_GATEWAY, "internal", format!("Could not read the list of FiveM server builds ({m}). Check your internet connection and try again."))
        })
    })
    .await?;
    Ok(Json(core::artifacts_json(&list)))
}

/// `GET /installs/defaults` → `{name, dir, hostname, ports, saved}`.
pub async fn install_defaults(State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    let e = env();
    let v = blocking(move || {
        let servers = st.backend().list().map_err(ApiError::internal)?;
        Ok(core::defaults(&*e, &servers))
    })
    .await?;
    Ok(Json(v))
}

/// `DELETE /installs/saved`: forget the answers of an install that stopped part-way.
pub async fn install_saved_clear() -> Result<StatusCode, ApiError> {
    let e = env();
    blocking(move || {
        e.saved_clear();
        Ok(())
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /checks/ports` `{game, txadmin}` → `{game: {port, ok, text, suggestion}, txadmin: {...}}`.
pub async fn check_ports(
    State(st): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let (game, tx) = (
        port_of(&body, "game", "game")?,
        port_of(&body, "txadmin", "txAdmin")?,
    );
    let e = env();
    let v = blocking(move || {
        let servers = st.backend().list().map_err(ApiError::internal)?;
        let (g, t) = core::port_lines(game, tx, &servers, e.prober());
        Ok(json!({"game": g.json(), "txadmin": t.json()}))
    })
    .await?;
    Ok(Json(v))
}

/// `POST /checks/disk` `{dir}` → `{ok, free_bytes, needed_bytes, message}`.
pub async fn check_disk(Json(body): Json<Value>) -> Result<Json<Value>, ApiError> {
    let dir = body
        .get("dir")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    if dir.is_empty() {
        return Err(ApiError::bad_request(
            "Choose a folder for the server first.",
        ));
    }
    let e = env();
    let v =
        blocking(move || Ok(core::disk_check(e.disk(), std::path::Path::new(&dir), None).json()))
            .await?;
    Ok(Json(v))
}

/// `POST /checks/db-login` `{admin: {user, password, host, port} | null}` → `{ok, message}`.
/// The password is only used for the test; it is never stored, logged or returned.
pub async fn check_db_login(Json(body): Json<Value>) -> Result<Json<Value>, ApiError> {
    let admin = match body.get("admin") {
        None | Some(Value::Null) => None,
        Some(a) => Some(parse::<WireDbAdmin>(a.clone())?.to_admin()),
    };
    if let Some(a) = &admin {
        kora_core::db::validate_host(&a.host).map_err(|_| {
            ApiError::bad_request("That database server address does not look right.")
        })?;
    }
    let e = env();
    let automatic = admin.is_none();
    let v = blocking(move || {
        Ok(match e.db_login(admin.as_ref()) {
            Ok(()) => json!({"ok": true, "message": if automatic { "Kora can log in to the database automatically." } else { "Logged in to the database." }}),
            Err(m) => json!({"ok": false, "message": m}),
        })
    })
    .await?;
    Ok(Json(v))
}

/// `POST /installs/plan`: the same checks as starting, nothing changed, no secrets in the answer.
/// → `{summary, steps, needs_admin, elevated, disk, warnings, notes}`.
pub async fn install_plan(
    State(st): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let wire: WireInstall = parse(body)?;
    let e = env();
    let v = blocking(move || {
        let req = core::build_request(&wire).map_err(ApiError::bad_request)?;
        let servers = st.backend().list().map_err(ApiError::internal)?;
        Ok(core::plan(&*e, &req, &servers))
    })
    .await?;
    Ok(Json(v))
}

// ---- the install job's log ------------------------------------------------------------------

const KEEP_LOGS: usize = 20;
const MAX_LOG_LINES: usize = 5000;
const TAIL: usize = 500;

#[derive(Default)]
struct LogEntry {
    path: Option<PathBuf>,
    lines: Vec<String>,
}

#[derive(Default)]
struct Logs {
    by_job: HashMap<String, LogEntry>,
    order: VecDeque<String>,
}

static LOGS: Mutex<Option<Logs>> = Mutex::new(None);

fn with_logs<T>(f: impl FnOnce(&mut Logs) -> T) -> T {
    let mut g = LOGS.lock().unwrap_or_else(|p| p.into_inner());
    f(g.get_or_insert_with(Logs::default))
}

fn log_begin(job: &str) {
    with_logs(|l| {
        l.order.push_back(job.to_string());
        l.by_job.insert(job.to_string(), LogEntry::default());
        while l.order.len() > KEEP_LOGS {
            if let Some(old) = l.order.pop_front() {
                l.by_job.remove(&old);
            }
        }
    });
}

fn log_push(job: &str, line: String) {
    with_logs(|l| {
        if let Some(e) = l.by_job.get_mut(job) {
            if e.lines.len() < MAX_LOG_LINES {
                e.lines.push(line);
            }
        }
    });
}

fn log_finish(job: &str, path: Option<PathBuf>) {
    // The file also holds the error text, so prefer it once it exists.
    let from_file = path
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| t.lines().map(String::from).collect::<Vec<_>>());
    with_logs(|l| {
        if let Some(e) = l.by_job.get_mut(job) {
            e.path = path;
            if let Some(lines) = from_file {
                e.lines = lines;
            }
        }
    });
}

/// `GET /installs/{job}/log` → `{path, lines}` (the end of the log; `path` is empty until the log is written).
pub async fn install_log(Path(job): Path<String>) -> Result<Json<Value>, ApiError> {
    with_logs(|l| {
        l.by_job.get(&job).map(|e| {
            let start = e.lines.len().saturating_sub(TAIL);
            json!({"path": e.path.as_ref().map(|p| p.to_string_lossy().into_owned()), "lines": &e.lines[start..]})
        })
    })
    .map(Json)
    .ok_or_else(|| ApiError::not_found("There is no install with that id."))
}

// ---- starting an install --------------------------------------------------------------------

/// `POST /installs`: body = the UI's install request. Validates like the CLI, then runs the
/// installer as a job: every `ProgressEvent` is a job event (`{step, title, status, detail,
/// percent}`), the result is the outcome (`connect`, `txadmin_url`, `admin_user`,
/// `admin_password_file`, `notes`, ...; never the password). A failure is a JSON error string with
/// `message`, `kept` and `log_path`. Without administrator rights for what the install needs:
/// `needs_elevation` with the `kora install ...` command (no secrets on it).
pub async fn install_start(
    State(st): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let wire: WireInstall = parse(body)?;
    let e = env();
    let e2 = e.clone();
    let st2 = st.clone();
    let req = blocking(move || {
        let mut req = core::build_request(&wire).map_err(ApiError::bad_request)?;
        if req.dry_run {
            return Ok(req);
        }
        if req.license_key.is_none() {
            return Err(ApiError::bad_request("A licence key is needed. Get one from the Cfx.re portal: https://portal.cfx.re/servers/registration-keys"));
        }
        let servers = st2.backend().list().map_err(ApiError::internal)?;
        if servers.iter().any(|s| slug(&s.name) == slug(&req.name) || s.dir == req.dir) {
            return Err(ApiError::conflict(format!("A server called '{}' (or in that folder) already exists. Choose another name or folder.", req.name)));
        }
        if req.mode == Mode::Development {
            let owner = super::github::env().resolve_owner(req.github_owner.as_deref()).map_err(ApiError::bad_request)?;
            req.github_owner = Some(owner);
        }
        if !core::already_installed(&req) {
            let avoid = core::avoid_ports(&servers, &req.name, Some(&req.dir));
            core::settle_ports(&mut req, false, e2.prober(), &avoid).map_err(|m| ApiError::conflict(format!("{m} Go back and pick other ports.")))?;
        }
        let reasons = e2.admin_reasons(&req);
        if !reasons.is_empty() && !e2.is_elevated() {
            return Err(ApiError::needs_elevation(core::elevation_message(&req, &reasons), core::install_command(&req)));
        }
        Ok(req)
    })
    .await?;
    let name = req.name.clone();
    st.spawn_job(Op::InstallStart, Some(name), move |h| {
        let id = h.id().0.clone();
        log_begin(&id);
        let fin = core::run_install(&*e, &req, |ev| {
            h.event(core::progress_json(ev));
            log_push(&id, core::log_line(ev));
        });
        log_finish(&id, fin.log_path.clone());
        match fin.outcome {
            Ok(out) => Ok(core::outcome_json(&out)),
            Err(msg) => {
                let mut err = json!({"message": if fin.next.is_empty() { msg } else { format!("{msg}\n\n{}", fin.next) }, "kept": fin.kept});
                if let Some(p) = &fin.log_path {
                    err["log_path"] = json!(p.to_string_lossy());
                }
                Err(err.to_string())
            }
        }
    })
}
