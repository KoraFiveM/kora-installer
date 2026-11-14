//! Manager state machine: which screen is open, what every key / click / paste does, and the
//! (possibly slow) jobs that follow. No terminal in here; `view.rs` draws it, `mod.rs` runs it.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use kora_core::installer::registry::{self, ServerEntry};
use kora_core::service::ControlAction;
use ratatui::layout::{Position, Rect};

use super::adopt::{self, Candidate};
use super::api::manual::{ApplyOpts, ChangeStatus, ResourceChange, Scope, UpdateReport};
use super::api::notify::NotifyConfig;
use super::api::schedule::{Preset, RunAs, ScheduleConfig};
use super::backend::{Backend, Status, UpdateBadge};
use super::input::TextInput;
use super::logs::LogView;
use super::view;
use crate::clipboard::Clipboard;

/// Opens things for the user (fake-able in tests).
pub trait Launcher {
    fn open_folder(&mut self, dir: &std::path::Path) -> Result<(), String>;
    fn open_url(&mut self, url: &str) -> Result<(), String>;
}

pub struct Services {
    pub backend: Box<dyn Backend>,
    pub launcher: Box<dyn Launcher>,
    pub clipboard: Box<dyn Clipboard>,
}

/// Why the manager stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Quit,
    /// The user asked to install a new server: run the install wizard, then come back.
    InstallNew,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Good,
    Bad,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub level: Level,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct Row {
    pub entry: ServerEntry,
    pub status: Status,
    pub badge: Option<UpdateBadge>,
    /// Kora made it (has an install record) so "Delete" can take back what it created.
    pub managed: bool,
}

/// Everything a button or key can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Start,
    Stop,
    Restart,
    OpenTx,
    OpenFolder,
    Logs,
    Ensure,
    Delete,
    Update,
    Notify,
    Schedule,
    Add,
    Refresh,
    Quit,
    Back,
    InstallNew,
    AddExisting,
    TypePath,
    Confirm,
    Yes,
    No,
    Copy,
    Reload,
    PrevFile,
    NextFile,
    Top,
    End,
    PageUp,
    PageDown,
    ToggleRestart,
    ToggleAll,
    ApplySelected,
    Rollback,
    Save,
    SendTest,
    Remove,
    Retry,
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Row(usize),
    Act(Action),
}

// ---------------------------------------------------------------- screens

#[derive(Debug, Clone)]
pub struct EnsureState {
    pub entry: ServerEntry,
    pub input: TextInput,
    pub choices: Vec<String>,
    pub pick: Option<usize>,
    pub restart: bool,
}

impl EnsureState {
    /// Known resources matching what was typed.
    pub fn shown(&self) -> Vec<&String> {
        let q = self.input.text().to_lowercase();
        self.choices.iter().filter(|c| q.is_empty() || c.to_lowercase().contains(&q)).collect()
    }

    pub fn chosen(&self) -> String {
        match self.pick.and_then(|i| self.shown().get(i).map(|s| s.to_string())) {
            Some(n) => n,
            None => self.input.text().trim().to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DelStage {
    Files,
    Db,
    Name,
    /// A server Kora did not create: only take it off the list.
    Unlist,
}

#[derive(Debug, Clone)]
pub struct DeleteState {
    pub entry: ServerEntry,
    pub stage: DelStage,
    pub keep_files: bool,
    pub keep_db: bool,
    pub input: TextInput,
}

#[derive(Debug, Clone)]
pub struct UpItem {
    pub change: ResourceChange,
    pub checked: bool,
}

impl UpItem {
    /// Only clean fast-forwards are applied from the manager; local changes need a person.
    pub fn selectable(&self) -> bool {
        self.change.status == ChangeStatus::Clean
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpStage {
    Preview,
    Report,
}

#[derive(Debug, Clone)]
pub struct UpdateState {
    pub entry: ServerEntry,
    pub stage: UpStage,
    pub items: Vec<UpItem>,
    pub sel: usize,
    pub restart: bool,
    pub events: Vec<String>,
    pub report: Option<UpdateReport>,
    pub error: Option<String>,
    pub rollback: Option<TextInput>,
}

impl UpdateState {
    fn new(entry: ServerEntry) -> Self {
        Self { entry, stage: UpStage::Preview, items: vec![], sel: 0, restart: true, events: vec![], report: None, error: None, rollback: None }
    }

    pub fn checked_ids(&self) -> Vec<String> {
        self.items.iter().filter(|i| i.checked && i.selectable()).map(|i| i.change.id.clone()).collect()
    }
}

#[derive(Debug, Clone)]
pub struct NotifyState {
    pub entry: ServerEntry,
    pub current: NotifyConfig,
    pub input: TextInput,
}

pub const WEEKDAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

#[derive(Debug, Clone)]
pub struct ScheduleState {
    pub entry: ServerEntry,
    pub existing: Option<ScheduleConfig>,
    pub preset: Preset,
    pub time: TextInput,
    /// 0 = Monday .. 6 = Sunday.
    pub weekday: u8,
    pub run_as: RunAs,
    /// 0 preset, 1 time, 2 weekday, 3 run as.
    pub focus: usize,
}

impl ScheduleState {
    fn new(entry: ServerEntry, existing: Option<ScheduleConfig>) -> Self {
        let (preset, time, weekday, run_as) = match &existing {
            Some(c) => (c.preset, c.time.clone(), c.weekday.unwrap_or(0).min(6), c.run_as),
            None => (Preset::Daily, "04:00".to_string(), 0, RunAs::InstallingUser),
        };
        Self { entry, existing, preset, time: TextInput::with_text(40, &time), weekday, run_as, focus: 0 }
    }

    /// The fields that matter for the chosen preset (weekday only when weekly).
    pub fn fields(&self) -> Vec<usize> {
        if self.preset == Preset::Weekly {
            vec![0, 1, 2, 3]
        } else {
            vec![0, 1, 3]
        }
    }

    /// Validated config, or what is wrong with it.
    pub fn build(&self) -> Result<ScheduleConfig, String> {
        let time = self.time.text().trim().to_string();
        if self.preset == Preset::Custom {
            if time.is_empty() {
                return Err("Type when to run.".into());
            }
        } else if !valid_time(&time) {
            return Err("Time must look like 04:30 (24-hour, HH:MM).".into());
        }
        Ok(ScheduleConfig { preset: self.preset, time, weekday: (self.preset == Preset::Weekly).then_some(self.weekday), run_as: self.run_as })
    }
}

pub fn valid_time(t: &str) -> bool {
    let Some((h, m)) = t.split_once(':') else { return false };
    let ok = |s: &str, max: u32| (1..=2).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit()) && s.parse::<u32>().is_ok_and(|v| v <= max);
    ok(h, 23) && m.len() == 2 && ok(m, 59)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddStage {
    Menu,
    Found,
    Path,
}

#[derive(Debug, Clone)]
pub struct AddState {
    pub stage: AddStage,
    pub found: Vec<Candidate>,
    pub sel: usize,
    pub input: TextInput,
}

#[derive(Debug, Clone)]
pub enum Screen {
    Main,
    Logs(Box<LogView>),
    Ensure(Box<EnsureState>),
    Delete(Box<DeleteState>),
    Update(Box<UpdateState>),
    Notify(Box<NotifyState>),
    Schedule(Box<ScheduleState>),
    Add(Box<AddState>),
}

/// Slow work, run after the "working..." text has been drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    Control(ControlActionKey),
    Ensure,
    Delete,
    UpdatePreview,
    UpdateApply,
    Rollback(String),
    Scan,
    Adopt(usize),
    AdoptPath,
    NotifyTest,
}

/// `ControlAction` without needing `Eq` on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlActionKey {
    Start,
    Stop,
    Restart,
}

impl ControlActionKey {
    fn action(self) -> ControlAction {
        match self {
            ControlActionKey::Start => ControlAction::Start,
            ControlActionKey::Stop => ControlAction::Stop,
            ControlActionKey::Restart => ControlAction::Restart,
        }
    }
    fn verb(self) -> &'static str {
        match self {
            ControlActionKey::Start => "Starting",
            ControlActionKey::Stop => "Stopping",
            ControlActionKey::Restart => "Restarting",
        }
    }
}

impl Job {
    pub fn text(&self) -> String {
        match self {
            Job::Control(c) => format!("{} the server... (this can take a little while)", c.verb()),
            Job::Ensure => "Updating server.cfg...".into(),
            Job::Delete => "Deleting... please wait.".into(),
            Job::UpdatePreview => "Checking for updates...".into(),
            Job::UpdateApply => "Updating... please wait.".into(),
            Job::Rollback(_) => "Rolling back...".into(),
            Job::Scan => "Looking for server folders...".into(),
            Job::Adopt(_) | Job::AdoptPath => "Adding the server...".into(),
            Job::NotifyTest => "Sending a test message...".into(),
        }
    }
}

pub struct App {
    pub svc: Services,
    pub rows: Vec<Row>,
    pub sel: usize,
    pub screen: Screen,
    pub notice: Option<Notice>,
    pub load_error: Option<String>,
    pub busy: Option<Job>,
    pub exit: Option<Exit>,
}

fn managed(e: &ServerEntry) -> bool {
    e.dir.join(".kora").join("request.json").is_file()
}

impl App {
    pub fn new(svc: Services) -> Self {
        let mut a = Self { svc, rows: vec![], sel: 0, screen: Screen::Main, notice: None, load_error: None, busy: None, exit: None };
        a.reload();
        a
    }

    // ------------------------------------------------------------ list

    /// Re-read the registry and every server's state, keeping the selection on the same server.
    pub fn reload(&mut self) {
        let keep = self.rows.get(self.sel).map(|r| r.entry.name.clone());
        match self.svc.backend.list() {
            Ok(list) => {
                self.load_error = None;
                self.rows = list
                    .into_iter()
                    .map(|entry| Row { status: Status::Stopped, badge: None, managed: managed(&entry), entry })
                    .collect();
                self.refresh_status();
            }
            Err(e) => {
                self.load_error = Some(e);
                self.rows.clear();
            }
        }
        self.sel = keep.and_then(|n| self.rows.iter().position(|r| r.entry.name == n)).unwrap_or(0).min(self.rows.len().saturating_sub(1));
    }

    /// Cheap periodic refresh of running/stopped and the update badges.
    pub fn refresh_status(&mut self) {
        for r in &mut self.rows {
            r.status = self.svc.backend.status(&r.entry);
            r.badge = self.svc.backend.badge(&r.entry);
        }
    }

    pub fn current(&self) -> Option<&Row> {
        self.rows.get(self.sel)
    }

    fn entry(&self) -> Option<ServerEntry> {
        self.current().map(|r| r.entry.clone())
    }

    fn say(&mut self, level: Level, text: impl Into<String>) {
        self.notice = Some(Notice { level, text: text.into() });
    }

    fn select(&mut self, delta: isize) {
        if !self.rows.is_empty() {
            self.sel = (self.sel as isize + delta).clamp(0, self.rows.len() as isize - 1) as usize;
        }
    }

    // ------------------------------------------------------------ input

    pub fn handle_key(&mut self, k: KeyEvent) {
        if self.busy.is_some() {
            return;
        }
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
            self.exit = Some(Exit::Quit);
            return;
        }
        let a = match &mut self.screen {
            Screen::Main => main_key(k),
            Screen::Logs(_) => logs_key(k),
            Screen::Ensure(s) => ensure_key(s, k),
            Screen::Delete(s) => delete_key(s, k),
            Screen::Update(s) => update_key(s, k),
            Screen::Notify(s) => notify_key(s, k),
            Screen::Schedule(s) => schedule_key(s, k),
            Screen::Add(s) => add_key(s, k),
        };
        if let Some(a) = a {
            self.act(a);
        }
    }

    pub fn handle_paste(&mut self, text: &str) {
        if self.busy.is_some() {
            return;
        }
        let cut = match &mut self.screen {
            Screen::Ensure(s) => {
                s.pick = None;
                Some(s.input.paste(text))
            }
            Screen::Delete(s) if s.stage == DelStage::Name => Some(s.input.paste(text)),
            Screen::Notify(s) => Some(s.input.paste(text)),
            Screen::Schedule(s) if s.focus == 1 => Some(s.time.paste(text)),
            Screen::Add(s) if s.stage == AddStage::Path => Some(s.input.paste(text)),
            Screen::Update(s) => s.rollback.as_mut().map(|i| i.paste(text)),
            _ => None,
        };
        if cut == Some(true) {
            self.say(Level::Info, "That was too long, so it was shortened.");
        }
    }

    pub fn handle_mouse(&mut self, m: MouseEvent, area: Rect) {
        if self.busy.is_some() {
            return;
        }
        match m.kind {
            MouseEventKind::ScrollUp => self.scroll(-1),
            MouseEventKind::ScrollDown => self.scroll(1),
            MouseEventKind::Down(MouseButton::Left) => {
                let pos = Position::new(m.column, m.row);
                let hit = view::hits(self, area).into_iter().find(|(_, r)| r.contains(pos)).map(|(h, _)| h);
                match hit {
                    Some(Hit::Act(a)) => self.act(a),
                    Some(Hit::Row(i)) => self.click_row(i),
                    None => {}
                }
            }
            _ => {}
        }
    }

    fn scroll(&mut self, dir: isize) {
        match &mut self.screen {
            Screen::Main => self.select(dir),
            Screen::Logs(l) => {
                if dir < 0 {
                    l.scroll_up(3)
                } else {
                    l.scroll_down(3)
                }
            }
            Screen::Update(s) => s.sel = (s.sel as isize + dir).clamp(0, s.items.len().saturating_sub(1) as isize) as usize,
            Screen::Add(s) if s.stage == AddStage::Found => s.sel = (s.sel as isize + dir).clamp(0, s.found.len().saturating_sub(1) as isize) as usize,
            _ => {}
        }
    }

    fn click_row(&mut self, i: usize) {
        match &mut self.screen {
            Screen::Main => self.sel = i.min(self.rows.len().saturating_sub(1)),
            Screen::Update(s) => {
                s.sel = i;
                if let Some(it) = s.items.get_mut(i) {
                    if it.selectable() {
                        it.checked = !it.checked;
                    }
                }
            }
            Screen::Add(s) => s.sel = i,
            Screen::Schedule(s) => {
                s.focus = i;
                if i != 1 {
                    schedule_key(s, KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
                }
            }
            Screen::Ensure(s) => {
                s.pick = Some(i);
                let name = s.chosen();
                s.input.set(&name);
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------ actions

    pub fn act(&mut self, a: Action) {
        match a {
            Action::Quit => self.exit = Some(Exit::Quit),
            Action::InstallNew => self.exit = Some(Exit::InstallNew),
            Action::Refresh => {
                self.reload();
                self.say(Level::Info, "Refreshed.");
            }
            Action::Add => {
                self.notice = None;
                self.screen = Screen::Add(Box::new(AddState { stage: AddStage::Menu, found: vec![], sel: 0, input: TextInput::new(500) }));
            }
            Action::Back => self.back(),
            Action::Up | Action::Down | Action::Top | Action::End if matches!(self.screen, Screen::Main) => match a {
                Action::Up => self.select(-1),
                Action::Down => self.select(1),
                Action::Top => self.sel = 0,
                _ => self.sel = self.rows.len().saturating_sub(1),
            },
            Action::Start | Action::Stop | Action::Restart => {
                let Some(r) = self.current() else { return };
                if r.status == Status::Missing {
                    return self.say(Level::Bad, "That server's folder is missing.");
                }
                let key = match a {
                    Action::Start => ControlActionKey::Start,
                    Action::Stop => ControlActionKey::Stop,
                    _ => ControlActionKey::Restart,
                };
                self.busy = Some(Job::Control(key));
            }
            Action::OpenTx => {
                let Some(r) = self.current() else { return };
                let url = format!("http://127.0.0.1:{}", r.entry.ports.txadmin);
                match self.svc.launcher.open_url(&url) {
                    Ok(()) => self.say(Level::Good, format!("Opening txAdmin ({url}).")),
                    Err(e) => self.say(Level::Bad, format!("{e} Open {url} in your browser.")),
                }
            }
            Action::OpenFolder => {
                let Some(r) = self.current() else { return };
                let dir = r.entry.dir.clone();
                match self.svc.launcher.open_folder(&dir) {
                    Ok(()) => self.say(Level::Good, "Opening the folder."),
                    Err(e) => self.say(Level::Bad, e),
                }
            }
            Action::Logs => {
                if let Some(r) = self.current() {
                    let v = LogView::open(&r.entry.dir);
                    self.notice = None;
                    self.screen = Screen::Logs(Box::new(v));
                }
            }
            Action::Ensure => {
                if let Some(e) = self.entry() {
                    let choices = self.svc.backend.resources(&e);
                    self.notice = None;
                    self.screen = Screen::Ensure(Box::new(EnsureState { entry: e, input: TextInput::new(100), choices, pick: None, restart: true }));
                }
            }
            Action::Delete => {
                let Some(r) = self.current() else { return };
                let (entry, is_managed, status) = (r.entry.clone(), r.managed, r.status);
                if is_managed && status == Status::Running {
                    return self.say(Level::Bad, "Stop the server first, then delete it.");
                }
                let stage = if is_managed { DelStage::Files } else { DelStage::Unlist };
                self.notice = None;
                self.screen = Screen::Delete(Box::new(DeleteState { entry, stage, keep_files: false, keep_db: false, input: TextInput::new(100) }));
            }
            Action::Update => {
                if let Some(e) = self.entry() {
                    self.notice = None;
                    self.screen = Screen::Update(Box::new(UpdateState::new(e)));
                    self.busy = Some(Job::UpdatePreview);
                }
            }
            Action::Notify => {
                if let Some(e) = self.entry() {
                    let current = self.svc.backend.notify_load(&e);
                    self.notice = None;
                    self.screen = Screen::Notify(Box::new(NotifyState { entry: e, current, input: TextInput::new(300) }));
                }
            }
            Action::Schedule => {
                if let Some(e) = self.entry() {
                    let existing = self.svc.backend.schedule_get(&e);
                    self.notice = None;
                    self.screen = Screen::Schedule(Box::new(ScheduleState::new(e, existing)));
                }
            }
            other => self.screen_action(other),
        }
    }

    fn back(&mut self) {
        // One step back inside a multi-step screen, else to the list.
        match &mut self.screen {
            Screen::Add(s) if s.stage != AddStage::Menu => {
                s.stage = AddStage::Menu;
                return;
            }
            Screen::Update(s) if s.rollback.is_some() => {
                s.rollback = None;
                return;
            }
            _ => {}
        }
        self.screen = Screen::Main;
        self.reload();
    }

    fn screen_action(&mut self, a: Action) {
        let was_main = matches!(self.screen, Screen::Main);
        let mut screen = std::mem::replace(&mut self.screen, Screen::Main);
        match &mut screen {
            Screen::Main => {}
            Screen::Logs(l) => match a {
                Action::Up => l.scroll_up(1),
                Action::Down => l.scroll_down(1),
                Action::PrevFile => l.select(-1),
                Action::NextFile => l.select(1),
                Action::Reload => l.reload(),
                Action::Top => l.to_top(),
                Action::End => l.to_end(),
                Action::PageUp => l.scroll_up(10),
                Action::PageDown => l.scroll_down(10),
                Action::Copy => {
                    let t = l.copy_text();
                    let msg = match self.svc.clipboard.set_text(&t) {
                        Ok(()) => Notice { level: Level::Good, text: format!("Copied {} lines.", l.lines.len()) },
                        Err(e) => Notice { level: Level::Bad, text: e },
                    };
                    self.notice = Some(msg);
                }
                _ => {}
            },
            Screen::Ensure(s) => match a {
                Action::ToggleRestart => s.restart = !s.restart,
                Action::Confirm => {
                    if s.chosen().is_empty() {
                        self.say(Level::Bad, "Type or pick a resource name first.");
                    } else {
                        self.busy = Some(Job::Ensure);
                    }
                }
                _ => {}
            },
            Screen::Delete(s) => match (s.stage, a) {
                (DelStage::Files, Action::Yes) => {
                    s.keep_files = false;
                    s.stage = DelStage::Db;
                }
                (DelStage::Files, Action::No) => {
                    s.keep_files = true;
                    s.stage = DelStage::Db;
                }
                (DelStage::Db, Action::Yes) => {
                    s.keep_db = false;
                    s.stage = DelStage::Name;
                }
                (DelStage::Db, Action::No) => {
                    s.keep_db = true;
                    s.stage = DelStage::Name;
                }
                (DelStage::Name, Action::Confirm) => {
                    if s.input.text().trim() == s.entry.name {
                        self.busy = Some(Job::Delete);
                    } else {
                        self.say(Level::Bad, format!("That is not the server's name. Type exactly: {}", s.entry.name));
                    }
                }
                (DelStage::Unlist, Action::Yes) => self.busy = Some(Job::Delete),
                (DelStage::Unlist, Action::No) => {
                    screen = Screen::Main;
                }
                _ => {}
            },
            Screen::Update(s) => self.update_action(s, a),
            Screen::Notify(s) => match a {
                Action::Save => {
                    let url = s.input.text().trim().to_string();
                    if url.is_empty() {
                        self.say(Level::Info, "Nothing to save: paste a Discord webhook address first.");
                    } else if let Err(e) = check_webhook(&url) {
                        self.say(Level::Bad, e);
                    } else {
                        let cfg = NotifyConfig { discord_webhook: Some(url) };
                        match self.svc.backend.notify_save(&s.entry, &cfg) {
                            Ok(()) => {
                                s.current = cfg;
                                s.input.set("");
                                self.say(Level::Good, "Saved. Update reports will be sent to Discord.");
                            }
                            Err(e) => self.say(Level::Bad, format!("Couldn't save: {e}")),
                        }
                    }
                }
                Action::Remove => match self.svc.backend.notify_save(&s.entry, &NotifyConfig::default()) {
                    Ok(()) => {
                        s.current = NotifyConfig::default();
                        self.say(Level::Good, "Discord notifications are off.");
                    }
                    Err(e) => self.say(Level::Bad, format!("Couldn't save: {e}")),
                },
                Action::SendTest => {
                    if s.current.discord_webhook.is_none() {
                        self.say(Level::Info, "Save a webhook first, then send a test.");
                    } else {
                        self.busy = Some(Job::NotifyTest);
                    }
                }
                _ => {}
            },
            Screen::Schedule(s) => match a {
                Action::Save => match s.build() {
                    Ok(cfg) => match self.svc.backend.schedule_set(&s.entry, &cfg) {
                        Ok(()) => {
                            s.existing = Some(cfg);
                            self.say(Level::Good, "Schedule saved.");
                        }
                        Err(e) => self.say(Level::Bad, format!("Couldn't save the schedule: {e}")),
                    },
                    Err(e) => self.say(Level::Bad, e),
                },
                Action::Remove => match self.svc.backend.schedule_remove(&s.entry) {
                    Ok(()) => {
                        s.existing = None;
                        self.say(Level::Good, "Automatic updates are off.");
                    }
                    Err(e) => self.say(Level::Bad, format!("Couldn't remove the schedule: {e}")),
                },
                _ => {}
            },
            Screen::Add(s) => match (s.stage, a) {
                (AddStage::Menu, Action::AddExisting) => {
                    s.stage = AddStage::Found;
                    self.busy = Some(Job::Scan);
                }
                (AddStage::Found, Action::TypePath) | (AddStage::Menu, Action::TypePath) => s.stage = AddStage::Path,
                (AddStage::Found, Action::Confirm) => {
                    if s.found.is_empty() {
                        s.stage = AddStage::Path;
                    } else {
                        self.busy = Some(Job::Adopt(s.sel));
                    }
                }
                (AddStage::Path, Action::Confirm) => {
                    if s.input.text().trim().is_empty() {
                        self.say(Level::Info, "Type or paste the server's folder.");
                    } else {
                        self.busy = Some(Job::AdoptPath);
                    }
                }
                _ => {}
            },
        }
        self.screen = screen;
        if !was_main && matches!(self.screen, Screen::Main) {
            self.reload();
        }
    }

    fn update_action(&mut self, s: &mut UpdateState, a: Action) {
        match a {
            Action::ToggleRestart => s.restart = !s.restart,
            Action::ToggleAll => {
                let all = s.items.iter().filter(|i| i.selectable()).all(|i| i.checked);
                for i in s.items.iter_mut().filter(|i| i.selectable()) {
                    i.checked = !all;
                }
            }
            Action::PageUp => s.sel = s.sel.saturating_sub(5),
            Action::PageDown => s.sel = (s.sel + 5).min(s.items.len().saturating_sub(1)),
            Action::Retry => {
                s.error = None;
                s.stage = UpStage::Preview;
                self.busy = Some(Job::UpdatePreview);
            }
            Action::ApplySelected | Action::Confirm if s.stage == UpStage::Preview => {
                if s.rollback.is_some() {
                    return;
                }
                if s.checked_ids().is_empty() {
                    self.say(Level::Info, "Tick at least one resource first (Space).");
                } else {
                    self.busy = Some(Job::UpdateApply);
                }
            }
            Action::Rollback => match &s.rollback {
                None => {
                    let snap = s.report.as_ref().and_then(|r| r.snapshot.clone()).unwrap_or_default();
                    s.rollback = Some(TextInput::with_text(100, &snap));
                }
                Some(i) => {
                    let id = i.text().trim().to_string();
                    if id.is_empty() {
                        self.say(Level::Bad, "Type the snapshot id to roll back to.");
                    } else {
                        self.busy = Some(Job::Rollback(id));
                    }
                }
            },
            _ => {}
        }
    }

    // ------------------------------------------------------------ jobs

    /// Run the pending job (after the screen has shown its "working" text). True if one ran.
    pub fn run_pending(&mut self) -> bool {
        let Some(job) = self.busy.clone() else { return false };
        self.run_job(job);
        self.busy = None;
        true
    }

    fn run_job(&mut self, job: Job) {
        match job {
            Job::Control(k) => {
                let Some(e) = self.entry() else { return };
                match self.svc.backend.control(&e, k.action()) {
                    Ok(()) => {
                        let past = match k {
                            ControlActionKey::Start => "started",
                            ControlActionKey::Stop => "stopped",
                            ControlActionKey::Restart => "restarted",
                        };
                        self.refresh_status();
                        self.say(Level::Good, format!("'{}' {past}.", e.name));
                    }
                    Err(err) => self.say(Level::Bad, err),
                }
            }
            Job::Ensure => {
                let Screen::Ensure(s) = &self.screen else { return };
                let (e, name, restart) = (s.entry.clone(), s.chosen(), s.restart);
                match self.svc.backend.ensure(&e, &name, restart) {
                    Ok(msg) => {
                        self.refresh_status();
                        self.screen = Screen::Main;
                        self.say(Level::Good, msg);
                    }
                    Err(err) => self.say(Level::Bad, err),
                }
            }
            Job::Delete => {
                let Screen::Delete(s) = &self.screen else { return };
                let s = (**s).clone();
                let res = if s.stage == DelStage::Unlist {
                    self.svc.backend.unregister(&s.entry).map(|()| vec![])
                } else {
                    self.svc.backend.delete(&s.entry, s.keep_files, s.keep_db)
                };
                match res {
                    Ok(notes) => {
                        self.screen = Screen::Main;
                        self.reload();
                        let verb = if s.stage == DelStage::Unlist { "Removed from the list" } else { "Deleted" };
                        let extra = if notes.is_empty() { String::new() } else { format!(" {}", notes.join(" ")) };
                        self.say(Level::Good, format!("{verb}: '{}'.{extra}", s.entry.name));
                    }
                    Err(err) => {
                        self.screen = Screen::Main;
                        self.reload();
                        self.say(Level::Bad, format!("Delete did not finish: {err}"));
                    }
                }
            }
            Job::UpdatePreview => {
                let Screen::Update(s) = &mut self.screen else { return };
                match self.svc.backend.preview(&s.entry, Scope::All) {
                    Ok(p) => {
                        s.items = p.resources.into_iter().map(|c| UpItem { checked: c.status == ChangeStatus::Clean, change: c }).collect();
                        s.sel = 0;
                        s.error = None;
                    }
                    Err(e) => s.error = Some(e),
                }
            }
            Job::UpdateApply => {
                let Screen::Update(s) = &mut self.screen else { return };
                let ids = s.checked_ids();
                let mut events = vec![];
                let res = self.svc.backend.apply(&s.entry, Scope::Resources(ids), ApplyOpts { restart_if_needed: s.restart }, &mut |ev| {
                    events.push(format!("{}: {}", ev.resource, ev.message))
                });
                s.events = events;
                match res {
                    Ok(r) => {
                        s.report = Some(r);
                        s.error = None;
                        s.stage = UpStage::Report;
                    }
                    Err(e) => {
                        s.error = Some(e);
                        s.stage = UpStage::Report;
                    }
                }
                self.refresh_status();
            }
            Job::Rollback(id) => {
                let Screen::Update(s) = &mut self.screen else { return };
                match self.svc.backend.rollback(&s.entry, &id) {
                    Ok(()) => {
                        s.rollback = None;
                        s.events.push(format!("Rolled back to {id}."));
                        s.error = None;
                        self.say(Level::Good, format!("Rolled back to {id}."));
                    }
                    Err(e) => self.say(Level::Bad, format!("Rollback failed: {e}")),
                }
            }
            Job::Scan => {
                let roots = self.svc.backend.scan_roots();
                let known: Vec<PathBuf> = self.rows.iter().map(|r| r.entry.dir.clone()).collect();
                let found = adopt::scan(&roots, 3, &known);
                if let Screen::Add(s) = &mut self.screen {
                    s.found = found;
                    s.sel = 0;
                }
            }
            Job::Adopt(i) => {
                let Screen::Add(s) = &self.screen else { return };
                let Some(c) = s.found.get(i).cloned() else { return };
                self.adopt(c);
            }
            Job::AdoptPath => {
                let Screen::Add(s) = &self.screen else { return };
                let p = PathBuf::from(s.input.text().trim());
                if !p.is_absolute() {
                    return self.say(Level::Bad, "Type the full folder path (starting from the drive or /).");
                }
                if !p.is_dir() {
                    return self.say(Level::Bad, format!("{} is not a folder.", p.display()));
                }
                if self.rows.iter().any(|r| registry::same_path(&r.entry.dir, &p)) {
                    return self.say(Level::Info, "That server is already in the list.");
                }
                match adopt::classify(&p) {
                    Some(c) => self.adopt(c),
                    None => self.say(Level::Bad, "That folder doesn't look like an FXServer or Kora server (no server.cfg + txData, FXServer program, artifacts or .kora folder)."),
                }
            }
            Job::NotifyTest => {
                let Screen::Notify(s) = &self.screen else { return };
                match self.svc.backend.notify_test(&s.current) {
                    Ok(()) => self.say(Level::Good, "Test message sent. Check your Discord channel."),
                    Err(e) => self.say(Level::Bad, format!("The test message failed: {e}")),
                }
            }
        }
    }

    /// Register a found folder (registry entry only: the folder itself is not touched).
    fn adopt(&mut self, c: Candidate) {
        let taken: Vec<String> = self.rows.iter().map(|r| r.entry.name.clone()).collect();
        let entry = adopt::to_entry(&c, &taken);
        let name = entry.name.clone();
        match self.svc.backend.register(entry) {
            Ok(()) => {
                self.screen = Screen::Main;
                self.reload();
                if let Some(i) = self.rows.iter().position(|r| r.entry.name == name) {
                    self.sel = i;
                }
                self.say(Level::Good, format!("Added '{name}'. Nothing in its folder was changed."));
            }
            Err(e) => self.say(Level::Bad, format!("Couldn't add it: {e}")),
        }
    }
}

/// Discord webhook addresses only.
pub fn check_webhook(url: &str) -> Result<(), String> {
    let ok = ["https://discord.com/api/webhooks/", "https://discordapp.com/api/webhooks/", "https://canary.discord.com/api/webhooks/", "https://ptb.discord.com/api/webhooks/"]
        .iter()
        .any(|p| url.starts_with(p) && url.len() > p.len());
    if ok {
        Ok(())
    } else {
        Err("That doesn't look like a Discord webhook (https://discord.com/api/webhooks/...).".into())
    }
}

/// Show a webhook without its secret token.
pub fn mask_webhook(url: &str) -> String {
    match url.rfind('/') {
        Some(i) if i + 1 < url.len() => format!("{}/{}", &url[..i], "*".repeat(8)),
        _ => "*".repeat(8),
    }
}

// ---------------------------------------------------------------- key maps

fn main_key(k: KeyEvent) -> Option<Action> {
    Some(match k.code {
        KeyCode::Char('s') => Action::Start,
        KeyCode::Char('x') => Action::Stop,
        KeyCode::Char('r') => Action::Restart,
        KeyCode::Char('t') => Action::OpenTx,
        KeyCode::Char('o') => Action::OpenFolder,
        KeyCode::Char('l') => Action::Logs,
        KeyCode::Char('e') => Action::Ensure,
        KeyCode::Char('d') => Action::Delete,
        KeyCode::Char('u') => Action::Update,
        KeyCode::Char('n') => Action::Notify,
        KeyCode::Char('c') => Action::Schedule,
        KeyCode::Char('a') => Action::Add,
        KeyCode::Char('i') => Action::InstallNew,
        KeyCode::Char('R') | KeyCode::F(5) => Action::Refresh,
        KeyCode::Char('q') | KeyCode::Esc => Action::Quit,
        KeyCode::Up | KeyCode::Char('k') => Action::Up,
        KeyCode::Down | KeyCode::Char('j') => Action::Down,
        KeyCode::Home => Action::Top,
        KeyCode::End => Action::End,
        _ => return None,
    })
}

fn logs_key(k: KeyEvent) -> Option<Action> {
    Some(match k.code {
        KeyCode::Esc | KeyCode::Char('q') => Action::Back,
        KeyCode::Char('c') => Action::Copy,
        KeyCode::Char('r') | KeyCode::F(5) => Action::Reload,
        KeyCode::Left | KeyCode::BackTab => Action::PrevFile,
        KeyCode::Right | KeyCode::Tab => Action::NextFile,
        KeyCode::Up | KeyCode::Char('k') => Action::Up,
        KeyCode::Down | KeyCode::Char('j') => Action::Down,
        KeyCode::Home | KeyCode::Char('g') => Action::Top,
        KeyCode::End | KeyCode::Char('G') => Action::End,
        KeyCode::PageUp => Action::PageUp,
        KeyCode::PageDown => Action::PageDown,
        _ => return None,
    })
}

fn ensure_key(s: &mut EnsureState, k: KeyEvent) -> Option<Action> {
    match k.code {
        KeyCode::Esc => Some(Action::Back),
        KeyCode::Enter => Some(Action::Confirm),
        KeyCode::Tab => Some(Action::ToggleRestart),
        KeyCode::Down => {
            let n = s.shown().len();
            s.pick = match s.pick {
                None if n > 0 => Some(0),
                Some(i) if i + 1 < n => Some(i + 1),
                p => p,
            };
            None
        }
        KeyCode::Up => {
            s.pick = match s.pick {
                Some(0) | None => None,
                Some(i) => Some(i - 1),
            };
            None
        }
        _ => {
            if s.input.key(k) {
                s.pick = None;
            }
            None
        }
    }
}

fn delete_key(s: &mut DeleteState, k: KeyEvent) -> Option<Action> {
    match (s.stage, k.code) {
        (_, KeyCode::Esc) => Some(Action::Back),
        (DelStage::Name, KeyCode::Enter) => Some(Action::Confirm),
        (DelStage::Name, _) => {
            s.input.key(k);
            None
        }
        (_, KeyCode::Char('y') | KeyCode::Char('Y')) => Some(Action::Yes),
        (_, KeyCode::Char('n') | KeyCode::Char('N')) => Some(Action::No),
        _ => None,
    }
}

fn update_key(s: &mut UpdateState, k: KeyEvent) -> Option<Action> {
    if let Some(i) = s.rollback.as_mut() {
        return match k.code {
            KeyCode::Esc => Some(Action::Back),
            KeyCode::Enter => Some(Action::Rollback),
            _ => {
                i.key(k);
                None
            }
        };
    }
    match (s.stage, k.code) {
        (_, KeyCode::Esc | KeyCode::Char('q')) => Some(Action::Back),
        (UpStage::Preview, KeyCode::Up | KeyCode::Char('k')) => {
            s.sel = s.sel.saturating_sub(1);
            None
        }
        (UpStage::Preview, KeyCode::Down | KeyCode::Char('j')) => {
            s.sel = (s.sel + 1).min(s.items.len().saturating_sub(1));
            None
        }
        (UpStage::Preview, KeyCode::PageUp) => Some(Action::PageUp),
        (UpStage::Preview, KeyCode::PageDown) => Some(Action::PageDown),
        (UpStage::Preview, KeyCode::Char(' ')) => {
            if let Some(it) = s.items.get_mut(s.sel) {
                if it.selectable() {
                    it.checked = !it.checked;
                }
            }
            None
        }
        (UpStage::Preview, KeyCode::Char('a')) => Some(Action::ToggleAll),
        (UpStage::Preview, KeyCode::Enter | KeyCode::Char('u')) => Some(Action::ApplySelected),
        (UpStage::Preview, KeyCode::Char('r')) => Some(if s.error.is_some() { Action::Retry } else { Action::ToggleRestart }),
        (_, KeyCode::Char('b')) => Some(Action::Rollback),
        _ => None,
    }
}

fn notify_key(s: &mut NotifyState, k: KeyEvent) -> Option<Action> {
    match k.code {
        KeyCode::Esc => Some(Action::Back),
        KeyCode::Enter => Some(Action::Save),
        KeyCode::F(2) => Some(Action::SendTest),
        KeyCode::F(3) => Some(Action::Remove),
        _ => {
            s.input.key(k);
            None
        }
    }
}

fn schedule_key(s: &mut ScheduleState, k: KeyEvent) -> Option<Action> {
    let fields = s.fields();
    let pos = fields.iter().position(|f| *f == s.focus).unwrap_or(0);
    match k.code {
        KeyCode::Esc => return Some(Action::Back),
        KeyCode::Enter => return Some(Action::Save),
        KeyCode::F(3) => return Some(Action::Remove),
        KeyCode::Tab | KeyCode::Down => s.focus = fields[(pos + 1) % fields.len()],
        KeyCode::BackTab | KeyCode::Up => s.focus = fields[(pos + fields.len() - 1) % fields.len()],
        _ if s.focus == 1 => {
            s.time.key(k);
        }
        KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') => {
            let d: i32 = if k.code == KeyCode::Left { -1 } else { 1 };
            match s.focus {
                0 => {
                    let all = [Preset::Daily, Preset::Weekly, Preset::Custom];
                    let i = all.iter().position(|p| *p == s.preset).unwrap_or(0) as i32;
                    s.preset = all[(i + d).rem_euclid(3) as usize];
                }
                2 => s.weekday = (s.weekday as i32 + d).rem_euclid(7) as u8,
                _ => {
                    s.run_as = if s.run_as == RunAs::System { RunAs::InstallingUser } else { RunAs::System };
                }
            }
        }
        _ => {}
    }
    None
}

fn add_key(s: &mut AddState, k: KeyEvent) -> Option<Action> {
    match (s.stage, k.code) {
        (_, KeyCode::Esc) => Some(Action::Back),
        (AddStage::Menu, KeyCode::Char('i')) => Some(Action::InstallNew),
        (AddStage::Menu, KeyCode::Char('e')) => Some(Action::AddExisting),
        (AddStage::Menu, KeyCode::Char('p')) => Some(Action::TypePath),
        (AddStage::Found, KeyCode::Up | KeyCode::Char('k')) => {
            s.sel = s.sel.saturating_sub(1);
            None
        }
        (AddStage::Found, KeyCode::Down | KeyCode::Char('j')) => {
            s.sel = (s.sel + 1).min(s.found.len().saturating_sub(1));
            None
        }
        (AddStage::Found, KeyCode::Enter) => Some(Action::Confirm),
        (AddStage::Found, KeyCode::Char('p')) => Some(Action::TypePath),
        (AddStage::Path, KeyCode::Enter) => Some(Action::Confirm),
        (AddStage::Path, _) => {
            s.input.key(k);
            None
        }
        _ => None,
    }
}
