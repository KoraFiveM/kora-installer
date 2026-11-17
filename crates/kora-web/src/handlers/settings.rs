//! Per-server settings: Discord webhook, update schedule, ports.
//!
//! Each `pub async fn` is mounted by `crate::routes` for the [`Op`] of the same name; the HTTP
//! method and path come from `Op::route`, so only the handler's extractors live here.
#![allow(clippy::unused_async)]

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use kora_core::api::panel_updates::schedule_state_json;
use kora_core::api::{Status, WebhookView};
use kora_core::installer::Ports;
use kora_core::notify::{self, NotifyConfig};
use kora_core::service::ScheduleConfig;
use serde_json::{json, Value};

use crate::{blocking, ApiError, AppState};

fn webhook_state(c: &NotifyConfig) -> Value {
    let v = WebhookView::from(c);
    // `configured` is what the UI reads; `set` is the core's name for the same thing.
    serde_json::to_value(kora_core::api::wire::WebhookState { configured: v.set, set: v.set, redacted: v.redacted }).unwrap_or(Value::Null)
}

pub async fn webhook_get(State(st): State<AppState>, Path(s): Path<String>) -> Result<Json<Value>, ApiError> {
    blocking(move || {
        let e = st.entry(&s)?;
        let c = st.backend().notify_load(&e);
        Ok(Json(webhook_state(&c)))
    })
    .await
}

pub async fn webhook_set(State(st): State<AppState>, Path(s): Path<String>, Json(body): Json<Value>) -> Result<Json<Value>, ApiError> {
    let url = body.get("url").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if url.is_empty() {
        return Err(ApiError::bad_request("Paste the Discord webhook address."));
    }
    notify::validate_webhook(&url).map_err(|e| ApiError::bad_request(e.to_string()))?;
    blocking(move || {
        let e = st.entry(&s)?;
        let cfg = NotifyConfig { discord_webhook: Some(url) };
        st.backend().notify_save(&e, &cfg).map_err(ApiError::core)?;
        Ok(Json(webhook_state(&cfg)))
    })
    .await
}

pub async fn webhook_remove(State(st): State<AppState>, Path(s): Path<String>) -> Result<Response, ApiError> {
    blocking(move || {
        let e = st.entry(&s)?;
        st.backend().notify_save(&e, &NotifyConfig::default()).map_err(ApiError::core)?;
        Ok(StatusCode::NO_CONTENT.into_response())
    })
    .await
}

pub async fn webhook_test(State(st): State<AppState>, Path(s): Path<String>) -> Result<Response, ApiError> {
    blocking(move || {
        let e = st.entry(&s)?;
        let mut b = st.backend();
        let cfg = b.notify_load(&e);
        if !cfg.discord_webhook.as_deref().is_some_and(|u| !u.trim().is_empty()) {
            return Err(ApiError::bad_request("Add a webhook first."));
        }
        b.notify_test(&e, &cfg).map_err(ApiError::core)?;
        Ok(StatusCode::NO_CONTENT.into_response())
    })
    .await
}

pub async fn schedule_get(State(st): State<AppState>, Path(s): Path<String>) -> Result<Json<Value>, ApiError> {
    blocking(move || {
        let e = st.entry(&s)?;
        let cfg = st.backend().schedule_get(&e);
        Ok(Json(schedule_state_json(cfg.as_ref())))
    })
    .await
}

/// The schedule change needs administrator rights: on Windows the elevated worker does it (one
/// permission prompt), on Linux the helper when it is installed; otherwise this is the
/// `needs_elevation` answer with the command to run.
fn run_elevated(ask: crate::elevated::AdminAsk, args: Vec<String>) -> Result<(), ApiError> {
    let mut command = vec!["kora".to_string()];
    command.extend(args);
    crate::elevated::privileged_as(
        ask,
        "change the automatic updates",
        "Changing the automatic updates needs administrator rights. Run the command shown from a terminal opened as administrator, then come back here.",
        command,
        vec![],
        &mut |_| {},
    )
    .map(|_| ())
}

pub async fn schedule_set(
    State(st): State<AppState>,
    Extension(caller): Extension<crate::remote::Caller>,
    headers: HeaderMap,
    Path(s): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let ask = crate::elevated::AdminAsk::of(&caller, &headers);
    let cfg: ScheduleConfig = serde_json::from_value(body)
        .map_err(|_| ApiError::bad_request("The schedule needs a preset (daily, weekly or custom), a time like 04:00, a weekday for weekly, who it runs as, and whether it is on."))?;
    cfg.validate().map_err(|e| ApiError::bad_request(e.to_string()))?;
    blocking(move || {
        let e = st.entry(&s)?;
        let mut b = st.backend();
        if let Some(args) = b.schedule_elevation(&e, Some(&cfg)) {
            run_elevated(ask, args)?;
        } else {
            b.schedule_set(&e, &cfg).map_err(ApiError::core)?;
        }
        let now = b.schedule_get(&e).unwrap_or(cfg);
        Ok(Json(schedule_state_json(Some(&now))))
    })
    .await
}

pub async fn schedule_remove(State(st): State<AppState>, Extension(caller): Extension<crate::remote::Caller>, headers: HeaderMap, Path(s): Path<String>) -> Result<Response, ApiError> {
    let ask = crate::elevated::AdminAsk::of(&caller, &headers);
    blocking(move || {
        let e = st.entry(&s)?;
        let mut b = st.backend();
        if let Some(args) = b.schedule_elevation(&e, None) {
            run_elevated(ask, args)?;
        } else {
            b.schedule_remove(&e).map_err(ApiError::core)?;
        }
        Ok(StatusCode::NO_CONTENT.into_response())
    })
    .await
}

fn port_of(body: &Value, key: &str) -> Result<u16, ApiError> {
    let bad = || ApiError::bad_request("Ports must be whole numbers from 1024 to 65535.");
    let n = body.get(key).and_then(Value::as_u64).ok_or_else(bad)?;
    if (1024..=65535).contains(&n) {
        Ok(n as u16)
    } else {
        Err(bad())
    }
}

pub async fn ports_set(State(st): State<AppState>, Path(s): Path<String>, Json(body): Json<Value>) -> Result<Json<Value>, ApiError> {
    let game = port_of(&body, "game")?;
    let txadmin = port_of(&body, "txadmin")?;
    if game == txadmin {
        return Err(ApiError::bad_request("The game and txAdmin ports must be different."));
    }
    blocking(move || {
        let e = st.entry(&s)?;
        let mut b = st.backend();
        match b.status(&e) {
            Status::Stopped => {}
            Status::Running => return Err(ApiError::conflict("Stop the server before changing its ports.")),
            Status::Missing => return Err(ApiError::conflict("The server's folder is missing, so its ports cannot be changed.")),
        }
        if e.ports.game == game && e.ports.txadmin == txadmin {
            return Ok(Json(json!({"notes": ["Those are already the ports this server uses."]})));
        }
        let notes = b.ports_set(&e, Ports { game, txadmin }).map_err(ApiError::core)?;
        Ok(Json(json!({"notes": notes})))
    })
    .await
}
