//! git/gh bootstrap and wrapper; shells out to `git` and `gh` (no embedded git library).
//!
//! Every external command goes through [`CommandRunner`], so tests use a fake and nothing here
//! talks to the real GitHub. Bootstrap installation is only *planned* ([`plan_bootstrap`]); the
//! caller decides when (and with which privileges) to run the described steps.

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
}

/// Real runner using `std::process::Command`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn run(&self, program: &str, args: &[&str], cwd: Option<&Path>) -> std::io::Result<Output> {
        let mut c = std::process::Command::new(program);
        c.args(args);
        if let Some(d) = cwd {
            c.current_dir(d);
        }
        // Never block on interactive prompts from git.
        c.env("GIT_TERMINAL_PROMPT", "0");
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
    Some(ToolInfo { version, path: locate(r, tool) })
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

pub fn detect(r: &dyn CommandRunner) -> Tools {
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

const GH_KEYRING: &str = "/etc/apt/keyrings/githubcli-archive-keyring.gpg";

/// Plan what is needed to get missing tools installed. Empty when nothing is missing.
/// Returns `Err(Invalid)` for unsupported OSes.
pub fn plan_bootstrap(os: Os, tools: &Tools) -> Result<Vec<PlanStep>> {
    let need_git = tools.git.is_none();
    let need_gh = tools.gh.is_none();
    let mut steps = Vec::new();
    if !need_git && !need_gh {
        return Ok(steps);
    }
    match os {
        Os::DebianLike => {
            if need_gh {
                steps.push(step(
                    "Install wget and certificates (needed to fetch the GitHub CLI package key)",
                    true,
                    "apt-get",
                    &["install", "-y", "wget", "ca-certificates"],
                ));
                steps.push(step(
                    "Create the folder for package signing keys",
                    true,
                    "mkdir",
                    &["-p", "-m", "755", "/etc/apt/keyrings"],
                ));
                steps.push(step(
                    "Download GitHub's official package signing key",
                    true,
                    "sh",
                    &[
                        "-c",
                        &format!(
                            "wget -nv -O {GH_KEYRING} https://cli.github.com/packages/githubcli-archive-keyring.gpg && chmod go+r {GH_KEYRING}"
                        ),
                    ],
                ));
                steps.push(step(
                    "Add GitHub's official package source for the GitHub CLI",
                    true,
                    "sh",
                    &[
                        "-c",
                        &format!(
                            "echo \"deb [arch=$(dpkg --print-architecture) signed-by={GH_KEYRING}] https://cli.github.com/packages stable main\" > /etc/apt/sources.list.d/github-cli.list"
                        ),
                    ],
                ));
            }
            steps.push(step("Refresh the package list", true, "apt-get", &["update"]));
            let mut args = vec!["install", "-y"];
            let mut names = Vec::new();
            if need_git {
                args.push("git");
                names.push("git");
            }
            if need_gh {
                args.push("gh");
                names.push("the GitHub CLI (gh)");
            }
            steps.push(step(&format!("Install {}", names.join(" and ")), true, "apt-get", &args));
        }
        Os::Windows => {
            // winget ships with Windows 10 1809+/11 and Server 2025; no Chocolatey needed.
            let common = ["--exact", "--silent", "--accept-package-agreements", "--accept-source-agreements"];
            if need_git {
                let mut a = vec!["install", "--id", "Git.Git"];
                a.extend(common);
                steps.push(step("Install Git for Windows using winget", true, "winget", &a));
            }
            if need_gh {
                let mut a = vec!["install", "--id", "GitHub.cli"];
                a.extend(common);
                steps.push(step("Install the GitHub CLI (gh) using winget", true, "winget", &a));
            }
        }
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

/// Device/browser flow via gh. gh keeps the token in its own store.
pub fn auth_login(r: &dyn CommandRunner, admin_org: bool) -> Result<()> {
    let scopes = required_scopes(admin_org).join(",");
    run_ok(
        r,
        "gh",
        &["auth", "login", "--web", "--hostname", "github.com", "--git-protocol", "https", "--scopes", &scopes],
        None,
    )?;
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
}

impl ForkState {
    pub fn new(target_owner: &str, target_is_org: bool, repos: &[String]) -> Self {
        ForkState {
            target_owner: target_owner.to_string(),
            target_is_org,
            pending: repos.to_vec(),
            done: vec![],
            failed: vec![],
        }
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
}

impl Default for ForkConfig {
    fn default() -> Self {
        ForkConfig {
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

/// What is at `<target>/<name>` before we fork.
enum ForkProbe {
    Missing,
    /// A fork of the expected upstream (or an answer we cannot inspect).
    Present,
    /// A repository with that name exists but is not a fork of the upstream: never reuse it.
    Foreign(String),
}

/// Does `<target>/<name>` already exist, and is it really a fork of `upstream`? (prevents
/// duplicate forks after a crash/resume and refuses to adopt an unrelated repository.)
fn probe_fork(r: &dyn CommandRunner, target: &str, name: &str, upstream: &str) -> Result<ForkProbe> {
    let path = format!("repos/{target}/{name}");
    let o = run(r, "gh", &["api", &path], None)?;
    if o.ok() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&o.stdout) {
            if let Some(is_fork) = v.get("fork").and_then(|f| f.as_bool()) {
                let parent = v.pointer("/parent/full_name").and_then(|p| p.as_str()).unwrap_or("");
                if !is_fork {
                    return Ok(ForkProbe::Foreign(format!("{target}/{name} exists but is not a fork of {upstream}")));
                }
                if !parent.is_empty() && !parent.eq_ignore_ascii_case(upstream) {
                    return Ok(ForkProbe::Foreign(format!("{target}/{name} is a fork of {parent}, not {upstream}")));
                }
            }
        }
        return Ok(ForkProbe::Present);
    }
    if is_rate_limited(&o.stderr) {
        return Err(GitError::Command { cmd: format!("gh api {path}"), code: o.code, stderr: o.stderr });
    }
    Ok(ForkProbe::Missing)
}

/// Wait until the (asynchronously created) fork has its commits, so a clone right after succeeds.
fn wait_fork_ready(r: &dyn CommandRunner, target: &str, name: &str, cfg: &ForkConfig) -> std::result::Result<(), String> {
    if cfg.ready_timeout.is_zero() {
        return Ok(());
    }
    let path = format!("repos/{target}/{name}/commits?per_page=1");
    let mut waited = Duration::ZERO;
    loop {
        if let Ok(o) = r.run("gh", &["api", &path, "--jq", "length"], None) {
            if o.ok() && o.stdout.trim() == "1" {
                return Ok(());
            }
        }
        if waited >= cfg.ready_timeout {
            return Err(format!("fork {target}/{name} was created but is still not ready after {}s", cfg.ready_timeout.as_secs()));
        }
        r.sleep(cfg.ready_poll);
        waited += cfg.ready_poll.max(Duration::from_millis(1));
    }
}

/// Run (or resume) forking. `on_progress` is called after every state change so the caller can
/// persist `state`. Never forks a repo already in `done` or already present at the target.
pub fn run_forks(
    r: &dyn CommandRunner,
    state: &mut ForkState,
    cfg: &ForkConfig,
    on_progress: &mut dyn FnMut(&ForkState),
) -> Result<ForkOutcome> {
    check("target owner", valid_name(&state.target_owner), &state.target_owner)?;
    let mut in_batch = 0usize;
    while let Some(repo) = state.pending.first().cloned() {
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
            let o = match probe_fork(r, &state.target_owner, repo_name(&repo), &repo) {
                // already there: no new fork, but it may still be filling up after a crash
                Ok(ForkProbe::Present) => break wait_fork_ready(r, &state.target_owner, repo_name(&repo), cfg),
                Ok(ForkProbe::Foreign(why)) => break Err(why),
                Ok(ForkProbe::Missing) => {
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
                break wait_fork_ready(r, &state.target_owner, repo_name(&repo), cfg);
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
            break Err(o.stderr.trim().to_string());
        };
        state.pending.remove(0);
        match result {
            Ok(()) => state.done.push(repo),
            Err(error) => state.failed.push(FailedFork { repo, error }),
        }
        on_progress(state);
        in_batch += 1;
        if in_batch >= cfg.batch_size && !state.pending.is_empty() {
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

/// Prod update: move an existing pinned clone to a newer pinned commit.
pub fn move_pinned(r: &dyn CommandRunner, repo: &Path, commit: &str) -> Result<String> {
    check("commit", valid_commit(commit), commit)?;
    let previous = head_commit(r, repo)?;
    run_ok(r, "git", &["fetch", "origin"], Some(repo))?;
    checkout_pinned(r, repo, commit)?;
    Ok(previous)
}

#[cfg(test)]
mod tests {
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
        }
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
                    ok("")
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
                c2.borrow_mut().push(name);
                return ok("");
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
        assert!(deb.iter().any(|s| s.args.join(" ").contains("cli.github.com/packages")));
        assert_eq!(deb.last().unwrap().args, vec!["install", "-y", "git", "gh"]);
        let only_git = Tools { git: None, gh: both.gh.clone() };
        let w = plan_bootstrap(Os::Windows, &only_git).unwrap();
        assert_eq!(w.len(), 1);
        assert!(w[0].args.contains(&"Git.Git".to_string()));
        assert!(plan_bootstrap(Os::Unsupported, &none).is_err());
    }
}
