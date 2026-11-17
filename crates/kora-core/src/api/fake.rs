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
}

/// Cheap to clone; clones share state. Give `move || Box::new(fake.clone())` to the panel.
#[derive(Clone, Default)]
pub struct FakeBackend(pub Arc<Mutex<FakeState>>);

/// A registry entry for tests.
pub fn entry(name: &str, mode: Mode, game: u16, txadmin: u16) -> ServerEntry {
    ServerEntry { name: name.into(), dir: PathBuf::from(format!("/srv/{name}")), mode, ports: PortsRecord { game, txadmin }, created_unix: 0 }
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
        s.preview.clone().unwrap_or_else(|| {
            Ok(UpdatePreview {
                resources: vec![],
                new_release: None,
                notes: vec![],
            })
        })
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
    fn apply(
        &mut self,
        e: &ServerEntry,
        scope: Scope,
        o: ApplyOpts,
        on: &mut dyn FnMut(UpdateEvent),
    ) -> Res<UpdateReport> {
        let (events, result) = {
            let mut s = self.state();
            let targets = o
                .targets
                .as_ref()
                .map(|t| format!(" targets={t:?}"))
                .unwrap_or_default();
            s.calls.push(format!(
                "apply {} {scope:?} restart={:?}{targets}",
                e.name, o.restart
            ));
            (
                s.apply_events.clone(),
                s.apply
                    .clone()
                    .unwrap_or_else(|| Ok(UpdateReport::default())),
            )
        };
        for ev in events {
            on(ev);
        }
        result
    }
    fn rollback(&mut self, e: &ServerEntry, snapshot: &str, force: bool) -> Res<()> {
        let mut s = self.state();
        s.calls.push(format!(
            "rollback {} {snapshot}{}",
            e.name,
            if force { " force" } else { "" }
        ));
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
