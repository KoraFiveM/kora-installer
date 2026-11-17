use super::*;
use crate::installer::registry::PortsRecord;
use std::cell::RefCell;

const SHA: &str = "0123456789012345678901234567890123456789";

fn catalog() -> Catalog {
    let rel = format!(r#""latest":"1.0.0","versions":[{{"version":"1.0.0","tag":"v1.0.0","commit":"{SHA}"}}]"#);
    let json = format!(
        r#"{{"schemaVersion":3,"creators":[{{"id":"acme","name":"Acme"}}],"resources":[
        {{"id":"kora-core","repository":"https://github.com/KoraFiveM/kora_core.git","path":"[kora]/kora_core","group":"kora","kind":"kora","branch":"dev","order":1,{rel}}},
        {{"id":"kora-new","repository":"https://github.com/KoraFiveM/kora_new.git","path":"[kora]/kora_new","group":"kora","kind":"kora","branch":"dev","order":2,{rel},"dependsOn":["compat-a"]}},
        {{"id":"compat-a","repository":"https://github.com/o/ca.git","path":"a","group":"compat","kind":"optional","branch":"main","order":3,"type":"compat",{rel}}},
        {{"id":"ox-lib","repository":"https://github.com/KoraFiveM/compat.git","path":"[compat]/ox_lib","group":"compat","kind":"optional","branch":"main","order":6,"type":"compat","subpath":"ox_lib","latest":"1.0.0","versions":[{{"version":"1.0.0","commit":"{SHA}"}}]}},
        {{"id":"bridge-q","repository":"https://github.com/o/q.git","path":"q","group":"other","kind":"optional","branch":"main","order":4,"type":"bridge",{rel},"dependsOn":["compat-a"]}},
        {{"id":"acme-r","repository":"https://github.com/o/r.git","path":"r","group":"other","kind":"optional","branch":"main","order":5,"type":"creator","creator":"acme",{rel},"dependsOn":["bridge-q"]}}
        ]}}"#
    );
    Catalog::parse_validated(&json).unwrap()
}

fn server(dir: &Path, mode: Mode, owner: Option<&str>) -> ServerFile {
    ServerFile {
        name: "s".into(),
        mode,
        dir: dir.to_path_buf(),
        ports: PortsRecord { game: 30120, txadmin: 40120 },
        game_build: 1,
        artifact_build: 1,
        database: None,
        github_owner: owner.map(String::from),
        created_unix: 0,
        kora_version: "0".into(),
        resources: vec![ResourceRecord {
            id: "kora-core".into(),
            path: "[kora]/kora_core".into(),
            repository: "https://github.com/KoraFiveM/kora_core.git".into(),
            branch: "dev".into(),
            commit: None,
            third_party: false,
            ..Default::default()
        }],
    }
}

#[derive(Default)]
struct Fake {
    forked: RefCell<Vec<String>>,
    cloned: RefCell<Vec<String>>,
    fail: Option<&'static str>,
}

impl Cloner for Fake {
    fn prepare_forks(&self, owner: &str, repos: &[String]) -> std::result::Result<git::ForkState, String> {
        self.forked.borrow_mut().extend(repos.iter().map(|r| format!("{owner}<-{r}")));
        let mut st = git::ForkState::new(owner, false, &[]);
        st.done = repos.to_vec();
        Ok(st)
    }
    fn clone_item(&self, item: &PlanItem, fork: Option<&str>, partial: &Path) -> std::result::Result<(), String> {
        let fork_owner = fork.map(|f| f.split('/').next().unwrap());
        if self.fail == Some(item.id.as_str()) {
            std::fs::create_dir_all(partial).unwrap(); // a half-made clone must be cleaned up
            return Err("network down".into());
        }
        self.cloned.borrow_mut().push(format!("{} commit={:?} fork={:?}", item.id, item.commit, fork_owner.filter(|_| item.fork)));
        std::fs::create_dir_all(partial.join(".git")).unwrap();
        if let Some(sub) = &item.subpath {
            std::fs::create_dir_all(partial.join(sub)).unwrap();
            std::fs::write(partial.join(sub).join("fxmanifest.lua"), "x").unwrap();
        }
        Ok(())
    }
}

fn setup(cfg: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join("resources/[kora]/kora_core/.git")).unwrap();
    std::fs::write(d.path().join("server.cfg"), cfg).unwrap();
    d
}

#[test]
fn plan_pins_bridges_and_creators_and_adds_dependencies() {
    let d = setup("");
    let sf = server(d.path(), Mode::Production, None);
    let p = plan(&catalog(), &sf, &["acme-r".into()]).unwrap();
    let got: Vec<(&str, &str, bool)> = p.items.iter().map(|i| (i.id.as_str(), i.rel.as_str(), i.dependency)).collect();
    assert_eq!(got, [("compat-a", "[compat]/a", true), ("bridge-q", "[bridges]/q", true), ("acme-r", "[creators]/[acme]/r", false)]);
    assert!(p.items.iter().all(|i| i.commit.as_deref() == Some(SHA) && !i.fork));
    assert_eq!(p.ensure, ["[compat]", "[bridges]", "[creators]"]);
}

#[test]
fn plan_skips_installed_and_rejects_unknown() {
    let d = setup("");
    let sf = server(d.path(), Mode::Production, None);
    let p = plan(&catalog(), &sf, &["kora-core".into(), "bridge-q".into()]).unwrap();
    assert_eq!(p.already, ["kora-core"]);
    assert_eq!(p.items.len(), 2, "bridge-q + its compat");
    assert!(plan(&catalog(), &sf, &["nope".into()]).is_err());
}

#[test]
fn installs_into_new_folders_and_appends_ensures_on_an_old_layout() {
    // an older server: single resources ensured one by one, nothing in the new folders
    let cfg = "exec secrets.cfg\nensure kora_core\n";
    let d = setup(cfg);
    let mut sf = server(d.path(), Mode::Production, None);
    let p = plan(&catalog(), &sf, &["bridge-q".into()]).unwrap();
    let f = Fake::default();
    let out = execute(&f, &mut sf, &p, &mut |_| {}).unwrap();
    assert!(out.ok(), "{out:?}");
    assert_eq!(out.installed, ["compat-a", "bridge-q"]);
    assert!(d.path().join("resources/[compat]/a/.git").is_dir() && d.path().join("resources/[bridges]/q/.git").is_dir());
    assert!(d.path().join("resources/[kora]/kora_core/.git").is_dir(), "existing resources are left alone");
    assert_eq!(std::fs::read_to_string(d.path().join("server.cfg")).unwrap(), format!("{cfg}ensure [compat]\nensure [bridges]\n"));
    assert_eq!(out.ensured, ["[compat]", "[bridges]"]);
    assert!(f.cloned.borrow()[0].contains(SHA));
    // recorded, including the third-party flag; the file on disk agrees
    let saved = ServerFile::load(d.path()).unwrap();
    assert_eq!(saved.resources.len(), 3);
    assert!(saved.resources.iter().find(|r| r.id == "bridge-q").unwrap().third_party);
    assert!(!saved.resources.iter().find(|r| r.id == "kora-core").unwrap().third_party);
    // running it again changes nothing
    let p2 = plan(&catalog(), &saved, &["bridge-q".into()]).unwrap();
    assert!(p2.is_empty());
    let out2 = execute(&f, &mut sf, &p2, &mut |_| {}).unwrap();
    assert!(out2.installed.is_empty() && out2.ensured.is_empty());
    assert_eq!(std::fs::read_to_string(d.path().join("server.cfg")).unwrap(), format!("{cfg}ensure [compat]\nensure [bridges]\n"));
}

#[test]
fn existing_ensure_lines_are_not_duplicated() {
    let d = setup("ensure [kora]\nensure [compat]\n");
    let mut sf = server(d.path(), Mode::Production, None);
    let p = plan(&catalog(), &sf, &["acme-r".into()]).unwrap();
    execute(&Fake::default(), &mut sf, &p, &mut |_| {}).unwrap();
    assert_eq!(std::fs::read_to_string(d.path().join("server.cfg")).unwrap(), "ensure [kora]\nensure [compat]\nensure [bridges]\nensure [creators]\n");
}

#[test]
fn development_forks_kora_resources_but_never_third_party() {
    let d = setup("");
    let mut sf = server(d.path(), Mode::Development, Some("my-org"));
    let p = plan(&catalog(), &sf, &["kora-new".into(), "bridge-q".into()]).unwrap();
    let f = Fake::default();
    let out = execute(&f, &mut sf, &p, &mut |_| {}).unwrap();
    assert!(out.ok(), "{out:?}");
    assert_eq!(*f.forked.borrow(), ["my-org<-KoraFiveM/kora_new", "my-org<-o/ca"]);
    let cloned = f.cloned.borrow();
    let get = |id: &str| cloned.iter().find(|c| c.starts_with(id)).unwrap().clone();
    assert!(get("kora-new").contains("commit=None") && get("kora-new").contains("fork=Some(\"my-org\")"));
    assert!(get("compat-a").contains("fork=Some"), "compat is Kora's own, so it follows the dev rules");
    assert!(get("bridge-q").contains(&format!("commit=Some(\"{SHA}\")")) && get("bridge-q").contains("fork=None"), "pinned, never forked, even in dev mode");
    let saved = ServerFile::load(d.path()).unwrap();
    assert_eq!(saved.resources.iter().find(|r| r.id == "bridge-q").unwrap().commit.as_deref(), Some(SHA));
    assert_eq!(saved.resources.iter().find(|r| r.id == "kora-new").unwrap().commit, None);
}

/// Fake `gh`/`git`: records every command and answers like GitHub would for a user `me` whose
/// fork of `KoraFiveM/kora_new` is called `kora_new-1`.
struct Gh(RefCell<Vec<String>>);

impl CommandRunner for Gh {
    fn run(&self, p: &str, a: &[&str], _cwd: Option<&Path>) -> std::io::Result<git::Output> {
        let line = format!("{p} {}", a.join(" "));
        self.0.borrow_mut().push(line.clone());
        let ok = |s: &str| git::Output { code: 0, stdout: s.into(), stderr: String::new() };
        Ok(if line.starts_with("gh api orgs/") {
            git::Output { code: 1, stdout: String::new(), stderr: "HTTP 404".into() }
        } else if line.starts_with("gh api graphql") {
            ok(
                r#"{"data":{"repositoryOwner":{"repositories":{"nodes":[{"nameWithOwner":"me/kora_new-1","defaultBranchRef":{"name":"dev"},"parent":{"nameWithOwner":"KoraFiveM/kora_new"}}],"pageInfo":{"hasNextPage":false}}}}}"#,
            )
        } else if line.starts_with("gh repo fork o/ca") {
            ok("https://github.com/me/ca\n")
        } else if line.starts_with("gh api repos/") {
            ok("1\n")
        } else if line.starts_with("gh repo clone") {
            std::fs::create_dir_all(Path::new(a[3]).join(".git")).unwrap();
            ok("")
        } else if line.contains("get-url") {
            git::Output { code: 2, stdout: String::new(), stderr: "no such remote".into() }
        } else {
            ok("")
        })
    }
    fn sleep(&self, _d: std::time::Duration) {}
}

#[test]
fn development_add_reuses_an_existing_fork_and_clones_it_under_its_real_name() {
    let d = setup("");
    let mut sf = server(d.path(), Mode::Development, Some("me"));
    let p = plan(&catalog(), &sf, &["kora-new".into()]).unwrap();
    let gh = Gh(RefCell::new(vec![]));
    let cloner = GitCloner(&gh);
    let st = cloner.forks_with("me", &["KoraFiveM/kora_new".into(), "o/ca".into()], &git::ForkConfig { ready_timeout: std::time::Duration::ZERO, ..Default::default() }).unwrap();
    assert_eq!(st.fork_of("KoraFiveM/kora_new"), "me/kora_new-1");
    assert_eq!(st.counts_text(), "created 1 fork, reused 1 you already had");
    // execute() with a cloner that hands back this state and clones through the fake gh
    struct Prepared<'a>(GitCloner<'a>, git::ForkState);
    impl Cloner for Prepared<'_> {
        fn prepare_forks(&self, _o: &str, _r: &[String]) -> std::result::Result<git::ForkState, String> {
            Ok(self.1.clone())
        }
        fn clone_item(&self, item: &PlanItem, fork: Option<&str>, partial: &Path) -> std::result::Result<(), String> {
            self.0.clone_item(item, fork, partial)
        }
    }
    let mut said = vec![];
    let out = execute(&Prepared(GitCloner(&gh), st), &mut sf, &p, &mut |m| said.push(m.to_string())).unwrap();
    assert!(out.ok(), "{out:?}");
    let calls = gh.0.borrow();
    assert_eq!(calls.iter().filter(|c| c.starts_with("gh repo fork")).count(), 1, "{calls:?}");
    assert!(!calls.iter().any(|c| c.starts_with("gh repo fork KoraFiveM/kora_new")));
    assert!(calls.iter().any(|c| c.starts_with("gh repo clone me/kora_new-1 ")), "{calls:?}");
    assert!(said.iter().any(|m| m.contains("reused 1 you already had")), "{said:?}");
    assert!(!said.iter().any(|m| m.starts_with("Forking")));
}

#[test]
fn a_development_server_without_a_github_owner_cannot_fork() {
    let d = setup("");
    let mut sf = server(d.path(), Mode::Development, None);
    let p = plan(&catalog(), &sf, &["kora-new".into()]).unwrap();
    assert!(execute(&Fake::default(), &mut sf, &p, &mut |_| {}).is_err());
    // third-party only needs no owner
    let p = plan(&catalog(), &sf, &["acme-r".into()]).unwrap();
    // acme-r pulls compat-a, which is Kora's own and would need a fork
    assert!(p.items.iter().any(|i| i.fork));
}

#[test]
fn a_failed_clone_is_reported_cleaned_up_and_not_ensured() {
    let d = setup("ensure [kora]\n");
    let mut sf = server(d.path(), Mode::Production, None);
    let p = plan(&catalog(), &sf, &["bridge-q".into()]).unwrap();
    let f = Fake { fail: Some("bridge-q"), ..Default::default() };
    let out = execute(&f, &mut sf, &p, &mut |_| {}).unwrap();
    assert_eq!(out.failed.len(), 1);
    assert_eq!(out.failed[0].0, "bridge-q");
    assert_eq!(out.installed, ["compat-a"]);
    assert!(!d.path().join("resources/[bridges]/q").exists() && !d.path().join("resources/[bridges]/q.kora-partial").exists());
    assert_eq!(std::fs::read_to_string(d.path().join("server.cfg")).unwrap(), "ensure [kora]\nensure [compat]\n", "only folders that got something");
    assert!(!ServerFile::load(d.path()).unwrap().resources.iter().any(|r| r.id == "bridge-q"));
}

#[test]
fn a_folder_inside_another_clone_or_not_ours_is_refused() {
    let d = setup("");
    let mut sf = server(d.path(), Mode::Production, None);
    // the old layout's `[compat]` is a clone itself
    std::fs::create_dir_all(d.path().join("resources/[compat]/.git")).unwrap();
    let p = plan(&catalog(), &sf, &["compat-a".into()]).unwrap();
    let out = execute(&Fake::default(), &mut sf, &p, &mut |_| {}).unwrap();
    assert_eq!(out.failed.len(), 1);
    assert!(out.failed[0].1.contains("inside the resource"));
    // a user's own folder is never overwritten
    std::fs::remove_dir_all(d.path().join("resources/[compat]")).unwrap();
    std::fs::create_dir_all(d.path().join("resources/[compat]/a")).unwrap();
    std::fs::write(d.path().join("resources/[compat]/a/mine.lua"), "x").unwrap();
    let out = execute(&Fake::default(), &mut sf, &p, &mut |_| {}).unwrap();
    assert!(out.failed[0].1.contains("was not created by Kora"));
    assert!(d.path().join("resources/[compat]/a/mine.lua").is_file());
}

#[test]
fn a_subpath_resource_is_a_pinned_copy_of_one_folder_even_in_dev_mode() {
    let d = setup("ensure [kora]\n");
    let mut sf = server(d.path(), Mode::Development, Some("my-org"));
    let p = plan(&catalog(), &sf, &["ox-lib".into()]).unwrap();
    assert_eq!(p.items.len(), 1);
    assert!(!p.items[0].fork && p.items[0].commit.as_deref() == Some(SHA) && p.items[0].third_party);
    let f = Fake::default();
    let out = execute(&f, &mut sf, &p, &mut |_| {}).unwrap();
    assert!(out.ok(), "{out:?}");
    assert!(f.forked.borrow().is_empty(), "nothing is forked for a copy");
    let dest = d.path().join("resources/[compat]/ox_lib");
    assert!(dest.join("fxmanifest.lua").is_file() && !dest.join(".git").exists());
    assert!(crate::installer::pinned::marker_matches(&dest, SHA));
    assert_eq!(std::fs::read_to_string(d.path().join("server.cfg")).unwrap(), "ensure [kora]\nensure [compat]\n");
    // already there: nothing to do the second time
    let saved = ServerFile::load(d.path()).unwrap();
    assert!(plan(&catalog(), &saved, &["ox-lib".into()]).unwrap().is_empty());
}
