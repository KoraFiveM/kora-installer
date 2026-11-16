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
/// files are written, and never beneath another link, so nothing can be written through one.
fn rootfs_absolute_link_ok(link: &Path, target: &str) -> bool {
    link.starts_with("alpine")
        && target.starts_with('/')
        && !target.contains(['\\', '\0'])
        && !Path::new(target).components().any(|c| matches!(c, Component::ParentDir))
}

/// A relative symlink target is acceptable only if it stays inside the extraction root. `links`
/// holds every link in the archive: a `..` after passing through one of them would be resolved
/// from wherever that link points (so `a -> b/l/../..` with `b/l -> ..` escapes), so it is refused.
fn link_target_ok(link: &Path, target: &str, links: &HashSet<PathBuf>) -> bool {
    if target.is_empty() || target.contains(['\\', '\0']) || Path::new(target).is_absolute() {
        return false;
    }
    let mut cur = link.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut through_link = false;
    for c in Path::new(target).components() {
        match c {
            Component::Normal(p) => {
                cur.push(p);
                through_link |= links.contains(&cur);
            }
            Component::ParentDir => {
                if through_link || !cur.pop() {
                    return false;
                }
            }
            Component::CurDir => {}
            _ => return false,
        }
    }
    true
}

/// Create `root/rel` and any missing parents, refusing to go through a symlink or a file. Unlike
/// `create_dir_all`, this never follows a link, so it cannot be steered outside `root`.
fn make_dirs(root: &Path, rel: &Path) -> Result<()> {
    let mut p = root.to_path_buf();
    for c in rel.components() {
        p.push(c);
        match fs::symlink_metadata(&p) {
            Ok(m) if m.file_type().is_symlink() => return bad(format!("{} goes through a link", rel.display())),
            Ok(m) if m.is_dir() => {}
            Ok(_) => return bad(format!("{} goes through a file", rel.display())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => fs::create_dir(&p)?,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
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
        make_dirs(self.root, rel)
    }
    fn file(&mut self, rel: &Path, r: &mut dyn Read, mode: Option<u32>) -> Result<()> {
        if let Some(p) = rel.parent() {
            make_dirs(self.root, p)?;
        }
        // `create_new` also refuses an existing symlink at the final component.
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
        // Targets are checked in `finish`, once every link in the archive is known.
        self.links.push((rel.to_path_buf(), target.to_string()));
        Ok(())
    }
    fn finish(self) -> Result<()> {
        #[cfg(not(unix))]
        if !self.links.is_empty() {
            return bad("symlinks are not supported on this platform");
        }
        let all: HashSet<PathBuf> = self.links.iter().map(|(rel, _)| rel.clone()).collect();
        for (rel, target) in &self.links {
            if !link_target_ok(rel, target, &all) && !rootfs_absolute_link_ok(rel, target) {
                return bad(format!("link {} escapes the archive root", rel.display()));
            }
            // Nothing may live beneath a link: it would be written wherever the link points.
            if rel.ancestors().skip(1).any(|a| all.contains(a)) {
                return bad(format!("{} is inside another link", rel.display()));
            }
        }
        #[cfg(unix)]
        for (rel, target) in self.links {
            if let Some(p) = rel.parent() {
                make_dirs(self.root, p)?;
            }
            let path = self.root.join(&rel);
            if fs::symlink_metadata(&path).is_ok() {
                return bad(format!("link {} would replace another entry", rel.display()));
            }
            std::os::unix::fs::symlink(&target, path)?;
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
    let mut r = sevenz_rust2::ArchiveReader::open(path, sevenz_rust2::Password::empty()).map_err(|e| ArtifactError::Archive(e.to_string()))?;
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
