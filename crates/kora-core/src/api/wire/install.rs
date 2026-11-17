//! The install wizard: defaults, checks, the plan, the request, progress events, the outcome.
//!
//! `InstallRequest` is what the web page sends. The handler reads it leniently (`api::install::WireInstall`,
//! every field optional with a default); this type is the full form the page always fills in.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::keys::Profile;
use crate::installer::{InstallOutcome, Mode, Ports, Source, Versions};
use crate::problem::Problem;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum DbChoice {
    UseExisting,
    InstallIfMissing,
    Skip,
}

/// `"all"` or `{"only": [ids]}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ResourceSelection {
    All,
    Only(Vec<String>),
}

/// The database administrator login, used once for the install. Never stored or returned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DbAdmin {
    pub host: String,
    pub port: u16,
    pub user: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub password: Option<String>,
}

/// `POST /installs` and `POST /installs/plan`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct InstallRequest {
    /// The old spelling (`production` = clone + released, `development` = fork + latest). Optional: `source` and
    /// `versions` win when they are sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub mode: Option<Mode>,
    /// Plain clones of the originals, or your own copies (forks).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub source: Option<Source>,
    /// Released versions (pinned) or the latest changes (the main branch).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub versions: Option<Versions>,
    pub name: String,
    pub dir: String,
    pub hostname: String,
    /// A pasted licence key (never stored in the install record; see `license_key_name`).
    pub license_key: Option<String>,
    /// Use the saved licence key with this name instead of carrying a key (the key is looked up by Kora).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub license_key_name: Option<String>,
    /// With a pasted `license_key`: save it under this name for the next install.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub save_license_key_as: Option<String>,
    pub ports: Ports,
    pub txadmin_public: bool,
    pub artifact_url: Option<String>,
    pub game_build: Option<u32>,
    pub resources: ResourceSelection,
    pub db: DbChoice,
    pub service: bool,
    pub firewall: bool,
    pub github_owner: Option<String>,
    pub dry_run: bool,
    pub db_admin: Option<DbAdmin>,
    pub start: bool,
    /// Cfx.re account that becomes the txAdmin admin, e.g. `fivem:4465268`. Null = a generated local admin.
    pub cfx_identifier: Option<String>,
}

/// The answers of an install that stopped part-way, as saved: never a licence key or a database login.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SavedInstall {
    /// The old spelling, always sent.
    pub mode: Mode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub source: Option<Source>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub versions: Option<Versions>,
    pub name: String,
    pub dir: String,
    pub hostname: String,
    pub ports: Ports,
    pub txadmin_public: bool,
    pub artifact_url: Option<String>,
    pub game_build: Option<u32>,
    pub resources: ResourceSelection,
    pub db: DbChoice,
    pub service: bool,
    pub firewall: bool,
    pub github_owner: Option<String>,
    pub start: bool,
    /// The saved licence key to use, by name.
    pub license_key_name: Option<String>,
    /// The Cfx.re account that becomes the txAdmin admin (not a secret).
    pub cfx_identifier: Option<String>,
}

/// A stopped install that can carry on (also after the panel was restarted).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Resumable {
    pub job: String,
    pub name: String,
    pub failed_step: Option<String>,
}

/// `GET /installs/defaults`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct InstallDefaults {
    pub name: String,
    pub dir: String,
    pub hostname: String,
    pub ports: Ports,
    /// Answers saved from an install that stopped part-way (never contains secrets).
    pub saved: Option<SavedInstall>,
    /// What Kora remembers between installs (nothing secret).
    pub profile: Profile,
    /// The saved answers name a saved licence key that no longer exists (its old name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub saved_key_missing: Option<String>,
    pub resumable: Vec<Resumable>,
}

/// One port box of the wizard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PortLine {
    pub port: u16,
    pub ok: bool,
    pub text: String,
    /// The next free port when this one is taken.
    pub suggestion: Option<u16>,
}

/// `POST /checks/ports`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PortsCheck {
    pub game: PortLine,
    pub txadmin: PortLine,
}

/// `POST /checks/disk` `{dir}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DirRequest {
    pub dir: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DiskCheck {
    pub ok: bool,
    pub free_bytes: u64,
    pub needed_bytes: u64,
    pub message: String,
}

/// `POST /checks/db-login` `{admin}`; `null` = let Kora log in by itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DbLoginRequest {
    pub admin: Option<DbAdmin>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DbLoginCheck {
    pub ok: bool,
    pub message: String,
    /// Only on a failure: a login that failed on a database that is installed but stopped carries the "Start it for me" button.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub problem: Option<Problem>,
}

/// `POST /checks/cfx` `{username}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CfxRequest {
    pub username: String,
}

/// Answer of `POST /checks/cfx` (HTTP 200 whether or not the account was found).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CfxCheck {
    pub ok: bool,
    pub id: Option<u64>,
    pub username: Option<String>,
    pub identifier: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PlanStep {
    pub id: String,
    pub title: String,
}

/// `POST /installs/plan`: what the install would do. Nothing is changed and no secret is returned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct InstallPlan {
    /// The answers in plain words.
    pub summary: String,
    pub steps: Vec<PlanStep>,
    /// Plain-language list of what needs administrator rights.
    pub needs_admin: Vec<String>,
    /// Kora already runs with administrator rights.
    pub elevated: bool,
    pub disk: DiskCheck,
    pub warnings: Vec<String>,
    /// The same list as `warnings` (older name).
    pub notes: Vec<String>,
    /// Windows: these parts are done after ONE permission prompt when the install starts.
    pub admin_prompt: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Artifact {
    pub build: u32,
    pub url: String,
}

/// `GET /artifacts`: builds that are not known to be broken, newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ArtifactList {
    pub recommended: Option<u32>,
    pub items: Vec<Artifact>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    Running,
    Done,
    Skipped,
    Failed,
}

/// A step of the install changed (an install job event).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ProgressEvent {
    /// Stable step id, e.g. `prepare.git`, `build.clone`; `admin` for the Windows permission prompt.
    pub step: String,
    pub title: String,
    pub status: StepStatus,
    /// Live detail line ("Cloning kora_core (12/55)").
    pub detail: String,
    /// 0..=100 overall progress.
    pub percent: u8,
}

/// What an install job reports: a step changed, or (an install run by the Linux helper) a line of its output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(untagged)]
pub enum InstallEvent {
    Progress(ProgressEvent),
    Line(super::servers::StatusEvent),
}

/// `POST /installs/{job}/resume`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ResumeRequest {
    /// Skip this step first (only steps that are safe to skip).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub skip: Option<String>,
    /// Carry on without the parts that need administrator rights.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub without_admin: Option<bool>,
}

/// `GET /installs/{job}/log`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct InstallLog {
    /// Where the log file is (null until it is written).
    pub path: Option<PathBuf>,
    /// The end of the log.
    pub lines: Vec<String>,
}

/// The result of an install job.
pub type InstallResult = InstallOutcome;
