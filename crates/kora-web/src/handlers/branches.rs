//! Branches and pull requests of a Development server's resources (see `kora_core::branches`).
//!
//! Each `pub async fn` is mounted by `crate::routes` for the `Op` of the same name. Everything that
//! needs git or `gh` goes through the `Backend`; the handlers only check what the caller sent
//! (branch names, pull request numbers, resource names) and that the server is a Development one.
#![allow(clippy::unused_async)]

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::Json;
use kora_core::api::{valid_resource_name, Op};
use kora_core::branches::{self as br, Changes, PrSource, Reply, Request, Target};
use kora_core::git::branches::codes;
use kora_core::installer::registry::ServerEntry;
use kora_core::problem::Problem;
use serde_json::{json, Value};

use crate::{blocking, ApiError, AppState};

/// Resources one bulk request may name.
const MAX_ONLY: usize = 300;

/// A packed problem becomes its API error; "an update is running" is a 409 like everywhere else.
fn fail(m: String) -> ApiError {
    if Problem::is_encoded(&m) {
        let p = Problem::decode(&m);
        if p.code == codes::BUSY {
            return ApiError::problem(StatusCode::CONFLICT, p);
        }
        if p.code == codes::NOT_DEV {
            return ApiError::problem(StatusCode::CONFLICT, p);
        }
    }
    ApiError::core(m)
}

/// The server. Every server can switch a resource to a branch or a pull request now (a server on released versions is
/// told, in the panel, that it will run unreleased code until the resource is put back).
fn dev(st: &AppState, name: &str) -> Result<ServerEntry, ApiError> {
    st.entry(name)
}

fn resource(name: &str) -> Result<(), ApiError> {
    if valid_resource_name(name) {
        Ok(())
    } else {
        Err(ApiError::bad_request("That is not a resource name."))
    }
}

fn changes_of(v: &Value) -> Result<Changes, ApiError> {
    match v.get("changes").filter(|c| !c.is_null()) {
        None => Ok(Changes::Ask),
        Some(c) => serde_json::from_value(c.clone()).map_err(|_| ApiError::bad_request("What to do with changes must be \"ask\", \"stash\" or \"discard\".")),
    }
}

fn branch_name(v: &Value, key: &str) -> Result<String, ApiError> {
    let name = v.get(key).and_then(Value::as_str).unwrap_or("").trim().to_string();
    if !br::valid_branch(&name) {
        return Err(ApiError::bad_request("That is not a branch name Kora can use."));
    }
    Ok(name)
}

fn remote_of(v: &Value) -> Result<Option<String>, ApiError> {
    match v.get("remote").and_then(Value::as_str) {
        None | Some("") => Ok(None),
        Some(r @ ("origin" | "upstream")) => Ok(Some(r.to_string())),
        Some(_) => Err(ApiError::bad_request("A branch can be taken from \"origin\" (your fork) or \"upstream\" (the original).")),
    }
}

/// `{action: "branch", branch, remote?}`, `{action: "pr", pr, source?}` or `{action: "default"}`.
fn target_of(v: &Value) -> Result<Target, ApiError> {
    match v.get("action").and_then(Value::as_str).unwrap_or("") {
        "default" => Ok(Target::Default),
        "branch" => Ok(Target::Branch { name: branch_name(v, "branch")?, remote: remote_of(v)? }),
        "pr" => {
            let input = match v.get("pr") {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Number(n)) => n.to_string(),
                _ => String::new(),
            };
            // Checked here so nothing odd gets as far as a command line.
            br::parse_pr_input(&input).map_err(|p| ApiError::problem(StatusCode::BAD_REQUEST, p))?;
            let source = match v.get("source").and_then(Value::as_str) {
                None | Some("") => None,
                Some("upstream") => Some(PrSource::Upstream),
                Some("fork") => Some(PrSource::Fork),
                Some(_) => return Err(ApiError::bad_request("The pull request's place must be \"upstream\" or \"fork\".")),
            };
            Ok(Target::Pr { input, source })
        }
        _ => Err(ApiError::bad_request("Say what to do: \"branch\", \"pr\" or \"default\".")),
    }
}

async fn ask(st: &AppState, e: ServerEntry, req: Request) -> Result<Reply, ApiError> {
    let st = st.clone();
    blocking(move || st.backend().branches(&e, &req, &mut |_, _, _| {}).map_err(fail)).await
}

/// `GET /servers/{s}/branches` → `{resources: [..]}`: where each resource is (nothing is downloaded).
pub async fn branches_list(State(st): State<AppState>, Path(s): Path<String>) -> Result<Json<Value>, ApiError> {
    let st2 = st.clone();
    let e = blocking(move || dev(&st2, &s)).await?;
    Ok(Json(ask(&st, e, Request::Overview).await?.json()))
}

/// `GET /servers/{s}/resources/{name}/branches?refresh=1` → branches, kept-aside changes, current state.
pub async fn branch_choices(State(st): State<AppState>, Path((s, name)): Path<(String, String)>, Query(q): Query<HashMap<String, String>>) -> Result<Json<Value>, ApiError> {
    resource(&name)?;
    let refresh = q.get("refresh").is_some_and(|v| v == "1" || v == "true");
    let st2 = st.clone();
    let e = blocking(move || dev(&st2, &s)).await?;
    Ok(Json(ask(&st, e, Request::Choices { resource: name, refresh }).await?.json()))
}

/// `GET /servers/{s}/resources/{name}/prs` → `{prs: [..], notes: [..]}`: open pull requests of the original and the fork.
pub async fn pr_list(State(st): State<AppState>, Path((s, name)): Path<(String, String)>) -> Result<Json<Value>, ApiError> {
    resource(&name)?;
    let st2 = st.clone();
    let e = blocking(move || dev(&st2, &s)).await?;
    Ok(Json(ask(&st, e, Request::Prs { resource: name }).await?.json()))
}

/// `POST /servers/{s}/resources/{name}/branch` `{action, branch?, remote?, pr?, source?, changes?}` → the outcome.
/// `status` is `switched`, `already` or `needs_choice` (uncommitted changes and no choice given: nothing was touched).
pub async fn branch_switch(State(st): State<AppState>, Path((s, name)): Path<(String, String)>, Json(body): Json<Value>) -> Result<Json<Value>, ApiError> {
    resource(&name)?;
    let target = target_of(&body)?;
    let changes = changes_of(&body)?;
    let st2 = st.clone();
    let e = blocking(move || dev(&st2, &s)).await?;
    Ok(Json(ask(&st, e, Request::Switch { resource: name, target, changes }).await?.json()))
}

/// `POST /servers/{s}/resources/{name}/stash/restore` `{index}` → the outcome. The kept-aside changes stay in the list.
pub async fn stash_restore(State(st): State<AppState>, Path((s, name)): Path<(String, String)>, Json(body): Json<Value>) -> Result<Json<Value>, ApiError> {
    resource(&name)?;
    let index =
        body.get("index").and_then(Value::as_u64).and_then(|i| u32::try_from(i).ok()).filter(|i| *i < 1000).ok_or_else(|| ApiError::bad_request("Say which kept-aside changes to bring back."))?;
    let st2 = st.clone();
    let e = blocking(move || dev(&st2, &s)).await?;
    Ok(Json(ask(&st, e, Request::RestoreStash { resource: name, index }).await?.json()))
}

/// `POST /servers/{s}/branches/preview` `{branch, fetch?}` → a job. Result: which resources have the branch.
pub async fn bulk_preview(State(st): State<AppState>, Path(s): Path<String>, Json(body): Json<Value>) -> Result<Response, ApiError> {
    let branch = branch_name(&body, "branch")?;
    let fetch = body.get("fetch").and_then(Value::as_bool).unwrap_or(true);
    let st2 = st.clone();
    let e = blocking(move || dev(&st2, &s)).await?;
    let make = st.backend.clone();
    st.spawn_job(Op::BranchBulkPreview, Some(e.name.clone()), move |h| {
        let r = make().branches(&e, &Request::BulkPreview { branch, fetch }, &mut |done, total, id| h.event(json!({"type": "checked", "done": done, "total": total, "id": id})))?;
        Ok(r.json())
    })
}

/// `POST /servers/{s}/branches/apply` `{action: "branch"|"default", branch?, resources?, changes?}` → a job. Result: one line per resource.
pub async fn bulk_apply(State(st): State<AppState>, Path(s): Path<String>, Json(body): Json<Value>) -> Result<Response, ApiError> {
    let target = match target_of(&body)? {
        Target::Pr { .. } => return Err(ApiError::bad_request("Pull requests are tried one resource at a time.")),
        t => t,
    };
    let changes = changes_of(&body)?;
    let only: Option<Vec<String>> = match body.get("resources").filter(|v| !v.is_null()) {
        None => None,
        Some(v) => {
            let list: Vec<String> = serde_json::from_value(v.clone()).map_err(|_| ApiError::bad_request("'resources' must be a list of names."))?;
            if list.is_empty() || list.len() > MAX_ONLY || list.iter().any(|n| !valid_resource_name(n)) {
                return Err(ApiError::bad_request("'resources' must be a list of resource names."));
            }
            Some(list)
        }
    };
    let st2 = st.clone();
    let e = blocking(move || dev(&st2, &s)).await?;
    let make = st.backend.clone();
    st.spawn_job(Op::BranchBulkApply, Some(e.name.clone()), move |h| {
        let r = make().branches(&e, &Request::BulkApply { target, only, changes }, &mut |done, total, id| h.event(json!({"type": "switched", "done": done, "total": total, "id": id})))?;
        Ok(r.json())
    })
}
