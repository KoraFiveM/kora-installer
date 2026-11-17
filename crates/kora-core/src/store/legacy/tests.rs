//! The migration from the JSON files of Kora v0.7.0 (and v0.6.x, which has the same shapes).
//!
//! The fixtures are written the way v0.7.0 wrote them: the same serde types where they still exist
//! (`Registry`, `Stored`, `InstallRequest`, `Profile`) pretty-printed, and the exact layouts of
//! `secrets.json`, `panel.json` and `hub.json` (`serde_json::to_string_pretty` of the old structs).

use super::*;
use crate::hub::NodeStore;
use crate::installer::paused::PausedStore;
use crate::installer::registry::{PortsRecord, Registry};
use crate::installer::{DbAdmin, DbChoice, Mode, Ports, ResourceSelection};
use crate::keystore::KeyStore;
use crate::profile::ProfileStore;
use crate::store::backup;

fn j(r: &InstallRequest) -> serde_json::Value {
    serde_json::to_value(r).unwrap()
}

const K1: &str = "cfxk_AAAAAAAAAAAAAAAA_1111";
const K2: &str = "cfxk_BBBBBBBBBBBBBBBB_2222";

fn request(dir: &str, key_name: Option<&str>) -> InstallRequest {
    InstallRequest {
        mode: Mode::Development,
        name: "Dev Den".into(),
        dir: dir.into(),
        hostname: "Dev Den".into(),
        license_key: None,
        license_key_name: key_name.map(String::from),
        cfx_identifier: Some("fivem:42".into()),
        ports: Ports { game: 30120, txadmin: 40120 },
        txadmin_public: true,
        artifact_url: Some("https://example.invalid/a".into()),
        game_build: Some(3095),
        resources: ResourceSelection::All,
        db: DbChoice::InstallIfMissing,
        service: true,
        firewall: false,
        github_owner: Some("KoraFiveM".into()),
        catalog: None,
        dry_run: false,
        db_admin: Some(DbAdmin { host: "127.0.0.1".into(), port: 3306, user: "root".into(), password: None }),
        start: true,
        app: None,
    }
}

fn servers() -> Vec<ServerEntry> {
    vec![
        ServerEntry { name: "Dev Den".into(), dir: "/srv/dev-den".into(), mode: Mode::Development, ports: PortsRecord { game: 30120, txadmin: 40120 }, created_unix: 1_700_000_000 },
        ServerEntry { name: "Live".into(), dir: "/srv/live".into(), mode: Mode::Production, ports: PortsRecord { game: 30121, txadmin: 40121 }, created_unix: 1_700_000_500 },
    ]
}

const SECRETS_JSON: &str = r#"{
  "version": 1,
  "keys": [
    { "name": "Main", "key": "cfxk_AAAAAAAAAAAAAAAA_1111", "added": 1700000001 },
    { "name": "Test server", "key": "cfxk_BBBBBBBBBBBBBBBB_2222", "added": 1700000002 }
  ]
}"#;

const PANEL_JSON: &str = r#"{
  "enabled": true,
  "tailscale": true,
  "tailscale_users": { "sam@example.com": "admin", "pat@example.com": "viewer" },
  "users": [
    { "name": "alice", "role": "operator", "salt": "aa11", "hash": "bb22", "created": 1700000010 },
    { "name": "Bob", "role": "viewer", "salt": "cc33", "hash": "dd44", "created": 1700000020 }
  ],
  "allowed_hosts": ["box.example.ts.net", "100.64.0.9"],
  "allow_remote_destructive": true
}"#;

const HUB_JSON: &str = r#"{
  "nodes": [
    { "id": "n01020304", "name": "Home", "url": "http://10.0.0.2:47120", "token": "node-token-1" },
    { "id": "n0a0b0c0d", "name": "Cloud", "url": "https://panel.example.com", "token": "node-token-2" }
  ]
}"#;

/// A Kora home exactly as v0.7.0 leaves it (plus the stopped install).
fn v070_home() -> tempfile::TempDir {
    let h = tempfile::tempdir().unwrap();
    let p = h.path();
    std::fs::write(p.join("servers.json"), serde_json::to_vec_pretty(&Registry { servers: servers() }).unwrap()).unwrap();
    std::fs::write(p.join("secrets.json"), SECRETS_JSON).unwrap();
    std::fs::write(
        p.join("profile.json"),
        serde_json::to_vec_pretty(&Profile { cfx_username: Some("JSixty".into()), cfx_identifier: Some("fivem:42".into()), github_owner: Some("KoraFiveM".into()) }).unwrap(),
    )
    .unwrap();
    std::fs::write(p.join("last-install.json"), serde_json::to_vec_pretty(&request("/srv/dev-den", Some("Main"))).unwrap()).unwrap();
    std::fs::write(p.join("panel.json"), PANEL_JSON).unwrap();
    std::fs::write(p.join("hub.json"), HUB_JSON).unwrap();
    std::fs::create_dir(p.join("paused-installs")).unwrap();
    let st = Stored {
        job: "0123456789abcdef".into(),
        req: request("/srv/dev-den", Some("Main")),
        via_helper: true,
        failed_step: Some("build.clone".into()),
        finished: vec![serde_json::json!({"step": "prepare.git", "status": "done"})],
        saved_at: crate::monitor::now_unix(),
    };
    std::fs::write(p.join("paused-installs/0123456789abcdef.json"), serde_json::to_vec_pretty(&st).unwrap()).unwrap();
    h
}

fn assert_everything_imported(home: &Path) {
    let reg = Registry::load_from(&home.join("servers.json")).unwrap();
    assert_eq!(reg.servers, servers(), "servers, in order");
    let keys = KeyStore::at(home.join("secrets.json"));
    let list = keys.list().unwrap();
    assert_eq!(list.iter().map(|k| (k.name.as_str(), k.last4.as_str(), k.added)).collect::<Vec<_>>(), [("Main", "1111", 1_700_000_001), ("Test server", "2222", 1_700_000_002)]);
    assert_eq!((keys.get("main").unwrap().as_str(), keys.get("TEST SERVER").unwrap().as_str()), (K1, K2));
    let p = ProfileStore::at(home.join("profile.json")).load();
    assert_eq!((p.cfx_username.as_deref(), p.cfx_identifier.as_deref(), p.github_owner.as_deref()), (Some("JSixty"), Some("fivem:42"), Some("KoraFiveM")));
    let answers = crate::installer::saved::load_beside(&home.join("last-install.json")).unwrap();
    assert_eq!(j(&answers), j(&request("/srv/dev-den", Some("Main"))));
    let paused = PausedStore::at(home.join("paused-installs")).list();
    assert_eq!(paused.len(), 1);
    let st = &paused[0];
    assert_eq!((st.job.as_str(), st.via_helper, st.failed_step.as_deref(), st.finished.len()), ("0123456789abcdef", true, Some("build.clone"), 1));
    assert_eq!(j(&st.req), j(&request("/srv/dev-den", Some("Main"))));
    let nodes = NodeStore::at(home.join("hub.json")).load().unwrap();
    assert_eq!(
        nodes.iter().map(|n| (n.id.as_str(), n.name.as_str(), n.url.as_str(), n.token.as_str())).collect::<Vec<_>>(),
        [("n01020304", "Home", "http://10.0.0.2:47120", "node-token-1"), ("n0a0b0c0d", "Cloud", "https://panel.example.com", "node-token-2")]
    );
    let s = Store::open(home).unwrap();
    s.read(|c| {
        let users: Vec<(String, String, String, String, i64)> = c
            .prepare("SELECT name, role, salt, hash, created FROM remote_users ORDER BY seq")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
            .collect::<Result<_, _>>()?;
        assert_eq!(users, [("alice".into(), "operator".into(), "aa11".into(), "bb22".into(), 1_700_000_010), ("Bob".into(), "viewer".into(), "cc33".into(), "dd44".into(), 1_700_000_020)]);
        let ts: Vec<(String, String)> = c.prepare("SELECT login, role FROM remote_tailscale_users ORDER BY login")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        assert_eq!(ts, [("pat@example.com".into(), "viewer".into()), ("sam@example.com".into(), "admin".into())]);
        let hosts: Vec<String> = c.prepare("SELECT host FROM remote_hosts ORDER BY seq")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
        assert_eq!(hosts, ["box.example.ts.net", "100.64.0.9"]);
        let flag = |k: &str| c.query_row("SELECT value FROM remote_settings WHERE key = ?1", [k], |r| r.get::<_, String>(0)).unwrap();
        assert_eq!((flag("enabled"), flag("tailscale"), flag("allow_remote_destructive")), ("1".into(), "1".into(), "1".into()));
        Ok(())
    })
    .unwrap();
}

fn aside(home: &Path, rel: &str) -> PathBuf {
    aside_dir(home).join(rel)
}

#[test]
fn every_field_of_a_v070_home_survives_and_the_old_files_are_moved_aside_not_deleted() {
    let h = v070_home();
    let originals: Vec<(String, Vec<u8>)> = ["servers.json", "secrets.json", "profile.json", "last-install.json", "panel.json", "hub.json", "paused-installs/0123456789abcdef.json"]
        .iter()
        .map(|n| (n.to_string(), std::fs::read(h.path().join(n)).unwrap()))
        .collect();
    assert_everything_imported_after_open(h.path());
    for (name, bytes) in &originals {
        assert!(!h.path().join(name).exists(), "{name} should have moved");
        assert_eq!(&std::fs::read(aside(h.path(), name)).unwrap(), bytes, "{name} is kept byte for byte");
    }
    assert!(!h.path().join("paused-installs").exists(), "the emptied folder goes");
}

fn assert_everything_imported_after_open(home: &Path) {
    drop(Store::open(home).unwrap());
    assert_everything_imported(home);
}

#[cfg(unix)]
#[test]
fn the_files_kept_aside_stay_private() {
    use std::os::unix::fs::PermissionsExt;
    let h = v070_home();
    std::fs::set_permissions(h.path().join("secrets.json"), std::fs::Permissions::from_mode(0o600)).unwrap();
    drop(Store::open(h.path()).unwrap());
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&aside(h.path(), "secrets.json")), 0o600, "moved, not copied: keeps its mode");
    assert_eq!(mode(&aside_dir(h.path())), 0o700);
    assert_eq!(mode(&aside_dir(h.path()).join("paused-installs")), 0o700);
}

#[test]
fn importing_is_idempotent_and_a_crash_between_commit_and_move_is_harmless() {
    let h = v070_home();
    drop(Store::open(h.path()).unwrap());
    // The process died after the commit but before moving: the files are back (same content).
    for n in ["servers.json", "secrets.json", "panel.json", "hub.json", "profile.json", "last-install.json"] {
        std::fs::copy(aside(h.path(), n), h.path().join(n)).unwrap();
    }
    std::fs::create_dir(h.path().join("paused-installs")).unwrap();
    std::fs::copy(aside(h.path(), "paused-installs/0123456789abcdef.json"), h.path().join("paused-installs/0123456789abcdef.json")).unwrap();
    assert_everything_imported_after_open(h.path());
    assert!(!h.path().join("servers.json").exists(), "moved again");
    assert!(backup::list(h.path()).is_empty(), "same content: no replacement, so no backup");
    // and again, with nothing there
    assert_everything_imported_after_open(h.path());
    // keys were not doubled
    assert_eq!(KeyStore::at(h.path().join("secrets.json")).list().unwrap().len(), 2);
}

#[test]
fn an_import_that_fails_half_way_changes_nothing_and_is_retried_safely() {
    let h = v070_home();
    *TEST_FAIL_HOME.lock().unwrap() = Some(h.path().to_path_buf());
    let e = Store::open(h.path()).unwrap_err();
    *TEST_FAIL_HOME.lock().unwrap() = None;
    assert!(e.message.contains("half-way"), "{e:?}");
    for n in ["servers.json", "secrets.json", "profile.json", "last-install.json", "panel.json", "hub.json"] {
        assert!(h.path().join(n).is_file(), "{n} untouched");
    }
    assert!(!aside_dir(h.path()).exists() || std::fs::read_dir(aside_dir(h.path())).unwrap().count() == 0);
    // nothing was committed: not a single row
    let c = rusqlite::Connection::open(super::super::db_path(h.path())).unwrap();
    for t in ["servers", "license_keys", "hub_nodes", "remote_users", "paused_installs"] {
        let n: i64 = c.query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0, "{t}");
    }
    drop(c);
    // the retry imports everything
    assert_everything_imported_after_open(h.path());
}

#[test]
fn a_damaged_key_file_is_a_guided_problem_never_replaced_and_other_files_still_import() {
    let h = v070_home();
    let junk = format!("{{\"keys\": [{{\"name\": \"Main\", \"key\": \"{K1}\" ");
    std::fs::write(h.path().join("secrets.json"), &junk).unwrap();
    let keys = KeyStore::at(h.path().join("secrets.json"));
    for e in [keys.list().unwrap_err(), keys.add("New", K2).unwrap_err(), keys.get("Main").unwrap_err(), keys.remove("Main").unwrap_err()] {
        assert_eq!(e.code, crate::problem::codes::KEYS_UNREADABLE);
        assert!(e.message.contains("no key was lost"), "{}", e.message);
        let all = format!("{} {:?}", e.message, e.details);
        assert!(!all.contains("AAAAAAAA"), "no key in an error: {all}");
    }
    assert_eq!(std::fs::read_to_string(h.path().join("secrets.json")).unwrap(), junk, "left exactly as it was");
    // the rest of Kora works
    assert_eq!(Registry::load_from(&h.path().join("servers.json")).unwrap().servers, servers());
    // wrong type: serde would have quoted the value; we do not
    std::fs::write(h.path().join("secrets.json"), format!("{{\"keys\": \"{K1}\"}}")).unwrap();
    let e = keys.list().unwrap_err();
    assert!(!format!("{} {:?}", e.message, e.details).contains("AAAAAAAA"));
    // written by a newer Kora
    std::fs::write(h.path().join("secrets.json"), r#"{"version": 99, "keys": []}"#).unwrap();
    assert_eq!(keys.add("New", K2).unwrap_err().code, crate::problem::codes::KEYS_UNREADABLE);
    // fixing the file is enough: it is imported at the next start
    std::fs::write(h.path().join("secrets.json"), SECRETS_JSON).unwrap();
    assert_eq!(keys.list().unwrap().len(), 2);
    assert!(!h.path().join("secrets.json").exists());
}

#[test]
fn a_damaged_server_list_or_node_file_blocks_only_its_own_part() {
    let h = v070_home();
    std::fs::write(h.path().join("servers.json"), "{ nope").unwrap();
    std::fs::write(h.path().join("hub.json"), "[1]").unwrap();
    std::fs::write(h.path().join("panel.json"), r#"{"users":[{"name":"x","role":"emperor","salt":"a","hash":"b"}]}"#).unwrap();
    let e = Registry::load_from(&h.path().join("servers.json")).unwrap_err().to_string();
    assert!(e.contains("server list") && e.contains("servers.json") && e.contains("exactly as it was"), "{e}");
    assert!(Registry::default().save_to(&h.path().join("servers.json")).is_err(), "never written over");
    assert!(NodeStore::at(h.path().join("hub.json")).load().is_err());
    assert!(std::fs::read_to_string(h.path().join("servers.json")).unwrap() == "{ nope");
    assert!(h.path().join("panel.json").exists());
    // keys and the rest are fine
    assert_eq!(KeyStore::at(h.path().join("secrets.json")).list().unwrap().len(), 2);
}

#[test]
fn damaged_files_that_only_save_typing_are_set_aside_and_kora_carries_on() {
    let h = v070_home();
    std::fs::write(h.path().join("profile.json"), "{ nope").unwrap();
    std::fs::write(h.path().join("last-install.json"), "").unwrap();
    std::fs::write(h.path().join("paused-installs/0123456789abcdef.json"), "garbage").unwrap();
    drop(Store::open(h.path()).unwrap());
    assert_eq!(ProfileStore::at(h.path().join("profile.json")).load(), Profile::default());
    assert!(crate::installer::saved::load_beside(&h.path().join("last-install.json")).is_none());
    assert!(PausedStore::at(h.path().join("paused-installs")).list().is_empty());
    assert_eq!(std::fs::read(aside(h.path(), "profile.json")).unwrap(), b"{ nope", "kept, not lost");
    assert_eq!(std::fs::read(aside(h.path(), "paused-installs/0123456789abcdef.json")).unwrap(), b"garbage");
    assert_eq!(KeyStore::at(h.path().join("secrets.json")).list().unwrap().len(), 2);
}

#[cfg(unix)]
#[test]
fn an_old_file_that_is_a_link_is_not_followed() {
    let h = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    std::fs::write(elsewhere.path().join("x.json"), SECRETS_JSON).unwrap();
    std::os::unix::fs::symlink(elsewhere.path().join("x.json"), h.path().join("secrets.json")).unwrap();
    let e = KeyStore::at(h.path().join("secrets.json")).list().unwrap_err();
    assert_eq!(e.code, crate::problem::codes::KEYS_UNREADABLE);
    assert!(e.details.unwrap_or_default().contains("link"));
    assert!(std::fs::symlink_metadata(h.path().join("secrets.json")).unwrap().file_type().is_symlink(), "left alone");
}

#[test]
fn old_shapes_from_v06_are_read_and_secrets_an_old_version_wrote_are_dropped() {
    let h = tempfile::tempdir().unwrap();
    // v0.6.x: no license_key_name, and a database password that an early version serialised
    let mut v = serde_json::to_value(request("/srv/old", None)).unwrap();
    v.as_object_mut().unwrap().remove("license_key_name");
    v["license_key"] = serde_json::json!("cfxk_SHOULDNOTSTAY_9999");
    v["db_admin"]["password"] = serde_json::json!("hunter2");
    std::fs::write(h.path().join("last-install.json"), serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    // a registry with no `servers` key at all, and secrets.json with no version and a key without `added`
    std::fs::write(h.path().join("servers.json"), "{}").unwrap();
    std::fs::write(h.path().join("secrets.json"), format!(r#"{{"keys":[{{"name":"Old","key":"{K1}"}}]}}"#)).unwrap();
    std::fs::write(h.path().join("hub.json"), "").unwrap_or(());
    drop(Store::open(h.path()).unwrap_or_else(|e| panic!("{e:?}")));
    let a = crate::installer::saved::load_beside(&h.path().join("last-install.json")).unwrap();
    assert_eq!((a.license_key, a.license_key_name, a.db_admin.unwrap().password), (None, None, None));
    let all: Vec<u8> =
        std::fs::read_dir(super::super::store_dir(h.path())).unwrap().flatten().filter(|e| e.path().is_file() && e.file_name() != "kora.db").flat_map(|e| std::fs::read(e.path()).unwrap()).collect();
    let _ = all;
    let db = std::fs::read(super::super::db_path(h.path())).unwrap();
    assert!(!db.windows(7).any(|w| w == b"hunter2") && !db.windows(10).any(|w| w == b"SHOULDNOTS"), "no secret in the store");
    assert!(Registry::load_from(&h.path().join("servers.json")).unwrap().servers.is_empty());
    let k = KeyStore::at(h.path().join("secrets.json")).list().unwrap();
    assert_eq!((k[0].name.as_str(), k[0].added), ("Old", 0));
}

#[test]
fn names_that_only_differ_by_case_keep_every_key() {
    let h = tempfile::tempdir().unwrap();
    std::fs::write(h.path().join("secrets.json"), format!(r#"{{"keys":[{{"name":"Main","key":"{K1}"}},{{"name":"main","key":"{K2}"}}]}}"#)).unwrap();
    let keys = KeyStore::at(h.path().join("secrets.json"));
    let list = keys.list().unwrap();
    assert_eq!(list.iter().map(|k| k.name.as_str()).collect::<Vec<_>>(), ["Main", "main (2)"]);
    assert_eq!((keys.get("Main").unwrap().as_str(), keys.get("main (2)").unwrap().as_str()), (K1, K2));
}

#[test]
fn an_old_file_that_comes_back_after_a_downgrade_is_imported_again_after_a_backup() {
    let h = v070_home();
    drop(Store::open(h.path()).unwrap());
    // an older Kora ran again, added a server and wrote servers.json
    let mut s = servers();
    s.push(ServerEntry { name: "Added by old Kora".into(), dir: "/srv/new".into(), mode: Mode::Production, ports: PortsRecord { game: 30130, txadmin: 40130 }, created_unix: 5 });
    std::fs::write(h.path().join("servers.json"), serde_json::to_vec_pretty(&Registry { servers: s.clone() }).unwrap()).unwrap();
    assert_eq!(Registry::load_from(&h.path().join("servers.json")).unwrap().servers, s);
    assert_eq!(backup::list(h.path()).len(), 1, "a backup of the store before it was replaced");
    assert_eq!(std::fs::read_dir(aside_dir(h.path())).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().starts_with("servers.json")).count(), 2, "both generations kept");
}

#[test]
fn stopped_installs_that_are_too_old_are_kept_aside_but_not_imported() {
    let h = tempfile::tempdir().unwrap();
    std::fs::create_dir(h.path().join("paused-installs")).unwrap();
    let st = Stored { job: "oldjob".into(), req: request("/srv/a", Some("Main")), via_helper: false, failed_step: None, finished: vec![], saved_at: 1 };
    std::fs::write(h.path().join("paused-installs/oldjob.json"), serde_json::to_vec(&st).unwrap()).unwrap();
    assert!(PausedStore::at(h.path().join("paused-installs")).list().is_empty());
    assert!(aside(h.path(), "paused-installs/oldjob.json").is_file());
}

#[test]
fn two_threads_opening_a_fresh_home_import_once() {
    let h = v070_home();
    let p = h.path().to_path_buf();
    let hs: Vec<_> = (0..6)
        .map(|_| {
            let p = p.clone();
            std::thread::spawn(move || drop(Store::open(&p).unwrap()))
        })
        .collect();
    for t in hs {
        t.join().unwrap();
    }
    assert_everything_imported(h.path());
}

#[test]
fn a_fresh_home_without_old_files_creates_nothing_extra() {
    let h = tempfile::tempdir().unwrap();
    drop(Store::open(h.path()).unwrap());
    assert!(!aside_dir(h.path()).exists());
    assert!(!any_legacy_files(h.path()));
}

#[test]
fn the_way_back_export_gives_the_old_files_again_with_every_field() {
    let h = v070_home();
    // what the old Kora had, parsed (whitespace and key order do not matter)
    let load = |p: &Path| serde_json::from_slice::<serde_json::Value>(&std::fs::read(p).unwrap()).unwrap();
    let originals: Vec<(&str, serde_json::Value)> = ["servers.json", "secrets.json", "profile.json", "last-install.json", "panel.json", "hub.json", "paused-installs/0123456789abcdef.json"]
        .iter()
        .map(|n| (*n, load(&h.path().join(n))))
        .collect();
    let s = Store::open(h.path()).unwrap();
    let out = tempfile::tempdir().unwrap();
    let files = crate::store::export::export_json(&s, &out.path().join("back")).unwrap();
    assert_eq!(files.len(), 7, "{files:?}");
    for (name, original) in originals {
        assert_eq!(load(&out.path().join("back").join(name)), original, "{name}");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for f in &files {
            assert_eq!(std::fs::metadata(f).unwrap().permissions().mode() & 0o777, 0o600, "{}", f.display());
        }
    }
    // and the exported folder, put in a home, is imported to the same state (round trip both ways)
    let again = tempfile::tempdir().unwrap();
    for f in &files {
        let rel = f.strip_prefix(out.path().join("back")).unwrap();
        std::fs::create_dir_all(again.path().join(rel).parent().unwrap()).unwrap();
        std::fs::copy(f, again.path().join(rel)).unwrap();
    }
    assert_everything_imported_after_open(again.path());
}

#[test]
fn an_extra_home_file_moves_in_as_a_document_and_comes_back_out() {
    let h = tempfile::tempdir().unwrap();
    let consent = serde_json::json!({"granted": true, "asked_at": 1700000000, "answer": {"who": "sam", "kept": [1, 2]}});
    std::fs::write(h.path().join("reach-consent.json"), serde_json::to_vec_pretty(&consent).unwrap()).unwrap();
    let s = Store::open(h.path()).unwrap();
    assert_eq!(crate::store::documents::get(&s, "reach-consent").unwrap(), Some(consent.clone()));
    assert!(!h.path().join("reach-consent.json").exists() && aside(h.path(), "reach-consent.json").is_file());
    // read-modify-write in one transaction, and the way back
    crate::store::documents::update(&s, "reach-consent", |d| {
        d["granted"] = false.into();
        Ok(())
    })
    .unwrap();
    let out = tempfile::tempdir().unwrap();
    crate::store::export::export_json(&s, out.path()).unwrap();
    let back: serde_json::Value = serde_json::from_slice(&std::fs::read(out.path().join("reach-consent.json")).unwrap()).unwrap();
    assert_eq!(back["granted"], false);
    // idempotent: a second open does nothing; a damaged file blocks only its own part and is left alone
    drop(s);
    std::fs::write(h.path().join("reach-consent.json"), "{ nope").unwrap();
    let s = Store::open(h.path()).unwrap();
    assert!(s.blocked("reach-consent").is_some());
    assert_eq!(std::fs::read_to_string(h.path().join("reach-consent.json")).unwrap(), "{ nope");
    assert!(crate::store::documents::valid_name("reach-consent") && !crate::store::documents::valid_name("Bad Name"));
}
