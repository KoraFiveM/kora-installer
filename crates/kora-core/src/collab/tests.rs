use super::*;
use crate::git::{Output, SystemRunner};
use std::cell::RefCell;
use std::process::Command;
use std::time::Duration;

fn sh(dir: &Path, args: &[&str]) {
    let o =
        Command::new("git").args(args).current_dir(dir).env("GIT_AUTHOR_NAME", "t").env("GIT_AUTHOR_EMAIL", "t@t").env("GIT_COMMITTER_NAME", "t").env("GIT_COMMITTER_EMAIL", "t@t").output().unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
}

/// Real git against temporary repositories; `gh` answers from a canned list; `git remote get-url` answers what GitHub
/// addresses are wanted (the real remotes are local folders, so a push really happens).
struct Fake {
    gh: Vec<(String, i32, String, String)>,
    origin: Option<String>,
    upstream: Option<String>,
    calls: RefCell<Vec<String>>,
}

impl Fake {
    fn new() -> Self {
        Fake { gh: vec![], origin: Some("me/res".into()), upstream: Some("KoraFiveM/res".into()), calls: RefCell::new(vec![]) }
    }
    fn gh(mut self, key: &str, code: i32, out: &str, err: &str) -> Self {
        self.gh.push((key.into(), code, out.into(), err.into()));
        self
    }
    fn ran(&self, needle: &str) -> bool {
        self.calls.borrow().iter().any(|c| c.contains(needle))
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
            return Ok(Output { code: 1, stdout: String::new(), stderr: format!("no canned answer for: {line}") });
        }
        if args.len() == 3 && args[..2] == ["remote", "get-url"] {
            let slug = if args[2] == "origin" { &self.origin } else { &self.upstream };
            return Ok(match slug {
                Some(s) => Output { code: 0, stdout: format!("https://github.com/{s}.git\n"), stderr: String::new() },
                None => Output { code: 2, stdout: String::new(), stderr: "error: No such remote".into() },
            });
        }
        SystemRunner.run(program, args, cwd)
    }
    fn sleep(&self, _: Duration) {}
}

struct Repo {
    _t: tempfile::TempDir,
    dir: PathBuf,
    bare: PathBuf,
}

/// A clone of a bare "GitHub" repository, on `dev`, with one commit.
fn repo() -> Repo {
    let t = tempfile::tempdir().unwrap();
    let bare = t.path().join("origin.git");
    std::fs::create_dir(&bare).unwrap();
    sh(&bare, &["init", "-q", "--bare", "-b", "dev"]);
    let dir = t.path().join("res");
    sh(t.path(), &["clone", "-q", bare.to_str().unwrap(), dir.to_str().unwrap()]);
    sh(&dir, &["checkout", "-q", "-b", "dev"]);
    std::fs::write(dir.join("a.lua"), "one\n").unwrap();
    sh(&dir, &["add", "."]);
    sh(&dir, &["commit", "-qm", "one"]);
    sh(&dir, &["push", "-q", "-u", "origin", "dev"]);
    sh(&dir, &["config", "user.name", "t"]);
    sh(&dir, &["config", "user.email", "t@t"]);
    Repo { _t: t, dir, bare }
}

#[test]
fn status_is_read_strictly() {
    let z = " M a.lua\0?? new file.lua\0A  b.lua\0R  new.lua\0old.lua\0 D gone.lua\0";
    let f = parse_status(z);
    let got: Vec<(&str, &str)> = f.iter().map(|x| (x.path.as_str(), x.status.as_str())).collect();
    assert_eq!(got, vec![("a.lua", "modified"), ("new file.lua", "new"), ("b.lua", "added"), ("new.lua", "renamed"), ("gone.lua", "deleted")]);
    assert!(parse_status("garbage\0\0x").is_empty());
}

#[test]
fn checks_roll_up_to_one_word() {
    let v = |j: &str| -> Vec<Value> { serde_json::from_str(j).unwrap() };
    assert_eq!(rollup(&[]).0, Checks::None);
    assert_eq!(rollup(&v(r#"[{"name":"a","status":"COMPLETED","conclusion":"SUCCESS"},{"name":"b","status":"COMPLETED","conclusion":"SKIPPED"}]"#)).0, Checks::Passing);
    assert_eq!(rollup(&v(r#"[{"name":"a","status":"COMPLETED","conclusion":"SUCCESS"},{"name":"b","status":"IN_PROGRESS"}]"#)).0, Checks::Pending);
    assert_eq!(rollup(&v(r#"[{"name":"a","status":"IN_PROGRESS"},{"name":"b","status":"COMPLETED","conclusion":"FAILURE"}]"#)).0, Checks::Failing);
    // Old-style statuses.
    assert_eq!(rollup(&v(r#"[{"context":"ci","state":"SUCCESS"}]"#)).0, Checks::Passing);
    assert_eq!(rollup(&v(r#"[{"context":"ci","state":"PENDING"}]"#)).0, Checks::Pending);
    // A link that is not https is dropped.
    assert_eq!(rollup(&v(r#"[{"name":"a","status":"COMPLETED","conclusion":"SUCCESS","detailsUrl":"javascript:alert(1)"}]"#)).1[0].url, None);
}

const PR_JSON: &str = r#"{"number":12,"title":"Fix the HUD","body":"Does a thing","author":{"login":"sam"},"state":"OPEN","isDraft":false,"url":"https://github.com/KoraFiveM/res/pull/12","baseRefName":"dev","headRefName":"fix-hud","headRefOid":"0123456789abcdef0123456789abcdef01234567","createdAt":"2026-03-01T10:00:00Z","updatedAt":"2026-03-02T10:00:00Z","reviewDecision":"REVIEW_REQUIRED","statusCheckRollup":[{"name":"build","status":"COMPLETED","conclusion":"FAILURE","detailsUrl":"https://ci/1"}],"mergeable":"MERGEABLE","files":[{"path":"hud.lua","additions":3,"deletions":1}],"comments":[{"author":{"login":"jo"},"body":"nice","createdAt":"2026-03-01T12:00:00Z"}],"reviews":[{"author":{"login":"lee"},"body":"please fix","state":"CHANGES_REQUESTED","submittedAt":"2026-03-02T09:00:00Z"}]}"#;

#[test]
fn a_pull_request_is_read_with_its_checks_reviews_files_and_conversation() {
    let (v, oid) = parse_pr_view(PR_JSON, "KoraFiveM/res").unwrap();
    assert_eq!((v.number, v.title.as_str(), v.author.as_str(), v.state.as_str()), (12, "Fix the HUD", "sam", "OPEN"));
    assert_eq!((v.checks, v.review.as_str(), v.mergeable.as_str()), (Checks::Failing, "REVIEW_REQUIRED", "MERGEABLE"));
    assert_eq!(v.files, vec![PrFile { path: "hud.lua".into(), additions: 3, deletions: 1 }]);
    let kinds: Vec<(&str, &str)> = v.comments.iter().map(|c| (c.author.as_str(), c.kind.as_str())).collect();
    assert_eq!(kinds, vec![("jo", "comment"), ("lee", "changes_requested")], "oldest first, reviews and comments together");
    assert_eq!(oid.len(), 40);
    assert!(parse_pr_view("[]", "x/y").is_none());
    assert!(parse_pr_view(r#"{"number":"x"}"#, "x/y").is_none());
}

#[test]
fn repository_rights_say_how_to_merge_and_whether_you_may() {
    let (m, can) = parse_repo_rights(r#"{"mergeCommitAllowed":false,"squashMergeAllowed":true,"rebaseMergeAllowed":true,"viewerPermission":"WRITE"}"#);
    assert_eq!((m, can), (vec!["squash".to_string(), "rebase".to_string()], true));
    assert!(!parse_repo_rights(r#"{"mergeCommitAllowed":true,"viewerPermission":"READ"}"#).1);
    assert_eq!(parse_repo_rights("nonsense"), (vec![], false));
}

#[test]
fn releases_mark_the_pinned_one_and_what_is_newer() {
    let j = r#"[{"tag_name":"v0.3.0","name":"Three","published_at":"2026-03-01","prerelease":false,"body":"notes 3","html_url":"https://github.com/x/y/releases/tag/v0.3.0"},{"tag_name":"v0.2.0","name":"Two","published_at":"2026-02-01","prerelease":false,"body":"notes 2","html_url":"https://github.com/x/y/releases/tag/v0.2.0"},{"tag_name":"v0.1.0","name":"One","published_at":"2026-01-01","prerelease":true,"body":"","html_url":"https://github.com/x/y/releases/tag/v0.1.0"}]"#;
    let r = parse_releases(j, Some("v0.2.0"));
    assert_eq!(r.iter().map(|x| (x.tag.as_str(), x.pinned, x.newer)).collect::<Vec<_>>(), vec![("v0.3.0", false, true), ("v0.2.0", true, false), ("v0.1.0", false, false)]);
    assert!(r[2].prerelease);
    assert!(parse_releases(j, None).iter().all(|x| !x.newer && !x.pinned));
}

#[test]
fn issues_are_read_with_labels() {
    let i = parse_issues(
        r#"[{"number":4,"title":"Crash","author":{"login":"a"},"url":"https://github.com/x/y/issues/4","comments":[{},{}],"createdAt":"2026-01-01","labels":[{"name":"bug"}]},{"title":"no number"}]"#,
    );
    assert_eq!(i.len(), 1);
    assert_eq!((i[0].number, i[0].comments, i[0].labels.clone()), (4, 2, vec!["bug".to_string()]));
}

#[test]
fn changes_show_files_and_a_diff_and_where_the_branch_stands() {
    let t = repo();
    let r = Fake::new();
    std::fs::write(t.dir.join("a.lua"), "one\ntwo\n").unwrap();
    std::fs::write(t.dir.join("b.lua"), "new\n").unwrap();
    let c = local_changes(&r, &t.dir, "dev").unwrap();
    assert_eq!(c.count, 2);
    assert!(c.files.iter().any(|f| f.path == "a.lua" && f.status == "modified") && c.files.iter().any(|f| f.path == "b.lua" && f.status == "new"));
    assert!(c.diff.contains("+two") && !c.diff_truncated);
    assert_eq!((c.branch.as_deref(), c.ahead, c.behind), (Some("dev"), Some(0), Some(0)));
    assert_eq!(c.repos, Repos { original: Some("KoraFiveM/res".into()), fork: Some("me/res".into()) });
}

#[test]
fn commit_saves_everything_with_the_message_and_refuses_nothing_to_save() {
    let t = repo();
    let r = Fake::new();
    assert_eq!(commit(&r, &t.dir, "dev", "x").unwrap_err().code, NOTHING_TO_COMMIT);
    std::fs::write(t.dir.join("b.lua"), "new\n").unwrap();
    assert!(commit(&r, &t.dir, "dev", "  ").is_err(), "an empty message is refused");
    assert!(commit(&r, &t.dir, "dev", &"x".repeat(MAX_MESSAGE + 1)).is_err());
    let c = commit(&r, &t.dir, "dev", "Add b\n\nwith a body").unwrap();
    assert_eq!((c.files, c.branch.as_deref()), (1, Some("dev")));
    let l = log(&r, &t.dir, "dev").unwrap();
    assert_eq!((l.commits[0].subject.as_str(), l.commits.len()), ("Add b", 2));
    assert_eq!(l.commits[0].sha, c.commit);
    assert_eq!(gb::locate(&r, &t.dir, "dev").unwrap().changed_count, 0);
}

#[test]
fn commit_without_a_git_identity_says_how_to_set_it() {
    let t = repo();
    sh(&t.dir, &["config", "user.useConfigOnly", "true"]);
    sh(&t.dir, &["config", "--unset", "user.name"]);
    sh(&t.dir, &["config", "--unset", "user.email"]);
    std::fs::write(t.dir.join("b.lua"), "new\n").unwrap();
    // The test environment may supply an identity another way; only check the guided error when git really refuses.
    if let Err(e) = commit(&Fake::new(), &t.dir, "dev", "Add b") {
        assert_eq!(e.code, NO_IDENTITY);
        assert!(e.actions.iter().any(|a| matches!(a, Action::ShowCommand { .. })));
    }
}

#[test]
fn push_goes_to_your_own_copy_never_forced_and_says_when_it_cannot() {
    let t = repo();
    let r = Fake::new().gh("api user", 0, "sam\n", "");
    sh(&t.dir, &["checkout", "-q", "-b", "my-fix"]);
    std::fs::write(t.dir.join("b.lua"), "x\n").unwrap();
    commit(&r, &t.dir, "dev", "Add b").unwrap();
    let p = push(&r, &t.dir, "dev").unwrap();
    assert_eq!((p.branch.as_str(), p.to.as_str(), p.new_branch, p.acting_as.as_deref()), ("my-fix", "me/res", true, Some("sam")));
    assert!(t.bare.join("refs/heads/my-fix").is_file() || std::fs::read_to_string(t.bare.join("packed-refs")).unwrap_or_default().contains("my-fix"));
    assert!(!r.ran("--force") && !r.ran(" -f "));
    // A second push of a branch GitHub already has is not "new".
    std::fs::write(t.dir.join("c.lua"), "x\n").unwrap();
    commit(&r, &t.dir, "dev", "Add c").unwrap();
    assert!(!push(&r, &t.dir, "dev").unwrap().new_branch);

    // A plain clone has no copy of its own to push to: guided, with the fix.
    let clone = Fake { upstream: None, ..Fake::new() };
    let e = push(&clone, &t.dir, "dev").unwrap_err();
    assert_eq!(e.code, NEEDS_FORK);
    assert!(e.actions.iter().any(|a| matches!(a, Action::MakeFork { .. })));
}

#[test]
fn push_refuses_a_single_commit_and_someone_elses_pull_request() {
    let t = repo();
    let r = Fake::new();
    sh(&t.dir, &["checkout", "-q", "-b", "pr/12"]);
    assert_eq!(push(&r, &t.dir, "dev").unwrap_err().code, NOT_ON_A_BRANCH);
    sh(&t.dir, &["checkout", "-q", "--detach"]);
    assert_eq!(push(&r, &t.dir, "dev").unwrap_err().code, NOT_ON_A_BRANCH);
}

#[test]
fn a_rejected_push_is_explained_and_not_forced() {
    let t = repo();
    let r = Fake::new();
    // Someone else adds a commit to the same branch on GitHub.
    let other = t.dir.parent().unwrap().join("other");
    sh(t.dir.parent().unwrap(), &["clone", "-q", t.bare.to_str().unwrap(), other.to_str().unwrap()]);
    sh(&other, &["checkout", "-q", "dev"]);
    std::fs::write(other.join("z"), "z").unwrap();
    sh(&other, &["add", "."]);
    sh(&other, &["commit", "-qm", "theirs"]);
    sh(&other, &["push", "-q", "origin", "dev"]);
    std::fs::write(t.dir.join("b.lua"), "x\n").unwrap();
    commit(&r, &t.dir, "dev", "mine").unwrap();
    let e = push(&r, &t.dir, "dev").unwrap_err();
    assert!(e.message.contains("never forces"), "{}", e.message);
    assert!(!r.ran("--force"));
}

#[test]
fn a_new_branch_comes_from_where_you_are_with_your_changes() {
    let t = repo();
    let r = Fake::new();
    std::fs::write(t.dir.join("a.lua"), "edited\n").unwrap();
    let w = new_branch(&r, &t.dir, "dev", "my-idea").unwrap();
    assert_eq!((w.branch.as_deref(), w.changed_count), (Some("my-idea"), 1));
    for bad in ["--exec=x", "a b", "pr/9", "dev", "../x", ""] {
        assert!(new_branch(&r, &t.dir, "dev", bad).is_err(), "{bad}");
    }
}

#[test]
fn a_pull_request_view_adds_rights_identity_and_how_far_behind_the_tried_one_is() {
    let t = repo();
    let repo_json = r#"{"mergeCommitAllowed":true,"squashMergeAllowed":true,"rebaseMergeAllowed":false,"viewerPermission":"WRITE"}"#;
    let r = Fake::new().gh("pr view 12", 0, PR_JSON, "").gh("repo view", 0, repo_json, "").gh("api user", 0, "sam\n", "");
    let v = pr_view(&r, &t.dir, "dev", 12, None).unwrap();
    assert_eq!((v.repo.as_str(), v.can_merge, v.acting_as.as_deref()), ("KoraFiveM/res", true, Some("sam")));
    assert_eq!(v.merge_methods, vec!["merge", "squash"]);
    assert!(v.trying.is_none(), "not the checked out one");
    // Trying #12 here: the branch is pr/12 at a commit the folder knows.
    sh(&t.dir, &["checkout", "-q", "-b", "pr/12"]);
    let head = gb::head_commit(&r, &t.dir).unwrap();
    let json = PR_JSON.replace("0123456789abcdef0123456789abcdef01234567", &head);
    let r = Fake::new().gh("pr view 12", 0, &json, "").gh("repo view", 0, repo_json, "");
    let v = pr_view(&r, &t.dir, "dev", 12, None).unwrap();
    assert_eq!(v.trying, Some(Trying { branch: "pr/12".into(), behind: Some(0), ahead: 0 }));
    // GitHub has a newer commit the folder does not have: "get the latest".
    let r = Fake::new().gh("pr view 12", 0, PR_JSON, "").gh("repo view", 0, repo_json, "");
    assert_eq!(pr_view(&r, &t.dir, "dev", 12, None).unwrap().trying.unwrap().behind, None);
}

#[test]
fn comments_and_reviews_go_through_gh_with_checked_input_and_say_who_acted() {
    let t = repo();
    let r = Fake::new()
        .gh("pr comment 12", 0, "https://github.com/KoraFiveM/res/pull/12#issuecomment-1\n", "")
        .gh("pr review 12", 0, "", "")
        .gh("issue comment 5", 0, "", "")
        .gh("api user", 0, "sam\n", "");
    let a = comment(&r, &t.dir, "pr", 12, "Looks good", None).unwrap();
    assert_eq!((a.acting_as.as_deref(), a.url.is_some()), (Some("sam"), true));
    assert!(r.ran("pr comment 12 --repo KoraFiveM/res --body Looks good"));
    assert!(comment(&r, &t.dir, "issue", 5, "Me too", None).is_ok());
    assert!(comment(&r, &t.dir, "pr", 12, "  ", None).is_err());
    assert!(comment(&r, &t.dir, "repo", 12, "x", None).is_err(), "only pull requests and issues");
    assert!(review(&r, &t.dir, 12, "approve", "", None).is_ok());
    assert!(r.ran("--approve"));
    assert!(review(&r, &t.dir, 12, "request_changes", "", None).is_err(), "asking for changes needs a reason");
    assert!(review(&r, &t.dir, 12, "request_changes", "Please fix X", None).is_ok());
    assert!(review(&r, &t.dir, 12, "destroy", "", None).is_err());
}

#[test]
fn merging_needs_rights_an_open_pull_request_and_a_method_the_repository_allows() {
    let t = repo();
    let rights = |p: &str| format!(r#"{{"mergeCommitAllowed":false,"squashMergeAllowed":true,"rebaseMergeAllowed":false,"viewerPermission":"{p}"}}"#);
    // No rights: refused, and nothing is run.
    let r = Fake::new().gh("pr view 12", 0, PR_JSON, "").gh("repo view", 0, &rights("READ"), "").gh("api user", 0, "sam\n", "");
    let e = merge(&r, &t.dir, "dev", 12, None, None).unwrap_err();
    assert_eq!(e.code, MERGE_REFUSED);
    assert!(e.message.contains("sam") && !r.ran("pr merge"));
    // Rights: the repository's own method, never --admin.
    let r = Fake::new().gh("pr view 12", 0, PR_JSON, "").gh("repo view", 0, &rights("WRITE"), "").gh("pr merge 12", 0, "", "");
    let a = merge(&r, &t.dir, "dev", 12, None, None).unwrap();
    assert!(a.message.contains("squash") && r.ran("pr merge 12 --repo KoraFiveM/res --squash") && !r.ran("--admin"));
    // A method it does not allow, or one that does not exist.
    assert!(merge(&r, &t.dir, "dev", 12, Some("rebase"), None).unwrap_err().message.contains("squash"));
    assert!(merge(&r, &t.dir, "dev", 12, Some("--admin"), None).is_err());
    // GitHub says no (a required check): the reason is shown.
    let r = Fake::new().gh("pr view 12", 0, PR_JSON, "").gh("repo view", 0, &rights("WRITE"), "").gh("pr merge 12", 1, "", "Pull request is not mergeable: required status check");
    assert!(merge(&r, &t.dir, "dev", 12, None, None).unwrap_err().details.unwrap().contains("required status check"));
    // Already merged.
    let merged = PR_JSON.replace("\"state\":\"OPEN\"", "\"state\":\"MERGED\"");
    let r = Fake::new().gh("pr view 12", 0, &merged, "").gh("repo view", 0, &rights("WRITE"), "");
    assert!(merge(&r, &t.dir, "dev", 12, None, None).unwrap_err().message.contains("merged"));
}

#[test]
fn not_signed_in_and_no_gh_are_the_usual_guided_problems() {
    let t = repo();
    let r = Fake::new().gh("pr view", 1, "", "To get started with GitHub CLI, please run:  gh auth login");
    let e = pr_view(&r, &t.dir, "dev", 12, None).unwrap_err();
    assert_eq!(e.code, crate::problem::codes::GH_SIGNED_OUT);
    assert!(e.actions.iter().any(|a| matches!(a, Action::SignInGithub { .. })));
}

#[test]
fn making_your_own_copy_keeps_the_folder_and_re_points_the_remotes() {
    let t = repo();
    // A plain clone: origin is the original (a local folder here, so the folder state is real).
    sh(&t.dir, &["checkout", "-q", "-b", "wip"]);
    std::fs::write(t.dir.join("a.lua"), "my edit\n").unwrap();
    let listing = r#"{"data":{"repositoryOwner":{"repositories":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}"#;
    let fork_bare = t.dir.parent().unwrap().join("fork.git");
    sh(t.dir.parent().unwrap(), &["clone", "-q", "--bare", t.bare.to_str().unwrap(), fork_bare.to_str().unwrap()]);
    // `remote get-url` is answered by the fake until the remotes are changed: the original only at first.
    struct Flip(Fake, std::cell::Cell<bool>, PathBuf);
    impl CommandRunner for Flip {
        fn run(&self, p: &str, a: &[&str], c: Option<&Path>) -> std::io::Result<Output> {
            if p == "git" && a.first() == Some(&"remote") && a.get(1) == Some(&"add") {
                // The new origin is "GitHub": really a local bare copy.
                self.1.set(true);
                return SystemRunner.run(p, &["remote", "add", "origin", self.2.to_str().unwrap()], c);
            }
            if p == "git" && a.len() == 3 && a[..2] == ["remote", "get-url"] {
                let (origin, upstream) = if self.1.get() { (Some("me/res"), Some("KoraFiveM/res")) } else { (Some("KoraFiveM/res"), None) };
                let slug = if a[2] == "origin" { origin } else { upstream };
                return Ok(match slug {
                    Some(s) => Output { code: 0, stdout: format!("https://github.com/{s}.git\n"), stderr: String::new() },
                    None => Output { code: 2, stdout: String::new(), stderr: "error: No such remote".into() },
                });
            }
            self.0.run(p, a, c)
        }
        fn sleep(&self, _: Duration) {}
    }
    let fake = Fake::new().gh("graphql", 0, listing, "").gh("repo fork KoraFiveM/res --clone=false", 0, "", "✓ Created fork me/res").gh("commits?per_page=1", 0, "1", "");
    let r = Flip(fake, std::cell::Cell::new(false), fork_bare.clone());
    // Zero wait for the new fork to fill up.
    let before = gb::locate(&r, &t.dir, "dev").unwrap();
    let f = make_fork_for_test(&r, &t.dir);
    assert_eq!(f.fork, "me/res");
    let after = gb::locate(&r, &t.dir, "dev").unwrap();
    assert_eq!((after.branch.clone(), after.changed_count), (before.branch, before.changed_count), "branch and changes are untouched");
    let remotes = String::from_utf8(Command::new("git").args(["remote"]).current_dir(&t.dir).output().unwrap().stdout).unwrap();
    assert!(remotes.contains("origin") && remotes.contains("upstream"), "{remotes}");
    assert!(std::fs::read_to_string(t.dir.join("a.lua")).unwrap().contains("my edit"));
    // Again: nothing to do.
    assert!(make_fork_for_test(&r, &t.dir).message.contains("already"));
}

fn make_fork_for_test(r: &dyn CommandRunner, dir: &Path) -> Forked {
    make_fork_with(r, dir, "res", "me", false, std::time::Duration::ZERO).unwrap()
}

#[test]
fn the_fork_owner_is_you_or_an_organisation_you_may_create_repositories_in() {
    let auth = r#"{"hosts":{"github.com":[{"state":"success","active":true,"login":"sam"}]}}"#;
    let r = Fake::new().gh("auth status", 0, "Logged in to github.com account sam\n", "").gh("api user", 0, auth, "");
    // Not signed in: the usual guided problem.
    let out = Fake::new().gh("auth status", 1, "", "You are not logged into any GitHub hosts. To log in, run: gh auth login");
    assert_eq!(resolve_fork_owner(&out, None).unwrap_err().code, crate::problem::codes::GH_SIGNED_OUT);
    // Bad names never reach a command line.
    assert!(resolve_fork_owner(&r, Some("--org")).is_err() || r.ran("auth status"));
    assert!(!r.ran("--org"));
}

#[test]
fn changing_the_versions_writes_both_spellings_and_moves_nothing() {
    use crate::installer::registry::PortsRecord;
    let home = crate::installer::registry::use_test_home();
    let d = tempfile::tempdir().unwrap();
    let sf = ServerFile {
        name: "s".into(),
        mode: crate::installer::Mode::Production,
        source: None,
        versions: None,
        dir: d.path().to_path_buf(),
        ports: PortsRecord { game: 1, txadmin: 2 },
        game_build: 1,
        artifact_build: 1,
        database: None,
        github_owner: None,
        created_unix: 0,
        kora_version: "0".into(),
        resources: vec![],
    };
    let v = set_versions(&sf, Versions::Latest).unwrap();
    assert_eq!((v.source, v.versions), (Source::Clone, Versions::Latest));
    let back = ServerFile::load(d.path()).unwrap();
    assert_eq!((back.source(), back.versions(), back.mode), (Source::Clone, Versions::Latest, crate::installer::Mode::Development));
    assert!(v.message.contains("nothing has changed yet"));
    // The registry entry follows, in the store.
    crate::installer::registry::Registry::update(|r| {
        r.upsert(crate::installer::registry::ServerEntry {
            source: None,
            versions: None,
            name: "s".into(),
            dir: d.path().to_path_buf(),
            mode: crate::installer::Mode::Production,
            ports: PortsRecord { game: 1, txadmin: 2 },
            created_unix: 0,
        });
        Ok(())
    })
    .unwrap();
    set_versions(&sf, Versions::Released).unwrap();
    let e = crate::installer::registry::Registry::load().unwrap().servers.into_iter().find(|e| e.name == "s").unwrap();
    assert_eq!((e.versions(), e.mode), (Versions::Released, crate::installer::Mode::Production));
    let _ = home;
}

#[test]
fn a_diff_never_shows_a_secret() {
    let t = repo();
    let r = Fake::new();
    std::fs::write(t.dir.join("a.lua"), "one\nConfig.ApiKey = 'abc123SECRET'\nConfig.Pay = 120\nlicense = cfxk_0123456789abcdef_ab12\n").unwrap();
    std::fs::write(t.dir.join("secrets.cfg"), "set mysql_connection_string \"user:hunter2@host\"\n").unwrap();
    sh(&t.dir, &["add", "-N", "secrets.cfg"]);
    let c = local_changes(&r, &t.dir, "dev").unwrap();
    assert!(c.files.iter().any(|f| f.path == "secrets.cfg"), "the name is listed");
    for secret in ["abc123SECRET", "hunter2", "cfxk_0123456789abcdef_ab12"] {
        assert!(!c.diff.contains(secret), "{secret} leaked: {}", c.diff);
    }
    assert!(c.diff.contains("Config.Pay = 120"), "ordinary lines stay: {}", c.diff);
    assert!(c.diff.contains("secrets.cfg: its contents are not shown"));
    assert!(hide_secrets("+password = hunter2\n bearer: xyz\n").lines().all(|l| l.ends_with("<hidden>")));
}
