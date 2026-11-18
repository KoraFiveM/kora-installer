//! Several PROCESSES using one Kora home at the same moment, and a process killed in the middle of a write.
//!
//! The test binary starts itself again as workers (`worker_entry`, selected by environment variables):
//! no extra programs are needed, on Linux or Windows.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use kora_core::api::{JobRegistry, JobState, Op};
use kora_core::installer::registry::{PortsRecord, Registry, ServerEntry};
use kora_core::installer::Mode;
use kora_core::keystore::KeyStore;
use kora_core::store::{meta_get, meta_set, Store};

fn spawn(role: &str, home: &Path, id: usize, count: usize) -> std::process::Child {
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "worker_entry", "--nocapture", "--test-threads=1"])
        .env("KORA_STORE_ROLE", role)
        .env("KORA_STORE_HOME", home)
        .env("KORA_STORE_ID", id.to_string())
        .env("KORA_STORE_COUNT", count.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap()
}

fn run_workers(role: &str, home: &Path, workers: usize, count: usize) {
    let kids: Vec<_> = (0..workers).map(|i| spawn(role, home, i, count)).collect();
    for mut k in kids {
        let st = k.wait().unwrap();
        assert!(st.success(), "worker of role {role} failed: {st:?}");
    }
}

fn entry(name: String, n: u64) -> ServerEntry {
    ServerEntry {
        source: None,
        versions: None,
        name,
        dir: PathBuf::from(format!("/srv/{n}")),
        mode: Mode::Production,
        ports: PortsRecord { game: 30000 + n as u16, txadmin: 40000 + n as u16 },
        created_unix: n,
    }
}

/// Not a test of its own: the body of a worker process. Does nothing in a normal run.
#[test]
fn worker_entry() {
    let Ok(role) = std::env::var("KORA_STORE_ROLE") else { return };
    let home = PathBuf::from(std::env::var("KORA_STORE_HOME").unwrap());
    let id: usize = std::env::var("KORA_STORE_ID").unwrap().parse().unwrap();
    let count: usize = std::env::var("KORA_STORE_COUNT").unwrap().parse().unwrap();
    match role.as_str() {
        "keys" => {
            let keys = KeyStore::at(home.join("secrets.json"));
            for i in 0..count {
                keys.add(&format!("key-{id}-{i}"), &format!("cfxk_process_{id:03}_key_{i:04}")).unwrap();
            }
        }
        "registry" => {
            for i in 0..count {
                Registry::update_at(&home.join("servers.json"), |r| {
                    r.upsert(entry(format!("srv-{id}-{i}"), (id * 1000 + i) as u64));
                    Ok(())
                })
                .unwrap();
            }
        }
        "counter" => {
            for _ in 0..count {
                let s = Store::open(&home).unwrap();
                s.write(|tx| {
                    let n: u64 = meta_get(tx, "counter")?.and_then(|v| v.parse().ok()).unwrap_or(0);
                    meta_set(tx, "counter", &(n + 1).to_string())
                })
                .unwrap();
            }
        }
        "die_mid_write" => {
            // A write that never commits: rows written, then the process vanishes.
            let s = Store::open(&home).unwrap();
            let _ = s.write(|tx| {
                for i in 0..500 {
                    tx.execute("INSERT INTO license_keys (name, name_folded, added) VALUES (?1, ?1, 0)", [format!("half-{i}")])?;
                    tx.execute("INSERT INTO license_key_material (key_id, material) VALUES (last_insert_rowid(), 'cfxk_never_committed')", [])?;
                }
                std::process::abort();
                #[allow(unreachable_code)]
                Ok(())
            });
        }
        "write_until_killed" => {
            let keys = KeyStore::at(home.join("secrets.json"));
            let mut i = 0u64;
            loop {
                keys.add(&format!("k{i}"), &format!("cfxk_until_killed_{i:06}")).unwrap();
                i += 1;
            }
        }
        "job_hang" => {
            // A durable job that shows progress and then never ends (the process is killed meanwhile).
            let reg = JobRegistry::durable_in(home.clone());
            // The id file is written only once the event has been recorded, so the test knows the moment
            // it may kill this process (a fixed sleep was a guess that a slow machine lost).
            let (shown, shown_rx) = std::sync::mpsc::channel::<()>();
            let id = reg
                .spawn(Op::UpdateApply, Some("srv".into()), move |j| {
                    j.event(serde_json::json!({"step": "download", "status": "running"}));
                    let _ = shown.send(());
                    std::thread::sleep(std::time::Duration::from_secs(600));
                    Ok(serde_json::Value::Null)
                })
                .unwrap();
            shown_rx.recv_timeout(std::time::Duration::from_secs(30)).expect("the job never showed its event");
            std::fs::write(home.join("job-id.txt"), id.0).unwrap();
            std::thread::sleep(std::time::Duration::from_secs(600));
        }
        other => panic!("unknown role {other}"),
    }
}

#[test]
fn processes_adding_keys_at_once_lose_nothing() {
    let h = tempfile::tempdir().unwrap();
    run_workers("keys", h.path(), 4, 15);
    let list = KeyStore::at(h.path().join("secrets.json")).list().unwrap();
    assert_eq!(list.len(), 60, "every add from every process is there");
}

#[test]
fn processes_changing_the_registry_at_once_lose_nothing() {
    let h = tempfile::tempdir().unwrap();
    run_workers("registry", h.path(), 4, 15);
    let r = Registry::load_from(&h.path().join("servers.json")).unwrap();
    assert_eq!(r.servers.len(), 60);
    for id in 0..4 {
        for i in 0..15 {
            assert!(r.find(&format!("srv-{id}-{i}")).is_some(), "srv-{id}-{i}");
        }
    }
}

#[test]
fn read_modify_write_counters_in_processes_do_not_lose_updates() {
    let h = tempfile::tempdir().unwrap();
    run_workers("counter", h.path(), 4, 40);
    let s = Store::open(h.path()).unwrap();
    assert_eq!(s.read(|c| meta_get(c, "counter")).unwrap().as_deref(), Some("160"));
}

#[test]
fn processes_starting_together_on_a_fresh_home_create_migrate_and_import_once() {
    let h = tempfile::tempdir().unwrap();
    // old files in place too: the import must happen exactly once
    std::fs::write(h.path().join("secrets.json"), r#"{"version":1,"keys":[{"name":"Main","key":"cfxk_AAAAAAAAAAAAAAAA_1111","added":1}]}"#).unwrap();
    std::fs::write(h.path().join("servers.json"), serde_json::to_vec(&Registry { servers: vec![entry("only".into(), 1)] }).unwrap()).unwrap();
    run_workers("counter", h.path(), 6, 2);
    let s = Store::open(h.path()).unwrap();
    let n: i64 = s.read(|c| Ok(c.query_row("SELECT COUNT(*) FROM schema_migrations", [], |r| r.get(0))?)).unwrap();
    assert_eq!(n as usize, kora_core::store::schema::MIGRATIONS.len(), "each migration recorded once");
    assert_eq!(KeyStore::at(h.path().join("secrets.json")).list().unwrap().len(), 1, "imported once");
    assert_eq!(Registry::load_from(&h.path().join("servers.json")).unwrap().servers.len(), 1);
    assert_eq!(s.read(|c| meta_get(c, "counter")).unwrap().as_deref(), Some("12"));
    assert!(h.path().join("store/before-sqlite/secrets.json").is_file());
}

fn assert_consistent(home: &Path) -> usize {
    let s = Store::open(home).unwrap();
    s.read(|c| {
        let ok: String = c.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        assert_eq!(ok, "ok");
        // every key has its material: a key is never half there
        let orphans: i64 = c.query_row("SELECT COUNT(*) FROM license_keys k LEFT JOIN license_key_material m ON m.key_id = k.id WHERE m.key_id IS NULL", [], |r| r.get(0))?;
        assert_eq!(orphans, 0, "a key without its value");
        let never: i64 = c.query_row("SELECT COUNT(*) FROM license_key_material WHERE material = 'cfxk_never_committed'", [], |r| r.get(0))?;
        assert_eq!(never, 0, "a write that never committed left rows behind");
        Ok(c.query_row("SELECT COUNT(*) FROM license_keys", [], |r| r.get::<_, i64>(0))? as usize)
    })
    .unwrap()
}

#[test]
fn a_process_that_dies_in_the_middle_of_a_write_leaves_a_consistent_database() {
    let h = tempfile::tempdir().unwrap();
    KeyStore::at(h.path().join("secrets.json")).add("Before", "cfxk_before_the_crash_0001").unwrap();
    let mut k = spawn("die_mid_write", h.path(), 0, 0);
    assert!(!k.wait().unwrap().success(), "it aborted");
    assert_eq!(assert_consistent(h.path()), 1, "only the committed key");
    // and the store keeps working afterwards
    KeyStore::at(h.path().join("secrets.json")).add("After", "cfxk_after_the_crash_0002").unwrap();
    assert_eq!(assert_consistent(h.path()), 2);
}

#[test]
fn a_process_killed_at_a_random_moment_between_writes_leaves_a_consistent_database() {
    for delay_ms in [40u64, 90, 160] {
        let h = tempfile::tempdir().unwrap();
        KeyStore::at(h.path().join("secrets.json")).add("Seed", "cfxk_seed_for_the_kill_0001").unwrap();
        let mut k = spawn("write_until_killed", h.path(), 0, 0);
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
        let _ = k.kill(); // SIGKILL / TerminateProcess: no cleanup runs
        let _ = k.wait();
        let n = assert_consistent(h.path());
        assert!(n >= 1);
        // the keys that are there are complete and readable
        let keys = KeyStore::at(h.path().join("secrets.json"));
        for info in keys.list().unwrap() {
            assert!(keys.get(&info.name).unwrap().starts_with("cfxk_"));
        }
    }
}

#[test]
fn a_panel_killed_while_a_job_runs_leaves_an_interrupted_job_for_the_next_panel() {
    let h = tempfile::tempdir().unwrap();
    let mut k = spawn("job_hang", h.path(), 0, 0);
    let idfile = h.path().join("job-id.txt");
    let started = std::time::Instant::now();
    while !idfile.exists() {
        assert!(started.elapsed() < std::time::Duration::from_secs(20), "the worker never started its job");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let id = std::fs::read_to_string(&idfile).unwrap();
    // the worker wrote the id only after its event was recorded (the first event of a quiet job is stored at once)
    k.kill().unwrap();
    let _ = k.wait();
    // the next panel starts (for real: the pid of the dead process is really gone)
    let next = JobRegistry::durable_in(h.path().to_path_buf());
    let j = next.get(&id).expect("the job is remembered");
    assert_eq!(j.state, JobState::Interrupted);
    assert_eq!(j.events.len(), 1, "what it had shown is kept");
    let e: serde_json::Value = serde_json::from_str(j.error.as_deref().unwrap()).unwrap();
    assert_eq!(e["code"], "job.interrupted");
}
