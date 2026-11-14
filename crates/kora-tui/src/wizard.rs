//! Pure state machine for the install wizard. No terminal I/O lives here, so every
//! transition is unit-testable.
//!
//! Stages: Welcome -> Prepare -> Choose (one decision per screen) -> Review.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use kora_core::firewall::{self, PortRequest};
use kora_core::installer::{DbAdmin, DbChoice, InstallRequest, Mode, Ports, ResourceSelection};
use ratatui::layout::Rect;
use std::path::PathBuf;

use crate::wizard_view;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
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
    License,
    // Advanced only
    Hostname,
    GithubOwner,
    GameBuild,
    TxAdminPublic,
    SetupCode,
    Review,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardState {
    Running,
    Done,
    Quit,
}

/// Port checker: (game, txadmin) -> plain-language conflict messages.
pub type PortCheck = fn(u16, u16) -> Vec<String>;

fn real_port_check(game: u16, tx: u16) -> Vec<String> {
    let req = PortRequest {
        server: "kora".into(),
        game_port: game,
        txadmin_port: tx,
        expose_txadmin: false,
    };
    firewall::port_conflicts(&req).unwrap_or_default()
}

/// Real database login check (fake-able in tests). `None` = automatic (environment / root socket).
pub type DbTester = fn(Option<&DbAdmin>) -> Result<(), String>;

pub struct Wizard {
    pub screen: Screen,
    state: WizardState,
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
    pub license: String,
    pub hostname: String,
    host_touched: bool,
    pub github_owner: String,
    pub game_build: String,
    /// TODO: the website setup-code API does not exist yet; the code is only stored here.
    pub setup_code: String,
    /// Selected row for choice screens.
    pub selected: usize,
    /// Focused input for text screens.
    pub focus: usize,
    pub help_open: bool,
    pub error: Option<String>,
    port_check: PortCheck,
    db_tester: DbTester,
    /// false = automatic (root socket / no password), true = enter an admin login.
    pub dba_custom: bool,
    pub dba_user: String,
    pub dba_pass: String,
    pub dba_host: String,
    pub dba_port: String,
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
            .field("setup_code", &if self.setup_code.is_empty() { "<empty>" } else { "<redacted>" })
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
        Self::with_port_check(real_port_check)
    }

    pub fn with_port_check(port_check: PortCheck) -> Self {
        Self::with_hooks(port_check, kora_core::installer::check_db_admin)
    }

    pub fn with_hooks(port_check: PortCheck, db_tester: DbTester) -> Self {
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
            license: String::new(),
            github_owner: String::new(),
            game_build: String::new(),
            setup_code: String::new(),
            selected: 0,
            focus: 0,
            help_open: false,
            error: None,
            port_check,
            db_tester,
            dba_custom: false,
            dba_user: "root".into(),
            dba_pass: String::new(),
            dba_host: "127.0.0.1".into(),
            dba_port: "3306".into(),
            db_test: None,
        }
    }

    pub fn state(&self) -> WizardState {
        self.state
    }

    /// Screens in the current order (depends on earlier answers).
    pub fn flow(&self) -> Vec<Screen> {
        use Screen::*;
        let mut v = vec![Welcome, Prepare, Mode, Name, Folder, Ports, Artifact];
        if self.artifact_custom {
            v.push(ArtifactUrl);
        }
        v.push(Resources);
        if !self.resources_all {
            v.push(ResourceList);
        }
        v.push(Database);
        if self.db != DbChoice::Skip {
            v.push(DbAdmin);
            if self.dba_custom {
                v.push(DbAdminLogin);
            }
        }
        v.extend([Firewall, Service, License]);
        if self.advanced {
            v.push(Hostname);
            if self.mode == kora_core::installer::Mode::Development {
                v.push(GithubOwner);
            }
            v.extend([GameBuild, TxAdminPublic, SetupCode]);
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
            Screen::Welcome => 0,
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
            Screen::DbAdminLogin => 4,
            Screen::Name
            | Screen::Folder
            | Screen::ArtifactUrl
            | Screen::ResourceList
            | Screen::License
            | Screen::Hostname
            | Screen::GithubOwner
            | Screen::GameBuild
            | Screen::SetupCode => 1,
            _ => 0,
        }
    }

    pub fn option_count(&self) -> usize {
        match self.screen {
            Screen::Welcome | Screen::Mode | Screen::Artifact | Screen::Resources | Screen::DbAdmin => 2,
            Screen::Database => 3,
            Screen::Firewall | Screen::Service | Screen::TxAdminPublic => 2,
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
            (Screen::DbAdminLogin, 2) => &mut self.dba_host,
            (Screen::DbAdminLogin, _) => &mut self.dba_port,
            (Screen::ArtifactUrl, _) => &mut self.artifact_url,
            (Screen::ResourceList, _) => &mut self.resource_ids,
            (Screen::License, _) => &mut self.license,
            (Screen::Hostname, _) => &mut self.hostname,
            (Screen::GithubOwner, _) => &mut self.github_owner,
            (Screen::GameBuild, _) => &mut self.game_build,
            (Screen::SetupCode, _) => &mut self.setup_code,
            _ => return None,
        })
    }

    /// Sync `selected` from stored answers when entering a screen.
    fn load_selection(&mut self) {
        self.focus = 0;
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
            Screen::TxAdminPublic => self.txadmin_public = i == 0,
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
            Screen::Folder => self.dir_touched = true,
            Screen::Hostname => self.host_touched = true,
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

    fn validate_db_admin(&self) -> Result<(), String> {
        if !self.dba_custom {
            return Ok(());
        }
        if self.dba_user.trim().is_empty() {
            return Err("Please enter the database admin user name (often 'root').".into());
        }
        if self.dba_host.trim().is_empty() {
            return Err("Please enter the database server address (usually 127.0.0.1).".into());
        }
        match self.dba_port.trim().parse::<u16>() {
            Ok(p) if p > 0 => Ok(()),
            _ => Err("The database port must be a number such as 3306.".into()),
        }
    }

    pub fn handle_paste(&mut self, text: &str) {
        if self.help_open || !self.is_text_screen() {
            return;
        }
        for c in text.chars().filter(|c| !c.is_control()) {
            self.type_char(c);
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if self.state != WizardState::Running {
            return;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
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
        if self.on_db_admin_screen()
            && (key.code == KeyCode::F(2)
                || (ctrl && key.code == KeyCode::Char('t'))
                || (!self.is_text_screen() && key.code == KeyCode::Char('t')))
        {
            self.test_login();
            return;
        }
        if self.is_text_screen() {
            match key.code {
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
            KeyCode::Up | KeyCode::Left if n > 0 => self.select(self.selected.saturating_sub(1)),
            KeyCode::Down | KeyCode::Right if n > 0 => self.select(self.selected + 1),
            KeyCode::Char(c @ '1'..='9') if n > 0 => {
                let i = c as usize - '1' as usize;
                if i < n {
                    self.select(i);
                }
            }
            KeyCode::Char('y') if matches!(self.screen, Screen::Firewall | Screen::Service | Screen::TxAdminPublic) => self.select(0),
            KeyCode::Char('n') if matches!(self.screen, Screen::Firewall | Screen::Service | Screen::TxAdminPublic) => self.select(1),
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
        } else if let Some(i) = l.options.iter().position(|r| hit(*r)) {
            self.select(i);
        } else if let Some(i) = l.inputs.iter().position(|r| hit(*r)) {
            self.focus = i;
        }
    }

    pub fn go_back(&mut self) {
        self.error = None;
        let flow = self.flow();
        let idx = flow.iter().position(|s| *s == self.screen).unwrap_or(0);
        if idx == 0 {
            self.state = WizardState::Quit;
        } else {
            self.screen = flow[idx - 1];
            self.load_selection();
        }
    }

    /// Validate the current screen and advance. Sets `error` and stays on failure.
    pub fn go_next(&mut self) {
        if let Err(e) = self.validate_current() {
            self.error = Some(e);
            return;
        }
        self.error = None;
        self.apply_selection();
        let flow = self.flow();
        let idx = flow.iter().position(|s| *s == self.screen).unwrap_or(0);
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
                let (g, t) = validate_ports(&self.name, &self.game_port, &self.tx_port)?;
                match (self.port_check)(g, t).into_iter().next() {
                    Some(c) => Err(c),
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
            Screen::License => validate_license(&self.license),
            Screen::GameBuild => {
                let g = self.game_build.trim();
                if g.is_empty() || g.parse::<u32>().is_ok() {
                    Ok(())
                } else {
                    Err("The game build must be a number (or leave it empty).".into())
                }
            }
            Screen::SetupCode => {
                let c = self.setup_code.trim();
                if c.is_empty() || c.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                    Ok(())
                } else {
                    Err("Setup codes look like KORA-7F3K-92QX.".into())
                }
            }
            _ => Ok(()),
        }
    }

    /// Plain-language list of everything the install will change on this computer.
    pub fn system_changes(&self) -> Vec<String> {
        let mut v = vec![
            format!("Create the folder {} and put your server there", self.dir.trim()),
            "Download the FiveM server software and Kora's resources (needs internet)".to_string(),
            "Install Git if it is missing".to_string(),
        ];
        match self.db {
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
        if self.firewall {
            v.push(format!(
                "Allow ports {} and {} through the firewall (needs administrator rights)",
                self.game_port.trim(),
                self.tx_port.trim()
            ));
        }
        if self.service {
            v.push("Start the server automatically when this computer starts (needs administrator rights)".into());
        }
        if self.mode == kora_core::installer::Mode::Development {
            v.push("Copy (fork) Kora's resources to your GitHub account so you can change them".into());
        }
        if !self.license.trim().is_empty() {
            v.push("Save your license key inside the server's private settings file (it never leaves this computer)".into());
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
            cfx_identifier: None,
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

    fn wiz() -> Wizard {
        Wizard::with_port_check(|_, _| vec![])
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
    fn choices_change_flow() {
        let mut w = wiz();
        w.handle_key(key(KeyCode::Down)); // advanced
        assert!(w.advanced);
        assert!(w.flow().contains(&Screen::SetupCode));
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

        let mut w = Wizard::with_port_check(|g, _| vec![format!("Port {g} (game) is already used")]);
        w.screen = Screen::Ports;
        w.go_next();
        assert_eq!(w.screen, Screen::Ports);
        assert!(w.error.as_ref().unwrap().contains("already used"));
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
        let mut w = Wizard::with_hooks(|_, _| vec![], ok_tester);
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
        assert_eq!(w.db_test, Some(Err("Access denied".into())));
        assert_eq!(w.dba_pass, "bad", "Ctrl+T must not type a character");
        // fix the password: result is cleared, retest succeeds
        for _ in 0..3 {
            w.handle_key(key(KeyCode::Backspace));
        }
        assert!(w.db_test.is_none());
        type_str(&mut w, "pw");
        w.handle_key(key(KeyCode::F(2)));
        assert_eq!(w.db_test, Some(Ok(())));
        // the view shows the result and masks the password
        let v = w.view();
        assert!(v.inputs[1].masked && v.has_test);
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
        assert!(!w.system_changes().iter().any(|c| c.contains("firewall")));
    }
}
