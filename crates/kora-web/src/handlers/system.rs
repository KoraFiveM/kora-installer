//! System information and the live server-status event stream.
//!
//! Each `pub async fn` is mounted by `crate::routes` for the [`Op`] of the same name; the HTTP
//! method and path come from `Op::route`, so only the handler's extractors live here.
#![allow(clippy::unused_async)]

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::Json;
use serde_json::{json, Value};
use tokio_stream::wrappers::ReceiverStream;

use super::servers::list_json;
use crate::{blocking, ApiError, AppState};

/// How often the event stream looks at the servers again.
const POLL: Duration = Duration::from_secs(3);

/// Is this machine without a desktop session (so "open folder" cannot work)?
pub(crate) fn headless() -> bool {
    if let Some(v) = seam::headless_override() {
        return v;
    }
    if std::env::var_os("SSH_CONNECTION").is_some() {
        return true;
    }
    cfg!(unix)
        && !cfg!(target_os = "macos")
        && std::env::var_os("DISPLAY").is_none()
        && std::env::var_os("WAYLAND_DISPLAY").is_none()
}

/// A test seam: lets tests pretend to be headless or not, and choose what "elevated" means.
pub mod seam {
    use std::sync::Mutex;

    static HEADLESS: Mutex<Option<bool>> = Mutex::new(None);

    /// Tests only: force the headless answer (`None` = detect for real).
    pub fn set_headless(v: Option<bool>) {
        *HEADLESS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = v;
    }
    pub(super) fn headless_override() -> Option<bool> {
        *HEADLESS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

pub async fn system_info(State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    let loopback = st
        .allowed_hosts
        .iter()
        .any(|h| h.starts_with("127.") || h.starts_with("localhost:") || h.starts_with("[::1]"));
    blocking(move || {
        let elevated = kora_core::elevate::is_elevated();
        let headless = headless();
        Ok(Json(json!({
            "version": env!("CARGO_PKG_VERSION"),
            "platform": if cfg!(windows) { "windows" } else { "linux" },
            "elevated": elevated,
            // The UI reads `admin`; `elevated` is the same answer under the plan's name.
            "admin": elevated,
            "helper_installed": crate::elevated::helper_installed(),
            "headless": headless,
            "local": loopback && !headless,
            "kora_home": kora_core::installer::registry::kora_home(),
        })))
    })
    .await
}

fn servers_event(v: &Value) -> Event {
    Event::default().event("servers").data(v.to_string())
}

pub async fn events(State(st): State<AppState>) -> Sse<ReceiverStream<Result<Event, Infallible>>> {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(16);
    tokio::spawn(async move {
        let mut last: Option<Value> = None;
        loop {
            if tx.is_closed() {
                return;
            }
            let s = st.clone();
            let now = blocking(move || list_json(&mut *s.backend())).await;
            if let Ok(v) = now {
                if last.as_ref() != Some(&v) {
                    if tx.send(Ok(servers_event(&v))).await.is_err() {
                        return;
                    }
                    last = Some(v);
                }
            }
            tokio::select! {
                () = tokio::time::sleep(POLL) => {}
                () = tx.closed() => return,
            }
        }
    });
    Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}
