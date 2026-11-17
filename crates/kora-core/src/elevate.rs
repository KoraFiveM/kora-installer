//! Asking for administrator rights ourselves (Windows UAC prompt / `sudo`), so non-technical users
//! never have to "right-click > Run as administrator".
//!
//! * [`admin_reasons`] / [`needs_admin`]: does THIS request need admin, and why (plain language).
//! * [`relaunch_elevated`]: start this program again with the same arguments, elevated.
//! * [`ensure_elevated`]: the decision logic front ends share (with an injectable [`Elevator`]).

use std::ffi::{OsStr, OsString};

use crate::db;
use crate::installer::{DbChoice, InstallRequest, Versions};

/// Set in the elevated copy so it can wait for a key before its console window closes.
pub const RELAUNCHED_ENV: &str = "KORA_RELAUNCHED";

/// Are we running with administrator / root rights?
pub fn is_elevated() -> bool {
    crate::installer::is_admin()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElevateError {
    /// The user said no at the Windows prompt (or sudo password refused).
    Declined,
    Failed(String),
    /// No way to elevate here (no `sudo`, unsupported platform).
    Unsupported,
}

impl std::fmt::Display for ElevateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ElevateError::Declined => f.write_str("administrator permission was not given"),
            ElevateError::Failed(m) => write!(f, "could not ask for administrator permission: {m}"),
            ElevateError::Unsupported => f.write_str("this computer cannot ask for administrator permission from here (no sudo)"),
        }
    }
}

impl std::error::Error for ElevateError {}

// ---------------------------------------------------------------------------------------------
// Reasons
// ---------------------------------------------------------------------------------------------

/// Plain-language reasons this request needs administrator rights (empty = none). A dry run
/// changes nothing, so it needs nothing.
pub fn admin_reasons(req: &InstallRequest) -> Vec<String> {
    let windows = cfg!(windows);
    let db_missing = req.db == DbChoice::InstallIfMissing
        && !req.dry_run
        && !crate::installer::db_override(req)
        && match db::detect(&db::SystemRunner, db::Os::current()) {
            Ok(found) => found.is_empty(),
            Err(_) => true,
        };
    // Windows: a database server is there but no login was given, and Kora's own MariaDB login is
    // kept where only administrators can read it: creating the new database needs them too.
    let kora_login_hidden = windows
        && !db_missing
        && req.db != DbChoice::Skip
        && !req.dry_run
        && !crate::installer::db_override(req)
        && crate::installer::windows_db::base_dir().is_some_and(|b| b.exists())
        && crate::installer::windows_db::saved_admin().is_none();
    admin_reasons_with(req, windows, db_missing, kora_login_hidden)
}

/// Pure part of [`admin_reasons`]: `db_missing` = `--db install` and no database server was found.
pub fn admin_reasons_for(req: &InstallRequest, windows: bool, db_missing: bool) -> Vec<String> {
    admin_reasons_with(req, windows, db_missing, false)
}

/// [`admin_reasons_for`]; `kora_login_hidden` = Windows, Kora's own MariaDB is installed and its
/// administrator login (needed to create the new database) is readable by administrators only.
pub fn admin_reasons_with(req: &InstallRequest, windows: bool, db_missing: bool, kora_login_hidden: bool) -> Vec<String> {
    let mut v = Vec::new();
    if req.dry_run {
        return v;
    }
    if kora_login_hidden {
        v.push("create the new database in Kora's own database server (its administrator login is kept for administrators only)".to_string());
    }
    if db_missing {
        v.push(if windows { "install the database server (MariaDB) as a Windows service".to_string() } else { "install the database server (MariaDB)".to_string() });
    }
    if req.service {
        v.push(if windows { "register the server as a Windows service".to_string() } else { "register the server as a system service (systemd)".to_string() });
    }
    if req.firewall {
        v.push(if req.txadmin_public { "open the game and txAdmin ports in the firewall".to_string() } else { "open the game port in the firewall".to_string() });
    }
    if req.service && req.versions() == Versions::Latest {
        v.push("set up the daily automatic update task".to_string());
    }
    v
}

pub fn needs_admin(req: &InstallRequest) -> bool {
    !admin_reasons(req).is_empty()
}

/// "a", "a and b", "a, b and c".
pub fn join_reasons(reasons: &[String]) -> String {
    match reasons {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

// ---------------------------------------------------------------------------------------------
// Argument quoting (CommandLineToArgvW rules)
// ---------------------------------------------------------------------------------------------

/// Quote one argument so `CommandLineToArgvW` (and the MSVC runtime) reads it back unchanged.
pub fn quote_windows_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.chars().any(|c| matches!(c, ' ' | '\t' | '\n' | '\x0b' | '"')) {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            c => {
                out.push_str(&"\\".repeat(backslashes));
                backslashes = 0;
                out.push(c);
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2)); // trailing backslashes would escape the closing quote
    out.push('"');
    out
}

/// Space-joined quoted arguments (one command-line string).
pub fn join_windows_args(args: &[OsString]) -> String {
    args.iter().map(|a| quote_windows_arg(&a.to_string_lossy())).collect::<Vec<_>>().join(" ")
}

/// PowerShell single-quoted string literal.
pub fn powershell_literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// The PowerShell script that starts `exe args` with the UAC prompt. Exit code 1223 = user declined.
/// Only single quotes are used so the script survives command-line quoting untouched.
pub fn powershell_script(exe: &str, args: &[OsString]) -> String {
    let arg_part = if args.is_empty() { String::new() } else { format!(" -ArgumentList {}", powershell_literal(&join_windows_args(args))) };
    format!(
        "try {{ Start-Process -FilePath {}{} -Verb RunAs -ErrorAction Stop }} catch {{ $e = $_.Exception; $c = 0; while ($e -ne $null -and $c -eq 0) {{ if ($e.NativeErrorCode -eq 1223) {{ $c = 1223 }}; $e = $e.InnerException }}; if ($c -eq 1223) {{ exit 1223 }}; [Console]::Error.WriteLine($_.Exception.Message); exit 1 }}",
        powershell_literal(exe),
        arg_part
    )
}

/// Map the PowerShell helper's exit code to a result.
pub fn interpret_powershell_exit(code: Option<i32>, stderr: &str) -> Result<(), ElevateError> {
    match code {
        Some(0) => Ok(()),
        Some(1223) => Err(ElevateError::Declined),
        _ => {
            let msg = stderr.trim();
            Err(ElevateError::Failed(if msg.is_empty() { "the Windows permission prompt could not be shown".into() } else { msg.to_string() }))
        }
    }
}

/// Environment variables worth carrying through `sudo` (its default resets the environment).
pub const SUDO_KEEP_ENV: &[&str] = &["CFX_LICENSE_KEY", "KORA_DB_ADMIN_PASSWORD", "KORA_CATALOG", "KORA_HOME", "NO_COLOR"];

/// `sudo` arguments (after the program name) for re-running `exe args` as root.
pub fn sudo_args(exe: &OsStr, args: &[OsString], keep_env: &[String]) -> Vec<OsString> {
    let mut v: Vec<OsString> = Vec::new();
    if !keep_env.is_empty() {
        v.push(format!("--preserve-env={}", keep_env.join(",")).into());
    }
    v.push("--".into());
    v.push(exe.to_os_string());
    v.extend(args.iter().cloned());
    v
}

// ---------------------------------------------------------------------------------------------
// Relaunch
// ---------------------------------------------------------------------------------------------

/// Re-run this program elevated with `args` (everything after the program name).
/// Windows: `Ok(())` once the elevated copy has STARTED (it opens its own window); the caller exits 0.
/// Unix: replaces this process with `sudo`; returns only on failure.
pub fn relaunch_elevated(args: &[OsString]) -> Result<(), ElevateError> {
    let exe = std::env::current_exe().map_err(|e| ElevateError::Failed(format!("cannot find the kora program: {e}")))?;
    relaunch_exe(&exe, args)
}

#[cfg(windows)]
fn relaunch_exe(exe: &std::path::Path, args: &[OsString]) -> Result<(), ElevateError> {
    use std::process::Stdio;
    let script = powershell_script(&exe.to_string_lossy(), args);
    let out = crate::sysexe::command("powershell")
        .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &script])
        .env(RELAUNCHED_ENV, "1")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| ElevateError::Failed(format!("could not start PowerShell: {e}")))?;
    interpret_powershell_exit(out.status.code(), &String::from_utf8_lossy(&out.stderr))
}

/// What the `sudo` relaunch carries over: the variables to keep, and the `KORA_HOME` to set first.
/// `sudo` makes `HOME` root's, so the Kora folder (server list, saved answers) is decided here,
/// before elevating, and handed over as `KORA_HOME`; an explicit `KORA_HOME` is simply kept.
pub fn sudo_env(present: &dyn Fn(&str) -> bool, user_kora_home: std::path::PathBuf) -> (Vec<String>, Option<std::path::PathBuf>) {
    let set = crate::userhome::kora_home_for_relaunch(present("KORA_HOME"), user_kora_home);
    let keep = SUDO_KEEP_ENV.iter().filter(|k| present(k) || (**k == "KORA_HOME" && set.is_some())).map(|k| k.to_string()).collect();
    (keep, set)
}

#[cfg(unix)]
fn relaunch_exe(exe: &std::path::Path, args: &[OsString]) -> Result<(), ElevateError> {
    use std::os::unix::process::CommandExt;
    let Some(sudo) = crate::sysexe::unix_sudo() else { return Err(ElevateError::Unsupported) };
    let (keep, set) = sudo_env(&|k| std::env::var_os(k).is_some(), crate::installer::registry::kora_home());
    let mut cmd = std::process::Command::new(sudo);
    if let Some(h) = &set {
        // Made by this user now, so files root writes there later can still be replaced by them.
        let _ = std::fs::create_dir_all(h);
        cmd.env("KORA_HOME", h);
    }
    let err = cmd.args(sudo_args(exe.as_os_str(), args, &keep)).env(RELAUNCHED_ENV, "1").exec();
    Err(ElevateError::Failed(err.to_string()))
}

#[cfg(not(any(windows, unix)))]
fn relaunch_exe(_exe: &std::path::Path, _args: &[OsString]) -> Result<(), ElevateError> {
    Err(ElevateError::Unsupported)
}

// ---------------------------------------------------------------------------------------------
// Shared decision logic
// ---------------------------------------------------------------------------------------------

/// Seam for tests.
pub trait Elevator {
    fn is_elevated(&self) -> bool;
    fn relaunch(&self, args: &[OsString]) -> Result<(), ElevateError>;
}

/// The real thing.
pub struct SystemElevator;

impl Elevator for SystemElevator {
    fn is_elevated(&self) -> bool {
        is_elevated()
    }
    fn relaunch(&self, args: &[OsString]) -> Result<(), ElevateError> {
        relaunch_elevated(args)
    }
}

/// What kind of command is being run (drives the Windows "always elevated" rule).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmdKind {
    Install {
        dry_run: bool,
    },
    Delete,
    Uninstall {
        dry_run: bool,
    },
    Start,
    Stop,
    Restart,
    Update {
        auto: bool,
    },
    Status,
    /// `--version`, `--help`, the Windows service host, anything read-only.
    Other,
}

impl CmdKind {
    /// Interactive and privileged: on Windows these always run elevated. (Bare `kora`, `kora manager` and
    /// `kora install` with no options only open the panel, so they are `Other` and never elevate.)
    pub fn privileged(self) -> bool {
        match self {
            CmdKind::Delete | CmdKind::Start | CmdKind::Stop | CmdKind::Restart => true,
            CmdKind::Install { dry_run } | CmdKind::Uninstall { dry_run } => !dry_run,
            CmdKind::Update { auto } => !auto,
            CmdKind::Status | CmdKind::Other => false,
        }
    }
}

/// The facts the decision depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Env {
    pub windows: bool,
    pub elevated: bool,
    /// stdin AND stdout are terminals.
    pub tty: bool,
    pub no_elevate: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Need {
    /// Carry on in this process.
    Nothing,
    /// Windows: relaunch immediately, no questions.
    Always,
    /// Unix: relaunch because of these reasons.
    Reasons(Vec<String>),
}

/// Decision table. `reasons` = [`admin_reasons`] for the command (only used off Windows).
pub fn decide(env: &Env, kind: CmdKind, reasons: &[String]) -> Need {
    if env.no_elevate || !env.tty || env.elevated {
        return Need::Nothing;
    }
    if env.windows {
        return if kind.privileged() { Need::Always } else { Need::Nothing };
    }
    if reasons.is_empty() {
        Need::Nothing
    } else {
        Need::Reasons(reasons.to_vec())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Elevation {
    /// Carry on in this process.
    Continue,
    /// The elevated copy took over, or the user declined: exit with this code.
    Exit(i32),
}

/// Carry out a [`Need`]: relaunch elevated with `args` and say plain sentences through `say`.
pub fn ensure_elevated(need: &Need, args: &[OsString], elevator: &dyn Elevator, say: &mut dyn FnMut(&str)) -> Elevation {
    match need {
        Need::Nothing => Elevation::Continue,
        Need::Always => {
            say("Kora runs as administrator so it can set up services and the database. Windows will now ask you to approve.");
            match elevator.relaunch(args) {
                Ok(()) => Elevation::Exit(0),
                Err(ElevateError::Declined) => {
                    say("Kora needs administrator permission to run. Nothing was changed. Run it again and choose Yes.");
                    Elevation::Exit(1)
                }
                Err(e) => {
                    say(&format!("{e}. Continuing without it."));
                    Elevation::Continue
                }
            }
        }
        Need::Reasons(reasons) => {
            say(&format!("Kora needs administrator permission to {}. {}", join_reasons(reasons), approve_hint()));
            match elevator.relaunch(args) {
                Ok(()) => Elevation::Exit(0),
                Err(ElevateError::Declined) => {
                    say(declined_message());
                    Elevation::Exit(0)
                }
                Err(e) => {
                    say(&format!("{e}. Continuing without it."));
                    Elevation::Continue
                }
            }
        }
    }
}

/// Keep the console window open until a key is pressed? (Windows only: a window that Kora's
/// own relaunch or a double-click created closes the moment the program ends.)
///
/// Never in session 0 (`no_desktop`): the MSI's custom actions, services and scheduled jobs run there with a console of
/// their own that nobody can see, so "Press Enter" would wait for ever (the installer hung on it).
pub fn should_pause(windows: bool, owns_console: bool, relaunched: bool, stdin_tty: bool, stdout_tty: bool, no_desktop: bool) -> bool {
    windows && !no_desktop && stdin_tty && stdout_tty && (owns_console || relaunched)
}

/// True when this process runs in Windows session 0: no person is looking at its window.
#[cfg(windows)]
pub fn in_session_zero() -> bool {
    #[link(name = "kernel32")]
    extern "system" {
        fn ProcessIdToSessionId(process_id: u32, session_id: *mut u32) -> i32;
        fn GetCurrentProcessId() -> u32;
    }
    let mut session = u32::MAX;
    // SAFETY: `session` is a valid out pointer; the call only reads the process table.
    let ok = unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) };
    ok != 0 && session == 0
}

#[cfg(not(windows))]
pub fn in_session_zero() -> bool {
    false
}

/// True when this process is the only one attached to its console (double-click, or our elevated copy).
#[cfg(windows)]
pub fn owns_console() -> bool {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetConsoleProcessList(list: *mut u32, count: u32) -> u32;
    }
    let mut ids = [0u32; 4];
    // SAFETY: `ids` is a valid buffer of the stated length.
    let n = unsafe { GetConsoleProcessList(ids.as_mut_ptr(), ids.len() as u32) };
    n == 1
}

#[cfg(not(windows))]
pub fn owns_console() -> bool {
    false
}

pub fn was_relaunched() -> bool {
    std::env::var_os(RELAUNCHED_ENV).is_some()
}

fn approve_hint() -> &'static str {
    if cfg!(windows) {
        "Windows will now ask you to approve."
    } else {
        "You may be asked for your password."
    }
}

pub fn declined_message() -> &'static str {
    "No problem, nothing was changed. To go ahead, run the command again and choose Yes when Windows asks. \
     Or avoid needing administrator permission: use `--db existing` (or `--db skip`) and leave out `--service` and `--firewall`."
}

/// The wording of the "still not elevated" error in the prepare step (reached only when elevation
/// was declined, is unavailable, or `--no-elevate` was given).
pub fn not_elevated_db_message(version: &str, folder: &str) -> String {
    format!(
        "No MariaDB/MySQL database server was found, and setting one up needs administrator permission (it creates a Windows service), which Kora did not get. Run the install again and choose Yes when Windows asks for permission (don't use --no-elevate). Kora will then download the official MariaDB {version} from MariaDB's official download mirrors, verify its checksum and put it in {folder}. Or install MariaDB yourself and use --db existing, or use --db skip."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(unused_imports)]
    use crate::installer::Mode;
    use crate::installer::{Ports, ResourceSelection};
    use std::cell::{Cell, RefCell};

    fn req() -> InstallRequest {
        InstallRequest {
            source: None,
            versions: None,
            mode: Mode::Production,
            name: "s".into(),
            dir: "d".into(),
            hostname: String::new(),
            license_key: None,
            license_key_name: None,
            cfx_identifier: None,
            ports: Ports::default(),
            txadmin_public: false,
            artifact_url: None,
            game_build: None,
            resources: ResourceSelection::All,
            db: DbChoice::UseExisting,
            service: false,
            firewall: false,
            github_owner: None,
            catalog: None,
            dry_run: false,
            db_admin: None,
            start: false,
            app: None,
        }
    }

    fn q(s: &str) -> String {
        quote_windows_arg(s)
    }

    #[test]
    fn quoting_follows_commandlinetoargv_rules() {
        assert_eq!(q("plain"), "plain");
        assert_eq!(q(""), "\"\"");
        assert_eq!(q("C:\\Program Files\\Kora"), "\"C:\\Program Files\\Kora\"");
        assert_eq!(q("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(q("a&b"), "a&b", "& needs no CommandLineToArgvW quoting (no shell involved)");
        assert_eq!(q("My & Server"), "\"My & Server\"");
        assert_eq!(q("C:\\dir with space\\"), "\"C:\\dir with space\\\\\"", "trailing backslash doubled");
        assert_eq!(q("C:\\nospace\\"), "C:\\nospace\\", "no quotes, so backslashes stay");
        assert_eq!(q("a\\\"b"), "\"a\\\\\\\"b\"", "backslashes before a quote are doubled, quote escaped");
        assert_eq!(q("Café 日本"), "\"Café 日本\"");
        assert_eq!(q("naïve"), "naïve");
    }

    /// Reference decoder implementing CommandLineToArgvW for arguments after argv[0].
    fn parse(cmd: &str) -> Vec<String> {
        let chars: Vec<char> = cmd.chars().collect();
        let (mut i, mut out) = (0, Vec::new());
        while i < chars.len() {
            while i < chars.len() && (chars[i] == ' ' || chars[i] == '\t') {
                i += 1;
            }
            if i >= chars.len() {
                break;
            }
            let (mut arg, mut in_q) = (String::new(), false);
            while i < chars.len() {
                let mut bs = 0;
                while i < chars.len() && chars[i] == '\\' {
                    bs += 1;
                    i += 1;
                }
                if i < chars.len() && chars[i] == '"' {
                    arg.push_str(&"\\".repeat(bs / 2));
                    if bs % 2 == 1 {
                        arg.push('"');
                    } else {
                        in_q = !in_q;
                    }
                    i += 1;
                } else {
                    arg.push_str(&"\\".repeat(bs));
                    if i >= chars.len() {
                        break;
                    }
                    if !in_q && (chars[i] == ' ' || chars[i] == '\t') {
                        break;
                    }
                    arg.push(chars[i]);
                    i += 1;
                }
            }
            out.push(arg);
        }
        out
    }

    #[test]
    fn quoted_arguments_round_trip() {
        let args: Vec<OsString> =
            ["install", "--name", "My \"Best\" Server", "--dir", "C:\\Users\\Jo Smith\\kora\\", "--license-key", "a&b|c^d%PATH%", "--x", "", "ünï 日本語 🎮", "back\\\\slash", "end\\\\", "q\\\"x"]
                .iter()
                .map(OsString::from)
                .collect();
        let line = join_windows_args(&args);
        let back = parse(&line);
        let want: Vec<String> = args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(back, want, "{line}");
    }

    #[test]
    fn powershell_script_is_single_quoted_and_escapes_apostrophes() {
        let s = powershell_script("C:\\Users\\O'Neil\\kora.exe", &["install".into(), "--name".into(), "Bob's \"srv\"".into()]);
        assert!(s.contains("-FilePath 'C:\\Users\\O''Neil\\kora.exe'"), "{s}");
        assert!(s.contains("-ArgumentList 'install --name \"Bob''s \\\"srv\\\"\"'"), "{s}");
        assert!(s.contains("-Verb RunAs"));
        assert!(s.contains("1223"));
        let none = powershell_script("k.exe", &[]);
        assert!(!none.contains("-ArgumentList"), "an empty argument list is an error in Start-Process");
    }

    #[test]
    fn powershell_exit_codes_map_to_errors() {
        assert_eq!(interpret_powershell_exit(Some(0), ""), Ok(()));
        assert_eq!(interpret_powershell_exit(Some(1223), ""), Err(ElevateError::Declined));
        assert!(matches!(interpret_powershell_exit(Some(1), " boom \n"), Err(ElevateError::Failed(m)) if m == "boom"));
        assert!(matches!(interpret_powershell_exit(None, ""), Err(ElevateError::Failed(_))));
    }

    #[test]
    fn sudo_arguments_keep_args_after_double_dash() {
        let a = sudo_args(OsStr::new("/bin/kora"), &["install".into(), "--name".into(), "x y".into()], &["CFX_LICENSE_KEY".into()]);
        let a: Vec<String> = a.iter().map(|s| s.to_string_lossy().into_owned()).collect();
        assert_eq!(a, ["--preserve-env=CFX_LICENSE_KEY", "--", "/bin/kora", "install", "--name", "x y"]);
        let b = sudo_args(OsStr::new("k"), &[], &[]);
        assert_eq!(b.len(), 2);
    }

    // Linux paths and sudo: only meaningful on Unix (on Windows `/home/..` is not absolute).
    #[cfg(unix)]
    #[test]
    fn the_sudo_relaunch_pins_the_users_kora_home() {
        let home = std::path::PathBuf::from("/home/sam/.kora");
        let (keep, set) = sudo_env(&|k| k == "NO_COLOR", home.clone());
        assert_eq!(keep, ["KORA_HOME", "NO_COLOR"]);
        assert_eq!(set, Some(home.clone()));
        let a = sudo_args(OsStr::new("/bin/kora"), &["install".into()], &keep);
        assert_eq!(a[0], OsString::from("--preserve-env=KORA_HOME,NO_COLOR"));
        // already set: kept as it is, not replaced
        let (keep, set) = sudo_env(&|k| k == "KORA_HOME", home);
        assert_eq!((keep, set), (vec!["KORA_HOME".to_string()], None));
        // nothing usable: nothing pinned
        let (keep, set) = sudo_env(&|_| false, ".kora".into());
        assert!(keep.is_empty() && set.is_none());
    }

    #[test]
    fn reasons_cover_each_privileged_thing() {
        let mut r = req();
        assert!(admin_reasons_for(&r, true, false).is_empty());
        r.db = DbChoice::InstallIfMissing;
        let v = admin_reasons_for(&r, true, true);
        assert_eq!(v, ["install the database server (MariaDB) as a Windows service"]);
        assert!(admin_reasons_for(&r, true, false).is_empty(), "a database is already there");
        r.service = true;
        r.firewall = true;
        let v = admin_reasons_for(&r, true, false);
        assert_eq!(v, ["register the server as a Windows service", "open the game port in the firewall"]);
        r.txadmin_public = true;
        r.mode = Mode::Development;
        let v = admin_reasons_for(&r, false, false);
        assert_eq!(v.len(), 3);
        assert!(v[0].contains("systemd") && v[1].contains("txAdmin") && v[2].contains("update"));
        r.dry_run = true;
        assert!(admin_reasons_for(&r, true, true).is_empty(), "a dry run changes nothing");
    }

    #[test]
    fn kora_s_own_database_login_is_a_reason_on_windows() {
        let r = InstallRequest { db: DbChoice::UseExisting, ..req() };
        let v = admin_reasons_with(&r, true, false, true);
        assert_eq!(v.len(), 1);
        assert!(v[0].contains("Kora's own database server"), "{v:?}");
        assert!(admin_reasons_with(&InstallRequest { dry_run: true, ..r }, true, false, true).is_empty());
    }

    #[test]
    fn reasons_are_joined_in_plain_language() {
        let s = |v: &[&str]| join_reasons(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(s(&["a"]), "a");
        assert_eq!(s(&["a", "b"]), "a and b");
        assert_eq!(s(&["a", "b", "c"]), "a, b and c");
    }

    struct Fake {
        elevated: bool,
        result: Result<(), ElevateError>,
        calls: Cell<u32>,
        seen: RefCell<Vec<OsString>>,
    }

    impl Fake {
        fn new(elevated: bool, result: Result<(), ElevateError>) -> Self {
            Fake { elevated, result, calls: Cell::new(0), seen: RefCell::new(vec![]) }
        }
    }

    impl Elevator for Fake {
        fn is_elevated(&self) -> bool {
            self.elevated
        }
        fn relaunch(&self, args: &[OsString]) -> Result<(), ElevateError> {
            self.calls.set(self.calls.get() + 1);
            *self.seen.borrow_mut() = args.to_vec();
            self.result.clone()
        }
    }

    fn go(f: &Fake, need: Need) -> (Elevation, Vec<String>) {
        let mut said = Vec::new();
        let args: Vec<OsString> = vec!["install".into(), "--code".into(), "KORA-1".into()];
        let r = ensure_elevated(&need, &args, f, &mut |m| said.push(m.to_string()));
        (r, said)
    }

    #[test]
    fn relaunches_with_the_original_args_and_exits_zero() {
        let f = Fake::new(false, Ok(()));
        let (r, said) = go(&f, Need::Reasons(vec!["register the server as a Windows service".into()]));
        assert_eq!(r, Elevation::Exit(0));
        assert_eq!(f.calls.get(), 1);
        assert_eq!(*f.seen.borrow(), ["install", "--code", "KORA-1"].map(OsString::from));
        assert!(said[0].starts_with("Kora needs administrator permission to register the server as a Windows service."), "{said:?}");
    }

    #[test]
    fn declined_with_reasons_prints_the_alternatives_and_exits_zero() {
        let f = Fake::new(false, Err(ElevateError::Declined));
        let (r, said) = go(&f, Need::Reasons(vec!["x".into()]));
        assert_eq!(r, Elevation::Exit(0));
        assert!(said.last().unwrap().contains("--db existing"), "{said:?}");
    }

    #[test]
    fn always_mode_says_why_then_relaunches() {
        let f = Fake::new(false, Ok(()));
        let (r, said) = go(&f, Need::Always);
        assert_eq!(r, Elevation::Exit(0));
        assert_eq!(said, ["Kora runs as administrator so it can set up services and the database. Windows will now ask you to approve."]);
        assert_eq!(*f.seen.borrow(), ["install", "--code", "KORA-1"].map(OsString::from), "args preserved, code not consumed");
    }

    #[test]
    fn always_mode_declined_exits_one_with_a_friendly_message() {
        let f = Fake::new(false, Err(ElevateError::Declined));
        let (r, said) = go(&f, Need::Always);
        assert_eq!(r, Elevation::Exit(1));
        assert_eq!(said.last().unwrap(), "Kora needs administrator permission to run. Nothing was changed. Run it again and choose Yes.");
    }

    #[test]
    fn unsupported_or_failed_continues_so_the_clear_error_shows() {
        for need in [Need::Always, Need::Reasons(vec!["x".into()])] {
            for e in [ElevateError::Unsupported, ElevateError::Failed("x".into())] {
                let f = Fake::new(false, Err(e));
                assert_eq!(go(&f, need.clone()).0, Elevation::Continue);
            }
        }
    }

    #[test]
    fn nothing_needed_means_no_relaunch_and_no_output() {
        let f = Fake::new(false, Ok(()));
        let (r, said) = go(&f, Need::Nothing);
        assert_eq!((r, f.calls.get(), said.len()), (Elevation::Continue, 0, 0));
    }

    const ALL: [CmdKind; 10] = [
        CmdKind::Install { dry_run: false },
        CmdKind::Install { dry_run: true },
        CmdKind::Delete,
        CmdKind::Uninstall { dry_run: false },
        CmdKind::Uninstall { dry_run: true },
        CmdKind::Start,
        CmdKind::Update { auto: false },
        CmdKind::Update { auto: true },
        CmdKind::Status,
        CmdKind::Other,
    ];

    #[test]
    fn windows_decision_table() {
        let reasons: Vec<String> = vec![];
        for kind in ALL {
            let base = Env { windows: true, elevated: false, tty: true, no_elevate: false };
            let want = if kind.privileged() { Need::Always } else { Need::Nothing };
            assert_eq!(decide(&base, kind, &reasons), want, "{kind:?}");
            // elevated, no tty, --no-elevate: never relaunch
            for env in [Env { elevated: true, ..base }, Env { tty: false, ..base }, Env { no_elevate: true, ..base }] {
                assert_eq!(decide(&env, kind, &reasons), Need::Nothing, "{kind:?} {env:?}");
            }
        }
        let privileged: Vec<CmdKind> = ALL.into_iter().filter(|k| k.privileged()).collect();
        assert_eq!(privileged, [CmdKind::Install { dry_run: false }, CmdKind::Delete, CmdKind::Uninstall { dry_run: false }, CmdKind::Start, CmdKind::Update { auto: false }]);
        assert!(CmdKind::Stop.privileged() && CmdKind::Restart.privileged());
        assert!(!CmdKind::Status.privileged() && !CmdKind::Other.privileged(), "--version/--help/status stay unelevated");
    }

    #[test]
    fn unix_decision_follows_the_reasons() {
        let env = Env { windows: false, elevated: false, tty: true, no_elevate: false };
        let r = vec!["open the game port in the firewall".to_string()];
        for kind in ALL {
            assert_eq!(decide(&env, kind, &[]), Need::Nothing);
            assert_eq!(decide(&env, kind, &r), Need::Reasons(r.clone()));
            assert_eq!(decide(&Env { tty: false, ..env }, kind, &r), Need::Nothing, "headless: no prompts");
            assert_eq!(decide(&Env { elevated: true, ..env }, kind, &r), Need::Nothing);
            assert_eq!(decide(&Env { no_elevate: true, ..env }, kind, &r), Need::Nothing);
        }
    }

    #[test]
    fn pause_only_for_windows_consoles_that_would_close() {
        // (windows, owns_console, relaunched, stdin_tty, stdout_tty)
        assert!(should_pause(true, true, false, true, true, false), "double-click");
        assert!(should_pause(true, false, true, true, true, false), "our elevated copy");
        assert!(!should_pause(true, false, false, true, true, false), "ran from an open terminal");
        assert!(!should_pause(true, true, true, false, true, false), "no stdin: scheduler/CI, never block");
        assert!(!should_pause(true, true, true, true, false, false), "output piped");
        assert!(!should_pause(false, true, true, true, true, false), "not Windows");
        assert!(!should_pause(true, true, true, true, true, true), "session 0 (the installer's actions, services): nobody can press Enter");
    }

    #[test]
    fn prepare_message_matches_reality() {
        let m = not_elevated_db_message("11.4", "C:\\ProgramData\\Kora\\mariadb");
        assert!(!m.contains("right-click"));
        assert!(m.contains("--no-elevate") && m.contains("--db skip") && m.contains("11.4"));
    }

    /// GitHub's Windows runners are already elevated (so the UAC path itself cannot run there).
    #[cfg(windows)]
    #[test]
    fn ci_runner_is_elevated_and_the_powershell_script_parses() {
        if std::env::var_os("GITHUB_ACTIONS").is_some() {
            assert!(is_elevated(), "the runner is expected to be elevated");
        }
        let script = powershell_script("C:\\Program Files\\O'Neil\\kora.exe", &["install".into(), "--name".into(), "A & \"B\" 日本".into(), "--dir".into(), "C:\\x y\\".into()]);
        let check = format!("$tokens = $null; $errs = $null; $null = [System.Management.Automation.Language.Parser]::ParseInput({}, [ref]$tokens, [ref]$errs); if ($errs.Count -gt 0) {{ $errs | ForEach-Object {{ [Console]::Error.WriteLine($_.Message) }}; exit 1 }}", powershell_literal(&script));
        let out = std::process::Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", &check]).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }
}
