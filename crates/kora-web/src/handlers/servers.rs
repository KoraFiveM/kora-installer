//! The server list, one server, start/stop/restart, delete, adopt, open folder, logs.
//!
//! Each `pub async fn` is mounted by `crate::routes` for the [`Op`] of the same name; the HTTP
//! method and path come from `Op::route`, so only the handler's extractors live here.
#![allow(clippy::unused_async)]

use kora_core::api::Op;

use crate::ApiError;

pub async fn servers_list() -> ApiError {
    ApiError::not_implemented(Op::ServersList)
}

pub async fn server_get() -> ApiError {
    ApiError::not_implemented(Op::ServerGet)
}

pub async fn server_control() -> ApiError {
    ApiError::not_implemented(Op::ServerControl)
}

pub async fn server_delete() -> ApiError {
    ApiError::not_implemented(Op::ServerDelete)
}

pub async fn server_unregister() -> ApiError {
    ApiError::not_implemented(Op::ServerUnregister)
}

pub async fn servers_scan() -> ApiError {
    ApiError::not_implemented(Op::ServersScan)
}

pub async fn servers_adopt() -> ApiError {
    ApiError::not_implemented(Op::ServersAdopt)
}

pub async fn server_open_folder() -> ApiError {
    ApiError::not_implemented(Op::ServerOpenFolder)
}

pub async fn logs_list() -> ApiError {
    ApiError::not_implemented(Op::LogsList)
}

pub async fn logs_tail() -> ApiError {
    ApiError::not_implemented(Op::LogsTail)
}
