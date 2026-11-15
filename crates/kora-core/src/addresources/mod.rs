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
use crate::installer::{repo_slug, InstallError, Mode, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
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

/// Load the catalog (remote, with the embedded snapshot as fallback). The note says when the
/// bundled copy was used.
pub fn load_catalog() -> Result<(Catalog, Option<String>)> {
    let (c, origin) = catalog::load(
        &crate::installer::HttpClient::default(),
        catalog::CATALOG_URL,
    )
    .map_err(|e| InstallError(e.to_string()))?;
    let note = match origin {
        catalog::CatalogOrigin::Remote => None,
        catalog::CatalogOrigin::Embedded(why) => Some(format!(
            "Used the catalog bundled with this installer ({why})."
        )),
    };
    Ok((c, note))
}

/// What to install for `ids` (plus dependencies) on `server`.
pub fn plan(cat: &Catalog, server: &ServerFile, ids: &[String]) -> Result<AddPlan> {
    let picker = Picker::with_ids(cat, installed_ids(server), ids)
        .map_err(|e| InstallError(e.to_string()))?;
    plan_from_picker(&picker, server, ids)
}

pub fn plan_from_picker(
    picker: &Picker<'_>,
    server: &ServerFile,
    asked: &[String],
) -> Result<AddPlan> {
    let norm = catalog::normalize_id;
    let asked_n: Vec<String> = asked.iter().map(|a| norm(a)).collect();
    let items: Vec<PlanItem> = picker
        .resolved()
        .into_iter()
        .map(|r| {
            let third_party = r.is_pinned_copy();
            let pinned = server.mode == Mode::Production || third_party;
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
                fork: server.mode == Mode::Development && !third_party,
                third_party,
                subpath: r.subpath.clone(),
                dependency: !asked_n.contains(&norm(&r.id)) && !picker.is_picked(&r.id),
                repository: r.repository.clone(),
            }
        })
        .collect();
    let already: Vec<String> = asked
        .iter()
        .filter(|a| picker.is_installed(a))
        .cloned()
        .collect();
    let ensure = layout::folders_for(items.iter().map(|i| i.rel.as_str()));
    Ok(AddPlan {
        items,
        already,
        ensure,
    })
}

/// The git side of an install, so tests can run without a network.
pub trait Cloner {
    /// Make sure forks of `repos` (`owner/name`) exist under `owner` (development servers).
    fn prepare_forks(&self, owner: &str, repos: &[String]) -> std::result::Result<(), String>;
    /// Clone `item` into `partial` (a fresh path that does not exist yet).
    fn clone_item(
        &self,
        item: &PlanItem,
        fork_owner: Option<&str>,
        partial: &Path,
    ) -> std::result::Result<(), String>;
}

pub struct GitCloner<'a>(pub &'a dyn CommandRunner);

impl Cloner for GitCloner<'_> {
    fn prepare_forks(&self, owner: &str, repos: &[String]) -> std::result::Result<(), String> {
        if repos.is_empty() {
            return Ok(());
        }
        let is_org = git::org_exists(self.0, owner).map_err(|e| e.to_string())?;
        let mut state = git::ForkState::new(owner, is_org, repos);
        let out = git::run_forks(self.0, &mut state, &git::ForkConfig::default(), &mut |_| {})
            .map_err(|e| e.to_string())?;
        match out {
            git::ForkOutcome::Paused => {
                Err("GitHub rate limit reached while forking. Try again later.".into())
            }
            git::ForkOutcome::Finished if !state.failed.is_empty() => Err(format!(
                "could not fork {} ({})",
                state.failed[0].repo, state.failed[0].error
            )),
            git::ForkOutcome::Finished => Ok(()),
        }
    }

    fn clone_item(
        &self,
        item: &PlanItem,
        fork_owner: Option<&str>,
        partial: &Path,
    ) -> std::result::Result<(), String> {
        let res = match (item.fork, fork_owner, &item.commit) {
            (true, Some(owner), _) => {
                let name = item.repo.rsplit('/').next().unwrap_or("");
                git::clone_from_fork(
                    self.0,
                    &format!("{owner}/{name}"),
                    &item.repo,
                    partial,
                    Some(&item.branch),
                )
            }
            (true, None, _) => {
                return Err(
                    "this development server has no GitHub account recorded for its forks".into(),
                )
            }
            (false, _, Some(c)) => git::clone_pinned(self.0, &item.repo, c, partial),
            (false, _, None) => git::clone_default_head(self.0, &item.repo, partial),
        };
        res.map_err(|e| e.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
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

fn dest_of(server_dir: &Path, rel: &str) -> PathBuf {
    server_dir.join("resources").join(rel)
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
pub fn execute(
    cloner: &dyn Cloner,
    server: &mut ServerFile,
    plan: &AddPlan,
    say: &mut dyn FnMut(&str),
) -> Result<AddOutcome> {
    let mut out = AddOutcome::default();
    let resources = server.dir.join("resources");
    std::fs::create_dir_all(&resources)
        .map_err(|e| InstallError(format!("cannot create the resources folder: {e}")))?;
    let owner = server.github_owner.clone();
    let fork_repos: Vec<String> = plan
        .items
        .iter()
        .filter(|i| i.fork)
        .map(|i| i.repo.clone())
        .collect();
    if !fork_repos.is_empty() {
        let Some(o) = &owner else {
            return Err(InstallError("this development server has no GitHub account recorded, so its resources cannot be forked".into()));
        };
        say("Forking on GitHub (this can take a while)");
        cloner.prepare_forks(o, &fork_repos).map_err(InstallError)?;
    }
    let mut done_rels: Vec<String> = Vec::new();
    for item in &plan.items {
        let dest = dest_of(&server.dir, &item.rel);
        say(&format!("Getting {}", item.id));
        let step = (|| -> std::result::Result<bool, String> {
            let present = if item.subpath.is_some() {
                item.commit
                    .as_deref()
                    .is_some_and(|c| pinned::marker_matches(&dest, c))
            } else {
                dest.join(".git").exists()
            };
            if present {
                return Ok(false);
            }
            if let Some(parent) = inside_another_clone(&resources, &dest) {
                return Err(format!(
                    "its folder is inside the resource at {}",
                    parent.display()
                ));
            }
            if dest.exists()
                && std::fs::read_dir(&dest)
                    .map(|mut d| d.next().is_some())
                    .unwrap_or(true)
            {
                return Err(format!(
                    "{} already exists and was not created by Kora; move it away first",
                    dest.display()
                ));
            }
            let name = dest
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let partial = dest.with_file_name(format!("{name}.kora-partial"));
            let _ = std::fs::remove_dir_all(&partial);
            if let Some(p) = dest.parent() {
                std::fs::create_dir_all(p).map_err(|e| format!("cannot create the folder: {e}"))?;
            }
            if dest.exists() {
                let _ = std::fs::remove_dir(&dest); // an empty leftover
            }
            if let Err(e) = cloner.clone_item(item, owner.as_deref(), &partial) {
                let _ = std::fs::remove_dir_all(&partial);
                return Err(e);
            }
            pinned::place(
                &partial,
                &dest,
                item.subpath.as_deref(),
                &item.repo,
                item.commit.as_deref(),
            )?;
            Ok(true)
        })();
        match step {
            Ok(fresh) => {
                if !server
                    .resources
                    .iter()
                    .any(|r| catalog::normalize_id(&r.id) == catalog::normalize_id(&item.id))
                {
                    server.resources.push(ResourceRecord {
                        id: item.id.clone(),
                        path: item.rel.clone(),
                        repository: item.repository.clone(),
                        branch: item.branch.clone(),
                        commit: item.commit.clone(),
                        third_party: item.third_party,
                    });
                }
                if fresh {
                    out.installed.push(item.id.clone());
                } else {
                    out.notes.push(format!(
                        "{} was already on disk; it was recorded, not changed.",
                        item.id
                    ));
                }
                done_rels.push(item.rel.clone());
            }
            Err(e) => out.failed.push((item.id.clone(), e)),
        }
    }
    server
        .save()
        .map_err(|e| InstallError(format!("cannot write kora-server.json: {e}")))?;
    // Only folders that now really hold something new.
    let folders = layout::folders_for(done_rels.iter().map(String::as_str));
    match append_ensures_to_cfg(&server.dir.join("server.cfg"), &folders) {
        Ok(added) => out.ensured = added,
        Err(e) => out.notes.push(e),
    }
    Ok(out)
}

/// Append the missing `ensure [folder]` lines to `server.cfg`. Returns the folders added.
pub fn append_ensures_to_cfg(
    cfg_path: &Path,
    folders: &[String],
) -> std::result::Result<Vec<String>, String> {
    if folders.is_empty() {
        return Ok(vec![]);
    }
    let cfg = std::fs::read_to_string(cfg_path).map_err(|e| {
        format!(
            "Could not read {} ({e}); add `ensure` lines for {} yourself.",
            cfg_path.display(),
            folders.join(", ")
        )
    })?;
    match layout::append_ensures(&cfg, folders) {
        None => Ok(vec![]),
        Some(new) => {
            std::fs::write(cfg_path, &new).map_err(|e| {
                format!(
                    "Could not write {} ({e}); add `ensure` lines for {} yourself.",
                    cfg_path.display(),
                    folders.join(", ")
                )
            })?;
            Ok(layout::missing_ensures(&cfg, folders))
        }
    }
}

#[cfg(test)]
mod tests;
