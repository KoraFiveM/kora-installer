//! Backups of a server: list, make, check, preview a restore, restore, undo, delete, settings, download.
//!
//! Each `pub async fn` is mounted by `crate::routes` for the [`kora_core::api::Op`] of the same name.
//!
//! Who may do what (see `remote/roles.rs`): reading the list, making a backup (into the usual folder),
//! checking one and previewing a restore need an **operator**; restoring, undoing, deleting, pruning,
//! changing the settings, choosing another folder, and downloading the archive need an **administrator**
//! (a download hands out the licence key, the database login and the txAdmin admin; a restore replaces a
//! running server's data), and from a remote session signed in with a user token also the
//! remote-destructive switch. A password only travels in a request body, once; it is never put in a job
//! event, the log or an answer.
#![allow(clippy::unused_async)]

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use kora_core::api::wire::{BackupCreateRequest, BackupsRemoved, PassphraseRequest};
use kora_core::api::Op;
use kora_core::backup::BackupSettings;
use serde_json::{json, Value};

use crate::remote::{Caller, Role};
use crate::{blocking, ApiError, AppState};

/// Failed scheduled backups go to the panel's alert list and, through it, to Discord and the generic webhook.
struct MonitorAnnounce(kora_core::monitor::Monitor);

impl kora_core::backup::scheduler::Announce for MonitorAnnounce {
    fn failed(&self, e: &kora_core::installer::registry::ServerEntry, message: &str) {
        self.0.raise_external(e, kora_core::monitor::AlertKind::BackupFailed, format!("The scheduled backup did not work: {message}"), kora_core::monitor::now_unix());
    }
    fn recovered(&self, e: &kora_core::installer::registry::ServerEntry) {
        self.0.resolve_external(&e.name, kora_core::monitor::AlertKind::BackupFailed, kora_core::monitor::now_unix());
    }
}

/// Start looking for due scheduled backups (they only run while the panel runs, see
/// `kora_core::backup::scheduler`).
pub fn start_scheduler(make: crate::BackendFactory, monitor: kora_core::monitor::Monitor) -> kora_core::backup::scheduler::Handle {
    kora_core::backup::scheduler::spawn(make, std::sync::Arc::new(MonitorAnnounce(monitor)))
}

fn to_json<T: serde::Serialize>(v: &T) -> Result<Value, ApiError> {
    serde_json::to_value(v).map_err(|x| ApiError::internal(x.to_string()))
}

fn is_admin(c: &Caller) -> bool {
    c.identity.as_ref().is_some_and(|i| i.role >= Role::Admin)
}

/// A backup id as Kora makes them: `20260317-041507` or `...-2`.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 40 && id.bytes().all(|b| b.is_ascii_digit() || b == b'-')
}

fn check_id(id: &str) -> Result<(), ApiError> {
    if valid_id(id) {
        Ok(())
    } else {
        Err(ApiError::bad_request("That is not a backup number."))
    }
}

pub async fn backups_overview(State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    blocking(move || {
        let v = st.backend().backups_overview().map_err(ApiError::core)?;
        Ok(Json(to_json(&v)?))
    })
    .await
}

pub async fn backups_list(State(st): State<AppState>, Path(s): Path<String>) -> Result<Json<Value>, ApiError> {
    blocking(move || {
        let e = st.entry(&s)?;
        let v = st.backend().backups_view(&e).map_err(ApiError::core)?;
        Ok(Json(to_json(&v)?))
    })
    .await
}

pub async fn backup_create(State(st): State<AppState>, Extension(caller): Extension<Caller>, Path(s): Path<String>, Json(body): Json<BackupCreateRequest>) -> Result<Response, ApiError> {
    let st2 = st.clone();
    let e = blocking(move || st2.entry(&s)).await?;
    let admin = is_admin(&caller);
    if body.folder.as_deref().is_some_and(|f| !f.trim().is_empty()) && !admin {
        return Err(ApiError::forbidden("Only an administrator can choose another folder for a backup."));
    }
    let make = st.backend.clone();
    let name = e.name.clone();
    st.spawn_job(Op::BackupCreate, Some(name), move |h| {
        let r = make().backup_create(&e, &body, admin, &mut |ev| h.event(json!(ev)))?;
        serde_json::to_value(&r).map_err(|x| x.to_string())
    })
}

pub async fn backup_settings_set(State(st): State<AppState>, Path(s): Path<String>, Json(body): Json<BackupSettings>) -> Result<Json<Value>, ApiError> {
    blocking(move || {
        let e = st.entry(&s)?;
        let v = st.backend().backup_settings_set(&e, &body).map_err(ApiError::core)?;
        Ok(Json(to_json(&v)?))
    })
    .await
}

pub async fn backup_prune(State(st): State<AppState>, Path(s): Path<String>) -> Result<Json<Value>, ApiError> {
    blocking(move || {
        let e = st.entry(&s)?;
        let removed = st.backend().backup_prune(&e).map_err(ApiError::core)?;
        Ok(Json(to_json(&BackupsRemoved { removed })?))
    })
    .await
}

pub async fn backup_undo(State(st): State<AppState>, Path(s): Path<String>, Json(body): Json<PassphraseRequest>) -> Result<Response, ApiError> {
    let st2 = st.clone();
    let e = blocking(move || st2.entry(&s)).await?;
    let make = st.backend.clone();
    let name = e.name.clone();
    st.spawn_job(Op::BackupUndo, Some(name), move |h| {
        let r = make().backup_undo(&e, body.passphrase.as_deref(), &mut |ev| h.event(json!(ev)))?;
        serde_json::to_value(&r).map_err(|x| x.to_string())
    })
}

pub async fn backup_verify(State(st): State<AppState>, Path((s, id)): Path<(String, String)>, Json(body): Json<PassphraseRequest>) -> Result<Response, ApiError> {
    check_id(&id)?;
    let st2 = st.clone();
    let e = blocking(move || st2.entry(&s)).await?;
    let make = st.backend.clone();
    let name = e.name.clone();
    st.spawn_job(Op::BackupVerify, Some(name), move |h| {
        let r = make().backup_verify(&e, &id, body.passphrase.as_deref(), &mut |ev| h.event(json!(ev)))?;
        serde_json::to_value(&r).map_err(|x| x.to_string())
    })
}

pub async fn backup_preview(State(st): State<AppState>, Path((s, id)): Path<(String, String)>, Json(body): Json<PassphraseRequest>) -> Result<Json<Value>, ApiError> {
    check_id(&id)?;
    blocking(move || {
        let e = st.entry(&s)?;
        let v = st.backend().backup_preview(&e, &id, body.passphrase.as_deref()).map_err(ApiError::core)?;
        Ok(Json(to_json(&v)?))
    })
    .await
}

pub async fn backup_restore(State(st): State<AppState>, Path((s, id)): Path<(String, String)>, Json(body): Json<PassphraseRequest>) -> Result<Response, ApiError> {
    check_id(&id)?;
    let st2 = st.clone();
    let e = blocking(move || st2.entry(&s)).await?;
    let make = st.backend.clone();
    let name = e.name.clone();
    st.spawn_job(Op::BackupRestore, Some(name), move |h| {
        let r = make().backup_restore(&e, &id, body.passphrase.as_deref(), &mut |ev| h.event(json!(ev)))?;
        serde_json::to_value(&r).map_err(|x| x.to_string())
    })
}

pub async fn backup_delete(State(st): State<AppState>, Path((s, id)): Path<(String, String)>) -> Result<Response, ApiError> {
    check_id(&id)?;
    blocking(move || {
        let e = st.entry(&s)?;
        st.backend().backup_delete(&e, &id).map_err(ApiError::core)?;
        Ok(StatusCode::NO_CONTENT.into_response())
    })
    .await
}

/// The archive itself, streamed from disk (never held in memory).
pub async fn backup_download(State(st): State<AppState>, Path((s, id)): Path<(String, String)>) -> Result<Response, ApiError> {
    check_id(&id)?;
    let (path, len) = blocking(move || {
        let e = st.entry(&s)?;
        let p = st.backend().backup_file(&e, &id).map_err(ApiError::core)?;
        let len = std::fs::metadata(&p).map_err(|x| ApiError::internal(x.to_string()))?.len();
        Ok((p, len))
    })
    .await?;
    stream_file(path, len).await
}

/// Send a file as a download.
pub(crate) async fn stream_file(path: std::path::PathBuf, len: u64) -> Result<Response, ApiError> {
    use std::io::Read;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "backup".into());
    // Only characters that are safe in a header; a name made by Kora already is.
    let safe: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '_' }).collect();
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<axum::body::Bytes, std::io::Error>>(4);
    let mut file = std::fs::File::open(&path).map_err(|x| ApiError::not_found(format!("The file could not be opened: {x}")))?;
    tokio::task::spawn_blocking(move || {
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            match file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if tx.blocking_send(Ok(axum::body::Bytes::copy_from_slice(&buf[..n]))).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = tx.blocking_send(Err(e));
                    break;
                }
            }
        }
    });
    let body = Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx));
    let mut resp = Response::new(body);
    let h = resp.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{safe}\"")) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_only_digits_and_dashes() {
        assert!(valid_id("20260317-041507") && valid_id("20260317-041507-2"));
        assert!(!valid_id("../etc") && !valid_id("") && !valid_id("a b"));
    }
}
