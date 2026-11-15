//! Failure and "all done" screens with clickable actions: open the log / folder, copy the path,
//! copy the last lines of the log. State, layout and key/mouse handling are pure (the clipboard
//! and the opener are injected), so everything is testable with ratatui's `TestBackend`.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use kora_core::installer::InstallOutcome;
use ratatui::{
    layout::{Position, Rect},
    style::Modifier,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::clipboard::{last_lines, Clipboard};
use crate::open::{file_url, osc8_overlay, root_for_log, Opener};
use crate::theme::{self, theme, Tone};
use crate::wizard_view::wrap;

/// How many log lines "Copy details" puts on the clipboard.
pub const DETAIL_LINES: usize = 40;
/// SGR used for the path line; the OSC 8 overlay repeats it so the look doesn't change.
pub const PATH_SGR: &str = "4;36";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Failure,
    Outcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    OpenLog,
    OpenFolder,
    CopyPath,
    CopyDetails,
    /// Failure caused by missing administrator rights: restart with permission.
    Restart,
    OpenTxAdmin,
    CopyConnect,
    CopyPassword,
    StartServer,
    StopServer,
}

impl Action {
    fn label(self) -> &'static str {
        match self {
            Action::OpenLog => "[ Open log ]",
            Action::OpenFolder => "[ Open folder ]",
            Action::CopyPath => "[ Copy path ]",
            Action::CopyDetails => "[ Copy details ]",
            Action::Restart => "[ Restart as administrator ]",
            Action::OpenTxAdmin => "[ Open txAdmin ]",
            Action::CopyConnect => "[ Copy connect address ]",
            Action::CopyPassword => "[ Copy password ]",
            Action::StartServer => "[ Start server ]",
            Action::StopServer => "[ Stop server ]",
        }
    }
}

/// Messages from a background start.
#[derive(Debug, Clone)]
pub enum ServerMsg {
    Progress(String),
    Finished(kora_core::launch::Report),
    Failed(String),
}

/// Starting and stopping the server from the final screen (fake-able in tests).
pub trait ServerCtl {
    /// Begin starting in the background; progress comes through `poll`.
    fn start(&mut self, name: &str);
    fn poll(&mut self) -> Vec<ServerMsg>;
    /// Stop it (blocks briefly).
    fn stop(&mut self, name: &str) -> Result<(), String>;
}

/// Used where no server can be controlled (failure screens, tests).
pub struct NoServer;

impl ServerCtl for NoServer {
    fn start(&mut self, _: &str) {}
    fn poll(&mut self) -> Vec<ServerMsg> {
        vec![]
    }
    fn stop(&mut self, _: &str) -> Result<(), String> {
        Err("There is no server to stop.".into())
    }
}

/// What the final screen knows about the server.
#[derive(Debug, Clone, Default)]
pub struct ServerView {
    pub name: String,
    pub attempted: bool,
    pub running: bool,
    pub stopped: bool,
    pub game_listening: bool,
    pub problem: Option<String>,
    pub log_tail: Vec<String>,
    pub admin_user: Option<String>,
    pub admin_password_file: Option<PathBuf>,
    /// Shown on this screen only (button + text); never logged.
    pub admin_password: Option<String>,
    /// A start is in progress: the latest status line.
    pub busy: Option<String>,
}

/// The password line of `secrets/txadmin-admin.txt`.
pub fn read_admin_password(file: &Path) -> Option<String> {
    let t = std::fs::read_to_string(file).ok()?;
    t.lines().find_map(|l| l.strip_prefix("Password:")).map(|p| p.trim().to_string()).filter(|p| !p.is_empty())
}

/// What the actions need from the outside world.
pub struct Services<'a> {
    pub server: &'a mut dyn ServerCtl,
    pub clipboard: &'a mut dyn Clipboard,
    pub opener: &'a mut dyn Opener,
    pub elevator: &'a mut crate::elev::Elevator,
    /// Arguments the restart repeats.
    pub args: &'a [std::ffi::OsString],
}

/// Extra next steps for common Windows failures (plain words).
pub fn windows_hints(message: &str) -> Vec<String> {
    let m = message.to_ascii_lowercase();
    let mut v = vec![];
    if ["access is denied", "access denied", "permission denied", "os error 5", "being used by another process", "os error 32"].iter().any(|k| m.contains(k)) {
        v.push("Windows refused to change a file. An antivirus or another program may be using it: close other programs that use the folder, wait a minute and run kora install again. If it keeps happening, check Windows Security > Protection history.".to_string());
    }
    if is_admin_error(message) {
        v.push("If the button does not work: right-click Windows Terminal, choose Run as administrator, and run kora install again.".to_string());
    }
    v
}

/// Does the failure message say that administrator rights are missing?
pub fn is_admin_error(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("administrator") || m.contains("elevat") || m.contains("admin rights") || m.contains("run as admin")
}

pub struct ResultScreen {
    pub kind: Kind,
    pub message: String,
    /// Newline separated "what to do next" items (shown as a numbered list on failure).
    pub next: String,
    pub log: Option<PathBuf>,
    pub folder: Option<PathBuf>,
    pub notes: Vec<String>,
    pub connect: String,
    pub txadmin_url: String,
    pub status: Option<(String, bool)>,
    pub hyperlinks: bool,
    /// Add Windows-specific hints (antivirus, Administrator) to the failure steps.
    pub windows: bool,
    /// Kora can restart itself with permission (not elevated, and the platform can prompt).
    pub can_elevate: bool,
    /// The restart worked: the caller must exit now.
    pub restarted: bool,
    pub srv: ServerView,
}

impl ResultScreen {
    pub fn failure(message: &str, next: &str, log: Option<&Path>) -> Self {
        Self {
            kind: Kind::Failure,
            message: message.into(),
            next: next.into(),
            folder: log.map(root_for_log),
            log: log.map(Path::to_path_buf),
            notes: vec![],
            connect: String::new(),
            txadmin_url: String::new(),
            status: None,
            hyperlinks: false,
            windows: cfg!(windows),
            can_elevate: false,
            restarted: false,
            srv: ServerView::default(),
        }
    }

    pub fn outcome(o: &InstallOutcome) -> Self {
        let srv = ServerView {
            name: o.name.clone(),
            attempted: o.start_attempted,
            running: o.running,
            stopped: false,
            game_listening: o.game_listening,
            problem: o.launch_problem.clone(),
            log_tail: o.log_tail.clone(),
            admin_user: o.admin_user.clone(),
            admin_password_file: o.admin_password_file.clone(),
            admin_password: o.admin_password_file.as_deref().and_then(read_admin_password),
            busy: None,
        };
        Self {
            kind: Kind::Outcome,
            message: String::new(),
            next: String::new(),
            log: None,
            folder: Some(o.dir.clone()),
            notes: o.notes.clone(),
            connect: o.connect.clone(),
            txadmin_url: o.txadmin_url.clone(),
            status: None,
            hyperlinks: false,
            windows: cfg!(windows),
            can_elevate: false,
            restarted: false,
            srv,
        }
    }

    pub fn with_windows(mut self, on: bool) -> Self {
        self.windows = on;
        self
    }

    pub fn with_elevation(mut self, on: bool) -> Self {
        self.can_elevate = on;
        self
    }

    fn offers_restart(&self) -> bool {
        self.kind == Kind::Failure && self.can_elevate && is_admin_error(&self.message)
    }

    pub fn with_hyperlinks(mut self, on: bool) -> Self {
        self.hyperlinks = on;
        self
    }

    /// The path that is shown as a link (and copied by "Copy path").
    fn shown_path(&self) -> Option<&Path> {
        match self.kind {
            Kind::Failure => self.log.as_deref(),
            Kind::Outcome => self.folder.as_deref(),
        }
    }

    fn path_action(&self) -> Action {
        match self.kind {
            Kind::Failure => Action::OpenLog,
            Kind::Outcome => Action::OpenFolder,
        }
    }

    fn actions(&self) -> Vec<Action> {
        let mut v = vec![];
        if self.kind == Kind::Outcome {
            let r = &self.srv;
            let idle = r.busy.is_none();
            if r.running && idle {
                v.extend([Action::OpenTxAdmin, Action::CopyConnect]);
            } else if idle && !r.name.is_empty() {
                v.push(Action::StartServer);
            }
            if r.admin_password.is_some() {
                v.push(Action::CopyPassword);
            }
        }
        if self.kind == Kind::Failure && self.log.is_some() {
            v.push(Action::OpenLog);
        }
        if self.folder.is_some() {
            v.push(Action::OpenFolder);
        }
        if self.shown_path().is_some() {
            v.push(Action::CopyPath);
        }
        if self.kind == Kind::Outcome && self.srv.running && self.srv.busy.is_none() {
            v.push(Action::StopServer);
        }
        if self.kind == Kind::Failure {
            v.push(Action::CopyDetails);
        }
        if self.offers_restart() {
            v.insert(0, Action::Restart);
        }
        v
    }

    /// Apply messages from a background start; call regularly while `is_busy`.
    pub fn tick(&mut self, svc: &mut Services) {
        for m in svc.server.poll() {
            match m {
                ServerMsg::Progress(t) => self.srv.busy = Some(t),
                ServerMsg::Finished(r) => {
                    self.srv.busy = None;
                    self.srv.attempted = true;
                    self.srv.stopped = false;
                    self.srv.running = r.running();
                    self.srv.game_listening = r.game_listening;
                    self.srv.problem = r.problem.map(|_| r.message.clone());
                    self.srv.log_tail = r.log_tail.clone();
                    self.status = Some(if r.running() { ("Your server is running.".into(), true) } else { (r.message.clone(), false) });
                }
                ServerMsg::Failed(e) => {
                    self.srv.busy = None;
                    self.srv.attempted = true;
                    self.srv.running = false;
                    self.srv.problem = Some(e.clone());
                    self.status = Some((e, false));
                }
            }
        }
    }

    pub fn is_busy(&self) -> bool {
        self.srv.busy.is_some()
    }

    /// Run an action. Never panics; problems become a plain message on screen.
    pub fn act(&mut self, a: Action, svc: &mut Services) {
        if !self.actions().contains(&a) {
            return;
        }
        let root = self.folder.clone();
        self.status = Some(match a {
            Action::OpenLog | Action::OpenFolder => {
                let target = if a == Action::OpenLog { self.log.clone() } else { self.folder.clone() };
                match (target, root) {
                    (Some(t), Some(r)) => match svc.opener.open(&t, &r) {
                        Ok(()) => (format!("Opening {}...", t.display()), true),
                        Err(e) => (e, false),
                    },
                    _ => ("There is nothing to open.".into(), false),
                }
            }
            Action::CopyPath => match self.shown_path().map(|p| p.display().to_string()) {
                Some(p) => match svc.clipboard.set_text(&p) {
                    Ok(()) => ("Copied the path to the clipboard.".into(), true),
                    Err(e) => (format!("{e} The path is: {p}"), false),
                },
                None => ("There is no path to copy.".into(), false),
            },
            Action::OpenTxAdmin => match svc.opener.open_url(&self.txadmin_url) {
                Ok(()) => (format!("Opening {}...", self.txadmin_url), true),
                Err(e) => (e, false),
            },
            Action::CopyConnect => match svc.clipboard.set_text(&self.connect) {
                Ok(()) => (format!("Copied {}. Paste it into FiveM (F8, then connect <address>).", self.connect), true),
                Err(e) => (format!("{e} The address is: {}", self.connect), false),
            },
            Action::CopyPassword => match self.srv.admin_password.clone() {
                Some(p) => match svc.clipboard.set_text(&p) {
                    Ok(()) => ("Copied the txAdmin password. Change it after your first login.".into(), true),
                    Err(e) => (e, false),
                },
                None => ("There is no generated password (a Cfx.re account is linked).".into(), false),
            },
            Action::StartServer => {
                self.srv.busy = Some("Starting txAdmin...".into());
                self.srv.problem = None;
                self.status = None;
                svc.server.start(&self.srv.name.clone());
                return;
            }
            Action::StopServer => match svc.server.stop(&self.srv.name.clone()) {
                Ok(()) => {
                    self.srv.running = false;
                    self.srv.stopped = true;
                    self.srv.problem = None;
                    ("Stopped the server.".into(), true)
                }
                Err(e) => (e, false),
            },
            Action::Restart => match (svc.elevator)(svc.args) {
                Ok(()) => {
                    self.restarted = true;
                    ("Restarting Kora with permission...".into(), true)
                }
                Err(e) => (crate::elev::failure_note(&e, self.windows), false),
            },
            Action::CopyDetails => {
                let text = self.details_text();
                match svc.clipboard.set_text(&text) {
                    Ok(()) => ("Copied the details. Paste them into your message to support.".into(), true),
                    Err(e) => (e, false),
                }
            }
        });
    }

    /// The error plus the last lines of the install log, ready to paste to support.
    pub fn details_text(&self) -> String {
        let mut t = format!("Kora install failed: {}\n", self.message.trim());
        if let Some(log) = &self.log {
            match read_tail(log, DETAIL_LINES) {
                Ok(tail) if !tail.trim().is_empty() => {
                    t.push_str(&format!("\n--- last lines of {} ---\n{tail}\n", log.display()));
                }
                _ => t.push_str("\n(The log file could not be read.)\n"),
            }
        }
        t
    }

    /// Returns true when the screen should close.
    pub fn handle_key(&mut self, key: KeyEvent, svc: &mut Services) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return key.code == KeyCode::Char('c');
        }
        match key.code {
            KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q' | 'Q') => return true,
            KeyCode::Char('l' | 'L') => self.act(Action::OpenLog, svc),
            KeyCode::Char('f' | 'F') => self.act(Action::OpenFolder, svc),
            KeyCode::Char('c' | 'C') => self.act(Action::CopyPath, svc),
            KeyCode::Char('d' | 'D') => self.act(Action::CopyDetails, svc),
            KeyCode::Char('r' | 'R') => self.act(Action::Restart, svc),
            KeyCode::Char('o' | 'O') => self.act(Action::OpenTxAdmin, svc),
            KeyCode::Char('a' | 'A') => self.act(Action::CopyConnect, svc),
            KeyCode::Char('p' | 'P') => self.act(Action::CopyPassword, svc),
            KeyCode::Char('s' | 'S') => self.act(if self.srv.running { Action::StopServer } else { Action::StartServer }, svc),
            _ => {}
        }
        self.restarted
    }

    pub fn handle_mouse(&mut self, ev: MouseEvent, area: Rect, svc: &mut Services) {
        if ev.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        let pos = Position::new(ev.column, ev.row);
        let b = build(area, self);
        let hit = b.buttons.iter().find(|(_, r)| r.contains(pos)).map(|(a, _)| *a);
        let hit = hit.or_else(|| b.path.as_ref().filter(|p| p.rect.contains(pos)).map(|_| self.path_action()));
        if let Some(a) = hit {
            self.act(a, svc);
        }
    }

    /// OSC 8 overlay that makes the path line a real hyperlink (None when unsupported, wrapped or hidden).
    pub fn hyperlink_overlay(&self, area: Rect) -> Option<String> {
        if !self.hyperlinks {
            return None;
        }
        let p = build(area, self).path?;
        let (x, y, text) = p.link?;
        osc8_overlay(x, y, PATH_SGR, &text, &file_url(self.shown_path()?))
    }
}

/// Last `n` lines of a file, reading at most the final 64 KiB.
pub fn read_tail(path: &Path, n: usize) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    let start = len.saturating_sub(64 * 1024);
    f.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::new();
    f.take(64 * 1024).read_to_end(&mut buf)?;
    let text = String::from_utf8_lossy(&buf);
    // When we started mid-file the first line is probably cut; drop it.
    let text = if start > 0 { text.split_once('\n').map_or("", |(_, rest)| rest).to_string() } else { text.into_owned() };
    Ok(last_lines(&text, n))
}

struct PathRow {
    /// Clickable area (whole rows the path occupies).
    rect: Rect,
    /// Exact (x, y, text) of an unwrapped path, for the hyperlink overlay.
    link: Option<(u16, u16, String)>,
}

struct Built {
    lines: Vec<Line<'static>>,
    path: Option<PathRow>,
    buttons: Vec<(Action, Rect)>,
}

fn clip(r: Rect, area: Rect) -> Rect {
    r.intersection(area)
}

/// Shared by rendering and hit-testing.
fn build(area: Rect, s: &ResultScreen) -> Built {
    let inner = Rect::new(area.x + 1, area.y + 1, area.width.saturating_sub(2), area.height.saturating_sub(2));
    let w = inner.width as usize;
    let t = theme();
    let dim = t.hint();
    let bold = t.label();
    let mut lines: Vec<Line<'static>> = vec![];
    match s.kind {
        Kind::Failure => {
            lines.push(Line::from(vec![t.chip(Tone::Fail, "FAIL"), Span::styled(" The install could not finish", t.danger())]));
            lines.push(Line::raw(""));
            lines.extend(wrap(&s.message, w).into_iter().map(Line::from));
            lines.push(Line::raw(""));
            lines.push(Line::styled("What to do next:", bold));
            let mut items: Vec<String> = s.next.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect();
            if s.offers_restart() {
                items.insert(0, "Click Restart as administrator (or press R). Windows or your computer asks for permission once, then run kora install again - what was done so far is kept.".into());
            }
            if s.windows {
                items.extend(windows_hints(&s.message));
            }
            if s.log.is_some() {
                items.push("Need help? Press D (or click Copy details) and paste the text into your message to support.".into());
            }
            for (i, item) in items.iter().enumerate() {
                let prefix = format!("{}. ", i + 1);
                let pad = " ".repeat(prefix.len());
                for (j, l) in wrap(item, w.saturating_sub(prefix.len())).into_iter().enumerate() {
                    lines.push(Line::raw(format!("{}{l}", if j == 0 { &prefix } else { &pad })));
                }
            }
        }
        Kind::Outcome => {
            let r = &s.srv;
            let (head, tone, word) = if let Some(_b) = &r.busy {
                ("Starting your server...", Tone::Warn, "WAIT")
            } else if r.running {
                ("Your server is running", Tone::Ok, "OK")
            } else if r.stopped {
                ("Your server is stopped", Tone::Warn, "WARN")
            } else if r.attempted {
                ("Your server did not start", Tone::Fail, "FAIL")
            } else if r.name.is_empty() {
                ("Your server is ready!", Tone::Ok, "OK")
            } else {
                ("Your server is installed but not started", Tone::Warn, "WARN")
            };
            lines.push(Line::from(vec![t.chip(tone, word), Span::styled(format!(" {head}"), t.tone(tone).add_modifier(Modifier::BOLD))]));
            lines.push(Line::raw(""));
            if let Some(b) = &r.busy {
                lines.extend(wrap(b, w).into_iter().map(Line::from));
                lines.push(Line::raw(""));
            }
            if let (Some(p), false) = (&r.problem, r.running) {
                lines.extend(wrap(p, w).into_iter().map(|l| Line::styled(l, t.danger())));
                for l in &r.log_tail {
                    lines.extend(wrap(l, w.saturating_sub(2)).into_iter().take(2).map(|t| Line::styled(format!("| {t}"), dim)));
                }
                lines.push(Line::raw(""));
            }
            if r.running {
                lines.push(kv("Game server:       ", if r.game_listening { "listening" } else { "not listening yet (txAdmin is still starting it)" }));
            }
            lines.push(kv("Players connect:   ", &s.connect));
            lines.push(kv("Control panel:     ", &s.txadmin_url));
            if let Some(u) = &r.admin_user {
                match (&r.admin_password, &r.admin_password_file) {
                    (Some(p), _) => lines.push(kv("txAdmin login:     ", &format!("{u} / {p}"))),
                    (None, Some(f)) => lines.push(kv("txAdmin login:     ", &format!("{u} (password in {})", f.display()))),
                    _ => lines.push(kv("txAdmin login:     ", u)),
                }
            }
            lines.push(Line::raw(""));
            for n in &s.notes {
                for (i, l) in wrap(n, w.saturating_sub(2)).into_iter().enumerate() {
                    lines.push(Line::raw(format!("{}{l}", if i == 0 { format!("{} ", t.glyphs.bullet) } else { "  ".to_string() })));
                }
            }
        }
    }
    // The path, on its own line(s) so it can be clicked.
    let mut path = None;
    if let Some(p) = s.shown_path() {
        lines.push(Line::raw(""));
        let label = if s.kind == Kind::Failure { "Details were saved in (click to open):" } else { "Your server folder (click to open):" };
        lines.push(Line::raw(label));
        let text = p.display().to_string();
        let row = inner.y + lines.len() as u16;
        let shown = wrap(&format!("  {text}"), w);
        let n = shown.len().max(1) as u16;
        let link = (shown.len() == 1 && text.chars().count() + 2 <= w).then(|| (inner.x + 2, row, text.clone()));
        path = Some(PathRow { rect: clip(Rect::new(inner.x, row, inner.width, n), inner), link });
        let link_style = t.brand_style().add_modifier(Modifier::UNDERLINED);
        for l in shown {
            lines.push(Line::styled(l, link_style));
        }
    }
    // Buttons: flow left to right, wrapping onto new rows.
    lines.push(Line::raw(""));
    let mut buttons = vec![];
    let (mut x, mut row) = (0usize, inner.y + lines.len() as u16);
    for a in s.actions() {
        let bw = a.label().chars().count();
        if x > 0 && x + bw > w {
            x = 0;
            row += 1;
        }
        if x == 0 {
            lines.push(Line::raw(""));
        }
        buttons.push((a, clip(Rect::new(inner.x + x as u16, row, bw as u16, 1), inner)));
        x += bw + 2;
    }
    lines.push(Line::raw(""));
    match &s.status {
        Some((msg, ok)) => {
            let st = if *ok { t.success() } else { t.warn() };
            lines.extend(wrap(msg, w).into_iter().map(|l| Line::styled(l, st)));
        }
        None => lines.push(Line::raw("")),
    }
    lines.push(Line::styled(
        if s.kind == Kind::Failure { "Press Enter to exit." } else if s.srv.running { "Press Enter to finish (the server keeps running)." } else { "Press Enter to finish." },
        dim,
    ));
    Built { lines, path, buttons }
}

/// `Label:   value`: dim label, bold value (the text is unchanged).
fn kv(label: &str, value: &str) -> Line<'static> {
    let t = theme();
    Line::from(vec![Span::styled(label.to_string(), t.hint()), Span::styled(value.to_string(), t.label())])
}

pub fn render_result(f: &mut Frame, s: &ResultScreen) {
    if theme::guard_size(f, theme::MIN_WIDTH, theme::MIN_HEIGHT) {
        return;
    }
    let t = theme();
    let area = f.area();
    let b = build(area, s);
    let (tone, title) = match s.kind {
        Kind::Failure => (Tone::Fail, "Kora - something went wrong"),
        Kind::Outcome => (Tone::Ok, if s.srv.running { "Kora - your server is running" } else { "Kora - all done" }),
    };
    f.render_widget(Paragraph::new(b.lines.clone()).block(t.panel_tone(title, tone)), area);
    let st = t.button();
    for (a, r) in &b.buttons {
        if r.width == a.label().chars().count() as u16 && r.height > 0 {
            f.render_widget(Paragraph::new(a.label()).style(st), *r);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard::FakeClipboard;
    use crossterm::event::{KeyEventKind, KeyEventState};
    use ratatui::{backend::TestBackend, Terminal};

    fn el() -> crate::elev::Elevator {
        Box::new(|_| Err(crate::elev::ElevateError::Declined))
    }

    #[derive(Default)]
    struct FakeOpener {
        opened: Vec<(PathBuf, PathBuf)>,
        urls: Vec<String>,
        fail: bool,
    }

    impl Opener for FakeOpener {
        fn open_url(&mut self, url: &str) -> Result<(), String> {
            self.urls.push(url.to_string());
            Ok(())
        }
        fn open(&mut self, t: &Path, r: &Path) -> Result<(), String> {
            if self.fail {
                return Err(format!("Couldn't open it automatically. Open this yourself: {}", t.display()));
            }
            crate::open::validate_target(t, r)?;
            self.opened.push((t.to_path_buf(), r.to_path_buf()));
            Ok(())
        }
    }

    fn dir() -> PathBuf {
        if cfg!(windows) { PathBuf::from("C:\\Kora\\my server & co") } else { PathBuf::from("/srv/kora/my server & co") }
    }

    fn screen() -> ResultScreen {
        let log = dir().join(".kora").join("install.log");
        ResultScreen::failure(
            "Port 30120 is already in use.",
            "What was done so far is kept.\nTo try again: run `kora install` again.\nTo start over: run `kora delete X`.",
            Some(&log),
        )
    }

    fn text(s: &ResultScreen, w: u16, h: u16) -> String {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| render_result(f, s)).unwrap();
        let b = t.backend().buffer().clone();
        (0..b.area.height).map(|y| (0..b.area.width).map(|x| b[(x, y)].symbol().to_string()).collect::<String>()).collect::<Vec<_>>().join("\n")
    }

    fn click(r: Rect) -> MouseEvent {
        MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: r.x + 1, row: r.y, modifiers: KeyModifiers::NONE }
    }

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent { code: c, modifiers: KeyModifiers::NONE, kind: KeyEventKind::Press, state: KeyEventState::NONE }
    }

    #[test]
    fn failure_shows_numbered_steps_buttons_and_path() {
        let t = text(&screen(), 110, 30);
        for want in [
            "could not finish", "Port 30120", "What to do next:", "2. To try again", "3. To start over", "4. Need help?",
            "[ Open log ]", "[ Open folder ]", "[ Copy path ]", "[ Copy details ]", "install.log", "Press Enter to exit",
        ] {
            assert!(t.contains(want), "missing {want:?} in\n{t}");
        }
        for (w, h) in [(80, 24), (40, 12), (8, 3)] {
            text(&screen(), w, h); // never panics on small windows
        }
    }

    #[test]
    fn windows_failures_get_plain_extra_steps() {
        let log = dir().join(".kora").join("install.log");
        let s = ResultScreen::failure("cannot rename C:\\x: Access is denied. (os error 5)", "To try again: run kora install again.", Some(&log)).with_windows(true);
        let t = text(&s, 120, 34);
        assert!(t.contains("antivirus") && t.contains("Protection"), "{t}");
        let s = ResultScreen::failure("this needs Administrator rights", "x", None).with_windows(true);
        assert!(text(&s, 120, 34).contains("Run as administrator"));
        let s = ResultScreen::failure("Access is denied", "x", None).with_windows(false);
        assert!(!text(&s, 120, 34).contains("antivirus"), "Linux never sees Windows hints");
    }

    #[test]
    fn buttons_and_path_are_clickable() {
        let area = Rect::new(0, 0, 110, 30);
        let (mut cb, mut op) = (FakeClipboard::default(), FakeOpener::default());
        let mut s = screen();
        let b = build(area, &s);
        let rect = |a: Action| b.buttons.iter().find(|(x, _)| *x == a).unwrap().1;
        let log = dir().join(".kora").join("install.log");
        s.handle_mouse(click(rect(Action::OpenLog)), area, &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[], server: &mut NoServer });
        s.handle_mouse(click(rect(Action::OpenFolder)), area, &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[], server: &mut NoServer });
        s.handle_mouse(click(b.path.as_ref().unwrap().rect), area, &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[], server: &mut NoServer });
        assert_eq!(op.opened, vec![(log.clone(), dir()), (dir(), dir()), (log.clone(), dir())]);
        s.handle_mouse(click(rect(Action::CopyPath)), area, &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[], server: &mut NoServer });
        assert_eq!(cb.text.as_deref(), Some(log.display().to_string().as_str()));
        assert!(text(&s, 110, 30).contains("Copied the path"));
        // a click elsewhere or a right click does nothing
        let before = op.opened.len();
        s.handle_mouse(click(Rect::new(0, 0, 1, 1)), area, &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[], server: &mut NoServer });
        s.handle_mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Right), ..click(rect(Action::OpenLog)) }, area, &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[], server: &mut NoServer });
        assert_eq!(op.opened.len(), before);
    }

    #[test]
    fn keys_l_f_c_d_and_exit() {
        let (mut cb, mut op) = (FakeClipboard::default(), FakeOpener::default());
        let mut s = screen();
        for c in ['l', 'F', 'c'] {
            assert!(!s.handle_key(key(KeyCode::Char(c)), &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[], server: &mut NoServer }));
        }
        assert_eq!(op.opened.len(), 2);
        assert!(!s.handle_key(key(KeyCode::Char('x')), &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[], server: &mut NoServer }), "random keys don't close it");
        assert!(s.handle_key(key(KeyCode::Enter), &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[], server: &mut NoServer }));
        assert!(s.handle_key(key(KeyCode::Esc), &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[], server: &mut NoServer }));
        assert!(s.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL), &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[], server: &mut NoServer }));
    }

    #[test]
    fn copy_details_copies_last_40_lines_of_the_log() {
        let tmp = tempfile::tempdir().unwrap();
        let meta = tmp.path().join(".kora");
        std::fs::create_dir(&meta).unwrap();
        let log = meta.join("install.log");
        let body: String = (1..=100).map(|i| format!("line {i}\n")).collect();
        std::fs::write(&log, body).unwrap();
        let mut s = ResultScreen::failure("Boom happened", "x", Some(&log));
        let (mut cb, mut op) = (FakeClipboard::default(), FakeOpener::default());
        s.handle_key(key(KeyCode::Char('d')), &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[], server: &mut NoServer });
        let copied = cb.text.unwrap();
        assert!(copied.contains("Boom happened") && copied.contains("line 100") && copied.contains("line 61"));
        assert!(!copied.contains("line 60\n"), "only the last {DETAIL_LINES} lines");
        // a missing log doesn't break copying
        let s = ResultScreen::failure("Boom", "x", Some(&meta.join("nope.log")));
        assert!(s.details_text().contains("could not be read"));
    }

    #[test]
    fn read_tail_handles_big_files() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("big.log");
        let body: String = (1..=20000).map(|i| format!("a fairly long log line number {i}\n")).collect();
        std::fs::write(&p, body).unwrap();
        let t = read_tail(&p, 3).unwrap();
        assert_eq!(t.lines().count(), 3);
        assert!(t.ends_with("number 20000"));
    }

    #[test]
    fn failures_are_reported_plainly() {
        let area = Rect::new(0, 0, 110, 30);
        let mut cb = FakeClipboard { fail: true, ..Default::default() };
        let mut op = FakeOpener { fail: true, ..Default::default() };
        let mut s = screen();
        s.handle_key(key(KeyCode::Char('l')), &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[], server: &mut NoServer });
        let t = text(&s, 110, 30);
        assert!(t.contains("Open this yourself") && t.contains("install.log"), "{t}");
        s.handle_key(key(KeyCode::Char('c')), &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[], server: &mut NoServer });
        assert!(text(&s, 110, 30).contains("clipboard isn't available"));
        s.handle_key(key(KeyCode::Char('d')), &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[], server: &mut NoServer });
        assert!(s.status.as_ref().is_some_and(|(_, ok)| !ok));
        let _ = area;
    }

    #[test]
    fn outcome_has_folder_buttons_only() {
        let o = InstallOutcome { dir: dir(), connect: "localhost:30120".into(), txadmin_url: "http://localhost:40120".into(), notes: vec!["Forward port 30120.".into()], ..Default::default() };
        let s = ResultScreen::outcome(&o);
        let t = text(&s, 100, 24);
        assert!(t.contains("ready") && t.contains("[ Open folder ]") && t.contains("[ Copy path ]"));
        assert!(!t.contains("Open log") && !t.contains("Copy details"));
        let (mut cb, mut op) = (FakeClipboard::default(), FakeOpener::default());
        let mut s = s;
        s.act(Action::OpenLog, &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[], server: &mut NoServer });
        assert!(op.opened.is_empty(), "actions that aren't offered do nothing");
        s.handle_key(key(KeyCode::Char('f')), &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[], server: &mut NoServer });
        assert_eq!(op.opened, vec![(dir(), dir())]);
    }

    #[test]
    fn hyperlink_overlay_only_when_supported_and_unwrapped() {
        let area = Rect::new(0, 0, 110, 30);
        assert!(screen().hyperlink_overlay(area).is_none());
        let s = screen().with_hyperlinks(true);
        let o = s.hyperlink_overlay(area).unwrap();
        assert!(o.contains("\x1b]8;;file://") && o.contains("install.log"));
        // the overlay lands exactly on the rendered path text
        let t = text(&s, 110, 30);
        let path = dir().join(".kora").join("install.log").display().to_string();
        assert!(t.contains(&path));
        // too narrow to keep the path on one line: no overlay, the layout is untouched
        assert!(s.hyperlink_overlay(Rect::new(0, 0, 30, 30)).is_none());
    }

    fn admin_failure() -> ResultScreen {
        ResultScreen::failure("No MariaDB/MySQL database server was found, and setting one up needs administrator rights.", "Nothing was created.", None)
            .with_windows(true)
            .with_elevation(true)
    }

    #[test]
    fn admin_failure_offers_a_restart_button_and_a_short_fallback() {
        let t = text(&admin_failure(), 110, 30);
        assert!(t.contains("[ Restart as administrator ]"), "{t}");
        assert!(t.contains("1. Click Restart as administrator"), "{t}");
        assert!(t.contains("If the button does not work"), "{t}");
        // not offered for other errors, or when already elevated
        assert!(!text(&screen().with_elevation(true), 110, 30).contains("Restart as administrator"));
        assert!(!text(&admin_failure().with_elevation(false), 110, 30).contains("[ Restart as administrator ]"));
    }

    #[test]
    fn restart_button_and_key_call_the_callback_with_the_original_args() {
        use std::cell::RefCell;
        use std::ffi::OsString;
        use std::rc::Rc;
        let seen: Rc<RefCell<Vec<Vec<OsString>>>> = Rc::default();
        let s2 = seen.clone();
        let mut elevator: crate::elev::Elevator = Box::new(move |a| {
            s2.borrow_mut().push(a.to_vec());
            Ok(())
        });
        let args: Vec<OsString> = vec!["install".into(), "--name".into(), "X".into()];
        let (mut cb, mut op) = (FakeClipboard::default(), FakeOpener::default());
        let area = Rect::new(0, 0, 110, 30);
        let mut s = admin_failure();
        let r = build(area, &s).buttons.iter().find(|(a, _)| *a == Action::Restart).unwrap().1;
        s.handle_mouse(click(r), area, &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut elevator, args: &args, server: &mut NoServer });
        assert!(s.restarted);
        assert_eq!(seen.borrow().as_slice(), std::slice::from_ref(&args));
        let mut s = admin_failure();
        assert!(s.handle_key(key(KeyCode::Char('r')), &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut elevator, args: &args, server: &mut NoServer }), "closes after a restart");
        assert_eq!(seen.borrow().len(), 2);
    }

    #[test]
    fn declined_restart_stays_and_explains() {
        let (mut cb, mut op) = (FakeClipboard::default(), FakeOpener::default());
        let mut s = admin_failure();
        assert!(!s.handle_key(key(KeyCode::Char('r')), &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[], server: &mut NoServer }));
        assert!(!s.restarted);
        assert!(text(&s, 110, 34).contains("nothing was changed"));
    }

    #[test]
    fn outcome_and_failure_wait_for_a_key() {
        let (mut cb, mut op) = (FakeClipboard::default(), FakeOpener::default());
        let mut s = screen();
        for k in [KeyCode::Char('x'), KeyCode::Down, KeyCode::Tab] {
            assert!(!s.handle_key(key(k), &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[], server: &mut NoServer }));
        }
    }

    // ---- the "server is running" outcome -------------------------------------------------------

    #[derive(Default)]
    struct FakeServer {
        started: Vec<String>,
        stopped: Vec<String>,
        queue: Vec<ServerMsg>,
    }

    impl ServerCtl for FakeServer {
        fn start(&mut self, name: &str) {
            self.started.push(name.into());
        }
        fn poll(&mut self) -> Vec<ServerMsg> {
            std::mem::take(&mut self.queue)
        }
        fn stop(&mut self, name: &str) -> Result<(), String> {
            self.stopped.push(name.into());
            Ok(())
        }
    }

    fn outcome(running: bool, attempted: bool, problem: Option<&str>) -> InstallOutcome {
        InstallOutcome {
            dir: dir(),
            connect: "192.168.1.20:30120".into(),
            txadmin_url: "http://localhost:40120".into(),
            notes: vec![],
            name: "alpha".into(),
            start_attempted: attempted,
            running,
            game_listening: running,
            launch_problem: problem.map(String::from),
            log_tail: if problem.is_some() { vec!["Could not authenticate server license key".into()] } else { vec![] },
            ..Default::default()
        }
    }

    fn report(running: bool) -> kora_core::launch::Report {
        use kora_core::launch::{Method, Problem, Report};
        Report {
            method: Method::Detached,
            pid: Some(1),
            txadmin_up: true,
            game_listening: running,
            problem: (!running).then_some(Problem::LicenceRejected),
            message: if running { "Your server is running.".into() } else { kora_core::launch::LICENCE_MESSAGE.into() },
            log_tail: vec![],
        }
    }

    #[test]
    fn running_outcome_says_so_and_offers_the_server_buttons() {
        let pw = tempfile::tempdir().unwrap();
        let f = pw.path().join("txadmin-admin.txt");
        std::fs::write(&f, "txAdmin master account\nUsername: kora-admin\nPassword: s3cretPW\n").unwrap();
        let mut o = outcome(true, true, None);
        o.admin_user = Some("kora-admin".into());
        o.admin_password_file = Some(f);
        let mut s = ResultScreen::outcome(&o);
        let t = text(&s, 110, 30);
        for want in ["Your server is running", "[ Open txAdmin ]", "[ Copy connect address ]", "[ Open folder ]", "[ Stop server ]", "[ Copy password ]", "192.168.1.20:30120", "kora-admin / s3cretPW", "listening"] {
            assert!(t.contains(want), "missing {want:?} in\n{t}");
        }
        assert!(!t.contains("[ Start server ]"));
        let (mut cb, mut op, mut sv) = (FakeClipboard::default(), FakeOpener::default(), FakeServer::default());
        let mut svc = Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[], server: &mut sv };
        s.handle_key(key(KeyCode::Char('o')), &mut svc);
        s.handle_key(key(KeyCode::Char('p')), &mut svc);
        assert_eq!(svc.clipboard.get_text().unwrap(), "s3cretPW");
        s.handle_key(key(KeyCode::Char('a')), &mut svc);
        assert_eq!(svc.clipboard.get_text().unwrap(), "192.168.1.20:30120");
        s.handle_key(key(KeyCode::Char('s')), &mut svc);
        assert_eq!(op.urls, ["http://localhost:40120"]);
        assert_eq!(sv.stopped, ["alpha"]);
        let t = text(&s, 110, 30);
        assert!(t.contains("Your server is stopped") && t.contains("[ Start server ]") && !t.contains("[ Stop server ]"), "{t}");
    }

    #[test]
    fn a_failed_start_shows_the_reason_and_a_start_button_and_retry_works() {
        let o = outcome(false, true, Some("Your licence key was rejected. Check it at portal.cfx.re, then run kora start"));
        let mut s = ResultScreen::outcome(&o);
        let t = text(&s, 110, 30);
        for want in ["Your server did not start", "Your licence key was rejected", "Could not authenticate", "[ Start server ]", "[ Open folder ]"] {
            assert!(t.contains(want), "missing {want:?} in\n{t}");
        }
        assert!(!t.contains("[ Open txAdmin ]") && !t.contains("Stop server"));
        let (mut cb, mut op, mut sv) = (FakeClipboard::default(), FakeOpener::default(), FakeServer::default());
        let mut svc = Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[], server: &mut sv };
        s.handle_key(key(KeyCode::Char('s')), &mut svc);
        assert!(s.is_busy());
        assert!(text(&s, 110, 30).contains("Starting your server..."));
        assert!(!text(&s, 110, 30).contains("[ Start server ]"), "no double start while busy");
        svc.server.start("never");
        sv.queue = vec![ServerMsg::Progress("Waiting for the game server...".into())];
        let mut svc = Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[], server: &mut sv };
        s.tick(&mut svc);
        assert!(text(&s, 110, 30).contains("Waiting for the game server..."));
        sv.queue = vec![ServerMsg::Finished(report(true))];
        let mut svc = Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[], server: &mut sv };
        s.tick(&mut svc);
        assert!(!s.is_busy());
        let t = text(&s, 110, 30);
        assert!(t.contains("Your server is running") && t.contains("[ Stop server ]"), "{t}");
        assert_eq!(sv.started[0], "alpha");
    }

    #[test]
    fn not_started_says_exactly_that() {
        let s = ResultScreen::outcome(&outcome(false, false, None));
        let t = text(&s, 110, 30);
        assert!(t.contains("installed but not started") && t.contains("[ Start server ]") && !t.contains("did not start"), "{t}");
        // old callers (no server name) keep the plain screen
        let plain = ResultScreen::outcome(&InstallOutcome { dir: dir(), ..Default::default() });
        assert!(!text(&plain, 110, 30).contains("Start server"));
    }

    #[test]
    fn a_failed_background_start_is_shown_plainly() {
        let mut s = ResultScreen::outcome(&outcome(false, false, None));
        let (mut cb, mut op, mut sv) = (FakeClipboard::default(), FakeOpener::default(), FakeServer::default());
        sv.queue = vec![ServerMsg::Failed("Could not start the server: FXServer.exe is missing".into())];
        let mut svc = Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[], server: &mut sv };
        s.tick(&mut svc);
        let t = text(&s, 110, 30);
        assert!(t.contains("did not start") && t.contains("FXServer.exe is missing"), "{t}");
    }

    #[test]
    fn the_password_is_never_in_the_outcome_data() {
        let o = outcome(true, true, None);
        let json = serde_json::to_string(&o).unwrap();
        assert!(!json.to_lowercase().contains("password\":\"") && !format!("{o:?}").contains("s3cret"));
    }
}
