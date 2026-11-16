//! Pure rendering + layout for the wizard. `layout` is shared by rendering and mouse hit-testing.

use kora_core::installer::{DbChoice, Mode};
use ratatui::{
    layout::{Alignment, Constraint, Layout as RLayout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, LineGauge, Paragraph, Tabs},
    Frame,
};

use crate::theme::{self, theme, Theme, Tone};
use crate::wizard::{Screen, Wizard};

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
                v.lines.push(String::new());
                v.lines.push(n);
            }
            v.lines.push(String::new());
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
                String::new(),
                l("  - Download the game server software and Kora's files from the internet."),
                l("  - Create a folder for your server."),
                l("  - Install Git (a download tool) and a database if they are missing."),
                l("  - Create a database and user used only by this server."),
                l("  - Allow the server through your firewall and start it automatically (you will be asked)."),
                String::new(),
                l("Some of these need administrator rights. Windows will show a permission prompt; on Linux you may be asked for your sudo password."),
                String::new(),
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
                v.lines.push(String::new());
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
                v.lines.push(String::new());
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
            v.lines.extend(test_lines(w, "Press Ctrl+T or F2 (or click Test login) to check it. Kora also tests it when you press Next, and will not continue if it fails."));
            v.inputs = vec![
                inp("Admin user", &w.dba_user, "root", w.focus == 0),
                InputView { masked: true, ..inp("Admin password (hidden)", &w.dba_pass, "leave empty if none", w.focus == 1) },
                InputView { masked: true, ..inp("Confirm admin password (hidden)", &w.dba_pass2, "type it again", w.focus == 2) },
                inp("Database server address", &w.dba_host, "127.0.0.1", w.focus == 3),
                inp("Database port", &w.dba_port, "3306", w.focus == 4),
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
        Screen::FolderAccess => {
            v.title = l("The background service can't reach this folder");
            v.lines = vec![
                format!(
                    "The server runs as its own account, which can't get into {} ({}), so it couldn't reach {}.",
                    w.access_dirs_text(),
                    if w.access_dirs.len() == 1 { "it is private to its owner" } else { "they are private to their owners" },
                    w.dir.trim()
                ),
                String::new(),
                l("Choose how to fix it:"),
            ];
            v.options = vec![
                opt(
                    "Let it pass through these folders",
                    &format!(
                        "Kora runs: chmod o+x {}. Other accounts can then pass through, but can't list what is in them.",
                        w.access_dirs.iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join(" ")
                    ),
                ),
                opt(&format!("Use {} instead", w.kora_home_dir()), "A folder made for Kora servers. Kora creates a normal 'kora' user account that owns it."),
            ];
            v.help = l("'Pass through' lets other accounts open something inside only when they know its exact name and it is readable by everyone. If you keep private files in your home folder, choose the second option.");
        }
        Screen::Shortcut => {
            v.title = l("Put a Kora shortcut on my desktop");
            v.lines = vec![l("Kora is also added to the Start Menu so you can open it later to manage your servers.")];
            v.options = vec![
                opt("Yes, put a Kora shortcut on my desktop", "Recommended."),
                opt("No, the Start Menu is enough", "You can add it later with `kora app install`."),
            ];
        }
        Screen::License => {
            v.title = l("Your server license key");
            v.lines = vec![
                l("FiveM requires a free license key for every server. Create one at portal.cfx.re (Registration keys), then paste it below. What you type is hidden."),
                String::new(),
                l("Not ready? Leave it empty and add it later."),
            ];
            v.inputs = vec![InputView { masked: true, ..inp("License key", &w.license, "paste here", true) }];
            v.help = l("Go to portal.cfx.re, sign in, open 'Registration keys' and choose 'New registration key'. The key is stored only in your server's private settings file.");
        }
        Screen::Cfx => {
            v.title = l("Your Cfx.re account");
            v.lines = vec![
                l("Type your Cfx.re username so you can sign in to txAdmin (the server control panel) with your Cfx.re account. Only the username is sent, to forum.cfx.re, to find your account number."),
                String::new(),
                l("Leave empty to use a generated local admin; you can sign in with the password saved in the server folder."),
            ];
            if w.cfx_checking() {
                v.lines.extend([String::new(), l("Checking forum.cfx.re ...")]);
            }
            v.inputs = vec![inp("Cfx.re username", &w.cfx_name, "e.g. JSixty (or leave empty)", true)];
            v.help = l("It is the name on your profile at forum.cfx.re, the same one you use to sign in to txAdmin. Kora looks it up once to get your account number; nothing else is sent.");
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
        Screen::GithubAccount if w.gh.is_some() => {
            if let Some(g) = &w.gh {
                v.title = g.title().to_string();
                v.lines = g.lines();
                v.help = g.help();
            }
        }
        Screen::GithubAccount => {}
        Screen::GameBuild => {
            v.title = l("Game build (optional)");
            v.lines = vec![l("Newer game versions are called builds. Leave empty to use the default.")];
            v.inputs = vec![inp("Game build", &w.game_build, "e.g. 3258", true)];
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
                (DbChoice::Skip, _) => "not used".to_string(),
                (_, None) => "automatic".to_string(),
                (_, Some(a)) => format!("{}@{}:{}", a.user, a.host, a.port),
            };
            let mut left = vec![
                format!("Name: {}", w.name.trim()),
                format!("Folder: {}", w.dir.trim()),
                format!("DB admin: {admin}"),
                format!(
                    "Admin: {}",
                    match w.cfx_account() {
                        Some((n, id)) if !n.is_empty() => format!("{n} ({id})"),
                        Some((_, id)) => id,
                        None => "generated local account".to_string(),
                    }
                ),
                format!("License Key: {}", if w.license.trim().is_empty() { "not set (add later)" } else { "provided (hidden)" }),
                format!("Server Software: {}", if w.artifact_custom { "chosen build" } else { "recommended" }),
            ];
            let mut right = vec![
                format!("Type: {mode}"),
                format!("Features: {}", if w.resources_all { "everything".to_string() } else { w.resource_ids.trim().to_string() }),
                format!("Database: {db}"),
                format!("Ports: game {}, control panel {}", w.game_port.trim(), w.tx_port.trim()),
            ];
            if !w.game_build.trim().is_empty() {
                right.push(format!("Game build: {}", w.game_build.trim()));
            }
            let rows = left.len().max(right.len());
            left.resize(rows, String::new());
            right.resize(rows, String::new());
            let lw = left.iter().map(|x| x.chars().count()).max().unwrap_or(0);
            v.lines = left
                .iter()
                .zip(&right)
                .map(|(a, b)| format!("{a:<lw$}  |   {b}").trim_end().to_string())
                .collect();
            v.lines.extend([
                String::new(),
                "This will change your computer as follows:".into(),
            ]);
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
    let t = theme();
    let p = Paragraph::new(label.to_string()).alignment(Alignment::Center).style(style).block(t.frame(style));
    f.render_widget(p, r);
}

/// Style one already-wrapped body line: bullets, section headings and the review columns.
fn body_line(s: &str, t: &Theme) -> Line<'static> {
    let trimmed = s.trim_start();
    let indent = &s[..s.len() - trimmed.len()];
    if !indent.is_empty() {
        if let Some(rest) = trimmed.strip_prefix("- ") {
            return Line::from(vec![Span::raw(indent.to_string()), Span::styled("- ", t.brand_style()), Span::raw(rest.to_string())]);
        }
    }
    if s.ends_with(':') && !s.contains("  ") && s.len() < 70 {
        return Line::styled(s.to_string(), t.label());
    }
    Line::raw(s.to_string())
}

fn pair(s: &str) -> Option<(&str, &str)> {
    s.split_once("  |   ").or_else(|| s.strip_suffix("  |").map(|a| (a, "")))
}

/// All body lines: the review's `left | right` rows become two aligned columns (when they fit),
/// everything else is word-wrapped and styled line by line.
fn body_lines(lines: &[String], width: usize, t: &Theme) -> Vec<Line<'static>> {
    let lw = lines.iter().filter_map(|s| pair(s)).map(|(a, _)| a.trim_end().chars().count()).max().unwrap_or(0);
    let rw = lines.iter().filter_map(|s| pair(s)).map(|(_, b)| b.chars().count()).max().unwrap_or(0);
    let fits = lw + 5 + rw <= width;
    let mut out = vec![];
    for s in lines {
        match pair(s) {
            Some((a, b)) if fits => {
                let a = a.trim_end();
                let mut spans = kv(a, t);
                spans.push(Span::raw(" ".repeat(lw.saturating_sub(a.chars().count()))));
                spans.push(Span::styled(format!("  {}  ", t.glyphs.vbar), t.border(false)));
                spans.extend(kv(b, t));
                out.push(Line::from(spans));
            }
            _ => out.extend(wrap(s, width).iter().map(|l| body_line(l, t))),
        }
    }
    out
}

/// `Key: value` -> dim key, plain value.
fn kv(s: &str, t: &Theme) -> Vec<Span<'static>> {
    match s.split_once(": ") {
        Some((k, v)) if k.len() < 24 && !k.contains(' ') || k == "DB admin" || k == "License Key" || k == "Server Software" || k == "Game build" => {
            vec![Span::styled(format!("{k}: "), t.hint()), Span::styled(v.to_string(), t.label())]
        }
        _ => vec![Span::raw(s.to_string())],
    }
}

fn header(f: &mut Frame, w: &Wizard, area: Rect, t: &Theme) {
    let g = &t.glyphs;
    let (pos, total) = w.position();
    let block = Block::new()
        .borders(Borders::BOTTOM)
        .border_set(t.border_set())
        .border_style(t.border(false))
        .title_bottom(Line::styled(format!(" step {pos} of {total} "), t.hint()).right_aligned());
    let inner = block.inner(area);
    f.render_widget(block, area);
    let [row0, row1] = RLayout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(inner);
    let [mark, tabs] = RLayout::horizontal([Constraint::Length(8), Constraint::Min(0)]).areas(row0);
    f.render_widget(Paragraph::new(Span::styled(" KORA ", t.wordmark())), mark);
    let stages = ["Welcome", "Prepare", "Choose", "Review"];
    let cur = w.stage();
    let titles: Vec<Line> = stages
        .iter()
        .enumerate()
        .map(|(i, s)| {
            if i < cur {
                Line::styled(format!("{} {s}", g.ok), t.success())
            } else if i == cur {
                Line::styled((*s).to_string(), t.selected().add_modifier(Modifier::UNDERLINED))
            } else {
                Line::styled((*s).to_string(), t.disabled())
            }
        })
        .collect();
    f.render_widget(
        Tabs::new(titles).select(cur.min(stages.len() - 1)).divider(Span::styled(format!(" {} ", g.crumb), t.disabled())).highlight_style(Style::new()).padding("", ""),
        tabs,
    );
    let ratio = if total == 0 { 0.0 } else { pos as f64 / total as f64 };
    let gw = row1.width.min(48);
    f.render_widget(
        LineGauge::default()
            .ratio(ratio.clamp(0.0, 1.0))
            .filled_symbol(g.gauge_full)
            .unfilled_symbol(g.gauge_empty)
            .filled_style(t.brand_style())
            .unfilled_style(t.disabled())
            .label(""),
        Rect::new(row1.x, row1.y, gw, row1.height),
    );
}

pub fn render(f: &mut Frame, w: &Wizard) {
    if theme::guard_size(f, theme::MIN_WIDTH, theme::MIN_HEIGHT) {
        return;
    }
    let t = theme();
    let g = t.glyphs;
    let area = f.area();
    let v = w.view();
    let l = layout(area, &v);

    header(f, w, clamp(l.header, area), &t);

    // Body.
    f.render_widget(t.panel(v.title.clone(), true), clamp(l.body, area));
    let inner = Rect::new(l.body.x + 1, l.body.y + 1, l.body.width.saturating_sub(2), l.body.height.saturating_sub(2));
    let inner = clamp(inner, area);
    let wrapped = body_lines(&v.lines, inner.width as usize, &t);
    if l.text.height > 0 {
        f.render_widget(Paragraph::new(wrapped), clamp(l.text, inner));
    }
    for (i, ((label, desc), r)) in v.options.iter().zip(&l.options).enumerate() {
        if r.height == 0 {
            continue;
        }
        let sel = i == v.selected;
        let marker = if sel { g.radio_on } else { g.radio_off };
        let st = if sel { t.selected() } else { t.text() };
        let mut lines = vec![Line::styled(format!("{marker} {label}"), st)];
        lines.extend(wrap(desc, (r.width as usize).saturating_sub(4)).into_iter().map(|d| Line::styled(format!("    {d}"), t.hint())));
        f.render_widget(Paragraph::new(lines), *r);
    }
    let link = t.button();
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
                let (tone, sym) = if n.ok { (Tone::Ok, g.ok) } else { (Tone::Warn, g.warn) };
                f.render_widget(Paragraph::new(format!("{sym} {}", n.text)).style(t.tone(tone)), nr);
            }
            if let (Some(a), true) = (&n.action, ar.width > 0 && ar.height > 0) {
                f.render_widget(Paragraph::new(a.clone()).style(t.button_warn()), ar);
            }
        }
    }
    for (iv, r) in v.inputs.iter().zip(&l.inputs) {
        if r.height == 0 {
            continue;
        }
        let shown = if iv.masked { "*".repeat(iv.value.chars().count()) } else { iv.value.clone() };
        let content = if shown.is_empty() {
            let mut spans = vec![];
            if iv.focused {
                spans.push(Span::styled(g.cursor, t.brand_style()));
            }
            spans.push(Span::styled(iv.hint.clone(), t.disabled()));
            Line::from(spans)
        } else {
            Line::from(format!("{shown}{}", if iv.focused { g.cursor } else { "" }))
        };
        let title = Line::styled(format!(" {} ", iv.label), if iv.focused { t.title() } else { t.hint() });
        f.render_widget(Paragraph::new(content).block(t.frame(t.border(iv.focused)).title(title)), *r);
    }

    // Footer: message row (error / status / key hints), then the buttons.
    if l.message.height > 0 {
        let hints: &[(&str, &str)] = if v.elevate_label.is_some() {
            &[("R", "restart with permission"), ("Esc", "back"), ("F1", "help")]
        } else if w.is_text_screen() {
            &[("Enter", "next"), ("Esc", "back"), ("Ctrl+V", "paste"), ("F1", "help")]
        } else {
            &[("Enter", "next"), ("Esc", "back"), ("F1", "help")]
        };
        let line = match (&w.error, &w.status) {
            (Some(e), _) => Line::from(vec![t.chip(Tone::Fail, "FAIL"), Span::styled(format!(" {e}"), t.danger())]),
            (None, Some(s)) => Line::from(vec![t.chip(Tone::Ok, "OK"), Span::styled(format!(" {s}"), t.success())]),
            (None, None) => t.hints(hints, l.message.width as usize),
        };
        f.render_widget(Paragraph::new(line), l.message);
    }
    button(f, l.back, "Back", t.border(false).patch(t.hint()));
    button(f, l.help, "Help", t.border(false).patch(t.hint()));
    button(f, l.test, "Test login", t.border(false).patch(t.hint()));
    if let Some(label) = &v.elevate_label {
        button(f, l.elevate, label, t.button_warn());
    }
    button(f, l.next, &v.next_label, t.button_primary());

    if let (Screen::GithubAccount, Some(gh)) = (w.screen, &w.gh) {
        crate::github_screens::render(f, gh, crate::github_screens::region(&l));
    }
    if w.help_open {
        let hw = area.width.saturating_sub(8).clamp(1, 70);
        let lines: Vec<Line<'static>> = wrap(&v.help, hw.saturating_sub(4) as usize).into_iter().map(Line::from).collect();
        theme::modal(f, area, "Help (press any key)", lines, Tone::Warn, 70);
    }
}
