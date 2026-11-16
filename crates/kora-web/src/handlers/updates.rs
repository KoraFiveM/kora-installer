//! Update preview/apply/auto, snapshots and rollback.
//!
//! Each `pub async fn` is mounted by `crate::routes` for the [`Op`] of the same name; the HTTP
//! method and path come from `Op::route`, so only the handler's extractors live here.
#![allow(clippy::unused_async)]

use kora_core::api::Op;

use crate::ApiError;

pub async fn update_preview() -> ApiError {
    ApiError::not_implemented(Op::UpdatePreview)
}

pub async fn update_apply() -> ApiError {
    ApiError::not_implemented(Op::UpdateApply)
}

pub async fn update_auto() -> ApiError {
    ApiError::not_implemented(Op::UpdateAuto)
}

pub async fn snapshots_list() -> ApiError {
    ApiError::not_implemented(Op::SnapshotsList)
}

pub async fn snapshot_rollback() -> ApiError {
    ApiError::not_implemented(Op::SnapshotRollback)
}
