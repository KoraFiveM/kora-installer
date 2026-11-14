//! Shared installer contract between the orchestrator (kora-core) and front ends (CLI/TUI).
//! Keep this file stable; both sides code against it.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
        f.debug_struct("DbAdmin")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .finish()
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
}

impl std::fmt::Debug for InstallRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstallRequest")
            .field("mode", &self.mode)
            .field("name", &self.name)
            .field("dir", &self.dir)
            .field("license_key", &self.license_key.as_ref().map(|_| "<redacted>"))
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

/// Sink the orchestrator reports to (TUI channel, CLI printer, test collector).
pub trait ProgressSink: Send {
    fn event(&mut self, ev: ProgressEvent);
}

/// Result summary shown on the Launch screen.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallOutcome {
    pub dir: PathBuf,
    pub connect: String,
    pub txadmin_url: String,
    pub notes: Vec<String>,
}
