//! Kora's own state in one embedded SQLite database: `<kora home>/store/kora.db`.
//!
//! This replaces the JSON files that were each rewritten whole (`servers.json`, `secrets.json`,
//! `profile.json`, `paused-installs/`, `last-install.json`, `panel.json`, `hub.json`), so the panel and a
//! `kora` command running at the same moment can no longer overwrite each other's change, and adds
//! durable jobs ([`jobs`]) and an audit log ([`audit`]).
//!
//! # What lives here, what stays a file
//! * In the database: everything Kora keeps ABOUT ITSELF in the Kora home (above).
//! * Still files: anything the game server or the person reads directly (`server.cfg`, `secrets.cfg`,
//!   txAdmin's files), logs, downloaded tools, caches that can be thrown away (`update-check.json`,
//!   `forks-cache.json`, `org-cache.json`), and everything that lives INSIDE a server folder
//!   (`kora-server.json`, `.kora/*`: plan state, schedule, alert settings, paid-asset records, the update
//!   lock). Those travel with the folder when it is moved or copied, are written by the privileged side
//!   as the server's own account, and `kora-server.json` is read by tools outside Kora. The file-based
//!   update lock stays a lock FILE on purpose (it must work between processes and survive a crash).
//!   `%ProgramFiles%\Kora\owners.json` belongs to the privileged side.
//!
//! # Concurrency
//! WAL mode with a busy timeout; every change runs in ONE short `BEGIN IMMEDIATE` transaction (one
//! writer at a time, and read-modify-write sequences are inside it, so two processes cannot lose each
//! other's update). A connection is opened per operation (cheap, and it never goes stale across a `fork`,
//! a deleted folder or a changed `KORA_HOME`).
//!
//! # Privacy and ownership
//! See [`privacy`]: the folder is private, the files are `0600` from creation (so are `-wal`/`-shm`),
//! links are refused, and files made as root inside someone else's folder are handed to its owner.
//!
//! # Where the keys' VALUES live
//! [`material::KeyMaterial`] is the one seam. Today [`material::SqliteKeyMaterial`] keeps them in the
//! `license_key_material` table; a later workstream can implement the trait over the system keychain.
//!
//! # Versions
//! `schema_migrations` holds one row per applied migration ([`schema`]). An older Kora that opens a
//! database made by a newer one refuses with a guided problem and changes nothing.
//!
//! The store API reports [`Problem`]s; the domain modules map them onto the error type each public
//! function already had (io error, string, Problem), so signatures elsewhere do not change.

pub mod audit;
pub mod backup;
pub mod export;
pub mod jobs;
pub mod legacy;
pub mod material;
pub mod privacy;
pub mod schema;

use std::path::{Path, PathBuf};
use std::time::Duration;

pub use rusqlite;
use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior};

use crate::problem::{Action, Problem};

pub mod codes {
    /// The database was written by a newer Kora.
    pub const STORE_NEWER: &str = "store.newer";
    /// The database file cannot be opened or is damaged.
    pub const STORE_UNREADABLE: &str = "store.unreadable";
    /// Another Kora process holds the database for too long.
    pub const STORE_BUSY: &str = "store.busy";
    /// An old JSON state file could not be imported (it is left exactly where it was).
    pub const STORE_IMPORT: &str = "store.import_failed";
    /// The data folder cannot be made private.
    pub const STORE_PRIVACY: &str = "store.not_private";
}

/// The folder inside the Kora home that holds the database, its backups and the old JSON files.
pub const DIR_NAME: &str = "store";
pub const DB_NAME: &str = "kora.db";
pub const BUSY_TIMEOUT: Duration = Duration::from_secs(10);

pub fn store_dir(home: &Path) -> PathBuf {
    home.join(DIR_NAME)
}

pub fn db_path(home: &Path) -> PathBuf {
    store_dir(home).join(DB_NAME)
}

impl From<rusqlite::Error> for Problem {
    fn from(e: rusqlite::Error) -> Problem {
        use rusqlite::ErrorCode as C;
        match e.sqlite_error_code() {
            Some(C::DatabaseBusy | C::DatabaseLocked) => Problem::new(
                codes::STORE_BUSY,
                "Kora's data file is busy: another Kora window or command has been using it for too long. Wait a moment and try again; if it keeps happening, close other Kora windows.",
            )
            .with_details(e.to_string())
            .with_action(Action::Retry),
            Some(C::NotADatabase | C::DatabaseCorrupt) => Problem::new(
                codes::STORE_UNREADABLE,
                "Kora's data file is damaged. Nothing was changed. Kora keeps a copy before every upgrade in the store/backups folder inside the Kora folder; see docs/store.md for how to go back to one.",
            )
            .with_details(e.to_string())
            .with_action(Action::CopyDetails),
            _ => Problem::new(codes::STORE_UNREADABLE, "Kora could not use its data file. Nothing was changed.").with_details(e.to_string()).with_action(Action::CopyDetails),
        }
    }
}

/// An open store. Cheap to make; make one per operation.
pub struct Store {
    conn: Connection,
    home: PathBuf,
    blocked: Vec<Blocked>,
}

/// An old JSON file that could not be imported (damaged, unreadable or from a newer Kora). It is left
/// exactly where it was, and the part of Kora that depends on it says so instead of starting empty.
#[derive(Debug, Clone)]
pub struct Blocked {
    /// `servers`, `keys`, `remote` or `hub`.
    pub domain: &'static str,
    /// Plain words for "why" (no secret).
    pub why: String,
    /// Technical details (no secret).
    pub details: String,
    pub file: PathBuf,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store").field("home", &self.home).finish()
    }
}

/// One process at a time may create/migrate/import inside this process; across processes SQLite's
/// `BEGIN IMMEDIATE` does the same.
static INIT: std::sync::Mutex<()> = std::sync::Mutex::new(());

impl Store {
    /// The store in the Kora home of this process.
    pub fn open_home() -> Result<Store, Problem> {
        Store::open(&crate::installer::registry::kora_home())
    }

    /// The store of the Kora home that holds `file` (this is how `X::at(<home>/something.json)` keeps
    /// working: the file name says which home, the data is in the database).
    pub fn open_beside(file: &Path) -> Result<Store, Problem> {
        let home = file.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        Store::open(home)
    }

    /// Open (creating, migrating and importing old JSON files as needed) the store in `home`.
    pub fn open(home: &Path) -> Result<Store, Problem> {
        let mut s = Store::open_raw(home)?;
        let _g = INIT.lock().unwrap_or_else(|p| p.into_inner());
        s.migrate()?;
        legacy::import_if_needed(&mut s)?;
        Ok(s)
    }

    /// For READ-ONLY questions ("what servers are there?"): `None` when this home has no store yet and no
    /// old JSON files to import, so asking never creates anything (a fresh machine, or a test that never
    /// set `KORA_HOME`).
    pub fn open_existing(home: &Path) -> Result<Option<Store>, Problem> {
        if !db_path(home).exists() && !legacy::any_legacy_files(home) {
            return Ok(None);
        }
        Store::open(home).map(Some)
    }

    pub fn open_existing_beside(file: &Path) -> Result<Option<Store>, Problem> {
        Store::open_existing(file.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new(".")))
    }

    /// Open without migrating or importing (backups, `store status`, tests).
    pub fn open_raw(home: &Path) -> Result<Store, Problem> {
        let dir = store_dir(home);
        let db = db_path(home);
        privacy::ensure_private_dir(home, &dir)?;
        privacy::ensure_private_file(&db)?;
        for s in privacy::SIDE_SUFFIXES {
            privacy::refuse_link(&privacy::side_file(&db, s))?;
        }
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW;
        let conn = Connection::open_with_flags(&db, flags)?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        // Deleted rows (a forgotten key) are zeroed in the file; commits reach the disk (a saved key is not lost
        // to a power cut); WAL lets the panel read while a command writes.
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA secure_delete = ON; PRAGMA synchronous = FULL; PRAGMA trusted_schema = OFF;")?;
        // Switching a brand-new file to WAL needs a moment alone with it and, unlike every other statement,
        // does not wait for the busy timeout: when two Kora programs start together, try again.
        let started = std::time::Instant::now();
        let mode: String = loop {
            match conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get::<_, String>(0)) {
                Ok(m) => break m,
                Err(e) if matches!(e.sqlite_error_code(), Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)) && started.elapsed() < BUSY_TIMEOUT => {
                    std::thread::sleep(Duration::from_millis(15));
                }
                Err(e) => return Err(e.into()),
            }
        };
        if !mode.eq_ignore_ascii_case("wal") {
            // e.g. a network drive: still correct, just slower and with writers blocking readers.
            let _ = conn.execute_batch("PRAGMA journal_mode = DELETE");
        }
        let s = Store { conn, home: home.to_path_buf(), blocked: Vec::new() };
        privacy::fix_owner(&db);
        Ok(s)
    }

    /// Is this part of the state waiting on an old file that could not be imported?
    pub fn blocked(&self, domain: &str) -> Option<&Blocked> {
        self.blocked.iter().find(|b| b.domain == domain)
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    pub fn path(&self) -> PathBuf {
        db_path(&self.home)
    }

    /// Read with the connection (no transaction needed: a single statement sees one snapshot).
    pub fn read<T>(&self, f: impl FnOnce(&Connection) -> Result<T, Problem>) -> Result<T, Problem> {
        f(&self.conn)
    }

    /// Run `f` in ONE `BEGIN IMMEDIATE` transaction: committed when it returns `Ok`, rolled back when it
    /// returns `Err` (or panics). Keep it short: it holds the one write lock.
    pub fn write<T>(&self, f: impl FnOnce(&Transaction) -> Result<T, Problem>) -> Result<T, Problem> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let out = f(&tx)?;
        tx.commit()?;
        privacy::fix_owner(&self.path());
        Ok(out)
    }

    pub fn schema_version(&self) -> Result<u32, Problem> {
        self.read(current_version)
    }

    /// Apply the migrations that are missing, each with a store backup before the first one when the
    /// database already holds data. An older Kora meeting a newer database refuses here.
    fn migrate(&mut self) -> Result<(), Problem> {
        let have = current_version(&self.conn)?;
        let latest = schema::latest();
        if have == latest {
            return Ok(());
        }
        if have > latest {
            return Err(newer_problem(have, latest, &self.path()));
        }
        if have > 0 {
            // An upgrade of real data: a consistent copy first (SQLite's own copy, safe while others use the file).
            backup::backup_before_migration(self, have, latest)?;
        }
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        // Another process may have done it while we waited for the lock.
        let have = current_version(&tx)?;
        if have > latest {
            return Err(newer_problem(have, latest, &self.path()));
        }
        tx.execute_batch("CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY NOT NULL, name TEXT NOT NULL, applied_at INTEGER NOT NULL, kora_version TEXT NOT NULL)")?;
        for m in schema::MIGRATIONS.iter().filter(|m| m.version > have) {
            tx.execute_batch(m.sql).map_err(|e| {
                Problem::new(codes::STORE_UNREADABLE, format!("Kora could not upgrade its data file (step {}). Nothing was changed.", m.version))
                    .with_details(e.to_string())
                    .with_action(Action::CopyDetails)
            })?;
            tx.execute(
                "INSERT INTO schema_migrations (version, name, applied_at, kora_version) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![m.version, m.name, crate::monitor::now_unix() as i64, env!("CARGO_PKG_VERSION")],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        // The last connection to close removes the side files; one left behind by a crash is still ours to hand over.
        privacy::fix_owner(&db_path(&self.home));
    }
}

/// The newest applied migration (0 for a brand-new file).
pub fn current_version(c: &Connection) -> Result<u32, Problem> {
    let has: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations')", [], |r| r.get(0))?;
    if !has {
        return Ok(0);
    }
    Ok(c.query_row("SELECT COALESCE(MAX(version), 0) FROM schema_migrations", [], |r| r.get::<_, i64>(0))? as u32)
}

fn newer_problem(have: u32, latest: u32, path: &Path) -> Problem {
    Problem::new(
        codes::STORE_NEWER,
        "Kora's data was saved by a newer version of Kora than this one, so this older Kora will not touch it (it could damage it). Update Kora to the newer version. To go back to this older version on purpose, see docs/store.md.",
    )
    .with_details(format!("{} is at version {have}; this Kora understands up to {latest}", path.display()))
    .with_action(Action::CopyDetails)
}

/// `meta` helpers (small values: markers, counters).
pub fn meta_get(c: &Connection, key: &str) -> Result<Option<String>, Problem> {
    match c.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get::<_, String>(0)) {
        Ok(v) => Ok(Some(v)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn meta_set(c: &Connection, key: &str, value: &str) -> Result<(), Problem> {
    c.execute("INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value", [key, value])?;
    Ok(())
}

pub fn meta_delete(c: &Connection, key: &str) -> Result<(), Problem> {
    c.execute("DELETE FROM meta WHERE key = ?1", [key])?;
    Ok(())
}

/// What the domain modules with an `io::Error` signature return for a store problem.
pub fn problem_to_io(p: Problem) -> std::io::Error {
    match &p.details {
        Some(d) if !d.is_empty() => std::io::Error::other(format!("{} ({d})", p.message)),
        _ => std::io::Error::other(p.message),
    }
}

/// What the domain modules with a `String` signature return for a store problem.
pub fn problem_to_string(p: Problem) -> String {
    match &p.details {
        Some(d) if !d.is_empty() => format!("{} ({d})", p.message),
        _ => p.message,
    }
}

#[cfg(test)]
mod tests;
