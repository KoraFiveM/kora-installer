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

use super::api::manual::{ApplyOpts, Scope, UpdateEvent, UpdatePreview, UpdateReport};
use super::api::notify::NotifyConfig;
use super::api::schedule::ScheduleConfig;
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
    pub schedule: Option<ScheduleConfig>,
    pub clip: Option<String>,
    pub launch_error: Option<String>,
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
        f.preview.clone().unwrap_or_else(|| Ok(UpdatePreview::default()))
    }
    fn apply(&mut self, e: &ServerEntry, scope: Scope, o: ApplyOpts, on: &mut dyn FnMut(UpdateEvent)) -> Res<UpdateReport> {
        let mut f = self.0.borrow_mut();
        f.calls.push(format!("apply {} {scope:?} restart={}", e.name, o.restart_if_needed));
        for ev in f.apply_events.clone() {
            on(ev);
        }
        f.apply.clone().unwrap_or_else(|| Ok(UpdateReport::default()))
    }
    fn rollback(&mut self, e: &ServerEntry, snapshot: &str) -> Res<()> {
        self.0.borrow_mut().calls.push(format!("rollback {} {snapshot}", e.name));
        Ok(())
    }
    fn notify_load(&mut self, _e: &ServerEntry) -> NotifyConfig {
        self.0.borrow().notify.clone()
    }
    fn notify_save(&mut self, _e: &ServerEntry, c: &NotifyConfig) -> Res<()> {
        let mut f = self.0.borrow_mut();
        f.calls.push(format!("notify_save {:?}", c.discord_webhook));
        f.notify = c.clone();
        Ok(())
    }
    fn notify_test(&mut self, _c: &NotifyConfig) -> Res<()> {
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
    let app = App::new(Services { backend: Box::new(FakeBackend(shared.clone())), launcher: Box::new(FakeLauncher(shared.clone())), clipboard: Box::new(FakeClip(shared.clone())) });
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
