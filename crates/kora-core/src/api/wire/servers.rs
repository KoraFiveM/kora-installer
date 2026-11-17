//! The server list, one server, control, delete, adopt, logs, console, resources.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::api::{Status, StatusDetail};
use crate::installer::{Mode, Ports};
use crate::service::ControlAction;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum BadgeKind {
    UpToDate,
    Updates,
    NeedsAttention,
    NewRelease,
}

/// The small update badge of a server (`None` on the server = not known yet).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Badge {
    pub kind: BadgeKind,
    /// How many updates (`kind: updates` only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub count: Option<u32>,
}

impl From<crate::api::UpdateBadge> for Badge {
    fn from(b: crate::api::UpdateBadge) -> Self {
        use crate::api::UpdateBadge as U;
        match b {
            U::UpToDate => Badge { kind: BadgeKind::UpToDate, count: None },
            U::Updates(n) => Badge { kind: BadgeKind::Updates, count: Some(n) },
            U::NeedsAttention => Badge { kind: BadgeKind::NeedsAttention, count: None },
            U::NewRelease => Badge { kind: BadgeKind::NewRelease, count: None },
        }
    }
}

/// One server in the list (`GET /servers`, the `servers` event, `POST /servers/adopt`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ServerSummary {
    pub name: String,
    pub dir: PathBuf,
    pub mode: Mode,
    pub ports: Ports,
    pub created_unix: u64,
    pub status: Status,
    pub badge: Option<Badge>,
    /// false = adopted (an existing server Kora did not install).
    pub managed: bool,
    /// `<this computer's network address>:<game port>`: what players on the same network type to join. `null` when
    /// Kora could not find the address. It is not an internet address (that is the router's, which Kora cannot see).
    pub connect: Option<String>,
}

/// `GET /servers` and the data of the `servers` event of `GET /events`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ServersList {
    pub servers: Vec<ServerSummary>,
}

/// `GET /servers/{s}`: the summary and what the launcher can see right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ServerDetail {
    #[serde(flatten)]
    pub summary: ServerSummary,
    pub running: bool,
    pub game_listening: bool,
    pub txadmin_up: bool,
    /// The same three facts as one object.
    pub status_detail: StatusDetail,
    pub console_enabled: bool,
}

/// `POST /servers/{s}/control`, `POST /nodes/{id}/servers/{s}/control`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ControlRequest {
    pub action: ControlAction,
}

/// What a job reports while it runs: a line of text (`status`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StatusEvent {
    Status { message: String },
}

/// Result of a control job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ControlResult {
    pub status: Status,
}

/// `DELETE /servers/{s}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DeleteRequest {
    /// The server's name, typed again to confirm.
    pub confirm_name: String,
    #[serde(default)]
    pub keep_files: bool,
    #[serde(default)]
    pub keep_db: bool,
    /// Only when dropping the database needs a login the install did not keep. Used once, never stored or returned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub db_admin_password: Option<String>,
}

/// Result of a job that changed things and has notes to show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Notes {
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ScanKind {
    Kora,
    OldKora,
    FxServer,
}

/// A server folder found by "Look for servers".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ScanCandidate {
    pub name: String,
    pub dir: PathBuf,
    pub mode: Mode,
    pub kind: ScanKind,
    pub ports: Ports,
}

/// Result of the scan job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ScanResult {
    pub candidates: Vec<ScanCandidate>,
}

/// `POST /servers/adopt`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AdoptRequest {
    /// The full folder path.
    pub dir: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LogFile {
    pub name: String,
    pub size: u64,
    pub modified_unix: u64,
}

/// `GET /servers/{s}/logs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LogsList {
    pub files: Vec<LogFile>,
}

/// `?lines=` of `GET /servers/{s}/logs/{file}` (default 300).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LogTailQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub lines: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LogTail {
    pub file: String,
    pub lines: Vec<String>,
}

/// `POST /servers/{s}/console`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ConsoleCommand {
    pub command: String,
}

/// Data of the `line` event of `GET /servers/{s}/console/stream`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ConsoleLine {
    pub line: String,
}

/// `GET /servers/{s}/resources`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ResourcesList {
    pub resources: Vec<String>,
}

/// `POST /servers/{s}/ensure`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct EnsureRequest {
    pub name: String,
    #[serde(default)]
    pub restart: bool,
}

/// `POST /servers/{s}/resources/{name}/control`: the same actions as a server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ResourceControlRequest {
    pub action: ControlAction,
}

/// One plain sentence about what was done.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Message {
    pub message: String,
}

impl From<crate::api::adopt::Kind> for ScanKind {
    fn from(k: crate::api::adopt::Kind) -> Self {
        use crate::api::adopt::Kind as K;
        match k {
            K::Kora => ScanKind::Kora,
            K::OldKora => ScanKind::OldKora,
            K::FxServer => ScanKind::FxServer,
        }
    }
}
