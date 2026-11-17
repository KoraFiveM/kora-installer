//! Backups of a server: the shapes the panel and the CLI send and receive. The archive's own shapes
//! (`Manifest`, `BackupInfo`, `BackupSettings` ...) live in [`crate::backup`] and already have the wire form.
//! A password (`passphrase`) only ever travels in a request body, once; it is never in an answer, a job
//! event, the activity log or a saved record.

use serde::{Deserialize, Serialize};

use crate::backup::{BackupInfo, BackupSettings, BackupSummary, DbKind};

/// The newest safety copy, taken by the last restore: what "Undo this restore" goes back to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct UndoInfo {
    pub id: String,
    pub created_unix: u64,
    pub encrypted: bool,
}

/// What the backup of this server's database will do, said before anyone presses anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DatabaseNote {
    pub kind: DbKind,
    pub note: String,
    /// The program that makes the copy (mariadb-dump / mysqldump) was not found.
    pub tool_missing: bool,
}

/// `GET /servers/{s}/backups`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BackupsView {
    /// Newest first.
    pub backups: Vec<BackupInfo>,
    pub settings: BackupSettings,
    pub summary: BackupSummary,
    /// Where backups are kept, as a path on the computer that runs Kora.
    pub folder: String,
    /// The folder is Kora's own (the default), not one the person chose.
    pub default_folder: bool,
    pub free_bytes: Option<u64>,
    pub undo: Option<UndoInfo>,
    pub database: DatabaseNote,
    /// What the schedule needs to run, in words.
    pub schedule_note: String,
}

/// `POST /servers/{s}/backups`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BackupCreateRequest {
    /// Protect the file with this password (age encryption). Left out: not protected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub passphrase: Option<String>,
    /// Keep every resource in full (a bigger file that needs no internet to restore).
    #[serde(default)]
    pub everything: bool,
    /// Stop the server while the backup runs (a strictly consistent copy).
    #[serde(default)]
    pub stop_server: bool,
    /// Keep this one somewhere else than the usual folder (administrators only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub folder: Option<String>,
    /// A copy made to move the server to another computer: needs a password, is never removed by the retention rules.
    #[serde(default)]
    pub for_move: bool,
}

/// The result of making a backup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BackupCreated {
    pub info: BackupInfo,
    /// Where the file is, on the computer that runs Kora.
    pub path: String,
    pub notes: Vec<String>,
    /// Older backups the retention rules removed.
    pub pruned: Vec<String>,
}

/// A request that may carry the password of a protected backup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PassphraseRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub passphrase: Option<String>,
}

/// The answer to "check this backup".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct VerifyResult {
    /// Every file in the backup matches its checksum.
    pub ok: bool,
    pub files: u32,
    pub bytes: u64,
    pub summary: String,
}

/// Which backups an operation removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BackupsRemoved {
    pub removed: Vec<String>,
}

/// One line of the servers list: the last backup of one server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ServerBackupLine {
    pub server: String,
    pub summary: BackupSummary,
}

/// `GET /backups`: the last backup of every server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BackupsOverview {
    pub servers: Vec<ServerBackupLine>,
}

/// One deleted server and the backups Kora kept of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ArchivedServer {
    pub name: String,
    pub deleted_unix: u64,
    pub backups: Vec<BackupInfo>,
}

/// `GET /backups/archived`: "Backups of servers that no longer exist".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ArchivedBackups {
    pub servers: Vec<ArchivedServer>,
}
