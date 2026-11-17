//! The audit log (`GET /audit`).
use serde::{Deserialize, Serialize};

/// One row: who did what, when. Never a secret or a request body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AuditRow {
    pub id: i64,
    /// Unix seconds.
    pub at: u64,
    /// The operation's name (`Op::name`).
    pub op: String,
    pub server: Option<String>,
    /// `local` (the person on this computer), `remote` (signed in from another device) or `cli` (a `kora` command).
    pub actor_kind: String,
    pub actor_name: Option<String>,
    /// For remote: `token` or `tailscale`.
    pub provider: Option<String>,
    pub role: Option<String>,
    /// `ok`, `failed`, `refused` or `started`.
    pub result: String,
    /// A short note Kora wrote (a job id, an error code).
    pub detail: Option<String>,
}

/// The filters and paging of `GET /audit`; every one is optional.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AuditQuery {
    /// At most this many (default 100, at most 500).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub limit: Option<u32>,
    /// Only rows older than this id (to show more).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub before: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub server: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub op: Option<String>,
    /// Part of the actor's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub actor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub result: Option<String>,
    /// Only rows at or after this time (unix seconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub since: Option<u64>,
}

/// `GET /audit`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AuditPage {
    /// Newest first.
    pub entries: Vec<AuditRow>,
    /// There are older rows than these.
    pub more: bool,
    pub retention_days: u64,
}
