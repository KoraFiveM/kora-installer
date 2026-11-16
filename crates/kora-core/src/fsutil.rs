//! Small filesystem helpers.
//!
//! * Windows realities: antivirus and the search indexer briefly hold files and folders that were
//!   just written, so a `rename` straight after extracting or cloning can fail with "Access is
//!   denied" / "being used by another process" and succeed a moment later.
//! * Root working inside a server folder: that folder belongs to the server's service account (which
//!   runs third-party resource code) or to the panel's user, so anything in it may be a planted
//!   symbolic link. [`write_no_follow`] and [`remove_inside`] never follow one.

use std::io;
use std::path::{Component, Path, PathBuf};
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

fn refused(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, msg)
}

/// A path taken from one of Kora's records that must name something INSIDE a folder: relative,
/// not empty, no `..`, no root or drive prefix. (The records live in folders other accounts can
/// write to, so `/etc` or `../../root` in them must never reach a delete.)
pub fn check_relative(rel: &Path) -> io::Result<()> {
    let mut any = false;
    for c in rel.components() {
        match c {
            Component::Normal(_) => any = true,
            Component::CurDir => {}
            _ => return Err(refused(format!("'{}' is not a plain path inside the server folder, so Kora will not touch it", rel.display()))),
        }
    }
    if any {
        Ok(())
    } else {
        Err(refused("an empty path in Kora's records was ignored".into()))
    }
}

fn normal_parts(rel: &Path) -> Vec<&std::ffi::OsStr> {
    rel.components()
        .filter_map(|c| match c {
            Component::Normal(n) => Some(n),
            _ => None,
        })
        .collect()
}

static NONCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn temp_name(name: &std::ffi::OsStr) -> String {
    let n = NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    format!(".{}.kora-tmp-{}-{n}-{t}", name.to_string_lossy(), std::process::id())
}

fn split_parent(path: &Path) -> io::Result<(&Path, &std::ffi::OsStr)> {
    match (path.parent(), path.file_name()) {
        (Some(p), Some(n)) => Ok((if p.as_os_str().is_empty() { Path::new(".") } else { p }, n)),
        _ => Err(io::Error::new(io::ErrorKind::InvalidInput, format!("{} is not a file path", path.display()))),
    }
}

fn symlink_parent_error(dir: &Path) -> io::Error {
    refused(format!("{} is a symbolic link (or not a folder), so Kora will not write into it", dir.display()))
}

#[cfg(unix)]
mod nofollow {
    use super::*;
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;

    pub fn cstr(b: &[u8]) -> io::Result<CString> {
        CString::new(b).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains a NUL byte"))
    }

    fn is_link_error(e: &io::Error) -> bool {
        matches!(e.raw_os_error(), Some(libc::ELOOP) | Some(libc::ENOTDIR))
    }

    /// Open a folder; with `follow = false` a symbolic link as the LAST component is refused.
    pub fn open_dir(path: &Path, follow: bool) -> io::Result<OwnedFd> {
        let c = cstr(path.as_os_str().as_bytes())?;
        let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | if follow { 0 } else { libc::O_NOFOLLOW };
        // SAFETY: valid NUL-terminated path; the fd is owned right away.
        let fd = unsafe { libc::open(c.as_ptr(), flags) };
        if fd < 0 {
            let e = io::Error::last_os_error();
            return Err(if is_link_error(&e) { symlink_parent_error(path) } else { e });
        }
        // SAFETY: `fd` is a fresh, valid descriptor nobody else owns.
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    /// One folder below `dir`, never through a symbolic link.
    pub fn open_child_dir(dir: &OwnedFd, name: &std::ffi::OsStr) -> io::Result<OwnedFd> {
        let c = cstr(name.as_bytes())?;
        // SAFETY: valid fd and NUL-terminated name.
        let fd = unsafe { libc::openat(dir.as_raw_fd(), c.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
        if fd < 0 {
            let e = io::Error::last_os_error();
            return Err(if is_link_error(&e) { refused(format!("'{}' is a symbolic link, so Kora will not go through it", name.to_string_lossy())) } else { e });
        }
        // SAFETY: fresh descriptor.
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    pub fn write(path: &Path, bytes: &[u8], mode: u32) -> io::Result<()> {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        let (parent, name) = split_parent(path)?;
        std::fs::create_dir_all(parent)?;
        let dir = open_dir(parent, false)?;
        let name_c = cstr(name.as_bytes())?;
        let mut tries = 0;
        loop {
            let tmp_c = cstr(temp_name(name).as_bytes())?;
            // O_EXCL + O_NOFOLLOW: a file or link someone planted under that name is never opened.
            // SAFETY: valid fd and NUL-terminated name.
            let fd = unsafe {
                libc::openat(dir.as_raw_fd(), tmp_c.as_ptr(), libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC, mode as libc::c_uint)
            };
            if fd < 0 {
                let e = io::Error::last_os_error();
                tries += 1;
                if e.kind() == io::ErrorKind::AlreadyExists && tries < 8 {
                    continue;
                }
                return Err(e);
            }
            // SAFETY: fresh descriptor, owned by the File from here on.
            let mut f = unsafe { std::fs::File::from_raw_fd(fd) };
            // fchmod on the open file (not the path): the umask must not loosen or tighten `mode`.
            let mut res = f.write_all(bytes).and_then(|_| f.set_permissions(std::fs::Permissions::from_mode(mode)));
            drop(f);
            if res.is_ok() {
                // SAFETY: valid fds and names. `renameat` replaces a link at `name` instead of following it.
                if unsafe { libc::renameat(dir.as_raw_fd(), tmp_c.as_ptr(), dir.as_raw_fd(), name_c.as_ptr()) } != 0 {
                    res = Err(io::Error::last_os_error());
                }
            }
            if res.is_err() {
                // SAFETY: valid fd and name.
                unsafe { libc::unlinkat(dir.as_raw_fd(), tmp_c.as_ptr(), 0) };
            }
            return res;
        }
    }

    /// Walk `rel`'s folders below `base` without following links; `None` when one is missing.
    fn walk(base: &Path, dirs: &[&std::ffi::OsStr]) -> io::Result<Option<OwnedFd>> {
        let mut fd = match open_dir(base, true) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        for d in dirs {
            fd = match open_child_dir(&fd, d) {
                Ok(f) => f,
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(e),
            };
        }
        Ok(Some(fd))
    }

    pub fn remove(base: &Path, rel: &Path) -> io::Result<bool> {
        let parts = normal_parts(rel);
        let Some((last, dirs)) = parts.split_last() else { return Ok(false) };
        let Some(dir) = walk(base, dirs)? else { return Ok(false) };
        let name = cstr(last.as_bytes())?;
        // SAFETY: zeroed stat is a valid out-buffer; valid fd and name.
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstatat(dir.as_raw_fd(), name.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) } != 0 {
            let e = io::Error::last_os_error();
            return if e.kind() == io::ErrorKind::NotFound { Ok(false) } else { Err(e) };
        }
        if st.st_mode & libc::S_IFMT == libc::S_IFDIR {
            // Through the open folder (`/proc/self/fd/N`), so swapping a folder above it for a link
            // after the walk changes nothing; std's remove_dir_all never follows links below that.
            let proc_dir = PathBuf::from(format!("/proc/self/fd/{}", dir.as_raw_fd()));
            let target = if proc_dir.is_dir() { proc_dir.join(last) } else { base.join(rel) };
            std::fs::remove_dir_all(target)?;
        } else {
            // A file or a link: the entry itself goes, never what a link points at.
            // SAFETY: valid fd and name.
            if unsafe { libc::unlinkat(dir.as_raw_fd(), name.as_ptr(), 0) } != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(true)
    }

    pub fn remove_empty_dir(base: &Path, rel: &Path) -> bool {
        let parts = normal_parts(rel);
        let Some((last, dirs)) = parts.split_last() else { return false };
        let Ok(Some(dir)) = walk(base, dirs) else { return false };
        let Ok(name) = cstr(last.as_bytes()) else { return false };
        // SAFETY: valid fd and name. AT_REMOVEDIR only removes an empty folder (never a link).
        unsafe { libc::unlinkat(dir.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR) == 0 }
    }
}

#[cfg(not(unix))]
mod nofollow {
    use super::*;

    fn is_link(p: &Path) -> bool {
        std::fs::symlink_metadata(p).map(|m| m.file_type().is_symlink()).unwrap_or(false)
    }

    pub fn write(path: &Path, bytes: &[u8], _mode: u32) -> io::Result<()> {
        use std::io::Write;
        let (parent, name) = split_parent(path)?;
        std::fs::create_dir_all(parent)?;
        if is_link(parent) {
            return Err(symlink_parent_error(parent));
        }
        let tmp = parent.join(temp_name(name));
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        let res = f.write_all(bytes);
        drop(f);
        let res = res.and_then(|_| rename_retrying(&tmp, path));
        if res.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        res
    }

    /// Every folder on the way (and the base's real location) checked; links and junctions refused.
    fn checked(base: &Path, rel: &Path) -> io::Result<Option<PathBuf>> {
        let parts = normal_parts(rel);
        let Ok(real_base) = std::fs::canonicalize(base) else { return Ok(None) };
        let mut p = base.to_path_buf();
        for (i, part) in parts.iter().enumerate() {
            p.push(part);
            match std::fs::symlink_metadata(&p) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(e),
                Ok(m) if m.file_type().is_symlink() && i + 1 < parts.len() => {
                    return Err(refused(format!("'{}' is a link, so Kora will not go through it", part.to_string_lossy())))
                }
                Ok(_) => {}
            }
        }
        if let Some(parent) = p.parent() {
            if !std::fs::canonicalize(parent).is_ok_and(|r| r.starts_with(&real_base)) {
                return Err(refused(format!("{} is not inside {}, so Kora will not touch it", p.display(), base.display())));
            }
        }
        Ok(Some(p))
    }

    pub fn remove(base: &Path, rel: &Path) -> io::Result<bool> {
        let Some(p) = checked(base, rel)? else { return Ok(false) };
        let m = std::fs::symlink_metadata(&p)?;
        if m.is_dir() {
            std::fs::remove_dir_all(&p)?;
        } else if m.file_type().is_symlink() {
            // A link (or junction) to a folder is removed with remove_dir, a file link with remove_file.
            std::fs::remove_file(&p).or_else(|_| std::fs::remove_dir(&p))?;
        } else {
            std::fs::remove_file(&p)?;
        }
        Ok(true)
    }

    pub fn remove_empty_dir(base: &Path, rel: &Path) -> bool {
        matches!(checked(base, rel), Ok(Some(p)) if !is_link(&p) && std::fs::remove_dir(&p).is_ok())
    }
}

/// Write a file the way root must write into a folder another account can change: never through
/// a symbolic link. The parent folder is refused if it is a link; the data goes to a fresh temporary
/// file (`O_CREAT|O_EXCL|O_NOFOLLOW`, permissions `mode` set on the open file) that is then renamed
/// over `path` (which replaces a link there instead of writing through it). Missing parent folders
/// are created. `mode` is ignored off Unix.
pub fn write_no_follow(path: &Path, bytes: &[u8], mode: u32) -> io::Result<()> {
    nofollow::write(path, bytes, mode)
}

/// Delete `base/rel` (a file, link or whole folder) without ever leaving `base`: `rel` must pass
/// [`check_relative`], no folder on the way may be a symbolic link (checked while walking, through
/// open folder handles, so a link swapped in meanwhile is not followed either), and a link at the
/// end is removed itself, never what it points to. `Ok(false)` when there was nothing to delete.
pub fn remove_inside(base: &Path, rel: &Path) -> io::Result<bool> {
    check_relative(rel)?;
    nofollow::remove(base, rel)
}

/// Remove the empty folder `base/rel` (same rules as [`remove_inside`]); `false` if it was not
/// removed (missing, not empty, a link, or `rel` not acceptable).
pub fn remove_empty_dir_inside(base: &Path, rel: &Path) -> bool {
    check_relative(rel).is_ok() && nofollow::remove_empty_dir(base, rel)
}

/// Is `path` itself a symbolic link (or junction)? Missing = no.
pub fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path).map(|m| m.file_type().is_symlink()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn record_paths_must_stay_inside() {
        for ok in ["a", "a/b", "./a", "resources/[x]/y"] {
            assert!(check_relative(Path::new(ok)).is_ok(), "{ok}");
        }
        for bad in ["", ".", "/etc", "../x", "a/../../b", "a/.."] {
            assert!(check_relative(Path::new(bad)).is_err(), "{bad}");
        }
        #[cfg(windows)]
        for bad in ["C:\\Windows", "C:x", "\\\\srv\\share"] {
            assert!(check_relative(Path::new(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn remove_inside_deletes_files_and_folders_but_never_leaves_the_base() {
        let t = tempfile::tempdir().unwrap();
        let base = t.path().join("srv");
        std::fs::create_dir_all(base.join("resources/[a]/x/deep")).unwrap();
        std::fs::write(base.join("resources/[a]/x/deep/f"), "x").unwrap();
        std::fs::write(base.join("server.cfg"), "x").unwrap();
        assert!(remove_inside(&base, Path::new("resources/[a]/x")).unwrap());
        assert!(!base.join("resources/[a]/x").exists() && base.join("resources/[a]").is_dir());
        assert!(remove_inside(&base, Path::new("server.cfg")).unwrap());
        assert!(!remove_inside(&base, Path::new("server.cfg")).unwrap(), "already gone");
        assert!(!remove_inside(&base, Path::new("missing/deeper")).unwrap());
        assert!(remove_inside(&base, Path::new("/etc")).is_err());
        assert!(remove_inside(&base, Path::new("../srv")).is_err());
        assert!(base.is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn remove_inside_does_not_follow_planted_links() {
        let t = tempfile::tempdir().unwrap();
        let base = t.path().join("srv");
        let victim = t.path().join("victim");
        std::fs::create_dir_all(victim.join("etc")).unwrap();
        std::fs::write(victim.join("etc/passwd"), "precious").unwrap();
        std::fs::create_dir_all(&base).unwrap();
        // `resources` is a link to the victim folder: `resources/etc` must not be reached.
        std::os::unix::fs::symlink(&victim, base.join("resources")).unwrap();
        let e = remove_inside(&base, Path::new("resources/etc")).unwrap_err();
        assert!(e.to_string().contains("symbolic link"), "{e}");
        assert!(victim.join("etc/passwd").is_file());
        // A link as the last part is removed itself; its target stays.
        std::os::unix::fs::symlink(victim.join("etc"), base.join("artifacts")).unwrap();
        assert!(remove_inside(&base, Path::new("artifacts")).unwrap());
        assert!(!is_symlink(&base.join("artifacts")) && victim.join("etc/passwd").is_file());
        assert!(!remove_empty_dir_inside(&base, Path::new("resources")), "a link is not an empty folder");
        std::fs::create_dir_all(base.join("empty")).unwrap();
        assert!(remove_empty_dir_inside(&base, Path::new("empty")));
    }

    #[test]
    fn write_no_follow_writes_and_replaces() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("a/.kora/created.json");
        write_no_follow(&p, b"one", 0o600).unwrap();
        write_no_follow(&p, b"two", 0o600).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"two");
        let names: Vec<_> = std::fs::read_dir(p.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names.len(), 1, "no temporary files left: {names:?}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[cfg(unix)]
    #[test]
    fn write_no_follow_never_writes_through_a_link() {
        let t = tempfile::tempdir().unwrap();
        let victim = t.path().join("shadow");
        std::fs::write(&victim, "root:secret").unwrap();
        let dir = t.path().join("srv/.kora");
        std::fs::create_dir_all(&dir).unwrap();
        // The file itself is a link: it is replaced, the target is untouched.
        std::os::unix::fs::symlink(&victim, dir.join("schedule.json")).unwrap();
        write_no_follow(&dir.join("schedule.json"), b"{}", 0o644).unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "root:secret");
        assert!(!is_symlink(&dir.join("schedule.json")));
        // The folder is a link: refused.
        let elsewhere = t.path().join("etc");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, t.path().join("srv2")).unwrap();
        let e = write_no_follow(&t.path().join("srv2/secrets.env"), b"x", 0o600).unwrap_err();
        assert!(e.to_string().contains("symbolic link"), "{e}");
        assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0);
    }

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
