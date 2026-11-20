//! Updates, snapshots, resources to add, the catalog, notifications, schedule, ports, GitHub.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::servers::Badge;
use crate::addresources::{AddOutcome, AddPlan};
use crate::api::install::github::GithubState;
use crate::catalog::{Creator, Group, Kind};
use crate::problem::Problem;
use crate::service::ScheduleConfig;
use crate::update::manual::{RestartPolicy, Scope};

/// `POST /servers/{s}/update/preview`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct UpdatePreviewRequest {
    pub scope: Scope,
}

/// `POST /servers/{s}/update/apply`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct UpdateApplyRequest {
    pub scope: Scope,
    pub restart: RestartPolicy,
    /// Resource ids to leave alone.
    pub skip: Vec<String>,
    /// The preview's planned commit per resource, so exactly what was shown is applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub targets: Option<BTreeMap<String, String>>,
}

/// Result of the preview job: the engine's preview and the badge it implies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct UpdatePreviewResult {
    #[serde(flatten)]
    pub preview: crate::update::manual::UpdatePreview,
    pub badge: Option<Badge>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct UpdatedResource {
    pub id: String,
    pub from: String,
    pub to: String,
}

/// Result of an update job. Reasons are plain sentences (`[id, sentence]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct UpdateReport {
    pub server: String,
    pub updated: Vec<UpdatedResource>,
    pub skipped: Vec<(String, String)>,
    pub deferred: Vec<(String, String)>,
    pub rolled_back: Vec<(String, String)>,
    pub sql_flagged: Vec<String>,
    pub needs_attention: Vec<(String, String)>,
    pub forks_synced: Vec<String>,
    pub forks_left_alone: Vec<(String, String)>,
    /// Resources the run never got to because an earlier one failed (nothing changed in them).
    pub not_attempted: Vec<String>,
    /// Automatic updates: versions held back because they were rolled back before.
    pub held: Vec<(String, String)>,
    pub notes: Vec<String>,
}

/// What an update job reports while it runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UpdateEvent {
    Resource {
        id: String,
        state: String,
    },
    Snapshot {
        id: String,
    },
    Health {
        message: String,
    },
    Progress {
        message: String,
    },
    /// One more resource checked (progress bar of the check).
    Checked {
        done: u32,
        total: u32,
        id: String,
    },
    /// The resources this run will update (progress bar of the update).
    Planned {
        ids: Vec<String>,
    },
    Done,
}

/// `GET /servers/{s}/snapshots`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SnapshotsList {
    pub snapshots: Vec<crate::update::manual::SnapshotInfo>,
}

/// `POST /servers/{s}/snapshots/{id}/rollback`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RollbackRequest {
    /// Roll back even resources with uncommitted changes (they are thrown away).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub force: Option<bool>,
}

/// An empty job result (`{}`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Nothing {}

// ---- catalog and adding resources -----------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CatalogRelease {
    pub version: String,
    pub tag: String,
    pub commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CatalogResource {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub title: Option<String>,
    /// One sentence about what it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub description: Option<String>,
    pub repository: String,
    pub path: String,
    pub group: Group,
    pub kind: Kind,
    pub branch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub latest: Option<String>,
    pub versions: Vec<CatalogRelease>,
    pub depends_on: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub creator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub dev_only: Option<bool>,
}

/// `GET /catalog`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Catalog {
    pub schema_version: u32,
    pub resources: Vec<CatalogResource>,
    pub creators: Vec<Creator>,
}

/// `GET /servers/{s}/catalog`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ServerCatalog {
    pub catalog: Catalog,
    pub installed: Vec<String>,
    pub note: Option<String>,
}

/// `POST /servers/{s}/resources/plan` and `POST /servers/{s}/resources`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct IdsRequest {
    pub ids: Vec<String>,
}

/// What an add-resources job reports: a resource is queued, then installed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResourceEvent {
    Resource { id: String, state: String },
}

pub type ResourcesPlan = AddPlan;
pub type ResourcesAdded = AddOutcome;

// ---- notifications, schedule, ports ----------------------------------------------------------

/// `GET /servers/{s}/webhook`: never the address itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct WebhookState {
    pub configured: bool,
    /// Same as `configured` (the core's name for it).
    pub set: bool,
    /// `discord.com/api/webhooks/<id>/***`
    pub redacted: Option<String>,
}

/// `PUT /servers/{s}/webhook`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct WebhookSetRequest {
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ScheduleState {
    pub schedule: Option<ScheduleConfig>,
    /// A sentence about the schedule.
    pub description: Option<String>,
    /// The operating-system job of a saved schedule could be seen from here. false with `schedule.enabled` true means
    /// "saved and on, but not visible" (not the same as off).
    pub job_found: bool,
    /// Plain-language trade-off text for the "run as" choice.
    pub run_as_help: String,
}

// ---- GitHub -----------------------------------------------------------------------------------

/// `GET /github/status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GithubStatus {
    pub signed_in: bool,
    pub user: Option<String>,
    pub state: GithubState,
    pub missing_scopes: Vec<String>,
    /// What to do when GitHub is not ready (null when it is).
    pub problem: Option<Problem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum OwnerKind {
    User,
    Org,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GithubOwner {
    pub login: String,
    pub kind: OwnerKind,
    pub can_fork: bool,
    pub reason: Option<String>,
}

/// `GET /github/owners`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GithubOwners {
    pub owners: Vec<GithubOwner>,
}

/// `POST /github/login` answer: the sign-in job, and the one-time code to type at `url`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GithubLoginStart {
    pub job: String,
    pub code: String,
    pub url: String,
}

/// Result of the sign-in job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GithubLoginResult {
    pub user: Option<String>,
}

/// `POST /servers/{s}/issues`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct IssueRequest {
    pub resource: String,
    pub title: String,
    pub text: String,
}

/// `POST /servers/{s}/pr/plan`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PrPlanRequest {
    pub resource: String,
}

/// `POST /servers/{s}/pr`: the plan is worked out again on the server and must be exactly what was sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PrOpenRequest {
    pub plan: crate::feedback::PrPlan,
    pub title: String,
    pub text: String,
}

/// `{url}` of the issue or pull request that was made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CreatedUrl {
    pub url: String,
}
