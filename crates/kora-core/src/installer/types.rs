//! Shared installer contract between the orchestrator (kora-core) and front ends (CLI, panel).
//! Keep this file stable; both sides code against it.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Plain clone pinned to catalog release tags.
    Production,
    /// Forks into the user's org/account, tracking `dev`.
    Development,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DbChoice {
    /// Use the detected local MariaDB/MySQL; create a NEW db+user (admin creds via `DbAdmin`).
    UseExisting,
    /// Install MariaDB first if none is found (needs admin), then as UseExisting.
    InstallIfMissing,
    /// Do not touch databases (user supplies a connection string later).
    Skip,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResourceSelection {
    /// Every catalog entry with kind kora|dependency (never internal).
    All,
    /// Dependencies + core (pre group) + these ids.
    Only(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Ports {
    pub game: u16,
    pub txadmin: u16,
}

impl Default for Ports {
    fn default() -> Self {
        Self { game: 30120, txadmin: 40120 }
    }
}

/// Database administrator login used once to create the NEW database + user.
/// The password lives in memory only: never serialised, redacted in `Debug`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbAdmin {
    pub host: String,
    pub port: u16,
    pub user: String,
    #[serde(skip)]
    pub password: Option<String>,
}

impl std::fmt::Debug for DbAdmin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DbAdmin").field("host", &self.host).field("port", &self.port).field("user", &self.user).field("password", &self.password.as_ref().map(|_| "<redacted>")).finish()
    }
}

/// Everything the wizard / website / flags decide. Secrets (license key) live here only in memory.
#[derive(Clone, Serialize, Deserialize)]
pub struct InstallRequest {
    pub mode: Mode,
    /// Display + registry name; also used for service and db names after sanitising.
    pub name: String,
    /// Server root directory (resources/, artifacts/, secrets.cfg...).
    pub dir: PathBuf,
    pub hostname: String,
    pub license_key: Option<String>,
    /// The saved key (see `keystore`) the licence key comes from, when it came from there. A name is
    /// not a secret: it is kept in the saved answers and in a stopped install, so they can carry on
    /// without the key being written anywhere else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_key_name: Option<String>,
    /// Linked Cfx identifier for the txAdmin master (e.g. "fivem:123456"), if known.
    pub cfx_identifier: Option<String>,
    pub ports: Ports,
    /// Open the txAdmin port in the firewall (default: keep it closed). txAdmin 8.1.2 cannot bind
    /// only the panel to loopback (see `txadmin` module docs), so this controls firewall exposure.
    #[serde(default)]
    pub txadmin_public: bool,
    /// Artifact download URL (validated), `None` = use recommended.
    pub artifact_url: Option<String>,
    pub game_build: Option<u32>,
    pub resources: ResourceSelection,
    pub db: DbChoice,
    /// Install + enable OS service (needs admin).
    pub service: bool,
    /// Open firewall ports (needs admin).
    pub firewall: bool,
    /// Dev mode only: GitHub org (or user) to fork into.
    pub github_owner: Option<String>,
    /// Catalog URL or local path override (tests / offline). `None` = default + embedded fallback.
    pub catalog: Option<String>,
    /// Plan only: describe every step and change, execute nothing.
    pub dry_run: bool,
    /// Optional database admin login (preferred over the `KORA_DB_*` environment variables).
    #[serde(default)]
    pub db_admin: Option<DbAdmin>,
    /// Start the server when the install is done (final `launch` step) and wait until it answers.
    #[serde(default)]
    pub start: bool,
    /// Retired in 0.7.0 and ignored: the old machine-wide "Kora app" is no longer installed (the install
    /// scripts make per-user shortcuts instead). Kept so saved requests and old clients still load.
    #[serde(default, skip_serializing)]
    pub app: Option<crate::app_install::AppChoice>,
}

impl std::fmt::Debug for InstallRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstallRequest")
            .field("mode", &self.mode)
            .field("name", &self.name)
            .field("dir", &self.dir)
            .field("license_key", &self.license_key.as_ref().map(|_| "<redacted>"))
            .field("license_key_name", &self.license_key_name)
            .field("ports", &self.ports)
            .field("db_admin", &self.db_admin)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepStatus {
    Pending,
    Running,
    Done,
    Skipped,
    Failed,
}

/// Progress event streamed to the front end.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressEvent {
    /// Stable step id, e.g. "prepare.git", "build.clone", "build.db".
    pub step: String,
    /// Plain-language title shown to the user.
    pub title: String,
    pub status: StepStatus,
    /// Live detail line ("Cloning kora_core (12/55)").
    pub detail: String,
    /// 0..=100 overall progress.
    pub percent: u8,
}

/// Sink the orchestrator reports to (CLI printer, panel job, test collector).
pub trait ProgressSink: Send {
    fn event(&mut self, ev: ProgressEvent);
}

/// Result summary shown on the Launch screen.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct InstallOutcome {
    pub dir: PathBuf,
    pub connect: String,
    pub txadmin_url: String,
    pub notes: Vec<String>,
    /// Registered server name (for Start / Stop).
    #[serde(default)]
    pub name: String,
    /// The `launch` step ran (the request asked for a start).
    #[serde(default)]
    pub start_attempted: bool,
    /// Started and answering (txAdmin up, no problem).
    #[serde(default)]
    pub running: bool,
    /// txAdmin answered on its port (true even when the game server was refused, e.g. a bad licence).
    #[serde(default)]
    pub txadmin_up: bool,
    #[serde(default)]
    pub game_listening: bool,
    /// Plain-language reason it is not running (rejected licence, crash, port clash, timeout...).
    #[serde(default)]
    pub launch_problem: Option<String>,
    /// Last log lines for a failed start.
    #[serde(default)]
    pub log_tail: Vec<String>,
    /// Generated local txAdmin admin (no Cfx identity was linked).
    #[serde(default)]
    pub admin_user: Option<String>,
    /// File holding that admin's password; the password itself is never put in the outcome or logs.
    #[serde(default)]
    pub admin_password_file: Option<PathBuf>,
}
