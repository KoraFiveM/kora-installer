//! Small filesystem helpers for Windows realities: antivirus and the search indexer briefly hold
//! files and folders that were just written, so a `rename` straight after extracting or cloning
//! can fail with "Access is denied" / "being used by another process" and succeed a moment later.

use std::io;
use std::path::Path;
use std::time::Duration;

/// Retry policy: `attempts` tries with `pause` between them.
pub fn rename_retrying_with(from: &Path, to: &Path, attempts: u32, pause: Duration, sleep: &dyn Fn(Duration)) -> io::Result<()> {
    let mut last = None;
    for i in 0..attempts.max(1) {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            // A missing source or a real conflict will not fix itself.
            Err(e) if matches!(e.kind(), io::ErrorKind::NotFound | io::ErrorKind::AlreadyExists) => return Err(e),
            Err(e) => {
                last = Some(e);
                if i + 1 < attempts {
                    sleep(pause);
                }
            }
        }
    }
    Err(last.unwrap_or_else(|| io::Error::other("rename failed")))
}

/// `rename`, retried for about 6 s on Windows (once elsewhere: a failure there is real).
pub fn rename_retrying(from: &Path, to: &Path) -> io::Result<()> {
    if cfg!(windows) {
        rename_retrying_with(from, to, 15, Duration::from_millis(400), &std::thread::sleep)
    } else {
        std::fs::rename(from, to)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn missing_source_fails_at_once_without_waiting() {
        let t = tempfile::tempdir().unwrap();
        let slept = Cell::new(0);
        let e = rename_retrying_with(&t.path().join("nope"), &t.path().join("x"), 5, Duration::from_millis(1), &|_| slept.set(slept.get() + 1)).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::NotFound);
        assert_eq!(slept.get(), 0);
    }

    #[test]
    fn a_blocked_rename_is_retried_then_reported() {
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("a");
        std::fs::write(&a, "x").unwrap();
        // Target is a non-empty directory: renaming a file onto it fails with something other than NotFound.
        let d = t.path().join("d");
        std::fs::create_dir(&d).unwrap();
        std::fs::write(d.join("f"), "x").unwrap();
        let slept = Cell::new(0);
        let r = rename_retrying_with(&a, &d, 4, Duration::from_millis(1), &|_| slept.set(slept.get() + 1));
        assert!(r.is_err());
        assert_eq!(slept.get(), 3, "pauses between the 4 attempts");
    }

    #[test]
    fn plain_rename_works() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("a"), "x").unwrap();
        rename_retrying(&t.path().join("a"), &t.path().join("b")).unwrap();
        assert!(t.path().join("b").is_file());
    }
}
