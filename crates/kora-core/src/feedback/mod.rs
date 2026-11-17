//! Report an issue (both modes) and open a pull request (development mode only) for a resource
//! of a server, through the `gh` command line the installer already sets up and signs in.
//! No token is read or stored here; `gh` keeps its own login.
//!
//! Issues go to the resource's catalog repository. A pull request goes from the resource's
//! fork branch (`origin`, the user's fork) to the same upstream repository and branch.
//! Nothing secret is put into issue text: only the Kora version, the system, the mode, the
//! resource id, its version and the builds (never paths, host names, ports, keys or passwords).

use std::path::PathBuf;

use crate::catalog;
use crate::git::{self, CommandRunner};
use crate::installer::registry::{ResourceRecord, ServerFile};
use crate::installer::{repo_slug, InstallError, Mode, Result};

pub const SIGN_IN_HINT: &str = "You are not signed in to GitHub. Sign in first (the manager offers it, or run `gh auth login`), then try again.";

pub fn require_sign_in(r: &dyn CommandRunner) -> Result<()> {
    if git::is_signed_in(r) {
        Ok(())
    } else {
        Err(InstallError(SIGN_IN_HINT.into()))
    }
}

fn find<'a>(server: &'a ServerFile, id: &str) -> Result<&'a ResourceRecord> {
    let n = catalog::normalize_id(id);
    let r = server.resources.iter().find(|r| catalog::normalize_id(&r.id) == n).ok_or_else(|| InstallError(format!("this server has no resource named '{id}'")))?;
    if r.is_portal() {
        return Err(InstallError(format!("'{id}' is a paid asset from the Cfx.re Portal; report problems with it to the creator who sold it.")));
    }
    Ok(r)
}

fn gh_err(e: git::GitError) -> InstallError {
    InstallError(match e {
        git::GitError::Spawn(..) => {
            "GitHub's `gh` program could not be started. Run `kora tools install gh` to get Kora's own copy, or use 'Install it for me' on the System check page in the panel.".to_string()
        }
        other => other.to_string(),
    })
}

/// Text appended to every issue so maintainers can reproduce it. No secrets, no paths.
pub fn diagnostics(server: &ServerFile, res: &ResourceRecord, commit: Option<&str>) -> String {
    let mode = match server.mode {
        Mode::Production => "production",
        Mode::Development => "development",
    };
    let mut s = String::from("---\nReported from Kora Panel.\n");
    s.push_str(&format!("- Kora installer: {}\n", env!("CARGO_PKG_VERSION")));
    s.push_str(&format!("- System: {} ({})\n", std::env::consts::OS, std::env::consts::ARCH));
    s.push_str(&format!("- Server mode: {mode}\n- Resource: {}\n", res.id));
    if let Some(c) = commit.or(res.commit.as_deref()) {
        s.push_str(&format!("- Commit: {}\n", c.chars().take(12).collect::<String>()));
    }
    s.push_str(&format!("- Game build: {}, server build: {}\n", server.game_build, server.artifact_build));
    s
}

/// Report an issue about `resource_id` of `server`. Returns the issue address.
pub fn report_issue(r: &dyn CommandRunner, server: &ServerFile, resource_id: &str, title: &str, text: &str) -> Result<String> {
    require_sign_in(r)?;
    let res = find(server, resource_id)?;
    let dir = server.dir.join("resources").join(&res.path);
    let head = if dir.join(".git").exists() { git::head_commit(r, &dir).ok() } else { None };
    let body = format!("{}\n\n{}", text.trim(), diagnostics(server, res, head.as_deref()));
    git::create_issue(r, &repo_slug(&res.repository), title.trim(), &body).map_err(gh_err)
}

/// What a pull request would contain; shown to the user before anything is sent.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PrPlan {
    pub resource: String,
    /// `owner/name` of the upstream (catalog) repository.
    pub upstream: String,
    pub base: String,
    pub head_owner: String,
    pub head_branch: String,
    pub ahead: u32,
    pub dir: PathBuf,
}

/// Check that a PR is possible: development server, a fork (not third-party), a named branch with
/// commits upstream does not have. Fetches `upstream`; changes nothing else.
pub fn plan_pr(r: &dyn CommandRunner, server: &ServerFile, resource_id: &str) -> Result<PrPlan> {
    if server.mode != Mode::Development {
        return Err(InstallError("Pull requests are only available on development servers (production servers use pinned copies, not forks).".into()));
    }
    let res = find(server, resource_id)?;
    if res.third_party {
        return Err(InstallError(format!("{} belongs to someone else and is a pinned copy, not your fork, so no pull request can be made from it. Use 'Report an issue' instead.", res.id)));
    }
    let dir = server.dir.join("resources").join(&res.path);
    if !dir.join(".git").exists() {
        return Err(InstallError(format!("{} is not installed as a git checkout.", res.id)));
    }
    let upstream = repo_slug(&res.repository);
    let head_owner = git::remote_owner(r, &dir, "origin").map_err(gh_err)?;
    if upstream.split('/').next() == Some(head_owner.as_str()) {
        return Err(InstallError(format!("{} is not a fork of yours (its origin is the Kora repository itself).", res.id)));
    }
    let head_branch = git::current_branch(r, &dir).map_err(gh_err)?.ok_or_else(|| InstallError(format!("{} is not on a branch (detached). Check out a branch with your changes first.", res.id)))?;
    let ahead = git::ahead_of_upstream(r, &dir, &res.branch).map_err(gh_err)?;
    if ahead == 0 {
        return Err(InstallError(format!("{} has no commits that {upstream} (branch {}) lacks, so there is nothing to propose. Commit your change first.", res.id, res.branch)));
    }
    Ok(PrPlan { resource: res.id.clone(), upstream, base: res.branch.clone(), head_owner, head_branch, ahead, dir })
}

/// Push the fork branch and open the PR. Returns the pull request address.
pub fn open_pr(r: &dyn CommandRunner, plan: &PrPlan, title: &str, body: &str) -> Result<String> {
    require_sign_in(r)?;
    git::push_branch(r, &plan.dir, &plan.head_branch).map_err(gh_err)?;
    git::create_pr(r, &plan.upstream, &plan.head_owner, &plan.head_branch, &plan.base, title.trim(), body.trim()).map_err(gh_err)
}

#[cfg(test)]
mod tests;
