//! GitHub for the panel: who is signed in, the fork targets, and the sign-in (device flow) as
//! plain functions behind a trait, so the web server can be tested without `gh` or the network.
//!
//! The logic is the same as `kora github owners` (`kora-cli`); it is kept here because that
//! lives in a front-end crate.

use serde::{Deserialize, Serialize};

use crate::git::owners::{self, GhAuth, OwnerKind, SystemLogin};
use crate::git::SystemRunner;
use crate::problem::{codes, known, Action, Problem};

/// Where GitHub stands for Kora, so the panel can offer the right fix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum GithubState {
    /// Signed in with every permission Kora needs.
    Ready,
    /// The GitHub CLI (`gh`) is not installed (or cannot be run).
    GhMissing,
    /// `gh` is installed but not signed in.
    SignedOut,
    /// Signed in, but some permissions (scopes) are missing.
    MissingScopes,
}

/// Who is signed in to GitHub (through `gh`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubUser {
    /// Signed in (also when some scopes are missing: issues may still work).
    pub signed_in: bool,
    pub user: Option<String>,
    pub state: GithubState,
    /// For [`GithubState::MissingScopes`]: the scopes to add.
    pub missing_scopes: Vec<String>,
}

impl GithubUser {
    pub fn ready(login: &str) -> Self {
        Self { signed_in: true, user: Some(login.to_string()), state: GithubState::Ready, missing_scopes: vec![] }
    }
    pub fn signed_out() -> Self {
        Self { signed_in: false, user: None, state: GithubState::SignedOut, missing_scopes: vec![] }
    }
    pub fn gh_missing() -> Self {
        Self { signed_in: false, user: None, state: GithubState::GhMissing, missing_scopes: vec![] }
    }
    pub fn missing_scopes(login: &str, scopes: Vec<String>) -> Self {
        Self { signed_in: true, user: Some(login.to_string()), state: GithubState::MissingScopes, missing_scopes: scopes }
    }

    /// What to tell the user when GitHub is not ready (`None` when it is).
    pub fn problem(&self) -> Option<Problem> {
        match self.state {
            GithubState::Ready => None,
            GithubState::GhMissing => Some(known::gh_missing()),
            GithubState::SignedOut => Some(known::github_signed_out()),
            GithubState::MissingScopes => Some(known::github_missing_scopes(&self.missing_scopes)),
        }
    }

    /// `GET /github/status`: `{signed_in, user, state, missing_scopes, problem}`.
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "signed_in": self.signed_in,
            "user": self.user,
            "state": self.state,
            "missing_scopes": self.missing_scopes,
            "problem": self.problem().map(|p| p.to_json()),
        })
    }
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

const GH_HELP: &str = "Is the GitHub CLI (gh) installed? You can also run `gh auth login` yourself.";

impl GithubEnv for SystemGithub {
    fn auth(&self) -> GithubUser {
        auth_with(&SystemRunner)
    }

    fn owners(&self) -> Result<Vec<OwnerChoice>, String> {
        let list = owners::list_owners(&SystemRunner).map_err(|e| format!("Could not read your GitHub accounts ({e}). {GH_HELP}"))?;
        Ok(list.into_iter().map(|o| OwnerChoice { login: o.login, is_org: o.kind == OwnerKind::Org, can_fork: o.can_fork, reason: o.reason }).collect())
    }

    fn begin_login(&self) -> Result<LoginFlow, String> {
        let session = owners::begin_login(&SystemLogin, false).map_err(|e| {
            if owners::gh_auth(&SystemRunner) == GhAuth::Missing {
                return known::gh_missing().encode();
            }
            format!("Could not start the GitHub sign-in ({e}). {GH_HELP}")
        })?;
        let (code, url) = (session.code.clone(), session.url.clone());
        Ok(LoginFlow {
            code,
            url,
            wait: Box::new(move || match session.wait() {
                Ok(GhAuth::LoggedIn { login }) => Ok(Some(login)),
                Ok(GhAuth::LoggedOut) => Err(Problem::new(codes::GH_SIGN_IN_INCOMPLETE, "GitHub did not confirm the sign-in. Start it again and approve it in your browser.")
                    .with_action(Action::SignInGithub { scopes: vec![] })
                    .encode()),
                Ok(GhAuth::Missing) => Err(known::gh_missing().encode()),
                Err(e) => {
                    Err(Problem::new(codes::GH_SIGN_IN_INCOMPLETE, "The GitHub sign-in did not finish.").with_details(e.to_string()).with_action(Action::SignInGithub { scopes: vec![] }).encode())
                }
            }),
        })
    }

    fn resolve_owner(&self, requested: Option<&str>) -> Result<String, String> {
        resolve_owner_with(&SystemRunner, requested)
    }
}

/// Where GitHub stands, told apart: `gh` missing, signed out, or signed in without every scope.
pub fn auth_with(runner: &dyn crate::git::CommandRunner) -> GithubUser {
    match owners::gh_auth(runner) {
        GhAuth::Missing => GithubUser::gh_missing(),
        GhAuth::LoggedOut => GithubUser::signed_out(),
        GhAuth::LoggedIn { login } => {
            // `gh auth status` answered a moment ago; if it cannot be read now, or lists no scopes at all
            // (a token from GH_TOKEN, a fine-grained token), do not invent a problem.
            let missing = crate::git::auth_status(runner).map(|s| if s.scopes.is_empty() { vec![] } else { s.missing_scopes(false) }).unwrap_or_default();
            if missing.is_empty() {
                GithubUser::ready(&login)
            } else {
                GithubUser::missing_scopes(&login, missing)
            }
        }
    }
}

/// Same rules as the CLI's `resolve_github_owner_with`.
pub fn resolve_owner_with(runner: &dyn crate::git::CommandRunner, requested: Option<&str>) -> Result<String, String> {
    if let Some(want) = requested {
        return match owners::check_requested_owner(runner, want)? {
            owners::OwnerCheck::Verified(l) | owners::OwnerCheck::Deferred(l) => Ok(l),
        };
    }
    // Nothing named: the accounts must be read, so say exactly what is missing first.
    match owners::gh_auth(runner) {
        GhAuth::Missing => return Err(known::gh_missing().with_action(Action::change_answers("mode")).encode()),
        GhAuth::LoggedOut => return Err(known::github_signed_out().encode()),
        GhAuth::LoggedIn { .. } => {}
    }
    let all = owners::list_owners(runner).map_err(|e| format!("Could not read your GitHub accounts ({e}). {GH_HELP}"))?;
    owners::choose_owner(&all, requested).map(|o| o.login)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{CommandRunner, Output};
    use std::path::Path;
    use std::time::Duration;

    struct Gh(fn(&str) -> std::io::Result<Output>);
    impl CommandRunner for Gh {
        fn run(&self, p: &str, a: &[&str], _: Option<&Path>) -> std::io::Result<Output> {
            (self.0)(&format!("{p} {}", a.join(" ")))
        }
        fn sleep(&self, _: Duration) {}
    }
    fn out(code: i32, text: &str) -> std::io::Result<Output> {
        Ok(Output { code, stdout: text.into(), stderr: String::new() })
    }

    #[test]
    fn status_tells_missing_signed_out_and_scopes_apart() {
        let missing = auth_with(&Gh(|_| Err(std::io::Error::new(std::io::ErrorKind::NotFound, "gh"))));
        assert_eq!((missing.state, missing.signed_in), (GithubState::GhMissing, false));
        assert_eq!(missing.problem().unwrap().code, codes::GH_MISSING);
        let out_ = auth_with(&Gh(|_| out(1, "You are not logged into any GitHub hosts")));
        assert_eq!(out_.state, GithubState::SignedOut);
        assert_eq!(out_.problem().unwrap().actions, vec![Action::SignInGithub { scopes: vec![] }]);
        let scopes = auth_with(&Gh(|l| match l {
            "gh api user" => out(0, r#"{"login":"sam","id":1}"#),
            _ => out(0, "Logged in to github.com account sam\n  - Token scopes: 'repo'"),
        }));
        assert_eq!((scopes.state, scopes.signed_in, scopes.user.as_deref()), (GithubState::MissingScopes, true, Some("sam")));
        assert_eq!(scopes.missing_scopes, vec!["read:org".to_string()]);
        let v = scopes.json();
        assert_eq!(v["state"], "missing_scopes");
        assert_eq!(v["problem"]["actions"][0], serde_json::json!({"kind": "sign_in_github", "scopes": ["read:org"]}));
        let ready = auth_with(&Gh(|l| match l {
            "gh api user" => out(0, r#"{"login":"sam","id":1}"#),
            _ => out(0, "Logged in to github.com account sam\n  - Token scopes: 'gist', 'read:org', 'repo'"),
        }));
        assert_eq!(ready, GithubUser::ready("sam"));
        assert!(ready.json()["problem"].is_null());
    }

    #[test]
    fn choosing_a_fork_owner_says_when_gh_is_missing_or_signed_out() {
        let e = resolve_owner_with(&Gh(|_| Err(std::io::Error::new(std::io::ErrorKind::NotFound, "gh"))), None).unwrap_err();
        assert_eq!(Problem::decode(&e).code, codes::GH_MISSING);
        let e = resolve_owner_with(&Gh(|_| out(1, "")), None).unwrap_err();
        assert_eq!(Problem::decode(&e).code, codes::GH_SIGNED_OUT);
    }
}
