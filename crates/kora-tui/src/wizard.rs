//! Pure state machine for the install wizard. No terminal I/O lives here, so every
//! transition is unit-testable.
//!
//! Stages: Welcome -> Prepare -> Choose (one decision per screen) -> Review.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use kora_core::firewall::ports::{suggest_free_port, PortProber, SystemProber};
use kora_core::firewall::PortRequest;
use kora_core::installer::{DbAdmin, DbChoice, InstallRequest, Mode, Ports, ResourceSelection, ServerEntry};
use ratatui::layout::Rect;
use std::path::{Path, PathBuf};

use crate::clipboard::{sanitize_paste, Clipboard, SystemClipboard};
use crate::elev::{self, Elevator};
use crate::wizard_view;
use std::ffi::OsString;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;

/// Looks a Cfx.re username up (the real forum in the app, a fake in tests).
pub type CfxLookup = Arc<dyn Fn(&str) -> Result<kora_core::cfx::CfxAccount, String> + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// First screen when Kora is not running with administrator rights.
    Permission,
    Welcome,
    Prepare,
    Mode,
    Name,
    Folder,
    Ports,
    Artifact,
    ArtifactUrl,
    Resources,
    ResourceList,
    Database,
    DbAdmin,
    DbAdminLogin,
    Firewall,
    Service,
    /// Windows: put a Kora shortcut on the desktop (the Start Menu entry is always added).
    Shortcut,
    License,
    /// Cfx.re username, so the person installing becomes the txAdmin admin (optional).
    Cfx,
    // Advanced only
    Hostname,
    GithubOwner,
    /// Development mode: sign in to GitHub and choose the owner of the forks.
    GithubAccount,
    GameBuild,
    TxAdminPublic,
    Review,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardState {
    Running,
    Done,
    Quit,
    /// Kora restarted itself with permission; this copy must exit.
    Elevated,
}

/// How the wizard checks ports: a prober (the game port needs TCP and UDP free, txAdmin TCP only) and the other Kora servers in the registry (their ports stay reserved even when
/// they are stopped). Injected so tests can use fakes.
pub struct PortEnv {
    pub prober: Box<dyn PortProber>,
    pub servers: Vec<ServerEntry>,
}

impl PortEnv {
    pub fn system() -> Self {
        Self { prober: Box::new(SystemProber), servers: kora_core::installer::Registry::load().map(|r| r.servers).unwrap_or_default() }
    }
}

/// Live status of one port box, shown under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortLine {
    pub port: u16,
    pub ok: bool,
    /// "30120 is free" / "30120 is already in use - 30121 is free".
    pub text: String,
    /// The next free port, offered as a `[ Use N ]` button.
    pub suggestion: Option<u16>,
}

/// Real database login check (fake-able in tests). `None` = automatic (environment / root socket).
pub type DbTester = fn(Option<&DbAdmin>) -> Result<(), String>;

pub struct Wizard {
    pub screen: Screen,
    pub(crate) state: WizardState,
    pub advanced: bool,
    pub mode: Mode,
    pub name: String,
    pub dir: String,
    dir_touched: bool,
    pub game_port: String,
    pub tx_port: String,
    pub artifact_custom: bool,
    pub artifact_url: String,
    pub resources_all: bool,
    pub resource_ids: String,
    pub db: DbChoice,
    pub firewall: bool,
    /// Advanced only: also open the txAdmin port in the firewall.
    pub txadmin_public: bool,
    pub service: bool,
    /// Windows: also put a Kora shortcut on the desktop (default on).
    pub desktop_shortcut: bool,
    pub license: String,
    pub hostname: String,
    host_touched: bool,
    pub github_owner: String,
    pub(crate) gh: Option<crate::github_screens::GithubFlow>,
    pub game_build: String,
    cfx_identifier: Option<String>,
    /// The Cfx.re username as typed on the Cfx screen (kept when going back).
    pub cfx_name: String,
    /// The name Cfx.re spells for the resolved account (shown on Review).
    cfx_username: Option<String>,
    cfx_lookup: CfxLookup,
    cfx_job: Option<Receiver<Result<kora_core::cfx::CfxAccount, String>>>,
    /// Selected row for choice screens.
    pub selected: usize,
    /// Focused input for text screens.
    pub focus: usize,
    pub help_open: bool,
    /// Platform wording: Windows gets UAC / SmartScreen / winget / firewall hints.
    pub windows: bool,
    /// Running as Administrator (Windows) or root.
    pub admin: bool,
    pub error: Option<String>,
    /// Show the permission screen first, and the restart button on Review.
    elevate_offer: bool,
    elev_args: Vec<OsString>,
    elevator: Elevator,
    /// The user chose the limited (no administrator) setup.
    pub limited: bool,
    /// Result of the last restart attempt, shown in the body.
    pub elev_note: Option<String>,
    reasons_cache: std::cell::RefCell<Option<Vec<String>>>,
    /// Friendly confirmation shown in the footer (e.g. "Pasted 14 characters"); never holds field content.
    pub status: Option<String>,
    clipboard: Box<dyn Clipboard>,
    ports: PortEnv,
    /// Live status of the game / txAdmin port boxes (refreshed whenever they change).
    pub ports_status: [Option<PortLine>; 2],
    ports_touched: bool,
    /// Why the ports were changed (the usual ones are taken).
    pub ports_note: Option<String>,
    /// After a re-check sent the user back to Ports, return straight to Review.
    jump_review: bool,
    db_tester: DbTester,
    /// false = automatic (root socket / no password), true = enter an admin login.
    pub dba_custom: bool,
    pub dba_user: String,
    pub dba_pass: String,
    /// Second entry of the admin password (must match).
    pub dba_pass2: String,
    pub dba_host: String,
    pub dba_port: String,
    /// Answers came back from an install that failed earlier (shown on Welcome).
    pub restored: bool,
    /// Result of the last "Test login" (cleared when the login changes).
    pub db_test: Option<Result<(), String>>,
}

impl std::fmt::Debug for Wizard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Wizard")
            .field("screen", &self.screen)
            .field("state", &self.state)
            .field("mode", &self.mode)
            .field("name", &self.name)
            .field("dir", &self.dir)
            .field("license", &if self.license.is_empty() { "<empty>" } else { "<redacted>" })
            .field("dba_pass", &if self.dba_pass.is_empty() { "<empty>" } else { "<redacted>" })
            .finish_non_exhaustive()
    }
}

/// Lowercase, filesystem/service-safe form of a display name.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if (c == ' ' || c == '-' || c == '_') && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

/// Default install folder per OS.
pub fn default_dir(name: &str) -> String {
    let s = match slug(name) {
        s if s.is_empty() => "my-server".to_string(),
        s => s,
    };
    if cfg!(windows) {
        format!("C:\\Kora\\{s}")
    } else {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/opt".into());
        format!("{}/kora/{s}", home.trim_end_matches('/'))
    }
}

pub fn validate_name(name: &str) -> Result<(), String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("Please give your server a name.".into());
    }
    if n.chars().count() > 32 {
        return Err("That name is too long. Please use 32 characters or fewer.".into());
    }
    if !n.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '-' || c == '_') {
        return Err("Please use only letters, numbers, spaces, dashes and underscores.".into());
    }
    if slug(n).is_empty() {
        return Err("The name needs at least one letter or number.".into());
    }
    Ok(())
}

/// Parse and validate ports (range, distinct). Conflict checking is separate.
pub fn validate_ports(name: &str, game: &str, tx: &str) -> Result<(u16, u16), String> {
    let parse = |label: &str, s: &str| -> Result<u16, String> {
        s.trim()
            .parse::<u16>()
            .map_err(|_| format!("The {label} port must be a number between 1024 and 65535."))
    };
    let g = parse("game", game)?;
    let t = parse("txAdmin", tx)?;
    let slug = match slug(name) {
        s if s.is_empty() => "server".to_string(),
        s => s,
    };
    PortRequest { server: slug, game_port: g, txadmin_port: t, expose_txadmin: false }
        .validate()
        .map_err(|e| e.to_string())?;
    Ok((g, t))
}

/// Light sanity check of a license key; never echoes the key.
pub fn validate_license(key: &str) -> Result<(), String> {
    let k = key.trim();
    if k.is_empty() {
        return Ok(()); // can be added later
    }
    if k.len() < 20 || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err("That doesn't look like a license key. Copy it again from portal.cfx.re (or leave it empty for now).".into());
    }
    Ok(())
}

impl Default for Wizard {
    fn default() -> Self {
        Self::new()
    }
}

impl Wizard {
    pub fn new() -> Self {
        let elevated = elev::is_elevated();
        Self::with_ports(PortEnv::system())
            .with_platform(cfg!(windows), elevated)
            .with_elevation(!elevated && elev::can_offer(), elev::original_args(), elev::system_elevator())
    }

    /// Offer (or not) to restart with permission. `args` are repeated by the restart.
    pub fn with_elevation(mut self, offer: bool, args: Vec<OsString>, elevator: Elevator) -> Self {
        self.elevate_offer = offer;
        self.elev_args = args;
        self.elevator = elevator;
        self.screen = if offer { Screen::Permission } else { Screen::Welcome };
        self
    }

    /// Platform facts used for the Prepare / Review wording (tests set them explicitly).
    pub fn with_platform(mut self, windows: bool, admin: bool) -> Self {
        self.windows = windows;
        self.admin = admin;
        self
    }

    pub fn with_ports(env: PortEnv) -> Self {
        Self::with_hooks(env, kora_core::installer::check_db_admin)
    }

    pub fn with_hooks(env: PortEnv, db_tester: DbTester) -> Self {
        let name = "My Kora Server".to_string();
        let ports = Ports::default();
        Self {
            screen: Screen::Welcome,
            state: WizardState::Running,
            advanced: false,
            mode: Mode::Production,
            dir: default_dir(&name),
            dir_touched: false,
            hostname: name.clone(),
            host_touched: false,
            name,
            game_port: ports.game.to_string(),
            tx_port: ports.txadmin.to_string(),
            artifact_custom: false,
            artifact_url: String::new(),
            resources_all: true,
            resource_ids: String::new(),
            db: DbChoice::InstallIfMissing,
            firewall: true,
            txadmin_public: false,
            service: true,
            desktop_shortcut: true,
            license: String::new(),
            github_owner: String::new(),
            gh: None,
            game_build: String::new(),
            cfx_identifier: None,
            cfx_name: String::new(),
            cfx_username: None,
            cfx_lookup: Arc::new(kora_core::cfx::lookup),
            cfx_job: None,
            selected: 0,
            focus: 0,
            help_open: false,
            windows: false,
            admin: false,
            error: None,
            elevate_offer: false,
            elev_args: vec![],
            elevator: elev::system_elevator(),
            limited: false,
            elev_note: None,
            reasons_cache: Default::default(),
            status: None,
            clipboard: Box::new(SystemClipboard::default()),
            ports: env,
            ports_status: [None, None],
            ports_touched: false,
            ports_note: None,
            jump_review: false,
            db_tester,
            dba_custom: false,
            dba_user: "root".into(),
            dba_pass: String::new(),
            dba_pass2: String::new(),
            dba_host: "127.0.0.1".into(),
            dba_port: "3306".into(),
            db_test: None,
            restored: false,
        }
    }

    pub fn state(&self) -> WizardState {
        self.state
    }

    /// Screens in the current order (depends on earlier answers).
    pub fn flow(&self) -> Vec<Screen> {
        use Screen::*;
        let mut v = vec![];
        if self.elevate_offer {
            v.push(Permission);
        }
        v.push(Welcome);
        v.push(Prepare);
        v.extend([Mode, Name]);
        v.push(Folder);
        v.push(Ports);
        {
            v.push(Artifact);
            if self.artifact_custom {
                v.push(ArtifactUrl);
            }
            v.push(Resources);
            if !self.resources_all {
                v.push(ResourceList);
            }
        }
        v.push(Database);
        if self.db != DbChoice::Skip {
            v.push(DbAdmin);
            if self.dba_custom {
                v.push(DbAdminLogin);
            }
        }
        v.extend([Firewall, Service]);
        if self.windows {
            v.push(Shortcut);
        }
        v.push(License);
        v.push(Cfx);
        if self.advanced {
            v.push(Hostname);
        }
        if self.mode == kora_core::installer::Mode::Development && self.gh.is_some() {
            v.push(GithubAccount);
        } else if self.advanced && self.mode == kora_core::installer::Mode::Development {
            v.push(GithubOwner);
        }
        if self.advanced {
            v.push(GameBuild);
            v.push(TxAdminPublic);
        }
        v.push(Review);
        v
    }

    pub fn position(&self) -> (usize, usize) {
        let f = self.flow();
        (f.iter().position(|s| *s == self.screen).unwrap_or(0) + 1, f.len())
    }

    /// Which of the four stages (Welcome, Prepare, Choose, Review) is active.
    pub fn stage(&self) -> usize {
        match self.screen {
            Screen::Permission | Screen::Welcome => 0,
            Screen::Prepare => 1,
            Screen::Review => 3,
            _ => 2,
        }
    }

    pub fn is_text_screen(&self) -> bool {
        self.input_count() > 0
    }

    pub fn input_count(&self) -> usize {
        match self.screen {
            Screen::Ports => 2,
            Screen::DbAdminLogin => 5,
            Screen::Name
            | Screen::Folder
            | Screen::ArtifactUrl
            | Screen::ResourceList
            | Screen::License
            | Screen::Cfx
            | Screen::Hostname
            | Screen::GithubOwner
            | Screen::GameBuild => 1,
            _ => 0,
        }
    }

    pub fn option_count(&self) -> usize {
        match self.screen {
            Screen::Permission | Screen::Welcome | Screen::Mode | Screen::Artifact | Screen::Resources | Screen::DbAdmin => 2,
            Screen::Database => 3,
            Screen::Firewall | Screen::Service | Screen::Shortcut | Screen::TxAdminPublic => 2,
            _ => 0,
        }
    }

    fn field_mut(&mut self) -> Option<&mut String> {
        Some(match (self.screen, self.focus) {
            (Screen::Name, _) => &mut self.name,
            (Screen::Folder, _) => &mut self.dir,
            (Screen::Ports, 0) => &mut self.game_port,
            (Screen::Ports, _) => &mut self.tx_port,
            (Screen::DbAdminLogin, 0) => &mut self.dba_user,
            (Screen::DbAdminLogin, 1) => &mut self.dba_pass,
            (Screen::DbAdminLogin, 2) => &mut self.dba_pass2,
            (Screen::DbAdminLogin, 3) => &mut self.dba_host,
            (Screen::DbAdminLogin, _) => &mut self.dba_port,
            (Screen::ArtifactUrl, _) => &mut self.artifact_url,
            (Screen::ResourceList, _) => &mut self.resource_ids,
            (Screen::License, _) => &mut self.license,
            (Screen::Cfx, _) => &mut self.cfx_name,
            (Screen::Hostname, _) => &mut self.hostname,
            (Screen::GithubOwner, _) => &mut self.github_owner,
            (Screen::GameBuild, _) => &mut self.game_build,
            _ => return None,
        })
    }

    /// Sync `selected` from stored answers when entering a screen.
    fn load_selection(&mut self) {
        self.reasons_cache.take();
        if self.screen == Screen::GithubAccount {
            self.gh_enter();
        }
        self.focus = 0;
        if self.screen == Screen::Ports {
            self.prefill_free_ports();
        }
        self.selected = match self.screen {
            Screen::Welcome => usize::from(self.advanced),
            Screen::Mode => usize::from(self.mode == Mode::Development),
            Screen::Artifact => usize::from(self.artifact_custom),
            Screen::Resources => usize::from(!self.resources_all),
            Screen::Database => match self.db {
                DbChoice::InstallIfMissing => 0,
                DbChoice::UseExisting => 1,
                DbChoice::Skip => 2,
            },
            Screen::DbAdmin => usize::from(self.dba_custom),
            Screen::Firewall => usize::from(!self.firewall),
            Screen::Service => usize::from(!self.service),
            Screen::Shortcut => usize::from(!self.desktop_shortcut),
            Screen::TxAdminPublic => usize::from(!self.txadmin_public),
            _ => 0,
        };
    }

    /// Apply the highlighted choice to the answers.
    fn apply_selection(&mut self) {
        let i = self.selected;
        match self.screen {
            Screen::Welcome => self.advanced = i == 1,
            Screen::Mode => {
                self.mode = if i == 1 { Mode::Development } else { Mode::Production }
            }
            Screen::Artifact => self.artifact_custom = i == 1,
            Screen::Resources => self.resources_all = i == 0,
            Screen::Database => {
                self.db = [DbChoice::InstallIfMissing, DbChoice::UseExisting, DbChoice::Skip][i.min(2)].clone()
            }
            Screen::DbAdmin => {
                let custom = i == 1;
                if custom != self.dba_custom {
                    self.db_test = None;
                }
                self.dba_custom = custom;
            }
            Screen::Firewall => self.firewall = i == 0,
            Screen::Service => self.service = i == 0,
            Screen::Shortcut => self.desktop_shortcut = i == 0,
            Screen::TxAdminPublic => self.txadmin_public = i == 0,
            Screen::GithubAccount => self.gh_apply(),
            _ => {}
        }
    }

    pub fn select(&mut self, i: usize) {
        let n = self.option_count();
        if n > 0 {
            self.selected = i.min(n - 1);
            self.apply_selection();
        }
    }

    fn type_char(&mut self, c: char) {
        let screen = self.screen;
        if screen == Screen::Ports && !c.is_ascii_digit() {
            return; // port boxes take digits only (letters are shortcuts there)
        }
        if let Some(f) = self.field_mut() {
            f.push(c);
        }
        self.after_edit(screen);
    }

    fn after_edit(&mut self, screen: Screen) {
        self.error = None;
        match screen {
            Screen::Name => {
                if !self.dir_touched {
                    self.dir = default_dir(&self.name);
                }
                if !self.host_touched {
                    self.hostname = self.name.clone();
                }
            }
            Screen::Ports => {
                self.ports_touched = true;
                self.ports_note = None;
                self.refresh_ports();
            }
            Screen::Folder => self.dir_touched = true,
            Screen::Hostname => self.host_touched = true,
            Screen::Cfx => {
                // A different name is a different account: forget the earlier answer and any lookup in flight.
                self.cfx_identifier = None;
                self.cfx_username = None;
                self.cfx_job = None;
            }
            Screen::DbAdminLogin => self.db_test = None,
            _ => {}
        }
    }

    /// The admin login the user chose (`None` = automatic).
    pub fn db_admin(&self) -> Option<DbAdmin> {
        if self.db == DbChoice::Skip || !self.dba_custom {
            return None;
        }
        let pw = self.dba_pass.clone();
        Some(DbAdmin {
            host: self.dba_host.trim().to_string(),
            port: self.dba_port.trim().parse().unwrap_or(3306),
            user: self.dba_user.trim().to_string(),
            password: (!pw.is_empty()).then_some(pw),
        })
    }

    pub fn on_db_admin_screen(&self) -> bool {
        matches!(self.screen, Screen::DbAdmin | Screen::DbAdminLogin)
    }

    /// "Test login": a real connection attempt through the db module; result is shown on screen.
    pub fn test_login(&mut self) {
        self.error = None;
        if let Err(e) = self.validate_db_admin() {
            self.db_test = Some(Err(e));
            return;
        }
        self.db_test = Some((self.db_tester)(self.db_admin().as_ref()));
    }

    /// Leaving a database screen: really connect first, so a wrong login or a stopped server is found
    /// now and not halfway through the install. A failed automatic login only blocks when the user
    /// said to use an existing database; with "set it up for me" the server may simply not be installed yet.
    fn check_database_before_leaving(&mut self) -> Result<(), String> {
        let blocks = match self.screen {
            Screen::DbAdminLogin => true,
            Screen::DbAdmin => !self.dba_custom,
            _ => return Ok(()),
        };
        if !blocks {
            return Ok(());
        }
        let result = (self.db_tester)(self.db_admin().as_ref());
        self.db_test = Some(result.clone());
        match result {
            Ok(()) => Ok(()),
            // Kora installs the database itself, so there may be nothing to connect to yet.
            Err(_) if self.screen == Screen::DbAdmin && self.db == DbChoice::InstallIfMissing => Ok(()),
            Err(e) => Err(format!("The database login did not work, so the install cannot continue yet: {e} Fix the login, or go Back and choose a different database option.")),
        }
    }

    fn validate_db_admin(&self) -> Result<(), String> {
        if !self.dba_custom {
            return Ok(());
        }
        if self.dba_user.trim().is_empty() {
            return Err("Please enter the database admin user name (often 'root').".into());
        }
        if self.dba_pass != self.dba_pass2 {
            return Err("The two passwords don't match. Type the same password in both boxes.".into());
        }
        if self.dba_host.trim().is_empty() {
            return Err("Please enter the database server address (usually 127.0.0.1).".into());
        }
        match self.dba_port.trim().parse::<u16>() {
            Ok(p) if p > 0 => Ok(()),
            _ => Err("The database port must be a number such as 3306.".into()),
        }
    }

    /// Fill the answers from an earlier install that failed (secrets are never saved, so the license
    /// key and database password are asked for again).
    pub fn restore_saved(&mut self, r: &InstallRequest) {
        self.mode = r.mode;
        self.name = r.name.clone();
        self.dir = r.dir.to_string_lossy().into_owned();
        self.dir_touched = true;
        self.hostname = r.hostname.clone();
        self.host_touched = true;
        self.game_port = r.ports.game.to_string();
        self.tx_port = r.ports.txadmin.to_string();
        self.ports_touched = true;
        self.artifact_custom = r.artifact_url.is_some();
        self.artifact_url = r.artifact_url.clone().unwrap_or_default();
        self.game_build = r.game_build.map(|g| g.to_string()).unwrap_or_default();
        match &r.resources {
            ResourceSelection::All => self.resources_all = true,
            ResourceSelection::Only(ids) => {
                self.resources_all = false;
                self.resource_ids = ids.join(", ");
            }
        }
        self.db = r.db.clone();
        self.firewall = r.firewall;
        self.service = r.service;
        self.txadmin_public = r.txadmin_public;
        self.github_owner = r.github_owner.clone().unwrap_or_default();
        self.cfx_identifier = r.cfx_identifier.clone();
        if let Some(a) = &r.db_admin {
            self.dba_custom = true;
            self.dba_user = a.user.clone();
            self.dba_host = a.host.clone();
            self.dba_port = a.port.to_string();
        }
        self.restored = true;
        self.refresh_ports();
    }

    /// Welcome text when answers from an earlier attempt were restored.
    pub fn welcome_note(&self) -> Option<String> {
        if self.restored {
            return Some("Your answers from the last attempt are filled in, so you can go straight through. Kora continues where it stopped. For your safety the license key and database password are not saved, so you will type those again.".into());
        }
        None
    }

    /// Use another clipboard (tests inject a fake one).
    pub fn with_clipboard(mut self, c: Box<dyn Clipboard>) -> Self {
        self.clipboard = c;
        self
    }

    /// Longest text the focused box accepts.
    fn field_max(&self) -> usize {
        match (self.screen, self.focus) {
            (Screen::Ports, _) | (Screen::DbAdminLogin, 4) => 5,
            (Screen::GameBuild, _) => 8,
            (Screen::Name, _) | (Screen::Hostname, _) => 64,
            (Screen::GithubOwner, _) | (Screen::DbAdminLogin, 0) => 64,
            (Screen::License, _) => 128,
            (Screen::Cfx, _) => 32,
            (Screen::DbAdminLogin, 1) | (Screen::DbAdminLogin, 2) => 256,
            (Screen::DbAdminLogin, 3) => 255,
            (Screen::Folder, _) => 260,
            (Screen::ResourceList, _) => 1024,
            (Screen::ArtifactUrl, _) => 2048,
            _ => 256,
        }
    }

    /// Is the focused box a hidden one (license key, database password)?
    pub fn focused_is_masked(&self) -> bool {
        matches!((self.screen, self.focus), (Screen::License, _) | (Screen::DbAdminLogin, 1) | (Screen::DbAdminLogin, 2))
    }

    /// Insert pasted text into the focused box: cleaned, length-capped, and confirmed in the
    /// footer by count only (the text itself is never shown or logged).
    pub fn handle_paste(&mut self, text: &str) {
        if self.gh_on_screen() && !self.help_open {
            self.gh_paste(text);
            return;
        }
        if self.help_open || !self.is_text_screen() {
            return;
        }
        let screen = self.screen;
        let max = self.field_max();
        let digits: String;
        let mut raw = text.trim();
        if screen == Screen::Ports {
            // Keep digits first, so the length cap counts only what is kept.
            digits = raw.chars().filter(char::is_ascii_digit).collect();
            raw = &digits;
        }
        if screen == Screen::Folder {
            // "Copy as path" in Windows Explorer wraps the path in quotes.
            raw = raw.trim_matches('"');
        }
        let used = self.field_mut().map_or(0, |f| f.chars().count());
        let (clean, cut) = sanitize_paste(raw, max.saturating_sub(used));
        let clean = if screen == Screen::Ports { clean.chars().filter(char::is_ascii_digit).collect() } else { clean };
        let n = clean.chars().count();
        if n == 0 {
            self.status = Some("Nothing to paste: the clipboard had no text for this box.".into());
            return;
        }
        if let Some(f) = self.field_mut() {
            f.push_str(&clean);
        }
        self.after_edit(screen);
        self.status = Some(if cut {
            format!("Pasted {n} characters (shortened to fit)")
        } else {
            format!("Pasted {n} character{}", if n == 1 { "" } else { "s" })
        });
    }

    /// Paste from the system clipboard into the focused box (button, Ctrl+V, Shift+Insert).
    pub fn paste_clipboard(&mut self) {
        if self.help_open || !self.is_text_screen() {
            return;
        }
        match self.clipboard.get_text() {
            Ok(t) => self.handle_paste(&t),
            Err(e) => self.error = Some(e),
        }
    }

    /// Empty the focused box.
    pub fn clear_field(&mut self) {
        let screen = self.screen;
        if let Some(f) = self.field_mut() {
            f.clear();
            self.after_edit(screen);
            self.status = Some("Cleared".into());
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if self.state != WizardState::Running {
            return;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        self.status = None;
        if ctrl && key.code == KeyCode::Char('c') {
            self.state = WizardState::Quit;
            return;
        }
        if self.help_open {
            self.help_open = false;
            return;
        }
        if key.code == KeyCode::F(1) {
            self.help_open = true;
            return;
        }
        if self.gh_on_screen() {
            self.gh_key(key);
            return;
        }
        if self.on_db_admin_screen()
            && (key.code == KeyCode::F(2)
                || (ctrl && key.code == KeyCode::Char('t'))
                || (!self.is_text_screen() && key.code == KeyCode::Char('t')))
        {
            self.test_login();
            return;
        }
        if self.is_text_screen() {
            let shift = key.modifiers.contains(KeyModifiers::SHIFT);
            let screen_is_ports = self.screen == Screen::Ports && !ctrl;
            match key.code {
                KeyCode::Char('v' | 'V') if ctrl => self.paste_clipboard(),
                KeyCode::Insert if shift => self.paste_clipboard(),
                KeyCode::Char('u') if ctrl => self.clear_field(),
                KeyCode::F(3) => self.use_suggestion(None),
                KeyCode::Char('u' | 'U') if screen_is_ports => self.use_suggestion(None),
                KeyCode::Char(c) if !ctrl => self.type_char(c),
                KeyCode::Backspace => {
                    let screen = self.screen;
                    if let Some(f) = self.field_mut() {
                        f.pop();
                    }
                    self.after_edit(screen);
                }
                KeyCode::Up => self.focus = self.focus.saturating_sub(1),
                KeyCode::Down => self.focus = (self.focus + 1).min(self.input_count() - 1),
                KeyCode::Tab => {
                    if self.input_count() > 1 && self.focus + 1 < self.input_count() {
                        self.focus += 1;
                    } else {
                        self.go_next();
                    }
                }
                KeyCode::Enter => self.go_next(),
                KeyCode::Esc | KeyCode::BackTab => self.go_back(),
                _ => {}
            }
            return;
        }
        let n = self.option_count();
        match key.code {
            KeyCode::Char('?') => self.help_open = true,
            KeyCode::Char('q') => self.state = WizardState::Quit,
            KeyCode::Char('r' | 'R') if self.screen == Screen::Review && !self.review_reasons().is_empty() => self.request_elevation(),
            KeyCode::Up | KeyCode::Left if n > 0 => self.select(self.selected.saturating_sub(1)),
            KeyCode::Down | KeyCode::Right if n > 0 => self.select(self.selected + 1),
            KeyCode::Char(c @ '1'..='9') if n > 0 => {
                let i = c as usize - '1' as usize;
                if i < n {
                    self.select(i);
                }
            }
            KeyCode::Char('y') if matches!(self.screen, Screen::Firewall | Screen::Service | Screen::Shortcut | Screen::TxAdminPublic) => self.select(0),
            KeyCode::Char('n') if matches!(self.screen, Screen::Firewall | Screen::Service | Screen::Shortcut | Screen::TxAdminPublic) => self.select(1),
            KeyCode::Enter | KeyCode::Tab => self.go_next(),
            KeyCode::Esc | KeyCode::Backspace | KeyCode::BackTab | KeyCode::Char('b') => self.go_back(),
            _ => {}
        }
    }

    /// Mouse handling uses the same layout functions as rendering.
    pub fn handle_mouse(&mut self, ev: MouseEvent, area: Rect) {
        if self.state != WizardState::Running || ev.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        self.status = None;
        let pos = (ev.column, ev.row);
        let hit = |r: Rect| r.contains(ratatui::layout::Position::new(pos.0, pos.1));
        if self.help_open {
            self.help_open = false;
            return;
        }
        let l = wizard_view::layout(area, &self.view());
        if hit(l.back) {
            self.go_back();
        } else if hit(l.next) {
            self.go_next();
        } else if hit(l.help) {
            self.help_open = true;
        } else if hit(l.test) {
            self.test_login();
        } else if hit(l.elevate) {
            self.request_elevation();
        } else if self.gh_on_screen() {
            self.gh_click(ev, crate::github_screens::region(&l));
        } else if let Some(i) = l.options.iter().position(|r| hit(*r)) {
            self.select(i);
            if self.screen == Screen::Permission {
                self.go_next(); // the two choices are buttons
            }
        } else if let Some(i) = l.actions.iter().position(|r| hit(*r)) {
            self.use_suggestion(Some(i));
        } else if let Some(i) = l.paste.iter().position(|r| hit(*r)) {
            self.focus = i;
            self.paste_clipboard();
        } else if let Some(i) = l.clear.iter().position(|r| hit(*r)) {
            self.focus = i;
            self.clear_field();
        } else if let Some(i) = l.inputs.iter().position(|r| hit(*r)) {
            self.focus = i;
        }
    }

    fn parse_port(s: &str) -> Option<u16> {
        s.trim().parse::<u16>().ok().filter(|p| *p >= 1024)
    }

    /// Ports claimed by the user's other Kora servers.
    fn reserved_ports(&self) -> Vec<u16> {
        self.ports
            .servers
            .iter()
            .filter(|s| slug(&s.name) != slug(&self.name) && s.dir != Path::new(self.dir.trim()))
            .flat_map(|s| [s.ports.game, s.ports.txadmin])
            .collect()
    }

    fn other_server_using(&self, port: u16) -> Option<&str> {
        self.ports
            .servers
            .iter()
            .filter(|s| slug(&s.name) != slug(&self.name) && s.dir != Path::new(self.dir.trim()))
            .find(|s| s.ports.game == port || s.ports.txadmin == port)
            .map(|s| s.name.as_str())
    }

    /// Status of both port boxes (`None` for a box that doesn't hold a usable number yet).
    fn compute_ports_status(&self) -> [Option<PortLine>; 2] {
        let reg = self.reserved_ports();
        let (g, t) = (Self::parse_port(&self.game_port), Self::parse_port(&self.tx_port));
        let prober = &*self.ports.prober;
        let line = |port: Option<u16>, other: Option<u16>, udp: bool| -> Option<PortLine> {
            let port = port?;
            let avoid: Vec<u16> = reg.iter().copied().chain(other).collect();
            let why = if Some(port) == other {
                Some("is the same as the other port".to_string())
            } else if let Some(name) = self.other_server_using(port) {
                Some(format!("is used by your other server '{name}'"))
            } else if !(prober.tcp_free(port) && (!udp || prober.udp_free(port))) {
                Some("is already in use".to_string())
            } else {
                None
            };
            Some(match why {
                None => PortLine { port, ok: true, text: format!("{port} is free"), suggestion: None },
                Some(why) => {
                    let suggestion = suggest_free_port(port, &avoid, udp, prober);
                    let text = match suggestion {
                        Some(s) => format!("{port} {why} - {s} is free"),
                        None => format!("{port} {why} - no free port found nearby, please type another"),
                    };
                    PortLine { port, ok: false, text, suggestion }
                }
            })
        };
        [line(g, t, true), line(t, g, false)]
    }

    pub fn refresh_ports(&mut self) {
        self.ports_status = self.compute_ports_status();
    }

    /// First problem with the chosen ports, in plain words (fresh check).
    fn port_problem(&self) -> Option<String> {
        self.compute_ports_status().into_iter().flatten().find(|l| !l.ok).map(|l| match l.suggestion {
            Some(s) => format!("Port {}. Press U (or click the Use {s} button) to switch to {s}, or type another port.", l.text),
            None => format!("Port {}.", l.text),
        })
    }

    /// Keep each port when it is fine, otherwise move it to the next free one.
    fn pick_free_ports(&self, g: u16, t: u16) -> Option<(u16, u16)> {
        let reg = self.reserved_ports();
        let p = &*self.ports.prober;
        let ok = |port: u16, avoid: &[u16], udp: bool| !avoid.contains(&port) && p.tcp_free(port) && (!udp || p.udp_free(port));
        let av_g: Vec<u16> = reg.iter().copied().chain([t]).collect();
        let ng = if ok(g, &av_g, true) { g } else { suggest_free_port(g, &av_g, true, p)? };
        let av_t: Vec<u16> = reg.iter().copied().chain([ng]).collect();
        let nt = if ok(t, &av_t, false) { t } else { suggest_free_port(t, &av_t, false, p)? };
        Some((ng, nt))
    }

    /// If the default ports (untouched) are taken, fill in free ones.
    fn prefill_free_ports(&mut self) {
        if self.ports_touched {
            self.refresh_ports();
            return;
        }
        if let (Some(g), Some(t)) = (Self::parse_port(&self.game_port), Self::parse_port(&self.tx_port)) {
            if let Some((ng, nt)) = self.pick_free_ports(g, t) {
                if (ng, nt) != (g, t) {
                    self.ports_note = Some(format!(
                        "The usual ports ({g} and {t}) are already in use on this computer, so we picked free ones for you: {ng} for players and {nt} for the control panel."
                    ));
                    self.game_port = ng.to_string();
                    self.tx_port = nt.to_string();
                }
            }
        }
        self.refresh_ports();
    }

    /// Switch a port box to its suggested free port (`None` = the focused box, else the other one).
    pub fn use_suggestion(&mut self, which: Option<usize>) {
        if self.screen != Screen::Ports {
            return;
        }
        let order: Vec<usize> = match which {
            Some(i) => vec![i],
            None => vec![self.focus.min(1), 1 - self.focus.min(1)],
        };
        for i in order {
            if let Some(PortLine { suggestion: Some(s), .. }) = self.ports_status[i].clone() {
                if i == 0 { self.game_port = s.to_string() } else { self.tx_port = s.to_string() }
                self.ports_touched = true;
                self.ports_note = None;
                self.error = None;
                self.refresh_ports();
                self.status = Some(format!("Using port {s}"));
                return;
            }
        }
    }

    /// On the way into Review (and when Install is confirmed): if a port got taken in the
    /// meantime, go back to the Ports screen. Returns true when it did.
    fn recheck_ports_or_redirect(&mut self) -> bool {
        let Some(problem) = self.port_problem() else { return false };
        self.screen = Screen::Ports;
        self.load_selection();
        self.jump_review = true;
        self.error = Some(format!("A port was taken in the meantime. {problem}"));
        true
    }

    pub fn go_back(&mut self) {
        self.error = None;
        self.cfx_job = None;
        if let (Screen::GithubAccount, Some(g)) = (self.screen, self.gh.as_mut()) {
            g.leave();
        }
        self.jump_review = false;
        let flow = self.flow();
        let idx = flow.iter().position(|s| *s == self.screen).unwrap_or(0);
        if idx == 0 {
            self.state = WizardState::Quit;
        } else {
            self.screen = flow[idx - 1];
            self.load_selection();
        }
    }

    /// Ask for permission by restarting Kora elevated. On success this copy must exit.
    pub fn request_elevation(&mut self) {
        self.error = None;
        self.elev_note = None;
        match (self.elevator)(&self.elev_args) {
            Ok(()) => self.state = WizardState::Elevated,
            Err(e) => self.elev_note = Some(elev::failure_note(&e, self.windows)),
        }
    }

    /// "Not now" (Linux only): no service, no firewall, and a database the user already has (or none).
    fn choose_limited(&mut self) {
        self.limited = true;
        self.elev_note = None;
        self.service = false;
        self.firewall = false;
        self.txadmin_public = false;
        if self.db == DbChoice::InstallIfMissing {
            self.db = DbChoice::UseExisting;
        }
    }

    /// Things the current answers need administrator rights for, when Kora is not elevated.
    pub fn review_reasons(&self) -> Vec<String> {
        if !self.elevate_offer {
            return vec![];
        }
        // Looking for a database server takes a moment: do it once per visit to the screen.
        self.reasons_cache.borrow_mut().get_or_insert_with(|| elev::admin_reasons(&self.build_request())).clone()
    }

    /// Validate the current screen and advance. Sets `error` and stays on failure.
    pub fn go_next(&mut self) {
        if self.screen == Screen::Permission {
            if self.selected == 0 {
                self.request_elevation();
                return;
            }
            if self.windows {
                // Windows has no limited setup: the only other choice is to quit.
                self.state = WizardState::Quit;
                return;
            }
            self.choose_limited();
        }
        if self.screen == Screen::Cfx && self.cfx_step() {
            return;
        }
        if self.screen == Screen::Review && !self.review_reasons().is_empty() {
            self.error = Some("Kora needs your permission first: press R (or click Restart with permission), or go back and change those answers.".into());
            return;
        }
        if let Err(e) = self.validate_current() {
            self.error = Some(e);
            if self.screen == Screen::Ports {
                self.refresh_ports();
            }
            return;
        }
        self.error = None;
        self.apply_selection();
        if let Err(e) = self.check_database_before_leaving() {
            self.error = Some(e);
            return;
        }
        if self.screen == Screen::Ports && std::mem::take(&mut self.jump_review) {
            self.screen = Screen::Review;
            self.load_selection();
            return;
        }
        let flow = self.flow();
        let idx = flow.iter().position(|s| *s == self.screen).unwrap_or(0);
        // A port can be taken between choosing it and installing: look again on the way into
        // Review and once more when the install is confirmed.
        let leaving_to_review = idx + 1 >= flow.len() || flow[idx + 1] == Screen::Review;
        if leaving_to_review && self.screen != Screen::Ports && self.recheck_ports_or_redirect() {
            return;
        }
        if idx + 1 >= flow.len() {
            self.state = WizardState::Done;
        } else {
            self.screen = flow[idx + 1];
            self.load_selection();
        }
    }

    fn validate_current(&self) -> Result<(), String> {
        match self.screen {
            Screen::Name => validate_name(&self.name),
            Screen::Folder => {
                if self.dir.trim().is_empty() {
                    Err("Please choose a folder for your server.".into())
                } else {
                    Ok(())
                }
            }
            Screen::Ports => {
                validate_ports(&self.name, &self.game_port, &self.tx_port)?;
                match self.port_problem() {
                    Some(p) => Err(p),
                    None => Ok(()),
                }
            }
            Screen::ArtifactUrl => {
                let u = self.artifact_url.trim();
                if u.starts_with("https://") && u.len() > 12 && !u.contains(' ') {
                    Ok(())
                } else {
                    Err("Please paste a web address starting with https://".into())
                }
            }
            Screen::ResourceList => {
                if self.resource_ids.split(',').any(|s| !s.trim().is_empty()) {
                    Ok(())
                } else {
                    Err("Type at least one resource name, separated by commas.".into())
                }
            }
            Screen::DbAdminLogin => self.validate_db_admin(),
            Screen::GithubAccount => self.gh_validate(),
            Screen::License => validate_license(&self.license),
            Screen::GameBuild => {
                let g = self.game_build.trim();
                if g.is_empty() || g.parse::<u32>().is_ok() {
                    Ok(())
                } else {
                    Err("The game build must be a number (or leave it empty).".into())
                }
            }
            _ => Ok(()),
        }
    }

    /// Use another Cfx.re lookup (tests inject a fake one, so they never reach the network).
    pub fn with_cfx_lookup(mut self, f: CfxLookup) -> Self {
        self.cfx_lookup = f;
        self
    }

    /// A Cfx.re lookup is running in the background.
    pub fn cfx_checking(&self) -> bool {
        self.cfx_job.is_some()
    }

    /// Resolved admin for the Review screen: `(username, identifier)`.
    pub fn cfx_account(&self) -> Option<(String, String)> {
        Some((self.cfx_username.clone().unwrap_or_default(), self.cfx_identifier.clone()?))
    }

    /// Enter on the Cfx screen. Returns true when it handled the key (a lookup started or is running, or the name is wrong);
    /// false when the screen can be left (empty = skip, or the name is already resolved).
    fn cfx_step(&mut self) -> bool {
        if self.cfx_job.is_some() {
            return true;
        }
        let name = self.cfx_name.trim().trim_start_matches('@').to_string();
        if name.is_empty() {
            self.cfx_identifier = None;
            self.cfx_username = None;
            return false;
        }
        if self.cfx_identifier.is_some() && self.cfx_username.is_some() {
            return false;
        }
        if !kora_core::cfx::valid_username(&name) {
            self.error = Some("A Cfx.re username has 3 to 20 letters, numbers, dots, dashes or underscores. It is the name on your forum.cfx.re profile. Or leave it empty to skip.".into());
            return true;
        }
        self.error = None;
        let (tx, rx) = mpsc::channel();
        let f = self.cfx_lookup.clone();
        std::thread::spawn(move || {
            let _ = tx.send(f(&name));
        });
        self.cfx_job = Some(rx);
        true
    }

    /// Poll the Cfx.re lookup. Found: remember the account and continue; failed: stay and show why.
    pub(crate) fn cfx_tick(&mut self) -> bool {
        let Some(rx) = &self.cfx_job else { return false };
        let r = match rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => Err("The Cfx.re check stopped unexpectedly. Try again, or leave the name empty to skip.".into()),
        };
        self.cfx_job = None;
        match r {
            Ok(a) => {
                self.status = Some(format!("Found {} ({})", a.username, a.identifier()));
                self.cfx_identifier = Some(a.identifier());
                self.cfx_username = Some(a.username);
                self.go_next();
            }
            Err(e) => self.error = Some(e),
        }
        true
    }

    /// Which ports will be opened in the firewall, in plain words (also says when none are).
    pub fn firewall_line(&self) -> String {
        let (g, t) = (self.game_port.trim(), self.tx_port.trim());
        if !self.firewall {
            return format!("Open no firewall ports (you chose to do it yourself). Players need port {g} (TCP and UDP) open to join");
        }
        let fw = if self.windows { "Windows Firewall" } else { "the firewall" };
        if self.txadmin_public {
            format!("Open these ports in {fw}: {g} TCP and UDP (players join here) and {t} TCP (txAdmin control panel, reachable from the network)")
        } else {
            format!("Open this port in {fw}: {g} TCP and UDP (players join here). The control panel port {t} stays closed, so only this computer can reach it{}", if self.windows { ". If Windows asks about FXServer later, choose Allow" } else { "" })
        }
    }

    /// Plain-language list of everything the install will change on this computer.
    pub fn system_changes(&self) -> Vec<String> {
        let mut v = vec![
            format!("Create the folder {} and put your server there", self.dir.trim()),
            "Download the FiveM server software and Kora's resources (needs internet)".to_string(),
            if self.windows {
                "Install Git (and, for a development server, the GitHub CLI) with winget if missing. Windows will ask you to confirm each one".to_string()
            } else {
                "Install Git if it is missing".to_string()
            },
        ];
        if self.windows && !self.admin {
            v.push("Kora is not running as Administrator, so it will not install a database server, add the Windows service or open firewall ports (start Kora with Run as administrator to include them)".into());
        }
        match self.db {
            DbChoice::InstallIfMissing if self.windows => v.push(
                "Download MariaDB (about 100 MB) from MariaDB's official mirrors, check its checksum and run it as a Windows service that only listens on this computer (if you don't have one), then create a new database and user just for this server".into(),
            ),
            DbChoice::InstallIfMissing => v.push(
                "Install the MariaDB database if you don't have one (needs administrator rights), then create a new database and user just for this server".into(),
            ),
            DbChoice::UseExisting => {
                v.push("Create a new database and user just for this server in your existing database".into())
            }
            DbChoice::Skip => v.push("Leave databases alone (you'll add a connection later)".into()),
        }
        if self.db != DbChoice::Skip {
            v.push(match self.db_admin() {
                None => "Log in to the database as its administrator automatically (root socket or the KORA_DB_ADMIN_* settings); existing data is never touched".to_string(),
                Some(a) => format!("Log in to the database at {}:{} as '{}' once, only to create the new database and user (existing data is never touched; the password is not saved)", a.host, a.port, a.user),
            });
        }
        v.push(self.firewall_line());
        if self.service && self.windows {
            v.push(format!(
                "Windows service \"kora-{}\": starts with Windows and restarts after a crash. It runs under its own limited account that can only change the server folder",
                slug(&self.name)
            ));
        } else if self.service {
            v.push("Start the server automatically when this computer starts (needs administrator rights)".into());
        }
        if self.mode == kora_core::installer::Mode::Development {
            v.push(match self.github_owner.trim() {
                "" => "Copy (fork) Kora's resources to your GitHub account so you can change them".to_string(),
                o => format!("Copy (fork) Kora's resources to the GitHub account \"{o}\" so you can change them"),
            });
        }
        if !self.license.trim().is_empty() {
            v.push("Save your license key inside the server's private settings file (it never leaves this computer)".into());
        }
        if self.cfx_identifier.is_some() {
            v.push("Send your Cfx.re username to forum.cfx.re once (already done) and make that account the txAdmin admin".into());
        }
        v
    }

    /// Build the request. Only valid once the wizard is `Done`.
    pub fn build_request(&self) -> InstallRequest {
        let name = self.name.trim().to_string();
        let non_empty = |s: &str| {
            let t = s.trim();
            (!t.is_empty()).then(|| t.to_string())
        };
        InstallRequest {
            mode: self.mode,
            hostname: non_empty(&self.hostname).unwrap_or_else(|| name.clone()),
            name,
            dir: PathBuf::from(self.dir.trim()),
            license_key: non_empty(&self.license),
            cfx_identifier: self.cfx_identifier.clone(),
            ports: Ports {
                game: self.game_port.trim().parse().unwrap_or(30120),
                txadmin: self.tx_port.trim().parse().unwrap_or(40120),
            },
            artifact_url: if self.artifact_custom { non_empty(&self.artifact_url) } else { None },
            game_build: self.game_build.trim().parse().ok(),
            resources: if self.resources_all {
                ResourceSelection::All
            } else {
                ResourceSelection::Only(
                    self.resource_ids.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect(),
                )
            },
            db: self.db.clone(),
            service: self.service,
            firewall: self.firewall,
            txadmin_public: self.txadmin_public,
            github_owner: if self.mode == kora_core::installer::Mode::Development { non_empty(&self.github_owner) } else { None },
            catalog: None,
            dry_run: false,
            db_admin: self.db_admin(),
            start: true,
            app: Some(kora_core::app_install::AppChoice { desktop_shortcut: self.windows && self.desktop_shortcut }),
        }
    }

    pub fn view(&self) -> wizard_view::View {
        wizard_view::view(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState};

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent { code: c, modifiers: KeyModifiers::NONE, kind: KeyEventKind::Press, state: KeyEventState::NONE }
    }

    use std::cell::RefCell;
    use std::collections::HashSet;
    use std::rc::Rc;

    /// Fake prober whose taken ports can change while a wizard is running.
    #[derive(Clone, Default)]
    struct Fake {
        tcp: Rc<RefCell<HashSet<u16>>>,
        udp: Rc<RefCell<HashSet<u16>>>,
    }

    impl Fake {
        fn take(&self, ports: &[u16]) {
            self.tcp.borrow_mut().extend(ports.iter().copied());
        }
        fn take_udp(&self, ports: &[u16]) {
            self.udp.borrow_mut().extend(ports.iter().copied());
        }
        fn release(&self, port: u16) {
            self.tcp.borrow_mut().remove(&port);
            self.udp.borrow_mut().remove(&port);
        }
    }

    impl PortProber for Fake {
        fn tcp_free(&self, p: u16) -> bool {
            !self.tcp.borrow().contains(&p)
        }
        fn udp_free(&self, p: u16) -> bool {
            !self.udp.borrow().contains(&p)
        }
    }

    fn pwiz(f: &Fake) -> Wizard {
        Wizard::with_ports(PortEnv { prober: Box::new(f.clone()), servers: vec![] })
    }

    fn wiz() -> Wizard {
        pwiz(&Fake::default())
    }

    fn type_str(w: &mut Wizard, s: &str) {
        for c in s.chars() {
            w.handle_key(key(KeyCode::Char(c)));
        }
    }

    #[test]
    fn navigates_forward_and_back() {
        let mut w = wiz();
        assert_eq!(w.screen, Screen::Welcome);
        w.handle_key(key(KeyCode::Enter));
        assert_eq!(w.screen, Screen::Prepare);
        w.handle_key(key(KeyCode::Enter));
        assert_eq!(w.screen, Screen::Mode);
        w.handle_key(key(KeyCode::Esc));
        assert_eq!(w.screen, Screen::Prepare);
        w.handle_key(key(KeyCode::Esc));
        w.handle_key(key(KeyCode::Esc));
        w.handle_key(key(KeyCode::Esc));
        assert_eq!(w.state(), WizardState::Quit);
    }

    #[test]
    fn full_happy_path_builds_request() {
        let mut w = wiz();
        for _ in 0..3 {
            w.handle_key(key(KeyCode::Enter)); // welcome, prepare, mode(prod)
        }
        assert_eq!(w.screen, Screen::Name);
        for _ in 0..w.name.len() {
            w.handle_key(key(KeyCode::Backspace));
        }
        type_str(&mut w, "Alpha One");
        assert!(w.dir.ends_with("alpha-one"));
        let mut guard = 0;
        while w.state() == WizardState::Running && guard < 30 {
            w.handle_key(key(KeyCode::Enter));
            guard += 1;
        }
        assert_eq!(w.state(), WizardState::Done);
        let r = w.build_request();
        assert_eq!(r.name, "Alpha One");
        assert_eq!(r.mode, Mode::Production);
        assert_eq!(r.ports.game, 30120);
        assert!(r.license_key.is_none());
        assert!(matches!(r.resources, ResourceSelection::All));
        assert!(r.firewall && r.service);
    }

    #[test]
    fn desktop_shortcut_checkbox_is_windows_only_default_on_and_wired_to_the_app_option() {
        let mut w = wiz();
        w.windows = true;
        let flow = w.flow();
        let at = flow.iter().position(|s| *s == Screen::Shortcut).expect("the shortcut screen is in the Windows flow");
        assert_eq!(flow[at - 1], Screen::Service);
        assert_eq!(flow[at + 1], Screen::License);
        assert!(w.desktop_shortcut, "default on");
        assert!(w.build_request().app.unwrap().desktop_shortcut);
        w.screen = Screen::Shortcut;
        assert_eq!(w.option_count(), 2);
        w.handle_key(key(KeyCode::Char('n')));
        assert!(!w.desktop_shortcut);
        assert!(!w.build_request().app.unwrap().desktop_shortcut);
        w.handle_key(key(KeyCode::Char('y')));
        assert!(w.build_request().app.unwrap().desktop_shortcut);
        let v = crate::wizard_view::view(&w);
        assert!(v.title.to_string().contains("Kora shortcut on my desktop"));

        let mut l = wiz();
        l.windows = false;
        assert!(!l.flow().contains(&Screen::Shortcut), "no desktop on Linux");
        assert!(!l.build_request().app.unwrap().desktop_shortcut);
    }

    #[test]
    fn choices_change_flow() {
        let mut w = wiz();
        w.handle_key(key(KeyCode::Down)); // advanced
        assert!(w.advanced);
        w.handle_key(key(KeyCode::Enter));
        w.handle_key(key(KeyCode::Enter));
        w.handle_key(key(KeyCode::Char('2'))); // development
        assert_eq!(w.mode, Mode::Development);
        assert!(w.flow().contains(&Screen::GithubOwner));
        w.screen = Screen::Resources;
        w.select(1);
        assert!(w.flow().contains(&Screen::ResourceList));
        w.screen = Screen::Artifact;
        w.select(1);
        assert!(w.flow().contains(&Screen::ArtifactUrl));
    }

    #[test]
    fn name_validation() {
        assert!(validate_name("").is_err());
        assert!(validate_name("   ").is_err());
        assert!(validate_name("Good Name_1").is_ok());
        assert!(validate_name("bad/name").is_err());
        assert!(validate_name(&"x".repeat(40)).is_err());
        assert!(validate_name("---").is_err());
        let mut w = wiz();
        w.screen = Screen::Name;
        w.name = String::new();
        w.go_next();
        assert_eq!(w.screen, Screen::Name);
        assert!(w.error.is_some());
    }

    #[test]
    fn port_validation_and_conflicts() {
        assert_eq!(validate_ports("a", "30120", "40120").unwrap(), (30120, 40120));
        assert!(validate_ports("a", "abc", "40120").is_err());
        assert!(validate_ports("a", "80", "40120").is_err());
        assert!(validate_ports("a", "30120", "30120").is_err());
        assert!(validate_ports("a", "70000", "40120").is_err());

        let f = Fake::default();
        f.take(&[30120]);
        let mut w = pwiz(&f);
        w.screen = Screen::Ports;
        w.go_next();
        assert_eq!(w.screen, Screen::Ports);
        assert!(w.error.as_ref().unwrap().contains("already in use"));
    }

    #[test]
    fn license_is_masked_everywhere() {
        let key_text = "cfxk_ABCDEFGHIJKLMNOPQRST_1234";
        let mut w = wiz();
        w.screen = Screen::License;
        w.handle_paste(key_text);
        assert_eq!(w.license, key_text);
        assert!(!format!("{w:?}").contains("ABCDEFGH"));
        let req = w.build_request();
        assert!(!format!("{req:?}").contains("ABCDEFGH"));
        let v = w.view();
        assert!(v.inputs[0].masked);
        let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 34)).unwrap();
        t.draw(|f| wizard_view::render(f, &w)).unwrap();
        assert!(!format!("{:?}", t.backend().buffer()).contains("ABCDEFGH"));
        assert!(validate_license("short key").is_err());
        assert!(validate_license("").is_ok());
    }

    #[test]
    fn mouse_click_next_and_back() {
        let mut w = wiz();
        let area = Rect::new(0, 0, 100, 34);
        let l = wizard_view::layout(area, &w.view());
        let click = |r: Rect| MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: r.x + 1,
            row: r.y + 1,
            modifiers: KeyModifiers::NONE,
        };
        w.handle_mouse(click(l.next), area);
        assert_eq!(w.screen, Screen::Prepare);
        let l = wizard_view::layout(area, &w.view());
        w.handle_mouse(click(l.back), area);
        assert_eq!(w.screen, Screen::Welcome);
        let l = wizard_view::layout(area, &w.view());
        w.handle_mouse(click(l.options[1]), area);
        assert!(w.advanced);
        w.handle_mouse(click(l.help), area);
        assert!(w.help_open);
    }

    #[test]
    fn help_and_ctrl_c() {
        let mut w = wiz();
        w.handle_key(key(KeyCode::Char('?')));
        assert!(w.help_open);
        w.handle_key(key(KeyCode::Esc));
        assert!(!w.help_open);
        assert_eq!(w.state(), WizardState::Running);
        w.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(w.state(), WizardState::Quit);
    }

    #[test]
    fn every_screen_renders_at_various_sizes() {
        let mut w = wiz();
        w.advanced = true;
        w.mode = Mode::Development;
        w.artifact_custom = true;
        w.resources_all = false;
        w.license = "cfxk_ABCDEFGHIJKLMNOPQRST_1234".into();
        w.error = Some("Example problem message".into());
        for (cols, rows) in [(100u16, 34u16), (80, 24), (40, 12), (12, 4)] {
            for s in w.flow() {
                w.screen = s;
                w.load_selection();
                for help in [false, true] {
                    w.help_open = help;
                    let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(cols, rows)).unwrap();
                    t.draw(|f| wizard_view::render(f, &w)).unwrap();
                    if cols == 100 && !help {
                        let text = format!("{:?}", t.backend().buffer());
                        assert!(text.contains("Back") && text.contains("Next") || s == Screen::Review);
                    }
                }
            }
        }
    }

    fn ok_tester(a: Option<&DbAdmin>) -> Result<(), String> {
        match a {
            Some(a) if a.password.as_deref() == Some("pw") => Ok(()),
            Some(_) => Err("Access denied".into()),
            None => Ok(()),
        }
    }

    fn dbw() -> Wizard {
        let mut w = Wizard::with_hooks(PortEnv { prober: Box::new(Fake::default()), servers: vec![] }, ok_tester);
        w.db = DbChoice::UseExisting;
        w.screen = Screen::Database;
        w
    }

    #[test]
    fn db_admin_screen_flow_and_skip() {
        let mut w = dbw();
        assert!(w.flow().contains(&Screen::DbAdmin));
        assert!(!w.flow().contains(&Screen::DbAdminLogin));
        w.db = DbChoice::Skip;
        assert!(!w.flow().contains(&Screen::DbAdmin));
        w.db = DbChoice::InstallIfMissing;
        w.go_next();
        assert_eq!(w.screen, Screen::DbAdmin);
        w.handle_key(key(KeyCode::Char('2')));
        assert!(w.flow().contains(&Screen::DbAdminLogin));
        w.handle_key(key(KeyCode::Enter));
        assert_eq!(w.screen, Screen::DbAdminLogin);
        w.handle_key(key(KeyCode::Esc));
        assert_eq!(w.screen, Screen::DbAdmin);
        w.handle_key(key(KeyCode::Char('1')));
        assert!(w.build_request().db_admin.is_none());
    }

    #[test]
    fn db_admin_login_test_action() {
        let mut w = dbw();
        w.screen = Screen::DbAdmin;
        w.load_selection();
        w.handle_key(key(KeyCode::Char('t'))); // automatic login test
        assert_eq!(w.db_test, Some(Ok(())));
        w.handle_key(key(KeyCode::Char('2')));
        assert!(w.db_test.is_none());
        w.handle_key(key(KeyCode::Enter));
        assert_eq!(w.screen, Screen::DbAdminLogin);
        // wrong password
        w.handle_paste("admin");
        assert_eq!(w.dba_user, "rootadmin");
        for _ in 0..5 {
            w.handle_key(key(KeyCode::Backspace));
        }
        w.handle_key(key(KeyCode::Tab));
        type_str(&mut w, "bad");
        w.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL));
        assert!(matches!(&w.db_test, Some(Err(e)) if e.contains("don't match")), "the confirm box must match first");
        w.dba_pass2 = "bad".into();
        w.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL));
        assert_eq!(w.db_test, Some(Err("Access denied".into())));
        assert_eq!(w.dba_pass, "bad", "Ctrl+T must not type a character");
        // fix the password: result is cleared, retest succeeds
        for _ in 0..3 {
            w.handle_key(key(KeyCode::Backspace));
        }
        assert!(w.db_test.is_none());
        type_str(&mut w, "pw");
        w.dba_pass2 = "pw".into();
        w.handle_key(key(KeyCode::F(2)));
        assert_eq!(w.db_test, Some(Ok(())));
        // the view shows the result and masks the password
        let v = w.view();
        assert!(v.inputs[1].masked && v.inputs[2].masked && v.has_test);
        assert!(v.lines.iter().any(|l| l.contains("worked")));
        let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 34)).unwrap();
        t.draw(|f| wizard_view::render(f, &w)).unwrap();
        let text = format!("{:?}", t.backend().buffer());
        assert!(text.contains("Test login") && !text.contains("pw\""));
        let r = w.build_request();
        let a = r.db_admin.unwrap();
        assert_eq!((a.user.as_str(), a.host.as_str(), a.port, a.password.as_deref()), ("root", "127.0.0.1", 3306, Some("pw")));
        assert!(!format!("{a:?}").contains("pw\""));
        assert!(!w.system_changes().iter().any(|c| c.contains("pw")));
    }

    #[test]
    fn db_admin_login_validates_fields_and_mouse_test() {
        let mut w = dbw();
        w.dba_custom = true;
        w.screen = Screen::DbAdminLogin;
        w.dba_port = "abc".into();
        w.go_next();
        assert_eq!(w.screen, Screen::DbAdminLogin);
        assert!(w.error.as_ref().unwrap().contains("port"));
        w.dba_port = "33061".into();
        w.dba_pass = "pw".into();
        w.dba_pass2 = "other".into();
        w.go_next();
        assert_eq!(w.screen, Screen::DbAdminLogin);
        assert!(w.error.as_ref().unwrap().contains("don't match"));
        w.dba_pass2 = "pw".into();
        let area = Rect::new(0, 0, 100, 34);
        let l = wizard_view::layout(area, &w.view());
        w.handle_mouse(
            MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: l.test.x + 1, row: l.test.y + 1, modifiers: KeyModifiers::NONE },
            area,
        );
        assert_eq!(w.db_test, Some(Ok(())));
        w.go_next();
        assert_eq!(w.screen, Screen::Firewall);
        assert_eq!(w.build_request().db_admin.unwrap().port, 33061);
    }

    #[test]
    fn review_lists_changes() {
        let mut w = wiz();
        w.screen = Screen::Review;
        let changes = w.system_changes();
        assert!(changes.iter().any(|c| c.contains("firewall")));
        assert!(changes.iter().any(|c| c.contains("MariaDB")));
        w.firewall = false;
        let c = w.system_changes();
        assert!(!c.iter().any(|c| c.contains("Open this port")));
        assert!(c.iter().any(|c| c.contains("Open no firewall ports")));
    }

    #[test]
    fn firewall_line_names_every_port_that_is_opened() {
        let mut w = wiz();
        let l = w.firewall_line();
        assert!(l.contains("30120 TCP and UDP") && l.contains("40120 stays closed"), "{l}");
        w.txadmin_public = true;
        let l = w.firewall_line();
        assert!(l.contains("30120 TCP and UDP") && l.contains("40120 TCP") && l.contains("control panel"), "{l}");
    }

    #[test]
    fn a_failing_database_login_blocks_next_and_a_working_one_continues() {
        fn bad(_: Option<&DbAdmin>) -> Result<(), String> {
            Err("Access denied.".into())
        }
        let mut w = dbw();
        w.db_tester = bad;
        w.db = DbChoice::UseExisting;
        w.screen = Screen::DbAdmin;
        w.load_selection();
        w.go_next();
        assert_eq!(w.screen, Screen::DbAdmin, "must not continue with a broken database login");
        assert!(w.error.as_ref().unwrap().contains("Access denied"));
        // "set it up for me": the server may not exist yet, so a failed automatic probe does not block
        w.db = DbChoice::InstallIfMissing;
        w.go_next();
        assert_ne!(w.screen, Screen::DbAdmin);
        // a typed login always has to work
        w.screen = Screen::DbAdminLogin;
        w.dba_custom = true;
        w.go_next();
        assert_eq!(w.screen, Screen::DbAdminLogin);
        w.db_tester = |_| Ok(());
        w.go_next();
        assert_eq!(w.screen, Screen::Firewall);
    }

    fn render_text(w: &Wizard) -> String {
        let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(110, 40)).unwrap();
        t.draw(|f| wizard_view::render(f, w)).unwrap();
        let b = t.backend().buffer().clone();
        (0..b.area.height).map(|y| (0..b.area.width).map(|x| b[(x, y)].symbol().to_string()).collect::<String>()).collect::<Vec<_>>().join("\n")
    }

    // ---- paste / clear / clipboard ----

    use crate::clipboard::FakeClipboard;
    use crossterm::event::MouseButton as Mb;

    fn cwiz(text: Option<&str>) -> Wizard {
        wiz().with_clipboard(Box::new(FakeClipboard { text: text.map(String::from), fail: false }))
    }

    fn left_click(r: Rect) -> MouseEvent {
        MouseEvent { kind: MouseEventKind::Down(Mb::Left), column: r.x + 1, row: r.y, modifiers: KeyModifiers::NONE }
    }

    #[test]
    fn paste_is_sanitised_and_confirmed_by_count() {
        let mut w = cwiz(None);
        w.screen = Screen::Name;
        w.name.clear();
        w.handle_paste("  Alpha\r\nOne\t\x1b[0m  ");
        assert_eq!(w.name, "AlphaOne[0m");
        assert_eq!(w.status.as_deref(), Some("Pasted 11 characters"));
        // capped per field, and says so
        w.screen = Screen::Ports;
        w.focus = 0;
        w.game_port.clear();
        w.handle_paste("1234567890");
        assert_eq!(w.game_port, "12345");
        assert!(w.status.as_ref().unwrap().contains("shortened"));
        // empty paste is explained
        w.handle_paste("\n\n");
        assert!(w.status.as_ref().unwrap().contains("Nothing to paste"));
        // not a text screen: ignored
        w.screen = Screen::Mode;
        w.status = None;
        w.handle_paste("x");
        assert!(w.status.is_none());
    }

    #[test]
    fn folder_paste_strips_quotes_from_copy_as_path() {
        let mut w = cwiz(None);
        w.screen = Screen::Folder;
        w.dir.clear();
        w.handle_paste("\"C:\\Games\\my server\"\r\n");
        assert_eq!(w.dir, "C:\\Games\\my server");
    }

    #[test]
    fn masked_fields_stay_masked_and_are_never_echoed() {
        let secret = "cfxk_ABCDEFGHIJKLMNOPQRST_1234";
        let mut w = cwiz(Some(secret));
        w.screen = Screen::License;
        w.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL));
        assert_eq!(w.license, secret);
        assert!(w.focused_is_masked());
        assert_eq!(w.status.as_deref(), Some("Pasted 30 characters"));
        assert!(!render_text(&w).contains("ABCDEFGH"));
        assert!(!format!("{w:?}").contains("ABCDEFGH"));
        // database password too
        w.screen = Screen::DbAdminLogin;
        w.focus = 1;
        w.clipboard.set_text("hunter2-secret").unwrap();
        w.handle_key(KeyEvent::new(KeyCode::Insert, KeyModifiers::SHIFT));
        assert_eq!(w.dba_pass, "hunter2-secret");
        let t = render_text(&w);
        assert!(!t.contains("hunter2") && t.contains("**************"));
        assert!(!w.status.clone().unwrap().contains("hunter2"));
        assert!(!format!("{w:?}").contains("hunter2"));
        // Clear empties it
        w.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert!(w.dba_pass.is_empty());
    }

    #[test]
    fn keyboard_paste_variants_and_failure_message() {
        let mut w = cwiz(Some("Beta"));
        w.screen = Screen::Name;
        w.name.clear();
        w.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL));
        w.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::CONTROL | KeyModifiers::SHIFT));
        w.handle_key(KeyEvent::new(KeyCode::Insert, KeyModifiers::SHIFT));
        assert_eq!(w.name, "BetaBetaBeta");
        assert_eq!(w.dir, default_dir("BetaBetaBeta"), "folder follows the name like typing does");
        // no clipboard text / no clipboard at all -> plain message, nothing typed
        let mut w = wiz().with_clipboard(Box::new(FakeClipboard { text: None, fail: true }));
        w.screen = Screen::Name;
        w.name.clear();
        w.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL));
        assert!(w.name.is_empty());
        assert!(w.error.as_ref().unwrap().contains("clipboard isn't available"));
        assert!(render_text(&w).contains("clipboard isn't available"));
        // Ctrl+V on a choice screen does nothing and doesn't panic
        w.screen = Screen::Mode;
        w.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL));
    }

    #[test]
    fn paste_and_clear_buttons_are_clickable_for_every_text_field() {
        let area = Rect::new(0, 0, 110, 40);
        let mut w = cwiz(Some("12"));
        w.advanced = true;
        w.mode = Mode::Development;
        w.artifact_custom = true;
        w.resources_all = false;
        w.dba_custom = true;
        for s in w.flow() {
            w.screen = s;
            w.load_selection();
            for i in 0..w.input_count() {
                let l = wizard_view::layout(area, &w.view());
                assert_eq!(l.paste.len(), w.input_count());
                assert!(l.paste[i].width > 0 && l.clear[i].width > 0, "{s:?}/{i} has buttons");
                assert!(!l.paste[i].intersects(l.clear[i]) && !l.paste[i].intersects(l.inputs[i]));
                let text = render_text(&w);
                assert!(text.contains("[ Paste ]") && text.contains("[ Clear ]"), "{s:?}");
                w.handle_mouse(left_click(l.clear[i]), area);
                assert_eq!(w.focus, i);
                assert_eq!(w.status.as_deref(), Some("Cleared"), "{s:?}/{i}");
                w.handle_mouse(left_click(l.paste[i]), area);
                assert_eq!(w.status.as_deref(), Some("Pasted 2 characters"), "{s:?}/{i}");
            }
        }
        // a narrow window drops the buttons but keeps the field usable
        let narrow = Rect::new(0, 0, 40, 24);
        w.screen = Screen::Name;
        assert!(wizard_view::layout(narrow, &w.view()).paste[0].width == 0);
    }

    #[test]
    fn paste_event_goes_to_the_focused_field_and_help_blocks_it() {
        let mut w = cwiz(None);
        w.screen = Screen::DbAdminLogin;
        w.dba_custom = true;
        w.focus = 3;
        w.dba_host.clear();
        w.handle_paste("db.example.com\n");
        assert_eq!(w.dba_host, "db.example.com");
        assert!(w.db_test.is_none());
        w.help_open = true;
        w.handle_paste("zzz");
        assert_eq!(w.dba_host, "db.example.com");
    }

    #[test]
    fn help_mentions_new_keys() {
        let w = cwiz(None);
        assert!(w.view().help.contains("Ctrl+V"));
        let mut w = w;
        w.screen = Screen::License;
        let h = w.view().help;
        assert!(h.contains("Shift+Insert") && h.contains("Ctrl+U"));
    }

    // ---- port checks ----

    use kora_core::installer::registry::PortsRecord;

    fn at_ports(f: &Fake) -> Wizard {
        let mut w = pwiz(f);
        w.screen = Screen::Folder;
        w.go_next();
        assert_eq!(w.screen, Screen::Ports);
        w
    }

    fn walk_to(w: &mut Wizard, target: Screen) {
        let mut guard = 0;
        while w.screen != target && guard < 40 {
            w.go_next();
            assert!(w.error.is_none(), "{:?}: {:?}", w.screen, w.error);
            guard += 1;
        }
        assert_eq!(w.screen, target);
    }

    #[test]
    fn free_ports_show_a_plain_ok_line() {
        let w = at_ports(&Fake::default());
        assert_eq!(w.ports_status[0].as_ref().unwrap().text, "30120 is free");
        assert_eq!(w.ports_status[1].as_ref().unwrap().text, "40120 is free");
        let t = render_text(&w);
        assert!(t.contains("30120 is free") && !t.contains("[ Use"), "{t}");
        assert!(w.ports_note.is_none());
    }

    #[test]
    fn taken_default_port_is_prefilled_with_the_next_free_one() {
        let f = Fake::default();
        f.take(&[30120, 30121]);
        let w = at_ports(&f);
        assert_eq!((w.game_port.as_str(), w.tx_port.as_str()), ("30122", "40120"));
        assert!(w.ports_note.as_ref().unwrap().contains("30120") && w.ports_note.as_ref().unwrap().contains("30122"));
        assert!(render_text(&w).contains("30122 is free"));
        // UDP-only conflicts matter for the game port; txAdmin only needs TCP
        let f = Fake::default();
        f.take_udp(&[30120, 40120]);
        let w = at_ports(&f);
        assert_eq!((w.game_port.as_str(), w.tx_port.as_str()), ("30121", "40120"));
        // later ports are never reused: the two never collide
        let f = Fake::default();
        f.take(&[40120]);
        let w = at_ports(&f);
        assert_eq!(w.tx_port, "40121");
    }

    #[test]
    fn typed_taken_port_blocks_next_until_the_suggestion_is_used_by_click() {
        let f = Fake::default();
        let mut w = at_ports(&f);
        f.take(&[30125]);
        w.game_port.clear();
        type_str(&mut w, "30125");
        let line = w.ports_status[0].clone().unwrap();
        assert!(!line.ok && line.suggestion == Some(30126));
        assert_eq!(line.text, "30125 is already in use - 30126 is free");
        let t = render_text(&w);
        assert!(t.contains("30125 is already in use - 30126 is free") && t.contains("[ Use 30126 ]"), "{t}");
        w.go_next();
        assert_eq!(w.screen, Screen::Ports, "blocked while taken");
        assert!(w.error.as_ref().unwrap().contains("already in use"));
        let area = Rect::new(0, 0, 110, 40);
        let l = wizard_view::layout(area, &w.view());
        assert!(l.actions[0].width > 0 && l.actions[1].width == 0, "only the taken port has a button");
        w.handle_mouse(left_click(l.actions[0]), area);
        assert_eq!(w.game_port, "30126");
        assert_eq!(w.status.as_deref(), Some("Using port 30126"));
        assert!(w.error.is_none() && w.ports_status[0].as_ref().unwrap().ok);
        w.go_next();
        assert_eq!(w.screen, Screen::Artifact);
    }

    #[test]
    fn keys_use_the_suggestion_and_port_boxes_take_digits_only() {
        let f = Fake::default();
        let mut w = at_ports(&f);
        f.take(&[40120]);
        w.focus = 1;
        w.tx_port.clear();
        type_str(&mut w, "40120");
        w.handle_key(key(KeyCode::Char('x')));
        assert_eq!(w.tx_port, "40120", "letters are not typed into a port box");
        w.handle_key(key(KeyCode::Char('u')));
        assert_eq!(w.tx_port, "40121");
        w.focus = 0;
        f.take(&[30120]);
        w.game_port.clear();
        type_str(&mut w, "30120");
        w.handle_key(key(KeyCode::F(3)));
        assert_eq!(w.game_port, "30121");
        w.game_port.clear();
        w.handle_paste("  4x0 1y22\n");
        assert_eq!(w.game_port, "40122", "paste keeps digits only");
    }

    #[test]
    fn ports_of_other_kora_servers_count_as_taken_even_when_stopped() {
        use kora_core::installer::Mode as M;
        let other = ServerEntry { name: "Alpha RP".into(), dir: "/srv/alpha".into(), mode: M::Production, ports: PortsRecord { game: 30120, txadmin: 40120 }, created_unix: 0 };
        let mut w = Wizard::with_ports(PortEnv { prober: Box::new(Fake::default()), servers: vec![other] });
        w.screen = Screen::Folder;
        w.go_next();
        assert_eq!((w.game_port.as_str(), w.tx_port.as_str()), ("30121", "40121"), "defaults moved off the other server's ports");
        w.game_port.clear();
        type_str(&mut w, "30120");
        let t = render_text(&w);
        assert!(t.contains("30120 is used by your other server 'Alpha RP' - 30121 is free"), "{t}");
        w.go_next();
        assert_eq!(w.screen, Screen::Ports);
        // re-installing the same server (same name) is not blocked by its own entry
        w.name = "alpha rp".into();
        w.refresh_ports();
        assert!(w.ports_status[0].as_ref().unwrap().ok);
    }

    #[test]
    fn ports_are_rechecked_on_the_way_into_review_and_when_installing() {
        let f = Fake::default();
        let mut w = pwiz(&f);
        walk_to(&mut w, Screen::Cfx);
        f.take(&[30120]); // someone grabs the port after it was chosen
        w.go_next();
        assert_eq!(w.screen, Screen::Ports, "sent back instead of reaching Review");
        assert!(w.error.as_ref().unwrap().contains("taken in the meantime"));
        assert_eq!(w.game_port, "30121", "untouched defaults get the free port pre-filled");
        w.go_next();
        assert_eq!(w.screen, Screen::Review, "straight back to Review after fixing it");
        assert!(w.error.is_none());
        // and once more when Install is confirmed
        f.take(&[30121]);
        w.go_next();
        assert_eq!(w.state(), WizardState::Running);
        assert_eq!(w.screen, Screen::Ports);
        assert!(w.error.as_ref().unwrap().contains("taken in the meantime"));
        assert_eq!(w.game_port, "30122");
        f.release(30121);
        w.go_next(); // the pre-filled port is free -> back to Review
        assert_eq!(w.screen, Screen::Review);
        w.go_next();
        assert_eq!(w.state(), WizardState::Done);
        assert_eq!(w.build_request().ports.game, 30122);
    }

    #[test]
    fn help_and_hint_mention_the_port_keys() {
        let w = at_ports(&Fake::default());
        let h = w.view().help;
        assert!(h.contains("press U or F3") && h.contains("next free"), "{h}");
        assert!(h.contains("Ctrl+V"));
    }

    #[test]
    fn port_screen_renders_at_small_sizes_with_buttons() {
        let f = Fake::default();
        let mut w = at_ports(&f);
        f.take(&[30120, 40120]);
        w.game_port = "30120".into();
        w.tx_port = "40120".into();
        w.refresh_ports();
        for (c, r) in [(110u16, 40u16), (80, 24), (45, 14), (12, 4)] {
            let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(c, r)).unwrap();
            t.draw(|f| wizard_view::render(f, &w)).unwrap();
        }
        let area = Rect::new(0, 0, 80, 24);
        let l = wizard_view::layout(area, &w.view());
        assert!(l.actions[0].width > 0 && l.actions[1].width > 0 && !l.actions[0].intersects(l.actions[1]));
        w.handle_mouse(left_click(l.actions[1]), area);
        assert_eq!(w.tx_port, "40121");
    }

    // ---- Windows wording ----

    #[test]
    fn windows_prepare_screen_explains_admin_uac_antivirus_disk_and_ports() {
        let mut w = wiz().with_platform(true, false);
        w.screen = Screen::Prepare;
        let t = render_text(&w);
        for want in [
            "Before we start (Windows)", "not running as Administrator", "Run as administrator", "winget", "Protection history", "10 GB", "30120",
            "Windows protected your PC",
        ] {
            assert!(t.contains(want), "missing {want:?}\n{t}");
        }
        let mut w = wiz().with_platform(true, true);
        w.screen = Screen::Prepare;
        let t = render_text(&w);
        assert!(t.contains("Running as Administrator") && t.contains("User Account Control") && !t.contains("not running as Administrator"), "{t}");
        // Linux wording is unchanged
        let mut w = wiz();
        w.screen = Screen::Prepare;
        let t = render_text(&w);
        assert!(t.contains("Before we start") && !t.contains("(Windows)") && t.contains("sudo password"), "{t}");
    }

    #[test]
    fn windows_review_lines_are_specific() {
        let mut w = wiz().with_platform(true, false);
        w.screen = Screen::Review;
        let c = w.system_changes().join("\n");
        for want in [
            "Open this port in Windows Firewall: 30120 TCP and UDP", "choose Allow", "Windows service \"kora-my-kora-server\"", "limited account",
            "winget", "official mirrors", "not running as Administrator",
        ] {
            assert!(c.contains(want), "missing {want:?}\n{c}");
        }
        let c = wiz().with_platform(true, true).system_changes().join("\n");
        assert!(!c.contains("not running as Administrator"));
        let c = wiz().system_changes().join("\n");
        assert!(c.contains("Install Git if it is missing") && c.contains("Open this port in the firewall: 30120 TCP and UDP"), "{c}");
    }

    // ---- permission step ----

    use crate::elev::ElevateError;
    use crossterm::event::MouseEvent;

    type Calls = Rc<RefCell<Vec<Vec<OsString>>>>;

    /// A wizard that offers elevation; the fake elevator records its arguments and answers `answer`.
    fn ewiz(windows: bool, answer: Result<(), ElevateError>) -> (Wizard, Calls) {
        let calls: Calls = Rc::default();
        let c2 = calls.clone();
        let w = wiz().with_platform(windows, false).with_elevation(
            true,
            vec!["install".into(), "--name".into(), "X".into()],
            Box::new(move |a| {
                c2.borrow_mut().push(a.to_vec());
                answer.clone()
            }),
        );
        (w, calls)
    }

    fn rendered(w: &Wizard) -> String {
        render_text(w)
    }

    fn area() -> Rect {
        Rect::new(0, 0, 110, 40)
    }

    fn click_at(r: Rect) -> MouseEvent {
        MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: r.x + 1, row: r.y, modifiers: KeyModifiers::NONE }
    }

    #[test]
    fn permission_screen_is_first_only_when_not_elevated() {
        let (w, _) = ewiz(false, Ok(()));
        assert_eq!(w.screen, Screen::Permission);
        assert_eq!(w.flow()[..3], [Screen::Permission, Screen::Welcome, Screen::Prepare]);
        let t = rendered(&w);
        for want in ["To set up your server Kora needs your permission to:", "install the database server", "run your server in the background", "open the firewall ports", "sudo password", "[ Continue", "[ Not now - limited setup ]"] {
            assert!(t.contains(want), "missing {want:?}\n{t}");
        }
        // elevated (or no way to prompt): no panel at all
        let w = wiz().with_platform(false, true).with_elevation(false, vec![], crate::elev::system_elevator());
        assert_eq!(w.screen, Screen::Welcome);
        assert!(!w.flow().contains(&Screen::Permission));
        assert!(!rendered(&w).contains("needs your permission"));
        assert!(!wiz().flow().contains(&Screen::Permission), "default test wizard has no offer");
    }

    #[test]
    fn continue_calls_the_callback_with_the_original_args_and_exits() {
        let (mut w, calls) = ewiz(false, Ok(()));
        w.handle_key(key(KeyCode::Enter));
        assert_eq!(calls.borrow().as_slice(), &[vec![OsString::from("install"), "--name".into(), "X".into()]]);
        assert_eq!(w.state(), WizardState::Elevated);
        assert_eq!(w.screen, Screen::Permission);
    }

    #[test]
    fn declined_keeps_the_screen_and_says_nothing_changed() {
        let (mut w, calls) = ewiz(false, Err(ElevateError::Declined));
        w.handle_key(key(KeyCode::Enter));
        assert_eq!((w.state(), w.screen), (WizardState::Running, Screen::Permission));
        assert!(rendered(&w).contains("No problem - nothing was changed. You can try again, or choose the limited setup."));
        w.handle_key(key(KeyCode::Enter)); // try again
        assert_eq!(calls.borrow().len(), 2);
    }

    #[test]
    fn failures_show_the_reason_and_how_to_do_it_by_hand() {
        let (mut w, _) = ewiz(false, Err(ElevateError::Failed("sudo exploded".into())));
        w.handle_key(key(KeyCode::Enter));
        let t = rendered(&w);
        assert!(t.contains("sudo exploded") && t.contains("sudo kora install"), "{t}");
        let (mut w, _) = ewiz(true, Err(ElevateError::Unsupported));
        w.handle_key(key(KeyCode::Enter));
        let t = rendered(&w);
        assert!(t.contains("Right-click") && t.contains("Run as administrator"), "{t}");
    }

    #[test]
    fn limited_setup_uses_the_no_admin_defaults_and_review_says_what_is_skipped() {
        let (mut w, calls) = ewiz(false, Ok(()));
        w.handle_key(key(KeyCode::Down));
        w.handle_key(key(KeyCode::Enter));
        assert!(calls.borrow().is_empty());
        assert_eq!(w.screen, Screen::Welcome);
        let r = w.build_request();
        assert!(!r.service && !r.firewall);
        assert_eq!(r.db, DbChoice::UseExisting);
        assert!(w.review_reasons().is_empty(), "limited defaults need no permission");
        w.screen = Screen::Database;
        assert!(rendered(&w).contains("Limited setup"), "database screen explains");
        w.screen = Screen::Review;
        let t = rendered(&w);
        assert!(t.contains("no database server, background service or firewall rules"), "{t}");
        assert!(!t.contains("Restart with permission"));
    }

    #[test]
    fn windows_has_no_limited_option_and_quit_leaves() {
        let (w, _) = ewiz(true, Ok(()));
        let t = rendered(&w);
        assert!(t.contains("Kora needs to run as administrator") && t.contains("[ Restart as administrator ]") && t.contains("[ Quit ]"), "{t}");
        assert!(t.contains("Windows will ask you to approve this once."));
        assert!(!t.contains("limited") && !t.contains("Not now"), "{t}");
        let (mut w, calls) = ewiz(true, Ok(()));
        w.handle_key(key(KeyCode::Down));
        w.handle_key(key(KeyCode::Enter));
        assert_eq!(w.state(), WizardState::Quit);
        assert!(calls.borrow().is_empty());
        assert!(!w.limited);
    }

    #[test]
    fn review_blocks_with_a_restart_button_when_the_answers_need_admin() {
        let (mut w, calls) = ewiz(false, Err(ElevateError::Declined));
        w.handle_key(key(KeyCode::Down));
        w.handle_key(key(KeyCode::Enter)); // limited
        w.db = DbChoice::Skip;
        w.service = true; // user later ticks "start automatically"
        w.screen = Screen::Review;
        let t = rendered(&w);
        assert!(t.contains("Kora needs your permission to:") && t.contains("register the server as a") && t.contains("[ Restart with permission ]"), "{t}");
        w.handle_key(key(KeyCode::Enter)); // Install is blocked
        assert_eq!((w.state(), w.screen), (WizardState::Running, Screen::Review));
        assert!(w.error.as_deref().unwrap().contains("permission"));
        // R and a click both restart
        w.handle_key(key(KeyCode::Char('r')));
        assert_eq!(calls.borrow().len(), 1);
        let l = wizard_view::layout(area(), &w.view());
        assert!(l.elevate.width > 0);
        w.handle_mouse(click_at(l.elevate), area());
        assert_eq!(calls.borrow().len(), 2);
        // OK once the user stops asking for it
        let (mut w, _) = ewiz(false, Ok(()));
        w.screen = Screen::Review;
        w.service = false;
        w.firewall = false;
        w.db = DbChoice::Skip;
        w.handle_key(key(KeyCode::Enter));
        assert_eq!(w.state(), WizardState::Done);
    }

    #[test]
    fn review_has_no_restart_when_elevated() {
        let mut w = wiz().with_platform(false, true);
        w.screen = Screen::Review;
        assert!(w.review_reasons().is_empty());
        assert!(!rendered(&w).contains("Restart with permission"));
        w.handle_key(key(KeyCode::Char('r')));
        assert_eq!(w.state(), WizardState::Running);
    }

    #[test]
    fn permission_buttons_are_clickable() {
        let (mut w, calls) = ewiz(false, Err(ElevateError::Declined));
        let l = wizard_view::layout(area(), &w.view());
        assert_eq!(l.options.len(), 2);
        w.handle_mouse(click_at(l.options[0]), area());
        assert_eq!(calls.borrow().len(), 1);
        assert_eq!(w.screen, Screen::Permission);
        let l = wizard_view::layout(area(), &w.view()); // the message pushed the buttons down
        w.handle_mouse(click_at(l.options[1]), area());
        assert_eq!(w.screen, Screen::Welcome);
        assert!(w.limited);
        let (mut w, _) = ewiz(true, Ok(()));
        let l = wizard_view::layout(area(), &w.view());
        w.handle_mouse(click_at(l.options[1]), area()); // [ Quit ]
        assert_eq!(w.state(), WizardState::Quit);
    }

    #[test]
    fn help_mentions_the_permission() {
        let (w, _) = ewiz(true, Ok(()));
        assert!(w.view().help.contains("administrator"));
        let (w, _) = ewiz(false, Ok(()));
        assert!(w.view().help.contains("limited") || w.view().help.contains("Not now"));
        let mut w = wiz();
        w.screen = Screen::Review;
        let _ = w.view().help; // still renders
    }

    // ---- Cfx.re account screen ----

    use std::sync::Mutex;

    /// A fake forum: answers by name and records every lookup.
    fn cfx_wiz(calls: &Arc<Mutex<Vec<String>>>) -> Wizard {
        let calls = calls.clone();
        wiz().with_cfx_lookup(Arc::new(move |n: &str| {
            calls.lock().unwrap().push(n.to_string());
            match n.to_ascii_lowercase().as_str() {
                "jsixty" => Ok(kora_core::cfx::CfxAccount { id: 4465268, username: "JSixty".into() }),
                "offline" => Err("Could not reach forum.cfx.re. Check the internet connection and try again.".into()),
                _ => Err(format!("There is no Cfx.re account called '{n}'.")),
            }
        }))
    }

    fn at_cfx(w: &mut Wizard) {
        w.screen = Screen::Cfx;
        w.load_selection();
    }

    fn type_text(w: &mut Wizard, s: &str) {
        for c in s.chars() {
            w.handle_key(key(KeyCode::Char(c)));
        }
    }

    /// Enter, then wait for the background lookup like the runner's tick loop does.
    fn enter_and_wait(w: &mut Wizard) {
        w.handle_key(key(KeyCode::Enter));
        for _ in 0..500 {
            if !w.cfx_checking() {
                return;
            }
            w.tick();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("lookup did not finish");
    }

    #[test]
    fn the_cfx_screen_comes_after_the_licence_key_and_before_review() {
        let flow = wiz().flow();
        let at = flow.iter().position(|s| *s == Screen::Cfx).unwrap();
        assert_eq!((flow[at - 1], flow[at + 1]), (Screen::License, Screen::Review));
        let mut w = wiz();
        w.advanced = true;
        w.mode = Mode::Development;
        let f = w.flow();
        let at = f.iter().position(|s| *s == Screen::Cfx).unwrap();
        assert_eq!(f[at - 1], Screen::License);
        assert_eq!(&f[at + 1..at + 3], &[Screen::Hostname, Screen::GithubOwner]);
    }

    #[test]
    fn a_found_account_goes_into_the_request_and_onto_the_review_screen() {
        let calls = Arc::new(Mutex::new(vec![]));
        let mut w = cfx_wiz(&calls);
        at_cfx(&mut w);
        type_text(&mut w, "@jsixty");
        w.handle_key(key(KeyCode::Enter));
        assert!(w.cfx_checking(), "the lookup runs in the background");
        assert_eq!(w.screen, Screen::Cfx);
        for _ in 0..500 {
            if w.screen != Screen::Cfx {
                break;
            }
            w.tick();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(w.screen, Screen::Review);
        assert!(w.error.is_none());
        assert_eq!(w.status.as_deref(), Some("Found JSixty (fivem:4465268)"));
        assert_eq!(w.build_request().cfx_identifier.as_deref(), Some("fivem:4465268"));
        assert_eq!(calls.lock().unwrap().as_slice(), ["jsixty"]);
        assert!(w.view().lines.join("\n").contains("Admin: JSixty (fivem:4465268)"));
        assert!(w.system_changes().iter().any(|c| c.contains("forum.cfx.re")));
    }

    #[test]
    fn not_found_and_network_errors_keep_you_on_the_screen_with_the_message() {
        let calls = Arc::new(Mutex::new(vec![]));
        let mut w = cfx_wiz(&calls);
        at_cfx(&mut w);
        type_text(&mut w, "nobodyhere");
        enter_and_wait(&mut w);
        assert_eq!(w.screen, Screen::Cfx);
        assert!(w.error.as_deref().unwrap().contains("no Cfx.re account called 'nobodyhere'"));
        assert_eq!(w.build_request().cfx_identifier, None);
        // Retry with another name (typing clears the message), then a network failure.
        w.clear_field();
        type_text(&mut w, "offline");
        assert!(w.error.is_none());
        enter_and_wait(&mut w);
        assert_eq!(w.screen, Screen::Cfx);
        assert!(w.error.as_deref().unwrap().contains("Could not reach forum.cfx.re"));
        assert_eq!(calls.lock().unwrap().len(), 2);
        // And it can still be skipped.
        w.clear_field();
        enter_and_wait(&mut w);
        assert_eq!(w.screen, Screen::Review);
        assert_eq!(w.build_request().cfx_identifier, None);
    }

    #[test]
    fn empty_skips_without_any_lookup_and_review_says_generated_local_account() {
        let calls = Arc::new(Mutex::new(vec![]));
        let mut w = cfx_wiz(&calls);
        at_cfx(&mut w);
        w.handle_key(key(KeyCode::Enter));
        assert_eq!(w.screen, Screen::Review);
        assert!(calls.lock().unwrap().is_empty());
        assert_eq!(w.build_request().cfx_identifier, None);
        assert!(w.view().lines.join("\n").contains("Admin: generated local account"));
    }

    #[test]
    fn invalid_characters_are_rejected_before_any_lookup() {
        let calls = Arc::new(Mutex::new(vec![]));
        let mut w = cfx_wiz(&calls);
        at_cfx(&mut w);
        for bad in ["ab", "a b c", "../etc", "x?y=1", "näme", "way-too-long-name-for-cfx-account"] {
            w.clear_field();
            type_text(&mut w, bad);
            w.handle_key(key(KeyCode::Enter));
            assert_eq!(w.screen, Screen::Cfx, "{bad}");
            assert!(!w.cfx_checking(), "{bad}");
            assert!(w.error.as_deref().unwrap().contains("3 to 20"), "{bad}");
        }
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn going_back_keeps_the_typed_name_and_the_found_account() {
        let calls = Arc::new(Mutex::new(vec![]));
        let mut w = cfx_wiz(&calls);
        at_cfx(&mut w);
        type_text(&mut w, "jsixty");
        enter_and_wait(&mut w);
        assert_eq!(w.screen, Screen::Review);
        w.go_back();
        assert_eq!(w.screen, Screen::Cfx);
        assert_eq!(w.cfx_name, "jsixty");
        w.handle_key(key(KeyCode::Enter)); // already found: no second lookup
        assert_eq!(w.screen, Screen::Review);
        assert_eq!(calls.lock().unwrap().len(), 1);
        assert_eq!(w.build_request().cfx_identifier.as_deref(), Some("fivem:4465268"));
        // Editing the name forgets the old account.
        w.go_back();
        w.handle_key(key(KeyCode::Backspace));
        assert_eq!(w.build_request().cfx_identifier, None);
        // Back from the screen itself goes to the licence key and keeps the text.
        w.go_back();
        assert_eq!(w.screen, Screen::License);
        w.go_next();
        assert_eq!(w.cfx_name, "jsixt");
    }
}
