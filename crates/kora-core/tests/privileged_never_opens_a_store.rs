//! The privileged side (a child of the administrator service, `KORA_SERVICE=1`) must never open a person's store:
//! root would create `-wal` / `-shm` files and change permissions in the person's own data folder.
//!
//! This test runs as its own process (the guard is process-wide). It makes a real store in a temporary Kora home, puts
//! the process in service mode exactly as `kora` does at start (`privsvc::exec::scrub_overrides`), then runs the paths a
//! privileged child takes: every administrator step (run and undo), the migration's registry peek, and the
//! reading of the server list that a step could do. It fails if anything tried to open the store (`refused_opens`) and
//! if a single file in the store folder changed.

use std::path::Path;

use kora_core::installer::system_ops::{run_admin_step, Known};

fn listing(dir: &Path) -> Vec<(String, u64)> {
    let mut v: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap()).map(|e| (e.file_name().to_string_lossy().into_owned(), e.metadata().unwrap().len())).collect();
    v.sort();
    v
}

#[test]
fn no_privileged_path_opens_a_store() {
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("KORA_HOME", home.path());
    // A real store with a server in it, made the ordinary way (before the process is put in service mode).
    {
        let reg = kora_core::installer::registry::Registry::default();
        reg.save().unwrap();
    }
    let store_dir = home.path().join("store");
    assert!(store_dir.join("kora.db").exists());
    let before = listing(&store_dir);

    let server = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(server.path().join(".kora")).unwrap();
    let req = kora_core::installer::InstallRequest {
        source: None,
        versions: None,
        mode: kora_core::installer::Mode::Production,
        name: "Main".into(),
        dir: server.path().to_path_buf(),
        hostname: "Main".into(),
        license_key: None,
        license_key_name: None,
        cfx_identifier: None,
        ports: kora_core::installer::Ports::default(),
        txadmin_public: false,
        artifact_url: None,
        game_build: None,
        resources: kora_core::installer::ResourceSelection::All,
        db: kora_core::installer::DbChoice::UseExisting,
        service: true,
        firewall: true,
        github_owner: None,
        catalog: None,
        dry_run: true,
        db_admin: None,
        start: false,
        app: None,
    };
    std::fs::write(server.path().join(".kora/request.json"), serde_json::to_vec(&req).unwrap()).unwrap();

    std::env::set_var("KORA_SERVICE", "1");
    kora_core::privsvc::exec::scrub_overrides();
    assert_eq!(kora_core::store::refused_opens(), 0);

    // Every step, run and undo (they may fail here: nothing is installed; what matters is that they never open the store).
    for undo in [false, true] {
        for step in kora_core::installer::system_ops::RUN_STEPS {
            let known: Known = if undo { Some(("Main".to_string(), 30120u16, 40120u16)) } else { None };
            let _ = run_admin_step(undo, step, server.path(), false, false, known, &mut |_| {});
        }
    }
    // An undo that is NOT told the name (the old fallback read the person's server list).
    let _ = run_admin_step(true, "system.service", server.path(), false, false, None, &mut |_| {});

    // The migration's look at the list is a read-only peek, not a store open.
    let _ = kora_core::store::peek::registered_server_folders(home.path());

    assert_eq!(kora_core::store::refused_opens(), 0, "something on the privileged side tried to open a person's store");
    // ...and the guard really is on: an open IS refused and counted.
    assert!(kora_core::store::Store::open(home.path()).is_err());
    assert_eq!(kora_core::store::refused_opens(), 1);
    assert_eq!(listing(&store_dir), before, "the store folder was not touched");
}
