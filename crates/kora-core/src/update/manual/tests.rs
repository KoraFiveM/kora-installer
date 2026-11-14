use super::*;
use crate::installer::gitops::GitCli;
use crate::installer::registry::{PortsRecord, ResourceRecord};
use crate::update::HealthOutcome;
use std::cell::RefCell;
use std::process::Command;

fn sh(dir: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

fn commit(repo: &Path, content: &str, msg: &str) -> String {
    std::fs::write(repo.join("a"), content).unwrap();
    sh(repo, &["add", "."]);
    sh(repo, &["commit", "-qm", msg]);
    sh(repo, &["rev-parse", "HEAD"])
}

fn head(p: &Path) -> String {
    sh(p, &["rev-parse", "HEAD"])
}

struct Health {
    outcome: HealthOutcome,
    checks: RefCell<Vec<HealthRequest>>,
    recovers: RefCell<u32>,
}
impl Health {
    fn ok() -> Self {
        Health { outcome: HealthOutcome::Healthy, checks: RefCell::new(vec![]), recovers: RefCell::new(0) }
    }
    fn bad() -> Self {
        Health { outcome: HealthOutcome::Unhealthy { reason: "boot failed".into(), failed: vec![] }, ..Health::ok() }
    }
}
impl HealthCheck for Health {
    fn check(&self, r: &HealthRequest) -> HealthOutcome {
        self.checks.borrow_mut().push(r.clone());
        self.outcome.clone()
    }
    fn recover(&self, _: &HealthRequest) {
        *self.recovers.borrow_mut() += 1;
    }
}

const RES: [(&str, &str); 3] = [("kora-chat", "[kora]/kora_chat"), ("kora-hud", "[kora]/kora_hud"), ("kora-core", "[kora]/kora_core")];

struct Env {
    _t: tempfile::TempDir,
    sf: ServerFile,
}

impl Env {
    fn path(&self, i: usize) -> PathBuf {
        self.sf.dir.join("resources").join(RES[i].1)
    }
    fn up(&self, i: usize) -> PathBuf {
        self.sf.dir.parent().unwrap().join(format!("up-{}", RES[i].0))
    }
}

fn server(mode: Mode, resources: Vec<ResourceRecord>, t: tempfile::TempDir, dir: PathBuf) -> Env {
    let sf = ServerFile {
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
        resources,
    };
    Env { _t: t, sf }
}

/// Development server: three forks, each one commit behind its upstream. Folder has spaces.
fn dev_env() -> Env {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("my server");
    let mut recs = vec![];
    for (id, path) in RES {
        let up = t.path().join(format!("up-{id}"));
        std::fs::create_dir(&up).unwrap();
        sh(&up, &["init", "-q", "-b", "dev"]);
        commit(&up, "1", "one");
        let fork = dir.join("resources").join(path);
        std::fs::create_dir_all(fork.parent().unwrap()).unwrap();
        sh(t.path(), &["clone", "-q", up.to_str().unwrap(), fork.to_str().unwrap()]);
        sh(&fork, &["remote", "add", "upstream", up.to_str().unwrap()]);
        commit(&up, "2", &format!("{id} two"));
        recs.push(ResourceRecord { id: id.into(), path: path.into(), repository: "https://github.com/x/y.git".into(), branch: "dev".into(), commit: None });
    }
    server(Mode::Development, recs, t, dir)
}

fn opts(restart: RestartPolicy, skip: &[&str]) -> ApplyOpts {
    ApplyOpts { restart, skip: skip.iter().map(|s| s.to_string()).collect() }
}

fn run(env: &Env, scope: Scope, o: ApplyOpts, h: &Health, players: Option<u32>, now: u64) -> (UpdateReport, Vec<UpdateEvent>) {
    let mut ev = vec![];
    let r = apply_with(&env.sf, scope, o, &GitCli::default(), h, None, players, now, &mut |e| ev.push(e)).unwrap();
    (r, ev)
}

#[test]
fn preview_compares_without_changing_anything() {
    let env = dev_env();
    let before: Vec<String> = (0..3).map(|i| head(&env.path(i))).collect();
    let p = preview_with(&env.sf, Scope::All, &GitCli::default(), None).unwrap();
    assert_eq!(p.new_release, None);
    assert_eq!(p.resources.len(), 3);
    let chat = &p.resources[0];
    assert_eq!((chat.status, chat.behind, chat.needs_restart), (ChangeStatus::Clean, 1, false));
    assert_eq!(chat.changelog, vec!["kora-chat two"]);
    assert_eq!(chat.from, before[0]);
    assert_eq!(chat.to, head(&env.up(0)));
    assert!(p.resources[2].needs_restart, "kora-core needs a restart");
    for i in 0..3 {
        assert_eq!(head(&env.path(i)), before[i]);
    }
    assert!(render_preview(&p).contains("needs a server restart"));
    assert!(snapshots(&env.sf).is_empty());
}

#[test]
fn preview_of_one_resource_accepts_underscore_ids_and_rejects_unknown() {
    let env = dev_env();
    let p = preview_with(&env.sf, Scope::Resources(vec!["kora_hud".into()]), &GitCli::default(), None).unwrap();
    assert_eq!(p.resources.len(), 1);
    assert_eq!(p.resources[0].id, "kora-hud");
    assert!(preview_with(&env.sf, Scope::Resources(vec!["nope".into()]), &GitCli::default(), None).is_err());
}

#[test]
fn apply_one_updates_only_that_resource_and_saves_a_snapshot() {
    let env = dev_env();
    let old: Vec<String> = (0..3).map(|i| head(&env.path(i))).collect();
    let (r, ev) = run(&env, Scope::Resources(vec!["kora-chat".into()]), opts(RestartPolicy::WhenQuiet, &[]), &Health::ok(), Some(0), 1000);
    assert_eq!(r.updated.len(), 1);
    assert_eq!(head(&env.path(0)), head(&env.up(0)));
    assert_eq!(head(&env.path(1)), old[1]);
    assert_eq!(head(&env.path(2)), old[2]);
    assert_eq!(exit_code(&r), 0);
    assert!(ev.contains(&UpdateEvent::Snapshot("1000".into())));
    assert_eq!(ev.last(), Some(&UpdateEvent::Done));
    let s = snapshots(&env.sf);
    assert_eq!(s.len(), 1);
    assert_eq!((s[0].id.as_str(), s[0].created_unix, s[0].resources.clone()), ("1000", 1000, vec!["kora-chat".to_string()]));
}

#[test]
fn apply_all_with_a_skip() {
    let env = dev_env();
    let old_hud = head(&env.path(1));
    let h = Health::ok();
    let (r, _) = run(&env, Scope::All, opts(RestartPolicy::FullRestart, &["kora_hud"]), &h, Some(0), 5);
    let ids: Vec<&str> = r.updated.iter().map(|u| u.id.as_str()).collect();
    assert_eq!(ids, vec!["kora-chat", "kora-core"]);
    assert_eq!(head(&env.path(1)), old_hud);
    assert_eq!(head(&env.path(2)), head(&env.up(2)));
    assert!(h.checks.borrow()[0].full_restart);
}

#[test]
fn dirty_and_diverged_are_skipped_never_merged() {
    let env = dev_env();
    std::fs::write(env.path(1).join("a"), "my edit").unwrap();
    commit(&env.path(0), "local", "my own commit");
    let old: Vec<String> = (0..2).map(|i| head(&env.path(i))).collect();
    let p = preview_with(&env.sf, Scope::All, &GitCli::default(), None).unwrap();
    assert_eq!(p.resources[0].status, ChangeStatus::Diverged);
    assert_eq!(p.resources[1].status, ChangeStatus::Dirty);
    assert!(render_preview(&p).contains("uncommitted changes of your own"));
    let (r, _) = run(&env, Scope::All, opts(RestartPolicy::None, &[]), &Health::ok(), Some(0), 7);
    assert_eq!(r.skipped.len(), 2);
    assert_eq!(exit_code(&r), 3);
    for i in 0..2 {
        assert_eq!(head(&env.path(i)), old[i]);
    }
    assert_eq!(std::fs::read_to_string(env.path(1).join("a")).unwrap(), "my edit");
}

#[test]
fn health_failure_rolls_back_to_the_snapshot() {
    let env = dev_env();
    let old: Vec<String> = (0..3).map(|i| head(&env.path(i))).collect();
    let h = Health::bad();
    let (r, ev) = run(&env, Scope::All, opts(RestartPolicy::FullRestart, &[]), &h, Some(0), 9);
    assert_eq!(r.rolled_back.len(), 3);
    assert!(r.updated.is_empty());
    assert_eq!(exit_code(&r), 1);
    for i in 0..3 {
        assert_eq!(head(&env.path(i)), old[i], "previous commit restored");
    }
    assert_eq!(*h.recovers.borrow(), 1);
    assert!(ev.iter().any(|e| matches!(e, UpdateEvent::Health(m) if m.contains("snapshot"))));
}

#[test]
fn manual_rollback_from_a_saved_snapshot() {
    let env = dev_env();
    let old: Vec<String> = (0..3).map(|i| head(&env.path(i))).collect();
    run(&env, Scope::All, opts(RestartPolicy::None, &[]), &Health::ok(), Some(0), 20);
    assert_ne!(head(&env.path(0)), old[0]);
    run(&env, Scope::All, opts(RestartPolicy::None, &[]), &Health::ok(), Some(0), 21);
    assert_eq!(snapshots(&env.sf).len(), 1, "second run had nothing to update, so no snapshot");
    rollback_with(&env.sf, "20", &GitCli::default()).unwrap();
    for i in 0..3 {
        assert_eq!(head(&env.path(i)), old[i]);
    }
    assert!(rollback_with(&env.sf, "99", &GitCli::default()).is_err());
    assert!(rollback_with(&env.sf, "../x", &GitCli::default()).is_err());
}

#[test]
fn restart_policy_none_never_restarts_and_when_quiet_defers_with_players() {
    let env = dev_env();
    let h = Health::ok();
    let (r, _) = run(&env, Scope::All, opts(RestartPolicy::WhenQuiet, &[]), &h, Some(4), 30);
    assert_eq!(r.deferred.len(), 1, "kora-core waits for an empty server");
    assert_eq!(r.updated.len(), 2);
    assert_eq!(exit_code(&r), 0);
    let h2 = Health::ok();
    let (r2, _) = run(&env, Scope::All, opts(RestartPolicy::None, &[]), &h2, Some(4), 31);
    assert_eq!(r2.updated.len(), 1);
    assert!(h2.checks.borrow().is_empty(), "no restart or health probe with --restart none");
}

// ---------------------------------------------------------------- production

fn catalog(latest: &str, commits: &[(&str, &str)]) -> Catalog {
    let versions: Vec<String> = commits.iter().map(|(v, c)| format!(r#"{{"version":"{v}","tag":"v{v}","commit":"{c}"}}"#)).collect();
    let json = format!(
        r#"{{"schemaVersion":1,"resources":[{{"id":"kora-chat","repository":"https://github.com/x/kora_chat.git","path":"[kora]/kora_chat","group":"kora","kind":"kora","branch":"main","latest":"{latest}","versions":[{}]}}]}}"#,
        versions.join(",")
    );
    Catalog::parse(&json).unwrap()
}

/// Production server: one resource pinned at c1, origin has c1, c2, c3.
fn prod_env() -> (Env, [String; 3]) {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("prod server");
    let up = t.path().join("up-kora-chat");
    std::fs::create_dir(&up).unwrap();
    sh(&up, &["init", "-q", "-b", "main"]);
    let c1 = commit(&up, "1", "one");
    let c2 = commit(&up, "2", "two");
    let c3 = commit(&up, "3", "three");
    let fork = dir.join("resources/[kora]/kora_chat");
    std::fs::create_dir_all(fork.parent().unwrap()).unwrap();
    sh(t.path(), &["clone", "-q", up.to_str().unwrap(), fork.to_str().unwrap()]);
    sh(&fork, &["checkout", "-q", "--detach", &c1]);
    let rec = ResourceRecord { id: "kora-chat".into(), path: "[kora]/kora_chat".into(), repository: "https://github.com/x/kora_chat.git".into(), branch: "main".into(), commit: Some(c1.clone()) };
    let env = server(Mode::Production, vec![rec], t, dir);
    env.sf.save().unwrap();
    (env, [c1, c2, c3])
}

#[test]
fn production_detects_a_new_release_and_moves_to_the_pinned_commit() {
    let (env, [c1, c2, _c3]) = prod_env();
    let cat = catalog("0.2.0", &[("0.2.0", &c2), ("0.1.0", &c1)]);
    let p = preview_with(&env.sf, Scope::All, &GitCli::default(), Some(&cat)).unwrap();
    assert_eq!(p.new_release.as_deref(), Some("0.2.0"));
    assert_eq!((p.resources[0].from.as_str(), p.resources[0].to.as_str()), (c1.as_str(), c2.as_str()));
    assert_eq!(p.resources[0].changelog, vec!["two"], "commit subjects between the pins");
    assert!(render_preview(&p).contains("A new Kora release is available: 0.2.0"));
    assert_eq!(head(&env.path(0)), c1, "preview moves nothing");

    let mut ev = vec![];
    let r = apply_with(&env.sf, Scope::All, opts(RestartPolicy::None, &[]), &GitCli::default(), &Health::ok(), Some(&cat), Some(0), 50, &mut |e| ev.push(e)).unwrap();
    assert_eq!(r.updated.len(), 1);
    assert_eq!(head(&env.path(0)), c2, "pinned commit, not the branch head");
    assert_eq!(ServerFile::load(&env.sf.dir).unwrap().resources[0].commit.as_deref(), Some(c2.as_str()));

    let again = preview_with(&env.sf, Scope::All, &GitCli::default(), Some(&cat)).unwrap();
    assert_eq!(again.new_release, None, "up to date now");
    assert_eq!(again.resources[0].status, ChangeStatus::UpToDate);

    let mut sf = env.sf.clone();
    sf.resources[0].commit = Some(c2.clone());
    rollback_with(&sf, "50", &GitCli::default()).unwrap();
    assert_eq!(head(&env.path(0)), c1);
    assert_eq!(ServerFile::load(&env.sf.dir).unwrap().resources[0].commit.as_deref(), Some(c1.as_str()));
}

#[test]
fn production_without_a_catalog_is_a_plain_error() {
    let (env, _) = prod_env();
    let e = preview_with(&env.sf, Scope::All, &GitCli::default(), None).unwrap_err();
    assert!(e.to_string().contains("catalog"));
}

#[test]
fn production_health_failure_goes_back_to_the_previous_pin() {
    let (env, [c1, c2, _]) = prod_env();
    let cat = catalog("0.2.0", &[("0.2.0", &c2), ("0.1.0", &c1)]);
    let mut ev = vec![];
    let r = apply_with(&env.sf, Scope::All, opts(RestartPolicy::FullRestart, &[]), &GitCli::default(), &Health::bad(), Some(&cat), Some(0), 60, &mut |e| ev.push(e)).unwrap();
    assert_eq!(r.rolled_back.len(), 1);
    assert_eq!(head(&env.path(0)), c1);
    assert_eq!(ServerFile::load(&env.sf.dir).unwrap().resources[0].commit.as_deref(), Some(c1.as_str()));
}
