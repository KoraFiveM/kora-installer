//! The everyday GitHub loop for one resource, without leaving the panel: see what changed, commit it, push it, read a
//! pull request (checks, reviews, files, conversation), comment, approve or ask for changes, merge, read issues and
//! releases, and make your own copy (fork) of a resource that is a plain clone.
//!
//! Threat model (the same rules for every function here):
//! - It runs `git` and `gh` only through [`CommandRunner`] with fixed argument lists. Nothing from a browser or a command
//!   line is ever put in a shell; every name, number and method is validated first and a value that could be read as an
//!   option never starts an argument.
//! - Only inside the server's `resources` folder (the same folder rule as [`crate::branches`]).
//! - Never a forced push, never a deleted remote branch, never rewritten history. A push is `git push origin <branch>` of
//!   the branch that is checked out, and git itself refuses anything that is not a fast-forward.
//! - Whose GitHub account acts: the one `gh` on the panel's computer is signed in as, shared by every panel user. Every
//!   result that acts on GitHub says `acting_as`, so the panel can say "as <login>". The routes that act need the admin
//!   role (see `remote/roles.rs`).
//! - GitHub text (titles, bodies, comments) is shown as plain text by the panel, never as HTML, and is cut to a fixed length
//!   here.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::branches::{find, folder, lock_for};
use crate::git::branches::{self as gb, codes, problem_of, valid_branch};
use crate::git::{run, run_ok, CommandRunner, GitError};
use crate::installer::registry::{Registry, ServerEntry, ServerFile};
use crate::installer::{Source, Versions};
use crate::problem::{Action, Problem};

type PResult<T> = Result<T, Problem>;

pub const NEEDS_FORK: &str = "collab.needs_fork";
pub const NO_IDENTITY: &str = "collab.no_identity";
pub const NOTHING_TO_COMMIT: &str = "collab.nothing_to_commit";
pub const NO_PUSH_RIGHTS: &str = "collab.no_push_rights";
pub const MERGE_REFUSED: &str = "collab.merge_refused";
pub const NOT_ON_A_BRANCH: &str = "collab.not_on_a_branch";

const MAX_DIFF: usize = 60_000;
const MAX_FILES: usize = 200;
const MAX_TEXT: usize = 4_000;
const MAX_MESSAGE: usize = 2_000;
const MAX_BODY: usize = 10_000;
const MAX_LOG: usize = 30;

fn cut(s: &str, max: usize) -> String {
    let mut out: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        out.push('…');
    }
    out
}

fn text_ok(what: &str, s: &str, max: usize) -> PResult<()> {
    if s.trim().is_empty() {
        return Err(Problem::new(codes::BAD_NAME, format!("Write {what} first.")));
    }
    if s.chars().count() > max || s.contains('\0') {
        return Err(Problem::new(codes::BAD_NAME, format!("That {what} is too long, or has characters Kora cannot send.")));
    }
    Ok(())
}

fn pr<T>(r: crate::git::Result<T>) -> PResult<T> {
    r.map_err(|e| problem_of(&e))
}

/// `owner/name` of a remote, when it is a GitHub address.
fn remote_slug(r: &dyn CommandRunner, repo: &Path, remote: &str) -> PResult<Option<String>> {
    let o = pr(run(r, "git", &["remote", "get-url", remote], Some(repo)))?;
    Ok(o.ok().then(|| crate::git::slug_of_url(o.stdout.trim())).flatten())
}

/// The GitHub login `gh` acts as (the panel computer's sign-in), when it can be read.
pub fn acting_as(r: &dyn CommandRunner) -> Option<String> {
    let o = run(r, "gh", &["api", "user", "--jq", ".login"], None).ok()?;
    let login = o.stdout.trim().to_string();
    (o.ok() && !login.is_empty() && login.len() <= 64 && login.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')).then_some(login)
}

// ---------------------------------------------------------------------------------------------
// Where this resource's repositories are
// ---------------------------------------------------------------------------------------------

/// The repositories of a resource: the original, and the person's own copy when there is one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Repos {
    /// `owner/name` of the original repository.
    pub original: Option<String>,
    /// `owner/name` of the person's own copy (fork), when there is one.
    pub fork: Option<String>,
}

/// A plain clone has only `origin` (the original). A fork has `origin` (the copy) and `upstream` (the original).
pub fn repos(r: &dyn CommandRunner, dir: &Path) -> PResult<Repos> {
    let up = remote_slug(r, dir, "upstream")?;
    let origin = remote_slug(r, dir, "origin")?;
    Ok(match up {
        Some(u) => Repos { original: Some(u), fork: origin },
        None => Repos { original: origin, fork: None },
    })
}

fn needs_fork(what: &str) -> Problem {
    Problem::new(NEEDS_FORK, format!("{what} needs your own copy (a fork) of this resource on GitHub, and this one is a plain copy of the original. Choose \"Make my own copy (fork)\" first."))
        .with_action(Action::MakeFork { resource: None })
}

// ---------------------------------------------------------------------------------------------
// Local changes, history
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ChangedFile {
    pub path: String,
    /// `modified`, `added`, `deleted`, `renamed` or `new` (not yet known to git).
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LocalChanges {
    pub branch: Option<String>,
    pub files: Vec<ChangedFile>,
    /// The real number of changed files (`files` stops at 200).
    pub count: usize,
    /// A readable diff (read-only), cut at about 60 000 characters.
    pub diff: String,
    pub diff_truncated: bool,
    /// Commits on this branch that are not on its remote yet, and the other way round. `None` = no remote branch.
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub repos: Repos,
}

fn status_word(x: char, y: char) -> &'static str {
    match (x, y) {
        ('?', '?') => "new",
        ('A', _) | (_, 'A') => "added",
        ('D', _) | (_, 'D') => "deleted",
        ('R', _) | (_, 'R') => "renamed",
        _ => "modified",
    }
}

/// `git status --porcelain=v1 -z` read strictly: `XY path` entries separated by NUL (a renamed file has a second path).
pub fn parse_status(z: &str) -> Vec<ChangedFile> {
    let mut out = Vec::new();
    let mut parts = z.split('\0');
    while let Some(e) = parts.next() {
        let mut c = e.chars();
        let (Some(x), Some(y), Some(' ')) = (c.next(), c.next(), c.next()) else { continue };
        let path: String = c.collect();
        if x == 'R' || y == 'R' || x == 'C' || y == 'C' {
            // The old path follows; the new one is what is shown.
            let _ = parts.next();
        }
        out.push(ChangedFile { path, status: status_word(x, y).to_string() });
    }
    out
}

/// Files whose contents are never shown in a diff.
const SECRET_FILES: &[&str] = &[
    "**/secrets.cfg",
    "**/server.cfg",
    "**/.env",
    "**/.env.*",
    "**/*.env",
    "**/*.pem",
    "**/*.key",
    "**/id_rsa*",
    "**/id_ed25519*",
    "**/*secret*",
    "**/*credential*",
    "**/*password*",
    "**/*.p12",
    "**/*.pfx",
];

fn is_secret_file(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    ["secrets.cfg", "server.cfg", ".env", "id_rsa", "id_ed25519"].iter().any(|n| name == *n || name.starts_with(&format!("{n}.")))
        || [".env", ".pem", ".key", ".p12", ".pfx"].iter().any(|e| name.ends_with(e))
        || ["secret", "credential", "password"].iter().any(|w| name.contains(w))
}

/// Hide what looks like a secret in diff text: `cfxk_` licence keys anywhere, and the value after `=` or `:` on a line whose
/// name says key, token, password, secret or licence.
pub fn hide_secrets(diff: &str) -> String {
    const WORDS: &[&str] =
        &["password", "passwd", "secret", "token", "api_key", "apikey", "api-key", "licen", "private_key", "webhook", "authorization", "bearer", "connection_string", "mysql_connection"];
    let cleaned = crate::store::jobs::redact_text(diff);
    cleaned
        .lines()
        .map(|l| {
            let low = l.to_ascii_lowercase();
            let body = l.get(1..).unwrap_or("");
            let changed_line = l.starts_with('+') || l.starts_with('-') || l.starts_with(' ');
            if changed_line && WORDS.iter().any(|w| low.contains(w)) {
                if let Some(i) = body.find(['=', ':']) {
                    return format!("{}{}{} <hidden>", &l[..1], &body[..i], &body[i..=i]);
                }
            }
            l.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
        + if diff.ends_with('\n') { "\n" } else { "" }
}

pub fn local_changes(r: &dyn CommandRunner, dir: &Path, default_branch: &str) -> PResult<LocalChanges> {
    let here = gb::locate(r, dir, default_branch)?;
    let st = pr(run_ok(r, "git", &["status", "--porcelain=v1", "-z", "--untracked-files=normal"], Some(dir)))?;
    let all = parse_status(&st.stdout);
    let count = all.len();
    // Tracked changes against the last commit; new files are listed above but have no diff until they are committed.
    let head_ok = pr(run(r, "git", &["rev-parse", "--verify", "-q", "HEAD"], Some(dir)))?.ok();
    let mut diff = String::new();
    if head_ok && count > 0 {
        // Files that usually hold secrets are left out of the diff entirely (their names are still listed above), and the
        // rest has secret-looking values hidden: a diff is shown in the panel and must never leak a key or a password.
        let mut args: Vec<String> = ["-c", "core.quotepath=false", "diff", "--no-color", "--no-ext-diff", "HEAD", "--", "."].iter().map(|x| x.to_string()).collect();
        args.extend(SECRET_FILES.iter().map(|g| format!(":(exclude,glob){g}")));
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let d = pr(run_ok(r, "git", &args, Some(dir)))?;
        diff = hide_secrets(&d.stdout);
        let hidden: Vec<&str> = all.iter().filter(|f| is_secret_file(&f.path)).map(|f| f.path.as_str()).collect();
        for h in hidden {
            diff.push_str(&format!("\n# {h}: its contents are not shown here because it may hold secrets\n"));
        }
    }
    let diff_truncated = diff.chars().count() > MAX_DIFF;
    let diff = cut(&diff, MAX_DIFF);
    let (mut ahead, mut behind) = (None, None);
    if let Some(b) = &here.branch {
        let spec = format!("{b}@{{upstream}}...{b}");
        let o = pr(run(r, "git", &["rev-list", "--left-right", "--count", &spec], Some(dir)))?;
        if o.ok() {
            let mut n = o.stdout.split_whitespace().map(|x| x.parse::<u32>().ok());
            behind = n.next().flatten();
            ahead = n.next().flatten();
        }
    }
    Ok(LocalChanges { branch: here.branch, files: all.into_iter().take(MAX_FILES).collect(), count, diff, diff_truncated, ahead, behind, repos: repos(r, dir)? })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LogEntry {
    pub sha: String,
    pub subject: String,
    pub author: String,
    /// Unix time of the commit.
    pub when: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Log {
    pub branch: Option<String>,
    pub commits: Vec<LogEntry>,
}

pub fn log(r: &dyn CommandRunner, dir: &Path, default_branch: &str) -> PResult<Log> {
    let here = gb::locate(r, dir, default_branch)?;
    let n = format!("-{MAX_LOG}");
    let o = pr(run(r, "git", &["log", &n, "--format=%H%x1f%an%x1f%ct%x1f%s"], Some(dir)))?;
    let commits = if o.ok() {
        o.stdout
            .lines()
            .filter_map(|l| {
                let mut p = l.split('\u{1f}');
                let sha = p.next()?.to_string();
                if !crate::git::valid_commit(&sha) {
                    return None;
                }
                Some(LogEntry { sha, author: cut(p.next()?, 80), when: p.next()?.parse().ok()?, subject: cut(p.next().unwrap_or(""), 200) })
            })
            .collect()
    } else {
        Vec::new()
    };
    Ok(Log { branch: here.branch, commits })
}

// ---------------------------------------------------------------------------------------------
// Commit and push
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Committed {
    pub commit: String,
    pub files: usize,
    pub branch: Option<String>,
}

/// Commit everything that changed in the resource with `message`. Needs a name and email for git (the person's own);
/// without them the answer says how to set them.
pub fn commit(r: &dyn CommandRunner, dir: &Path, default_branch: &str, message: &str) -> PResult<Committed> {
    text_ok("a message for the commit", message, MAX_MESSAGE)?;
    let here = gb::locate(r, dir, default_branch)?;
    if here.changed_count == 0 {
        return Err(Problem::new(NOTHING_TO_COMMIT, "There is nothing to save: no file in this resource has changed."));
    }
    pr(run_ok(r, "git", &["add", "-A"], Some(dir)))?;
    let o = pr(run(r, "git", &["commit", "-m", message], Some(dir)))?;
    if !o.ok() {
        let low = format!("{}{}", o.stdout, o.stderr).to_ascii_lowercase();
        if low.contains("please tell me who you are") || low.contains("unable to auto-detect email") || low.contains("empty ident") {
            return Err(Problem::new(NO_IDENTITY, "Git does not know your name and email yet, so it cannot sign a commit. Run these two commands once, with your own name and email, then try again.")
                .with_action(Action::show_command(&["git", "config", "--global", "user.name", "Your Name"], false))
                .with_action(Action::show_command(&["git", "config", "--global", "user.email", "you@example.com"], false)));
        }
        return Err(problem_of(&GitError::Command { cmd: "git commit".into(), code: o.code, stderr: format!("{}{}", o.stderr, o.stdout) }));
    }
    let sha = gb::head_commit(r, dir)?;
    Ok(Committed { commit: sha, files: here.changed_count, branch: here.branch })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Pushed {
    pub branch: String,
    /// `owner/name` it went to.
    pub to: String,
    /// The branch did not exist on GitHub before.
    pub new_branch: bool,
    pub acting_as: Option<String>,
}

/// Push the checked-out branch to the person's own copy (`origin`). Never forced; refused on a plain clone (nothing of
/// yours to push to), on no branch, and for a pull request being tried (push your own branch instead).
pub fn push(r: &dyn CommandRunner, dir: &Path, default_branch: &str) -> PResult<Pushed> {
    let here = gb::locate(r, dir, default_branch)?;
    let Some(branch) = here.branch.clone() else {
        return Err(Problem::new(NOT_ON_A_BRANCH, "This resource is on a single commit, not on a branch, so there is nothing to push. Make a new branch from it first."));
    };
    if !valid_branch(&branch) {
        return Err(Problem::new(codes::BAD_NAME, "This branch has a name Kora will not push."));
    }
    if branch.starts_with(gb::PR_PREFIX) {
        return Err(Problem::new(NOT_ON_A_BRANCH, "This is someone else's pull request that you are trying. It is not yours to push. Make a new branch from it first if you want to build on it."));
    }
    let repos = repos(r, dir)?;
    let Some(fork) = repos.fork.clone() else {
        return Err(needs_fork("Pushing"));
    };
    let remote_branch = format!("refs/remotes/origin/{branch}");
    let new_branch = !pr(run(r, "git", &["show-ref", "--verify", "--quiet", &remote_branch], Some(dir)))?.ok();
    let o = pr(run(r, "git", &["push", "--set-upstream", "origin", &branch], Some(dir)))?;
    if !o.ok() {
        let low = o.stderr.to_ascii_lowercase();
        if low.contains("permission") || low.contains("403") || low.contains("denied") || low.contains("not allowed") {
            return Err(Problem::new(
                NO_PUSH_RIGHTS,
                format!("Your GitHub account is not allowed to push to {fork}. Ask the owner of that repository to give you write access, or push to your own copy."),
            )
            .with_details(cut(&o.stderr, 400))
            .with_action(Action::open_link(&format!("https://github.com/{fork}/settings/access"))));
        }
        if low.contains("non-fast-forward") || low.contains("rejected") || low.contains("fetch first") {
            return Err(Problem::new(
                codes::BRANCH_GONE,
                "GitHub has commits on this branch that you do not have, so Kora will not push over them (it never forces a push). Get them first, or push under a new branch name.",
            )
            .with_details(cut(&o.stderr, 400)));
        }
        return Err(problem_of(&GitError::Command { cmd: "git push".into(), code: o.code, stderr: o.stderr }));
    }
    Ok(Pushed { branch, to: fork, new_branch, acting_as: acting_as(r) })
}

/// Make a new branch from what is checked out now (changes come along), and switch to it.
pub fn new_branch(r: &dyn CommandRunner, dir: &Path, default_branch: &str, name: &str) -> PResult<gb::Where> {
    if !valid_branch(name) || name.starts_with(gb::PR_PREFIX) {
        return Err(Problem::new(codes::BAD_NAME, format!("'{}' is not a branch name Kora can use.", cut(name, 60))));
    }
    gb::locate(r, dir, default_branch)?;
    let exists = pr(run(r, "git", &["show-ref", "--verify", "--quiet", &format!("refs/heads/{name}")], Some(dir)))?.ok();
    if exists {
        return Err(Problem::new(codes::BAD_NAME, format!("There is already a branch called '{name}' here. Pick another name.")));
    }
    pr(run_ok(r, "git", &["checkout", "-b", name], Some(dir)))?;
    gb::locate(r, dir, default_branch)
}

// ---------------------------------------------------------------------------------------------
// Pull requests (through gh)
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "ChecksState"))]
#[serde(rename_all = "snake_case")]
pub enum Checks {
    Passing,
    Failing,
    Pending,
    /// The repository runs no checks on it.
    None,
}

/// One state out of a `statusCheckRollup` entry (check runs and old-style statuses).
fn check_state(v: &Value) -> Checks {
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_ascii_uppercase();
    let (status, conclusion, state) = (s("status"), s("conclusion"), s("state"));
    if !state.is_empty() {
        return match state.as_str() {
            "SUCCESS" => Checks::Passing,
            "FAILURE" | "ERROR" => Checks::Failing,
            _ => Checks::Pending,
        };
    }
    if !status.is_empty() && status != "COMPLETED" {
        return Checks::Pending;
    }
    match conclusion.as_str() {
        "SUCCESS" | "NEUTRAL" | "SKIPPED" => Checks::Passing,
        "" => Checks::Pending,
        _ => Checks::Failing,
    }
}

/// The overall state of a list of checks: any failing = failing, else any pending = pending, else passing; none = none.
pub fn rollup(items: &[Value]) -> (Checks, Vec<Check>) {
    let list: Vec<Check> = items
        .iter()
        .take(60)
        .map(|v| Check {
            name: cut(v.get("name").or_else(|| v.get("context")).and_then(Value::as_str).unwrap_or("check"), 120),
            state: check_state(v),
            url: v.get("detailsUrl").or_else(|| v.get("targetUrl")).and_then(Value::as_str).filter(|u| u.starts_with("https://")).map(str::to_string),
        })
        .collect();
    let overall = if list.is_empty() {
        Checks::None
    } else if list.iter().any(|c| c.state == Checks::Failing) {
        Checks::Failing
    } else if list.iter().any(|c| c.state == Checks::Pending) {
        Checks::Pending
    } else {
        Checks::Passing
    };
    (overall, list)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Check {
    pub name: String,
    pub state: Checks,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PrFile {
    pub path: String,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PrComment {
    pub author: String,
    pub body: String,
    pub created: String,
    /// `comment`, or a review: `approved`, `changes_requested`, `commented`.
    pub kind: String,
}

/// A pull request that is checked out here: how it stands against its newest commit on GitHub.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Trying {
    pub branch: String,
    /// Commits GitHub has that this folder does not (`None` = the folder has not downloaded its newest commit: get the latest).
    pub behind: Option<u32>,
    /// Commits made here on top of it.
    pub ahead: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PrView {
    pub number: u32,
    pub title: String,
    pub body: String,
    pub author: String,
    /// `OPEN`, `MERGED` or `CLOSED`.
    pub state: String,
    pub draft: bool,
    pub url: String,
    /// `owner/name` it is on.
    pub repo: String,
    pub base: String,
    pub head: String,
    pub created: String,
    pub updated: String,
    /// `APPROVED`, `CHANGES_REQUESTED`, `REVIEW_REQUIRED` or empty (no reviews needed).
    pub review: String,
    pub checks: Checks,
    pub check_list: Vec<Check>,
    /// `MERGEABLE`, `CONFLICTING` or `UNKNOWN`.
    pub mergeable: String,
    pub files: Vec<PrFile>,
    pub comments: Vec<PrComment>,
    /// Which ways of merging the repository allows: `merge`, `squash`, `rebase`.
    pub merge_methods: Vec<String>,
    /// Whether the signed-in account may merge here (`ADMIN`, `MAINTAIN` or `WRITE` on the repository).
    pub can_merge: bool,
    pub acting_as: Option<String>,
    /// Set when this pull request is the one checked out in the resource.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub trying: Option<Trying>,
}

const PR_FIELDS: &str = "number,title,body,author,state,isDraft,url,baseRefName,headRefName,headRefOid,createdAt,updatedAt,reviewDecision,statusCheckRollup,mergeable,files,comments,reviews";

fn pr_slug(r: &dyn CommandRunner, dir: &Path, source: Option<&str>) -> PResult<String> {
    let repos = repos(r, dir)?;
    let slug = match source {
        Some("fork") => repos.fork,
        _ => repos.original.or(repos.fork),
    };
    slug.ok_or_else(|| Problem::new(codes::NO_REMOTE, "This resource has no GitHub address to look for pull requests at."))
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

fn login(v: &Value) -> String {
    v.pointer("/author/login").and_then(Value::as_str).unwrap_or("someone").to_string()
}

/// Strictly read the answer of `gh pr view --json ...`.
pub fn parse_pr_view(json: &str, repo: &str) -> Option<(PrView, String)> {
    let v: Value = serde_json::from_str(json).ok()?;
    let number = u32::try_from(v.get("number")?.as_u64()?).ok()?;
    let rollup_items: Vec<Value> = v.get("statusCheckRollup").and_then(Value::as_array).cloned().unwrap_or_default();
    let (checks, check_list) = rollup(&rollup_items);
    let files = v
        .get("files")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .take(MAX_FILES)
                .filter_map(|f| {
                    Some(PrFile {
                        path: cut(f.get("path")?.as_str()?, 300),
                        additions: f.get("additions").and_then(Value::as_u64).unwrap_or(0).min(u64::from(u32::MAX)) as u32,
                        deletions: f.get("deletions").and_then(Value::as_u64).unwrap_or(0).min(u64::from(u32::MAX)) as u32,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let mut comments: Vec<PrComment> = v
        .get("comments")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|c| PrComment { author: login(c), body: cut(&s(c, "body"), MAX_TEXT), created: s(c, "createdAt"), kind: "comment".into() }).collect())
        .unwrap_or_default();
    if let Some(a) = v.get("reviews").and_then(Value::as_array) {
        comments.extend(a.iter().map(|c| PrComment { author: login(c), body: cut(&s(c, "body"), MAX_TEXT), created: s(c, "submittedAt"), kind: s(c, "state").to_ascii_lowercase() }));
    }
    comments.sort_by(|a, b| a.created.cmp(&b.created));
    comments.truncate(100);
    let head_oid = s(&v, "headRefOid");
    Some((
        PrView {
            number,
            title: cut(&s(&v, "title"), 300),
            body: cut(&s(&v, "body"), MAX_TEXT),
            author: login(&v),
            state: s(&v, "state"),
            draft: v.get("isDraft").and_then(Value::as_bool).unwrap_or(false),
            url: Some(s(&v, "url")).filter(|u| u.starts_with("https://")).unwrap_or_default(),
            repo: repo.to_string(),
            base: cut(&s(&v, "baseRefName"), 200),
            head: cut(&s(&v, "headRefName"), 200),
            created: s(&v, "createdAt"),
            updated: s(&v, "updatedAt"),
            review: s(&v, "reviewDecision"),
            checks,
            check_list,
            mergeable: Some(s(&v, "mergeable")).filter(|m| !m.is_empty()).unwrap_or_else(|| "UNKNOWN".into()),
            files,
            comments,
            merge_methods: Vec::new(),
            can_merge: false,
            acting_as: None,
            trying: None,
        },
        head_oid,
    ))
}

/// Which ways of merging a repository allows, and whether the signed-in account may merge (from `gh repo view`).
pub fn parse_repo_rights(json: &str) -> (Vec<String>, bool) {
    let Ok(v) = serde_json::from_str::<Value>(json) else { return (Vec::new(), false) };
    let on = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
    let mut m = Vec::new();
    if on("mergeCommitAllowed") {
        m.push("merge".to_string());
    }
    if on("squashMergeAllowed") {
        m.push("squash".to_string());
    }
    if on("rebaseMergeAllowed") {
        m.push("rebase".to_string());
    }
    let can = matches!(v.get("viewerPermission").and_then(Value::as_str), Some("ADMIN" | "MAINTAIN" | "WRITE"));
    (m, can)
}

/// A pull request in full. `source` is `fork` for a pull request on the person's own copy, else the original.
pub fn pr_view(r: &dyn CommandRunner, dir: &Path, default_branch: &str, number: u32, source: Option<&str>) -> PResult<PrView> {
    let slug = pr_slug(r, dir, source)?;
    let n = number.to_string();
    let o = pr(run_ok(r, "gh", &["pr", "view", &n, "--repo", &slug, "--json", PR_FIELDS], None))?;
    let (mut view, head_oid) = parse_pr_view(&o.stdout, &slug).ok_or_else(|| problem_of(&GitError::Parse("gh pr view".into(), cut(&o.stdout, 100))))?;
    if let Ok(o) = run_ok(r, "gh", &["repo", "view", &slug, "--json", "mergeCommitAllowed,squashMergeAllowed,rebaseMergeAllowed,viewerPermission"], None) {
        let (m, can) = parse_repo_rights(&o.stdout);
        view.merge_methods = m;
        view.can_merge = can;
    }
    view.acting_as = acting_as(r);
    // Is this the one being tried here?
    if let Ok(here) = gb::locate(r, dir, default_branch) {
        if let (Some(b), Some(pn)) = (&here.branch, here.pr) {
            if pn == number && crate::git::valid_commit(&head_oid) {
                let have = pr(run(r, "git", &["cat-file", "-e", &format!("{head_oid}^{{commit}}")], Some(dir)))?.ok();
                let (behind, ahead) = if have {
                    let o = pr(run(r, "git", &["rev-list", "--left-right", "--count", &format!("HEAD...{head_oid}")], Some(dir)))?;
                    let mut it = o.stdout.split_whitespace().map(|x| x.parse::<u32>().ok());
                    let ahead = it.next().flatten().unwrap_or(0);
                    (it.next().flatten(), ahead)
                } else {
                    (None, 0)
                };
                view.trying = Some(Trying { branch: b.clone(), behind, ahead });
            }
        }
    }
    Ok(view)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Acted {
    pub message: String,
    pub acting_as: Option<String>,
    pub url: Option<String>,
}

fn acted(r: &dyn CommandRunner, message: &str, url: Option<String>) -> Acted {
    Acted { message: message.to_string(), acting_as: acting_as(r), url }
}

/// Comment on a pull request or an issue as the signed-in account.
pub fn comment(r: &dyn CommandRunner, dir: &Path, kind: &str, number: u32, body: &str, source: Option<&str>) -> PResult<Acted> {
    text_ok("a comment", body, MAX_BODY)?;
    let slug = pr_slug(r, dir, source)?;
    let n = number.to_string();
    let sub = match kind {
        "pr" => "pr",
        "issue" => "issue",
        _ => return Err(Problem::generic("That is not something Kora can comment on.")),
    };
    let o = pr(run_ok(r, "gh", &[sub, "comment", &n, "--repo", &slug, "--body", body], None))?;
    Ok(acted(r, "Your comment was added.", Some(o.stdout.trim().to_string()).filter(|u| u.starts_with("https://"))))
}

/// Review a pull request: `approve`, `request_changes` (needs a reason) or `comment`.
pub fn review(r: &dyn CommandRunner, dir: &Path, number: u32, action: &str, body: &str, source: Option<&str>) -> PResult<Acted> {
    let slug = pr_slug(r, dir, source)?;
    let n = number.to_string();
    let flag = match action {
        "approve" => "--approve",
        "request_changes" => {
            text_ok("the reason you ask for changes", body, MAX_BODY)?;
            "--request-changes"
        }
        "comment" => {
            text_ok("a comment", body, MAX_BODY)?;
            "--comment"
        }
        _ => return Err(Problem::generic("A review is \"approve\", \"request_changes\" or \"comment\".")),
    };
    let mut args = vec!["pr", "review", n.as_str(), "--repo", slug.as_str(), flag];
    if !body.trim().is_empty() {
        args.extend(["--body", body]);
    }
    let o = pr(run(r, "gh", &args, None))?;
    if !o.ok() {
        let low = o.stderr.to_ascii_lowercase();
        if low.contains("can not approve your own") || low.contains("cannot approve your own") {
            return Err(Problem::new(MERGE_REFUSED, "GitHub does not let an account approve its own pull request.").with_details(cut(&o.stderr, 300)));
        }
        return Err(problem_of(&GitError::Command { cmd: "gh pr review".into(), code: o.code, stderr: o.stderr }));
    }
    let said = match action {
        "approve" => "You approved the pull request.",
        "request_changes" => "You asked for changes.",
        _ => "Your review comment was added.",
    };
    Ok(acted(r, said, None))
}

/// Merge a pull request the way the repository allows. `method` (`merge`, `squash`, `rebase`) must be one of those the
/// repository allows; `None` picks the first it allows. Never `--admin` (no bypassing the repository's rules).
pub fn merge(r: &dyn CommandRunner, dir: &Path, default_branch: &str, number: u32, method: Option<&str>, source: Option<&str>) -> PResult<Acted> {
    let v = pr_view(r, dir, default_branch, number, source)?;
    if v.state != "OPEN" {
        return Err(Problem::new(MERGE_REFUSED, format!("Pull request #{number} is {}, so it cannot be merged.", v.state.to_ascii_lowercase())).with_action(Action::open_link(&v.url)));
    }
    if !v.can_merge {
        return Err(Problem::new(
            MERGE_REFUSED,
            format!("Your GitHub account ({}) is not allowed to merge into {}. Ask someone with write access.", v.acting_as.as_deref().unwrap_or("not known"), v.repo),
        )
        .with_action(Action::open_link(&v.url)));
    }
    let chosen = match method {
        Some(m) if ["merge", "squash", "rebase"].contains(&m) => {
            if !v.merge_methods.iter().any(|x| x == m) {
                return Err(Problem::new(MERGE_REFUSED, format!("{} does not allow merging with \"{m}\". It allows: {}.", v.repo, v.merge_methods.join(", "))));
            }
            m.to_string()
        }
        Some(_) => return Err(Problem::generic("A merge is \"merge\", \"squash\" or \"rebase\".")),
        None => v.merge_methods.first().cloned().ok_or_else(|| Problem::new(MERGE_REFUSED, "This repository does not allow merging from here."))?,
    };
    let n = number.to_string();
    let flag = format!("--{chosen}");
    let o = pr(run(r, "gh", &["pr", "merge", &n, "--repo", &v.repo, &flag], None))?;
    if !o.ok() {
        return Err(Problem::new(MERGE_REFUSED, "GitHub refused the merge. A required check or review may be missing, or there may be a conflict.")
            .with_details(cut(&o.stderr, 400))
            .with_action(Action::open_link(&v.url)));
    }
    Ok(acted(r, &format!("Pull request #{number} was merged ({chosen})."), Some(v.url)))
}

// ---------------------------------------------------------------------------------------------
// Issues and releases
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct IssueItem {
    pub number: u32,
    pub title: String,
    pub author: String,
    pub url: String,
    pub comments: u32,
    pub created: String,
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Issues {
    pub repo: String,
    pub issues: Vec<IssueItem>,
    pub new_issue_url: String,
}

pub fn parse_issues(json: &str) -> Vec<IssueItem> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(json) else { return Vec::new() };
    items
        .iter()
        .filter_map(|v| {
            Some(IssueItem {
                number: u32::try_from(v.get("number")?.as_u64()?).ok()?,
                title: cut(&s(v, "title"), 300),
                author: login(v),
                url: Some(s(v, "url")).filter(|u| u.starts_with("https://")).unwrap_or_default(),
                comments: v.get("comments").and_then(Value::as_array).map_or(0, |a| a.len() as u32),
                created: s(v, "createdAt"),
                labels: v.get("labels").and_then(Value::as_array).map(|a| a.iter().filter_map(|l| l.get("name")?.as_str().map(|n| cut(n, 50))).take(10).collect()).unwrap_or_default(),
            })
        })
        .collect()
}

pub fn issues(r: &dyn CommandRunner, dir: &Path) -> PResult<Issues> {
    let slug = pr_slug(r, dir, None)?;
    let o = pr(run_ok(r, "gh", &["issue", "list", "--repo", &slug, "--state", "open", "--limit", "50", "--json", "number,title,author,url,comments,createdAt,labels"], None))?;
    Ok(Issues { new_issue_url: format!("https://github.com/{slug}/issues/new"), repo: slug, issues: parse_issues(&o.stdout) })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ReleaseItem {
    pub tag: String,
    pub name: String,
    pub published: String,
    pub prerelease: bool,
    pub url: String,
    pub notes: String,
    /// This is the release the server is pinned to.
    pub pinned: bool,
    /// This is newer than the release the server is pinned to.
    pub newer: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Releases {
    pub repo: String,
    /// The pinned release of this resource (a commit), when the server follows released versions.
    pub pinned_commit: Option<String>,
    pub releases: Vec<ReleaseItem>,
}

pub fn parse_releases(json: &str, pinned_tag: Option<&str>) -> Vec<ReleaseItem> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(json) else { return Vec::new() };
    let mut seen_pinned = pinned_tag.is_none();
    let mut out = Vec::new();
    for v in items.iter().take(15) {
        let tag = s(v, "tag_name");
        if tag.is_empty() || tag.len() > 100 {
            continue;
        }
        let pinned = pinned_tag == Some(tag.as_str());
        // GitHub lists the newest first: everything before the pinned one is newer.
        let newer = !seen_pinned && !pinned;
        if pinned {
            seen_pinned = true;
        }
        out.push(ReleaseItem {
            name: cut(&s(v, "name"), 200),
            published: s(v, "published_at"),
            prerelease: v.get("prerelease").and_then(Value::as_bool).unwrap_or(false),
            url: Some(s(v, "html_url")).filter(|u| u.starts_with("https://")).unwrap_or_default(),
            notes: cut(&s(v, "body"), MAX_TEXT),
            tag,
            pinned,
            newer,
        });
    }
    out
}

pub fn releases(r: &dyn CommandRunner, dir: &Path, pinned_commit: Option<&str>) -> PResult<Releases> {
    let slug = pr_slug(r, dir, None)?;
    let path = format!("repos/{slug}/releases?per_page=15");
    let o = pr(run_ok(r, "gh", &["api", &path], None))?;
    // The tag of the pinned commit, from git (tags are downloaded with the clone).
    let pinned_tag = pinned_commit.filter(|c| crate::git::valid_commit(c)).and_then(|c| {
        let o = run(r, "git", &["tag", "--points-at", c], Some(dir)).ok()?;
        o.ok().then(|| o.stdout.lines().next().map(str::to_string)).flatten().filter(|t| !t.is_empty())
    });
    Ok(Releases { releases: parse_releases(&o.stdout, pinned_tag.as_deref()), repo: slug, pinned_commit: pinned_commit.map(str::to_string) })
}

// ---------------------------------------------------------------------------------------------
// Make my own copy (fork)
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Forked {
    pub resource: String,
    /// `owner/name` of the new copy.
    pub fork: String,
    pub message: String,
}

/// Turn a plain clone into your own copy: fork it into `owner` (your account, or an organisation with `is_org`), point
/// `origin` at the fork and keep the original as `upstream`. The folder keeps its commit, branch and changes. Does
/// nothing when it already is a fork.
pub fn make_fork(r: &dyn CommandRunner, dir: &Path, id: &str, owner: &str, is_org: bool) -> PResult<Forked> {
    make_fork_with(r, dir, id, owner, is_org, crate::git::ForkConfig::default().ready_timeout)
}

/// [`make_fork`] with how long to wait for GitHub to finish creating the copy (zero in tests).
pub fn make_fork_with(r: &dyn CommandRunner, dir: &Path, id: &str, owner: &str, is_org: bool, ready_timeout: std::time::Duration) -> PResult<Forked> {
    let now = repos(r, dir)?;
    if let Some(f) = now.fork {
        return Ok(Forked { resource: id.to_string(), fork: f.clone(), message: format!("{id} already has your own copy ({f}).") });
    }
    let Some(original) = now.original else {
        return Err(Problem::new(codes::NO_REMOTE, "This resource has no GitHub address to make a copy of."));
    };
    let mut state = crate::git::ForkState::new(owner, is_org, std::slice::from_ref(&original));
    let cfg = crate::git::ForkConfig { batch_pause: std::time::Duration::ZERO, ready_timeout, ..Default::default() };
    pr(crate::git::run_forks(r, &mut state, &cfg, &mut |_| {}))?;
    if let Some(f) = state.failed.first() {
        return Err(Problem::new(NEEDS_FORK, f.error.clone()).with_action(Action::Retry));
    }
    let fork = state.fork_of(&original);
    let url = format!("https://github.com/{fork}.git");
    // The original becomes `upstream`; `origin` is now the fork. Nothing in the working folder changes.
    pr(run_ok(r, "git", &["remote", "rename", "origin", "upstream"], Some(dir)))?;
    pr(run_ok(r, "git", &["remote", "add", "origin", &url], Some(dir)))?;
    let mut args: Vec<&str> = vec!["fetch", "origin"];
    args.insert(0, "-c");
    args.insert(1, "http.lowSpeedLimit=1000");
    pr(run(r, "git", &args, Some(dir)))?;
    Ok(Forked { resource: id.to_string(), fork: fork.clone(), message: format!("{id} now has your own copy ({fork}).") })
}

/// Where the copies go: the signed-in account (`None` or its own login) or an organisation the account may create
/// repositories in. `(login, is_org)`.
pub fn resolve_fork_owner(r: &dyn CommandRunner, requested: Option<&str>) -> PResult<(String, bool)> {
    use crate::git::owners::{check_org, gh_auth, GhAuth};
    use crate::problem::{codes as pc, known};
    let login = match gh_auth(r) {
        GhAuth::LoggedIn { login } => login,
        GhAuth::Missing => return Err(known::gh_missing()),
        GhAuth::LoggedOut => return Err(known::github_signed_out()),
    };
    match requested.map(str::trim).filter(|o| !o.is_empty()) {
        None => Ok((login, false)),
        Some(o) if o.eq_ignore_ascii_case(&login) => Ok((login, false)),
        Some(o) => {
            crate::git::owners::check_login_syntax(o).map_err(|e| Problem::new(codes::BAD_NAME, e))?;
            let org = check_org(r, o);
            if org.can_fork {
                Ok((org.login, true))
            } else {
                Err(Problem::new(pc::GH_OWNER_INVALID, format!("You cannot make copies in '{o}': {}. Choose another GitHub account.", org.reason.as_deref().unwrap_or("not allowed"))))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ForkFailure {
    pub resource: String,
    pub problem: Problem,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ForkOutcome {
    /// Where the copies are (login or organisation).
    pub owner: String,
    pub forked: Vec<Forked>,
    /// Resources that were not plain clones of a repository (paid assets, copies of someone else's resource).
    pub skipped: Vec<String>,
    pub failed: Vec<ForkFailure>,
    /// Every resource of the server is now your own copy, so the server follows "your own copies".
    pub server_is_fork: bool,
}

/// Make your own copy of every resource that is a plain clone (or of `only` those). Takes the update lock. When nothing is
/// left that is a plain clone, the server is recorded as using your own copies.
pub fn fork_resources(r: &dyn CommandRunner, sf: &ServerFile, owner: Option<&str>, only: Option<&[String]>, progress: &mut dyn FnMut(usize, usize, &str)) -> PResult<ForkOutcome> {
    let (login, is_org) = resolve_fork_owner(r, owner)?;
    let _lock = lock_for(sf)?;
    let wanted: Option<Vec<String>> = only.map(|o| o.iter().map(|n| crate::catalog::normalize_id(n)).collect());
    let mut out = ForkOutcome { owner: login.clone(), forked: vec![], skipped: vec![], failed: vec![], server_is_fork: false };
    let todo: Vec<&crate::installer::registry::ResourceRecord> = sf.resources.iter().filter(|rec| wanted.as_ref().is_none_or(|w| w.contains(&crate::catalog::normalize_id(&rec.id)))).collect();
    let total = todo.len();
    progress(0, total, "");
    for (n, rec) in todo.iter().enumerate() {
        match folder(sf, rec) {
            Err(p) => out.skipped.push(format!("{}: {}", rec.id, p.message)),
            Ok(dir) => match make_fork(r, &dir, &rec.id, &login, is_org) {
                Ok(f) => out.forked.push(f),
                Err(p) => out.failed.push(ForkFailure { resource: rec.id.clone(), problem: p }),
            },
        }
        progress(n + 1, total, &rec.id);
    }
    // Everything that can have a copy has one: the server now uses your own copies.
    let all_done =
        out.failed.is_empty() && sf.resources.iter().all(|rec| folder(sf, rec).ok().is_none_or(|d| !d.join(".git").exists() || repos(r, &d).ok().is_some_and(|x| x.fork.is_some()) || rec.third_party));
    if all_done && sf.source() != Source::Fork {
        let mut next = sf.clone();
        next.source = Some(Source::Fork);
        next.versions = Some(sf.versions());
        next.github_owner = Some(login.clone());
        if next.save().is_ok() {
            sync_entry(&next);
            out.server_is_fork = true;
        }
    } else if sf.source() == Source::Fork {
        out.server_is_fork = true;
    }
    Ok(out)
}

/// Keep the registry entry (what the servers list reads) in step with the server file.
fn sync_entry(sf: &ServerFile) {
    // One transaction: a `kora` command and the panel changing the list at the same moment cannot lose each other's change.
    let _ = Registry::update(|reg| {
        if let Some(e) = reg.servers.iter().find(|e| e.name == sf.name || e.dir == sf.dir).cloned() {
            reg.upsert(ServerEntry { source: Some(sf.source()), versions: Some(sf.versions()), mode: crate::installer::compat_mode(sf.versions()), ..e });
        }
        Ok(())
    });
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct VersionsSet {
    pub source: Source,
    pub versions: Versions,
    /// What to do next, in words.
    pub message: String,
}

/// Change "Which versions" of a server. Nothing on disk moves: the next check for updates shows what would change
/// (to latest: each resource follows its branch; to released: each moves forward to a release that contains what it has,
/// and one that is ahead of its release is shown as needing attention rather than moved back).
pub fn set_versions(sf: &ServerFile, versions: Versions) -> PResult<VersionsSet> {
    let _lock = lock_for(sf)?;
    let mut next = sf.clone();
    next.source = Some(sf.source());
    next.versions = Some(versions);
    next.save().map_err(|e| Problem::generic(format!("Could not save the server's settings: {e}")))?;
    sync_entry(&next);
    let message = match versions {
        Versions::Released => "This server now follows released versions. Check for updates to see which resources move to a release; nothing has changed yet.",
        Versions::Latest => "This server now follows the latest changes. Check for updates to see what is new; nothing has changed yet.",
    };
    Ok(VersionsSet { source: next.source(), versions, message: message.into() })
}

// ---------------------------------------------------------------------------------------------
// On a server
// ---------------------------------------------------------------------------------------------

/// What a front end asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Changes {
        resource: String,
    },
    Log {
        resource: String,
    },
    Commit {
        resource: String,
        message: String,
    },
    Push {
        resource: String,
    },
    NewBranch {
        resource: String,
        name: String,
    },
    PrView {
        resource: String,
        number: u32,
        source: Option<String>,
    },
    Comment {
        resource: String,
        kind: String,
        number: u32,
        body: String,
        source: Option<String>,
    },
    Review {
        resource: String,
        number: u32,
        action: String,
        body: String,
        source: Option<String>,
    },
    Merge {
        resource: String,
        number: u32,
        method: Option<String>,
        source: Option<String>,
    },
    Issues {
        resource: String,
    },
    Releases {
        resource: String,
    },
    /// Make your own copy of these resources (`None` = every resource that is a plain clone). `owner`: a login or
    /// organisation; `None` = the signed-in account.
    Fork {
        owner: Option<String>,
        only: Option<Vec<String>>,
    },
    /// Follow released versions, or the latest changes.
    SetVersions {
        versions: Versions,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Changes(Box<LocalChanges>),
    Log(Log),
    Committed(Committed),
    Pushed(Pushed),
    Where(gb::Where),
    Pr(Box<PrView>),
    Acted(Acted),
    Issues(Issues),
    Releases(Releases),
    Forked(Box<ForkOutcome>),
    Versions(VersionsSet),
}

/// Do one [`Request`] on a server's resource. Writes take the server's update lock (never in the middle of an update).
pub fn run_on(r: &dyn CommandRunner, sf: &ServerFile, req: &Request, progress: &mut dyn FnMut(usize, usize, &str)) -> PResult<Reply> {
    let rec = |name: &str| find(sf, name);
    let dir_of = |name: &str| -> PResult<(PathBuf, String)> {
        let rec = rec(name)?;
        Ok((folder(sf, rec)?, rec.branch.clone()))
    };
    Ok(match req {
        Request::Changes { resource } => {
            let (d, b) = dir_of(resource)?;
            Reply::Changes(Box::new(local_changes(r, &d, &b)?))
        }
        Request::Log { resource } => {
            let (d, b) = dir_of(resource)?;
            Reply::Log(log(r, &d, &b)?)
        }
        Request::Commit { resource, message } => {
            let (d, b) = dir_of(resource)?;
            let _l = lock_for(sf)?;
            Reply::Committed(commit(r, &d, &b, message)?)
        }
        Request::Push { resource } => {
            let (d, b) = dir_of(resource)?;
            let _l = lock_for(sf)?;
            Reply::Pushed(push(r, &d, &b)?)
        }
        Request::NewBranch { resource, name } => {
            let (d, b) = dir_of(resource)?;
            let _l = lock_for(sf)?;
            let w = new_branch(r, &d, &b, name)?;
            crate::resourcestate::mark_changed(&sf.dir, &[rec(resource)?.id.clone()], &format!("moved to branch {name}"));
            Reply::Where(w)
        }
        Request::PrView { resource, number, source } => {
            let (d, b) = dir_of(resource)?;
            Reply::Pr(Box::new(pr_view(r, &d, &b, *number, source.as_deref())?))
        }
        Request::Comment { resource, kind, number, body, source } => {
            let (d, _) = dir_of(resource)?;
            Reply::Acted(comment(r, &d, kind, *number, body, source.as_deref())?)
        }
        Request::Review { resource, number, action, body, source } => {
            let (d, _) = dir_of(resource)?;
            Reply::Acted(review(r, &d, *number, action, body, source.as_deref())?)
        }
        Request::Merge { resource, number, method, source } => {
            let (d, b) = dir_of(resource)?;
            Reply::Acted(merge(r, &d, &b, *number, method.as_deref(), source.as_deref())?)
        }
        Request::Issues { resource } => {
            let (d, _) = dir_of(resource)?;
            Reply::Issues(issues(r, &d)?)
        }
        Request::Releases { resource } => {
            let rec = rec(resource)?;
            let d = folder(sf, rec)?;
            Reply::Releases(releases(r, &d, rec.commit.as_deref())?)
        }
        Request::Fork { owner, only } => Reply::Forked(Box::new(fork_resources(r, sf, owner.as_deref(), only.as_deref(), progress)?)),
        Request::SetVersions { versions } => Reply::Versions(set_versions(sf, *versions)?),
    })
}

/// [`run_on`] on a registered server (name or folder) with the right git for who owns its folder. For the command line.
pub fn run_for(server: &str, req: &Request, progress: &mut dyn FnMut(usize, usize, &str)) -> PResult<Reply> {
    let sf = crate::update::manual::load_server(server).map_err(|e| Problem::generic(e.to_string()))?;
    crate::installer::gitops::with_runner(&sf.dir, |r| run_on(r, &sf, req, progress))
}

impl Reply {
    pub fn json(&self) -> Value {
        let v = |x: &dyn erased::Ser| x.to_value();
        match self {
            Reply::Changes(x) => v(&**x),
            Reply::Log(x) => v(x),
            Reply::Committed(x) => v(x),
            Reply::Pushed(x) => v(x),
            Reply::Where(x) => v(x),
            Reply::Pr(x) => v(&**x),
            Reply::Acted(x) => v(x),
            Reply::Issues(x) => v(x),
            Reply::Releases(x) => v(x),
            Reply::Forked(x) => v(&**x),
            Reply::Versions(x) => v(x),
        }
    }
}

mod erased {
    pub trait Ser {
        fn to_value(&self) -> serde_json::Value;
    }
    impl<T: serde::Serialize> Ser for T {
        fn to_value(&self) -> serde_json::Value {
            serde_json::to_value(self).unwrap_or_default()
        }
    }
}

#[cfg(test)]
mod tests;
