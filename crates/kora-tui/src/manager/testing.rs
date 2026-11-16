//! Fakes and helpers for the manager tests.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use kora_core::installer::registry::{PortsRecord, ServerEntry};
use kora_core::installer::Mode;
use kora_core::service::ControlAction;
use ratatui::{backend::TestBackend, layout::Rect, Terminal};

use kora_core::addresources::{AddOutcome, AddPlan};
use kora_core::catalog::Catalog;
use kora_core::feedback::PrPlan;
use kora_core::notify::NotifyConfig;
use kora_core::service::ScheduleConfig;
use kora_core::update::manual::{ApplyOpts, Scope, SnapshotInfo, UpdateEvent, UpdatePreview};
use kora_core::update::UpdateReport;
use super::app::*;
use super::backend::{Backend, Res, Status, UpdateBadge};
use crate::clipboard::Clipboard;

#[derive(Default)]
pub struct Fake {
    pub servers: Vec<ServerEntry>,
    pub status: HashMap<String, Status>,
    pub badges: HashMap<String, UpdateBadge>,
    pub calls: Vec<String>,
    pub list_error: Option<String>,
    pub control_error: Option<String>,
    pub ensure_error: Option<String>,
    pub delete_error: Option<String>,
    pub resources: Vec<String>,
    pub roots: Vec<PathBuf>,
    pub preview: Option<Res<UpdatePreview>>,
    pub apply: Option<Res<UpdateReport>>,
    pub apply_events: Vec<UpdateEvent>,
    pub notify: NotifyConfig,
    pub notify_test_error: Option<String>,
    pub notify_save_error: Option<String>,
    pub snapshots: Vec<SnapshotInfo>,
    /// `Some(args)`: changing the schedule needs elevation and these are the arguments.
    pub elevation: Option<Vec<String>>,
    /// What the live-progress hook saw (a copy of the progress at each event).
    pub frames: Vec<super::app::Live>,
    pub checked: Option<Res<super::backend::UpdateBadge>>,
    pub schedule: Option<ScheduleConfig>,
    pub clip: Option<String>,
    pub launch_error: Option<String>,
    /// Catalog + installed ids for the add-resources screen.
    pub add_catalog: Option<(Catalog, Vec<String>)>,
    pub add_catalog_error: Option<String>,
    pub add_result: Option<Res<AddOutcome>>,
    pub signed_in: bool,
    pub issue_result: Option<Res<String>>,
    pub pr_plan: Option<Res<PrPlan>>,
    pub pr_result: Option<Res<String>>,
    pub ports_error: Option<String>,
}

pub type Shared = Rc<RefCell<Fake>>;

pub struct FakeBackend(pub Shared);
pub struct FakeLauncher(pub Shared);
pub struct FakeClip(pub Shared);

impl Backend for FakeBackend {
    fn list(&mut self) -> Res<Vec<ServerEntry>> {
        let f = self.0.borrow();
        match &f.list_error {
            Some(e) => Err(e.clone()),
            None => Ok(f.servers.clone()),
        }
    }
    fn status(&mut self, e: &ServerEntry) -> Status {
        self.0.borrow().status.get(&e.name).copied().unwrap_or(Status::Stopped)
    }
    fn badge(&mut self, e: &ServerEntry) -> Option<UpdateBadge> {
        self.0.borrow().badges.get(&e.name).copied()
    }
    fn control(&mut self, e: &ServerEntry, a: ControlAction) -> Res<()> {
        let mut f = self.0.borrow_mut();
        f.calls.push(format!("{a:?} {}", e.name));
        if let Some(err) = &f.control_error {
            return Err(err.clone());
        }
        let st = if a == ControlAction::Stop { Status::Stopped } else { Status::Running };
        f.status.insert(e.name.clone(), st);
        Ok(())
    }
    fn resources(&mut self, _e: &ServerEntry) -> Vec<String> {
        self.0.borrow().resources.clone()
    }
    fn ensure(&mut self, e: &ServerEntry, name: &str, restart: bool) -> Res<String> {
        let mut f = self.0.borrow_mut();
        f.calls.push(format!("ensure {} {name} restart={restart}", e.name));
        match &f.ensure_error {
            Some(x) => Err(x.clone()),
            None => Ok(format!("ensured {name}")),
        }
    }
    fn delete(&mut self, e: &ServerEntry, keep_files: bool, keep_db: bool) -> Res<Vec<String>> {
        let mut f = self.0.borrow_mut();
        f.calls.push(format!("delete {} keep_files={keep_files} keep_db={keep_db}", e.name));
        if let Some(x) = &f.delete_error {
            return Err(x.clone());
        }
        f.servers.retain(|s| s.name != e.name);
        Ok(vec!["note one".into()])
    }
    fn unregister(&mut self, e: &ServerEntry) -> Res<()> {
        let mut f = self.0.borrow_mut();
        f.calls.push(format!("unregister {}", e.name));
        f.servers.retain(|s| s.name != e.name);
        Ok(())
    }
    fn register(&mut self, e: ServerEntry) -> Res<()> {
        let mut f = self.0.borrow_mut();
        f.calls.push(format!("register {}", e.name));
        f.servers.push(e);
        Ok(())
    }
    fn scan_roots(&self) -> Vec<PathBuf> {
        self.0.borrow().roots.clone()
    }
    fn preview(&mut self, _e: &ServerEntry, scope: Scope) -> Res<UpdatePreview> {
        let f = self.0.borrow();
        assert_eq!(scope, Scope::All);
        f.preview.clone().unwrap_or_else(|| {
            Ok(UpdatePreview {
                resources: vec![],
                new_release: None,
                notes: vec![],
            })
        })
    }
    fn apply(
        &mut self,
        e: &ServerEntry,
        scope: Scope,
        o: ApplyOpts,
        on: &mut dyn FnMut(UpdateEvent),
    ) -> Res<UpdateReport> {
        let (events, result) = {
            let mut f = self.0.borrow_mut();
            f.calls.push(format!("apply {} {scope:?} restart={:?}", e.name, o.restart));
            (f.apply_events.clone(), f.apply.clone().unwrap_or_else(|| Ok(UpdateReport::default())))
        };
        for ev in events {
            on(ev); // the live hook borrows the fake too, so no borrow is held here
        }
        result
    }
    fn rollback(&mut self, e: &ServerEntry, snapshot: &str, _force: bool) -> Res<()> {
        self.0
            .borrow_mut()
            .calls
            .push(format!("rollback {} {snapshot}", e.name));
        Ok(())
    }
    fn snapshots(&mut self, _e: &ServerEntry) -> Vec<SnapshotInfo> {
        self.0.borrow().snapshots.clone()
    }
    fn check_updates(&mut self, e: &ServerEntry) -> Res<UpdateBadge> {
        let mut f = self.0.borrow_mut();
        f.calls.push(format!("check {}", e.name));
        match f.checked.clone() {
            Some(r) => r,
            None => f
                .preview
                .clone()
                .unwrap_or_else(|| {
                    Ok(UpdatePreview {
                        resources: vec![],
                        new_release: None,
                        notes: vec![],
                    })
                })
                .map(|p| super::backend::badge_of(&p)),
        }
    }
    fn ports_set(&mut self, e: &ServerEntry, p: kora_core::installer::Ports) -> Res<Vec<String>> {
        let mut f = self.0.borrow_mut();
        f.calls.push(format!("ports {} {} {}", e.name, p.game, p.txadmin));
        match &f.ports_error {
            Some(msg) => Err(msg.clone()),
            None => Ok(vec!["Firewall rules were not changed.".into()]),
        }
    }

    fn schedule_elevation(&mut self, _e: &ServerEntry, _set: Option<&ScheduleConfig>) -> Option<Vec<String>> {
        self.0.borrow().elevation.clone()
    }
    fn add_catalog(&mut self, _e: &ServerEntry) -> Res<(Catalog, Vec<String>, Option<String>)> {
        let f = self.0.borrow();
        if let Some(e) = &f.add_catalog_error {
            return Err(e.clone());
        }
        let (c, i) = f.add_catalog.clone().expect("a catalog");
        Ok((c, i, None))
    }
    fn add_plan(&mut self, _e: &ServerEntry, ids: &[String]) -> Res<AddPlan> {
        let f = self.0.borrow();
        let (c, inst) = f.add_catalog.clone().expect("a catalog");
        let sf = kora_core::installer::registry::ServerFile {
            name: "x".into(),
            mode: Mode::Production,
            dir: PathBuf::from("/nowhere"),
            ports: PortsRecord { game: 1, txadmin: 2 },
            game_build: 1,
            artifact_build: 1,
            database: None,
            github_owner: None,
            created_unix: 0,
            kora_version: "0".into(),
            resources: inst
                .into_iter()
                .map(|id| kora_core::installer::registry::ResourceRecord { id, path: "x".into(), repository: "https://github.com/o/x.git".into(), branch: "dev".into(), commit: None, third_party: false, ..Default::default() })
                .collect(),
        };
        kora_core::addresources::plan(&c, &sf, ids).map_err(|e| e.to_string())
    }
    fn add_install(&mut self, e: &ServerEntry, ids: &[String]) -> Res<AddOutcome> {
        let mut f = self.0.borrow_mut();
        f.calls.push(format!("add_install {} {}", e.name, ids.join(",")));
        f.add_result.clone().unwrap_or_else(|| Ok(AddOutcome::default()))
    }
    fn github_signed_in(&mut self) -> bool {
        self.0.borrow().signed_in
    }
    fn report_issue(&mut self, e: &ServerEntry, resource: &str, title: &str, text: &str) -> Res<String> {
        let mut f = self.0.borrow_mut();
        f.calls.push(format!("issue {} {resource} [{title}] [{text}]", e.name));
        f.issue_result.clone().unwrap_or_else(|| Ok("https://github.com/KoraFiveM/x/issues/1".into()))
    }
    fn pr_plan(&mut self, e: &ServerEntry, resource: &str) -> Res<PrPlan> {
        let mut f = self.0.borrow_mut();
        f.calls.push(format!("pr_plan {} {resource}", e.name));
        f.pr_plan.clone().expect("a pr plan")
    }
    fn pr_open(&mut self, plan: &PrPlan, title: &str, text: &str) -> Res<String> {
        let mut f = self.0.borrow_mut();
        f.calls.push(format!("pr_open {} [{title}] [{text}]", plan.resource));
        f.pr_result.clone().unwrap_or_else(|| Ok("https://github.com/KoraFiveM/x/pull/2".into()))
    }
    fn notify_load(&mut self, _e: &ServerEntry) -> NotifyConfig {
        self.0.borrow().notify.clone()
    }
    fn notify_save(&mut self, _e: &ServerEntry, c: &NotifyConfig) -> Res<()> {
        let mut f = self.0.borrow_mut();
        if let Some(e) = &f.notify_save_error {
            return Err(e.clone());
        }
        f.calls.push(format!("notify_save {:?}", c.discord_webhook));
        f.notify = c.clone();
        Ok(())
    }
    fn notify_test(&mut self, _e: &ServerEntry, _c: &NotifyConfig) -> Res<()> {
        let mut f = self.0.borrow_mut();
        f.calls.push("notify_test".into());
        match &f.notify_test_error {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        }
    }
    fn schedule_get(&mut self, _e: &ServerEntry) -> Option<ScheduleConfig> {
        self.0.borrow().schedule.clone()
    }
    fn schedule_set(&mut self, _e: &ServerEntry, c: &ScheduleConfig) -> Res<()> {
        let mut f = self.0.borrow_mut();
        f.calls.push(format!("schedule_set {c:?}"));
        f.schedule = Some(c.clone());
        Ok(())
    }
    fn schedule_remove(&mut self, _e: &ServerEntry) -> Res<()> {
        let mut f = self.0.borrow_mut();
        f.calls.push("schedule_remove".into());
        f.schedule = None;
        Ok(())
    }
}

impl Launcher for FakeLauncher {
    fn open_folder(&mut self, dir: &Path) -> Result<(), String> {
        let mut f = self.0.borrow_mut();
        f.calls.push(format!("folder {}", dir.display()));
        f.launch_error.clone().map_or(Ok(()), Err)
    }
    fn open_url(&mut self, url: &str) -> Result<(), String> {
        let mut f = self.0.borrow_mut();
        f.calls.push(format!("url {url}"));
        f.launch_error.clone().map_or(Ok(()), Err)
    }
}

impl Clipboard for FakeClip {
    fn get_text(&mut self) -> Result<String, String> {
        self.0.borrow().clip.clone().ok_or_else(|| "empty".to_string())
    }
    fn set_text(&mut self, t: &str) -> Result<(), String> {
        self.0.borrow_mut().clip = Some(t.to_string());
        Ok(())
    }
}

pub const AREA: Rect = Rect { x: 0, y: 0, width: 120, height: 32 };

pub fn entry(name: &str, dir: &Path, game: u16) -> ServerEntry {
    ServerEntry { name: name.into(), dir: dir.to_path_buf(), mode: Mode::Production, ports: PortsRecord { game, txadmin: game + 10000 }, created_unix: 1 }
}

/// A folder that looks like one Kora installed.
pub fn managed_dir(root: &Path, name: &str) -> PathBuf {
    let d = root.join(name);
    std::fs::create_dir_all(d.join(".kora")).unwrap();
    std::fs::write(d.join(".kora/request.json"), "{}").unwrap();
    d
}

pub fn app_with(servers: Vec<ServerEntry>) -> (App, Shared) {
    let shared: Shared = Rc::new(RefCell::new(Fake { servers, ..Fake::default() }));
    let live_to = shared.clone();
    let app = App::new(Services {
        backend: Box::new(FakeBackend(shared.clone())),
        launcher: Box::new(FakeLauncher(shared.clone())),
        clipboard: Box::new(FakeClip(shared.clone())),
        live: Box::new(move |l| live_to.borrow_mut().frames.push(l.clone())),
    });
    (app, shared)
}

pub fn key(app: &mut App, c: KeyCode) {
    app.handle_key(KeyEvent::new(c, KeyModifiers::NONE));
    app.run_pending();
}

pub fn chr(app: &mut App, c: char) {
    key(app, KeyCode::Char(c));
}

pub fn typed(app: &mut App, s: &str) {
    for c in s.chars() {
        chr(app, c);
    }
}

fn mouse(kind: MouseEventKind, x: u16, y: u16) -> MouseEvent {
    MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE }
}

pub fn click_at(app: &mut App, area: Rect, x: u16, y: u16) {
    app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), x, y), area);
    app.run_pending();
}

pub fn wheel(app: &mut App, area: Rect, up: bool) {
    app.handle_mouse(mouse(if up { MouseEventKind::ScrollUp } else { MouseEventKind::ScrollDown }, 5, 5), area);
}

/// Click the middle of the control for `h` (panics if it is not on screen: that is a test failure).
pub fn click(app: &mut App, area: Rect, h: Hit) {
    let r = super::view::hits(app, area).into_iter().find(|(x, _)| *x == h).unwrap_or_else(|| panic!("{h:?} is not clickable on this screen:\n{}", text(app, area.width, area.height))).1;
    click_at(app, area, r.x + r.width / 2, r.y);
}

pub fn clickable(app: &App, area: Rect, h: Hit) -> bool {
    super::view::hits(app, area).iter().any(|(x, _)| *x == h)
}

pub fn text(app: &App, w: u16, h: u16) -> String {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| super::view::render(f, app)).unwrap();
    let buf = t.backend().buffer().clone();
    (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string()).collect::<Vec<_>>().join("\n")
}

pub fn screen_text(app: &App) -> String {
    text(app, AREA.width, AREA.height)
}
