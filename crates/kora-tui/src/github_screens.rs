//! Development mode: sign in to GitHub and choose where the forks go (your own account or one of
//! your organisations). All GitHub access goes through the [`GithubApi`] trait, so the screens are
//! tested with fakes; the state machine ([`GithubFlow`]) is pure apart from short worker threads.
//!
//! The wizard shows ONE screen (`Screen::GithubAccount`) that moves through phases: checking the
//! sign-in, the one-time code, the owner list, waiting for a new organisation, or typing the name
//! by hand when the list is not available.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    layout::{Alignment, Position, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::clipboard::{sanitize_paste, Clipboard, SystemClipboard};
use crate::open::{OpenCommand, OsKind};
use crate::wizard::{Screen, Wizard};
use crate::wizard_view::{wrap, BRAND};

pub use kora_core::git::owners::{GhAuth, Owner, OwnerKind};

const SPINNER: [&str; 4] = ["|", "/", "-", "\\"];
const LINK: Style = Style::new().fg(Color::Black).bg(Color::Gray);

// ---------------------------------------------------------------------------------------------
// What the screens need from GitHub (injected)
// ---------------------------------------------------------------------------------------------

/// A running `gh` sign-in. `done` yields once when the user approved (or it failed).
pub struct LoginHandle {
    pub code: String,
    pub url: String,
    pub done: Receiver<Result<GhAuth, String>>,
    pub cancel: Box<dyn FnMut() + Send>,
}

/// Background watch for a newly created organisation. `found` yields `Some(login)` when it
/// appeared or `None` when the wait timed out; setting `stop` ends the watch early.
pub struct OrgWatch {
    pub found: Receiver<Option<String>>,
    pub stop: Arc<AtomicBool>,
}

pub trait GithubApi: Send + Sync {
    fn auth(&self) -> GhAuth;
    /// Plain-language error text on failure (offline, rate limited, ...).
    fn list_owners(&self) -> Result<Vec<Owner>, String>;
    fn begin_login(&self) -> Result<LoginHandle, String>;
    fn org_creation_url(&self) -> String;
    /// `before` = logins of the organisations that existed before the user went to create one.
    fn watch_new_org(&self, before: Vec<String>) -> OrgWatch;
}

/// Opens a web address in the default browser (fake-able in tests).
pub trait UrlOpener {
    fn open_url(&mut self, url: &str) -> Result<(), String>;
}

/// Command that opens `url`. Only plain https addresses are accepted, and the address is always
/// ONE argument (never through a shell).
pub fn url_command(os: OsKind, url: &str) -> Result<OpenCommand, String> {
    if !url.starts_with("https://") || url.chars().any(|c| c.is_control() || c == '"' || c == ' ') {
        return Err(format!("Only https web addresses are opened. Open this yourself: {url}"));
    }
    let program = match os {
        OsKind::Windows => "explorer.exe",
        OsKind::Mac => "open",
        OsKind::Other => "xdg-open",
    };
    Ok(OpenCommand { program: program.into(), args: vec![url.into()] })
}

#[derive(Default)]
pub struct SystemUrlOpener;

impl UrlOpener for SystemUrlOpener {
    fn open_url(&mut self, url: &str) -> Result<(), String> {
        let cmd = url_command(OsKind::current(), url)?;
        let child = std::process::Command::new(&cmd.program)
            .args(&cmd.args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        match child {
            Ok(mut c) => {
                std::thread::spawn(move || {
                    let _ = c.wait();
                });
                Ok(())
            }
            Err(_) => Err(format!("Couldn't open the browser. Open this yourself: {url}")),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The real GitHub (through `gh`)
// ---------------------------------------------------------------------------------------------

/// Plain-language text for a failed GitHub call.
pub fn plain_error(e: &kora_core::git::GitError) -> String {
    use kora_core::git::GitError;
    match e {
        GitError::Spawn(..) => "Kora couldn't run GitHub's command line tool (gh).".into(),
        GitError::Command { stderr, .. } => {
            let low = stderr.to_ascii_lowercase();
            if kora_core::git::is_rate_limited(stderr) {
                "GitHub is limiting requests right now. Wait a minute and try again.".into()
            } else if ["could not resolve", "dial tcp", "connection", "timed out", "network", "offline"].iter().any(|k| low.contains(k)) {
                "Kora couldn't reach GitHub. Check your internet connection.".into()
            } else if low.contains("401") || low.contains("bad credentials") || low.contains("auth login") {
                "GitHub says you are not signed in any more. Press Try again to sign in.".into()
            } else {
                let first = stderr.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("something went wrong");
                format!("GitHub said: {first}")
            }
        }
        other => format!("Something went wrong: {other}"),
    }
}

/// A runner that stops answering once `stop` is set (ends the organisation watch quickly).
struct StoppableRunner(Arc<AtomicBool>);

impl kora_core::git::CommandRunner for StoppableRunner {
    fn run(&self, program: &str, args: &[&str], cwd: Option<&Path>) -> std::io::Result<kora_core::git::Output> {
        if self.0.load(Ordering::SeqCst) {
            return Err(std::io::Error::other("stopped"));
        }
        kora_core::git::SystemRunner.run(program, args, cwd)
    }
    fn sleep(&self, d: std::time::Duration) {
        kora_core::git::SystemRunner.sleep(d);
    }
}

/// The real thing: `gh` on this computer.
#[derive(Default)]
pub struct SystemGithub;

impl GithubApi for SystemGithub {
    fn auth(&self) -> GhAuth {
        kora_core::git::owners::gh_auth(&kora_core::git::SystemRunner)
    }

    fn list_owners(&self) -> Result<Vec<Owner>, String> {
        kora_core::git::owners::list_owners(&kora_core::git::SystemRunner).map_err(|e| plain_error(&e))
    }

    fn begin_login(&self) -> Result<LoginHandle, String> {
        let session = kora_core::git::owners::begin_login(&kora_core::git::owners::SystemLogin, false).map_err(|e| plain_error(&e))?;
        let (code, url) = (session.code.clone(), session.url.clone());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(session.wait().map_err(|e| plain_error(&e)));
        });
        // `gh` can only be waited for (not stopped) from here: a cancelled sign-in just stops being
        // listened to, and the one-time code expires on its own.
        Ok(LoginHandle { code, url, done: rx, cancel: Box::new(|| {}) })
    }

    fn org_creation_url(&self) -> String {
        kora_core::git::owners::org_creation_url().to_string()
    }

    fn watch_new_org(&self, before: Vec<String>) -> OrgWatch {
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        std::thread::spawn(move || {
            let runner = StoppableRunner(flag.clone());
            let nap = |d: std::time::Duration| {
                let end = std::time::Instant::now() + d;
                while std::time::Instant::now() < end && !flag.load(Ordering::SeqCst) {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            };
            let found = kora_core::git::owners::wait_for_new_org(&before, std::time::Duration::from_secs(600), &runner, &nap);
            let _ = tx.send(found);
        });
        OrgWatch { found: rx, stop }
    }
}

// ---------------------------------------------------------------------------------------------
// Remembered choice
// ---------------------------------------------------------------------------------------------

/// A GitHub user or organisation name: letters, digits and single dashes, at most 39 characters.
pub fn valid_owner(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty()
        && s.len() <= 39
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        && !s.starts_with('-')
        && !s.ends_with('-')
}

pub fn load_choice(path: &Path) -> Option<String> {
    let t = std::fs::read_to_string(path).ok()?;
    let t = t.lines().next()?.trim().to_string();
    valid_owner(&t).then_some(t)
}

pub fn save_choice(path: &Path, owner: &str) {
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(path, format!("{owner}\n"));
}

pub fn choice_path() -> PathBuf {
    kora_core::installer::registry::kora_home().join("github-owner.txt")
}

// ---------------------------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// A short background job is running (checking sign-in, loading the list).
    Loading(String),
    /// One-time code on screen; waiting for the user to approve in the browser.
    SignIn { code: String, url: String },
    Picker,
    /// The user went to create an organisation; waiting for it to show up.
    OrgWait,
    /// Typing the name by hand (no list available, cancelled sign-in, GitHub CLI missing).
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Btn {
    Refresh,
    CreateOrg,
    TypeName,
    Retry,
    Copy,
    OpenGithub,
    Cancel,
    Done,
    Paste,
}

impl Btn {
    fn label(self) -> &'static str {
        match self {
            Btn::Refresh => "[ Refresh ]",
            Btn::CreateOrg => "[ Create a new organisation... ]",
            Btn::TypeName => "[ Type a name instead ]",
            Btn::Retry => "[ Try again ]",
            Btn::Copy => "[ Copy code ]",
            Btn::OpenGithub => "[ Open GitHub ]",
            Btn::Cancel => "[ Cancel ]",
            Btn::Done => "[ I'm done ]",
            Btn::Paste => "[ Paste ]",
        }
    }
}

/// What the wizard should do after a key or click.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Stay,
    Next,
    Back,
    Quit,
    Help,
}

enum Job {
    Auth(GhAuth),
    Login(Result<LoginHandle, String>),
    Owners(Result<Vec<Owner>, String>),
}

pub struct GithubFlow {
    api: Arc<dyn GithubApi>,
    opener: Box<dyn UrlOpener>,
    clipboard: Box<dyn Clipboard>,
    store: Option<PathBuf>,
    pub phase: Phase,
    pub owners: Vec<Owner>,
    /// Highlighted (= chosen) entry of `owners`.
    pub cursor: usize,
    /// Typed name for the manual box.
    pub manual: String,
    /// Plain explanation shown above the box in the manual phase.
    pub note: Option<String>,
    pub signed_in: Option<String>,
    /// Short message for the wizard footer (copied, couldn't open, ...). Taken by the wizard.
    pub status: Option<String>,
    pub tick: usize,
    /// Owner to preselect (remembered choice or what the website suggested).
    prefer: Option<String>,
    started: bool,
    job: Option<Receiver<Job>>,
    login_cancel: Option<Box<dyn FnMut() + Send>>,
    login_done: Option<Receiver<Result<GhAuth, String>>>,
    watch: Option<OrgWatch>,
    org_before: Vec<String>,
}

fn spawn_job(f: impl FnOnce() -> Job + Send + 'static) -> Receiver<Job> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx
}

impl GithubFlow {
    pub fn new(api: Arc<dyn GithubApi>) -> Self {
        Self {
            api,
            opener: Box::<SystemUrlOpener>::default(),
            clipboard: Box::<SystemClipboard>::default(),
            store: Some(choice_path()),
            phase: Phase::Loading(String::new()),
            owners: vec![],
            cursor: 0,
            manual: String::new(),
            note: None,
            signed_in: None,
            status: None,
            tick: 0,
            prefer: None,
            started: false,
            job: None,
            login_cancel: None,
            login_done: None,
            watch: None,
            org_before: vec![],
        }
    }

    pub fn with_opener(mut self, o: Box<dyn UrlOpener>) -> Self {
        self.opener = o;
        self
    }

    pub fn with_clipboard(mut self, c: Box<dyn Clipboard>) -> Self {
        self.clipboard = c;
        self
    }

    /// Where the last choice is remembered (`None` = not at all).
    pub fn with_store(mut self, p: Option<PathBuf>) -> Self {
        self.store = p;
        self
    }

    /// Called whenever the screen is entered. `hint` is an owner suggested elsewhere (website).
    pub fn enter(&mut self, hint: &str) {
        if self.started && matches!(self.phase, Phase::Picker | Phase::Manual) {
            return;
        }
        self.started = true;
        self.prefer = self.store.as_deref().and_then(load_choice).or_else(|| valid_owner(hint).then(|| hint.trim().to_string()));
        self.start_check();
    }

    fn start_check(&mut self) {
        self.stop_all();
        self.note = None;
        self.phase = Phase::Loading("Checking your GitHub sign-in...".into());
        let api = self.api.clone();
        self.job = Some(spawn_job(move || Job::Auth(api.auth())));
    }

    fn start_owners(&mut self) {
        self.phase = Phase::Loading("Loading your GitHub accounts...".into());
        let api = self.api.clone();
        self.job = Some(spawn_job(move || Job::Owners(api.list_owners())));
    }

    fn start_login(&mut self) {
        self.phase = Phase::Loading("Asking GitHub for a sign-in code...".into());
        let api = self.api.clone();
        self.job = Some(spawn_job(move || Job::Login(api.begin_login())));
    }

    fn stop_all(&mut self) {
        self.job = None;
        if let Some(mut c) = self.login_cancel.take() {
            c();
        }
        self.login_done = None;
        if let Some(w) = self.watch.take() {
            w.stop.store(true, Ordering::SeqCst);
        }
    }

    /// Leaving the screen with Back: nothing keeps running.
    pub fn leave(&mut self) {
        self.stop_all();
        if !matches!(self.phase, Phase::Picker | Phase::Manual) {
            self.phase = Phase::Manual;
            self.started = false;
        }
    }

    /// True while something runs in the background (the wizard then redraws a few times a second).
    pub fn busy(&self) -> bool {
        self.job.is_some() || self.login_done.is_some() || self.watch.is_some()
    }

    /// Poll the background work. Returns true when something changed.
    pub fn tick(&mut self) -> bool {
        self.tick = self.tick.wrapping_add(1);
        let mut changed = false;
        if let Some(rx) = &self.job {
            match rx.try_recv() {
                Ok(j) => {
                    self.job = None;
                    self.on_job(j);
                    changed = true;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.job = None;
                    self.show_manual("Something went wrong while talking to GitHub.");
                    changed = true;
                }
            }
        }
        if let Some(rx) = &self.login_done {
            match rx.try_recv() {
                Ok(r) => {
                    self.login_done = None;
                    self.login_cancel = None;
                    changed = true;
                    match r {
                        Ok(GhAuth::LoggedIn { login }) => {
                            self.signed_in = Some(login);
                            self.status = Some(format!("Signed in as {}", self.signed_in.clone().unwrap_or_default()));
                            self.start_owners();
                        }
                        Ok(_) => self.show_manual("GitHub did not finish the sign-in. Try again, or type the name below."),
                        Err(e) => self.show_manual(&format!("Sign-in did not work: {e}")),
                    }
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.login_done = None;
                    self.login_cancel = None;
                    self.show_manual("Sign-in stopped unexpectedly. Try again, or type the name below.");
                    changed = true;
                }
            }
        }
        if let Some(w) = &self.watch {
            match w.found.try_recv() {
                Ok(found) => {
                    self.watch = None;
                    changed = true;
                    match found {
                        Some(login) => {
                            self.prefer = Some(login);
                            self.status = Some("Found your new organisation".into());
                        }
                        None => self.status = Some("No new organisation yet. Press Refresh once you have created it.".into()),
                    }
                    self.start_owners();
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.watch = None;
                    self.start_owners();
                    changed = true;
                }
            }
        }
        changed
    }

    fn on_job(&mut self, j: Job) {
        match j {
            Job::Auth(GhAuth::Missing) => {
                self.show_manual(
                    "GitHub's command line tool (gh) is not installed yet. Kora installs it during setup; you can then sign in the next time you run Kora. For now, type the name of the GitHub user or organisation that should hold your copies.",
                );
            }
            Job::Auth(GhAuth::LoggedOut) => self.start_login(),
            Job::Auth(GhAuth::LoggedIn { login }) => {
                self.signed_in = Some(login);
                self.start_owners();
            }
            Job::Login(Ok(h)) => {
                self.phase = Phase::SignIn { code: h.code, url: h.url };
                self.login_done = Some(h.done);
                self.login_cancel = Some(h.cancel);
            }
            Job::Login(Err(e)) => self.show_manual(&format!("Couldn't start the GitHub sign-in: {e}")),
            Job::Owners(Ok(list)) => self.set_owners(list),
            Job::Owners(Err(e)) => {
                self.show_manual(&format!("Couldn't load your GitHub accounts: {e} Press Try again, or type the name below."));
            }
        }
    }

    fn show_manual(&mut self, note: &str) {
        self.phase = Phase::Manual;
        self.note = Some(note.to_string());
        if self.manual.is_empty() {
            if let Some(p) = &self.prefer {
                self.manual = p.clone();
            }
        }
    }

    fn set_owners(&mut self, mut list: Vec<Owner>) {
        // Your own account first, then organisations, each group in the order given.
        list.sort_by_key(|o| o.kind != OwnerKind::User);
        let want = self.prefer.take();
        let pick = want
            .as_ref()
            .and_then(|w| list.iter().position(|o| o.can_fork && o.login.eq_ignore_ascii_case(w)))
            .or_else(|| list.iter().position(|o| o.can_fork));
        self.owners = list;
        self.note = None;
        self.cursor = pick.unwrap_or(0);
        if self.owners.is_empty() {
            self.show_manual("GitHub didn't list any account for you. Type the name of the user or organisation below.");
        } else {
            self.phase = Phase::Picker;
        }
    }

    /// The owner that Next would use, if any.
    pub fn chosen(&self) -> Option<String> {
        match self.phase {
            Phase::Picker => self.owners.get(self.cursor).filter(|o| o.can_fork).map(|o| o.login.clone()),
            Phase::Manual => valid_owner(&self.manual).then(|| self.manual.trim().to_string()),
            _ => None,
        }
    }

    /// Why Next can't continue yet.
    pub fn problem(&self) -> String {
        match self.phase {
            Phase::Picker => "None of these can be used. Pick another, create an organisation, or type a name.".into(),
            Phase::Manual => "Please type your GitHub user or organisation name (letters, numbers and dashes).".into(),
            Phase::SignIn { .. } => "Finish signing in in your browser first, or choose Cancel.".into(),
            Phase::OrgWait => "Finish creating the organisation first (then press I'm done), or choose Cancel.".into(),
            Phase::Loading(_) => "One moment, Kora is still talking to GitHub.".into(),
        }
    }

    /// Next was accepted: remember the choice and stop all background work.
    pub fn commit(&mut self) -> Option<String> {
        let c = self.chosen()?;
        self.stop_all();
        if let Some(p) = &self.store {
            save_choice(p, &c);
        }
        Some(c)
    }

    // ---- text shown by the generic wizard frame ----

    pub fn title(&self) -> &'static str {
        match self.phase {
            Phase::SignIn { .. } => "Sign in to GitHub",
            Phase::Loading(_) if self.signed_in.is_none() => "GitHub",
            _ => "Where should your copies (forks) of every Kora script go?",
        }
    }

    pub fn lines(&self) -> Vec<String> {
        let explain = "An organisation keeps all ~55 copies together and lets your team share them; your own account works too.";
        match &self.phase {
            Phase::SignIn { .. } => vec![
                "Kora makes your own copies of its scripts on GitHub so you can change them. For that you sign in to GitHub once, in your browser.".into(),
                "Open GitHub, type the code below when asked, and approve. Kora never sees your password.".into(),
            ],
            Phase::Picker | Phase::OrgWait | Phase::Loading(_) => {
                let mut v = vec![explain.to_string()];
                if let Some(l) = &self.signed_in {
                    v.push(format!("Signed in as {l}"));
                }
                v
            }
            Phase::Manual => {
                let mut v = vec![explain.to_string()];
                v.extend(self.note.clone());
                v
            }
        }
    }

    pub fn help(&self) -> String {
        "GitHub: Up/Down choose where your forks go, Enter next, R refresh the list, N create a new organisation, T type a name, Esc back. While signing in: C copy the code, O open GitHub, Esc cancel. In the name box: type the name, Ctrl+V paste, F5 try again, Esc back. An entry shown in grey can't be used: the reason is next to it.".into()
    }

    pub fn wants_tick(&self) -> bool {
        self.busy() || matches!(self.phase, Phase::Loading(_))
    }

    // ---- buttons ----

    fn buttons(&self) -> Vec<Btn> {
        match &self.phase {
            Phase::Loading(_) => vec![],
            Phase::SignIn { .. } => vec![Btn::Copy, Btn::OpenGithub, Btn::Cancel],
            Phase::Picker => vec![Btn::Refresh, Btn::CreateOrg, Btn::TypeName],
            Phase::OrgWait => vec![Btn::Done, Btn::OpenGithub, Btn::Cancel],
            Phase::Manual => vec![Btn::Retry, Btn::Paste],
        }
    }

    fn press(&mut self, b: Btn) -> Nav {
        match b {
            Btn::Refresh | Btn::Retry => {
                if b == Btn::Refresh {
                    self.prefer = self.chosen().or(self.prefer.take());
                    self.start_owners();
                } else {
                    self.start_check();
                }
            }
            Btn::CreateOrg => self.create_org(),
            Btn::TypeName => {
                let current = self.chosen();
                self.stop_all();
                self.phase = Phase::Manual;
                self.note = Some("Type the GitHub user or organisation name.".into());
                if self.manual.is_empty() {
                    self.manual = current.unwrap_or_default();
                }
            }
            Btn::Copy => {
                if let Phase::SignIn { code, .. } = &self.phase {
                    let code = code.clone();
                    self.status = Some(match self.clipboard.set_text(&code) {
                        Ok(()) => "Code copied. Paste it on GitHub.".to_string(),
                        Err(e) => e,
                    });
                }
            }
            Btn::OpenGithub => {
                let url = match &self.phase {
                    Phase::SignIn { url, .. } => Some(url.clone()),
                    Phase::OrgWait => Some(self.api.org_creation_url()),
                    _ => None,
                };
                if let Some(u) = url {
                    if let Err(e) = self.opener.open_url(&u) {
                        self.status = Some(e);
                    }
                }
            }
            Btn::Cancel => match self.phase {
                Phase::SignIn { .. } => {
                    self.stop_all();
                    self.show_manual("Sign-in cancelled. Press Try again to sign in, or type the name of the GitHub user or organisation below.");
                }
                _ => {
                    self.stop_all();
                    self.start_owners();
                }
            },
            Btn::Done => {
                self.stop_all();
                self.start_owners();
            }
            Btn::Paste => match self.clipboard.get_text() {
                Ok(t) => self.paste(&t),
                Err(e) => self.status = Some(e),
            },
        }
        Nav::Stay
    }

    fn create_org(&mut self) {
        self.org_before = self.owners.iter().filter(|o| o.kind == OwnerKind::Org).map(|o| o.login.clone()).collect();
        let url = self.api.org_creation_url();
        if let Err(e) = self.opener.open_url(&url) {
            self.status = Some(e);
        }
        self.stop_all();
        self.watch = Some(self.api.watch_new_org(self.org_before.clone()));
        self.phase = Phase::OrgWait;
    }

    pub fn paste(&mut self, text: &str) {
        if self.phase != Phase::Manual {
            return;
        }
        let (clean, _) = sanitize_paste(text, 39usize.saturating_sub(self.manual.chars().count()));
        self.status = Some(if clean.is_empty() { "Nothing to paste.".to_string() } else { format!("Pasted {} characters", clean.chars().count()) });
        self.manual.push_str(&clean);
    }

    // ---- input ----

    fn step(&mut self, down: bool) {
        let n = self.owners.len();
        let mut i = self.cursor;
        loop {
            i = if down { i + 1 } else { i.wrapping_sub(1) };
            if i >= n {
                return;
            }
            if self.owners[i].can_fork {
                self.cursor = i;
                return;
            }
        }
    }

    pub fn key(&mut self, k: KeyEvent) -> Nav {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        self.status = None;
        match (&self.phase, k.code) {
            (_, KeyCode::F(1)) => return Nav::Help,
            (Phase::Loading(_), KeyCode::Esc) => return Nav::Back,
            (Phase::SignIn { .. }, KeyCode::Char('c' | 'C')) => return self.press(Btn::Copy),
            (Phase::SignIn { .. } | Phase::OrgWait, KeyCode::Char('o' | 'O')) => return self.press(Btn::OpenGithub),
            (Phase::SignIn { .. } | Phase::OrgWait, KeyCode::Esc) => return self.press(Btn::Cancel),
            (Phase::OrgWait, KeyCode::Enter | KeyCode::Char('d' | 'D')) => return self.press(Btn::Done),
            (Phase::Picker, KeyCode::Up) => self.step(false),
            (Phase::Picker, KeyCode::Down) => self.step(true),
            (Phase::Picker, KeyCode::Home) => self.cursor = self.owners.iter().position(|o| o.can_fork).unwrap_or(self.cursor),
            (Phase::Picker, KeyCode::End) => self.cursor = self.owners.iter().rposition(|o| o.can_fork).unwrap_or(self.cursor),
            (Phase::Picker, KeyCode::Char('r' | 'R') | KeyCode::F(5)) => return self.press(Btn::Refresh),
            (Phase::Picker, KeyCode::Char('n' | 'N')) => return self.press(Btn::CreateOrg),
            (Phase::Picker, KeyCode::Char('t' | 'T')) => return self.press(Btn::TypeName),
            (Phase::Picker, KeyCode::Char('q')) => return Nav::Quit,
            (Phase::Picker, KeyCode::Enter | KeyCode::Tab) => return Nav::Next,
            (Phase::Picker, KeyCode::Esc | KeyCode::Backspace | KeyCode::BackTab | KeyCode::Char('b')) => return Nav::Back,
            (Phase::Manual, KeyCode::F(5)) => return self.press(Btn::Retry),
            (Phase::Manual, KeyCode::Char('v' | 'V')) if ctrl => return self.press(Btn::Paste),
            (Phase::Manual, KeyCode::Char('u')) if ctrl => self.manual.clear(),
            (Phase::Manual, KeyCode::Char(c)) if !ctrl => {
                if self.manual.chars().count() < 39 && !c.is_whitespace() {
                    self.manual.push(c);
                }
            }
            (Phase::Manual, KeyCode::Backspace) => {
                self.manual.pop();
            }
            (Phase::Manual, KeyCode::Enter | KeyCode::Tab) => return Nav::Next,
            (Phase::Manual, KeyCode::Esc | KeyCode::BackTab) => return Nav::Back,
            _ => {}
        }
        Nav::Stay
    }

    pub fn click(&mut self, ev: MouseEvent, region: Rect) -> Nav {
        if ev.kind != MouseEventKind::Down(MouseButton::Left) {
            return Nav::Stay;
        }
        self.status = None;
        let pos = Position::new(ev.column, ev.row);
        let l = self.layout(region);
        if let Some((b, _)) = l.buttons.iter().find(|(_, r)| r.contains(pos)) {
            return self.press(*b);
        }
        if let Some((i, _)) = l.entries.iter().find(|(_, r)| r.contains(pos)) {
            match self.owners.get(*i) {
                Some(o) if o.can_fork => self.cursor = *i,
                Some(o) => self.status = Some(o.reason.clone().unwrap_or_else(|| format!("{} can't be used.", o.login))),
                None => {}
            }
        }
        Nav::Stay
    }

    // ---- layout (shared by drawing and mouse hit-testing) ----

    pub fn layout(&self, region: Rect) -> GhLayout {
        let labels = self.buttons();
        let mut buttons = vec![];
        let mut x = region.x;
        // Flow the buttons into rows first, then anchor the block to the bottom of the region.
        let mut rows = 0u16;
        let mut placed: Vec<(Btn, u16, u16)> = vec![];
        for b in &labels {
            let w = b.label().chars().count() as u16;
            if x > region.x && x + w > region.right() {
                x = region.x;
                rows += 1;
            }
            placed.push((*b, x - region.x, rows));
            x += w + 2;
        }
        let nrows = if labels.is_empty() { 0 } else { rows + 1 };
        let top_of_buttons = region.bottom().saturating_sub(nrows);
        for (b, dx, r) in placed {
            let y = top_of_buttons + r;
            let w = b.label().chars().count() as u16;
            buttons.push((b, Rect::new(region.x + dx, y, w, 1).intersection(region)));
        }
        let area = Rect::new(region.x, region.y, region.width, top_of_buttons.saturating_sub(region.y + u16::from(nrows > 0)));
        let mut entries = vec![];
        if self.phase == Phase::Picker {
            let h = |o: &Owner| if o.can_fork { 1u16 } else { 2 };
            let mut top = 0;
            while top < self.cursor && (top..=self.cursor).map(|i| self.owners.get(i).map_or(1, h)).sum::<u16>() > area.height {
                top += 1;
            }
            let mut ey = area.y;
            for (i, o) in self.owners.iter().enumerate().skip(top) {
                if ey >= area.bottom() {
                    break;
                }
                let eh = h(o).min(area.bottom() - ey);
                entries.push((i, Rect::new(area.x, ey, area.width, eh)));
                ey += h(o);
            }
        }
        GhLayout { buttons, entries, area }
    }
}

/// Part of the wizard body below the generic text, where this screen draws its own content.
pub fn region(l: &crate::wizard_view::Layout) -> Rect {
    let inner = Rect::new(l.body.x + 1, l.body.y + 1, l.body.width.saturating_sub(2), l.body.height.saturating_sub(2));
    let y = l.text.bottom().max(inner.y) + 1;
    Rect::new(inner.x, y, inner.width, inner.bottom().saturating_sub(y))
}

pub struct GhLayout {
    pub buttons: Vec<(Btn, Rect)>,
    /// (index into `owners`, row)
    pub entries: Vec<(usize, Rect)>,
    /// The part of the region above the buttons.
    pub area: Rect,
}

// ---------------------------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------------------------

/// Label of one list entry, e.g. `octo (your account)`.
pub fn entry_label(o: &Owner) -> String {
    let what = match o.kind {
        OwnerKind::User => "your account".to_string(),
        OwnerKind::Org => match &o.role {
            Some(r) => format!("organisation, {r}"),
            None => "organisation".to_string(),
        },
    };
    format!("{} ({what})", o.login)
}

pub fn render(f: &mut Frame, flow: &GithubFlow, region: Rect) {
    if region.width < 4 || region.height == 0 {
        return;
    }
    let l = flow.layout(region);
    let spin = SPINNER[flow.tick % SPINNER.len()];
    let area = l.area;
    match &flow.phase {
        Phase::Loading(msg) => {
            f.render_widget(Paragraph::new(format!("{spin} {msg}")), area);
        }
        Phase::SignIn { code, url } => {
            let box_h = 5.min(area.height);
            let r = Rect::new(area.x, area.y, area.width, box_h);
            f.render_widget(
                Paragraph::new(vec![
                    Line::from(""),
                    Line::styled(code.clone(), Style::default().fg(BRAND).add_modifier(Modifier::BOLD | Modifier::REVERSED)),
                ])
                .alignment(Alignment::Center)
                .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(BRAND)).title(" Your one-time code ")),
                r,
            );
            let rest = Rect::new(area.x, area.y + box_h, area.width, area.height.saturating_sub(box_h));
            let mut lines: Vec<Line> = wrap(&format!("Go to {url} and type the code."), rest.width as usize).into_iter().map(Line::from).collect();
            lines.push(Line::from(""));
            lines.push(Line::styled(format!("{spin} Waiting for you to approve in the browser..."), Style::default().fg(Color::Yellow)));
            f.render_widget(Paragraph::new(lines), rest);
        }
        Phase::Picker => {
            for (i, r) in &l.entries {
                let o = &flow.owners[*i];
                let label = entry_label(o);
                if o.can_fork {
                    let sel = *i == flow.cursor;
                    let st = if sel { Style::default().fg(BRAND).add_modifier(Modifier::BOLD) } else { Style::default() };
                    f.render_widget(Paragraph::new(format!("{} {label}", if sel { "(•)" } else { "( )" })).style(st), *r);
                } else {
                    let why = o.reason.clone().unwrap_or_else(|| "Can't be used for your copies.".into());
                    let lines = vec![Line::from(format!("    {label}")), Line::from(format!("      {why}"))];
                    f.render_widget(Paragraph::new(lines).style(Style::default().fg(Color::DarkGray)), *r);
                }
            }
        }
        Phase::OrgWait => {
            let lines = vec![
                Line::from("GitHub is open in your browser. Create your organisation there (the free plan is fine), then come back."),
                Line::from(""),
                Line::styled(format!("{spin} Waiting for your new organisation..."), Style::default().fg(Color::Yellow)),
                Line::from(""),
                Line::from("Kora picks it up by itself. If it doesn't, press I'm done."),
            ];
            f.render_widget(Paragraph::new(lines).wrap(ratatui::widgets::Wrap { trim: false }), area);
        }
        Phase::Manual => {
            let box_r = Rect::new(area.x, area.y, area.width.min(60), 3.min(area.height));
            let content = if flow.manual.is_empty() {
                Line::styled("e.g. my-team", Style::default().fg(Color::DarkGray))
            } else {
                Line::from(format!("{}▏", flow.manual))
            };
            f.render_widget(
                Paragraph::new(content).block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(BRAND)).title(" GitHub name ")),
                box_r,
            );
        }
    }
    for (b, r) in &l.buttons {
        f.render_widget(Paragraph::new(b.label()).style(LINK), *r);
    }
}

// ---------------------------------------------------------------------------------------------
// Wizard hooks
// ---------------------------------------------------------------------------------------------

impl Wizard {
    /// Offer the GitHub sign-in and owner screens in Development mode.
    pub fn with_github(mut self, flow: GithubFlow) -> Self {
        self.gh = Some(flow);
        self
    }

    pub(crate) fn gh_on_screen(&self) -> bool {
        self.screen == Screen::GithubAccount && self.gh.is_some()
    }

    pub(crate) fn gh_enter(&mut self) {
        let hint = self.github_owner.clone();
        if let Some(g) = self.gh.as_mut() {
            g.enter(&hint);
        }
    }

    pub(crate) fn gh_apply(&mut self) {
        if let Some(c) = self.gh.as_mut().and_then(GithubFlow::commit) {
            self.github_owner = c;
        }
    }

    pub(crate) fn gh_validate(&self) -> Result<(), String> {
        match &self.gh {
            Some(g) if g.chosen().is_none() => Err(g.problem()),
            _ => Ok(()),
        }
    }

    fn gh_nav(&mut self, nav: Nav) {
        if let Some(s) = self.gh.as_mut().and_then(|g| g.status.take()) {
            self.status = Some(s);
        }
        match nav {
            Nav::Stay => {}
            Nav::Next => self.go_next(),
            Nav::Back => self.go_back(),
            Nav::Quit => self.state = crate::wizard::WizardState::Quit,
            Nav::Help => self.help_open = true,
        }
    }

    pub(crate) fn gh_key(&mut self, k: KeyEvent) {
        let nav = self.gh.as_mut().map_or(Nav::Stay, |g| g.key(k));
        self.gh_nav(nav);
    }

    pub(crate) fn gh_click(&mut self, ev: MouseEvent, region: Rect) {
        let nav = self.gh.as_mut().map_or(Nav::Stay, |g| g.click(ev, region));
        self.gh_nav(nav);
    }

    pub(crate) fn gh_paste(&mut self, text: &str) {
        if let Some(g) = self.gh.as_mut() {
            g.paste(text);
        }
    }

    /// Poll background work (call a few times a second). Returns true when the screen changed.
    pub fn tick(&mut self) -> bool {
        let on = self.gh_on_screen();
        let changed = match (&mut self.gh, on) {
            (Some(g), true) => g.tick(),
            _ => false,
        };
        if let Some(s) = self.gh.as_mut().and_then(|g| g.status.take()) {
            self.status = Some(s);
        }
        changed
    }

    /// True when the screen animates or waits for background work (the runner polls instead of blocking).
    pub fn wants_tick(&self) -> bool {
        self.gh_on_screen() && self.gh.as_ref().is_some_and(GithubFlow::wants_tick)
    }
}

#[cfg(test)]
mod tests;
