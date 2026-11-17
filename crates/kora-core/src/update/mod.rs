//! Update engine shared by manual updates and `kora update --auto`.
//!
//! Flow: [`plan`] (classify each resource) -> [`run_update`] (snapshot, apply,
//! health check, auto-rollback) -> [`UpdateReport`] (text summary + Discord payload).
//! Git and the server health check are behind traits so everything is testable offline.
//! SQL migrations are flagged and never rolled back automatically.

pub mod lock;

pub mod manual;
pub mod orgcache;
pub mod status;

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("git error: {0}")]
    Git(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("snapshot error: {0}")]
    Snapshot(#[from] serde_json::Error),
}

pub type GitResult<T> = Result<T, UpdateError>;

/// Most fetches that run at the same time.
pub const FETCH_PARALLEL: usize = 6;

/// How fresh a fetch must be. `Fresh` always asks the remote; `AllowRecent` may reuse a fetch this process made
/// moments ago (an apply right after the preview the person just looked at).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchMode {
    Fresh,
    AllowRecent,
}

/// Run `f` over `items` on at most `limit` threads; results keep the order of `items`.
pub fn par_map<I: Send, O: Send>(items: Vec<I>, limit: usize, f: impl Fn(I) -> O + Sync) -> Vec<O> {
    use std::sync::Mutex;
    let n = items.len();
    let queue: Mutex<std::vec::IntoIter<(usize, I)>> = Mutex::new(items.into_iter().enumerate().collect::<Vec<_>>().into_iter());
    let out: Mutex<Vec<Option<O>>> = Mutex::new((0..n).map(|_| None).collect());
    std::thread::scope(|sc| {
        for _ in 0..limit.max(1).min(n) {
            sc.spawn(|| loop {
                let job = queue.lock().unwrap_or_else(|e| e.into_inner()).next();
                let Some((i, item)) = job else { break };
                let r = f(item);
                out.lock().unwrap_or_else(|e| e.into_inner())[i] = Some(r);
            });
        }
    });
    out.into_inner().unwrap_or_else(|e| e.into_inner()).into_iter().flatten().collect()
}

/// Git operations the engine needs. Implemented by the git module (shells out to `git`).
/// `upstream` is a ref such as `upstream/dev`; `rev` is `"HEAD"` or a ref/commit.
pub trait GitOps {
    fn fetch(&self, repo: &Path) -> GitResult<()>;
    fn head_commit(&self, repo: &Path, rev: &str) -> GitResult<String>;
    /// Fast-forward only (`merge --ff-only`); must fail rather than merge.
    fn fast_forward(&self, repo: &Path, upstream: &str) -> GitResult<()>;
    /// Hard reset of the working tree to `commit`.
    fn reset_to(&self, repo: &Path, commit: &str) -> GitResult<()>;
    /// Uncommitted changes (tracked or untracked non-ignored).
    fn is_dirty(&self, repo: &Path) -> GitResult<bool>;
    /// True when HEAD has commits that are not in `upstream`.
    fn is_diverged(&self, repo: &Path, upstream: &str) -> GitResult<bool>;
    /// Commits in `upstream` that are not in HEAD.
    fn behind_count(&self, repo: &Path, upstream: &str) -> GitResult<u32>;
    /// Is `ancestor` contained in `descendant` (equal counts)? Default: unknown (an error).
    fn is_ancestor(&self, _repo: &Path, _ancestor: &str, _descendant: &str) -> GitResult<bool> {
        Err(UpdateError::Git("cannot compare commits here".into()))
    }
    /// The branch checked out now: `Some("HEAD")` when detached, `None` when unknown (default: unknown, so nothing is assumed).
    fn current_branch(&self, _repo: &Path) -> GitResult<Option<String>> {
        Ok(None)
    }
    /// True for the view a server on released versions uses: resources sit on a commit, so a named branch was put there on purpose.
    fn pinned_releases(&self) -> bool {
        false
    }
    /// Check out `branch` (`"HEAD"` = detach). Used to put a resource back on the branch a snapshot was taken on.
    fn switch_branch(&self, _repo: &Path, _branch: &str) -> GitResult<()> {
        Err(UpdateError::Git("switching branches is not supported here".into()))
    }
    /// Fetch what each target needs, possibly in parallel; one result per target, in order. Default: one by one.
    fn fetch_many(&self, targets: &[&UpdateTarget], _mode: FetchMode) -> Vec<GitResult<()>> {
        targets.iter().map(|t| self.fetch(&t.path)).collect()
    }
    /// Is `rev` a ref or commit this checkout already has? (local only; default: unknown = false)
    fn has_ref(&self, _repo: &Path, _rev: &str) -> bool {
        false
    }
    /// `owner/name` of a remote's GitHub URL (default: unknown).
    fn remote_slug(&self, _repo: &Path, _remote: &str) -> Option<String> {
        None
    }
    /// Production: fetch `origin` (where pinned release commits live).
    fn fetch_origin(&self, _repo: &Path) -> GitResult<()> {
        Ok(())
    }
    /// Production: move the checkout to a pinned release commit (detached).
    fn move_pinned(&self, _repo: &Path, _commit: &str) -> GitResult<()> {
        Err(UpdateError::Git("pinned moves are not supported here".into()))
    }
    /// Commit subjects in `from..to`, newest first, at most `max` (best effort; empty when unknown).
    fn log_subjects(&self, _repo: &Path, _from: &str, _to: &str, _max: usize) -> Vec<String> {
        Vec::new()
    }
    /// Development: bring the user's forks on GitHub up to date with upstream, fast-forward only
    /// (never forced), when the user is signed in to gh. Returns one outcome per fork that was
    /// looked at; empty when not possible. `progress(done, total)` is called as forks are looked at.
    /// Default: nothing.
    fn sync_forks(&self, _targets: &[UpdateTarget], _progress: &mut dyn FnMut(usize, usize)) -> Vec<(String, crate::git::SyncOutcome)> {
        Vec::new()
    }
}

/// Fold fork-sync outcomes into the report: synced ones are counted, untouched ones listed with
/// a plain explanation. Informational: never marks the update as needing attention.
pub fn add_fork_sync(report: &mut UpdateReport, results: Vec<(String, crate::git::SyncOutcome)>) {
    use crate::git::SyncOutcome as O;
    for (id, o) in results {
        match o {
            O::Synced { .. } => report.forks_synced.push(id),
            O::UpToDate => {}
            O::Ahead { ahead } => report.forks_left_alone.push((id, format!("your fork has {ahead} commit(s) of its own and upstream has nothing new; left as it is"))),
            O::Diverged { .. } => report.forks_left_alone.push((
                id,
                "your fork and upstream both have new commits; the fork was not touched. Fix: in the resource folder run `git fetch upstream && git merge upstream/<branch>`, then `git push`".into(),
            )),
            O::Unknown(why) => report.forks_left_alone.push((id, format!("could not check the fork ({why}); left untouched"))),
        }
    }
}

/// Does the update `from..to` contain SQL migrations?
pub trait MigrationDetector {
    fn has_migrations(&self, repo: &Path, from: &str, to: &str) -> bool;
}

/// Detector that never reports migrations.
pub struct NoMigrations;
impl MigrationDetector for NoMigrations {
    fn has_migrations(&self, _: &Path, _: &str, _: &str) -> bool {
        false
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthRequest {
    /// True when a full server restart is needed (otherwise `ensure` the updated resources).
    pub full_restart: bool,
    pub updated: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthOutcome {
    Healthy,
    Unhealthy { reason: String, failed: Vec<String> },
}

/// Hook into the running server (txAdmin / service layer).
pub trait HealthCheck {
    /// Restart or ensure per `req`, then verify boot succeeded and updated resources started.
    fn check(&self, req: &HealthRequest) -> HealthOutcome;
    /// Restart/ensure again after a rollback so the server runs the old code.
    fn recover(&self, req: &HealthRequest);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResourceClass {
    Core,
    Lib,
    Sql,
    Other,
}

impl ResourceClass {
    /// Classify by catalog id (`kora-core`, `kora-lib`, anything with `sql`).
    pub fn from_id(id: &str) -> Self {
        let n = id.replace('_', "-");
        if n == "kora-core" {
            Self::Core
        } else if n == "kora-lib" {
            Self::Lib
        } else if n.contains("sql") {
            Self::Sql
        } else {
            Self::Other
        }
    }
    pub fn needs_restart(self) -> bool {
        !matches!(self, Self::Other)
    }
}

#[derive(Debug, Clone)]
pub struct UpdateTarget {
    pub id: String,
    pub path: PathBuf,
    pub upstream_ref: String,
    pub class: ResourceClass,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkipReason {
    /// Local commits and nothing new upstream (informational).
    LocalCommits,
    Dirty,
    Diverged,
    Error(String),
    /// The person put this resource on another branch or a pull request on purpose (Development): updates leave it
    /// alone. Carries the branch it is on (`"HEAD"` = a single commit). Informational, never "needs attention".
    OtherBranch(String),
}

impl SkipReason {
    fn needs_attention(&self) -> bool {
        !matches!(self, SkipReason::LocalCommits | SkipReason::OtherBranch(_))
    }
    fn text(&self) -> String {
        match self {
            SkipReason::LocalCommits => "has local commits".into(),
            SkipReason::Dirty => "uncommitted changes".into(),
            SkipReason::Diverged => "diverged from upstream (it has commits of its own and upstream has new ones); skipped, nothing was changed".into(),
            SkipReason::Error(e) => format!("error: {e}"),
            SkipReason::OtherBranch(b) => format!("{}; not updated (go back to the normal version in the Branches tab to update it again)", crate::git::branches::describe_other(b)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    UpToDate,
    /// Fast-forward, then `ensure` live.
    FastForward,
    /// Fast-forward, but only with a full restart (core/lib/sql).
    NeedsRestart,
    Skipped(SkipReason),
}

#[derive(Debug, Clone)]
pub struct PlanItem {
    pub target: UpdateTarget,
    pub current: String,
    pub upstream: String,
    pub behind: u32,
    pub action: Action,
}

#[derive(Debug, Clone)]
pub struct UpdatePlan {
    pub items: Vec<PlanItem>,
}

impl UpdatePlan {
    pub fn pending(&self) -> impl Iterator<Item = &PlanItem> {
        self.items.iter().filter(|i| matches!(i.action, Action::FastForward | Action::NeedsRestart))
    }
}

/// Development: the branch a resource is on when it is not the one updates follow (`upstream/<branch>`); `None` = fine.
///
/// Released versions: a resource is checked out on a commit of its own (no branch), so a named branch or a pull request is
/// always something a person put there on purpose.
pub(crate) fn off_branch(git: &dyn GitOps, path: &Path, followed: &str) -> Option<String> {
    match followed.strip_prefix("upstream/") {
        Some(want) => match git.current_branch(path) {
            Ok(Some(cur)) if cur != want => Some(cur),
            _ => None,
        },
        None if git.pinned_releases() => git.current_branch(path).ok().flatten().filter(|cur| cur != "HEAD"),
        None => None,
    }
}

fn classify_fetched(git: &dyn GitOps, t: &UpdateTarget) -> GitResult<PlanItem> {
    let current = git.head_commit(&t.path, "HEAD")?;
    // Deliberately on another branch or a pull request: never pulled, never "up to date", never clobbered.
    if let Some(cur) = off_branch(git, &t.path, &t.upstream_ref) {
        return Ok(PlanItem { target: t.clone(), upstream: current.clone(), current, behind: 0, action: Action::Skipped(SkipReason::OtherBranch(cur)) });
    }
    let upstream = git.head_commit(&t.path, &t.upstream_ref)?;
    let behind = git.behind_count(&t.path, &t.upstream_ref)?;
    let ahead = git.is_diverged(&t.path, &t.upstream_ref)?;
    let action = if git.is_dirty(&t.path)? {
        Action::Skipped(SkipReason::Dirty)
    } else if ahead && behind > 0 {
        Action::Skipped(SkipReason::Diverged)
    } else if ahead {
        Action::Skipped(SkipReason::LocalCommits)
    } else if behind == 0 {
        Action::UpToDate
    } else if t.class.needs_restart() {
        Action::NeedsRestart
    } else {
        Action::FastForward
    };
    Ok(PlanItem { target: t.clone(), current, upstream, behind, action })
}

/// Fetch and classify every target. Per-resource git errors become `Skipped(Error)`.
pub fn plan(git: &dyn GitOps, targets: &[UpdateTarget]) -> UpdatePlan {
    plan_fetch(git, targets, FetchMode::Fresh, &[]).0
}

/// [`plan`] with every fetch done first through [`GitOps::fetch_many`] (parallel on the real adapter). Targets whose id
/// is in `skip_fetch` are not fetched (the caller knows their remote has not moved). Also returns the ids that were
/// really fetched successfully.
pub fn plan_fetch(git: &dyn GitOps, targets: &[UpdateTarget], mode: FetchMode, skip_fetch: &[String]) -> (UpdatePlan, Vec<String>) {
    plan_fetch_with(git, targets, mode, skip_fetch, &mut |_, _, _| {})
}

/// [`plan_fetch`] that reports progress: `progress(0, total, "")` before the fetch (one batch, so nothing finer
/// is known while it runs), then `progress(done, total, id)` as each target's result is read, in order.
pub fn plan_fetch_with(git: &dyn GitOps, targets: &[UpdateTarget], mode: FetchMode, skip_fetch: &[String], progress: &mut dyn FnMut(usize, usize, &str)) -> (UpdatePlan, Vec<String>) {
    let to_fetch: Vec<&UpdateTarget> = targets.iter().filter(|t| !skip_fetch.contains(&t.id)).collect();
    let total = targets.len();
    progress(0, total, "");
    let results = git.fetch_many(&to_fetch, mode);
    let mut fetched = Vec::new();
    let mut failed: Vec<(String, String)> = Vec::new();
    for (t, r) in to_fetch.iter().zip(results) {
        match r {
            Ok(()) => fetched.push(t.id.clone()),
            Err(e) => failed.push((t.id.clone(), e.to_string())),
        }
    }
    let items = targets
        .iter()
        .enumerate()
        .map(|(n, t)| {
            let item = match failed.iter().find(|(id, _)| *id == t.id) {
                Some((_, e)) => Err(UpdateError::Git(e.trim_start_matches("git error: ").to_string())),
                None => classify_fetched(git, t),
            }
            .unwrap_or_else(|e| PlanItem { target: t.clone(), current: String::new(), upstream: String::new(), behind: 0, action: Action::Skipped(SkipReason::Error(e.to_string())) });
            progress(n + 1, total, &t.id);
            item
        })
        .collect();
    (UpdatePlan { items }, fetched)
}

// ---------------------------------------------------------------- snapshot

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotEntry {
    pub id: String,
    pub path: PathBuf,
    pub commit: String,
    /// The branch the resource was on when the snapshot was taken (`"HEAD"` = none). A rollback goes back to it first, so
    /// it never resets a branch the person has switched to since. Older snapshots do not have it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

/// Previous commits, saved before applying (stored under `snapshots/`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub created_unix: u64,
    pub entries: Vec<SnapshotEntry>,
}

impl Snapshot {
    /// Written atomically (temp file in the same folder, then rename): a reader never sees half a snapshot.
    pub fn save(&self, file: &Path) -> Result<(), UpdateError> {
        let dir = file.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
        // Root (the update timer) writes this into the server folder: never through a planted link.
        let name = file.file_name().map(|n| n.to_os_string()).unwrap_or_else(|| "snapshot".into());
        crate::fsutil::write_no_follow(&dir.join(name), &serde_json::to_vec_pretty(self)?, 0o644)?;
        Ok(())
    }
    pub fn load(file: &Path) -> Result<Snapshot, UpdateError> {
        Ok(serde_json::from_slice(&std::fs::read(file)?)?)
    }
}

/// Snapshots kept per server; older ones are removed after each update.
pub const KEEP_SNAPSHOTS: usize = 20;

/// Where every snapshot of a server lives (manual and automatic runs alike): `<server>/.kora/snapshots/<id>.json`.
pub fn snapshot_dir(server_dir: &Path) -> PathBuf {
    server_dir.join(".kora").join("snapshots")
}

/// A snapshot id not used yet in `dir`: the time, with `-2`, `-3`... when two runs share a second.
pub fn new_snapshot_id(dir: &Path, now: u64) -> String {
    let mut id = now.to_string();
    let mut n = 1;
    while dir.join(format!("{id}.json")).exists() {
        n += 1;
        id = format!("{now}-{n}");
    }
    id
}

/// Saved snapshots of a server as `(id, snapshot)`, newest first.
pub fn list_snapshots(server_dir: &Path) -> Vec<(String, Snapshot)> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(snapshot_dir(server_dir)) else {
        return out;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let (Some(id), Ok(s)) = (p.file_stem().and_then(|x| x.to_str()), Snapshot::load(&p)) else {
            continue;
        };
        out.push((id.to_string(), s));
    }
    // Same second: `1700-10` is newer than `1700-9`, so compare the counter as a number.
    let order = |id: &str| id.rsplit_once('-').and_then(|(_, n)| n.parse::<u64>().ok()).unwrap_or(1);
    out.sort_by(|(ia, a), (ib, b)| b.created_unix.cmp(&a.created_unix).then_with(|| order(ib).cmp(&order(ia))).then_with(|| ib.cmp(ia)));
    out
}

/// Keep the newest `keep` snapshots; best effort.
pub fn prune_snapshots(server_dir: &Path, keep: usize) {
    for (id, _) in list_snapshots(server_dir).into_iter().skip(keep) {
        let _ = std::fs::remove_file(snapshot_dir(server_dir).join(format!("{id}.json")));
    }
}

// ------------------------------------------------------------------ holds

/// `<server>/.kora/held.json`: per resource, the target commit a rollback undid (automatic, or by hand).
/// Automatic updates do not try that commit again (it would fail every night); a newer target lifts the hold.
pub fn holds_path(server_dir: &Path) -> PathBuf {
    server_dir.join(".kora").join("held.json")
}

pub fn load_holds(server_dir: &Path) -> std::collections::BTreeMap<String, String> {
    std::fs::read(holds_path(server_dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_holds(server_dir: &Path, h: &std::collections::BTreeMap<String, String>) {
    let p = holds_path(server_dir);
    if h.is_empty() {
        let _ = std::fs::remove_file(&p);
        return;
    }
    // Never through a planted link (root writes this too).
    if let Ok(b) = serde_json::to_vec_pretty(h) {
        let _ = crate::fsutil::write_no_follow(&p, &b, 0o644);
    }
}

/// Text for a held resource.
pub fn held_text(commit: &str) -> String {
    format!("the update to {} was rolled back before; automatic updates wait for a newer version (update it by hand to try again)", short(commit))
}

/// Automatic updates: take out of the plan every pending item whose target is held, and lift holds
/// whose resource now has a different (newer) target. Returns the held `(id, commit)` pairs.
pub fn apply_holds(server_dir: &Path, plan: &mut UpdatePlan) -> Vec<(String, String)> {
    let mut holds = load_holds(server_dir);
    let before = holds.clone();
    let mut held = Vec::new();
    for item in &mut plan.items {
        let Some(c) = holds.get(&item.target.id).cloned() else {
            continue;
        };
        if item.upstream.is_empty() {
            continue; // could not check: keep the hold as it is
        }
        if item.upstream == c {
            if matches!(item.action, Action::FastForward | Action::NeedsRestart) {
                item.action = Action::UpToDate;
                held.push((item.target.id.clone(), c));
            }
        } else {
            holds.remove(&item.target.id);
        }
    }
    if holds != before {
        save_holds(server_dir, &holds);
    }
    held
}

/// After a run: hold the targets that were rolled back automatically, release the ones that updated.
pub fn record_holds(server_dir: &Path, plan: &UpdatePlan, report: &UpdateReport) {
    let mut holds = load_holds(server_dir);
    let before = holds.clone();
    for (id, _) in &report.rolled_back {
        if let Some(i) = plan.items.iter().find(|i| &i.target.id == id) {
            holds.insert(id.clone(), i.upstream.clone());
        }
    }
    for u in &report.updated {
        holds.remove(&u.id);
    }
    if holds != before {
        save_holds(server_dir, &holds);
    }
}

/// Manual rollback: reset every snapshot entry to its recorded commit.
pub fn rollback_snapshot(git: &dyn GitOps, snap: &Snapshot) -> Vec<(String, Result<(), String>)> {
    snap.entries
        .iter()
        .map(|e| {
            // Back on the branch the snapshot was taken on first: a reset must never rewrite some other branch.
            if let Some(b) = &e.branch {
                if let Ok(Some(now)) = git.current_branch(&e.path) {
                    if &now != b {
                        if let Err(x) = git.switch_branch(&e.path, b) {
                            return (e.id.clone(), Err(format!("could not go back to {} first: {x}", crate::git::branches::describe_other(b).replace("on branch ", "branch "))));
                        }
                    }
                }
            }
            (e.id.clone(), git.reset_to(&e.path, &e.commit).map_err(|x| x.to_string()))
        })
        .collect()
}

// ------------------------------------------------------------- quiet window

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeferReason {
    PlayersOnline(u32),
    PlayerCountUnknown,
    OutsideWindow,
}

impl DeferReason {
    fn text(&self) -> String {
        match self {
            Self::PlayersOnline(n) => format!("{n} player(s) online"),
            Self::PlayerCountUnknown => "player count unavailable".into(),
            Self::OutsideWindow => "outside quiet window".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestartDecision {
    Proceed,
    Defer(DeferReason),
}

#[derive(Debug, Clone)]
pub struct QuietConfig {
    /// Quiet window as minutes since midnight `(start, end)`; may wrap midnight. `None` = always.
    pub window: Option<(u32, u32)>,
    /// Restart allowed when players <= this (0 = empty server only).
    pub max_players: u32,
}

/// Decide whether restarts may happen now. Unknown player count defers (conservative).
pub fn decide_restart(players: Option<u32>, now_minute: u32, cfg: &QuietConfig) -> RestartDecision {
    if let Some((s, e)) = cfg.window {
        let inside = if s <= e { now_minute >= s && now_minute < e } else { now_minute >= s || now_minute < e };
        if !inside {
            return RestartDecision::Defer(DeferReason::OutsideWindow);
        }
    }
    match players {
        None => RestartDecision::Defer(DeferReason::PlayerCountUnknown),
        Some(n) if n > cfg.max_players => RestartDecision::Defer(DeferReason::PlayersOnline(n)),
        Some(_) => RestartDecision::Proceed,
    }
}

// ------------------------------------------------------------------ report

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Updated {
    pub id: String,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateReport {
    pub server: String,
    pub updated: Vec<Updated>,
    pub skipped: Vec<(String, SkipReason)>,
    pub deferred: Vec<(String, DeferReason)>,
    pub rolled_back: Vec<(String, String)>,
    /// SQL-migration updates; never auto-rolled-back.
    pub sql_flagged: Vec<String>,
    pub needs_attention: Vec<(String, String)>,
    /// Forks on GitHub fast-forwarded before updating (development).
    #[serde(default)]
    pub forks_synced: Vec<String>,
    /// Forks that were not touched (own commits / diverged / unknown) with the reason.
    #[serde(default)]
    pub forks_left_alone: Vec<(String, String)>,
    /// Resources the run never got to because an earlier one failed (nothing was changed in them).
    #[serde(default)]
    pub not_attempted: Vec<String>,
    /// Automatic updates only: resources whose new version was rolled back before, with the reason.
    #[serde(default)]
    pub held: Vec<(String, String)>,
    /// Warnings about the run as a whole (e.g. the built-in release list was used).
    #[serde(default)]
    pub notes: Vec<String>,
}

fn short(c: &str) -> &str {
    &c[..c.len().min(7)]
}

impl UpdateReport {
    pub fn has_problems(&self) -> bool {
        !self.needs_attention.is_empty() || !self.rolled_back.is_empty() || !self.not_attempted.is_empty()
    }

    pub fn summary(&self) -> String {
        let mut s = format!(
            "Kora update for {}: {} updated, {} skipped, {} deferred, {} rolled back, {} need attention\n",
            self.server,
            self.updated.len(),
            self.skipped.len(),
            self.deferred.len(),
            self.rolled_back.len(),
            self.needs_attention.len()
        );
        for u in &self.updated {
            s += &format!("  updated {} {} -> {}\n", u.id, short(&u.from), short(&u.to));
        }
        for (id, r) in &self.skipped {
            s += &format!("  skipped {id}: {}\n", r.text());
        }
        for (id, r) in &self.deferred {
            s += &format!("  deferred {id}: {}\n", r.text());
        }
        for (id, r) in &self.rolled_back {
            s += &format!("  rolled back {id}: {r}\n");
        }
        for (id, m) in &self.needs_attention {
            s += &format!("  ATTENTION {id}: {m}\n");
        }
        for id in &self.not_attempted {
            s += &format!("  not attempted {id}: the update stopped before it got here; nothing was changed\n");
        }
        for (id, m) in &self.held {
            s += &format!("  held {id}: {m}\n");
        }
        for n in &self.notes {
            s += &format!("  note: {n}\n");
        }
        if !self.forks_synced.is_empty() {
            s += &format!("  forks brought up to date on GitHub: {}\n", self.forks_synced.join(", "));
        }
        for (id, m) in &self.forks_left_alone {
            s += &format!("  fork {id}: {m}\n");
        }
        s
    }

    /// Discord webhook JSON body (not sent here).
    pub fn discord_payload(&self) -> serde_json::Value {
        let color = if !self.rolled_back.is_empty() {
            0xe74c3c
        } else if !self.needs_attention.is_empty() || !self.not_attempted.is_empty() || !self.held.is_empty() {
            0xf39c12
        } else {
            0x2ecc71
        };
        let list = |v: Vec<String>| {
            if v.is_empty() {
                "-".to_string()
            } else {
                let t = v.join("\n");
                t.chars().take(1000).collect()
            }
        };
        let fields = serde_json::json!([
            {"name": "Updated", "value": list(self.updated.iter().map(|u| format!("{} ({} → {})", u.id, short(&u.from), short(&u.to))).collect())},
            {"name": "Skipped", "value": list(self.skipped.iter().map(|(i, r)| format!("{i}: {}", r.text())).collect())},
            {"name": "Deferred", "value": list(self.deferred.iter().map(|(i, r)| format!("{i}: {}", r.text())).collect())},
            {"name": "Rolled back", "value": list(self.rolled_back.iter().map(|(i, r)| format!("{i}: {r}")).collect())},
            {"name": "Needs attention", "value": list(self.needs_attention.iter().map(|(i, m)| format!("{i}: {m}")).collect())},
        ]);
        let mut fields = fields;
        if !self.not_attempted.is_empty() {
            fields.as_array_mut().expect("array").push(serde_json::json!({"name": "Not attempted", "value": list(self.not_attempted.clone())}));
        }
        if !self.held.is_empty() {
            fields.as_array_mut().expect("array").push(serde_json::json!({"name": "Held back", "value": list(self.held.iter().map(|(i, m)| format!("{i}: {m}")).collect())}));
        }
        if !self.notes.is_empty() {
            fields.as_array_mut().expect("array").push(serde_json::json!({"name": "Notes", "value": list(self.notes.clone())}));
        }
        serde_json::json!({
            "username": "Kora Updater",
            "embeds": [{
                "title": format!("Kora update: {}", self.server),
                "color": color,
                "fields": fields,
            }]
        })
    }
}

// ------------------------------------------------------------------ engine

pub struct RunOptions<'a> {
    pub server: &'a str,
    pub snapshot_file: &'a Path,
    pub now_unix: u64,
    /// Result of [`decide_restart`]; restart-needing items are deferred unless `Proceed`.
    pub restart: RestartDecision,
}

/// Progress the engine reports while it applies a plan (streamed, not only before/after).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunEvent {
    /// The snapshot file was written (before anything moves).
    Snapshotted,
    /// About to move this resource.
    Applying(String),
    /// This resource moved to the new commit.
    Applied(String),
    /// Moving this resource failed.
    ApplyFailed(String, String),
    /// Restarting / checking the server.
    HealthCheck,
    /// This resource is being put back to the snapshot commit.
    RollingBack(String),
    /// The resources that will be tried, in order (sent once, before anything moves).
    Planned(Vec<String>),
    /// This resource is done without moving (carries the text to show): diverged, or never reached.
    Skipped(String, String),
}

/// Apply a plan: snapshot, fast-forward, health check, auto-rollback on failure.
pub fn run_update(git: &dyn GitOps, health: &dyn HealthCheck, migrations: &dyn MigrationDetector, plan: &UpdatePlan, opts: &RunOptions) -> Result<UpdateReport, UpdateError> {
    run_update_with(git, health, migrations, plan, opts, &mut |_| {})
}

/// [`run_update`] that calls `events` as each step happens.
pub fn run_update_with(
    git: &dyn GitOps,
    health: &dyn HealthCheck,
    migrations: &dyn MigrationDetector,
    plan: &UpdatePlan,
    opts: &RunOptions,
    events: &mut dyn FnMut(RunEvent),
) -> Result<UpdateReport, UpdateError> {
    let mut report = UpdateReport { server: opts.server.to_string(), ..Default::default() };
    let mut apply: Vec<&PlanItem> = Vec::new();
    for item in &plan.items {
        let id = item.target.id.clone();
        match (&item.action, &opts.restart) {
            (Action::UpToDate, _) => {}
            (Action::Skipped(r), _) => {
                report.skipped.push((id.clone(), r.clone()));
                if r.needs_attention() {
                    report.needs_attention.push((id, r.text()));
                }
            }
            (Action::NeedsRestart, RestartDecision::Defer(why)) => report.deferred.push((id, why.clone())),
            _ => apply.push(item),
        }
    }
    if apply.is_empty() {
        return Ok(report);
    }
    events(RunEvent::Planned(apply.iter().map(|i| i.target.id.clone()).collect()));

    Snapshot {
        created_unix: opts.now_unix,
        entries: apply
            .iter()
            .map(|i| SnapshotEntry { id: i.target.id.clone(), path: i.target.path.clone(), commit: i.current.clone(), branch: git.current_branch(&i.target.path).ok().flatten() })
            .collect(),
    }
    .save(opts.snapshot_file)?;
    events(RunEvent::Snapshotted);

    let sql: Vec<bool> = apply.iter().map(|i| migrations.has_migrations(&i.target.path, &i.current, &i.upstream)).collect();

    let mut applied: Vec<usize> = Vec::new();
    let mut failure: Option<(String, String)> = None;
    let mut stopped_at: Option<usize> = None;
    for (n, item) in apply.iter().enumerate() {
        events(RunEvent::Applying(item.target.id.clone()));
        match git.fast_forward(&item.target.path, &item.target.upstream_ref) {
            Ok(()) => {
                applied.push(n);
                events(RunEvent::Applied(item.target.id.clone()));
            }
            // The checkout has commits of its own (it can no longer fast-forward): that is `diverged`,
            // not a failure. Skip this resource, change nothing in it, and carry on with the rest.
            Err(_) if git.is_diverged(&item.target.path, &item.target.upstream_ref).unwrap_or(false) => {
                report.skipped.push((item.target.id.clone(), SkipReason::Diverged));
                report.needs_attention.push((item.target.id.clone(), SkipReason::Diverged.text()));
                events(RunEvent::Skipped(item.target.id.clone(), format!("skipped: {}", SkipReason::Diverged.text())));
            }
            Err(e) => {
                let msg = format!("fast-forward failed: {e}");
                events(RunEvent::ApplyFailed(item.target.id.clone(), msg.clone()));
                failure = Some((item.target.id.clone(), msg));
                stopped_at = Some(n);
                break;
            }
        }
    }
    // Everything after the failed one was never tried: say so, item by item.
    if let Some(n) = stopped_at {
        for i in &apply[n + 1..] {
            report.not_attempted.push(i.target.id.clone());
            events(RunEvent::Skipped(i.target.id.clone(), "not attempted: the update stopped before it got here".into()));
        }
    }

    let full_restart = applied.iter().any(|&n| apply[n].action == Action::NeedsRestart);
    let mut to_roll: Vec<usize> = Vec::new();
    let mut reason = String::new();
    // Set only when the health check ran (and so may have restarted the server).
    let mut checked: Option<HealthRequest> = None;
    if let Some((id, msg)) = &failure {
        reason = msg.clone();
        report.needs_attention.push((id.clone(), msg.clone()));
        to_roll = applied.clone();
    } else {
        events(RunEvent::HealthCheck);
        let req = HealthRequest { full_restart, updated: applied.iter().map(|&n| apply[n].target.id.clone()).collect() };
        if let HealthOutcome::Unhealthy { reason: r, failed } = health.check(&req) {
            reason = if failed.is_empty() { r } else { format!("{r} (failed: {})", failed.join(", ")) };
            to_roll = applied.clone();
        }
        checked = Some(req);
    }

    let mut kept: Vec<usize> = applied.clone();
    let mut rolled_ids = Vec::new();
    for &n in &to_roll {
        let item = apply[n];
        let id = item.target.id.clone();
        if sql[n] {
            report.needs_attention.push((id, format!("SQL migration included; not rolled back automatically ({reason})")));
            continue;
        }
        events(RunEvent::RollingBack(id.clone()));
        match git.reset_to(&item.target.path, &item.current) {
            Ok(()) => {
                kept.retain(|&k| k != n);
                rolled_ids.push(id.clone());
                report.rolled_back.push((id, reason.clone()));
            }
            Err(e) => report.needs_attention.push((id, format!("rollback failed: {e}"))),
        }
    }
    // Restart again only if the health check ran: a run that stopped before it never restarted the
    // server, which still runs the old code. The same restart rules as the check apply.
    if let (false, Some(req)) = (rolled_ids.is_empty(), checked) {
        health.recover(&HealthRequest { full_restart: req.full_restart, updated: rolled_ids });
    }

    for &n in &kept {
        let item = apply[n];
        report.updated.push(Updated { id: item.target.id.clone(), from: item.current.clone(), to: item.upstream.clone() });
        if sql[n] {
            report.sql_flagged.push(item.target.id.clone());
            if to_roll.is_empty() {
                report.needs_attention.push((item.target.id.clone(), "SQL migration applied; review and run it manually".into()));
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    #[derive(Clone)]
    struct Repo {
        head: String,
        upstream: String,
        dirty: bool,
        ahead: bool,
        behind: u32,
        /// Fast-forward fails (the checkout turned out to have its own commits).
        ff_fails: bool,
        /// Fast-forward fails for another reason (disk full, lock file...).
        ff_breaks: bool,
    }

    struct FakeGit(RefCell<HashMap<PathBuf, Repo>>);
    impl FakeGit {
        fn new(repos: Vec<(&str, Repo)>) -> Self {
            Self(RefCell::new(repos.into_iter().map(|(p, r)| (PathBuf::from(p), r)).collect()))
        }
        fn head(&self, p: &str) -> String {
            self.0.borrow()[&PathBuf::from(p)].head.clone()
        }
    }
    impl GitOps for FakeGit {
        fn fetch(&self, _: &Path) -> GitResult<()> {
            Ok(())
        }
        fn head_commit(&self, p: &Path, rev: &str) -> GitResult<String> {
            let r = self.0.borrow()[p].clone();
            Ok(if rev == "HEAD" { r.head } else { r.upstream })
        }
        fn fast_forward(&self, p: &Path, _: &str) -> GitResult<()> {
            let mut m = self.0.borrow_mut();
            let r = m.get_mut(p).unwrap();
            if r.ff_breaks {
                return Err(UpdateError::Git("fatal: Unable to create index.lock".into()));
            }
            if r.ff_fails {
                r.ahead = true;
                return Err(UpdateError::Git("fatal: Not possible to fast-forward, aborting.".into()));
            }
            r.head = r.upstream.clone();
            r.behind = 0;
            Ok(())
        }
        fn reset_to(&self, p: &Path, c: &str) -> GitResult<()> {
            self.0.borrow_mut().get_mut(p).unwrap().head = c.into();
            Ok(())
        }
        fn is_dirty(&self, p: &Path) -> GitResult<bool> {
            Ok(self.0.borrow()[p].dirty)
        }
        fn is_diverged(&self, p: &Path, _: &str) -> GitResult<bool> {
            Ok(self.0.borrow()[p].ahead)
        }
        fn behind_count(&self, p: &Path, _: &str) -> GitResult<u32> {
            Ok(self.0.borrow()[p].behind)
        }
    }

    struct FakeHealth {
        out: HealthOutcome,
        recovered: RefCell<Vec<HealthRequest>>,
    }
    impl FakeHealth {
        fn new(out: HealthOutcome) -> Self {
            Self { out, recovered: RefCell::new(vec![]) }
        }
    }
    impl HealthCheck for FakeHealth {
        fn check(&self, _: &HealthRequest) -> HealthOutcome {
            self.out.clone()
        }
        fn recover(&self, r: &HealthRequest) {
            self.recovered.borrow_mut().push(r.clone());
        }
    }

    struct Sql;
    impl MigrationDetector for Sql {
        fn has_migrations(&self, _: &Path, _: &str, _: &str) -> bool {
            true
        }
    }

    fn repo(head: &str, up: &str, behind: u32) -> Repo {
        Repo { head: head.into(), upstream: up.into(), dirty: false, ahead: false, behind, ff_fails: false, ff_breaks: false }
    }
    fn target(id: &str, path: &str) -> UpdateTarget {
        UpdateTarget { id: id.into(), path: path.into(), upstream_ref: "upstream/dev".into(), class: ResourceClass::from_id(id) }
    }
    fn opts<'a>(dir: &'a Path, d: RestartDecision, f: &'a Path) -> RunOptions<'a> {
        let _ = dir;
        RunOptions { server: "main", snapshot_file: f, now_unix: 1, restart: d }
    }

    #[test]
    fn clean_fast_forward() {
        let git = FakeGit::new(vec![("/a", repo("aaa", "bbb", 2)), ("/b", repo("ccc", "ccc", 0))]);
        let tmp = tempfile::tempdir().unwrap();
        let snap = tmp.path().join("snapshots/s.json");
        let p = plan(&git, &[target("kora-admin", "/a"), target("kora-x", "/b")]);
        assert_eq!(p.items[0].action, Action::FastForward);
        assert_eq!(p.items[1].action, Action::UpToDate);
        let h = FakeHealth::new(HealthOutcome::Healthy);
        let r = run_update(&git, &h, &NoMigrations, &p, &opts(tmp.path(), RestartDecision::Proceed, &snap)).unwrap();
        assert_eq!(r.updated.len(), 1);
        assert_eq!(git.head("/a"), "bbb");
        assert!(!r.has_problems());
        assert_eq!(Snapshot::load(&snap).unwrap().entries[0].commit, "aaa");
        assert!(r.summary().contains("1 updated"));
        assert_eq!(r.discord_payload()["embeds"][0]["color"], 0x2ecc71);
    }

    #[test]
    fn diverged_dirty_skipped() {
        let mut d = repo("a", "b", 1);
        d.ahead = true;
        let mut y = repo("c", "d", 1);
        y.dirty = true;
        let git = FakeGit::new(vec![("/d", d), ("/y", y)]);
        let tmp = tempfile::tempdir().unwrap();
        let snap = tmp.path().join("s.json");
        let p = plan(&git, &[target("x", "/d"), target("y", "/y")]);
        assert_eq!(p.items[0].action, Action::Skipped(SkipReason::Diverged));
        assert_eq!(p.items[1].action, Action::Skipped(SkipReason::Dirty));
        let h = FakeHealth::new(HealthOutcome::Healthy);
        let r = run_update(&git, &h, &NoMigrations, &p, &opts(tmp.path(), RestartDecision::Proceed, &snap)).unwrap();
        assert_eq!(r.skipped.len(), 2);
        assert_eq!(r.needs_attention.len(), 2);
        assert_eq!(git.head("/d"), "a");
        assert!(!snap.exists());
    }

    #[test]
    fn update_continues_after_a_resource_that_cannot_fast_forward() {
        let mut bad = repo("a", "b", 1);
        bad.ff_fails = true;
        let git = FakeGit::new(vec![("/bad", bad), ("/ok", repo("c", "d", 1))]);
        let tmp = tempfile::tempdir().unwrap();
        let snap = tmp.path().join("s.json");
        let p = plan(&git, &[target("kora-bad", "/bad"), target("kora-ok", "/ok")]);
        assert_eq!(p.items[0].action, Action::FastForward);
        let h = FakeHealth::new(HealthOutcome::Healthy);
        let r = run_update(&git, &h, &NoMigrations, &p, &opts(tmp.path(), RestartDecision::Proceed, &snap)).unwrap();
        assert_eq!(r.skipped, vec![("kora-bad".to_string(), SkipReason::Diverged)]);
        assert_eq!(r.updated.len(), 1);
        assert_eq!(r.updated[0].id, "kora-ok");
        assert!(r.rolled_back.is_empty());
        assert_eq!(git.head("/bad"), "a");
        assert_eq!(git.head("/ok"), "d");
        assert!(r.summary().contains("skipped kora-bad: diverged"), "{}", r.summary());
    }

    #[test]
    fn fork_sync_results_are_listed_without_failing_the_update() {
        use crate::git::SyncOutcome as O;
        let mut r = UpdateReport::default();
        add_fork_sync(&mut r, vec![("a".into(), O::Synced { commits: 2 }), ("b".into(), O::UpToDate), ("c".into(), O::Ahead { ahead: 1 }), ("d".into(), O::Diverged { ahead: 1, behind: 2 })]);
        assert_eq!(r.forks_synced, vec!["a".to_string()]);
        assert_eq!(r.forks_left_alone.len(), 2);
        assert!(!r.has_problems());
        let s = r.summary();
        assert!(s.contains("brought up to date on GitHub: a") && s.contains("fork d:") && s.contains("git merge upstream"), "{s}");
    }

    #[test]
    fn bad_update_rolled_back() {
        let git = FakeGit::new(vec![("/c", repo("old", "new", 1)), ("/o", repo("o1", "o2", 1))]);
        let tmp = tempfile::tempdir().unwrap();
        let snap = tmp.path().join("s.json");
        let p = plan(&git, &[target("kora-core", "/c"), target("kora-x", "/o")]);
        assert_eq!(p.items[0].action, Action::NeedsRestart);
        let h = FakeHealth::new(HealthOutcome::Unhealthy { reason: "boot failed".into(), failed: vec!["kora-core".into()] });
        let r = run_update(&git, &h, &NoMigrations, &p, &opts(tmp.path(), RestartDecision::Proceed, &snap)).unwrap();
        assert_eq!(git.head("/c"), "old");
        assert_eq!(git.head("/o"), "o1");
        assert_eq!(r.rolled_back.len(), 2);
        assert!(r.updated.is_empty());
        let rec = h.recovered.borrow();
        assert_eq!(rec.len(), 1);
        assert!(rec[0].full_restart);
        assert_eq!(r.discord_payload()["embeds"][0]["color"], 0xe74c3c);
    }

    #[test]
    fn players_online_defers_restart_items() {
        let git = FakeGit::new(vec![("/c", repo("old", "new", 1)), ("/o", repo("o1", "o2", 1))]);
        let tmp = tempfile::tempdir().unwrap();
        let snap = tmp.path().join("s.json");
        let cfg = QuietConfig { window: Some((240, 300)), max_players: 0 };
        assert_eq!(decide_restart(Some(3), 250, &cfg), RestartDecision::Defer(DeferReason::PlayersOnline(3)));
        assert_eq!(decide_restart(Some(0), 100, &cfg), RestartDecision::Defer(DeferReason::OutsideWindow));
        assert_eq!(decide_restart(None, 250, &cfg), RestartDecision::Defer(DeferReason::PlayerCountUnknown));
        assert_eq!(decide_restart(Some(0), 250, &cfg), RestartDecision::Proceed);
        let wrap = QuietConfig { window: Some((1380, 60)), max_players: 2 };
        assert_eq!(decide_restart(Some(2), 30, &wrap), RestartDecision::Proceed);
        let dec = decide_restart(Some(3), 250, &cfg);
        let p = plan(&git, &[target("kora-core", "/c"), target("kora-x", "/o")]);
        let h = FakeHealth::new(HealthOutcome::Healthy);
        let r = run_update(&git, &h, &NoMigrations, &p, &opts(tmp.path(), dec, &snap)).unwrap();
        assert_eq!(r.deferred.len(), 1);
        assert_eq!(git.head("/c"), "old");
        assert_eq!(git.head("/o"), "o2"); // live ensure still applied
        assert_eq!(r.updated.len(), 1);
    }

    #[test]
    fn sql_flagged_never_rolled_back() {
        let git = FakeGit::new(vec![("/s", repo("old", "new", 1))]);
        let tmp = tempfile::tempdir().unwrap();
        let snap = tmp.path().join("s.json");
        let p = plan(&git, &[target("kora-banking", "/s")]);
        let h = FakeHealth::new(HealthOutcome::Unhealthy { reason: "start failed".into(), failed: vec![] });
        let r = run_update(&git, &h, &Sql, &p, &opts(tmp.path(), RestartDecision::Proceed, &snap)).unwrap();
        assert_eq!(git.head("/s"), "new");
        assert!(r.rolled_back.is_empty());
        assert_eq!(r.sql_flagged, vec!["kora-banking".to_string()]);
        assert!(r.needs_attention[0].1.contains("not rolled back"));
        assert!(h.recovered.borrow().is_empty());
        // healthy path also flags
        let git = FakeGit::new(vec![("/s", repo("old", "new", 1))]);
        let p = plan(&git, &[target("kora-banking", "/s")]);
        let h = FakeHealth::new(HealthOutcome::Healthy);
        let r = run_update(&git, &h, &Sql, &p, &opts(tmp.path(), RestartDecision::Proceed, &snap)).unwrap();
        assert_eq!(r.updated.len(), 1);
        assert_eq!(r.needs_attention.len(), 1);
    }

    #[test]
    fn manual_snapshot_rollback() {
        let git = FakeGit::new(vec![("/a", repo("new", "new", 0))]);
        let s = Snapshot { created_unix: 1, entries: vec![SnapshotEntry { id: "a".into(), path: "/a".into(), commit: "old".into(), branch: None }] };
        assert!(rollback_snapshot(&git, &s)[0].1.is_ok());
        assert_eq!(git.head("/a"), "old");
    }

    #[test]
    fn par_map_keeps_order_and_never_exceeds_the_limit() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let (now, peak) = (AtomicUsize::new(0), AtomicUsize::new(0));
        let out = par_map((0..30).collect::<Vec<u32>>(), 4, |i| {
            let n = now.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(n, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(5));
            now.fetch_sub(1, Ordering::SeqCst);
            i * 2
        });
        assert_eq!(out, (0..30).map(|i| i * 2).collect::<Vec<_>>());
        let p = peak.load(Ordering::SeqCst);
        assert!((2..=4).contains(&p), "peak {p}");
        assert_eq!(par_map(Vec::<u8>::new(), 6, |x| x), Vec::<u8>::new());
        assert_eq!(par_map(vec![1, 2, 3], 0, |x| x + 1), vec![2, 3, 4], "a zero limit still runs everything");
    }

    #[test]
    fn a_mid_run_failure_lists_the_rest_as_not_attempted_and_restarts_nothing() {
        let mut bad = repo("b1", "b2", 1);
        bad.ff_breaks = true;
        let git = FakeGit::new(vec![("/a", repo("a1", "a2", 1)), ("/b", bad), ("/c", repo("c1", "c2", 1)), ("/d", repo("d1", "d2", 1))]);
        let tmp = tempfile::tempdir().unwrap();
        let snap = tmp.path().join("s.json");
        let p = plan(&git, &[target("kora-a", "/a"), target("kora-b", "/b"), target("kora-c", "/c"), target("kora-core", "/d")]);
        let h = FakeHealth::new(HealthOutcome::Healthy);
        let r = run_update(&git, &h, &NoMigrations, &p, &opts(tmp.path(), RestartDecision::Proceed, &snap)).unwrap();
        assert_eq!(r.not_attempted, vec!["kora-c".to_string(), "kora-core".to_string()]);
        assert_eq!(r.rolled_back.len(), 1, "the one that moved goes back");
        assert_eq!((git.head("/a"), git.head("/c"), git.head("/d")), ("a1".into(), "c1".into(), "d1".into()));
        assert!(h.recovered.borrow().is_empty(), "the server was never restarted, so it is not restarted now");
        assert!(r.has_problems());
        assert!(r.summary().contains("not attempted kora-c"), "{}", r.summary());
    }

    #[test]
    fn recovery_after_a_failed_check_uses_the_same_restart_choice_as_the_check() {
        let git = FakeGit::new(vec![("/o", repo("o1", "o2", 1))]);
        let tmp = tempfile::tempdir().unwrap();
        let snap = tmp.path().join("s.json");
        let p = plan(&git, &[target("kora-x", "/o")]);
        let h = FakeHealth::new(HealthOutcome::Unhealthy { reason: "x".into(), failed: vec![] });
        let r = run_update(&git, &h, &NoMigrations, &p, &opts(tmp.path(), RestartDecision::Proceed, &snap)).unwrap();
        assert_eq!(r.rolled_back.len(), 1);
        let rec = h.recovered.borrow();
        assert_eq!(rec.len(), 1);
        assert!(!rec[0].full_restart, "only a live resource changed: no full restart to recover with");
    }

    #[test]
    fn snapshots_are_saved_atomically_listed_newest_first_and_pruned() {
        let t = tempfile::tempdir().unwrap();
        let d = snapshot_dir(t.path());
        for (i, now) in [5u64, 5, 7, 6].into_iter().enumerate() {
            let id = new_snapshot_id(&d, now);
            Snapshot { created_unix: now, entries: vec![SnapshotEntry { id: format!("r{i}"), path: "/x".into(), commit: "c".into(), branch: None }] }.save(&d.join(format!("{id}.json"))).unwrap();
        }
        let ids: Vec<String> = list_snapshots(t.path()).into_iter().map(|(i, _)| i).collect();
        assert_eq!(ids, vec!["7", "6", "5-2", "5"]);
        assert!(std::fs::read_dir(&d).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().ends_with(".tmp")));
        prune_snapshots(t.path(), 2);
        let ids: Vec<String> = list_snapshots(t.path()).into_iter().map(|(i, _)| i).collect();
        assert_eq!(ids, vec!["7", "6"]);
    }

    #[test]
    fn a_rolled_back_target_is_held_until_a_newer_one_appears() {
        let t = tempfile::tempdir().unwrap();
        let git = FakeGit::new(vec![("/o", repo("o1", "o2", 1))]);
        let snap = t.path().join("s.json");
        let p = plan(&git, &[target("kora-x", "/o")]);
        let h = FakeHealth::new(HealthOutcome::Unhealthy { reason: "x".into(), failed: vec![] });
        let r = run_update(&git, &h, &NoMigrations, &p, &opts(t.path(), RestartDecision::Proceed, &snap)).unwrap();
        record_holds(t.path(), &p, &r);
        assert_eq!(load_holds(t.path()).get("kora-x").map(String::as_str), Some("o2"));
        git.0.borrow_mut().get_mut(Path::new("/o")).unwrap().behind = 1;
        // Next night: same target, held, nothing moves.
        let mut p = plan(&git, &[target("kora-x", "/o")]);
        assert_eq!(apply_holds(t.path(), &mut p), vec![("kora-x".to_string(), "o2".to_string())]);
        assert!(p.pending().next().is_none());
        // Upstream moves on: the hold is lifted and the new target is tried.
        git.0.borrow_mut().get_mut(Path::new("/o")).unwrap().upstream = "o3".into();
        let mut p = plan(&git, &[target("kora-x", "/o")]);
        assert!(apply_holds(t.path(), &mut p).is_empty());
        assert_eq!(p.pending().count(), 1);
        assert!(load_holds(t.path()).is_empty());
        assert!(!holds_path(t.path()).exists());
    }
}
