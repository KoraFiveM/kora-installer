//! Safe archive extraction (zip / 7z / tar.xz) into a fresh, empty directory.

use super::{ArtifactError, Platform, Result};
use std::collections::HashSet;
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy)]
pub struct ExtractLimits {
    pub max_total: u64,
    pub max_entries: usize,
}

impl Default for ExtractLimits {
    fn default() -> Self {
        ExtractLimits { max_total: 8 << 30, max_entries: 200_000 }
    }
}

fn bad<T>(m: impl Into<String>) -> Result<T> {
    Err(ArtifactError::Archive(m.into()))
}

/// Validate an archive entry name into a safe relative path (`None` = skip, e.g. `./`).
fn clean_name(name: &str) -> Result<Option<PathBuf>> {
    let name = name.trim_end_matches('/');
    if name.is_empty() || name == "." {
        return Ok(None);
    }
    if name.contains(['\\', '\0', ':']) {
        return bad(format!("unsafe path {name:?}"));
    }
    let mut out = PathBuf::new();
    for c in Path::new(name).components() {
        match c {
            Component::Normal(p) => out.push(p),
            Component::CurDir => {}
            _ => return bad(format!("unsafe path {name:?}")),
        }
    }
    Ok(if out.as_os_str().is_empty() { None } else { Some(out) })
}

/// The Linux FXServer archive ships an Alpine root filesystem under `alpine/` whose symlinks are
/// absolute *inside that rootfs* (e.g. `alpine/bin/ls -> /bin/busybox`), resolved by proot at
/// runtime. Accept absolute targets only there, and never with `..`. Links are created after all
/// files are written, so nothing can be extracted through one.
fn rootfs_absolute_link_ok(link: &Path, target: &str) -> bool {
    link.starts_with("alpine")
        && target.starts_with('/')
        && !target.contains(['\\', '\0'])
        && !Path::new(target).components().any(|c| matches!(c, Component::ParentDir))
}

/// A relative symlink target is acceptable only if it stays inside the extraction root.
fn link_target_ok(link: &Path, target: &str) -> bool {
    if target.is_empty() || target.contains(['\\', '\0']) || Path::new(target).is_absolute() {
        return false;
    }
    let mut depth = link.parent().map(|p| p.components().count() as i64).unwrap_or(0);
    for c in Path::new(target).components() {
        match c {
            Component::Normal(_) => depth += 1,
            Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            Component::CurDir => {}
            _ => return false,
        }
    }
    true
}

struct Ctx<'a> {
    root: &'a Path,
    limits: ExtractLimits,
    seen: HashSet<String>,
    total: u64,
    links: Vec<(PathBuf, String)>,
}

impl Ctx<'_> {
    fn claim(&mut self, rel: &Path) -> Result<()> {
        if self.seen.len() >= self.limits.max_entries {
            return bad("too many entries");
        }
        if !self.seen.insert(rel.to_string_lossy().to_lowercase()) {
            return bad(format!("duplicate entry {}", rel.display()));
        }
        Ok(())
    }
    fn dir(&mut self, rel: &Path) -> Result<()> {
        fs::create_dir_all(self.root.join(rel))?;
        Ok(())
    }
    fn file(&mut self, rel: &Path, r: &mut dyn Read, mode: Option<u32>) -> Result<()> {
        if let Some(p) = rel.parent() {
            fs::create_dir_all(self.root.join(p))?;
        }
        let path = self.root.join(rel);
        let mut f = fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
        let left = self.limits.max_total - self.total;
        let n = io::copy(&mut r.take(left + 1), &mut f)?;
        if n > left {
            return bad("archive exceeds extraction limit");
        }
        self.total += n;
        drop(f);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let m = mode.unwrap_or(0o644) & 0o755 | 0o600;
            fs::set_permissions(&path, fs::Permissions::from_mode(m))?;
        }
        #[cfg(not(unix))]
        let _ = mode;
        Ok(())
    }
    fn link(&mut self, rel: &Path, target: &str) -> Result<()> {
        if !link_target_ok(rel, target) && !rootfs_absolute_link_ok(rel, target) {
            return bad(format!("link {} escapes the archive root", rel.display()));
        }
        self.links.push((rel.to_path_buf(), target.to_string()));
        Ok(())
    }
    fn finish(self) -> Result<()> {
        for (rel, target) in self.links {
            if let Some(p) = rel.parent() {
                fs::create_dir_all(self.root.join(p))?;
            }
            #[cfg(unix)]
            std::os::unix::fs::symlink(&target, self.root.join(&rel))?;
            #[cfg(not(unix))]
            {
                let _ = target;
                return bad("symlinks are not supported on this platform");
            }
        }
        Ok(())
    }
}

/// Extract `archive` into `dest`, which must exist and be empty. Format is detected by magic
/// bytes. Rejects absolute/`..`/backslash paths, duplicates (case-insensitive), device files,
/// links leaving the root, and oversized output. Verifies the platform executable exists.
pub fn extract_archive(archive: &Path, dest: &Path, platform: Platform, limits: ExtractLimits) -> Result<()> {
    if fs::read_dir(dest)?.next().is_some() {
        return bad("destination is not empty");
    }
    let mut magic = [0u8; 6];
    let n = fs::File::open(archive)?.read(&mut magic)?;
    let magic = &magic[..n];
    let mut ctx = Ctx { root: dest, limits, seen: HashSet::new(), total: 0, links: vec![] };
    if magic.starts_with(&[0xFD, b'7', b'z', b'X', b'Z', 0]) {
        tar_xz(archive, &mut ctx)?;
    } else if magic.starts_with(b"7z\xbc\xaf\x27\x1c") {
        seven(archive, &mut ctx)?;
    } else if magic.starts_with(b"PK") {
        zip_(archive, &mut ctx)?;
    } else {
        return bad("unrecognised archive format");
    }
    ctx.finish()?;
    if !dest.join(platform.executable()).is_file() {
        return bad(format!("archive is missing {}", platform.executable()));
    }
    Ok(())
}

fn tar_xz(path: &Path, ctx: &mut Ctx) -> Result<()> {
    let mut ar = tar::Archive::new(xz2::read::XzDecoder::new(io::BufReader::new(fs::File::open(path)?)));
    for e in ar.entries()? {
        let mut e = e?;
        let raw = e.path_bytes().into_owned();
        let name = std::str::from_utf8(&raw).map_err(|_| ArtifactError::Archive("non-UTF-8 path".into()))?.to_string();
        let Some(rel) = clean_name(&name)? else { continue };
        let ty = e.header().entry_type();
        let mode = e.header().mode().ok();
        if ty.is_dir() {
            ctx.claim(&rel)?;
            ctx.dir(&rel)?;
        } else if ty.is_file() {
            ctx.claim(&rel)?;
            ctx.file(&rel, &mut e, mode)?;
        } else if ty.is_symlink() {
            ctx.claim(&rel)?;
            let t = e.link_name_bytes().map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
            ctx.link(&rel, &t)?;
        } else if ty.is_pax_global_extensions() || ty.is_pax_local_extensions() || ty.is_gnu_longname() || ty.is_gnu_longlink() {
            continue;
        } else {
            return bad(format!("unsupported entry type for {name:?}"));
        }
    }
    Ok(())
}

fn zip_(path: &Path, ctx: &mut Ctx) -> Result<()> {
    let mut z = zip::ZipArchive::new(io::BufReader::new(fs::File::open(path)?)).map_err(|e| ArtifactError::Archive(e.to_string()))?;
    for i in 0..z.len() {
        let mut f = z.by_index(i).map_err(|e| ArtifactError::Archive(e.to_string()))?;
        let zname = f.name().map_err(|e| ArtifactError::Archive(e.to_string()))?.into_owned();
        let Some(rel) = clean_name(&zname)? else { continue };
        if f.is_symlink() {
            return bad("symlinks in zip archives are not supported");
        }
        ctx.claim(&rel)?;
        if f.is_dir() {
            ctx.dir(&rel)?;
        } else {
            let mode = f.unix_mode();
            ctx.file(&rel, &mut f, mode)?;
        }
    }
    Ok(())
}

fn seven(path: &Path, ctx: &mut Ctx) -> Result<()> {
    let mut r = sevenz_rust::SevenZReader::open(path, sevenz_rust::Password::empty()).map_err(|e| ArtifactError::Archive(e.to_string()))?;
    let mut failure: Option<ArtifactError> = None;
    let res = r.for_each_entries(|entry, reader| {
        let step = (|| -> Result<()> {
            let Some(rel) = clean_name(entry.name())? else { return Ok(()) };
            ctx.claim(&rel)?;
            if entry.is_directory() {
                ctx.dir(&rel)
            } else if entry.is_anti_item() {
                bad("anti items are not supported")
            } else {
                ctx.file(&rel, reader, None)
            }
        })();
        match step {
            Ok(()) => Ok(true),
            Err(e) => {
                failure = Some(e);
                Ok(false)
            }
        }
    });
    if let Some(e) = failure {
        return Err(e);
    }
    res.map_err(|e| ArtifactError::Archive(e.to_string()))
}
