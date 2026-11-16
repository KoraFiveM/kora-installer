//! System information and the live server-status event stream.
//!
//! Each `pub async fn` is mounted by `crate::routes` for the [`Op`] of the same name; the HTTP
//! method and path come from `Op::route`, so only the handler's extractors live here.
#![allow(clippy::unused_async)]

use kora_core::api::Op;

use crate::ApiError;

pub async fn system_info() -> ApiError {
    ApiError::not_implemented(Op::SystemInfo)
}

pub async fn events() -> ApiError {
    ApiError::not_implemented(Op::Events)
}
