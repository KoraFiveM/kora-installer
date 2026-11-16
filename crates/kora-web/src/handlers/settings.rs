//! Per-server settings: Discord webhook, update schedule, ports.
//!
//! Each `pub async fn` is mounted by `crate::routes` for the [`Op`] of the same name; the HTTP
//! method and path come from `Op::route`, so only the handler's extractors live here.
#![allow(clippy::unused_async)]

use kora_core::api::Op;

use crate::ApiError;

pub async fn webhook_get() -> ApiError {
    ApiError::not_implemented(Op::WebhookGet)
}

pub async fn webhook_set() -> ApiError {
    ApiError::not_implemented(Op::WebhookSet)
}

pub async fn webhook_remove() -> ApiError {
    ApiError::not_implemented(Op::WebhookRemove)
}

pub async fn webhook_test() -> ApiError {
    ApiError::not_implemented(Op::WebhookTest)
}

pub async fn schedule_get() -> ApiError {
    ApiError::not_implemented(Op::ScheduleGet)
}

pub async fn schedule_set() -> ApiError {
    ApiError::not_implemented(Op::ScheduleSet)
}

pub async fn schedule_remove() -> ApiError {
    ApiError::not_implemented(Op::ScheduleRemove)
}

pub async fn ports_set() -> ApiError {
    ApiError::not_implemented(Op::PortsSet)
}
