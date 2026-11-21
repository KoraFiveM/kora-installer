//! `[addresources]`: add catalog resources (compat, bridges, creators, or any other) to a server
//! that is already installed.
//!
//! * [`plan`] resolves the chosen ids with the same dependency rules as a fresh install
//!   ([`crate::catalog::Picker`]): dependencies come along transitively, what is installed is skipped.
//! * [`execute`] clones every item into its folder (pinned for production and for every bridge /
//!   creator resource; Kora resources of a development server come from the user's fork), records
//!   it in `kora-server.json` and appends the missing `ensure [folder]` lines to `server.cfg`.
//!
//! Existing servers are never rearranged: nothing already installed is moved, renamed or removed,
//! old layouts only gain the new-style folders they now need.

use std::path::{Path, PathBuf};

use crate::catalog::{self, Catalog, Picker};
use crate::git::{self, CommandRunner};
use crate::installer::registry::{ResourceRecord, ServerFile};
use crate::installer::{layout, pinned};
use crate::installer::{repo_slug, Result, Source, Versions};
use crate::problem::Problem;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PlanItem {
    pub id: String,
    /// Relative to `<server>/resources/`.
    pub rel: String,
    /// `owner/name` of the catalog repository.
    pub repo: String,
    pub branch: String,
    /// Pinned commit, or `None` for "fork / branch head".
    pub commit: Option<String>,
    /// Come from the user's fork (development server, Kora's own resource).
    pub fork: bool,
    pub third_party: bool,
    /// Install only this subfolder of the repository (a pinned copy with no `.git`).
    pub subpath: Option<String>,
    /// Pulled in by something else rather than asked for.
    pub dependency: bool,
    pub repository: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AddPlan {
    pub items: Vec<PlanItem>,
    /// Ids that were asked for and are already installed.
    pub already: Vec<String>,
    /// Folders `server.cfg` must ensure afterwards (only the ones these items live in).
    pub ensure: Vec<String>,
}

impl AddPlan {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// Ids installed on the server (as recorded).
pub fn installed_ids(server: &ServerFile) -> Vec<String> {
    server.resources.iter().map(|r| r.id.clone()).collect()
}

/// Load the live catalog from GitHub. It fails if it cannot be read: there is no bundled copy.
pub fn load_catalog() -> Result<(Catalog, Option<String>)> {
    let c = catalog::load(&crate::installer::HttpClient::default(), catalog::CATALOG_URL).map_err(|e| Problem::generic(format!("could not read the Kora release list from GitHub: {e}")))?;
    Ok((c, None))
}

/// What to install for `ids` (plus dependencies) on `server`.
pub fn plan(cat: &Catalog, server: &ServerFile, ids: &[String]) -> Result<AddPlan> {
    let picker = Picker::with_ids(cat, installed_ids(server), server.versions() == Versions::Latest, ids).map_err(|e| Problem::generic(e.to_string()))?;
    plan_from_picker(&picker, server, ids)
}

pub fn plan_from_picker(picker: &Picker<'_>, server: &ServerFile, asked: &[String]) -> Result<AddPlan> {
    let norm = catalog::normalize_id;
    let asked_n: Vec<String> = asked.iter().map(|a| norm(a)).collect();
    let items: Vec<PlanItem> = picker
        .resolved()
        .into_iter()
        .map(|r| {
            let third_party = r.is_pinned_copy();
            let pinned = server.versions() == Versions::Released || third_party;
            let commit = match (pinned, r.latest_release()) {
                (true, Some(rel)) => Some(rel.commit.clone()),
                _ => None,
            };
            PlanItem {
                id: r.id.clone(),
                rel: r.install_rel(),
                repo: repo_slug(&r.repository),
                branch: r.branch.clone(),
                commit,
                fork: server.source() == Source::Fork && !third_party,
                third_party,
                subpath: r.subpath.clone(),
                dependency: !asked_n.contains(&norm(&r.id)) && !picker.is_picked(&r.id),
                repository: r.repository.clone(),
            }
        })
        .collect();
    let already: Vec<String> = asked.iter().filter(|a| picker.is_installed(a)).cloned().collect();
    let ensure = layout::folders_for(items.iter().map(|i| i.rel.as_str()));
    Ok(AddPlan { items, already, ensure })
}

/// The git side of an install, so tests can run without a network.
pub trait Cloner {
    /// Make sure forks of `repos` (`owner/name`) exist under `owner` (development servers),
    /// reusing the forks already there whatever they are called. The returned state has the real
    /// fork of every repo ([`git::ForkState::fork_of`]) and how many were created or reused.
    fn prepare_forks(&self, owner: &str, repos: &[String]) -> std::result::Result<git::ForkState, Problem>;
    /// Clone `item` into `partial` (a fresh path that does not exist yet). `fork` is the
    /// `owner/name` of the user's fork for items that come from a fork.
    fn clone_item(&self, item: &PlanItem, fork: Option<&str>, partial: &Path) -> std::result::Result<(), Problem>;
}

pub struct GitCloner<'a>(pub &'a dyn CommandRunner);

impl GitCloner<'_> {
    fn forks_with(&self, owner: &str, repos: &[String], cfg: &git::ForkConfig) -> std::result::Result<git::ForkState, Problem> {
        let is_org = git::org_exists(self.0, owner).map_err(|e| e.to_string())?;
        let mut state = git::ForkState::new(owner, is_org, repos);
        let out = git::run_forks(self.0, &mut state, cfg, &mut |_| {}).map_err(|e| e.to_string())?;
        match out {
            git::ForkOutcome::Paused => Err("GitHub rate limit reached while forking. Try again later.".into()),
            git::ForkOutcome::Finished if !state.failed.is_empty() => Err(format!("could not fork {} ({})", state.failed[0].repo, state.failed[0].error).into()),
            git::ForkOutcome::Finished => Ok(state),
        }
    }
}

impl Cloner for GitCloner<'_> {
    fn prepare_forks(&self, owner: &str, repos: &[String]) -> std::result::Result<git::ForkState, Problem> {
        if repos.is_empty() {
            return Ok(git::ForkState::new(owner, false, &[]));
        }
        let cfg = git::ForkConfig { cache_path: Some(git::forkfind::default_cache_path()), ..git::ForkConfig::default() };
        self.forks_with(owner, repos, &cfg)
    }

    fn clone_item(&self, item: &PlanItem, fork: Option<&str>, partial: &Path) -> std::result::Result<(), Problem> {
        let res = match (item.fork, fork, &item.commit) {
            (true, Some(fork), _) => git::clone_from_fork(self.0, fork, &item.repo, partial, Some(&item.branch)),
            (true, None, _) => return Err("this development server has no GitHub fork recorded for this resource".into()),
            (false, _, Some(c)) => git::clone_pinned(self.0, &item.repo, c, partial),
            (false, _, None) => git::clone_default_head(self.0, &item.repo, partial),
        };
        res.map_err(Problem::from)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AddOutcome {
    pub installed: Vec<String>,
    pub failed: Vec<(String, String)>,
    pub ensured: Vec<String>,
    pub notes: Vec<String>,
}

impl AddOutcome {
    pub fn ok(&self) -> bool {
        self.failed.is_empty()
    }
}

/// `None` for a path that would leave `resources/` (the catalog is checked when loaded; this is
/// a second guard right where folders are created).
fn dest_of(server_dir: &Path, rel: &str) -> Option<PathBuf> {
    crate::fsutil::join_inside(&server_dir.join("resources"), rel)
}

/// A folder the new resource would land in is itself (inside) another resource's clone.
fn inside_another_clone(resources: &Path, dest: &Path) -> Option<PathBuf> {
    let mut p = dest.parent()?;
    while p.starts_with(resources) && p != resources {
        if p.join(".git").exists() {
            return Some(p.to_path_buf());
        }
        p = p.parent()?;
    }
    None
}

/// Clone everything in `plan`, record it and append the missing `ensure` lines.
/// Items already on disk with a `.git` are recorded but not touched. `say` receives progress lines.
pub fn execute(cloner: &dyn Cloner, server: &mut ServerFile, plan: &AddPlan, say: &mut dyn FnMut(&str)) -> Result<AddOutcome> {
    let mut out = AddOutcome::default();
    let resources = server.dir.join("resources");
    std::fs::create_dir_all(&resources).map_err(|e| Problem::generic(format!("cannot create the resources folder: {e}")))?;
    let owner = server.github_owner.clone();
    let fork_repos: Vec<String> = plan.items.iter().filter(|i| i.fork).map(|i| i.repo.clone()).collect();
    let mut forks: Option<git::ForkState> = None;
    if !fork_repos.is_empty() {
        let Some(o) = &owner else {
            return Err(Problem::generic("this development server has no GitHub account recorded, so its resources cannot be forked"));
        };
        say(&format!("Looking for your forks in {o} and forking what is missing (this can take a while)"));
        let st = cloner.prepare_forks(o, &fork_repos)?;
        let text = st.counts_text();
        say(&format!("Forks in {o}: {text}"));
        out.notes.push(format!("GitHub forks in {o}: {text}."));
        forks = Some(st);
    }
    let mut done_rels: Vec<String> = Vec::new();
    for item in &plan.items {
        say(&format!("Getting {}", item.id));
        let step = (|| -> std::result::Result<bool, Problem> {
            let dest = dest_of(&server.dir, &item.rel).ok_or_else(|| format!("unsafe folder '{}'", item.rel))?;
            let present = if item.subpath.is_some() { item.commit.as_deref().is_some_and(|c| pinned::marker_matches(&dest, c)) } else { dest.join(".git").exists() };
            if present {
                return Ok(false);
            }
            if let Some(parent) = inside_another_clone(&resources, &dest) {
                return Err(format!("its folder is inside the resource at {}", parent.display()).into());
            }
            if dest.exists() && std::fs::read_dir(&dest).map(|mut d| d.next().is_some()).unwrap_or(true) {
                return Err(format!("{} already exists and was not created by Kora; move it away first", dest.display()).into());
            }
            let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let partial = dest.with_file_name(format!("{name}.kora-partial"));
            let _ = std::fs::remove_dir_all(&partial);
            if let Some(p) = dest.parent() {
                std::fs::create_dir_all(p).map_err(|e| format!("cannot create the folder: {e}"))?;
            }
            if dest.exists() {
                let _ = std::fs::remove_dir(&dest); // an empty leftover
            }
            let fork = item.fork.then(|| forks.as_ref().map(|s| s.fork_of(&item.repo))).flatten();
            if let Err(e) = cloner.clone_item(item, fork.as_deref(), &partial) {
                let _ = std::fs::remove_dir_all(&partial);
                return Err(e);
            }
            pinned::place(&partial, &dest, item.subpath.as_deref(), &item.repo, item.commit.as_deref())?;
            Ok(true)
        })();
        match step {
            Ok(fresh) => {
                if !server.resources.iter().any(|r| catalog::normalize_id(&r.id) == catalog::normalize_id(&item.id)) {
                    server.resources.push(ResourceRecord {
                        id: item.id.clone(),
                        path: item.rel.clone(),
                        repository: item.repository.clone(),
                        branch: item.branch.clone(),
                        commit: item.commit.clone(),
                        third_party: item.third_party,
                        ..Default::default()
                    });
                }
                if fresh {
                    out.installed.push(item.id.clone());
                } else {
                    out.notes.push(format!("{} was already on disk; it was recorded, not changed.", item.id));
                }
                done_rels.push(item.rel.clone());
            }
            Err(e) => out.failed.push((item.id.clone(), e.one_line())),
        }
    }
    server.save().map_err(|e| e.lead("cannot write kora-server.json"))?;
    // Only folders that now really hold something new.
    let folders = layout::folders_for(done_rels.iter().map(String::as_str));
    match append_ensures_to_cfg(&server.dir.join("server.cfg"), &folders) {
        Ok(added) => out.ensured = added,
        Err(e) => out.notes.push(e.one_line()),
    }
    Ok(out)
}

/// Append the missing `ensure [folder]` lines to `server.cfg`. Returns the folders added.
pub fn append_ensures_to_cfg(cfg_path: &Path, folders: &[String]) -> std::result::Result<Vec<String>, Problem> {
    if folders.is_empty() {
        return Ok(vec![]);
    }
    let cfg = std::fs::read_to_string(cfg_path).map_err(|e| format!("Could not read {} ({e}); add `ensure` lines for {} yourself.", cfg_path.display(), folders.join(", ")))?;
    match layout::append_ensures(&cfg, folders) {
        None => Ok(vec![]),
        Some(new) => {
            std::fs::write(cfg_path, &new).map_err(|e| format!("Could not write {} ({e}); add `ensure` lines for {} yourself.", cfg_path.display(), folders.join(", ")))?;
            Ok(layout::missing_ensures(&cfg, folders))
        }
    }
}

#[cfg(test)]
mod tests;
