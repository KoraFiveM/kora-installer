//! One update at a time per server folder, across processes: `<server>/.kora/update.lock`.
//!
//! Every entry point that changes resources or runs git in them (manual preview and apply,
//! rollback, `kora update --auto` from the schedule or the panel, the CLI) holds this lock for
//! the whole run. It is an OS file lock (`flock` on Linux, `LockFileEx` on Windows, through
//! `std::fs::File::try_lock`): it goes away by itself when the process ends, even after a crash,
//! so there is never a stale lock to clean up. The file itself is left in place on purpose
//! (deleting it would let two processes lock two different files).

use std::fs::{File, TryLockError};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// What a person sees when the lock is taken.
pub const BUSY: &str = "Another update is running for this server";

#[derive(Debug)]
pub enum LockError {
    /// Another process (or another run in this one) holds the lock.
    Busy,
    /// The lock file could not be created or opened.
    Io(std::io::Error),
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockError::Busy => write!(f, "{BUSY}. Try again when it has finished."),
            LockError::Io(e) => write!(f, "could not take the update lock in .kora: {e}"),
        }
    }
}

impl std::error::Error for LockError {}

/// Is this text the busy message (e.g. an error that went through a `String`)?
pub fn is_busy_message(s: &str) -> bool {
    s.contains(BUSY)
}

/// Held while an update runs; dropping it releases the lock.
#[derive(Debug)]
pub struct UpdateLock {
    _file: File,
}

pub fn path(server_dir: &Path) -> PathBuf {
    server_dir.join(".kora").join("update.lock")
}

/// Root (the update timer) takes this lock in a folder the service account can change, so the file
/// is never opened or created through a planted link (root would otherwise create, say,
/// `/etc/nologin`); see `fsutil::open_no_follow`.
fn open(p: &Path, create: bool) -> std::io::Result<File> {
    use crate::fsutil::{open_no_follow, Access};
    match open_no_follow(p, Access::ReadWrite, create.then_some(0o644)) {
        Ok(f) => Ok(f),
        // Created earlier by another account (the root timer): a read handle can still be locked.
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied && !crate::fsutil::is_symlink(p) => open_no_follow(p, Access::Read, None),
        Err(e) => Err(e),
    }
}

/// How long a lock that looks taken is retried before it counts as busy. A child process another
/// thread is starting (any `git` call) briefly holds a copy of every open file, the lock included,
/// so a lock released a moment ago can still look taken for a few milliseconds.
const GRACE: Duration = Duration::from_millis(1500);
const STEP: Duration = Duration::from_millis(50);

/// `try_lock` until it works or [`GRACE`] has passed.
fn lock_with_grace(file: &File) -> Result<(), LockError> {
    let deadline = Instant::now() + GRACE;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(()),
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => std::thread::sleep(STEP),
            Err(TryLockError::WouldBlock) => return Err(LockError::Busy),
            Err(TryLockError::Error(e)) => return Err(LockError::Io(e)),
        }
    }
}

/// Take the lock (no real waiting: only the short grace above).
pub fn acquire(server_dir: &Path) -> Result<UpdateLock, LockError> {
    let file = open(&path(server_dir), true).map_err(LockError::Io)?;
    lock_with_grace(&file)?;
    Ok(UpdateLock { _file: file })
}

/// Is an update running right now? Read-only: creates nothing (a missing lock file means no).
pub fn is_busy(server_dir: &Path) -> bool {
    let Ok(file) = open(&path(server_dir), false) else {
        return false;
    };
    match lock_with_grace(&file) {
        Ok(()) => {
            let _ = file.unlock();
            false
        }
        Err(LockError::Busy) => true,
        Err(LockError::Io(_)) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_holder_is_refused_until_the_first_lets_go() {
        let t = tempfile::tempdir().unwrap();
        assert!(!is_busy(t.path()), "no lock file yet");
        assert!(!path(t.path()).exists(), "is_busy creates nothing");
        let held = acquire(t.path()).unwrap();
        assert!(is_busy(t.path()));
        let e = acquire(t.path()).unwrap_err();
        assert!(matches!(e, LockError::Busy));
        assert!(is_busy_message(&e.to_string()), "{e}");
        drop(held);
        assert!(!is_busy(t.path()));
        let _again = acquire(t.path()).unwrap();
    }

    /// Root takes this lock in a folder the service account owns: a planted link is not followed
    /// (it would make root create, say, `/etc/nologin`).
    #[cfg(unix)]
    #[test]
    fn a_planted_link_is_never_followed() {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(t.path().join(".kora")).unwrap();
        let victim = t.path().join("nologin");
        std::os::unix::fs::symlink(&victim, path(t.path())).unwrap();
        assert!(matches!(acquire(t.path()), Err(LockError::Io(_))));
        assert!(!victim.exists());
        assert!(!is_busy(t.path()));
    }
}
