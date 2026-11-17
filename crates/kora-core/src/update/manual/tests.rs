use super::*;
use crate::installer::gitops::GitCli;
use crate::installer::registry::{PortsRecord, ResourceRecord};
use crate::update::HealthOutcome;
use std::cell::RefCell;
use std::path::PathBuf;
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
        recs.push(ResourceRecord { id: id.into(), path: path.into(), repository: "https://github.com/x/y.git".into(), branch: "dev".into(), commit: None, third_party: false, ..Default::default() });
    }
    server(Mode::Development, recs, t, dir)
}

fn opts(restart: RestartPolicy, skip: &[&str]) -> ApplyOpts {
    ApplyOpts {
        restart,
        skip: skip.iter().map(|s| s.to_string()).collect(),
        targets: None,
    }
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
    assert_eq!((0..3).map(|i| head(&env.path(i))).collect::<Vec<_>>(), before[..3]);
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
fn paid_portal_assets_are_never_updated() {
    let mut env = dev_env();
    let paid = env.sf.dir.join("resources/[paid]/qs-housing");
    std::fs::create_dir_all(&paid).unwrap();
    std::fs::write(paid.join("fxmanifest.lua"), "fx_version 'cerulean'\n").unwrap();
    env.sf.resources.push(ResourceRecord {
        id: "qs-housing".into(),
        path: "[paid]/qs-housing".into(),
        source: Some(crate::paidassets::SOURCE.into()),
        version: Some("1.0".into()),
        ..Default::default()
    });
    let p = preview_with(&env.sf, Scope::All, &GitCli::default(), None).unwrap();
    assert_eq!(p.resources.len(), 3, "the paid asset is not part of an update");
    assert!(p.resources.iter().all(|c| c.id != "qs-housing"));
    let e = preview_with(&env.sf, Scope::Resources(vec!["qs-housing".into()]), &GitCli::default(), None).unwrap_err();
    assert!(e.to_string().contains("paid asset"), "{e}");
    let (r, _) = run(&env, Scope::All, opts(RestartPolicy::WhenQuiet, &[]), &Health::ok(), Some(0), 1000);
    assert!(r.updated.iter().all(|u| u.id != "qs-housing"));
    assert!(paid.join("fxmanifest.lua").is_file(), "left alone");
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
    assert_eq!((0..2).map(|i| head(&env.path(i))).collect::<Vec<_>>(), old[..2]);
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
    assert_eq!((0..3).map(|i| head(&env.path(i))).collect::<Vec<_>>(), old[..3]);
    assert_eq!(*h.recovers.borrow(), 1);
    assert!(ev.iter().any(|e| matches!(e, UpdateEvent::Health(m) if m.contains("snapshot"))));
}

#[test]
fn manual_rollback_from_a_saved_snapshot() {
    let env = dev_env();
    let old: Vec<String> = (0..3).map(|i| head(&env.path(i))).collect();
    run(&env, Scope::All, opts(RestartPolicy::None, &[]), &Health::ok(), Some(0), 20);
    assert_ne!(head(&env.path(0)), old[0]);
    run(
        &env,
        Scope::All,
        opts(RestartPolicy::None, &[]),
        &Health::ok(),
        Some(0),
        21,
    );
    assert_eq!(
        snapshots(&env.sf).len(),
        1,
        "second run had nothing to update, so no snapshot"
    );
    rollback_with(&env.sf, "20", &GitCli::default(), false).unwrap();
    assert_eq!(
        (0..3).map(|i| head(&env.path(i))).collect::<Vec<_>>(),
        old[..3]
    );
    assert!(rollback_with(&env.sf, "99", &GitCli::default(), false).is_err());
    assert!(rollback_with(&env.sf, "../x", &GitCli::default(), false).is_err());
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
    let rec = ResourceRecord { id: "kora-chat".into(), path: "[kora]/kora_chat".into(), repository: "https://github.com/x/kora_chat.git".into(), branch: "main".into(), commit: Some(c1.clone()), third_party: false, ..Default::default() };
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
    rollback_with(&sf, "50", &GitCli::default(), false).unwrap();
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

fn status_of(env: &Env) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(env.sf.dir.join(".kora/update-status.json")).unwrap()).unwrap()
}

#[test]
fn events_stream_per_resource_in_order_from_inside_the_engine() {
    let env = dev_env();
    let (_, ev) = run(&env, Scope::All, opts(RestartPolicy::FullRestart, &[]), &Health::ok(), Some(0), 30);
    let pos = |e: &UpdateEvent| ev.iter().position(|x| x == e).unwrap_or_else(|| panic!("missing {e:?} in {ev:?}"));
    let snap = pos(&UpdateEvent::Snapshot("30".into()));
    let upd = |id: &str, st: &str| pos(&UpdateEvent::Resource { id: id.into(), state: st.into() });
    // each resource is announced, then done, before the next one starts; all after the snapshot
    assert!(snap < upd("kora-chat", "updating"));
    assert!(upd("kora-chat", "updating") < upd("kora-chat", "updated"));
    assert!(upd("kora-chat", "updated") < upd("kora-hud", "updating"), "streamed one by one: {ev:?}");
    let health = ev.iter().position(|e| matches!(e, UpdateEvent::Health(m) if m.contains("checking"))).unwrap();
    assert!(upd("kora-core", "updated") < health);
    assert_eq!(ev.last(), Some(&UpdateEvent::Done));
}

#[test]
fn a_full_run_writes_the_status_file_for_the_manager() {
    let env = dev_env();
    preview_with(&env.sf, Scope::All, &GitCli::default(), None).unwrap();
    let v = status_of(&env);
    assert_eq!((v["updates"].clone(), v["attention"].clone()), (serde_json::json!(3), serde_json::json!(0)));
    assert!(v["checked_at"].as_str().unwrap().ends_with('Z') && v["new_release"].is_null());
    run(&env, Scope::All, opts(RestartPolicy::None, &[]), &Health::ok(), Some(0), 31);
    let v = status_of(&env);
    assert_eq!((v["updates"].clone(), v["attention"].clone()), (serde_json::json!(0), serde_json::json!(0)));
}

#[test]
fn status_counts_attention_and_waiting() {
    let env = dev_env();
    std::fs::write(env.path(1).join("a"), "my edit").unwrap();
    run(&env, Scope::All, opts(RestartPolicy::None, &[]), &Health::ok(), Some(0), 32);
    assert_eq!(status_of(&env)["attention"], 1, "the dirty fork needs a person");
    // players online: restart-needing core is deferred = still an update waiting
    let env = dev_env();
    run(&env, Scope::All, opts(RestartPolicy::WhenQuiet, &[]), &Health::ok(), Some(4), 33);
    let v = status_of(&env);
    assert!(v["updates"].as_u64().unwrap() >= 1, "{v}");
}

#[test]
fn a_single_resource_apply_refreshes_the_whole_server_status_without_fetching() {
    let env = dev_env();
    preview_with(&env.sf, Scope::All, &GitCli::default(), None).unwrap();
    assert_eq!(status_of(&env)["updates"], 3);
    run(
        &env,
        Scope::Resources(vec!["kora-chat".into()]),
        opts(RestartPolicy::None, &[]),
        &Health::ok(),
        Some(0),
        34,
    );
    let v = status_of(&env);
    assert_eq!(
        (v["updates"].clone(), v["attention"].clone()),
        (serde_json::json!(2), serde_json::json!(0)),
        "{v}"
    );
}

#[test]
fn ensure_applies_files_and_says_plainly_that_nothing_was_restarted() {
    let env = dev_env();
    let h = Health::ok();
    let (r, ev) = run(&env, Scope::All, opts(RestartPolicy::Ensure, &[]), &h, Some(0), 35);
    assert!(!r.updated.is_empty());
    assert!(!h.checks.borrow()[0].full_restart);
    assert!(ev.iter().any(|e| matches!(e, UpdateEvent::Health(m) if m == ENSURE_NOTE)), "{ev:?}");
}

// ---------------------------------------------------------------- speed: parallel fetch, org hints

use crate::update::orgcache::{OrgReply, OrgSource};
use crate::update::{FetchMode, UpdateTarget};

/// Real git, but remotes look like `KoraFiveM/<folder>` and every `fetch_many` is recorded.
struct Spy {
    inner: GitCli,
    fetched: RefCell<Vec<Vec<String>>>,
}
impl Spy {
    fn new() -> Spy {
        Spy {
            inner: GitCli::default(),
            fetched: RefCell::new(vec![]),
        }
    }
    fn last(&self) -> Vec<String> {
        self.fetched.borrow().last().cloned().unwrap_or_default()
    }
}
impl GitOps for Spy {
    fn fetch(&self, r: &Path) -> GitResult<()> {
        self.inner.fetch(r)
    }
    fn fetch_many(&self, t: &[&UpdateTarget], m: FetchMode) -> Vec<GitResult<()>> {
        self.fetched
            .borrow_mut()
            .push(t.iter().map(|t| t.id.clone()).collect());
        self.inner.fetch_many(t, m)
    }
    fn has_ref(&self, r: &Path, rev: &str) -> bool {
        self.inner.has_ref(r, rev)
    }
    fn remote_slug(&self, r: &Path, _: &str) -> Option<String> {
        Some(format!("KoraFiveM/{}", r.file_name()?.to_str()?))
    }
    fn head_commit(&self, r: &Path, rev: &str) -> GitResult<String> {
        self.inner.head_commit(r, rev)
    }
    fn fast_forward(&self, r: &Path, u: &str) -> GitResult<()> {
        self.inner.fast_forward(r, u)
    }
    fn reset_to(&self, r: &Path, c: &str) -> GitResult<()> {
        self.inner.reset_to(r, c)
    }
    fn is_dirty(&self, r: &Path) -> GitResult<bool> {
        self.inner.is_dirty(r)
    }
    fn is_diverged(&self, r: &Path, u: &str) -> GitResult<bool> {
        self.inner.is_diverged(r, u)
    }
    fn behind_count(&self, r: &Path, u: &str) -> GitResult<u32> {
        self.inner.behind_count(r, u)
    }
    fn is_ancestor(&self, r: &Path, a: &str, b: &str) -> GitResult<bool> {
        self.inner.is_ancestor(r, a, b)
    }
    fn log_subjects(&self, r: &Path, f: &str, t: &str, m: usize) -> Vec<String> {
        self.inner.log_subjects(r, f, t, m)
    }
}

struct Org(OrgReply);
impl OrgSource for Org {
    fn get(&self, _: Option<&str>) -> OrgReply {
        self.0.clone()
    }
}
fn org_with(pushed: &str) -> Org {
    let body = format!(
        r#"[{{"full_name":"KoraFiveM/kora_chat","pushed_at":"{pushed}"}},{{"full_name":"KoraFiveM/kora_hud","pushed_at":"{pushed}"}},{{"full_name":"KoraFiveM/kora_core","pushed_at":"{pushed}"}}]"#
    );
    Org(OrgReply::Fresh { etag: None, body })
}
fn hints(o: &Org, file: &Path) -> OrgHints {
    OrgHints::load(o, Some(file), 1)
}

#[test]
fn org_hint_skips_fetches_of_unmoved_repositories_and_never_hides_a_change() {
    let env = dev_env();
    let file = env.sf.dir.parent().unwrap().join("org-cache.json");
    let spy = Spy::new();
    // First check: nothing is known about these clones, so all three are fetched.
    let mut h = hints(&org_with("t1"), &file);
    let p = preview_with_hints(&env.sf, Scope::All, &spy, None, None, Some(&mut h)).unwrap();
    assert_eq!(spy.last().len(), 3);
    assert!(p.resources.iter().all(|c| c.status == ChangeStatus::Clean));
    // Second check, the organisation reports the same push times: no fetch at all, same answer.
    let mut h = hints(&org_with("t1"), &file);
    let p2 = preview_with_hints(&env.sf, Scope::All, &spy, None, None, Some(&mut h)).unwrap();
    assert!(spy.last().is_empty(), "fetched {:?}", spy.last());
    assert_eq!(p2.resources, p.resources);
    // A push moves one repository: only that one is fetched and its new commit shows.
    let new = commit(&env.up(1), "3", "hud three");
    let body = match org_with("t1").0 {
        OrgReply::Fresh { body, .. } => body.replace(r#"kora_hud","pushed_at":"t1""#, r#"kora_hud","pushed_at":"t2""#),
        _ => unreachable!(),
    };
    let mut h = hints(&Org(OrgReply::Fresh { etag: None, body }), &file);
    let p3 = preview_with_hints(&env.sf, Scope::All, &spy, None, None, Some(&mut h)).unwrap();
    assert_eq!(spy.last(), vec!["kora-hud"]);
    assert_eq!(p3.resources[1].to, new);
    assert_eq!(p3.resources[1].behind, 2);
}

#[test]
fn org_hint_unavailable_falls_back_to_fetching_everything() {
    let env = dev_env();
    let file = env.sf.dir.parent().unwrap().join("org-cache.json");
    let spy = Spy::new();
    let mut h = hints(&org_with("t1"), &file);
    preview_with_hints(&env.sf, Scope::All, &spy, None, None, Some(&mut h)).unwrap();
    for (n, bad) in [
        OrgReply::RateLimited,
        OrgReply::Failed("offline".into()),
        OrgReply::Fresh { etag: None, body: "<html>".into() },
    ]
    .into_iter()
    .enumerate()
    {
        let up = commit(&env.up(0), &format!("x{n}"), "more");
        let mut h = hints(&Org(bad), &file);
        let p = preview_with_hints(&env.sf, Scope::All, &spy, None, None, Some(&mut h)).unwrap();
        assert_eq!(
            spy.last().len(),
            3,
            "a failed pre-check must run the full check"
        );
        assert_eq!(p.resources[0].to, up);
    }
}

#[test]
fn one_resource_preview_fetches_only_that_resource() {
    let env = dev_env();
    let spy = Spy::new();
    let p = preview_with(
        &env.sf,
        Scope::Resources(vec!["kora_hud".into()]),
        &spy,
        None,
    )
    .unwrap();
    assert_eq!(spy.last(), vec!["kora-hud"]);
    assert_eq!(p.resources.len(), 1);
}

#[test]
fn failed_fetch_is_never_reported_up_to_date_and_is_fetched_again_next_time() {
    let env = dev_env();
    let file = env.sf.dir.parent().unwrap().join("org-cache.json");
    let spy = Spy::new();
    let mut h = hints(&org_with("t1"), &file);
    preview_with_hints(&env.sf, Scope::All, &spy, None, None, Some(&mut h)).unwrap();
    // The remote of kora-chat disappears and the organisation says it moved: the fetch fails.
    std::fs::remove_dir_all(env.up(0)).unwrap();
    let mut h = hints(&org_with("t2"), &file);
    let p = preview_with_hints(&env.sf, Scope::All, &spy, None, None, Some(&mut h)).unwrap();
    assert_eq!(p.resources[0].status, ChangeStatus::Diverged);
    assert!(p.resources[0].changelog[0].starts_with("could not check"));
    // Still "t2" next time, yet the failed clone is fetched again (its record was dropped).
    let mut h = hints(&org_with("t2"), &file);
    preview_with_hints(&env.sf, Scope::All, &spy, None, None, Some(&mut h)).unwrap();
    assert!(spy.last().contains(&"kora-chat".to_string()));
}

#[test]
fn apply_right_after_a_preview_does_not_fetch_again() {
    let env = dev_env();
    let git = GitCli::default();
    preview_with(&env.sf, Scope::All, &git, None).unwrap();
    // Remotes are gone: a second fetch would fail and the apply would skip everything.
    for i in 0..3 {
        std::fs::remove_dir_all(env.up(i)).unwrap();
    }
    let (r, _) = run(
        &env,
        Scope::All,
        opts(RestartPolicy::None, &[]),
        &Health::ok(),
        Some(0),
        11,
    );
    assert_eq!(r.updated.len(), 3, "skipped: {:?}", r.skipped);
}

#[test]
fn production_with_the_pinned_commit_already_present_needs_no_network() {
    let env = dev_env();
    let git = GitCli::default();
    let path = env.path(0);
    sh(
        &path,
        &["remote", "set-url", "origin", "/nonexistent/nowhere"],
    );
    let present = head(&path);
    let t = |c: &str| UpdateTarget {
        id: "kora-chat".into(),
        path: path.clone(),
        upstream_ref: c.into(),
        class: crate::update::ResourceClass::from_id("kora-chat"),
    };
    let r = git.fetch_many(&[&t(&present)], FetchMode::Fresh);
    assert!(r[0].is_ok(), "the commit is here: no fetch");
    let r = git.fetch_many(
        &[&t("0123456789abcdef0123456789abcdef01234567")],
        FetchMode::Fresh,
    );
    assert!(r[0].is_err(), "a missing commit does ask the remote");
}

// ---------------------------------------------------------------- review fixes

fn prod_cat(env: &Env, c: &[(&str, &str)], latest: &str) -> Catalog {
    let _ = env;
    catalog(latest, c)
}

#[test]
fn production_never_moves_away_from_a_commit_of_the_users_own() {
    let (env, [c1, c2, _]) = prod_env();
    let mine = commit(&env.path(0), "my fix", "my own fix on the pinned checkout");
    let cat = prod_cat(&env, &[("0.2.0", &c2), ("0.1.0", &c1)], "0.2.0");
    let p = preview_with(&env.sf, Scope::All, &GitCli::default(), Some(&cat)).unwrap();
    assert_eq!(p.resources[0].status, ChangeStatus::Diverged, "{p:?}");
    let r = apply_with(
        &env.sf,
        Scope::All,
        opts(RestartPolicy::None, &[]),
        &GitCli::default(),
        &Health::ok(),
        Some(&cat),
        Some(0),
        70,
        &mut |_| {},
    )
    .unwrap();
    assert!(r.updated.is_empty());
    assert_eq!(
        r.skipped,
        vec![("kora-chat".to_string(), SkipReason::Diverged)]
    );
    assert_eq!(
        head(&env.path(0)),
        mine,
        "the user's commit is still checked out"
    );
    assert_eq!(exit_code(&r), 3);
}

#[test]
fn production_never_moves_to_an_older_release() {
    let (env, [c1, c2, c3]) = prod_env();
    sh(&env.path(0), &["checkout", "-q", "--detach", &c3]);
    let cat = prod_cat(&env, &[("0.2.0", &c2), ("0.1.0", &c1)], "0.2.0");
    let p = preview_with(&env.sf, Scope::All, &GitCli::default(), Some(&cat)).unwrap();
    assert_eq!(p.resources[0].status, ChangeStatus::UpToDate);
    let r = apply_with(
        &env.sf,
        Scope::All,
        opts(RestartPolicy::None, &[]),
        &GitCli::default(),
        &Health::ok(),
        Some(&cat),
        Some(0),
        71,
        &mut |_| {},
    )
    .unwrap();
    assert!(r.updated.is_empty());
    assert_eq!(head(&env.path(0)), c3);
}

#[test]
fn production_offline_with_a_missing_release_says_it_could_not_check() {
    let (env, [c1, _, _]) = prod_env();
    sh(
        &env.path(0),
        &["remote", "set-url", "origin", "/nonexistent/nowhere"],
    );
    let missing = "0123456789abcdef0123456789abcdef01234567";
    let cat = prod_cat(&env, &[("0.3.0", missing), ("0.1.0", &c1)], "0.3.0");
    let p = preview_with(&env.sf, Scope::All, &GitCli::default(), Some(&cat)).unwrap();
    assert_ne!(
        p.resources[0].status,
        ChangeStatus::Clean,
        "never 'ready' when nothing could be compared"
    );
    assert!(
        p.resources[0].changelog[0].contains("couldn't check (offline?)"),
        "{:?}",
        p.resources[0].changelog
    );
    assert_eq!(head(&env.path(0)), c1);
}

#[test]
fn production_lists_resources_the_catalog_dropped_and_warns_about_the_built_in_catalog() {
    let (env, [c1, c2, _]) = prod_env();
    let mut sf = env.sf.clone();
    sf.resources.push(ResourceRecord {
        id: "kora-gone".into(),
        path: "[kora]/kora_gone".into(),
        repository: "https://github.com/x/g.git".into(),
        branch: "main".into(),
        commit: Some(c1.clone()),
        third_party: false,
        ..Default::default()
    });
    let cat = prod_cat(&env, &[("0.2.0", &c2), ("0.1.0", &c1)], "0.2.0");
    let note = catalog_note(&catalog::CatalogOrigin::Embedded("offline".into()));
    let p = preview_with_hints(
        &sf,
        Scope::All,
        &GitCli::default(),
        Some(&cat),
        note.as_deref(),
        None,
    )
    .unwrap();
    assert!(
        p.notes
            .iter()
            .any(|n| n.contains("built into this version of Kora")),
        "{:?}",
        p.notes
    );
    assert!(
        p.notes
            .iter()
            .any(|n| n == "kora-gone: no longer in the catalog (left as is)"),
        "{:?}",
        p.notes
    );
    assert!(render_preview(&p).contains("Note: kora-gone"));
    assert_eq!(catalog_note(&catalog::CatalogOrigin::Remote), None);
}

#[test]
fn production_records_the_new_pin_without_losing_what_another_tool_wrote() {
    let (env, [c1, c2, _]) = prod_env();
    let mut on_disk = ServerFile::load(&env.sf.dir).unwrap();
    on_disk.kora_version = "9.9.9".into();
    on_disk.save().unwrap();
    let cat = prod_cat(&env, &[("0.2.0", &c2), ("0.1.0", &c1)], "0.2.0");
    apply_with(
        &env.sf,
        Scope::All,
        opts(RestartPolicy::None, &[]),
        &GitCli::default(),
        &Health::ok(),
        Some(&cat),
        Some(0),
        72,
        &mut |_| {},
    )
    .unwrap();
    let after = ServerFile::load(&env.sf.dir).unwrap();
    assert_eq!(
        after.kora_version, "9.9.9",
        "the file is read again under the lock"
    );
    assert_eq!(after.resources[0].commit.as_deref(), Some(c2.as_str()));
}

#[test]
fn preview_apply_and_rollback_refuse_while_another_update_runs() {
    let env = dev_env();
    let before: Vec<String> = (0..3).map(|i| head(&env.path(i))).collect();
    let held = crate::update::lock::acquire(&env.sf.dir).unwrap();
    let e = preview_with(&env.sf, Scope::All, &GitCli::default(), None).unwrap_err();
    assert!(lock::is_busy_message(&e.to_string()), "{e}");
    let e = apply_with(
        &env.sf,
        Scope::All,
        opts(RestartPolicy::None, &[]),
        &GitCli::default(),
        &Health::ok(),
        None,
        Some(0),
        80,
        &mut |_| {},
    )
    .unwrap_err();
    assert!(lock::is_busy_message(&e.to_string()), "{e}");
    let e = rollback_with(&env.sf, "1", &GitCli::default(), false).unwrap_err();
    assert!(lock::is_busy_message(&e.to_string()), "{e}");
    assert_eq!(
        (0..3).map(|i| head(&env.path(i))).collect::<Vec<_>>(),
        before,
        "nothing moved"
    );
    assert!(snapshots(&env.sf).is_empty());
    drop(held);
    assert!(preview_with(&env.sf, Scope::All, &GitCli::default(), None).is_ok());
}

#[test]
fn a_preview_cannot_start_while_an_apply_holds_the_lock() {
    // The apply holds the lock for its whole run, including the health check: a preview started then is refused.
    struct Probe<'a>(&'a ServerFile, RefCell<Option<String>>);
    impl HealthCheck for Probe<'_> {
        fn check(&self, _: &HealthRequest) -> HealthOutcome {
            *self.1.borrow_mut() = Some(
                match preview_with(self.0, Scope::All, &GitCli::default(), None) {
                    Ok(_) => "ran".into(),
                    Err(e) => e.to_string(),
                },
            );
            HealthOutcome::Healthy
        }
        fn recover(&self, _: &HealthRequest) {}
    }
    let env = dev_env();
    let probe = Probe(&env.sf, RefCell::new(None));
    apply_with(
        &env.sf,
        Scope::All,
        opts(RestartPolicy::FullRestart, &[]),
        &GitCli::default(),
        &probe,
        None,
        Some(0),
        81,
        &mut |_| {},
    )
    .unwrap();
    let got = probe.1.borrow().clone().unwrap();
    assert!(lock::is_busy_message(&got), "{got}");
}

#[test]
fn rollback_refuses_to_throw_away_uncommitted_changes_unless_forced() {
    let env = dev_env();
    let old = head(&env.path(0));
    run(
        &env,
        Scope::All,
        opts(RestartPolicy::None, &[]),
        &Health::ok(),
        Some(0),
        90,
    );
    std::fs::write(env.path(0).join("a"), "my edit").unwrap();
    let e = rollback_with(&env.sf, "90", &GitCli::default(), false)
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("kora-chat") && e.contains("--force") && e.starts_with("nothing was changed"),
        "{e}"
    );
    assert_ne!(head(&env.path(0)), old);
    assert_eq!(
        std::fs::read_to_string(env.path(0).join("a")).unwrap(),
        "my edit"
    );
    rollback_with(&env.sf, "90", &GitCli::default(), true).unwrap();
    assert_eq!(head(&env.path(0)), old);
}

#[test]
fn apply_takes_exactly_the_previewed_versions() {
    let env = dev_env();
    let p = preview_with(&env.sf, Scope::All, &GitCli::default(), None).unwrap();
    let mut targets = planned_targets(&p);
    targets.remove("kora-hud");
    let previewed = targets["kora-chat"].clone();
    // Upstream moves on after the preview.
    let newer = commit(&env.up(0), "3", "kora-chat three");
    sh(&env.path(0), &["fetch", "-q", "upstream"]);
    let hud = head(&env.path(1));
    let mut o = opts(RestartPolicy::None, &[]);
    o.targets = Some(targets);
    let (r, _) = run(&env, Scope::All, o, &Health::ok(), Some(0), 100);
    assert_eq!(
        head(&env.path(0)),
        previewed,
        "what was previewed, not {newer}"
    );
    assert_eq!(
        head(&env.path(1)),
        hud,
        "not in the preview's plan: left alone"
    );
    assert_eq!(r.updated.len(), 2);
    // A commit that is not on the original branch is refused.
    sh(&env.path(1), &["checkout", "-q", "-b", "side"]);
    let side = commit(&env.path(1), "side", "not on upstream");
    sh(&env.path(1), &["checkout", "-q", "dev"]);
    let mut o = opts(RestartPolicy::None, &[]);
    o.targets = Some([("kora-hud".to_string(), side)].into_iter().collect());
    let (r, _) = run(&env, Scope::All, o, &Health::ok(), Some(0), 101);
    assert!(r.updated.is_empty());
    assert!(
        r.needs_attention[0]
            .1
            .contains("no longer on the original repository"),
        "{:?}",
        r.needs_attention
    );
    assert_eq!(head(&env.path(1)), hud);
}

#[test]
fn rolled_back_versions_are_held_from_automatic_updates() {
    let env = dev_env();
    let (r, _) = run(
        &env,
        Scope::All,
        opts(RestartPolicy::FullRestart, &[]),
        &Health::bad(),
        Some(0),
        110,
    );
    assert_eq!(r.rolled_back.len(), 3);
    let holds = crate::update::load_holds(&env.sf.dir);
    assert_eq!(holds.get("kora-chat"), Some(&head(&env.up(0))));
    // A manual update that works releases the hold.
    run(
        &env,
        Scope::Resources(vec!["kora-chat".into()]),
        opts(RestartPolicy::None, &[]),
        &Health::ok(),
        Some(0),
        111,
    );
    assert!(!crate::update::load_holds(&env.sf.dir).contains_key("kora-chat"));
    // Rolling back by hand holds what was undone.
    rollback_with(&env.sf, "111", &GitCli::default(), false).unwrap();
    assert_eq!(
        crate::update::load_holds(&env.sf.dir).get("kora-chat"),
        Some(&head(&env.up(0)))
    );
}

#[test]
fn fork_sync_reports_progress_and_the_player_count_is_read_after_it() {
    struct Forks {
        inner: GitCli,
        log: RefCell<Vec<String>>,
    }
    impl GitOps for Forks {
        fn fetch(&self, r: &Path) -> GitResult<()> {
            self.inner.fetch(r)
        }
        fn fetch_many(&self, t: &[&UpdateTarget], m: FetchMode) -> Vec<GitResult<()>> {
            self.inner.fetch_many(t, m)
        }
        fn head_commit(&self, r: &Path, rev: &str) -> GitResult<String> {
            self.inner.head_commit(r, rev)
        }
        fn fast_forward(&self, r: &Path, u: &str) -> GitResult<()> {
            self.inner.fast_forward(r, u)
        }
        fn reset_to(&self, r: &Path, c: &str) -> GitResult<()> {
            self.inner.reset_to(r, c)
        }
        fn is_dirty(&self, r: &Path) -> GitResult<bool> {
            self.inner.is_dirty(r)
        }
        fn is_diverged(&self, r: &Path, u: &str) -> GitResult<bool> {
            self.inner.is_diverged(r, u)
        }
        fn behind_count(&self, r: &Path, u: &str) -> GitResult<u32> {
            self.inner.behind_count(r, u)
        }
        fn sync_forks(
            &self,
            t: &[UpdateTarget],
            progress: &mut dyn FnMut(usize, usize),
        ) -> Vec<(String, crate::git::SyncOutcome)> {
            for i in 0..t.len() {
                progress(i + 1, t.len());
            }
            self.log.borrow_mut().push("synced".into());
            vec![]
        }
    }
    let env = dev_env();
    let g = Forks {
        inner: GitCli::default(),
        log: RefCell::new(vec![]),
    };
    let mut ev = vec![];
    let players = || {
        g.log.borrow_mut().push("players".into());
        Some(3)
    };
    let r = apply_with_hints(
        &env.sf,
        Scope::All,
        opts(RestartPolicy::WhenQuiet, &[]),
        &g,
        &Health::ok(),
        None,
        None,
        &players,
        120,
        &mut |e| ev.push(e),
        None,
    )
    .unwrap();
    assert_eq!(*g.log.borrow(), vec!["synced", "players"]);
    assert!(
        ev.contains(&UpdateEvent::Progress(
            "Syncing your GitHub forks 3/3".into()
        )),
        "{ev:?}"
    );
    assert_eq!(r.deferred.len(), 1, "3 players online: kora-core waits");
}

#[test]
fn the_restart_policy_also_governs_the_restart_after_a_rollback() {
    let inner = Health::ok();
    let req = HealthRequest {
        full_restart: true,
        updated: vec!["x".into()],
    };
    PolicyHealth {
        inner: &inner,
        policy: RestartPolicy::None,
    }
    .recover(&req);
    assert_eq!(*inner.recovers.borrow(), 0, "none: never restarted");
    struct Rec(RefCell<Vec<bool>>);
    impl HealthCheck for Rec {
        fn check(&self, _: &HealthRequest) -> HealthOutcome {
            HealthOutcome::Healthy
        }
        fn recover(&self, r: &HealthRequest) {
            self.0.borrow_mut().push(r.full_restart);
        }
    }
    let rec = Rec(RefCell::new(vec![]));
    PolicyHealth {
        inner: &rec,
        policy: RestartPolicy::Ensure,
    }
    .recover(&req);
    PolicyHealth {
        inner: &rec,
        policy: RestartPolicy::WhenQuiet,
    }
    .recover(&HealthRequest {
        full_restart: false,
        updated: vec![],
    });
    assert_eq!(
        *rec.0.borrow(),
        vec![false, false],
        "ensure and a busy when_quiet never ask for a full restart"
    );
}

#[test]
fn a_report_with_not_attempted_items_counts_them_as_still_waiting() {
    let r = UpdateReport {
        not_attempted: vec!["a".into(), "b".into()],
        needs_attention: vec![("c".into(), "x".into())],
        ..Default::default()
    };
    let st = apply_status(&r, 1, &None);
    assert_eq!((st.updates, st.attention), (2, 1));
}

/// The planned ids; the plan comes before any resource is touched.
fn planned(ev: &[UpdateEvent]) -> Vec<String> {
    let at = ev.iter().position(|e| matches!(e, UpdateEvent::Planned(_))).unwrap_or_else(|| panic!("no plan: {ev:?}"));
    let first_resource = ev.iter().position(|e| matches!(e, UpdateEvent::Resource { .. })).unwrap_or(usize::MAX);
    assert!(at < first_resource, "the plan must come before the resources: {ev:?}");
    match &ev[at] {
        UpdateEvent::Planned(ids) => ids.clone(),
        _ => unreachable!(),
    }
}

#[test]
fn the_plan_comes_first_and_every_planned_resource_reaches_a_final_state() {
    // Success: every planned resource is announced, then ends "updated".
    let env = dev_env();
    let (_, ev) = run(&env, Scope::All, opts(RestartPolicy::FullRestart, &[]), &Health::ok(), Some(0), 40);
    let ids = planned(&ev);
    assert!(!ids.is_empty());
    for id in ids {
        assert!(ev.contains(&UpdateEvent::Resource { id: id.clone(), state: "updated".into() }), "{id} never finished: {ev:?}");
    }

    // A failed fast-forward: the failed one and the ones never tried are all given a final state, so a
    // progress bar counting them reaches the total.
    struct FailFirst(GitCli, std::cell::Cell<bool>);
    impl GitOps for FailFirst {
        fn fetch(&self, r: &Path) -> GitResult<()> {
            self.0.fetch(r)
        }
        fn fetch_many(&self, t: &[&UpdateTarget], m: FetchMode) -> Vec<GitResult<()>> {
            self.0.fetch_many(t, m)
        }
        fn head_commit(&self, r: &Path, rev: &str) -> GitResult<String> {
            self.0.head_commit(r, rev)
        }
        fn fast_forward(&self, r: &Path, u: &str) -> GitResult<()> {
            if self.1.replace(false) {
                return Err(UpdateError::Git("boom".into()));
            }
            self.0.fast_forward(r, u)
        }
        fn reset_to(&self, r: &Path, c: &str) -> GitResult<()> {
            self.0.reset_to(r, c)
        }
        fn is_dirty(&self, r: &Path) -> GitResult<bool> {
            self.0.is_dirty(r)
        }
        fn is_diverged(&self, r: &Path, u: &str) -> GitResult<bool> {
            self.0.is_diverged(r, u)
        }
        fn behind_count(&self, r: &Path, u: &str) -> GitResult<u32> {
            self.0.behind_count(r, u)
        }
    }
    let env = dev_env();
    let git = FailFirst(GitCli::default(), std::cell::Cell::new(true));
    let mut ev = vec![];
    apply_with(&env.sf, Scope::All, opts(RestartPolicy::FullRestart, &[]), &git, &Health::ok(), None, Some(0), 41, &mut |e| ev.push(e)).unwrap();
    let ids = planned(&ev);
    for id in &ids {
        let last = ev.iter().rev().find_map(|e| match e {
            UpdateEvent::Resource { id: i, state } if i == id => Some(state.clone()),
            _ => None,
        });
        let last = last.unwrap_or_else(|| panic!("{id} has no event: {ev:?}"));
        assert!(!matches!(last.as_str(), "updating" | "rolling back"), "{id} stuck at {last}: {ev:?}");
    }
}
