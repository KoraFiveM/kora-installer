//! The old "Kora app" (retired from the install flow in 0.7.0) and the per-user shortcuts that replace it.
//!
//! Since 0.7.0 the "app" is just the web panel in a browser tab: the install scripts (and `kora app install`)
//! make per-user shortcuts that run a bare `kora` (see [`user`]): no administrator rights, no copy in
//! `%ProgramFiles%`. What is described below is the OLD machine-wide app, which this module still detects
//! and removes ("Tidy up old shortcuts" in the panel, `kora app uninstall`):
//!
//! * Windows: `%ProgramFiles%\Kora\kora.exe`, on the machine `PATH`, a Start Menu shortcut "Kora" (and
//!   optionally a Desktop shortcut) that runs `kora.exe manager` with "Run as administrator" set, and an
//!   "Apps & features" entry whose uninstaller is `kora.exe uninstall --app`.
//! * Linux: `/opt/kora/bin/kora`, a `/usr/local/bin/kora` symlink and (desktop sessions only) a
//!   `kora.desktop` launcher that opens the manager in a terminal.
//!
//! Everything is decided by a pure planner ([`plan_uninstall`], and `user::plan_user_install`) that only *reads*
//! through the [`Sys`] trait, and a small executor that applies the resulting [`Action`]s, so a dry run
//! lists every change and tests run against an in-memory fake. Uninstalling removes only what the app
//! installed: never servers, never databases.
//!
//! `KORA_APP_PREFIX=<dir>` (Unix) puts every path under `<dir>` so the real system is untouched.

pub mod lnk;
pub mod pathlist;
mod sys;
#[cfg(test)]
mod tests;
pub mod user;

use std::cmp::Ordering;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use sys::RealSys;

pub const APP_NAME: &str = "Kora";
pub const PUBLISHER: &str = "Kora";
/// Marker line in the Linux launcher so uninstall only removes a file the app wrote.
pub const DESKTOP_MARKER: &str = "X-Kora-Managed=true";
const LOGO_PNG: &[u8] = include_bytes!("../../../../assets/kora-logo-512.png");

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct AppError(pub String);

pub type Result<T> = std::result::Result<T, AppError>;

/// Wizard / flag choice carried by the install request. `None` on the request = no app step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppChoice {
    /// Also put a "Kora" shortcut on the desktop (Windows; default on).
    pub desktop_shortcut: bool,
}

impl Default for AppChoice {
    fn default() -> Self {
        Self { desktop_shortcut: true }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AppOptions {
    pub desktop_shortcut: bool,
    /// Allow replacing a newer installed copy with this (older) program.
    pub force: bool,
    /// Plan only: report every change, make none.
    pub dry_run: bool,
}

/// Which program is running right now.
#[derive(Debug, Clone)]
pub struct Running {
    pub exe: PathBuf,
    pub version: String,
    pub sha256: String,
}

impl Running {
    pub fn current() -> Result<Running> {
        let exe = std::env::current_exe().map_err(|e| AppError(format!("cannot find the kora program: {e}")))?;
        let sha256 = crate::service::binary::file_sha256(&exe).map_err(|e| AppError(format!("cannot read the kora program: {e}")))?;
        Ok(Running { exe, version: env!("CARGO_PKG_VERSION").to_string(), sha256 })
    }
}

// ---------------------------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------------------------

/// Every path the app uses, resolved once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub windows: bool,
    /// Folder holding the program (`%ProgramFiles%\Kora`, `/opt/kora/bin`).
    pub dir: PathBuf,
    pub exe: PathBuf,
    /// `app.json` next to the program: installed version and hash.
    pub state: PathBuf,
    pub start_menu: Option<PathBuf>,
    pub desktop: Option<PathBuf>,
    /// Unix: `/usr/local/bin/kora`.
    pub link: Option<PathBuf>,
    /// Unix: the launcher file, and the logo it points at.
    pub desktop_entry: Option<PathBuf>,
    pub icon_png: Option<PathBuf>,
    /// A desktop session exists (a launcher is only useful then).
    pub desktop_session: bool,
    /// The running program lives in a per-user `~/.local/bin` (install.sh): no system symlink is made.
    pub user_install: bool,
}

pub const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Kora";

pub type EnvGet<'a> = &'a dyn Fn(&str) -> Option<OsString>;

impl Layout {
    /// Resolve from the environment. `running_exe` decides the `~/.local/bin` special case.
    pub fn resolve(windows: bool, get: EnvGet, running_exe: &Path) -> Result<Layout> {
        let var = |k: &str| get(k).filter(|v| !v.is_empty()).map(PathBuf::from);
        if windows {
            let pf = var("ProgramFiles").ok_or_else(|| AppError("ProgramFiles is not set".into()))?;
            // Joined with backslashes by hand so the layout is the same whatever machine computes it (tests).
            let wjoin = |base: &Path, parts: &[&str]| PathBuf::from(format!("{}\\{}", base.to_string_lossy().trim_end_matches(['\\', '/']), parts.join("\\")));
            let dir = wjoin(&pf, &[APP_NAME]);
            let data = var("ProgramData").unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
            let public = var("PUBLIC").unwrap_or_else(|| PathBuf::from(r"C:\Users\Public"));
            let lnk = format!("{APP_NAME}.lnk");
            return Ok(Layout {
                windows,
                exe: wjoin(&dir, &["kora.exe"]),
                state: wjoin(&dir, &["app.json"]),
                start_menu: Some(wjoin(&data, &["Microsoft", "Windows", "Start Menu", "Programs", &lnk])),
                desktop: Some(wjoin(&public, &["Desktop", &lnk])),
                dir,
                link: None,
                desktop_entry: None,
                icon_png: None,
                desktop_session: false,
                user_install: false,
            });
        }
        let prefix = var("KORA_APP_PREFIX").unwrap_or_default();
        let at = |p: &str| if prefix.as_os_str().is_empty() { PathBuf::from(p) } else { prefix.join(p.trim_start_matches('/')) };
        let dir = at("/opt/kora/bin");
        let in_user_bin = running_exe.parent().is_some_and(|p| p.ends_with(".local/bin"));
        let session = ["DISPLAY", "WAYLAND_DISPLAY", "XDG_CURRENT_DESKTOP"].iter().any(|k| var(k).is_some())
            || var("XDG_SESSION_TYPE").is_some_and(|t| t == Path::new("x11") || t == Path::new("wayland"));
        Ok(Layout {
            windows,
            exe: dir.join("kora"),
            state: dir.join("app.json"),
            start_menu: None,
            desktop: None,
            link: Some(at("/usr/local/bin/kora")),
            desktop_entry: Some(at("/usr/local/share/applications/kora.desktop")),
            icon_png: Some(at("/opt/kora/share/kora-logo-512.png")),
            dir,
            desktop_session: session,
            user_install: in_user_bin,
        })
    }

    pub fn from_env(running_exe: &Path) -> Result<Layout> {
        Layout::resolve(cfg!(windows), &|k| std::env::var_os(k), running_exe)
    }

    /// The folder a user's shell should find `kora` in.
    #[cfg(test)]
    pub fn command_dir(&self) -> Option<PathBuf> {
        if self.windows {
            Some(self.dir.clone())
        } else if self.user_install {
            None
        } else {
            self.link.as_ref().and_then(|l| l.parent().map(Path::to_path_buf))
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Data
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppState {
    pub version: String,
    pub sha256: String,
    pub installed_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutSpec {
    pub path: PathBuf,
    pub target: PathBuf,
    pub args: String,
    pub workdir: PathBuf,
    /// `<exe>,0`
    pub icon: String,
    pub description: String,
    pub run_as_admin: bool,
    /// Start the program in a minimised window (a console program opened from a shortcut should not
    /// flash a big black window). Not read back from a shortcut, so it is ignored when comparing.
    pub minimized: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninstallEntry {
    pub display_name: String,
    pub display_version: String,
    pub publisher: String,
    pub install_location: String,
    pub uninstall_string: String,
    pub display_icon: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopyKind {
    Fresh,
    /// Replace a different or older program.
    Replace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    CopyExe { from: PathBuf, to: PathBuf, kind: CopyKind },
    WriteState(AppState),
    AddMachinePath(String),
    /// Windows: add a folder to the PATH of this user only (no administrator rights).
    AddUserPath(String),
    BroadcastEnv,
    Shortcut(ShortcutSpec),
    Symlink { link: PathBuf, target: PathBuf },
    WriteLogo(PathBuf),
    DesktopEntry { path: PathBuf, content: String },
    RegisterUninstall(UninstallEntry),
    // ---- removal ----
    RemoveShortcut(PathBuf),
    RemoveMachinePath(String),
    RemoveUninstall,
    RemoveSymlink(PathBuf),
    RemoveFile(PathBuf),
    RemoveDirIfEmpty(PathBuf),
}

impl Action {
    pub fn describe(&self) -> String {
        use Action::*;
        match self {
            CopyExe { from, to, kind: CopyKind::Fresh } => format!("Copy {} to {}", from.display(), to.display()),
            CopyExe { from, to, kind: CopyKind::Replace } => format!("Replace {} with {}", to.display(), from.display()),
            WriteState(s) => format!("Record installed version {} in the app state file", s.version),
            AddMachinePath(d) => format!("Add {d} to the machine PATH"),
            AddUserPath(d) => format!("Add {d} to your PATH"),
            BroadcastEnv => "Tell running programs the environment changed (new terminals see the new PATH)".into(),
            Shortcut(s) => format!(
                "Create the shortcut {} -> {} {}{}",
                s.path.display(),
                s.target.display(),
                s.args,
                if s.run_as_admin { " (always run as administrator)" } else { "" }
            ),
            Symlink { link, target } => format!("Link {} -> {}", link.display(), target.display()),
            WriteLogo(p) => format!("Install the Kora logo at {}", p.display()),
            DesktopEntry { path, .. } => format!("Write the launcher {}", path.display()),
            RegisterUninstall(e) => format!("Register \"{}\" {} in Apps & features (uninstall: {})", e.display_name, e.display_version, e.uninstall_string),
            RemoveShortcut(p) => format!("Remove the shortcut {}", p.display()),
            RemoveMachinePath(d) => format!("Remove {d} from the machine PATH"),
            RemoveUninstall => "Remove the Apps & features entry".into(),
            RemoveSymlink(p) => format!("Remove the link {}", p.display()),
            RemoveFile(p) => format!("Delete {}", p.display()),
            RemoveDirIfEmpty(p) => format!("Remove the folder {} if it is empty", p.display()),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub actions: Vec<Action>,
    pub notes: Vec<String>,
}

/// Test fixture: the result of laying down the OLD machine-wide app on a fake machine.
#[cfg(test)]
#[derive(Debug, Clone)]
pub struct AppInstall {
    pub exe: PathBuf,
    pub version: String,
    /// One plain sentence per change made (or, in a dry run, that would be made).
    pub changes: Vec<String>,
    pub notes: Vec<String>,
    pub dry_run: bool,
}

#[cfg(test)]
impl AppInstall {
    pub fn changed(&self) -> bool {
        !self.changes.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct AppStatus {
    pub installed: bool,
    pub version: Option<String>,
    pub exe: PathBuf,
    pub exe_exists: bool,
    /// Windows: the machine PATH lists the folder. Unix: the `kora` link is ours.
    pub on_path: bool,
    pub start_menu: Option<bool>,
    pub desktop: Option<bool>,
    pub uninstall_registered: bool,
    pub desktop_entry: Option<bool>,
    pub running_version: String,
    /// This program is newer than the installed copy.
    pub update_available: bool,
}

impl AppStatus {
    /// `key: value` lines (stable; the Windows smoke test reads them).
    pub fn lines(&self) -> Vec<String> {
        let yn = |b: bool| if b { "yes" } else { "no" };
        let opt = |o: Option<bool>| o.map(yn).unwrap_or("n/a");
        vec![
            format!("installed: {}", yn(self.installed)),
            format!("version: {}", self.version.as_deref().unwrap_or("-")),
            format!("program: {}", self.exe.display()),
            format!("program_present: {}", yn(self.exe_exists)),
            format!("on_path: {}", yn(self.on_path)),
            format!("start_menu_shortcut: {}", opt(self.start_menu)),
            format!("desktop_shortcut: {}", opt(self.desktop)),
            format!("apps_and_features: {}", yn(self.uninstall_registered)),
            format!("launcher: {}", opt(self.desktop_entry)),
            format!("update_available: {}", yn(self.update_available)),
        ]
    }
}

// ---------------------------------------------------------------------------------------------
// System access
// ---------------------------------------------------------------------------------------------

/// Everything the app touches outside its own logic. Reads take `&self` so planning cannot change anything.
pub trait Sys {
    // filesystem
    fn exists(&self, p: &Path) -> bool;
    fn read(&self, p: &Path) -> Option<Vec<u8>>;
    fn sha256(&self, p: &Path) -> Option<String>;
    fn read_link(&self, p: &Path) -> Option<PathBuf>;
    fn write(&mut self, p: &Path, bytes: &[u8], mode: u32) -> Result<()>;
    /// Copy `from` to `to` without ever leaving `to` half-written; an in-use Windows program is renamed aside.
    fn copy_exe(&mut self, from: &Path, to: &Path) -> Result<()>;
    fn symlink(&mut self, target: &Path, link: &Path) -> Result<()>;
    /// Remove a file or symlink (a program that is in use is renamed away and deleted later).
    fn remove_file(&mut self, p: &Path) -> Result<()>;
    fn remove_dir_if_empty(&mut self, p: &Path) -> Result<()>;
    // Windows per-user PATH (raw, unexpanded); nothing to read elsewhere
    fn user_path(&self) -> Result<Option<String>> {
        Ok(None)
    }
    fn set_user_path(&mut self, _value: &str) -> Result<()> {
        Ok(())
    }
    // Windows machine PATH (raw, unexpanded)
    fn machine_path(&self) -> Result<Option<String>>;
    fn set_machine_path(&mut self, value: &str) -> Result<()>;
    fn broadcast_env_change(&mut self);
    // shortcuts
    fn create_shortcut(&mut self, s: &ShortcutSpec) -> Result<()>;
    fn read_shortcut(&self, p: &Path) -> Option<ShortcutSpec>;
    // Apps & features
    fn uninstall_entry(&self) -> Option<UninstallEntry>;
    fn set_uninstall_entry(&mut self, e: &UninstallEntry) -> Result<()>;
    fn remove_uninstall_entry(&mut self) -> Result<()>;
}

// ---------------------------------------------------------------------------------------------
// Versions
// ---------------------------------------------------------------------------------------------

fn parse_semver(v: &str) -> Option<(Vec<u64>, Option<String>)> {
    let v = v.trim().trim_start_matches('v');
    let v = v.split('+').next()?;
    let (core, pre) = match v.split_once('-') {
        Some((c, p)) => (c, Some(p.to_string())),
        None => (v, None),
    };
    let nums: Option<Vec<u64>> = core.split('.').map(|p| p.parse().ok()).collect();
    let nums = nums.filter(|n| !n.is_empty())?;
    Some((nums, pre))
}

/// Semver ordering (a pre-release is older than its release). `None` if either side is not a version.
pub fn compare_versions(a: &str, b: &str) -> Option<Ordering> {
    let ((an, ap), (bn, bp)) = (parse_semver(a)?, parse_semver(b)?);
    let len = an.len().max(bn.len());
    for i in 0..len {
        let (x, y) = (an.get(i).copied().unwrap_or(0), bn.get(i).copied().unwrap_or(0));
        if x != y {
            return Some(x.cmp(&y));
        }
    }
    Some(match (ap, bp) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => {
            let (xs, ys): (Vec<&str>, Vec<&str>) = (x.split('.').collect(), y.split('.').collect());
            let mut ord = Ordering::Equal;
            for i in 0..xs.len().max(ys.len()) {
                ord = match (xs.get(i), ys.get(i)) {
                    (None, Some(_)) => Ordering::Less,
                    (Some(_), None) => Ordering::Greater,
                    (Some(p), Some(q)) => match (p.parse::<u64>(), q.parse::<u64>()) {
                        (Ok(p), Ok(q)) => p.cmp(&q),
                        (Ok(_), Err(_)) => Ordering::Less,
                        (Err(_), Ok(_)) => Ordering::Greater,
                        _ => p.cmp(q),
                    },
                    (None, None) => Ordering::Equal,
                };
                if ord != Ordering::Equal {
                    break;
                }
            }
            ord
        }
    })
}

// ---------------------------------------------------------------------------------------------
// Rendering helpers
// ---------------------------------------------------------------------------------------------

/// `Exec=` value: quote a path containing spaces or special characters.
pub fn desktop_exec_arg(p: &Path) -> String {
    let s = p.to_string_lossy();
    if s.chars().any(|c| c.is_whitespace() || "\"'`$\\<>~|&;*?#()".contains(c)) {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('$', "\\$").replace('`', "\\`"))
    } else {
        s.into_owned()
    }
}

#[cfg(test)]
pub fn desktop_entry_text(exe: &Path, icon: &Path) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName={APP_NAME}\nComment=Install and manage your Kora FiveM servers\nExec={} manager\nIcon={}\nTerminal=true\nCategories=System;Utility;\n{DESKTOP_MARKER}\n",
        desktop_exec_arg(exe),
        icon.display()
    )
}

/// `"<exe>" uninstall --app`
#[cfg(test)]
pub fn uninstall_string(exe: &Path) -> String {
    format!("\"{}\" uninstall --app", exe.display())
}

#[cfg(test)]
fn shortcut_spec(l: &Layout, path: &Path) -> ShortcutSpec {
    ShortcutSpec {
        path: path.to_path_buf(),
        target: l.exe.clone(),
        args: "manager".into(),
        workdir: l.dir.clone(),
        icon: format!("{},0", l.exe.display()),
        description: "Open the Kora server manager".into(),
        run_as_admin: true,
        minimized: false,
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    let (a, b) = (a.to_string_lossy(), b.to_string_lossy());
    if cfg!(windows) || a.contains('\\') || b.contains('\\') {
        a.replace('/', "\\").eq_ignore_ascii_case(&b.replace('/', "\\"))
    } else {
        a == b
    }
}

fn load_state(sys: &dyn Sys, l: &Layout) -> Option<AppState> {
    sys.read(&l.state).and_then(|b| serde_json::from_slice(&b).ok())
}

#[cfg(test)]
fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// One-line fix when the folder holding the `kora` command is not on the user's `PATH`.
#[cfg(test)]
pub fn path_hint(command_dir: &Path, path_env: &str) -> Option<String> {
    let on = std::env::split_paths(path_env).any(|p| same_path(&p, command_dir));
    (!on).then(|| format!("{} is not on your PATH. Add it with:  export PATH=\"{}:$PATH\"  (put that line in ~/.profile)", command_dir.display(), command_dir.display()))
}

// ---------------------------------------------------------------------------------------------
// Planner
// ---------------------------------------------------------------------------------------------

/// Work out what installing (or refreshing) the app would change. Reads only.
#[cfg(test)]
pub fn plan_install(sys: &dyn Sys, l: &Layout, run: &Running, opts: &AppOptions) -> Plan {
    let mut plan = Plan::default();
    // ---- the program ----
    let mut copy_kind = None;
    let mut keep_state = false;
    let installed = load_state(sys, l);
    if same_path(&run.exe, &l.exe) {
        plan.notes.push("Running from the installed copy already.".into());
    } else if !sys.exists(&l.exe) {
        copy_kind = Some(CopyKind::Fresh);
    } else {
        let on_disk = sys.sha256(&l.exe);
        if on_disk.as_deref() == Some(run.sha256.as_str()) {
            // identical: nothing to copy
        } else {
            let ord = installed.as_ref().and_then(|s| compare_versions(&run.version, &s.version));
            match ord {
                Some(Ordering::Less) if !opts.force => {
                    keep_state = true;
                    plan.notes.push(format!(
                        "The installed Kora app is newer ({}) than this program ({}); keeping it. Use --force to replace it anyway.",
                        installed.as_ref().map(|s| s.version.as_str()).unwrap_or("?"),
                        run.version
                    ));
                }
                _ => copy_kind = Some(CopyKind::Replace),
            }
        }
    }
    let state_new = AppState { version: run.version.clone(), sha256: run.sha256.clone(), installed_unix: now_unix() };
    if let Some(kind) = copy_kind {
        plan.actions.push(Action::CopyExe { from: run.exe.clone(), to: l.exe.clone(), kind });
        plan.actions.push(Action::WriteState(state_new));
    } else if !keep_state && installed.as_ref().map(|s| (&s.version, &s.sha256)) != Some((&run.version, &run.sha256)) {
        // Identical program but the state file is missing or stale.
        plan.actions.push(Action::WriteState(state_new));
    }

    let installed_version = if keep_state { installed.as_ref().map(|s| s.version.clone()) } else { Some(run.version.clone()) };

    if l.windows {
        // ---- machine PATH ----
        let dir = l.dir.to_string_lossy().into_owned();
        match sys.machine_path() {
            Ok(cur) => {
                if !cur.as_deref().is_some_and(|p| pathlist::contains(p, &dir)) {
                    plan.actions.push(Action::AddMachinePath(dir));
                    plan.actions.push(Action::BroadcastEnv);
                }
            }
            Err(e) => plan.notes.push(format!("Could not read the machine PATH ({e}); `kora` was not added to it.")),
        }
        // ---- shortcuts ----
        let mut want = vec![];
        if let Some(p) = &l.start_menu {
            want.push(shortcut_spec(l, p));
        }
        if let (true, Some(p)) = (opts.desktop_shortcut, &l.desktop) {
            want.push(shortcut_spec(l, p));
        }
        for spec in want {
            if sys.read_shortcut(&spec.path).as_ref() != Some(&spec) {
                plan.actions.push(Action::Shortcut(spec));
            }
        }
        // ---- Apps & features ----
        let entry = UninstallEntry {
            display_name: APP_NAME.into(),
            display_version: installed_version.clone().unwrap_or_else(|| run.version.clone()),
            publisher: PUBLISHER.into(),
            install_location: l.dir.to_string_lossy().into_owned(),
            uninstall_string: uninstall_string(&l.exe),
            display_icon: format!("{},0", l.exe.display()),
        };
        if sys.uninstall_entry().as_ref() != Some(&entry) {
            plan.actions.push(Action::RegisterUninstall(entry));
        }
    } else {
        // ---- command on PATH ----
        if let Some(link) = &l.link {
            if l.user_install {
                plan.notes.push("kora is already installed for your user (~/.local/bin); no system-wide command was added.".into());
            } else {
                match sys.read_link(link) {
                    Some(t) if same_path(&t, &l.exe) => {}
                    Some(t) => plan.notes.push(format!("{} already points to {}; left alone (run `ln -sf {} {}` to use the Kora app).", link.display(), t.display(), l.exe.display(), link.display())),
                    None if sys.exists(link) => plan.notes.push(format!("{} already exists and is not a link; left alone.", link.display())),
                    None => plan.actions.push(Action::Symlink { link: link.clone(), target: l.exe.clone() }),
                }
            }
        }
        // ---- launcher (desktop sessions only) ----
        if let (true, Some(path), Some(icon)) = (l.desktop_session, &l.desktop_entry, &l.icon_png) {
            if sys.read(icon).as_deref() != Some(LOGO_PNG) {
                plan.actions.push(Action::WriteLogo(icon.clone()));
            }
            let content = desktop_entry_text(&l.exe, icon);
            match sys.read(path) {
                Some(cur) if cur == content.as_bytes() => {}
                Some(cur) if !String::from_utf8_lossy(&cur).contains(DESKTOP_MARKER) => {
                    plan.notes.push(format!("{} exists and was not made by Kora; left alone.", path.display()))
                }
                _ => plan.actions.push(Action::DesktopEntry { path: path.clone(), content }),
            }
        }
    }
    plan
}

/// Work out what removing the app would change. Only things the app made are listed.
pub fn plan_uninstall(sys: &dyn Sys, l: &Layout) -> Plan {
    let mut plan = Plan::default();
    if l.windows {
        for p in [&l.start_menu, &l.desktop].into_iter().flatten() {
            match sys.read_shortcut(p) {
                Some(s) if same_path(&s.target, &l.exe) => plan.actions.push(Action::RemoveShortcut(p.clone())),
                Some(_) => plan.notes.push(format!("{} does not point at the Kora app; left alone.", p.display())),
                None => {}
            }
        }
        let dir = l.dir.to_string_lossy().into_owned();
        if let Ok(Some(cur)) = sys.machine_path() {
            if pathlist::contains(&cur, &dir) {
                plan.actions.push(Action::RemoveMachinePath(dir));
                plan.actions.push(Action::BroadcastEnv);
            }
        }
        if sys.uninstall_entry().is_some_and(|e| e.display_name == APP_NAME && e.publisher == PUBLISHER) {
            plan.actions.push(Action::RemoveUninstall);
        }
    } else {
        if let Some(link) = &l.link {
            match sys.read_link(link) {
                Some(t) if same_path(&t, &l.exe) => plan.actions.push(Action::RemoveSymlink(link.clone())),
                Some(_) => plan.notes.push(format!("{} points somewhere else; left alone.", link.display())),
                None => {}
            }
        }
        if let Some(p) = &l.desktop_entry {
            if let Some(cur) = sys.read(p) {
                if String::from_utf8_lossy(&cur).contains(DESKTOP_MARKER) {
                    plan.actions.push(Action::RemoveFile(p.clone()));
                } else {
                    plan.notes.push(format!("{} was not made by Kora; left alone.", p.display()));
                }
            }
        }
        if let Some(p) = &l.icon_png {
            if sys.exists(p) {
                plan.actions.push(Action::RemoveFile(p.clone()));
                if let Some(d) = p.parent() {
                    plan.actions.push(Action::RemoveDirIfEmpty(d.to_path_buf()));
                }
            }
        }
    }
    if sys.exists(&l.exe) {
        plan.actions.push(Action::RemoveFile(l.exe.clone()));
    }
    if sys.exists(&l.state) {
        plan.actions.push(Action::RemoveFile(l.state.clone()));
    }
    plan.actions.push(Action::RemoveDirIfEmpty(l.dir.clone()));
    if !l.windows {
        if let Some(root) = l.dir.parent() {
            // /opt/kora also holds the service copies; it is only removed when nothing else is left.
            plan.actions.push(Action::RemoveDirIfEmpty(root.to_path_buf()));
        }
    }
    plan.actions.retain(|a| match a {
        // Do not list folder cleanups for folders that do not exist.
        Action::RemoveDirIfEmpty(p) => sys.exists(p),
        _ => true,
    });
    plan
}

// ---------------------------------------------------------------------------------------------
// Executor
// ---------------------------------------------------------------------------------------------

fn apply(sys: &mut dyn Sys, l: &Layout, a: &Action) -> Result<()> {
    match a {
        Action::CopyExe { from, to, .. } => sys.copy_exe(from, to),
        Action::WriteState(s) => sys.write(&l.state, &serde_json::to_vec_pretty(s).map_err(|e| AppError(e.to_string()))?, 0o644),
        Action::AddMachinePath(d) => {
            let cur = sys.machine_path()?.unwrap_or_default();
            match pathlist::add(&cur, d) {
                Some(v) => sys.set_machine_path(&v),
                None => Ok(()),
            }
        }
        Action::AddUserPath(d) => {
            let cur = sys.user_path()?.unwrap_or_default();
            match pathlist::add(&cur, d) {
                Some(v) => sys.set_user_path(&v),
                None => Ok(()),
            }
        }
        Action::RemoveMachinePath(d) => {
            let cur = sys.machine_path()?.unwrap_or_default();
            match pathlist::remove(&cur, d) {
                Some(v) => sys.set_machine_path(&v),
                None => Ok(()),
            }
        }
        Action::BroadcastEnv => {
            sys.broadcast_env_change();
            Ok(())
        }
        Action::Shortcut(s) => sys.create_shortcut(s),
        Action::Symlink { link, target } => sys.symlink(target, link),
        Action::WriteLogo(p) => sys.write(p, LOGO_PNG, 0o644),
        Action::DesktopEntry { path, content } => sys.write(path, content.as_bytes(), 0o644),
        Action::RegisterUninstall(e) => sys.set_uninstall_entry(e),
        Action::RemoveShortcut(p) | Action::RemoveFile(p) | Action::RemoveSymlink(p) => sys.remove_file(p),
        Action::RemoveUninstall => sys.remove_uninstall_entry(),
        Action::RemoveDirIfEmpty(p) => sys.remove_dir_if_empty(p),
    }
}

fn run_plan(sys: &mut dyn Sys, l: &Layout, plan: &Plan) -> Result<Vec<String>> {
    let mut done = vec![];
    for a in &plan.actions {
        apply(sys, l, a).map_err(|e| AppError(format!("{} failed: {e}", a.describe())))?;
        done.push(a.describe());
    }
    Ok(done)
}

/// Install or refresh the app with explicit dependencies (tests use a fake [`Sys`]).
#[cfg(test)]
pub fn install_app_with(sys: &mut dyn Sys, l: &Layout, run: &Running, opts: &AppOptions) -> Result<AppInstall> {
    let mut plan = plan_install(sys, l, run, opts);
    let changes = if opts.dry_run { plan.actions.iter().map(Action::describe).collect() } else { run_plan(sys, l, &plan)? };
    if !opts.dry_run {
        if l.windows {
            // the machine PATH was handled above
        } else if let Some(dir) = l.command_dir() {
            if let Some(h) = path_hint(&dir, &std::env::var("PATH").unwrap_or_default()) {
                plan.notes.push(h);
            }
        } else if l.user_install {
            if let Some(dir) = run.exe.parent() {
                if let Some(h) = path_hint(dir, &std::env::var("PATH").unwrap_or_default()) {
                    plan.notes.push(h);
                }
            }
        }
        if l.windows && changes.iter().any(|c| c.contains("machine PATH")) {
            plan.notes.push("Open a new terminal to use the `kora` command (the PATH of windows that were already open does not change).".into());
        }
    }
    Ok(AppInstall { exe: l.exe.clone(), version: run.version.clone(), changes, notes: plan.notes, dry_run: opts.dry_run })
}

/// Remove the app (shortcuts, PATH entry, Apps & features entry, the installed copy). Servers and databases are never touched.
pub fn uninstall_app_with(sys: &mut dyn Sys, l: &Layout, dry_run: bool) -> Result<Vec<String>> {
    let plan = plan_uninstall(sys, l);
    if dry_run {
        return Ok(plan.actions.iter().map(Action::describe).chain(plan.notes).collect());
    }
    let mut out = run_plan(sys, l, &plan)?;
    out.extend(plan.notes);
    Ok(out)
}

pub fn status_with(sys: &dyn Sys, l: &Layout, run: &Running) -> AppStatus {
    let state = load_state(sys, l);
    let exe_exists = sys.exists(&l.exe);
    let dir = l.dir.to_string_lossy().into_owned();
    let (on_path, start_menu, desktop, desktop_entry) = if l.windows {
        let has = |p: &Option<PathBuf>| p.as_ref().map(|p| sys.read_shortcut(p).is_some_and(|s| same_path(&s.target, &l.exe)));
        (sys.machine_path().ok().flatten().is_some_and(|p| pathlist::contains(&p, &dir)), has(&l.start_menu), has(&l.desktop), None)
    } else {
        (
            l.link.as_ref().and_then(|p| sys.read_link(p)).is_some_and(|t| same_path(&t, &l.exe)),
            None,
            None,
            l.desktop_entry.as_ref().map(|p| sys.read(p).is_some_and(|b| String::from_utf8_lossy(&b).contains(DESKTOP_MARKER))),
        )
    };
    let version = state.as_ref().map(|s| s.version.clone());
    AppStatus {
        installed: exe_exists && state.is_some(),
        update_available: version.as_deref().and_then(|v| compare_versions(&run.version, v)) == Some(Ordering::Greater),
        version,
        exe: l.exe.clone(),
        exe_exists,
        on_path,
        start_menu,
        desktop,
        uninstall_registered: l.windows && sys.uninstall_entry().is_some(),
        desktop_entry,
        running_version: run.version.clone(),
    }
}

// ---------------------------------------------------------------------------------------------
// Public API on the real system
// ---------------------------------------------------------------------------------------------

#[cfg(test)]
pub fn install_app(opts: &AppOptions) -> Result<AppInstall> {
    let run = Running::current()?;
    let l = Layout::from_env(&run.exe)?;
    install_app_with(&mut RealSys::new(), &l, &run, opts)
}

pub fn uninstall_app() -> Result<()> {
    uninstall_app_opts(false).map(|_| ())
}

/// Returns the sentences describing what was (or, with `dry_run`, would be) removed.
pub fn uninstall_app_opts(dry_run: bool) -> Result<Vec<String>> {
    let exe = std::env::current_exe().map_err(|e| AppError(format!("cannot find the kora program: {e}")))?;
    let l = Layout::from_env(&exe)?;
    uninstall_app_with(&mut RealSys::new(), &l, dry_run)
}

pub fn status() -> AppStatus {
    let exe = std::env::current_exe().unwrap_or_default();
    let run = Running { exe: exe.clone(), version: env!("CARGO_PKG_VERSION").into(), sha256: String::new() };
    match Layout::from_env(&exe) {
        Ok(l) => status_with(&RealSys::new(), &l, &run),
        Err(_) => AppStatus {
            installed: false,
            version: None,
            exe: PathBuf::new(),
            exe_exists: false,
            on_path: false,
            start_menu: None,
            desktop: None,
            uninstall_registered: false,
            desktop_entry: None,
            running_version: run.version,
            update_available: false,
        },
    }
}

/// Is the app step worth offering on this machine? (A program folder exists to put it in.)
pub fn supported() -> bool {
    Layout::from_env(&std::env::current_exe().unwrap_or_default()).is_ok()
}

/// Does `kind` of failure mean "needs administrator/root"? Used to turn a permission error into a note.
pub fn is_permission_error(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("permission denied") || m.contains("access is denied") || m.contains("requires elevation")
}
