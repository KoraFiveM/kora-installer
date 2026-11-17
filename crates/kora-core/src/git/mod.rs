//! git/gh bootstrap and wrapper; shells out to `git` and `gh` (no embedded git library).
//!
//! Every external command goes through [`CommandRunner`], so tests use a fake and nothing here
//! talks to the real GitHub. Bootstrap installation is only *planned* ([`plan_bootstrap`]); the
//! caller decides when (and with which privileges) to run the described steps.

pub mod branches;
pub mod forkfind;
pub mod owners;

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Errors from the git/gh layer.
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("could not run `{0}`: {1}")]
    Spawn(String, String),
    #[error("`{cmd}` failed (exit {code}): {stderr}")]
    Command { cmd: String, code: i32, stderr: String },
    #[error("invalid value for {0}: {1:?}")]
    Invalid(&'static str, String),
    #[error("could not parse output of `{0}`: {1:?}")]
    Parse(String, String),
}

pub type Result<T> = std::result::Result<T, GitError>;

/// Captured result of an external command.
#[derive(Debug, Clone, Default)]
pub struct Output {
    /// Exit code (`-1` if killed by a signal).
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn ok(&self) -> bool {
        self.code == 0
    }
}

/// Abstraction over process execution and sleeping (so backoff is testable without waiting).
pub trait CommandRunner {
    fn run(&self, program: &str, args: &[&str], cwd: Option<&Path>) -> std::io::Result<Output>;
    fn sleep(&self, d: Duration);
    /// An independent copy that can run on another thread (None: this runner is single-threaded, so callers stay sequential).
    fn fork(&self) -> Option<Box<dyn CommandRunner + Send>> {
        None
    }
}

impl CommandRunner for Box<dyn CommandRunner + Send> {
    fn run(&self, program: &str, args: &[&str], cwd: Option<&Path>) -> std::io::Result<Output> {
        (**self).run(program, args, cwd)
    }
    fn sleep(&self, d: Duration) {
        (**self).sleep(d)
    }
    fn fork(&self) -> Option<Box<dyn CommandRunner + Send>> {
        (**self).fork()
    }
}

/// Real runner using `std::process::Command`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn run(&self, program: &str, args: &[&str], cwd: Option<&Path>) -> std::io::Result<Output> {
        let mut c = system_command(program, args, cwd);
        // Never block on interactive prompts from git.
        c.env("GIT_TERMINAL_PROMPT", "0");
        // Only when no other layer (the sudo / root wrappers, or the user) already set `GIT_CONFIG_COUNT`
        // (`windows_git_env` checks the process; the wrappers' own lists are on the command itself).
        let own = std::env::var("GIT_CONFIG_COUNT").ok();
        let wrapped = c.get_envs().any(|(k, _)| k == "GIT_CONFIG_COUNT");
        let cred = crate::tools::gh_credential_helper().filter(|_| program == "git" && own.is_none() && !wrapped);
        for (k, v) in git_config_env(cfg!(windows), own.as_deref(), cred.as_deref()) {
            c.env(k, v);
        }
        let o = c.output()?;
        Ok(Output {
            code: o.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        })
    }
    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }
    fn fork(&self) -> Option<Box<dyn CommandRunner + Send>> {
        Some(Box::new(SystemRunner))
    }
}

// ---------------------------------------------------------------------------------------------
// Under sudo: GitHub as the person who ran sudo
// ---------------------------------------------------------------------------------------------

/// What actually runs for one `git` / `gh` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
}

/// `runuser` from root-owned folders only.
pub fn unix_runuser() -> Option<PathBuf> {
    ["/usr/sbin/runuser", "/sbin/runuser", "/usr/bin/runuser", "/bin/runuser"].iter().map(PathBuf::from).find(|p| p.is_file())
}

/// Under `sudo` the GitHub sign-in that counts is the user's own (root usually has none, and a
/// sign-in made as root would be stored in root's folder). With `user` = the account behind sudo
/// and `runuser` available:
/// * `gh ...` runs as that user (`runuser -u <user> -- gh ...`, in their home folder when no folder
///   is given), so it uses and stores their GitHub sign-in;
/// * `gh repo clone <repo> <dest>` becomes `git clone` as root (the server folder belongs to root
///   during the install), with git asking the user's `gh` for the credentials, like
///   `gh auth setup-git` does;
/// * `git ...` asks the user's `gh` for GitHub credentials the same way (private forks).
///
/// Anything else, or no sudo user / no runuser: unchanged. `git_config_count` = the caller's own
/// `GIT_CONFIG_COUNT` (then git is left alone). `bus` = the user's session bus socket, if any
/// (gh may keep the sign-in in the desktop keyring).
pub fn sudo_invocation(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
    user: Option<&crate::userhome::Account>,
    runuser: Option<&Path>,
    git_config_count: Option<&str>,
    bus: Option<&Path>,
) -> Invocation {
    let plain = Invocation { program: program.into(), args: args.iter().map(|s| s.to_string()).collect(), cwd: cwd.map(Path::to_path_buf), env: vec![] };
    let (Some(u), Some(ru)) = (user, runuser) else { return plain };
    if u.uid == 0 || !crate::userhome::valid_user_name(&u.name) {
        return plain;
    }
    let ru = ru.to_string_lossy().into_owned();
    let helper = |env: &mut Vec<(String, String)>| {
        if git_config_count.is_none() {
            let key = "credential.https://github.com.helper".to_string();
            env.extend([
                ("GIT_CONFIG_COUNT".to_string(), "2".to_string()),
                ("GIT_CONFIG_KEY_0".to_string(), key.clone()),
                ("GIT_CONFIG_VALUE_0".to_string(), String::new()),
                ("GIT_CONFIG_KEY_1".to_string(), key),
                ("GIT_CONFIG_VALUE_1".to_string(), format!("!{ru} -u {} -- {} auth git-credential", u.name, crate::tools::program_for("gh"))),
            ]);
        }
    };
    match program {
        "gh" if args.len() == 4 && args[0] == "repo" && args[1] == "clone" && valid_repo(args[2]) => {
            let mut env = Vec::new();
            helper(&mut env);
            Invocation {
                program: "git".into(),
                args: vec!["clone".into(), "--".into(), format!("https://github.com/{}.git", args[2]), args[3].to_string()],
                cwd: cwd.map(Path::to_path_buf),
                env,
            }
        }
        "gh" => {
            let mut a = vec!["-u".to_string(), u.name.clone(), "--".to_string(), crate::tools::program_for("gh")];
            a.extend(args.iter().map(|s| s.to_string()));
            let mut env = Vec::new();
            if let Some(b) = bus {
                env.push(("DBUS_SESSION_BUS_ADDRESS".to_string(), format!("unix:path={}", b.display())));
                if let Some(dir) = b.parent() {
                    env.push(("XDG_RUNTIME_DIR".to_string(), dir.display().to_string()));
                }
            }
            Invocation { program: ru, args: a, cwd: Some(cwd.map(Path::to_path_buf).unwrap_or_else(|| u.home.clone())), env }
        }
        "git" => {
            let mut inv = plain;
            helper(&mut inv.env);
            inv
        }
        _ => plain,
    }
}

/// Environment for a `git` that runs as root. Git reads more than the repository from its config
/// files: `core.hooksPath` and `core.fsmonitor` name programs it runs. Under `sudo` or the panel's
/// helper `HOME` (and so `~/.gitconfig`) can be an ordinary user's, and a repository's own
/// `.git/config` can be written by whoever owns the folder, so root git would run their programs.
/// `isolate` (HOME may not be root's own): the system and global config files are not read at all.
/// Always: hooks point at `/dev/null` and fsmonitor is off, overriding every config file. The
/// user's `gh` sign-in still works for private forks: `credential` is the helper command that asks
/// it (see [`sudo_invocation`]), passed the same way.
pub fn root_git_env(isolate: bool, credential: Option<&str>) -> Vec<(String, String)> {
    let mut env = Vec::new();
    // Environment variables git also turns into programs or prompts.
    for (k, v) in [("GIT_TERMINAL_PROMPT", "0"), ("GIT_SSH_COMMAND", "ssh"), ("GIT_EXTERNAL_DIFF", ""), ("GIT_ASKPASS", ""), ("GIT_PROXY_COMMAND", ""), ("GIT_EDITOR", "true"), ("GIT_PAGER", "cat")] {
        env.push((k.to_string(), v.to_string()));
    }
    if isolate {
        env.push(("GIT_CONFIG_NOSYSTEM".to_string(), "1".to_string()));
        env.push(("GIT_CONFIG_GLOBAL".to_string(), "/dev/null".to_string()));
    }
    // Every repository-local setting that makes git run a program. Command-line (env) config beats
    // `.git/config`, so these cannot be set by whoever owns the repository folder. (`filter.*` drivers
    // cannot be blanked by name; they only run on a checkout of a repo whose `.gitattributes` names one.)
    let mut pairs: Vec<(String, String)> = [
        ("core.hooksPath", "/dev/null"),
        ("core.fsmonitor", "false"),
        ("core.sshCommand", "ssh"),
        ("core.gitProxy", ""),
        ("core.askPass", ""),
        ("core.pager", "cat"),
        ("core.editor", "true"),
        ("diff.external", ""),
        ("gpg.program", "gpg"),
        ("protocol.ext.allow", "never"),
        ("uploadpack.packObjectsHook", ""),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    // A repo-local `credential.helper` is reset for every host (an empty value clears the list).
    pairs.push(("credential.helper".to_string(), String::new()));
    if let Some(c) = credential {
        let key = "credential.https://github.com.helper".to_string();
        pairs.push((key.clone(), String::new()));
        pairs.push((key, c.to_string()));
    }
    env.push(("GIT_CONFIG_COUNT".to_string(), pairs.len().to_string()));
    for (i, (k, v)) in pairs.into_iter().enumerate() {
        env.push((format!("GIT_CONFIG_KEY_{i}"), k));
        env.push((format!("GIT_CONFIG_VALUE_{i}"), v));
    }
    env
}

/// [`sudo_invocation`] for a process running as root (`root = Some(isolate)`, see [`root_git_env`]):
/// every `git` it ends up running gets the root-safe configuration (replacing any `GIT_CONFIG_*`
/// list, also the caller's own). `None` (not root): unchanged.
pub fn harden_for_root(mut inv: Invocation, root: Option<bool>, user: Option<&crate::userhome::Account>, runuser: Option<&Path>) -> Invocation {
    let Some(isolate) = root else { return inv };
    if inv.program != "git" {
        return inv;
    }
    inv.env.retain(|(k, _)| !k.starts_with("GIT_CONFIG_"));
    let cred = match (user, runuser) {
        (Some(u), Some(ru)) if u.uid != 0 && crate::userhome::valid_user_name(&u.name) => Some(format!("!{} -u {} -- {} auth git-credential", ru.to_string_lossy(), u.name, crate::tools::program_for("gh"))),
        _ => None,
    };
    inv.env.extend(root_git_env(isolate, cred.as_deref()));
    inv
}

/// Running as root? `Some(isolate)`: isolate the user-level git config when `HOME` may belong to
/// someone else (under sudo / the helper, or a `HOME` root does not own). `None` otherwise.
fn root_mode(sudo_user: bool) -> Option<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: geteuid has no preconditions.
        if unsafe { libc::geteuid() } != 0 {
            return None;
        }
        let home_is_roots = std::env::var_os("HOME").and_then(|h| std::fs::metadata(h).ok()).is_some_and(|m| m.uid() == 0);
        Some(sudo_user || !home_is_roots)
    }
    #[cfg(not(unix))]
    {
        let _ = sudo_user;
        windows_root_mode(crate::elevate::is_elevated())
    }
}

/// Windows: an elevated Kora hardens every git call like root's on Linux (no hooks, no fsmonitor,
/// no repository-chosen programs), because the repositories are in folders the person and the
/// server's own account can change. The person's own git settings are not isolated (their files
/// are theirs), but the hardening settings beat them too.
pub fn windows_root_mode(elevated: bool) -> Option<bool> {
    elevated.then_some(false)
}

/// The account behind `sudo`, `runuser` and the account's session bus (all `None` when not under sudo).
fn sudo_context() -> (Option<crate::userhome::Account>, Option<PathBuf>, Option<PathBuf>) {
    if !cfg!(unix) {
        return (None, None, None);
    }
    let Some(u) = crate::userhome::sudo_account() else { return (None, None, None) };
    let bus = PathBuf::from(format!("/run/user/{}/bus", u.uid));
    let bus = (std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() && bus.exists()).then_some(bus);
    (Some(u), unix_runuser(), bus)
}

/// `std::process::Command` for one git / gh call, run as the real user where that matters (see [`sudo_invocation`]).
pub fn system_command(program: &str, args: &[&str], cwd: Option<&Path>) -> std::process::Command {
    let (user, runuser, bus) = sudo_context();
    let inv = sudo_invocation(program, args, cwd, user.as_ref(), runuser.as_deref(), std::env::var("GIT_CONFIG_COUNT").ok().as_deref(), bus.as_deref());
    let inv = harden_for_root(inv, root_mode(user.is_some()), user.as_ref(), runuser.as_deref());
    // `git` / `gh`: a system copy, else Kora's own (never depends on this process's PATH being up to date).
    let program = crate::tools::program_for(&inv.program);
    let mut c = crate::sysexe::command(&program);
    c.args(&inv.args);
    if let Some(d) = &inv.cwd {
        c.current_dir(d);
    }
    for (k, v) in inv.env {
        c.env(k, v);
    }
    // Kora's own Git in front of the PATH, so `gh` (which starts `git`) finds it too.
    if let Some(p) = crate::tools::child_path() {
        c.env("PATH", p);
    }
    c
}

/// [`windows_git_env`] plus, when Kora's own `gh` is the one in use, the credential helper that asks it for
/// GitHub sign-in details (the private copy is in no global git config, so every Kora git call carries it).
/// `existing_count`: the caller's own `GIT_CONFIG_COUNT` (then git is left alone).
pub fn git_config_env(windows: bool, existing_count: Option<&str>, credential: Option<&str>) -> Vec<(String, String)> {
    if existing_count.is_some() {
        return vec![];
    }
    let mut pairs: Vec<(String, String)> = Vec::new();
    if windows {
        pairs.push(("core.longpaths".into(), "true".into()));
    }
    if let Some(c) = credential {
        let key = "credential.https://github.com.helper".to_string();
        pairs.push((key.clone(), String::new()));
        pairs.push((key, c.to_string()));
    }
    if pairs.is_empty() {
        return vec![];
    }
    let mut env = vec![("GIT_CONFIG_COUNT".to_string(), pairs.len().to_string())];
    for (i, (k, v)) in pairs.into_iter().enumerate() {
        env.push((format!("GIT_CONFIG_KEY_{i}"), k));
        env.push((format!("GIT_CONFIG_VALUE_{i}"), v));
    }
    env
}

/// Windows: allow paths over 260 characters (node_modules inside resources routinely exceed it, and
/// Git for Windows refuses them by default). Passed as environment config so no user config is
/// touched; skipped when the user already uses `GIT_CONFIG_COUNT` themselves.
pub fn windows_git_env(windows: bool, existing_count: Option<&str>) -> Vec<(&'static str, String)> {
    if !windows || existing_count.is_some() {
        return vec![];
    }
    vec![("GIT_CONFIG_COUNT", "1".into()), ("GIT_CONFIG_KEY_0", "core.longpaths".into()), ("GIT_CONFIG_VALUE_0", "true".into())]
}

fn run(r: &dyn CommandRunner, prog: &str, args: &[&str], cwd: Option<&Path>) -> Result<Output> {
    r.run(prog, args, cwd)
        .map_err(|e| GitError::Spawn(format!("{prog} {}", args.join(" ")), e.to_string()))
}

fn run_ok(r: &dyn CommandRunner, prog: &str, args: &[&str], cwd: Option<&Path>) -> Result<Output> {
    let o = run(r, prog, args, cwd)?;
    if o.ok() {
        Ok(o)
    } else {
        Err(GitError::Command {
            cmd: format!("{prog} {}", args.join(" ")),
            code: o.code,
            stderr: o.stderr.trim().to_string(),
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Validation (values end up as command arguments)
// ---------------------------------------------------------------------------------------------

fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// `owner/name`, both parts safe.
fn valid_repo(s: &str) -> bool {
    // The value is always passed as `owner/name` (never starting with the repo name), and real
    // repositories such as `KoraFiveM/-cfx-default-` start with a dash, so only the owner is
    // checked for a leading dash.
    let name_ok = |n: &str| !n.is_empty() && n != "." && n != ".." && n.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    match s.split_once('/') {
        Some((o, n)) => valid_name(o) && name_ok(n),
        None => false,
    }
}

fn valid_ref(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
}

fn valid_commit(s: &str) -> bool {
    (7..=64).contains(&s.len()) && s.chars().all(|c| c.is_ascii_hexdigit())
}

fn check(what: &'static str, ok: bool, v: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(GitError::Invalid(what, v.to_string()))
    }
}

// ---------------------------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolInfo {
    pub version: String,
    pub path: Option<PathBuf>,
}

/// Detected tool status; `None` means not installed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tools {
    pub git: Option<ToolInfo>,
    pub gh: Option<ToolInfo>,
}

fn locate(r: &dyn CommandRunner, tool: &str) -> Option<PathBuf> {
    let (prog, args): (&str, [&str; 1]) =
        if cfg!(windows) { ("where", [tool]) } else { ("which", [tool]) };
    let o = r.run(prog, &args, None).ok()?;
    if !o.ok() {
        return None;
    }
    o.stdout.lines().map(str::trim).find(|l| !l.is_empty()).map(PathBuf::from)
}

fn detect_tool(r: &dyn CommandRunner, tool: &str) -> Option<ToolInfo> {
    let o = r.run(tool, &["--version"], None).ok()?;
    if !o.ok() {
        return None;
    }
    let version = parse_version(&o.stdout)?;
    let path = locate(r, tool).or_else(|| crate::tools::tool_of(tool).and_then(crate::tools::locate).map(|f| f.path));
    Some(ToolInfo { version, path })
}

/// First token that starts with a digit on the first line:
/// `git version 2.43.0` -> `2.43.0`, `gh version 2.40.1 (2024-01-01)` -> `2.40.1`.
pub fn parse_version(out: &str) -> Option<String> {
    out.lines()
        .next()?
        .split_whitespace()
        .find(|t| t.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(str::to_string)
}

/// Where winget puts Git for Windows and the GitHub CLI. A program installed a moment ago is not on
/// the PATH of the running process (Windows only updates the PATH of programs started afterwards).
pub fn windows_tool_dirs(env: &dyn Fn(&str) -> Option<String>) -> Vec<PathBuf> {
    let mut v = Vec::new();
    for var in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
        if let Some(p) = env(var) {
            v.push(PathBuf::from(&p).join("Git").join("cmd"));
            v.push(PathBuf::from(&p).join("GitHub CLI"));
        }
    }
    if let Some(l) = env("LOCALAPPDATA") {
        v.push(PathBuf::from(&l).join("Programs").join("Git").join("cmd"));
        v.push(PathBuf::from(&l).join("Programs").join("GitHub CLI"));
    }
    v
}

/// `current` PATH with the existing `extra` folders appended (compared without regard to case);
/// `None` when nothing has to change.
pub fn path_with(current: &str, extra: &[PathBuf], exists: &dyn Fn(&Path) -> bool, sep: char) -> Option<String> {
    let have: Vec<String> = current.split(sep).map(|e| e.trim_end_matches(['\\', '/']).to_lowercase()).collect();
    let mut out = current.to_string();
    let mut changed = false;
    for d in extra {
        let key = d.to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase();
        if exists(d) && !have.contains(&key) && !out.split(sep).any(|e| e.trim_end_matches(['\\', '/']).to_lowercase() == key) {
            if !out.is_empty() && !out.ends_with(sep) {
                out.push(sep);
            }
            out.push_str(&d.to_string_lossy());
            changed = true;
        }
    }
    changed.then_some(out)
}

/// Make a just-installed git/gh findable in this process (Windows only; no-op elsewhere).
pub fn refresh_windows_path() {
    if !cfg!(windows) {
        return;
    }
    let dirs = windows_tool_dirs(&|k| std::env::var(k).ok());
    let cur = std::env::var("PATH").unwrap_or_default();
    if let Some(n) = path_with(&cur, &dirs, &|p| p.is_dir(), ';') {
        std::env::set_var("PATH", n);
    }
}

pub fn detect(r: &dyn CommandRunner) -> Tools {
    refresh_windows_path();
    Tools { git: detect_tool(r, "git"), gh: detect_tool(r, "gh") }
}

// ---------------------------------------------------------------------------------------------
// Bootstrap planning
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Os {
    /// Debian 12+/Ubuntu 22.04+ (apt).
    DebianLike,
    Windows,
    Unsupported,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "linux") && Path::new("/usr/bin/apt-get").exists() {
            Os::DebianLike
        } else {
            Os::Unsupported
        }
    }
}

/// One described bootstrap step. Not executed by planning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanStep {
    /// Plain-language description shown before elevation.
    pub description: String,
    pub needs_admin: bool,
    pub program: String,
    pub args: Vec<String>,
}

fn step(desc: &str, admin: bool, program: &str, args: &[&str]) -> PlanStep {
    PlanStep {
        description: desc.to_string(),
        needs_admin: admin,
        program: program.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
    }
}

/// Plan what the system package manager has to do for a missing tool. Empty when nothing is missing or when
/// Kora gets the tool itself: the GitHub CLI always comes from Kora's own private copy (`crate::tools`), and
/// so does Git on Windows (MinGit), so only Git on Linux is planned here (`apt-get`, administrator rights).
/// Returns `Err(Invalid)` for unsupported OSes.
pub fn plan_bootstrap(os: Os, tools: &Tools) -> Result<Vec<PlanStep>> {
    let mut steps = Vec::new();
    if tools.git.is_some() {
        return Ok(steps);
    }
    match os {
        Os::DebianLike => {
            steps.push(step("Refresh the package list", true, "apt-get", &["update"]));
            steps.push(step("Install git", true, "apt-get", &["install", "-y", "git"]));
        }
        Os::Windows => {}
        Os::Unsupported => {
            return Err(GitError::Invalid("operating system (not supported yet)", format!("{os:?}")))
        }
    }
    Ok(steps)
}

// ---------------------------------------------------------------------------------------------
// gh auth and orgs
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthStatus {
    pub logged_in: bool,
    pub scopes: Vec<String>,
}

/// Minimum scopes: `repo`, `read:org`; `admin:org` only when requested.
pub fn required_scopes(admin_org: bool) -> Vec<&'static str> {
    let mut v = vec!["repo", "read:org"];
    if admin_org {
        v.push("admin:org");
    }
    v
}

pub fn parse_scopes(text: &str) -> Vec<String> {
    for line in text.lines() {
        if let Some((_, rest)) = line.split_once("Token scopes:") {
            return rest
                .split(',')
                .map(|s| s.trim().trim_matches('\'').trim_matches('"').to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
    }
    Vec::new()
}

/// GitHub scope hierarchy: `admin:org` > `write:org` > `read:org` (a parent grants its children).
fn scope_satisfies(have: &str, need: &str) -> bool {
    have == need
        || matches!((have, need), ("admin:org", "read:org" | "write:org") | ("write:org", "read:org"))
}

impl AuthStatus {
    /// Required scopes that the current token lacks.
    pub fn missing_scopes(&self, admin_org: bool) -> Vec<String> {
        required_scopes(admin_org)
            .into_iter()
            .filter(|s| !self.scopes.iter().any(|h| scope_satisfies(h, s)))
            .map(str::to_string)
            .collect()
    }
}

pub fn auth_status(r: &dyn CommandRunner) -> Result<AuthStatus> {
    let o = run(r, "gh", &["auth", "status", "--hostname", "github.com"], None)?;
    // Older gh prints to stderr, newer to stdout.
    let text = format!("{}\n{}", o.stdout, o.stderr);
    Ok(AuthStatus { logged_in: o.ok(), scopes: if o.ok() { parse_scopes(&text) } else { vec![] } })
}

/// Arguments of `gh auth login` (device/browser flow, minimum scopes).
pub fn login_args(admin_org: bool) -> Vec<String> {
    let scopes = required_scopes(admin_org).join(",");
    ["auth", "login", "--web", "--hostname", "github.com", "--git-protocol", "https", "--scopes", &scopes].iter().map(|s| s.to_string()).collect()
}

/// Is `gh` signed in to github.com (so issues and pull requests can be made)?
pub fn is_signed_in(r: &dyn CommandRunner) -> bool {
    auth_status(r).map(|s| s.logged_in).unwrap_or(false)
}

/// `gh auth login --web` with the terminal handed to gh, so the user sees its one-time code and
/// address. For command-line use and for the manager (which closes its screen first). The token
/// stays in gh's own store; Kora never sees it.
pub fn auth_login_interactive() -> Result<()> {
    let args = login_args(false);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let st = system_command("gh", &args, None)
        .status()
        .map_err(|e| GitError::Spawn("gh auth login".into(), e.to_string()))?;
    if !st.success() {
        return Err(GitError::Command { cmd: "gh auth login".into(), code: st.code().unwrap_or(-1), stderr: "login was not completed".into() });
    }
    run_ok(&SystemRunner, "gh", &["auth", "setup-git", "--hostname", "github.com"], None)?;
    Ok(())
}

/// Device/browser flow via gh. gh keeps the token in its own store.
pub fn auth_login(r: &dyn CommandRunner, admin_org: bool) -> Result<()> {
    let args = login_args(admin_org);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    run_ok(r, "gh", &args, None)?;
    // Let git use gh's credentials for https clones/fetches.
    run_ok(r, "gh", &["auth", "setup-git", "--hostname", "github.com"], None)?;
    Ok(())
}

pub fn org_exists(r: &dyn CommandRunner, name: &str) -> Result<bool> {
    check("org name", valid_name(name), name)?;
    let path = format!("orgs/{name}");
    let o = run(r, "gh", &["api", &path], None)?;
    if o.ok() {
        return Ok(true);
    }
    if o.stderr.contains("404") || o.stderr.contains("Not Found") {
        return Ok(false);
    }
    Err(GitError::Command {
        cmd: format!("gh api {path}"),
        code: o.code,
        stderr: o.stderr.trim().to_string(),
    })
}

// ---------------------------------------------------------------------------------------------
// Paced fork engine
// ---------------------------------------------------------------------------------------------

/// True for GitHub primary/secondary rate limiting as reported by gh on stderr (HTTP 403/429).
pub fn is_rate_limited(stderr: &str) -> bool {
    let s = stderr.to_ascii_lowercase();
    s.contains("429")
        || s.contains("secondary rate limit")
        || s.contains("abuse detection")
        || (s.contains("403") && (s.contains("rate limit") || s.contains("retry") || s.contains("abuse")))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailedFork {
    pub repo: String,
    pub error: String,
}

/// How an existing fork compares with its upstream (from the GitHub compare API).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForkSync {
    UpToDate,
    /// Upstream has `n` commits the fork lacks and the fork has none of its own: safe to fast-forward.
    Behind(u32),
    /// The fork has `n` commits of its own and upstream has nothing new: leave untouched.
    Ahead(u32),
    /// Both sides have commits the other lacks.
    Diverged { ahead: u32, behind: u32 },
    /// Could not tell (reason). Never acted on.
    Unknown(String),
}

/// `ahead` = commits only the fork has, `behind` = commits only upstream has.
pub fn classify_counts(ahead: u32, behind: u32) -> ForkSync {
    match (ahead, behind) {
        (0, 0) => ForkSync::UpToDate,
        (0, b) => ForkSync::Behind(b),
        (a, 0) => ForkSync::Ahead(a),
        (a, b) => ForkSync::Diverged { ahead: a, behind: b },
    }
}

/// Classify the JSON of `compare/<upstream>:<b>...<fork>:<b>` (base = upstream, head = fork).
pub fn classify_compare(json: &str) -> ForkSync {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return ForkSync::Unknown("unreadable answer from GitHub".into());
    };
    let n = |k: &str| v.get(k).and_then(|x| x.as_u64()).and_then(|x| u32::try_from(x).ok());
    match (n("ahead_by"), n("behind_by")) {
        (Some(a), Some(b)) => classify_counts(a, b),
        _ => ForkSync::Unknown("GitHub did not report ahead/behind counts".into()),
    }
}

/// What happened to an adopted fork; persisted in the resumable state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncOutcome {
    /// Fast-forwarded on GitHub by this many commits.
    Synced { commits: u32 },
    UpToDate,
    Ahead { ahead: u32 },
    /// `ahead` is 0 when GitHub refused the fast-forward without telling how many own commits exist.
    Diverged { ahead: u32, behind: u32 },
    Unknown(String),
}

impl SyncOutcome {
    /// Forks that were not changed and need a person's attention or at least a mention.
    pub fn left_alone(&self) -> bool {
        matches!(self, SyncOutcome::Ahead { .. } | SyncOutcome::Diverged { .. } | SyncOutcome::Unknown(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncRecord {
    pub repo: String,
    pub branch: String,
    pub outcome: SyncOutcome,
}

/// Counts and plain-language lines for the install result and log.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncSummary {
    pub synced: usize,
    pub up_to_date: usize,
    pub left_alone: usize,
    /// One line per fork that was left alone, with the safe fix.
    pub lines: Vec<String>,
}

impl SyncSummary {
    pub fn headline(&self) -> String {
        format!("{} fork(s) brought up to date, {} already current, {} left alone", self.synced, self.up_to_date, self.left_alone)
    }
}

/// Plain-language line for a fork that was not touched. `fork` is the fork's `owner/name`,
/// `path` the resource folder on disk.
pub fn left_alone_line(rec: &SyncRecord, fork: &str, path: &str) -> Option<String> {
    let b = &rec.branch;
    let fix = format!("Safe fix: cd {path} && git fetch upstream && git merge upstream/{b} (or git rebase upstream/{b}, which needs a force-push you decide on yourself), then git push. Nothing was changed on this fork.");
    match &rec.outcome {
        SyncOutcome::Ahead { ahead } => Some(format!(
            "{fork} ({b}): your fork has {ahead} commit(s) of its own and upstream has nothing new, so it was left as it is. Nothing was changed on this fork."
        )),
        SyncOutcome::Diverged { ahead, behind } => {
            let own = if *ahead == 0 { "commits of its own".to_string() } else { format!("{ahead} commit(s) of its own") };
            Some(format!("{fork} ({b}): your fork has {own} and upstream has {behind} new commit(s), so GitHub cannot fast-forward it. It was not touched. {fix}"))
        }
        SyncOutcome::Unknown(why) => Some(format!("{fork} ({b}): could not check it against upstream ({why}); left untouched.")),
        _ => None,
    }
}

/// Persistable fork progress. Repos are `owner/name` sources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForkState {
    /// User or org that receives the forks.
    pub target_owner: String,
    /// True when `target_owner` is an organisation (`--org`), false for the user's account.
    pub target_is_org: bool,
    pub pending: Vec<String>,
    pub done: Vec<String>,
    pub failed: Vec<FailedFork>,
    /// Branch to compare/sync per repo (falls back to the fork's default branch).
    #[serde(default)]
    pub branches: std::collections::BTreeMap<String, String>,
    /// Result of syncing each adopted (already existing) fork.
    #[serde(default)]
    pub sync: Vec<SyncRecord>,
    /// GitHub API calls made through `gh` so far (for pacing and diagnostics).
    #[serde(default)]
    pub api_calls: u32,
    /// The real `owner/name` of the fork of each upstream (it need not have the upstream's name:
    /// GitHub may have added `-1`, or the user renamed it). Missing in files written before this
    /// was recorded: see [`ForkState::fork_of`].
    #[serde(default)]
    pub forks: std::collections::BTreeMap<String, String>,
    /// Upstreams whose fork Kora created (the rest of `done` were already there and reused).
    #[serde(default)]
    pub created: Vec<String>,
    /// What happened to the last repository handled, for progress text (not saved).
    #[serde(skip)]
    pub last_event: Option<String>,
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

impl ForkState {
    pub fn new(target_owner: &str, target_is_org: bool, repos: &[String]) -> Self {
        ForkState {
            target_owner: target_owner.to_string(),
            target_is_org,
            pending: repos.to_vec(),
            done: vec![],
            failed: vec![],
            branches: Default::default(),
            sync: vec![],
            api_calls: 0,
            forks: Default::default(),
            created: vec![],
            last_event: None,
        }
    }

    /// `owner/name` of the fork of `upstream`. Older state files did not record it; they only ever
    /// accepted a fork carrying the upstream's own name, so that name is right for them.
    pub fn fork_of(&self, upstream: &str) -> String {
        self.forks.get(upstream).cloned().unwrap_or_else(|| format!("{}/{}", self.target_owner, repo_name(upstream)))
    }

    /// (forks created by Kora, existing forks reused) among the finished repos.
    pub fn counts(&self) -> (usize, usize) {
        let created = self.done.iter().filter(|d| self.created.contains(d)).count();
        (created, self.done.len() - created)
    }

    /// `"created 2 forks, reused 14 you already had"`.
    pub fn counts_text(&self) -> String {
        let (c, u) = self.counts();
        match (c, u) {
            (0, 0) => "no forks needed".into(),
            (c, 0) => format!("created {}", plural(c, "fork", "forks")),
            (0, u) => format!("reused {} you already had", plural(u, "fork", "forks")),
            (c, u) => format!("created {}, reused {u} you already had", plural(c, "fork", "forks")),
        }
    }

    fn record_sync(&mut self, repo: &str, branch: &str, outcome: SyncOutcome) {
        self.sync.retain(|s| s.repo != repo);
        self.sync.push(SyncRecord { repo: repo.to_string(), branch: branch.to_string(), outcome });
    }

    /// Totals plus one explanatory line per fork left alone. `path_of(repo)` gives the resource folder.
    pub fn sync_summary(&self, path_of: &dyn Fn(&str) -> String) -> SyncSummary {
        let mut s = SyncSummary::default();
        for rec in &self.sync {
            match &rec.outcome {
                SyncOutcome::Synced { .. } => s.synced += 1,
                SyncOutcome::UpToDate => s.up_to_date += 1,
                _ => {
                    s.left_alone += 1;
                    if let Some(l) = left_alone_line(rec, &self.fork_of(&rec.repo), &path_of(&rec.repo)) {
                        s.lines.push(l);
                    }
                }
            }
        }
        s
    }

    /// Move failed repos back to pending (user chose "retry failed").
    pub fn requeue_failed(&mut self) {
        for f in self.failed.drain(..) {
            self.pending.push(f.repo);
        }
    }

    pub fn is_complete(&self) -> bool {
        self.pending.is_empty() && self.failed.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct ForkConfig {
    /// Forks per batch before a longer pause.
    pub batch_size: usize,
    pub batch_pause: Duration,
    pub base_backoff: Duration,
    pub max_backoff: Duration,
    /// Rate-limit retries for one repo before the run pauses (state stays resumable).
    pub max_retries: u32,
    /// GitHub creates forks asynchronously: how long to wait for a new fork to have its commits
    /// (0 = do not wait, used by tests).
    pub ready_timeout: Duration,
    pub ready_poll: Duration,
    /// Bring an adopted (already existing) fork up to date by fast-forward only.
    pub sync_existing: bool,
    /// With `sync_existing`, pacing counts GitHub API calls (a repo costs 1-3) instead of repos:
    /// pause after this many calls.
    pub batch_calls: usize,
    /// Shared upstream -> fork cache ([`forkfind::default_cache_path`]); `None` = no cache.
    pub cache_path: Option<PathBuf>,
}

impl Default for ForkConfig {
    fn default() -> Self {
        ForkConfig {
            cache_path: None,
            sync_existing: false,
            batch_calls: 15,
            batch_size: 5,
            batch_pause: Duration::from_secs(30),
            base_backoff: Duration::from_secs(60),
            max_backoff: Duration::from_secs(900),
            max_retries: 5,
            ready_timeout: Duration::from_secs(300),
            ready_poll: Duration::from_secs(3),
        }
    }
}

impl ForkConfig {
    pub fn backoff(&self, attempt: u32) -> Duration {
        let mult = 1u32.checked_shl(attempt).unwrap_or(u32::MAX);
        self.base_backoff.saturating_mul(mult).min(self.max_backoff)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForkOutcome {
    /// Nothing left pending (failures, if any, are in `state.failed`).
    Finished,
    /// Rate limit persisted through all retries; call again later to resume.
    Paused,
}

fn repo_name(repo: &str) -> &str {
    repo.rsplit('/').next().unwrap_or(repo)
}

/// Wait until the (asynchronously created) fork `fork` (`owner/name`) has its commits, so a clone
/// right after succeeds.
fn wait_fork_ready(r: &dyn CommandRunner, fork: &str, cfg: &ForkConfig) -> std::result::Result<(), String> {
    if cfg.ready_timeout.is_zero() {
        return Ok(());
    }
    let path = format!("repos/{fork}/commits?per_page=1");
    let mut waited = Duration::ZERO;
    loop {
        if let Ok(o) = r.run("gh", &["api", &path, "--jq", "length"], None) {
            if o.ok() && o.stdout.trim() == "1" {
                return Ok(());
            }
        }
        if waited >= cfg.ready_timeout {
            return Err(format!("fork {fork} was created but is still not ready after {}s", cfg.ready_timeout.as_secs()));
        }
        r.sleep(cfg.ready_poll);
        waited += cfg.ready_poll.max(Duration::from_millis(1));
    }
}

/// Wraps a runner and counts the `gh` calls (each one is a GitHub API request), so pacing is by cost.
pub struct CountingRunner<'a> {
    inner: &'a dyn CommandRunner,
    calls: std::cell::Cell<u32>,
}

impl<'a> CountingRunner<'a> {
    pub fn new(inner: &'a dyn CommandRunner) -> Self {
        CountingRunner { inner, calls: std::cell::Cell::new(0) }
    }
    pub fn calls(&self) -> u32 {
        self.calls.get()
    }
}

impl CommandRunner for CountingRunner<'_> {
    fn run(&self, program: &str, args: &[&str], cwd: Option<&Path>) -> std::io::Result<Output> {
        if program == "gh" {
            self.calls.set(self.calls.get() + 1);
        }
        self.inner.run(program, args, cwd)
    }
    fn sleep(&self, d: Duration) {
        self.inner.sleep(d)
    }
}

/// One attempt to compare a fork with its upstream and, only when it is purely behind, fast-forward it.
enum SyncStep {
    Done(SyncOutcome),
    RateLimited,
}

fn compare_fork(r: &dyn CommandRunner, fork: &str, upstream: &str, branch: &str) -> Result<std::result::Result<ForkSync, ()>> {
    let up_owner = upstream.split('/').next().unwrap_or("");
    let fork_owner = fork.split('/').next().unwrap_or("");
    let path = format!("repos/{fork}/compare/{up_owner}:{branch}...{fork_owner}:{branch}");
    let o = run(r, "gh", &["api", &path], None)?;
    if o.ok() {
        return Ok(Ok(classify_compare(&o.stdout)));
    }
    if is_rate_limited(&o.stderr) {
        return Ok(Err(()));
    }
    Ok(Ok(ForkSync::Unknown(first_line(&o.stderr, "GitHub refused the comparison"))))
}

fn first_line(s: &str, fallback: &str) -> String {
    s.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or(fallback).chars().take(200).collect()
}

/// Fast-forward only: compare, then `gh repo sync <fork> --branch <b>` when the fork is purely
/// behind. `--force` is never passed; a refusal because of divergence is recorded, not overridden.
fn sync_step(r: &dyn CommandRunner, fork: &str, upstream: &str, branch: &str) -> Result<SyncStep> {
    check("fork repo", valid_repo(fork), fork)?;
    check("upstream repo", valid_repo(upstream), upstream)?;
    check("branch", valid_ref(branch), branch)?;
    let state = match compare_fork(r, fork, upstream, branch)? {
        Ok(s) => s,
        Err(()) => return Ok(SyncStep::RateLimited),
    };
    let behind = match state {
        ForkSync::UpToDate => return Ok(SyncStep::Done(SyncOutcome::UpToDate)),
        ForkSync::Ahead(a) => return Ok(SyncStep::Done(SyncOutcome::Ahead { ahead: a })),
        ForkSync::Diverged { ahead, behind } => return Ok(SyncStep::Done(SyncOutcome::Diverged { ahead, behind })),
        ForkSync::Unknown(why) => return Ok(SyncStep::Done(SyncOutcome::Unknown(why))),
        ForkSync::Behind(n) => n,
    };
    let o = run(r, "gh", &["repo", "sync", fork, "--branch", branch], None)?;
    if o.ok() {
        return Ok(SyncStep::Done(SyncOutcome::Synced { commits: behind }));
    }
    if is_rate_limited(&o.stderr) {
        return Ok(SyncStep::RateLimited);
    }
    let low = o.stderr.to_ascii_lowercase();
    if low.contains("diverg") || low.contains("--force") || low.contains("conflict") || low.contains("not a fast-forward") {
        // GitHub says the fork has history upstream lacks (e.g. it changed since we compared): look again.
        return Ok(SyncStep::Done(match compare_fork(r, fork, upstream, branch)? {
            Ok(ForkSync::Diverged { ahead, behind }) => SyncOutcome::Diverged { ahead, behind },
            Ok(ForkSync::Ahead(a)) => SyncOutcome::Ahead { ahead: a },
            _ => SyncOutcome::Diverged { ahead: 0, behind },
        }));
    }
    Ok(SyncStep::Done(SyncOutcome::Unknown(first_line(&o.stderr, "gh repo sync failed"))))
}

/// [`sync_step`] with the pacing backoff on rate limits. `None` = still limited after all retries
/// (the caller pauses and resumes later).
pub fn sync_fork_retrying(r: &dyn CommandRunner, fork: &str, upstream: &str, branch: &str, cfg: &ForkConfig) -> Result<Option<SyncOutcome>> {
    let mut attempt = 0u32;
    loop {
        match sync_step(r, fork, upstream, branch)? {
            SyncStep::Done(o) => return Ok(Some(o)),
            SyncStep::RateLimited => {
                if attempt >= cfg.max_retries {
                    return Ok(None);
                }
                r.sleep(cfg.backoff(attempt));
                attempt += 1;
            }
        }
    }
}

/// One fork to bring up to date outside the install flow (`kora update`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncJob {
    pub id: String,
    /// `owner/name` of the user's fork.
    pub fork: String,
    /// `owner/name` of the upstream repository.
    pub upstream: String,
    pub branch: String,
}

/// Paced sync of many forks (same batches and pauses as forking). Stops at a persistent rate limit:
/// the remaining jobs are reported as `Unknown("rate limited ...")` and left untouched.
/// `progress(done, total)` is called after each fork.
pub fn sync_forks_paced(
    r: &dyn CommandRunner,
    jobs: &[SyncJob],
    cfg: &ForkConfig,
    progress: &mut dyn FnMut(usize, usize),
) -> Vec<(String, SyncOutcome)> {
    let counter = CountingRunner::new(r);
    let mut out = Vec::new();
    let mut in_batch = 0usize;
    let mut limited = false;
    for (i, j) in jobs.iter().enumerate() {
        if limited {
            out.push((
                j.id.clone(),
                SyncOutcome::Unknown("GitHub rate limit reached; will be checked next time".into()),
            ));
            progress(i + 1, jobs.len());
            continue;
        }
        let before = counter.calls();
        let res = match sync_fork_retrying(&counter, &j.fork, &j.upstream, &j.branch, cfg) {
            Ok(Some(o)) => o,
            Ok(None) => {
                limited = true;
                SyncOutcome::Unknown("GitHub rate limit reached; will be checked next time".into())
            }
            Err(e) => SyncOutcome::Unknown(e.to_string()),
        };
        out.push((j.id.clone(), res));
        progress(i + 1, jobs.len());
        in_batch += (counter.calls() - before).max(1) as usize;
        if in_batch >= cfg.batch_calls && i + 1 < jobs.len() {
            in_batch = 0;
            r.sleep(cfg.batch_pause);
        }
    }
    out
}

/// Run (or resume) forking. `on_progress` is called after every state change so the caller can
/// persist `state`. Never forks a repo already in `done` or one the owner already has a fork of,
/// whatever that fork is called: existing forks are found by their upstream (see [`forkfind`]),
/// and the real fork of every repo is recorded in `state.forks`.
pub fn run_forks(
    r: &dyn CommandRunner,
    state: &mut ForkState,
    cfg: &ForkConfig,
    on_progress: &mut dyn FnMut(&ForkState),
) -> Result<ForkOutcome> {
    check("target owner", valid_name(&state.target_owner), &state.target_owner)?;
    let mut finder = forkfind::ForkFinder::new(&state.target_owner, cfg.cache_path.as_deref());
    let out = run_forks_with(r, state, cfg, &mut finder, on_progress);
    finder.save();
    out
}

fn run_forks_with(
    r: &dyn CommandRunner,
    state: &mut ForkState,
    cfg: &ForkConfig,
    finder: &mut forkfind::ForkFinder,
    on_progress: &mut dyn FnMut(&ForkState),
) -> Result<ForkOutcome> {
    use forkfind::Found;
    let counter = CountingRunner::new(r);
    let r: &dyn CommandRunner = &counter;
    let mut in_batch = 0usize;
    while let Some(repo) = state.pending.first().cloned() {
        let calls_before = counter.calls();
        // Some(default branch) when an already existing fork was adopted for this repo.
        let mut adopted: Option<Option<String>> = None;
        // The fork found or made for this repo.
        let mut fork: Option<String> = None;
        if state.done.contains(&repo) {
            state.pending.remove(0);
            on_progress(state);
            continue;
        }
        if !valid_repo(&repo) {
            state.pending.remove(0);
            state.failed.push(FailedFork { repo: repo.clone(), error: "invalid repository name".into() });
            on_progress(state);
            continue;
        }
        let mut attempt = 0u32;
        let result: std::result::Result<(), String> = loop {
            let o = match finder.find(r, &repo) {
                // already there: no new fork, but it may still be filling up after a crash
                Ok(Found::Present { slug, default_branch }) => {
                    adopted = Some(default_branch);
                    let ready = wait_fork_ready(r, &slug, cfg);
                    fork = Some(slug);
                    break ready;
                }
                Ok(Found::Foreign(why)) => break Err(why),
                Ok(Found::Missing) => {
                    let mut args = vec!["repo", "fork", repo.as_str(), "--clone=false"];
                    if state.target_is_org {
                        args.push("--org");
                        args.push(state.target_owner.as_str());
                    }
                    run(r, "gh", &args, None)?
                }
                Err(GitError::Command { code, stderr, .. }) => Output { code, stdout: String::new(), stderr },
                Err(e) => return Err(e),
            };
            if o.ok() || o.stderr.contains("already exists") {
                // GitHub hands back the owner's existing fork (under whatever name it has) instead
                // of making a second one; gh then says "<owner>/<name> already exists".
                let existed = o.stderr.contains("already exists");
                let Some(slug) = finder.after_fork(r, &repo, &o) else {
                    break Err(format!("GitHub accepted the fork request for {repo} but Kora could not find the fork in {}", state.target_owner));
                };
                let ready = if existed {
                    adopted = Some(None);
                    Ok(())
                } else {
                    wait_fork_ready(r, &slug, cfg)
                };
                fork = Some(slug);
                break ready;
            }
            if is_rate_limited(&o.stderr) {
                if attempt >= cfg.max_retries {
                    on_progress(state);
                    return Ok(ForkOutcome::Paused);
                }
                r.sleep(cfg.backoff(attempt));
                attempt += 1;
                continue;
            }
            break Err(owners::friendly_fork_error(&o.stderr, &state.target_owner));
        };
        if let Some(f) = &fork {
            state.forks.insert(repo.clone(), f.clone());
        }
        if let (true, true, Some(default_branch), Some(f)) = (cfg.sync_existing, result.is_ok(), adopted.clone(), &fork) {
            match state.branches.get(&repo).cloned().or(default_branch) {
                Some(branch) => match sync_fork_retrying(r, f, &repo, &branch, cfg) {
                    Ok(Some(o)) => state.record_sync(&repo, &branch, o),
                    Ok(None) => {
                        state.api_calls += counter.calls() - calls_before;
                        on_progress(state);
                        return Ok(ForkOutcome::Paused);
                    }
                    Err(e) => state.record_sync(&repo, &branch, SyncOutcome::Unknown(e.to_string())),
                },
                None => state.record_sync(&repo, "?", SyncOutcome::Unknown("could not tell which branch to compare".into())),
            }
        }
        let used = counter.calls() - calls_before;
        state.api_calls += used;
        state.pending.remove(0);
        let shown = fork.clone().unwrap_or_else(|| repo.clone());
        match result {
            Ok(()) => {
                if adopted.is_some() {
                    state.last_event = Some(format!("Using your existing fork {shown}"));
                } else {
                    if !state.created.contains(&repo) {
                        state.created.push(repo.clone());
                    }
                    state.last_event = Some(format!("Created fork {shown}"));
                }
                state.done.push(repo)
            }
            Err(error) => {
                state.last_event = Some(format!("Could not fork {repo}"));
                state.failed.push(FailedFork { repo, error })
            }
        }
        on_progress(state);
        // Without syncing a repo counts as 1 (as before); with it pacing counts real API calls.
        let (weight, limit) = if cfg.sync_existing { (used.max(1) as usize, cfg.batch_calls) } else { (1, cfg.batch_size) };
        in_batch += weight;
        if in_batch >= limit && !state.pending.is_empty() {
            in_batch = 0;
            r.sleep(cfg.batch_pause);
        }
    }
    Ok(ForkOutcome::Finished)
}

// ---------------------------------------------------------------------------------------------
// Clone, update, rollback
// ---------------------------------------------------------------------------------------------

/// Clone `<fork_owner>/<name>` into `dest` and set `upstream` to the original repo.
pub fn clone_from_fork(
    r: &dyn CommandRunner,
    fork: &str,
    upstream: &str,
    dest: &Path,
    branch: Option<&str>,
) -> Result<()> {
    check("fork repo", valid_repo(fork), fork)?;
    check("upstream repo", valid_repo(upstream), upstream)?;
    let d = dest.to_string_lossy();
    run_ok(r, "gh", &["repo", "clone", fork, &d], None)?;
    let url = format!("https://github.com/{upstream}.git");
    let has = run(r, "git", &["remote", "get-url", "upstream"], Some(dest))?.ok();
    let sub = if has { "set-url" } else { "add" };
    run_ok(r, "git", &["remote", sub, "upstream", &url], Some(dest))?;
    if let Some(b) = branch {
        check("branch", valid_ref(b), b)?;
        run_ok(r, "git", &["checkout", b], Some(dest))?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepoState {
    UpToDate,
    /// Clean, no local commits, upstream is ahead: safe to fast-forward.
    FastForwardable { behind: u32 },
    /// Both sides have new commits.
    Diverged { ahead: u32, behind: u32 },
    /// Local commits only (nothing to pull).
    LocalAhead { ahead: u32 },
    /// Uncommitted changes (takes priority over everything else).
    Dirty,
}

fn parse_counts(cmd: &str, out: &str) -> Result<(u32, u32)> {
    let mut it = out.split_whitespace().map(str::parse::<u32>);
    match (it.next(), it.next()) {
        (Some(Ok(a)), Some(Ok(b))) => Ok((a, b)),
        _ => Err(GitError::Parse(cmd.to_string(), out.to_string())),
    }
}

pub fn head_commit(r: &dyn CommandRunner, repo: &Path) -> Result<String> {
    Ok(run_ok(r, "git", &["rev-parse", "HEAD"], Some(repo))?.stdout.trim().to_string())
}

/// Fetch `upstream` and classify the repo against `upstream/<branch>`. Never changes the tree.
pub fn check_state(r: &dyn CommandRunner, repo: &Path, branch: &str) -> Result<RepoState> {
    check("branch", valid_ref(branch), branch)?;
    run_ok(r, "git", &["fetch", "upstream"], Some(repo))?;
    let st = run_ok(r, "git", &["status", "--porcelain"], Some(repo))?;
    if !st.stdout.trim().is_empty() {
        return Ok(RepoState::Dirty);
    }
    let range = format!("HEAD...upstream/{branch}");
    let o = run_ok(r, "git", &["rev-list", "--left-right", "--count", &range], Some(repo))?;
    let (ahead, behind) = parse_counts("git rev-list", &o.stdout)?;
    Ok(match (ahead, behind) {
        (0, 0) => RepoState::UpToDate,
        (0, b) => RepoState::FastForwardable { behind: b },
        (a, 0) => RepoState::LocalAhead { ahead: a },
        (a, b) => RepoState::Diverged { ahead: a, behind: b },
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpdateResult {
    UpToDate,
    /// `previous` is the rollback point (persist it in the server's snapshots).
    Updated { previous: String, current: String },
    /// Not touched; flag "needs attention".
    Skipped(RepoState),
}

/// Fast-forward only. Dirty, diverged and locally-ahead repos are skipped; nothing is merged.
pub fn update_ff_only(r: &dyn CommandRunner, repo: &Path, branch: &str) -> Result<UpdateResult> {
    match check_state(r, repo, branch)? {
        RepoState::UpToDate => Ok(UpdateResult::UpToDate),
        RepoState::FastForwardable { .. } => {
            let previous = head_commit(r, repo)?;
            let target = format!("upstream/{branch}");
            run_ok(r, "git", &["merge", "--ff-only", &target], Some(repo))?;
            let current = head_commit(r, repo)?;
            Ok(UpdateResult::Updated { previous, current })
        }
        other => Ok(UpdateResult::Skipped(other)),
    }
}

/// Roll a checkout back to a recorded commit (`git reset --hard`).
pub fn reset_to_commit(r: &dyn CommandRunner, repo: &Path, commit: &str) -> Result<()> {
    check("commit", valid_commit(commit), commit)?;
    run_ok(r, "git", &["reset", "--hard", commit], Some(repo))?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Issues and pull requests (through `gh`; no token handling of our own)
// ---------------------------------------------------------------------------------------------

fn clean_text(what: &'static str, v: &str, max: usize, allow_newlines: bool) -> Result<()> {
    let bad = v.chars().any(|c| c.is_control() && !(allow_newlines && matches!(c, '\n' | '\r' | '\t')));
    check(what, !v.trim().is_empty() && v.chars().count() <= max && !bad, &v.chars().take(40).collect::<String>())
}

/// The first `https://github.com/...` line of gh's output.
fn url_from(out: &Output, cmd: &str) -> Result<String> {
    out.stdout
        .lines()
        .map(str::trim)
        .rev()
        .find(|l| l.starts_with("https://github.com/"))
        .map(str::to_string)
        .ok_or_else(|| GitError::Parse(cmd.to_string(), out.stdout.chars().take(200).collect()))
}

/// `gh issue create` in `repo` (`owner/name`). Returns the issue's address. Title and body are
/// passed as separate arguments (never through a shell).
pub fn create_issue(r: &dyn CommandRunner, repo: &str, title: &str, body: &str) -> Result<String> {
    check("repo", valid_repo(repo), repo)?;
    clean_text("issue title", title, 256, false)?;
    clean_text("issue text", body, 60_000, true)?;
    let o = run_ok(r, "gh", &["issue", "create", "--repo", repo, "--title", title, "--body", body], None)?;
    url_from(&o, "gh issue create")
}

/// Current branch of a checkout; `None` when detached.
pub fn current_branch(r: &dyn CommandRunner, repo: &Path) -> Result<Option<String>> {
    let o = run_ok(r, "git", &["rev-parse", "--abbrev-ref", "HEAD"], Some(repo))?;
    let b = o.stdout.trim();
    Ok(if b.is_empty() || b == "HEAD" { None } else { Some(b.to_string()) })
}

/// `owner` of the `github.com/<owner>/<name>` remote (https or ssh form).
pub fn remote_owner(r: &dyn CommandRunner, repo: &Path, remote: &str) -> Result<String> {
    check("remote", valid_name(remote), remote)?;
    let o = run_ok(r, "git", &["remote", "get-url", remote], Some(repo))?;
    owner_of_url(o.stdout.trim()).ok_or_else(|| GitError::Parse(format!("git remote get-url {remote}"), o.stdout.trim().to_string()))
}

/// `owner` in `https://github.com/owner/name(.git)` or `git@github.com:owner/name(.git)`.
pub fn owner_of_url(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://github.com/").or_else(|| url.strip_prefix("git@github.com:")).or_else(|| url.strip_prefix("ssh://git@github.com/"))?;
    let (owner, name) = rest.split_once('/')?;
    (valid_name(owner) && !name.is_empty()).then(|| owner.to_string())
}

/// `owner/name` in a `github.com` remote URL (https or ssh form, optional `.git`).
pub fn slug_of_url(url: &str) -> Option<String> {
    let owner = owner_of_url(url)?;
    let rest = url.trim().rsplit_once(&format!("{owner}/"))?.1;
    let name = rest.trim_end_matches('/').trim_end_matches(".git");
    let slug = format!("{owner}/{name}");
    valid_repo(&slug).then_some(slug)
}

/// Commits on HEAD that `upstream/<base>` does not have (fetches `upstream` first).
pub fn ahead_of_upstream(r: &dyn CommandRunner, repo: &Path, base: &str) -> Result<u32> {
    check("branch", valid_ref(base), base)?;
    run_ok(r, "git", &["fetch", "upstream"], Some(repo))?;
    let range = format!("upstream/{base}..HEAD");
    let o = run_ok(r, "git", &["rev-list", "--count", &range], Some(repo))?;
    o.stdout.trim().parse().map_err(|_| GitError::Parse("git rev-list --count".into(), o.stdout.trim().to_string()))
}

/// Push `branch` to the fork (`origin`).
pub fn push_branch(r: &dyn CommandRunner, repo: &Path, branch: &str) -> Result<()> {
    check("branch", valid_ref(branch), branch)?;
    run_ok(r, "git", &["push", "--set-upstream", "origin", branch], Some(repo))?;
    Ok(())
}

/// `gh pr create` from `<head_owner>:<head_branch>` into `<upstream>`'s `<base>`. Returns the address.
pub fn create_pr(r: &dyn CommandRunner, upstream: &str, head_owner: &str, head_branch: &str, base: &str, title: &str, body: &str) -> Result<String> {
    check("upstream repo", valid_repo(upstream), upstream)?;
    check("fork owner", valid_name(head_owner), head_owner)?;
    check("branch", valid_ref(head_branch), head_branch)?;
    check("base branch", valid_ref(base), base)?;
    clean_text("pull request title", title, 256, false)?;
    clean_text("pull request text", body, 60_000, true)?;
    let head = format!("{head_owner}:{head_branch}");
    let o = run_ok(r, "gh", &["pr", "create", "--repo", upstream, "--head", &head, "--base", base, "--title", title, "--body", body], None)?;
    url_from(&o, "gh pr create")
}

/// Prod: clone without checkout, then check out the pinned commit detached.
pub fn clone_pinned(r: &dyn CommandRunner, repo: &str, commit: &str, dest: &Path) -> Result<()> {
    check("repo", valid_repo(repo), repo)?;
    check("commit", valid_commit(commit), commit)?;
    let url = format!("https://github.com/{repo}.git");
    let d = dest.to_string_lossy();
    run_ok(r, "git", &["clone", "--no-checkout", &url, &d], None)?;
    checkout_pinned(r, dest, commit)
}

/// Check out `commit` detached. A pinned release commit is not always reachable from a branch or
/// tag (e.g. a release commit only referenced by a PR), so a plain clone may lack it: fetch it by
/// id (GitHub serves any commit by SHA) and retry.
fn checkout_pinned(r: &dyn CommandRunner, repo: &Path, commit: &str) -> Result<()> {
    let first = run(r, "git", &["checkout", "--detach", commit], Some(repo))?;
    if first.ok() {
        return Ok(());
    }
    run_ok(r, "git", &["fetch", "origin", commit], Some(repo))?;
    run_ok(r, "git", &["checkout", "--detach", commit], Some(repo))?;
    Ok(())
}

/// Prod, resource without catalog releases: shallow clone of the repository's default branch head.
pub fn clone_default_head(r: &dyn CommandRunner, repo: &str, dest: &Path) -> Result<()> {
    check("repo", valid_repo(repo), repo)?;
    let url = format!("https://github.com/{repo}.git");
    let d = dest.to_string_lossy();
    run_ok(r, "git", &["clone", "--depth", "1", &url, &d], None)?;
    Ok(())
}

/// Prod update: move an existing pinned clone to a newer pinned commit. No network when the
/// commit is already in the clone (the preview usually fetched it).
pub fn move_pinned(r: &dyn CommandRunner, repo: &Path, commit: &str) -> Result<String> {
    check("commit", valid_commit(commit), commit)?;
    let previous = head_commit(r, repo)?;
    let present = run(
        r,
        "git",
        &["cat-file", "-e", &format!("{commit}^{{commit}}")],
        Some(repo),
    )?
    .ok();
    if !present {
        run_ok(r, "git", &["fetch", "origin"], Some(repo))?;
    }
    checkout_pinned(r, repo, commit)?;
    Ok(previous)
}

#[cfg(test)]
mod tests {

    #[test]
    fn an_elevated_kora_on_windows_hardens_git_like_root_on_linux() {
        assert_eq!(windows_root_mode(true), Some(false));
        assert_eq!(windows_root_mode(false), None);
        let inv = Invocation { program: "git".into(), args: vec!["pull".into()], cwd: None, env: vec![] };
        let h = harden_for_root(inv, windows_root_mode(true), None, None);
        let v = h.env.iter().find(|(k, _)| k == "GIT_CONFIG_PARAMETERS" || k.starts_with("GIT_CONFIG_VALUE_") || k.starts_with("GIT_CONFIG_KEY_")).is_some();
        assert!(v, "{:?}", h.env);
        assert!(h.env.iter().any(|(_, val)| val == "core.fsmonitor") || h.env.iter().any(|(_, val)| val.contains("fsmonitor")), "{:?}", h.env);
    }
    use super::*;
    use std::cell::RefCell;

    struct Fake {
        calls: RefCell<Vec<String>>,
        sleeps: RefCell<Vec<Duration>>,
        handler: Box<dyn Fn(&str) -> Output>,
    }

    impl Fake {
        fn new(h: impl Fn(&str) -> Output + 'static) -> Self {
            Fake { calls: RefCell::new(vec![]), sleeps: RefCell::new(vec![]), handler: Box::new(h) }
        }
        fn count(&self, needle: &str) -> usize {
            self.calls.borrow().iter().filter(|c| c.contains(needle)).count()
        }
    }

    impl CommandRunner for Fake {
        fn run(&self, p: &str, a: &[&str], _cwd: Option<&Path>) -> std::io::Result<Output> {
            let line = format!("{p} {}", a.join(" "));
            self.calls.borrow_mut().push(line.clone());
            Ok((self.handler)(&line))
        }
        fn sleep(&self, d: Duration) {
            self.sleeps.borrow_mut().push(d);
        }
    }

    fn ok(s: &str) -> Output {
        Output { code: 0, stdout: s.into(), stderr: String::new() }
    }
    fn fail(s: &str) -> Output {
        Output { code: 1, stdout: String::new(), stderr: s.into() }
    }
    fn cfg() -> ForkConfig {
        ForkConfig {
            batch_size: 2,
            batch_pause: Duration::from_secs(10),
            base_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(8),
            max_retries: 4,
            ready_timeout: Duration::ZERO,
            ready_poll: Duration::from_secs(1),
            sync_existing: false,
            batch_calls: 6,
            cache_path: None,
        }
    }
    fn sync_cfg() -> ForkConfig {
        ForkConfig { sync_existing: true, ..cfg() }
    }
    /// Fake GitHub: `forks` maps repo name -> (ahead, behind) of the fork vs upstream; names not in
    /// the map do not exist. `sync_ok` false makes `gh repo sync` refuse as GitHub does when diverged.
    fn github(forks: &'static [(&'static str, u32, u32)]) -> Fake {
        Fake::new(move |c| {
            if let Some(rest) = c.strip_prefix("gh api repos/me/") {
                let name = rest.split('/').next().unwrap();
                let Some((_, a, b)) = forks.iter().find(|(n, _, _)| *n == name) else { return fail("HTTP 404") };
                if rest.contains("/compare/") {
                    return ok(&format!(r#"{{"ahead_by":{a},"behind_by":{b},"status":"x"}}"#));
                }
                return ok(&format!(r#"{{"fork":true,"default_branch":"dev","parent":{{"full_name":"up/{name}"}}}}"#));
            }
            if c.starts_with("gh repo sync") {
                let name = c.split_whitespace().nth(3).unwrap().rsplit('/').next().unwrap();
                let (_, a, _) = forks.iter().find(|(n, _, _)| *n == name).unwrap();
                return if *a == 0 { ok("") } else { fail("can't sync because there are diverging changes; you can use `--force` to overwrite the destination branch") };
            }
            ok("")
        })
    }
    fn repos(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn admin_org_scope_satisfies_read_org() {
        let st = AuthStatus { logged_in: true, scopes: vec!["repo".into(), "admin:org".into()] };
        assert!(st.missing_scopes(true).is_empty());
        let st = AuthStatus { logged_in: true, scopes: vec!["repo".into(), "read:org".into()] };
        assert_eq!(st.missing_scopes(true), vec!["admin:org".to_string()]);
    }

    #[test]
    fn foreign_repo_with_same_name_is_not_adopted_and_fork_waits_until_ready() {
        let f = Fake::new(|c| {
            if c.starts_with("gh api repos/me/mine") {
                ok(r#"{"fork":false}"#)
            } else if c.starts_with("gh api repos/me/other") {
                ok(r#"{"fork":true,"parent":{"full_name":"zzz/other"}}"#)
            } else {
                ok("")
            }
        });
        let mut st = ForkState::new("me", false, &repos(&["a/mine", "a/other"]));
        run_forks(&f, &mut st, &cfg(), &mut |_| {}).unwrap();
        assert_eq!(st.failed.len(), 2, "{:?}", st.failed);
        assert_eq!(f.count("gh repo fork"), 0);

        // async fork: commits endpoint is empty twice, then ready.
        let n = std::cell::Cell::new(0);
        let f = Fake::new(move |c| {
            if c.contains("/commits") {
                n.set(n.get() + 1);
                return if n.get() < 3 { fail("HTTP 409: Git Repository is empty") } else { ok("1\n") };
            }
            if c.starts_with("gh repo fork") {
                return ok("https://github.com/me/new\n");
            }
            if c.starts_with("gh api repos/me/") { fail("HTTP 404") } else { ok("") }
        });
        let mut c = cfg();
        c.ready_timeout = Duration::from_secs(30);
        let mut st = ForkState::new("me", false, &repos(&["a/new"]));
        run_forks(&f, &mut st, &c, &mut |_| {}).unwrap();
        assert!(st.is_complete(), "{st:?}");
        assert_eq!(f.sleeps.borrow().len(), 2);
    }

    #[test]
    fn failed_existence_check_never_forks() {
        // A network/auth/server error on the lookup must not be read as "repo missing".
        let f = Fake::new(|c| {
            if c.starts_with("gh api repos/me/") {
                fail("error connecting to api.github.com")
            } else {
                ok("")
            }
        });
        let mut st = ForkState::new("me", false, &repos(&["a/mine"]));
        run_forks(&f, &mut st, &cfg(), &mut |_| {}).unwrap();
        assert_eq!(f.count("gh repo fork"), 0, "must not fork a repo whose existence is unknown");
        assert_eq!(st.failed.len(), 1);
    }

    #[test]
    fn rate_limit_detection() {
        assert!(is_rate_limited("HTTP 429: Too Many Requests"));
        assert!(is_rate_limited("HTTP 403: You have exceeded a secondary rate limit"));
        assert!(!is_rate_limited("HTTP 403: Resource not accessible by integration"));
        assert!(!is_rate_limited("HTTP 404: Not Found"));
    }

    #[test]
    fn backoff_is_exponential_and_capped() {
        let c = cfg();
        let v: Vec<u64> = (0..6).map(|i| c.backoff(i).as_secs()).collect();
        assert_eq!(v, vec![1, 2, 4, 8, 8, 8]);
        assert_eq!(c.backoff(200), c.max_backoff);
    }

    #[test]
    fn fork_backs_off_then_succeeds() {
        let tries = std::cell::Cell::new(0);
        let f = Fake::new(move |c| {
            if c.starts_with("gh api") {
                fail("HTTP 404: Not Found")
            } else if c.starts_with("gh repo fork") {
                tries.set(tries.get() + 1);
                if tries.get() <= 3 {
                    fail("HTTP 403: You have exceeded a secondary rate limit")
                } else {
                    ok("https://github.com/kora-org/one\n")
                }
            } else {
                ok("")
            }
        });
        let mut st = ForkState::new("kora-org", true, &repos(&["a/one"]));
        let out = run_forks(&f, &mut st, &cfg(), &mut |_| {}).unwrap();
        assert_eq!(out, ForkOutcome::Finished);
        assert_eq!(st.done, repos(&["a/one"]));
        assert_eq!(*f.sleeps.borrow(), vec![Duration::from_secs(1), Duration::from_secs(2), Duration::from_secs(4)]);
        assert_eq!(f.count("--org kora-org"), 4);
    }

    #[test]
    fn persistent_rate_limit_pauses_and_resumes_without_duplicates() {
        let limited = std::rc::Rc::new(std::cell::Cell::new(true));
        let l2 = limited.clone();
        let created = std::rc::Rc::new(RefCell::new(Vec::<String>::new()));
        let c2 = created.clone();
        let f = Fake::new(move |c| {
            if let Some(path) = c.strip_prefix("gh api repos/kora-org/") {
                return if c2.borrow().iter().any(|n| n == path) { ok("{}") } else { fail("HTTP 404") };
            }
            if c.starts_with("gh repo fork") {
                if c.contains("a/two") && l2.get() {
                    return fail("HTTP 429");
                }
                let name = c.split_whitespace().nth(3).unwrap().rsplit('/').next().unwrap().to_string();
                c2.borrow_mut().push(name.clone());
                return ok(&format!("https://github.com/kora-org/{name}\n"));
            }
            ok("")
        });
        let mut st = ForkState::new("kora-org", true, &repos(&["a/one", "a/two", "a/three"]));
        let mut saves = 0;
        let out = run_forks(&f, &mut st, &cfg(), &mut |_| saves += 1).unwrap();
        assert_eq!(out, ForkOutcome::Paused);
        assert_eq!(st.done, repos(&["a/one"]));
        assert_eq!(st.pending, repos(&["a/two", "a/three"]));
        assert!(saves >= 1);

        // Serialise / reload like a crashed process would.
        let json = serde_json::to_string(&st).unwrap();
        let mut st: ForkState = serde_json::from_str(&json).unwrap();
        limited.set(false);
        let out = run_forks(&f, &mut st, &cfg(), &mut |_| {}).unwrap();
        assert_eq!(out, ForkOutcome::Finished);
        assert_eq!(st.done, repos(&["a/one", "a/two", "a/three"]));
        assert!(st.is_complete());
        assert_eq!(f.count("gh repo fork a/one"), 1);
        assert_eq!(f.count("gh repo fork a/three"), 1);
    }

    #[test]
    fn existing_fork_is_not_forked_again_and_failures_are_recorded() {
        let f = Fake::new(|c| {
            if c.starts_with("gh api repos/me/have") {
                ok("{}")
            } else if c.starts_with("gh api") {
                fail("HTTP 404")
            } else if c.contains("a/bad") {
                fail("HTTP 403: Resource not accessible")
            } else if c.starts_with("gh repo fork a/good") {
                ok("https://github.com/me/good\n")
            } else {
                ok("")
            }
        });
        let mut st = ForkState::new("me", false, &repos(&["a/have", "a/bad", "a/good"]));
        let out = run_forks(&f, &mut st, &cfg(), &mut |_| {}).unwrap();
        assert_eq!(out, ForkOutcome::Finished);
        assert_eq!(f.count("gh repo fork a/have"), 0);
        assert_eq!(st.done, repos(&["a/have", "a/good"]));
        assert_eq!(st.failed.len(), 1);
        assert_eq!(st.failed[0].repo, "a/bad");
        assert_eq!(f.count("--org"), 0); // user target
        // batch_size 2 with 3 repos: exactly one batch pause.
        assert_eq!(*f.sleeps.borrow(), vec![Duration::from_secs(10)]);
        st.requeue_failed();
        assert_eq!(st.pending, repos(&["a/bad"]));
    }

    /// One page of the GraphQL fork listing: (fork slug, parent, grandparent).
    fn listing(nodes: &[(&str, &str, Option<&str>)]) -> String {
        let n: Vec<String> = nodes
            .iter()
            .map(|(slug, parent, gp)| {
                let gp = gp.map(|g| format!(r#","parent":{{"nameWithOwner":"{g}"}}"#)).unwrap_or_default();
                format!(r#"{{"nameWithOwner":"{slug}","defaultBranchRef":{{"name":"main"}},"parent":{{"nameWithOwner":"{parent}"{gp}}}}}"#)
            })
            .collect();
        format!(r#"{{"data":{{"repositoryOwner":{{"repositories":{{"nodes":[{}],"pageInfo":{{"hasNextPage":false,"endCursor":null}}}}}}}}}}"#, n.join(","))
    }

    fn ready_cfg() -> ForkConfig {
        ForkConfig { ready_timeout: Duration::from_secs(300), ..cfg() }
    }

    #[test]
    fn a_fork_with_a_suffix_is_found_by_listing_and_not_forked_again() {
        // `me/kora_core` is an unrelated repository; the fork GitHub made is `me/kora_core-1`.
        let f = Fake::new(|c| {
            if c.starts_with("gh api graphql") {
                ok(&listing(&[("me/kora_core-1", "KoraFiveM/kora_core", None), ("me/other", "zzz/other", None)]))
            } else if c.starts_with("gh api repos/me/kora_core-1/commits") {
                ok("1\n")
            } else if c.starts_with("gh api repos/me/kora_core") {
                ok(r#"{"fork":false}"#)
            } else {
                fail("unexpected")
            }
        });
        let mut st = ForkState::new("me", false, &repos(&["KoraFiveM/kora_core"]));
        let out = run_forks(&f, &mut st, &ready_cfg(), &mut |_| {}).unwrap();
        assert_eq!(out, ForkOutcome::Finished);
        assert!(st.is_complete(), "{st:?}");
        assert_eq!(f.count("gh repo fork"), 0);
        assert_eq!(f.count("gh api graphql"), 1);
        assert_eq!(st.fork_of("KoraFiveM/kora_core"), "me/kora_core-1");
        assert_eq!(st.counts(), (0, 1));
        assert_eq!(st.counts_text(), "reused 1 fork you already had");
        assert_eq!(st.last_event.as_deref(), Some("Using your existing fork me/kora_core-1"));
        assert!(f.sleeps.borrow().is_empty(), "no waiting on a fork that is ready");
    }

    #[test]
    fn a_renamed_fork_is_reused_and_synced_under_its_real_name() {
        let f = Fake::new(|c| {
            if c.starts_with("gh api graphql") {
                ok(&listing(&[("me/my-custom-name", "up/res", None)]))
            } else if c.contains("/compare/") {
                ok(r#"{"ahead_by":0,"behind_by":3}"#)
            } else {
                ok("")
            }
        });
        let mut st = ForkState::new("me", false, &repos(&["up/res"]));
        st.branches.insert("up/res".into(), "dev".into());
        run_forks(&f, &mut st, &sync_cfg(), &mut |_| {}).unwrap();
        assert!(st.is_complete(), "{st:?}");
        assert_eq!(f.count("gh repo fork"), 0);
        assert_eq!(f.count("gh api repos/me/my-custom-name/compare/up:dev...me:dev"), 1);
        assert_eq!(f.count("gh repo sync me/my-custom-name --branch dev"), 1);
        assert_eq!(st.sync[0].outcome, SyncOutcome::Synced { commits: 3 });
        assert_eq!(f.count("repos/me/res"), 0, "never looks at the upstream's name");
    }

    #[test]
    fn gh_reporting_an_existing_fork_under_another_name_uses_that_name() {
        // The listing does not show it (e.g. it could not see it), so gh is asked to fork and GitHub
        // hands back the existing fork `me/thing-1`.
        let f = Fake::new(|c| {
            if c.starts_with("gh api graphql") {
                ok(&listing(&[]))
            } else if c.starts_with("gh repo fork up/thing") {
                Output { code: 0, stdout: String::new(), stderr: "me/thing-1 already exists\n".into() }
            } else if c.starts_with("gh repo fork up/fresh") {
                ok("https://github.com/me/fresh-2\n")
            } else if c.starts_with("gh api repos/me/fresh-2/commits") {
                ok("1\n")
            } else {
                fail("HTTP 404")
            }
        });
        let mut st = ForkState::new("me", false, &repos(&["up/thing", "up/fresh"]));
        let mut c = ready_cfg();
        c.batch_size = 10;
        run_forks(&f, &mut st, &c, &mut |_| {}).unwrap();
        assert!(st.is_complete(), "{st:?}");
        assert_eq!(st.fork_of("up/thing"), "me/thing-1");
        assert_eq!(st.fork_of("up/fresh"), "me/fresh-2");
        assert_eq!(f.count("repos/me/thing/commits"), 0, "never waits on the upstream's name");
        assert!(f.sleeps.borrow().is_empty(), "no 300 s wait: {:?}", f.sleeps.borrow());
        assert_eq!(st.counts(), (1, 1));
        assert_eq!(st.counts_text(), "created 1 fork, reused 1 you already had");
        assert_eq!(f.count("gh api graphql"), 1, "one listing per run");
    }

    #[test]
    fn a_fork_of_a_fork_of_the_upstream_is_accepted() {
        // listing: parent is a friend's fork whose parent is the upstream
        let f = Fake::new(|c| if c.starts_with("gh api graphql") { ok(&listing(&[("me/x", "friend/x", Some("up/x"))])) } else { ok("") });
        let mut st = ForkState::new("me", false, &repos(&["up/x"]));
        run_forks(&f, &mut st, &cfg(), &mut |_| {}).unwrap();
        assert!(st.is_complete() && f.count("gh repo fork") == 0, "{st:?}");
        assert_eq!(st.fork_of("up/x"), "me/x");

        // same-name lookup (listing unavailable): parent is the friend's fork, source the upstream
        let f = Fake::new(|c| {
            if c.starts_with("gh api graphql") {
                fail("HTTP 502")
            } else if c.starts_with("gh api repos/me/x") {
                ok(r#"{"full_name":"me/x","fork":true,"parent":{"full_name":"friend/x"},"source":{"full_name":"up/x"}}"#)
            } else {
                ok("")
            }
        });
        let mut st = ForkState::new("me", false, &repos(&["up/x"]));
        run_forks(&f, &mut st, &cfg(), &mut |_| {}).unwrap();
        assert!(st.is_complete() && f.count("gh repo fork") == 0, "{st:?}");
    }

    #[test]
    fn a_failed_listing_falls_back_to_the_same_name_lookup() {
        let f = Fake::new(|c| {
            if c.starts_with("gh api graphql") {
                fail("error connecting to api.github.com")
            } else if c.starts_with("gh api repos/me/") {
                // GitHub follows a rename: the old name answers with the renamed fork
                ok(r#"{"full_name":"me/renamed","fork":true,"default_branch":"dev","parent":{"full_name":"up/res"}}"#)
            } else {
                ok("")
            }
        });
        let mut st = ForkState::new("me", false, &repos(&["up/res"]));
        run_forks(&f, &mut st, &cfg(), &mut |_| {}).unwrap();
        assert!(st.is_complete(), "{st:?}");
        assert_eq!(f.count("gh api repos/me/res"), 1);
        assert_eq!(f.count("gh repo fork"), 0, "a failed listing is not 'no forks'");
        assert_eq!(st.fork_of("up/res"), "me/renamed");
    }

    #[test]
    fn an_old_fork_state_file_still_loads() {
        let old = r#"{"target_owner":"me","target_is_org":false,"pending":[],"done":["up/a"],"failed":[{"repo":"up/b","error":"x"}],"branches":{},"sync":[],"api_calls":3}"#;
        let st: ForkState = serde_json::from_str(old).unwrap();
        assert!(st.forks.is_empty() && st.created.is_empty());
        assert_eq!(st.fork_of("up/a"), "me/a", "older files only ever accepted the same name");
        assert_eq!(st.counts(), (0, 1));
        let older = r#"{"target_owner":"me","target_is_org":false,"pending":["up/a"],"done":[],"failed":[]}"#;
        assert!(serde_json::from_str::<ForkState>(older).is_ok());
    }

    fn cache_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kora-forkcache-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.join("forks-cache.json")
    }

    #[test]
    fn the_cache_is_verified_before_use_and_stale_entries_are_dropped() {
        let p = cache_dir("verify");
        std::fs::write(&p, r#"{"owners":{"me":{"up/x":"me/x-renamed","up/y":"me/gone"},"other":{"up/x":"other/x"}}}"#).unwrap();
        let f = Fake::new(|c| {
            if c.starts_with("gh api graphql") {
                fail("HTTP 502")
            } else if c.starts_with("gh api repos/me/x-renamed") {
                ok(r#"{"full_name":"me/x-renamed","fork":true,"parent":{"full_name":"up/x"}}"#)
            } else if c.starts_with("gh api repos/me/gone") {
                fail("gh: Not Found (HTTP 404)")
            } else if c.starts_with("gh api repos/me/y") {
                ok(r#"{"full_name":"me/y","fork":true,"parent":{"full_name":"up/y"}}"#)
            } else {
                fail("unexpected")
            }
        });
        let mut st = ForkState::new("me", false, &repos(&["up/x", "up/y"]));
        let c = ForkConfig { cache_path: Some(p.clone()), batch_size: 10, ..cfg() };
        run_forks(&f, &mut st, &c, &mut |_| {}).unwrap();
        assert!(st.is_complete(), "{st:?}");
        assert_eq!(st.fork_of("up/x"), "me/x-renamed");
        assert_eq!(f.count("gh api repos/me/x "), 0, "the verified cache hit needs no same-name lookup");
        assert_eq!(st.fork_of("up/y"), "me/y");
        let saved: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(saved.pointer("/owners/me/up~1y").and_then(|v| v.as_str()), Some("me/y"), "{saved}");
        assert_eq!(saved.pointer("/owners/me/up~1x").and_then(|v| v.as_str()), Some("me/x-renamed"));
        assert_eq!(saved.pointer("/owners/other/up~1x").and_then(|v| v.as_str()), Some("other/x"), "other owners are kept");
        let leftovers: Vec<_> = std::fs::read_dir(p.parent().unwrap()).unwrap().filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().ends_with(".tmp")).collect();
        assert!(leftovers.is_empty());

        // A cached fork that is no longer a fork of the upstream is dropped, then looked up by name.
        std::fs::write(&p, r#"{"owners":{"me":{"up/z":"me/not-a-fork"}}}"#).unwrap();
        let f = Fake::new(|c| {
            if c.starts_with("gh api graphql") {
                fail("HTTP 502")
            } else if c.starts_with("gh api repos/me/not-a-fork") {
                ok(r#"{"full_name":"me/not-a-fork","fork":false}"#)
            } else if c.starts_with("gh api repos/me/z") {
                fail("HTTP 404")
            } else if c.starts_with("gh repo fork up/z") {
                ok("https://github.com/me/z\n")
            } else {
                fail("unexpected")
            }
        });
        let mut st = ForkState::new("me", false, &repos(&["up/z"]));
        run_forks(&f, &mut st, &c, &mut |_| {}).unwrap();
        assert!(st.is_complete(), "{st:?}");
        assert_eq!(st.fork_of("up/z"), "me/z");
        assert_eq!(st.counts(), (1, 0));
        let saved = std::fs::read_to_string(&p).unwrap();
        assert!(!saved.contains("not-a-fork") && saved.contains("\"me/z\""), "{saved}");
    }

    #[test]
    fn a_successful_listing_replaces_stale_cache_entries() {
        let p = cache_dir("listing");
        std::fs::write(&p, r#"{"owners":{"me":{"up/a":"me/old-a","up/b":"me/b"}}}"#).unwrap();
        let f = Fake::new(|c| {
            if c.starts_with("gh api graphql") {
                ok(&listing(&[("me/a-1", "up/a", None)]))
            } else if c.starts_with("gh repo fork up/b") {
                ok("https://github.com/me/b\n")
            } else {
                ok("")
            }
        });
        let mut st = ForkState::new("me", false, &repos(&["up/a", "up/b"]));
        run_forks(&f, &mut st, &ForkConfig { cache_path: Some(p.clone()), ..cfg() }, &mut |_| {}).unwrap();
        assert_eq!(st.fork_of("up/a"), "me/a-1");
        assert_eq!(f.count("gh repo fork up/a"), 0);
        // up/b's cached fork was not in the listing: forked again (GitHub returns it if it exists)
        assert_eq!(f.count("gh repo fork up/b"), 1);
        let saved = std::fs::read_to_string(&p).unwrap();
        assert!(saved.contains("me/a-1") && !saved.contains("me/old-a"), "{saved}");
    }

    #[test]
    fn fork_output_and_listing_parsing() {
        let o = |out: &str, err: &str| Output { code: 0, stdout: out.into(), stderr: err.into() };
        assert_eq!(forkfind::parse_fork_output(&o("https://github.com/Me/res-1\n", ""), "me").as_deref(), Some("Me/res-1"));
        assert_eq!(forkfind::parse_fork_output(&o("", "me/res-1 already exists\n"), "me").as_deref(), Some("me/res-1"));
        assert_eq!(forkfind::parse_fork_output(&o("", "✓ Created fork \u{1b}[1mme/res\u{1b}[0m\n"), "me").as_deref(), Some("me/res"));
        assert_eq!(forkfind::parse_fork_output(&o("", "! org/res already exists"), "me"), None, "another owner's repo is not ours");
        assert_eq!(forkfind::parse_fork_output(&o("", ""), "me"), None);
        // two pages, a collaborator repository that is not the owner's, a page with errors
        let two = format!("{}\n{}", listing(&[("me/a", "up/a", None), ("someone/b", "up/b", None)]), listing(&[("ME/c", "Up/C", None)]));
        let idx = forkfind::parse_listing(&two, "me").unwrap();
        assert_eq!(idx.len(), 2);
        assert_eq!(idx.find("up/c").unwrap().slug, "ME/c");
        assert!(idx.find("up/b").is_none());
        assert!(forkfind::parse_listing(r#"{"errors":[{"message":"x"}],"data":null}"#, "me").is_none());
        assert!(forkfind::parse_listing("", "me").is_none());
        assert!(forkfind::parse_listing(r#"{"data":{"repositoryOwner":null}}"#, "me").is_none());
    }

    #[test]
    fn classifier_table() {
        let j = |a: i64, b: i64| format!(r#"{{"status":"x","ahead_by":{a},"behind_by":{b},"total_commits":{a}}}"#);
        assert_eq!(classify_compare(&j(0, 0)), ForkSync::UpToDate);
        assert_eq!(classify_compare(&j(0, 7)), ForkSync::Behind(7));
        assert_eq!(classify_compare(&j(3, 0)), ForkSync::Ahead(3));
        assert_eq!(classify_compare(&j(2, 5)), ForkSync::Diverged { ahead: 2, behind: 5 });
        for bad in ["", "not json", "{}", r#"{"ahead_by":1}"#, r#"{"ahead_by":"1","behind_by":0}"#, r#"{"ahead_by":-1,"behind_by":0}"#] {
            assert!(matches!(classify_compare(bad), ForkSync::Unknown(_)), "{bad}");
        }
    }

    #[test]
    fn adoption_syncs_only_pure_behind_and_never_forces() {
        let f = github(&[("cur", 0, 0), ("behind", 0, 4), ("own", 2, 0), ("div", 3, 5)]);
        let mut st = ForkState::new("me", false, &repos(&["up/cur", "up/behind", "up/own", "up/div"]));
        st.branches.insert("up/cur".into(), "dev".into());
        let out = run_forks(&f, &mut st, &sync_cfg(), &mut |_| {}).unwrap();
        assert_eq!(out, ForkOutcome::Finished);
        assert!(st.is_complete(), "{st:?}");
        assert_eq!(f.count("gh repo sync"), 1);
        assert_eq!(f.count("gh repo sync me/behind --branch dev"), 1);
        assert_eq!(f.count("--force"), 0);
        assert_eq!(f.count("gh repo fork"), 0);
        let get = |n: &str| st.sync.iter().find(|s| s.repo == n).unwrap().outcome.clone();
        assert_eq!(get("up/cur"), SyncOutcome::UpToDate);
        assert_eq!(get("up/behind"), SyncOutcome::Synced { commits: 4 });
        assert_eq!(get("up/own"), SyncOutcome::Ahead { ahead: 2 });
        assert_eq!(get("up/div"), SyncOutcome::Diverged { ahead: 3, behind: 5 });
        // summary: counts and plain lines with the safe fix, only for the untouched ones
        let s = st.sync_summary(&|r| format!("resources/{}", repo_name(r)));
        assert_eq!((s.synced, s.up_to_date, s.left_alone), (1, 1, 2));
        assert_eq!(s.lines.len(), 2);
        assert!(s.lines[0].contains("2 commit(s) of its own") && s.lines[0].contains("nothing new"));
        assert!(s.lines[1].contains("cd resources/div && git fetch upstream && git merge upstream/dev") && s.lines[1].contains("Nothing was changed"), "{}", s.lines[1]);
        // survives a save/reload
        let again: ForkState = serde_json::from_str(&serde_json::to_string(&st).unwrap()).unwrap();
        assert_eq!(again.sync, st.sync);
    }

    #[test]
    fn refused_sync_is_diverged_and_not_retried_with_force() {
        // compare says behind-only, but gh refuses (the fork changed meanwhile)
        let n = std::cell::Cell::new(0);
        let f = Fake::new(move |c| {
            if c.contains("/compare/") {
                n.set(n.get() + 1);
                return ok(if n.get() == 1 { r#"{"ahead_by":0,"behind_by":2}"# } else { r#"{"ahead_by":1,"behind_by":2}"# });
            }
            if c.starts_with("gh repo sync") {
                return fail("can't sync because there are diverging changes; use --force");
            }
            ok(r#"{"fork":true,"default_branch":"dev"}"#)
        });
        let mut st = ForkState::new("me", false, &repos(&["up/x"]));
        run_forks(&f, &mut st, &sync_cfg(), &mut |_| {}).unwrap();
        assert_eq!(st.sync[0].outcome, SyncOutcome::Diverged { ahead: 1, behind: 2 });
        assert_eq!(f.count("gh repo sync"), 1);
        assert_eq!(f.count("--force"), 0);
        assert!(st.is_complete());
    }

    #[test]
    fn unknown_and_foreign_are_left_alone() {
        let f = Fake::new(|c| {
            if c.starts_with("gh api repos/me/foreign") {
                ok(r#"{"fork":true,"parent":{"full_name":"zzz/foreign"}}"#)
            } else if c.contains("/compare/") {
                fail("HTTP 500 boom")
            } else {
                ok(r#"{"fork":true,"default_branch":"dev"}"#)
            }
        });
        let mut st = ForkState::new("me", false, &repos(&["up/foreign", "up/odd"]));
        run_forks(&f, &mut st, &sync_cfg(), &mut |_| {}).unwrap();
        assert_eq!(st.failed.len(), 1);
        assert_eq!(st.failed[0].repo, "up/foreign");
        assert!(matches!(&st.sync[0].outcome, SyncOutcome::Unknown(w) if w.contains("boom")));
        assert_eq!(f.count("gh repo sync"), 0);
    }

    #[test]
    fn pacing_counts_api_calls_and_rate_limit_pauses_resumably() {
        // each adopted fork costs probe + compare (+ sync): batch_calls is 6
        let f = github(&[("a", 0, 1), ("b", 0, 1), ("c", 0, 0)]);
        let mut st = ForkState::new("me", false, &repos(&["up/a", "up/b", "up/c"]));
        run_forks(&f, &mut st, &sync_cfg(), &mut |_| {}).unwrap();
        // the one fork listing per run (unreadable here, so each repo is looked up by name) + a: 3
        // calls, b: 3 calls -> 6 reached -> one pause before c (2 calls)
        assert_eq!(*f.sleeps.borrow(), vec![Duration::from_secs(10)]);
        assert_eq!(st.api_calls, 9);
        assert_eq!(f.count("gh "), 9);
        assert_eq!(f.count("gh api graphql"), 1);

        // persistent rate limit on compare pauses; state stays resumable and nothing is lost
        let limited = std::rc::Rc::new(std::cell::Cell::new(true));
        let l2 = limited.clone();
        let f = Fake::new(move |c| {
            if c.contains("/compare/") {
                return if l2.get() { fail("HTTP 403: secondary rate limit") } else { ok(r#"{"ahead_by":0,"behind_by":0}"#) };
            }
            ok(r#"{"fork":true,"default_branch":"dev"}"#)
        });
        let mut st = ForkState::new("me", false, &repos(&["up/a"]));
        assert_eq!(run_forks(&f, &mut st, &sync_cfg(), &mut |_| {}).unwrap(), ForkOutcome::Paused);
        assert_eq!(st.pending, repos(&["up/a"]));
        limited.set(false);
        assert_eq!(run_forks(&f, &mut st, &sync_cfg(), &mut |_| {}).unwrap(), ForkOutcome::Finished);
        assert_eq!(st.sync[0].outcome, SyncOutcome::UpToDate);
    }

    #[test]
    fn sync_is_off_unless_asked_for() {
        let f = github(&[("behind", 0, 4)]);
        let mut st = ForkState::new("me", false, &repos(&["up/behind"]));
        run_forks(&f, &mut st, &cfg(), &mut |_| {}).unwrap();
        assert_eq!(f.count("gh repo sync") + f.count("/compare/"), 0);
        assert!(st.sync.is_empty());
    }

    #[test]
    fn paced_job_sync_stops_at_a_persistent_rate_limit() {
        let f = Fake::new(|c| {
            if c.contains("/compare/") {
                fail("HTTP 429")
            } else {
                ok("")
            }
        });
        let job = |i: &str| SyncJob {
            id: i.into(),
            fork: format!("me/{i}"),
            upstream: format!("up/{i}"),
            branch: "dev".into(),
        };
        let mut seen = vec![];
        let out = sync_forks_paced(&f, &[job("a"), job("b")], &sync_cfg(), &mut |d, t| {
            seen.push((d, t))
        });
        assert_eq!(seen, vec![(1, 2), (2, 2)], "progress after each fork");
        assert!(out
            .iter()
            .all(|(_, o)| matches!(o, SyncOutcome::Unknown(w) if w.contains("rate limit"))));
        assert_eq!(f.count("/compare/"), 5); // 1 + 4 retries, then b is not even tried
    }

    fn git_fake(status: &'static str, counts: &'static str) -> Fake {
        let heads = std::cell::Cell::new(0);
        Fake::new(move |c| {
            if c.starts_with("git status") {
                ok(status)
            } else if c.starts_with("git rev-list") {
                ok(counts)
            } else if c.starts_with("git rev-parse") {
                heads.set(heads.get() + 1);
                ok(if heads.get() == 1 { "aaaaaaa\n" } else { "bbbbbbb\n" })
            } else {
                ok("")
            }
        })
    }

    #[test]
    fn diverged_dirty_and_ahead_are_skipped_never_merged() {
        let p = Path::new("/x");
        let f = git_fake("", "2\t3\n");
        assert_eq!(
            update_ff_only(&f, p, "dev").unwrap(),
            UpdateResult::Skipped(RepoState::Diverged { ahead: 2, behind: 3 })
        );
        let f2 = git_fake(" M file.lua\n", "0\t3\n");
        assert_eq!(update_ff_only(&f2, p, "dev").unwrap(), UpdateResult::Skipped(RepoState::Dirty));
        let f3 = git_fake("", "1\t0\n");
        assert_eq!(
            update_ff_only(&f3, p, "dev").unwrap(),
            UpdateResult::Skipped(RepoState::LocalAhead { ahead: 1 })
        );
        for fk in [&f, &f2, &f3] {
            assert_eq!(fk.count("merge"), 0);
            assert_eq!(fk.count("git fetch upstream"), 1);
        }
    }

    #[test]
    fn clean_behind_fast_forwards_and_records_previous() {
        let f = git_fake("", "0\t4\n");
        let r = update_ff_only(&f, Path::new("/x"), "dev").unwrap();
        assert_eq!(r, UpdateResult::Updated { previous: "aaaaaaa".into(), current: "bbbbbbb".into() });
        assert_eq!(f.count("git merge --ff-only upstream/dev"), 1);
        let f = git_fake("", "0\t0\n");
        assert_eq!(update_ff_only(&f, Path::new("/x"), "dev").unwrap(), UpdateResult::UpToDate);
    }

    #[test]
    fn reset_and_pinned_clone_validate_and_order_commands() {
        let f = Fake::new(|_| ok(""));
        reset_to_commit(&f, Path::new("/x"), "abcdef1234").unwrap();
        assert!(reset_to_commit(&f, Path::new("/x"), "--hard").is_err());
        clone_pinned(&f, "kora/res", "abcdef1234", Path::new("/d")).unwrap();
        let calls = f.calls.borrow();
        assert_eq!(calls[0], "git reset --hard abcdef1234");
        assert_eq!(calls[1], "git clone --no-checkout https://github.com/kora/res.git /d");
        assert_eq!(calls[2], "git checkout --detach abcdef1234");
        assert_eq!(calls.len(), 3);
    }

    #[test]
    fn clone_from_fork_sets_upstream() {
        let f = Fake::new(|c| if c.contains("get-url") { fail("no such remote") } else { ok("") });
        clone_from_fork(&f, "me/res", "kora/res", Path::new("/d"), Some("dev")).unwrap();
        assert_eq!(f.count("git remote add upstream https://github.com/kora/res.git"), 1);
        assert_eq!(f.count("git checkout dev"), 1);
    }

    #[test]
    fn long_paths_are_enabled_for_git_on_windows_only() {
        assert!(windows_git_env(false, None).is_empty());
        assert!(windows_git_env(true, Some("2")).is_empty());
        assert!(windows_git_env(true, None).contains(&("GIT_CONFIG_VALUE_0", "true".to_string())));
    }

    #[test]
    fn windows_tool_folders_are_appended_once_and_case_insensitively() {
        let env = |k: &str| match k {
            "ProgramFiles" => Some("C:\\Program Files".to_string()),
            "LOCALAPPDATA" => Some("C:\\Users\\Zoë Müller\\AppData\\Local".to_string()),
            _ => None,
        };
        let dirs = windows_tool_dirs(&env);
        assert!(dirs.contains(&PathBuf::from("C:\\Program Files").join("Git").join("cmd")));
        let all = |_: &Path| true;
        let already = format!("C:\\Windows;{}\\", dirs[0].to_string_lossy().to_uppercase());
        let n = path_with(&already, &dirs, &all, ';').unwrap();
        assert_eq!(n.to_lowercase().matches(&dirs[0].to_string_lossy().to_lowercase()).count(), 1, "already present (other case): {n}");
        assert!(n.contains("GitHub CLI") && n.contains("Zoë Müller"));
        assert_eq!(path_with(&n, &dirs, &all, ';'), None, "second pass changes nothing");
        assert_eq!(path_with("C:\\Windows", &dirs, &|_: &Path| false, ';'), None, "missing folders are not added");
    }

    #[test]
    fn detection_and_auth_and_org() {
        let f = Fake::new(|c| match c {
            "git --version" => ok("git version 2.43.0\n"),
            "gh --version" => fail("not found"),
            c if c.starts_with("which") => ok("/usr/bin/git\n"),
            c if c.starts_with("gh auth status") => {
                Output { code: 0, stdout: String::new(), stderr: "  - Token scopes: 'read:org', 'repo'\n".into() }
            }
            "gh api orgs/nope" => fail("gh: Not Found (HTTP 404)"),
            "gh api orgs/yes" => ok("{}"),
            "gh api orgs/boom" => fail("HTTP 500"),
            _ => ok(""),
        });
        let t = detect(&f);
        assert_eq!(t.git.as_ref().unwrap().version, "2.43.0");
        assert!(t.gh.is_none());
        let a = auth_status(&f).unwrap();
        assert!(a.logged_in);
        assert!(a.missing_scopes(false).is_empty());
        assert_eq!(a.missing_scopes(true), vec!["admin:org".to_string()]);
        assert!(!org_exists(&f, "nope").unwrap());
        assert!(org_exists(&f, "yes").unwrap());
        assert!(org_exists(&f, "boom").is_err());
        assert!(org_exists(&f, "a/b").is_err());
        auth_login(&f, false).unwrap();
        assert_eq!(f.count("--scopes repo,read:org"), 1);
        assert_eq!(f.count("admin:org"), 0);
    }

    #[test]
    fn bootstrap_plans() {
        let none = Tools { git: None, gh: None };
        let both = Tools {
            git: Some(ToolInfo { version: "2".into(), path: None }),
            gh: Some(ToolInfo { version: "2".into(), path: None }),
        };
        assert!(plan_bootstrap(Os::DebianLike, &both).unwrap().is_empty());
        let deb = plan_bootstrap(Os::DebianLike, &none).unwrap();
        assert!(deb.iter().all(|s| s.needs_admin && !s.description.is_empty()));
        assert_eq!(deb.last().unwrap().args, vec!["install", "-y", "git"]);
        // gh never comes from apt: Kora downloads its own copy.
        assert!(!deb.iter().any(|s| s.args.join(" ").contains("gh") || s.args.join(" ").contains("cli.github.com")));
        let only_gh = Tools { git: both.git.clone(), gh: None };
        assert!(plan_bootstrap(Os::DebianLike, &only_gh).unwrap().is_empty());
        // Windows: nothing for the system to do (Kora's own MinGit and gh).
        assert!(plan_bootstrap(Os::Windows, &none).unwrap().is_empty());
        assert!(plan_bootstrap(Os::Unsupported, &none).is_err());
    }

    #[test]
    fn git_config_env_carries_the_private_gh_credential_helper_unless_the_caller_has_their_own_config() {
        let kv = |v: Vec<(String, String)>| v.into_iter().collect::<std::collections::BTreeMap<_, _>>();
        assert!(git_config_env(false, None, None).is_empty());
        let w = kv(git_config_env(true, None, None));
        assert_eq!((w["GIT_CONFIG_COUNT"].as_str(), w["GIT_CONFIG_KEY_0"].as_str()), ("1", "core.longpaths"));
        let c = kv(git_config_env(true, None, Some("!'/k/gh' auth git-credential")));
        assert_eq!(c["GIT_CONFIG_COUNT"], "3");
        // The empty value clears any helper listed before it; the next one is gh's.
        assert_eq!((c["GIT_CONFIG_KEY_1"].as_str(), c["GIT_CONFIG_VALUE_1"].as_str()), ("credential.https://github.com.helper", ""));
        assert_eq!(c["GIT_CONFIG_VALUE_2"], "!'/k/gh' auth git-credential");
        assert!(git_config_env(true, Some("2"), Some("x")).is_empty(), "the caller's own GIT_CONFIG_COUNT is left alone");
    }

    #[test]
    fn issue_and_pr_commands_are_built_safely() {
        let f = Fake::new(|l| if l.contains("issue create") { ok("Creating issue...\nhttps://github.com/KoraFiveM/kora_lib/issues/7\n") } else { ok("https://github.com/KoraFiveM/kora_lib/pull/9\n") });
        let url = create_issue(&f, "KoraFiveM/kora_lib", "It breaks; \"quoted\" $(x)", "line1\nline2").unwrap();
        assert_eq!(url, "https://github.com/KoraFiveM/kora_lib/issues/7");
        assert_eq!(f.calls.borrow()[0], "gh issue create --repo KoraFiveM/kora_lib --title It breaks; \"quoted\" $(x) --body line1\nline2");
        let url = create_pr(&f, "KoraFiveM/kora_lib", "me", "fix/x", "dev", "T", "B").unwrap();
        assert!(url.ends_with("/pull/9"));
        assert!(f.calls.borrow()[1].contains("--head me:fix/x --base dev"));
        // bad inputs never reach gh
        for bad in [("", "b"), ("t\nx", "b"), ("t", "")] {
            assert!(create_issue(&f, "KoraFiveM/kora_lib", bad.0, bad.1).is_err());
        }
        assert!(create_issue(&f, "no-slash", "t", "b").is_err());
        assert!(create_pr(&f, "KoraFiveM/kora_lib", "me", "--evil", "dev", "t", "b").is_err());
        assert_eq!(f.calls.borrow().len(), 2);
        // gh output without an address is an error, not a guess
        let g = Fake::new(|_| ok("done\n"));
        assert!(matches!(create_issue(&g, "a/b", "t", "b"), Err(GitError::Parse(..))));
    }

    #[test]
    fn remote_owner_parses_both_url_forms() {
        assert_eq!(owner_of_url("https://github.com/me/kora_lib.git").as_deref(), Some("me"));
        assert_eq!(owner_of_url("git@github.com:me/kora_lib.git").as_deref(), Some("me"));
        assert_eq!(owner_of_url("https://example.com/me/x"), None);
        assert_eq!(owner_of_url("https://github.com/-bad/x"), None);
    }

    #[test]
    fn login_arguments_use_the_minimum_scopes() {
        let a = login_args(false);
        assert!(a.contains(&"--web".to_string()) && a.contains(&"repo,read:org".to_string()));
        assert!(!a.iter().any(|x| x.contains("admin:org")));
        assert!(login_args(true).iter().any(|x| x.contains("admin:org")));
    }

    fn sam() -> crate::userhome::Account {
        crate::userhome::Account { name: "sam".into(), uid: 1000, gid: 1000, home: "/home/sam".into() }
    }

    #[test]
    fn under_sudo_gh_runs_as_the_user_and_git_asks_their_gh() {
        let u = sam();
        let ru = Path::new("/usr/sbin/runuser");
        let bus = Path::new("/run/user/1000/bus");
        let i = sudo_invocation("gh", &["auth", "status"], None, Some(&u), Some(ru), None, Some(bus));
        assert_eq!(i.program, "/usr/sbin/runuser");
        assert_eq!(i.args, ["-u", "sam", "--", "gh", "auth", "status"]);
        assert_eq!(i.cwd.as_deref(), Some(Path::new("/home/sam")), "never root's private folder");
        assert!(i.env.contains(&("DBUS_SESSION_BUS_ADDRESS".into(), "unix:path=/run/user/1000/bus".into())));
        // clone: git as root, credentials from the user's gh
        let i = sudo_invocation("gh", &["repo", "clone", "sam/kora_core", "/srv/x/resources/kora_core"], None, Some(&u), Some(ru), None, None);
        assert_eq!(i.program, "git");
        assert_eq!(i.args, ["clone", "--", "https://github.com/sam/kora_core.git", "/srv/x/resources/kora_core"]);
        assert!(i.env.contains(&("GIT_CONFIG_VALUE_1".into(), "!/usr/sbin/runuser -u sam -- gh auth git-credential".into())), "{:?}", i.env);
        assert!(i.env.contains(&("GIT_CONFIG_VALUE_0".into(), String::new())), "other helpers are reset for github.com only");
        let i = sudo_invocation("git", &["fetch", "origin"], Some(Path::new("/srv/x")), Some(&u), Some(ru), None, None);
        assert_eq!((i.program.as_str(), i.cwd.as_deref()), ("git", Some(Path::new("/srv/x"))));
        assert_eq!(i.env.len(), 5);
        // the caller's own git config list is left alone
        assert!(sudo_invocation("git", &["fetch"], None, Some(&u), Some(ru), Some("1"), None).env.is_empty());
    }

    fn env_of<'a>(i: &'a Invocation, k: &str) -> Option<&'a str> {
        i.env.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
    }

    #[test]
    fn git_as_root_never_reads_the_users_config_or_runs_hooks_but_keeps_their_gh_sign_in() {
        let u = sam();
        let ru = Path::new("/usr/sbin/runuser");
        let fetch = sudo_invocation("git", &["fetch", "origin"], Some(Path::new("/srv/x")), Some(&u), Some(ru), None, None);
        let i = harden_for_root(fetch, Some(true), Some(&u), Some(ru));
        assert_eq!(env_of(&i, "GIT_CONFIG_GLOBAL"), Some("/dev/null"), "~/.gitconfig of the user is not read");
        assert_eq!(env_of(&i, "GIT_CONFIG_NOSYSTEM"), Some("1"));
        let pairs: Vec<(&str, &str)> = (0..env_of(&i, "GIT_CONFIG_COUNT").unwrap().parse::<usize>().unwrap())
            .map(|n| (env_of(&i, &format!("GIT_CONFIG_KEY_{n}")).unwrap(), env_of(&i, &format!("GIT_CONFIG_VALUE_{n}")).unwrap()))
            .collect();
        assert!(pairs.contains(&("core.hooksPath", "/dev/null")) && pairs.contains(&("core.fsmonitor", "false")), "{pairs:?}");
        // Every other repo-local setting that runs a program is overridden too (a repository's own
        // `.git/config` is writable by whoever owns the folder).
        for k in ["core.sshCommand", "core.gitProxy", "core.askPass", "core.pager", "core.editor", "diff.external", "credential.helper", "protocol.ext.allow", "uploadpack.packObjectsHook"] {
            assert!(pairs.iter().any(|(pk, _)| *pk == k), "{k} must be pinned: {pairs:?}");
        }
        assert_eq!(env_of(&i, "GIT_TERMINAL_PROMPT"), Some("0"));
        assert!(pairs.contains(&("credential.https://github.com.helper", "!/usr/sbin/runuser -u sam -- gh auth git-credential")), "private forks still work: {pairs:?}");
        assert_eq!(i.env.iter().filter(|(k, _)| k == "GIT_CONFIG_COUNT").count(), 1);
        // The clone rewrite too, and even when the caller brought its own GIT_CONFIG_COUNT.
        let clone = sudo_invocation("gh", &["repo", "clone", "sam/a", "/srv/x/a"], None, Some(&u), Some(ru), Some("3"), None);
        let i = harden_for_root(clone, Some(true), Some(&u), Some(ru));
        assert_eq!((i.program.as_str(), env_of(&i, "GIT_CONFIG_GLOBAL")), ("git", Some("/dev/null")));
        // Root logged in directly (HOME is root's): its own config stays, hooks are still off.
        let plain = harden_for_root(sudo_invocation("git", &["status"], None, None, None, None, None), Some(false), None, None);
        assert!(env_of(&plain, "GIT_CONFIG_GLOBAL").is_none() && env_of(&plain, "GIT_CONFIG_KEY_0") == Some("core.hooksPath"));
        // gh itself runs as the user (not root): untouched. Not root at all: untouched.
        let gh = harden_for_root(sudo_invocation("gh", &["auth", "status"], None, Some(&u), Some(ru), None, None), Some(true), Some(&u), Some(ru));
        assert!(env_of(&gh, "GIT_CONFIG_GLOBAL").is_none());
        let me = harden_for_root(sudo_invocation("git", &["status"], None, None, None, None, None), None, None, None);
        assert!(me.env.is_empty());
    }

    #[test]
    fn without_sudo_or_runuser_nothing_changes() {
        let u = sam();
        let ru = Some(Path::new("/usr/sbin/runuser"));
        let plain = |i: Invocation, p: &str| assert!(i.program == p && i.env.is_empty() && i.cwd.is_none(), "{i:?}");
        plain(sudo_invocation("gh", &["auth", "status"], None, None, ru, None, None), "gh");
        plain(sudo_invocation("gh", &["auth", "status"], None, Some(&u), None, None, None), "gh");
        let root = crate::userhome::Account { name: "root".into(), uid: 0, gid: 0, home: "/root".into() };
        plain(sudo_invocation("gh", &["auth", "status"], None, Some(&root), ru, None, None), "gh");
        plain(sudo_invocation("apt-get", &["install", "gh"], None, Some(&u), ru, None, None), "apt-get");
        // an unexpected clone shape is not rewritten into git, it just runs as the user
        let i = sudo_invocation("gh", &["repo", "clone", "-x", "d"], None, Some(&u), ru, None, None);
        assert_eq!(i.args[..4], ["-u", "sam", "--", "gh"]);
    }

    #[test]
    fn move_pinned_fetches_only_when_the_commit_is_missing() {
        let sha = "0123456789abcdef0123456789abcdef01234567";
        let here = Fake::new(|l| {
            if l.starts_with("git rev-parse HEAD") {
                ok("aaa\n")
            } else {
                ok("")
            }
        });
        assert_eq!(move_pinned(&here, Path::new("/r"), sha).unwrap(), "aaa");
        assert_eq!(here.count("fetch"), 0, "{:?}", here.calls.borrow());
        assert_eq!(here.count("checkout --detach"), 1);
        let missing = Fake::new(|l| {
            if l.starts_with("git cat-file") {
                fail("fatal: Not a valid object name")
            } else {
                ok("")
            }
        });
        move_pinned(&missing, Path::new("/r"), sha).unwrap();
        assert_eq!(missing.count("git fetch origin"), 1);
    }
}
