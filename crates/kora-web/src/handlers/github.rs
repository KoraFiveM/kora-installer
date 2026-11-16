//! GitHub sign-in, fork owners, issue reports and pull requests.
//!
//! Each `pub async fn` is mounted by `crate::routes` for the [`Op`] of the same name; the HTTP
//! method and path come from `Op::route`, so only the handler's extractors live here.
//!
//! Anything that runs `gh`, git or the network goes through the `Backend` (issues, pull requests,
//! "is `gh` signed in") or [`GithubEnv`] (who is signed in, owners, the sign-in itself). Tests
//! replace the latter with [`set_env`].
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use kora_core::api::install::github::{GithubEnv, SystemGithub};
use kora_core::api::{valid_resource_name, Op};
use kora_core::feedback::PrPlan;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{blocking, ApiError, AppState};

static ENV: RwLock<Option<Arc<dyn GithubEnv>>> = RwLock::new(None);

/// Replace the GitHub seam (tests). `None` goes back to the real `gh`.
pub fn set_env(e: Option<Arc<dyn GithubEnv>>) {
    *ENV.write().unwrap_or_else(|p| p.into_inner()) = e;
}

/// The GitHub seam in use.
pub fn env() -> Arc<dyn GithubEnv> {
    ENV.read()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
        .unwrap_or_else(|| Arc::new(SystemGithub))
}

const SIGN_IN_FIRST: &str = "Sign in to GitHub first.";
const MAX_TITLE: usize = 256;
const MAX_TEXT: usize = 60_000;

fn bad_gateway(m: String) -> ApiError {
    ApiError::new(StatusCode::BAD_GATEWAY, "internal", m)
}

fn parse<T: for<'de> Deserialize<'de>>(v: Value) -> Result<T, ApiError> {
    serde_json::from_value(v).map_err(|_| {
        ApiError::bad_request("That request was not understood. Reload the page and try again.")
    })
}

/// `GET /github/status` → `{signed_in, user}`.
pub async fn github_status() -> Result<Json<Value>, ApiError> {
    let e = env();
    let u = blocking(move || Ok(e.auth())).await?;
    Ok(Json(json!({"signed_in": u.signed_in, "user": u.user})))
}

/// Only one sign-in at a time (a second `gh auth login` would fight over the same device flow).
static LOGIN_ACTIVE: AtomicBool = AtomicBool::new(false);

struct LoginGuard;
impl Drop for LoginGuard {
    fn drop(&mut self) {
        LOGIN_ACTIVE.store(false, Ordering::SeqCst);
    }
}

/// `POST /github/login` → `202 {job, code, url}`. The job completes (result `{user}`) when the
/// sign-in is approved in the browser, or fails with a plain message.
pub async fn github_login(State(st): State<AppState>) -> Result<Response, ApiError> {
    if LOGIN_ACTIVE.swap(true, Ordering::SeqCst) {
        return Err(ApiError::conflict("A GitHub sign-in is already in progress. Finish it in your browser, or wait a minute and try again."));
    }
    let guard = LoginGuard;
    let e = env();
    let flow = blocking(move || e.begin_login().map_err(bad_gateway)).await?;
    let (code, url) = (flow.code.clone(), flow.url.clone());
    let wait = flow.wait;
    let id = st
        .registry
        .spawn(Op::GithubLogin, None, move |_h| {
            let _guard = guard;
            wait().map(|user| json!({"user": user}))
        })
        .map_err(|c| ApiError::conflict(crate::conflict_message(&c)))?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({"job": id.0, "code": code, "url": url})),
    )
        .into_response())
}

/// `GET /github/owners` → `{owners: [{login, kind: "user"|"org", can_fork, reason}]}`.
pub async fn github_owners() -> Result<Json<Value>, ApiError> {
    let e = env();
    let owners = blocking(move || {
        if !e.auth().signed_in {
            return Err(ApiError::forbidden(SIGN_IN_FIRST));
        }
        e.owners().map_err(ApiError::core)
    })
    .await?;
    let list: Vec<Value> = owners
        .iter()
        .map(|o| json!({"login": o.login, "kind": if o.is_org { "org" } else { "user" }, "can_fork": o.can_fork, "reason": o.reason}))
        .collect();
    Ok(Json(json!({"owners": list})))
}

#[derive(Deserialize)]
struct IssueBody {
    #[serde(default)]
    resource: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    text: String,
}

fn check_resource(r: &str) -> Result<(), ApiError> {
    if valid_resource_name(r) {
        Ok(())
    } else {
        Err(ApiError::bad_request(
            "Choose which resource this is about.",
        ))
    }
}

/// `POST /servers/{s}/issues` `{resource, title, text}` → `{url}`.
pub async fn issue_report(
    State(st): State<AppState>,
    Path(s): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let b: IssueBody = parse(body)?;
    let url = blocking(move || {
        let e = st.entry(&s)?;
        check_resource(&b.resource)?;
        let title = b.title.trim();
        if title.is_empty() {
            return Err(ApiError::bad_request("Give the issue a short title."));
        }
        if title.chars().count() > MAX_TITLE || b.text.len() > MAX_TEXT {
            return Err(ApiError::bad_request("The title or the text is too long."));
        }
        let mut be = st.backend();
        if !be.github_signed_in() {
            return Err(ApiError::forbidden(SIGN_IN_FIRST));
        }
        be.report_issue(&e, &b.resource, title, &b.text)
            .map_err(ApiError::core)
    })
    .await?;
    Ok(Json(json!({"url": url})))
}

#[derive(Deserialize)]
struct PlanBody {
    #[serde(default)]
    resource: String,
}

/// `POST /servers/{s}/pr/plan` `{resource}` → the plan (resource, upstream, base, head_owner, head_branch, ahead, dir).
pub async fn pr_plan(
    State(st): State<AppState>,
    Path(s): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<PrPlan>, ApiError> {
    let b: PlanBody = parse(body)?;
    let plan = blocking(move || {
        let e = st.entry(&s)?;
        check_resource(&b.resource)?;
        let mut be = st.backend();
        if !be.github_signed_in() {
            return Err(ApiError::forbidden(SIGN_IN_FIRST));
        }
        be.pr_plan(&e, &b.resource).map_err(ApiError::core)
    })
    .await?;
    Ok(Json(plan))
}

#[derive(Deserialize)]
struct OpenBody {
    plan: Value,
    #[serde(default)]
    title: String,
    #[serde(default)]
    text: String,
}

/// `POST /servers/{s}/pr` `{plan, title, text}` → `{url}`. The plan is worked out again here from
/// the resource and must be exactly what the client sent: a plan the client changed (another
/// folder, branch or repository) is refused, so the browser cannot make Kora push somewhere else.
pub async fn pr_open(
    State(st): State<AppState>,
    Path(s): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let b: OpenBody = parse(body)?;
    let client: PrPlan = parse(b.plan)?;
    let url = blocking(move || {
        let e = st.entry(&s)?;
        check_resource(&client.resource)?;
        let title = b.title.trim();
        if title.is_empty() {
            return Err(ApiError::bad_request("Give the pull request a title."));
        }
        if title.chars().count() > MAX_TITLE || b.text.len() > MAX_TEXT {
            return Err(ApiError::bad_request("The title or the text is too long."));
        }
        let mut be = st.backend();
        if !be.github_signed_in() {
            return Err(ApiError::forbidden(SIGN_IN_FIRST));
        }
        let fresh = be.pr_plan(&e, &client.resource).map_err(ApiError::core)?;
        if fresh != client {
            return Err(ApiError::bad_request("The details of this pull request have changed since you looked at them. Check them again before sending."));
        }
        be.pr_open(&fresh, title, &b.text).map_err(ApiError::core)
    })
    .await?;
    Ok(Json(json!({"url": url})))
}
