use super::*;
use crate::git::Output;
use crate::installer::registry::PortsRecord;
use std::cell::RefCell;
use std::path::Path;
use std::time::Duration;

struct Fake {
    calls: RefCell<Vec<String>>,
    signed_in: bool,
    origin: &'static str,
    branch: &'static str,
    ahead: &'static str,
}

impl Fake {
    fn new() -> Self {
        Fake { calls: RefCell::new(vec![]), signed_in: true, origin: "https://github.com/me/kora_lib.git", branch: "fix/thing", ahead: "2" }
    }
}

impl CommandRunner for Fake {
    fn run(&self, p: &str, a: &[&str], _cwd: Option<&Path>) -> std::io::Result<Output> {
        let line = format!("{p} {}", a.join(" "));
        self.calls.borrow_mut().push(line.clone());
        let out = |s: &str| Output { code: 0, stdout: s.to_string(), stderr: String::new() };
        Ok(if line.starts_with("gh auth status") {
            Output { code: if self.signed_in { 0 } else { 1 }, stdout: "Token scopes: 'repo', 'read:org'".into(), stderr: String::new() }
        } else if line.starts_with("git remote get-url origin") {
            out(self.origin)
        } else if line.starts_with("git rev-parse --abbrev-ref") {
            out(self.branch)
        } else if line.starts_with("git rev-list --count") {
            out(self.ahead)
        } else if line.starts_with("git rev-parse HEAD") {
            out("0123456789abcdef0123456789abcdef01234567")
        } else if line.starts_with("gh issue create") {
            out("https://github.com/KoraFiveM/kora_lib/issues/3")
        } else if line.starts_with("gh pr create") {
            out("https://github.com/KoraFiveM/kora_lib/pull/4")
        } else {
            out("")
        })
    }
    fn sleep(&self, _: Duration) {}
}

fn server(dir: &Path, mode: Mode, third_party: bool) -> ServerFile {
    std::fs::create_dir_all(dir.join("resources/[kora]/kora_lib/.git")).unwrap();
    ServerFile {
        name: "s".into(),
        mode,
        dir: dir.to_path_buf(),
        ports: PortsRecord { game: 30120, txadmin: 40120 },
        game_build: 3258,
        artifact_build: 38621,
        database: None,
        github_owner: Some("me".into()),
        created_unix: 0,
        kora_version: "0".into(),
        resources: vec![ResourceRecord {
            id: "kora-lib".into(),
            path: "[kora]/kora_lib".into(),
            repository: "https://github.com/KoraFiveM/kora_lib.git".into(),
            branch: "dev".into(),
            commit: None,
            third_party,
            ..Default::default()
        }],
    }
}

#[test]
fn issue_goes_to_the_resources_repository_with_safe_diagnostics() {
    let d = tempfile::tempdir().unwrap();
    for mode in [Mode::Production, Mode::Development] {
        let sf = server(d.path(), mode, false);
        let f = Fake::new();
        let url = report_issue(&f, &sf, "kora_lib", "Crash on start", "It crashes.").unwrap();
        assert!(url.ends_with("/issues/3"));
        let call = f.calls.borrow().iter().find(|c| c.starts_with("gh issue create")).unwrap().clone();
        assert!(call.contains("--repo KoraFiveM/kora_lib --title Crash on start"));
        assert!(call.contains("It crashes."));
        assert!(call.contains("Commit: 0123456789ab") && call.contains("Resource: kora-lib"));
        assert!(!call.contains(d.path().to_str().unwrap()), "no local paths in an issue");
    }
}

#[test]
fn nothing_is_sent_when_not_signed_in_or_unknown() {
    let d = tempfile::tempdir().unwrap();
    let sf = server(d.path(), Mode::Production, false);
    let f = Fake { signed_in: false, ..Fake::new() };
    let e = report_issue(&f, &sf, "kora-lib", "t", "b").unwrap_err();
    assert!(e.to_string().contains("not signed in"));
    assert!(!f.calls.borrow().iter().any(|c| c.contains("issue create")));
    assert!(report_issue(&Fake::new(), &sf, "nope", "t", "b").is_err());
    assert!(report_issue(&Fake::new(), &sf, "kora-lib", "", "b").is_err(), "empty title is refused");
}

#[test]
fn a_pr_is_dev_mode_only_and_never_for_third_party_copies() {
    let d = tempfile::tempdir().unwrap();
    let e = plan_pr(&Fake::new(), &server(d.path(), Mode::Production, false), "kora-lib").unwrap_err();
    assert!(e.to_string().contains("development servers"));
    let e = plan_pr(&Fake::new(), &server(d.path(), Mode::Development, true), "kora-lib").unwrap_err();
    assert!(e.to_string().contains("belongs to someone else"));
}

#[test]
fn pr_plan_checks_fork_branch_and_commits() {
    let d = tempfile::tempdir().unwrap();
    let sf = server(d.path(), Mode::Development, false);
    let p = plan_pr(&Fake::new(), &sf, "kora-lib").unwrap();
    assert_eq!((p.upstream.as_str(), p.base.as_str(), p.head_owner.as_str(), p.head_branch.as_str(), p.ahead), ("KoraFiveM/kora_lib", "dev", "me", "fix/thing", 2));
    // origin is the Kora repo itself: not a fork
    let f = Fake { origin: "https://github.com/KoraFiveM/kora_lib.git", ..Fake::new() };
    assert!(plan_pr(&f, &sf, "kora-lib").unwrap_err().to_string().contains("not a fork"));
    // detached
    let f = Fake { branch: "HEAD", ..Fake::new() };
    assert!(plan_pr(&f, &sf, "kora-lib").unwrap_err().to_string().contains("detached"));
    // nothing to propose
    let f = Fake { ahead: "0", ..Fake::new() };
    assert!(plan_pr(&f, &sf, "kora-lib").unwrap_err().to_string().contains("nothing to propose"));
}

#[test]
fn open_pr_pushes_the_fork_branch_then_opens_it_against_upstream() {
    let d = tempfile::tempdir().unwrap();
    let sf = server(d.path(), Mode::Development, false);
    let f = Fake::new();
    let plan = plan_pr(&f, &sf, "kora-lib").unwrap();
    let url = open_pr(&f, &plan, "Fix the thing", "Details").unwrap();
    assert!(url.ends_with("/pull/4"));
    let calls = f.calls.borrow();
    let push = calls.iter().position(|c| c == "git push --set-upstream origin fix/thing").unwrap();
    let pr = calls.iter().position(|c| c.starts_with("gh pr create")).unwrap();
    assert!(push < pr);
    assert!(calls[pr].contains("--repo KoraFiveM/kora_lib --head me:fix/thing --base dev"));
}
