//! The GitHub loop for one resource: changes, log, commit, push, pull requests, issues, releases; making your own copy
//! of a resource; and the "which versions" setting (see `kora_core::collab`).
//!
//! Each `pub async fn` is mounted by `crate::routes` for the `Op` of the same name. Everything that needs git or `gh` goes
//! through the `Backend`; the handlers only check what the caller sent (resource names, numbers, short texts, methods)
//! and never build a command line. The routes that act on GitHub or change the code on disk are admin-only
//! (`remote/roles.rs`), and every answer that acted says which GitHub account did it (`acting_as`).
#![allow(clippy::unused_async)]

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use kora_core::api::{valid_resource_name, Op};
use kora_core::collab::{Reply, Request};
use kora_core::installer::registry::ServerEntry;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{blocking, ApiError, AppState};

fn resource(name: &str) -> Result<(), ApiError> {
    if valid_resource_name(name) {
        Ok(())
    } else {
        Err(ApiError::bad_request("That is not a resource name."))
    }
}

fn number(n: &str) -> Result<u32, ApiError> {
    n.parse::<u32>().ok().filter(|n| (1..=9_999_999).contains(n)).ok_or_else(|| ApiError::bad_request("That is not a pull request or issue number."))
}

/// `fork` (your own copy) or nothing (the original).
fn side(s: Option<&str>) -> Result<Option<String>, ApiError> {
    match s {
        None | Some("") | Some("upstream") => Ok(None),
        Some("fork") => Ok(Some("fork".into())),
        Some(_) => Err(ApiError::bad_request("The side must be \"upstream\" (the original) or \"fork\" (your own copy).")),
    }
}

/// The answer, marked with the GitHub account that acted (when the answer names one) for the audit log.
fn acted(v: Value) -> Response {
    let login = v.get("acting_as").and_then(Value::as_str).filter(|l| !l.is_empty() && l.len() <= 64).map(str::to_string);
    let mut r = Json(v).into_response();
    if let Some(l) = login {
        r.extensions_mut().insert(crate::audit::GithubActor(l));
    }
    r
}

async fn ask(st: &AppState, e: ServerEntry, req: Request) -> Result<Reply, ApiError> {
    let st = st.clone();
    blocking(move || st.backend().collab(&e, &req, &mut |_, _, _| {}).map_err(ApiError::core)).await
}

async fn entry(st: &AppState, s: String) -> Result<ServerEntry, ApiError> {
    let st2 = st.clone();
    blocking(move || st2.entry(&s)).await
}

/// `GET .../changes` → the files that changed and a read-only diff.
pub async fn changes(State(st): State<AppState>, Path((s, name)): Path<(String, String)>) -> Result<Json<Value>, ApiError> {
    resource(&name)?;
    let e = entry(&st, s).await?;
    Ok(Json(ask(&st, e, Request::Changes { resource: name }).await?.json()))
}

/// `GET .../log` → the latest commits of the branch.
pub async fn log(State(st): State<AppState>, Path((s, name)): Path<(String, String)>) -> Result<Json<Value>, ApiError> {
    resource(&name)?;
    let e = entry(&st, s).await?;
    Ok(Json(ask(&st, e, Request::Log { resource: name }).await?.json()))
}

#[derive(Deserialize)]
pub struct CommitBody {
    message: String,
}

/// `POST .../commit` `{message}` → the new commit.
pub async fn commit(State(st): State<AppState>, Path((s, name)): Path<(String, String)>, Json(b): Json<CommitBody>) -> Result<Json<Value>, ApiError> {
    resource(&name)?;
    let e = entry(&st, s).await?;
    Ok(Json(ask(&st, e, Request::Commit { resource: name, message: b.message }).await?.json()))
}

/// `POST .../push` → where it went and as whom. Never forced.
pub async fn push(State(st): State<AppState>, Path((s, name)): Path<(String, String)>) -> Result<Response, ApiError> {
    resource(&name)?;
    let e = entry(&st, s).await?;
    Ok(acted(ask(&st, e, Request::Push { resource: name }).await?.json()))
}

#[derive(Deserialize)]
pub struct NewBranchBody {
    name: String,
}

/// `POST .../new-branch` `{name}` → where the resource is now.
pub async fn new_branch(State(st): State<AppState>, Path((s, name)): Path<(String, String)>, Json(b): Json<NewBranchBody>) -> Result<Json<Value>, ApiError> {
    resource(&name)?;
    if !kora_core::git::branches::valid_branch(&b.name) {
        return Err(ApiError::bad_request("That is not a branch name Kora can use."));
    }
    let e = entry(&st, s).await?;
    Ok(Json(ask(&st, e, Request::NewBranch { resource: name, name: b.name }).await?.json()))
}

/// `GET .../prs/{number}?source=fork` → the pull request in full.
pub async fn pr_view(State(st): State<AppState>, Path((s, name, n)): Path<(String, String, String)>, Query(q): Query<HashMap<String, String>>) -> Result<Json<Value>, ApiError> {
    resource(&name)?;
    let (number, source) = (number(&n)?, side(q.get("source").map(String::as_str))?);
    let e = entry(&st, s).await?;
    Ok(Json(ask(&st, e, Request::PrView { resource: name, number, source }).await?.json()))
}

#[derive(Deserialize)]
pub struct CommentBody {
    body: String,
    #[serde(default)]
    source: Option<String>,
}

async fn comment_on(st: AppState, s: String, name: String, n: String, kind: &str, b: CommentBody) -> Result<Response, ApiError> {
    resource(&name)?;
    let (number, source) = (number(&n)?, side(b.source.as_deref())?);
    let e = entry(&st, s).await?;
    Ok(acted(ask(&st, e, Request::Comment { resource: name, kind: kind.into(), number, body: b.body, source }).await?.json()))
}

/// `POST .../prs/{number}/comment` `{body, source?}`.
pub async fn pr_comment(State(st): State<AppState>, Path((s, name, n)): Path<(String, String, String)>, Json(b): Json<CommentBody>) -> Result<Response, ApiError> {
    comment_on(st, s, name, n, "pr", b).await
}

/// `POST .../issues/{number}/comment` `{body}`.
pub async fn issue_comment(State(st): State<AppState>, Path((s, name, n)): Path<(String, String, String)>, Json(b): Json<CommentBody>) -> Result<Response, ApiError> {
    comment_on(st, s, name, n, "issue", b).await
}

#[derive(Deserialize)]
pub struct ReviewBody {
    action: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    source: Option<String>,
}

/// `POST .../prs/{number}/review` `{action: approve|request_changes|comment, body?, source?}`.
pub async fn pr_review(State(st): State<AppState>, Path((s, name, n)): Path<(String, String, String)>, Json(b): Json<ReviewBody>) -> Result<Response, ApiError> {
    resource(&name)?;
    if !["approve", "request_changes", "comment"].contains(&b.action.as_str()) {
        return Err(ApiError::bad_request("A review is \"approve\", \"request_changes\" or \"comment\"."));
    }
    let (number, source) = (number(&n)?, side(b.source.as_deref())?);
    let e = entry(&st, s).await?;
    Ok(acted(ask(&st, e, Request::Review { resource: name, number, action: b.action, body: b.body.unwrap_or_default(), source }).await?.json()))
}

#[derive(Deserialize)]
pub struct MergeBody {
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    source: Option<String>,
}

/// `POST .../prs/{number}/merge` `{method?, source?}`.
pub async fn pr_merge(State(st): State<AppState>, Path((s, name, n)): Path<(String, String, String)>, Json(b): Json<MergeBody>) -> Result<Response, ApiError> {
    resource(&name)?;
    if b.method.as_deref().is_some_and(|m| !["merge", "squash", "rebase"].contains(&m)) {
        return Err(ApiError::bad_request("A merge is \"merge\", \"squash\" or \"rebase\"."));
    }
    let (number, source) = (number(&n)?, side(b.source.as_deref())?);
    let e = entry(&st, s).await?;
    Ok(acted(ask(&st, e, Request::Merge { resource: name, number, method: b.method, source }).await?.json()))
}

/// `GET .../issues` → open issues of the resource's repository.
pub async fn issues_list(State(st): State<AppState>, Path((s, name)): Path<(String, String)>) -> Result<Json<Value>, ApiError> {
    resource(&name)?;
    let e = entry(&st, s).await?;
    Ok(Json(ask(&st, e, Request::Issues { resource: name }).await?.json()))
}

/// `GET .../releases` → releases and which one the server is pinned to.
pub async fn releases(State(st): State<AppState>, Path((s, name)): Path<(String, String)>) -> Result<Json<Value>, ApiError> {
    resource(&name)?;
    let e = entry(&st, s).await?;
    Ok(Json(ask(&st, e, Request::Releases { resource: name }).await?.json()))
}

#[derive(Deserialize)]
pub struct ForkBody {
    #[serde(default)]
    owner: Option<String>,
    #[serde(default)]
    resources: Option<Vec<String>>,
}

/// `POST /servers/{s}/fork` `{owner?, resources?}` → a job. Result: what was copied, skipped and failed.
pub async fn fork(State(st): State<AppState>, Path(s): Path<String>, Json(b): Json<ForkBody>) -> Result<Response, ApiError> {
    if let Some(list) = &b.resources {
        if list.is_empty() || list.len() > 300 || list.iter().any(|n| !valid_resource_name(n)) {
            return Err(ApiError::bad_request("'resources' must be a list of resource names."));
        }
    }
    if let Some(o) = &b.owner {
        if !o.is_empty() {
            kora_core::git::owners::check_login_syntax(o).map_err(ApiError::bad_request)?;
        }
    }
    let e = entry(&st, s).await?;
    let make = st.backend.clone();
    st.spawn_job(Op::ResourcesFork, Some(e.name.clone()), move |h| {
        let req = Request::Fork { owner: b.owner.filter(|o| !o.is_empty()), only: b.resources };
        let r = make().collab(&e, &req, &mut |done, total, id| h.event(json!({"type": "forked", "done": done, "total": total, "id": id})))?;
        Ok(r.json())
    })
}

#[derive(Deserialize)]
pub struct VersionsBody {
    versions: kora_core::installer::Versions,
}

/// `PUT /servers/{s}/versions` `{versions: released|latest}` → the new setting. Nothing on disk moves.
pub async fn versions_set(State(st): State<AppState>, Path(s): Path<String>, Json(b): Json<VersionsBody>) -> Result<Json<Value>, ApiError> {
    let e = entry(&st, s).await?;
    Ok(Json(ask(&st, e, Request::SetVersions { versions: b.versions }).await?.json()))
}
