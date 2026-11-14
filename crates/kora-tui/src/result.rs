//! Failure and "all done" screens with clickable actions: open the log / folder, copy the path,
//! copy the last lines of the log. State, layout and key/mouse handling are pure (the clipboard
//! and the opener are injected), so everything is testable with ratatui's `TestBackend`.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use kora_core::installer::InstallOutcome;
use ratatui::{
    layout::{Position, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::clipboard::{last_lines, Clipboard};
use crate::open::{file_url, osc8_overlay, root_for_log, Opener};
use crate::wizard_view::{wrap, BRAND};

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
}

impl Action {
    fn label(self) -> &'static str {
        match self {
            Action::OpenLog => "[ Open log ]",
            Action::OpenFolder => "[ Open folder ]",
            Action::CopyPath => "[ Copy path ]",
            Action::CopyDetails => "[ Copy details ]",
            Action::Restart => "[ Restart as administrator ]",
        }
    }
}

/// What the actions need from the outside world.
pub struct Services<'a> {
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
        }
    }

    pub fn outcome(o: &InstallOutcome) -> Self {
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
        if self.kind == Kind::Failure && self.log.is_some() {
            v.push(Action::OpenLog);
        }
        if self.folder.is_some() {
            v.push(Action::OpenFolder);
        }
        if self.shown_path().is_some() {
            v.push(Action::CopyPath);
        }
        if self.kind == Kind::Failure {
            v.push(Action::CopyDetails);
        }
        if self.offers_restart() {
            v.insert(0, Action::Restart);
        }
        v
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
    let dim = Style::default().fg(Color::DarkGray);
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let mut lines: Vec<Line<'static>> = vec![];
    match s.kind {
        Kind::Failure => {
            lines.push(Line::styled("The install could not finish", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)));
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
            lines.push(Line::styled("Your server is ready!", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)));
            lines.push(Line::raw(""));
            lines.push(Line::raw(format!("Players connect:   {}", s.connect)));
            lines.push(Line::raw(format!("Control panel:     {}", s.txadmin_url)));
            lines.push(Line::raw(""));
            for n in &s.notes {
                for (i, l) in wrap(n, w.saturating_sub(2)).into_iter().enumerate() {
                    lines.push(Line::raw(format!("{}{l}", if i == 0 { "• " } else { "  " })));
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
        let link_style = Style::default().fg(BRAND).add_modifier(Modifier::UNDERLINED);
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
        Some((t, ok)) => {
            let st = if *ok { Style::default().fg(Color::Green) } else { Style::default().fg(Color::Yellow) };
            lines.extend(wrap(t, w).into_iter().map(|l| Line::styled(l, st)));
        }
        None => lines.push(Line::raw("")),
    }
    lines.push(Line::styled(
        if s.kind == Kind::Failure { "Press Enter to exit." } else { "Press Enter to finish." },
        dim,
    ));
    Built { lines, path, buttons }
}

pub fn render_result(f: &mut Frame, s: &ResultScreen) {
    let area = f.area();
    let b = build(area, s);
    let (colour, title) = match s.kind {
        Kind::Failure => (Color::Red, " Kora - something went wrong "),
        Kind::Outcome => (BRAND, " Kora - all done "),
    };
    f.render_widget(
        Paragraph::new(b.lines.clone()).block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(colour)).title(title)),
        area,
    );
    let st = Style::default().fg(Color::Black).bg(Color::Gray);
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
        fail: bool,
    }

    impl Opener for FakeOpener {
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
        s.handle_mouse(click(rect(Action::OpenLog)), area, &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[] });
        s.handle_mouse(click(rect(Action::OpenFolder)), area, &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[] });
        s.handle_mouse(click(b.path.as_ref().unwrap().rect), area, &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[] });
        assert_eq!(op.opened, vec![(log.clone(), dir()), (dir(), dir()), (log.clone(), dir())]);
        s.handle_mouse(click(rect(Action::CopyPath)), area, &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[] });
        assert_eq!(cb.text.as_deref(), Some(log.display().to_string().as_str()));
        assert!(text(&s, 110, 30).contains("Copied the path"));
        // a click elsewhere or a right click does nothing
        let before = op.opened.len();
        s.handle_mouse(click(Rect::new(0, 0, 1, 1)), area, &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[] });
        s.handle_mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Right), ..click(rect(Action::OpenLog)) }, area, &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[] });
        assert_eq!(op.opened.len(), before);
    }

    #[test]
    fn keys_l_f_c_d_and_exit() {
        let (mut cb, mut op) = (FakeClipboard::default(), FakeOpener::default());
        let mut s = screen();
        for c in ['l', 'F', 'c'] {
            assert!(!s.handle_key(key(KeyCode::Char(c)), &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[] }));
        }
        assert_eq!(op.opened.len(), 2);
        assert!(!s.handle_key(key(KeyCode::Char('x')), &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[] }), "random keys don't close it");
        assert!(s.handle_key(key(KeyCode::Enter), &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[] }));
        assert!(s.handle_key(key(KeyCode::Esc), &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[] }));
        assert!(s.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL), &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[] }));
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
        s.handle_key(key(KeyCode::Char('d')), &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[] });
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
        s.handle_key(key(KeyCode::Char('l')), &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[] });
        let t = text(&s, 110, 30);
        assert!(t.contains("Open this yourself") && t.contains("install.log"), "{t}");
        s.handle_key(key(KeyCode::Char('c')), &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[] });
        assert!(text(&s, 110, 30).contains("clipboard isn't available"));
        s.handle_key(key(KeyCode::Char('d')), &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[] });
        assert!(s.status.as_ref().is_some_and(|(_, ok)| !ok));
        let _ = area;
    }

    #[test]
    fn outcome_has_folder_buttons_only() {
        let o = InstallOutcome { dir: dir(), connect: "localhost:30120".into(), txadmin_url: "http://localhost:40120".into(), notes: vec!["Forward port 30120.".into()] };
        let s = ResultScreen::outcome(&o);
        let t = text(&s, 100, 24);
        assert!(t.contains("ready") && t.contains("[ Open folder ]") && t.contains("[ Copy path ]"));
        assert!(!t.contains("Open log") && !t.contains("Copy details"));
        let (mut cb, mut op) = (FakeClipboard::default(), FakeOpener::default());
        let mut s = s;
        s.act(Action::OpenLog, &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[] });
        assert!(op.opened.is_empty(), "actions that aren't offered do nothing");
        s.handle_key(key(KeyCode::Char('f')), &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[] });
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
        s.handle_mouse(click(r), area, &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut elevator, args: &args });
        assert!(s.restarted);
        assert_eq!(seen.borrow().as_slice(), std::slice::from_ref(&args));
        let mut s = admin_failure();
        assert!(s.handle_key(key(KeyCode::Char('r')), &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut elevator, args: &args }), "closes after a restart");
        assert_eq!(seen.borrow().len(), 2);
    }

    #[test]
    fn declined_restart_stays_and_explains() {
        let (mut cb, mut op) = (FakeClipboard::default(), FakeOpener::default());
        let mut s = admin_failure();
        assert!(!s.handle_key(key(KeyCode::Char('r')), &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[] }));
        assert!(!s.restarted);
        assert!(text(&s, 110, 34).contains("nothing was changed"));
    }

    #[test]
    fn outcome_and_failure_wait_for_a_key() {
        let (mut cb, mut op) = (FakeClipboard::default(), FakeOpener::default());
        let mut s = screen();
        for k in [KeyCode::Char('x'), KeyCode::Down, KeyCode::Tab] {
            assert!(!s.handle_key(key(k), &mut Services { clipboard: &mut cb, opener: &mut op, elevator: &mut el(), args: &[] }));
        }
    }
}
