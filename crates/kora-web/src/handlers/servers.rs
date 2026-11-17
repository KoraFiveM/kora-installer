//! The server list, one server, start/stop/restart, delete, adopt, open folder, logs.
//!
//! Each `pub async fn` is mounted by `crate::routes` for the [`Op`] of the same name; the HTTP
//! method and path come from `Op::route`, so only the handler's extractors live here.
#![allow(clippy::unused_async)]

use std::path::{Path as FsPath, PathBuf};
use std::sync::Mutex;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use kora_core::api::{adopt, logs, wire, Backend, Op, Status, UpdateBadge};
use kora_core::installer::registry::{self, ServerEntry};
use kora_core::service::ControlAction;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{blocking, ApiError, AppState};

/// Kora installed this server (it has its install record); otherwise it was adopted.
/// The manager decides the same way.
pub(crate) fn is_managed(e: &ServerEntry) -> bool {
    e.dir.join(".kora").join("request.json").is_file()
}

fn summary(b: &mut dyn Backend, e: &ServerEntry) -> wire::ServerSummary {
    let status = b.status(e);
    let badge = b.badge(e);
    summary_with(e, status, badge)
}

fn summary_with(e: &ServerEntry, status: Status, badge: Option<UpdateBadge>) -> wire::ServerSummary {
    wire::ServerSummary {
        name: e.name.clone(),
        dir: e.dir.clone(),
        mode: e.mode,
        source: e.source(),
        versions: e.versions(),
        ports: kora_core::installer::Ports { game: e.ports.game, txadmin: e.ports.txadmin },
        created_unix: e.created_unix,
        status,
        badge: badge.map(wire::Badge::from),
        managed: is_managed(e),
        connect: connect_address(e.ports.game),
    }
}

/// `<this computer's network address>:<game port>`, the address someone on the same network types to
/// join, or `null` when the address cannot be found. It is NOT an address players on the internet can
/// use: that is the router's public address (which Kora cannot see) with the same port forwarded.
fn connect_address(game_port: u16) -> Option<String> {
    kora_core::launch::lan_address().map(|ip| format!("{ip}:{game_port}"))
}

/// The body of `GET /servers` (also what the events stream sends).
pub(crate) fn list_json(b: &mut dyn Backend) -> Result<Value, ApiError> {
    let list = b.list().map_err(ApiError::internal)?;
    let servers: Vec<wire::ServerSummary> = list.iter().map(|e| summary(b, e)).collect();
    Ok(serde_json::to_value(wire::ServersList { servers }).unwrap_or(Value::Null))
}

pub async fn servers_list(State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    blocking(move || list_json(&mut *st.backend()).map(Json)).await
}

pub async fn server_get(State(st): State<AppState>, Path(s): Path<String>) -> Result<Json<wire::ServerDetail>, ApiError> {
    blocking(move || {
        let e = st.entry(&s)?;
        let mut b = st.backend();
        let detail = b.status_detail(&e);
        let summary = summary(&mut *b, &e);
        Ok(Json(wire::ServerDetail {
            summary,
            running: detail.running,
            game_listening: detail.game_listening,
            txadmin_up: detail.txadmin_up,
            status_detail: detail,
            console_enabled: kora_core::console::is_installed(&e.dir),
        }))
    })
    .await
}

#[derive(Deserialize)]
pub struct ControlBody {
    action: String,
}

pub async fn server_control(State(st): State<AppState>, Path(s): Path<String>, Json(body): Json<ControlBody>) -> Result<Response, ApiError> {
    let action = match body.action.as_str() {
        "start" => ControlAction::Start,
        "stop" => ControlAction::Stop,
        "restart" => ControlAction::Restart,
        _ => return Err(ApiError::bad_request("The action must be start, stop or restart.")),
    };
    let st2 = st.clone();
    let e = blocking(move || st2.entry(&s)).await?;
    if blocking({
        let (st, e) = (st.clone(), e.clone());
        move || Ok(st.backend().status(&e))
    })
    .await?
        == Status::Missing
    {
        return Err(ApiError::conflict("The folder for this server is missing, so it cannot be changed."));
    }
    let name = e.name.clone();
    let stc = st.clone();
    st.spawn_job(Op::ServerControl, Some(name), move |h| {
        let message = match action {
            ControlAction::Start => "Starting",
            ControlAction::Stop => "Stopping",
            ControlAction::Restart => "Restarting",
        };
        h.event(to_value(&wire::StatusEvent::Status { message: message.into() }));
        let mut b = stc.backend();
        b.control(&e, action)?;
        Ok(to_value(&wire::ControlResult { status: b.status(&e) }))
    })
}

use wire::DeleteRequest as DeleteBody;

/// A wire value as the JSON a job stores.
fn to_value<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

pub async fn server_delete(State(st): State<AppState>, Path(s): Path<String>, Json(body): Json<DeleteBody>) -> Result<Response, ApiError> {
    let stc = st.clone();
    let (e, running) = blocking(move || {
        let e = stc.entry(&s)?;
        let running = stc.backend().status(&e) == Status::Running;
        Ok((e, running))
    })
    .await?;
    if body.confirm_name != e.name {
        return Err(ApiError::bad_request("The name you typed does not match this server."));
    }
    if !is_managed(&e) {
        return Err(ApiError::bad_request("This server was added from an existing folder, so Kora can only remove it from the list."));
    }
    if running {
        return Err(ApiError::conflict("Stop the server first, then delete it."));
    }
    // Windows: removing the service, firewall rules or update task (or dropping the database with
    // Kora's own MariaDB login) needs the elevated worker: one permission prompt, asked when the
    // delete reaches those parts.
    let admin = if crate::elevated::service_ready() {
        let (dir, keep_db, pw) = (e.dir.clone(), body.keep_db, body.db_admin_password.as_deref().is_some_and(|p| !p.is_empty()));
        let needs = blocking(move || Ok(!kora_core::elevate::is_elevated() && kora_core::installer::system_ops::delete_needs_admin_at(&dir, keep_db, pw))).await?;
        if needs {
            let mut command = vec!["kora".to_string(), "delete".into(), e.name.clone(), "--yes".into()];
            if body.keep_files {
                command.push("--keep-files".into());
            }
            if body.keep_db {
                command.push("--keep-db".into());
            }
            let ctx = kora_core::privsvc::problems::Context { what: "remove the server's background service, firewall rules and database".into(), command, can_skip_admin: false };
            crate::elevated::precheck(&ctx, "Removing the server's background service needs administrator rights. Run this command as administrator:")?;
            Some(kora_core::installer::system_ops::SharedAdmin(std::sync::Arc::new(crate::elevated::ServiceAdmin::new(ctx))))
        } else {
            None
        }
    } else {
        None
    };
    let name = e.name.clone();
    let stc = st.clone();
    st.spawn_job(Op::ServerDelete, Some(name), move |h| {
        h.event(to_value(&wire::StatusEvent::Status { message: "Deleting the server".into() }));
        let notes = stc.backend().delete_with_admin(&e, body.keep_files, body.keep_db, body.db_admin_password.as_deref(), admin).map_err(delete_error)?;
        Ok(to_value(&wire::Notes { notes }))
    })
}

/// A failed database drop is reported as `{code: "needs_db_password", message}` so the panel can ask for the
/// administrator password and try again; the message is written for the panel, not for the command line.
fn delete_error(e: String) -> String {
    if e.contains("could not remove database") && e.contains("KORA_DB_ADMIN_PASSWORD") {
        let what = e.split(". Everything else").next().unwrap_or("could not remove the database");
        let what = what.strip_prefix("could not remove ").unwrap_or(what);
        return json!({
            "code": "needs_db_password",
            "message": format!("Everything else was removed, but the {what} could not be deleted: Kora needs the database administrator password. Enter it and try again, or keep the database and delete the server without it."),
        })
        .to_string();
    }
    e
}

pub async fn server_unregister(State(st): State<AppState>, Path(s): Path<String>) -> Result<Response, ApiError> {
    blocking(move || {
        let e = st.entry(&s)?;
        if is_managed(&e) {
            return Err(ApiError::bad_request("Kora installed this server, so use Delete server instead."));
        }
        st.backend().unregister(&e).map_err(ApiError::core)?;
        Ok(StatusCode::NO_CONTENT.into_response())
    })
    .await
}

pub async fn servers_scan(State(st): State<AppState>) -> Result<Response, ApiError> {
    let stc = st.clone();
    st.spawn_job(Op::ServersScan, None, move |_h| {
        let mut b = stc.backend();
        let roots = b.scan_roots();
        let known: Vec<PathBuf> = b.list()?.into_iter().map(|e| e.dir).collect();
        let found = adopt::scan(&roots, 3, &known);
        let candidates = found
            .iter()
            .map(|c| wire::ScanCandidate {
                name: c.name.clone(),
                dir: c.dir.clone(),
                mode: c.mode,
                kind: c.kind.into(),
                ports: kora_core::installer::Ports { game: c.ports.game, txadmin: c.ports.txadmin },
            })
            .collect();
        Ok(to_value(&wire::ScanResult { candidates }))
    })
}

use wire::AdoptRequest as AdoptBody;

pub async fn servers_adopt(State(st): State<AppState>, Json(body): Json<AdoptBody>) -> Result<Json<wire::ServerSummary>, ApiError> {
    blocking(move || {
        let p = PathBuf::from(body.dir.trim());
        if !p.is_absolute() {
            return Err(ApiError::bad_request("Type the full folder path (starting from the drive or /)."));
        }
        if !p.is_dir() {
            return Err(ApiError::bad_request(format!("{} is not a folder.", p.display())));
        }
        let mut b = st.backend();
        let list = b.list().map_err(ApiError::internal)?;
        if list.iter().any(|e| registry::same_path(&e.dir, &p)) {
            return Err(ApiError::bad_request("That server is already in the list."));
        }
        let Some(c) = adopt::classify(&p) else {
            return Err(ApiError::bad_request("That folder doesn't look like an FXServer or Kora server (no server.cfg + txData, FXServer program, artifacts or .kora folder)."));
        };
        let taken: Vec<String> = list.iter().map(|e| e.name.clone()).collect();
        let entry = adopt::to_entry(&c, &taken);
        b.register(entry.clone()).map_err(ApiError::core)?;
        Ok(Json(summary(&mut *b, &entry)))
    })
    .await
}

// ---------------------------------------------------------------- open folder

type Opener = Box<dyn Fn(&FsPath) -> Result<(), String> + Send + Sync>;
static OPENER: Mutex<Option<Opener>> = Mutex::new(None);

/// Tests only: replace the program that opens a folder (`None` = the real one).
pub fn set_opener(f: Option<Opener>) {
    *OPENER.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = f;
}

fn open_real(dir: &FsPath) -> Result<(), String> {
    use std::process::{Command, Stdio};
    let prog = if cfg!(windows) { "explorer.exe" } else { "xdg-open" };
    Command::new(prog).arg(dir).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map(|_| ()).map_err(|_| "Could not open the folder on this computer.".to_string())
}

pub async fn server_open_folder(State(st): State<AppState>, Path(s): Path<String>) -> Result<Response, ApiError> {
    blocking(move || {
        let e = st.entry(&s)?;
        if super::system::headless() {
            return Err(ApiError::bad_request("This computer has no desktop, so the folder can't be opened here. The server's folder is ".to_string() + &e.dir.display().to_string() + "."));
        }
        if !e.dir.is_dir() {
            return Err(ApiError::conflict("The folder for this server is missing."));
        }
        let g = OPENER.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        match g.as_ref() {
            Some(f) => f(&e.dir),
            None => open_real(&e.dir),
        }
        .map_err(ApiError::internal)?;
        Ok(StatusCode::NO_CONTENT.into_response())
    })
    .await
}

// ---------------------------------------------------------------- logs

pub async fn logs_list(State(st): State<AppState>, Path(s): Path<String>) -> Result<Json<wire::LogsList>, ApiError> {
    blocking(move || {
        let e = st.entry(&s)?;
        let files: Vec<wire::LogFile> = logs::discover(&e.dir)
            .into_iter()
            .map(|l| {
                let m = std::fs::metadata(&l.path).ok();
                let modified = m.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
                wire::LogFile { name: l.name.to_string(), size: m.map_or(0, |m| m.len()), modified_unix: modified }
            })
            .collect();
        Ok(Json(wire::LogsList { files }))
    })
    .await
}

use wire::LogTailQuery as TailQuery;

pub async fn logs_tail(State(st): State<AppState>, Path((s, file)): Path<(String, String)>, Query(q): Query<TailQuery>) -> Result<Json<wire::LogTail>, ApiError> {
    blocking(move || {
        let e = st.entry(&s)?;
        if file.contains('/') || file.contains('\\') || file.contains("..") {
            return Err(ApiError::bad_request("That is not a log file name."));
        }
        let src = logs::discover(&e.dir).into_iter().find(|l| l.name == file).ok_or_else(|| ApiError::not_found("That log file does not exist."))?;
        let n = q.lines.unwrap_or(300).clamp(1, logs::MAX_LINES);
        let lines = logs::tail(&src.path, n).map_err(ApiError::internal)?;
        Ok(Json(wire::LogTail { file, lines }))
    })
    .await
}
