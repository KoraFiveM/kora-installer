//! Branches and pull requests of ONE resource checkout (a Development server's clone of the person's fork).
//!
//! Everything here goes through the [`CommandRunner`], so tests use temporary repositories and a fake `gh`.
//! The rules, in one place:
//! - Never a force push, never a push at all, never a deleted branch or stash, nothing on the remote.
//! - Every value that reaches a command line is validated first ([`valid_branch`], [`PrInput`]).
//! - Uncommitted changes are never lost silently: the caller chooses [`Changes::Stash`] (kept aside, findable again with
//!   [`stashes`] / [`restore_stash`]) or [`Changes::Discard`]; with [`Changes::Ask`] nothing happens and the files are listed.
//! - A pull request is a local branch called `pr/<number>`: that name is how the rest of Kora recognises "trying PR #12".
//! - Failures are [`Problem`]s with plain words and actions, never a raw git message.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{check, run, run_ok, slug_of_url, valid_commit, valid_ref, CommandRunner, GitError, Output};
use crate::problem::{codes as pcodes, known, Action, Problem, Tool};

/// Stable codes of the problems raised here (`branch.*`).
pub mod codes {
    pub const NOT_DEV: &str = "branch.not_development";
    pub const NOT_REPO: &str = "branch.not_a_repository";
    pub const NO_NETWORK: &str = "branch.no_network";
    pub const BRANCH_GONE: &str = "branch.branch_gone";
    pub const BAD_NAME: &str = "branch.bad_name";
    pub const PR_NOT_FOUND: &str = "branch.pr_not_found";
    pub const PR_CLOSED: &str = "branch.pr_closed";
    pub const PR_FORK_GONE: &str = "branch.pr_fork_gone";
    pub const PR_OTHER_REPO: &str = "branch.pr_other_repository";
    pub const PR_REWRITTEN: &str = "branch.pr_rewritten";
    pub const NO_REMOTE: &str = "branch.no_remote";
    pub const STASH_CONFLICT: &str = "branch.stash_conflict";
    pub const STASH_NEEDS_CLEAN: &str = "branch.stash_needs_clean";
    pub const STASH_GONE: &str = "branch.stash_gone";
    pub const BUSY: &str = "branch.busy";
    pub const UNSUPPORTED: &str = "branch.unsupported";
}

/// Prefix of the local branch a pull request is checked out on.
pub const PR_PREFIX: &str = "pr/";
/// Every stash Kora makes starts with this (so [`stashes`] can tell them from the person's own).
pub const STASH_MARK: &str = "Kora:";
const MAX_FILES: usize = 30;
const FETCH: [&str; 4] = ["-c", "http.lowSpeedLimit=1000", "-c", "http.lowSpeedTime=30"];

// ---------------------------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------------------------

/// What is checked out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "SwitchKind"))]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// The resource's normal branch.
    Default,
    /// Another branch.
    Branch,
    /// A pull request (`pr/<n>`).
    Pr,
    /// No branch (a commit on its own).
    Detached,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "SwitchNow"))]
pub struct Where {
    pub kind: Kind,
    /// Local branch name; `None` when detached.
    pub branch: Option<String>,
    /// The pull request number when `kind` is `pr`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub pr: Option<u32>,
    /// Files with uncommitted changes (at most [`MAX_FILES`]) and the real count.
    pub changed_files: Vec<String>,
    pub changed_count: usize,
}

impl Where {
    pub fn dirty(&self) -> bool {
        self.changed_count > 0
    }
    /// Plain words: "the normal version (dev)", "pull request #12", "branch fix-hud", "a single commit".
    pub fn label(&self, default_branch: &str) -> String {
        match (self.kind, &self.branch, self.pr) {
            (Kind::Default, _, _) => format!("the normal version ({default_branch})"),
            (Kind::Pr, _, Some(n)) => format!("pull request #{n}"),
            (Kind::Detached, _, _) => "a single commit (no branch)".into(),
            (_, Some(b), _) => format!("branch {b}"),
            _ => "an unknown branch".into(),
        }
    }
}

/// The number in a `pr/<n>` (or `pr/<n>-<hash>`) branch name.
pub fn pr_number_of(branch: &str) -> Option<u32> {
    let rest = branch.strip_prefix(PR_PREFIX)?;
    rest.split('-').next()?.parse().ok()
}

/// What an update should say about a checkout that is not on `default_branch` ("on branch fix-hud", "trying pull request #12").
pub fn describe_other(current: &str) -> String {
    if current == "HEAD" || current.is_empty() {
        "on a single commit (no branch)".into()
    } else if let Some(n) = pr_number_of(current) {
        format!("trying pull request #{n}")
    } else {
        format!("on branch {current}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "BranchLists"))]
pub struct Branches {
    pub current: Option<String>,
    pub default_branch: String,
    /// Branches that exist in the folder.
    pub local: Vec<String>,
    /// Branches on the person's fork (`origin`).
    pub fork: Vec<String>,
    /// Branches on the original repository (`upstream`).
    pub upstream: Vec<String>,
}

/// A kept-aside set of changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "BranchStash"))]
pub struct Stash {
    /// Position in git's stash list (`stash@{index}`).
    pub index: u32,
    /// Plain description, e.g. "kept aside before leaving dev (to branch fix-hud)".
    pub message: String,
    /// The branch the changes were made on.
    pub branch: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "ChangesChoice"))]
#[serde(rename_all = "snake_case")]
pub enum Changes {
    /// Do nothing and say which files have changes.
    #[default]
    Ask,
    /// Keep them aside (git stash, including new files).
    Stash,
    /// Throw them away.
    Discard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "SwitchStatus"))]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Switched,
    /// Already there, nothing to do (or a pull request with nothing new).
    Already,
    /// There are uncommitted changes and no choice was given: nothing was touched.
    NeedsChoice,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "SwitchOutcome"))]
pub struct Outcome {
    pub status: Status,
    /// Where the checkout is now.
    pub now: Where,
    /// When changes were kept aside.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub stashed: Option<Stash>,
    /// When commits that belonged to no branch were saved on a branch of this name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub saved_branch: Option<String>,
    /// For pull requests: its title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub title: Option<String>,
    /// Plain sentence about what happened.
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum PrSource {
    /// On the original repository.
    Upstream,
    /// On the person's fork.
    Fork,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PullRequest {
    pub number: u32,
    pub title: String,
    pub author: String,
    pub source: PrSource,
    /// `owner/name` the pull request is on.
    pub repo: String,
    /// The branch it comes from.
    pub branch: String,
    /// `owner/name` the branch is in; `None` = that repository was deleted.
    pub from: Option<String>,
    pub draft: bool,
    pub url: String,
    pub updated: String,
    /// When it was opened.
    #[serde(default)]
    pub created: String,
    /// `APPROVED`, `CHANGES_REQUESTED`, `REVIEW_REQUIRED`, or empty when GitHub did not say.
    #[serde(default)]
    pub review: String,
    /// Whether its checks pass, fail, are still running, or there are none.
    #[serde(default = "no_checks")]
    pub checks: crate::collab::Checks,
}

fn no_checks() -> crate::collab::Checks {
    crate::collab::Checks::None
}

/// A pull request the person named: a number, or an address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrInput {
    pub number: u32,
    /// `owner/name` when the person pasted an address.
    pub repo: Option<String>,
}

/// `12`, `#12`, `https://github.com/owner/name/pull/12` (also with `/files`, `?x=y`, `#issuecomment`).
pub fn parse_pr_input(text: &str) -> Result<PrInput, Problem> {
    let bad = || Problem::new(codes::BAD_NAME, "That doesn't look like a pull request. Type its number (like 12) or paste its address from GitHub.");
    let t = text.trim();
    if t.is_empty() || t.len() > 300 {
        return Err(bad());
    }
    let number = |s: &str| -> Option<u32> {
        let ok = !s.is_empty() && s.len() <= 7 && s.bytes().all(|b| b.is_ascii_digit());
        ok.then(|| s.parse().ok()).flatten().filter(|n| *n > 0)
    };
    if let Some(n) = number(t.strip_prefix('#').unwrap_or(t)) {
        return Ok(PrInput { number: n, repo: None });
    }
    let rest = t.strip_prefix("https://github.com/").or_else(|| t.strip_prefix("http://github.com/")).or_else(|| t.strip_prefix("github.com/")).ok_or_else(bad)?;
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    let parts: Vec<&str> = rest.split('/').collect();
    if parts.len() < 4 || parts[2] != "pull" {
        return Err(bad());
    }
    let repo = format!("{}/{}", parts[0], parts[1]);
    if !super::valid_repo(&repo) {
        return Err(bad());
    }
    Ok(PrInput { number: number(parts[3]).ok_or_else(bad)?, repo: Some(repo) })
}

// ---------------------------------------------------------------------------------------------
// Validation and error mapping
// ---------------------------------------------------------------------------------------------

/// A branch name safe to put on a command line and to make a local branch of.
pub fn valid_branch(s: &str) -> bool {
    valid_ref(s)
        && s.len() <= 200
        && !s.contains("..")
        && !s.contains("//")
        && !s.ends_with('/')
        && !s.ends_with('.')
        && !s.ends_with(".lock")
        && !s.split('/').any(|p| p.starts_with('.') || p.is_empty())
}

fn bad_name(what: &str, v: &str) -> Problem {
    Problem::new(codes::BAD_NAME, format!("'{}' is not a {what} Kora can use.", v.chars().take(60).collect::<String>()))
}

/// Turn what git or gh said into something a person can act on.
pub fn problem_of(e: &GitError) -> Problem {
    match e {
        GitError::Spawn(cmd, why) if cmd.starts_with("gh ") => known::gh_missing().with_details(why.clone()),
        GitError::Spawn(_, why) => {
            Problem::new(pcodes::INSTALL_TOOL_MISSING, "Git is not installed, or Kora cannot find it.").with_details(why.clone()).with_action(Action::InstallTool { tool: Tool::Git })
        }
        GitError::Invalid(what, v) => bad_name(what, v),
        GitError::Parse(cmd, _) => Problem::new(pcodes::FAILED, "GitHub or git answered in a way Kora did not understand.").with_details(cmd.clone()).with_action(Action::CopyDetails),
        GitError::Command { cmd, stderr, .. } => {
            let low = stderr.to_ascii_lowercase();
            let is_gh = cmd.starts_with("gh ");
            if low.contains("not a git repository") {
                Problem::new(codes::NOT_REPO, "This resource's folder is not a git repository, so its branch cannot be changed.").with_action(Action::CopyDetails)
            } else if is_gh && (low.contains("gh auth login") || low.contains("not logged in") || low.contains("http 401") || low.contains("bad credentials") || low.contains("authentication")) {
                known::github_signed_out()
            } else if super::is_rate_limited(stderr) {
                Problem::new(pcodes::GH_RATE_LIMITED, "GitHub says to slow down. Wait a minute and try again.").with_action(Action::Retry)
            } else if [
                "could not resolve host",
                "unable to access",
                "timed out",
                "connection refused",
                "network is unreachable",
                "failed to connect",
                "operation too slow",
                "tls connection",
                "could not connect",
            ]
            .iter()
            .any(|m| low.contains(m))
            {
                Problem::new(codes::NO_NETWORK, "Kora could not reach GitHub. Check your internet connection and try again.").with_details(stderr.clone()).with_action(Action::Retry)
            } else {
                Problem::new(pcodes::FAILED, "Git could not do that.").with_details(format!("{cmd}: {}", stderr.chars().take(400).collect::<String>())).with_action(Action::CopyDetails)
            }
        }
    }
}

type PResult<T> = Result<T, Problem>;

fn pr<T>(r: super::Result<T>) -> PResult<T> {
    r.map_err(|e| problem_of(&e))
}

// ---------------------------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------------------------

/// Where `repo` is now. `default_branch` decides `Kind::Default`. Not a repository of its own: a Problem.
pub fn locate(r: &dyn CommandRunner, repo: &Path, default_branch: &str) -> PResult<Where> {
    // One call: branch and changes. `## main...origin/main`, `## HEAD (no branch)`, `## No commits yet on main`.
    let o = pr(run_ok(r, "git", &["status", "--porcelain=v1", "-b", "--untracked-files=normal"], Some(repo)))?;
    ensure_own_repo(r, repo)?;
    let mut lines = o.stdout.lines();
    let head = lines.next().unwrap_or("");
    let branch = parse_status_head(head);
    let files: Vec<&str> = lines.filter(|l| l.len() > 3).collect();
    let changed_files: Vec<String> = files.iter().take(MAX_FILES).map(|l| l[3..].trim().trim_matches('"').to_string()).collect();
    let (kind, pr_n) = match &branch {
        None => (Kind::Detached, None),
        Some(b) if b == default_branch => (Kind::Default, None),
        Some(b) => match pr_number_of(b) {
            Some(n) => (Kind::Pr, Some(n)),
            None => (Kind::Branch, None),
        },
    };
    Ok(Where { kind, branch, pr: pr_n, changed_count: files.len(), changed_files })
}

fn parse_status_head(head: &str) -> Option<String> {
    let h = head.strip_prefix("## ")?;
    if h.starts_with("HEAD (no branch)") {
        return None;
    }
    if let Some(b) = h.strip_prefix("No commits yet on ").or_else(|| h.strip_prefix("Initial commit on ")) {
        return Some(b.trim().to_string());
    }
    Some(h.split("...").next().unwrap_or(h).split(' ').next().unwrap_or(h).to_string())
}

/// The folder must be the top of its own repository (not a folder inside some bigger one).
fn ensure_own_repo(r: &dyn CommandRunner, repo: &Path) -> PResult<()> {
    let o = pr(run_ok(r, "git", &["rev-parse", "--show-toplevel"], Some(repo)))?;
    let top = std::fs::canonicalize(o.stdout.trim()).ok();
    let me = std::fs::canonicalize(repo).ok();
    match (top, me) {
        (Some(a), Some(b)) if a != b => Err(Problem::new(codes::NOT_REPO, "This resource's folder is not a git repository of its own, so its branch cannot be changed.")),
        _ => Ok(()),
    }
}

/// Branches of the folder, the fork and the original repository (what is already downloaded; see [`fetch_remotes`]).
pub fn branches(r: &dyn CommandRunner, repo: &Path, default_branch: &str) -> PResult<Branches> {
    let o = pr(run_ok(r, "git", &["for-each-ref", "--format=%(refname)", "refs/heads", "refs/remotes"], Some(repo)))?;
    let (mut local, mut fork, mut upstream) = (Vec::new(), Vec::new(), Vec::new());
    for l in o.stdout.lines().map(str::trim) {
        if let Some(b) = l.strip_prefix("refs/heads/") {
            local.push(b.to_string());
        } else if let Some(b) = l.strip_prefix("refs/remotes/origin/") {
            fork.push(b.to_string());
        } else if let Some(b) = l.strip_prefix("refs/remotes/upstream/") {
            upstream.push(b.to_string());
        }
    }
    for v in [&mut local, &mut fork, &mut upstream] {
        v.retain(|b| b != "HEAD" && valid_branch(b));
        v.sort_by_key(|a| (a != default_branch, a.to_lowercase()));
        v.dedup();
    }
    let current = pr(run(r, "git", &["symbolic-ref", "--short", "-q", "HEAD"], Some(repo)))?;
    let current = current.ok().then(|| current.stdout.trim().to_string()).filter(|s| !s.is_empty());
    Ok(Branches { current, default_branch: default_branch.to_string(), local, fork, upstream })
}

/// Download the branch lists of the fork and the original (network). A missing `upstream` is fine.
pub fn fetch_remotes(r: &dyn CommandRunner, repo: &Path) -> PResult<()> {
    for remote in ["origin", "upstream"] {
        let has = pr(run(r, "git", &["remote", "get-url", remote], Some(repo)))?.ok();
        if !has {
            continue;
        }
        let mut args: Vec<&str> = FETCH.to_vec();
        args.extend(["fetch", remote]);
        pr(run_ok(r, "git", &args, Some(repo)))?;
    }
    Ok(())
}

/// The sets of changes Kora kept aside in this folder, newest first.
pub fn stashes(r: &dyn CommandRunner, repo: &Path) -> PResult<Vec<Stash>> {
    let o = pr(run_ok(r, "git", &["stash", "list", "--format=%gd%x09%gs"], Some(repo)))?;
    Ok(o.stdout.lines().filter_map(parse_stash_line).collect())
}

fn parse_stash_line(l: &str) -> Option<Stash> {
    let (reference, subject) = l.split_once('\t')?;
    let index: u32 = reference.strip_prefix("stash@{")?.strip_suffix('}')?.parse().ok()?;
    // `On <branch>: Kora: <message>`
    let (on, msg) = subject.split_once(": ")?;
    let msg = msg.strip_prefix(STASH_MARK)?.trim();
    Some(Stash { index, message: msg.to_string(), branch: on.strip_prefix("On ").unwrap_or(on).to_string() })
}

// ---------------------------------------------------------------------------------------------
// Moving
// ---------------------------------------------------------------------------------------------

enum Prep {
    NeedsChoice,
    Ready { stashed: Option<Stash>, saved_branch: Option<String> },
}

fn branch_exists(r: &dyn CommandRunner, repo: &Path, full_ref: &str) -> PResult<bool> {
    Ok(pr(run(r, "git", &["show-ref", "--verify", "--quiet", full_ref], Some(repo)))?.ok())
}

/// Deal with uncommitted changes and with commits that belong to no branch, before leaving `now`.
fn prepare_leave(r: &dyn CommandRunner, repo: &Path, now: &Where, going_to: &str, changes: Changes) -> PResult<Prep> {
    let mut stashed = None;
    if now.dirty() {
        match changes {
            Changes::Ask => return Ok(Prep::NeedsChoice),
            Changes::Stash => {
                let from = now.branch.clone().unwrap_or_else(|| "a single commit".into());
                let msg = format!("{STASH_MARK} kept aside on {from} before switching to {going_to}");
                pr(run_ok(r, "git", &["stash", "push", "--include-untracked", "-m", &msg], Some(repo)))?;
                stashed = stashes(r, repo)?.into_iter().next();
            }
            Changes::Discard => {
                pr(run_ok(r, "git", &["reset", "--hard"], Some(repo)))?;
                pr(run_ok(r, "git", &["clean", "-fd"], Some(repo)))?;
            }
        }
    }
    // Leaving a commit that no branch has would make it hard to find again: give it a branch first.
    let mut saved_branch = None;
    if now.kind == Kind::Detached {
        let on_branch = pr(run_ok(r, "git", &["for-each-ref", "--contains", "HEAD", "--count=1", "--format=%(refname)", "refs/heads", "refs/remotes"], Some(repo)))?;
        if on_branch.stdout.trim().is_empty() {
            let short = pr(run_ok(r, "git", &["rev-parse", "--short=7", "HEAD"], Some(repo)))?.stdout.trim().to_string();
            let mut name = format!("kora-saved-{short}");
            let mut n = 1;
            while branch_exists(r, repo, &format!("refs/heads/{name}"))? {
                n += 1;
                name = format!("kora-saved-{short}-{n}");
            }
            pr(run_ok(r, "git", &["branch", &name, "HEAD"], Some(repo)))?;
            saved_branch = Some(name);
        }
    }
    Ok(Prep::Ready { stashed, saved_branch })
}

fn needs_choice(now: Where) -> Outcome {
    let n = now.changed_count;
    Outcome {
        status: Status::NeedsChoice,
        message: format!("{} {} changes of your own that haven't been saved anywhere. Choose what to do with them first.", plural_files(n), if n == 1 { "has" } else { "have" }),
        now,
        stashed: None,
        saved_branch: None,
        title: None,
    }
}

fn plural_files(n: usize) -> String {
    if n == 1 {
        "1 file".into()
    } else {
        format!("{n} files")
    }
}

fn finish(r: &dyn CommandRunner, repo: &Path, default_branch: &str, stashed: Option<Stash>, saved_branch: Option<String>, title: Option<String>, what: &str) -> PResult<Outcome> {
    let now = locate(r, repo, default_branch)?;
    let mut message = format!("Now on {}.", now.label(default_branch));
    if !what.is_empty() {
        message = what.to_string();
    }
    if stashed.is_some() {
        message.push_str(" Your changes were kept aside: you can bring them back later.");
    }
    if let Some(b) = &saved_branch {
        message.push_str(&format!(" The commit you were on was saved as branch {b}."));
    }
    Ok(Outcome { status: Status::Switched, now, stashed, saved_branch, title, message })
}

/// Switch to a branch. `remote` says where to take it from when the folder does not have it yet (`origin` = the fork,
/// `upstream` = the original); `None` looks at the folder first, then the fork, then the original.
pub fn switch_branch(r: &dyn CommandRunner, repo: &Path, default_branch: &str, name: &str, remote: Option<&str>, changes: Changes) -> PResult<Outcome> {
    if !valid_branch(name) {
        return Err(bad_name("branch name", name));
    }
    if let Some(rm) = remote {
        if !matches!(rm, "origin" | "upstream") {
            return Err(bad_name("place to take a branch from", rm));
        }
    }
    let now = locate(r, repo, default_branch)?;
    // What to check out, decided before anything is touched.
    let local_ref = format!("refs/heads/{name}");
    let have_local = branch_exists(r, repo, &local_ref)?;
    let from_remote = |rm: &str| -> PResult<bool> { branch_exists(r, repo, &format!("refs/remotes/{rm}/{name}")) };
    // (local branch name to use, remote branch to create it from)
    let plan: (String, Option<String>) = match remote {
        None if have_local => (name.to_string(), None),
        Some(rm) if have_local && tracks(r, repo, name)?.as_deref().is_none_or(|t| t == rm) => (name.to_string(), None),
        Some(rm) => {
            if !from_remote(rm)? {
                return Err(gone(name));
            }
            // A branch of the same name from the other place already exists here: do not mix them up.
            let local = if have_local { format!("{rm}-{name}") } else { name.to_string() };
            (local, Some(format!("{rm}/{name}")))
        }
        None => {
            let rm = if from_remote("origin")? {
                "origin"
            } else if from_remote("upstream")? {
                "upstream"
            } else {
                return Err(gone(name));
            };
            (name.to_string(), Some(format!("{rm}/{name}")))
        }
    };
    let (local, create_from) = plan;
    if now.branch.as_deref() == Some(local.as_str()) {
        return Ok(Outcome { status: Status::Already, message: format!("Already on {}.", now.label(default_branch)), now, stashed: None, saved_branch: None, title: None });
    }
    if create_from.is_some() && branch_exists(r, repo, &format!("refs/heads/{local}"))? {
        // `origin-name` exists too: the person already did this once; just go to it.
        return switch_branch(r, repo, default_branch, &local, None, changes);
    }
    let (stashed, saved_branch) = match prepare_leave(r, repo, &now, &local, changes)? {
        Prep::NeedsChoice => return Ok(needs_choice(now)),
        Prep::Ready { stashed, saved_branch } => (stashed, saved_branch),
    };
    match create_from {
        Some(from) => pr(run_ok(r, "git", &["checkout", "-b", &local, "--track", &from], Some(repo)))?,
        None => pr(run_ok(r, "git", &["checkout", &local], Some(repo)))?,
    };
    finish(r, repo, default_branch, stashed, saved_branch, None, "")
}

fn gone(name: &str) -> Problem {
    Problem::new(codes::BRANCH_GONE, format!("There is no branch called '{name}' here any more. It may have been deleted or renamed on GitHub. Refresh the list to see what exists."))
        .with_action(Action::Retry)
}

/// The remote a local branch follows (`origin`, `upstream`), when it follows one.
fn tracks(r: &dyn CommandRunner, repo: &Path, branch: &str) -> PResult<Option<String>> {
    let o = pr(run(r, "git", &["config", "--get", &format!("branch.{branch}.remote")], Some(repo)))?;
    Ok(o.ok().then(|| o.stdout.trim().to_string()).filter(|s| !s.is_empty()))
}

/// Back to the resource's normal branch. Creates it from the fork (or the original) when the folder lost it.
pub fn back_to_default(r: &dyn CommandRunner, repo: &Path, default_branch: &str, changes: Changes) -> PResult<Outcome> {
    switch_branch(r, repo, default_branch, default_branch, None, changes)
}

/// The commit checked out now.
pub fn head_commit(r: &dyn CommandRunner, repo: &Path) -> PResult<String> {
    Ok(pr(run_ok(r, "git", &["rev-parse", "HEAD"], Some(repo)))?.stdout.trim().to_string())
}

/// Back to the released version a server is pinned to: exactly that commit, on no branch (how a released resource is
/// installed). Kept changes and commits that belong to no branch are dealt with like in any other switch.
pub fn back_to_release(r: &dyn CommandRunner, repo: &Path, default_branch: &str, commit: &str, changes: Changes) -> PResult<Outcome> {
    check_commit(commit)?;
    let now = locate(r, repo, default_branch)?;
    let head = head_commit(r, repo)?;
    if now.kind == Kind::Detached && !head.is_empty() && (head.starts_with(commit) || commit.starts_with(&head)) {
        return Ok(Outcome { status: Status::Already, message: "Already on the released version.".into(), now, stashed: None, saved_branch: None, title: None });
    }
    let (stashed, saved_branch) = match prepare_leave(r, repo, &now, "the released version", changes)? {
        Prep::NeedsChoice => return Ok(needs_choice(now)),
        Prep::Ready { stashed, saved_branch } => (stashed, saved_branch),
    };
    let spec = format!("{commit}^{{commit}}");
    if !pr(run(r, "git", &["cat-file", "-e", &spec], Some(repo)))?.ok() {
        let mut args: Vec<&str> = FETCH.to_vec();
        args.extend(["fetch", "--tags", "origin"]);
        pr(run_ok(r, "git", &args, Some(repo)))?;
    }
    pr(run_ok(r, "git", &["checkout", "--detach", commit], Some(repo)))?;
    let what = format!("Back on the released version ({}).", &commit[..commit.len().min(7)]);
    finish(r, repo, default_branch, stashed, saved_branch, None, &what)
}

/// Switch the checkout to `name`'s tip only if safe; used by rollback to return to the branch a snapshot was taken on.
/// `"HEAD"` means "no branch". Never creates branches and never touches changes: a dirty conflict is an error.
pub(crate) fn switch_to(r: &dyn CommandRunner, repo: &Path, branch: &str) -> super::Result<()> {
    if branch == "HEAD" {
        run_ok(r, "git", &["checkout", "--detach"], Some(repo))?;
        return Ok(());
    }
    check("branch", valid_branch(branch), branch)?;
    run_ok(r, "git", &["checkout", branch], Some(repo))?;
    Ok(())
}

/// The branch checked out now (`None` when detached or unknown).
pub(crate) fn current(r: &dyn CommandRunner, repo: &Path) -> super::Result<Option<String>> {
    let o = run(r, "git", &["symbolic-ref", "--short", "-q", "HEAD"], Some(repo))?;
    if o.ok() {
        return Ok(Some(o.stdout.trim().to_string()).filter(|s| !s.is_empty()));
    }
    // Detached: say so (`HEAD`), but only when this really is a repository.
    run_ok(r, "git", &["rev-parse", "--git-dir"], Some(repo))?;
    Ok(Some("HEAD".to_string()))
}

// ---------------------------------------------------------------------------------------------
// Stashes
// ---------------------------------------------------------------------------------------------

/// Bring a set of kept-aside changes back. Needs a clean folder; a conflict leaves everything as it was (the set stays kept).
/// The stash is NOT deleted: it stays in the list so nothing can be lost.
pub fn restore_stash(r: &dyn CommandRunner, repo: &Path, default_branch: &str, index: u32) -> PResult<Outcome> {
    let now = locate(r, repo, default_branch)?;
    if now.dirty() {
        return Err(
            Problem::new(codes::STASH_NEEDS_CLEAN, "This resource has changes of its own right now. Put them aside or discard them first, then bring the old ones back.").with_action(Action::Retry)
        );
    }
    let list = stashes(r, repo)?;
    let Some(stash) = list.iter().find(|s| s.index == index).cloned() else {
        return Err(Problem::new(codes::STASH_GONE, "Those kept-aside changes are not there any more."));
    };
    let reference = format!("stash@{{{index}}}");
    let o = pr(run(r, "git", &["stash", "apply", &reference], Some(repo)))?;
    if !o.ok() {
        // Conflicts: put the folder back exactly as it was (it was clean), keep the stash.
        let _ = run(r, "git", &["reset", "--hard"], Some(repo));
        let _ = run(r, "git", &["clean", "-fd"], Some(repo));
        return Err(Problem::new(codes::STASH_CONFLICT, "Your kept-aside changes don't fit on top of this version (they touch the same lines). Nothing was changed, and they are still kept aside.")
            .with_details(first_lines(&o))
            .with_action(Action::show_command(&["git", "stash", "show", "-p", &reference], false))
            .with_action(Action::CopyDetails));
    }
    let after = locate(r, repo, default_branch)?;
    Ok(Outcome {
        status: Status::Switched,
        message: format!("Your changes are back ({}). They also stay in the kept-aside list, so nothing can be lost.", plural_files(after.changed_count)),
        now: after,
        stashed: Some(stash),
        saved_branch: None,
        title: None,
    })
}

fn first_lines(o: &Output) -> String {
    format!("{}\n{}", o.stdout.trim(), o.stderr.trim()).trim().lines().take(8).collect::<Vec<_>>().join("\n")
}

// ---------------------------------------------------------------------------------------------
// Pull requests (through gh)
// ---------------------------------------------------------------------------------------------

#[cfg(test)]
pub(crate) fn slugs_for_test(r: &dyn CommandRunner, repo: &Path) -> (Option<String>, Option<String>) {
    slugs(r, repo).unwrap()
}

fn slugs(r: &dyn CommandRunner, repo: &Path) -> PResult<(Option<String>, Option<String>)> {
    let get = |remote: &str| -> PResult<Option<String>> {
        let o = pr(run(r, "git", &["remote", "get-url", remote], Some(repo)))?;
        Ok(o.ok().then(|| slug_of_url(o.stdout.trim())).flatten())
    };
    let (up, origin) = (get("upstream")?, get("origin")?);
    // A plain clone has only `origin`, and that IS the original repository.
    if up.is_none() {
        return Ok((origin, None));
    }
    Ok((up, origin))
}

/// Open pull requests of the original repository and of the person's fork, labelled by where they are.
/// `(list, notes)`: notes are plain sentences about a side that could not be read.
pub fn list_prs(r: &dyn CommandRunner, repo: &Path) -> PResult<(Vec<PullRequest>, Vec<String>)> {
    let (up, fork) = slugs(r, repo)?;
    let mut sides: Vec<(PrSource, String)> = Vec::new();
    if let Some(u) = &up {
        sides.push((PrSource::Upstream, u.clone()));
    }
    if let Some(f) = &fork {
        // The user works on the original itself: one list is enough.
        if up.as_deref().is_none_or(|u| !u.eq_ignore_ascii_case(f)) {
            sides.push((PrSource::Fork, f.clone()));
        }
    }
    if sides.is_empty() {
        return Err(Problem::new(codes::NO_REMOTE, "This resource has no GitHub address to look for pull requests at."));
    }
    let mut out = Vec::new();
    let mut notes = Vec::new();
    let mut first_error: Option<Problem> = None;
    let mut worked = 0;
    for (source, slug) in &sides {
        let args = [
            "pr",
            "list",
            "--repo",
            slug.as_str(),
            "--state",
            "open",
            "--limit",
            "100",
            "--json",
            "number,title,author,headRefName,headRepository,headRepositoryOwner,isDraft,url,updatedAt,createdAt,reviewDecision,statusCheckRollup",
        ];
        match run_ok(r, "gh", &args, None) {
            Ok(o) => {
                worked += 1;
                out.extend(parse_pr_list(&o.stdout, *source, slug));
            }
            Err(e) => {
                let p = problem_of(&e);
                // Not signed in / no gh / no network: the same for both sides, so say it once, properly.
                if matches!(p.code.as_str(), pcodes::GH_MISSING | pcodes::GH_SIGNED_OUT | pcodes::GH_MISSING_SCOPES | codes::NO_NETWORK | pcodes::GH_RATE_LIMITED) {
                    return Err(p);
                }
                notes.push(format!("Could not read the pull requests on {slug} ({}).", p.message));
                first_error.get_or_insert(p);
            }
        }
    }
    if worked == 0 {
        return Err(first_error.unwrap_or_else(|| Problem::generic("Could not read the pull requests.")));
    }
    out.sort_by(|a, b| b.updated.cmp(&a.updated).then(a.number.cmp(&b.number)));
    Ok((out, notes))
}

fn parse_pr_list(json: &str, source: PrSource, slug: &str) -> Vec<PullRequest> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(json) else { return Vec::new() };
    items.iter().filter_map(|v| pr_from_json(v, source, slug)).collect()
}

fn pr_from_json(v: &Value, source: PrSource, slug: &str) -> Option<PullRequest> {
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let number = v.get("number")?.as_u64()?;
    let owner = v.pointer("/headRepositoryOwner/login").and_then(Value::as_str);
    let name = v.pointer("/headRepository/name").and_then(Value::as_str);
    let from = match (owner, name) {
        (Some(o), Some(n)) if !o.is_empty() && !n.is_empty() => Some(format!("{o}/{n}")),
        _ => None,
    };
    Some(PullRequest {
        number: u32::try_from(number).ok()?,
        title: s("title"),
        author: v.pointer("/author/login").and_then(Value::as_str).unwrap_or("someone").to_string(),
        source,
        repo: slug.to_string(),
        branch: s("headRefName"),
        from,
        draft: v.get("isDraft").and_then(Value::as_bool).unwrap_or(false),
        url: s("url"),
        updated: s("updatedAt"),
        created: s("createdAt"),
        review: s("reviewDecision"),
        checks: crate::collab::rollup(&v.get("statusCheckRollup").and_then(Value::as_array).cloned().unwrap_or_default()).0,
    })
}

struct PrDetails {
    title: String,
    state: String,
    url: String,
    head_gone: bool,
}

/// `gh pr view`; `Ok(None)` when that repository has no such pull request.
fn view_pr(r: &dyn CommandRunner, repo_slug: &str, number: u32) -> PResult<Option<PrDetails>> {
    let n = number.to_string();
    let args = ["pr", "view", n.as_str(), "--repo", repo_slug, "--json", "number,title,state,url,headRepository"];
    match run_ok(r, "gh", &args, None) {
        Ok(o) => {
            let v: Value = serde_json::from_str(&o.stdout).map_err(|_| problem_of(&GitError::Parse("gh pr view".into(), o.stdout.chars().take(100).collect())))?;
            Ok(Some(PrDetails {
                title: v.get("title").and_then(Value::as_str).unwrap_or("").to_string(),
                state: v.get("state").and_then(Value::as_str).unwrap_or("OPEN").to_string(),
                url: v.get("url").and_then(Value::as_str).unwrap_or("").to_string(),
                head_gone: v.get("headRepository").is_none_or(Value::is_null),
            }))
        }
        Err(GitError::Command { stderr, .. })
            if {
                let l = stderr.to_ascii_lowercase();
                l.contains("could not resolve to a pullrequest") || l.contains("no pull requests found") || l.contains("http 404")
            } =>
        {
            Ok(None)
        }
        Err(e) => Err(problem_of(&e)),
    }
}

fn remote_for(r: &dyn CommandRunner, repo: &Path, slug: &str) -> PResult<String> {
    for remote in ["upstream", "origin"] {
        let o = pr(run(r, "git", &["remote", "get-url", remote], Some(repo)))?;
        if o.ok() && slug_of_url(o.stdout.trim()).is_some_and(|s| s.eq_ignore_ascii_case(slug)) {
            return Ok(remote.to_string());
        }
    }
    Err(Problem::new(codes::NO_REMOTE, format!("This resource's folder is not connected to {slug}, so Kora cannot download that pull request.")))
}

/// Check out a pull request on a branch called `pr/<number>`. `source_hint` is where the picker found it; a bare number
/// without a hint is looked up on the original first, then on the fork. Trying it again later fetches its new commits.
#[allow(clippy::too_many_arguments)]
pub fn checkout_pr(r: &dyn CommandRunner, repo: &Path, default_branch: &str, input: &PrInput, source_hint: Option<PrSource>, changes: Changes) -> PResult<Outcome> {
    let (up, fork) = slugs(r, repo)?;
    let same = matches!((&up, &fork), (Some(u), Some(f)) if u.eq_ignore_ascii_case(f));
    let mut candidates: Vec<String> = Vec::new();
    match (&input.repo, source_hint) {
        (Some(want), _) => {
            let known = [up.as_ref(), fork.as_ref()].into_iter().flatten().find(|s| s.eq_ignore_ascii_case(want));
            match known {
                Some(s) => candidates.push(s.clone()),
                None => {
                    return Err(Problem::new(
                        codes::PR_OTHER_REPO,
                        format!("That pull request is on {want}, which is not this resource's repository or your fork of it. Kora can only try pull requests of those two."),
                    )
                    .with_action(Action::open_link(&format!("https://github.com/{want}/pull/{}", input.number))))
                }
            }
        }
        (None, Some(PrSource::Fork)) => candidates.extend(fork.clone()),
        (None, Some(PrSource::Upstream)) => candidates.extend(up.clone()),
        (None, None) => {
            candidates.extend(up.clone());
            if !same {
                candidates.extend(fork.clone());
            }
        }
    }
    if candidates.is_empty() {
        return Err(Problem::new(codes::NO_REMOTE, "This resource has no GitHub address to find pull requests at."));
    }
    let mut found: Option<(String, PrDetails)> = None;
    for slug in &candidates {
        if let Some(d) = view_pr(r, slug, input.number)? {
            found = Some((slug.clone(), d));
            break;
        }
    }
    let Some((slug, d)) = found else {
        return Err(Problem::new(codes::PR_NOT_FOUND, format!("There is no pull request #{} on {}. Check the number.", input.number, candidates.join(" or "))));
    };
    match d.state.as_str() {
        "OPEN" => {}
        "MERGED" => {
            return Err(Problem::new(
                codes::PR_CLOSED,
                format!("Pull request #{} has already been merged, so there is nothing separate to try. Go back to the normal version to get it.", input.number),
            )
            .with_action(Action::open_link(&d.url)))
        }
        _ => return Err(Problem::new(codes::PR_CLOSED, format!("Pull request #{} was closed without being merged.", input.number)).with_action(Action::open_link(&d.url))),
    }
    let remote = remote_for(r, repo, &slug)?;
    let n = input.number;
    // GitHub keeps every pull request's commits on the repository it was opened against, even when the fork it came
    // from is gone, so this one fetch works for both. Into a private ref of ours; the branch is made after.
    let private = format!("refs/kora/pr-{n}");
    let spec = format!("+refs/pull/{n}/head:{private}");
    let mut args: Vec<&str> = FETCH.to_vec();
    args.extend(["fetch", remote.as_str(), spec.as_str()]);
    if let Err(e) = run_ok(r, "git", &args, Some(repo)) {
        let p = problem_of(&e);
        if d.head_gone && p.code == pcodes::FAILED {
            return Err(Problem::new(codes::PR_FORK_GONE, format!("The fork that pull request #{n} came from has been deleted, and GitHub no longer has its commits."))
                .with_details(p.details.unwrap_or_default())
                .with_action(Action::open_link(&d.url)));
        }
        return Err(p);
    }
    let sha = pr(run_ok(r, "git", &["rev-parse", &private], Some(repo)))?.stdout.trim().to_string();
    check_commit(&sha)?;
    let now = locate(r, repo, default_branch)?;
    // The local branch: `pr/12`, fast-forwarded when the author added commits; a new one when they rewrote history.
    let base = format!("{PR_PREFIX}{n}");
    let exists = branch_exists(r, repo, &format!("refs/heads/{base}"))?;
    let (local, fast_forward) = if !exists {
        (base.clone(), false)
    } else {
        let is_anc = |a: &str, b: &str| -> PResult<bool> { Ok(pr(run(r, "git", &["merge-base", "--is-ancestor", a, b], Some(repo)))?.ok()) };
        let tip = format!("refs/heads/{base}");
        let tip_sha = pr(run_ok(r, "git", &["rev-parse", &tip], Some(repo)))?.stdout.trim().to_string();
        if tip_sha == sha {
            (base.clone(), false)
        } else if is_anc(&tip, &sha)? {
            (base.clone(), true)
        } else if is_anc(&sha, &tip)? {
            // Nothing new (the branch already has everything).
            (base.clone(), false)
        } else {
            (format!("{base}-{}", &sha[..7]), false)
        }
    };
    let up_to_date = exists && local == base && !fast_forward;
    if up_to_date && now.branch.as_deref() == Some(local.as_str()) {
        return Ok(Outcome {
            status: Status::Already,
            message: format!("Already trying pull request #{n}, and there is nothing new in it."),
            now,
            stashed: None,
            saved_branch: None,
            title: Some(d.title),
        });
    }
    let (stashed, saved_branch) = match prepare_leave(r, repo, &now, &format!("pull request #{n}"), changes)? {
        Prep::NeedsChoice => return Ok(needs_choice(now)),
        Prep::Ready { stashed, saved_branch } => (stashed, saved_branch),
    };
    if branch_exists(r, repo, &format!("refs/heads/{local}"))? {
        pr(run_ok(r, "git", &["checkout", &local], Some(repo)))?;
        if fast_forward {
            if let Err(e) = run_ok(r, "git", &["merge", "--ff-only", &sha], Some(repo)) {
                return Err(problem_of(&e));
            }
        }
    } else {
        pr(run_ok(r, "git", &["checkout", "-b", &local, &sha], Some(repo)))?;
    }
    let what = if fast_forward {
        format!("Now trying pull request #{n}, with its newest commits.")
    } else if local != base {
        format!("Now trying pull request #{n}. Its author rewrote it since you last tried it, so it is on a new branch ({local}); your earlier copy is still there.")
    } else {
        format!("Now trying pull request #{n}.")
    };
    finish(r, repo, default_branch, stashed, saved_branch, Some(d.title), &what)
}

fn check_commit(sha: &str) -> PResult<()> {
    if valid_commit(sha) {
        Ok(())
    } else {
        Err(Problem::generic("Git gave an unexpected answer."))
    }
}

// ---------------------------------------------------------------------------------------------
// Tests (real git in temporary folders, a fake gh)
// ---------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests;
