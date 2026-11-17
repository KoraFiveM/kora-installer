//! Per-user shortcuts: the "Kora app" since 0.7.0.
//!
//! The app is the web panel in a browser tab. These shortcuts only start it: a bare `kora` makes sure the
//! background panel runs and opens the browser. Nothing here needs administrator rights.
//!
//! * Windows: `Kora.lnk` in the user's Start Menu (and optionally on their Desktop), minimised so no
//!   console window flashes, and NOT "Run as administrator".
//! * Linux (desktop sessions only): `~/.local/share/applications/kora.desktop` with `Terminal=false`.
//!
//! Also here: finding what the OLD machine-wide app left behind ([`detect_legacy`]), so the panel can
//! offer "Tidy up old shortcuts", and re-pointing the background panel when it started from the old copy.
//!
//! Planning only reads through [`Sys`]; the executor reuses [`apply`](super::apply). Tests use a fake.

use std::path::{Path, PathBuf};

use super::{desktop_exec_arg, plan_uninstall, same_path, Action, AppError, CopyKind, EnvGet, Layout, Plan, RealSys, Result, ShortcutSpec, Sys, DESKTOP_MARKER, LOGO_PNG};

/// Where the per-user shortcuts and (if needed) a per-user copy of the program go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserLayout {
    pub windows: bool,
    /// A per-user copy of the program: `%LOCALAPPDATA%\Kora\bin\kora.exe`, `~/.local/bin/kora`. Only made
    /// when the program that runs now lives in the old machine-wide folder (which is about to be removed).
    pub program: PathBuf,
    pub start_menu: Option<PathBuf>,
    pub desktop: Option<PathBuf>,
    pub desktop_entry: Option<PathBuf>,
    pub icon_png: Option<PathBuf>,
    /// Linux: a desktop session exists (a launcher is only useful then).
    pub desktop_session: bool,
}

fn wjoin(base: &Path, parts: &[&str]) -> PathBuf {
    PathBuf::from(format!("{}\\{}", base.to_string_lossy().trim_end_matches(['\\', '/']), parts.join("\\")))
}

impl UserLayout {
    /// Resolve from the environment. `is_dir` is asked about the OneDrive-redirected Desktop only.
    pub fn resolve(windows: bool, get: EnvGet, is_dir: &dyn Fn(&Path) -> bool) -> Result<UserLayout> {
        let var = |k: &str| get(k).filter(|v| !v.is_empty()).map(PathBuf::from);
        if windows {
            let profile = var("USERPROFILE");
            let need = |what: &str| AppError(format!("{what} is not set, so Kora cannot find your user folders"));
            let local = match var("LOCALAPPDATA") {
                Some(l) => l,
                None => wjoin(profile.as_deref().ok_or_else(|| need("LOCALAPPDATA"))?, &["AppData", "Local"]),
            };
            let roaming = match var("APPDATA") {
                Some(a) => a,
                None => wjoin(profile.as_deref().ok_or_else(|| need("APPDATA"))?, &["AppData", "Roaming"]),
            };
            // A OneDrive-backed Desktop is the one people actually see.
            let desktop_dir = var("OneDrive").map(|o| wjoin(&o, &["Desktop"])).filter(|d| is_dir(d)).or_else(|| profile.as_deref().map(|p| wjoin(p, &["Desktop"])));
            let lnk = format!("{}.lnk", super::APP_NAME);
            return Ok(UserLayout {
                windows,
                program: wjoin(&local, &["Kora", "bin", "kora.exe"]),
                start_menu: Some(wjoin(&roaming, &["Microsoft", "Windows", "Start Menu", "Programs", &lnk])),
                desktop: desktop_dir.map(|d| wjoin(&d, &[&lnk])),
                desktop_entry: None,
                icon_png: None,
                desktop_session: false,
            });
        }
        let home = var("HOME").filter(|h| h.is_absolute()).ok_or_else(|| AppError("HOME is not set, so Kora cannot find your user folders".into()))?;
        let data = var("XDG_DATA_HOME").filter(|d| d.is_absolute()).unwrap_or_else(|| home.join(".local").join("share"));
        let session =
            ["DISPLAY", "WAYLAND_DISPLAY", "XDG_CURRENT_DESKTOP"].iter().any(|k| var(k).is_some()) || var("XDG_SESSION_TYPE").is_some_and(|t| t == Path::new("x11") || t == Path::new("wayland"));
        Ok(UserLayout {
            windows,
            program: home.join(".local").join("bin").join("kora"),
            start_menu: None,
            desktop: None,
            desktop_entry: Some(data.join("applications").join("kora.desktop")),
            icon_png: Some(data.join("icons").join("hicolor").join("512x512").join("apps").join("kora.png")),
            desktop_session: session,
        })
    }

    pub fn from_env() -> Result<UserLayout> {
        UserLayout::resolve(cfg!(windows), &|k| std::env::var_os(k), &|p| p.is_dir())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserOptions {
    /// Also a "Kora" shortcut on the desktop (Windows).
    pub desktop_shortcut: bool,
    /// Plan only: report every change, make none.
    pub dry_run: bool,
}

impl Default for UserOptions {
    fn default() -> Self {
        Self { desktop_shortcut: true, dry_run: false }
    }
}

/// The launcher text: starts a bare `kora` (the panel), without a terminal window.
pub fn desktop_entry_text(exe: &Path, icon: &Path) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName={}\nComment=Install and manage your FiveM servers in your browser\nExec={}\nIcon={}\nTerminal=false\nCategories=Network;Utility;\n{DESKTOP_MARKER}\n",
        super::APP_NAME,
        desktop_exec_arg(exe),
        icon.display()
    )
}

/// The Windows shortcut: a bare `kora`, minimised, never "Run as administrator".
pub fn shortcut_spec(exe: &Path, path: &Path) -> ShortcutSpec {
    ShortcutSpec {
        path: path.to_path_buf(),
        target: exe.to_path_buf(),
        args: String::new(),
        workdir: split_last(exe).0,
        icon: format!("{},0", exe.display()),
        description: "Open Kora: install and manage your FiveM servers in your browser".into(),
        run_as_admin: false,
        minimized: true,
    }
}

/// Is `a` the same shortcut as `b`? (`minimized` is not stored in a way we can read back.)
fn same_shortcut(a: &ShortcutSpec, b: &ShortcutSpec) -> bool {
    ShortcutSpec { minimized: b.minimized, ..a.clone() } == *b
}

/// What making the per-user shortcuts would change. Reads only. `legacy_dir` is the folder of the old
/// machine-wide app: when the program that runs now lives there it is about to be removed, so the shortcuts
/// point at a per-user copy (made here when missing) instead.
/// The program the shortcuts run: this one, or the per-user copy when this one is in the old folder.
pub fn target_exe(ul: &UserLayout, run_exe: &Path, legacy_dir: Option<&Path>) -> PathBuf {
    if runs_from(run_exe, legacy_dir) {
        ul.program.clone()
    } else {
        run_exe.to_path_buf()
    }
}

/// Split off the last path component, whichever separator the path uses (a Windows path is also looked at on
/// Linux in tests). `(folder, name)`.
fn split_last(p: &Path) -> (PathBuf, String) {
    let s = p.to_string_lossy();
    match s.rfind(['\\', '/']) {
        Some(i) => (PathBuf::from(&s[..i]), s[i + 1..].to_string()),
        None => (PathBuf::new(), s.into_owned()),
    }
}

fn runs_from(run_exe: &Path, dir: Option<&Path>) -> bool {
    dir.is_some_and(|d| same_path(&split_last(run_exe).0, d))
}

pub fn plan_user_install(sys: &dyn Sys, ul: &UserLayout, run_exe: &Path, legacy_dir: Option<&Path>, opts: &UserOptions) -> Plan {
    let mut plan = Plan::default();
    let exe = target_exe(ul, run_exe, legacy_dir);
    if runs_from(run_exe, legacy_dir) && !sys.exists(&ul.program) {
        plan.actions.push(Action::CopyExe { from: run_exe.to_path_buf(), to: ul.program.clone(), kind: CopyKind::Fresh });
        if let Some(dir) = ul.program.parent().filter(|_| ul.windows) {
            let dir = dir.to_string_lossy().into_owned();
            let on = sys.user_path().ok().flatten().is_some_and(|p| super::pathlist::contains(&p, &dir));
            if !on {
                plan.actions.push(Action::AddUserPath(dir));
                plan.actions.push(Action::BroadcastEnv);
            }
        }
    }
    if ul.windows {
        let mut want = vec![];
        if let Some(p) = &ul.start_menu {
            want.push(shortcut_spec(&exe, p));
        }
        if let (true, Some(p)) = (opts.desktop_shortcut, &ul.desktop) {
            want.push(shortcut_spec(&exe, p));
        }
        for spec in want {
            if !sys.read_shortcut(&spec.path).is_some_and(|cur| same_shortcut(&cur, &spec)) {
                plan.actions.push(Action::Shortcut(spec));
            }
        }
    } else if let (Some(path), Some(icon)) = (&ul.desktop_entry, &ul.icon_png) {
        if !ul.desktop_session {
            plan.notes.push("There is no desktop on this computer, so no launcher was added. Run `kora` in a terminal to open the panel.".into());
        } else {
            let content = desktop_entry_text(&exe, icon);
            match sys.read(path) {
                Some(cur) if cur == content.as_bytes() => {}
                Some(cur) if !String::from_utf8_lossy(&cur).contains(DESKTOP_MARKER) => plan.notes.push(format!("{} exists and was not made by Kora; left alone.", path.display())),
                _ => {
                    if sys.read(icon).as_deref() != Some(LOGO_PNG) {
                        plan.actions.push(Action::WriteLogo(icon.clone()));
                    }
                    plan.actions.push(Action::DesktopEntry { path: path.clone(), content });
                }
            }
        }
    }
    plan
}

#[derive(Debug, Clone)]
pub struct UserInstall {
    /// The program the shortcuts run.
    pub exe: PathBuf,
    /// One plain sentence per change made (or, in a dry run, that would be made).
    pub changes: Vec<String>,
    pub notes: Vec<String>,
    pub dry_run: bool,
}

/// Make (or refresh) the per-user shortcuts through `sys`.
pub fn install_user_with(sys: &mut dyn Sys, ul: &UserLayout, run_exe: &Path, legacy_dir: Option<&Path>, opts: &UserOptions) -> Result<UserInstall> {
    let plan = plan_user_install(sys, ul, run_exe, legacy_dir, opts);
    let exe = target_exe(ul, run_exe, legacy_dir);
    let mut changes = Vec::new();
    // The shortcuts of a plan never need the old layout's state file, but `apply` wants a layout.
    let dummy = dummy_layout(ul);
    for a in &plan.actions {
        if !opts.dry_run {
            super::apply(sys, &dummy, a).map_err(|e| AppError(format!("{} failed: {e}", a.describe())))?;
        }
        changes.push(a.describe());
    }
    Ok(UserInstall { exe, changes, notes: plan.notes, dry_run: opts.dry_run })
}

/// `apply` only reads the state-file path of a layout for `WriteState`, which a per-user plan never has.
fn dummy_layout(ul: &UserLayout) -> Layout {
    Layout {
        windows: ul.windows,
        dir: PathBuf::new(),
        exe: PathBuf::new(),
        state: PathBuf::new(),
        start_menu: None,
        desktop: None,
        link: None,
        desktop_entry: None,
        icon_png: None,
        desktop_session: false,
        user_install: true,
    }
}

/// Is this one of the shortcuts [`shortcut_spec`] makes: a bare `kora`, no arguments? (The old machine-wide
/// shortcut at the same path ran `kora manager` and is left to the old app's removal.)
fn is_kora_shortcut(s: &ShortcutSpec) -> bool {
    split_last(&s.target).1.to_ascii_lowercase().starts_with("kora") && s.args.trim().is_empty()
}

/// What removing the per-user shortcuts would change: the Start Menu and Desktop shortcuts (Windows), the
/// launcher and its icon (Linux). Only what [`plan_user_install`] makes is listed; a file with the same
/// name that Kora did not make is left alone with a note. Reads only.
pub fn plan_user_uninstall(sys: &dyn Sys, ul: &UserLayout) -> Plan {
    let mut plan = Plan::default();
    if ul.windows {
        for p in [&ul.start_menu, &ul.desktop].into_iter().flatten() {
            match sys.read_shortcut(p) {
                Some(s) if is_kora_shortcut(&s) => plan.actions.push(Action::RemoveShortcut(p.clone())),
                Some(_) => plan.notes.push(format!("{} is not one of the shortcuts Kora made; left alone.", p.display())),
                None => {}
            }
        }
    } else {
        if let Some(path) = &ul.desktop_entry {
            if let Some(cur) = sys.read(path) {
                if String::from_utf8_lossy(&cur).contains(DESKTOP_MARKER) {
                    plan.actions.push(Action::RemoveFile(path.clone()));
                } else {
                    plan.notes.push(format!("{} was not made by Kora; left alone.", path.display()));
                }
            }
        }
        if let Some(icon) = &ul.icon_png {
            // Only the picture Kora wrote itself.
            if sys.read(icon).as_deref() == Some(LOGO_PNG) {
                plan.actions.push(Action::RemoveFile(icon.clone()));
            }
        }
    }
    plan
}

/// Remove the per-user shortcuts through `sys`. Returns one plain sentence per change made (or, in a dry
/// run, that would be made) and then the notes. Nothing else is touched: not the program, not the servers.
pub fn uninstall_user_with(sys: &mut dyn Sys, ul: &UserLayout, dry_run: bool) -> Result<Vec<String>> {
    let plan = plan_user_uninstall(sys, ul);
    let dummy = dummy_layout(ul);
    let mut out = Vec::new();
    for a in &plan.actions {
        if !dry_run {
            super::apply(sys, &dummy, a).map_err(|e| AppError(format!("{} failed: {e}", a.describe())))?;
        }
        out.push(a.describe());
    }
    out.extend(plan.notes);
    Ok(out)
}

/// [`uninstall_user_with`] on the real system.
pub fn uninstall_user(dry_run: bool) -> Result<Vec<String>> {
    let ul = UserLayout::from_env()?;
    uninstall_user_with(&mut RealSys::new(), &ul, dry_run)
}

/// Per-user shortcuts that are in place (`None` = not applicable on this system).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UserStatus {
    pub start_menu: Option<bool>,
    pub desktop: Option<bool>,
    pub launcher: Option<bool>,
}

impl UserStatus {
    /// The way in is there: the Start Menu shortcut (Windows) or the launcher (Linux).
    pub fn in_place(&self) -> bool {
        self.start_menu.or(self.launcher).unwrap_or(false)
    }

    /// Any of the shortcuts exists (what removing them would remove).
    pub fn any(&self) -> bool {
        [self.start_menu, self.desktop, self.launcher].into_iter().any(|v| v == Some(true))
    }
}

pub fn user_status_with(sys: &dyn Sys, ul: &UserLayout) -> UserStatus {
    let is_kora = |p: &Option<PathBuf>| p.as_ref().map(|p| sys.read_shortcut(p).is_some_and(|s| is_kora_shortcut(&s)));
    UserStatus {
        start_menu: is_kora(&ul.start_menu),
        desktop: is_kora(&ul.desktop),
        launcher: ul.desktop_entry.as_ref().map(|p| sys.read(p).is_some_and(|b| String::from_utf8_lossy(&b).contains(DESKTOP_MARKER))),
    }
}

// ---------------------------------------------------------------------------------------------
// The old machine-wide app
// ---------------------------------------------------------------------------------------------

/// What the old machine-wide app (and its administrator shortcuts) left on this computer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Legacy {
    /// One plain sentence per thing found (what removing it would do).
    pub items: Vec<String>,
}

impl Legacy {
    pub fn present(&self) -> bool {
        !self.items.is_empty()
    }
}

/// Look for the old app: the program in `%ProgramFiles%\Kora` / `/opt/kora/bin`, its machine PATH entry, the
/// "Apps & features" entry, the shortcuts that run it as administrator and the Linux terminal launcher.
/// Folder clean-ups do not count (an empty folder is not "an old app").
pub fn detect_legacy(sys: &dyn Sys, old: &Layout) -> Legacy {
    let plan = plan_uninstall(sys, old);
    Legacy { items: plan.actions.iter().filter(|a| !matches!(a, Action::RemoveDirIfEmpty(_))).map(Action::describe).collect() }
}

/// The old app's folder (for [`plan_user_install`]), when it can be worked out here.
pub fn legacy_layout() -> Option<Layout> {
    let exe = std::env::current_exe().ok()?;
    Layout::from_env(&exe).ok()
}

/// [`detect_legacy`] on the real system.
pub fn legacy_present() -> Legacy {
    match legacy_layout() {
        Some(l) => detect_legacy(&RealSys::new(), &l),
        None => Legacy::default(),
    }
}

/// Make the per-user shortcuts on the real system.
pub fn install_user(opts: &UserOptions) -> Result<UserInstall> {
    let run = std::env::current_exe().map_err(|e| AppError(format!("cannot find the kora program: {e}")))?;
    let ul = UserLayout::from_env()?;
    let old = legacy_layout();
    let out = install_user_with(&mut RealSys::new(), &ul, &run, old.as_ref().map(|l| l.dir.as_path()), opts)?;
    if !opts.dry_run {
        if let Some(l) = &old {
            // The background panel may have been set up from the old copy: it must not lose its program.
            repoint_background_panel(&l.exe, &out.exe);
        }
    }
    Ok(out)
}

/// The text of a start-up file with the program `old` replaced by `new` (`None` when it does not mention `old`).
/// Windows paths compare without regard to case.
pub fn repoint_text(text: &str, old: &Path, new: &Path) -> Option<String> {
    let (old, new) = (old.display().to_string(), new.display().to_string());
    if old.is_empty() {
        return None;
    }
    let ci = old.contains('\\') || cfg!(windows);
    let (hay, needle) = if ci { (text.to_ascii_lowercase(), old.to_ascii_lowercase()) } else { (text.to_string(), old.clone()) };
    // ASCII lower-casing keeps every byte offset, so the match positions are valid in `text`.
    let mut out = String::with_capacity(text.len());
    let (mut at, mut found) = (0, false);
    while let Some(i) = hay[at..].find(&needle) {
        out.push_str(&text[at..at + i]);
        out.push_str(&new);
        at += i + needle.len();
        found = true;
    }
    out.push_str(&text[at..]);
    found.then_some(out)
}

/// Best effort: point the background panel's start-up file (Windows Startup script, systemd user unit) at
/// `new` when it runs `old`. The running panel is left alone; it picks the new program up at its next start.
pub fn repoint_background_panel(old: &Path, new: &Path) {
    use crate::monitor::panel_service as svc;
    let file = if cfg!(windows) { svc::startup_script_path() } else { svc::unit_path() };
    let Ok(text) = std::fs::read_to_string(&file) else { return };
    if let Some(fixed) = repoint_text(&text, old, new) {
        let _ = std::fs::write(&file, fixed);
        #[cfg(unix)]
        {
            use crate::service::{CommandRunner, SystemRunner};
            let _ = SystemRunner.run("systemctl", &["--user".to_string(), "daemon-reload".to_string()]);
        }
    }
}

/// The status of the per-user shortcuts on the real system.
pub fn user_status() -> UserStatus {
    match UserLayout::from_env() {
        Ok(ul) => user_status_with(&RealSys::new(), &ul),
        Err(_) => UserStatus::default(),
    }
}
