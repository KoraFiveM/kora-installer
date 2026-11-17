//! The page of one resource (see `kora_core::branches::page` and `kora_core::resourcestate`).
//!
//! Read-only: what the resource is, where its files are, and whether the running server still has an old version of
//! it. The buttons on the page use the existing operations (check, update, control, branches).
#![allow(clippy::unused_async)]

use axum::extract::{Path, State};
use axum::Json;
use kora_core::api::wire as w;
use kora_core::api::{valid_resource_name, Status};
use kora_core::branches::{Reply, Request};
use kora_core::console;
use kora_core::resourcestate;
use serde_json::Value;

use crate::{blocking, ApiError, AppState};

/// `GET /servers/{s}/resources/{name}` → the resource, its git state, the server's state and the restart notice.
pub async fn resource_page(State(st): State<AppState>, Path((s, name)): Path<(String, String)>) -> Result<Json<Value>, ApiError> {
    if !valid_resource_name(&name) {
        return Err(ApiError::bad_request("That is not a resource name."));
    }
    blocking(move || {
        let e = st.entry(&s)?;
        let installed = st.backend().resources(&e);
        let wanted = kora_core::catalog::normalize_id(&name);
        let Some(id) = installed.iter().find(|r| kora_core::catalog::normalize_id(r) == wanted).cloned() else {
            return Err(ApiError::not_found(format!("'{name}' is not installed on this server.")));
        };
        let reply = st.backend().branches(&e, &Request::Page { resource: id.clone() }, &mut |_, _, _| {}).map_err(ApiError::core)?;
        let Reply::Page(page) = reply else {
            return Err(ApiError::internal("The resource page got an answer it did not ask for."));
        };
        let running = st.backend().status(&e) == Status::Running;
        let restart = resourcestate::info(&e.dir, &id, running);
        let out = w::ResourcePage {
            id: page.id.clone(),
            folder: page.folder.clone(),
            repository: page.repository.clone(),
            default_branch: page.default_branch.clone(),
            pinned_commit: page.pinned_commit.clone(),
            third_party: page.third_party,
            source: page.source.clone(),
            version: page.version.clone(),
            branch: page.branch.clone(),
            repos: page.repos.clone(),
            restart,
            server: w::ResourceServerState {
                name: e.name.clone(),
                mode: e.mode,
                source: e.source(),
                versions: e.versions(),
                running,
                console_enabled: console::is_installed(&e.dir),
                console_waiting_for_restart: console::waiting_for_restart(&e.dir),
            },
        };
        let out = serde_json::to_value(out).map_err(|x| ApiError::internal(x.to_string()))?;
        Ok(Json(out))
    })
    .await
}
