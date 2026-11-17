//! Resources installed from a `subpath` of a repository: only that subfolder is copied into place
//! (a plain, pinned copy: it has no `.git`, so it can never be a fork checkout or be pulled).
//! A small marker file records where the copy came from, so a resumed install can recognise it.

use std::path::{Component, Path, PathBuf};

/// Written inside every subpath copy.
pub const MARKER: &str = ".kora-source.json";

fn marker_text(repo: &str, commit: &str, subpath: &str) -> String {
    serde_json::json!({ "repository": repo, "commit": commit, "subpath": subpath }).to_string()
}

/// Is `dest` a finished copy of `commit`?
pub fn marker_matches(dest: &Path, commit: &str) -> bool {
    std::fs::read(dest.join(MARKER))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v.get("commit").and_then(|c| c.as_str()).map(|c| c == commit || (c.len() >= 7 && commit.starts_with(c)) || (commit.len() >= 7 && c.starts_with(commit))))
        .unwrap_or(false)
}

/// A folder that Kora copied out of a repository (marker, no `.git`).
pub fn is_subpath_copy(dest: &Path) -> bool {
    dest.join(MARKER).is_file() && !dest.join(".git").exists()
}

/// `partial/<sub>`, checked to be a real folder inside the clone. A repository can contain
/// symlinks (`a -> /etc`), and `is_dir` / `rename` follow them, so every step is looked at
/// without following links and the final folder must still be inside the clone.
fn checked_subfolder(partial: &Path, sub: &str) -> Result<PathBuf, String> {
    let missing = || format!("the repository has no folder '{sub}' at that version");
    let mut src = partial.to_path_buf();
    for c in Path::new(sub).components() {
        src.push(c);
        let meta = std::fs::symlink_metadata(&src).map_err(|_| missing())?;
        if meta.file_type().is_symlink() {
            return Err(format!("the folder '{sub}' in the repository is a link; it was not installed"));
        }
        if !meta.is_dir() {
            return Err(missing());
        }
    }
    let inside = match (std::fs::canonicalize(partial), std::fs::canonicalize(&src)) {
        (Ok(root), Ok(real)) => real.starts_with(&root) && real != root,
        _ => false,
    };
    if !inside {
        return Err(format!("unsafe subpath '{sub}'"));
    }
    Ok(src)
}

/// Turn a finished clone at `partial` into the final folder `dest`.
/// * No `subpath`: the clone itself is moved to `dest`.
/// * With `subpath`: only `partial/<subpath>` is moved to `dest` (plus the marker), then the clone is deleted.
pub fn place(partial: &Path, dest: &Path, subpath: Option<&str>, repo: &str, commit: Option<&str>) -> Result<(), String> {
    let Some(sub) = subpath else {
        return crate::fsutil::rename_retrying(partial, dest).map_err(|e| format!("cannot move the clone into place: {e}"));
    };
    if sub.is_empty()
        || sub.starts_with('/')
        || sub.contains(['\\', ':'])
        || sub.split('/').any(|s| s == ".." || s.eq_ignore_ascii_case(".git"))
        || !Path::new(sub).components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
    {
        let _ = std::fs::remove_dir_all(partial);
        return Err(format!("unsafe subpath '{sub}'"));
    }
    let src = match checked_subfolder(partial, sub) {
        Ok(src) => src,
        Err(e) => {
            let _ = std::fs::remove_dir_all(partial);
            return Err(e);
        }
    };
    let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let staging = dest.with_file_name(format!("{name}.kora-sub"));
    let _ = std::fs::remove_dir_all(&staging);
    crate::fsutil::rename_retrying(&src, &staging).map_err(|e| format!("cannot take the '{sub}' folder out of the clone: {e}"))?;
    let _ = std::fs::remove_dir_all(partial);
    crate::fsutil::write_no_follow(&staging.join(MARKER), marker_text(repo, commit.unwrap_or(""), sub).as_bytes(), 0o644).map_err(|e| format!("cannot write the source marker: {e}"))?;
    crate::fsutil::rename_retrying(&staging, dest).map_err(|e| format!("cannot move the copy into place: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "0123456789012345678901234567890123456789";

    #[test]
    fn only_the_subfolder_is_kept_and_marked() {
        let d = tempfile::tempdir().unwrap();
        let partial = d.path().join("c.kora-partial");
        std::fs::create_dir_all(partial.join(".git")).unwrap();
        std::fs::create_dir_all(partial.join("ox_lib")).unwrap();
        std::fs::write(partial.join("ox_lib/fxmanifest.lua"), "x").unwrap();
        std::fs::write(partial.join("README.md"), "x").unwrap();
        let dest = d.path().join("[compat]/ox_lib");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        place(&partial, &dest, Some("ox_lib"), "KoraFiveM/compat", Some(SHA)).unwrap();
        assert!(dest.join("fxmanifest.lua").is_file());
        assert!(!dest.join("README.md").exists() && !dest.join(".git").exists());
        assert!(!partial.exists() && !d.path().join("[compat]/ox_lib.kora-sub").exists());
        assert!(marker_matches(&dest, SHA) && is_subpath_copy(&dest));
        assert!(!marker_matches(&dest, "ffffffffffffffffffffffffffffffffffffffff"));
    }

    #[test]
    fn a_whole_clone_is_just_moved_and_bad_subpaths_fail() {
        let d = tempfile::tempdir().unwrap();
        let partial = d.path().join("a.kora-partial");
        std::fs::create_dir_all(partial.join(".git")).unwrap();
        let dest = d.path().join("a");
        place(&partial, &dest, None, "o/a", None).unwrap();
        assert!(dest.join(".git").is_dir() && !is_subpath_copy(&dest));
        let partial = d.path().join("b.kora-partial");
        std::fs::create_dir_all(&partial).unwrap();
        assert!(place(&partial, &d.path().join("b"), Some("../x"), "o/b", None).is_err());
        assert!(place(&partial, &d.path().join("b"), Some("missing"), "o/b", None).unwrap_err().contains("no folder"));
        assert!(!partial.exists(), "a clone that cannot be used is removed");
        for evil in ["C:/x", "a/.GIT", "."] {
            std::fs::create_dir_all(partial.join("a/.GIT")).unwrap();
            assert!(place(&partial, &d.path().join("b"), Some(evil), "o/b", None).is_err(), "{evil}");
        }
    }

    #[test]
    #[cfg(unix)]
    fn a_subpath_through_a_symlink_is_refused_and_nothing_moves() {
        let d = tempfile::tempdir().unwrap();
        let outside = d.path().join("etc");
        std::fs::create_dir_all(outside.join("ssh")).unwrap();
        std::fs::write(outside.join("ssh/sshd_config"), "x").unwrap();
        for (link, sub) in [("a", "a/ssh"), ("ssh", "ssh")] {
            let partial = d.path().join("c.kora-partial");
            std::fs::create_dir_all(&partial).unwrap();
            let target = if link == "a" { outside.clone() } else { outside.join("ssh") };
            std::os::unix::fs::symlink(&target, partial.join(link)).unwrap();
            let dest = d.path().join("res");
            let err = place(&partial, &dest, Some(sub), "o/c", Some(SHA)).unwrap_err();
            assert!(err.contains("link"), "{err}");
            assert!(outside.join("ssh/sshd_config").is_file(), "the real folder stays put");
            assert!(!dest.exists() && !partial.exists());
        }
    }
}
