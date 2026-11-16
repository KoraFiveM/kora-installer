//! What each thing the manager can do is served by in the shared operation list (`kora_core::api::Op`),
//! so the web panel offers the same. Every match is exhaustive with NO wildcard arm: adding an
//! `Action`, `Job`, `Screen` or `Exit` variant fails to compile until it is classified here.
use kora_core::api::parity::Parity::{self, Route, UiOnly};
use kora_core::api::Op;

use super::app::{Action, Exit, Job, Screen};

/// A key or button.
pub fn action(a: Action) -> Parity {
    match a {
        Action::Start | Action::Stop | Action::Restart => Route(Op::ServerControl),
        Action::OpenTx => {
            UiOnly("opens the txAdmin address in a browser; the panel shows it as a link")
        }
        Action::OpenFolder => Route(Op::ServerOpenFolder),
        Action::Logs => Route(Op::LogsList),
        Action::Console => Route(Op::ConsoleStream),
        Action::Send => Route(Op::ConsoleSend),
        Action::EnableConsole => Route(Op::ConsoleEnable),
        Action::Ensure => Route(Op::ResourceEnsure),
        Action::Delete => Route(Op::ServerDelete),
        Action::Update => Route(Op::UpdatePreview),
        Action::Notify => Route(Op::WebhookGet),
        Action::Schedule => Route(Op::ScheduleGet),
        Action::Ports => Route(Op::PortsSet),
        Action::Add => UiOnly(
            "opens the add-a-server menu; its choices are InstallNew, AddExisting and TypePath",
        ),
        Action::Refresh => Route(Op::ServersList),
        Action::Quit => UiOnly("leaves the terminal screen"),
        Action::Back => UiOnly("navigation"),
        Action::InstallNew => Route(Op::InstallStart),
        Action::AddExisting => Route(Op::ServersScan),
        Action::TypePath => Route(Op::ServersAdopt),
        Action::Confirm | Action::Yes | Action::No => {
            UiOnly("answers a question on screen; the panel confirms in the UI")
        }
        Action::Copy => UiOnly("clipboard"),
        Action::Reload => Route(Op::LogsTail),
        Action::PrevFile | Action::NextFile => Route(Op::LogsTail),
        Action::Top | Action::End | Action::PageUp | Action::PageDown => UiOnly("scrolling"),
        Action::ToggleRestart | Action::CycleRestart => Route(Op::UpdateApply),
        Action::ToggleAll | Action::Toggle => UiOnly("ticking boxes in a list"),
        Action::ApplySelected => Route(Op::UpdateApply),
        Action::Rollback => Route(Op::SnapshotRollback),
        Action::CheckUpdates => Route(Op::UpdatePreview),
        Action::Snapshots => Route(Op::SnapshotsList),
        Action::Paste => UiOnly("clipboard"),
        Action::Save => {
            UiOnly("saves the form on screen; the operation depends on the screen, see `screen`")
        }
        Action::SendTest => Route(Op::WebhookTest),
        Action::Remove => UiOnly(
            "removes the setting on screen; the operation depends on the screen, see `screen`",
        ),
        Action::Retry => UiOnly("repeats the last step"),
        Action::Up | Action::Down => UiOnly("moving the selection"),
        Action::AddResources => Route(Op::ResourcesAdd),
        Action::ReportIssue => Route(Op::IssueReport),
        Action::OpenPr => Route(Op::PrOpen),
        Action::Browse => Route(Op::CatalogForServer),
        Action::NextField => UiOnly("moving between fields"),
        Action::Install => Route(Op::ResourcesPlan),
    }
}

/// Slow work the manager runs after drawing "working...".
pub fn job(j: &Job) -> Parity {
    match j {
        Job::Control(_) => Route(Op::ServerControl),
        Job::Ensure => Route(Op::ResourceEnsure),
        Job::Delete => Route(Op::ServerDelete),
        Job::UpdatePreview | Job::CheckUpdates => Route(Op::UpdatePreview),
        Job::UpdateApply => Route(Op::UpdateApply),
        Job::Rollback(_) => Route(Op::SnapshotRollback),
        Job::Scan => Route(Op::ServersScan),
        Job::Adopt(_) | Job::AdoptPath => Route(Op::ServersAdopt),
        Job::NotifyTest => Route(Op::WebhookTest),
        Job::Snapshots => Route(Op::SnapshotsList),
        Job::AddLoad => Route(Op::CatalogForServer),
        Job::AddInstall => Route(Op::ResourcesAdd),
        Job::IssueSend => Route(Op::IssueReport),
        Job::PrPlan => Route(Op::PrPlan),
        Job::PrOpen => Route(Op::PrOpen),
    }
}

/// Everything one screen can do (`Parity::UiOnly` entries are navigation only).
pub fn screen(s: &Screen) -> &'static [Parity] {
    match s {
        Screen::Main => &[Route(Op::ServersList), Route(Op::ServerGet)],
        Screen::ConfirmRestart => &[Route(Op::ServerControl)],
        Screen::Logs(_) => &[Route(Op::LogsList), Route(Op::LogsTail)],
        Screen::Console(_) => &[
            Route(Op::ConsoleStream),
            Route(Op::ConsoleSend),
            Route(Op::ConsoleEnable),
        ],
        Screen::Ensure(_) => &[Route(Op::ResourceEnsure)],
        Screen::Delete(_) => &[Route(Op::ServerDelete), Route(Op::ServerUnregister)],
        Screen::Update(_) => &[
            Route(Op::UpdatePreview),
            Route(Op::UpdateApply),
            Route(Op::SnapshotsList),
            Route(Op::SnapshotRollback),
        ],
        Screen::Notify(_) => &[
            Route(Op::WebhookGet),
            Route(Op::WebhookSet),
            Route(Op::WebhookRemove),
            Route(Op::WebhookTest),
        ],
        Screen::Schedule(_) => &[
            Route(Op::ScheduleGet),
            Route(Op::ScheduleSet),
            Route(Op::ScheduleRemove),
        ],
        Screen::Ports(_) => &[Route(Op::PortsSet)],
        Screen::Add(_) => &[Route(Op::ServersScan), Route(Op::ServersAdopt)],
        Screen::AddRes(_) => &[
            Route(Op::CatalogForServer),
            Route(Op::ResourcesPlan),
            Route(Op::ResourcesAdd),
        ],
        Screen::Feedback(_) => &[
            Route(Op::IssueReport),
            Route(Op::PrPlan),
            Route(Op::PrOpen),
            Route(Op::GithubStatus),
        ],
    }
}

/// Why the manager stopped.
pub fn exit(e: &Exit) -> Parity {
    match e {
        Exit::Quit => UiOnly("leaves the terminal screen"),
        Exit::Elevate(_) => {
            UiOnly("the panel answers needs_elevation with the command to run instead")
        }
        Exit::InstallNew => Route(Op::InstallStart),
        Exit::GithubLogin => Route(Op::GithubLogin),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kora_core::api::Op;
    use std::collections::HashSet;

    const ACTIONS: &[Action] = &[
        Action::Start,
        Action::Stop,
        Action::Restart,
        Action::OpenTx,
        Action::OpenFolder,
        Action::Logs,
        Action::Console,
        Action::Send,
        Action::EnableConsole,
        Action::Ensure,
        Action::Delete,
        Action::Update,
        Action::Notify,
        Action::Schedule,
        Action::Ports,
        Action::Add,
        Action::Refresh,
        Action::Quit,
        Action::Back,
        Action::InstallNew,
        Action::AddExisting,
        Action::TypePath,
        Action::Confirm,
        Action::Yes,
        Action::No,
        Action::Copy,
        Action::Reload,
        Action::PrevFile,
        Action::NextFile,
        Action::Top,
        Action::End,
        Action::PageUp,
        Action::PageDown,
        Action::ToggleRestart,
        Action::ToggleAll,
        Action::ApplySelected,
        Action::Rollback,
        Action::CheckUpdates,
        Action::Snapshots,
        Action::CycleRestart,
        Action::Paste,
        Action::Save,
        Action::SendTest,
        Action::Remove,
        Action::Retry,
        Action::Up,
        Action::Down,
        Action::AddResources,
        Action::ReportIssue,
        Action::OpenPr,
        Action::Toggle,
        Action::Browse,
        Action::NextField,
        Action::Install,
    ];

    fn jobs() -> Vec<Job> {
        use super::super::app::ControlActionKey as K;
        vec![
            Job::Control(K::Start),
            Job::Ensure,
            Job::Delete,
            Job::UpdatePreview,
            Job::UpdateApply,
            Job::Rollback(String::new()),
            Job::Scan,
            Job::Adopt(0),
            Job::AdoptPath,
            Job::NotifyTest,
            Job::CheckUpdates,
            Job::Snapshots,
            Job::AddLoad,
            Job::AddInstall,
            Job::IssueSend,
            Job::PrPlan,
            Job::PrOpen,
        ]
    }

    fn reached() -> HashSet<Op> {
        let mut ops: HashSet<Op> = ACTIONS.iter().filter_map(|a| action(*a).op()).collect();
        ops.extend(jobs().iter().filter_map(|j| job(j).op()));
        for e in [
            Exit::Quit,
            Exit::Elevate(vec![]),
            Exit::InstallNew,
            Exit::GithubLogin,
        ] {
            ops.extend(exit(&e).op());
        }
        ops
    }

    #[test]
    fn every_referenced_op_exists() {
        for op in reached() {
            assert!(Op::ALL.contains(&op), "{op:?}");
        }
        for a in ACTIONS {
            if let UiOnly(why) = action(*a) {
                assert!(why.len() > 4, "{a:?}: give a reason");
            }
        }
    }

    /// Everything the manager's backend can do (see `kora_core::api::Backend`) is served by a route.
    #[test]
    fn every_manager_operation_maps_to_a_route() {
        let reached = reached();
        let needed = [
            Op::ServersList,
            Op::ServerControl,
            Op::ServerDelete,
            Op::ServersScan,
            Op::ServersAdopt,
            Op::ServerOpenFolder,
            Op::LogsList,
            Op::LogsTail,
            Op::ConsoleStream,
            Op::ConsoleSend,
            Op::ConsoleEnable,
            Op::ResourceEnsure,
            Op::UpdatePreview,
            Op::UpdateApply,
            Op::SnapshotsList,
            Op::SnapshotRollback,
            Op::CatalogForServer,
            Op::ResourcesPlan,
            Op::ResourcesAdd,
            Op::IssueReport,
            Op::PrPlan,
            Op::PrOpen,
            Op::PortsSet,
            Op::GithubLogin,
            Op::InstallStart,
            Op::WebhookTest,
        ];
        let missing: Vec<_> = needed.iter().filter(|o| !reached.contains(o)).collect();
        assert!(
            missing.is_empty(),
            "the manager has no way to reach {missing:?}"
        );
    }

    /// Screens carry several operations at once (the contextual Save/Remove keys, taking a server off the list).
    #[test]
    fn screens_carry_the_operations_buttons_cannot_name() {
        use super::super::app::{DelStage, DeleteState};
        use super::super::input::TextInput;
        let entry = kora_core::installer::registry::ServerEntry {
            name: "prod".into(),
            dir: "/srv/prod".into(),
            mode: kora_core::installer::Mode::Production,
            ports: kora_core::installer::registry::PortsRecord {
                game: 30120,
                txadmin: 40120,
            },
            created_unix: 0,
        };
        let del = Screen::Delete(Box::new(DeleteState {
            entry,
            stage: DelStage::Unlist,
            keep_files: true,
            keep_db: true,
            input: TextInput::new(10),
        }));
        assert!(screen(&del).contains(&Route(Op::ServerUnregister)));
        assert!(screen(&Screen::Main).contains(&Route(Op::ServersList)));
        assert!(screen(&Screen::ConfirmRestart).contains(&Route(Op::ServerControl)));
    }

    #[test]
    fn screen_operations_exist() {
        // Every screen's operations are in the list, including the settings saved with the contextual keys.
        for p in [
            Route(Op::WebhookSet),
            Route(Op::WebhookRemove),
            Route(Op::ScheduleSet),
            Route(Op::ScheduleRemove),
            Route(Op::PortsSet),
        ] {
            assert!(Op::ALL.contains(&p.op().unwrap()));
        }
    }
}
