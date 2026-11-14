//! Update engine shared by manual updates and `kora update --auto`.
//!
//! Flow: [`plan`] (classify each resource) -> [`run_update`] (snapshot, apply,
//! health check, auto-rollback) -> [`UpdateReport`] (text summary + Discord payload).
//! Git and the server health check are behind traits so everything is testable offline.
//! SQL migrations are flagged and never rolled back automatically.

pub mod manual;

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
}

impl SkipReason {
    fn needs_attention(&self) -> bool {
        !matches!(self, SkipReason::LocalCommits)
    }
    fn text(&self) -> String {
        match self {
            SkipReason::LocalCommits => "has local commits".into(),
            SkipReason::Dirty => "uncommitted changes".into(),
            SkipReason::Diverged => "diverged from upstream".into(),
            SkipReason::Error(e) => format!("error: {e}"),
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
        self.items
            .iter()
            .filter(|i| matches!(i.action, Action::FastForward | Action::NeedsRestart))
    }
}

fn classify(git: &dyn GitOps, t: &UpdateTarget) -> GitResult<PlanItem> {
    git.fetch(&t.path)?;
    let current = git.head_commit(&t.path, "HEAD")?;
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
    let items = targets
        .iter()
        .map(|t| {
            classify(git, t).unwrap_or_else(|e| PlanItem {
                target: t.clone(),
                current: String::new(),
                upstream: String::new(),
                behind: 0,
                action: Action::Skipped(SkipReason::Error(e.to_string())),
            })
        })
        .collect();
    UpdatePlan { items }
}

// ---------------------------------------------------------------- snapshot

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotEntry {
    pub id: String,
    pub path: PathBuf,
    pub commit: String,
}

/// Previous commits, saved before applying (stored under `snapshots/`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub created_unix: u64,
    pub entries: Vec<SnapshotEntry>,
}

impl Snapshot {
    pub fn save(&self, file: &Path) -> Result<(), UpdateError> {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(file, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
    pub fn load(file: &Path) -> Result<Snapshot, UpdateError> {
        Ok(serde_json::from_slice(&std::fs::read(file)?)?)
    }
}

/// Manual rollback: reset every snapshot entry to its recorded commit.
pub fn rollback_snapshot(git: &dyn GitOps, snap: &Snapshot) -> Vec<(String, Result<(), String>)> {
    snap.entries
        .iter()
        .map(|e| (e.id.clone(), git.reset_to(&e.path, &e.commit).map_err(|x| x.to_string())))
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
}

fn short(c: &str) -> &str {
    &c[..c.len().min(7)]
}

impl UpdateReport {
    pub fn has_problems(&self) -> bool {
        !self.needs_attention.is_empty() || !self.rolled_back.is_empty()
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
        s
    }

    /// Discord webhook JSON body (not sent here).
    pub fn discord_payload(&self) -> serde_json::Value {
        let color = if !self.rolled_back.is_empty() {
            0xe74c3c
        } else if !self.needs_attention.is_empty() {
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

/// Apply a plan: snapshot, fast-forward, health check, auto-rollback on failure.
pub fn run_update(
    git: &dyn GitOps,
    health: &dyn HealthCheck,
    migrations: &dyn MigrationDetector,
    plan: &UpdatePlan,
    opts: &RunOptions,
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

    Snapshot {
        created_unix: opts.now_unix,
        entries: apply
            .iter()
            .map(|i| SnapshotEntry { id: i.target.id.clone(), path: i.target.path.clone(), commit: i.current.clone() })
            .collect(),
    }
    .save(opts.snapshot_file)?;

    let sql: Vec<bool> = apply
        .iter()
        .map(|i| migrations.has_migrations(&i.target.path, &i.current, &i.upstream))
        .collect();

    let mut applied: Vec<usize> = Vec::new();
    let mut failure: Option<(String, String)> = None;
    for (n, item) in apply.iter().enumerate() {
        match git.fast_forward(&item.target.path, &item.target.upstream_ref) {
            Ok(()) => applied.push(n),
            Err(e) => {
                failure = Some((item.target.id.clone(), format!("fast-forward failed: {e}")));
                break;
            }
        }
    }

    let full_restart = applied.iter().any(|&n| apply[n].action == Action::NeedsRestart);
    let mut to_roll: Vec<usize> = Vec::new();
    let mut reason = String::new();
    if let Some((id, msg)) = &failure {
        reason = msg.clone();
        report.needs_attention.push((id.clone(), msg.clone()));
        to_roll = applied.clone();
    } else {
        let req = HealthRequest {
            full_restart,
            updated: applied.iter().map(|&n| apply[n].target.id.clone()).collect(),
        };
        if let HealthOutcome::Unhealthy { reason: r, failed } = health.check(&req) {
            reason = if failed.is_empty() { r } else { format!("{r} (failed: {})", failed.join(", ")) };
            to_roll = applied.clone();
        }
    }

    let mut kept: Vec<usize> = applied.clone();
    let mut rolled_ids = Vec::new();
    for &n in &to_roll {
        let item = apply[n];
        let id = item.target.id.clone();
        if sql[n] {
            report
                .needs_attention
                .push((id, format!("SQL migration included; not rolled back automatically ({reason})")));
            continue;
        }
        match git.reset_to(&item.target.path, &item.current) {
            Ok(()) => {
                kept.retain(|&k| k != n);
                rolled_ids.push(id.clone());
                report.rolled_back.push((id, reason.clone()));
            }
            Err(e) => report.needs_attention.push((id, format!("rollback failed: {e}"))),
        }
    }
    if !rolled_ids.is_empty() {
        health.recover(&HealthRequest { full_restart, updated: rolled_ids });
    }

    for &n in &kept {
        let item = apply[n];
        report.updated.push(Updated {
            id: item.target.id.clone(),
            from: item.current.clone(),
            to: item.upstream.clone(),
        });
        if sql[n] {
            report.sql_flagged.push(item.target.id.clone());
            if to_roll.is_empty() {
                report
                    .needs_attention
                    .push((item.target.id.clone(), "SQL migration applied; review and run it manually".into()));
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
        Repo { head: head.into(), upstream: up.into(), dirty: false, ahead: false, behind }
    }
    fn target(id: &str, path: &str) -> UpdateTarget {
        UpdateTarget {
            id: id.into(),
            path: path.into(),
            upstream_ref: "upstream/dev".into(),
            class: ResourceClass::from_id(id),
        }
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
        let s = Snapshot { created_unix: 1, entries: vec![SnapshotEntry { id: "a".into(), path: "/a".into(), commit: "old".into() }] };
        assert!(rollback_snapshot(&git, &s)[0].1.is_ok());
        assert_eq!(git.head("/a"), "old");
    }
}
