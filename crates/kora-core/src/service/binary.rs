//! Where the `kora` program lives while a background service runs it, and whether the service
//! account can reach its server folder.
//!
//! A service must not execute a program from a folder that an ordinary user can modify (that
//! would let that user run code as the service account), and the service account cannot enter a
//! private home folder anyway. So when `kora` is not already in a root-owned, non-writable place
//! (for example `/usr/local/bin` from `install.sh`), the installer puts one copy in
//! `/opt/kora/<version>-<sha12>/kora` (Windows: `%ProgramFiles%\Kora\bin\...`). The path contains
//! the content hash, so an existing file there is, by construction, the same program, and nothing
//! that was installed earlier is ever overwritten.

use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::{Plan, Platform, SystemStep};

/// Root folder for the shared copies.
pub fn default_root(platform: Platform) -> Option<PathBuf> {
    match platform {
        Platform::Linux => Some(PathBuf::from("/opt/kora")),
        Platform::Windows => std::env::var_os("ProgramFiles").map(|p| PathBuf::from(p).join("Kora").join("bin")),
    }
}

/// Hex SHA-256 of a file.
pub fn file_sha256(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

/// `<root>/<version>-<sha12>/kora[.exe]`.
pub fn versioned_path(root: &Path, version: &str, sha256: &str, platform: Platform) -> PathBuf {
    let name = if platform == Platform::Windows { "kora.exe" } else { "kora" };
    root.join(format!("{version}-{}", &sha256[..sha256.len().min(12)])).join(name)
}

/// The 12 hex characters a [`versioned_path`] folder (`<version>-<sha12>`) promises, if `dest` is one.
pub fn pinned_sha12(dest: &Path) -> Option<String> {
    let folder = dest.parent()?.file_name()?.to_str()?;
    let tail = folder.rsplit_once('-')?.1;
    (tail.len() == 12 && tail.bytes().all(|b| b.is_ascii_hexdigit())).then(|| tail.to_ascii_lowercase())
}

/// For a pinned destination, check that `copy` really hashes to the promised prefix.
pub fn verify_pinned_copy(dest: &Path, copy: &Path) -> std::io::Result<()> {
    let Some(want) = pinned_sha12(dest) else { return Ok(()) };
    let got = file_sha256(copy)?;
    if got.starts_with(&want) {
        Ok(())
    } else {
        Err(std::io::Error::other(format!("the copy of the kora program changed while it was being copied (expected sha256 {want}..., got {}...): nothing was installed", &got[..12])))
    }
}

/// Copy `src` to `dest` unless it is already there.
pub fn install_plan(src: &str, dest: &Path) -> Plan {
    let d = dest.to_string_lossy();
    Plan::new(vec![SystemStep::copy(
        format!("Put a root-owned copy of the kora program at {d} (the background service runs it from there, not from your home folder)"),
        src,
        &d,
        0o755,
    )])
}

/// Remove a copy made by [`install_plan`] and its (then empty) folders.
pub fn remove_plan(dest: &Path) -> Plan {
    let mut steps = vec![SystemStep::remove(format!("Delete the kora program copy {}", dest.display()), &dest.to_string_lossy())];
    if let Some(dir) = dest.parent() {
        steps.push(SystemStep::remove_dir_if_empty(format!("Remove the folder {} if it is now empty", dir.display()), &dir.to_string_lossy()));
        if let Some(root) = dir.parent() {
            steps.push(SystemStep::remove_dir_if_empty(format!("Remove {} if it is now empty", root.display()), &root.to_string_lossy()));
        }
    }
    Plan::new(steps)
}

/// Owner and permission bits of one path component (Unix).
#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub uid: u32,
    pub mode: u32,
}

/// Safe to run as a service: the file and every folder above it belong to root and nobody but
/// root can write to them; the file is executable and readable by others. `chain` = root folder
/// first, the file last.
#[cfg(unix)]
pub fn chain_is_service_safe(chain: &[Entry]) -> bool {
    let Some((file, dirs)) = chain.split_last() else { return false };
    let locked = |e: &Entry| e.uid == 0 && e.mode & 0o022 == 0;
    file.mode & 0o005 == 0o005 && locked(file) && dirs.iter().all(|d| locked(d) && d.mode & 0o001 != 0)
}

/// Can a different unprivileged account walk down to the folder? Returns the first folder that
/// blocks it. `chain` = root folder first.
#[cfg(unix)]
pub fn first_blocking_dir(chain: &[(PathBuf, Entry)]) -> Option<&Path> {
    chain.iter().find(|(_, e)| e.mode & 0o001 == 0).map(|(p, _)| p.as_path())
}

#[cfg(unix)]
fn entries_down_to(path: &Path) -> Vec<(PathBuf, Entry)> {
    use std::os::unix::fs::MetadataExt;
    let mut anc: Vec<PathBuf> = path.ancestors().map(Path::to_path_buf).collect();
    anc.reverse();
    anc.into_iter()
        .filter_map(|p| std::fs::metadata(&p).ok().map(|m| (p, Entry { uid: m.uid(), mode: m.mode() })))
        .collect()
}

/// Is `exe` somewhere a root-run service may execute it from (see [`chain_is_service_safe`])?
#[cfg(unix)]
pub fn exe_is_service_safe(exe: &Path) -> bool {
    match std::fs::canonicalize(exe) {
        Ok(real) => {
            let chain: Vec<Entry> = entries_down_to(&real).into_iter().map(|(_, e)| e).collect();
            chain_is_service_safe(&chain)
        }
        Err(_) => false,
    }
}

#[cfg(not(unix))]
pub fn exe_is_service_safe(_exe: &Path) -> bool {
    false // program folders are copied under Program Files
}

/// A plain-language problem if another account cannot reach `dir` (private home folder etc.).
/// For a folder that does not exist yet its nearest existing parent is examined.
#[cfg(unix)]
pub fn unreachable_reason(dir: &Path) -> Option<String> {
    let mut p = dir.to_path_buf();
    while !p.exists() {
        p = p.parent()?.to_path_buf();
    }
    let chain = entries_down_to(&std::fs::canonicalize(&p).unwrap_or(p));
    first_blocking_dir(&chain).map(|b| {
        format!(
            "the background service runs as its own account, which cannot enter {} (it is private to its owner). Choose a server folder outside private folders, for example /srv/kora/<name>, or run `chmod o+x {}`.",
            b.display(),
            b.display()
        )
    })
}

#[cfg(not(unix))]
pub fn unreachable_reason(_dir: &Path) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_swapped_source_is_rejected_when_the_folder_pins_a_hash() {
        let dir = std::env::temp_dir().join(format!("kora-pin-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("kora");
        std::fs::write(&src, b"good").unwrap();
        let sha = file_sha256(&src).unwrap();
        let dest = versioned_path(Path::new("/opt/kora"), "0.1.0", &sha, Platform::Linux);
        assert!(verify_pinned_copy(&dest, &src).is_ok());
        std::fs::write(&src, b"evil").unwrap(); // swapped after hashing
        assert!(verify_pinned_copy(&dest, &src).unwrap_err().to_string().contains("changed while"));
        // not a pinned folder: no claim to check
        assert!(verify_pinned_copy(Path::new("/opt/kora/kora"), &src).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn versioned_path_pins_version_and_hash() {
        let sha = "0123456789abcdef0123";
        assert_eq!(versioned_path(Path::new("/opt/kora"), "0.1.0", sha, Platform::Linux), Path::new("/opt/kora/0.1.0-0123456789ab/kora"));
        assert!(versioned_path(Path::new("C:/K"), "0.1.0", sha, Platform::Windows).ends_with("kora.exe"));
    }

    #[test]
    fn install_and_remove_plans_touch_only_the_copy_and_empty_folders() {
        let dest = Path::new("/opt/kora/0.1.0-abc/kora");
        let i = install_plan("/home/me/kora", dest);
        assert!(matches!(&i.steps[0].action, super::super::Action::CopyFile { src, dst, mode: Some(0o755) } if src == "/home/me/kora" && dst == "/opt/kora/0.1.0-abc/kora"));
        let r = remove_plan(dest);
        let acts: Vec<String> = r.steps.iter().map(|s| format!("{:?}", s.action)).collect();
        assert!(acts[0].contains("RemoveFile") && acts[1].contains("RemoveEmptyDir") && acts[2].contains("RemoveEmptyDir"));
        assert!(r.steps.iter().all(|s| s.ignore_failure));
    }

    #[test]
    fn sha256_of_a_file() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("x");
        std::fs::write(&f, b"abc").unwrap();
        assert_eq!(file_sha256(&f).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[cfg(unix)]
    #[test]
    fn service_safe_chain_rules() {
        let root_dir = Entry { uid: 0, mode: 0o040755 };
        let exe = Entry { uid: 0, mode: 0o100755 };
        assert!(chain_is_service_safe(&[root_dir, root_dir, exe]));
        assert!(!chain_is_service_safe(&[root_dir, Entry { uid: 1000, mode: 0o040755 }, exe]), "a folder owned by a user");
        assert!(!chain_is_service_safe(&[root_dir, Entry { uid: 0, mode: 0o040775 }, exe]), "group-writable folder");
        assert!(!chain_is_service_safe(&[Entry { uid: 0, mode: 0o040700 }, exe]), "private folder: service cannot enter");
        assert!(!chain_is_service_safe(&[root_dir, Entry { uid: 1000, mode: 0o100755 }]), "file owned by a user");
        assert!(!chain_is_service_safe(&[root_dir, Entry { uid: 0, mode: 0o100750 }]), "not executable by others");
        assert!(!chain_is_service_safe(&[]));
    }

    #[cfg(unix)]
    #[test]
    fn private_folders_block_the_service_account() {
        let e = |m| Entry { uid: 1000, mode: m };
        let chain = vec![(PathBuf::from("/"), e(0o040755)), (PathBuf::from("/home/me"), e(0o040700)), (PathBuf::from("/home/me/kora"), e(0o040755))];
        assert_eq!(first_blocking_dir(&chain), Some(Path::new("/home/me")));
        assert_eq!(first_blocking_dir(&chain[..1]), None);
    }

    #[cfg(unix)]
    #[test]
    fn real_tempdir_is_reachable_and_a_user_owned_exe_is_not_service_safe() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("a/b");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(t.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(unreachable_reason(&dir.join("not/yet")).is_none());
        std::fs::set_permissions(t.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let why = unreachable_reason(&dir).unwrap();
        assert!(why.contains("cannot enter") && why.contains(&*t.path().to_string_lossy()), "{why}");
        std::fs::set_permissions(t.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        let exe = dir.join("kora");
        std::fs::write(&exe, b"x").unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!exe_is_service_safe(&exe), "a program under the (world-writable) temp folder must be copied before a service runs it");
        assert!(!exe_is_service_safe(&dir.join("missing")));
    }
}
