//! What each wizard screen (and the result screen's buttons) is served by in the shared operation list.
//! Exhaustive matches with NO wildcard arm: a new screen or button fails to compile until it is classified.
use kora_core::api::parity::Parity::{self, Route, UiOnly};
use kora_core::api::Op;

use crate::github_screens::Btn;
use crate::result::Action;
use crate::wizard::Screen;

/// A wizard screen: the operation that checks or collects what it asks for. Screens that only fill in a
/// field of the request are served by `InstallStart` (the request) and `InstallPlan` (the review).
pub fn screen(s: Screen) -> Parity {
    match s {
        Screen::Permission => UiOnly(
            "asks to restart with administrator rights; the panel answers needs_elevation instead",
        ),
        Screen::Welcome => UiOnly("introduction"),
        Screen::Prepare => Route(Op::InstallDefaults),
        Screen::Mode
        | Screen::Name
        | Screen::ArtifactUrl
        | Screen::Database
        | Screen::Firewall
        | Screen::Service
        | Screen::Shortcut => Route(Op::InstallStart),
        Screen::Cfx => Route(Op::CheckCfx),
        Screen::License
        | Screen::Hostname
        | Screen::GithubOwner
        | Screen::GameBuild
        | Screen::TxAdminPublic => Route(Op::InstallStart),
        Screen::Folder => Route(Op::CheckDisk),
        Screen::Ports => Route(Op::CheckPorts),
        Screen::Artifact => Route(Op::ArtifactsList),
        Screen::Resources | Screen::ResourceList => Route(Op::CatalogGet),
        Screen::DbAdmin | Screen::DbAdminLogin => Route(Op::CheckDbLogin),
        Screen::GithubAccount => Route(Op::GithubOwners),
        Screen::Review => Route(Op::InstallPlan),
    }
}

/// A button on the result screen.
pub fn result_action(a: Action) -> Parity {
    match a {
        Action::OpenLog => Route(Op::InstallLog),
        Action::OpenFolder => Route(Op::ServerOpenFolder),
        Action::CopyPath | Action::CopyDetails | Action::CopyConnect | Action::CopyPassword => {
            UiOnly("clipboard")
        }
        Action::Restart => UiOnly(
            "restarts Kora with administrator rights; the panel answers needs_elevation instead",
        ),
        Action::OpenTxAdmin => {
            UiOnly("opens the txAdmin address in a browser; the panel shows it as a link")
        }
        Action::StartServer | Action::StopServer => Route(Op::ServerControl),
    }
}

/// A button on the GitHub sign-in / owner picker.
pub fn github_button(b: Btn) -> Parity {
    match b {
        Btn::Refresh | Btn::Done => Route(Op::GithubOwners),
        Btn::Retry => Route(Op::GithubStatus),
        Btn::CreateOrg | Btn::TypeName | Btn::Copy | Btn::OpenGithub | Btn::Cancel | Btn::Paste => {
            UiOnly("navigation, clipboard or opening a web page")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREENS: &[Screen] = &[
        Screen::Permission,
        Screen::Welcome,
        Screen::Prepare,
        Screen::Mode,
        Screen::Name,
        Screen::Folder,
        Screen::Ports,
        Screen::Artifact,
        Screen::ArtifactUrl,
        Screen::Resources,
        Screen::ResourceList,
        Screen::Database,
        Screen::DbAdmin,
        Screen::DbAdminLogin,
        Screen::Firewall,
        Screen::Service,
        Screen::Shortcut,
        Screen::License,
        Screen::Cfx,
        Screen::Hostname,
        Screen::GithubOwner,
        Screen::GithubAccount,
        Screen::GameBuild,
        Screen::TxAdminPublic,
        Screen::Review,
    ];
    const RESULT: &[Action] = &[
        Action::OpenLog,
        Action::OpenFolder,
        Action::CopyPath,
        Action::CopyDetails,
        Action::Restart,
        Action::OpenTxAdmin,
        Action::CopyConnect,
        Action::CopyPassword,
        Action::StartServer,
        Action::StopServer,
    ];
    const GH: &[Btn] = &[
        Btn::Refresh,
        Btn::CreateOrg,
        Btn::TypeName,
        Btn::Retry,
        Btn::Copy,
        Btn::OpenGithub,
        Btn::Cancel,
        Btn::Done,
        Btn::Paste,
    ];

    #[test]
    fn every_referenced_op_is_in_the_list() {
        let ops = SCREENS
            .iter()
            .map(|s| screen(*s))
            .chain(RESULT.iter().map(|a| result_action(*a)))
            .chain(GH.iter().map(|b| github_button(*b)));
        for p in ops {
            if let Some(op) = p.op() {
                assert!(Op::ALL.contains(&op), "{op:?}");
            }
        }
    }

    #[test]
    fn the_wizard_reaches_the_install_operations() {
        let ops: Vec<Op> = SCREENS.iter().filter_map(|s| screen(*s).op()).collect();
        for need in [
            Op::InstallStart,
            Op::InstallPlan,
            Op::CheckPorts,
            Op::CheckDisk,
            Op::CheckDbLogin,
            Op::InstallDefaults,
            Op::CatalogGet,
            Op::ArtifactsList,
        ] {
            assert!(ops.contains(&need), "{need:?}");
        }
    }
}
