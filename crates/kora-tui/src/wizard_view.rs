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

/// Width of the `[ Paste ]` / `[ Clear ]` buttons and the narrowest body that shows them.
const BUTTON_W: u16 = 9;
const BUTTONS_MIN_WIDTH: u16 = 46;

pub struct InputView {
    pub label: String,
    pub value: String,
    pub masked: bool,
    pub focused: bool,
    pub hint: String,
    /// Status line under the box (port checks) with an optional action button.
    pub note: Option<Note>,
}

pub struct Note {
    pub text: String,
    pub ok: bool,
    /// Button label such as `[ Use 30121 ]`.
    pub action: Option<String>,
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
    /// Review only: label of the `[ Restart with permission ]` button when something needs administrator rights.
    pub elevate_label: Option<String>,
}

fn opt(a: &str, b: &str) -> (String, String) {
    (a.to_string(), b.to_string())
}

fn inp(label: &str, value: &str, hint: &str, focused: bool) -> InputView {
    InputView { label: label.into(), value: value.into(), masked: false, focused, hint: hint.into(), note: None }
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
        elevate_label: None,
    };
    let l = |s: &str| s.to_string();
    match w.screen {
        Screen::Permission if w.windows => {
            v.title = l("Kora needs to run as administrator");
            v.lines = vec![
                l("To set up your server Kora needs your permission to:"),
                l("  - install the database server (a Windows service)"),
                l("  - run your server in the background"),
                l("  - open the firewall ports"),
                l("Windows will ask you to approve this once."),
            ];
            v.lines.extend(w.elev_note.clone());
            v.options = vec![
                opt("[ Restart as administrator ]  (Enter)", "Opens Kora again in a new window. Windows asks you to approve it once."),
                opt("[ Quit ]", "Leave without changing anything. Kora can't set up a server without this permission."),
            ];
            v.next_label = l("OK");
            v.help = l("Kora installs a Windows service and firewall rules, which Windows only allows when Kora runs as administrator. Nothing has been changed yet.");
        }
        Screen::Permission => {
            v.title = l("Permission needed");
            v.lines = vec![
                l("To set up your server Kora needs your permission to:"),
                l("  - install the database server (a system service)"),
                l("  - run your server in the background"),
                l("  - open the firewall ports"),
                l("Your computer will ask for your sudo password once."),
            ];
            v.lines.extend(w.elev_note.clone());
            v.options = vec![
                opt("[ Continue (your computer will ask permission) ]  (Enter)", "Restarts Kora with sudo, then you answer the questions."),
                opt("[ Not now - limited setup ]", "Kora skips the database install, the background service and the firewall. You can use a database you already have."),
            ];
            v.next_label = l("OK");
            v.help = l("Choose 'Not now' to install only the server files. You can run Kora again later with sudo to add the rest. Nothing has been changed yet.");
        }
        Screen::Welcome => {
            v.title = l("Welcome to Kora");
            v.lines = vec![l("This will set up your own game server, step by step. It takes a few minutes and you can go back at any time.")];
            if let Some(n) = w.welcome_note() {
                v.lines.push(n);
            }
            v.lines.push(l("How would you like to continue?"));
            v.options = vec![
                opt("Take me through it (recommended)", "A few simple questions with sensible answers already filled in."),
                opt("Advanced", "Also ask about extra options such as the server's public name, game build and the txAdmin panel."),
            ];
            v.help = l("Pick 'Take me through it' if you are unsure. You can change every answer later by going back.");
        }
        Screen::Prepare if w.windows => {
            v.title = l("Before we start (Windows)");
            v.lines = vec![
                l("Kora will download the game server and Kora's files, create your server folder and set up a database. Nothing is changed until you confirm on the last screen."),
                if w.admin {
                    l("Running as Administrator. Windows may ask you to confirm a few times (a \"User Account Control\" prompt): choose Yes.")
                } else {
                    l("Kora is not running as Administrator, so it will skip the database service, the Windows service (start with Windows) and the firewall rules. To include them, start Kora again with Run as administrator.")
                },
                l("Git and the GitHub CLI are installed with winget (Windows' app installer) if they are missing, with a confirmation prompt for each."),
                l("Windows Security may scan the downloads for a minute. If a file goes missing, check Windows Security > Protection history."),
                l("Needs about 10 GB free on the server's drive and about 1 GB on C:. Ports 30120 and 40120 must be free; Kora checks and suggests free ones."),
                l("If Windows says \"Windows protected your PC\", choose More info > Run anyway (kora.exe is not code-signed yet)."),
            ];
            v.help = l("Kora never sends your license key anywhere except into your own server's private settings.");
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
            if let Some(n) = &w.ports_note {
                v.lines.push(n.clone());
            }
            v.lines.push(l("Kora checks that nothing else on this computer uses them. If one is taken, press U (or click the Use button) to switch to the next free port."));
            v.inputs = vec![
                inp("Game port (players connect here)", &w.game_port, "", w.focus == 0),
                inp("Control panel port (txAdmin)", &w.tx_port, "", w.focus == 1),
            ];
            for (iv, st) in v.inputs.iter_mut().zip(&w.ports_status) {
                iv.note = st.as_ref().map(|s| Note { text: s.text.clone(), ok: s.ok, action: s.suggestion.map(|p| format!("[ Use {p} ]")) });
            }
            v.help = l("Ports must be between 1024 and 65535 and different from each other. A port that is already used by another program (or by another of your Kora servers) can't be used: press U or F3, or click the Use button, to take the next free one. If you are behind a home router you may need to forward the game port to this computer.");
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
            if w.limited {
                v.lines.push(l("Limited setup: without permission Kora can't install a database server, so use one you already have on this computer, or skip this and connect one later. Choosing 'Set it up for me' needs permission and will ask you to restart."));
            }
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
            match w.welcome_note() {
                Some(n) => {
                    v.lines = vec![
                        n,
                        l("Questions the website already answered are skipped; you will see all of them again on the last screen before anything is changed."),
                        l("Your license key and database login are never sent by the website, so you will still be asked for those."),
                    ];
                }
                None => {
                    v.lines = vec![
                        l("If you prepared your install on the Kora website you got a code like KORA-7F3K-92QX. Paste it here and press Enter to load your choices, or leave the box empty and press Next to answer everything here."),
                        l("A code works once and lasts 30 minutes."),
                    ];
                    v.inputs = vec![inp("Setup code", &w.setup_code, "KORA-XXXX-XXXX", true)];
                }
            }
            v.help = l("Create a code on the Kora website (Installer page) after linking Discord, GitHub and your Cfx.re account. If it does not work you can simply continue without it.");
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
                    "Features: {}   Server software: {}{}",
                    if w.resources_all { "everything".to_string() } else { w.resource_ids.trim().to_string() },
                    if w.artifact_custom { "chosen build" } else { "recommended" },
                    if w.game_build.trim().is_empty() { String::new() } else { format!("   Game build: {}", w.game_build.trim()) },
                ),
                format!(
                    "License key: {}",
                    if w.license.trim().is_empty() { "not set (add later)" } else { "provided (hidden)" }
                ),
                String::new(),
                "This will change your computer as follows:".into(),
            ];
            let reasons = w.review_reasons();
            if !reasons.is_empty() {
                let mut top = vec![l("Kora needs your permission to:")];
                top.extend(reasons.iter().map(|r| format!("  - {r}")));
                top.push(l("Press R (or click Restart with permission). Kora reopens and you answer the questions again. Or go Back and change those answers."));
                top.extend(w.elev_note.clone());
                top.push(String::new());
                v.lines.splice(0..0, top);
                v.elevate_label = Some(l("[ Restart with permission ]"));
            } else if w.limited {
                v.lines.insert(0, l("Limited setup: no database server, background service or firewall rules will be added. You can run Kora again later with sudo to add them."));
            }
            if w.redeemed.is_some() {
                v.lines.insert(0, "Loaded from your setup code on the website: type, name, features, server software, ports and public name.".into());
            }
            v.lines.extend(w.system_changes().into_iter().map(|c| format!("  - {c}")));
            v.lines.push(String::new());
            v.lines.push("Choose Install to begin, or Back to change something.".into());
            v.help = l("Nothing has been changed yet. Quit now (q) to leave everything as it is.");
        }
    }
    if v.help.is_empty() {
        v.help = l("Use Up/Down (or 1-9) to choose, Enter for Next, Esc for Back. Type to fill in boxes; Ctrl+V or Shift+Insert (or the Paste button) pastes, Ctrl+U (or Clear) empties a box. Press F1 or ? for this help. Ctrl+C quits.");
    } else {
        v.help.push_str("\n\nKeys: Up/Down choose, Enter next, Esc back, F1 or ? help, Ctrl+V / Shift+Insert paste, Ctrl+U clear, U or F3 use the suggested port (Ports screen), Ctrl+C quit.");
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
    /// `[ Restart with permission ]` on Review (`Rect::default()` when absent).
    pub elevate: Rect,
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
    /// Status line under each input that has a note, and its action button (`Rect::default()` when absent).
    pub notes: Vec<Rect>,
    pub actions: Vec<Rect>,
    /// `[ Paste ]` / `[ Clear ]` buttons, one per input (empty rect when the window is too narrow).
    pub paste: Vec<Rect>,
    pub clear: Vec<Rect>,
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
    let elevate = match &v.elevate_label {
        Some(t) => btn(footer.x + 11 + 11, t.chars().count() as u16 + 2),
        None => Rect::default(),
    };
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
    let (mut paste, mut clear) = (Vec::new(), Vec::new());
    let (mut notes, mut actions) = (Vec::new(), Vec::new());
    let buttons = inner.width >= BUTTONS_MIN_WIDTH;
    let bw = if buttons { inner.width - 2 * BUTTON_W - 2 } else { inner.width };
    for iv in &v.inputs {
        inputs.push(clamp(Rect::new(inner.x, y, bw, 3), inner));
        let mut extra = 0u16;
        match &iv.note {
            Some(n) => {
                let label_w = n.action.as_ref().map_or(0, |a| a.chars().count() as u16);
                let ax = if buttons { inner.x + bw + 1 } else { inner.right().saturating_sub(label_w) };
                let nw = if buttons { bw } else { inner.width.saturating_sub(label_w + 1) };
                notes.push(clamp(Rect::new(inner.x, y + 3, nw, 1), inner));
                actions.push(if label_w > 0 { clamp(Rect::new(ax, y + 3, label_w, 1), inner) } else { Rect::default() });
                extra = 1;
            }
            None => {
                notes.push(Rect::default());
                actions.push(Rect::default());
            }
        }
        if buttons {
            paste.push(clamp(Rect::new(inner.x + bw + 1, y + 1, BUTTON_W, 1), inner));
            clear.push(clamp(Rect::new(inner.x + bw + 2 + BUTTON_W, y + 1, BUTTON_W, 1), inner));
        } else {
            paste.push(Rect::default());
            clear.push(Rect::default());
        }
        y = y.saturating_add(3 + extra);
    }
    Layout { elevate, header, body, message, back, help, test, next, text, options, inputs, notes, actions, paste, clear }
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
    let link = Style::default().fg(Color::Black).bg(Color::Gray);
    for (i, p) in l.paste.iter().enumerate() {
        if p.width > 0 {
            f.render_widget(Paragraph::new("[ Paste ]").style(link), *p);
            f.render_widget(Paragraph::new("[ Clear ]").style(link), l.clear[i]);
        }
    }
    for (i, iv) in v.inputs.iter().enumerate() {
        if let Some(n) = &iv.note {
            let (nr, ar) = (l.notes[i], l.actions[i]);
            if nr.width > 0 && nr.height > 0 {
                let st = if n.ok { Style::default().fg(Color::Green) } else { Style::default().fg(Color::Yellow) };
                f.render_widget(Paragraph::new(n.text.clone()).style(st), nr);
            }
            if let (Some(a), true) = (&n.action, ar.width > 0 && ar.height > 0) {
                f.render_widget(Paragraph::new(a.clone()).style(Style::default().fg(Color::Black).bg(Color::Yellow)), ar);
            }
        }
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
        let hint = if w.is_text_screen() {
            "Enter = next   Esc = back   Ctrl+V = paste   F1 = help"
        } else {
            "Enter = next   Esc = back   F1 / ? = help"
        };
        let hint = if v.elevate_label.is_some() { "R = restart with permission   Esc = back   F1 / ? = help" } else { hint };
        let (msg, st) = match (&w.error, &w.status) {
            (Some(e), _) => (e.clone(), Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            (None, Some(s)) => (s.clone(), Style::default().fg(Color::Green)),
            (None, None) => (hint.to_string(), Style::default().fg(Color::DarkGray)),
        };
        f.render_widget(Paragraph::new(msg).style(st), l.message);
    }
    let dim = Style::default().fg(Color::Gray);
    button(f, l.back, "Back", dim);
    button(f, l.help, "Help", dim);
    button(f, l.test, "Test login", dim);
    if let Some(t) = &v.elevate_label {
        button(f, l.elevate, t, Style::default().fg(Color::Black).bg(Color::Yellow));
    }
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
