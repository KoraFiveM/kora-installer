use super::*;

fn home() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[test]
fn a_new_store_is_created_migrated_and_private() {
    let h = home();
    let s = Store::open(h.path()).unwrap();
    assert_eq!(s.schema_version().unwrap(), schema::latest());
    assert!(db_path(h.path()).is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&store_dir(h.path())), 0o700);
        for p in std::fs::read_dir(store_dir(h.path())).unwrap().flatten().map(|e| e.path()).filter(|p| p.is_file()) {
            assert_eq!(mode(&p), 0o600, "{}", p.display());
        }
        // writes make the -wal / -shm files appear while a connection is open: they are private too
        s.write(|tx| meta_set(tx, "k", "v")).unwrap();
        let mut seen = 0;
        for suffix in privacy::SIDE_SUFFIXES {
            let p = privacy::side_file(&db_path(h.path()), suffix);
            if p.exists() {
                seen += 1;
                assert_eq!(mode(&p), 0o600, "{}", p.display());
            }
        }
        assert!(seen > 0, "WAL mode should have left a -wal or -shm file");
    }
}

#[test]
fn opening_twice_does_not_migrate_twice() {
    let h = home();
    drop(Store::open(h.path()).unwrap());
    let s = Store::open(h.path()).unwrap();
    let n: i64 = s.read(|c| Ok(c.query_row("SELECT COUNT(*) FROM schema_migrations", [], |r| r.get(0))?)).unwrap();
    assert_eq!(n as usize, schema::MIGRATIONS.len());
}

#[test]
fn a_newer_database_is_refused_politely_and_left_alone() {
    let h = home();
    drop(Store::open(h.path()).unwrap());
    {
        let c = Connection::open(db_path(h.path())).unwrap();
        c.execute("INSERT INTO schema_migrations (version, name, applied_at, kora_version) VALUES (999, 'future', 0, '9.9.9')", []).unwrap();
    }
    let before = std::fs::read(db_path(h.path())).unwrap().len();
    let e = Store::open(h.path()).unwrap_err();
    assert_eq!(e.code, codes::STORE_NEWER, "{e:?}");
    assert!(e.message.contains("newer version of Kora"), "{}", e.message);
    assert!(!e.actions.is_empty());
    assert!(std::fs::read(db_path(h.path())).unwrap().len() >= before);
    let c = Connection::open(db_path(h.path())).unwrap();
    assert_eq!(current_version(&c).unwrap(), 999, "untouched");
}

#[test]
fn an_upgrade_of_existing_data_makes_a_backup_first() {
    let h = home();
    {
        // a database that is at version 1 only, with a row
        let c = Connection::open_with_flags(db_path(h.path()), OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE);
        drop(c);
        let s = Store::open_raw(h.path()).unwrap();
        let tx = Transaction::new_unchecked(&s.conn, TransactionBehavior::Immediate).unwrap();
        tx.execute_batch("CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY NOT NULL, name TEXT NOT NULL, applied_at INTEGER NOT NULL, kora_version TEXT NOT NULL)").unwrap();
        tx.execute_batch(schema::MIGRATIONS[0].sql).unwrap();
        tx.execute("INSERT INTO schema_migrations VALUES (1, 'x', 0, '0.7.0')", []).unwrap();
        tx.execute("INSERT INTO profile VALUES ('github_owner', 'sam')", []).unwrap();
        tx.commit().unwrap();
    }
    let s = Store::open(h.path()).unwrap();
    assert_eq!(s.schema_version().unwrap(), schema::latest());
    let backups = backup::list(h.path());
    assert_eq!(backups.len(), 1, "{backups:?}");
    let b = Connection::open(&backups[0]).unwrap();
    assert_eq!(current_version(&b).unwrap(), 1, "the copy is the old state");
    let who: String = b.query_row("SELECT value FROM profile WHERE field='github_owner'", [], |r| r.get(0)).unwrap();
    assert_eq!(who, "sam");
}

#[test]
fn a_failed_change_rolls_back() {
    let h = home();
    let s = Store::open(h.path()).unwrap();
    let r: Result<(), Problem> = s.write(|tx| {
        meta_set(tx, "a", "1")?;
        Err(Problem::generic("no"))
    });
    assert!(r.is_err());
    assert_eq!(s.read(|c| meta_get(c, "a")).unwrap(), None);
}

#[test]
fn a_damaged_file_is_reported_not_replaced() {
    let h = home();
    std::fs::create_dir_all(store_dir(h.path())).unwrap();
    std::fs::write(db_path(h.path()), b"this is not a database, but it is long enough to be mistaken for a header block....................").unwrap();
    let e = Store::open(h.path()).unwrap_err();
    assert_eq!(e.code, codes::STORE_UNREADABLE, "{e:?}");
    assert!(std::fs::read(db_path(h.path())).unwrap().starts_with(b"this is not"));
}

#[cfg(unix)]
#[test]
fn a_link_in_place_of_the_database_is_refused() {
    let h = home();
    std::fs::create_dir_all(store_dir(h.path())).unwrap();
    let victim = h.path().join("victim");
    std::fs::write(&victim, "precious").unwrap();
    std::os::unix::fs::symlink(&victim, db_path(h.path())).unwrap();
    assert!(Store::open(h.path()).is_err());
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "precious");
}

#[test]
fn an_on_demand_backup_is_a_private_consistent_copy() {
    let h = home();
    let s = Store::open(h.path()).unwrap();
    s.write(|tx| meta_set(tx, "hello", "world")).unwrap();
    let p = backup::backup_now(&s, None).unwrap();
    let b = Connection::open(&p).unwrap();
    assert_eq!(meta_get(&b, "hello").unwrap().as_deref(), Some("world"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
    }
    assert!(backup::backup_now(&s, Some(&p)).is_err(), "never overwrites");
}
