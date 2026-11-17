//! A thread-safe in-memory [`Backend`] for tests of front ends that need `Send` (the web panel).
//! Enabled for this crate's tests and, for other crates, with the `fake` feature.
//!
//! State lives in [`FakeState`] behind `Arc<Mutex<..>>`; every mutating call is appended to
//! `calls` so tests can assert on what a request did.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use super::backend::{badge_of, Backend, Res, Status, StatusDetail, UpdateBadge};
use crate::addresources::{AddOutcome, AddPlan};
use crate::catalog::Catalog;
use crate::feedback::PrPlan;
use crate::installer::registry::{PortsRecord, ServerEntry};
use crate::installer::{Mode, Ports};
use crate::notify::NotifyConfig;
use crate::paidassets::{AssetsView, FoundResource, ImportOpts, ImportPlan, ImportReport, ImportedAsset};
use crate::service::{ControlAction, ScheduleConfig};
use crate::update::manual::{ApplyOpts, Scope, SnapshotInfo, UpdateEvent, UpdatePreview};
use crate::update::UpdateReport;

#[derive(Default)]
pub struct FakeState {
    /// `reach_check` answers this (`None`: a plausible checklist).
    pub reach_report: Option<crate::reach::ReachReport>,
    pub reach_consent: crate::reach::settings::Consent,
    pub forwarding: HashMap<String, bool>,
    pub firewall_open_error: Option<String>,
    pub servers: Vec<ServerEntry>,
    pub status: HashMap<String, Status>,
    pub badges: HashMap<String, UpdateBadge>,
    pub calls: Vec<String>,
    pub list_error: Option<String>,
    pub control_error: Option<String>,
    pub ensure_error: Option<String>,
    pub delete_error: Option<String>,
    pub resources: Vec<String>,
    /// Resource id -> folder name, for ids whose folder is named differently.
    pub resource_folders: HashMap<String, String>,
    pub roots: Vec<PathBuf>,
    pub preview: Option<Res<UpdatePreview>>,
    pub apply: Option<Res<UpdateReport>>,
    pub apply_events: Vec<UpdateEvent>,
    pub rollback_error: Option<String>,
    pub snapshots: Vec<SnapshotInfo>,
    pub catalog: Option<(Catalog, Vec<String>)>,
    pub add_plan: Option<Res<AddPlan>>,
    pub add_result: Option<Res<AddOutcome>>,
    pub signed_in: bool,
    pub issue_result: Option<Res<String>>,
    pub pr_plan: Option<Res<PrPlan>>,
    pub pr_result: Option<Res<String>>,
    pub notify: HashMap<String, NotifyConfig>,
    pub notify_save_error: Option<String>,
    pub notify_test_error: Option<String>,
    pub schedule: HashMap<String, ScheduleConfig>,
    /// `Some(args)`: changing the schedule needs elevation and these are the `kora` arguments.
    pub elevation: Option<Vec<String>>,
    pub ports_error: Option<String>,
    /// Paid assets per server name.
    pub assets: HashMap<String, AssetsView>,
    /// What `asset_inspect` answers (`None`: an importable plan named after the zip).
    pub asset_plan: Option<Res<ImportPlan>>,
    /// What `asset_import` answers (`None`: one asset named after the zip).
    pub asset_import: Option<Res<ImportReport>>,
    pub asset_remove_error: Option<String>,
    /// What `branches` answers (`None`: an error that says nothing is set up). Every request is in `calls` as `branches {request:?}`.
    pub branches: Option<Res<crate::branches::Reply>>,
    /// The answer to the next `collab` request.
    pub collab: Option<Res<crate::collab::Reply>>,
}

/// Cheap to clone; clones share state. Give `move || Box::new(fake.clone())` to the panel.
#[derive(Clone, Default)]
pub struct FakeBackend(pub Arc<Mutex<FakeState>>);

/// A registry entry for tests.
pub fn entry(name: &str, mode: Mode, game: u16, txadmin: u16) -> ServerEntry {
    ServerEntry { source: None, versions: None, name: name.into(), dir: PathBuf::from(format!("/srv/{name}")), mode, ports: PortsRecord { game, txadmin }, created_unix: 0 }
}

impl FakeBackend {
    pub fn new() -> Self {
        Self::default()
    }
    /// Two servers: `prod` (production, running, up to date) and `dev` (development, stopped).
    pub fn sample() -> Self {
        let f = Self::new();
        {
            let mut s = f.state();
            s.servers = vec![entry("prod", Mode::Production, 30120, 40120), entry("dev", Mode::Development, 30130, 40130)];
            s.status.insert("prod".into(), Status::Running);
            s.badges.insert("prod".into(), UpdateBadge::UpToDate);
            s.resources = vec!["kora_lib".into(), "kora_hud".into()];
        }
        f
    }
    pub fn state(&self) -> MutexGuard<'_, FakeState> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    pub fn calls(&self) -> Vec<String> {
        self.state().calls.clone()
    }
}

fn err_or<T>(e: &Option<String>, ok: impl FnOnce() -> T) -> Res<T> {
    match e {
        Some(x) => Err(x.clone()),
        None => Ok(ok()),
    }
}

impl Backend for FakeBackend {
    fn collab(&mut self, e: &ServerEntry, req: &crate::collab::Request, say: &mut dyn FnMut(usize, usize, &str)) -> Res<crate::collab::Reply> {
        let mut s = self.state();
        s.calls.push(format!("collab {} {req:?}", e.name));
        if matches!(req, crate::collab::Request::Fork { .. }) {
            say(0, 1, "");
            say(1, 1, "kora_hud");
        }
        s.collab.clone().unwrap_or_else(|| Err("No collab answer is set up.".into()))
    }
    fn branches(&mut self, e: &ServerEntry, req: &crate::branches::Request, say: &mut dyn FnMut(usize, usize, &str)) -> Res<crate::branches::Reply> {
        let mut s = self.state();
        s.calls.push(format!("branches {} {req:?}", e.name));
        if matches!(req, crate::branches::Request::BulkPreview { .. } | crate::branches::Request::BulkApply { .. }) {
            say(0, 1, "");
            say(1, 1, "kora_hud");
        }
        s.branches.clone().unwrap_or_else(|| Err("No branch answer is set up.".into()))
    }
    fn list(&mut self) -> Res<Vec<ServerEntry>> {
        let s = self.state();
        err_or(&s.list_error, || s.servers.clone())
    }
    fn status(&mut self, e: &ServerEntry) -> Status {
        self.state().status.get(&e.name).copied().unwrap_or(Status::Stopped)
    }
    fn status_detail(&mut self, e: &ServerEntry) -> StatusDetail {
        let running = self.status(e) == Status::Running;
        StatusDetail { running, game_listening: running, txadmin_up: running }
    }
    fn badge(&mut self, e: &ServerEntry) -> Option<UpdateBadge> {
        self.state().badges.get(&e.name).copied()
    }
    fn control(&mut self, e: &ServerEntry, a: ControlAction) -> Res<()> {
        let mut s = self.state();
        s.calls.push(format!("{a:?} {}", e.name));
        if let Some(x) = &s.control_error {
            return Err(x.clone());
        }
        let st = if a == ControlAction::Stop { Status::Stopped } else { Status::Running };
        s.status.insert(e.name.clone(), st);
        Ok(())
    }
    fn resources(&mut self, _e: &ServerEntry) -> Vec<String> {
        self.state().resources.clone()
    }
    fn resource_folder(&mut self, _e: &ServerEntry, id: &str) -> Option<String> {
        let s = self.state();
        s.resources.iter().any(|r| r == id).then(|| s.resource_folders.get(id).cloned().unwrap_or_else(|| id.to_string()))
    }
    fn ensure(&mut self, e: &ServerEntry, name: &str, restart: bool) -> Res<String> {
        let mut s = self.state();
        s.calls.push(format!("ensure {} {name} restart={restart}", e.name));
        err_or(&s.ensure_error, || format!("ensured {name}"))
    }
    fn delete(&mut self, e: &ServerEntry, keep_files: bool, keep_db: bool) -> Res<Vec<String>> {
        let mut s = self.state();
        s.calls.push(format!("delete {} keep_files={keep_files} keep_db={keep_db}", e.name));
        if let Some(x) = &s.delete_error {
            return Err(x.clone());
        }
        s.servers.retain(|x| x.name != e.name);
        Ok(vec!["Deleted.".into()])
    }
    fn delete_with_db_password(&mut self, e: &ServerEntry, keep_files: bool, keep_db: bool, db_admin_password: Option<&str>) -> Res<Vec<String>> {
        let Some(pw) = db_admin_password.filter(|p| !p.is_empty()) else {
            return self.delete(e, keep_files, keep_db);
        };
        let mut s = self.state();
        // Records that a password was given, never the password itself.
        s.calls.push(format!("delete {} keep_files={keep_files} keep_db={keep_db} db_password=set", e.name));
        if let Some(x) = &s.delete_error {
            if !x.contains("KORA_DB_ADMIN_PASSWORD") || pw == "wrong" {
                return Err(x.clone());
            }
        }
        s.servers.retain(|x| x.name != e.name);
        Ok(vec!["Deleted.".into()])
    }
    fn unregister(&mut self, e: &ServerEntry) -> Res<()> {
        let mut s = self.state();
        s.calls.push(format!("unregister {}", e.name));
        s.servers.retain(|x| x.name != e.name);
        Ok(())
    }
    fn register(&mut self, e: ServerEntry) -> Res<()> {
        let mut s = self.state();
        s.calls.push(format!("register {}", e.name));
        s.servers.push(e);
        Ok(())
    }
    fn scan_roots(&self) -> Vec<PathBuf> {
        self.state().roots.clone()
    }
    fn preview(&mut self, e: &ServerEntry, scope: Scope) -> Res<UpdatePreview> {
        let mut s = self.state();
        s.calls.push(format!("preview {} {scope:?}", e.name));
        s.preview.clone().unwrap_or_else(|| Ok(UpdatePreview { resources: vec![], new_release: None, notes: vec![] }))
    }
    fn preview_events(&mut self, e: &ServerEntry, scope: Scope, on: &mut dyn FnMut(UpdateEvent)) -> Res<UpdatePreview> {
        let p = self.preview(e, scope)?;
        // One event per resource, in order, as the real check reports them (the fake has no network).
        let total = p.resources.len();
        for (n, r) in p.resources.iter().enumerate() {
            on(UpdateEvent::Checked { done: n + 1, total, id: r.id.clone() });
        }
        Ok(p)
    }
    fn apply(&mut self, e: &ServerEntry, scope: Scope, o: ApplyOpts, on: &mut dyn FnMut(UpdateEvent)) -> Res<UpdateReport> {
        let (events, result) = {
            let mut s = self.state();
            let targets = o.targets.as_ref().map(|t| format!(" targets={t:?}")).unwrap_or_default();
            s.calls.push(format!("apply {} {scope:?} restart={:?}{targets}", e.name, o.restart));
            (s.apply_events.clone(), s.apply.clone().unwrap_or_else(|| Ok(UpdateReport::default())))
        };
        for ev in events {
            on(ev);
        }
        result
    }
    fn rollback(&mut self, e: &ServerEntry, snapshot: &str, force: bool) -> Res<()> {
        let mut s = self.state();
        s.calls.push(format!("rollback {} {snapshot}{}", e.name, if force { " force" } else { "" }));
        err_or(&s.rollback_error, || ())
    }
    fn snapshots(&mut self, _e: &ServerEntry) -> Vec<SnapshotInfo> {
        self.state().snapshots.clone()
    }
    fn check_updates(&mut self, e: &ServerEntry) -> Res<UpdateBadge> {
        self.preview(e, Scope::All).map(|p| badge_of(&p))
    }
    fn add_catalog(&mut self, _e: &ServerEntry) -> Res<(Catalog, Vec<String>, Option<String>)> {
        match self.state().catalog.clone() {
            Some((c, i)) => Ok((c, i, None)),
            None => Err("The catalog could not be loaded.".into()),
        }
    }
    fn add_plan(&mut self, e: &ServerEntry, ids: &[String]) -> Res<AddPlan> {
        let mut s = self.state();
        s.calls.push(format!("add_plan {} {}", e.name, ids.join(",")));
        s.add_plan.clone().unwrap_or_else(|| Ok(AddPlan::default()))
    }
    fn add_install(&mut self, e: &ServerEntry, ids: &[String]) -> Res<AddOutcome> {
        let mut s = self.state();
        s.calls.push(format!("add_install {} {}", e.name, ids.join(",")));
        s.add_result.clone().unwrap_or_else(|| Ok(AddOutcome::default()))
    }
    fn github_signed_in(&mut self) -> bool {
        self.state().signed_in
    }
    fn report_issue(&mut self, e: &ServerEntry, resource: &str, title: &str, text: &str) -> Res<String> {
        let mut s = self.state();
        s.calls.push(format!("issue {} {resource} [{title}] [{text}]", e.name));
        s.issue_result.clone().unwrap_or_else(|| Ok("https://github.com/KoraFiveM/x/issues/1".into()))
    }
    fn pr_plan(&mut self, e: &ServerEntry, resource: &str) -> Res<PrPlan> {
        let mut s = self.state();
        s.calls.push(format!("pr_plan {} {resource}", e.name));
        s.pr_plan.clone().unwrap_or_else(|| Err("No pull request can be made for that resource.".into()))
    }
    fn pr_open(&mut self, plan: &PrPlan, title: &str, text: &str) -> Res<String> {
        let mut s = self.state();
        s.calls.push(format!("pr_open {} [{title}] [{text}]", plan.resource));
        s.pr_result.clone().unwrap_or_else(|| Ok("https://github.com/KoraFiveM/x/pull/2".into()))
    }
    fn notify_load(&mut self, e: &ServerEntry) -> NotifyConfig {
        self.state().notify.get(&e.name).cloned().unwrap_or_default()
    }
    fn notify_save(&mut self, e: &ServerEntry, c: &NotifyConfig) -> Res<()> {
        let mut s = self.state();
        if let Some(x) = &s.notify_save_error {
            return Err(x.clone());
        }
        s.calls.push(format!("notify_save {} set={}", e.name, c.discord_webhook.is_some()));
        s.notify.insert(e.name.clone(), c.clone());
        Ok(())
    }
    fn notify_test(&mut self, e: &ServerEntry, _c: &NotifyConfig) -> Res<()> {
        let mut s = self.state();
        s.calls.push(format!("notify_test {}", e.name));
        err_or(&s.notify_test_error, || ())
    }
    fn schedule_get(&mut self, e: &ServerEntry) -> Option<ScheduleConfig> {
        self.state().schedule.get(&e.name).cloned()
    }
    fn schedule_set(&mut self, e: &ServerEntry, c: &ScheduleConfig) -> Res<()> {
        let mut s = self.state();
        s.calls.push(format!("schedule_set {} {c:?}", e.name));
        s.schedule.insert(e.name.clone(), c.clone());
        Ok(())
    }
    fn schedule_remove(&mut self, e: &ServerEntry) -> Res<()> {
        let mut s = self.state();
        s.calls.push(format!("schedule_remove {}", e.name));
        s.schedule.remove(&e.name);
        Ok(())
    }
    fn schedule_elevation(&mut self, _e: &ServerEntry, _set: Option<&ScheduleConfig>) -> Option<Vec<String>> {
        self.state().elevation.clone()
    }
    fn ports_set(&mut self, e: &ServerEntry, p: Ports) -> Res<Vec<String>> {
        let mut s = self.state();
        s.calls.push(format!("ports {} {} {}", e.name, p.game, p.txadmin));
        if let Some(x) = &s.ports_error {
            return Err(x.clone());
        }
        if let Some(x) = s.servers.iter_mut().find(|x| x.name == e.name) {
            x.ports = PortsRecord { game: p.game, txadmin: p.txadmin };
        }
        Ok(vec!["Firewall rules were not changed.".into()])
    }
    fn reach_check(&mut self, e: &ServerEntry, consent: Option<crate::reach::settings::Consent>) -> Res<crate::reach::ReachReport> {
        let mut s = self.state();
        s.calls.push(format!("reach_check {} {consent:?}", e.name));
        let on = s.forwarding.get(&e.name).copied().unwrap_or(false);
        let mut f = crate::reach::sample_facts(&e.name, e.ports.game);
        f.forwarding.enabled = on;
        Ok(s.reach_report.clone().unwrap_or_else(|| crate::reach::assess(&f)))
    }
    fn reach_consent(&mut self) -> crate::reach::settings::Consent {
        self.state().reach_consent
    }
    fn reach_consent_set(&mut self, c: crate::reach::settings::Consent) -> Res<()> {
        let mut s = self.state();
        s.calls.push(format!("reach_consent_set {c:?}"));
        s.reach_consent = c;
        Ok(())
    }
    fn forwarding_set(&mut self, e: &ServerEntry, on: bool) -> Res<crate::reach::ForwardingView> {
        let mut s = self.state();
        s.calls.push(format!("forwarding_set {} {on}", e.name));
        s.forwarding.insert(e.name.clone(), on);
        Ok(crate::reach::ForwardingView { enabled: on, status: None })
    }
    fn firewall_open(&mut self, e: &ServerEntry) -> Res<Vec<String>> {
        let mut s = self.state();
        s.calls.push(format!("firewall_open {}", e.name));
        match &s.firewall_open_error {
            Some(x) => Err(x.clone()),
            None => Ok(vec![format!("Opened port {} (TCP and UDP) in the firewall.", e.ports.game)]),
        }
    }
    fn assets_list(&mut self, e: &ServerEntry) -> Res<AssetsView> {
        Ok(self.state().assets.get(&e.name).cloned().unwrap_or_default())
    }
    fn asset_inspect(&mut self, e: &ServerEntry, zip: &Path) -> Res<ImportPlan> {
        let mut s = self.state();
        // The zip's name and size, so tests can check what reached the backend.
        let size = std::fs::metadata(zip).map(|m| m.len()).unwrap_or(0);
        s.calls.push(format!("asset_inspect {} {} {size}", e.name, file_name(zip)));
        s.asset_plan.clone().unwrap_or_else(|| {
            let name = zip.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            Ok(ImportPlan {
                zip_name: file_name(zip),
                resources: vec![FoundResource { name, zip_path: String::new(), has_fxmanifest: true, has_fxap: true, escrowed: true, size, files: 1, version: None, replaces: false }],
                problems: vec![],
                total_size: size,
                can_import: true,
            })
        })
    }
    fn asset_import(&mut self, e: &ServerEntry, zip: &Path, o: &ImportOpts, say: &mut dyn FnMut(&str)) -> Res<ImportReport> {
        let result = {
            let mut s = self.state();
            s.calls.push(format!("asset_import {} {} replace={}", e.name, file_name(zip), o.replace));
            s.asset_import.clone()
        };
        say("Unpacking");
        result.unwrap_or_else(|| {
            let name = zip.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            Ok(ImportReport { zip_name: file_name(zip), imported: vec![ImportedAsset { name: name.clone(), version: None, replaced: false, backup: None }], ensured: vec![name], notes: vec![] })
        })
    }
    fn asset_remove(&mut self, e: &ServerEntry, name: &str) -> Res<Vec<String>> {
        let mut s = self.state();
        s.calls.push(format!("asset_remove {} {name}", e.name));
        if let Some(x) = &s.asset_remove_error {
            return Err(x.clone());
        }
        let Some(v) = s.assets.get_mut(&e.name).filter(|v| v.assets.iter().any(|a| a.name == name)) else {
            return Err(format!("There is no paid asset called {name} on this server (not found)."));
        };
        v.assets.retain(|a| a.name != name);
        Ok(vec![format!("{name} was removed.")])
    }

    // ---- backups: canned answers; every call is recorded.
    fn backups_view(&mut self, e: &ServerEntry) -> Res<crate::api::wire::BackupsView> {
        self.state().calls.push(format!("backups_view {}", e.name));
        let info = sample_backup(&e.name, "20260317-041507", false);
        let settings = crate::backup::BackupSettings::default();
        let summary = crate::backup::BackupSummary { last_ok_unix: Some(info.created_unix), text: "Last backup 3 days ago".into(), warning: None, scheduled: false };
        Ok(crate::api::wire::BackupsView {
            backups: vec![info],
            settings,
            summary,
            folder: "/home/me/.kora/server-backups/prod".into(),
            default_folder: true,
            free_bytes: Some(50 << 30),
            undo: None,
            database: crate::api::wire::DatabaseNote { kind: crate::backup::DbKind::Dumped, note: "The server's own database is copied as one consistent snapshot.".into(), tool_missing: false },
            schedule_note: "Scheduled backups run while Kora's background service is running.".into(),
        })
    }
    fn backups_overview(&mut self) -> Res<crate::api::wire::BackupsOverview> {
        let servers = self.state().servers.iter().map(|e| e.name.clone()).collect::<Vec<_>>();
        Ok(crate::api::wire::BackupsOverview {
            servers: servers
                .into_iter()
                .map(|server| crate::api::wire::ServerBackupLine {
                    server,
                    summary: crate::backup::BackupSummary { last_ok_unix: None, text: "No backup yet".into(), warning: None, scheduled: false },
                })
                .collect(),
        })
    }
    fn backup_create(&mut self, e: &ServerEntry, req: &crate::api::wire::BackupCreateRequest, admin: bool, say: &mut dyn FnMut(crate::backup::BackupEvent)) -> Res<crate::api::wire::BackupCreated> {
        self.state().calls.push(format!("backup_create {} encrypted={} everything={} stop={} admin={admin}", e.name, req.passphrase.is_some(), req.everything, req.stop_server));
        say(crate::backup::BackupEvent { step: "database".into(), message: "Copying the database".into(), percent: 30 });
        let info = sample_backup(&e.name, "20260320-030000", req.passphrase.is_some());
        Ok(crate::api::wire::BackupCreated { info, path: "/home/me/.kora/server-backups/prod/prod-20260320-030000.tar.gz".into(), notes: vec![], pruned: vec![] })
    }
    fn backup_settings_set(&mut self, e: &ServerEntry, s: &crate::backup::BackupSettings) -> Res<crate::backup::BackupSettings> {
        self.state().calls.push(format!("backup_settings_set {} schedule={}", e.name, s.schedule.enabled));
        Ok(s.clone())
    }
    fn backup_verify(&mut self, e: &ServerEntry, id: &str, _p: Option<&str>, say: &mut dyn FnMut(crate::backup::BackupEvent)) -> Res<crate::api::wire::VerifyResult> {
        self.state().calls.push(format!("backup_verify {} {id}", e.name));
        say(crate::backup::BackupEvent { step: "verify".into(), message: "Checking the file".into(), percent: 50 });
        Ok(crate::api::wire::VerifyResult { ok: true, files: 12, bytes: 4096, summary: "All 12 files match their checksums.".into() })
    }
    fn backup_preview(&mut self, e: &ServerEntry, id: &str, p: Option<&str>) -> Res<crate::backup::restore::RestorePreview> {
        self.state().calls.push(format!("backup_preview {} {id} password={}", e.name, p.is_some()));
        Ok(crate::backup::restore::RestorePreview {
            id: id.into(),
            created_unix: 1_773_700_000,
            encrypted: false,
            needs_passphrase: false,
            server: None,
            database: None,
            resources: vec![],
            stopped: false,
            server_running: true,
            will_change: vec!["The server's settings".into()],
            will_keep: vec!["secrets.cfg and database.json stay as they are now".into()],
            warnings: vec![],
        })
    }
    fn backup_restore(&mut self, e: &ServerEntry, id: &str, p: Option<&str>, say: &mut dyn FnMut(crate::backup::BackupEvent)) -> Res<crate::backup::restore::RestoreReport> {
        self.state().calls.push(format!("backup_restore {} {id} password={}", e.name, p.is_some()));
        say(crate::backup::BackupEvent { step: "files".into(), message: "Putting the configuration back".into(), percent: 70 });
        Ok(crate::backup::restore::RestoreReport {
            safety_backup: Some("20260320-040000".into()),
            database_restored: true,
            files_restored: 4,
            resources_restored: vec![],
            problems: vec![],
            notes: vec![],
            restarted: true,
        })
    }
    fn backup_undo(&mut self, e: &ServerEntry, _p: Option<&str>, _say: &mut dyn FnMut(crate::backup::BackupEvent)) -> Res<crate::backup::restore::RestoreReport> {
        self.state().calls.push(format!("backup_undo {}", e.name));
        Ok(crate::backup::restore::RestoreReport { safety_backup: None, database_restored: true, files_restored: 4, resources_restored: vec![], problems: vec![], notes: vec![], restarted: false })
    }
    fn backup_delete(&mut self, e: &ServerEntry, id: &str) -> Res<()> {
        self.state().calls.push(format!("backup_delete {} {id}", e.name));
        Ok(())
    }
    fn backup_prune(&mut self, e: &ServerEntry) -> Res<Vec<String>> {
        self.state().calls.push(format!("backup_prune {}", e.name));
        Ok(vec!["20260101-000000".into()])
    }
    fn backups_archived(&mut self) -> Res<crate::api::wire::ArchivedBackups> {
        self.state().calls.push("backups_archived".into());
        Ok(crate::api::wire::ArchivedBackups { servers: vec![crate::api::wire::ArchivedServer { name: "Old RP".into(), deleted_unix: 1_773_000_000, backups: vec![sample_backup("Old RP", "20260301-030000", false)] }] })
    }
    fn import_inspect(&mut self, src: &crate::backup::import::ImportSource, passphrase: Option<&str>, allow_path: bool) -> Res<crate::backup::import::ImportPreview> {
        self.state().calls.push(format!("import_inspect path={} upload={} password={} local={allow_path}", src.path.is_some(), src.upload.is_some(), passphrase.is_some()));
        Ok(crate::backup::import::ImportPreview {
            file_name: "prod-20260320-030000.tar.gz.age".into(),
            encrypted: true,
            needs_passphrase: false,
            created_unix: 1_773_700_000,
            server: Some(crate::backup::ServerInfo {
                name: "Old server".into(),
                mode: "production".into(),
                source: crate::installer::Source::Fork,
                versions: crate::installer::Versions::Latest,
                game_port: 30120,
                txadmin_port: 40120,
                game_build: 3258,
                artifact_build: 17000,
                os: "windows".into(),
                dir: "C:\\Servers\\old".into(),
                github_owner: Some("old-owner".into()),
            }),
            database: Some(crate::backup::DatabaseBackup {
                kind: crate::backup::DbKind::Dumped,
                name: Some("kora_ab12".into()),
                host: Some("127.0.0.1".into()),
                port: Some(3306),
                dump_bytes: 9_000_000,
                note: String::new(),
            }),
            by_reference: 40,
            in_full: 2,
            suggested: Some(crate::backup::import::ImportChoices { name: "Old server".into(), dir: "/home/me/kora/old-server".into(), game_port: 30121, txadmin_port: 40121 }),
            clashes: vec![crate::backup::import::Clash { kind: "game_port".into(), message: "The game port 30120 is already used on this computer.".into(), suggestion: Some("30121".into()) }],
            will_create: vec!["A new server \"Old server\"".into()],
            wont_carry: vec!["The operating-system service".into()],
            warnings: vec!["This server was made on a different operating system.".into()],
            saved_keys: vec!["Main".into()],
            licence_note: crate::backup::import::LICENCE_NOTE.into(),
            db_login: Some(crate::backup::import::DbLoginNote { ok: false, message: "Kora could not log in to the database as root.".into() }),
            fork: Some(crate::backup::import::ForkNote { owner: "old-owner".into(), usable: false, signed_in_as: Some("me".into()), message: "This server follows copies (forks) in the GitHub account old-owner, which the account signed in here (me) cannot use. Choose what to do about them below.".into() }),
        })
    }
    fn import_run(&mut self, req: &crate::backup::import::ImportRequest, allow_path: bool, say: &mut dyn FnMut(crate::backup::BackupEvent)) -> Res<crate::backup::import::ImportResult> {
        self.state().calls.push(format!("import_run {} dir={} local={allow_path}", req.choices.name, req.choices.dir));
        say(crate::backup::BackupEvent { step: "database".into(), message: "Creating a new database and login".into(), percent: 50 });
        Ok(crate::backup::import::ImportResult {
            name: req.choices.name.clone(),
            dir: req.choices.dir.clone(),
            resources_fetched: vec![],
            problems: vec![],
            notes: vec![],
            started: false,
            next_steps: vec!["Open the ports".into()],
            old_machine_steps: vec!["Remove it from the OLD computer".into()],
        })
    }
    fn support_preview(&mut self, server: Option<&str>) -> Res<crate::api::wire::BundleListing> {
        self.state().calls.push(format!("support_preview {}", server.unwrap_or("-")));
        let mut b = crate::support::bundle::tests_bundle_for_fake();
        b.id = "20260320-030000".into();
        Ok(crate::support::service::listing(b))
    }
    fn support_summary(&mut self, server: Option<&str>) -> Res<String> {
        self.state().calls.push(format!("support_summary {}", server.unwrap_or("-")));
        Ok("--- about.txt
Kora 0.8.0
"
        .into())
    }
    fn backup_file(&mut self, e: &ServerEntry, id: &str) -> Res<PathBuf> {
        self.state().calls.push(format!("backup_file {} {id}", e.name));
        Err("The fake has no archive files.".into())
    }
}

/// A backup listing for the fake.
pub fn sample_backup(server: &str, id: &str, encrypted: bool) -> crate::backup::BackupInfo {
    crate::backup::BackupInfo {
        id: id.into(),
        file: format!("{server}-{id}.tar.gz{}", if encrypted { ".age" } else { "" }),
        bytes: 12_345_678,
        sha256: "ab".repeat(32),
        encrypted,
        created_unix: 1_773_700_000,
        label: crate::backup::Label::Manual,
        kora_version: env!("CARGO_PKG_VERSION").into(),
        server: server.into(),
        database: crate::backup::DbKind::Dumped,
        by_reference: 40,
        in_full: 2,
        stopped: false,
        too_new: false,
    }
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_is_send_and_shares_state() {
        fn is_send<T: Send + Sync>() {}
        is_send::<FakeBackend>();
        let f = FakeBackend::sample();
        let mut g = f.clone();
        let e = g.list().unwrap().into_iter().find(|e| e.name == "dev").unwrap();
        g.control(&e, ControlAction::Start).unwrap();
        assert_eq!(f.calls(), vec!["Start dev"]);
        assert_eq!(f.clone().status(&e), Status::Running);
    }
}
