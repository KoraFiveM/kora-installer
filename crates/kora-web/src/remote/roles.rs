//! Roles and what each operation needs (plan section 1, "Remote destructive actions").
//!
//! Every `match` over [`Op`] here is exhaustive (no wildcard arm): adding an operation without deciding
//! its role, whether it is destructive and whether it is local-only does not compile.
use kora_core::api::{Method, Op};
use serde::{Deserialize, Serialize};

/// Lowest to highest. A role can do everything the roles below it can.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Read-only: lists, status, updates available. No logs and no console.
    Viewer,
    /// Day-to-day running: start/stop/restart, updates, logs, console output, ensure, webhook test.
    Operator,
    /// Everything, subject to the remote-destructive switch.
    Admin,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Viewer => "viewer",
            Role::Operator => "operator",
            Role::Admin => "admin",
        }
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Role {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "viewer" => Ok(Role::Viewer),
            "operator" => Ok(Role::Operator),
            "admin" => Ok(Role::Admin),
            _ => Err(format!(
                "'{s}' is not a role. Use viewer, operator or admin."
            )),
        }
    }
}

/// The lowest role that may run `op` from a remote session.
pub fn min_role(op: Op) -> Role {
    use Role::{Admin, Operator, Viewer};
    match op {
        // Reading status and lists.
        Op::Meta
        | Op::SessionCreate
        | Op::SystemInfo
        | Op::ServersList
        | Op::ServerGet
        | Op::ResourcesList
        | Op::BranchesList
        | Op::SnapshotsList
        | Op::CatalogForServer
        | Op::WebhookGet
        | Op::ScheduleGet
        | Op::GithubStatus
        | Op::GithubOwners
        | Op::CatalogGet
        | Op::ArtifactsList
        | Op::SelfUpdateCheck
        | Op::AppStatus
        | Op::JobsGet
        | Op::JobEvents
        | Op::Events
        | Op::SessionLogout
        // Monitoring charts are status, like the server list.
        | Op::MetricsGet
        | Op::AlertsList => Viewer,
        // Running servers day to day. Logs and the console can hold player names and addresses,
        // so they are operator and above even though they only read.
        Op::ServerControl
        | Op::ServersScan
        | Op::LogsList
        | Op::LogsTail
        | Op::ConsoleStream
        | Op::ResourceEnsure
        // A fixed command for one installed resource (not free console text), like server control.
        | Op::ResourceControl
        // Branch lists, pull request lists and the "who has this branch" check only read (they may ask GitHub).
        | Op::BranchChoices
        | Op::PrList
        | Op::BranchBulkPreview
        | Op::UpdatePreview
        | Op::UpdateApply
        | Op::UpdateAuto
        | Op::ResourcesPlan
        | Op::WebhookTest
        | Op::CheckPorts
        | Op::CheckDisk
        | Op::InstallLog
        | Op::AlertsAck
        // Lists imported paid assets with hints taken from the console output.
        | Op::AssetsList
        // Read-only machine checks: which tools and database are here, and where.
        | Op::ToolsStatus
        | Op::SystemCheck
        // Names and last four characters of the saved licence keys, so an install can pick one; and what
        // the wizard pre-fills. Changing a server's key by NAME is day-to-day; pasting a new key or saving
        // one needs an admin (the handler checks that).
        | Op::KeysList
        | Op::ServerKeySet
        | Op::ProfileGet => Operator,
        // Everything that changes the machine, accounts, settings or the panel itself.
        Op::ServerDelete
        | Op::ServerUnregister
        | Op::ServersAdopt
        | Op::ServerOpenFolder
        | Op::ConsoleSend
        | Op::ConsoleEnable
        | Op::SnapshotRollback
        | Op::ResourcesAdd
        | Op::WebhookSet
        | Op::WebhookRemove
        | Op::ScheduleSet
        | Op::ScheduleRemove
        | Op::PortsSet
        | Op::IssueReport
        | Op::PrPlan
        | Op::PrOpen
        | Op::GithubLogin
        | Op::InstallDefaults
        | Op::InstallSavedClear
        | Op::CheckDbLogin
        | Op::CheckCfx
        | Op::InstallPlan
        | Op::InstallStart
        | Op::InstallResume
        | Op::SelfUpdateApply
        | Op::AppInstall
        | Op::AppUninstall
        | Op::UninstallPlan
        | Op::UninstallRun
        | Op::RemoteGet
        | Op::RemoteSet
        | Op::UsersList
        | Op::UserAdd
        | Op::UserRemove
        // Alert settings hold webhook addresses; the node list holds tokens.
        | Op::AlertSettingsGet
        | Op::AlertSettingsSet
        | Op::NodesList
        | Op::NodeAdd
        | Op::NodeRemove
        // The hub acts on other panels with their stored tokens (local only, see below).
        | Op::NodeServers
        | Op::NodeServerControl
        | Op::Fleet
        // Mints sign-in codes for the launch link (local only).
        | Op::SessionLink
        | Op::PanelRestart
        // Uploading, importing and removing paid assets writes into the server folder.
        | Op::AssetInspect
        | Op::AssetImport
        | Op::AssetRemove
        | Op::AssetDownloads
        | Op::ToolInstall
        | Op::DbStart
        // Adding, renaming, replacing and deleting saved licence keys.
        | Op::KeyAdd
        | Op::KeyUpdate
        | Op::KeyRemove
        | Op::ProfileSet
        // Moving a resource onto another branch or a pull request changes the code the server runs.
        | Op::BranchSwitch
        | Op::StashRestore
        | Op::BranchBulkApply => Admin,
    }
}

/// Destructive operations: deleting or unregistering a server, uninstalling Kora, console input, updating
/// Kora itself, changing the schedule. A remote session signed in with a **user token** is refused these
/// unless `allow_remote_destructive` is on (`kora panel remote-actions on`), even for admins. A
/// **Tailscale** admin is not held back: see [`remote_gate`]. The typed-name confirmation the handlers
/// enforce applies in every case.
///
/// Installing a server is NOT in this list: see [`installs`].
pub fn destructive(op: Op) -> bool {
    match op {
        Op::ServerDelete
        | Op::ServerUnregister
        | Op::UninstallRun
        | Op::AppUninstall
        | Op::ConsoleSend
        | Op::SelfUpdateApply
        | Op::ScheduleSet
        | Op::ScheduleRemove => true,
        Op::Meta
        | Op::SessionCreate
        | Op::SystemInfo
        | Op::ServersList
        | Op::ServerGet
        | Op::ServerControl
        | Op::ServersScan
        | Op::ServersAdopt
        | Op::ServerOpenFolder
        | Op::LogsList
        | Op::LogsTail
        | Op::ConsoleStream
        | Op::ConsoleEnable
        | Op::ResourcesList
        | Op::ResourceEnsure
        | Op::ResourceControl
        | Op::BranchesList
        | Op::BranchChoices
        | Op::PrList
        | Op::StashRestore
        | Op::BranchSwitch
        | Op::BranchBulkApply
        | Op::BranchBulkPreview
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
        | Op::CheckCfx
        | Op::InstallPlan
        | Op::InstallStart
        | Op::InstallResume
        | Op::InstallLog
        | Op::SelfUpdateCheck
        | Op::AppStatus
        | Op::AppInstall
        | Op::UninstallPlan
        | Op::JobsGet
        | Op::JobEvents
        | Op::Events
        | Op::SessionLogout
        | Op::SessionLink
        | Op::PanelRestart
        | Op::RemoteGet
        | Op::RemoteSet
        | Op::UsersList
        | Op::UserAdd
        | Op::UserRemove
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
        // Removing a paid asset keeps it as a backup.
        | Op::AssetsList
        | Op::AssetInspect
        | Op::AssetImport
        | Op::AssetRemove
        | Op::AssetDownloads
        // Verified pinned downloads into Kora's own folder, and starting the installed database: admin-only, not destructive.
        | Op::ToolsStatus
        | Op::ToolInstall
        | Op::SystemCheck
        | Op::DbStart
        // A saved key can be added again; servers installed with it are not touched.
        | Op::KeysList
        | Op::KeyAdd
        | Op::KeyUpdate
        | Op::KeyRemove
        | Op::ServerKeySet
        | Op::ProfileGet
        | Op::ProfileSet => false,
    }
}

/// Installing a new server: starting an install and resuming a paused one. Admin only (see [`min_role`]),
/// and allowed for every remote admin by default, token or Tailscale. A leaked install-only token can be
/// undone by removing the server it made; removing is what token sessions cannot do until the switch is on.
pub fn installs(op: Op) -> bool {
    match op {
        Op::InstallStart | Op::InstallResume => true,
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
        | Op::CheckCfx
        | Op::InstallPlan
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
        | Op::SessionLogout
        | Op::SessionLink
        | Op::PanelRestart
        | Op::RemoteGet
        | Op::RemoteSet
        | Op::UsersList
        | Op::UserAdd
        | Op::UserRemove
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
        | Op::BranchBulkApply => false,
    }
}

/// How a remote session signed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    /// Tailscale: the Tailscale daemon verified which person and device this is.
    Tailscale,
    /// A user token: a secret in a link, whoever holds it is that user.
    Token,
}

/// Does the machine's `remote actions` switch decide whether this session may run `op`? Only a token
/// session, and only for the destructive operations. A Tailscale identity is verified by the daemon
/// (a person on a known device), so Tailscale admins get the destructive actions by default; a token can
/// leak in a chat or a backup, so it cannot delete anything until someone on the machine says it may.
/// Installs are never held back by the switch. Pure, so every session kind x action class is tested.
pub fn needs_switch(op: Op, via: Via) -> bool {
    destructive(op) && via == Via::Token
}

/// Role, local-only and the `remote actions` switch together: may a remote session of `role` that signed
/// in `via` run `op` when the switch is `switch_on`? (`security::authorise` applies the same rules and
/// words the refusals.)
pub fn remote_allowed(op: Op, role: Role, via: Via, switch_on: bool) -> bool {
    !local_only(op) && role >= min_role(op) && (switch_on || !needs_switch(op, via))
}

/// Never available to a remote session, whatever its role: it acts on this computer's desktop, or it
/// manages remote access itself (a remote admin must not be able to widen their own access).
pub fn local_only(op: Op) -> bool {
    match op {
        Op::ServerOpenFolder
        | Op::RemoteGet
        | Op::RemoteSet
        | Op::UsersList
        | Op::UserAdd
        | Op::UserRemove
        // Node tokens are stored here: a remote caller must not manage the node list.
        | Op::NodesList
        | Op::NodeAdd
        | Op::NodeRemove
        // The hub uses those tokens to read and control other panels: a remote session must not borrow
        // them (it would reach panels its own account was never given).
        | Op::NodeServers
        | Op::NodeServerControl
        | Op::Fleet
        // Sign-in codes stand in for the launch token, which never leaves this computer.
        | Op::SessionLink
        // Restarting stops this computer's panel (and drops every session).
        | Op::PanelRestart
        // Lists files in the Downloads folder of the computer that runs Kora.
        | Op::AssetDownloads => true,
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
        | Op::LogsList
        | Op::LogsTail
        | Op::ConsoleStream
        | Op::ConsoleSend
        | Op::ConsoleEnable
        | Op::ResourcesList
        | Op::ResourceEnsure
        | Op::ResourceControl
        | Op::BranchesList
        | Op::BranchChoices
        | Op::PrList
        | Op::StashRestore
        | Op::BranchBulkPreview
        | Op::BranchSwitch
        | Op::BranchBulkApply
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
        | Op::CheckCfx
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
        | Op::SessionLogout
        | Op::MetricsGet
        | Op::AlertsList
        | Op::AlertsAck
        | Op::AlertSettingsGet
        | Op::AlertSettingsSet
        | Op::AssetsList
        | Op::AssetInspect
        | Op::AssetImport
        | Op::AssetRemove
        | Op::ToolsStatus
        | Op::ToolInstall
        | Op::SystemCheck
        | Op::DbStart
        // Saved keys are managed by admins remotely too (the handlers check the role); nothing here is
        // about this computer's desktop.
        | Op::KeysList
        | Op::KeyAdd
        | Op::KeyUpdate
        | Op::KeyRemove
        | Op::ServerKeySet
        | Op::ProfileGet
        | Op::ProfileSet => false,
    }
}

const PREFIX: &str = "/api/v1";

fn method_of(m: &str) -> Option<Method> {
    match m {
        "GET" | "HEAD" => Some(Method::Get),
        "POST" => Some(Method::Post),
        "PUT" => Some(Method::Put),
        "DELETE" => Some(Method::Delete),
        _ => None,
    }
}

fn is_param(seg: &str) -> bool {
    seg.starts_with('{') && seg.ends_with('}')
}

/// Does `pattern` (a route such as `/servers/{s}/logs`) match `segs`? On a match, returns which segments
/// were literal, so the caller can prefer the most specific pattern the way the router does.
fn matches(pattern: &str, segs: &[&str]) -> Option<Vec<bool>> {
    let pat: Vec<&str> = pattern.trim_start_matches('/').split('/').collect();
    if pat.len() != segs.len() {
        return None;
    }
    let mut lit = Vec::with_capacity(pat.len());
    for (p, s) in pat.iter().zip(segs) {
        if is_param(p) {
            if s.is_empty() {
                return None;
            }
            lit.push(false);
        } else if p == s {
            lit.push(true);
        } else {
            return None;
        }
    }
    Some(lit)
}

/// The operation a request will reach, found the way the router finds it: the most specific path first
/// (a literal segment beats a parameter, so `/servers/scan` is not a server called "scan"), then the
/// method. `None` when no operation has that path and method (the router will answer 404 or 405).
pub fn op_for(method: &str, path: &str) -> Option<Op> {
    let rest = path.strip_prefix(PREFIX)?;
    let m = method_of(method)?;
    let segs: Vec<&str> = rest.trim_start_matches('/').split('/').collect();
    let mut best: Option<(Vec<bool>, &'static str)> = None;
    for op in Op::ALL {
        let pattern = op.route().1;
        if let Some(lit) = matches(pattern, &segs) {
            // Lexicographic: the first differing segment decides, literal (true) wins.
            if best.as_ref().is_none_or(|(b, _)| lit > *b) {
                best = Some((lit, pattern));
            }
        }
    }
    let (_, pattern) = best?;
    Op::ALL
        .iter()
        .copied()
        .find(|op| op.route().1 == pattern && op.route().0 == m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_op_is_classified_and_reachable() {
        // The exhaustive matches above make this a compile-time guarantee; this also checks the
        // classification is coherent and that every route resolves back to its own op.
        for op in Op::ALL {
            let role = min_role(*op);
            if destructive(*op) || local_only(*op) {
                assert_eq!(
                    role,
                    Role::Admin,
                    "{op:?} is destructive or local-only, so admin"
                );
            }
            let (m, p) = op.route();
            let path = format!(
                "{PREFIX}{}",
                p.replace("{s}", "prod")
                    .replace("{id}", "x")
                    .replace("{file}", "f.log")
                    .replace("{job}", "j")
                    .replace("{name}", "bob")
            );
            assert_eq!(op_for(m.as_str(), &path), Some(*op), "{op:?} {path}");
        }
    }

    #[test]
    fn viewer_ops_are_reads() {
        for op in Op::ALL {
            if min_role(*op) == Role::Viewer {
                let m = op.route().0;
                assert!(
                    m == Method::Get || matches!(op, Op::SessionCreate | Op::SessionLogout),
                    "{op:?} is a viewer op but not a GET"
                );
            }
        }
    }

    #[test]
    fn the_listed_destructive_ops() {
        let want = [
            Op::ServerDelete,
            Op::ServerUnregister,
            Op::UninstallRun,
            Op::AppUninstall,
            Op::ConsoleSend,
            Op::SelfUpdateApply,
            Op::ScheduleSet,
            Op::ScheduleRemove,
        ];
        for op in Op::ALL {
            assert_eq!(destructive(*op), want.contains(op), "{op:?}");
        }
    }

    #[test]
    fn installing_is_its_own_class_and_not_destructive() {
        for op in Op::ALL {
            assert_eq!(installs(*op), matches!(op, Op::InstallStart | Op::InstallResume), "{op:?}");
            if installs(*op) {
                assert!(!destructive(*op), "{op:?}: the switch must not hold an install back");
                assert_eq!(min_role(*op), Role::Admin, "{op:?}");
                assert!(!local_only(*op), "{op:?}");
            }
        }
    }

    /// Every session kind x action class, for an admin with the switch off and on.
    #[test]
    fn who_may_do_what_remotely() {
        use Via::{Tailscale, Token};
        let delete = [Op::ServerDelete, Op::ServerUnregister, Op::UninstallRun, Op::AppUninstall];
        let install = [Op::InstallStart, Op::InstallResume];
        // Ordinary admin work: no switch involved.
        let ordinary = [Op::ServerControl, Op::ResourcesAdd, Op::PortsSet, Op::WebhookSet, Op::AssetImport];
        let local = [Op::ServerOpenFolder, Op::RemoteSet, Op::UserAdd, Op::SessionLink];
        for op in delete.iter().chain(&[Op::ConsoleSend, Op::SelfUpdateApply, Op::ScheduleSet, Op::ScheduleRemove]) {
            assert!(remote_allowed(*op, Role::Admin, Tailscale, false), "{op:?}: Tailscale admin, switch off");
            assert!(remote_allowed(*op, Role::Admin, Tailscale, true), "{op:?}: Tailscale admin, switch on");
            assert!(!remote_allowed(*op, Role::Admin, Token, false), "{op:?}: token admin, switch off");
            assert!(remote_allowed(*op, Role::Admin, Token, true), "{op:?}: token admin, switch on");
            for via in [Tailscale, Token] {
                assert!(!remote_allowed(*op, Role::Operator, via, true), "{op:?}: operators never");
                assert!(!remote_allowed(*op, Role::Viewer, via, true), "{op:?}: viewers never");
            }
        }
        for op in install {
            assert!(remote_allowed(op, Role::Admin, Tailscale, false), "{op:?}");
            assert!(remote_allowed(op, Role::Admin, Token, false), "{op:?}: token admin installs by default");
            assert!(remote_allowed(op, Role::Admin, Token, true), "{op:?}");
            for via in [Tailscale, Token] {
                assert!(!remote_allowed(op, Role::Operator, via, true), "{op:?}: operators cannot install");
                assert!(!remote_allowed(op, Role::Viewer, via, true), "{op:?}");
            }
        }
        for op in ordinary {
            for via in [Tailscale, Token] {
                assert!(remote_allowed(op, Role::Admin, via, false), "{op:?}");
            }
        }
        for op in local {
            for via in [Tailscale, Token] {
                for on in [false, true] {
                    assert!(!remote_allowed(op, Role::Admin, via, on), "{op:?}: local only, whatever the switch");
                }
            }
        }
    }

    #[test]
    fn literal_segments_beat_parameters() {
        assert_eq!(
            op_for("POST", "/api/v1/servers/scan"),
            Some(Op::ServersScan)
        );
        assert_eq!(
            op_for("POST", "/api/v1/servers/adopt"),
            Some(Op::ServersAdopt)
        );
        assert_eq!(op_for("GET", "/api/v1/servers/scan"), None);
        assert_eq!(
            op_for("DELETE", "/api/v1/servers/prod"),
            Some(Op::ServerDelete)
        );
        assert_eq!(op_for("HEAD", "/api/v1/servers/prod"), Some(Op::ServerGet));
        assert_eq!(op_for("GET", "/api/v1/nothing"), None);
        assert_eq!(op_for("PATCH", "/api/v1/servers"), None);
    }

    #[test]
    fn hub_and_sign_in_links_are_local_only() {
        for op in [Op::NodesList, Op::NodeAdd, Op::NodeRemove, Op::NodeServers, Op::NodeServerControl, Op::Fleet, Op::SessionLink, Op::PanelRestart] {
            assert!(local_only(op), "{op:?}");
        }
        assert_eq!(op_for("POST", "/api/v1/session/link"), Some(Op::SessionLink));
        assert_eq!(op_for("POST", "/api/v1/session/logout"), Some(Op::SessionLogout));
    }

    #[test]
    fn roles_parse_and_order() {
        assert!(Role::Viewer < Role::Operator && Role::Operator < Role::Admin);
        assert_eq!("Admin".parse::<Role>(), Ok(Role::Admin));
        assert!("root".parse::<Role>().is_err());
    }
}
