//! Live console, sending commands, enabling the console helper resource.
//!
//! Each `pub async fn` is mounted by `crate::routes` for the [`Op`] of the same name; the HTTP
//! method and path come from `Op::route`, so only the handler's extractors live here.
#![allow(clippy::unused_async)]

use kora_core::api::Op;

use crate::ApiError;

pub async fn console_stream() -> ApiError {
    ApiError::not_implemented(Op::ConsoleStream)
}

pub async fn console_send() -> ApiError {
    ApiError::not_implemented(Op::ConsoleSend)
}

pub async fn console_enable() -> ApiError {
    ApiError::not_implemented(Op::ConsoleEnable)
}
