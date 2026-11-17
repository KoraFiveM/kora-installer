//! Unpack a downloaded tool archive safely: only plain files and folders, only inside the target folder
//! (no `..`, no absolute paths, no drive letters, no backslashes, no links), never overwriting, and a
//! size limit so a decompression bomb stops early.

use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use super::pins::Format;

/// Most bytes one archive may unpack to (gh is ~45 MB, MinGit ~250 MB).
const LIMIT: u64 = 1 << 30;
/// Most entries one archive may hold.
const MAX_ENTRIES: usize = 20_000;

/// `top/a/b` → `a/b` (when `strip_top`), `a/b` → `a/b`. `None` for the top folder itself or an empty name.
/// Errors on anything that could leave the target folder.
pub fn safe_relative(name: &str, strip_top: bool) -> Result<Option<PathBuf>, String> {
    if name.contains('\\') || name.contains(':') || name.contains('\0') || name.starts_with('/') {
        return Err(format!("unsafe path in the download: {name}"));
    }
    let mut parts = name.split('/').filter(|c| !c.is_empty() && *c != ".");
    if strip_top {
        let _ = parts.next();
    }
    let rel: PathBuf = parts.collect();
    if rel.as_os_str().is_empty() {
        return Ok(None);
    }
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(format!("unsafe path in the download: {name}"));
    }
    Ok(Some(rel))
}

fn io(what: &str, e: impl std::fmt::Display) -> String {
    format!("{what}: {e}")
}

fn write_file(out: &Path, src: &mut dyn Read, total: &mut u64, exec: bool) -> Result<(), String> {
    if let Some(p) = out.parent() {
        std::fs::create_dir_all(p).map_err(|e| io("cannot create a folder", e))?;
    }
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(out).map_err(|e| io(&format!("cannot write {}", out.display()), e))?;
    let left = LIMIT.saturating_sub(*total);
    let n = std::io::copy(&mut src.take(left + 1), &mut f).map_err(|e| io("cannot unpack", e))?;
    *total += n;
    f.flush().map_err(|e| io("cannot unpack", e))?;
    if n > left {
        return Err("the download unpacks to an unreasonable size".into());
    }
    #[cfg(unix)]
    if exec {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(out, std::fs::Permissions::from_mode(0o755)).map_err(|e| io("cannot make the program runnable", e))?;
    }
    #[cfg(not(unix))]
    let _ = exec;
    Ok(())
}

/// Unpack `archive` into the (new, empty or missing) folder `dest`.
pub fn unpack(archive: &Path, format: Format, dest: &Path, strip_top: bool) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| io("cannot create a folder", e))?;
    match format {
        Format::Zip => unpack_zip(archive, dest, strip_top),
        Format::TarGz => unpack_tar_gz(archive, dest, strip_top),
    }
}

fn unpack_zip(archive: &Path, dest: &Path, strip_top: bool) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| io("cannot open the download", e))?;
    let mut z = zip::ZipArchive::new(std::io::BufReader::new(file)).map_err(|e| format!("the download is not a valid zip file: {e}"))?;
    if z.len() > MAX_ENTRIES {
        return Err("the download holds an unreasonable number of files".into());
    }
    let mut total = 0u64;
    for i in 0..z.len() {
        let mut f = z.by_index(i).map_err(|e| format!("bad entry in the download: {e}"))?;
        let name = f.name().map_err(|e| format!("bad entry name in the download: {e}"))?.into_owned();
        if f.is_symlink() {
            return Err(format!("the download holds a link ({name}); it was not installed"));
        }
        let Some(rel) = safe_relative(&name, strip_top)? else { continue };
        let out = dest.join(&rel);
        if f.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| io("cannot create a folder", e))?;
            continue;
        }
        write_file(&out, &mut f, &mut total, false)?;
    }
    Ok(())
}

fn unpack_tar_gz(archive: &Path, dest: &Path, strip_top: bool) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| io("cannot open the download", e))?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(std::io::BufReader::new(file)));
    let mut total = 0u64;
    let mut count = 0usize;
    for entry in tar.entries().map_err(|e| format!("the download is not a valid archive: {e}"))? {
        let mut entry = entry.map_err(|e| format!("bad entry in the download: {e}"))?;
        count += 1;
        if count > MAX_ENTRIES {
            return Err("the download holds an unreasonable number of files".into());
        }
        let kind = entry.header().entry_type();
        let name = entry.path().map_err(|e| format!("bad entry name in the download: {e}"))?.to_string_lossy().into_owned();
        // Extended-attribute headers and the like carry no files.
        if kind.is_pax_global_extensions() || kind.is_pax_local_extensions() || kind.is_gnu_longname() || kind.is_gnu_longlink() {
            continue;
        }
        let Some(rel) = safe_relative(&name, strip_top)? else { continue };
        let out = dest.join(&rel);
        if kind.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| io("cannot create a folder", e))?;
        } else if kind.is_file() {
            let exec = entry.header().mode().map(|m| m & 0o111 != 0).unwrap_or(false);
            write_file(&out, &mut entry, &mut total, exec)?;
        } else {
            return Err(format!("the download holds something that is not a plain file ({name}); it was not installed"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_made_relative_and_unsafe_ones_refused() {
        assert_eq!(safe_relative("top/bin/gh", true).unwrap(), Some(PathBuf::from("bin/gh")));
        assert_eq!(safe_relative("bin/gh.exe", false).unwrap(), Some(PathBuf::from("bin/gh.exe")));
        assert_eq!(safe_relative("top/", true).unwrap(), None);
        assert_eq!(safe_relative("./a/./b", false).unwrap(), Some(PathBuf::from("a/b")));
        for bad in ["../x", "a/../../x", "/etc/passwd", "C:/x", "a\\b", "top/../../x", "a/b\0c"] {
            assert!(safe_relative(bad, bad.starts_with("top")).is_err(), "{bad} must be refused");
        }
    }

    fn make_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (n, b) in entries {
            z.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(b).unwrap();
        }
        z.finish().unwrap();
    }

    #[test]
    fn a_zip_unpacks_with_its_folders() {
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("a.zip");
        make_zip(&a, &[("cmd/git.exe", b"x"), ("etc/gitconfig", b"y")]);
        let out = t.path().join("out");
        unpack(&a, Format::Zip, &out, false).unwrap();
        assert_eq!(std::fs::read(out.join("cmd/git.exe")).unwrap(), b"x");
        assert!(out.join("etc/gitconfig").is_file());
    }

    #[test]
    fn a_zip_entry_that_climbs_out_is_refused_and_nothing_outside_is_written() {
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("a.zip");
        // `start_file` keeps the name as given.
        make_zip(&a, &[("ok.txt", b"1"), ("../evil.txt", b"2")]);
        let out = t.path().join("out");
        assert!(unpack(&a, Format::Zip, &out, false).is_err());
        assert!(!t.path().join("evil.txt").exists());
    }

    fn make_tgz(path: &Path, entries: &[(&str, &[u8], u32)]) {
        let gz = flate2::write::GzEncoder::new(std::fs::File::create(path).unwrap(), flate2::Compression::fast());
        let mut b = tar::Builder::new(gz);
        for (n, data, mode) in entries {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(*mode);
            h.set_entry_type(tar::EntryType::Regular);
            // `append_data` would refuse `..`; write the raw name like a hostile archive would.
            h.as_gnu_mut().unwrap().name[..n.len()].copy_from_slice(n.as_bytes());
            h.set_cksum();
            b.append(&h, *data).unwrap();
        }
        b.into_inner().unwrap().finish().unwrap();
    }

    #[test]
    fn a_tar_gz_unpacks_without_its_top_folder_and_keeps_the_run_bit() {
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("a.tgz");
        make_tgz(&a, &[("gh_1/bin/gh", b"prog", 0o755), ("gh_1/LICENSE", b"l", 0o644)]);
        let out = t.path().join("out");
        unpack(&a, Format::TarGz, &out, true).unwrap();
        assert_eq!(std::fs::read(out.join("bin/gh")).unwrap(), b"prog");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(out.join("bin/gh")).unwrap().permissions().mode() & 0o111, 0o111);
            assert_eq!(std::fs::metadata(out.join("LICENSE")).unwrap().permissions().mode() & 0o111, 0);
        }
    }

    #[test]
    fn a_tar_entry_that_climbs_out_or_is_a_link_is_refused() {
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("a.tgz");
        make_tgz(&a, &[("top/../../evil", b"2", 0o644)]);
        assert!(unpack(&a, Format::TarGz, &t.path().join("out"), true).is_err());
        assert!(!t.path().join("evil").exists());

        let gz = flate2::write::GzEncoder::new(std::fs::File::create(&a).unwrap(), flate2::Compression::fast());
        let mut b = tar::Builder::new(gz);
        let mut h = tar::Header::new_gnu();
        h.set_entry_type(tar::EntryType::Symlink);
        h.set_size(0);
        h.set_link_name("/etc/passwd").unwrap();
        h.set_cksum();
        b.append_data(&mut h, "top/link", std::io::empty()).unwrap();
        b.into_inner().unwrap().finish().unwrap();
        let e = unpack(&a, Format::TarGz, &t.path().join("out2"), true).unwrap_err();
        assert!(e.contains("not a plain file"), "{e}");
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("a.bin");
        std::fs::write(&a, b"not an archive").unwrap();
        assert!(unpack(&a, Format::Zip, &t.path().join("o1"), false).is_err());
        assert!(unpack(&a, Format::TarGz, &t.path().join("o2"), false).is_err());
    }
}
