//! Everything a front end (the web panel, the CLI) does to the outside world, behind one trait so
//! the pages can be tested with a fake. The real implementation is `super::system::SystemBackend`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::addresources::{AddOutcome, AddPlan};
use crate::catalog::Catalog;
use crate::feedback::PrPlan;
use crate::installer::registry::ServerEntry;
use crate::installer::Ports;
use crate::service::ControlAction;

use crate::notify::NotifyConfig;
use crate::paidassets::{AssetsView, ImportOpts, ImportPlan, ImportReport};
use crate::service::ScheduleConfig;
use crate::update::manual::{ApplyOpts, ChangeStatus, Scope, SnapshotInfo, UpdateEvent, UpdatePreview};
use crate::update::UpdateReport;

/// The error is text for a person, or a packed [`crate::problem::Problem`] (`Err(problem.into())`)
/// when the front end should offer specific actions. The panel unpacks it (`ApiError::core`); a
/// terminal front end shows it with [`crate::problem::plain_text`]. Plain strings stay valid and
/// become a generic problem.
pub type Res<T> = Result<T, String>;

/// Live state of a server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Running,
    Stopped,
    /// The folder is gone.
    Missing,
}

/// What is known about updates, shown as a small badge. Absent = not known yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "SampleBadge"))]
#[serde(rename_all = "snake_case")]
pub enum UpdateBadge {
    UpToDate,
    Updates(u32),
    NeedsAttention,
    NewRelease,
}

/// Detail for `server_get`: what the launcher can see right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
    /// The folder of an installed resource: the name FXServer knows it by (`start <folder>`). It can
    /// differ from the id [`resources`](Self::resources) lists (`kora-lib` lives in `kora_lib`).
    /// `None` = not installed.
    fn resource_folder(&mut self, e: &ServerEntry, id: &str) -> Option<String> {
        self.resources(e).iter().any(|r| r == id).then(|| id.to_string())
    }
    /// Make the server load `name`; optionally restart it so it takes effect. Returns what happened.
    fn ensure(&mut self, e: &ServerEntry, name: &str, restart: bool) -> Res<String>;
    /// Delete what Kora created (like `kora delete`). Returns notes to show.
    fn delete(&mut self, e: &ServerEntry, keep_files: bool, keep_db: bool) -> Res<Vec<String>>;
    /// [`Backend::delete`] with the database administrator password, for when dropping the database needs a
    /// login the install did not keep. Front ends without a password prompt keep the default (no password).
    fn delete_with_db_password(&mut self, e: &ServerEntry, keep_files: bool, keep_db: bool, _db_admin_password: Option<&str>) -> Res<Vec<String>> {
        self.delete(e, keep_files, keep_db)
    }
    /// [`Backend::delete_with_db_password`] with an administrator helper for the system-wide parts
    /// (the Windows panel's worker, see `installer::system_ops`). Front ends without one keep the default.
    fn delete_with_admin(&mut self, e: &ServerEntry, keep_files: bool, keep_db: bool, db_admin_password: Option<&str>, _admin: Option<crate::installer::system_ops::SharedAdmin>) -> Res<Vec<String>> {
        self.delete_with_db_password(e, keep_files, keep_db, db_admin_password)
    }
    /// Take the server off the manager's list without touching anything else.
    fn unregister(&mut self, e: &ServerEntry) -> Res<()>;
    fn register(&mut self, e: ServerEntry) -> Res<()>;
    /// Where "Add an existing server" looks first.
    fn scan_roots(&self) -> Vec<PathBuf>;

    fn preview(&mut self, e: &ServerEntry, scope: Scope) -> Res<UpdatePreview>;
    /// [`Backend::preview`] that also reports each resource as it is checked. Front ends that show progress override it.
    fn preview_events(&mut self, e: &ServerEntry, scope: Scope, _on: &mut dyn FnMut(UpdateEvent)) -> Res<UpdatePreview> {
        self.preview(e, scope)
    }
    fn apply(&mut self, e: &ServerEntry, scope: Scope, o: ApplyOpts, on: &mut dyn FnMut(UpdateEvent)) -> Res<UpdateReport>;
    /// `force`: roll back even resources with uncommitted changes (they are thrown away).
    fn rollback(&mut self, e: &ServerEntry, snapshot: &str, force: bool) -> Res<()>;
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
    /// The GitHub login Kora acts as (what the audit log records for an operation that acts on GitHub). `None`: not known.
    fn github_acting_as(&mut self) -> Option<String> {
        None
    }
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

    /// "Can players reach my server?": the checklist of [`crate::reach`]. `consent` replaces the saved
    /// choice about outside lookups for this one check. Front ends without it keep the default.
    fn reach_check(&mut self, _e: &ServerEntry, _consent: Option<crate::reach::settings::Consent>) -> Res<crate::reach::ReachReport> {
        Err(NO_REACH.into())
    }
    /// Which outside lookups the person allowed.
    fn reach_consent(&mut self) -> crate::reach::settings::Consent {
        crate::reach::settings::Consent::default()
    }
    fn reach_consent_set(&mut self, _c: crate::reach::settings::Consent) -> Res<()> {
        Err(NO_REACH.into())
    }
    /// Switch automatic port forwarding of the server's game port on or off.
    fn forwarding_set(&mut self, _e: &ServerEntry, _on: bool) -> Res<crate::reach::ForwardingView> {
        Err(NO_REACH.into())
    }
    /// Open the game port in this computer's firewall (needs administrator rights). Returns notes.
    fn firewall_open(&mut self, _e: &ServerEntry) -> Res<Vec<String>> {
        Err(NO_REACH.into())
    }
    /// Paid (Cfx.re Portal) assets imported on the server, and console hints about licence problems.
    fn assets_list(&mut self, _e: &ServerEntry) -> Res<AssetsView> {
        Err(NO_PAID_ASSETS.into())
    }
    /// What importing the zip at `zip` would do. Reads only.
    fn asset_inspect(&mut self, _e: &ServerEntry, _zip: &Path) -> Res<ImportPlan> {
        Err(NO_PAID_ASSETS.into())
    }
    /// Import the zip at `zip` into `resources/[paid]`. `say` receives progress lines.
    fn asset_import(&mut self, _e: &ServerEntry, _zip: &Path, _o: &ImportOpts, _say: &mut dyn FnMut(&str)) -> Res<ImportReport> {
        Err(NO_PAID_ASSETS.into())
    }
    /// Branches and pull requests of a Development server's resources (see [`crate::branches`]). `say(done, total, resource)`
    /// reports progress of the bulk operations. Front ends without it keep the default.
    fn branches(&mut self, _e: &ServerEntry, _req: &crate::branches::Request, _say: &mut dyn FnMut(usize, usize, &str)) -> Res<crate::branches::Reply> {
        Err(NO_BRANCHES.into())
    }
    /// The GitHub loop for one resource, forking, and the "which versions" setting (see [`crate::collab`]).
    fn collab(&mut self, _e: &ServerEntry, _req: &crate::collab::Request, _say: &mut dyn FnMut(usize, usize, &str)) -> Res<crate::collab::Reply> {
        Err(NO_BRANCHES.into())
    }
    /// Take an imported paid asset off the server (it is kept as a backup). Returns notes to show.
    fn asset_remove(&mut self, _e: &ServerEntry, _name: &str) -> Res<Vec<String>> {
        Err(NO_PAID_ASSETS.into())
    }

    // ---- backups of a server (see `crate::backup`). The defaults ARE the real thing, written over this trait
    // (they stop and start the server through `control`); a test double overrides them.
    /// The backups of a server, their settings and the "last backup" line.
    fn backups_view(&mut self, e: &ServerEntry) -> Res<crate::api::wire::BackupsView> {
        crate::backup::service::view(self, e)
    }
    /// The "last backup" line of every server.
    fn backups_overview(&mut self) -> Res<crate::api::wire::BackupsOverview> {
        crate::backup::service::overview(self)
    }
    /// Make a backup. `admin`: the caller may choose a folder.
    fn backup_create(&mut self, e: &ServerEntry, req: &crate::api::wire::BackupCreateRequest, admin: bool, say: &mut dyn FnMut(crate::backup::BackupEvent)) -> Res<crate::api::wire::BackupCreated> {
        crate::backup::service::create_backup(self, e, req, admin, say)
    }
    fn backup_settings_set(&mut self, e: &ServerEntry, s: &crate::backup::BackupSettings) -> Res<crate::backup::BackupSettings> {
        crate::backup::service::settings_set(self, e, s)
    }
    fn backup_verify(&mut self, e: &ServerEntry, id: &str, passphrase: Option<&str>, say: &mut dyn FnMut(crate::backup::BackupEvent)) -> Res<crate::api::wire::VerifyResult> {
        crate::backup::service::verify(self, e, id, passphrase, say)
    }
    fn backup_preview(&mut self, e: &ServerEntry, id: &str, passphrase: Option<&str>) -> Res<crate::backup::restore::RestorePreview> {
        crate::backup::service::preview(self, e, id, passphrase)
    }
    fn backup_restore(&mut self, e: &ServerEntry, id: &str, passphrase: Option<&str>, say: &mut dyn FnMut(crate::backup::BackupEvent)) -> Res<crate::backup::restore::RestoreReport> {
        crate::backup::service::restore_backup(self, e, id, passphrase, say)
    }
    fn backup_undo(&mut self, e: &ServerEntry, passphrase: Option<&str>, say: &mut dyn FnMut(crate::backup::BackupEvent)) -> Res<crate::backup::restore::RestoreReport> {
        crate::backup::service::undo(self, e, passphrase, say)
    }
    fn backup_delete(&mut self, e: &ServerEntry, id: &str) -> Res<()> {
        crate::backup::service::delete(self, e, id)
    }
    fn backup_prune(&mut self, e: &ServerEntry) -> Res<Vec<String>> {
        crate::backup::service::prune(self, e)
    }
    /// Backups of deleted servers.
    fn backups_archived(&mut self) -> Res<crate::api::wire::ArchivedBackups> {
        Ok(crate::backup::service::archived_view())
    }

    // ---- bringing a server from a backup (see `crate::backup::import`). `allow_path`: a file on this computer may be named
    // (a local session); a remote one must upload.
    fn import_inspect(&mut self, src: &crate::backup::import::ImportSource, passphrase: Option<&str>, allow_path: bool) -> Res<crate::backup::import::ImportPreview> {
        let servers = self.list()?;
        crate::backup::service::import_inspect(src, passphrase, allow_path, &servers)
    }
    fn import_run(&mut self, req: &crate::backup::import::ImportRequest, allow_path: bool, say: &mut dyn FnMut(crate::backup::BackupEvent)) -> Res<crate::backup::import::ImportResult> {
        crate::backup::service::import_run(self, req, allow_path, say)
    }

    // ---- the support bundle (see `crate::support`)
    fn support_preview(&mut self, server: Option<&str>) -> Res<crate::api::wire::BundleListing> {
        crate::support::service::preview(self, server)
    }
    fn support_summary(&mut self, server: Option<&str>) -> Res<String> {
        Ok(crate::support::service::summary(self, server))
    }
    /// The archive file of a backup, to download it (it holds secrets: administrators only).
    fn backup_file(&mut self, e: &ServerEntry, id: &str) -> Res<PathBuf> {
        crate::backup::service::file(self, e, id)
    }
}

/// What a front end without the reachability check answers.
pub const NO_REACH: &str = "The reachability check isn't available here.";

/// What a front end without branch switching answers.
pub const NO_BRANCHES: &str = "Branches can't be changed from here.";

/// What a front end without paid-asset support answers.
pub const NO_PAID_ASSETS: &str = "Paid assets can't be managed from here.";
