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

/// One line of the administrator service's own audit file: what it did with administrator rights, for whom, and how it went.
/// Never a secret, a path or a typed value: the service writes only the operation's short label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ServiceAuditRow {
    /// Unix seconds.
    pub at: u64,
    /// The account it was done for: its name when the service knew it, else its id.
    pub account: String,
    /// The operation, in short (`run system.firewall [my-server]`, `backup job set [my-server]`).
    pub what: String,
    /// `done`, `failed`, `refused` or `stopped`.
    pub result: String,
    /// The service's own words after the result (`code 1`, why it was refused).
    pub detail: Option<String>,
}

/// `GET /audit/service?limit=`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ServiceAuditQuery {
    /// At most this many (default 200, at most 500).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub limit: Option<u32>,
}

/// The answer: the service's lines, newest first. `available` is false when the service is not there for this account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ServiceAuditPage {
    pub available: bool,
    pub entries: Vec<ServiceAuditRow>,
    /// The lines of every account on this computer (the signed-in account administers the machine); otherwise only the lines about this account.
    pub all_accounts: bool,
    /// Why there is nothing to show, or what to know, in plain words.
    pub note: Option<String>,
}
