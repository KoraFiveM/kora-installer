//! Everything a front end (TUI, CLI, web panel) does to the outside world, behind one trait so
//! the screens can be tested with a fake. The real implementation is `super::system::SystemBackend`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::addresources::{AddOutcome, AddPlan};
use crate::catalog::Catalog;
use crate::feedback::PrPlan;
use crate::installer::registry::ServerEntry;
use crate::installer::Ports;
use crate::service::ControlAction;

use crate::notify::NotifyConfig;
use crate::service::ScheduleConfig;
use crate::update::manual::{ApplyOpts, ChangeStatus, Scope, SnapshotInfo, UpdateEvent, UpdatePreview};
use crate::update::UpdateReport;

pub type Res<T> = Result<T, String>;

/// Live state of a server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Running,
    Stopped,
    /// The folder is gone.
    Missing,
}

/// What is known about updates, shown as a small badge. Absent = not known yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateBadge {
    UpToDate,
    Updates(u32),
    NeedsAttention,
    NewRelease,
}

/// Detail for `server_get`: what the launcher can see right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusDetail {
    pub running: bool,
    pub game_listening: bool,
    pub txadmin_up: bool,
}

impl UpdateBadge {
    pub fn text(self) -> String {
        match self {
            UpdateBadge::UpToDate => "up to date".into(),
            UpdateBadge::Updates(1) => "1 update".into(),
            UpdateBadge::Updates(n) => format!("{n} updates"),
            UpdateBadge::NeedsAttention => "needs attention".into(),
            UpdateBadge::NewRelease => "new release".into(),
        }
    }
}

/// The badge for a fresh preview (what `[ Check for updates ]` shows).
pub fn badge_of(p: &UpdatePreview) -> UpdateBadge {
    let n = p.resources.iter().filter(|r| r.status == ChangeStatus::Clean).count() as u32;
    let attention = p.resources.iter().any(|r| matches!(r.status, ChangeStatus::Dirty | ChangeStatus::Diverged));
    if attention {
        UpdateBadge::NeedsAttention
    } else if n > 0 {
        UpdateBadge::Updates(n)
    } else if p.new_release.is_some() {
        UpdateBadge::NewRelease
    } else {
        UpdateBadge::UpToDate
    }
}

pub trait Backend {
    /// The registered servers.
    fn list(&mut self) -> Res<Vec<ServerEntry>>;
    /// Service state, falling back to a port probe when there is no service.
    fn status(&mut self, e: &ServerEntry) -> Status;
    /// Running / game port listening / txAdmin answering. The default derives it from `status`.
    fn status_detail(&mut self, e: &ServerEntry) -> StatusDetail {
        let running = self.status(e) == Status::Running;
        StatusDetail { running, game_listening: running, txadmin_up: running }
    }
    fn badge(&mut self, e: &ServerEntry) -> Option<UpdateBadge>;
    /// Start, stop or restart like the CLI: through the OS service when there is one, else by
    /// launching / stopping the detached process. `Err` carries why the server did not come up.
    fn control(&mut self, e: &ServerEntry, a: ControlAction) -> Res<()>;
    /// Resource names known for this server (for picking in the Ensure screen).
    fn resources(&mut self, e: &ServerEntry) -> Vec<String>;
    /// Make the server load `name`; optionally restart it so it takes effect. Returns what happened.
    fn ensure(&mut self, e: &ServerEntry, name: &str, restart: bool) -> Res<String>;
    /// Delete what Kora created (like `kora delete`). Returns notes to show.
    fn delete(&mut self, e: &ServerEntry, keep_files: bool, keep_db: bool) -> Res<Vec<String>>;
    /// [`Backend::delete`] with the database administrator password, for when dropping the database needs a
    /// login the install did not keep. Front ends without a password prompt keep the default (no password).
    fn delete_with_db_password(&mut self, e: &ServerEntry, keep_files: bool, keep_db: bool, _db_admin_password: Option<&str>) -> Res<Vec<String>> {
        self.delete(e, keep_files, keep_db)
    }
    /// Take the server off the manager's list without touching anything else.
    fn unregister(&mut self, e: &ServerEntry) -> Res<()>;
    fn register(&mut self, e: ServerEntry) -> Res<()>;
    /// Where "Add an existing server" looks first.
    fn scan_roots(&self) -> Vec<PathBuf>;

    fn preview(&mut self, e: &ServerEntry, scope: Scope) -> Res<UpdatePreview>;
    fn apply(&mut self, e: &ServerEntry, scope: Scope, o: ApplyOpts, on: &mut dyn FnMut(UpdateEvent)) -> Res<UpdateReport>;
    fn rollback(&mut self, e: &ServerEntry, snapshot: &str) -> Res<()>;
    /// Saved snapshots, newest first.
    fn snapshots(&mut self, e: &ServerEntry) -> Vec<SnapshotInfo>;
    /// `[ Check for updates ]`: a full preview (the core writes `update-status.json` for it), then the badge.
    fn check_updates(&mut self, e: &ServerEntry) -> Res<UpdateBadge> {
        self.preview(e, Scope::All).map(|p| badge_of(&p))
    }

    /// The catalog to browse (remote, else the bundled one) and the ids installed on the server.
    fn add_catalog(&mut self, e: &ServerEntry) -> Res<(Catalog, Vec<String>, Option<String>)>;
    /// What adding `ids` (plus dependencies) would do.
    fn add_plan(&mut self, e: &ServerEntry, ids: &[String]) -> Res<AddPlan>;
    /// Install `ids` (plus dependencies) into the right folders, pinned, and add the missing `ensure` lines.
    fn add_install(&mut self, e: &ServerEntry, ids: &[String]) -> Res<AddOutcome>;

    /// Is `gh` signed in to GitHub?
    fn github_signed_in(&mut self) -> bool;
    /// Report an issue about a resource of the server. Returns the issue address.
    fn report_issue(&mut self, e: &ServerEntry, resource: &str, title: &str, text: &str) -> Res<String>;
    /// Check that a pull request can be made from the resource's fork (development servers).
    fn pr_plan(&mut self, e: &ServerEntry, resource: &str) -> Res<PrPlan>;
    /// Push the fork branch and open the pull request. Returns its address.
    fn pr_open(&mut self, plan: &PrPlan, title: &str, text: &str) -> Res<String>;

    fn notify_load(&mut self, e: &ServerEntry) -> NotifyConfig;
    fn notify_save(&mut self, e: &ServerEntry, c: &NotifyConfig) -> Res<()>;
    fn notify_test(&mut self, e: &ServerEntry, c: &NotifyConfig) -> Res<()>;

    fn schedule_get(&mut self, e: &ServerEntry) -> Option<ScheduleConfig>;
    fn schedule_set(&mut self, e: &ServerEntry, c: &ScheduleConfig) -> Res<()>;
    fn schedule_remove(&mut self, e: &ServerEntry) -> Res<()>;
    /// When changing the schedule needs administrator rights this process does not have: the
    /// `kora` arguments that do it elevated (the manager closes, runs them, and comes back).
    fn schedule_elevation(&mut self, e: &ServerEntry, set: Option<&ScheduleConfig>) -> Option<Vec<String>>;

    /// Change the game and txAdmin ports of a stopped server. Returns notes to show (firewall, restart).
    fn ports_set(&mut self, e: &ServerEntry, p: Ports) -> Res<Vec<String>>;
}
