//! The list of operations every front end offers (plan section 2.1): one variant per row, a stable
//! name, a route relative to `/api/v1`, whether it is a slow job, and whether [`super::Backend`] can
//! serve it today. The web router is an exhaustive `match` over [`Op`].

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Op {
    Meta,
    SessionCreate,
    SystemInfo,
    ServersList,
    ServerGet,
    ServerControl,
    ServerDelete,
    ServerUnregister,
    ServersScan,
    ServersAdopt,
    ServerOpenFolder,
    LogsList,
    LogsTail,
    ConsoleStream,
    ConsoleSend,
    ConsoleEnable,
    ResourcesList,
    ResourceEnsure,
    /// Start, stop or restart one resource on a running server (through the console helper).
    ResourceControl,
    UpdatePreview,
    UpdateApply,
    UpdateAuto,
    SnapshotsList,
    SnapshotRollback,
    CatalogForServer,
    ResourcesPlan,
    ResourcesAdd,
    WebhookGet,
    WebhookSet,
    WebhookRemove,
    WebhookTest,
    ScheduleGet,
    ScheduleSet,
    ScheduleRemove,
    PortsSet,
    IssueReport,
    PrPlan,
    PrOpen,
    GithubStatus,
    GithubLogin,
    GithubOwners,
    CatalogGet,
    ArtifactsList,
    InstallDefaults,
    InstallSavedClear,
    CheckPorts,
    CheckDisk,
    CheckDbLogin,
    InstallPlan,
    InstallStart,
    /// Carry on with an install that stopped at a failed step: retry it, or skip it when safe.
    InstallResume,
    InstallLog,
    SelfUpdateCheck,
    SelfUpdateApply,
    AppStatus,
    AppInstall,
    AppUninstall,
    UninstallPlan,
    UninstallRun,
    JobsGet,
    JobEvents,
    Events,
    MetricsGet,
    AlertsList,
    AlertsAck,
    AlertSettingsGet,
    AlertSettingsSet,
    NodesList,
    NodeAdd,
    NodeRemove,
    NodeServers,
    NodeServerControl,
    Fleet,
    RemoteGet,
    RemoteSet,
    UsersList,
    UserAdd,
    UserRemove,
    SessionLogout,
    SessionLink,
    /// Restart the background panel (after a self-update). Local admin only.
    PanelRestart,
    CheckCfx,
    AssetsList,
    AssetInspect,
    AssetImport,
    AssetRemove,
    AssetDownloads,
    /// Git and the GitHub CLI: found, version, Kora's own copy or the system's.
    ToolsStatus,
    /// Install Kora's own copy of git or gh (a job with progress).
    ToolInstall,
    /// The System check: git, gh, GitHub sign-in, database, administrator rights.
    SystemCheck,
    /// Start the installed database server (a job; may need administrator rights).
    DbStart,
    /// The saved licence keys: names and last four characters only.
    KeysList,
    /// Save a licence key under a name (admins).
    KeyAdd,
    /// Rename a saved key, or put a new key under its name (admins).
    KeyUpdate,
    /// Forget a saved key (admins). Servers installed with it are not touched.
    KeyRemove,
    /// Change the licence key of an installed server: a saved key by name, or a new one.
    ServerKeySet,
    /// What Kora remembers: the Cfx.re name, its identifier and the GitHub owner.
    ProfileGet,
    /// Change or forget those remembered answers.
    ProfileSet,
    /// Development: which branch each resource is on.
    BranchesList,
    /// Development: the branches (and kept-aside changes) of one resource.
    BranchChoices,
    /// Development: open pull requests of the original repository and the person's fork, for one resource.
    PrList,
    /// Development: change one resource's branch, try a pull request, or go back to the normal version.
    BranchSwitch,
    /// Development: bring kept-aside changes back.
    StashRestore,
    /// Development: which resources have a branch (to switch them all).
    BranchBulkPreview,
    /// Development: switch every resource that has a branch, or all back to normal.
    BranchBulkApply,
}

impl Op {
    pub const ALL: &'static [Op] = &[
        Op::Meta,
        Op::SessionCreate,
        Op::SystemInfo,
        Op::ServersList,
        Op::ServerGet,
        Op::ServerControl,
        Op::ServerDelete,
        Op::ServerUnregister,
        Op::ServersScan,
        Op::ServersAdopt,
        Op::ServerOpenFolder,
        Op::LogsList,
        Op::LogsTail,
        Op::ConsoleStream,
        Op::ConsoleSend,
        Op::ConsoleEnable,
        Op::ResourcesList,
        Op::ResourceEnsure,
        Op::ResourceControl,
        Op::UpdatePreview,
        Op::UpdateApply,
        Op::UpdateAuto,
        Op::SnapshotsList,
        Op::SnapshotRollback,
        Op::CatalogForServer,
        Op::ResourcesPlan,
        Op::ResourcesAdd,
        Op::WebhookGet,
        Op::WebhookSet,
        Op::WebhookRemove,
        Op::WebhookTest,
        Op::ScheduleGet,
        Op::ScheduleSet,
        Op::ScheduleRemove,
        Op::PortsSet,
        Op::IssueReport,
        Op::PrPlan,
        Op::PrOpen,
        Op::GithubStatus,
        Op::GithubLogin,
        Op::GithubOwners,
        Op::CatalogGet,
        Op::ArtifactsList,
        Op::InstallDefaults,
        Op::InstallSavedClear,
        Op::CheckPorts,
        Op::CheckDisk,
        Op::CheckDbLogin,
        Op::InstallPlan,
        Op::InstallStart,
        Op::InstallResume,
        Op::InstallLog,
        Op::SelfUpdateCheck,
        Op::SelfUpdateApply,
        Op::AppStatus,
        Op::AppInstall,
        Op::AppUninstall,
        Op::UninstallPlan,
        Op::UninstallRun,
        Op::JobsGet,
        Op::JobEvents,
        Op::Events,
        Op::MetricsGet,
        Op::AlertsList,
        Op::AlertsAck,
        Op::AlertSettingsGet,
        Op::AlertSettingsSet,
        Op::NodesList,
        Op::NodeAdd,
        Op::NodeRemove,
        Op::NodeServers,
        Op::NodeServerControl,
        Op::Fleet,
        Op::RemoteGet,
        Op::RemoteSet,
        Op::UsersList,
        Op::UserAdd,
        Op::UserRemove,
        Op::SessionLogout,
        Op::SessionLink,
        Op::PanelRestart,
        Op::CheckCfx,
        Op::AssetsList,
        Op::AssetInspect,
        Op::AssetImport,
        Op::AssetRemove,
        Op::AssetDownloads,
        Op::ToolsStatus,
        Op::ToolInstall,
        Op::SystemCheck,
        Op::DbStart,
        Op::KeysList,
        Op::KeyAdd,
        Op::KeyUpdate,
        Op::KeyRemove,
        Op::ServerKeySet,
        Op::ProfileGet,
        Op::ProfileSet,
        Op::BranchesList,
        Op::BranchChoices,
        Op::PrList,
        Op::BranchSwitch,
        Op::StashRestore,
        Op::BranchBulkPreview,
        Op::BranchBulkApply,
    ];

    /// Stable snake_case name (used in `/meta` capabilities and job records).
    pub fn name(self) -> &'static str {
        match self {
            Op::Meta => "meta",
            Op::SessionCreate => "session_create",
            Op::SystemInfo => "system_info",
            Op::ServersList => "servers_list",
            Op::ServerGet => "server_get",
            Op::ServerControl => "server_control",
            Op::ServerDelete => "server_delete",
            Op::ServerUnregister => "server_unregister",
            Op::ServersScan => "servers_scan",
            Op::ServersAdopt => "servers_adopt",
            Op::ServerOpenFolder => "server_open_folder",
            Op::LogsList => "logs_list",
            Op::LogsTail => "logs_tail",
            Op::ConsoleStream => "console_stream",
            Op::ConsoleSend => "console_send",
            Op::ConsoleEnable => "console_enable",
            Op::ResourcesList => "resources_list",
            Op::ResourceEnsure => "resource_ensure",
            Op::ResourceControl => "resource_control",
            Op::UpdatePreview => "update_preview",
            Op::UpdateApply => "update_apply",
            Op::UpdateAuto => "update_auto",
            Op::SnapshotsList => "snapshots_list",
            Op::SnapshotRollback => "snapshot_rollback",
            Op::CatalogForServer => "catalog_for_server",
            Op::ResourcesPlan => "resources_plan",
            Op::ResourcesAdd => "resources_add",
            Op::WebhookGet => "webhook_get",
            Op::WebhookSet => "webhook_set",
            Op::WebhookRemove => "webhook_remove",
            Op::WebhookTest => "webhook_test",
            Op::ScheduleGet => "schedule_get",
            Op::ScheduleSet => "schedule_set",
            Op::ScheduleRemove => "schedule_remove",
            Op::PortsSet => "ports_set",
            Op::IssueReport => "issue_report",
            Op::PrPlan => "pr_plan",
            Op::PrOpen => "pr_open",
            Op::GithubStatus => "github_status",
            Op::GithubLogin => "github_login",
            Op::GithubOwners => "github_owners",
            Op::CatalogGet => "catalog_get",
            Op::ArtifactsList => "artifacts_list",
            Op::InstallDefaults => "install_defaults",
            Op::InstallSavedClear => "install_saved_clear",
            Op::CheckPorts => "check_ports",
            Op::CheckDisk => "check_disk",
            Op::CheckDbLogin => "check_db_login",
            Op::InstallPlan => "install_plan",
            Op::InstallStart => "install_start",
            Op::InstallResume => "install_resume",
            Op::InstallLog => "install_log",
            Op::SelfUpdateCheck => "self_update_check",
            Op::SelfUpdateApply => "self_update_apply",
            Op::AppStatus => "app_status",
            Op::AppInstall => "app_install",
            Op::AppUninstall => "app_uninstall",
            Op::UninstallPlan => "uninstall_plan",
            Op::UninstallRun => "uninstall_run",
            Op::JobsGet => "jobs_get",
            Op::JobEvents => "job_events",
            Op::Events => "events",
            Op::MetricsGet => "metrics_get",
            Op::AlertsList => "alerts_list",
            Op::AlertsAck => "alerts_ack",
            Op::AlertSettingsGet => "alert_settings_get",
            Op::AlertSettingsSet => "alert_settings_set",
            Op::NodesList => "nodes_list",
            Op::NodeAdd => "node_add",
            Op::NodeRemove => "node_remove",
            Op::NodeServers => "node_servers",
            Op::NodeServerControl => "node_server_control",
            Op::Fleet => "fleet",
            Op::RemoteGet => "remote_get",
            Op::RemoteSet => "remote_set",
            Op::UsersList => "users_list",
            Op::UserAdd => "user_add",
            Op::UserRemove => "user_remove",
            Op::SessionLogout => "session_logout",
            Op::SessionLink => "session_link",
            Op::PanelRestart => "panel_restart",
            Op::CheckCfx => "check_cfx",
            Op::AssetsList => "assets_list",
            Op::AssetInspect => "asset_inspect",
            Op::AssetImport => "asset_import",
            Op::AssetRemove => "asset_remove",
            Op::AssetDownloads => "asset_downloads",
            Op::ToolsStatus => "tools_status",
            Op::ToolInstall => "tool_install",
            Op::SystemCheck => "system_check",
            Op::DbStart => "db_start",
            Op::KeysList => "keys_list",
            Op::KeyAdd => "key_add",
            Op::KeyUpdate => "key_update",
            Op::KeyRemove => "key_remove",
            Op::ServerKeySet => "server_key_set",
            Op::ProfileGet => "profile_get",
            Op::ProfileSet => "profile_set",
            Op::BranchesList => "branches_list",
            Op::BranchChoices => "branch_choices",
            Op::PrList => "pr_list",
            Op::BranchSwitch => "branch_switch",
            Op::StashRestore => "stash_restore",
            Op::BranchBulkPreview => "branch_bulk_preview",
            Op::BranchBulkApply => "branch_bulk_apply",
        }
    }

    /// Method and path relative to `/api/v1`; `{s}` is the server name, `{id}`/`{file}`/`{job}` as named.
    pub fn route(self) -> (Method, &'static str) {
        match self {
            Op::Meta => (Method::Get, "/meta"),
            Op::SessionCreate => (Method::Post, "/session"),
            Op::SystemInfo => (Method::Get, "/system"),
            Op::ServersList => (Method::Get, "/servers"),
            Op::ServerGet => (Method::Get, "/servers/{s}"),
            Op::ServerControl => (Method::Post, "/servers/{s}/control"),
            Op::ServerDelete => (Method::Delete, "/servers/{s}"),
            Op::ServerUnregister => (Method::Delete, "/servers/{s}/registration"),
            Op::ServersScan => (Method::Post, "/servers/scan"),
            Op::ServersAdopt => (Method::Post, "/servers/adopt"),
            Op::ServerOpenFolder => (Method::Post, "/servers/{s}/open-folder"),
            Op::LogsList => (Method::Get, "/servers/{s}/logs"),
            Op::LogsTail => (Method::Get, "/servers/{s}/logs/{file}"),
            Op::ConsoleStream => (Method::Get, "/servers/{s}/console/stream"),
            Op::ConsoleSend => (Method::Post, "/servers/{s}/console"),
            Op::ConsoleEnable => (Method::Put, "/servers/{s}/console/enabled"),
            Op::ResourcesList => (Method::Get, "/servers/{s}/resources"),
            Op::ResourceEnsure => (Method::Post, "/servers/{s}/ensure"),
            Op::ResourceControl => (Method::Post, "/servers/{s}/resources/{name}/control"),
            Op::UpdatePreview => (Method::Post, "/servers/{s}/update/preview"),
            Op::UpdateApply => (Method::Post, "/servers/{s}/update/apply"),
            Op::UpdateAuto => (Method::Post, "/servers/{s}/update/auto"),
            Op::SnapshotsList => (Method::Get, "/servers/{s}/snapshots"),
            Op::SnapshotRollback => (Method::Post, "/servers/{s}/snapshots/{id}/rollback"),
            Op::CatalogForServer => (Method::Get, "/servers/{s}/catalog"),
            Op::ResourcesPlan => (Method::Post, "/servers/{s}/resources/plan"),
            Op::ResourcesAdd => (Method::Post, "/servers/{s}/resources"),
            Op::WebhookGet => (Method::Get, "/servers/{s}/webhook"),
            Op::WebhookSet => (Method::Put, "/servers/{s}/webhook"),
            Op::WebhookRemove => (Method::Delete, "/servers/{s}/webhook"),
            Op::WebhookTest => (Method::Post, "/servers/{s}/webhook/test"),
            Op::ScheduleGet => (Method::Get, "/servers/{s}/schedule"),
            Op::ScheduleSet => (Method::Put, "/servers/{s}/schedule"),
            Op::ScheduleRemove => (Method::Delete, "/servers/{s}/schedule"),
            Op::PortsSet => (Method::Put, "/servers/{s}/ports"),
            Op::IssueReport => (Method::Post, "/servers/{s}/issues"),
            Op::PrPlan => (Method::Post, "/servers/{s}/pr/plan"),
            Op::PrOpen => (Method::Post, "/servers/{s}/pr"),
            Op::GithubStatus => (Method::Get, "/github/status"),
            Op::GithubLogin => (Method::Post, "/github/login"),
            Op::GithubOwners => (Method::Get, "/github/owners"),
            Op::CatalogGet => (Method::Get, "/catalog"),
            Op::ArtifactsList => (Method::Get, "/artifacts"),
            Op::InstallDefaults => (Method::Get, "/installs/defaults"),
            Op::InstallSavedClear => (Method::Delete, "/installs/saved"),
            Op::CheckPorts => (Method::Post, "/checks/ports"),
            Op::CheckDisk => (Method::Post, "/checks/disk"),
            Op::CheckDbLogin => (Method::Post, "/checks/db-login"),
            Op::InstallPlan => (Method::Post, "/installs/plan"),
            Op::InstallStart => (Method::Post, "/installs"),
            Op::InstallResume => (Method::Post, "/installs/{job}/resume"),
            Op::InstallLog => (Method::Get, "/installs/{job}/log"),
            Op::SelfUpdateCheck => (Method::Get, "/self-update"),
            Op::SelfUpdateApply => (Method::Post, "/self-update"),
            Op::AppStatus => (Method::Get, "/app"),
            Op::AppInstall => (Method::Put, "/app"),
            Op::AppUninstall => (Method::Delete, "/app"),
            Op::UninstallPlan => (Method::Post, "/uninstall/plan"),
            Op::UninstallRun => (Method::Post, "/uninstall"),
            Op::JobsGet => (Method::Get, "/jobs/{id}"),
            Op::JobEvents => (Method::Get, "/jobs/{id}/events"),
            Op::Events => (Method::Get, "/events"),
            Op::MetricsGet => (Method::Get, "/servers/{s}/metrics"),
            Op::AlertsList => (Method::Get, "/alerts"),
            Op::AlertsAck => (Method::Post, "/alerts/{id}/ack"),
            Op::AlertSettingsGet => (Method::Get, "/servers/{s}/alerts"),
            Op::AlertSettingsSet => (Method::Put, "/servers/{s}/alerts"),
            Op::NodesList => (Method::Get, "/nodes"),
            Op::NodeAdd => (Method::Post, "/nodes"),
            Op::NodeRemove => (Method::Delete, "/nodes/{id}"),
            Op::NodeServers => (Method::Get, "/nodes/{id}/servers"),
            Op::NodeServerControl => (Method::Post, "/nodes/{id}/servers/{s}/control"),
            Op::Fleet => (Method::Get, "/fleet"),
            Op::RemoteGet => (Method::Get, "/remote"),
            Op::RemoteSet => (Method::Put, "/remote"),
            Op::UsersList => (Method::Get, "/users"),
            Op::UserAdd => (Method::Post, "/users"),
            Op::UserRemove => (Method::Delete, "/users/{name}"),
            Op::SessionLogout => (Method::Post, "/session/logout"),
            // A one-time sign-in code for a link (local callers only; the CLI prints the link).
            Op::SessionLink => (Method::Post, "/session/link"),
            // Restart the background panel so a self-update takes effect (local callers only).
            Op::PanelRestart => (Method::Post, "/panel/restart"),
            Op::CheckCfx => (Method::Post, "/checks/cfx"),
            Op::AssetsList => (Method::Get, "/servers/{s}/assets"),
            Op::AssetInspect => (Method::Post, "/servers/{s}/assets/inspect"),
            Op::AssetImport => (Method::Post, "/servers/{s}/assets"),
            Op::AssetRemove => (Method::Delete, "/servers/{s}/assets/{name}"),
            Op::AssetDownloads => (Method::Get, "/assets/downloads"),
            Op::ToolsStatus => (Method::Get, "/tools"),
            Op::ToolInstall => (Method::Post, "/tools/{tool}/install"),
            Op::SystemCheck => (Method::Get, "/system/check"),
            Op::DbStart => (Method::Post, "/system/database/start"),
            Op::KeysList => (Method::Get, "/keys"),
            Op::KeyAdd => (Method::Post, "/keys"),
            Op::KeyUpdate => (Method::Put, "/keys/{name}"),
            Op::KeyRemove => (Method::Delete, "/keys/{name}"),
            Op::ServerKeySet => (Method::Put, "/servers/{s}/licence-key"),
            Op::ProfileGet => (Method::Get, "/profile"),
            Op::ProfileSet => (Method::Put, "/profile"),
            Op::BranchesList => (Method::Get, "/servers/{s}/branches"),
            Op::BranchChoices => (Method::Get, "/servers/{s}/resources/{name}/branches"),
            Op::PrList => (Method::Get, "/servers/{s}/resources/{name}/prs"),
            Op::BranchSwitch => (Method::Post, "/servers/{s}/resources/{name}/branch"),
            Op::StashRestore => (Method::Post, "/servers/{s}/resources/{name}/stash/restore"),
            Op::BranchBulkPreview => (Method::Post, "/servers/{s}/branches/preview"),
            Op::BranchBulkApply => (Method::Post, "/servers/{s}/branches/apply"),
        }
    }

    /// A slow operation: answers `202 {"job": id}` and reports through a job.
    pub fn is_job(self) -> bool {
        matches!(
            self,
            Op::ServerControl
                | Op::ServerDelete
                | Op::ServersScan
                | Op::UpdatePreview
                | Op::UpdateApply
                | Op::UpdateAuto
                | Op::SnapshotRollback
                | Op::ResourcesAdd
                | Op::GithubLogin
                | Op::InstallStart
                | Op::InstallResume
                | Op::SelfUpdateApply
                | Op::UninstallRun
                | Op::NodeServerControl
                | Op::AssetImport
                | Op::ToolInstall
                | Op::DbStart
                | Op::BranchBulkPreview
                | Op::BranchBulkApply
        )
    }

    /// Jobs that only read: they may run next to a mutating job of the same server.
    pub fn is_read_only_job(self) -> bool {
        matches!(self, Op::UpdatePreview | Op::ServersScan | Op::BranchBulkPreview)
    }

    /// True when [`super::Backend`] (or the api helpers) can serve it today.
    pub fn implemented(self) -> bool {
        matches!(
            self,
            Op::ServersList
                | Op::ServerGet
                | Op::ServerControl
                | Op::ServerDelete
                | Op::ServerUnregister
                | Op::ServersScan
                | Op::ServersAdopt
                | Op::LogsList
                | Op::LogsTail
                | Op::ConsoleSend
                | Op::ConsoleEnable
                | Op::ResourcesList
                | Op::ResourceEnsure
                | Op::ResourceControl
                | Op::UpdatePreview
                | Op::UpdateApply
                | Op::SnapshotsList
                | Op::SnapshotRollback
                | Op::CatalogForServer
                | Op::ResourcesPlan
                | Op::ResourcesAdd
                | Op::WebhookGet
                | Op::WebhookSet
                | Op::WebhookRemove
                | Op::WebhookTest
                | Op::ScheduleGet
                | Op::ScheduleSet
                | Op::ScheduleRemove
                | Op::PortsSet
                | Op::IssueReport
                | Op::PrPlan
                | Op::PrOpen
                | Op::GithubStatus
                | Op::NodesList
                | Op::NodeAdd
                | Op::NodeRemove
                | Op::NodeServers
                | Op::NodeServerControl
                | Op::Fleet
                | Op::AssetsList
                | Op::AssetInspect
                | Op::AssetImport
                | Op::AssetRemove
                | Op::AssetDownloads
                | Op::BranchesList
                | Op::BranchChoices
                | Op::PrList
                | Op::BranchSwitch
                | Op::StashRestore
                | Op::BranchBulkPreview
                | Op::BranchBulkApply
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Exhaustive: adding a variant without a line here does not compile.
    fn listed(op: Op) -> bool {
        match op {
            Op::Meta
            | Op::SessionCreate
            | Op::SystemInfo
            | Op::ServersList
            | Op::ServerGet
            | Op::ServerControl
            | Op::ServerDelete
            | Op::ServerUnregister
            | Op::ServersScan
            | Op::ServersAdopt
            | Op::ServerOpenFolder
            | Op::LogsList
            | Op::LogsTail
            | Op::ConsoleStream
            | Op::ConsoleSend
            | Op::ConsoleEnable
            | Op::ResourcesList
            | Op::ResourceEnsure
            | Op::ResourceControl
            | Op::UpdatePreview
            | Op::UpdateApply
            | Op::UpdateAuto
            | Op::SnapshotsList
            | Op::SnapshotRollback
            | Op::CatalogForServer
            | Op::ResourcesPlan
            | Op::ResourcesAdd
            | Op::WebhookGet
            | Op::WebhookSet
            | Op::WebhookRemove
            | Op::WebhookTest
            | Op::ScheduleGet
            | Op::ScheduleSet
            | Op::ScheduleRemove
            | Op::PortsSet
            | Op::IssueReport
            | Op::PrPlan
            | Op::PrOpen
            | Op::GithubStatus
            | Op::GithubLogin
            | Op::GithubOwners
            | Op::CatalogGet
            | Op::ArtifactsList
            | Op::InstallDefaults
            | Op::InstallSavedClear
            | Op::CheckPorts
            | Op::CheckDisk
            | Op::CheckDbLogin
            | Op::InstallPlan
            | Op::InstallStart
            | Op::InstallResume
            | Op::InstallLog
            | Op::SelfUpdateCheck
            | Op::SelfUpdateApply
            | Op::AppStatus
            | Op::AppInstall
            | Op::AppUninstall
            | Op::UninstallPlan
            | Op::UninstallRun
            | Op::JobsGet
            | Op::JobEvents
            | Op::Events
            | Op::MetricsGet
            | Op::AlertsList
            | Op::AlertsAck
            | Op::AlertSettingsGet
            | Op::AlertSettingsSet
            | Op::NodesList
            | Op::NodeAdd
            | Op::NodeRemove
            | Op::NodeServers
            | Op::NodeServerControl
            | Op::Fleet
            | Op::RemoteGet
            | Op::RemoteSet
            | Op::UsersList
            | Op::UserAdd
            | Op::UserRemove
            | Op::SessionLogout
            | Op::SessionLink
            | Op::PanelRestart
            | Op::CheckCfx
            | Op::AssetsList
            | Op::AssetInspect
            | Op::AssetImport
            | Op::AssetRemove
            | Op::AssetDownloads
            | Op::ToolsStatus
            | Op::ToolInstall
            | Op::SystemCheck
            | Op::DbStart
            | Op::KeysList
            | Op::KeyAdd
            | Op::KeyUpdate
            | Op::KeyRemove
            | Op::ServerKeySet
            | Op::ProfileGet
            | Op::ProfileSet
            | Op::BranchesList
            | Op::BranchChoices
            | Op::PrList
            | Op::BranchSwitch
            | Op::StashRestore
            | Op::BranchBulkPreview
            | Op::BranchBulkApply
            => Op::ALL.contains(&op),
        }
    }

    #[test]
    fn all_is_complete() {
        assert_eq!(Op::ALL.len(), 105);
        for op in Op::ALL {
            assert!(listed(*op), "{op:?} missing from ALL");
        }
        let uniq: HashSet<_> = Op::ALL.iter().collect();
        assert_eq!(uniq.len(), Op::ALL.len(), "duplicate in ALL");
    }

    #[test]
    fn names_are_unique_snake_case() {
        let mut seen = HashSet::new();
        for op in Op::ALL {
            let n = op.name();
            assert!(n.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'), "{n}");
            assert!(seen.insert(n), "duplicate name {n}");
        }
    }

    #[test]
    fn routes_are_unique_and_relative() {
        let mut seen = HashSet::new();
        for op in Op::ALL {
            let (m, p) = op.route();
            assert!(p.starts_with('/'), "{p}");
            assert!(seen.insert((m, p)), "duplicate route {} {p}", m.as_str());
        }
    }

    #[test]
    fn jobs_and_read_only_jobs() {
        assert!(Op::ServerControl.is_job() && Op::InstallStart.is_job());
        assert!(!Op::ServersList.is_job());
        for op in Op::ALL {
            if op.is_read_only_job() {
                assert!(op.is_job(), "{op:?}");
            }
        }
    }

    #[test]
    fn implemented_ops_are_a_subset() {
        assert!(Op::ServerControl.implemented());
        assert!(!Op::Meta.implemented());
    }
}
