//! The shapes of a server backup: the manifest inside the archive, the small listing record kept
//! next to it, and the choices a person makes. All secret-free (the archive itself is not).

use serde::{Deserialize, Serialize};

use crate::installer::{Source, Versions};

/// Version of the archive layout. A Kora that reads a higher number refuses politely.
pub const FORMAT: u32 = 1;

/// Why a backup was made. Decides what pruning may remove.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Label {
    /// Started by a person.
    #[default]
    Manual,
    /// Started by the schedule.
    Scheduled,
    /// Taken automatically just before a restore, so the restore can be undone.
    BeforeRestore,
    /// Made to move the server to another computer (never pruned).
    Move,
}

impl Label {
    pub fn text(self) -> &'static str {
        match self {
            Label::Manual => "Made by you",
            Label::Scheduled => "Scheduled",
            Label::BeforeRestore => "Safety copy before a restore",
            Label::Move => "For moving the server",
        }
    }
}

/// How the resources are kept in the archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ResourceChoice {
    /// By reference (repository and exact commit) where they can be fetched again; full files for the rest.
    #[default]
    Auto,
    /// Every resource in full, history included: a bigger archive that needs no internet to restore.
    Everything,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum DbKind {
    /// A consistent logical dump of the server's own database is in the archive.
    Dumped,
    /// The server has no database that Kora knows (skipped at install, or none recorded).
    None,
    /// The server uses a database on another computer; it was dumped over the network with the recorded login.
    External,
    /// A dump was wanted but could not be made (only for a backup the person chose to carry on with).
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DatabaseBackup {
    pub kind: DbKind,
    /// The database name (never a password).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub port: Option<u16>,
    /// Size of the dump, uncompressed.
    #[serde(default)]
    pub dump_bytes: u64,
    /// What a person should know (honest wording for "skipped" and "external").
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum StoredAs {
    /// Repository, branch and exact commit; fetched again on restore.
    Reference,
    /// The folder's files (and its git history) are in the archive.
    Full,
}

/// Where a resource is relative to its normal version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum VersionKind {
    /// On the version the server follows (the released version, or the main branch).
    #[default]
    Normal,
    /// On a named branch.
    Branch,
    /// Trying a pull request.
    Pr,
    /// On a commit that is not a branch and not the normal version.
    Detached,
}

/// How a resource was set when the backup was made: put back exactly so on restore and import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct VersionState {
    pub kind: VersionKind,
    /// The branch (also for a pull request: its local branch).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub branch: Option<String>,
    /// The pull request being tried.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub pr: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub pr_title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ResourceBackup {
    pub id: String,
    /// Relative to `<server>/resources/`.
    pub path: String,
    pub stored: StoredAs,
    /// Where the files are fetched from (the person's copy for a fork server).
    #[serde(default)]
    pub repository: String,
    /// The original repository (the catalogue's), for "use the originals" when a fork cannot be used.
    #[serde(default)]
    pub upstream: String,
    /// Normal version, a branch, or a pull request being tried.
    #[serde(default)]
    pub state: VersionState,
    #[serde(default)]
    pub branch: String,
    /// The commit the folder was on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub commit: Option<String>,
    /// Why the files had to be kept in full ("changed here", "kept-aside changes", "paid asset", ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub why_full: Option<String>,
    /// Bytes in the archive for this resource (0 by reference).
    #[serde(default)]
    pub bytes: u64,
}

/// One file inside the archive, with its checksum.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FileEntry {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ServerInfo {
    pub name: String,
    /// The old spelling of the pair below ("production" or "development"), still written for older Kora versions.
    pub mode: String,
    /// Plain clones of the original repositories, or the person's own copies (forks).
    pub source: Source,
    /// Released versions or the latest changes.
    pub versions: Versions,
    pub game_port: u16,
    pub txadmin_port: u16,
    pub game_build: u32,
    /// The FXServer build the server ran. Not in the archive: it is downloaded again.
    pub artifact_build: u32,
    /// "linux" or "windows": where the backup was made.
    pub os: String,
    /// The folder it lived in (shown, and used to rewrite paths when it moves).
    pub dir: String,
    /// The GitHub account the forks are in (a fork server only). A login is not a secret.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub github_owner: Option<String>,
    /// How the server ran: as a service (or by hand), or in a container. Older backups have no such field: service.
    #[serde(default)]
    pub run_mode: crate::container::RunMode,
}

/// What the archive says about itself: written first (without the checksums) and last (with them).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Manifest {
    pub format: u32,
    pub kora_version: String,
    pub created_unix: u64,
    pub label: Label,
    pub server: ServerInfo,
    pub database: DatabaseBackup,
    pub resources: Vec<ResourceBackup>,
    /// The server was stopped while it was copied: strictly consistent. Otherwise "consistent enough":
    /// the database is one snapshot, the files were copied one by one while the server ran.
    pub stopped: bool,
    /// Things worth knowing, in plain words.
    #[serde(default)]
    pub notes: Vec<String>,
    /// About how many bytes the files add up to, uncompressed (for the space check before a restore).
    #[serde(default)]
    pub payload_bytes: u64,
    /// Every file in the archive (empty in the first copy).
    #[serde(default)]
    pub files: Vec<FileEntry>,
}

impl Manifest {
    pub fn by_reference(&self) -> usize {
        self.resources.iter().filter(|r| r.stored == StoredAs::Reference).count()
    }
    pub fn in_full(&self) -> usize {
        self.resources.iter().filter(|r| r.stored == StoredAs::Full).count()
    }
}

/// The small file next to an archive: enough to list it without opening it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BackupInfo {
    pub id: String,
    /// The archive's file name.
    pub file: String,
    pub bytes: u64,
    pub sha256: String,
    pub encrypted: bool,
    pub created_unix: u64,
    pub label: Label,
    pub kora_version: String,
    pub server: String,
    pub database: DbKind,
    pub by_reference: u32,
    pub in_full: u32,
    pub stopped: bool,
    /// Made by a newer Kora than this one: it can be listed but not restored here.
    #[serde(default)]
    pub too_new: bool,
}

/// A line of the answer about where backups go.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Retention {
    /// Keep this many of the newest backups (0 = no limit).
    pub keep_last: u32,
    /// Also keep the newest backup of each day for this many days (0 = off).
    pub keep_daily_days: u32,
}

impl Default for Retention {
    fn default() -> Self {
        Retention { keep_last: 7, keep_daily_days: 0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Cadence {
    #[default]
    Daily,
    Weekly,
}

/// The backup schedule of one server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BackupSchedule {
    pub enabled: bool,
    pub cadence: Cadence,
    /// `HH:MM`, 24 hours, local time.
    pub time: String,
    /// 0 = Monday .. 6 = Sunday (weekly).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub weekday: Option<u8>,
}

impl Default for BackupSchedule {
    fn default() -> Self {
        BackupSchedule { enabled: false, cadence: Cadence::Daily, time: "03:30".into(), weekday: None }
    }
}

/// Per-server backup settings (kept in Kora's own store, never in the server folder, so deleting the
/// server does not delete them).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BackupSettings {
    /// A folder to keep backups in; empty = Kora's own backups folder.
    #[serde(default)]
    pub folder: String,
    #[serde(default)]
    pub retention: Retention,
    #[serde(default)]
    pub schedule: BackupSchedule,
    #[serde(default)]
    pub resources: ResourceChoice,
    /// Stop the server while a SCHEDULED backup runs (strictly consistent, but players are kicked).
    #[serde(default)]
    pub stop_for_scheduled: bool,
}

impl Default for BackupSettings {
    fn default() -> Self {
        BackupSettings { folder: String::new(), retention: Retention::default(), schedule: BackupSchedule::default(), resources: ResourceChoice::Auto, stop_for_scheduled: false }
    }
}

/// The last attempts, kept in the store: what the Overview shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BackupStatus {
    /// When the newest backup that worked finished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub last_ok_unix: Option<u64>,
    /// The last attempt failed: when and why (plain words).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub last_failed_unix: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub last_error: Option<String>,
    /// The safety copy taken by the last restore (what "Undo this restore" goes back to).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub undo_id: Option<String>,
    /// The last scheduled run, so a missed one is noticed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub last_scheduled_unix: Option<u64>,
}

/// The "last backup" line of the Overview and the servers list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BackupSummary {
    pub last_ok_unix: Option<u64>,
    /// "No backup yet" / "Last backup 3 days ago".
    pub text: String,
    /// The newest backup is older than the schedule implies, or the last run failed.
    pub warning: Option<String>,
    pub scheduled: bool,
}
