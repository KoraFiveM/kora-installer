//! Update preview/apply/auto, snapshots and rollback.
//!
//! Each `pub async fn` is mounted by `crate::routes` for the [`Op`] of the same name; the HTTP
//! method and path come from `Op::route`, so only the handler's extractors live here.
#![allow(clippy::unused_async)]

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use axum::extract::{Path, State};
use axum::response::Response;
use axum::Json;
use kora_core::api::panel_updates::{badge_json, report_json, update_event_json};
use kora_core::api::{badge_of, valid_resource_name, Op};
use kora_core::installer::registry::ServerEntry;
use kora_core::update::lock;
use kora_core::update::manual::{ApplyOpts, RestartPolicy, Scope, SnapshotInfo};
use kora_core::update::UpdateReport;
use serde_json::{json, Value};

use crate::{blocking, ApiError, AppState};

/// What `kora update --auto <server>` does. Tests replace it so the real updater never runs.
pub type AutoRunner = Arc<dyn Fn(&str) -> Result<UpdateReport, String> + Send + Sync>;

static AUTO_RUNNER: RwLock<Option<AutoRunner>> = RwLock::new(None);

/// Replace the automatic updater (`None` puts the real one back). For tests.
pub fn set_auto_runner(r: Option<AutoRunner>) {
    *AUTO_RUNNER.write().unwrap_or_else(|e| e.into_inner()) = r;
}

fn run_auto(server: &str) -> Result<UpdateReport, String> {
    let custom = AUTO_RUNNER.read().unwrap_or_else(|e| e.into_inner()).clone();
    match custom {
        Some(f) => f(server),
        None => kora_core::installer::gitops::auto_update(server).map_err(|e| e.to_string()),
    }
}

fn parse_scope(v: Option<&Value>) -> Result<Scope, ApiError> {
    let Some(v) = v.filter(|v| !v.is_null()) else {
        return Ok(Scope::All);
    };
    let scope: Scope = serde_json::from_value(v.clone()).map_err(|_| ApiError::bad_request("The scope must be \"all\" or {\"resources\": [names]}."))?;
    if let Scope::Resources(list) = &scope {
        if list.is_empty() {
            return Err(ApiError::bad_request("Pick at least one resource to update."));
        }
        if let Some(bad) = list.iter().find(|n| !valid_resource_name(n)) {
            return Err(ApiError::bad_request(format!("'{bad}' is not a resource name.")));
        }
    }
    Ok(scope)
}

fn names(v: Option<&Value>, what: &str) -> Result<Vec<String>, ApiError> {
    let Some(v) = v.filter(|v| !v.is_null()) else {
        return Ok(vec![]);
    };
    let list: Vec<String> = serde_json::from_value(v.clone()).map_err(|_| ApiError::bad_request(format!("'{what}' must be a list of names.")))?;
    match list.iter().find(|n| !valid_resource_name(n)) {
        Some(bad) => Err(ApiError::bad_request(format!("'{bad}' is not a resource name."))),
        None => Ok(list),
    }
}

fn body_of(b: Option<Json<Value>>) -> Value {
    b.map(|Json(v)| v).unwrap_or(Value::Null)
}

/// `{"<resource>": "<commit>"}`: the preview's planned targets (optional).
fn targets(v: Option<&Value>) -> Result<Option<BTreeMap<String, String>>, ApiError> {
    let Some(v) = v.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let bad = || ApiError::bad_request("'targets' must map resource names to commit ids.");
    let map: BTreeMap<String, String> = serde_json::from_value(v.clone()).map_err(|_| bad())?;
    let commit = |c: &str| (7..=64).contains(&c.len()) && c.bytes().all(|b| b.is_ascii_hexdigit());
    if map.iter().any(|(k, c)| !valid_resource_name(k) || !commit(c)) {
        return Err(bad());
    }
    Ok(Some(map))
}

/// 409 while another update (another tab, the CLI, the nightly run) holds this server's update lock.
async fn not_busy(e: &ServerEntry) -> Result<(), ApiError> {
    let dir = e.dir.clone();
    if blocking(move || Ok(lock::is_busy(&dir))).await? {
        return Err(ApiError::conflict(format!("{}.", lock::BUSY)));
    }
    Ok(())
}

async fn known(st: &AppState, name: String) -> Result<ServerEntry, ApiError> {
    let st = st.clone();
    blocking(move || st.entry(&name)).await
}

pub async fn update_preview(State(st): State<AppState>, Path(s): Path<String>, body: Option<Json<Value>>) -> Result<Response, ApiError> {
    let body = body_of(body);
    let scope = parse_scope(body.get("scope"))?;
    let e = known(&st, s.clone()).await?;
    not_busy(&e).await?;
    let make = st.backend.clone();
    st.spawn_job(Op::UpdatePreview, Some(s), move |h| {
        // Each resource checked is an event on the job (the response shape is unchanged).
        let p = make().preview_events(&e, scope, &mut |ev| h.event(update_event_json(&ev)))?;
        let mut v = serde_json::to_value(&p).map_err(|x| x.to_string())?;
        v["badge"] = badge_json(badge_of(&p));
        Ok(v)
    })
}

pub async fn update_apply(State(st): State<AppState>, Path(s): Path<String>, body: Option<Json<Value>>) -> Result<Response, ApiError> {
    let body = body_of(body);
    let scope = parse_scope(body.get("scope"))?;
    let restart = match body.get("restart").filter(|v| !v.is_null()) {
        // Same default as the CLI and the panel: never kick players without being asked.
        None => RestartPolicy::WhenQuiet,
        Some(v) => serde_json::from_value(v.clone()).map_err(|_| ApiError::bad_request("The restart choice must be none, ensure, full_restart or when_quiet."))?,
    };
    let skip = names(body.get("skip"), "skip")?;
    let targets = targets(body.get("targets"))?;
    let e = known(&st, s.clone()).await?;
    not_busy(&e).await?;
    let make = st.backend.clone();
    st.spawn_job(Op::UpdateApply, Some(s), move |h| {
        let report = make().apply(&e, scope, ApplyOpts { restart, skip, targets }, &mut |ev| h.event(update_event_json(&ev)))?;
        Ok(report_json(&report))
    })
}

pub async fn update_auto(State(st): State<AppState>, Path(s): Path<String>) -> Result<Response, ApiError> {
    let e = known(&st, s.clone()).await?;
    not_busy(&e).await?;
    st.spawn_job(Op::UpdateAuto, Some(s), move |_h| run_auto(&e.name).map(|r| report_json(&r)).map_err(Into::into))
}

fn snapshot_json(s: &SnapshotInfo) -> Value {
    json!({"id": s.id, "created_unix": s.created_unix, "resources": s.resources, "on_other_branch": s.on_other_branch})
}

pub async fn snapshots_list(State(st): State<AppState>, Path(s): Path<String>) -> Result<Json<Value>, ApiError> {
    blocking(move || {
        let e = st.entry(&s)?;
        let list = st.backend().snapshots(&e);
        Ok(Json(json!({"snapshots": list.iter().map(snapshot_json).collect::<Vec<_>>()})))
    })
    .await
}

pub async fn snapshot_rollback(State(st): State<AppState>, Path((s, id)): Path<(String, String)>, body: Option<Json<Value>>) -> Result<Response, ApiError> {
    let force = match body_of(body).get("force") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(ApiError::bad_request("'force' must be true or false.")),
    };
    let (e, listed) = {
        let st = st.clone();
        let id = id.clone();
        blocking(move || {
            let e = st.entry(&s)?;
            let found = st.backend().snapshots(&e).iter().any(|x| x.id == id);
            Ok((e, found))
        })
        .await?
    };
    if !listed {
        return Err(ApiError::not_found("That snapshot no longer exists."));
    }
    not_busy(&e).await?;
    let make = st.backend.clone();
    let name = e.name.clone();
    st.spawn_job(Op::SnapshotRollback, Some(name), move |h| {
        h.event(json!({"type": "snapshot", "id": id}));
        make().rollback(&e, &id, force)?;
        Ok(json!({}))
    })
}
