//! `kora github`: the GitHub accounts a development install can fork into.

use clap::Subcommand;

#[derive(Subcommand)]
pub(crate) enum GithubCmd {
    /// List the GitHub accounts and organisations your forks can go into.
    Owners,
}

/// Print the fork targets (`kora github owners`, `--github-owner list`).
pub(crate) fn github_owners() -> i32 {
    use kora_core::git::{owners, SystemRunner};
    match owners::list_owners(&SystemRunner) {
        Ok(o) => {
            print!("{}", owners::format_owner_list(&o));
            0
        }
        Err(e) => {
            eprintln!("error: could not read your GitHub accounts ({e}). Is `gh` installed and logged in? Run `gh auth login`.");
            1
        }
    }
}

/// Check `--github-owner` against the real list; with no value, only the personal account is
/// chosen automatically, and only when it is the sole usable option.
pub(crate) fn resolve_github_owner(requested: Option<&str>) -> Result<String, String> {
    resolve_github_owner_with(&kora_core::git::SystemRunner, requested)
}

fn resolve_github_owner_with(runner: &dyn kora_core::git::CommandRunner, requested: Option<&str>) -> Result<String, String> {
    use kora_core::git::owners;
    if let Some(want) = requested {
        // A named owner needs no `gh` yet: the install sets `gh` up and signs in, then re-checks it
        // (before any fork) in the prerequisites step.
        return match owners::check_requested_owner(runner, want)? {
            owners::OwnerCheck::Verified(l) => Ok(l),
            owners::OwnerCheck::Deferred(l) => {
                println!("GitHub CLI is not installed or not signed in yet: '{l}' will be checked after the sign-in during setup.");
                Ok(l)
            }
        };
    }
    let all = owners::list_owners(runner).map_err(|e| format!("could not read your GitHub accounts ({e}). Is `gh` installed and logged in? Run `gh auth login`."))?;
    owners::choose_owner(&all, requested).map(|o| o.login)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fake `gh`: `None` = not installed, `Some(false)` = signed out, `Some(true)` = signed in as `me` (org `Team`).
    struct FakeGh(Option<bool>, std::sync::Mutex<Vec<String>>);
    impl kora_core::git::CommandRunner for FakeGh {
        fn run(&self, p: &str, a: &[&str], _: Option<&std::path::Path>) -> std::io::Result<kora_core::git::Output> {
            use kora_core::git::Output;
            let line = format!("{p} {}", a.join(" "));
            self.1.lock().unwrap().push(line.clone());
            let ok = |s: &str| Ok(Output { code: 0, stdout: s.into(), stderr: String::new() });
            match self.0 {
                None => Err(std::io::Error::new(std::io::ErrorKind::NotFound, "gh not found")),
                Some(false) => Ok(Output { code: 1, stdout: String::new(), stderr: "not logged in".into() }),
                Some(true) => match line.as_str() {
                    "gh auth status --hostname github.com" => ok(""),
                    "gh api user" => ok(r#"{"login":"me"}"#),
                    l if l.starts_with("gh api user/orgs") => ok(r#"[{"login":"Team"}]"#),
                    l if l.starts_with("gh api user/memberships/orgs/") => ok(r#"{"state":"active","role":"member"}"#),
                    l if l.starts_with("gh api orgs/") => ok(r#"{"members_can_create_repositories":true}"#),
                    _ => Ok(Output { code: 1, stdout: String::new(), stderr: "unexpected".into() }),
                },
            }
        }
        fn sleep(&self, _: std::time::Duration) {}
    }

    #[test]
    fn github_owner_syntax_is_rejected_immediately() {
        for bad in ["", "-a", "a-", "a b", "a/b", &"x".repeat(40)] {
            let g = FakeGh(Some(true), Default::default());
            assert!(resolve_github_owner_with(&g, Some(bad)).is_err(), "{bad:?}");
            assert!(g.1.lock().unwrap().is_empty(), "gh is not consulted for {bad:?}");
        }
    }

    #[test]
    fn github_owner_is_verified_when_gh_is_ready() {
        let g = FakeGh(Some(true), Default::default());
        assert_eq!(resolve_github_owner_with(&g, Some("team")).unwrap(), "Team");
        let e = resolve_github_owner_with(&g, Some("nobody")).unwrap_err();
        assert!(e.contains("not your GitHub account") && e.contains("Team"), "{e}");
    }

    #[test]
    fn github_owner_is_deferred_when_gh_is_missing_or_signed_out() {
        for state in [None, Some(false)] {
            let g = FakeGh(state, Default::default());
            assert_eq!(resolve_github_owner_with(&g, Some("someone")).unwrap(), "someone", "install proceeds to the gh bootstrap step");
            assert!(!g.1.lock().unwrap().iter().any(|c| c.contains("repo fork")));
        }
    }
}
