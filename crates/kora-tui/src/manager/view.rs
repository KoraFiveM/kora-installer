//! Drawing and hit-testing of the manager. One `build` produces both the text and the clickable
//! rectangles of the current screen, so what is drawn is exactly what can be clicked.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use kora_core::installer::Mode;

use kora_core::service::{self as schedule, Preset, RunAs};
use kora_core::update::manual::ChangeStatus;
use super::addres::{AddResStage, AddResState, FbKind, FbStage, FeedbackState, ListView};
use super::app::*;
use super::console::{self, ConsoleState, Tone};
use super::backend::Status;
use super::input::TextInput;
use crate::wizard_view::{wrap, BRAND};

const DIM: Style = Style::new().fg(Color::DarkGray);
const BOLD: Style = Style::new().add_modifier(Modifier::BOLD);
const BTN: Style = Style::new().fg(Color::Black).bg(Color::Gray);
const BTN_OFF: Style = Style::new().fg(Color::DarkGray);

/// A panel's text area plus where its buttons and rows are.
struct Page {
    inner: Rect,
    lines: Vec<Line<'static>>,
    hits: Vec<(Hit, Rect)>,
}

impl Page {
    fn new(outer: Rect) -> Self {
        let inner = Rect::new(outer.x + 1, outer.y + 1, outer.width.saturating_sub(2), outer.height.saturating_sub(2));
        Self { inner, lines: vec![], hits: vec![] }
    }

    fn w(&self) -> usize {
        self.inner.width as usize
    }

    fn room(&self) -> usize {
        (self.inner.height as usize).saturating_sub(self.lines.len())
    }

    fn line(&mut self, l: impl Into<Line<'static>>) {
        if self.room() > 0 {
            self.lines.push(l.into());
        }
    }

    fn text(&mut self, s: &str, style: Style) {
        for l in wrap(s, self.w().max(1)) {
            self.line(Line::styled(l, style));
        }
    }

    fn blank(&mut self) {
        self.line(Line::raw(""));
    }

    /// A clickable row.
    fn row(&mut self, h: Hit, l: Line<'static>) {
        if self.room() > 0 {
            let y = self.inner.y + self.lines.len() as u16;
            self.hits.push((h, Rect::new(self.inner.x, y, self.inner.width, 1)));
            self.lines.push(l);
        }
    }

    /// Buttons flowing left to right, wrapping onto new rows. Disabled ones are drawn dim and do nothing.
    fn buttons(&mut self, items: &[(Action, String, bool)]) {
        let w = self.w();
        let mut spans: Vec<Span<'static>> = vec![];
        let mut x = 0usize;
        let flush = |p: &mut Page, spans: &mut Vec<Span<'static>>| {
            if !spans.is_empty() {
                p.line(Line::from(std::mem::take(spans)));
            }
        };
        for (a, label, on) in items {
            let text = format!(" {label} ");
            let bw = text.chars().count();
            if x > 0 && x + bw > w {
                flush(self, &mut spans);
                x = 0;
            }
            if self.room() == 0 {
                return;
            }
            if *on {
                let y = self.inner.y + self.lines.len() as u16;
                let r = Rect::new(self.inner.x + x as u16, y, (bw as u16).min(self.inner.width.saturating_sub(x as u16)), 1);
                self.hits.push((Hit::Act(*a), r));
            }
            spans.push(Span::styled(text, if *on { BTN } else { BTN_OFF }));
            spans.push(Span::raw("  "));
            x += bw + 2;
        }
        flush(self, &mut spans);
    }
}

fn btn(a: Action, label: &str, on: bool) -> (Action, String, bool) {
    (a, label.to_string(), on)
}

fn field(label: &str, input: &TextInput, w: usize, focused: bool) -> Line<'static> {
    let mut v = vec![Span::styled(format!("{label} "), BOLD)];
    v.extend(input.spans(w.saturating_sub(label.chars().count() + 2), focused));
    Line::from(v)
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let t: String = s.chars().take(n.saturating_sub(1)).collect();
        format!("{t}…")
    }
}

fn status_span(s: Status) -> Span<'static> {
    match s {
        Status::Running => Span::styled("● running", Style::default().fg(Color::Green)),
        Status::Stopped => Span::styled("○ stopped", DIM),
        Status::Missing => Span::styled("! missing", Style::default().fg(Color::Red)),
    }
}

fn badge_style(b: super::backend::UpdateBadge) -> Style {
    use super::backend::UpdateBadge as B;
    match b {
        B::UpToDate => Style::default().fg(Color::Green),
        B::Updates(_) | B::NewRelease => Style::default().fg(Color::Yellow),
        B::NeedsAttention => Style::default().fg(Color::Red),
    }
}

fn mode_text(m: Mode) -> &'static str {
    match m {
        Mode::Production => "production",
        Mode::Development => "development",
    }
}

pub struct Built {
    panels: Vec<(Rect, String, Color, Vec<Line<'static>>)>,
    pub hits: Vec<(Hit, Rect)>,
    footer: Vec<Line<'static>>,
    outer: Rect,
}

pub fn hits(app: &App, area: Rect) -> Vec<(Hit, Rect)> {
    build(app, area).hits
}

fn help_for(app: &App) -> &'static str {
    match &app.screen {
        Screen::Main => "↑↓ select  s start  x stop  r restart  t txAdmin  o folder  l logs  e ensure  u update  v check  n notify  c schedule  d delete  a add  q quit",
        Screen::Console(_) => "type a command, Enter sends  ↑↓ earlier commands  PgUp PgDn scroll  F2 turn on  Esc back",
        Screen::Logs(_) => "↑↓ PgUp PgDn scroll  Tab next file  c copy  r reload  Esc back",
        Screen::Ensure(_) => "type a name or ↑↓ pick  Enter ensure  Tab restart on/off  Esc back",
        Screen::Delete(s) if s.stage == DelStage::Name => "type the name, Enter confirms  Esc cancels",
        Screen::Delete(_) => "y yes  n no  Esc cancel",
        Screen::Update(s) if s.confirm_rollback.is_some() => "y roll back  n / Esc cancel",
        Screen::Update(s) if s.stage == UpStage::Snapshots => "↑↓ pick  Enter / b roll back  Esc back",
        Screen::Update(s) if s.stage == UpStage::Report => "b roll back  s snapshots  Esc back",
        Screen::Update(_) => "↑↓ move  Space tick  a all  Enter update  r restart policy  s snapshots  Esc back",
        Screen::Notify(_) => "paste the webhook  Enter save  F4 paste from clipboard  F2 send test  F3 remove  Esc back",
        Screen::Schedule(_) => "↑↓ field  ←→ change  Enter save  F3 remove  Esc back",
        Screen::Add(_) => "↑↓ pick  Enter add  p type a folder  Esc back",
        Screen::AddRes(s) => match s.stage {
            AddResStage::Menu => "c website code  b browse the catalog  Esc back",
            AddResStage::Code => "paste the code, Enter checks it  Esc back",
            AddResStage::Sections => "↑↓ pick  Enter open  i review and install  Esc back",
            AddResStage::Items => "↑↓ move  Space tick  a all/none  Enter open  i review and install  Esc back",
            AddResStage::Review => "Enter / y install  n or Esc back",
            AddResStage::Result => "Esc back",
        },
        Screen::Feedback(s) => match s.stage {
            FbStage::Pick => "↑↓ pick a resource  Enter next  Esc back",
            FbStage::Compose => "type  Tab next field  Enter send  Esc back",
            FbStage::Done => "c copy the address  Esc back",
        },
    }
}

pub fn build(app: &App, area: Rect) -> Built {
    let outer = area;
    let body_outer = Rect::new(area.x + 1, area.y + 1, area.width.saturating_sub(2), area.height.saturating_sub(4));
    let mut b = Built { panels: vec![], hits: vec![], footer: vec![], outer };
    // footer: notice + key help
    let w = area.width.saturating_sub(2) as usize;
    let note = match (&app.busy, &app.notice) {
        (Some(j), _) => Line::styled(j.text(), Style::default().fg(Color::Yellow)),
        (None, Some(n)) => Line::styled(
            trunc(&n.text, w),
            Style::default().fg(match n.level {
                Level::Info => Color::Gray,
                Level::Good => Color::Green,
                Level::Bad => Color::Red,
            }),
        ),
        _ => Line::raw(""),
    };
    b.footer = vec![note, Line::styled(trunc(help_for(app), w), DIM)];
    if let Some(j) = &app.busy {
        let r = Rect::new(body_outer.x, body_outer.y, body_outer.width, body_outer.height.min(5));
        let mut p = Page::new(r);
        p.blank();
        p.text(&j.text(), Style::default().fg(Color::Yellow));
        p.text("Please wait; the screen updates when it is done.", DIM);
        b.panels.push((r, " Working ".into(), Color::Yellow, p.lines));
        return b;
    }
    match &app.screen {
        Screen::Main => main(app, body_outer, &mut b),
        other => {
            let (title, page) = screen(app, other, body_outer);
            b.hits = page.hits;
            b.panels.push((body_outer, title, BRAND, page.lines));
        }
    }
    b
}

fn main(app: &App, body: Rect, b: &mut Built) {
    let wide = body.width >= 90;
    let (lr, rr) = if wide {
        let lw = (body.width * 2 / 5).clamp(34, 56);
        (Rect::new(body.x, body.y, lw, body.height), Rect::new(body.x + lw, body.y, body.width - lw, body.height))
    } else {
        let lh = ((app.rows.len() as u16 + 4).max(6)).min(body.height / 2).max(5);
        (Rect::new(body.x, body.y, body.width, lh), Rect::new(body.x, body.y + lh, body.width, body.height.saturating_sub(lh)))
    };
    // ---- list
    let mut p = Page::new(lr);
    if let Some(e) = &app.load_error {
        p.text(e, Style::default().fg(Color::Red));
        p.blank();
        p.buttons(&[btn(Action::Refresh, "Try again (R)", true)]);
    } else if app.rows.is_empty() {
        p.blank();
        p.text("No servers yet - install one", BOLD);
        p.text("A new server takes a few minutes. Servers you already run can be added too.", DIM);
        p.blank();
        p.buttons(&[btn(Action::InstallNew, "Install a new server (i)", true), btn(Action::Add, "Add an existing server (a)", true)]);
    } else {
        let visible = p.room().saturating_sub(2).max(1);
        let start = app.sel.saturating_sub(visible - 1).min(app.rows.len().saturating_sub(visible));
        let wnd = p.w();
        for (i, r) in app.rows.iter().enumerate().skip(start).take(visible) {
            let sel = i == app.sel;
            let mut spans = vec![Span::styled(if sel { "▶ " } else { "  " }, Style::default().fg(BRAND))];
            let name = format!("{:<14}", trunc(&r.entry.name, 14));
            spans.push(Span::styled(name, if sel { Style::default().fg(BRAND).add_modifier(Modifier::BOLD) } else { BOLD }));
            spans.push(Span::raw(" "));
            spans.push(status_span(r.status));
            if wnd >= 40 {
                if let Some(bd) = r.badge {
                    spans.push(Span::raw("  "));
                    spans.push(Span::styled(bd.text(), badge_style(bd)));
                }
            }
            p.row(Hit::Row(i), Line::from(spans));
        }
        p.blank();
        p.buttons(&[btn(Action::Add, "Add server (a)", true)]);
    }
    b.hits.extend(p.hits);
    b.panels.push((lr, format!(" Servers ({}) ", app.rows.len()), BRAND, p.lines));
    // ---- detail
    let mut p = Page::new(rr);
    let title = match app.current() {
        Some(r) => format!(" {} ", r.entry.name),
        None => " Details ".into(),
    };
    if let Some(r) = app.current() {
        let e = &r.entry;
        let mut st = vec![Span::styled("Status   ", DIM), status_span(r.status)];
        if let Some(bd) = r.badge {
            st.push(Span::raw("   "));
            st.push(Span::styled(bd.text(), badge_style(bd)));
        }
        p.line(Line::from(st));
        p.line(Line::from(vec![Span::styled("Mode     ", DIM), Span::raw(mode_text(e.mode))]));
        p.line(Line::from(vec![Span::styled("Game     ", DIM), Span::raw(format!("port {}", e.ports.game))]));
        p.line(Line::from(vec![Span::styled("txAdmin  ", DIM), Span::raw(format!("http://127.0.0.1:{}", e.ports.txadmin))]));
        p.text(&format!("Folder   {}", e.dir.display()), Style::default());
        if !r.managed {
            p.text("Added from an existing folder: Kora manages it but did not create it.", DIM);
        }
        p.blank();
        let there = r.status != Status::Missing;
        let run = r.status == Status::Running;
        p.buttons(&[
            btn(Action::Start, "Start (s)", there && !run),
            btn(Action::Stop, "Stop (x)", there && run),
            btn(Action::Restart, "Restart (r)", there && run),
            btn(Action::OpenTx, "Open txAdmin (t)", there),
            btn(Action::OpenFolder, "Open folder (o)", there),
            btn(Action::Logs, "View logs (l)", there),
            btn(Action::Console, "Live console (w)", there),
            btn(Action::Ensure, "Ensure a resource (e)", there),
            btn(Action::AddResources, "Add resources (g)", there),
            btn(Action::ReportIssue, "Report an issue (b)", there),
            btn(Action::OpenPr, "Open pull request (p)", there && e.mode == Mode::Development),
            btn(Action::Update, "Update (u)", there),
            btn(Action::CheckUpdates, "Check for updates (v)", there),
            btn(Action::Notify, "Notifications (n)", true),
            btn(Action::Schedule, "Schedule (c)", true),
            btn(Action::Delete, if r.managed { "Delete (d)" } else { "Remove from list (d)" }, true),
        ]);
    } else {
        p.text("Pick a server on the left to see what you can do with it.", DIM);
    }
    b.hits.extend(p.hits);
    b.panels.push((rr, title, BRAND, p.lines));
}

fn screen(app: &App, s: &Screen, body: Rect) -> (String, Page) {
    let mut p = Page::new(body);
    let name = app.current().map(|r| r.entry.name.clone()).unwrap_or_default();
    let title = match s {
        Screen::Main => String::new(),
        Screen::Console(c) => {
            console_screen(&mut p, c);
            format!(" Live console - {name} ")
        }
        Screen::Logs(l) => {
            logs(&mut p, l);
            format!(" Logs - {name} ")
        }
        Screen::Ensure(s) => {
            ensure(&mut p, s);
            format!(" Ensure a resource - {name} ")
        }
        Screen::Delete(s) => {
            delete(&mut p, s);
            format!(" Delete - {name} ")
        }
        Screen::Update(s) => {
            update(&mut p, s);
            format!(" Update - {name} ")
        }
        Screen::Notify(s) => {
            notify(&mut p, s);
            format!(" Notifications - {name} ")
        }
        Screen::Schedule(s) => {
            schedule(&mut p, s);
            format!(" Automatic updates - {name} ")
        }
        Screen::Add(s) => {
            add(&mut p, s);
            " Add a server ".into()
        }
        Screen::AddRes(s) => {
            add_resources(&mut p, s);
            format!(" Add resources - {name} ")
        }
        Screen::Feedback(s) => {
            feedback(&mut p, s);
            format!(" {} - {name} ", s.heading())
        }
    };
    (title, p)
}

fn back() -> (Action, String, bool) {
    btn(Action::Back, "Back (Esc)", true)
}

fn console_screen(p: &mut Page, c: &ConsoleState) {
    let src = c.source_name();
    p.line(Line::from(vec![
        Span::styled("Live ", Style::default().fg(BRAND).add_modifier(Modifier::BOLD)),
        Span::styled(match &src { Some(n) => format!("following {n}"), None => "no output yet: start the server".into() }, DIM),
        Span::raw("   "),
        if c.enabled { Span::styled("● commands on", Style::default().fg(BRAND)) } else { Span::styled("○ commands off", Style::default().fg(Color::Yellow)) },
    ]));
    p.buttons(&[
        btn(Action::Send, "Send (Enter)", c.enabled && !c.input.is_empty()),
        btn(Action::EnableConsole, "Turn on commands (F2)", !c.enabled),
        btn(Action::PageUp, "Older", !c.lines.is_empty()),
        btn(Action::PageDown, "Newer", c.scroll > 0),
        btn(Action::End, "Latest", c.scroll > 0),
        back(),
    ]);
    let w = p.w();
    // Keep three rows for the separator, the note and the input box.
    let room = p.room().saturating_sub(3);
    for line in c.window(room) {
        let style = match console::tone(line) {
            Tone::Plain => Style::default(),
            Tone::Typed => Style::default().fg(BRAND).add_modifier(Modifier::BOLD),
            Tone::Error => Style::default().fg(Color::Red),
            Tone::Warn => Style::default().fg(Color::Yellow),
        };
        p.line(Line::styled(trunc(line, w), style));
    }
    while p.room() > 3 {
        p.blank();
    }
    let status = if c.scroll > 0 { format!("scrolled up {} lines (End shows the latest)", c.scroll) } else { "following the latest output".to_string() };
    p.line(Line::styled(format!("{}  {}", "─".repeat(6), status), DIM));
    match (&c.note, c.enabled) {
        (Some(n), _) => p.line(Line::styled(trunc(n, w), Style::default().fg(BRAND))),
        (None, false) => p.line(Line::styled(trunc("Commands need the Kora console helper in the server: press F2 once, then restart the server.", w), Style::default().fg(Color::Yellow))),
        (None, true) => p.blank(),
    }
    let mut v = vec![Span::styled("> ", Style::default().fg(BRAND).add_modifier(Modifier::BOLD))];
    v.extend(c.input.spans(w.saturating_sub(3), true));
    p.line(Line::from(v));
}

fn logs(p: &mut Page, l: &logs::LogView) {
    let mut tabs: Vec<Span<'static>> = vec![];
    for (i, s) in l.sources.iter().enumerate() {
        let st = if i == l.idx { Style::default().fg(BRAND).add_modifier(Modifier::BOLD | Modifier::UNDERLINED) } else { DIM };
        tabs.push(Span::styled(s.name.to_string(), st));
        tabs.push(Span::raw("   "));
    }
    if tabs.is_empty() {
        p.text("No log files yet. They appear once the server has run.", DIM);
    } else {
        p.line(Line::from(tabs));
    }
    p.buttons(&[
        btn(Action::PrevFile, "◀ File", l.sources.len() > 1),
        btn(Action::NextFile, "File ▶", l.sources.len() > 1),
        btn(Action::Copy, "Copy (c)", !l.lines.is_empty()),
        btn(Action::Reload, "Reload (r)", !l.sources.is_empty()),
        btn(Action::Top, "Top", !l.lines.is_empty()),
        btn(Action::End, "End", !l.lines.is_empty()),
        back(),
    ]);
    if let Some(e) = &l.error {
        p.text(e, Style::default().fg(Color::Red));
    }
    let room = p.room().saturating_sub(1);
    let w = p.w();
    for line in l.window(room) {
        p.line(Line::raw(trunc(line, w)));
    }
    if !l.lines.is_empty() {
        let end = l.lines.len() - l.scroll;
        let note = format!("line {end} of {}{}", l.lines.len(), if l.scroll == 0 { " (end)" } else { "" });
        // keep the position note on the last row
        while p.room() > 1 {
            p.blank();
        }
        p.line(Line::styled(note, DIM));
    }
}

fn list_row(p: &mut Page, i: usize, sel: bool, check: Option<bool>, label: &str, note: &str) {
    let w = p.w();
    let mark = match check {
        Some(true) => "[x] ",
        Some(false) => "[ ] ",
        None => "",
    };
    let mut spans = vec![Span::styled(if sel { "▶ " } else { "  " }, Style::default().fg(BRAND)), Span::raw(mark)];
    spans.push(Span::styled(trunc(label, w.saturating_sub(8 + note.chars().count().min(w / 2)).max(8)), if sel { Style::default().fg(BRAND).add_modifier(Modifier::BOLD) } else { Style::default() }));
    if !note.is_empty() {
        spans.push(Span::styled(format!("  {note}"), DIM));
    }
    p.row(Hit::Row(i), Line::from(spans));
}

fn add_resources(p: &mut Page, s: &AddResState) {
    if let Some(n) = &s.catalog_note {
        p.text(n, DIM);
    }
    match s.stage {
        AddResStage::Menu => {
            p.text("Add more resources to this server: compatibility layers, bridges to other frameworks, and resources from creators. They are downloaded at the version the catalog pins, put in their own folders and loaded with one `ensure` line per folder. Nothing you already have is moved.", DIM);
            p.blank();
            p.buttons(&[btn(Action::UseCode, "I have a website code (c)", s.cat.is_some()), btn(Action::Browse, "Browse the catalog (b)", s.cat.is_some()), back()]);
        }
        AddResStage::Code => {
            p.text("Make your choice on the Kora website, then paste the code it gives you. A code works once and lasts 30 minutes.", DIM);
            p.blank();
            let w = p.w();
            p.line(field("Code:", &s.code, w, true));
            p.blank();
            p.buttons(&[btn(Action::Confirm, "Check code (Enter)", true), back()]);
        }
        AddResStage::Sections => {
            p.text("Pick a section. Nothing is chosen for you; whatever a choice needs is added automatically.", DIM);
            p.blank();
            for (i, r) in s.section_rows().iter().enumerate() {
                list_row(p, i, i == s.sel, None, &r.label, &r.note);
            }
            p.blank();
            p.buttons(&[btn(Action::Install, "Review and install (i)", !s.picked.is_empty()), back()]);
        }
        AddResStage::Items => {
            p.line(Line::styled(s.list_title(), BOLD));
            if let Some(b) = s.creator_blurb().filter(|b| !b.is_empty()) {
                p.text(&b, DIM);
            }
            let rows = s.item_rows();
            let room = p.room().saturating_sub(3).max(1);
            let start = s.sel.saturating_sub(room - 1);
            if rows.is_empty() {
                p.text("Nothing here yet.", DIM);
            }
            for (i, r) in rows.iter().enumerate().skip(start).take(room) {
                list_row(p, i, i == s.sel, r.checked, &r.label, &r.note);
            }
            p.blank();
            let creators = s.view == ListView::CreatorIndex;
            p.buttons(&[
                btn(Action::Toggle, "Tick (Space)", !creators),
                btn(Action::ToggleAll, "All / none (a)", !creators),
                btn(Action::Install, &format!("Review and install ({})", s.picked.len()), !s.picked.is_empty()),
                back(),
            ]);
        }
        AddResStage::Review => {
            p.text("These will be added. Each is downloaded at its pinned version (resources of someone else are never forked), put in its folder, and the folder is added to server.cfg if needed.", DIM);
            p.blank();
            if let Some(plan) = &s.plan {
                for i in &plan.items {
                    let how = if i.third_party { "pinned" } else if i.fork { "your fork" } else if i.commit.is_some() { "pinned" } else { "latest" };
                    let line = format!("  {}{}  ->  {}  ({how})", i.id, if i.dependency { " (needed)" } else { "" }, i.rel);
                    p.line(Line::raw(trunc(&line, p.w())));
                }
                if !plan.already.is_empty() {
                    p.text(&format!("Already installed: {}", plan.already.join(", ")), DIM);
                }
                p.blank();
                p.text(&format!("server.cfg gets: {}", plan.ensure.iter().map(|f| format!("ensure {f}")).collect::<Vec<_>>().join(", ")), DIM);
            }
            p.blank();
            p.buttons(&[btn(Action::Confirm, "Install (Enter)", s.plan.as_ref().is_some_and(|x| !x.is_empty())), back()]);
        }
        AddResStage::Result => {
            if let Some(o) = &s.outcome {
                for id in &o.installed {
                    p.line(Line::styled(format!("  added  {id}"), Style::default().fg(Color::Green)));
                }
                for (id, e) in &o.failed {
                    p.line(Line::styled(trunc(&format!("  failed {id}: {e}"), p.w()), Style::default().fg(Color::Red)));
                }
                for n in &o.notes {
                    p.text(n, DIM);
                }
                if !o.ensured.is_empty() {
                    p.text(&format!("Added to server.cfg: {}", o.ensured.iter().map(|f| format!("ensure {f}")).collect::<Vec<_>>().join(", ")), DIM);
                }
                if !o.installed.is_empty() {
                    p.text("Restart the server so it loads the new resources.", Style::default().fg(Color::Yellow));
                }
            }
            p.blank();
            p.buttons(&[back()]);
        }
    }
}

fn feedback(p: &mut Page, s: &FeedbackState) {
    match s.stage {
        FbStage::Pick => {
            p.text(
                match s.kind {
                    FbKind::Issue => "Which resource is the report about? It goes to that resource's GitHub page, with your Kora version and system (never passwords, keys or folders).",
                    FbKind::Pr => "Which resource has the change you want to propose? Kora checks your fork first.",
                },
                DIM,
            );
            p.blank();
            let room = p.room().saturating_sub(3).max(1);
            let start = s.sel.saturating_sub(room - 1);
            for (i, c) in s.choices.iter().enumerate().skip(start).take(room) {
                list_row(p, i, i == s.sel, None, c, "");
            }
            p.blank();
            p.buttons(&[btn(Action::Confirm, "Next (Enter)", !s.choices.is_empty()), back()]);
        }
        FbStage::Compose => {
            p.line(Line::from(vec![Span::styled("Resource: ", BOLD), Span::raw(s.resource.clone())]));
            if let Some(pl) = &s.plan {
                p.text(
                    &format!("Sending will push branch '{}' of your fork to GitHub and open a pull request into {} ({}), with {} new commit(s).", pl.head_branch, pl.upstream, pl.base, pl.ahead),
                    Style::default().fg(Color::Yellow),
                );
            } else {
                p.text("This is public on GitHub, under your account.", DIM);
            }
            p.blank();
            let w = p.w();
            p.line(field("Title:", &s.title, w, s.focus == 0));
            p.line(field("Details:", &s.body, w, s.focus == 1));
            p.blank();
            p.buttons(&[btn(Action::Confirm, if s.kind == FbKind::Issue { "Send report (Enter)" } else { "Open pull request (Enter)" }, true), btn(Action::NextField, "Next field (Tab)", true), back()]);
        }
        FbStage::Done => {
            p.text(if s.kind == FbKind::Issue { "Your report was sent:" } else { "Your pull request is open:" }, BOLD);
            p.line(Line::styled(s.url.clone().unwrap_or_default(), Style::default().fg(BRAND)));
            p.blank();
            p.buttons(&[btn(Action::Copy, "Copy address (c)", true), back()]);
        }
    }
}

fn ensure(p: &mut Page, s: &EnsureState) {
    p.text("Make the server load a resource: Kora adds `ensure <name>` to its server.cfg (once) and can restart the server so it starts right away.", DIM);
    p.blank();
    let w = p.w();
    p.line(field("Resource name:", &s.input, w, true));
    p.blank();
    let shown = s.shown();
    if shown.is_empty() {
        p.text(if s.choices.is_empty() { "No resources are recorded for this server: type the name." } else { "No known resource matches: Enter uses what you typed." }, DIM);
    } else {
        p.text("Known resources:", DIM);
        let room = p.room().saturating_sub(3).max(1);
        let start = s.pick.unwrap_or(0).saturating_sub(room - 1);
        for (i, c) in shown.iter().enumerate().skip(start).take(room) {
            let on = s.pick == Some(i);
            p.row(Hit::Row(i), Line::styled(format!("{} {c}", if on { "▶" } else { " " }), if on { Style::default().fg(BRAND) } else { Style::default() }));
        }
    }
    p.blank();
    p.buttons(&[btn(Action::Confirm, "Ensure (Enter)", true), btn(Action::ToggleRestart, &format!("[{}] Restart the server now (Tab)", if s.restart { "x" } else { " " }), true), back()]);
}

fn delete(p: &mut Page, s: &DeleteState) {
    let yn = |p: &mut Page, yes: &str, no: &str| {
        p.buttons(&[btn(Action::Yes, yes, true), btn(Action::No, no, true), back()]);
    };
    match s.stage {
        DelStage::Files => {
            p.text(&format!("Delete '{}'. You are asked about each part.", s.entry.name), BOLD);
            p.blank();
            p.text(&format!("1/3  Delete the server folder and everything in it?\n{}", s.entry.dir.display()), Style::default());
            p.blank();
            yn(p, "Yes, delete the folder (y)", "No, keep the files (n)");
        }
        DelStage::Db => {
            p.text(if s.keep_files { "Files: kept." } else { "Files: will be deleted." }, DIM);
            p.blank();
            p.text("2/3  Delete the server's database and its database user?", Style::default());
            p.blank();
            yn(p, "Yes, delete the database (y)", "No, keep the database (n)");
        }
        DelStage::Name => {
            p.text(if s.keep_files { "Files: kept." } else { "Files: will be deleted." }, DIM);
            p.text(if s.keep_db { "Database: kept." } else { "Database: will be deleted." }, DIM);
            p.text("GitHub forks are never deleted.", DIM);
            p.blank();
            p.text(&format!("3/3  Type the server name to confirm: {}", s.entry.name), Style::default());
            let w = p.w();
            p.line(field(">", &s.input, w, true));
            p.blank();
            p.buttons(&[btn(Action::Confirm, "Delete now (Enter)", true), back()]);
        }
        DelStage::Unlist => {
            p.text(&format!("Remove '{}' from the manager's list?", s.entry.name), BOLD);
            p.blank();
            p.text("Kora did not create this server, so nothing is deleted: the folder, its files and its database stay exactly as they are.", Style::default());
            p.blank();
            yn(p, "Yes, remove it (y)", "No (n)");
        }
    }
}

fn state_colour(state: &str) -> Color {
    if state.starts_with("updated") {
        Color::Green
    } else if state.starts_with("rolled back") || state.starts_with("skipped") {
        Color::Red
    } else if state.starts_with("waiting") {
        Color::Yellow
    } else {
        Color::Gray
    }
}

fn progress_lines(p: &mut Page, rows: &[(String, String)], snapshot: &Option<String>, health: &Option<String>, room: usize) {
    let w = p.w();
    if let Some(id) = snapshot {
        p.text(&format!("Snapshot: {id} (a way back if something goes wrong)"), DIM);
    }
    let skip = rows.len().saturating_sub(room);
    for (id, st) in rows.iter().skip(skip) {
        p.line(Line::from(vec![Span::styled(format!("  {:<24}", trunc(id, 24)), BOLD), Span::styled(trunc(st, w.saturating_sub(28)), Style::default().fg(state_colour(st)))]));
    }
    if let Some(h) = health {
        p.text(h, DIM);
    }
}

fn update(p: &mut Page, s: &UpdateState) {
    if let Some(id) = &s.confirm_rollback {
        p.text(&format!("Roll back to snapshot {id}?"), BOLD);
        p.text("Resources go back to how they were before that update. The server is not restarted for you.", DIM);
        p.blank();
        p.buttons(&[btn(Action::Yes, "Yes, roll back (y)", true), btn(Action::No, "No (n)", true)]);
        return;
    }
    if s.stage == UpStage::Snapshots {
        if s.snapshots.is_empty() {
            p.text("No snapshots yet. One is saved automatically before every update.", DIM);
        } else {
            p.text("Saved snapshots (newest first). Pick one to put resources back to how they were.", DIM);
            let room = p.room().saturating_sub(5).max(1);
            let start = s.snap_sel.saturating_sub(room - 1).min(s.snapshots.len().saturating_sub(room));
            for (i, sn) in s.snapshots.iter().enumerate().skip(start).take(room) {
                let names = if sn.resources.len() > 3 { format!("{} and {} more", sn.resources[..3].join(", "), sn.resources.len() - 3) } else { sn.resources.join(", ") };
                let line = format!("{} {}  ({} resource(s): {})", if i == s.snap_sel { "▶" } else { " " }, sn.id, sn.resources.len(), names);
                let w = p.w();
                p.row(Hit::Row(i), Line::styled(trunc(&line, w), if i == s.snap_sel { BOLD } else { Style::default() }));
            }
        }
        p.blank();
        p.buttons(&[btn(Action::Rollback, "Roll back (Enter)", !s.snapshots.is_empty()), back()]);
        return;
    }
    if s.stage == UpStage::Report {
        match (&s.report, &s.error) {
            (_, Some(e)) => p.text(&format!("The update did not finish: {e}"), Style::default().fg(Color::Red)),
            (Some(r), None) => {
                let colour = if r.has_problems() { Color::Red } else { Color::Green };
                p.text(&format!("{} updated, {} skipped, {} waiting, {} rolled back, {} need you.", r.updated.len(), r.skipped.len(), r.deferred.len(), r.rolled_back.len(), r.needs_attention.len()), Style::default().fg(colour));
                for (id, why) in &r.rolled_back {
                    p.text(&format!("Rolled back: {id}: {why}"), Style::default().fg(Color::Red));
                }
                for (id, why) in &r.needs_attention {
                    p.text(&format!("Needs you: {id}: {why}"), Style::default().fg(Color::Red));
                }
                if !r.sql_flagged.is_empty() {
                    p.text(&format!("Has SQL changes to run yourself: {}", r.sql_flagged.join(", ")), Style::default().fg(Color::Yellow));
                }
            }
            _ => {}
        }
        p.blank();
        let room = p.room().saturating_sub(4);
        progress_lines(p, &s.progress, &s.snapshot, &s.health, room);
        p.blank();
        p.buttons(&[btn(Action::Rollback, "Roll back (b)", s.snapshot.is_some()), btn(Action::Snapshots, "Snapshots (s)", true), back()]);
        return;
    }
    if let Some(e) = &s.error {
        p.text(&format!("Couldn't check for updates: {e}"), Style::default().fg(Color::Red));
        p.blank();
        p.buttons(&[btn(Action::Retry, "Try again (r)", true), back()]);
        return;
    }
    if let Some(v) = &s.new_release {
        p.text(&format!("A new Kora release is available: {v}"), Style::default().fg(Color::Yellow));
    }
    if s.items.is_empty() {
        p.text("Everything is up to date.", Style::default().fg(Color::Green));
        p.blank();
        p.buttons(&[btn(Action::Snapshots, "Snapshots (s)", true), back()]);
        return;
    }
    let n = s.checked_ids().len();
    let total = s.items.iter().filter(|i| i.selectable()).count();
    p.text(&format!("{n} of {total} updatable resource(s) ticked. Resources with local changes are never touched here."), DIM);
    let w = p.w();
    p.line(Line::styled(format!("  {:<3} {:<22} {:<23} {:<14} {}", "", "Resource", "From → to", "Status", "Restart"), DIM));
    let room = p.room().saturating_sub(10).max(1);
    let start = s.sel.saturating_sub(room - 1).min(s.items.len().saturating_sub(room));
    for (i, it) in s.items.iter().enumerate().skip(start).take(room) {
        let c = &it.change;
        let mark = if !it.selectable() { "[-]" } else if it.checked { "[x]" } else { "[ ]" };
        let (tag, col) = match c.status {
            ChangeStatus::Clean => (format!("{} behind", c.behind), Color::Yellow),
            ChangeStatus::Dirty => ("Dirty".to_string(), Color::Red),
            ChangeStatus::Diverged => ("Diverged".to_string(), Color::Red),
            ChangeStatus::UpToDate => ("Up to date".to_string(), Color::Green),
        };
        let spans = vec![
            Span::raw(format!("{} {mark} ", if i == s.sel { "▶" } else { " " })),
            Span::styled(format!("{:<22} ", trunc(&c.id, 22)), BOLD),
            Span::raw(format!("{:<23}", format!("{} → {}", trunc(&c.from, 10), trunc(&c.to, 10)))),
            Span::styled(format!("{tag:<14}"), Style::default().fg(col)),
            Span::styled(if c.needs_restart { "needs restart" } else { "" }, DIM),
        ];
        p.row(Hit::Row(i), Line::from(spans));
    }
    if let Some(it) = s.items.get(s.sel) {
        p.blank();
        p.text(&format!("Changes in {} (newest first):", it.change.id), BOLD);
        if it.change.changelog.is_empty() {
            p.text("  (no changes listed)", DIM);
        }
        for l in it.change.changelog.iter().take(4) {
            p.line(Line::styled(format!("  {}", trunc(l, w.saturating_sub(2))), DIM));
        }
    }
    p.blank();
    p.buttons(&[
        btn(Action::ApplySelected, "Update ticked (Enter)", n > 0),
        btn(Action::ToggleAll, if n == total { "Untick all (a)" } else { "Update all: tick all (a)" }, true),
        btn(Action::CycleRestart, &format!("Restart: {} (r)", restart_text(s.restart)), true),
        btn(Action::Snapshots, "Snapshots (s)", true),
        back(),
    ]);
}

/// Standalone page drawn while an update runs: no `App` can exist then (the backend is busy).
pub fn render_live(f: &mut Frame, l: &Live) {
    let area = f.area();
    f.render_widget(Clear, area);
    f.render_widget(Block::default().borders(Borders::ALL).border_style(Style::default().fg(BRAND)).title(" Kora manager "), area);
    let body = Rect::new(area.x + 1, area.y + 1, area.width.saturating_sub(2), area.height.saturating_sub(2));
    let mut p = Page::new(body);
    p.text("Updating... please wait; do not close this window.", Style::default().fg(Color::Yellow));
    p.blank();
    let room = p.room().saturating_sub(2);
    progress_lines(&mut p, &l.rows, &l.snapshot, &l.health, room);
    f.render_widget(Paragraph::new(p.lines).block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Yellow)).title(format!(" Update - {} ", l.server))), body);
}

fn notify(p: &mut Page, s: &NotifyState) {
    p.text("Get a Discord message when an automatic update runs (updated, failed, or needs you).", DIM);
    p.blank();
    match &s.current.discord_webhook {
        Some(u) => p.text(&format!("Current: {} (the secret part is never shown)", mask_webhook(u)), Style::default().fg(Color::Green)),
        None => p.text("Current: not set", DIM),
    }
    p.blank();
    // The address is a secret: it is never drawn, only what it was recognised as.
    let typed = s.input.text().trim().to_string();
    let w = p.w();
    if typed.is_empty() {
        p.text("New webhook: (paste it here, or use the Paste button)", Style::default());
    } else {
        match check_webhook(&typed) {
            Ok(()) => p.text(&format!("New webhook: {} - looks right, press Enter to save", mask_webhook(&typed)), Style::default().fg(Color::Green)),
            Err(e) => {
                p.text(&format!("New webhook: {} characters entered", typed.chars().count()), Style::default());
                for l in wrap(&e, w) {
                    p.line(Line::styled(l, Style::default().fg(Color::Red)));
                }
            }
        }
    }
    p.text("In Discord: channel settings, Integrations, Webhooks, Copy URL.", DIM);
    p.blank();
    let has = s.current.discord_webhook.is_some();
    p.buttons(&[
        btn(Action::Paste, "Paste (F4)", true),
        btn(Action::Save, "Save (Enter)", !typed.is_empty()),
        btn(Action::SendTest, "Send test (F2)", has),
        btn(Action::Remove, "Remove (F3)", has),
        back(),
    ]);
}

fn schedule(p: &mut Page, s: &ScheduleState) {
    p.text("Run updates automatically at a set time. Only safe, clean updates are applied; anything else is left for you.", DIM);
    p.blank();
    match &s.existing {
        Some(c) => p.text(&format!("Now: {}", schedule::describe(c)), Style::default().fg(Color::Green)),
        None => p.text("Now: off. This server is not updated automatically.", DIM),
    }
    p.blank();
    let mark = |f: usize| if s.focus == f { "▶ " } else { "  " };
    let preset = match s.preset {
        Preset::Daily => "Every day",
        Preset::Weekly => "Every week",
        Preset::Custom => "Custom day",
    };
    p.row(Hit::Row(0), Line::raw(format!("{}Repeat:   ◀ {preset} ▶", mark(0))));
    let w = p.w();
    let mut v = vec![Span::raw(mark(1)), Span::styled("Time (24h HH:MM): ", BOLD)];
    v.extend(s.time.spans(w.saturating_sub(24), s.focus == 1));
    p.row(Hit::Row(1), Line::from(v));
    if s.preset != Preset::Daily {
        p.row(Hit::Row(2), Line::raw(format!("{}Day:      ◀ {} ▶", mark(2), weekday_name(s.weekday))));
    }
    let who = if s.run_as == RunAs::System { "System (the machine account)" } else { "As you (recommended)" };
    p.row(Hit::Row(3), Line::raw(format!("{}Run as:   ◀ {who} ▶", mark(3))));
    for l in wrap(&s.run_as_help(), w.saturating_sub(2)) {
        p.line(Line::styled(format!("  {l}"), DIM));
    }
    p.blank();
    p.buttons(&[btn(Action::Save, "Save (Enter)", true), btn(Action::Remove, "Remove schedule (F3)", s.existing.is_some()), back()]);
}

fn add(p: &mut Page, s: &AddState) {
    match s.stage {
        AddStage::Menu => {
            p.text("What do you want to add?", BOLD);
            p.blank();
            p.buttons(&[btn(Action::InstallNew, "Install a new server (i)", true)]);
            p.text("Starts the install wizard.", DIM);
            p.blank();
            p.buttons(&[btn(Action::AddExisting, "Add an existing server (e)", true), btn(Action::TypePath, "Type a folder (p)", true)]);
            p.text("Finds servers already on this computer. Adding one only puts it on this list: nothing in its folder is changed.", DIM);
            p.blank();
            p.buttons(&[back()]);
        }
        AddStage::Found => {
            if s.found.is_empty() {
                p.text("No server folders found in the usual places.", BOLD);
                p.text("Type the folder yourself instead.", DIM);
            } else {
                p.text(&format!("Found {} folder(s). Pick one:", s.found.len()), BOLD);
                let room = p.room().saturating_sub(4).max(1);
                let start = s.sel.saturating_sub(room - 1);
                let w = p.w();
                for (i, c) in s.found.iter().enumerate().skip(start).take(room) {
                    let on = i == s.sel;
                    let l = format!("{} {:<14} {:<17} {}", if on { "▶" } else { " " }, trunc(&c.name, 14), c.kind.label(), trunc(&c.dir.display().to_string(), w.saturating_sub(36)));
                    p.row(Hit::Row(i), Line::styled(l, if on { Style::default().fg(BRAND) } else { Style::default() }));
                }
            }
            p.blank();
            p.buttons(&[btn(Action::Confirm, "Add selected (Enter)", !s.found.is_empty()), btn(Action::TypePath, "Type a folder (p)", true), back()]);
        }
        AddStage::Path => {
            p.text("Type or paste the full path of the server folder (the one with server.cfg or kora-server.json).", DIM);
            p.blank();
            let w = p.w();
            p.line(field("Folder:", &s.input, w, true));
            p.blank();
            p.buttons(&[btn(Action::Confirm, "Add (Enter)", true), back()]);
        }
    }
}

use super::logs;

pub fn render(f: &mut Frame, app: &App) {
    let area = f.area();
    let b = build(app, area);
    f.render_widget(Clear, area);
    f.render_widget(Block::default().borders(Borders::ALL).border_style(Style::default().fg(BRAND)).title(" Kora manager "), b.outer);
    for (r, title, colour, lines) in b.panels {
        f.render_widget(Paragraph::new(lines).block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(colour)).title(title)), r);
    }
    if area.height >= 4 {
        let fr = Rect::new(area.x + 1, area.y + area.height - 3, area.width.saturating_sub(2), 2);
        f.render_widget(Paragraph::new(b.footer), fr);
    }
}
