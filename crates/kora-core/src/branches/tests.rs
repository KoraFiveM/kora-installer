use super::*;
use crate::git::SystemRunner;
use crate::installer::registry::PortsRecord;
#[allow(unused_imports)]
use crate::installer::{Mode, Source, Versions};
use std::process::Command;

fn sh(dir: &Path, args: &[&str]) {
    let o =
        Command::new("git").args(args).current_dir(dir).env("GIT_AUTHOR_NAME", "t").env("GIT_AUTHOR_EMAIL", "t@t").env("GIT_COMMITTER_NAME", "t").env("GIT_COMMITTER_EMAIL", "t@t").output().unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
}

pub(crate) struct Srv {
    pub t: tempfile::TempDir,
    pub sf: ServerFile,
}

/// A Development server whose resources `a` and `b` are clones of their own upstream; only `a` has a `feature` branch.
pub(crate) fn server(mode: Mode) -> Srv {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("srv");
    let mut records = vec![];
    for (id, with_feature) in [("kora-a", true), ("kora-b", false)] {
        let up = t.path().join(format!("up-{id}"));
        std::fs::create_dir(&up).unwrap();
        sh(&up, &["init", "-q", "-b", "dev"]);
        std::fs::write(up.join("f"), "1").unwrap();
        sh(&up, &["add", "."]);
        sh(&up, &["commit", "-qm", "one"]);
        if with_feature {
            sh(&up, &["branch", "feature"]);
        }
        let path = format!("[kora]/{}", id.replace('-', "_"));
        let fork = dir.join("resources").join(&path);
        std::fs::create_dir_all(fork.parent().unwrap()).unwrap();
        sh(t.path(), &["clone", "-q", up.to_str().unwrap(), fork.to_str().unwrap()]);
        sh(&fork, &["remote", "add", "upstream", up.to_str().unwrap()]);
        sh(&fork, &["fetch", "-q", "upstream"]);
        sh(&fork, &["config", "user.email", "t@t"]);
        sh(&fork, &["config", "user.name", "t"]);
        records.push(ResourceRecord { id: id.into(), path, repository: "https://github.com/x/y.git".into(), branch: "dev".into(), ..Default::default() });
    }
    let sf = ServerFile {
        source: None,
        versions: None,
        name: "t".into(),
        mode,
        dir,
        ports: PortsRecord { game: 1, txadmin: 2 },
        game_build: 1,
        artifact_build: 1,
        database: None,
        github_owner: None,
        created_unix: 0,
        kora_version: "0".into(),
        resources: records,
    };
    Srv { t, sf }
}

fn path_of(s: &Srv, i: usize) -> PathBuf {
    s.sf.dir.join("resources").join(&s.sf.resources[i].path)
}

#[test]
fn overview_shows_each_resources_branch() {
    let s = server(Mode::Development);
    sh(&path_of(&s, 1), &["checkout", "-q", "-b", "pr/7"]);
    std::fs::write(path_of(&s, 0).join("f"), "edit").unwrap();
    let rows = overview(&SystemRunner, &s.sf).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!((rows[0].state, rows[0].changed_count, rows[0].label.as_str()), (State::Default, 1, "the normal version (dev)"));
    assert_eq!((rows[1].state, rows[1].pr, rows[1].label.as_str()), (State::Pr, Some(7), "pull request #7"));
}

#[test]
fn every_server_gets_branches_whatever_it_follows() {
    for mode in [Mode::Production, Mode::Development] {
        let s = server(mode);
        assert_eq!(overview(&SystemRunner, &s.sf).unwrap().len(), 2);
        assert!(choices(&SystemRunner, &s.sf, "kora-a", false).is_ok());
    }
}

#[test]
fn an_unknown_resource_and_an_escaping_folder_are_refused() {
    let mut s = server(Mode::Development);
    assert!(choices(&SystemRunner, &s.sf, "nope", false).is_err());
    s.sf.resources[0].path = "../../outside".into();
    let e = choices(&SystemRunner, &s.sf, "kora-a", false).unwrap_err();
    assert!(e.message.contains("not inside"), "{}", e.message);
    s.sf.resources[0].path = "/etc".into();
    assert!(switch(&SystemRunner, &s.sf, "kora-a", &Target::Default, Changes::Stash).is_err());
}

#[test]
fn a_link_out_of_the_resources_folder_is_not_followed() {
    #[cfg(unix)]
    {
        let mut s = server(Mode::Development);
        let outside = s.t.path().join("up-kora-a");
        let link = s.sf.dir.join("resources").join("link");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        s.sf.resources[0].path = "link".into();
        let e = choices(&SystemRunner, &s.sf, "kora-a", false).unwrap_err();
        assert!(e.message.contains("outside"), "{}", e.message);
    }
}

#[test]
fn switching_one_resource_asks_to_restart_and_never_during_an_update() {
    let s = server(Mode::Development);
    let t = Target::Branch { name: "feature".into(), remote: None };
    let held = lock::acquire(&s.sf.dir).unwrap();
    let e = switch(&SystemRunner, &s.sf, "kora-a", &t, Changes::Ask).unwrap_err();
    assert_eq!(e.code, codes::BUSY);
    drop(held);
    let r = switch(&SystemRunner, &s.sf, "kora-a", &t, Changes::Ask).unwrap();
    assert!(r.needs_restart && r.restart_note.is_some());
    assert_eq!(r.outcome.now.branch.as_deref(), Some("feature"));
    let c = choices(&SystemRunner, &s.sf, "kora-a", false).unwrap();
    assert_eq!(c.resource.state, State::Branch);
    assert!(c.branches.upstream.contains(&"feature".to_string()));
    // Nothing to restart when nothing moved.
    let again = switch(&SystemRunner, &s.sf, "kora-a", &t, Changes::Ask).unwrap();
    assert!(!again.needs_restart);
}

#[test]
fn bulk_preview_says_who_has_the_branch_and_bulk_apply_moves_only_those() {
    let s = server(Mode::Development);
    let p = bulk_preview(&SystemRunner, &s.sf, "feature", false, &mut |_, _, _| {}).unwrap();
    assert_eq!((p.have, p.missing), (1, 1));
    assert_eq!(p.rows[0].from.as_deref(), Some("origin"));
    assert!(!p.rows[1].has);
    // One resource has changes: it is not touched without a choice, the other one still moves.
    std::fs::write(path_of(&s, 0).join("f"), "edit").unwrap();
    let t = Target::Branch { name: "feature".into(), remote: None };
    let res = bulk_apply(&SystemRunner, &s.sf, &t, None, Changes::Ask, &mut |_, _, _| {}).unwrap();
    assert_eq!((res.switched, res.needs_choice, res.skipped, res.failed), (0, 1, 1, 0));
    assert_eq!(res.items[0].files, vec!["f"]);
    assert_eq!(res.items[1].status, BulkStatus::Skipped);
    let res = bulk_apply(&SystemRunner, &s.sf, &t, None, Changes::Stash, &mut |_, _, _| {}).unwrap();
    assert_eq!((res.switched, res.skipped), (1, 1));
    assert!(res.items[0].stashed.is_some() && res.needs_restart);
    // Back to default for all: only the one that moved changes.
    let res = bulk_apply(&SystemRunner, &s.sf, &Target::Default, None, Changes::Ask, &mut |_, _, _| {}).unwrap();
    assert_eq!((res.switched, res.items[1].status), (1, BulkStatus::Already));
    assert_eq!(overview(&SystemRunner, &s.sf).unwrap().iter().filter(|r| r.state == State::Default).count(), 2);
}

#[test]
fn bulk_apply_can_be_limited_and_reports_progress() {
    let s = server(Mode::Development);
    let t = Target::Branch { name: "feature".into(), remote: None };
    let mut seen = vec![];
    let res = bulk_apply(&SystemRunner, &s.sf, &t, Some(&["kora-a".to_string()]), Changes::Ask, &mut |d, n, id| seen.push((d, n, id.to_string()))).unwrap();
    assert_eq!(res.items.len(), 1);
    assert_eq!(seen, vec![(0, 1, String::new()), (1, 1, "kora-a".to_string())]);
    assert!(bulk_apply(&SystemRunner, &s.sf, &Target::Branch { name: "-x".into(), remote: None }, None, Changes::Ask, &mut |_, _, _| {}).is_err());
}

#[test]
fn a_resource_that_is_not_a_repository_does_not_stop_the_others() {
    let s = server(Mode::Development);
    let broken = path_of(&s, 1);
    std::fs::remove_dir_all(broken.join(".git")).unwrap();
    // Not its own repository any more (the temp folder is not inside another one, so git says "not a repository").
    let res = bulk_apply(&SystemRunner, &s.sf, &Target::Default, None, Changes::Ask, &mut |_, _, _| {}).unwrap();
    assert_eq!(res.items[0].status, BulkStatus::Already);
    assert_eq!(res.items[1].status, BulkStatus::Failed);
    assert_eq!(res.items[1].problem.as_ref().unwrap().code, codes::NOT_REPO);
    let rows = overview(&SystemRunner, &s.sf).unwrap();
    assert_eq!(rows[1].state, State::Unavailable);
}

// ---- how updates treat a resource that is not on its normal branch

mod updates {
    use super::*;
    use crate::installer::gitops::{update_server, GitCli};
    use crate::update::manual::{self, ChangeStatus, Scope};
    use crate::update::{HealthCheck, HealthOutcome, HealthRequest, SkipReason};

    struct Healthy;
    impl HealthCheck for Healthy {
        fn check(&self, _: &HealthRequest) -> HealthOutcome {
            HealthOutcome::Healthy
        }
        fn recover(&self, _: &HealthRequest) {}
    }

    /// Both upstreams get a new commit on `dev`.
    fn upstream_moves(s: &Srv) {
        for id in ["kora-a", "kora-b"] {
            let up = s.t.path().join(format!("up-{id}"));
            std::fs::write(up.join("f"), "2").unwrap();
            sh(&up, &["commit", "-qam", "two"]);
        }
    }

    fn read(s: &Srv, i: usize) -> String {
        std::fs::read_to_string(path_of(s, i).join("f")).unwrap()
    }

    #[test]
    fn the_scheduled_update_leaves_a_resource_on_another_branch_alone_and_says_so() {
        let s = server(Mode::Development);
        s.sf.save().unwrap();
        switch(&SystemRunner, &s.sf, "kora-a", &Target::Branch { name: "feature".into(), remote: None }, Changes::Ask).unwrap();
        sh(&path_of(&s, 1), &["checkout", "-q", "-b", "pr/12"]);
        upstream_moves(&s);
        let rep = update_server("t", &s.sf.dir, &s.sf, &GitCli::default(), &Healthy, &|| Some(0)).unwrap();
        // Nothing was pulled into either, and nothing needs attention.
        assert!(rep.updated.is_empty(), "{}", rep.summary());
        assert!(rep.needs_attention.is_empty(), "{:?}", rep.needs_attention);
        assert_eq!((read(&s, 0), read(&s, 1)), ("1".to_string(), "1".to_string()));
        let text = rep.summary();
        assert!(text.contains("on branch feature") && text.contains("trying pull request #12"), "{text}");
        assert!(!rep.has_problems());
        // Back to the normal version: the next run updates it like any other.
        bulk_apply(&SystemRunner, &s.sf, &Target::Default, None, Changes::Ask, &mut |_, _, _| {}).unwrap();
        let rep = update_server("t", &s.sf.dir, &s.sf, &GitCli::default(), &Healthy, &|| Some(0)).unwrap();
        assert_eq!(rep.updated.len(), 2, "{}", rep.summary());
        assert_eq!((read(&s, 0), read(&s, 1)), ("2".to_string(), "2".to_string()));
    }

    #[test]
    fn the_preview_shows_other_branch_and_it_is_not_a_pending_update() {
        let s = server(Mode::Development);
        s.sf.save().unwrap();
        switch(&SystemRunner, &s.sf, "kora-a", &Target::Branch { name: "feature".into(), remote: None }, Changes::Ask).unwrap();
        upstream_moves(&s);
        let p = manual::preview_with(&s.sf, Scope::All, &GitCli::default(), None).unwrap();
        let a = p.resources.iter().find(|c| c.id == "kora-a").unwrap();
        assert_eq!(a.status, ChangeStatus::OtherBranch);
        assert!(a.note.as_deref().unwrap().contains("on branch feature"), "{:?}", a.note);
        assert_eq!(a.from, a.to, "it does not move");
        assert_eq!(p.resources.iter().find(|c| c.id == "kora-b").unwrap().status, ChangeStatus::Clean);
        assert!(crate::api::badge_of(&p) != crate::api::UpdateBadge::NeedsAttention);
        assert!(manual::render_preview(&p).contains("kora-a: on branch feature"));
        // The person moves it after the preview: applying that preview still leaves it alone.
        let targets = manual::planned_targets(&p);
        switch(&SystemRunner, &s.sf, "kora-b", &Target::Branch { name: "dev".into(), remote: None }, Changes::Ask).unwrap();
        sh(&path_of(&s, 1), &["checkout", "-q", "-b", "mine"]);
        let rep = manual::apply_with(
            &s.sf,
            Scope::All,
            manual::ApplyOpts { targets: Some(targets), ..manual::ApplyOpts::new(manual::RestartPolicy::None) },
            &GitCli::default(),
            &Healthy,
            None,
            Some(0),
            100,
            &mut |_| {},
        )
        .unwrap();
        assert!(rep.updated.is_empty(), "{}", rep.summary());
        assert!(matches!(rep.skipped.iter().find(|(id, _)| id == "kora-b").map(|(_, r)| r), Some(SkipReason::OtherBranch(b)) if b == "mine"));
        assert_eq!(read(&s, 1), "1");
    }

    #[test]
    fn a_rollback_goes_back_to_the_branch_it_was_taken_on_before_it_resets() {
        let s = server(Mode::Development);
        s.sf.save().unwrap();
        upstream_moves(&s);
        let rep = update_server("t", &s.sf.dir, &s.sf, &GitCli::default(), &Healthy, &|| Some(0)).unwrap();
        assert_eq!(rep.updated.len(), 2);
        let snaps = crate::update::list_snapshots(&s.sf.dir);
        assert_eq!(snaps[0].1.entries[0].branch.as_deref(), Some("dev"));
        // Meanwhile the person put kora-a on `feature` and made a commit there.
        switch(&SystemRunner, &s.sf, "kora-a", &Target::Branch { name: "feature".into(), remote: None }, Changes::Ask).unwrap();
        std::fs::write(path_of(&s, 0).join("mine"), "x").unwrap();
        sh(&path_of(&s, 0), &["add", "."]);
        sh(&path_of(&s, 0), &["commit", "-qm", "my work"]);
        let feature_tip = String::from_utf8(Command::new("git").args(["rev-parse", "feature"]).current_dir(path_of(&s, 0)).output().unwrap().stdout).unwrap();
        manual::rollback_with(&s.sf, &snaps[0].0, &GitCli::default(), false).unwrap();
        // It is on dev again, at the old commit; `feature` was not touched.
        assert_eq!(read(&s, 0), "1");
        let now = gb_locate(&s, 0);
        assert_eq!(now, Some("dev".to_string()));
        let after = String::from_utf8(Command::new("git").args(["rev-parse", "feature"]).current_dir(path_of(&s, 0)).output().unwrap().stdout).unwrap();
        assert_eq!(feature_tip, after);
    }

    fn gb_locate(s: &Srv, i: usize) -> Option<String> {
        crate::git::branches::locate(&SystemRunner, &path_of(s, i), "dev").ok().and_then(|w| w.branch)
    }

    #[test]
    fn old_snapshots_without_a_branch_still_load_and_roll_back_as_before() {
        let j = r#"{"created_unix":1,"entries":[{"id":"a","path":"/a","commit":"abc"}]}"#;
        let snap: crate::update::Snapshot = serde_json::from_str(j).unwrap();
        assert_eq!(snap.entries[0].branch, None);
        assert!(!serde_json::to_string(&snap).unwrap().contains("branch"));
    }
}

#[test]
fn the_page_of_a_resource_works_in_both_modes() {
    for mode in [Mode::Development, Mode::Production] {
        let s = server(mode);
        let p = page(&SystemRunner, &s.sf, "kora-a").unwrap();
        assert_eq!((p.id.as_str(), p.default_branch.as_str(), p.branch.state), ("kora-a", "dev", State::Default));
        assert!(page(&SystemRunner, &s.sf, "nope").is_err());
    }
}

#[test]
fn changing_the_files_is_remembered_until_a_restart() {
    use crate::resourcestate as rs;
    let s = server(Mode::Development);
    assert!(!rs::info(&s.sf.dir, "kora-a", true).needed);
    switch(&SystemRunner, &s.sf, "kora-a", &Target::Branch { name: "feature".into(), remote: None }, Changes::Ask).unwrap();
    let i = rs::info(&s.sf.dir, "kora-a", true);
    assert!(i.needed, "{i:?}");
    assert!(i.why.unwrap().contains("feature"));
    // Another resource is not affected, and a restart of the server clears it.
    assert!(!rs::info(&s.sf.dir, "kora-b", true).needed);
    rs::mark_server_restarted(&s.sf.dir);
    assert!(!rs::info(&s.sf.dir, "kora-a", true).needed);
}

#[test]
fn on_released_versions_the_pinned_commit_is_the_normal_version() {
    let mut s = server(Mode::Production);
    let dir = path_of(&s, 0);
    let head = String::from_utf8(Command::new("git").args(["rev-parse", "HEAD"]).current_dir(&dir).output().unwrap().stdout).unwrap().trim().to_string();
    sh(&dir, &["checkout", "-q", "--detach", &head]);
    s.sf.resources[0].commit = Some(head.clone());
    // Detached at the pin: normal.
    let rows = overview(&SystemRunner, &s.sf).unwrap();
    assert_eq!((rows[0].state, rows[0].label.as_str()), (State::Default, format!("the released version ({})", &head[..7]).as_str()));
    // On a branch (even the dev branch): not normal.
    let r = switch(&SystemRunner, &s.sf, "kora-a", &Target::Branch { name: "feature".into(), remote: None }, Changes::Ask).unwrap();
    assert!(r.needs_restart);
    assert_eq!(overview(&SystemRunner, &s.sf).unwrap()[0].state, State::Branch);
    // One step back to exactly the pin, on no branch; again is a no-op.
    let back = switch(&SystemRunner, &s.sf, "kora-a", &Target::Default, Changes::Ask).unwrap();
    assert_eq!(back.outcome.status, Status::Switched, "{}", back.outcome.message);
    assert_eq!(overview(&SystemRunner, &s.sf).unwrap()[0].state, State::Default);
    let now = String::from_utf8(Command::new("git").args(["rev-parse", "HEAD"]).current_dir(&dir).output().unwrap().stdout).unwrap().trim().to_string();
    assert_eq!(now, head);
    let again = switch(&SystemRunner, &s.sf, "kora-a", &Target::Default, Changes::Ask).unwrap();
    assert_eq!(again.outcome.status, Status::Already);
}

#[test]
fn a_clone_has_only_origin_and_that_is_the_original() {
    // Only `origin`: it is reported as the original repository, not as "your fork".
    let s = server(Mode::Production);
    let dir = path_of(&s, 0);
    sh(&dir, &["remote", "remove", "upstream"]);
    sh(&dir, &["remote", "set-url", "origin", "https://github.com/KoraFiveM/kora_a.git"]);
    let (up, fork) = crate::git::branches::slugs_for_test(&SystemRunner, &dir);
    assert_eq!((up.as_deref(), fork), (Some("KoraFiveM/kora_a"), None));
}

#[test]
fn the_title_of_a_tried_pull_request_comes_with_its_row_and_only_for_that_number() {
    let s = server(Mode::Development);
    sh(&path_of(&s, 0), &["checkout", "-q", "-b", "pr/7"]);
    assert_eq!(overview(&SystemRunner, &s.sf).unwrap()[0].pr_title, None, "nothing stored yet");
    crate::resourcestate::set_pr_title(&s.sf.dir, "kora-a", 7, "Fix the HUD");
    let row = &overview(&SystemRunner, &s.sf).unwrap()[0];
    assert_eq!((row.pr, row.pr_title.as_deref()), (Some(7), Some("Fix the HUD")));
    // Trying another one: the old title is not shown for it.
    sh(&path_of(&s, 0), &["checkout", "-q", "-b", "pr/8"]);
    assert_eq!(overview(&SystemRunner, &s.sf).unwrap()[0].pr_title, None);
}
