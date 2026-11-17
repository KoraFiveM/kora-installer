use super::*;
use crate::git::SystemRunner;
use std::cell::RefCell;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

fn sh(dir: &Path, args: &[&str]) {
    let o =
        Command::new("git").args(args).current_dir(dir).env("GIT_AUTHOR_NAME", "t").env("GIT_AUTHOR_EMAIL", "t@t").env("GIT_COMMITTER_NAME", "t").env("GIT_COMMITTER_EMAIL", "t@t").output().unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
}

fn commit(dir: &Path, file: &str, text: &str) {
    std::fs::write(dir.join(file), text).unwrap();
    sh(dir, &["add", "."]);
    sh(dir, &["commit", "-qm", &format!("{file} {text}")]);
}

/// Runs real git; answers the GitHub parts (`gh`, remote addresses, the pull request fetch) itself.
struct Fake {
    /// (part of the gh command line, exit code, stdout, stderr)
    gh: Vec<(String, i32, String, String)>,
    /// Where `refs/pull/<n>/head` really lives for the test: (n, path, branch).
    pulls: Vec<(u32, PathBuf, String)>,
    calls: RefCell<Vec<String>>,
}

impl Fake {
    fn new() -> Self {
        Fake { gh: vec![], pulls: vec![], calls: RefCell::new(vec![]) }
    }
}

impl CommandRunner for Fake {
    fn run(&self, program: &str, args: &[&str], cwd: Option<&Path>) -> std::io::Result<Output> {
        let line = format!("{program} {}", args.join(" "));
        self.calls.borrow_mut().push(line.clone());
        if program == "gh" {
            for (key, code, out, err) in &self.gh {
                if line.contains(key.as_str()) {
                    return Ok(Output { code: *code, stdout: out.clone(), stderr: err.clone() });
                }
            }
            return Ok(Output { code: 1, stdout: String::new(), stderr: "no canned answer".into() });
        }
        if args.len() == 3 && args[..2] == ["remote", "get-url"] && !matches!(args[2], "origin" | "upstream") {
            return SystemRunner.run(program, args, cwd);
        }
        if args.len() == 3 && args[..2] == ["remote", "get-url"] {
            let url = if args[2] == "origin" { "https://github.com/me/res.git\n" } else { "https://github.com/KoraFiveM/res.git\n" };
            let real = SystemRunner.run(program, args, cwd)?;
            return Ok(if real.ok() { Output { code: 0, stdout: url.into(), stderr: String::new() } } else { real });
        }
        if let Some(i) = args.iter().position(|a| a.starts_with("+refs/pull/")) {
            let n: u32 = args[i].trim_start_matches("+refs/pull/").split('/').next().unwrap().parse().unwrap();
            let Some((_, path, branch)) = self.pulls.iter().find(|(p, _, _)| *p == n) else {
                return Ok(Output { code: 128, stdout: String::new(), stderr: "fatal: couldn't find remote ref refs/pull/9/head".into() });
            };
            let target = args[i].split(':').nth(1).unwrap().to_string();
            let spec = format!("+refs/heads/{branch}:{target}");
            let p = path.to_string_lossy().to_string();
            return SystemRunner.run(program, &["fetch", &p, &spec], cwd);
        }
        SystemRunner.run(program, args, cwd)
    }
    fn sleep(&self, _: Duration) {}
}

struct World {
    _t: tempfile::TempDir,
    up: PathBuf,
    fork: PathBuf,
}

/// An upstream repository (`dev`, plus a `feature` branch), a fork of it that is the resource folder.
fn world() -> World {
    let t = tempfile::tempdir().unwrap();
    let up = t.path().join("up");
    std::fs::create_dir(&up).unwrap();
    sh(&up, &["init", "-q", "-b", "dev"]);
    commit(&up, "a", "1");
    sh(&up, &["branch", "feature"]);
    sh(&up, &["checkout", "-q", "feature"]);
    commit(&up, "f", "feature");
    sh(&up, &["checkout", "-q", "dev"]);
    let fork = t.path().join("fork");
    sh(t.path(), &["clone", "-q", up.to_str().unwrap(), fork.to_str().unwrap()]);
    sh(&fork, &["remote", "add", "upstream", up.to_str().unwrap()]);
    sh(&fork, &["fetch", "-q", "upstream"]);
    sh(&fork, &["config", "user.email", "t@t"]);
    sh(&fork, &["config", "user.name", "t"]);
    World { _t: t, up, fork }
}

fn on(w: &World) -> Where {
    locate(&SystemRunner, &w.fork, "dev").unwrap()
}

#[test]
fn locates_default_branch_pr_and_detached() {
    let w = world();
    let now = on(&w);
    assert_eq!((now.kind, now.branch.as_deref(), now.dirty()), (Kind::Default, Some("dev"), false));
    sh(&w.fork, &["checkout", "-q", "-b", "my-fix"]);
    assert_eq!(on(&w).kind, Kind::Branch);
    sh(&w.fork, &["checkout", "-q", "-b", "pr/12"]);
    let n = on(&w);
    assert_eq!((n.kind, n.pr), (Kind::Pr, Some(12)));
    sh(&w.fork, &["checkout", "-q", "--detach"]);
    let n = on(&w);
    assert_eq!((n.kind, n.branch), (Kind::Detached, None));
}

#[test]
fn a_folder_that_is_not_a_repository_is_a_plain_problem() {
    let t = tempfile::tempdir().unwrap();
    let e = locate(&SystemRunner, t.path(), "dev").unwrap_err();
    // Inside a temp dir that is not in any repository: git says so.
    assert_eq!(e.code, codes::NOT_REPO, "{e:?}");
    assert!(!e.message.contains("fatal"));
}

#[test]
fn a_folder_inside_another_repository_does_not_count_as_its_own() {
    let w = world();
    let inner = w.fork.join("sub");
    std::fs::create_dir(&inner).unwrap();
    let e = locate(&SystemRunner, &inner, "dev").unwrap_err();
    assert_eq!(e.code, codes::NOT_REPO);
}

#[test]
fn lists_local_fork_and_upstream_branches_default_first() {
    let w = world();
    let b = branches(&SystemRunner, &w.fork, "dev").unwrap();
    assert_eq!(b.current.as_deref(), Some("dev"));
    assert_eq!(b.local, vec!["dev"]);
    assert_eq!(b.fork, vec!["dev", "feature"]);
    assert_eq!(b.upstream, vec!["dev", "feature"]);
}

#[test]
fn switches_to_a_remote_branch_then_back_to_default() {
    let w = world();
    let r = SystemRunner;
    let o = switch_branch(&r, &w.fork, "dev", "feature", None, Changes::Ask).unwrap();
    assert_eq!(o.status, Status::Switched);
    assert_eq!((o.now.kind, o.now.branch.as_deref()), (Kind::Branch, Some("feature")));
    assert!(w.fork.join("f").exists());
    // Already there.
    assert_eq!(switch_branch(&r, &w.fork, "dev", "feature", None, Changes::Ask).unwrap().status, Status::Already);
    let o = back_to_default(&r, &w.fork, "dev", Changes::Ask).unwrap();
    assert_eq!(o.now.kind, Kind::Default);
    assert!(!w.fork.join("f").exists());
}

#[test]
fn a_branch_that_exists_nowhere_is_a_guided_problem() {
    let w = world();
    let e = switch_branch(&SystemRunner, &w.fork, "dev", "nope", None, Changes::Ask).unwrap_err();
    assert_eq!(e.code, codes::BRANCH_GONE);
    assert!(e.actions.contains(&Action::Retry));
    // Nothing moved.
    assert_eq!(on(&w).kind, Kind::Default);
}

#[test]
fn bad_branch_names_never_reach_git() {
    let w = world();
    let f = Fake::new();
    for bad in ["-x", "--detach", "a..b", "a b", "a;rm", "x/", ".hidden", "a.lock", "", "$(id)"] {
        let e = switch_branch(&f, &w.fork, "dev", bad, None, Changes::Stash).unwrap_err();
        assert_eq!(e.code, codes::BAD_NAME, "{bad}");
    }
    assert!(f.calls.borrow().iter().all(|c| !c.contains("checkout")), "{:?}", f.calls.borrow());
    let e = switch_branch(&f, &w.fork, "dev", "feature", Some("evil"), Changes::Ask).unwrap_err();
    assert_eq!(e.code, codes::BAD_NAME);
}

#[test]
fn changes_are_never_lost_silently() {
    let w = world();
    let r = SystemRunner;
    std::fs::write(w.fork.join("a"), "my edit").unwrap();
    std::fs::write(w.fork.join("new.txt"), "brand new").unwrap();
    // No choice yet: nothing is touched, the files are listed.
    let o = switch_branch(&r, &w.fork, "dev", "feature", None, Changes::Ask).unwrap();
    assert_eq!(o.status, Status::NeedsChoice);
    assert_eq!(o.now.changed_count, 2);
    assert!(o.now.changed_files.contains(&"a".to_string()) && o.now.changed_files.contains(&"new.txt".to_string()), "{:?}", o.now.changed_files);
    assert_eq!(on(&w).kind, Kind::Default);
    assert_eq!(std::fs::read_to_string(w.fork.join("a")).unwrap(), "my edit");
    // Keep them aside.
    let o = switch_branch(&r, &w.fork, "dev", "feature", None, Changes::Stash).unwrap();
    assert_eq!(o.status, Status::Switched);
    let st = o.stashed.expect("stashed");
    assert!(st.message.contains("feature") && st.branch == "dev", "{st:?}");
    assert!(!on(&w).dirty());
    assert!(!w.fork.join("new.txt").exists());
    // Findable again, and brought back.
    assert_eq!(stashes(&r, &w.fork).unwrap().len(), 1);
    back_to_default(&r, &w.fork, "dev", Changes::Ask).unwrap();
    let o = restore_stash(&r, &w.fork, "dev", st.index).unwrap();
    assert_eq!(o.now.changed_count, 2);
    assert_eq!(std::fs::read_to_string(w.fork.join("a")).unwrap(), "my edit");
    assert_eq!(std::fs::read_to_string(w.fork.join("new.txt")).unwrap(), "brand new");
    // The stash is never deleted.
    assert_eq!(stashes(&r, &w.fork).unwrap().len(), 1);
}

#[test]
fn discarding_throws_the_changes_away() {
    let w = world();
    std::fs::write(w.fork.join("a"), "my edit").unwrap();
    std::fs::write(w.fork.join("new.txt"), "x").unwrap();
    let o = switch_branch(&SystemRunner, &w.fork, "dev", "feature", None, Changes::Discard).unwrap();
    assert_eq!(o.status, Status::Switched);
    assert!(o.stashed.is_none());
    assert!(!on(&w).dirty());
    assert!(!w.fork.join("new.txt").exists());
    assert!(stashes(&SystemRunner, &w.fork).unwrap().is_empty());
}

#[test]
fn bringing_changes_back_that_conflict_leaves_everything_as_it_was() {
    let w = world();
    let r = SystemRunner;
    std::fs::write(w.fork.join("a"), "mine").unwrap();
    let st = switch_branch(&r, &w.fork, "dev", "feature", None, Changes::Stash).unwrap().stashed.unwrap();
    // A different change to the same line arrives meanwhile.
    commit(&w.fork, "a", "theirs");
    let e = restore_stash(&r, &w.fork, "dev", st.index).unwrap_err();
    assert_eq!(e.code, codes::STASH_CONFLICT);
    assert!(e.message.contains("still kept aside"));
    assert!(!on(&w).dirty(), "the folder is clean again");
    assert_eq!(std::fs::read_to_string(w.fork.join("a")).unwrap(), "theirs");
    assert_eq!(stashes(&r, &w.fork).unwrap().len(), 1);
    // With changes in the way it asks first.
    std::fs::write(w.fork.join("a"), "again").unwrap();
    assert_eq!(restore_stash(&r, &w.fork, "dev", st.index).unwrap_err().code, codes::STASH_NEEDS_CLEAN);
    assert_eq!(restore_stash(&r, &w.fork, "dev", 7).unwrap_err().code, codes::STASH_NEEDS_CLEAN);
}

#[test]
fn leaving_a_detached_commit_saves_it_on_a_branch() {
    let w = world();
    sh(&w.fork, &["checkout", "-q", "--detach"]);
    commit(&w.fork, "mine", "work");
    let o = back_to_default(&SystemRunner, &w.fork, "dev", Changes::Ask).unwrap();
    let saved = o.saved_branch.expect("saved");
    assert!(saved.starts_with("kora-saved-"));
    assert!(branches(&SystemRunner, &w.fork, "dev").unwrap().local.contains(&saved));
    assert_eq!(on(&w).kind, Kind::Default);
}

#[test]
fn a_remote_branch_of_the_same_name_from_the_other_place_does_not_clobber() {
    let w = world();
    let r = SystemRunner;
    switch_branch(&r, &w.fork, "dev", "feature", Some("origin"), Changes::Ask).unwrap();
    back_to_default(&r, &w.fork, "dev", Changes::Ask).unwrap();
    // The same name from `upstream` is a separate local branch.
    let o = switch_branch(&r, &w.fork, "dev", "feature", Some("upstream"), Changes::Ask).unwrap();
    assert_eq!(o.now.branch.as_deref(), Some("upstream-feature"));
}

// ---- pull requests

const LIST: &str = r#"[{"number":5,"title":"Fix the HUD","author":{"login":"sam"},"headRefName":"fix-hud","headRepository":{"name":"res"},"headRepositoryOwner":{"login":"sam"},"isDraft":false,"url":"https://github.com/KoraFiveM/res/pull/5","updatedAt":"2026-01-02T00:00:00Z"},
{"number":3,"title":"Gone fork","author":{"login":"ghost"},"headRefName":"x","headRepository":null,"headRepositoryOwner":null,"isDraft":true,"url":"https://github.com/KoraFiveM/res/pull/3","updatedAt":"2026-01-01T00:00:00Z"}]"#;

fn open_view(title: &str, state: &str, gone: bool) -> String {
    let head = if gone { "null".to_string() } else { r#"{"name":"res"}"#.to_string() };
    format!(r#"{{"number":5,"title":"{title}","state":"{state}","url":"https://github.com/KoraFiveM/res/pull/5","headRepository":{head}}}"#)
}

/// A pull request source: a branch in the upstream repo (stands for `refs/pull/5/head`).
fn with_pr(w: &World, n: u32, branch: &str) -> Fake {
    sh(&w.up, &["checkout", "-q", "-b", branch, "dev"]);
    commit(&w.up, "pr-file", "pr one");
    sh(&w.up, &["checkout", "-q", "dev"]);
    let mut f = Fake::new();
    f.pulls.push((n, w.up.clone(), branch.to_string()));
    f
}

#[test]
fn lists_pull_requests_from_both_places_labelled() {
    let w = world();
    let mut f = Fake::new();
    f.gh.push(("--repo KoraFiveM/res".into(), 0, LIST.into(), String::new()));
    f.gh.push(("--repo me/res".into(), 0, r#"[{"number":1,"title":"My own","author":{"login":"me"},"headRefName":"mine","headRepository":{"name":"res"},"headRepositoryOwner":{"login":"me"},"isDraft":false,"url":"https://github.com/me/res/pull/1","updatedAt":"2026-02-01T00:00:00Z"}]"#.into(), String::new()));
    let (list, notes) = list_prs(&f, &w.fork).unwrap();
    assert!(notes.is_empty());
    assert_eq!(list.iter().map(|p| (p.number, p.source)).collect::<Vec<_>>(), vec![(1, PrSource::Fork), (5, PrSource::Upstream), (3, PrSource::Upstream)]);
    let p = list.iter().find(|p| p.number == 5).unwrap();
    assert_eq!((p.title.as_str(), p.author.as_str(), p.from.as_deref(), p.repo.as_str()), ("Fix the HUD", "sam", Some("sam/res"), "KoraFiveM/res"));
    assert_eq!(list.iter().find(|p| p.number == 3).unwrap().from, None);
}

#[test]
fn listing_pull_requests_explains_signed_out_missing_gh_and_no_network() {
    let w = world();
    let mut f = Fake::new();
    f.gh.push(("pr list".into(), 4, String::new(), "To get started with GitHub CLI, please run:  gh auth login".into()));
    assert_eq!(list_prs(&f, &w.fork).unwrap_err().code, pcodes::GH_SIGNED_OUT);
    let mut f = Fake::new();
    f.gh.push(("pr list".into(), 1, String::new(), "error connecting to api.github.com: Could not resolve host".into()));
    let e = list_prs(&f, &w.fork).unwrap_err();
    assert_eq!(e.code, codes::NO_NETWORK);
    assert!(e.actions.contains(&Action::Retry));
    // gh itself missing.
    struct NoGh;
    impl CommandRunner for NoGh {
        fn run(&self, p: &str, a: &[&str], c: Option<&Path>) -> std::io::Result<Output> {
            if p == "gh" {
                return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "not found"));
            }
            if a.first() == Some(&"remote") {
                return Ok(Output { code: 0, stdout: "https://github.com/a/b.git\n".into(), stderr: String::new() });
            }
            SystemRunner.run(p, a, c)
        }
        fn sleep(&self, _: Duration) {}
    }
    let e = list_prs(&NoGh, &w.fork).unwrap_err();
    assert_eq!(e.code, pcodes::GH_MISSING);
    assert!(e.actions.iter().any(|a| matches!(a, Action::InstallTool { tool: Tool::Gh })));
}

#[test]
fn parses_what_people_paste() {
    assert_eq!(parse_pr_input("12").unwrap(), PrInput { number: 12, repo: None });
    assert_eq!(parse_pr_input(" #12 ").unwrap().number, 12);
    let p = parse_pr_input("https://github.com/KoraFiveM/res/pull/45/files?x=1#diff").unwrap();
    assert_eq!((p.number, p.repo.as_deref()), (45, Some("KoraFiveM/res")));
    for bad in ["", "0", "abc", "-5", "12; rm -rf /", "https://evil.example/o/n/pull/1", "https://github.com/o/n/issues/1", "99999999", "https://github.com/o/pull/1"] {
        assert!(parse_pr_input(bad).is_err(), "{bad}");
    }
}

#[test]
fn checks_out_a_pull_request_and_refreshes_it_later() {
    let w = world();
    let mut f = with_pr(&w, 5, "fix-hud");
    f.gh.push(("pr view 5 --repo KoraFiveM/res".into(), 0, open_view("Fix the HUD", "OPEN", false), String::new()));
    let input = PrInput { number: 5, repo: None };
    let o = checkout_pr(&f, &w.fork, "dev", &input, Some(PrSource::Upstream), Changes::Ask).unwrap();
    assert_eq!(o.status, Status::Switched);
    assert_eq!((o.now.kind, o.now.pr, o.title.as_deref()), (Kind::Pr, Some(5), Some("Fix the HUD")));
    assert!(w.fork.join("pr-file").exists());
    // Nothing new: already there.
    assert_eq!(checkout_pr(&f, &w.fork, "dev", &input, None, Changes::Ask).unwrap().status, Status::Already);
    // The author adds a commit: trying again fast-forwards.
    sh(&w.up, &["checkout", "-q", "fix-hud"]);
    commit(&w.up, "pr-file", "pr two");
    sh(&w.up, &["checkout", "-q", "dev"]);
    let o = checkout_pr(&f, &w.fork, "dev", &input, None, Changes::Ask).unwrap();
    assert_eq!(o.status, Status::Switched);
    assert!(o.message.contains("newest commits"), "{}", o.message);
    assert_eq!(std::fs::read_to_string(w.fork.join("pr-file")).unwrap(), "pr two");
    // Back to the normal version.
    assert_eq!(back_to_default(&f, &w.fork, "dev", Changes::Ask).unwrap().now.kind, Kind::Default);
}

#[test]
fn a_rewritten_pull_request_gets_a_new_branch_and_nothing_is_forced() {
    let w = world();
    let mut f = with_pr(&w, 5, "fix-hud");
    f.gh.push(("pr view 5".into(), 0, open_view("Fix", "OPEN", false), String::new()));
    let input = PrInput { number: 5, repo: None };
    checkout_pr(&f, &w.fork, "dev", &input, None, Changes::Ask).unwrap();
    back_to_default(&f, &w.fork, "dev", Changes::Ask).unwrap();
    // The author force-pushes something unrelated.
    sh(&w.up, &["checkout", "-q", "fix-hud"]);
    sh(&w.up, &["reset", "-q", "--hard", "dev"]);
    commit(&w.up, "other", "rewritten");
    sh(&w.up, &["checkout", "-q", "dev"]);
    let o = checkout_pr(&f, &w.fork, "dev", &input, None, Changes::Ask).unwrap();
    let b = o.now.branch.unwrap();
    assert!(b.starts_with("pr/5-"), "{b}");
    assert_eq!(o.now.pr, Some(5));
    assert!(branches(&f, &w.fork, "dev").unwrap().local.contains(&"pr/5".to_string()), "the earlier copy is kept");
    assert!(f.calls.borrow().iter().all(|c| !c.contains("--force") && !c.contains("push") && !c.contains("branch -D") && !c.contains("stash drop")));
}

#[test]
fn pull_request_failures_are_guided() {
    let w = world();
    let input = PrInput { number: 5, repo: None };
    // Not found anywhere.
    let mut f = Fake::new();
    f.gh.push(("pr view".into(), 1, String::new(), "GraphQL: Could not resolve to a PullRequest with the number of 5. (repository.pullRequest)".into()));
    assert_eq!(checkout_pr(&f, &w.fork, "dev", &input, None, Changes::Ask).unwrap_err().code, codes::PR_NOT_FOUND);
    // Merged and closed.
    for (state, word) in [("MERGED", "merged"), ("CLOSED", "closed")] {
        let mut f = Fake::new();
        f.gh.push(("pr view".into(), 0, open_view("x", state, false), String::new()));
        let e = checkout_pr(&f, &w.fork, "dev", &input, None, Changes::Ask).unwrap_err();
        assert_eq!(e.code, codes::PR_CLOSED);
        assert!(e.message.contains(word), "{}", e.message);
        assert!(e.actions.iter().any(|a| matches!(a, Action::OpenLink { .. })));
    }
    // Fork deleted and GitHub has no commits for it.
    let mut f = Fake::new();
    f.gh.push(("pr view".into(), 0, open_view("x", "OPEN", true), String::new()));
    let e = checkout_pr(&f, &w.fork, "dev", &input, None, Changes::Ask).unwrap_err();
    assert_eq!(e.code, codes::PR_FORK_GONE, "{e:?}");
    // Another repository's address.
    let f = Fake::new();
    let other = PrInput { number: 5, repo: Some("someone/else".into()) };
    assert_eq!(checkout_pr(&f, &w.fork, "dev", &other, None, Changes::Ask).unwrap_err().code, codes::PR_OTHER_REPO);
    // Not signed in.
    let mut f = Fake::new();
    f.gh.push(("pr view".into(), 4, String::new(), "To get started with GitHub CLI, please run:  gh auth login".into()));
    assert_eq!(checkout_pr(&f, &w.fork, "dev", &input, None, Changes::Ask).unwrap_err().code, pcodes::GH_SIGNED_OUT);
    // The folder is untouched by all of it.
    assert_eq!(on(&w).kind, Kind::Default);
}

#[test]
fn a_pull_request_with_changes_in_the_way_asks_first() {
    let w = world();
    let mut f = with_pr(&w, 5, "fix-hud");
    f.gh.push(("pr view".into(), 0, open_view("Fix", "OPEN", false), String::new()));
    std::fs::write(w.fork.join("a"), "edit").unwrap();
    let input = PrInput { number: 5, repo: None };
    assert_eq!(checkout_pr(&f, &w.fork, "dev", &input, None, Changes::Ask).unwrap().status, Status::NeedsChoice);
    assert_eq!(on(&w).kind, Kind::Default);
    let o = checkout_pr(&f, &w.fork, "dev", &input, None, Changes::Stash).unwrap();
    assert!(o.stashed.is_some());
    assert_eq!(o.now.kind, Kind::Pr);
}

#[test]
fn describes_other_branches_in_plain_words() {
    assert_eq!(describe_other("pr/12"), "trying pull request #12");
    assert_eq!(describe_other("pr/12-abc1234"), "trying pull request #12");
    assert_eq!(describe_other("fix-hud"), "on branch fix-hud");
    assert_eq!(describe_other("HEAD"), "on a single commit (no branch)");
    assert_eq!(pr_number_of("pr/x"), None);
}

#[test]
fn git_problems_are_plain_never_raw() {
    let p = problem_of(&GitError::Command { cmd: "git fetch origin".into(), code: 128, stderr: "fatal: unable to access 'https://github.com/x/y.git/': Could not resolve host: github.com".into() });
    assert_eq!(p.code, codes::NO_NETWORK);
    assert!(!p.message.contains("fatal"));
    let p = problem_of(&GitError::Command { cmd: "git checkout x".into(), code: 1, stderr: "error: weird".into() });
    assert!(!p.message.contains("error: weird") && p.details.unwrap().contains("weird"));
}
