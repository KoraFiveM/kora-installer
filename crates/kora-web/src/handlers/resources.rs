//! Installed resources, ensure, catalog, adding resources.
//!
//! Each `pub async fn` is mounted by `crate::routes` for the [`Op`] of the same name; the HTTP
//! method and path come from `Op::route`, so only the handler's extractors live here.
#![allow(clippy::unused_async)]

use std::sync::{Arc, RwLock};

use axum::extract::{Path, State};
use axum::response::Response;
use axum::Json;
use kora_core::api::panel_updates::catalog_json;
use kora_core::api::{valid_resource_name, Op, Status};
use kora_core::catalog::Catalog;
use kora_core::console;
use kora_core::update::lock;
use serde_json::{json, Value};

use crate::{blocking, ApiError, AppState};

/// Loads the catalog and says when the bundled copy was used. Tests replace it so nothing
/// touches the network.
pub type CatalogLoader = Arc<dyn Fn() -> Result<(Catalog, Option<String>), String> + Send + Sync>;

static CATALOG_LOADER: RwLock<Option<CatalogLoader>> = RwLock::new(None);

/// Replace the catalog loader (`None` puts the real one back). For tests.
pub fn set_catalog_loader(l: Option<CatalogLoader>) {
    *CATALOG_LOADER.write().unwrap_or_else(|e| e.into_inner()) = l;
}

fn load_catalog() -> Result<(Catalog, Option<String>), String> {
    let custom = CATALOG_LOADER.read().unwrap_or_else(|e| e.into_inner()).clone();
    match custom {
        Some(f) => f(),
        None => kora_core::addresources::load_catalog().map_err(|e| e.to_string()),
    }
}

const MAX_IDS: usize = 300;

fn ids_of(body: &Value) -> Result<Vec<String>, ApiError> {
    let list: Vec<String> = body.get("ids").and_then(|v| serde_json::from_value(v.clone()).ok()).ok_or_else(|| ApiError::bad_request("Send the resources to add as {\"ids\": [names]}."))?;
    if list.is_empty() {
        return Err(ApiError::bad_request("Pick at least one resource."));
    }
    if list.len() > MAX_IDS {
        return Err(ApiError::bad_request("That is too many resources at once."));
    }
    match list.iter().find(|n| !valid_resource_name(n)) {
        Some(bad) => Err(ApiError::bad_request(format!("'{bad}' is not a resource name."))),
        None => Ok(list),
    }
}

pub async fn resources_list(State(st): State<AppState>, Path(s): Path<String>) -> Result<Json<Value>, ApiError> {
    blocking(move || {
        let e = st.entry(&s)?;
        let list = st.backend().resources(&e);
        Ok(Json(json!({"resources": list})))
    })
    .await
}

pub async fn resource_ensure(State(st): State<AppState>, Path(s): Path<String>, Json(body): Json<Value>) -> Result<Json<Value>, ApiError> {
    let name = body.get("name").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if !valid_resource_name(&name) {
        return Err(ApiError::bad_request("Give a resource name: letters, digits, _ . and - only."));
    }
    let restart = body.get("restart").and_then(Value::as_bool).unwrap_or(false);
    blocking(move || {
        let e = st.entry(&s)?;
        // A restart in the middle of an update would start the server on half-swapped files.
        if restart && lock::is_busy(&e.dir) {
            return Err(ApiError::conflict(format!("{}.", lock::BUSY)));
        }
        let message = st.backend().ensure(&e, &name, restart).map_err(ApiError::core)?;
        Ok(Json(json!({"message": message})))
    })
    .await
}

/// Start, stop or restart one installed resource on a running server. The command goes through the
/// console helper like a typed one, but the caller only picks the resource and the action: the text
/// is built here from a checked name, so this needs no more than server control does.
pub async fn resource_control(State(st): State<AppState>, Path((s, name)): Path<(String, String)>, Json(body): Json<Value>) -> Result<Json<Value>, ApiError> {
    // `ensure` restarts a running resource and starts a stopped one, so "restart" never fails
    // just because the resource was not running.
    let (verb, said) = match body.get("action").and_then(Value::as_str) {
        Some("start") => ("start", "start"),
        Some("stop") => ("stop", "stop"),
        Some("restart") => ("ensure", "restart"),
        _ => return Err(ApiError::bad_request("The action must be start, stop or restart.")),
    };
    if !valid_resource_name(&name) {
        return Err(ApiError::bad_request(format!("'{name}' is not a resource name.")));
    }
    blocking(move || {
        let e = st.entry(&s)?;
        // FXServer knows a resource by its folder name, which is not always the id the list shows
        // (`kora-lib` is the folder `kora_lib`): the command must use the folder name.
        let Some(folder) = st.backend().resource_folder(&e, &name) else {
            return Err(ApiError::not_found(format!("'{name}' is not installed on this server.")));
        };
        if !valid_resource_name(&folder) {
            return Err(ApiError::conflict(format!("The folder of '{name}' ('{folder}') has a name that cannot be sent as a command. Use the server's own console for this one.")));
        }
        if verb == "stop" && (name == console::RESOURCE || folder == console::RESOURCE) {
            return Err(ApiError::bad_request("Kora uses this resource to send commands to the server, so it cannot be stopped from here."));
        }
        // An update is swapping files: loading a resource now could load half of it.
        if verb != "stop" && lock::is_busy(&e.dir) {
            return Err(ApiError::conflict(format!("{}.", lock::BUSY)));
        }
        if st.backend().status(&e) != Status::Running {
            return Err(ApiError::conflict("The server is not running. Start the server first; its resources start with it."));
        }
        if !console::is_installed(&e.dir) {
            return Err(ApiError::conflict("The live console is not switched on for this server yet. Turn it on in the Console tab first (it needs a restart)."));
        }
        if console::waiting_for_restart(&e.dir) {
            return Err(ApiError::conflict(console::NEEDS_RESTART));
        }
        console::send(&e.dir, &format!("{verb} {folder}")).map_err(ApiError::internal)?;
        if verb != "stop" {
            // The resource is loaded again from the files on disk.
            kora_core::resourcestate::mark_resource_restarted(&e.dir, &name);
        }
        Ok(Json(json!({
            "message": format!("Asked the server to {said} {folder}. The Console tab shows what happened.")
        })))
    })
    .await
}

pub async fn catalog_for_server(State(st): State<AppState>, Path(s): Path<String>) -> Result<Json<Value>, ApiError> {
    blocking(move || {
        let e = st.entry(&s)?;
        let latest = e.versions() == kora_core::installer::Versions::Latest;
        let (cat, installed, note) = st.backend().add_catalog(&e).map_err(ApiError::core)?;
        let note = note.or_else(|| (e.source() == kora_core::installer::Source::Fork).then(|| "Resources are copied into your GitHub account, so you can change them.".to_string()));
        Ok(Json(json!({"catalog": catalog_json(&cat, Some(latest)), "installed": installed, "note": note})))
    })
    .await
}

pub async fn resources_plan(State(st): State<AppState>, Path(s): Path<String>, Json(body): Json<Value>) -> Result<Json<Value>, ApiError> {
    let ids = ids_of(&body)?;
    blocking(move || {
        let e = st.entry(&s)?;
        let plan = st.backend().add_plan(&e, &ids).map_err(ApiError::core)?;
        Ok(Json(serde_json::to_value(plan).map_err(|x| ApiError::internal(x.to_string()))?))
    })
    .await
}

pub async fn resources_add(State(st): State<AppState>, Path(s): Path<String>, Json(body): Json<Value>) -> Result<Response, ApiError> {
    let ids = ids_of(&body)?;
    let st2 = st.clone();
    let e = blocking(move || st2.entry(&s)).await?;
    let make = st.backend.clone();
    let name = e.name.clone();
    st.spawn_job(Op::ResourcesAdd, Some(name), move |h| {
        for id in &ids {
            h.event(json!({"type": "resource", "id": id, "state": "queued"}));
        }
        let out = make().add_install(&e, &ids)?;
        for id in &out.installed {
            h.event(json!({"type": "resource", "id": id, "state": "installed"}));
        }
        Ok(serde_json::to_value(&out)?)
    })
}

pub async fn catalog_get() -> Result<Json<Value>, ApiError> {
    let (cat, _note) = blocking(|| load_catalog().map_err(ApiError::core)).await?;
    Ok(Json(catalog_json(&cat, None)))
}
