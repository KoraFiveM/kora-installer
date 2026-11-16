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

/// `root/rel` when `rel` is a plain relative path that stays inside `root`. On Windows a `rel`
/// such as `C:/x` or `\\server\share` would make `join` throw `root` away, so anything with a
/// drive, root or `..` is refused (`None`).
pub fn join_inside(root: &Path, rel: &str) -> Option<PathBuf> {
    if rel.is_empty() || rel.contains(':') || !Path::new(rel).components().all(|c| matches!(c, Component::Normal(_))) {
        return None;
    }
    let p = root.join(rel);
    (p.starts_with(root) && p != root).then_some(p)
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
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

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

    /// May a symbolic link inside a folder owned by `uid` with permission bits `mode` be followed?
    /// Root: only in folders root owns that nobody else can write (`/home` -> `/var/home` on
    /// Fedora Silverblue is fine; a link the service account planted in a server folder is not).
    /// Anyone else: also in their own folders (a dotfiles link in `~/.config` is fine).
    pub fn default_trust(uid: u32, mode: u32) -> bool {
        // SAFETY: no preconditions.
        let me = unsafe { libc::geteuid() };
        (uid == 0 && mode & 0o022 == 0) || (me != 0 && uid == me)
    }

    fn fstat_fd(fd: &impl AsRawFd) -> io::Result<libc::stat> {
        // SAFETY: zeroed stat is a valid out-buffer; valid fd.
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(fd.as_raw_fd(), &mut st) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(st)
    }

    fn openat_dir(dir: &OwnedFd, name: &std::ffi::CStr) -> io::Result<OwnedFd> {
        // SAFETY: valid fd and NUL-terminated name.
        let fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: fresh descriptor.
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    fn read_link_at(dir: &OwnedFd, name: &std::ffi::CStr) -> io::Result<PathBuf> {
        let mut buf = vec![0u8; libc::PATH_MAX as usize];
        // SAFETY: valid fd, name and buffer of the stated length.
        let n = unsafe { libc::readlinkat(dir.as_raw_fd(), name.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        buf.truncate(n as usize);
        Ok(PathBuf::from(std::ffi::OsString::from_vec(buf)))
    }

    enum Part {
        Root,
        Up,
        Name(std::ffi::OsString),
    }

    fn parts_of(p: &Path) -> Vec<Part> {
        p.components()
            .filter_map(|c| match c {
                Component::RootDir => Some(Part::Root),
                Component::ParentDir => Some(Part::Up),
                Component::Normal(n) => Some(Part::Name(n.to_os_string())),
                _ => None,
            })
            .collect()
    }

    /// Open the folder `path`, one part at a time from `/`, through open folder handles (so swapping
    /// a folder for a link half way changes nothing). A symbolic link on the way is followed only
    /// when `trust` accepts the folder it is in. `create`: make missing folders (with that mode,
    /// less the umask); otherwise `None` is returned when a part is missing.
    pub fn walk_dir(path: &Path, create: Option<u32>, trust: &dyn Fn(u32, u32) -> bool) -> io::Result<Option<OwnedFd>> {
        let abs = std::path::absolute(path)?;
        let mut todo: std::collections::VecDeque<Part> = parts_of(&abs).into();
        let mut fd = open_dir(Path::new("/"), true)?;
        let mut shown = PathBuf::from("/");
        let (mut links, mut made) = (0, 0);
        while let Some(part) = todo.pop_front() {
            let name = match part {
                Part::Root => {
                    fd = open_dir(Path::new("/"), true)?;
                    shown = PathBuf::from("/");
                    continue;
                }
                Part::Up => {
                    fd = openat_dir(&fd, c"..")?;
                    shown.pop();
                    continue;
                }
                Part::Name(n) => n,
            };
            let c = cstr(name.as_bytes())?;
            match openat_dir(&fd, &c) {
                Ok(next) => {
                    fd = next;
                    shown.push(&name);
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    let Some(mode) = create else { return Ok(None) };
                    made += 1;
                    // SAFETY: valid fd and name. mkdirat never follows a link at `name`.
                    if made > 64 || (unsafe { libc::mkdirat(fd.as_raw_fd(), c.as_ptr(), mode as libc::mode_t) } != 0 && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists) {
                        return Err(io::Error::last_os_error());
                    }
                    todo.push_front(Part::Name(name));
                }
                Err(e) if is_link_error(&e) => {
                    let here = shown.join(&name);
                    // SAFETY: zeroed stat is a valid out-buffer; valid fd and name.
                    let mut st: libc::stat = unsafe { std::mem::zeroed() };
                    let is_link = unsafe { libc::fstatat(fd.as_raw_fd(), c.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) } == 0 && st.st_mode & libc::S_IFMT == libc::S_IFLNK;
                    if !is_link {
                        return Err(io::Error::new(io::ErrorKind::NotADirectory, format!("{} is not a folder", here.display())));
                    }
                    let parent = fstat_fd(&fd)?;
                    if !trust(parent.st_uid, parent.st_mode) {
                        return Err(refused(format!(
                            "{} is a symbolic link inside a folder another account can change, so Kora will not follow it (use the folder's real path instead)",
                            here.display()
                        )));
                    }
                    links += 1;
                    if links > 40 {
                        return Err(io::Error::from_raw_os_error(libc::ELOOP));
                    }
                    let target = read_link_at(&fd, &c)?;
                    for p in parts_of(&target).into_iter().rev() {
                        todo.push_front(p);
                    }
                }
                Err(e) => return Err(e),
            }
        }
        Ok(Some(fd))
    }

    pub fn write(path: &Path, bytes: &[u8], mode: u32, trust: &dyn Fn(u32, u32) -> bool) -> io::Result<()> {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        let (parent, name) = split_parent(path)?;
        let dir = walk_dir(parent, Some(0o777), trust)?.ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
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

    pub fn open_file(path: &Path, access: Access, create_mode: Option<u32>, trust: &dyn Fn(u32, u32) -> bool) -> io::Result<std::fs::File> {
        let (parent, name) = split_parent(path)?;
        let dir = walk_dir(parent, create_mode.map(|_| 0o777), trust)?.ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        let c = cstr(name.as_bytes())?;
        let rw = match access {
            Access::Read => libc::O_RDONLY,
            Access::ReadWrite => libc::O_RDWR,
            Access::Append => libc::O_WRONLY | libc::O_APPEND,
        };
        // O_NONBLOCK: a planted FIFO must not hang the caller (it is refused just below anyway).
        let flags = rw | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK | if create_mode.is_some() { libc::O_CREAT } else { 0 };
        // SAFETY: valid fd and NUL-terminated name.
        let fd = unsafe { libc::openat(dir.as_raw_fd(), c.as_ptr(), flags, create_mode.unwrap_or(0o600) as libc::c_uint) };
        if fd < 0 {
            let e = io::Error::last_os_error();
            return Err(if e.raw_os_error() == Some(libc::ELOOP) { refused(format!("{} is a symbolic link, so Kora will not open it", path.display())) } else { e });
        }
        // SAFETY: fresh descriptor, owned by the File from here on.
        let f = unsafe { std::fs::File::from_raw_fd(fd) };
        let st = fstat_fd(&f)?;
        if st.st_mode & libc::S_IFMT != libc::S_IFREG {
            return Err(refused(format!("{} is not a plain file, so Kora will not open it", path.display())));
        }
        // A hard link to a file elsewhere (say /etc/shadow) would be written in place.
        if access != Access::Read && st.st_nlink > 1 {
            return Err(refused(format!("{} has other names (hard links), so Kora will not write to it", path.display())));
        }
        // SAFETY: valid fd; only clears O_NONBLOCK again.
        unsafe { libc::fcntl(f.as_raw_fd(), libc::F_SETFL, libc::fcntl(f.as_raw_fd(), libc::F_GETFL) & !libc::O_NONBLOCK) };
        Ok(f)
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

    /// No owners to read here: a link (or junction) is never trusted.
    pub fn default_trust(_uid: u32, _mode: u32) -> bool {
        false
    }

    /// The folder itself must not be a link or junction (unless `trust` allows it). Folders above
    /// it are not checked here: moving a profile folder to another drive with a junction is common.
    fn check_parts(path: &Path, trust: &dyn Fn(u32, u32) -> bool) -> io::Result<()> {
        if is_link(path) && !trust(0, 0) {
            return Err(symlink_parent_error(path));
        }
        Ok(())
    }

    pub fn walk_dir(path: &Path, create: Option<u32>, trust: &dyn Fn(u32, u32) -> bool) -> io::Result<Option<()>> {
        check_parts(path, trust)?;
        if create.is_some() {
            std::fs::create_dir_all(path)?;
            check_parts(path, trust)?;
        }
        Ok(path.is_dir().then_some(()))
    }

    pub fn open_file(path: &Path, access: Access, create_mode: Option<u32>, trust: &dyn Fn(u32, u32) -> bool) -> io::Result<std::fs::File> {
        let (parent, _) = split_parent(path)?;
        walk_dir(parent, create_mode, trust)?.ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        if is_link(path) {
            return Err(refused(format!("{} is a link, so Kora will not open it", path.display())));
        }
        let mut o = std::fs::OpenOptions::new();
        match access {
            Access::Read => o.read(true),
            Access::ReadWrite => o.read(true).write(true),
            Access::Append => o.append(true),
        };
        o.create(create_mode.is_some()).open(path)
    }

    pub fn write(path: &Path, bytes: &[u8], _mode: u32, trust: &dyn Fn(u32, u32) -> bool) -> io::Result<()> {
        use std::io::Write;
        let (parent, name) = split_parent(path)?;
        std::fs::create_dir_all(parent)?;
        check_parts(parent, trust)?;
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
    nofollow::write(path, bytes, mode, &nofollow::default_trust)
}

/// How [`open_no_follow`] opens a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Read,
    ReadWrite,
    /// Write only, at the end.
    Append,
}

/// Open (with `create_mode`: create if missing, plus missing parent folders) a file in a folder
/// another account may change, for a log or a lock file root keeps open. The folders on the way
/// are walked without following a link anyone but root (or, for a normal user, they themselves)
/// could have planted; a link at the file itself is refused, and so is anything that is not a
/// plain file (a FIFO would hang, a device...). For writing, a file with other hard links is
/// refused too. `create_mode` is ignored off Unix.
pub fn open_no_follow(path: &Path, access: Access, create_mode: Option<u32>) -> io::Result<std::fs::File> {
    nofollow::open_file(path, access, create_mode, &nofollow::default_trust)
}

/// `create_dir_all` that never goes through a link another account could have planted (see
/// [`open_no_follow`] for which links are followed). Missing folders get the default permissions.
pub fn create_dir_no_follow(path: &Path) -> io::Result<()> {
    nofollow::walk_dir(path, Some(0o777), &nofollow::default_trust).map(|_| ())
}

/// `Err` when a part of `path` is a symbolic link another account could have planted (or a part
/// is not a folder); missing parts are fine. Root checks a folder with this before working in it.
pub fn check_links(path: &Path) -> io::Result<()> {
    check_links_with(path, &nofollow::default_trust)
}

/// The rule [`check_links`] uses: may a link inside a folder with this owner and mode be followed?
pub(crate) fn check_links_default_trust(uid: u32, mode: u32) -> bool {
    nofollow::default_trust(uid, mode)
}

/// [`check_links`] with the rule for which links may be followed given (tests: root's view).
pub(crate) fn check_links_with(path: &Path, trust: &dyn Fn(u32, u32) -> bool) -> io::Result<()> {
    nofollow::walk_dir(path, None, trust).map(|_| ())
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
    fn join_inside_keeps_paths_under_the_root() {
        let root = Path::new("srv/resources");
        assert_eq!(join_inside(root, "[kora]/a"), Some(root.join("[kora]/a")));
        for evil in ["", "..", "a/../../x", "/etc", "C:/x", "C:x", "./a", "a:b"] {
            assert_eq!(join_inside(root, evil), None, "{evil}");
        }
    }

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
        // The folder is a link in a folder another account can change (root's view): refused.
        let elsewhere = t.path().join("etc");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, t.path().join("srv2")).unwrap();
        let strict = |_: u32, _: u32| false;
        let e = nofollow::write(&t.path().join("srv2/secrets.env"), b"x", 0o600, &strict).unwrap_err();
        assert!(e.to_string().contains("symbolic link"), "{e}");
        // ... also deeper down, and no folder is created through it.
        let e = nofollow::write(&t.path().join("srv2/new/deeper/x"), b"x", 0o600, &strict).unwrap_err();
        assert!(e.to_string().contains("symbolic link"), "{e}");
        assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0);
    }

    /// A user's own setup (`~/.config/systemd/user` linked into a dotfiles folder): a link in a folder
    /// the user owns is theirs, so user-level writes go through it (root's rule is stricter).
    #[cfg(unix)]
    #[test]
    fn a_link_in_ones_own_folder_is_followed() {
        let t = tempfile::tempdir().unwrap();
        let dotfiles = t.path().join("dotfiles/systemd/user");
        std::fs::create_dir_all(&dotfiles).unwrap();
        std::fs::create_dir_all(t.path().join(".config/systemd")).unwrap();
        std::os::unix::fs::symlink(&dotfiles, t.path().join(".config/systemd/user")).unwrap();
        // (tempdirs are private to the account running the tests, root included)
        write_no_follow(&t.path().join(".config/systemd/user/kora-panel.service"), b"[Unit]", 0o644).unwrap();
        assert_eq!(std::fs::read(dotfiles.join("kora-panel.service")).unwrap(), b"[Unit]");
        assert!(check_links(&t.path().join(".config/systemd/user")).is_ok());
        // Root's rule: a link in a folder that root does not own is never followed.
        let root_rule = |uid: u32, mode: u32| uid == 0 && mode & 0o022 == 0;
        let me = std::fs::metadata(t.path()).map(|m| std::os::unix::fs::MetadataExt::uid(&m)).unwrap();
        if me != 0 {
            assert!(nofollow::walk_dir(&t.path().join(".config/systemd/user"), None, &root_rule).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn folders_are_made_and_checked_without_going_through_planted_links() {
        let t = tempfile::tempdir().unwrap();
        create_dir_no_follow(&t.path().join("a/b/c")).unwrap();
        assert!(t.path().join("a/b/c").is_dir());
        create_dir_no_follow(&t.path().join("a/b/c")).unwrap();
        assert!(check_links(&t.path().join("a/b/c/missing/x")).is_ok(), "missing parts are fine");
        // A link planted where root would go: refused, nothing made at its target.
        let victim = t.path().join("victim");
        std::fs::create_dir_all(&victim).unwrap();
        std::os::unix::fs::symlink(&victim, t.path().join("a/b/srv")).unwrap();
        let strict = |_: u32, _: u32| false;
        let e = nofollow::walk_dir(&t.path().join("a/b/srv/new"), Some(0o777), &strict).unwrap_err();
        assert!(e.to_string().contains("symbolic link") && e.to_string().contains("a/b/srv"), "{e}");
        assert_eq!(std::fs::read_dir(&victim).unwrap().count(), 0);
        // A trusted link (like /home -> /var/home) is followed, relative ones and `..` included.
        std::os::unix::fs::symlink("../b/c", t.path().join("a/b/rel")).unwrap();
        let ok = |_: u32, _: u32| true;
        assert!(nofollow::walk_dir(&t.path().join("a/b/rel/made"), Some(0o777), &ok).unwrap().is_some());
        assert!(t.path().join("a/b/c/made").is_dir());
        // A file on the way is not a folder.
        std::fs::write(t.path().join("a/file"), "x").unwrap();
        assert!(create_dir_no_follow(&t.path().join("a/file/x")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn open_no_follow_appends_and_refuses_links_fifos_and_hard_links() {
        use std::io::Write;
        let t = tempfile::tempdir().unwrap();
        let log = t.path().join("srv/update.log");
        for line in ["one\n", "two\n"] {
            let mut f = open_no_follow(&log, Access::Append, Some(0o644)).unwrap();
            f.write_all(line.as_bytes()).unwrap();
        }
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "one\ntwo\n");
        assert!(open_no_follow(&t.path().join("srv/none"), Access::ReadWrite, None).is_err_and(|e| e.kind() == io::ErrorKind::NotFound));
        // A link where the log goes (root would otherwise create /etc/nologin or append to a system file).
        let victim = t.path().join("nologin");
        std::os::unix::fs::symlink(&victim, t.path().join("srv/link.log")).unwrap();
        let e = open_no_follow(&t.path().join("srv/link.log"), Access::Append, Some(0o644)).unwrap_err();
        assert!(e.to_string().contains("symbolic link"), "{e}");
        assert!(!victim.exists());
        // A link as the folder (root's rule): refused.
        std::os::unix::fs::symlink(t.path(), t.path().join("srv/.kora")).unwrap();
        let strict = |_: u32, _: u32| false;
        assert!(nofollow::open_file(&t.path().join("srv/.kora/update.lock"), Access::ReadWrite, Some(0o644), &strict).is_err());
        assert!(!t.path().join("update.lock").exists());
        // A FIFO would block a writer forever: refused at once.
        let fifo = cstr_path(&t.path().join("srv/fifo"));
        // SAFETY: valid path.
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o644) }, 0);
        assert!(open_no_follow(&t.path().join("srv/fifo"), Access::Append, Some(0o644)).is_err());
        // A second name for another file: not written.
        std::fs::write(t.path().join("precious"), "x").unwrap();
        std::fs::hard_link(t.path().join("precious"), t.path().join("srv/hard.log")).unwrap();
        let e = open_no_follow(&t.path().join("srv/hard.log"), Access::Append, Some(0o644)).unwrap_err();
        assert!(e.to_string().contains("hard links"), "{e}");
        assert!(open_no_follow(&t.path().join("srv/hard.log"), Access::Read, None).is_ok(), "reading is harmless");
    }

    #[cfg(unix)]
    fn cstr_path(p: &Path) -> std::ffi::CString {
        std::ffi::CString::new(p.as_os_str().as_encoded_bytes()).unwrap()
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
