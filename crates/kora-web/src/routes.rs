//! The Op → handler table. The `match` has no wildcard arm, so adding an [`Op`] without a handler
//! does not compile; the HTTP method and path always come from [`Op::route`].
use axum::handler::Handler;
use axum::routing::{on, MethodFilter, MethodRouter};
use axum::Router;
use kora_core::api::{Method, Op};

use crate::{handlers, AppState};

fn on_op<H, T>(op: Op, h: H) -> MethodRouter<AppState>
where
    H: Handler<T, AppState>,
    T: 'static,
{
    let filter = match op.route().0 {
        Method::Get => MethodFilter::GET,
        Method::Post => MethodFilter::POST,
        Method::Put => MethodFilter::PUT,
        Method::Delete => MethodFilter::DELETE,
    };
    on(filter, h)
}

/// The handler for `op`, bound to `op`'s HTTP method.
pub fn handler(op: Op) -> MethodRouter<AppState> {
    match op {
        Op::Meta => on_op(op, crate::meta),
        Op::SessionCreate => on_op(op, crate::session_create),
        Op::SystemInfo => on_op(op, handlers::system::system_info),
        Op::ServersList => on_op(op, handlers::servers::servers_list),
        Op::ServerGet => on_op(op, handlers::servers::server_get),
        Op::ServerControl => on_op(op, handlers::servers::server_control),
        Op::ServerDelete => on_op(op, handlers::servers::server_delete),
        Op::ServerUnregister => on_op(op, handlers::servers::server_unregister),
        Op::ServersScan => on_op(op, handlers::servers::servers_scan),
        Op::ServersAdopt => on_op(op, handlers::servers::servers_adopt),
        Op::ServerOpenFolder => on_op(op, handlers::servers::server_open_folder),
        Op::LogsList => on_op(op, handlers::servers::logs_list),
        Op::LogsTail => on_op(op, handlers::servers::logs_tail),
        Op::ConsoleStream => on_op(op, handlers::console::console_stream),
        Op::ConsoleSend => on_op(op, handlers::console::console_send),
        Op::ConsoleEnable => on_op(op, handlers::console::console_enable),
        Op::ResourcesList => on_op(op, handlers::resources::resources_list),
        Op::ResourceEnsure => on_op(op, handlers::resources::resource_ensure),
        Op::UpdatePreview => on_op(op, handlers::updates::update_preview),
        Op::UpdateApply => on_op(op, handlers::updates::update_apply),
        Op::UpdateAuto => on_op(op, handlers::updates::update_auto),
        Op::SnapshotsList => on_op(op, handlers::updates::snapshots_list),
        Op::SnapshotRollback => on_op(op, handlers::updates::snapshot_rollback),
        Op::CatalogForServer => on_op(op, handlers::resources::catalog_for_server),
        Op::ResourcesPlan => on_op(op, handlers::resources::resources_plan),
        Op::ResourcesAdd => on_op(op, handlers::resources::resources_add),
        Op::WebhookGet => on_op(op, handlers::settings::webhook_get),
        Op::WebhookSet => on_op(op, handlers::settings::webhook_set),
        Op::WebhookRemove => on_op(op, handlers::settings::webhook_remove),
        Op::WebhookTest => on_op(op, handlers::settings::webhook_test),
        Op::ScheduleGet => on_op(op, handlers::settings::schedule_get),
        Op::ScheduleSet => on_op(op, handlers::settings::schedule_set),
        Op::ScheduleRemove => on_op(op, handlers::settings::schedule_remove),
        Op::PortsSet => on_op(op, handlers::settings::ports_set),
        Op::IssueReport => on_op(op, handlers::github::issue_report),
        Op::PrPlan => on_op(op, handlers::github::pr_plan),
        Op::PrOpen => on_op(op, handlers::github::pr_open),
        Op::GithubStatus => on_op(op, handlers::github::github_status),
        Op::GithubLogin => on_op(op, handlers::github::github_login),
        Op::GithubOwners => on_op(op, handlers::github::github_owners),
        Op::CatalogGet => on_op(op, handlers::resources::catalog_get),
        Op::ArtifactsList => on_op(op, handlers::install::artifacts_list),
        Op::InstallDefaults => on_op(op, handlers::install::install_defaults),
        Op::InstallSavedClear => on_op(op, handlers::install::install_saved_clear),
        Op::CheckPorts => on_op(op, handlers::install::check_ports),
        Op::CheckDisk => on_op(op, handlers::install::check_disk),
        Op::CheckDbLogin => on_op(op, handlers::install::check_db_login),
        Op::InstallPlan => on_op(op, handlers::install::install_plan),
        Op::InstallStart => on_op(op, handlers::install::install_start),
        Op::InstallLog => on_op(op, handlers::install::install_log),
        Op::SelfUpdateCheck => on_op(op, handlers::maintenance::self_update_check),
        Op::SelfUpdateApply => on_op(op, handlers::maintenance::self_update_apply),
        Op::AppStatus => on_op(op, handlers::maintenance::app_status),
        Op::AppInstall => on_op(op, handlers::maintenance::app_install),
        Op::AppUninstall => on_op(op, handlers::maintenance::app_uninstall),
        Op::UninstallPlan => on_op(op, handlers::maintenance::uninstall_plan),
        Op::UninstallRun => on_op(op, handlers::maintenance::uninstall_run),
        Op::JobsGet => on_op(op, crate::jobs::job_get),
        Op::JobEvents => on_op(op, crate::jobs::job_events),
        Op::Events => on_op(op, handlers::system::events),
        Op::MetricsGet => on_op(op, handlers::monitor::metrics_get),
        Op::AlertsList => on_op(op, handlers::monitor::alerts_list),
        Op::AlertsAck => on_op(op, handlers::monitor::alerts_ack),
        Op::AlertSettingsGet => on_op(op, handlers::monitor::alert_settings_get),
        Op::AlertSettingsSet => on_op(op, handlers::monitor::alert_settings_set),
        Op::NodesList => on_op(op, handlers::hub::nodes_list),
        Op::NodeAdd => on_op(op, handlers::hub::node_add),
        Op::NodeRemove => on_op(op, handlers::hub::node_remove),
        Op::NodeServers => on_op(op, handlers::hub::node_servers),
        Op::NodeServerControl => on_op(op, handlers::hub::node_server_control),
        Op::Fleet => on_op(op, handlers::hub::fleet),
    }
}

/// `{s}` → `:s` (axum 0.7 path syntax).
pub fn axum_path(p: &str) -> String {
    p.split('/')
        .map(|seg| match seg.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
            Some(name) => format!(":{name}"),
            None => seg.to_string(),
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Every operation, mounted at its route (relative to `/api/v1`).
pub fn api_routes() -> Router<AppState> {
    Op::ALL.iter().fold(Router::new(), |r, op| r.route(&axum_path(op.route().1), handler(*op)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_convert() {
        assert_eq!(axum_path("/servers/{s}/logs/{file}"), "/servers/:s/logs/:file");
        assert_eq!(axum_path("/meta"), "/meta");
    }

    #[test]
    fn every_op_mounts_without_conflicts() {
        // Panics on overlapping method+path, so this checks the table is consistent.
        let _ = api_routes();
    }
}
