//! The real [`Backend`]: registry file, OS services, port probe, txAdmin/FXServer config.
//! Thin on purpose; the rules live in `kora-core`.
//!
//! ## "Ensure a resource"
//! Neither txAdmin nor FXServer exposes a console API Kora can call without credentials (txAdmin's
//! live console needs an admin session; FXServer's RCON is not configured by Kora). The simplest
//! reliable path is therefore: put `ensure <name>` into `<server>/server.cfg` (once; never twice)
//! and, when asked, restart the server's service so it loads. A server that is not running simply
//! loads it on its next start. Names are validated like the installer does (letters, digits,
//! `_ . -`, optionally wrapped in `[ ]` for a category folder).

use crate::problem::Problem;
use std::path::{Path, PathBuf};

use crate::addresources::{self as add, AddOutcome, AddPlan};
use crate::catalog::Catalog;
use crate::feedback::{self, PrPlan};
use crate::firewall::ports::{PortProber, SystemProber};
use crate::git::{self, SystemRunner};
use crate::installer::registry::{self, Registry, ServerEntry, ServerFile};
use crate::installer::{self, host, RollbackOptions};
use crate::launch;
use crate::paidassets;
use crate::service::{ControlAction, ServiceState};

use super::backend::{Backend, Res, Status, StatusDetail, UpdateBadge};
use crate::notify::{self, NotifyConfig};
use crate::service::{self as schedule, ScheduleConfig};
use crate::update::manual::{self, ApplyOpts, Scope, SnapshotInfo, UpdateEvent, UpdatePreview};
use crate::update::UpdateReport;

#[derive(Default)]
pub struct SystemBackend;

/// Is `name` something we may write into server.cfg? It is also used as a folder name, so `.`,
/// `..` (or any all-dots name) and a leading `-` (read as an option by git and other tools) are refused.
pub fn valid_resource_name(v: &str) -> bool {
    let core = v.strip_prefix('[').and_then(|s| s.strip_suffix(']')).unwrap_or(v);
    !core.is_empty() && v.len() <= 100 && !core.starts_with('-') && !core.bytes().all(|b| b == b'.') && core.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

/// `cfg` with `ensure <name>` added at the end, or `None` when it is already ensured.
pub fn with_ensure(cfg: &str, name: &str) -> Option<String> {
    let already = cfg.lines().any(|l| {
        let mut w = l.split_whitespace();
        w.next() == Some("ensure")
            && w.next() == Some(name)
            && match w.next() {
                None => true,
                Some(x) => x.starts_with('#'),
            }
    });
    if already {
        return None;
    }
    let mut out = cfg.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&format!("ensure {name}\n"));
    Some(out)
}

/// `<dir>/.kora/update-status.json`, written by the core updater (`update::status`: auto runs, previews, applies):
/// `{"checked_at": "<RFC3339>", "updates": 3, "attention": 1, "new_release": "1.4.0"|null}`.
/// Older files used booleans for `attention` / `new_release`; both shapes read.
/// A missing, unreadable or older-shaped file means "not known" (`None`) or whatever fields it has.
pub fn read_badge(dir: &Path) -> Option<UpdateBadge> {
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join(".kora").join("update-status.json")).ok()?).ok()?;
    let flag = |k: &str| match v.get(k) {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::String(s)) => !s.is_empty(),
        Some(serde_json::Value::Number(n)) => n.as_f64().is_some_and(|x| x > 0.0),
        _ => false,
    };
    if flag("attention") || flag("needs_attention") {
        return Some(UpdateBadge::NeedsAttention);
    }
    let n = v.get("updates").or_else(|| v.get("behind")).and_then(|x| x.as_u64()).unwrap_or(0) as u32;
    if n > 0 {
        return Some(UpdateBadge::Updates(n));
    }
    if flag("new_release") {
        return Some(UpdateBadge::NewRelease);
    }
    Some(UpdateBadge::UpToDate)
}

/// The record the manual update engine wants (it reads `kora-server.json`).
fn record(e: &ServerEntry) -> Res<manual::ServerRecord> {
    manual::load_server(&e.dir.to_string_lossy()).map_err(Problem::from)
}

struct Quiet;
impl crate::installer::ProgressSink for Quiet {
    fn event(&mut self, _ev: crate::installer::ProgressEvent) {}
}

/// What `control` needs from the machine (fakeable).
trait ControlOps {
    fn has_service(&self, server: &str) -> bool;
    fn start(&self, server: &str) -> Result<launch::Report, Problem>;
    /// `Ok(true)` when something was stopped.
    fn stop_detached(&self, server: &str) -> Result<bool, Problem>;
    fn service_control(&self, server: &str, a: ControlAction) -> Result<(), Problem>;
}

struct SystemControl;

impl ControlOps for SystemControl {
    fn has_service(&self, server: &str) -> bool {
        !matches!(host::service_state(server), Ok(ServiceState::NotInstalled) | Err(_))
    }
    fn start(&self, server: &str) -> Result<launch::Report, Problem> {
        launch::start_named(server, &mut |_, _| {})
    }
    fn stop_detached(&self, server: &str) -> Result<bool, Problem> {
        launch::stop_named_detached(server)
    }
    fn service_control(&self, server: &str, a: ControlAction) -> Result<(), Problem> {
        host::service_control(server, a)
    }
}

fn start_result(server: &str, r: Result<launch::Report, Problem>) -> Res<()> {
    let r = r.map_err(|e| e.lead(&format!("could not start '{server}'")))?;
    if r.running() {
        return Ok(());
    }
    let mut log = String::new();
    let skip = r.log_tail.len().saturating_sub(8);
    for l in r.log_tail.iter().skip(skip) {
        log.push_str("\n  | ");
        log.push_str(l);
    }
    Err(Problem::generic(format!("'{server}' did not start properly: {}", r.message)).with_details(log.trim_start_matches('\n')))
}

/// The CLI's rules (`kora start|stop|restart`): no OS service means Kora runs FXServer itself
/// (detached); with a service, Start still goes through the launcher, Stop/Restart through the service.
fn control_with(ops: &dyn ControlOps, server: &str, a: ControlAction) -> Res<()> {
    let stop_err = |e: Problem| e.lead(&format!("could not stop '{server}'"));
    if !ops.has_service(server) {
        if matches!(a, ControlAction::Stop | ControlAction::Restart) {
            // `Ok(false)`: nothing started by Kora was running; fine for Stop and Restart alike.
            ops.stop_detached(server).map_err(stop_err)?;
            if a == ControlAction::Stop {
                return Ok(());
            }
        }
        return start_result(server, ops.start(server));
    }
    if a == ControlAction::Start {
        return start_result(server, ops.start(server));
    }
    let verb = if a == ControlAction::Stop { "stop" } else { "restart" };
    ops.service_control(server, a).map_err(|e| e.lead(&format!("could not {verb} '{server}'")))
}

/// The folder name (last part of the recorded path) of the resource with this id.
fn folder_of(sf: &ServerFile, id: &str) -> Option<String> {
    let r = sf.resources.iter().find(|r| r.id == id)?;
    r.path.rsplit(['/', '\\']).next().filter(|f| !f.is_empty()).map(str::to_string)
}

impl Backend for SystemBackend {
    fn list(&mut self) -> Res<Vec<ServerEntry>> {
        Registry::load().map(|r| r.servers).map_err(|e| Problem::from(e).lead(&format!("Can't read the server list ({})", registry::registry_path().display())))
    }

    fn status(&mut self, e: &ServerEntry) -> Status {
        if !e.dir.is_dir() {
            return Status::Missing;
        }
        match host::service_state(&e.name) {
            Ok(ServiceState::Running) => return Status::Running,
            Ok(ServiceState::Stopped) => return Status::Stopped,
            _ => {}
        }
        // No service (started by hand, or an adopted server): is something listening on its game port?
        if SystemProber.udp_free(e.ports.game) && SystemProber.tcp_free(e.ports.game) {
            Status::Stopped
        } else {
            Status::Running
        }
    }

    fn badge(&mut self, e: &ServerEntry) -> Option<UpdateBadge> {
        read_badge(&e.dir)
    }

    fn status_detail(&mut self, e: &ServerEntry) -> StatusDetail {
        let spec = launch::Spec { name: e.name.clone(), dir: e.dir.clone(), game_port: e.ports.game, txadmin_port: e.ports.txadmin };
        let st = launch::status_of(&launch::SystemEnv::default(), &spec);
        StatusDetail { running: st.running && e.dir.is_dir(), game_listening: st.game_listening, txadmin_up: st.txadmin_up }
    }

    fn control(&mut self, e: &ServerEntry, a: ControlAction) -> Res<()> {
        if a != ControlAction::Start {
            // So the monitor does not report this stop as a crash.
            crate::monitor::mark_stop_intent(&e.dir);
        }
        control_with(&SystemControl, &e.name, a)?;
        if a != ControlAction::Stop {
            // Everything is loaded fresh: no resource still needs a restart.
            crate::resourcestate::mark_server_restarted(&e.dir);
        }
        Ok(())
    }

    fn resources(&mut self, e: &ServerEntry) -> Vec<String> {
        ServerFile::load(&e.dir).map(|s| s.resources.into_iter().map(|r| r.id).collect()).unwrap_or_default()
    }

    fn resource_folder(&mut self, e: &ServerEntry, id: &str) -> Option<String> {
        folder_of(&ServerFile::load(&e.dir).ok()?, id)
    }

    fn ensure(&mut self, e: &ServerEntry, name: &str, restart: bool) -> Res<String> {
        if !valid_resource_name(name) {
            return Err(Problem::generic(format!("'{name}' is not a resource name (letters, digits, _ . - only).")));
        }
        // The panel lists resources by their catalog id (`kora-lib`); server.cfg needs the folder (`kora_lib`).
        let folder = self.resource_folder(e, name);
        let name = folder.as_deref().unwrap_or(name);
        let cfg_path = e.dir.join("server.cfg");
        let cfg = std::fs::read_to_string(&cfg_path).map_err(|x| Problem::from(x).lead(&format!("Can't read {}", cfg_path.display())))?;
        let mut said = match with_ensure(&cfg, name) {
            Some(new) => {
                std::fs::write(&cfg_path, new).map_err(|x| Problem::from(x).lead(&format!("Can't write {}", cfg_path.display())))?;
                format!("Added 'ensure {name}' to server.cfg.")
            }
            None => format!("'ensure {name}' is already in server.cfg."),
        };
        match self.status(e) {
            Status::Running if restart => {
                self.control(e, ControlAction::Restart)?;
                said.push_str(" The server was restarted.");
            }
            Status::Running => said.push_str(" Restart the server to load it."),
            _ => said.push_str(" It loads the next time the server starts."),
        }
        Ok(said)
    }

    fn delete(&mut self, e: &ServerEntry, keep_files: bool, keep_db: bool) -> Res<Vec<String>> {
        installer::rollback(&e.dir, &RollbackOptions { keep_files, keep_db }, &mut Quiet)
            .map_err(Problem::from)
            .inspect(|_| crate::backup::settings::archive_server(&e.name, crate::monitor::now_unix()))
    }

    fn delete_with_db_password(&mut self, e: &ServerEntry, keep_files: bool, keep_db: bool, db_admin_password: Option<&str>) -> Res<Vec<String>> {
        installer::rollback_with_db_password(&e.dir, &RollbackOptions { keep_files, keep_db }, db_admin_password, &mut Quiet)
            .map_err(Problem::from)
            .inspect(|_| crate::backup::settings::archive_server(&e.name, crate::monitor::now_unix()))
    }

    fn delete_with_admin(&mut self, e: &ServerEntry, keep_files: bool, keep_db: bool, db_admin_password: Option<&str>, admin: Option<installer::system_ops::SharedAdmin>) -> Res<Vec<String>> {
        installer::rollback_with_admin(&e.dir, &RollbackOptions { keep_files, keep_db }, db_admin_password, admin, &mut Quiet)
            .map_err(Problem::from)
            .inspect(|_| crate::backup::settings::archive_server(&e.name, crate::monitor::now_unix()))
    }

    fn unregister(&mut self, e: &ServerEntry) -> Res<()> {
        crate::reach::forward::release_for_dir(&e.dir);
        Registry::update(|r| {
            r.remove(&e.name);
            Ok(())
        })
        .map_err(Problem::from)
    }

    fn register(&mut self, e: ServerEntry) -> Res<()> {
        Registry::update(|r| {
            r.upsert(e);
            Ok(())
        })
        .map_err(Problem::from)
    }

    fn scan_roots(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = Vec::new();
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
        if let Some(h) = home {
            v.push(h);
        }
        if cfg!(windows) {
            v.extend(["C:\\", "D:\\"].iter().map(PathBuf::from).filter(|p| p.is_dir()));
        } else {
            v.extend(["/srv", "/opt", "/home"].iter().map(PathBuf::from).filter(|p| p.is_dir()));
        }
        v
    }

    fn preview(&mut self, e: &ServerEntry, scope: Scope) -> Res<UpdatePreview> {
        manual::preview(&record(e)?, scope).map_err(Problem::from)
    }

    fn preview_events(&mut self, e: &ServerEntry, scope: Scope, on: &mut dyn FnMut(UpdateEvent)) -> Res<UpdatePreview> {
        manual::preview_events(&record(e)?, scope, on).map_err(Problem::from)
    }

    fn apply(&mut self, e: &ServerEntry, scope: Scope, o: ApplyOpts, on: &mut dyn FnMut(UpdateEvent)) -> Res<UpdateReport> {
        // `manual::apply` sends the Discord notification itself: not a second time here.
        manual::apply(&record(e)?, scope, o, on).map_err(Problem::from)
    }

    fn rollback(&mut self, e: &ServerEntry, snapshot: &str, force: bool) -> Res<()> {
        manual::rollback(&record(e)?, snapshot, force).map_err(Problem::from)
    }

    fn snapshots(&mut self, e: &ServerEntry) -> Vec<SnapshotInfo> {
        record(e).map(|r| manual::snapshots(&r)).unwrap_or_default()
    }

    fn add_catalog(&mut self, e: &ServerEntry) -> Res<(Catalog, Vec<String>, Option<String>)> {
        let sf = ServerFile::load(&e.dir).map_err(|x| Problem::from(x).lead(&format!("Can't read {}", e.dir.join("kora-server.json").display())))?;
        let (cat, note) = add::load_catalog().map_err(Problem::from)?;
        Ok((cat, add::installed_ids(&sf), note))
    }

    fn add_plan(&mut self, e: &ServerEntry, ids: &[String]) -> Res<AddPlan> {
        let sf = ServerFile::load(&e.dir).map_err(Problem::from)?;
        let (cat, _) = add::load_catalog().map_err(Problem::from)?;
        add::plan(&cat, &sf, ids).map_err(Problem::from)
    }

    fn add_install(&mut self, e: &ServerEntry, ids: &[String]) -> Res<AddOutcome> {
        let mut sf = ServerFile::load(&e.dir).map_err(Problem::from)?;
        let (cat, _) = add::load_catalog().map_err(Problem::from)?;
        let plan = add::plan(&cat, &sf, ids).map_err(Problem::from)?;
        if plan.items.iter().any(|i| i.fork) {
            feedback::require_sign_in(&SystemRunner).map_err(Problem::from)?;
        }
        add::execute(&add::GitCloner(&SystemRunner), &mut sf, &plan, &mut |_| {}).map_err(Problem::from)
    }

    fn github_signed_in(&mut self) -> bool {
        git::is_signed_in(&SystemRunner)
    }

    fn report_issue(&mut self, e: &ServerEntry, resource: &str, title: &str, text: &str) -> Res<String> {
        let sf = ServerFile::load(&e.dir).map_err(Problem::from)?;
        feedback::report_issue(&SystemRunner, &sf, resource, title, text).map_err(Problem::from)
    }

    fn pr_plan(&mut self, e: &ServerEntry, resource: &str) -> Res<PrPlan> {
        let sf = ServerFile::load(&e.dir).map_err(Problem::from)?;
        feedback::plan_pr(&SystemRunner, &sf, resource).map_err(Problem::from)
    }

    fn pr_open(&mut self, plan: &PrPlan, title: &str, text: &str) -> Res<String> {
        feedback::open_pr(&SystemRunner, plan, title, text).map_err(Problem::from)
    }

    fn notify_load(&mut self, e: &ServerEntry) -> NotifyConfig {
        notify::load(e)
    }

    fn notify_save(&mut self, e: &ServerEntry, c: &NotifyConfig) -> Res<()> {
        notify::save(e, c).map_err(Problem::from)
    }

    fn notify_test(&mut self, e: &ServerEntry, c: &NotifyConfig) -> Res<()> {
        notify::send_test(c, &e.name).map_err(Problem::from)
    }

    fn schedule_get(&mut self, e: &ServerEntry) -> Option<ScheduleConfig> {
        schedule::get(e)
    }

    fn schedule_set(&mut self, e: &ServerEntry, c: &ScheduleConfig) -> Res<()> {
        schedule::set(e, c).map_err(Problem::from)
    }

    fn schedule_remove(&mut self, e: &ServerEntry) -> Res<()> {
        schedule::remove(e).map_err(Problem::from)
    }

    fn ports_set(&mut self, e: &ServerEntry, p: crate::installer::Ports) -> Res<Vec<String>> {
        // Only a stopped server: changing the ports under a running process would leave it on the old ones.
        if self.status(e) != Status::Stopped {
            return Err(Problem::generic("Stop the server first, then change its ports."));
        }
        crate::installer::change_ports::change_ports(&e.dir, &e.name, p, &SystemProber).map(|c| c.notes).map_err(Problem::from)
    }

    fn schedule_elevation(&mut self, e: &ServerEntry, set: Option<&ScheduleConfig>) -> Option<Vec<String>> {
        if crate::elevate::is_elevated() {
            return None;
        }
        Some(match set {
            Some(c) => schedule::set_args(e, c),
            None => vec!["schedule".into(), "off".into(), e.dir.to_string_lossy().into_owned()],
        })
    }

    fn reach_check(&mut self, e: &ServerEntry, consent: Option<crate::reach::settings::Consent>) -> Res<crate::reach::ReachReport> {
        Ok(crate::reach::check(&crate::reach::SystemReach::default(), e, consent))
    }

    fn reach_consent(&mut self) -> crate::reach::settings::Consent {
        crate::reach::settings::load_consent()
    }

    fn reach_consent_set(&mut self, c: crate::reach::settings::Consent) -> Res<()> {
        crate::reach::settings::save_consent(&c)
    }

    fn forwarding_set(&mut self, e: &ServerEntry, on: bool) -> Res<crate::reach::ForwardingView> {
        crate::reach::set_forwarding(e, on)
    }

    fn firewall_open(&mut self, e: &ServerEntry) -> Res<Vec<String>> {
        crate::reach::fw::open_game_port(e)
    }

    fn assets_list(&mut self, e: &ServerEntry) -> Res<paidassets::AssetsView> {
        paidassets::view(&e.dir)
    }

    fn asset_inspect(&mut self, e: &ServerEntry, zip: &Path) -> Res<paidassets::ImportPlan> {
        Ok(paidassets::inspect(&e.dir, zip))
    }

    fn asset_import(&mut self, e: &ServerEntry, zip: &Path, o: &paidassets::ImportOpts, say: &mut dyn FnMut(&str)) -> Res<paidassets::ImportReport> {
        paidassets::import(&e.dir, zip, o, say)
    }

    fn asset_remove(&mut self, e: &ServerEntry, name: &str) -> Res<Vec<String>> {
        paidassets::remove(&e.dir, name)
    }

    fn collab(&mut self, e: &ServerEntry, req: &crate::collab::Request, say: &mut dyn FnMut(usize, usize, &str)) -> Res<crate::collab::Reply> {
        let sf = crate::installer::registry::ServerFile::load(&e.dir).map_err(|err| Problem::from(err).lead(&format!("cannot read {}", e.dir.join("kora-server.json").display())))?;
        crate::installer::gitops::with_runner(&e.dir, |r| crate::collab::run_on(r, &sf, req, say))
    }

    fn branches(&mut self, e: &ServerEntry, req: &crate::branches::Request, say: &mut dyn FnMut(usize, usize, &str)) -> Res<crate::branches::Reply> {
        let sf = crate::installer::registry::ServerFile::load(&e.dir).map_err(|err| Problem::from(err).lead(&format!("cannot read {}", e.dir.join("kora-server.json").display())))?;
        crate::installer::gitops::with_runner(&e.dir, |r| crate::branches::run(r, &sf, req, say))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_names_are_checked() {
        assert!(valid_resource_name("kora_core"));
        assert!(valid_resource_name("[standalone]"));
        assert!(!valid_resource_name(""));
        assert!(!valid_resource_name("a b"));
        assert!(!valid_resource_name("x;quit"));
        assert!(!valid_resource_name("[]"));
        for evil in [".", "..", "...", "[..]", "-x", "--upload-pack=x", "[-x]"] {
            assert!(!valid_resource_name(evil), "{evil}");
        }
        for ok in ["a-b", "a.b", "x-", "v1.2"] {
            assert!(valid_resource_name(ok), "{ok}");
        }
    }

    #[test]
    fn the_folder_fxserver_knows_comes_from_the_recorded_path_not_the_id() {
        use crate::installer::registry::{PortsRecord, ResourceRecord};
        let rec = |id: &str, path: &str| ResourceRecord { id: id.into(), path: path.into(), ..Default::default() };
        let sf = ServerFile {
            source: None,
            versions: None,
            name: "a".into(),
            mode: installer::Mode::Production,
            dir: PathBuf::from("/srv/a"),
            ports: PortsRecord { game: 30120, txadmin: 40120 },
            game_build: 0,
            artifact_build: 0,
            database: None,
            github_owner: None,
            created_unix: 0,
            kora_version: String::new(),
            resources: vec![rec("kora-lib", "[pre]/kora_lib"), rec("c-one", "[creators]/[acme]/c_one"), rec("plain", "plain"), rec("odd", "")],
        };
        assert_eq!(folder_of(&sf, "kora-lib").as_deref(), Some("kora_lib"));
        assert_eq!(folder_of(&sf, "c-one").as_deref(), Some("c_one"));
        assert_eq!(folder_of(&sf, "plain").as_deref(), Some("plain"));
        assert_eq!(folder_of(&sf, "kora_lib"), None, "looked up by id, like the list the panel shows");
        assert_eq!(folder_of(&sf, "odd"), None);
    }

    #[test]
    fn ensure_is_appended_once() {
        let cfg = "set a b\nensure foo\n";
        assert_eq!(with_ensure(cfg, "bar").unwrap(), "set a b\nensure foo\nensure bar\n");
        assert!(with_ensure(cfg, "foo").is_none());
        assert_eq!(with_ensure("set a b", "x").unwrap(), "set a b\nensure x\n");
        assert!(with_ensure("ensure foo # note\n", "foo").is_none());
        assert!(with_ensure("# ensure foo\n", "foo").is_some(), "a comment does not count");
        assert!(with_ensure("ensure foobar\n", "foo").is_some(), "a longer name does not count");
    }

    #[test]
    fn badge_file_is_read() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(read_badge(d.path()), None);
        std::fs::create_dir(d.path().join(".kora")).unwrap();
        let w = |s: &str| std::fs::write(d.path().join(".kora/update-status.json"), s).unwrap();
        w(r#"{"checked_at":5,"updates":3,"attention":false,"new_release":false}"#);
        assert_eq!(read_badge(d.path()), Some(UpdateBadge::Updates(3)));
        w(r#"{"updates":0,"attention":true}"#);
        assert_eq!(read_badge(d.path()), Some(UpdateBadge::NeedsAttention));
        w(r#"{"behind":2}"#);
        assert_eq!(read_badge(d.path()), Some(UpdateBadge::Updates(2)), "the old shape still reads");
        w("not json");
        assert_eq!(read_badge(d.path()), None);
        w(r#"{"new_release":true}"#);
        assert_eq!(read_badge(d.path()), Some(UpdateBadge::NewRelease));
        w(r#"{}"#);
        assert_eq!(read_badge(d.path()), Some(UpdateBadge::UpToDate));
    }

    #[test]
    fn badge_reads_what_the_status_writer_writes() {
        use crate::update::status::{write, UpdateStatus};
        let d = tempfile::tempdir().unwrap();
        write(d.path(), &UpdateStatus::new(1_700_000_000, 0, 2, None)).unwrap();
        assert_eq!(read_badge(d.path()), Some(UpdateBadge::NeedsAttention), "attention is a count");
        write(d.path(), &UpdateStatus::new(1_700_000_000, 3, 0, Some("1.4.0".into()))).unwrap();
        assert_eq!(read_badge(d.path()), Some(UpdateBadge::Updates(3)));
        write(d.path(), &UpdateStatus::new(1_700_000_000, 0, 0, Some("1.4.0".into()))).unwrap();
        assert_eq!(read_badge(d.path()), Some(UpdateBadge::NewRelease));
        write(d.path(), &UpdateStatus::new(1_700_000_000, 0, 0, None)).unwrap();
        assert_eq!(read_badge(d.path()), Some(UpdateBadge::UpToDate));
    }

    use std::cell::RefCell;

    struct FakeOps {
        service: bool,
        started_ok: bool,
        calls: RefCell<Vec<String>>,
    }
    impl ControlOps for FakeOps {
        fn has_service(&self, _: &str) -> bool {
            self.service
        }
        fn start(&self, _: &str) -> Result<launch::Report, Problem> {
            self.calls.borrow_mut().push("start".into());
            Ok(launch::Report {
                method: launch::Method::Detached,
                pid: Some(1),
                txadmin_up: self.started_ok,
                game_listening: false,
                problem: if self.started_ok { None } else { Some(launch::Problem::Crashed) },
                message: if self.started_ok { "up".into() } else { "it crashed".into() },
                log_tail: if self.started_ok { vec![] } else { vec!["boom 1".into(), "boom 2".into()] },
            })
        }
        fn stop_detached(&self, _: &str) -> Result<bool, Problem> {
            self.calls.borrow_mut().push("stop_detached".into());
            Ok(false)
        }
        fn service_control(&self, _: &str, a: ControlAction) -> Result<(), Problem> {
            self.calls.borrow_mut().push(format!("service_{a:?}"));
            Ok(())
        }
    }

    fn run(service: bool, ok: bool, a: ControlAction) -> (Res<()>, Vec<String>) {
        let ops = FakeOps { service, started_ok: ok, calls: RefCell::new(vec![]) };
        let r = control_with(&ops, "s", a);
        (r, ops.calls.into_inner())
    }

    #[test]
    fn control_without_a_service_launches_detached() {
        assert_eq!(run(false, true, ControlAction::Start), (Ok(()), vec!["start".to_string()]));
        assert_eq!(run(false, true, ControlAction::Stop), (Ok(()), vec!["stop_detached".to_string()]));
        assert_eq!(run(false, true, ControlAction::Restart), (Ok(()), vec!["stop_detached".to_string(), "start".to_string()]));
    }

    #[test]
    fn control_with_a_service_starts_via_launcher_and_stops_via_service() {
        assert_eq!(run(true, true, ControlAction::Start).1, ["start"]);
        assert_eq!(run(true, true, ControlAction::Stop).1, ["service_Stop"]);
        assert_eq!(run(true, true, ControlAction::Restart).1, ["service_Restart"]);
    }

    #[test]
    fn a_server_that_did_not_come_up_is_an_error_with_the_log_tail() {
        let (r, _) = run(false, false, ControlAction::Start);
        let m = r.unwrap_err();
        assert!(m.to_string().contains("did not start properly: it crashed") && m.to_string().contains("| boom 2"), "{m}");
    }
}
