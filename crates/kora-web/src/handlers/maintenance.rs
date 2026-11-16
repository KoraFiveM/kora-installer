//! Kora itself: self-update, the Kora app, uninstall.
//!
//! Each `pub async fn` is mounted by `crate::routes` for the [`Op`] of the same name; the HTTP
//! method and path come from `Op::route`, so only the handler's extractors live here.
#![allow(clippy::unused_async)]

use kora_core::api::Op;

use crate::ApiError;

pub async fn self_update_check() -> ApiError {
    ApiError::not_implemented(Op::SelfUpdateCheck)
}

pub async fn self_update_apply() -> ApiError {
    ApiError::not_implemented(Op::SelfUpdateApply)
}

pub async fn app_status() -> ApiError {
    ApiError::not_implemented(Op::AppStatus)
}

pub async fn app_install() -> ApiError {
    ApiError::not_implemented(Op::AppInstall)
}

pub async fn app_uninstall() -> ApiError {
    ApiError::not_implemented(Op::AppUninstall)
}

pub async fn uninstall_plan() -> ApiError {
    ApiError::not_implemented(Op::UninstallPlan)
}

pub async fn uninstall_run() -> ApiError {
    ApiError::not_implemented(Op::UninstallRun)
}
