//! Which server folders the helper will install into.
//!
//! `kora install` as root creates the folder, writes into it and finally hands it (`chown -R`) to
//! the server's own account. Through the helper the panel's user chooses that folder, so without a
//! limit they could aim root at any folder on the machine (`--dir /etc/systemd/system`). The helper
//! therefore only installs into a folder the user could have made themselves: a new folder (or an
//! empty one, or the folder of an earlier attempt that Kora left behind) whose nearest existing
//! parent belongs to the user, or a folder inside the `kora` account's home (`/home/kora`, the
//! default when Kora runs as root). Anything else is still possible by running the install with
//! `sudo` directly. No link is followed: a folder that is a symbolic link is refused, and the
//! parent's owner is read from where its path really leads.

use std::path::{Path, PathBuf};

/// The `kora` account (owner of `/home/kora`), if it exists: `(home, uid)`.
pub type KoraAccount = Option<(PathBuf, u32)>;

fn refuse(dir: &Path, why: &str) -> String {
    format!(
        "The Kora helper will not install into {}: {why}. Choose a new folder in your home folder (for example ~/kora/<name>) or in {}, or run the install command yourself with sudo.",
        dir.display(),
        crate::userhome::KORA_USER_HOME
    )
}

/// Check `dir` for an install the helper runs for the user with uid `user_uid` (see the module docs).
#[cfg(unix)]
pub fn check_install_dir(dir: &Path, user_uid: u32, kora: &KoraAccount) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    if !dir.is_absolute() || dir.components().any(|c| matches!(c, std::path::Component::ParentDir | std::path::Component::CurDir)) || dir.parent().is_none() {
        return Err(refuse(dir, "it is not a full folder path (or it contains '..')"));
    }
    let kora_home = Path::new(crate::userhome::KORA_USER_HOME);
    // Nearest part of the path that exists (without following a link at that part).
    let mut existing = dir.to_path_buf();
    let meta = loop {
        match std::fs::symlink_metadata(&existing) {
            Ok(m) => break m,
            Err(_) => match existing.parent() {
                Some(p) => existing = p.to_path_buf(),
                None => return Err(refuse(dir, "none of its parent folders exist")),
            },
        }
    };
    let parent_to_check = if existing == dir {
        if meta.file_type().is_symlink() || !meta.is_dir() {
            return Err(refuse(dir, "it is a symbolic link or not a folder"));
        }
        let empty = std::fs::read_dir(dir).map(|mut d| d.next().is_none()).unwrap_or(false);
        let earlier_attempt = std::fs::symlink_metadata(dir.join(".kora").join("request.json")).is_ok_and(|m| m.is_file());
        if !empty && !earlier_attempt {
            return Err(refuse(dir, "the folder already exists and is not empty"));
        }
        dir.parent().map(Path::to_path_buf).unwrap_or_default()
    } else {
        existing
    };
    // `/home/kora` does not exist yet: the install creates the `kora` account and its home itself.
    if dir.starts_with(kora_home) && dir != kora_home && std::fs::symlink_metadata(kora_home).is_err() {
        return Ok(());
    }
    let real = std::fs::canonicalize(&parent_to_check).map_err(|e| refuse(dir, &format!("cannot inspect {}: {e}", parent_to_check.display())))?;
    let owner = std::fs::metadata(&real).map_err(|e| refuse(dir, &e.to_string()))?.uid();
    if owner == user_uid && owner != 0 {
        return Ok(());
    }
    if let Some((home, uid)) = kora {
        let home_is_real = std::fs::symlink_metadata(home).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink());
        if home.as_path() == kora_home && home_is_real && inside_home(&real, home) && owner == *uid && *uid != 0 {
            return Ok(());
        }
    }
    Err(refuse(dir, &format!("{} does not belong to you", real.display())))
}

/// Is the resolved path `real` inside `home`? `home` is resolved too: `/home` is a link to
/// `/var/home` on Fedora Silverblue and similar systems, so `/home/kora` itself never prefixes a
/// resolved path there.
#[cfg(unix)]
fn inside_home(real: &Path, home: &Path) -> bool {
    std::fs::canonicalize(home).is_ok_and(|h| real.starts_with(h))
}

#[cfg(not(unix))]
pub fn check_install_dir(dir: &Path, _user_uid: u32, _kora: &KoraAccount) -> Result<(), String> {
    Err(refuse(dir, "the helper is not available on this system"))
}

/// The `kora` account as the passwd database knows it.
pub fn kora_account() -> KoraAccount {
    crate::userhome::lookup_name(crate::userhome::KORA_USER).map(|a| (a.home, a.uid))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A temp folder owned by a non-root "user": its own uid normally; when the tests run as root,
    /// the folder is handed to uid 4242 so the rules can still be shown.
    fn user_tmp() -> (tempfile::TempDir, u32) {
        let t = tempfile::tempdir().unwrap();
        // SAFETY: no preconditions.
        let me = unsafe { libc::geteuid() };
        if me != 0 {
            return (t, me);
        }
        let c = std::ffi::CString::new(t.path().as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: valid path.
        assert_eq!(unsafe { libc::chown(c.as_ptr(), 4242, 4242) }, 0);
        (t, 4242)
    }

    #[test]
    fn a_new_or_empty_folder_under_the_users_own_folder_is_fine() {
        let (t, me) = user_tmp();
        assert!(check_install_dir(&t.path().join("kora/new"), me, &None).is_ok());
        std::fs::create_dir_all(t.path().join("empty")).unwrap();
        assert!(check_install_dir(&t.path().join("empty"), me, &None).is_ok());
        std::fs::create_dir_all(t.path().join("again/.kora")).unwrap();
        std::fs::write(t.path().join("again/.kora/request.json"), "{}").unwrap();
        assert!(check_install_dir(&t.path().join("again"), me, &None).is_ok(), "an earlier attempt can be resumed");
    }

    #[test]
    fn system_folders_links_and_full_folders_are_refused() {
        let (t, me) = user_tmp();
        for bad in ["/etc/systemd/system", "/root/.ssh", "/usr/local/bin/x", "/etc"] {
            let e = check_install_dir(Path::new(bad), 4242, &None).unwrap_err();
            assert!(e.contains("will not install") && e.contains("sudo"), "{bad}: {e}");
        }
        for bad in ["relative/x", "/home/u/../../etc"] {
            assert!(check_install_dir(Path::new(bad), me, &None).is_err(), "{bad}");
        }
        std::fs::create_dir_all(t.path().join("full")).unwrap();
        std::fs::write(t.path().join("full/data"), "x").unwrap();
        assert!(check_install_dir(&t.path().join("full"), me, &None).unwrap_err().contains("not empty"));
        // A link to a system folder, as the folder itself or on the way.
        std::os::unix::fs::symlink("/etc", t.path().join("link")).unwrap();
        assert!(check_install_dir(&t.path().join("link"), me, &None).unwrap_err().contains("symbolic link"));
        assert!(check_install_dir(&t.path().join("link/newdir"), me, &None).unwrap_err().contains("/etc does not belong to you"));
        // Someone else's folder.
        assert!(check_install_dir(&t.path().join("new"), me.wrapping_add(1), &None).is_err());
        // Root itself is never "the user".
        assert!(check_install_dir(Path::new("/etc/x"), 0, &None).is_err());
    }

    #[test]
    fn a_home_reached_through_a_link_still_counts() {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(t.path().join("var/home/kora/srv")).unwrap();
        std::os::unix::fs::symlink(t.path().join("var/home"), t.path().join("home")).unwrap();
        let real = std::fs::canonicalize(t.path().join("home/kora/srv")).unwrap();
        assert!(inside_home(&real, &t.path().join("home/kora")));
        assert!(!inside_home(&real, &t.path().join("home/other")));
        assert!(!inside_home(&std::fs::canonicalize(t.path()).unwrap(), &t.path().join("home/kora")));
    }

    #[test]
    fn the_kora_home_is_accepted_only_as_the_real_home_of_the_kora_account() {
        // A fake account whose home is not /home/kora is not the kora home.
        let (t, me) = user_tmp();
        let fake: KoraAccount = Some((t.path().to_path_buf(), me));
        assert!(check_install_dir(&t.path().join("x"), me.wrapping_add(1), &fake).is_err());
        // /home/kora (missing on test machines): the install makes it.
        if std::fs::symlink_metadata("/home/kora").is_err() {
            assert!(check_install_dir(Path::new("/home/kora/my-server"), 4242, &None).is_ok());
            assert!(check_install_dir(Path::new("/home/kora"), 4242, &None).is_err());
        }
    }
}
