//! The install wizard: defaults, checks, artifacts, plan, the install job and its log.
//!
//! Each `pub async fn` is mounted by `crate::routes` for the [`Op`] of the same name; the HTTP
//! method and path come from `Op::route`, so only the handler's extractors live here.
#![allow(clippy::unused_async)]

use kora_core::api::Op;

use crate::ApiError;

pub async fn artifacts_list() -> ApiError {
    ApiError::not_implemented(Op::ArtifactsList)
}

pub async fn install_defaults() -> ApiError {
    ApiError::not_implemented(Op::InstallDefaults)
}

pub async fn install_saved_clear() -> ApiError {
    ApiError::not_implemented(Op::InstallSavedClear)
}

pub async fn check_ports() -> ApiError {
    ApiError::not_implemented(Op::CheckPorts)
}

pub async fn check_disk() -> ApiError {
    ApiError::not_implemented(Op::CheckDisk)
}

pub async fn check_db_login() -> ApiError {
    ApiError::not_implemented(Op::CheckDbLogin)
}

pub async fn install_plan() -> ApiError {
    ApiError::not_implemented(Op::InstallPlan)
}

pub async fn install_start() -> ApiError {
    ApiError::not_implemented(Op::InstallStart)
}

pub async fn install_log() -> ApiError {
    ApiError::not_implemented(Op::InstallLog)
}
