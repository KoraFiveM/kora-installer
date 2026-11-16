//! Installed resources, ensure, catalog, adding resources.
//!
//! Each `pub async fn` is mounted by `crate::routes` for the [`Op`] of the same name; the HTTP
//! method and path come from `Op::route`, so only the handler's extractors live here.
#![allow(clippy::unused_async)]

use kora_core::api::Op;

use crate::ApiError;

pub async fn resources_list() -> ApiError {
    ApiError::not_implemented(Op::ResourcesList)
}

pub async fn resource_ensure() -> ApiError {
    ApiError::not_implemented(Op::ResourceEnsure)
}

pub async fn catalog_for_server() -> ApiError {
    ApiError::not_implemented(Op::CatalogForServer)
}

pub async fn resources_plan() -> ApiError {
    ApiError::not_implemented(Op::ResourcesPlan)
}

pub async fn resources_add() -> ApiError {
    ApiError::not_implemented(Op::ResourcesAdd)
}

pub async fn catalog_get() -> ApiError {
    ApiError::not_implemented(Op::CatalogGet)
}
