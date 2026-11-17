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
