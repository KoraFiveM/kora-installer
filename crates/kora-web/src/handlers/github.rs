//! GitHub sign-in, fork owners, issue reports and pull requests.
//!
//! Each `pub async fn` is mounted by `crate::routes` for the [`Op`] of the same name; the HTTP
//! method and path come from `Op::route`, so only the handler's extractors live here.
#![allow(clippy::unused_async)]

use kora_core::api::Op;

use crate::ApiError;

pub async fn github_status() -> ApiError {
    ApiError::not_implemented(Op::GithubStatus)
}

pub async fn github_login() -> ApiError {
    ApiError::not_implemented(Op::GithubLogin)
}

pub async fn github_owners() -> ApiError {
    ApiError::not_implemented(Op::GithubOwners)
}

pub async fn issue_report() -> ApiError {
    ApiError::not_implemented(Op::IssueReport)
}

pub async fn pr_plan() -> ApiError {
    ApiError::not_implemented(Op::PrPlan)
}

pub async fn pr_open() -> ApiError {
    ApiError::not_implemented(Op::PrOpen)
}
