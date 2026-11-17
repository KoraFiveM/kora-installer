//! Manual update flow: preview, snapshot, apply, rollback; single resources or everything.
//!
//! Development servers fetch `upstream` and fast-forward only (never merge; dirty or diverged
//! forks are skipped with a plain reason, exactly like `--auto`). Production servers move each
//! resource to the PINNED commit of the catalog's `latest` release (always a commit, never a tag).
//! The engine underneath is the one `kora update --auto` uses ([`super::plan`] and
//! [`super::run_update`]); this module adds scope selection, the restart policy, snapshot ids and
//! events, and keeps the pinned commits in `kora-server.json` current.

use std::collections::BTreeMap;
use std::path::Path;

use super::lock;
use super::orgcache::{self, OrgHints};
use super::status;
use super::{
    decide_restart, plan_fetch, plan_fetch_with, run_update_with, Action, DeferReason, FetchMode, GitOps, GitResult, HealthCheck, HealthOutcome, HealthRequest, NoMigrations, PlanItem, QuietConfig,
    ResourceClass, RestartDecision, RunEvent, RunOptions, SkipReason, Snapshot, UpdateError, UpdatePlan, UpdateReport, UpdateTarget,
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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    All,
    Resources(Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeStatus {
    UpToDate,
    /// Can be updated.
    Clean,
    /// Uncommitted local changes: skipped.
    Dirty,
    /// Local commits and new upstream commits (or could not be checked): skipped.
    Diverged,
    /// Put on another branch or a pull request on purpose: left alone (see [`ResourceChange::note`]). Not a problem.
    OtherBranch,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResourceChange {
    pub id: String,
    pub from: String,
    pub to: String,
    pub behind: u32,
    /// Commit subjects between old and new, newest first, at most ~20.
    pub changelog: Vec<String>,
    pub needs_restart: bool,
    pub status: ChangeStatus,
    /// Plain sentence about this resource when it is not a normal update ("trying pull request #12; not updated").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct UpdatePreview {
    pub resources: Vec<ResourceChange>,
    /// Production: the newest release version the catalog offers, when it is newer than what is installed.
    pub new_release: Option<String>,
    /// Warnings about the check as a whole: the built-in release list was used, resources no longer in the catalog.
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ApplyOpts {
    pub restart: RestartPolicy,
    /// Resource ids to leave alone.
    pub skip: Vec<String>,
    /// The preview's planned target per resource id (`ResourceChange::to`). When given, only these
    /// resources are updated, and exactly to these commits, even if upstream has moved on since.
    #[serde(default)]
    pub targets: Option<BTreeMap<String, String>>,
}

impl ApplyOpts {
    pub fn new(restart: RestartPolicy) -> Self {
        ApplyOpts { restart, skip: Vec::new(), targets: None }
    }
}

/// The planned targets of a preview: every resource that is ready to update, with the commit it moves to.
pub fn planned_targets(p: &UpdatePreview) -> BTreeMap<String, String> {
    p.resources.iter().filter(|c| c.status == ChangeStatus::Clean).map(|c| (c.id.clone(), c.to.clone())).collect()
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateEvent {
    Resource {
        id: String,
        state: String,
    },
    /// A snapshot with this id was saved.
    Snapshot(String),
    Health(String),
    /// Something slow before the update itself (e.g. "Syncing your GitHub forks 3/14").
    Progress(String),
    /// A check: `done` of `total` resources have been looked at (`id` is the one just finished).
    /// `done` 0 with an empty `id` means the checks are starting (the network part is one batch).
    Checked {
        done: usize,
        total: usize,
        id: String,
    },
    /// The resources an update is about to try, in order (the total for the progress bar).
    Planned(Vec<String>),
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

fn snapshot_dir(server: &ServerRecord) -> std::path::PathBuf {
    super::snapshot_dir(&server.dir)
}

/// Take the server's update lock (see [`lock`]); busy is a plain error.
fn locked(server: &ServerRecord) -> Result<lock::UpdateLock> {
    lock::acquire(&server.dir).map_err(|e| InstallError(e.to_string()))
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
    fn fetch_many(&self, targets: &[&UpdateTarget], mode: FetchMode) -> Vec<GitResult<()>> {
        // A failed fetch is not an error when the pinned commit is already here; otherwise nothing can be compared.
        self.0
            .fetch_many(targets, mode)
            .into_iter()
            .zip(targets)
            .map(|(r, t)| match r {
                Err(e) if !self.0.has_ref(&t.path, &t.upstream_ref) => {
                    Err(UpdateError::Git(format!("couldn't check (offline?): the release could not be downloaded ({})", e.to_string().trim_start_matches("git error: "))))
                }
                _ => Ok(()),
            })
            .collect()
    }
    fn has_ref(&self, repo: &Path, rev: &str) -> bool {
        self.0.has_ref(repo, rev)
    }
    fn remote_slug(&self, repo: &Path, remote: &str) -> Option<String> {
        self.0.remote_slug(repo, remote)
    }
    fn head_commit(&self, repo: &Path, rev: &str) -> GitResult<String> {
        if rev == "HEAD" {
            self.0.head_commit(repo, rev)
        } else {
            Ok(rev.to_string())
        }
    }
    fn fast_forward(&self, repo: &Path, commit: &str) -> GitResult<()> {
        // Never a downgrade and never away from commits of the user's own (only a rollback goes back).
        if !self.0.is_ancestor(repo, "HEAD", commit)? {
            return Err(UpdateError::Git("the release does not contain what is checked out now; left as it is".into()));
        }
        self.0.move_pinned(repo, commit)
    }
    fn reset_to(&self, repo: &Path, commit: &str) -> GitResult<()> {
        self.0.reset_to(repo, commit)
    }
    fn is_dirty(&self, repo: &Path) -> GitResult<bool> {
        self.0.is_dirty(repo)
    }
    /// "Ahead" = what is checked out is not part of the release: a commit of the user's own on the
    /// detached checkout, or a newer release than the target. Moving would leave it behind.
    fn is_diverged(&self, repo: &Path, commit: &str) -> GitResult<bool> {
        Ok(!self.0.is_ancestor(repo, "HEAD", commit)?)
    }
    fn is_ancestor(&self, repo: &Path, a: &str, b: &str) -> GitResult<bool> {
        self.0.is_ancestor(repo, a, b)
    }
    fn behind_count(&self, repo: &Path, commit: &str) -> GitResult<u32> {
        if self.0.head_commit(repo, "HEAD")? == commit {
            return Ok(0);
        }
        self.0.behind_count(repo, commit)
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
    /// Production: resources the catalog no longer lists (left as they are).
    gone: Vec<String>,
    /// Resources that cannot be updated as asked, with the reason (shown as "could not check").
    rejected: Vec<(UpdateTarget, String)>,
    /// Development: `(id, upstream/<branch>)` of targets moved to a previewed commit instead of the branch.
    planned_dev: Vec<(String, String)>,
}

fn is_commit_id(c: &str) -> bool {
    (7..=64).contains(&c.len()) && c.chars().all(|x| x.is_ascii_hexdigit())
}

fn select(server: &ServerRecord, scope: &Scope, skip: &[String], catalog: Option<&Catalog>, planned: Option<&BTreeMap<String, String>>) -> Result<Selection> {
    let wanted: Option<Vec<String>> = match scope {
        Scope::All => None,
        Scope::Resources(ids) => Some(ids.iter().map(|i| normalize_id(i)).collect()),
    };
    if let Some(w) = &wanted {
        if let Some(bad) = w.iter().find(|id| !server.resources.iter().any(|r| &normalize_id(&r.id) == *id)) {
            return err(format!("this server has no resource named '{bad}'"));
        }
        if let Some(paid) = server.resources.iter().find(|r| r.is_portal() && w.contains(&normalize_id(&r.id))) {
            return err(format!("'{}' is a paid asset from the Cfx.re Portal; to update it, download the newer zip and import it again", paid.id));
        }
    }
    let skip: Vec<String> = skip.iter().map(|s| normalize_id(s)).collect();
    let planned: Option<BTreeMap<String, String>> = planned.map(|m| m.iter().map(|(k, v)| (normalize_id(k), v.trim().to_ascii_lowercase())).collect());
    let prod = server.mode == Mode::Production;
    let cat = if prod { Some(catalog.ok_or_else(|| InstallError("could not read the release catalog, so releases cannot be compared".into()))?) } else { None };
    let mut sel = Selection { targets: Vec::new(), versions: Vec::new(), gone: Vec::new(), rejected: Vec::new(), planned_dev: Vec::new() };
    for r in &server.resources {
        let n = normalize_id(&r.id);
        if wanted.as_ref().is_some_and(|w| !w.contains(&n)) || skip.contains(&n) {
            continue;
        }
        // Paid (Cfx.re Portal) assets are zip imports, not git clones: replaced by importing a newer zip.
        if r.is_portal() {
            continue;
        }
        // An apply of a preview touches only what the preview offered.
        let plan_to = match &planned {
            Some(m) => match m.get(&n) {
                Some(c) => Some(c.clone()),
                None => continue,
            },
            None => None,
        };
        // Development tracks forks; third-party (bridge / creator) clones are pinned and have no upstream.
        if !prod && r.third_party {
            continue;
        }
        let path = server.dir.join("resources").join(&r.path);
        // A copy of one subfolder of a repository has no git history to compare: left as installed.
        if crate::installer::pinned::is_subpath_copy(&path) {
            continue;
        }
        let mut t = UpdateTarget { id: r.id.clone(), path, upstream_ref: String::new(), class: ResourceClass::from_id(&r.id) };
        if let Some(c) = &plan_to {
            if !is_commit_id(c) {
                sel.rejected.push((t, format!("'{c}' is not a commit id")));
                continue;
            }
        }
        match cat {
            None => {
                let branch_ref = format!("upstream/{}", r.branch);
                t.upstream_ref = match plan_to {
                    Some(c) => {
                        sel.planned_dev.push((r.id.clone(), branch_ref));
                        c
                    }
                    None => branch_ref,
                };
            }
            Some(c) => {
                let Some(res) = c.find_normalized(&r.id) else {
                    sel.gone.push(r.id.clone());
                    continue;
                };
                let rel = match &plan_to {
                    // Exactly the release that was previewed, as long as the catalog still lists it.
                    Some(p) => match res.versions.iter().find(|v| v.commit.eq_ignore_ascii_case(p)) {
                        Some(v) => v,
                        None => {
                            sel.rejected.push((t, "the version you checked is no longer in the release catalog; check again".into()));
                            continue;
                        }
                    },
                    None => match res.latest_release() {
                        Some(v) => v,
                        // No release is published for it: nothing to move to.
                        None => continue,
                    },
                };
                sel.versions.push((r.id.clone(), rel.version.clone()));
                t.upstream_ref = rel.commit.clone();
            }
        }
        sel.targets.push(t);
    }
    Ok(sel)
}

/// Notes for the person: the catalog the check used, and resources the catalog dropped.
fn selection_notes(sel: &Selection, catalog_note: Option<&str>) -> Vec<String> {
    let mut notes: Vec<String> = catalog_note.map(str::to_string).into_iter().collect();
    notes.extend(sel.gone.iter().map(|id| format!("{id}: no longer in the catalog (left as is)")));
    notes
}

/// The plan, plus the resources [`select`] refused (as "could not check") and, for previewed
/// development targets, a check that the previewed commit is still on the original branch.
fn finish_plan(g: &dyn GitOps, sel: &Selection, mut p: UpdatePlan) -> UpdatePlan {
    for item in &mut p.items {
        let Some((_, branch_ref)) = sel.planned_dev.iter().find(|(id, _)| *id == item.target.id) else {
            continue;
        };
        if !matches!(item.action, Action::FastForward | Action::NeedsRestart) {
            continue;
        }
        // The person moved it to another branch after the preview: never update it behind their back.
        if let Some(cur) = super::off_branch(g, &item.target.path, branch_ref) {
            item.action = Action::Skipped(SkipReason::OtherBranch(cur));
            continue;
        }
        if !g.is_ancestor(&item.target.path, &item.target.upstream_ref, branch_ref).unwrap_or(false) {
            item.action = Action::Skipped(SkipReason::Error("the version you checked is no longer on the original repository; check again".into()));
        }
    }
    for (t, why) in &sel.rejected {
        p.items.push(PlanItem { target: t.clone(), current: String::new(), upstream: String::new(), behind: 0, action: Action::Skipped(SkipReason::Error(why.clone())) });
    }
    p
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
    let mut note = None;
    let status = match &item.action {
        Action::Skipped(r @ SkipReason::OtherBranch(_)) => {
            note = Some(r.text());
            ChangeStatus::OtherBranch
        }
        Action::UpToDate | Action::Skipped(SkipReason::LocalCommits) => ChangeStatus::UpToDate,
        Action::FastForward | Action::NeedsRestart => ChangeStatus::Clean,
        Action::Skipped(SkipReason::Dirty) => ChangeStatus::Dirty,
        Action::Skipped(SkipReason::Diverged) => ChangeStatus::Diverged,
        Action::Skipped(SkipReason::Error(e)) => {
            changelog.push(format!("could not check: {}", e.trim_start_matches("git error: ")));
            ChangeStatus::Diverged
        }
    };
    if item.behind > 0 && !matches!(status, ChangeStatus::UpToDate) {
        changelog = git.log_subjects(&item.target.path, &item.current, &item.upstream, CHANGELOG_MAX);
    }
    ResourceChange {
        id: item.target.id.clone(),
        from: item.current.clone(),
        to: if matches!(status, ChangeStatus::UpToDate | ChangeStatus::OtherBranch) { item.current.clone() } else { item.upstream.clone() },
        behind: item.behind,
        changelog,
        needs_restart: item.target.class.needs_restart(),
        status,
        note,
    }
}

/// Fetch and compare. Changes nothing in any checkout. Holds the server's update lock while it runs
/// git (a fetch must not race an update): busy is an error.
pub fn preview(server: &ServerRecord, scope: Scope) -> Result<UpdatePreview> {
    preview_events(server, scope, &mut |_| {})
}

/// [`preview`] that reports each resource as it is checked ([`UpdateEvent::Checked`]).
pub fn preview_events(server: &ServerRecord, scope: Scope, sink: &mut dyn FnMut(UpdateEvent)) -> Result<UpdatePreview> {
    if lock::is_busy(&server.dir) {
        return err(lock::LockError::Busy.to_string());
    }
    let (cat, note) = load_catalog_if_needed(server)?;
    // One organisation-level request tells which repositories moved (whole-server checks only: a single resource is one fetch).
    let mut hints = org_hints_for(server, &scope);
    with_git(&server.dir, |g| preview_checked(server, scope, g, cat.as_ref(), note.as_deref(), hints.as_mut(), sink))
}

/// The organisation hint for a whole-server check of a development server (None otherwise, or when `KORA_NO_ORG_HINT` is set).
fn org_hints_for(server: &ServerRecord, scope: &Scope) -> Option<OrgHints> {
    (server.mode == Mode::Development && *scope == Scope::All && std::env::var_os("KORA_NO_ORG_HINT").is_none()).then(|| OrgHints::load(&orgcache::HttpOrg, Some(&orgcache::cache_path()), now_unix()))
}

/// Ids of the targets whose remote has not moved since this clone was last fetched (per the organisation), plus the
/// `owner/name` of every KoraFiveM target so the ones that do get fetched can be recorded.
fn hint_skips(g: &dyn GitOps, hints: Option<&OrgHints>, targets: &[UpdateTarget]) -> (Vec<String>, Vec<(String, String)>) {
    let mut skip = Vec::new();
    let mut slugs = Vec::new();
    let Some(h) = hints.filter(|h| h.is_live()) else { return (skip, slugs) };
    for t in targets.iter().filter(|t| t.upstream_ref.starts_with("upstream/")) {
        let Some(slug) = g.remote_slug(&t.path, "upstream") else { continue };
        if !slug.to_ascii_lowercase().starts_with(&format!("{}/", orgcache::ORG.to_ascii_lowercase())) {
            continue;
        }
        // Only when the remote-tracking branch is really here: otherwise the plan could not even compare.
        if h.is_current(&t.path, &slug) && g.has_ref(&t.path, &t.upstream_ref) {
            skip.push(t.id.clone());
        }
        slugs.push((t.id.clone(), slug));
    }
    (skip, slugs)
}

fn record_hints(hints: Option<&mut OrgHints>, targets: &[UpdateTarget], slugs: &[(String, String)], fetched: &[String], skipped: &[String]) {
    let Some(h) = hints.filter(|h| h.is_live()) else { return };
    for t in targets {
        let Some((_, slug)) = slugs.iter().find(|(id, _)| *id == t.id) else { continue };
        if fetched.contains(&t.id) {
            h.mark_fetched(&t.path, slug);
        } else if !skipped.contains(&t.id) {
            // The fetch failed: nothing is known about this clone any more.
            h.forget(&t.path, slug);
        }
    }
    h.save();
}

/// The catalog a production server compares against, and a warning when it is the built-in copy.
fn load_catalog_if_needed(server: &ServerRecord) -> Result<(Option<Catalog>, Option<String>)> {
    if server.mode != Mode::Production {
        return Ok((None, None));
    }
    let (c, origin) = catalog::load(&HttpClient::default(), catalog::CATALOG_URL).map_err(|e| InstallError(e.to_string()))?;
    Ok((Some(c), catalog_note(&origin)))
}

/// The warning for a catalog that did not come from the internet (None for the live one).
pub fn catalog_note(origin: &catalog::CatalogOrigin) -> Option<String> {
    match origin {
        catalog::CatalogOrigin::Remote => None,
        catalog::CatalogOrigin::Embedded(why) => {
            Some(format!("the online release list could not be read ({why}), so the list built into this version of Kora was used: newer releases may be missing"))
        }
    }
}

/// [`preview`] with injected git and catalog.
pub fn preview_with(server: &ServerRecord, scope: Scope, git: &dyn GitOps, catalog: Option<&Catalog>) -> Result<UpdatePreview> {
    preview_with_hints(server, scope, git, catalog, None, None)
}

/// [`preview_with`] with a catalog warning and an organisation hint (None: every resource is fetched, in parallel).
pub fn preview_with_hints(server: &ServerRecord, scope: Scope, git: &dyn GitOps, catalog: Option<&Catalog>, catalog_note: Option<&str>, hints: Option<&mut OrgHints>) -> Result<UpdatePreview> {
    preview_checked(server, scope, git, catalog, catalog_note, hints, &mut |_| {})
}

fn preview_checked(
    server: &ServerRecord,
    scope: Scope,
    git: &dyn GitOps,
    catalog: Option<&Catalog>,
    catalog_note: Option<&str>,
    hints: Option<&mut OrgHints>,
    sink: &mut dyn FnMut(UpdateEvent),
) -> Result<UpdatePreview> {
    let _lock = locked(server)?;
    let sel = select(server, &scope, &[], catalog, None)?;
    let pinned = PinnedGit(git);
    let g = git_view(server, git, &pinned);
    let (skip, slugs) = hint_skips(g, hints.as_deref(), &sel.targets);
    let (p, fetched) = plan_fetch_with(g, &sel.targets, FetchMode::Fresh, &skip, &mut |done, total, id| sink(UpdateEvent::Checked { done, total, id: id.to_string() }));
    record_hints(hints, &sel.targets, &slugs, &fetched, &skip);
    let p = finish_plan(g, &sel, p);
    let resources: Vec<ResourceChange> = p.items.iter().map(|i| change_of(g, i)).collect();
    let pre = UpdatePreview { new_release: new_release(&resources, &sel.versions), resources, notes: selection_notes(&sel, catalog_note) };
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
    let moving: Vec<&ResourceChange> = changes.iter().filter(|c| !matches!(c.status, ChangeStatus::UpToDate | ChangeStatus::OtherBranch)).collect();
    let pick = moving.iter().find(|c| normalize_id(&c.id) == "kora-core").or_else(|| moving.first())?;
    versions.iter().find(|(id, _)| *id == pick.id).map(|(_, v)| v.clone())
}

// ---------------------------------------------------------------- apply

/// Wraps the real health check to honour the chosen restart policy.
struct PolicyHealth<'a> {
    inner: &'a dyn HealthCheck,
    policy: RestartPolicy,
}

impl PolicyHealth<'_> {
    /// The request as the chosen policy allows it; `None` = do not touch the server at all.
    fn shape(&self, req: &HealthRequest) -> Option<HealthRequest> {
        match self.policy {
            RestartPolicy::None => None,
            RestartPolicy::Ensure => Some(HealthRequest { full_restart: false, ..req.clone() }),
            RestartPolicy::FullRestart => Some(HealthRequest { full_restart: true, ..req.clone() }),
            // Restart-needing resources only got this far if the server was quiet (else they were deferred).
            RestartPolicy::WhenQuiet => Some(req.clone()),
        }
    }
}

impl HealthCheck for PolicyHealth<'_> {
    fn check(&self, req: &HealthRequest) -> HealthOutcome {
        match self.shape(req) {
            None => HealthOutcome::Healthy,
            Some(r) => self.inner.check(&r),
        }
    }
    /// Same rules as `check`: never a restart the policy would not have made.
    fn recover(&self, req: &HealthRequest) {
        if let Some(r) = self.shape(req) {
            self.inner.recover(&r);
        }
    }
}

/// Snapshot, apply, restart per policy, health check, auto-rollback on failure.
pub fn apply(server: &ServerRecord, scope: Scope, opts: ApplyOpts, sink: &mut dyn FnMut(UpdateEvent)) -> Result<UpdateReport> {
    if lock::is_busy(&server.dir) {
        return err(lock::LockError::Busy.to_string());
    }
    let (cat, note) = load_catalog_if_needed(server)?;
    let health = service_health(&server.name, &server.dir, server);
    // Asked after the fork sync (minutes on a big server), right before deciding about a restart.
    let players = || {
        if health.running() {
            online_players(server.ports.game)
        } else {
            Some(0)
        }
    };
    let mut hints = org_hints_for(server, &scope);
    let report = with_git(&server.dir, |g| apply_with_hints(server, scope, opts, g, &health, cat.as_ref(), note.as_deref(), &players, now_unix(), sink, hints.as_mut()))?;
    // Same Discord notification as the automatic path (it never fails the update). Sent here only.
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
    apply_with_hints(server, scope, opts, git, health, catalog, None, &|| players, now, sink, None)
}

/// [`apply_with`] with a catalog warning, a player count asked when needed, and an organisation hint.
/// Holds the server's update lock for the whole run. Fetches the preview just made are not repeated.
#[allow(clippy::too_many_arguments)]
pub fn apply_with_hints(
    server: &ServerRecord,
    scope: Scope,
    opts: ApplyOpts,
    git: &dyn GitOps,
    health: &dyn HealthCheck,
    catalog: Option<&Catalog>,
    catalog_note: Option<&str>,
    players: &dyn Fn() -> Option<u32>,
    now: u64,
    sink: &mut dyn FnMut(UpdateEvent),
    hints: Option<&mut OrgHints>,
) -> Result<UpdateReport> {
    let _lock = locked(server)?;
    let sel = select(server, &scope, &opts.skip, catalog, opts.targets.as_ref())?;
    let pinned = PinnedGit(git);
    let g = git_view(server, git, &pinned);
    // Development: fast-forward the user's forks on GitHub first (never forced); production has no forks.
    let fork_sync = if server.mode == Mode::Development {
        // Planned targets are commits: the forks are found from the branch the resource tracks.
        let fork_targets: Vec<UpdateTarget> = sel
            .targets
            .iter()
            .map(|t| match sel.planned_dev.iter().find(|(id, _)| *id == t.id) {
                Some((_, b)) => UpdateTarget { upstream_ref: b.clone(), ..t.clone() },
                None => t.clone(),
            })
            .collect();
        g.sync_forks(&fork_targets, &mut |done, total| sink(UpdateEvent::Progress(format!("Syncing your GitHub forks {done}/{total}"))))
    } else {
        Vec::new()
    };
    let (skip, slugs) = hint_skips(g, hints.as_deref(), &sel.targets);
    let (the_plan, fetched) = plan_fetch(g, &sel.targets, FetchMode::AllowRecent, &skip);
    record_hints(hints, &sel.targets, &slugs, &fetched, &skip);
    let the_plan = finish_plan(g, &sel, the_plan);
    let restart = match opts.restart {
        RestartPolicy::WhenQuiet => decide_restart(players(), 0, &QuietConfig { window: None, max_players: 0 }),
        _ => RestartDecision::Proceed,
    };
    let will_apply: Vec<&str> = the_plan.pending().filter(|i| i.action == Action::FastForward || restart == RestartDecision::Proceed).map(|i| i.target.id.as_str()).collect();

    let pending_release = new_release(&the_plan.items.iter().map(|i| change_of(g, i)).collect::<Vec<_>>(), &sel.versions);
    let snaps = snapshot_dir(server);
    let id = super::new_snapshot_id(&snaps, now);
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
            RunEvent::Planned(ids) => sink(UpdateEvent::Planned(ids)),
            RunEvent::Skipped(id, state) => sink(UpdateEvent::Resource { id, state }),
        };
        run_update_with(g, &ph, &NoMigrations, &the_plan, &run, &mut on).map_err(|e| InstallError(e.to_string()))?
    };

    super::add_fork_sync(&mut report, fork_sync);
    let changed: Vec<String> = report.updated.iter().map(|u| u.id.clone()).collect();
    crate::resourcestate::mark_changed(&server.dir, &changed, "updated");
    report.notes = selection_notes(&sel, catalog_note);
    if server.mode == Mode::Production && !report.updated.is_empty() {
        let moved: Vec<(String, String)> = report.updated.iter().map(|u| (u.id.clone(), u.to.clone())).collect();
        if let Err(m) = record_commits(server, &moved) {
            report.needs_attention.push(("kora-server.json".into(), m));
        }
    }
    // A version that had to be rolled back is not retried by the nightly update; one that worked is released.
    super::record_holds(&server.dir, &the_plan, &report);
    super::prune_snapshots(&server.dir, super::KEEP_SNAPSHOTS);

    for (id, why) in &report.skipped {
        sink(UpdateEvent::Resource { id: id.clone(), state: format!("skipped: {}", why.text()) });
    }
    for (id, why) in &report.deferred {
        sink(UpdateEvent::Resource { id: id.clone(), state: format!("waiting: {}", defer_text(why)) });
    }
    for (id, why) in &report.rolled_back {
        sink(UpdateEvent::Resource { id: id.clone(), state: format!("rolled back: {why}") });
    }
    for id in &report.not_attempted {
        sink(UpdateEvent::Resource { id: id.clone(), state: "not attempted: the update stopped before it got here".into() });
    }
    if opts.restart == RestartPolicy::Ensure && !report.updated.is_empty() {
        sink(UpdateEvent::Health(ENSURE_NOTE.into()));
    }
    if !will_apply.is_empty() {
        sink(UpdateEvent::Health(if !report.rolled_back.is_empty() {
            "server did not come back healthy; went back to the snapshot".into()
        } else if !report.not_attempted.is_empty() || report.needs_attention.iter().any(|(_, m)| m.starts_with("fast-forward failed")) {
            "the update stopped part way; what had changed was put back".into()
        } else {
            "server is healthy".into()
        }));
    }
    // The status file is about the whole server: after a partial apply the counts are worked out
    // again from what is on disk (no network).
    let st = if scope == Scope::All { apply_status(&report, now, &pending_release) } else { local_status(server, g, catalog, &report, now) };
    status::record(&server.dir, &st);
    sink(UpdateEvent::Done);
    Ok(report)
}

/// Status after a run: what is still waiting, what needs a person, and the release to announce while something is left.
pub fn apply_status(report: &UpdateReport, now: u64, release: &Option<String>) -> status::UpdateStatus {
    let mut attention: Vec<&str> =
        report.needs_attention.iter().map(|(i, _)| i.as_str()).chain(report.rolled_back.iter().map(|(i, _)| i.as_str())).chain(report.held.iter().map(|(i, _)| i.as_str())).collect();
    attention.sort_unstable();
    attention.dedup();
    let waiting = report.deferred.len() + report.not_attempted.len();
    let leftover = waiting > 0 || !attention.is_empty();
    status::UpdateStatus::new(now, waiting, attention.len(), if leftover { release.clone() } else { None })
}

/// Whole-server status from the local checkouts only (no fetch), plus what `report` says needs a person.
fn local_status(server: &ServerRecord, g: &dyn GitOps, catalog: Option<&Catalog>, report: &UpdateReport, now: u64) -> status::UpdateStatus {
    let Ok(all) = select(server, &Scope::All, &[], catalog, None) else {
        return apply_status(report, now, &None);
    };
    let ids: Vec<String> = all.targets.iter().map(|t| t.id.clone()).collect();
    let (p, _) = plan_fetch(g, &all.targets, FetchMode::AllowRecent, &ids);
    let ready = p.pending().count();
    // "Could not check" (e.g. never fetched) is not known to need anyone: not counted.
    let mut attention: Vec<String> = p
        .items
        .iter()
        .filter(|i| matches!(i.action, Action::Skipped(SkipReason::Dirty | SkipReason::Diverged)))
        .map(|i| i.target.id.clone())
        .chain(report.needs_attention.iter().map(|(i, _)| i.clone()))
        .chain(report.rolled_back.iter().map(|(i, _)| i.clone()))
        .collect();
    attention.sort_unstable();
    attention.dedup();
    let release = (ready > 0 || !attention.is_empty()).then(|| new_release(&p.items.iter().map(|i| change_of(g, i)).collect::<Vec<_>>(), &all.versions)).flatten();
    status::UpdateStatus::new(now, ready, attention.len(), release)
}

fn defer_text(d: &DeferReason) -> String {
    match d {
        DeferReason::PlayersOnline(n) => format!("{n} player(s) online"),
        DeferReason::PlayerCountUnknown => "player count unavailable".into(),
        DeferReason::OutsideWindow => "outside quiet window".into(),
    }
}

/// Production: keep each resource's pinned commit in `kora-server.json` current. The file is read
/// again here (the caller holds the update lock), so nothing another tool wrote meanwhile is lost.
fn record_commits(server: &ServerRecord, moved: &[(String, String)]) -> std::result::Result<(), String> {
    let mut sf = ServerFile::load(&server.dir).map_err(|e| format!("updated, but could not read kora-server.json to record the new versions: {e}"))?;
    sf.dir = server.dir.clone();
    for r in &mut sf.resources {
        if let Some((_, c)) = moved.iter().find(|(id, _)| *id == r.id) {
            r.commit = Some(c.clone());
        }
    }
    sf.save().map_err(|e| format!("updated, but could not record the new versions: {e}"))
}

// ---------------------------------------------------------------- snapshots

/// Saved snapshots (manual and automatic updates), newest first.
pub fn snapshots(server: &ServerRecord) -> Vec<SnapshotInfo> {
    super::list_snapshots(&server.dir).into_iter().map(|(id, s)| SnapshotInfo { id, created_unix: s.created_unix, resources: s.entries.into_iter().map(|e| e.id).collect() }).collect()
}

/// Put every resource in the snapshot back on its previous commit, and restart if the server runs.
/// Refuses when a resource has uncommitted changes (a rollback would throw them away) unless `force`.
pub fn rollback(server: &ServerRecord, snapshot_id: &str, force: bool) -> Result<()> {
    let _lock = locked(server)?;
    with_git(&server.dir, |g| rollback_locked(server, snapshot_id, g, force))?;
    let health = service_health(&server.name, &server.dir, server);
    health.recover(&HealthRequest { full_restart: true, updated: Vec::new() });
    Ok(())
}

/// [`rollback`] with injected git (no restart).
pub fn rollback_with(server: &ServerRecord, snapshot_id: &str, git: &dyn GitOps, force: bool) -> Result<()> {
    let _lock = locked(server)?;
    rollback_locked(server, snapshot_id, git, force)
}

fn rollback_locked(server: &ServerRecord, snapshot_id: &str, git: &dyn GitOps, force: bool) -> Result<()> {
    if !valid_snapshot_id(snapshot_id) {
        return err(format!("'{snapshot_id}' is not a snapshot id (see --list-snapshots)"));
    }
    let file = snapshot_dir(server).join(format!("{snapshot_id}.json"));
    if !file.is_file() {
        return err(format!("no snapshot '{snapshot_id}' for this server (see --list-snapshots)"));
    }
    let snap = Snapshot::load(&file).map_err(|e| InstallError(e.to_string()))?;
    if !force {
        let dirty: Vec<&str> = snap.entries.iter().filter(|e| git.is_dirty(&e.path).unwrap_or(false)).map(|e| e.id.as_str()).collect();
        if !dirty.is_empty() {
            return err(format!(
                "nothing was changed: {} {} changed files of your own that a rollback would throw away. Put your changes aside first (for example `git stash` in the resource folder), or roll back anyway, which throws them away (`--force` on the command line)",
                dirty.join(", "),
                if dirty.len() == 1 { "has" } else { "have" }
            ));
        }
    }
    // Where each resource is now: the version being undone, held back from the nightly update.
    let undone: Vec<(String, String)> = snap.entries.iter().filter_map(|e| git.head_commit(&e.path, "HEAD").ok().map(|c| (e.id.clone(), c))).collect();
    let results = super::rollback_snapshot(git, &snap);
    let failed: Vec<String> = results.iter().filter_map(|(id, r)| r.as_ref().err().map(|e| format!("{id}: {e}"))).collect();
    let restored: Vec<(String, String)> = snap.entries.iter().filter(|e| results.iter().any(|(id, r)| *id == e.id && r.is_ok())).map(|e| (e.id.clone(), e.commit.clone())).collect();
    hold_undone(&server.dir, &undone, &restored);
    let restored_ids: Vec<String> = restored.iter().map(|(id, _)| id.clone()).collect();
    crate::resourcestate::mark_changed(&server.dir, &restored_ids, "rolled back");
    if server.mode == Mode::Production {
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

/// After a rollback by hand: the versions that were undone are held from automatic updates too.
fn hold_undone(dir: &Path, undone: &[(String, String)], restored: &[(String, String)]) {
    let mut report = UpdateReport::default();
    let mut plan = UpdatePlan { items: Vec::new() };
    for (id, to) in restored {
        let Some((_, from)) = undone.iter().find(|(i, _)| i == id) else {
            continue;
        };
        if from == to {
            continue;
        }
        report.rolled_back.push((id.clone(), String::new()));
        plan.items.push(PlanItem {
            target: UpdateTarget { id: id.clone(), path: Default::default(), upstream_ref: String::new(), class: ResourceClass::Other },
            current: to.clone(),
            upstream: from.clone(),
            behind: 0,
            action: Action::UpToDate,
        });
    }
    super::record_holds(dir, &plan, &report);
}

// ---------------------------------------------------------------- text

fn short(c: &str) -> &str {
    &c[..c.len().min(7)]
}

/// Plain-language preview for the terminal.
pub fn render_preview(p: &UpdatePreview) -> String {
    let mut s = String::new();
    for n in &p.notes {
        s += &format!("Note: {n}\n");
    }
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
            ChangeStatus::OtherBranch => s += &format!("{}: {}\n", c.id, c.note.as_deref().unwrap_or("on another branch; not updated")),
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
