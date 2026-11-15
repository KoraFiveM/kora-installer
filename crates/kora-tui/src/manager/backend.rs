//! Everything the manager does to the outside world, behind one trait so the screens can be
//! tested with a fake. The real implementation is in `adapter.rs`.

use std::path::PathBuf;

use kora_core::installer::registry::ServerEntry;
use kora_core::service::ControlAction;

use kora_core::notify::NotifyConfig;
use kora_core::service::ScheduleConfig;
use kora_core::update::manual::{ApplyOpts, ChangeStatus, Scope, SnapshotInfo, UpdateEvent, UpdatePreview};
use kora_core::update::UpdateReport;

pub type Res<T> = Result<T, String>;

/// Live state of a server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Running,
    Stopped,
    /// The folder is gone.
    Missing,
}

/// What is known about updates, shown as a small badge. Absent = not known yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateBadge {
    UpToDate,
    Updates(u32),
    NeedsAttention,
    NewRelease,
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
    fn badge(&mut self, e: &ServerEntry) -> Option<UpdateBadge>;
    fn control(&mut self, e: &ServerEntry, a: ControlAction) -> Res<()>;
    /// Resource names known for this server (for picking in the Ensure screen).
    fn resources(&mut self, e: &ServerEntry) -> Vec<String>;
    /// Make the server load `name`; optionally restart it so it takes effect. Returns what happened.
    fn ensure(&mut self, e: &ServerEntry, name: &str, restart: bool) -> Res<String>;
    /// Delete what Kora created (like `kora delete`). Returns notes to show.
    fn delete(&mut self, e: &ServerEntry, keep_files: bool, keep_db: bool) -> Res<Vec<String>>;
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

    fn notify_load(&mut self, e: &ServerEntry) -> NotifyConfig;
    fn notify_save(&mut self, e: &ServerEntry, c: &NotifyConfig) -> Res<()>;
    fn notify_test(&mut self, e: &ServerEntry, c: &NotifyConfig) -> Res<()>;

    fn schedule_get(&mut self, e: &ServerEntry) -> Option<ScheduleConfig>;
    fn schedule_set(&mut self, e: &ServerEntry, c: &ScheduleConfig) -> Res<()>;
    fn schedule_remove(&mut self, e: &ServerEntry) -> Res<()>;
    /// When changing the schedule needs administrator rights this process does not have: the
    /// `kora` arguments that do it elevated (the manager closes, runs them, and comes back).
    fn schedule_elevation(&mut self, e: &ServerEntry, set: Option<&ScheduleConfig>) -> Option<Vec<String>>;
}
