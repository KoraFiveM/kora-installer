//! Backups of the store itself (not of servers or their databases).
//!
//! A backup is a consistent copy made by SQLite while others may be using the database (`VACUUM INTO`,
//! which reads one snapshot), written into `<kora home>/store/backups/` as a private file. One is made
//! before every schema migration of existing data and before re-importing old JSON files, and on demand
//! with `kora store backup`. The newest [`KEEP`] are kept. A later "backups and move to another machine"
//! workstream can build on [`backup_to`].

use std::path::{Path, PathBuf};

use crate::problem::{Action, Problem};

use super::{codes, privacy, Store};

/// How many automatic backups are kept (on-demand ones are never pruned).
pub const KEEP: usize = 10;

pub fn backups_dir(home: &Path) -> PathBuf {
    super::store_dir(home).join("backups")
}

fn failed(e: impl std::fmt::Display) -> Problem {
    Problem::new(codes::STORE_UNREADABLE, "Kora could not make a copy of its data file, so it stopped before changing anything.").with_details(e.to_string()).with_action(Action::CopyDetails)
}

/// Copy the store to `dest` (which must not exist). The copy is private from the first instant.
pub fn backup_to(s: &Store, dest: &Path) -> Result<(), Problem> {
    if dest.exists() {
        return Err(failed(format!("{} already exists", dest.display())));
    }
    // An empty private file first: VACUUM INTO accepts an existing zero-length target.
    privacy::ensure_private_file(dest)?;
    let r = s.read(|c| {
        c.execute("VACUUM INTO ?1", [dest.to_string_lossy().as_ref()])?;
        Ok(())
    });
    if let Err(e) = r {
        let _ = std::fs::remove_file(dest);
        return Err(e);
    }
    privacy::tighten(dest);
    Ok(())
}

fn stamp() -> String {
    let secs = crate::monitor::now_unix();
    // Sortable and unique enough: seconds plus a counter for two in the same second.
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    format!("{secs}-{:03}", N.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 1000)
}

/// An on-demand backup (`kora store backup`): returns where it went.
pub fn backup_now(s: &Store, to: Option<&Path>) -> Result<PathBuf, Problem> {
    let dest = match to {
        Some(p) => p.to_path_buf(),
        None => {
            let dir = backups_dir(s.home());
            privacy::ensure_private_dir(s.home(), &super::store_dir(s.home()))?;
            if !dir.is_dir() {
                std::fs::create_dir_all(&dir).map_err(failed)?;
                privacy::tighten_dir(&dir);
            }
            dir.join(format!("kora-{}-manual.db", stamp()))
        }
    };
    backup_to(s, &dest)?;
    Ok(dest)
}

/// Before a schema migration of existing data.
pub fn backup_before_migration(s: &Store, from: u32, to: u32) -> Result<PathBuf, Problem> {
    automatic(s, &format!("before-v{to}-from-v{from}"))
}

/// Before re-importing old JSON files over existing data.
pub fn backup_before_import(s: &Store) -> Result<PathBuf, Problem> {
    automatic(s, "before-import")
}

fn automatic(s: &Store, why: &str) -> Result<PathBuf, Problem> {
    let dir = backups_dir(s.home());
    std::fs::create_dir_all(&dir).map_err(failed)?;
    privacy::tighten_dir(&dir);
    let dest = dir.join(format!("kora-{}-{why}.db", stamp()));
    backup_to(s, &dest)?;
    prune(&dir);
    Ok(dest)
}

/// Keep the newest [`KEEP`] automatic backups.
fn prune(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut autos: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "db") && !p.to_string_lossy().ends_with("-manual.db")).collect();
    autos.sort();
    while autos.len() > KEEP {
        let _ = std::fs::remove_file(autos.remove(0));
    }
}

/// The backups there are, newest last.
pub fn list(home: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(backups_dir(home)) else { return vec![] };
    let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "db")).collect();
    v.sort();
    v
}

// ---- restore -----------------------------------------------------------------------------------------

fn restore_problem(why: &str, details: impl std::fmt::Display) -> Problem {
    Problem::new(codes::STORE_UNREADABLE, format!("Kora did not restore that backup ({why}). Nothing was changed.")).with_details(details.to_string()).with_action(Action::CopyDetails)
}

/// What a backup file is, found out WITHOUT touching it (opened read-only).
#[derive(Debug, PartialEq, Eq)]
pub struct BackupFacts {
    pub schema_version: u32,
}

/// Does `backup` open as a Kora data file, pass SQLite's integrity check, and have a schema version this Kora
/// understands (1 up to [`super::schema::latest`])? A newer one is refused with the same guidance as opening it.
pub fn check_backup(backup: &Path) -> Result<BackupFacts, Problem> {
    let meta = std::fs::symlink_metadata(backup).map_err(|e| restore_problem("the file cannot be read", format!("{}: {e}", backup.display())))?;
    if !meta.is_file() {
        return Err(restore_problem("it is not a plain file", backup.display()));
    }
    let c = rusqlite::Connection::open_with_flags(backup, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW)
        .map_err(|e| restore_problem("it is not a Kora data file", e))?;
    let ok: String = c.query_row("PRAGMA integrity_check", [], |r| r.get(0)).map_err(|e| restore_problem("it is not a Kora data file, or is damaged", e))?;
    if ok != "ok" {
        return Err(restore_problem("it is damaged", ok));
    }
    let v = super::current_version(&c).map_err(|p| restore_problem("it is not a Kora data file", p.message))?;
    if v == 0 {
        return Err(restore_problem("it is not a Kora data file (no version)", backup.display()));
    }
    if v > super::schema::latest() {
        return Err(Problem::new(codes::STORE_NEWER, "That backup was saved by a newer version of Kora than this one, so this Kora will not use it. Update Kora first.")
            .with_details(format!("backup version {v}, this Kora understands up to {}", super::schema::latest()))
            .with_action(Action::CopyDetails));
    }
    Ok(BackupFacts { schema_version: v })
}

/// Put `backup` in place of the store in `home`. Refuses (changing nothing) when `panel_running`, when a job of a
/// live process is still running, when the backup does not check out ([`check_backup`]) or is the store itself.
/// First makes a backup of the current store (`before-restore`), returned. The restored file is opened once so an older
/// one is brought up to date (with its own backup first). Run with no other Kora program using the store.
pub fn restore(home: &Path, backup: &Path, panel_running: bool) -> Result<Option<PathBuf>, Problem> {
    if panel_running {
        return Err(Problem::new(
            codes::STORE_BUSY,
            "Kora Panel is running, and it is using the data file. Stop it first (close its window, or `kora panel restart` does not help: stop the background service), then restore again.",
        )
        .with_action(Action::Retry));
    }
    check_backup(backup)?;
    let db = super::db_path(home);
    let same = std::fs::canonicalize(backup).ok().zip(std::fs::canonicalize(&db).ok()).is_some_and(|(a, b)| a == b);
    if same {
        return Err(restore_problem("that is the current data file itself", backup.display()));
    }
    let mut before = None;
    if db.exists() {
        let s = Store::open_raw(home)?;
        // Jobs of live processes mean someone is using it.
        let busy: i64 = s.read(|c| {
            let n = c.query_row("SELECT COUNT(*) FROM jobs WHERE state = 'running'", [], |r| r.get::<_, i64>(0)).unwrap_or(0);
            Ok(n)
        })?;
        if busy > 0 {
            let alive: i64 = s.read(|c| {
                let mut st = c.prepare("SELECT owner_pid FROM jobs WHERE state = 'running'")?;
                let pids = st.query_map([], |r| r.get::<_, i64>(0))?.filter_map(Result::ok).filter(|p| super::jobs::pid_alive(*p as u32) && *p as u32 != std::process::id()).count();
                Ok(pids as i64)
            })?;
            if alive > 0 {
                return Err(Problem::new(codes::STORE_BUSY, "Another Kora program is running a job right now, so the data file is in use. Wait for it to finish (or stop it), then restore again.")
                    .with_action(Action::Retry));
            }
        }
        before = Some(automatic(&s, "before-restore")?);
        drop(s);
    }
    super::privacy::ensure_private_dir(home, &super::store_dir(home))?;
    // Copy into a private temporary file next to the store, then swap it in (a crash leaves either file whole).
    let tmp = db.with_extension("restoring");
    let _ = std::fs::remove_file(&tmp);
    privacy::ensure_private_file(&tmp)?;
    let src =
        rusqlite::Connection::open_with_flags(backup, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW).map_err(|e| restore_problem("it cannot be read", e))?;
    let _ = std::fs::remove_file(&tmp);
    src.execute("VACUUM INTO ?1", [tmp.to_string_lossy().as_ref()]).map_err(|e| restore_problem("it cannot be copied", e))?;
    drop(src);
    privacy::tighten(&tmp);
    for suffix in privacy::SIDE_SUFFIXES {
        let _ = std::fs::remove_file(privacy::side_file(&db, suffix));
    }
    crate::fsutil::rename_retrying(&tmp, &db).map_err(|e| restore_problem("the data file could not be replaced", e))?;
    privacy::fix_owner(&db);
    // Opening migrates an older backup forward and imports old files that are lying about.
    Store::open(home)?;
    Ok(before)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home_with_key(name: &str) -> tempfile::TempDir {
        let h = tempfile::tempdir().unwrap();
        crate::keystore::KeyStore::at(h.path().join("secrets.json")).add(name, "cfxk_restore_test_key_0001").unwrap();
        h
    }

    #[test]
    fn a_backup_is_put_back_after_a_backup_of_what_is_there() {
        let h = home_with_key("Before");
        let s = Store::open(h.path()).unwrap();
        let b = backup_now(&s, None).unwrap();
        drop(s);
        let keys = crate::keystore::KeyStore::at(h.path().join("secrets.json"));
        keys.remove("Before").unwrap();
        keys.add("After", "cfxk_restore_test_key_0002").unwrap();
        let undo = restore(h.path(), &b, false).unwrap().expect("a backup of the current store is made first");
        assert_eq!(keys.list().unwrap().iter().map(|k| k.name.as_str()).collect::<Vec<_>>(), ["Before"]);
        // the store as it was before the restore is kept, and is itself restorable
        assert_eq!(check_backup(&undo).unwrap().schema_version, super::super::schema::latest());
        restore(h.path(), &undo, false).unwrap();
        assert_eq!(keys.list().unwrap().iter().map(|k| k.name.as_str()).collect::<Vec<_>>(), ["After"]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(super::super::db_path(h.path())).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn it_refuses_while_a_panel_runs_and_changes_nothing() {
        let h = home_with_key("Keep");
        let s = Store::open(h.path()).unwrap();
        let b = backup_now(&s, None).unwrap();
        drop(s);
        let e = restore(h.path(), &b, true).unwrap_err();
        assert_eq!(e.code, codes::STORE_BUSY);
        assert!(list(h.path()).len() == 1, "no extra backup either");
    }

    #[test]
    fn a_backup_that_is_not_one_or_is_from_a_newer_kora_is_refused_and_nothing_changes() {
        let h = home_with_key("Keep");
        let junk = h.path().join("junk.db");
        std::fs::write(&junk, b"this is not a database at all, but long enough to look like a header block........").unwrap();
        assert!(restore(h.path(), &junk, false).is_err());
        assert!(restore(h.path(), &h.path().join("missing.db"), false).is_err());
        // an ordinary SQLite file with no Kora tables
        let other = h.path().join("other.db");
        rusqlite::Connection::open(&other).unwrap().execute_batch("CREATE TABLE t(x)").unwrap();
        assert!(restore(h.path(), &other, false).unwrap_err().message.contains("not a Kora data file"));
        // a newer version
        let newer = h.path().join("newer.db");
        {
            let s = Store::open(h.path()).unwrap();
            backup_to(&s, &newer).unwrap();
        }
        rusqlite::Connection::open(&newer).unwrap().execute("INSERT INTO schema_migrations (version, name, applied_at, kora_version) VALUES (999, 'future', 0, '9')", []).unwrap();
        assert_eq!(restore(h.path(), &newer, false).unwrap_err().code, codes::STORE_NEWER);
        // the current store itself
        assert!(restore(h.path(), &super::super::db_path(h.path()), false).is_err());
        assert_eq!(crate::keystore::KeyStore::at(h.path().join("secrets.json")).list().unwrap().len(), 1, "untouched");
        assert!(list(h.path()).is_empty(), "no backup was made for refused restores");
    }

    #[test]
    fn an_older_backup_is_brought_up_to_date_after_restoring() {
        let h = tempfile::tempdir().unwrap();
        // a version-1 store with a profile row
        let old = h.path().join("old.db");
        {
            let c = rusqlite::Connection::open(&old).unwrap();
            c.execute_batch("CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY NOT NULL, name TEXT NOT NULL, applied_at INTEGER NOT NULL, kora_version TEXT NOT NULL)").unwrap();
            c.execute_batch(super::super::schema::MIGRATIONS[0].sql).unwrap();
            c.execute("INSERT INTO schema_migrations VALUES (1, 'x', 0, '0.8')", []).unwrap();
            c.execute("INSERT INTO profile VALUES ('github_owner', 'sam')", []).unwrap();
        }
        assert_eq!(check_backup(&old).unwrap().schema_version, 1);
        assert_eq!(restore(h.path(), &old, false).unwrap(), None, "nothing was there to back up");
        let s = Store::open(h.path()).unwrap();
        assert_eq!(s.schema_version().unwrap(), super::super::schema::latest());
        assert_eq!(crate::profile::ProfileStore::at(h.path().join("profile.json")).load().github_owner.as_deref(), Some("sam"));
    }
}
