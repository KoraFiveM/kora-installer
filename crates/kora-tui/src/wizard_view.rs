//! Pure rendering + layout for the wizard. `layout` is shared by rendering and mouse hit-testing.

use kora_core::installer::{DbChoice, Mode};
use ratatui::{
    layout::{Alignment, Constraint, Layout as RLayout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::wizard::{Screen, Wizard};

pub const BRAND: Color = Color::Cyan;

pub struct InputView {
    pub label: String,
    pub value: String,
    pub masked: bool,
    pub focused: bool,
    pub hint: String,
}

pub struct View {
    pub title: String,
    pub lines: Vec<String>,
    /// (label, description)
    pub options: Vec<(String, String)>,
    pub selected: usize,
    pub inputs: Vec<InputView>,
    pub help: String,
    pub next_label: String,
    /// Show the "Test login" button (database admin screens).
    pub has_test: bool,
}

fn opt(a: &str, b: &str) -> (String, String) {
    (a.to_string(), b.to_string())
}

fn inp(label: &str, value: &str, hint: &str, focused: bool) -> InputView {
    InputView { label: label.into(), value: value.into(), masked: false, focused, hint: hint.into() }
}

pub fn view(w: &Wizard) -> View {
    let mut v = View {
        title: String::new(),
        lines: vec![],
        options: vec![],
        selected: w.selected,
        inputs: vec![],
        help: String::new(),
        next_label: "Next".into(),
        has_test: false,
    };
    let l = |s: &str| s.to_string();
    match w.screen {
        Screen::Welcome => {
            v.title = l("Welcome to Kora");
            v.lines = vec![
                l("This will set up your own game server, step by step. It takes a few minutes and you can go back at any time."),
                l("How would you like to continue?"),
            ];
            v.options = vec![
                opt("Take me through it (recommended)", "A few simple questions with sensible answers already filled in."),
                opt("Advanced", "Also ask about extra options such as the server's public name, game build and setup code."),
            ];
            v.help = l("Pick 'Take me through it' if you are unsure. You can change every answer later by going back.");
        }
        Screen::Prepare => {
            v.title = l("Before we start");
            v.lines = vec![
                l("To set up your server, Kora may need to:"),
                l("  - Download the game server software and Kora's files from the internet."),
                l("  - Create a folder for your server."),
                l("  - Install Git (a download tool) and a database if they are missing."),
                l("  - Create a database and user used only by this server."),
                l("  - Allow the server through your firewall and start it automatically (you will be asked)."),
                l("Some of these need administrator rights. Windows will show a permission prompt; on Linux you may be asked for your sudo password."),
                l("Nothing is changed until you confirm on the last screen."),
            ];
            v.help = l("Kora never sends your license key anywhere except into your own server's private settings.");
        }
        Screen::Mode => {
            v.title = l("What kind of server is this?");
            v.options = vec![
                opt("Production - for real players", "Uses tested, fixed versions of everything. Pick this to run a live server."),
                opt("Development - for building and changing things", "Copies Kora's files to your GitHub account so you can edit them. Needs a GitHub account."),
            ];
            v.help = l("You can always make a separate development server later.");
        }
        Screen::Name => {
            v.title = l("Name your server");
            v.lines = vec![l("This name is shown in Kora and used to name the server's files. Letters, numbers, spaces and dashes are fine.")];
            v.inputs = vec![inp("Server name", &w.name, "", true)];
        }
        Screen::Folder => {
            v.title = l("Where should the server live?");
            v.lines = vec![l("Everything for this server will be kept in one folder. The suggestion below works for most people.")];
            v.inputs = vec![inp("Install folder", &w.dir, "", true)];
        }
        Screen::Ports => {
            v.title = l("Network ports");
            v.lines = vec![
                l("Ports are like numbered doors your server uses to talk to players and to its web control panel. The defaults are almost always right."),
                l("Use Up/Down to switch between the two. We check that no other program is already using them."),
            ];
            v.inputs = vec![
                inp("Game port (players connect here)", &w.game_port, "", w.focus == 0),
                inp("Control panel port (txAdmin)", &w.tx_port, "", w.focus == 1),
            ];
            v.help = l("Ports must be between 1024 and 65535 and different from each other. If you are behind a home router you may need to forward the game port to this computer.");
        }
        Screen::Artifact => {
            v.title = l("Server software version");
            v.options = vec![
                opt("Recommended (best choice)", "Kora picks a version known to work well."),
                opt("I want a specific version", "Paste the download address of a FiveM server build."),
            ];
        }
        Screen::ArtifactUrl => {
            v.title = l("Server software download address");
            v.lines = vec![l("Paste the https:// address of the FiveM server build (a .zip or .tar.xz).")];
            v.inputs = vec![inp("Download address", &w.artifact_url, "https://...", true)];
        }
        Screen::Resources => {
            v.title = l("Which Kora features to install?");
            v.options = vec![
                opt("Everything (recommended)", "All Kora resources and the things they depend on."),
                opt("Let me choose", "Only the essentials plus the ones you name."),
            ];
        }
        Screen::ResourceList => {
            v.title = l("Choose features");
            v.lines = vec![l("Type the names of the resources you want, separated by commas. Essentials are always included.")];
            v.inputs = vec![inp("Resources", &w.resource_ids, "kora_banking, kora_jobs", true)];
        }
        Screen::Database => {
            v.title = l("Database");
            v.lines = vec![l("Kora keeps player data in a database (MariaDB or MySQL). It will create its own private database and user, so your other data is untouched.")];
            v.options = vec![
                opt("Set it up for me (recommended)", "Use the one on this computer, or install one if there isn't one."),
                opt("Use the one I already have", "Needs a database already running on this computer."),
                opt("Skip for now", "You'll connect a database yourself later."),
            ];
        }
        Screen::DbAdmin => {
            v.title = l("Database administrator login");
            v.has_test = true;
            v.lines = vec![
                l("Kora needs an admin login for MariaDB to create a NEW database and user for your server. We never touch existing data."),
            ];
            v.options = vec![
                opt("Use automatic (root socket / no password)", "Works when you run Kora as root (or with sudo) on the computer that hosts MariaDB, or with the KORA_DB_ADMIN_* settings."),
                opt("I'll enter a user and password", "Type the admin user, its password, and where the database server is."),
            ];
            v.lines.extend(test_lines(w, "Press t (or click Test login) to try the automatic login now."));
            v.help = l("The admin login is used once to create the new database and user and is never saved to disk. If you chose 'install it for me' and no database exists yet, the test will fail until it is installed - that is fine.");
        }
        Screen::DbAdminLogin => {
            v.title = l("Database administrator login");
            v.has_test = true;
            v.lines = vec![
                l("Enter the MariaDB admin account. It is used once, only to create the new database and user, and the password is never saved."),
            ];
            v.lines.extend(test_lines(w, "Press Ctrl+T or F2 (or click Test login) to check it before you continue."));
            v.inputs = vec![
                inp("Admin user", &w.dba_user, "root", w.focus == 0),
                InputView { masked: true, ..inp("Admin password (hidden)", &w.dba_pass, "leave empty if none", w.focus == 1) },
                inp("Database server address", &w.dba_host, "127.0.0.1", w.focus == 2),
                inp("Database port", &w.dba_port, "3306", w.focus == 3),
            ];
        }
        Screen::Firewall => {
            v.title = l("Let players in?");
            v.lines = vec![l("Your computer's firewall blocks new connections by default. Kora can open only the ports it needs. This needs administrator rights.")];
            v.options = vec![
                opt("Yes, allow the server through the firewall", "Recommended for a server that others will join."),
                opt("No, I'll do it myself", "Players may not be able to connect until you do."),
            ];
        }
        Screen::TxAdminPublic => {
            v.title = l("Open the txAdmin web panel to the network?");
            v.lines = vec![l("txAdmin is the server's admin web page. By default Kora does not open its port in the firewall, so only this computer (or an SSH tunnel) can reach it.")];
            v.options = vec![
                opt("No, keep it private", "Recommended."),
                opt("Yes, open the txAdmin port too", "Anyone who can reach the port sees the login page."),
            ];
        }
        Screen::Service => {
            v.title = l("Start automatically?");
            v.lines = vec![l("Kora can start your server whenever this computer starts, and keep it running in the background. This needs administrator rights.")];
            v.options = vec![
                opt("Yes, start it automatically", "Recommended for a server that should always be online."),
                opt("No, I'll start it myself", "You can add this later."),
            ];
        }
        Screen::License => {
            v.title = l("Your server license key");
            v.lines = vec![
                l("FiveM requires a free license key for every server. Create one at portal.cfx.re (Registration keys), then paste it below. What you type is hidden."),
                l("Not ready? Leave it empty and add it later."),
            ];
            v.inputs = vec![InputView { masked: true, ..inp("License key", &w.license, "paste here", true) }];
            v.help = l("Go to portal.cfx.re, sign in, open 'Registration keys' and choose 'New registration key'. The key is stored only in your server's private settings file.");
        }
        Screen::Hostname => {
            v.title = l("Public server name");
            v.lines = vec![l("The name players see in the server list.")];
            v.inputs = vec![inp("Public name", &w.hostname, "", true)];
        }
        Screen::GithubOwner => {
            v.title = l("GitHub account");
            v.lines = vec![l("Your GitHub user or organisation where Kora's files will be copied. Leave empty to be asked later.")];
            v.inputs = vec![inp("GitHub name", &w.github_owner, "", true)];
        }
        Screen::GameBuild => {
            v.title = l("Game build (optional)");
            v.lines = vec![l("Newer game versions are called builds. Leave empty to use the default.")];
            v.inputs = vec![inp("Game build", &w.game_build, "e.g. 3258", true)];
        }
        Screen::SetupCode => {
            v.title = l("Setup code (optional)");
            // TODO: wire to the website setup-code API once it exists; for now it is only stored.
            v.lines = vec![l("If you prepared your install on the Kora website you got a code like KORA-7F3K-92QX. Enter it here, or leave empty. (Not active yet: it is just remembered for now.)")];
            v.inputs = vec![inp("Setup code", &w.setup_code, "KORA-XXXX-XXXX", true)];
        }
        Screen::Review => {
            v.title = l("Ready to install");
            v.next_label = l("Install");
            let mode = if w.mode == Mode::Production { "Production" } else { "Development" };
            let db = match w.db {
                DbChoice::InstallIfMissing => "set up for me",
                DbChoice::UseExisting => "use existing",
                DbChoice::Skip => "skip",
            };
            let admin = match (w.db.clone(), w.db_admin()) {
                (DbChoice::Skip, _) => String::new(),
                (_, None) => "   DB admin: automatic".to_string(),
                (_, Some(a)) => format!("   DB admin: {}@{}:{}", a.user, a.host, a.port),
            };
            v.lines = vec![
                format!("Name: {}   Type: {mode}", w.name.trim()),
                format!("Folder: {}", w.dir.trim()),
                format!("Ports: game {}, control panel {}   Database: {db}{admin}", w.game_port.trim(), w.tx_port.trim()),
                format!(
                    "License key: {}",
                    if w.license.trim().is_empty() { "not set (add later)" } else { "provided (hidden)" }
                ),
                String::new(),
                "This will change your computer as follows:".into(),
            ];
            v.lines.extend(w.system_changes().into_iter().map(|c| format!("  - {c}")));
            v.lines.push(String::new());
            v.lines.push("Choose Install to begin, or Back to change something.".into());
            v.help = l("Nothing has been changed yet. Quit now (q) to leave everything as it is.");
        }
    }
    if v.help.is_empty() {
        v.help = l("Use Up/Down (or 1-9) to choose, Enter for Next, Esc for Back. Type to fill in boxes. Press F1 or ? for this help. Ctrl+C quits.");
    } else {
        v.help.push_str("\n\nKeys: Up/Down choose, Enter next, Esc back, F1 or ? help, Ctrl+C quit.");
    }
    v
}

fn test_lines(w: &Wizard, prompt: &str) -> Vec<String> {
    vec![match &w.db_test {
        None => prompt.to_string(),
        Some(Ok(())) => "Login test: the login worked.".to_string(),
        Some(Err(e)) => format!("Login test failed: {e}"),
    }]
}

/// Word-wrap into lines no wider than `width` characters.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for para in text.split('\n') {
        let indent: String = para.chars().take_while(|c| *c == ' ').collect();
        let mut cur = String::new();
        for word in para.split_whitespace() {
            let wl = word.chars().count();
            let cl = cur.chars().count();
            if cl == 0 {
                cur = format!("{indent}{word}");
            } else if cl + 1 + wl <= width {
                cur.push(' ');
                cur.push_str(word);
            } else {
                out.push(std::mem::take(&mut cur));
                cur = format!("{indent}{word}");
            }
        }
        out.push(cur);
    }
    out
}

pub struct Layout {
    pub header: Rect,
    pub body: Rect,
    pub message: Rect,
    pub back: Rect,
    pub help: Rect,
    pub test: Rect,
    pub next: Rect,
    pub text: Rect,
    pub options: Vec<Rect>,
    pub inputs: Vec<Rect>,
}

fn clamp(r: Rect, area: Rect) -> Rect {
    r.intersection(area)
}

pub fn layout(area: Rect, v: &View) -> Layout {
    let [header, body, footer] =
        RLayout::vertical([Constraint::Length(3), Constraint::Min(0), Constraint::Length(4)]).areas(area);
    let message = Rect::new(footer.x, footer.y, footer.width, 1.min(footer.height));
    let by = footer.y + 1;
    let btn = |x: u16, w: u16| clamp(Rect::new(x, by, w, 3), area);
    let back = btn(footer.x, 10);
    let help = btn(footer.x + 11, 10);
    let test = if v.has_test { btn(footer.x + 22, 14) } else { Rect::default() };
    let nw = (v.next_label.len() as u16 + 6).max(10);
    let next = btn(footer.right().saturating_sub(nw), nw);

    let inner = Rect::new(body.x + 1, body.y + 1, body.width.saturating_sub(2), body.height.saturating_sub(2));
    let width = inner.width as usize;
    let mut y = inner.y;
    let mut text_lines = 0u16;
    for line in &v.lines {
        text_lines += wrap(line, width).len() as u16;
    }
    let text = clamp(Rect::new(inner.x, y, inner.width, text_lines), inner);
    y = y.saturating_add(text_lines + u16::from(text_lines > 0));
    let mut options = Vec::new();
    for (_, desc) in &v.options {
        let h = 1 + wrap(desc, width.saturating_sub(4)).len() as u16;
        options.push(clamp(Rect::new(inner.x, y, inner.width, h), inner));
        y = y.saturating_add(h + 1);
    }
    let mut inputs = Vec::new();
    for _ in &v.inputs {
        inputs.push(clamp(Rect::new(inner.x, y, inner.width, 3), inner));
        y = y.saturating_add(3);
    }
    Layout { header, body, message, back, help, test, next, text, options, inputs }
}

fn button(f: &mut Frame, r: Rect, label: &str, style: Style) {
    if r.width == 0 || r.height == 0 {
        return;
    }
    let p = Paragraph::new(label.to_string())
        .alignment(Alignment::Center)
        .style(style)
        .block(Block::default().borders(Borders::ALL).border_style(style));
    f.render_widget(p, r);
}

pub fn render(f: &mut Frame, w: &Wizard) {
    let area = f.area();
    let v = w.view();
    let l = layout(area, &v);

    // Header: brand + stages.
    let stages = ["Welcome", "Prepare", "Choose", "Review"];
    let mut spans = vec![Span::styled(" KORA ", Style::default().fg(Color::Black).bg(BRAND).add_modifier(Modifier::BOLD)), Span::raw("  ")];
    for (i, s) in stages.iter().enumerate() {
        let st = if i == w.stage() {
            Style::default().fg(BRAND).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else if i < w.stage() {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        spans.push(Span::styled(format!("{}{s}", if i < w.stage() { "✓ " } else { "" }), st));
        if i + 1 < stages.len() {
            spans.push(Span::styled("  ›  ", Style::default().fg(Color::DarkGray)));
        }
    }
    let (pos, total) = w.position();
    f.render_widget(
        Paragraph::new(Line::from(spans)).block(
            Block::default().borders(Borders::BOTTOM).title_bottom(Line::from(format!(" step {pos} of {total} ")).right_aligned()),
        ),
        clamp(l.header, area),
    );

    // Body.
    let block = Block::default().borders(Borders::ALL).border_style(Style::default().fg(BRAND)).title(format!(" {} ", v.title));
    f.render_widget(block, clamp(l.body, area));
    let inner = Rect::new(l.body.x + 1, l.body.y + 1, l.body.width.saturating_sub(2), l.body.height.saturating_sub(2));
    let inner = clamp(inner, area);
    let wrapped: Vec<Line> = v.lines.iter().flat_map(|s| wrap(s, inner.width as usize)).map(Line::from).collect();
    if l.text.height > 0 {
        f.render_widget(Paragraph::new(wrapped), clamp(l.text, inner));
    }
    for (i, ((label, desc), r)) in v.options.iter().zip(&l.options).enumerate() {
        if r.height == 0 {
            continue;
        }
        let sel = i == v.selected;
        let marker = if sel { "(•)" } else { "( )" };
        let st = if sel { Style::default().fg(BRAND).add_modifier(Modifier::BOLD) } else { Style::default() };
        let mut lines = vec![Line::styled(format!("{marker} {label}"), st)];
        lines.extend(wrap(desc, (r.width as usize).saturating_sub(4)).into_iter().map(|d| Line::styled(format!("    {d}"), Style::default().fg(Color::Gray))));
        f.render_widget(Paragraph::new(lines), *r);
    }
    for (iv, r) in v.inputs.iter().zip(&l.inputs) {
        if r.height == 0 {
            continue;
        }
        let shown = if iv.masked { "*".repeat(iv.value.chars().count()) } else { iv.value.clone() };
        let content = if shown.is_empty() {
            Line::styled(iv.hint.clone(), Style::default().fg(Color::DarkGray))
        } else {
            Line::from(format!("{shown}{}", if iv.focused { "▏" } else { "" }))
        };
        let bs = if iv.focused { Style::default().fg(BRAND) } else { Style::default().fg(Color::DarkGray) };
        f.render_widget(
            Paragraph::new(content).block(Block::default().borders(Borders::ALL).border_style(bs).title(format!(" {} ", iv.label))),
            *r,
        );
    }

    // Footer.
    if l.message.height > 0 {
        let (msg, st) = match &w.error {
            Some(e) => (e.clone(), Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            None => ("Enter = next   Esc = back   F1 / ? = help".to_string(), Style::default().fg(Color::DarkGray)),
        };
        f.render_widget(Paragraph::new(msg).style(st), l.message);
    }
    let dim = Style::default().fg(Color::Gray);
    button(f, l.back, "Back", dim);
    button(f, l.help, "Help", dim);
    button(f, l.test, "Test login", dim);
    button(f, l.next, &v.next_label, Style::default().fg(Color::Black).bg(BRAND));

    if w.help_open {
        let hw = area.width.saturating_sub(8).clamp(1, 70);
        let lines = wrap(&v.help, hw.saturating_sub(2) as usize);
        let hh = (lines.len() as u16 + 2).min(area.height);
        let r = clamp(
            Rect::new(area.x + area.width.saturating_sub(hw) / 2, area.y + area.height.saturating_sub(hh) / 2, hw, hh),
            area,
        );
        f.render_widget(Clear, r);
        f.render_widget(
            Paragraph::new(lines.into_iter().map(Line::from).collect::<Vec<_>>())
                .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Yellow)).title(" Help (press any key) ")),
            r,
        );
    }
}
