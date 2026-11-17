//! The support bundle: prepare (and show what is in it), read one file, save it, download it again, and the short
//! cleaned summary a bug report may attach.
//!
//! Each `pub async fn` is mounted by `crate::routes` for the [`kora_core::api::Op`] of the same name.
//!
//! Nothing here uploads anything. The bundle is built once by `kora_core::support` (every text through its one
//! redaction function) and kept in memory for fifteen minutes, so what `create` saves is exactly what `preview` and
//! `file` showed. Administrators only (the bundle holds the computer's diagnostics, even with every secret removed);
//! the short summary is open to operators because the issue report uses it.
#![allow(clippy::unused_async)]

use axum::extract::{Path, State};
use axum::response::Response;
use axum::Json;
use kora_core::api::wire::{BundleCreateRequest, BundleFileRequest, BundleFileText, SupportRequest, SupportSummary};
use kora_core::support::service;
use serde_json::{json, Value};

use crate::{blocking, ApiError, AppState};

pub async fn support_bundle_preview(State(st): State<AppState>, Json(body): Json<SupportRequest>) -> Result<Json<Value>, ApiError> {
    blocking(move || {
        let v = st.backend().support_preview(body.server.as_deref()).map_err(ApiError::core)?;
        Ok(Json(serde_json::to_value(v).map_err(|x| ApiError::internal(x.to_string()))?))
    })
    .await
}

pub async fn support_bundle_file(Json(body): Json<BundleFileRequest>) -> Result<Json<Value>, ApiError> {
    blocking(move || {
        let text = service::file_text(&body.id, &body.name).map_err(ApiError::core)?;
        Ok(Json(serde_json::to_value(BundleFileText { name: body.name, text }).map_err(|x| ApiError::internal(x.to_string()))?))
    })
    .await
}

pub async fn support_bundle_create(Json(body): Json<BundleCreateRequest>) -> Result<Json<Value>, ApiError> {
    blocking(move || {
        let saved = service::save(&body.id).map_err(ApiError::core)?;
        Ok(Json(serde_json::to_value(saved).map_err(|x| ApiError::internal(x.to_string()))?))
    })
    .await
}

pub async fn support_bundle_download(Path(file): Path<String>) -> Result<Response, ApiError> {
    let (path, len) = blocking(move || {
        let p = service::saved_path(&file).ok_or_else(|| ApiError::not_found("That support bundle is not there."))?;
        let len = std::fs::metadata(&p).map_err(|x| ApiError::internal(x.to_string()))?.len();
        Ok((p, len))
    })
    .await?;
    super::backups::stream_file(path, len).await
}

pub async fn support_summary(State(st): State<AppState>, Json(body): Json<SupportRequest>) -> Result<Json<Value>, ApiError> {
    blocking(move || {
        let text = st.backend().support_summary(body.server.as_deref()).map_err(ApiError::core)?;
        Ok(Json(json!(SupportSummary { text })))
    })
    .await
}
