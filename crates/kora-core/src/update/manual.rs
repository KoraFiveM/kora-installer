//! Manual update flow: preview, snapshot, apply, rollback; single resources or everything.
//!
//! Development servers fetch `upstream` and fast-forward only (never merge; dirty or diverged
//! forks are skipped with a plain reason, exactly like `--auto`). Production servers move each
//! resource to the PINNED commit of the catalog's `latest` release (always a commit, never a tag).
//! The engine underneath is the one `kora update --auto` uses ([`super::plan`] and
//! [`super::run_update`]); this module adds scope selection, the restart policy, snapshot ids and
//! events, and keeps the pinned commits in `kora-server.json` current.

use std::path::{Path, PathBuf};

use super::status;
use super::{
    decide_restart, plan, rollback_snapshot, run_update_with, Action, DeferReason, GitOps, GitResult, HealthCheck, HealthOutcome,
    HealthRequest, NoMigrations, QuietConfig, RestartDecision, ResourceClass, RunEvent, RunOptions, SkipReason, Snapshot, UpdateReport,
    UpdateTarget,
};
use crate::catalog::{self, normalize_id, Catalog};
use crate::installer::gitops::{online_players, service_health, with_git};
use crate::installer::http::HttpClient;
use crate::installer::registry::{now_unix, Registry, ServerFile};
use crate::installer::types::Mode;
use crate::installer::{InstallError, Result};

/// A registered server as recorded in its `kora-server.json`.
pub type ServerRecord = ServerFile;

/// What `Restart::Ensure` really does. txAdmin 8.1.2 / FXServer expose no way to `ensure <resource>` on a running
/// server without credentials (txAdmin's web API and live console need a signed-in admin session; FXServer's rcon is
/// off unless the owner sets `rcon_password`), and Kora does not keep or reuse the txAdmin password. So `Ensure` applies
/// the files, does NOT restart, and says so: the new files load on the next restart, or when an admin runs
/// `ensure <resource>` in txAdmin's live console.
pub const ENSURE_NOTE: &str = "files updated, server not restarted: restart the server, or run `ensure <resource>` in txAdmin's live console, to load them";

/// How many commit subjects a changelog carries.
const CHANGELOG_MAX: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    All,
    Resources(Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeStatus {
    UpToDate,
    /// Can be updated.
    Clean,
    /// Uncommitted local changes: skipped.
    Dirty,
    /// Local commits and new upstream commits (or could not be checked): skipped.
    Diverged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceChange {
    pub id: String,
    pub from: String,
    pub to: String,
    pub behind: u32,
    /// Commit subjects between old and new, newest first, at most ~20.
    pub changelog: Vec<String>,
    pub needs_restart: bool,
    pub status: ChangeStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdatePreview {
    pub resources: Vec<ResourceChange>,
    /// Production: the newest release version the catalog offers, when it is newer than what is installed.
    pub new_release: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartPolicy {
    /// Apply the files, restart nothing.
    None,
    /// Apply the files, no restart (a live `ensure` needs txAdmin credentials Kora does not keep: see [`ENSURE_NOTE`]).
    Ensure,
    /// Always restart the whole server.
    FullRestart,
    /// Restart only when nobody is playing; otherwise leave restart-needing resources for later.
    WhenQuiet,
}

#[derive(Debug, Clone)]
pub struct ApplyOpts {
    pub restart: RestartPolicy,
    /// Resource ids to leave alone.
    pub skip: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateEvent {
    Resource { id: String, state: String },
    /// A snapshot with this id was saved.
    Snapshot(String),
    Health(String),
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotInfo {
    pub id: String,
    pub created_unix: u64,
    pub resources: Vec<String>,
}

fn err<T>(m: impl Into<String>) -> Result<T> {
    Err(InstallError(m.into()))
}

/// Process exit code for a finished update: 0 ok, 1 rolled back, 3 needs attention.
pub fn exit_code(r: &UpdateReport) -> i32 {
    if !r.rolled_back.is_empty() {
        1
    } else if !r.needs_attention.is_empty() {
        3
    } else {
        0
    }
}

/// Find a registered server (name or folder) and read its `kora-server.json`.
pub fn load_server(server: &str) -> Result<ServerRecord> {
    let as_dir = Path::new(server);
    let dir = if as_dir.is_absolute() && as_dir.join("kora-server.json").is_file() {
        as_dir.to_path_buf()
    } else {
        let reg = Registry::load().map_err(|e| InstallError(format!("cannot read the server registry: {e}")))?;
        reg.find(server).ok_or_else(|| InstallError(format!("no registered server named '{server}' (see `kora status`)")))?.dir.clone()
    };
    ServerFile::load(&dir).map_err(|e| InstallError(format!("cannot read {}: {e}", dir.join("kora-server.json").display())))
}

fn snapshot_dir(server: &ServerRecord) -> PathBuf {
    server.dir.join(".kora").join("snapshots")
}

fn valid_snapshot_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

// ---------------------------------------------------------------- production git view

/// Production view of git: the "upstream" of a resource is a pinned commit, and "fast-forward"
/// moves the checkout to it (detached). Everything else is the plain adapter.
struct PinnedGit<'a>(&'a dyn GitOps);

impl GitOps for PinnedGit<'_> {
    fn fetch(&self, repo: &Path) -> GitResult<()> {
        // Best effort: the commit may already be present, and moving fetches it by id anyway.
        let _ = self.0.fetch_origin(repo);
        Ok(())
    }
    fn head_commit(&self, repo: &Path, rev: &str) -> GitResult<String> {
        if rev == "HEAD" {
            self.0.head_commit(repo, rev)
        } else {
            Ok(rev.to_string())
        }
    }
    fn fast_forward(&self, repo: &Path, commit: &str) -> GitResult<()> {
        self.0.move_pinned(repo, commit)
    }
    fn reset_to(&self, repo: &Path, commit: &str) -> GitResult<()> {
        self.0.reset_to(repo, commit)
    }
    fn is_dirty(&self, repo: &Path) -> GitResult<bool> {
        self.0.is_dirty(repo)
    }
    fn is_diverged(&self, _: &Path, _: &str) -> GitResult<bool> {
        Ok(false)
    }
    fn behind_count(&self, repo: &Path, commit: &str) -> GitResult<u32> {
        if self.0.head_commit(repo, "HEAD")? == commit {
            return Ok(0);
        }
        Ok(self.0.behind_count(repo, commit).unwrap_or(1))
    }
    fn log_subjects(&self, repo: &Path, from: &str, to: &str, max: usize) -> Vec<String> {
        self.0.log_subjects(repo, from, to, max)
    }
}

// ---------------------------------------------------------------- selection

struct Selection {
    targets: Vec<UpdateTarget>,
    /// Pinned release version per target id (production).
    versions: Vec<(String, String)>,
}

fn select(server: &ServerRecord, scope: &Scope, skip: &[String], catalog: Option<&Catalog>) -> Result<Selection> {
    let wanted: Option<Vec<String>> = match scope {
        Scope::All => None,
        Scope::Resources(ids) => Some(ids.iter().map(|i| normalize_id(i)).collect()),
    };
    if let Some(w) = &wanted {
        if let Some(bad) = w.iter().find(|id| !server.resources.iter().any(|r| &normalize_id(&r.id) == *id)) {
            return err(format!("this server has no resource named '{bad}'"));
        }
    }
    let skip: Vec<String> = skip.iter().map(|s| normalize_id(s)).collect();
    let prod = server.mode == Mode::Production;
    let cat = if prod {
        Some(catalog.ok_or_else(|| InstallError("could not read the release catalog, so releases cannot be compared".into()))?)
    } else {
        None
    };
    let mut sel = Selection { targets: Vec::new(), versions: Vec::new() };
    for r in &server.resources {
        let n = normalize_id(&r.id);
        if wanted.as_ref().is_some_and(|w| !w.contains(&n)) || skip.contains(&n) {
            continue;
        }
        // Development tracks forks; third-party (bridge / creator) clones are pinned and have no upstream.
        if !prod && r.third_party {
            continue;
        }
        let path = server.dir.join("resources").join(&r.path);
        // A copy of one subfolder of a repository has no git history to compare: left as installed.
        if crate::installer::pinned::is_subpath_copy(&path) {
            continue;
        }
        let upstream_ref = match cat {
            None => format!("upstream/{}", r.branch),
            Some(c) => match c.find_normalized(&r.id).and_then(|res| res.latest_release()) {
                Some(rel) => {
                    sel.versions.push((r.id.clone(), rel.version.clone()));
                    rel.commit.clone()
                }
                // No release is published for it: nothing to move to.
                None => continue,
            },
        };
        sel.targets.push(UpdateTarget { id: r.id.clone(), path, upstream_ref, class: ResourceClass::from_id(&r.id) });
    }
    Ok(sel)
}

fn git_view<'a>(server: &ServerRecord, git: &'a dyn GitOps, pinned: &'a PinnedGit<'a>) -> &'a dyn GitOps {
    if server.mode == Mode::Production {
        pinned
    } else {
        git
    }
}

// ---------------------------------------------------------------- preview

fn change_of(git: &dyn GitOps, item: &super::PlanItem) -> ResourceChange {
    let mut changelog = Vec::new();
    let status = match &item.action {
        Action::UpToDate | Action::Skipped(SkipReason::LocalCommits) => ChangeStatus::UpToDate,
        Action::FastForward | Action::NeedsRestart => ChangeStatus::Clean,
        Action::Skipped(SkipReason::Dirty) => ChangeStatus::Dirty,
        Action::Skipped(SkipReason::Diverged) => ChangeStatus::Diverged,
        Action::Skipped(SkipReason::Error(e)) => {
            changelog.push(format!("could not check: {e}"));
            ChangeStatus::Diverged
        }
    };
    if item.behind > 0 && !matches!(status, ChangeStatus::UpToDate) {
        changelog = git.log_subjects(&item.target.path, &item.current, &item.upstream, CHANGELOG_MAX);
    }
    ResourceChange {
        id: item.target.id.clone(),
        from: item.current.clone(),
        to: if status == ChangeStatus::UpToDate { item.current.clone() } else { item.upstream.clone() },
        behind: item.behind,
        changelog,
        needs_restart: item.target.class.needs_restart(),
        status,
    }
}

/// Fetch and compare. Changes nothing in any checkout.
pub fn preview(server: &ServerRecord, scope: Scope) -> Result<UpdatePreview> {
    let cat = load_catalog_if_needed(server)?;
    with_git(&server.dir, |g| preview_with(server, scope, g, cat.as_ref()))
}

fn load_catalog_if_needed(server: &ServerRecord) -> Result<Option<Catalog>> {
    if server.mode != Mode::Production {
        return Ok(None);
    }
    let (c, _) = catalog::load(&HttpClient::default(), catalog::CATALOG_URL).map_err(|e| InstallError(e.to_string()))?;
    Ok(Some(c))
}

/// [`preview`] with injected git and catalog.
pub fn preview_with(server: &ServerRecord, scope: Scope, git: &dyn GitOps, catalog: Option<&Catalog>) -> Result<UpdatePreview> {
    let sel = select(server, &scope, &[], catalog)?;
    let pinned = PinnedGit(git);
    let g = git_view(server, git, &pinned);
    let p = plan(g, &sel.targets);
    let resources: Vec<ResourceChange> = p.items.iter().map(|i| change_of(g, i)).collect();
    let pre = UpdatePreview { new_release: new_release(&resources, &sel.versions), resources };
    // Scope::All only: a one-resource preview says nothing about the server as a whole.
    if scope == Scope::All {
        let ready = pre.resources.iter().filter(|c| c.status == ChangeStatus::Clean).count();
        let attention = pre.resources.iter().filter(|c| matches!(c.status, ChangeStatus::Dirty | ChangeStatus::Diverged)).count();
        status::record(&server.dir, &status::UpdateStatus::new(now_unix(), ready, attention, pre.new_release.clone()));
    }
    Ok(pre)
}

/// The release version to announce: the core's when it changes, else the first changing resource's.
fn new_release(changes: &[ResourceChange], versions: &[(String, String)]) -> Option<String> {
    let moving: Vec<&ResourceChange> = changes.iter().filter(|c| c.status != ChangeStatus::UpToDate).collect();
    let pick = moving.iter().find(|c| normalize_id(&c.id) == "kora-core").or_else(|| moving.first())?;
    versions.iter().find(|(id, _)| *id == pick.id).map(|(_, v)| v.clone())
}

// ---------------------------------------------------------------- apply

/// Wraps the real health check to honour the chosen restart policy.
struct PolicyHealth<'a> {
    inner: &'a dyn HealthCheck,
    policy: RestartPolicy,
}

impl HealthCheck for PolicyHealth<'_> {
    fn check(&self, req: &HealthRequest) -> HealthOutcome {
        match self.policy {
            RestartPolicy::None => HealthOutcome::Healthy,
            RestartPolicy::Ensure => self.inner.check(&HealthRequest { full_restart: false, ..req.clone() }),
            RestartPolicy::FullRestart => self.inner.check(&HealthRequest { full_restart: true, ..req.clone() }),
            RestartPolicy::WhenQuiet => self.inner.check(req),
        }
    }
    fn recover(&self, req: &HealthRequest) {
        if self.policy != RestartPolicy::None {
            self.inner.recover(req);
        }
    }
}

fn new_snapshot_id(dir: &Path, now: u64) -> String {
    let mut id = now.to_string();
    let mut n = 1;
    while dir.join(format!("{id}.json")).exists() {
        n += 1;
        id = format!("{now}-{n}");
    }
    id
}

/// Snapshot, apply, restart per policy, health check, auto-rollback on failure.
pub fn apply(server: &ServerRecord, scope: Scope, opts: ApplyOpts, sink: &mut dyn FnMut(UpdateEvent)) -> Result<UpdateReport> {
    let cat = load_catalog_if_needed(server)?;
    let health = service_health(&server.name, &server.dir, server);
    let players = if health.running() { online_players(server.ports.game) } else { Some(0) };
    let report = with_git(&server.dir, |g| apply_with(server, scope, opts, g, &health, cat.as_ref(), players, now_unix(), sink))?;
    // Same Discord notification as the automatic path (it never fails the update).
    let record = crate::installer::registry::ServerEntry { name: server.name.clone(), dir: server.dir.clone(), mode: server.mode, ports: server.ports, created_unix: server.created_unix };
    crate::notify::after_update(&record, &report);
    Ok(report)
}

/// [`apply`] with injected git, health, catalog, player count and clock.
#[allow(clippy::too_many_arguments)]
pub fn apply_with(
    server: &ServerRecord,
    scope: Scope,
    opts: ApplyOpts,
    git: &dyn GitOps,
    health: &dyn HealthCheck,
    catalog: Option<&Catalog>,
    players: Option<u32>,
    now: u64,
    sink: &mut dyn FnMut(UpdateEvent),
) -> Result<UpdateReport> {
    let sel = select(server, &scope, &opts.skip, catalog)?;
    let pinned = PinnedGit(git);
    let g = git_view(server, git, &pinned);
    let the_plan = plan(g, &sel.targets);
    let restart = match opts.restart {
        RestartPolicy::WhenQuiet => decide_restart(players, 0, &QuietConfig { window: None, max_players: 0 }),
        _ => RestartDecision::Proceed,
    };
    let will_apply: Vec<&str> = the_plan
        .pending()
        .filter(|i| i.action == Action::FastForward || restart == RestartDecision::Proceed)
        .map(|i| i.target.id.as_str())
        .collect();

    let pending_release = new_release(&the_plan.items.iter().map(|i| change_of(g, i)).collect::<Vec<_>>(), &sel.versions);
    let snaps = snapshot_dir(server);
    let id = new_snapshot_id(&snaps, now);
    let file = snaps.join(format!("{id}.json"));
    let ph = PolicyHealth { inner: health, policy: opts.restart };
    let run = RunOptions { server: &server.name, snapshot_file: &file, now_unix: now, restart };
    // Streamed from inside the engine, so a front end sees each resource as it happens.
    let mut report = {
        let snap_id = id.clone();
        let mut on = |e: RunEvent| match e {
            RunEvent::Snapshotted => sink(UpdateEvent::Snapshot(snap_id.clone())),
            RunEvent::Applying(r) => sink(UpdateEvent::Resource { id: r, state: "updating".into() }),
            RunEvent::Applied(r) => sink(UpdateEvent::Resource { id: r, state: "updated".into() }),
            RunEvent::ApplyFailed(r, why) => sink(UpdateEvent::Resource { id: r, state: format!("failed: {why}") }),
            RunEvent::HealthCheck => sink(UpdateEvent::Health("checking the server".into())),
            RunEvent::RollingBack(r) => sink(UpdateEvent::Resource { id: r, state: "rolling back".into() }),
        };
        run_update_with(g, &ph, &NoMigrations, &the_plan, &run, &mut on).map_err(|e| InstallError(e.to_string()))?
    };

    if server.mode == Mode::Production && !report.updated.is_empty() {
        let moved: Vec<(String, String)> = report.updated.iter().map(|u| (u.id.clone(), u.to.clone())).collect();
        if let Err(m) = record_commits(server, &moved) {
            report.needs_attention.push(("kora-server.json".into(), m));
        }
    }

    for (id, why) in &report.skipped {
        sink(UpdateEvent::Resource { id: id.clone(), state: format!("skipped: {}", why.text()) });
    }
    for (id, why) in &report.deferred {
        sink(UpdateEvent::Resource { id: id.clone(), state: format!("waiting: {}", defer_text(why)) });
    }
    for (id, why) in &report.rolled_back {
        sink(UpdateEvent::Resource { id: id.clone(), state: format!("rolled back: {why}") });
    }
    if opts.restart == RestartPolicy::Ensure && !report.updated.is_empty() {
        sink(UpdateEvent::Health(ENSURE_NOTE.into()));
    }
    if !will_apply.is_empty() {
        sink(UpdateEvent::Health(if report.rolled_back.is_empty() {
            "server is healthy".into()
        } else {
            "server did not come back healthy; went back to the snapshot".into()
        }));
    }
    // The status file is about the whole server: after a single-resource apply it is left for the next full check.
    if scope == Scope::All {
        status::record(&server.dir, &apply_status(&report, now, &pending_release));
    }
    sink(UpdateEvent::Done);
    Ok(report)
}

/// Status after a run: what is still waiting, what needs a person, and the release to announce while something is left.
pub fn apply_status(report: &UpdateReport, now: u64, release: &Option<String>) -> status::UpdateStatus {
    let mut attention: Vec<&str> = report.needs_attention.iter().map(|(i, _)| i.as_str()).chain(report.rolled_back.iter().map(|(i, _)| i.as_str())).collect();
    attention.sort_unstable();
    attention.dedup();
    let waiting = report.deferred.len();
    let leftover = waiting > 0 || !attention.is_empty();
    status::UpdateStatus::new(now, waiting, attention.len(), if leftover { release.clone() } else { None })
}

fn defer_text(d: &DeferReason) -> String {
    match d {
        DeferReason::PlayersOnline(n) => format!("{n} player(s) online"),
        DeferReason::PlayerCountUnknown => "player count unavailable".into(),
        DeferReason::OutsideWindow => "outside quiet window".into(),
    }
}

/// Production: keep each resource's pinned commit in `kora-server.json` current.
fn record_commits(server: &ServerRecord, moved: &[(String, String)]) -> std::result::Result<(), String> {
    let mut sf = server.clone();
    for r in &mut sf.resources {
        if let Some((_, c)) = moved.iter().find(|(id, _)| *id == r.id) {
            r.commit = Some(c.clone());
        }
    }
    sf.save().map_err(|e| format!("updated, but could not record the new versions: {e}"))
}

// ---------------------------------------------------------------- snapshots

/// Saved snapshots, newest first.
pub fn snapshots(server: &ServerRecord) -> Vec<SnapshotInfo> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(snapshot_dir(server)) else { return out };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let (Some(id), Ok(s)) = (p.file_stem().and_then(|x| x.to_str()), Snapshot::load(&p)) else { continue };
        out.push(SnapshotInfo { id: id.to_string(), created_unix: s.created_unix, resources: s.entries.into_iter().map(|e| e.id).collect() });
    }
    out.sort_by(|a, b| b.created_unix.cmp(&a.created_unix).then_with(|| b.id.cmp(&a.id)));
    out
}

/// Put every resource in the snapshot back on its previous commit, and restart if the server runs.
pub fn rollback(server: &ServerRecord, snapshot_id: &str) -> Result<()> {
    with_git(&server.dir, |g| rollback_with(server, snapshot_id, g))?;
    let health = service_health(&server.name, &server.dir, server);
    health.recover(&HealthRequest { full_restart: true, updated: Vec::new() });
    Ok(())
}

/// [`rollback`] with injected git (no restart).
pub fn rollback_with(server: &ServerRecord, snapshot_id: &str, git: &dyn GitOps) -> Result<()> {
    if !valid_snapshot_id(snapshot_id) {
        return err(format!("'{snapshot_id}' is not a snapshot id (see --list-snapshots)"));
    }
    let file = snapshot_dir(server).join(format!("{snapshot_id}.json"));
    if !file.is_file() {
        return err(format!("no snapshot '{snapshot_id}' for this server (see --list-snapshots)"));
    }
    let snap = Snapshot::load(&file).map_err(|e| InstallError(e.to_string()))?;
    let results = rollback_snapshot(git, &snap);
    let failed: Vec<String> = results.iter().filter_map(|(id, r)| r.as_ref().err().map(|e| format!("{id}: {e}"))).collect();
    if server.mode == Mode::Production {
        let restored: Vec<(String, String)> =
            snap.entries.iter().filter(|e| results.iter().any(|(id, r)| *id == e.id && r.is_ok())).map(|e| (e.id.clone(), e.commit.clone())).collect();
        if let Err(m) = record_commits(server, &restored) {
            return err(m);
        }
    }
    if failed.is_empty() {
        Ok(())
    } else {
        err(format!("could not roll back everything: {}", failed.join("; ")))
    }
}

// ---------------------------------------------------------------- text

fn short(c: &str) -> &str {
    &c[..c.len().min(7)]
}

/// Plain-language preview for the terminal.
pub fn render_preview(p: &UpdatePreview) -> String {
    let mut s = String::new();
    if let Some(v) = &p.new_release {
        s += &format!("A new Kora release is available: {v}\n");
    }
    let n = p.resources.iter().filter(|c| c.status == ChangeStatus::Clean).count();
    if n == 0 {
        s += "Nothing to update.\n";
    }
    for c in &p.resources {
        match c.status {
            ChangeStatus::UpToDate => {}
            ChangeStatus::Clean => {
                let restart = if c.needs_restart { " (needs a server restart)" } else { "" };
                s += &format!("{}: {} -> {}, {} new change(s){restart}\n", c.id, short(&c.from), short(&c.to), c.behind);
                for l in &c.changelog {
                    s += &format!("    - {l}\n");
                }
            }
            ChangeStatus::Dirty => s += &format!("{}: skipped, it has uncommitted changes of your own\n", c.id),
            ChangeStatus::Diverged => {
                let why = c.changelog.first().cloned().unwrap_or_else(|| "it has changes of your own as well as new upstream ones".into());
                s += &format!("{}: skipped, {why}\n", c.id);
            }
        }
    }
    s
}

/// Plain-language result for the terminal.
pub fn render_report(r: &UpdateReport) -> String {
    let mut s = r.summary();
    for id in &r.sql_flagged {
        s += &format!("  {id} includes a database migration: it is never rolled back automatically.\n");
    }
    s
}

#[cfg(test)]
mod tests;
