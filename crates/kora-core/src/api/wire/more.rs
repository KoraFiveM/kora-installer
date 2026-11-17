//! Paid assets, monitoring and alerts, the fleet (hub), tools and the System check, maintenance, branches.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::servers::ServerSummary;
use crate::branches::ResourceBranch;
use crate::git::branches::{Changes as ChangesChoice, PrSource};
use crate::monitor::alerts::AlertKind;
use crate::monitor::Sample;
use crate::paidassets::hints::EntitlementHint;
use crate::paidassets::{DownloadZip, ImportPlan as AssetPlan, PaidAsset};
use crate::problem::{Problem, Tool};
use crate::service::ControlAction;

// ---- paid assets ------------------------------------------------------------------------------

/// `GET /servers/{s}/assets`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PaidAssets {
    pub assets: Vec<PaidAsset>,
    pub hints: Vec<EntitlementHint>,
    pub portal_url: String,
}

/// `POST /servers/{s}/assets/inspect` with a JSON body: a zip from the Downloads folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AssetInspectRequest {
    pub download: String,
}

/// What importing a zip would do. `upload`: the id of an uploaded zip kept for the import (null when
/// refused); `download`: the name of the zip from the Downloads folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AssetInspectResult {
    pub plan: AssetPlan,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    pub upload: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub download: Option<String>,
}

/// `POST /servers/{s}/assets`: import an inspected zip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AssetImportRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub upload: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub download: Option<String>,
    pub replace: bool,
}

/// What an import job reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AssetImportEvent {
    Status { message: String },
    Asset { name: String, state: String },
}

/// `GET /assets/downloads`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RecentDownloads {
    pub found: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub folder: Option<PathBuf>,
    pub zips: Vec<DownloadZip>,
}

// ---- monitoring and alerts --------------------------------------------------------------------

/// `?range=` of `GET /servers/{s}/metrics`: `1h`, `6h` or `24h` (default `1h`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MetricsQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub range: Option<String>,
}

/// `GET /servers/{s}/metrics`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Metrics {
    pub samples: Vec<Sample>,
    pub latest: Option<Sample>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum AlertTone {
    Danger,
    Warning,
    Success,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PanelAlert {
    pub id: u64,
    pub server: String,
    pub kind: AlertKind,
    pub tone: AlertTone,
    pub title: String,
    pub message: String,
    pub raised: u64,
    /// When the problem went away; null while it is still there.
    pub resolved: Option<u64>,
    pub acked: bool,
}

/// `GET /alerts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AlertsList {
    pub alerts: Vec<PanelAlert>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AlertRules {
    pub stopped: bool,
    pub recovered: bool,
    pub disk: bool,
    pub ram: bool,
    pub updates: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AlertWebhook {
    pub configured: bool,
    pub redacted: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AlertEmail {
    pub supported: bool,
}

/// `GET /servers/{s}/alerts`: the address of the generic webhook is never returned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AlertSettings {
    pub enabled: bool,
    pub rules: AlertRules,
    pub disk_min_gb: f64,
    pub disk_min_percent: f64,
    pub ram_max_percent: f64,
    /// Also post to the server's Discord webhook (Notifications tab).
    pub discord: bool,
    pub webhook: AlertWebhook,
    pub email: AlertEmail,
}

/// Any of the rules; what is left out stays as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AlertRulesUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub stopped: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub recovered: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub disk: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub ram: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub updates: Option<bool>,
}

/// `PUT /servers/{s}/alerts`: what is left out stays as it is.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AlertSettingsUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub rules: Option<AlertRulesUpdate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub disk_min_gb: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub disk_min_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub ram_max_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub discord: Option<bool>,
    /// Omit to keep the address; `""` removes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub webhook_url: Option<String>,
}

// ---- fleet (hub) ------------------------------------------------------------------------------

/// `POST /nodes`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct NodeAddRequest {
    pub name: String,
    pub url: String,
    pub token: String,
}

/// Another panel this one can see.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct HubNode {
    pub id: String,
    pub name: String,
    pub url: String,
    /// The token is never returned.
    pub token_set: bool,
    /// Plain http:// to a non-private, non-Tailscale address.
    pub insecure: bool,
    pub warning: Option<String>,
    pub reachable: bool,
    pub version: Option<String>,
    pub api: Option<u32>,
    pub platform: Option<String>,
    pub ops: Vec<String>,
    pub server_count: Option<usize>,
    pub can_control: bool,
    pub last_error: Option<String>,
}

/// `GET /nodes`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct NodesList {
    pub nodes: Vec<HubNode>,
}

/// `POST /nodes` answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct NodeAdded {
    pub node: HubNode,
}

/// A server in the combined list: `node_id` is null for this panel's own servers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FleetServer {
    #[serde(flatten)]
    pub server: ServerSummary,
    pub node_id: Option<String>,
    pub node_name: String,
    /// Only what the node supports; empty when it is too old to be controlled from here.
    pub actions: Vec<ControlAction>,
}

/// `GET /nodes/{id}/servers`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct NodeServers {
    pub servers: Vec<FleetServer>,
}

/// `GET /fleet`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Fleet {
    pub servers: Vec<FleetServer>,
    pub nodes: Vec<HubNode>,
    /// True when some node could not be reached.
    pub partial: bool,
}

// ---- tools and the System check ---------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ToolSource {
    /// Already on this computer.
    System,
    /// Kora's own copy.
    Private,
}

/// One tool of `GET /tools`: Git or the GitHub CLI, from this computer or Kora's own private copy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ToolStatus {
    pub tool: Tool,
    pub name: String,
    pub label: String,
    pub installed: bool,
    pub version: Option<String>,
    pub source: Option<ToolSource>,
    pub path: Option<String>,
    /// Found, but it did not run.
    pub broken: bool,
    /// Kora can download its own copy on this computer.
    pub can_install: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ToolsList {
    pub tools: Vec<ToolStatus>,
}

/// What an install-tool job reports (`message`, `fraction` 0..1 for a download).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolInstallEvent {
    Status {
        message: String,
    },
    Progress {
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        fraction: Option<f32>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum CheckState {
    Ok,
    Problem,
    Info,
}

/// One line of the System check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CheckItem {
    /// `git`, `gh`, `github`, `database`, `admin` or `service`.
    pub id: String,
    pub label: String,
    pub state: CheckState,
    pub summary: String,
    /// What to do about it, with the fix buttons (null when fine).
    pub problem: Option<Problem>,
}

/// `GET /system/check`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SystemCheck {
    pub items: Vec<CheckItem>,
    /// No line needs fixing.
    pub ok: bool,
}

// ---- maintenance ------------------------------------------------------------------------------

/// `GET /self-update`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SelfUpdateInfo {
    pub current: String,
    pub latest: Option<String>,
    pub available: bool,
    /// Same as `available` (the older name).
    pub update_available: bool,
    /// Published versions, newest first.
    pub versions: Vec<String>,
}

/// `POST /self-update`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SelfUpdateRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub version: Option<String>,
}

/// Result of the self-update job. `output` is what the administrator helper printed, when it ran the update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SelfUpdateResult {
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub output: Option<Value>,
}

/// `GET /app`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AppStatus {
    /// The OLD machine-wide Kora app (Program Files, /opt/kora) is installed.
    pub installed: bool,
    pub version: Option<String>,
    pub path: Option<String>,
    pub desktop_shortcut: bool,
    pub supported: bool,
    /// The old app, or an old administrator shortcut, is still here: offer "Tidy up old shortcuts".
    pub legacy: bool,
    /// What tidying up would remove, one plain sentence each.
    pub legacy_items: Vec<String>,
    /// The per-user shortcut that opens the panel is in place.
    pub shortcuts: bool,
}

/// `PUT /app`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AppInstallRequest {
    pub desktop_shortcut: bool,
    pub dry_run: bool,
}

/// `DELETE /app`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AppUninstallRequest {
    pub dry_run: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum UninstallItemKind {
    System,
    Folder,
    Database,
    App,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct UninstallItem {
    /// `<server>/service|firewall|timer|account|program|registry|folder|database`, or `app`.
    pub item: String,
    pub description: String,
    pub kind: UninstallItemKind,
    pub remove: bool,
}

/// `POST /uninstall/plan`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct UninstallPlan {
    pub items: Vec<UninstallItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct UninstallAnswer {
    pub item: String,
    pub remove: bool,
}

/// `POST /uninstall`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct UninstallRequest {
    pub answers: Vec<UninstallAnswer>,
    pub delete_files: bool,
    pub delete_databases: bool,
    /// Only these servers (empty: everything).
    pub only: Vec<String>,
}

/// Result of the uninstall job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct UninstallResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub removed: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub kept: Option<Vec<String>>,
    pub notes: Vec<String>,
    /// What the administrator helper printed, when it ran the removal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub output: Option<Value>,
}

// ---- branches ---------------------------------------------------------------------------------

/// `GET /servers/{s}/branches`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BranchesList {
    pub resources: Vec<ResourceBranch>,
}

/// `?refresh=1` of `GET /servers/{s}/resources/{name}/branches`: download the newest lists from GitHub first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BranchChoicesQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub refresh: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Remote {
    Origin,
    Upstream,
}

/// `POST /servers/{s}/resources/{name}/branch`: another branch, a pull request, or back to the normal version.
/// `changes`: what to do with changes of the person's own that are in the way (`ask` = do nothing, say which files).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SwitchRequest {
    Default {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        changes: Option<ChangesChoice>,
    },
    Branch {
        branch: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        remote: Option<Remote>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        changes: Option<ChangesChoice>,
    },
    Pr {
        /// A pull request number or its address.
        pr: PrRef,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        source: Option<PrSource>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        changes: Option<ChangesChoice>,
    },
}

/// A pull request: its number or its address (`https://github.com/owner/repo/pull/12`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(untagged)]
pub enum PrRef {
    Number(u32),
    Text(String),
}

/// `POST /servers/{s}/resources/{name}/stash/restore`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct StashRestoreRequest {
    pub index: u32,
}

/// `POST /servers/{s}/branches/preview`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BulkPreviewRequest {
    pub branch: String,
    /// Download the newest branch lists first (default true).
    pub fetch: bool,
}

/// `POST /servers/{s}/branches/apply`: every resource that has the branch, or all back to the normal version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum BulkApplyRequest {
    Branch {
        branch: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        resources: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        changes: Option<ChangesChoice>,
    },
    Default {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        resources: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        changes: Option<ChangesChoice>,
    },
}

/// What a bulk-preview job reports: one more resource looked at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BulkEvent {
    Checked { done: usize, total: usize, id: String },
    Switched { done: usize, total: usize, id: String },
}

// ---- jobs without events, and small results ----------------------------------------------------

/// A job that reports no events (an empty type: there is never a value of it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum NoEvents {}

/// Result of an install-tool job: Kora's own copy (`version`, `path`, `source`), or the system package run by
/// the administrator helper (`output`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ToolInstallResult {
    pub tool: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub source: Option<ToolSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub output: Option<Value>,
}

/// Result of the start-the-database job: a plain sentence, or what the administrator helper printed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DbStartResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub output: Option<Value>,
}
