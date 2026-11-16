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

fn cmd(parts: &[&str]) -> Vec<String> {
    let mut v = vec!["kora".to_string()];
    v.extend(parts.iter().map(|s| (*s).to_string()));
    v
}

/// An optional JSON body: a missing or empty body means "all defaults".
fn body<T: serde::de::DeserializeOwned + Default>(
    bytes: &axum::body::Bytes,
) -> Result<T, ApiError> {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(T::default());
    }
    serde_json::from_slice(bytes)
        .map_err(|_| ApiError::bad_request("The request was not understood."))
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

pub async fn self_update_apply(
    State(st): State<AppState>,
    bytes: axum::body::Bytes,
) -> Result<Response, ApiError> {
    let req: SelfUpdateReq = body(&bytes)?;
    let version = req
        .version
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    if let Some(v) = &version {
        if v.len() > 40
            || !v
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
        {
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
        if !crate::elevated::helper_installed() {
            return Err(ApiError::needs_elevation(msg, c));
        }
        // The helper does it: the same command, run as administrator, as a job.
        return st.spawn_job(Op::SelfUpdateApply, None, move |h| {
            let out = crate::elevated::privileged_msg(msg, c, vec![], &mut |m| h.event(json!({"type": "status", "message": m})))
                .map_err(|e| e.message)?;
            Ok(json!({"version": v2, "output": out["output"]}))
        });
    }
    st.spawn_job(Op::SelfUpdateApply, None, move |h| {
        let new = maint::current().self_update_apply(version.as_deref(), &mut |m| {
            h.event(json!({"type": "status", "message": m}))
        })?;
        Ok(json!({"version": new}))
    })
}

pub async fn app_status() -> Result<Json<Value>, ApiError> {
    let s = blocking(|| Ok(maint::current().app_status())).await?;
    Ok(Json(json!({
        "installed": s.installed,
        "version": s.version,
        "path": s.path,
        "desktop_shortcut": s.desktop_shortcut,
        "supported": s.supported,
    })))
}

#[derive(Deserialize)]
#[serde(default)]
pub struct AppInstallReq {
    pub desktop_shortcut: bool,
    pub force: bool,
    pub dry_run: bool,
}
impl Default for AppInstallReq {
    fn default() -> Self {
        Self {
            desktop_shortcut: true,
            force: false,
            dry_run: false,
        }
    }
}

pub async fn app_install(bytes: axum::body::Bytes) -> Result<Json<Value>, ApiError> {
    let req: AppInstallReq = body(&bytes)?;
    blocking(move || {
        let m = maint::current();
        if maint::app_change_needs_admin(m.as_ref(), req.dry_run) {
            let mut c = cmd(&["app", "install", "--yes"]);
            if !req.desktop_shortcut {
                c.push("--no-desktop-shortcut".into());
            }
            if req.force {
                c.push("--force".into());
            }
            crate::elevated::privileged_msg("Installing the Kora app needs administrator rights. Run this command as administrator:", c, vec![], &mut |_| {})?;
            return Ok(Json(json!({"message": "The Kora app is installed."})));
        }
        let lines = m.app_install(&AppOptions { desktop_shortcut: req.desktop_shortcut, force: req.force, dry_run: req.dry_run }).map_err(ApiError::core)?;
        let head = if req.dry_run { "Nothing was changed. The Kora app install would" } else { "The Kora app is installed" };
        Ok(Json(json!({"message": format!("{head}: {}.", lines.join("; ")).replace(": .", ".") })))
    })
    .await
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct AppUninstallReq {
    pub dry_run: bool,
}

pub async fn app_uninstall(
    Query(q): Query<AppUninstallReq>,
    bytes: axum::body::Bytes,
) -> Result<Json<Value>, ApiError> {
    // The UI sends `{dry_run}` as a body; a query string works too.
    let b: AppUninstallReq = body(&bytes)?;
    let dry_run = b.dry_run || q.dry_run;
    blocking(move || {
        let m = maint::current();
        if maint::app_change_needs_admin(m.as_ref(), dry_run) {
            crate::elevated::privileged_msg("Removing the Kora app needs administrator rights. Run this command as administrator:", cmd(&["uninstall", "--app", "--yes"]), vec![], &mut |_| {})?;
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

pub async fn uninstall_plan(
    State(st): State<AppState>,
    bytes: axum::body::Bytes,
) -> Result<Json<Value>, ApiError> {
    let req: PlanReq = body(&bytes)?;
    let only = req.server.filter(|s| !s.is_empty());
    if let Some(n) = only.clone() {
        let st2 = st.clone();
        blocking(move || st2.entry(&n).map(|_| ())).await?;
    }
    let items = blocking(move || {
        maint::current()
            .uninstall_plan(only.as_deref())
            .map_err(ApiError::core)
    })
    .await?;
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

pub async fn uninstall_run(
    State(st): State<AppState>,
    bytes: axum::body::Bytes,
) -> Result<Response, ApiError> {
    let req: RunReq = body(&bytes)?;
    let st2 = st.clone();
    let only = req.only.clone();
    blocking(move || only.iter().try_for_each(|n| st2.entry(n).map(|_| ()))).await?;
    let r = UninstallRequest {
        answers: req
            .answers
            .into_iter()
            .map(|a| (a.item, a.remove))
            .collect(),
        delete_files: req.delete_files,
        delete_databases: req.delete_databases,
        only: req.only,
    };
    let admin = blocking(|| Ok(maint::current().is_admin())).await?;
    if !admin {
        let mut c = cmd(&["uninstall", "--yes"]);
        if r.delete_files {
            c.push("--delete-files".into());
        }
        if r.delete_databases {
            c.push("--delete-databases".into());
        }
        if let [one] = r.only.as_slice() {
            c.push(one.clone());
        }
        let msg = "Removing services, firewall rules and accounts needs administrator rights. Run this command as administrator:";
        if !crate::elevated::helper_installed() {
            return Err(ApiError::needs_elevation(msg, c));
        }
        let server = if let [one] = r.only.as_slice() { Some(one.clone()) } else { None };
        return st.spawn_job(Op::UninstallRun, server, move |h| {
            let out = crate::elevated::privileged_msg(msg, c, vec![], &mut |m| h.event(json!({"type": "status", "message": m})))
                .map_err(|e| e.message)?;
            Ok(json!({"notes": ["Your server folders and databases were kept unless you chose to delete them."], "output": out["output"]}))
        });
    }
    let server = if let [one] = r.only.as_slice() {
        Some(one.clone())
    } else {
        None
    };
    st.spawn_job(Op::UninstallRun, server, move |h| {
        let out = maint::current().uninstall_run(&r, &mut |m| {
            h.event(json!({"type": "status", "message": m}))
        })?;
        if !out.failed.is_empty() {
            return Err(format!(
                "Some things could not be removed: {}",
                out.failed.join("; ")
            ));
        }
        let mut notes = vec![
            "Your server folders and databases were kept unless you chose to delete them."
                .to_string(),
        ];
        notes.extend(out.kept.iter().cloned());
        Ok(json!({"removed": out.removed, "kept": out.kept, "notes": notes}))
    })
}
