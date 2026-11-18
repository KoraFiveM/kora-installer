//! Looking into a person's store from the PRIVILEGED side (the administrator service's migration, running as
//! root or SYSTEM at package or MSI install time) without touching it.
//!
//! Opening `kora.db` the ordinary way ([`super::Store`]) would create `-wal` / `-shm` files, fix permissions and may
//! migrate or import: files root made in the person's store folder would lock them out of their own data. So this
//! module opens the database read-only AND immutable (SQLite then takes no lock and writes nothing, not even a side
//! file), refuses a link as the file or as its folder, and reads one thing: the registered server folders. Because
//! an immutable open ignores a write-ahead log, a row the panel wrote moments ago may not be seen yet; the caller only
//! uses the answer to find folders to check from the system, never to trust.
//!
//! A machine that has not run the new Kora yet still has the old `servers.json`: that is accepted too.

use std::path::{Path, PathBuf};

/// `(name, folder)` of every server on the person's list, from the store or the old `servers.json`. Never writes,
/// never creates, never follows a link; anything unreadable gives an empty list.
pub fn registered_server_folders(home: &Path) -> Vec<(String, PathBuf)> {
    let db = super::db_path(home);
    if let Some(v) = from_database(&db) {
        return v;
    }
    from_legacy_file(&home.join("servers.json"))
}

fn is_plain_file(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok_and(|m| m.is_file())
}

fn is_plain_dir(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir())
}

/// `file:` URI for SQLite with the characters that mean something in a URI escaped.
fn uri_of(db: &Path) -> String {
    let mut out = String::from("file:");
    for b in db.to_string_lossy().bytes() {
        match b {
            b'%' | b'?' | b'#' | b' ' => out.push_str(&format!("%{b:02X}")),
            _ => out.push(b as char),
        }
    }
    // (non-ASCII bytes are passed through as they are: SQLite reads the URI path as UTF-8)
    out.push_str("?mode=ro&immutable=1");
    out
}

fn from_database(db: &Path) -> Option<Vec<(String, PathBuf)>> {
    let dir = db.parent()?;
    if !is_plain_dir(dir) || !is_plain_file(db) {
        return None;
    }
    use rusqlite::{Connection, OpenFlags};
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW | OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let c = Connection::open_with_flags(uri_of(db), flags).ok()?;
    let mut st = c.prepare("SELECT name, dir FROM servers ORDER BY seq").ok()?;
    let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).ok()?;
    Some(rows.filter_map(Result::ok).map(|(n, d)| (n, PathBuf::from(d))).collect())
}

fn from_legacy_file(file: &Path) -> Vec<(String, PathBuf)> {
    #[derive(serde::Deserialize)]
    struct Entry {
        name: String,
        dir: PathBuf,
    }
    #[derive(serde::Deserialize, Default)]
    struct File {
        #[serde(default)]
        servers: Vec<Entry>,
    }
    if !is_plain_file(file) {
        return vec![];
    }
    let Ok(bytes) = std::fs::read(file) else { return vec![] };
    serde_json::from_slice::<File>(&bytes).map(|f| f.servers.into_iter().map(|e| (e.name, e.dir)).collect()).unwrap_or_default()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    /// Everything about a folder's files that a write would change: names, sizes, owners, modes, times.
    fn snapshot(dir: &Path) -> Vec<(String, u64, u32, u32, u32, i64)> {
        let mut v: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                let m = e.metadata().unwrap();
                (e.file_name().to_string_lossy().into_owned(), m.len(), m.uid(), m.gid(), m.mode(), m.mtime_nsec() + m.mtime() * 1_000_000_000)
            })
            .collect();
        v.sort();
        v
    }

    fn home_with_servers() -> tempfile::TempDir {
        let h = tempfile::tempdir().unwrap();
        let s = super::super::Store::open(h.path()).unwrap();
        s.write(|tx| {
            tx.execute("INSERT INTO servers (name, seq, dir, record) VALUES ('Main', 1, '/home/jo/kora/main', '{}'), ('Dev', 2, '/home/jo/kora/dev one', '{}')", [])?;
            Ok(())
        })
        .unwrap();
        drop(s);
        h
    }

    #[test]
    fn the_list_is_read_and_nothing_in_the_store_folder_changes() {
        let h = home_with_servers();
        let dir = super::super::store_dir(h.path());
        let before = snapshot(&dir);
        let got = registered_server_folders(h.path());
        assert_eq!(got, [("Main".to_string(), PathBuf::from("/home/jo/kora/main")), ("Dev".to_string(), PathBuf::from("/home/jo/kora/dev one"))]);
        for _ in 0..3 {
            registered_server_folders(h.path());
        }
        assert_eq!(snapshot(&dir), before, "no file created (-wal, -shm), removed, resized, re-owned or touched");
        assert_eq!(snapshot(h.path()).len(), std::fs::read_dir(h.path()).unwrap().count());
    }

    #[test]
    fn an_old_servers_json_is_still_accepted_and_is_not_touched() {
        let h = tempfile::tempdir().unwrap();
        std::fs::write(h.path().join("servers.json"), r#"{"servers":[{"name":"Old","dir":"/srv/old","mode":"production","ports":{"game":30120,"txadmin":40120},"created_unix":1}]}"#).unwrap();
        let before = snapshot(h.path());
        assert_eq!(registered_server_folders(h.path()), [("Old".to_string(), PathBuf::from("/srv/old"))]);
        assert_eq!(snapshot(h.path()), before);
    }

    #[test]
    fn nothing_there_a_link_or_junk_gives_an_empty_list_and_creates_nothing() {
        let h = tempfile::tempdir().unwrap();
        assert!(registered_server_folders(h.path()).is_empty());
        assert!(!super::super::store_dir(h.path()).exists(), "never creates the store");
        let real = home_with_servers();
        let linked = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(super::super::store_dir(real.path()), super::super::store_dir(linked.path())).unwrap();
        assert!(registered_server_folders(linked.path()).is_empty(), "a linked store folder is refused");
        let junk = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(super::super::store_dir(junk.path())).unwrap();
        std::fs::write(super::super::db_path(junk.path()), "not a database").unwrap();
        assert!(registered_server_folders(junk.path()).is_empty());
        assert_eq!(snapshot(&super::super::store_dir(junk.path())).len(), 1);
    }
}
