//! Kora itself: self-update, the Kora app, uninstall.
//!
//! Each `pub async fn` is mounted by `crate::routes` for the [`Op`] of the same name; the HTTP
//! method and path come from `Op::route`, so only the handler's extractors live here.
//!
//! Everything that would change the machine goes through [`kora_core::api::maintenance`], which tests
//! replace with a fake. Where the CLI would ask for administrator rights, these answer `needs_elevation`
//! with the command to run instead (the panel itself runs as the signed-in user).
#![allow(clippy::unused_async)]

use axum::extract::{Query, State};
use axum::response::Response;
use axum::Json;
use kora_core::api::maintenance::{self as maint, UninstallRequest};
use kora_core::api::Op;
use kora_core::app_install::AppOptions;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{blocking, ApiError, AppState};
use kora_core::privsvc::problems::Context;

/// A job error that keeps a Problem's code and choices (the declined prompt's "Ask again", ...).
fn job_error(e: ApiError) -> kora_core::api::JobFailure {
    kora_core::api::JobFailure::new(e.to_problem())
}

fn cmd(parts: &[&str]) -> Vec<String> {
    let mut v = vec!["kora".to_string()];
    v.extend(parts.iter().map(|s| (*s).to_string()));
    v
}

/// An optional JSON body: a missing or empty body means "all defaults".
fn body<T: serde::de::DeserializeOwned + Default>(bytes: &axum::body::Bytes) -> Result<T, ApiError> {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(T::default());
    }
    serde_json::from_slice(bytes).map_err(|_| ApiError::bad_request("The request was not understood."))
}

pub async fn self_update_check() -> Result<Json<Value>, ApiError> {
    let info = blocking(|| maint::current().self_update_info().map_err(ApiError::core)).await?;
    Ok(Json(json!({
        "current": info.current,
        "latest": info.latest,
        "available": info.available,
        "update_available": info.available,
        "versions": info.versions,
    })))
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct SelfUpdateReq {
    pub version: Option<String>,
}

pub async fn self_update_apply(State(st): State<AppState>, bytes: axum::body::Bytes) -> Result<Response, ApiError> {
    let req: SelfUpdateReq = body(&bytes)?;
    let version = req.version.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    if let Some(v) = &version {
        if v.len() > 40 || !v.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+')) {
            return Err(ApiError::bad_request("That is not a version number."));
        }
    }
    let v2 = version.clone();
    let needs = blocking(move || Ok(maint::current().self_update_needs_admin())).await?;
    if needs {
        let mut c = cmd(&["self-update", "--yes"]);
        if let Some(v) = &v2 {
            c.push("--version".into());
            c.push(v.clone());
        }
        let msg = "Updating Kora needs administrator rights. Run this command as administrator:";
        let what = "update Kora";
        crate::elevated::precheck_verb(&Context { what: what.into(), command: c.clone(), can_skip_admin: false }, msg)?;
        // The administrator service does it: the same command, run as administrator, as a job.
        return st.spawn_job(Op::SelfUpdateApply, None, move |h| {
            let out = crate::elevated::privileged_as(what, msg, c, vec![], &mut |m| h.event(json!({"type": "status", "message": m}))).map_err(job_error)?;
            Ok(json!({"version": v2, "output": out["output"]}))
        });
    }
    st.spawn_job(Op::SelfUpdateApply, None, move |h| {
        let new = maint::current().self_update_apply(version.as_deref(), &mut |m| h.event(json!({"type": "status", "message": m})))?;
        Ok(json!({"version": new}))
    })
}

pub async fn app_status() -> Result<Json<Value>, ApiError> {
    let s = blocking(|| Ok(maint::current().app_status())).await?;
    Ok(Json(json!({
        // The old machine-wide app (kept under these names for older UIs).
        "installed": s.installed,
        "version": s.version,
        "path": s.path,
        "desktop_shortcut": s.desktop_shortcut,
        "supported": s.supported,
        // Since 0.7.0: is the old app, or an old administrator shortcut, still here ("Tidy up old shortcuts")?
        "legacy": s.legacy,
        "legacy_items": s.legacy_items,
        // The per-user shortcut that opens the panel is in place.
        "shortcuts": s.shortcuts,
    })))
}

#[derive(Deserialize)]
#[serde(default)]
pub struct AppInstallReq {
    pub desktop_shortcut: bool,
    /// Accepted from older UIs; there is nothing to force any more.
    pub force: bool,
    pub dry_run: bool,
}
impl Default for AppInstallReq {
    fn default() -> Self {
        Self { desktop_shortcut: true, force: false, dry_run: false }
    }
}

/// "Tidy up old shortcuts": make the per-user shortcuts that open the panel (no administrator rights, done
/// first so the old ones keep working until the end), then remove the old machine-wide copy and its
/// shortcuts. That last part needs administrator rights: through the helper when it is installed, otherwise
/// the `needs_elevation` answer with the command to run.
pub async fn app_install(bytes: axum::body::Bytes) -> Result<Json<Value>, ApiError> {
    let req: AppInstallReq = body(&bytes)?;
    blocking(move || {
        let m = maint::current();
        let made = m.app_install(&AppOptions { desktop_shortcut: req.desktop_shortcut, force: false, dry_run: req.dry_run }).map_err(ApiError::core)?;
        let old = m.app_status();
        let made_text = if made.is_empty() { "The Kora shortcuts are already in place".to_string() } else { format!("The Kora shortcuts are ready: {}", made.join("; ")) };
        if req.dry_run {
            let tail = if old.legacy { format!(" Removing the old copy would: {}.", old.legacy_items.join("; ")) } else { String::new() };
            return Ok(Json(json!({"message": format!("Nothing was changed. {made_text}.{tail}")})));
        }
        if !old.legacy {
            return Ok(Json(json!({"message": format!("{made_text}.")})));
        }
        if maint::tidy_needs_admin(m.as_ref(), false) {
            let msg = "Your new Kora shortcuts are ready, and the old ones keep working. Removing the old copy of Kora needs administrator rights. Run this command as administrator:";
            crate::elevated::privileged_as("remove the old copy of Kora", msg, cmd(&["uninstall", "--app", "--yes"]), vec![], &mut |_| {})?;
        } else {
            m.app_uninstall(false).map_err(ApiError::core)?;
        }
        Ok(Json(json!({"message": "Done. Kora now opens from your own shortcuts, and the old copy and its administrator shortcuts are gone. Your servers were not touched."})))
    })
    .await
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct AppUninstallReq {
    pub dry_run: bool,
}

pub async fn app_uninstall(Query(q): Query<AppUninstallReq>, bytes: axum::body::Bytes) -> Result<Json<Value>, ApiError> {
    // The UI sends `{dry_run}` as a body; a query string works too.
    let b: AppUninstallReq = body(&bytes)?;
    let dry_run = b.dry_run || q.dry_run;
    blocking(move || {
        let m = maint::current();
        if maint::app_change_needs_admin(m.as_ref(), dry_run) {
            crate::elevated::privileged_as(
                "remove the Kora app",
                "Removing the Kora app needs administrator rights. Run this command as administrator:",
                cmd(&["uninstall", "--app", "--yes"]),
                vec![],
                &mut |_| {},
            )?;
            return Ok(Json(json!({"message": "The Kora app was removed. Your servers and databases were not touched."})));
        }
        let lines = m.app_uninstall(dry_run).map_err(ApiError::core)?;
        let head = if dry_run { "Nothing was changed. Removing the Kora app would" } else { "The Kora app was removed" };
        let tail = if lines.is_empty() { String::new() } else { format!(": {}", lines.join("; ")) };
        Ok(Json(json!({"message": format!("{head}{tail}. Your servers and databases were not touched.")})))
    })
    .await
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct PlanReq {
    pub server: Option<String>,
}

pub async fn uninstall_plan(State(st): State<AppState>, bytes: axum::body::Bytes) -> Result<Json<Value>, ApiError> {
    let req: PlanReq = body(&bytes)?;
    let only = req.server.filter(|s| !s.is_empty());
    if let Some(n) = only.clone() {
        let st2 = st.clone();
        blocking(move || st2.entry(&n).map(|_| ())).await?;
    }
    let items = blocking(move || maint::current().uninstall_plan(only.as_deref()).map_err(ApiError::core)).await?;
    let items: Vec<Value> = items.iter().map(|i| json!({"item": i.item, "description": i.description, "kind": i.kind, "remove": i.remove})).collect();
    Ok(Json(json!({"items": items})))
}

#[derive(Deserialize)]
pub struct Answer {
    pub item: String,
    pub remove: bool,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct RunReq {
    pub answers: Vec<Answer>,
    pub delete_files: bool,
    pub delete_databases: bool,
    pub only: Vec<String>,
}

pub async fn uninstall_run(State(st): State<AppState>, bytes: axum::body::Bytes) -> Result<Response, ApiError> {
    let req: RunReq = body(&bytes)?;
    let st2 = st.clone();
    let only = req.only.clone();
    blocking(move || only.iter().try_for_each(|n| st2.entry(n).map(|_| ()))).await?;
    let r = UninstallRequest { answers: req.answers.into_iter().map(|a| (a.item, a.remove)).collect(), delete_files: req.delete_files, delete_databases: req.delete_databases, only: req.only };
    let admin = blocking(|| Ok(maint::current().is_admin())).await?;
    if !admin {
        // The administrator part never deletes files: the folders are the person's, and this
        // process deletes them afterwards (an elevated delete in a folder others can change is a
        // risk). Which ones is decided now, before the server list changes.
        // (Windows only: on Linux the folders belong to the servers' own accounts, so the root
        // helper deletes them as before.)
        let split = crate::elevated::service_ready() && crate::elevated::can_route("uninstall");
        let folders = if split {
            blocking({
                let r = UninstallRequest { answers: r.answers.clone(), delete_files: r.delete_files, delete_databases: r.delete_databases, only: r.only.clone() };
                move || Ok(maint::current().uninstall_folders(&r))
            })
            .await?
        } else {
            Vec::new()
        };
        let mut c = cmd(&["uninstall", "--yes"]);
        if r.delete_files && !split {
            c.push("--delete-files".into());
        }
        if r.delete_databases {
            c.push("--delete-databases".into());
        }
        if let [one] = r.only.as_slice() {
            c.push(one.clone());
        }
        let msg = "Removing services, firewall rules and accounts needs administrator rights. Run this command as administrator:";
        let what = "remove the services, firewall rules and accounts Kora set up";
        crate::elevated::precheck_verb(&Context { what: what.into(), command: c.clone(), can_skip_admin: false }, msg)?;
        let server = if let [one] = r.only.as_slice() { Some(one.clone()) } else { None };
        return st.spawn_job(Op::UninstallRun, server, move |h| {
            let out = crate::elevated::privileged_as(what, msg, c, vec![], &mut |m| h.event(json!({"type": "status", "message": m}))).map_err(job_error)?;
            let mut notes = vec!["Your server folders and databases were kept unless you chose to delete them.".to_string()];
            for (name, dir) in &folders {
                h.event(json!({"type": "status", "message": format!("Deleting the server folder of {name}")}));
                match maint::current().delete_server_files(dir) {
                    Ok(n) => notes.extend(n),
                    Err(e) => notes.push(format!("{name}: the server folder could not be deleted ({e}). Delete {} yourself if you no longer need it.", dir.display())),
                }
            }
            Ok(json!({"notes": notes, "output": out["output"]}))
        });
    }
    let server = if let [one] = r.only.as_slice() { Some(one.clone()) } else { None };
    st.spawn_job(Op::UninstallRun, server, move |h| {
        let out = maint::current().uninstall_run(&r, &mut |m| h.event(json!({"type": "status", "message": m})))?;
        if !out.failed.is_empty() {
            return Err(format!("Some things could not be removed: {}", out.failed.join("; ")).into());
        }
        let mut notes = vec!["Your server folders and databases were kept unless you chose to delete them.".to_string()];
        notes.extend(out.kept.iter().cloned());
        Ok(json!({"removed": out.removed, "kept": out.kept, "notes": notes}))
    })
}
