//! GitHub for the panel: who is signed in, the fork targets, and the sign-in (device flow) as
//! plain functions behind a trait, so the web server can be tested without `gh` or the network.
//!
//! The logic is the same as the wizard's GitHub screens (`kora-tui`) and `kora github owners`
//! (`kora-cli`); it is duplicated here because those live in front-end crates.

use crate::git::owners::{self, GhAuth, OwnerKind, SystemLogin};
use crate::git::SystemRunner;

/// Who is signed in to GitHub (through `gh`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubUser {
    pub signed_in: bool,
    pub user: Option<String>,
}

/// One place a fork can go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerChoice {
    pub login: String,
    pub is_org: bool,
    pub can_fork: bool,
    pub reason: Option<String>,
}

/// A sign-in that has started: the one-time code to type and where, plus a way to wait for the
/// user to approve it in the browser.
pub struct LoginFlow {
    pub code: String,
    pub url: String,
    /// Blocks until the sign-in is approved (the user's login name) or fails (a plain message).
    pub wait: Box<dyn FnOnce() -> Result<Option<String>, String> + Send>,
}

/// Everything the panel does with GitHub that touches `gh` or the network.
pub trait GithubEnv: Send + Sync {
    fn auth(&self) -> GithubUser;
    fn owners(&self) -> Result<Vec<OwnerChoice>, String>;
    fn begin_login(&self) -> Result<LoginFlow, String>;
    /// The fork target for a development install (what `kora install --mode dev` decides):
    /// a named owner is checked (or deferred when `gh` is not ready yet); with none, the personal
    /// account is used only when it is the only usable choice.
    fn resolve_owner(&self, requested: Option<&str>) -> Result<String, String>;
}

/// The real `gh`.
pub struct SystemGithub;

const GH_HELP: &str =
    "Is the GitHub CLI (gh) installed? You can also run `gh auth login` yourself.";

impl GithubEnv for SystemGithub {
    fn auth(&self) -> GithubUser {
        match owners::gh_auth(&SystemRunner) {
            GhAuth::LoggedIn { login } => GithubUser {
                signed_in: true,
                user: Some(login),
            },
            _ => GithubUser {
                signed_in: false,
                user: None,
            },
        }
    }

    fn owners(&self) -> Result<Vec<OwnerChoice>, String> {
        let list = owners::list_owners(&SystemRunner)
            .map_err(|e| format!("Could not read your GitHub accounts ({e}). {GH_HELP}"))?;
        Ok(list
            .into_iter()
            .map(|o| OwnerChoice {
                login: o.login,
                is_org: o.kind == OwnerKind::Org,
                can_fork: o.can_fork,
                reason: o.reason,
            })
            .collect())
    }

    fn begin_login(&self) -> Result<LoginFlow, String> {
        let session = owners::begin_login(&SystemLogin, false)
            .map_err(|e| format!("Could not start the GitHub sign-in ({e}). {GH_HELP}"))?;
        let (code, url) = (session.code.clone(), session.url.clone());
        Ok(LoginFlow {
            code,
            url,
            wait: Box::new(move || {
                match session.wait() {
                Ok(GhAuth::LoggedIn { login }) => Ok(Some(login)),
                Ok(GhAuth::LoggedOut) => Err("GitHub did not confirm the sign-in. Start it again and approve it in your browser.".to_string()),
                Ok(GhAuth::Missing) => Err(format!("The GitHub CLI (gh) was not found. {GH_HELP}")),
                Err(e) => Err(format!("The GitHub sign-in did not finish ({e}).")),
            }
            }),
        })
    }

    fn resolve_owner(&self, requested: Option<&str>) -> Result<String, String> {
        resolve_owner_with(&SystemRunner, requested)
    }
}

/// Same rules as the CLI's `resolve_github_owner_with`.
pub fn resolve_owner_with(
    runner: &dyn crate::git::CommandRunner,
    requested: Option<&str>,
) -> Result<String, String> {
    if let Some(want) = requested {
        return match owners::check_requested_owner(runner, want)? {
            owners::OwnerCheck::Verified(l) | owners::OwnerCheck::Deferred(l) => Ok(l),
        };
    }
    let all = owners::list_owners(runner)
        .map_err(|e| format!("Could not read your GitHub accounts ({e}). {GH_HELP}"))?;
    owners::choose_owner(&all, requested).map(|o| o.login)
}
