//! Monitoring: history for the charts, alerts and their per-server settings.
//!
//! The sampler is a [`Monitor`] started by `kora_web::run` (see [`start_system_monitor`]) and kept in a
//! module-level slot, so the router needs no new field. When no monitor is installed (the router used
//! on its own, as in most tests) the answers are simply empty.
#![allow(clippy::unused_async)]

use std::net::ToSocketAddrs;
use std::sync::Mutex;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use kora_core::monitor::{self, alerts, Monitor, Range};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::remote::Caller;
use crate::{blocking, ApiError, AppState, BackendFactory};

static CURRENT: Mutex<Option<Monitor>> = Mutex::new(None);

/// Make `m` the monitor the handlers read (`None` removes it). Tests install one with fake probes.
pub fn install(m: Option<Monitor>) {
    *CURRENT.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = m;
}

fn current() -> Option<Monitor> {
    CURRENT.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

/// Start the real sampler (every 15 s) and install it. Returns the handle so the caller can stop it.
pub fn start_system_monitor(make: BackendFactory) -> Monitor {
    let m = Monitor::system(make);
    m.start();
    install(Some(m.clone()));
    m
}

#[derive(Deserialize)]
pub struct MetricsQuery {
    range: Option<String>,
}

pub async fn metrics_get(State(st): State<AppState>, Path(s): Path<String>, Query(q): Query<MetricsQuery>) -> Result<Json<Value>, ApiError> {
    let range = match q.range.as_deref() {
        None => Range::Hour,
        Some(r) => Range::parse(r).ok_or_else(|| ApiError::bad_request("The range must be 1h, 6h or 24h."))?,
    };
    blocking(move || {
        let e = st.entry(&s)?;
        let view = current().map(|m| m.metrics(&e, range, monitor::now_unix())).unwrap_or_default();
        Ok(Json(json!({"samples": view.samples, "latest": view.latest})))
    })
    .await
}

pub async fn alerts_list() -> Json<Value> {
    let list = current().map(|m| m.alerts()).unwrap_or_default();
    Json(json!({"alerts": list}))
}

pub async fn alerts_ack(Path(id): Path<String>) -> Result<Response, ApiError> {
    let id: u64 = id.parse().map_err(|_| ApiError::bad_request("That is not an alert number."))?;
    match current() {
        Some(m) if m.ack(id) => Ok(StatusCode::NO_CONTENT.into_response()),
        _ => Err(ApiError::not_found("There is no alert with that number.")),
    }
}

pub async fn alert_settings_get(State(st): State<AppState>, Path(s): Path<String>) -> Result<Json<Value>, ApiError> {
    blocking(move || {
        let e = st.entry(&s)?;
        Ok(Json(alerts::settings_view(&alerts::load_settings(&e.dir))))
    })
    .await
}

pub async fn alert_settings_set(State(st): State<AppState>, Extension(caller): Extension<Caller>, Path(s): Path<String>, Json(body): Json<Value>) -> Result<Json<Value>, ApiError> {
    blocking(move || {
        let e = st.entry(&s)?;
        let cur = alerts::load_settings(&e.dir);
        let mut next = alerts::apply_update(&cur, &body).map_err(ApiError::bad_request)?;
        if next.webhook_url != cur.webhook_url {
            // The panel sends this address requests from inside the network. Someone on another computer
            // must not be able to aim it at this machine or the private network around it, now or later
            // (the flag makes every send check the address again).
            next.webhook_set_remotely = caller.remote && next.webhook_url.is_some();
            if next.webhook_set_remotely {
                alerts::public_webhook(next.webhook_url.as_deref().unwrap_or(""), &|h, p| (h, p).to_socket_addrs().map(Iterator::collect)).map_err(ApiError::bad_request)?;
            }
        }
        alerts::save_settings(&e.dir, &next).map_err(ApiError::internal)?;
        Ok(Json(alerts::settings_view(&next)))
    })
    .await
}
