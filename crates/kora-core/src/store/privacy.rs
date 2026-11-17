//! Keeping the store private, and keeping its files with the right owner.
//!
//! Threat model: the store holds saved licence keys, remote-access token hashes and hub node tokens, so it
//! must be exactly as private as `secrets.json` was. Another account on the machine must not read it;
//! and when `sudo kora ...` (or the Linux helper) runs as root inside someone else's Kora home, nothing
//! root creates may end up owned by root (the person could no longer open their own store) or be
//! written THROUGH a link that person planted (root would overwrite or hand over any file).
//!
//! * Everything lives in one private folder, `<kora home>/store/` (`0700` on Linux; on Windows the
//!   inherited permissions are removed and only the signed-in user is granted access, the same
//!   `icacls` rule `secrets.json` has always used, applied to the FOLDER so the `-wal`/`-shm`/journal
//!   files SQLite creates and deletes by itself inherit it). The database file itself is created
//!   `0600`; SQLite gives its `-wal`/`-shm`/`-journal` files the same permissions (and, for root, the same owner).
//! * The folder and the files are never followed through a link: a link anywhere we are about to
//!   open is refused (and SQLite is also asked for `SQLITE_OPEN_NOFOLLOW`).
//! * As root in a folder that belongs to someone else, every file and folder we create is handed
//!   to that folder's owner with `lchown` (never `chown`, which follows a link swapped in meanwhile).
//!
//! The decisions are small pure functions (unit-tested on every OS); the system calls around them are thin.

use std::path::{Path, PathBuf};

use crate::problem::{Action, Problem};

use super::codes;

/// The names SQLite creates next to the database file.
pub const SIDE_SUFFIXES: [&str; 3] = ["-wal", "-shm", "-journal"];

pub fn side_file(db: &Path, suffix: &str) -> PathBuf {
    let mut s = db.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// The icacls arguments that make `dir` private to `user` on Windows: no inherited permissions, and
/// full control for the user that applies to everything created inside (`OI` objects, `CI` containers).
pub fn windows_dir_acl_args(dir: &Path, user: &str) -> Result<Vec<String>, String> {
    if user.is_empty() || !user.chars().all(|c| c.is_alphanumeric() || " ._-\\$".contains(c)) {
        return Err("the Windows user name has characters Kora does not pass to icacls".into());
    }
    Ok(vec![dir.to_string_lossy().into_owned(), "/inheritance:r".into(), "/grant:r".into(), format!("{user}:(OI)(CI)F")])
}

/// Is a Unix mode readable or writable by group or others?
pub fn too_open(mode: u32) -> bool {
    mode & 0o077 != 0
}

fn privacy_problem(what: &str, details: String) -> Problem {
    Problem::new(codes::STORE_PRIVACY, format!("Kora could not make its data folder private ({what}), so it will not store anything there. Nothing was changed."))
        .with_details(details)
        .with_action(Action::CopyDetails)
}

/// Refuse a link where we are about to create or open something.
pub fn refuse_link(path: &Path) -> Result<(), Problem> {
    if crate::fsutil::is_symlink(path) {
        return Err(privacy_problem(
            "a link is in the way",
            format!("{} is a symbolic link. Kora never writes through a link. Remove the link (the real folder or file is untouched) and try again.", path.display()),
        ));
    }
    Ok(())
}

/// Create the private store folder (and the Kora home above it when missing). Idempotent.
pub fn ensure_private_dir(home: &Path, dir: &Path) -> Result<(), Problem> {
    refuse_link(dir)?;
    let existed = dir.is_dir();
    crate::fsutil::create_dir_no_follow(home).map_err(|e| privacy_problem("the Kora folder cannot be created", format!("{}: {e}", home.display())))?;
    if !existed {
        std::fs::create_dir(dir).or_else(|e| if dir.is_dir() { Ok(()) } else { Err(e) }).map_err(|e| privacy_problem("the data folder cannot be created", format!("{}: {e}", dir.display())))?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // On the folder itself (never through a link: checked above), whatever umask created it.
        let m = std::fs::symlink_metadata(dir).map_err(|e| privacy_problem("the data folder cannot be read", format!("{}: {e}", dir.display())))?;
        if !m.is_dir() {
            return Err(privacy_problem("it is not a folder", dir.display().to_string()));
        }
        if too_open(m.permissions().mode()) || m.permissions().mode() & 0o700 != 0o700 {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| privacy_problem("its permissions cannot be set", format!("{}: {e}", dir.display())))?;
        }
    }
    #[cfg(windows)]
    if !existed {
        let user = std::env::var("USERNAME").ok().filter(|u| !u.is_empty()).ok_or_else(|| privacy_problem("the Windows user name is not known", "USERNAME is not set".into()))?;
        let args = windows_dir_acl_args(dir, &user).map_err(|e| privacy_problem("the Windows user name is unusual", e))?;
        let spec = crate::db::CommandSpec { program: "icacls".into(), args, ..crate::db::CommandSpec::default() };
        let ok = crate::db::CommandRunner::run(&crate::db::SystemRunner, &spec).map(|o| o.ok()).unwrap_or(false);
        if !ok {
            let _ = std::fs::remove_dir(dir);
            return Err(privacy_problem("icacls could not restrict the folder", dir.display().to_string()));
        }
    }
    if !existed {
        hand_to_owner_of_parent(dir);
    }
    Ok(())
}

/// Create the (empty) database file, owner-only from the first instant. Existing files are left as they are.
pub fn ensure_private_file(path: &Path) -> Result<(), Problem> {
    refuse_link(path)?;
    if path.exists() {
        tighten(path);
        return Ok(());
    }
    // `create_new` + 0600 + no-follow: a file someone raced into place is not ours to write.
    match crate::fsutil::open_no_follow(path, crate::fsutil::Access::ReadWrite, Some(0o600)) {
        Ok(_) => {}
        Err(e) => return Err(privacy_problem("the data file cannot be created", format!("{}: {e}", path.display()))),
    }
    hand_to_owner_of_parent(path);
    Ok(())
}

/// A database copied around may be readable by others: take that away (best effort, on the open file).
pub fn tighten(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(f) = crate::fsutil::open_no_follow(path, crate::fsutil::Access::Read, None) {
            if f.metadata().is_ok_and(|m| too_open(m.permissions().mode())) {
                let _ = f.set_permissions(std::fs::Permissions::from_mode(0o600));
            }
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// A subfolder of the private folder (backups): `0700` too, and handed to the owner when root made it.
/// (On Windows it inherits the private folder's access list.)
pub fn tighten_dir(dir: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if !crate::fsutil::is_symlink(dir) {
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
    }
    hand_to_owner_of_parent(dir);
}

/// Running as root (not through a link from someone else's folder): give `path` to the owner of its
/// folder. `lchown`: if the folder's owner swapped `path` for a link meanwhile, the link itself is
/// handed over, never what it points to. Nothing happens off Unix, when not root, or when the folder
/// is root's own.
pub fn hand_to_owner_of_parent(path: &Path) {
    #[cfg(unix)]
    {
        use crate::userhome::UserEnv;
        use std::os::unix::fs::MetadataExt;
        if crate::userhome::SystemUserEnv.euid() != 0 {
            return;
        }
        if let Some(m) = path.parent().and_then(|p| std::fs::metadata(p).ok()) {
            if m.uid() != 0 {
                let _ = std::os::unix::fs::lchown(path, Some(m.uid()), Some(m.gid()));
            }
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Hand the database and its side files over (see [`hand_to_owner_of_parent`]). The files' owner is
/// taken from the store folder, which was handed to the Kora home's owner when it was created.
pub fn fix_owner(db: &Path) {
    hand_to_owner_of_parent(db);
    for s in SIDE_SUFFIXES {
        let p = side_file(db, s);
        if std::fs::symlink_metadata(&p).is_ok() {
            hand_to_owner_of_parent(&p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_acl_covers_the_folder_and_what_is_created_in_it() {
        let a = windows_dir_acl_args(Path::new(r"C:\Users\Sam\AppData\Local\Kora\store"), "Sam").unwrap();
        assert_eq!(a[1..], ["/inheritance:r", "/grant:r", "Sam:(OI)(CI)F"]);
        assert!(windows_dir_acl_args(Path::new("x"), "").is_err());
        assert!(windows_dir_acl_args(Path::new("x"), "a b;c").is_err());
        assert!(windows_dir_acl_args(Path::new("x"), r"DOMAIN\sam").is_ok());
    }

    #[test]
    fn open_modes_are_spotted() {
        assert!(!too_open(0o600) && !too_open(0o700));
        assert!(too_open(0o644) && too_open(0o660) && too_open(0o607));
    }

    #[test]
    fn side_files_sit_next_to_the_database() {
        assert_eq!(side_file(Path::new("/a/kora.db"), "-wal"), PathBuf::from("/a/kora.db-wal"));
    }

    #[cfg(unix)]
    #[test]
    fn the_folder_and_file_are_private_and_links_are_refused() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("home/store");
        ensure_private_dir(&t.path().join("home"), &dir).unwrap();
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        let f = dir.join("kora.db");
        ensure_private_file(&f).unwrap();
        assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600);
        // a too-open leftover is tightened
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();
        ensure_private_file(&f).unwrap();
        assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600);
        // links
        let victim = t.path().join("victim");
        std::fs::write(&victim, "x").unwrap();
        let link = dir.join("other.db");
        std::os::unix::fs::symlink(&victim, &link).unwrap();
        assert!(ensure_private_file(&link).is_err());
        let linkdir = t.path().join("home2");
        std::fs::create_dir(&linkdir).unwrap();
        std::os::unix::fs::symlink(t.path(), linkdir.join("store")).unwrap();
        assert!(ensure_private_dir(&linkdir, &linkdir.join("store")).is_err());
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "x");
    }
}
