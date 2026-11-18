//! Kora itself: self-update, the Kora app and uninstall, behind one trait so the panel can be tested
//! without touching the machine. [`current`] is the real thing unless a test installed an override.
//!
//! The logic is the same as `kora self-update`, `kora app ...` and `kora uninstall`; this module only
//! reshapes it into values an HTTP handler can return.

use crate::problem::Problem;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::app_install::{self as app, AppOptions};
use crate::installer::registry::{self, Registry};
use crate::installer::system::{self, Kind, Target};
use crate::installer::uninstall as un;
use crate::installer::{self, load_created_from, load_request_from, slug, ProgressEvent, ProgressSink, RollbackOptions};
use crate::self_update::{self as su, release::select_assets, release::this_asset_name, replace::Flow, replace::RealFs, Decision, Source};
use crate::service::{Platform, SystemRunner};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfUpdateInfo {
    pub current: String,
    pub latest: Option<String>,
    pub available: bool,
    /// Published versions, newest first.
    pub versions: Vec<String>,
}

/// The "Kora app" as the panel shows it. Since 0.7.0 the app is the panel plus per-user shortcuts;
/// `installed`/`version`/`path`/`desktop_shortcut` describe the OLD machine-wide copy (`legacy`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppStatusView {
    /// The old machine-wide app is installed (`%ProgramFiles%\Kora`, `/opt/kora`).
    pub installed: bool,
    pub version: Option<String>,
    pub path: Option<String>,
    pub desktop_shortcut: bool,
    pub supported: bool,
    /// The old app, its machine-wide shortcuts or its administrator launchers are still here: the panel
    /// offers "Tidy up old shortcuts".
    pub legacy: bool,
    /// What the tidy-up would remove, one plain sentence each.
    pub legacy_items: Vec<String>,
    /// The per-user shortcut (Start Menu / launcher) is in place.
    pub shortcuts: bool,
}

/// One thing the uninstall would (or would not) remove.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanItem {
    pub item: String,
    pub description: String,
    /// `system`, `folder`, `database` or `app`.
    pub kind: &'static str,
    pub remove: bool,
}

#[derive(Debug, Clone, Default)]
pub struct UninstallRequest {
    pub answers: Vec<(String, bool)>,
    pub delete_files: bool,
    pub delete_databases: bool,
    /// Registry names; empty = every registered server.
    pub only: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UninstallOutcome {
    pub removed: Vec<String>,
    pub kept: Vec<String>,
    pub failed: Vec<String>,
}

pub trait Maintenance: Send + Sync {
    fn is_admin(&self) -> bool;
    fn self_update_info(&self) -> Result<SelfUpdateInfo, Problem>;
    /// The program would be replaced in a folder this user cannot write to.
    fn self_update_needs_admin(&self) -> bool;
    /// Download, verify and swap the program. Returns the new version.
    fn self_update_apply(&self, version: Option<&str>, status: &mut dyn FnMut(&str)) -> Result<String, Problem>;
    fn app_status(&self) -> AppStatusView;
    /// Make the per-user shortcuts (no administrator rights; `force` is ignored). Lines describing the
    /// changes made (or, with `dry_run`, that would be).
    fn app_install(&self, opts: &AppOptions) -> Result<Vec<String>, Problem>;
    fn app_uninstall(&self, dry_run: bool) -> Result<Vec<String>, Problem>;
    fn uninstall_plan(&self, only: Option<&str>) -> Result<Vec<PlanItem>, Problem>;
    fn uninstall_run(&self, req: &UninstallRequest, status: &mut dyn FnMut(&str)) -> Result<UninstallOutcome, Problem>;
    /// The server folders an uninstall `req` would delete: `(name, folder)`.
    fn uninstall_folders(&self, req: &UninstallRequest) -> Vec<(String, PathBuf)> {
        folders_to_delete(&Registry::load().unwrap_or_default(), req)
    }
    /// Delete what Kora put in a server folder, as the person (never elevated: the folder is
    /// theirs, and an elevated delete in a folder others can change is a risk). Notes.
    fn delete_server_files(&self, dir: &Path) -> Result<Vec<String>, Problem> {
        installer::rollback(dir, &RollbackOptions { keep_files: false, keep_db: true }, &mut NullSink)
    }
}

/// Which registered servers' folders `req` deletes (everything, `only`, `--delete-files` or the
/// `<name>/folder` answers).
pub fn folders_to_delete(reg: &Registry, req: &UninstallRequest) -> Vec<(String, PathBuf)> {
    reg.servers
        .iter()
        .filter(|s| req.only.is_empty() || req.only.iter().any(|o| matches_only(s, o)))
        .filter(|s| req.delete_files || req.answers.iter().any(|(i, r)| *r && *i == format!("{}/folder", s.name)))
        .map(|s| (s.name.clone(), s.dir.clone()))
        .collect()
}

static OVERRIDE: RwLock<Option<Arc<dyn Maintenance>>> = RwLock::new(None);

/// Test seam: use `m` instead of the real machine (`None` restores the real one).
pub fn set_override(m: Option<Arc<dyn Maintenance>>) {
    *OVERRIDE.write().unwrap_or_else(|e| e.into_inner()) = m;
}

pub fn current() -> Arc<dyn Maintenance> {
    OVERRIDE.read().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_else(|| Arc::new(SystemMaintenance))
}

/// Would `kora app uninstall` (not a dry run) ask for administrator rights? Same test as the CLI.
pub fn app_change_needs_admin(m: &dyn Maintenance, dry_run: bool) -> bool {
    !dry_run && !m.is_admin() && std::env::var_os("KORA_APP_PREFIX").is_none()
}

/// Does the tidy-up still have a machine-wide part that this user cannot do (removing the old copy)?
pub fn tidy_needs_admin(m: &dyn Maintenance, dry_run: bool) -> bool {
    m.app_status().legacy && app_change_needs_admin(m, dry_run)
}

pub struct SystemMaintenance;

struct NullSink;
impl ProgressSink for NullSink {
    fn event(&mut self, _ev: ProgressEvent) {}
}

fn kind_word(k: Kind) -> &'static str {
    match k {
        Kind::UpdateTimer => "timer",
        Kind::BackupJob => "backups",
        Kind::Service => "service",
        Kind::Firewall => "firewall",
        Kind::ServiceAccount => "account",
        Kind::Binary => "program",
    }
}

fn matches_only(s: &registry::ServerEntry, only: &str) -> bool {
    s.name == only || slug(&s.name) == only || registry::same_path(&s.dir, Path::new(only))
}

fn question_of(title: &str) -> String {
    format!("{}?", title.trim_end_matches('?'))
}

/// The system items for one server, as `(plan item, the question `uninstall::run` asks for it)`.
fn server_system_items(reg: &Registry, name: &str, dir: &Path, keep_files: bool) -> Result<Vec<(PlanItem, String)>, Problem> {
    let (Some(req), Some(created)) = (load_request_from(dir), load_created_from(dir)) else {
        return Ok(vec![]);
    };
    let target = Target::from_request(&req);
    let used = created.binary.as_deref().is_some_and(|b| system::binary_used_elsewhere(reg, b, &[dir]));
    let mut items = system::items_for(&target, &created, Platform::current(), &SystemRunner, keep_files, used).map_err(|e| e.lead(name))?;
    items.extend(system::backup_job_item(&target, Platform::current(), &SystemRunner));
    Ok(items
        .into_iter()
        .map(|i| {
            let mut d = i.title.trim_end_matches('?').to_string();
            if let Some(n) = &i.note {
                d = format!("{d} ({n})");
            }
            (PlanItem { item: format!("{name}/{}", kind_word(i.kind)), description: d, kind: "system", remove: true }, question_of(&i.title))
        })
        .collect())
}

struct AnswerAsker {
    /// Question → item id.
    questions: HashMap<String, String>,
    answers: HashMap<String, bool>,
    forget: String,
}

impl un::Asker for AnswerAsker {
    fn ask(&mut self, question: &str, default_yes: bool) -> bool {
        if let Some(id) = self.questions.get(question) {
            return self.answers.get(id).copied().unwrap_or(default_yes);
        }
        if question.starts_with(&self.forget) {
            return self.answers.get(&format!("{}/registry", self.forget_name())).copied().unwrap_or(default_yes);
        }
        default_yes
    }
}

impl AnswerAsker {
    fn forget_name(&self) -> &str {
        self.forget.trim_start_matches("Forget '").trim_end_matches('\'')
    }
}

impl Maintenance for SystemMaintenance {
    fn is_admin(&self) -> bool {
        installer::is_admin()
    }

    fn self_update_info(&self) -> Result<SelfUpdateInfo, Problem> {
        let cur = su::running_version();
        let releases = su::fetch_releases(&Source::from_env()).map_err(|e| e.to_string())?;
        let (latest, available) = match su::decide(&cur, &releases, None).map_err(|e| e.to_string())? {
            Decision::UpToDate { latest } => (latest.to_string(), false),
            Decision::Update(r) => (r.version().map(|v| v.to_string()).unwrap_or(r.tag_name), true),
        };
        let mut vs: Vec<_> = releases.iter().filter(|r| !r.draft).filter_map(|r| r.version()).collect();
        vs.sort();
        vs.reverse();
        Ok(SelfUpdateInfo { current: cur.to_string(), latest: Some(latest), available, versions: vs.iter().map(ToString::to_string).collect() })
    }

    fn self_update_needs_admin(&self) -> bool {
        let Ok(running) = std::env::current_exe() else {
            return false;
        };
        let target = su::resolve_target(&running, cfg!(windows), su::installed_app());
        let dir = target.exe.parent().map(Path::to_path_buf).unwrap_or_default();
        !su::dir_writable(&dir) && !crate::elevate::is_elevated()
    }

    fn self_update_apply(&self, version: Option<&str>, status: &mut dyn FnMut(&str)) -> Result<String, Problem> {
        let cur = su::running_version();
        let src = Source::from_env();
        status("Looking for the release");
        let releases = su::fetch_releases(&src).map_err(|e| e.to_string())?;
        let rel = match su::decide(&cur, &releases, version).map_err(|e| e.to_string())? {
            Decision::UpToDate { latest } => return Ok(latest.to_string()),
            Decision::Update(r) => r,
        };
        let new = rel.version().ok_or("that release has no version number")?;
        let asset = this_asset_name().ok_or_else(|| su::SelfUpdateError::UnsupportedPlatform.to_string())?;
        select_assets(&rel, asset).map_err(|e| format!("{e}. Nothing was changed."))?;
        let running = std::env::current_exe().map_err(|e| format!("cannot find the kora program: {e}"))?;
        let target = su::resolve_target(&running, cfg!(windows), su::installed_app());
        status("Downloading and checking the new version");
        let a = su::apply(&src, &rel, asset, &target.exe, &RealFs, Flow::native()).map_err(|e| e.to_string())?;
        if let Some(s) = &target.state {
            // Not fatal: the program itself was replaced.
            let _ = su::update_app_state(s, &new, &a.sha256);
        }
        Ok(new.to_string())
    }

    fn app_status(&self) -> AppStatusView {
        let st = app::status();
        let legacy = app::user::legacy_present();
        AppStatusView {
            installed: st.installed,
            version: st.version.clone(),
            path: st.installed.then(|| st.exe.display().to_string()),
            desktop_shortcut: st.desktop.unwrap_or(false),
            supported: app::supported(),
            legacy: legacy.present(),
            legacy_items: legacy.items,
            shortcuts: app::user::user_status().in_place(),
        }
    }

    fn app_install(&self, opts: &AppOptions) -> Result<Vec<String>, Problem> {
        let out = app::user::install_user(&app::user::UserOptions { desktop_shortcut: opts.desktop_shortcut, dry_run: opts.dry_run }).map_err(|e| e.to_string())?;
        let mut lines = out.changes.clone();
        lines.extend(out.notes.iter().map(|n| format!("note: {n}")));
        Ok(lines)
    }

    fn app_uninstall(&self, dry_run: bool) -> Result<Vec<String>, Problem> {
        app::uninstall_app_opts(dry_run).map_err(Problem::from)
    }

    fn uninstall_plan(&self, only: Option<&str>) -> Result<Vec<PlanItem>, Problem> {
        let reg = Registry::load().map_err(|e| e.lead("cannot read the server list"))?;
        let mut out = Vec::new();
        for s in reg.servers.iter().filter(|s| only.is_none_or(|o| matches_only(s, o))) {
            let recorded = load_request_from(&s.dir).is_some() && load_created_from(&s.dir).is_some();
            out.extend(server_system_items(&reg, &s.name, &s.dir, true)?.into_iter().map(|(i, _)| i));
            out.push(PlanItem {
                item: format!("{}/registry", s.name),
                description: format!("Forget '{}' in the Kora server list (its folder and database stay)", s.name),
                kind: "system",
                remove: recorded,
            });
            if !recorded {
                continue;
            }
            out.push(PlanItem { item: format!("{}/folder", s.name), description: format!("Delete the server folder {} and what Kora put in it", s.dir.display()), kind: "folder", remove: false });
            if dir_has_db(&s.dir) {
                out.push(PlanItem { item: format!("{}/database", s.name), description: format!("Drop the database and database user Kora created for '{}'", s.name), kind: "database", remove: false });
            }
        }
        if only.is_none() && (self.app_status().installed || app::user::user_status().any()) {
            out.push(PlanItem {
                item: "app".into(),
                description: "Kora's shortcuts (and the old machine-wide Kora app, if it is still installed). Your servers are not touched".into(),
                kind: "app",
                remove: false,
            });
        }
        if only.is_none() && !crate::tools::private_present().is_empty() {
            out.push(PlanItem {
                item: "tools".into(),
                description: format!("Kora's own copies of {} (in Kora's folder; nothing else on this computer changes)", crate::tools::private_present().join(" and ")),
                kind: "system",
                remove: true,
            });
        }
        Ok(out)
    }

    fn uninstall_run(&self, req: &UninstallRequest, status: &mut dyn FnMut(&str)) -> Result<UninstallOutcome, Problem> {
        let mut reg = Registry::load().map_err(|e| e.lead("cannot read the server list"))?;
        let answers: HashMap<String, bool> = req.answers.iter().cloned().collect();
        let admin = installer::is_admin();
        let runner = SystemRunner;
        let env = un::Env { platform: Platform::current(), runner: &runner, admin };
        let names: Vec<(String, std::path::PathBuf)> =
            reg.servers.iter().filter(|s| req.only.is_empty() || req.only.iter().any(|o| matches_only(s, o))).map(|s| (s.name.clone(), s.dir.clone())).collect();
        let all_names: Vec<String> = reg.servers.iter().map(|s| s.name.clone()).collect();
        let mut sum = un::Summary::default();
        for (name, dir) in &names {
            let del_files = req.delete_files || answers.get(&format!("{name}/folder")).copied().unwrap_or(false);
            let del_db = req.delete_databases || answers.get(&format!("{name}/database")).copied().unwrap_or(false);
            let questions = server_system_items(&reg, name, dir, !del_files)?.into_iter().map(|(i, q)| (q, i.item)).collect();
            let mut asker = AnswerAsker { questions, answers: answers.clone(), forget: format!("Forget '{name}'") };
            let opts = un::Options { dry_run: false, only: Some(name.clone()), delete_files: del_files, delete_databases: del_db };
            let mut say = |s: &str| {
                let t = s.trim();
                if !t.is_empty() {
                    status(t);
                }
            };
            let mut delete = |d: &Path, o: &RollbackOptions| installer::rollback(d, o, &mut NullSink);
            // Only the servers this run took out of its list are removed, inside one transaction: a server that was
            // registered meanwhile (the panel, another `kora` command) is never dropped by writing a stale whole list.
            let before: Vec<String> = all_names.clone();
            let mut save = |r: &Registry| {
                let removed: Vec<&String> = before.iter().filter(|n| !r.servers.iter().any(|s| &s.name == *n)).collect();
                Registry::update(|cur| {
                    cur.servers.retain(|s| !removed.contains(&&s.name));
                    Ok(())
                })
            };
            let s = un::run(&mut reg, &opts, &env, &mut asker, &mut say, &mut delete, &mut save);
            sum.removed.extend(s.removed);
            sum.kept.extend(s.kept);
            sum.failed.extend(s.failed);
        }
        if req.only.is_empty() {
            if let Some(base) = installer::windows_db::base_dir() {
                let record = installer::windows_db::load_record(&base).map(|r| (r, base.clone()));
                let opts = un::Options { dry_run: false, only: None, delete_files: req.delete_files, delete_databases: req.delete_databases };
                let mut asker = un::Defaults;
                let mut say = |s: &str| {
                    let t = s.trim();
                    if !t.is_empty() {
                        status(t);
                    }
                };
                let mut remove = |r: &installer::windows_db::Record, b: &Path| installer::windows_db::remove(r, b, &mut |_m| {});
                un::mariadb_cleanup(&reg, &opts, admin, record, &mut asker, &mut say, &mut remove, &mut sum);
            }
        }
        if req.only.is_empty() && answers.get("tools").copied().unwrap_or(false) {
            status("Removing Kora's own copies of Git and the GitHub CLI");
            match crate::tools::remove_private() {
                Ok(gone) if !gone.is_empty() => sum.removed.push(format!("Kora's own copies of {}", gone.join(" and "))),
                Ok(_) => {}
                Err(e) => sum.failed.push(format!("Kora's own Git and GitHub CLI copies: {e}")),
            }
        }
        if answers.get("app").copied().unwrap_or(false) {
            status("Removing the Kora shortcuts");
            match app::user::uninstall_user(false) {
                Ok(lines) if !lines.is_empty() => sum.removed.push("Kora's shortcuts".into()),
                Ok(_) => {}
                Err(e) => sum.failed.push(format!("Kora's shortcuts: {e}")),
            }
            if app::user::legacy_present().present() {
                status("Removing the old Kora app");
                match app::uninstall_app_opts(false) {
                    Ok(_) => sum.removed.push("the old Kora app".into()),
                    Err(e) => sum.failed.push(format!("the old Kora app: {e}")),
                }
            }
        }
        Ok(UninstallOutcome { removed: sum.removed, kept: sum.kept, failed: sum.failed })
    }
}

fn dir_has_db(dir: &Path) -> bool {
    match load_request_from(dir) {
        Some(r) => !matches!(r.db, installer::DbChoice::Skip) && dir.join(".kora").join("db-provision.json").is_file(),
        None => false,
    }
}

/// A scripted [`Maintenance`] for tests: records calls, changes nothing.
#[cfg(any(test, feature = "fake"))]
pub mod fake {
    use super::*;
    use std::sync::Mutex;

    pub struct FakeMaintenance {
        pub admin: bool,
        pub update: Mutex<Result<SelfUpdateInfo, Problem>>,
        pub needs_admin: bool,
        pub app: Mutex<AppStatusView>,
        pub plan: Mutex<Vec<PlanItem>>,
        pub outcome: Mutex<Result<UninstallOutcome, Problem>>,
        pub calls: Mutex<Vec<String>>,
    }

    impl FakeMaintenance {
        pub fn new(admin: bool) -> Arc<Self> {
            Arc::new(Self {
                admin,
                update: Mutex::new(Ok(SelfUpdateInfo { current: "0.4.0".into(), latest: Some("0.4.1".into()), available: true, versions: vec!["0.4.1".into(), "0.4.0".into()] })),
                needs_admin: false,
                app: Mutex::new(AppStatusView { installed: false, version: None, path: None, desktop_shortcut: false, supported: true, legacy: false, legacy_items: vec![], shortcuts: false }),
                plan: Mutex::new(vec![
                    PlanItem { item: "prod/service".into(), description: "Remove the background service".into(), kind: "system", remove: true },
                    PlanItem { item: "prod/folder".into(), description: "Delete the server folder".into(), kind: "folder", remove: false },
                ]),
                outcome: Mutex::new(Ok(UninstallOutcome { removed: vec!["prod: service".into()], kept: vec![], failed: vec![] })),
                calls: Mutex::new(vec![]),
            })
        }
        pub fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
        fn log(&self, s: String) {
            self.calls.lock().unwrap().push(s);
        }
    }

    impl Maintenance for FakeMaintenance {
        fn is_admin(&self) -> bool {
            self.admin
        }
        fn self_update_info(&self) -> Result<SelfUpdateInfo, Problem> {
            self.update.lock().unwrap().clone()
        }
        fn self_update_needs_admin(&self) -> bool {
            self.needs_admin
        }
        fn self_update_apply(&self, version: Option<&str>, status: &mut dyn FnMut(&str)) -> Result<String, Problem> {
            self.log(format!("self_update_apply {}", version.unwrap_or("-")));
            status("Downloading");
            Ok(version.unwrap_or("0.4.1").to_string())
        }
        fn app_status(&self) -> AppStatusView {
            self.app.lock().unwrap().clone()
        }
        fn app_install(&self, opts: &AppOptions) -> Result<Vec<String>, Problem> {
            self.log(format!("app_install shortcut={} force={} dry={}", opts.desktop_shortcut, opts.force, opts.dry_run));
            Ok(vec!["copy the program".into()])
        }
        fn app_uninstall(&self, dry_run: bool) -> Result<Vec<String>, Problem> {
            self.log(format!("app_uninstall dry={dry_run}"));
            Ok(vec!["remove the program".into()])
        }
        fn uninstall_plan(&self, only: Option<&str>) -> Result<Vec<PlanItem>, Problem> {
            self.log(format!("uninstall_plan {}", only.unwrap_or("-")));
            Ok(self.plan.lock().unwrap().clone())
        }
        fn uninstall_run(&self, req: &UninstallRequest, status: &mut dyn FnMut(&str)) -> Result<UninstallOutcome, Problem> {
            self.log(format!("uninstall_run answers={:?} files={} db={} only={:?}", req.answers, req.delete_files, req.delete_databases, req.only));
            status("Removing");
            self.outcome.lock().unwrap().clone()
        }
        fn uninstall_folders(&self, req: &UninstallRequest) -> Vec<(String, PathBuf)> {
            let all = ["prod", "dev"];
            all.iter()
                .filter(|n| req.only.is_empty() || req.only.iter().any(|o| o == *n))
                .filter(|n| req.delete_files || req.answers.iter().any(|(i, r)| *r && *i == format!("{n}/folder")))
                .map(|n| (n.to_string(), PathBuf::from(format!("/srv/{n}"))))
                .collect()
        }
        fn delete_server_files(&self, dir: &Path) -> Result<Vec<String>, Problem> {
            self.log(format!("delete_server_files {}", dir.display()));
            Ok(vec![])
        }
    }
}
