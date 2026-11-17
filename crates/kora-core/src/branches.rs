//! Branches and pull requests for the resources of a Development server.
//!
//! Each resource of a Development server is a git clone of the person's fork. This module lets them see which branch each
//! one is on, change it, try a pull request, go back to the normal version, and do the same for every resource at once.
//! The git work for one folder is in [`crate::git::branches`]; this layer adds the server: which resources, only inside the
//! server's `resources` folder, the update lock, and the bulk operations.
//!
//! How this fits with updates (the decision, also in docs/panel.md):
//! - The branch a resource is on is whatever git says. Nothing about it is stored, so there is nothing to get out of step.
//! - A resource that is not on its normal branch is **left alone by updates**, manual and scheduled: the preview and the
//!   report say "on branch X" or "trying pull request #12; not updated" (informational, not a problem). Updates are never
//!   merged into someone's branch and a branch is never reported "up to date" because it happens to be ahead.
//! - Trying a pull request again fetches its new commits (fast-forward only; a rewritten PR gets a new branch).
//! - Every update snapshot records the branch each resource was on, and a rollback returns to that branch before it resets.
//! - Production servers do not get any of this: they stay pinned to their releases.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::catalog::normalize_id;
use crate::git::branches::{self as gb, codes, Branches, Changes, Kind, Outcome, PrSource, PullRequest, Stash, Status};
use crate::git::CommandRunner;
use crate::installer::registry::{ResourceRecord, ServerFile};
use crate::installer::types::Mode;
use crate::problem::{Action, Problem};
use crate::update::{lock, par_map, FETCH_PARALLEL};

pub use crate::git::branches::{parse_pr_input, valid_branch};

type PResult<T> = Result<T, Problem>;

/// Said after any change of files while the server is running.
pub const RESTART_NOTE: &str = "The files changed, but a running server still has the old version loaded. Restart the resource (or the server) to load the new one.";

// ---------------------------------------------------------------------------------------------
// Which resources, and where
// ---------------------------------------------------------------------------------------------

fn ensure_dev(sf: &ServerFile) -> PResult<()> {
    if sf.mode == Mode::Development {
        Ok(())
    } else {
        Err(Problem::new(codes::NOT_DEV, "Changing branches is only for Development servers. A Production server stays on its tested releases, so it cannot be moved onto another branch."))
    }
}

/// The folder of a resource, only ever inside `<server>/resources`.
fn folder_of(sf: &ServerFile, rec: &ResourceRecord) -> PResult<PathBuf> {
    let rel = Path::new(&rec.path);
    let clean = !rec.path.is_empty() && !rel.is_absolute() && rel.components().all(|c| matches!(c, Component::Normal(_)));
    let root = sf.dir.join("resources");
    let full = root.join(rel);
    if !clean {
        return Err(Problem::generic(format!("The folder of {} is not inside the server's resources folder, so Kora will not touch it.", rec.id)));
    }
    // A link that points out of the resources folder is not followed.
    if let (Ok(a), Ok(b)) = (std::fs::canonicalize(&full), std::fs::canonicalize(&root)) {
        if !a.starts_with(&b) {
            return Err(Problem::generic(format!("The folder of {} points outside the server's resources folder, so Kora will not touch it.", rec.id)));
        }
    }
    Ok(full)
}

/// Resources that are git clones of a fork (paid assets, bridge copies and single-folder copies are not).
fn tracked(sf: &ServerFile, rec: &ResourceRecord) -> Result<PathBuf, &'static str> {
    if rec.is_portal() {
        return Err("A paid asset imported from a zip: it is not a git clone.");
    }
    if rec.third_party {
        return Err("A copy of someone else's resource, kept on a fixed version: it has no fork to switch.");
    }
    let path = sf.dir.join("resources").join(&rec.path);
    if crate::installer::pinned::is_subpath_copy(&path) {
        return Err("A copy of one folder of a repository: there is no branch to switch.");
    }
    Ok(path)
}

fn find<'a>(sf: &'a ServerFile, name: &str) -> PResult<&'a ResourceRecord> {
    let n = normalize_id(name);
    sf.resources
        .iter()
        .find(|r| normalize_id(&r.id) == n || r.path == name)
        .ok_or_else(|| Problem::generic(format!("This server has no resource called '{name}'.")))
}

fn lock_for(sf: &ServerFile) -> PResult<lock::UpdateLock> {
    lock::acquire(&sf.dir).map_err(|e| match e {
        lock::LockError::Busy => Problem::new(codes::BUSY, "An update is running for this server right now. Try again when it has finished.").with_action(Action::Retry),
        other => Problem::generic(other.to_string()),
    })
}

// ---------------------------------------------------------------------------------------------
// What each resource is on
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// On its normal branch.
    Default,
    Branch,
    Pr,
    Detached,
    /// Not a fork clone: no branches here (see `note`).
    Pinned,
    /// Could not be read (see `note`).
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceBranch {
    pub id: String,
    /// The folder, relative to `resources/`.
    pub folder: String,
    pub state: State,
    /// Local branch name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<u32>,
    /// The normal branch (`dev`).
    pub default_branch: String,
    /// Plain words for where it is: "the normal version (dev)", "pull request #12".
    pub label: String,
    pub changed_count: usize,
    /// Sets of changes Kora kept aside in this folder.
    pub stashes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// For `unavailable`: what to do about it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<Problem>,
}

fn base_row(rec: &ResourceRecord) -> ResourceBranch {
    ResourceBranch {
        id: rec.id.clone(),
        folder: rec.path.clone(),
        state: State::Unavailable,
        branch: None,
        pr: None,
        default_branch: rec.branch.clone(),
        label: String::new(),
        changed_count: 0,
        stashes: 0,
        note: None,
        problem: None,
    }
}

fn row_of(r: &dyn CommandRunner, sf: &ServerFile, rec: &ResourceRecord) -> ResourceBranch {
    let mut row = base_row(rec);
    if let Err(why) = tracked(sf, rec) {
        row.state = State::Pinned;
        row.label = "fixed version".into();
        row.note = Some(why.into());
        return row;
    }
    let dir = match folder(sf, rec) {
        Ok(d) => d,
        Err(p) => {
            row.label = "not available".into();
            row.note = Some(p.message.clone());
            row.problem = Some(p);
            return row;
        }
    };
    match gb::locate(r, &dir, &rec.branch) {
        Ok(w) => {
            row.state = match w.kind {
                Kind::Default => State::Default,
                Kind::Branch => State::Branch,
                Kind::Pr => State::Pr,
                Kind::Detached => State::Detached,
            };
            row.label = w.label(&rec.branch);
            row.changed_count = w.changed_count;
            row.stashes = gb::stashes(r, &dir).map(|s| s.len()).unwrap_or(0);
            row.branch = w.branch;
            row.pr = w.pr;
        }
        Err(p) => {
            row.label = "not available".into();
            row.note = Some(p.message.clone());
            row.problem = Some(p);
        }
    }
    row
}

/// The branch of every resource (what is on disk; no network). Development servers only.
pub fn overview(r: &dyn CommandRunner, sf: &ServerFile) -> PResult<Vec<ResourceBranch>> {
    ensure_dev(sf)?;
    // One runner per worker when the runner can be copied; otherwise one by one.
    let mut jobs: Vec<(usize, Option<Box<dyn CommandRunner + Send>>)> = Vec::new();
    for i in 0..sf.resources.len() {
        jobs.push((i, r.fork()));
    }
    if jobs.iter().all(|(_, f)| f.is_some()) && jobs.len() > 1 {
        let rows = par_map(jobs, FETCH_PARALLEL, |(i, f)| row_of(f.as_deref().expect("checked"), sf, &sf.resources[i]));
        return Ok(rows);
    }
    Ok(sf.resources.iter().map(|rec| row_of(r, sf, rec)).collect())
}

// ---------------------------------------------------------------------------------------------
// One resource
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choices {
    pub resource: ResourceBranch,
    pub branches: Branches,
    /// Changes Kora kept aside here, newest first.
    pub stashes: Vec<Stash>,
    /// Plain notes (e.g. "could not refresh: showing what Kora already has").
    pub notes: Vec<String>,
}

fn folder(sf: &ServerFile, rec: &ResourceRecord) -> PResult<PathBuf> {
    tracked(sf, rec).map_err(|why| Problem::new(codes::UNSUPPORTED, format!("{}: {why}", rec.id)))?;
    let p = folder_of(sf, rec)?;
    if !p.is_dir() {
        return Err(Problem::new(codes::NOT_REPO, format!("The folder of {} is missing.", rec.id)));
    }
    Ok(p)
}

/// Branches, kept-aside changes and the current state of one resource. `refresh` downloads the branch lists first (network).
pub fn choices(r: &dyn CommandRunner, sf: &ServerFile, resource: &str, refresh: bool) -> PResult<Choices> {
    ensure_dev(sf)?;
    let rec = find(sf, resource)?;
    let dir = folder(sf, rec)?;
    let mut notes = Vec::new();
    if refresh {
        if let Err(p) = gb::fetch_remotes(r, &dir) {
            notes.push(format!("Could not refresh the list from GitHub ({}). Showing what Kora already has.", p.message));
        }
    }
    gb::locate(r, &dir, &rec.branch)?;
    Ok(Choices { resource: row_of(r, sf, rec), branches: gb::branches(r, &dir, &rec.branch)?, stashes: gb::stashes(r, &dir)?, notes })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrChoices {
    pub prs: Vec<PullRequest>,
    pub notes: Vec<String>,
}

/// Open pull requests of the original repository and of the person's fork (network, through `gh`).
pub fn prs(r: &dyn CommandRunner, sf: &ServerFile, resource: &str) -> PResult<PrChoices> {
    ensure_dev(sf)?;
    let rec = find(sf, resource)?;
    let dir = folder(sf, rec)?;
    let (prs, notes) = gb::list_prs(r, &dir)?;
    Ok(PrChoices { prs, notes })
}

/// Where to go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A branch; `remote` = `origin` (the fork) or `upstream` (the original) when the folder does not have it yet.
    Branch { name: String, remote: Option<String> },
    /// A pull request: a number or an address, and where the picker found it.
    Pr { input: String, source: Option<PrSource> },
    /// The resource's normal branch.
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwitchResult {
    pub resource: String,
    #[serde(flatten)]
    pub outcome: Outcome,
    /// True when files changed: the running server still has the old version until it is restarted.
    pub needs_restart: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restart_note: Option<String>,
}

fn go(r: &dyn CommandRunner, dir: &Path, default_branch: &str, target: &Target, changes: Changes) -> PResult<Outcome> {
    match target {
        Target::Branch { name, remote } => gb::switch_branch(r, dir, default_branch, name, remote.as_deref(), changes),
        Target::Default => gb::back_to_default(r, dir, default_branch, changes),
        Target::Pr { input, source } => gb::checkout_pr(r, dir, default_branch, &parse_pr_input(input)?, *source, changes),
    }
}

/// Change one resource. Takes the server's update lock, so it never runs in the middle of an update.
pub fn switch(r: &dyn CommandRunner, sf: &ServerFile, resource: &str, target: &Target, changes: Changes) -> PResult<SwitchResult> {
    ensure_dev(sf)?;
    let rec = find(sf, resource)?;
    let dir = folder(sf, rec)?;
    let _lock = lock_for(sf)?;
    let outcome = go(r, &dir, &rec.branch, target, changes)?;
    let moved = outcome.status == Status::Switched;
    Ok(SwitchResult { resource: rec.id.clone(), needs_restart: moved, restart_note: moved.then(|| RESTART_NOTE.to_string()), outcome })
}

/// Bring a set of kept-aside changes back (needs a clean folder; the set stays in the list).
pub fn restore_stash(r: &dyn CommandRunner, sf: &ServerFile, resource: &str, index: u32) -> PResult<SwitchResult> {
    ensure_dev(sf)?;
    let rec = find(sf, resource)?;
    let dir = folder(sf, rec)?;
    let _lock = lock_for(sf)?;
    let outcome = gb::restore_stash(r, &dir, &rec.branch, index)?;
    Ok(SwitchResult { resource: rec.id.clone(), needs_restart: true, restart_note: Some(RESTART_NOTE.to_string()), outcome })
}

// ---------------------------------------------------------------------------------------------
// Every resource at once
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkRow {
    pub id: String,
    /// The resource has this branch (here, on the fork or on the original).
    pub has: bool,
    /// Where Kora would take it from when the folder does not have it yet (`origin` / `upstream`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// Already on it.
    pub already: bool,
    /// Where the resource is now, in words.
    pub now: String,
    pub changed_count: usize,
    /// Why it cannot be looked at, if so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkPreview {
    pub branch: String,
    pub rows: Vec<BulkRow>,
    pub have: usize,
    pub missing: usize,
    pub notes: Vec<String>,
}

/// Which resources have `branch`. With `fetch` the lists are refreshed from GitHub first (slow; reports progress).
pub fn bulk_preview(r: &dyn CommandRunner, sf: &ServerFile, branch: &str, fetch: bool, progress: &mut dyn FnMut(usize, usize, &str)) -> PResult<BulkPreview> {
    ensure_dev(sf)?;
    if !valid_branch(branch) {
        return Err(Problem::new(codes::BAD_NAME, format!("'{}' is not a branch name Kora can use.", branch.chars().take(60).collect::<String>())));
    }
    let recs: Vec<&ResourceRecord> = sf.resources.iter().filter(|rec| tracked(sf, rec).is_ok()).collect();
    let total = recs.len();
    let mut notes = Vec::new();
    let mut rows = Vec::new();
    progress(0, total, "");
    for (n, rec) in recs.iter().enumerate() {
        let mut row = BulkRow { id: rec.id.clone(), has: false, from: None, already: false, now: String::new(), changed_count: 0, note: None };
        match folder(sf, rec) {
            Err(p) => row.note = Some(p.message),
            Ok(dir) => {
                if fetch {
                    if let Err(p) = gb::fetch_remotes(r, &dir) {
                        row.note = Some(format!("could not refresh from GitHub: {}", p.message));
                    }
                }
                match (gb::locate(r, &dir, &rec.branch), gb::branches(r, &dir, &rec.branch)) {
                    (Ok(w), Ok(b)) => {
                        row.now = w.label(&rec.branch);
                        row.changed_count = w.changed_count;
                        row.already = w.branch.as_deref() == Some(branch);
                        if b.local.iter().any(|x| x == branch) {
                            row.has = true;
                        } else if b.fork.iter().any(|x| x == branch) {
                            row.has = true;
                            row.from = Some("origin".into());
                        } else if b.upstream.iter().any(|x| x == branch) {
                            row.has = true;
                            row.from = Some("upstream".into());
                        }
                    }
                    (Err(p), _) | (_, Err(p)) => row.note = Some(p.message),
                }
            }
        }
        progress(n + 1, total, &rec.id);
        rows.push(row);
    }
    let have = rows.iter().filter(|x| x.has).count();
    let missing = rows.iter().filter(|x| !x.has && x.note.is_none()).count();
    if rows.is_empty() {
        notes.push("This server has no resources that can change branch.".into());
    }
    Ok(BulkPreview { branch: branch.to_string(), rows, have, missing, notes })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BulkStatus {
    Switched,
    Already,
    /// Has uncommitted changes and no choice was given: untouched.
    NeedsChoice,
    /// Does not have the branch: left as it is.
    Skipped,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkItem {
    pub id: String,
    pub status: BulkStatus,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stashed: Option<Stash>,
    /// Files with changes (for `needs_choice`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    /// For `failed`: what to do about it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<Problem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkResult {
    pub items: Vec<BulkItem>,
    pub switched: usize,
    pub needs_choice: usize,
    pub failed: usize,
    pub skipped: usize,
    pub needs_restart: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restart_note: Option<String>,
}

/// Switch every resource that has `target` (a branch) to it, or every resource back to its normal branch (`Target::Default`).
/// `only` limits it to some resources. One resource failing never stops the others. Holds the update lock for the whole run.
pub fn bulk_apply(
    r: &dyn CommandRunner,
    sf: &ServerFile,
    target: &Target,
    only: Option<&[String]>,
    changes: Changes,
    progress: &mut dyn FnMut(usize, usize, &str),
) -> PResult<BulkResult> {
    ensure_dev(sf)?;
    let branch = match target {
        Target::Branch { name, .. } => {
            if !valid_branch(name) {
                return Err(Problem::new(codes::BAD_NAME, format!("'{}' is not a branch name Kora can use.", name.chars().take(60).collect::<String>())));
            }
            Some(name.clone())
        }
        Target::Default => None,
        Target::Pr { .. } => return Err(Problem::new(codes::UNSUPPORTED, "Pull requests are tried one resource at a time.")),
    };
    let wanted: Option<Vec<String>> = only.map(|o| o.iter().map(|n| normalize_id(n)).collect());
    let recs: Vec<&ResourceRecord> = sf.resources.iter().filter(|rec| tracked(sf, rec).is_ok() && wanted.as_ref().is_none_or(|w| w.contains(&normalize_id(&rec.id)))).collect();
    let _lock = lock_for(sf)?;
    let total = recs.len();
    progress(0, total, "");
    let mut items = Vec::new();
    for (n, rec) in recs.iter().enumerate() {
        let item = bulk_one(r, sf, rec, target, branch.as_deref(), changes);
        progress(n + 1, total, &rec.id);
        items.push(item);
    }
    let count = |s: BulkStatus| items.iter().filter(|i| i.status == s).count();
    let switched = count(BulkStatus::Switched);
    Ok(BulkResult {
        switched,
        needs_choice: count(BulkStatus::NeedsChoice),
        failed: count(BulkStatus::Failed),
        skipped: count(BulkStatus::Skipped),
        needs_restart: switched > 0,
        restart_note: (switched > 0).then(|| RESTART_NOTE.to_string()),
        items,
    })
}

fn bulk_one(r: &dyn CommandRunner, sf: &ServerFile, rec: &ResourceRecord, target: &Target, branch: Option<&str>, changes: Changes) -> BulkItem {
    let item = |status, message: String| BulkItem { id: rec.id.clone(), status, message, stashed: None, files: Vec::new(), problem: None };
    let dir = match folder(sf, rec) {
        Ok(d) => d,
        Err(p) => return BulkItem { problem: Some(p.clone()), ..item(BulkStatus::Failed, p.message) },
    };
    // A resource without the branch is left exactly as it is.
    if let Some(b) = branch {
        match gb::branches(r, &dir, &rec.branch) {
            Ok(list) if !(list.local.iter().chain(&list.fork).chain(&list.upstream).any(|x| x == b)) => {
                return item(BulkStatus::Skipped, format!("It has no branch called {b}; left as it is."));
            }
            Err(p) => return BulkItem { problem: Some(p.clone()), ..item(BulkStatus::Failed, p.message) },
            _ => {}
        }
    }
    match go(r, &dir, &rec.branch, target, changes) {
        Ok(o) => {
            let status = match o.status {
                Status::Switched => BulkStatus::Switched,
                Status::Already => BulkStatus::Already,
                Status::NeedsChoice => BulkStatus::NeedsChoice,
            };
            BulkItem { stashed: o.stashed, files: o.now.changed_files, ..item(status, o.message) }
        }
        Err(p) => BulkItem { problem: Some(p.clone()), ..item(BulkStatus::Failed, p.message) },
    }
}

#[cfg(test)]
mod tests;
