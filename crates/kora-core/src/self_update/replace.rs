//! Atomic replacement of the program file, as a flow over a small filesystem trait so the exact
//! sequence (and its rollback) is testable with a fake.
//!
//! Unix: the running file may be replaced by renaming over it. A hard link keeps the old file as
//! `<name>.old`; `rename(new, target)` is atomic, so `target` is always either old or new.
//! Windows: a running exe cannot be overwritten but CAN be renamed: `target -> target.old`, then
//! `new -> target`; if that fails the old file is renamed back. `.old` is deleted at the next start.

use std::io;
use std::path::{Path, PathBuf};

pub trait Fs {
    fn exists(&self, p: &Path) -> bool;
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    fn remove_file(&self, p: &Path) -> io::Result<()>;
    /// Keep the content of `from` at `to` (hard link, falling back to a copy).
    fn keep_copy(&self, from: &Path, to: &Path) -> io::Result<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Unix,
    Windows,
}

impl Flow {
    pub fn native() -> Flow {
        if cfg!(windows) {
            Flow::Windows
        } else {
            Flow::Unix
        }
    }
}

pub struct RealFs;

impl Fs for RealFs {
    fn exists(&self, p: &Path) -> bool {
        std::fs::symlink_metadata(p).is_ok()
    }
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        crate::fsutil::rename_retrying(from, to)
    }
    fn remove_file(&self, p: &Path) -> io::Result<()> {
        std::fs::remove_file(p)
    }
    fn keep_copy(&self, from: &Path, to: &Path) -> io::Result<()> {
        std::fs::hard_link(from, to).or_else(|_| std::fs::copy(from, to).map(|_| ()))
    }
}

/// `<target>.old` (appended to the whole file name: `kora.exe.old`).
pub fn backup_path(target: &Path) -> PathBuf {
    let mut s = target.as_os_str().to_os_string();
    s.push(".old");
    PathBuf::from(s)
}

#[derive(Debug)]
pub struct ReplaceError {
    pub message: String,
    /// The target is back to (or never left) its previous content.
    pub target_intact: bool,
}

impl std::fmt::Display for ReplaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Put `new` (already verified, same directory) in place of `target`. Returns the backup path
/// when an old file existed. On error `new` is left for the caller to delete.
pub fn replace(fs: &dyn Fs, flow: Flow, target: &Path, new: &Path) -> Result<Option<PathBuf>, ReplaceError> {
    let old = backup_path(target);
    let had = fs.exists(target);
    let err = |m: String, intact: bool| ReplaceError { message: m, target_intact: intact };
    if fs.exists(&old) {
        fs.remove_file(&old).map_err(|e| err(format!("cannot remove the previous backup {}: {e}", old.display()), true))?;
    }
    match flow {
        Flow::Unix => {
            if had {
                fs.keep_copy(target, &old).map_err(|e| err(format!("cannot keep a backup at {}: {e}", old.display()), true))?;
            }
            if let Err(e) = fs.rename(new, target) {
                // the target was never touched; drop the backup we just made
                if had {
                    let _ = fs.remove_file(&old);
                }
                return Err(err(format!("cannot replace {}: {e}", target.display()), true));
            }
        }
        Flow::Windows => {
            if had {
                fs.rename(target, &old).map_err(|e| err(format!("cannot move the current program aside: {e}"), true))?;
            }
            if let Err(e) = fs.rename(new, target) {
                if had {
                    return match fs.rename(&old, target) {
                        Ok(()) => Err(err(format!("cannot put the new program in place (the old one was restored): {e}"), true)),
                        Err(e2) => Err(err(format!("cannot put the new program in place ({e}) and cannot restore the old one ({e2}); it is at {}", old.display()), false)),
                    };
                }
                return Err(err(format!("cannot put the new program in place: {e}"), true));
            }
        }
    }
    Ok(had.then_some(old))
}

/// Delete a leftover `<exe>.old` (Windows only keeps one around). Best effort; a locked file
/// (a still-running old copy) is simply left for next time.
pub fn cleanup_old(fs: &dyn Fs, windows: bool, exe: &Path) -> bool {
    if !windows {
        return false;
    }
    let old = backup_path(exe);
    fs.exists(&old) && fs.remove_file(&old).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeSet;

    /// Files are names only; records every operation; one operation can be made to fail.
    #[derive(Default)]
    struct Fake {
        files: RefCell<BTreeSet<String>>,
        log: RefCell<Vec<String>>,
        fail: Vec<String>,
    }
    impl Fake {
        fn with(files: &[&str], fail: &[&str]) -> Fake {
            Fake { files: RefCell::new(files.iter().map(|s| s.to_string()).collect()), log: Default::default(), fail: fail.iter().map(|s| s.to_string()).collect() }
        }
        fn op(&self, s: String) -> io::Result<()> {
            self.log.borrow_mut().push(s.clone());
            if self.fail.contains(&s) {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"));
            }
            Ok(())
        }
        fn has(&self, f: &str) -> bool {
            self.files.borrow().contains(f)
        }
    }
    fn n(p: &Path) -> String {
        p.to_string_lossy().into_owned()
    }
    impl Fs for Fake {
        fn exists(&self, p: &Path) -> bool {
            self.has(&n(p))
        }
        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            self.op(format!("rename {} {}", n(from), n(to)))?;
            let mut f = self.files.borrow_mut();
            f.remove(&n(from));
            f.insert(n(to));
            Ok(())
        }
        fn remove_file(&self, p: &Path) -> io::Result<()> {
            self.op(format!("remove {}", n(p)))?;
            self.files.borrow_mut().remove(&n(p));
            Ok(())
        }
        fn keep_copy(&self, from: &Path, to: &Path) -> io::Result<()> {
            self.op(format!("keep {} {}", n(from), n(to)))?;
            self.files.borrow_mut().insert(n(to));
            Ok(())
        }
    }
    fn p(s: &str) -> &Path {
        Path::new(s)
    }

    #[test]
    fn windows_flow_renames_aside_then_in() {
        let fs = Fake::with(&["kora.exe", "new.tmp"], &[]);
        let b = replace(&fs, Flow::Windows, p("kora.exe"), p("new.tmp")).unwrap();
        assert_eq!(b, Some(PathBuf::from("kora.exe.old")));
        assert_eq!(*fs.log.borrow(), ["rename kora.exe kora.exe.old", "rename new.tmp kora.exe"]);
        assert!(fs.has("kora.exe") && fs.has("kora.exe.old") && !fs.has("new.tmp"));
    }

    #[test]
    fn windows_flow_removes_stale_old_first() {
        let fs = Fake::with(&["kora.exe", "kora.exe.old", "new.tmp"], &[]);
        replace(&fs, Flow::Windows, p("kora.exe"), p("new.tmp")).unwrap();
        assert_eq!(fs.log.borrow()[0], "remove kora.exe.old");
    }

    #[test]
    fn windows_flow_rolls_back_when_the_move_in_fails() {
        let fs = Fake::with(&["kora.exe", "new.tmp"], &["rename new.tmp kora.exe"]);
        let e = replace(&fs, Flow::Windows, p("kora.exe"), p("new.tmp")).unwrap_err();
        assert!(e.target_intact, "{e}");
        assert!(fs.has("kora.exe") && !fs.has("kora.exe.old") && fs.has("new.tmp"));
        assert_eq!(fs.log.borrow().last().unwrap(), "rename kora.exe.old kora.exe");
    }

    #[test]
    fn windows_flow_reports_a_failed_rollback() {
        let fs = Fake::with(&["kora.exe", "new.tmp"], &["rename new.tmp kora.exe", "rename kora.exe.old kora.exe"]);
        let e = replace(&fs, Flow::Windows, p("kora.exe"), p("new.tmp")).unwrap_err();
        assert!(!e.target_intact);
        assert!(e.message.contains("kora.exe.old"), "{e}");
    }

    #[test]
    fn windows_flow_cannot_move_aside_leaves_everything() {
        let fs = Fake::with(&["kora.exe", "new.tmp"], &["rename kora.exe kora.exe.old"]);
        let e = replace(&fs, Flow::Windows, p("kora.exe"), p("new.tmp")).unwrap_err();
        assert!(e.target_intact);
        assert!(fs.has("kora.exe") && fs.has("new.tmp"));
    }

    #[test]
    fn unix_flow_keeps_backup_and_renames_over() {
        let fs = Fake::with(&["kora", "new.tmp"], &[]);
        let b = replace(&fs, Flow::Unix, p("kora"), p("new.tmp")).unwrap();
        assert_eq!(b, Some(PathBuf::from("kora.old")));
        assert_eq!(*fs.log.borrow(), ["keep kora kora.old", "rename new.tmp kora"]);
    }

    #[test]
    fn unix_flow_failure_leaves_target_and_drops_backup() {
        let fs = Fake::with(&["kora", "new.tmp"], &["rename new.tmp kora"]);
        let e = replace(&fs, Flow::Unix, p("kora"), p("new.tmp")).unwrap_err();
        assert!(e.target_intact);
        assert!(fs.has("kora") && !fs.has("kora.old"));
    }

    #[test]
    fn first_install_has_no_backup() {
        let fs = Fake::with(&["new.tmp"], &[]);
        assert_eq!(replace(&fs, Flow::Windows, p("kora.exe"), p("new.tmp")).unwrap(), None);
        assert!(fs.has("kora.exe"));
    }

    #[test]
    fn cleanup_is_windows_only() {
        let fs = Fake::with(&["kora.exe", "kora.exe.old"], &[]);
        assert!(!cleanup_old(&fs, false, p("kora.exe")));
        assert!(fs.has("kora.exe.old"));
        assert!(cleanup_old(&fs, true, p("kora.exe")));
        assert!(!fs.has("kora.exe.old") && fs.has("kora.exe"));
        assert!(!cleanup_old(&fs, true, p("kora.exe")));
    }
}
