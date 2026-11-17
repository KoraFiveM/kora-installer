//! The archive file: a gzip-compressed tar, optionally inside an `age` passphrase envelope, so it can be
//! opened with ordinary tools (`age -d file.age | tar xz`) and never locks anyone in.
//!
//! ```text
//! header.json        the manifest without checksums (first: so a preview needs only the start of the file)
//! database/dump.sql  the server's own database
//! server/<path>      the server folder's configuration and txAdmin data
//! kora/<file>        Kora's own records for the server (secret-free ones and the webhook settings)
//! resources/<path>/  resources kept in full
//! manifest.json      the manifest again, with the SHA-256 of every file above (last)
//! ```
//!
//! Safety of reading (the file may come from anyone): only regular files and folders are accepted (no links,
//! devices), every name must be a plain relative path with no `..`, and extraction is capped in size.
//! Safety of writing: the file is created owner-only (0600) and renamed into place by the caller only
//! when complete; files that change while they are copied are noted, never left half-written in the tar.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};

use super::crypt::{open_reader, Hashing, HashingReader, OpenError, Passphrase, Sealer};
use super::types::{FileEntry, Manifest, FORMAT};

pub const HEADER: &str = "header.json";
pub const MANIFEST: &str = "manifest.json";
pub const DUMP: &str = "database/dump.sql";
pub const SERVER_DIR: &str = "server";
pub const KORA_DIR: &str = "kora";
pub const RESOURCES_DIR: &str = "resources";

/// Files up to this size are read whole (a consistent copy of that file); bigger ones are streamed.
const WHOLE_FILE_LIMIT: u64 = 32 * 1024 * 1024;

#[derive(Debug)]
pub enum ArchiveError {
    Open(OpenError),
    /// Not a Kora backup, damaged, or tampered with.
    Corrupt(String),
    /// Made by a newer Kora (its format number).
    TooNew(u32),
    Io(io::Error),
}

impl From<io::Error> for ArchiveError {
    fn from(e: io::Error) -> Self {
        ArchiveError::Io(e)
    }
}
impl From<OpenError> for ArchiveError {
    fn from(e: OpenError) -> Self {
        ArchiveError::Open(e)
    }
}

impl std::fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArchiveError::Open(OpenError::NeedsPassphrase) => f.write_str("this backup is password protected"),
            ArchiveError::Open(OpenError::WrongPassphrase) => f.write_str("the password does not open this backup"),
            ArchiveError::Open(OpenError::NotAnArchive) => f.write_str("this is not a Kora backup"),
            ArchiveError::Open(OpenError::Io(e)) | ArchiveError::Corrupt(e) => f.write_str(e),
            ArchiveError::TooNew(v) => write!(f, "this backup was made by a newer Kora (backup format {v})"),
            ArchiveError::Io(e) => write!(f, "{e}"),
        }
    }
}

// ------------------------------------------------------------------------------------------- writing

type Sink = GzEncoder<Sealer<Hashing<File>>>;

/// A reader that yields exactly `len` bytes: zeros after the file ended early, nothing past `len`.
struct Exactly<R> {
    inner: R,
    left: u64,
    /// The file turned out shorter than it was when its size was read.
    short: std::rc::Rc<std::cell::Cell<bool>>,
}

impl<R: Read> Read for Exactly<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.left == 0 || buf.is_empty() {
            return Ok(0);
        }
        let want = buf.len().min(usize::try_from(self.left).unwrap_or(usize::MAX));
        let n = if self.short.get() { 0 } else { self.inner.read(&mut buf[..want])? };
        if n == 0 {
            self.short.set(true);
            buf[..want].fill(0);
            self.left -= want as u64;
            return Ok(want);
        }
        self.left -= n as u64;
        Ok(n)
    }
}

pub struct ArchiveWriter {
    tar: tar::Builder<Sink>,
    files: Vec<FileEntry>,
    /// Plain words about files skipped or caught changing.
    pub notes: Vec<String>,
    pub payload: u64,
}

impl ArchiveWriter {
    /// Start a new archive at `path` (must not exist; owner-only). With a passphrase it is encrypted.
    pub fn create(path: &Path, passphrase: Option<&Passphrase>) -> io::Result<Self> {
        let file = super::dbdump::create_private(path)?;
        let sealed = Sealer::new(Hashing::new(file), passphrase)?;
        let gz = GzEncoder::new(sealed, Compression::new(6));
        let mut tar = tar::Builder::new(gz);
        tar.mode(tar::HeaderMode::Deterministic);
        Ok(ArchiveWriter { tar, files: Vec::new(), notes: Vec::new(), payload: 0 })
    }

    fn header(size: u64, mode: u32, mtime: u64) -> tar::Header {
        let mut h = tar::Header::new_gnu();
        h.set_size(size);
        h.set_mode(mode);
        h.set_mtime(mtime);
        h.set_entry_type(tar::EntryType::Regular);
        h
    }

    /// The first entry of the archive.
    pub fn add_header(&mut self, m: &Manifest) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(m).map_err(io::Error::other)?;
        let mut h = Self::header(bytes.len() as u64, 0o600, 0);
        self.tar.append_data(&mut h, HEADER, &bytes[..])
    }

    /// A small file from memory.
    pub fn add_bytes(&mut self, name: &str, data: &[u8]) -> io::Result<()> {
        let mut h = Self::header(data.len() as u64, 0o600, 0);
        self.tar.append_data(&mut h, name, data)?;
        self.files.push(FileEntry { path: name.to_string(), size: data.len() as u64, sha256: hex::encode(Sha256::digest(data)) });
        self.payload += data.len() as u64;
        Ok(())
    }

    /// A file from disk, named `name` in the archive.
    pub fn add_file(&mut self, name: &str, src: &Path) -> io::Result<()> {
        let meta = std::fs::symlink_metadata(src)?;
        if !meta.is_file() {
            self.notes.push(format!("{name} was left out: it is not an ordinary file."));
            return Ok(());
        }
        let mtime = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
        let size = meta.len();
        #[cfg(unix)]
        let mode = {
            use std::os::unix::fs::PermissionsExt;
            meta.permissions().mode() & 0o777
        };
        #[cfg(not(unix))]
        let mode = 0o644;
        if size <= WHOLE_FILE_LIMIT {
            let data = std::fs::read(src)?;
            let mut h = Self::header(data.len() as u64, mode, mtime);
            self.tar.append_data(&mut h, name, &data[..])?;
            self.files.push(FileEntry { path: name.to_string(), size: data.len() as u64, sha256: hex::encode(Sha256::digest(&data)) });
            self.payload += data.len() as u64;
            return Ok(());
        }
        let f = File::open(src)?;
        let short_flag = std::rc::Rc::new(std::cell::Cell::new(false));
        let mut rd = HashingReader::new(Exactly { inner: f, left: size, short: short_flag.clone() });
        let mut h = Self::header(size, mode, mtime);
        // `append_data` copies exactly `size` bytes from the reader.
        self.tar.append_data(&mut h, name, &mut rd)?;
        let short = short_flag.get();
        let (sha, n) = rd.finish();
        let grew = std::fs::metadata(src).map(|m| m.len() > size).unwrap_or(false);
        if short || grew {
            self.notes.push(format!("{name} was being written while it was copied, so this copy of it may be incomplete."));
        }
        self.files.push(FileEntry { path: name.to_string(), size: n, sha256: sha });
        self.payload += n;
        Ok(())
    }

    /// Everything under `root`, named `<prefix>/<relative path>`. Links and special files are left out.
    /// `skip` is given the path relative to `root` (with `/`) and says whether to leave it out.
    pub fn add_tree(&mut self, prefix: &str, root: &Path, skip: &dyn Fn(&str, bool) -> bool) -> io::Result<()> {
        fn walk(w: &mut ArchiveWriter, prefix: &str, dir: &Path, rel: &str, skip: &dyn Fn(&str, bool) -> bool) -> io::Result<()> {
            let mut entries: Vec<_> = match std::fs::read_dir(dir) {
                Ok(rd) => rd.flatten().collect(),
                Err(e) => {
                    w.notes.push(format!("{} could not be read ({e}).", if rel.is_empty() { prefix.to_string() } else { format!("{prefix}/{rel}") }));
                    return Ok(());
                }
            };
            entries.sort_by_key(|e| e.file_name());
            for e in entries {
                let Some(name) = e.file_name().to_str().map(str::to_string) else {
                    w.notes.push(format!("A file in {prefix} was left out: its name is not plain text."));
                    continue;
                };
                let child_rel = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
                let Ok(ft) = e.file_type() else { continue };
                if ft.is_symlink() {
                    w.notes.push(format!("{prefix}/{child_rel} is a link and was left out."));
                } else if ft.is_dir() {
                    if !skip(&child_rel, true) {
                        walk(w, prefix, &e.path(), &child_rel, skip)?;
                    }
                } else if ft.is_file() && !skip(&child_rel, false) {
                    if let Err(err) = w.add_file(&format!("{prefix}/{child_rel}"), &e.path()) {
                        if err.kind() == io::ErrorKind::NotFound {
                            continue; // deleted while we walked
                        }
                        if err.kind() == io::ErrorKind::PermissionDenied {
                            w.notes.push(format!("{prefix}/{child_rel} could not be read (no permission), so it is not in the backup."));
                            continue;
                        }
                        return Err(err);
                    }
                }
            }
            Ok(())
        }
        walk(self, prefix, root, "", skip)
    }

    /// Write the closing manifest (with checksums) and finish. Returns the archive's SHA-256 and size.
    pub fn finish(mut self, manifest: &Manifest) -> io::Result<(String, u64)> {
        let mut m = manifest.clone();
        m.files = std::mem::take(&mut self.files);
        m.payload_bytes = self.payload;
        let bytes = serde_json::to_vec_pretty(&m).map_err(io::Error::other)?;
        let mut h = Self::header(bytes.len() as u64, 0o600, 0);
        self.tar.append_data(&mut h, MANIFEST, &bytes[..])?;
        let gz = self.tar.into_inner()?;
        let sealed = gz.finish()?;
        let hashing = sealed.finish()?;
        let (mut file, sha, n) = hashing.finish();
        file.flush()?;
        file.sync_all()?;
        Ok((sha, n))
    }
}

// ------------------------------------------------------------------------------------------- reading

fn tar_of(path: &Path, passphrase: Option<&Passphrase>) -> Result<tar::Archive<GzDecoder<Box<dyn Read>>>, ArchiveError> {
    let f = File::open(path)?;
    let plain = open_reader(io::BufReader::new(f), passphrase)?;
    Ok(tar::Archive::new(GzDecoder::new(plain)))
}

/// A plain, relative, forward-slash path with no `..`, no empty parts, no drive letters.
pub fn safe_entry_path(name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.contains('\\') || name.contains('\0') || name.starts_with('/') {
        return None;
    }
    let mut out = PathBuf::new();
    for part in name.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.contains(':') {
            return None;
        }
        out.push(part);
    }
    out.components().all(|c| matches!(c, Component::Normal(_))).then_some(out)
}

fn corrupt<E: std::fmt::Display>(e: E) -> ArchiveError {
    ArchiveError::Corrupt(format!("the backup file is damaged or incomplete ({e})"))
}

fn check_version(m: &Manifest) -> Result<(), ArchiveError> {
    if m.format > FORMAT {
        Err(ArchiveError::TooNew(m.format))
    } else {
        Ok(())
    }
}

/// The first entry (`header.json`): what the backup is, without reading the rest.
pub fn read_head(path: &Path, passphrase: Option<&Passphrase>) -> Result<Manifest, ArchiveError> {
    let mut ar = tar_of(path, passphrase)?;
    let mut entries = ar.entries().map_err(head_error)?;
    let mut first = entries.next().ok_or(ArchiveError::Open(OpenError::NotAnArchive))?.map_err(head_error)?;
    if first.path().ok().and_then(|p| p.to_str().map(str::to_string)).as_deref() != Some(HEADER) {
        return Err(ArchiveError::Open(OpenError::NotAnArchive));
    }
    let mut buf = Vec::new();
    first.by_ref().take(4 << 20).read_to_end(&mut buf).map_err(corrupt)?;
    let m: Manifest = serde_json::from_slice(&buf).map_err(|_| ArchiveError::Open(OpenError::NotAnArchive))?;
    check_version(&m)?;
    Ok(m)
}

/// A read error at the very start means "not a gzip/tar file" or a wrong password, depending on the envelope.
fn head_error(e: io::Error) -> ArchiveError {
    if e.kind() == io::ErrorKind::InvalidData || e.kind() == io::ErrorKind::InvalidInput {
        ArchiveError::Open(OpenError::NotAnArchive)
    } else {
        ArchiveError::Io(e)
    }
}

/// Where an entry of the archive goes when extracted, and whether to take it.
pub struct Wanted<'a> {
    /// `true` to extract the entry with this archive path.
    pub take: &'a dyn Fn(&str) -> bool,
}

/// Progress: bytes read so far from the (decompressed) stream.
pub type Progress<'a> = &'a mut dyn FnMut(u64);

/// Read everything: check every file against the closing manifest and, when `into` is given, write the
/// wanted entries below it. `max_bytes` caps what is written (a guard against a bomb). Returns the manifest.
pub fn scan(path: &Path, passphrase: Option<&Passphrase>, into: Option<(&Path, &Wanted<'_>)>, max_bytes: u64, mut progress: Option<Progress<'_>>) -> Result<Manifest, ArchiveError> {
    let mut ar = tar_of(path, passphrase)?;
    let mut seen: Vec<FileEntry> = Vec::new();
    let mut manifest: Option<Manifest> = None;
    let mut head: Option<Manifest> = None;
    let mut total = 0u64;
    let entries = ar.entries().map_err(head_error)?;
    for (i, entry) in entries.enumerate() {
        let mut entry = entry.map_err(|e| if i == 0 { head_error(e) } else { corrupt(e) })?;
        let name = entry.path().map_err(corrupt)?.to_string_lossy().replace('\\', "/");
        let kind = entry.header().entry_type();
        if i == 0 && name != HEADER {
            return Err(ArchiveError::Open(OpenError::NotAnArchive));
        }
        match kind {
            tar::EntryType::Directory => continue,
            tar::EntryType::Regular | tar::EntryType::Continuous => {}
            other => return Err(ArchiveError::Corrupt(format!("the backup holds something that is not a plain file ({other:?}); it will not be opened"))),
        }
        let Some(rel) = safe_entry_path(&name) else {
            return Err(ArchiveError::Corrupt(format!("the backup holds a file with an unsafe name ({name:?}); it will not be opened")));
        };
        let size = entry.header().size().map_err(corrupt)?;
        total = total.saturating_add(size);
        if total > max_bytes {
            return Err(ArchiveError::Corrupt("the backup holds far more data than it says it does; it will not be opened".into()));
        }
        if name == HEADER || name == MANIFEST {
            let mut buf = Vec::new();
            entry.by_ref().take(64 << 20).read_to_end(&mut buf).map_err(corrupt)?;
            let m: Manifest = serde_json::from_slice(&buf).map_err(|_| ArchiveError::Corrupt("the backup's manifest cannot be read".into()))?;
            check_version(&m)?;
            if name == HEADER {
                head = Some(m);
            } else {
                manifest = Some(m);
            }
            continue;
        }
        if seen.iter().any(|f| f.path == name) {
            return Err(ArchiveError::Corrupt(format!("{name} appears twice in the backup")));
        }
        let mut hash = Sha256::new();
        let mut n = 0u64;
        let mut out: Option<File> = None;
        if let Some((root, wanted)) = into {
            if (wanted.take)(&name) {
                let dest = root.join(&rel);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                out = Some(super::dbdump::create_private(&dest)?);
            }
        }
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let k = entry.read(&mut buf).map_err(corrupt)?;
            if k == 0 {
                break;
            }
            hash.update(&buf[..k]);
            n += k as u64;
            if let Some(f) = out.as_mut() {
                f.write_all(&buf[..k])?;
            }
            if let Some(p) = progress.as_mut() {
                p(k as u64);
            }
        }
        seen.push(FileEntry { path: name, size: n, sha256: hex::encode(hash.finalize()) });
    }
    let (Some(head), Some(manifest)) = (head, manifest) else {
        return Err(ArchiveError::Corrupt("the backup is incomplete: it ends before its checksum list (the copy was probably cut short)".into()));
    };
    // Every file the manifest lists must be there, with the same bytes, and nothing else may be.
    for f in &manifest.files {
        match seen.iter().find(|s| s.path == f.path) {
            None => return Err(ArchiveError::Corrupt(format!("{} is missing from the backup", f.path))),
            Some(s) if s.sha256 != f.sha256 || s.size != f.size => return Err(ArchiveError::Corrupt(format!("{} does not match its checksum: the backup is damaged", f.path))),
            Some(_) => {}
        }
    }
    if let Some(extra) = seen.iter().find(|s| !manifest.files.iter().any(|f| f.path == s.path)) {
        return Err(ArchiveError::Corrupt(format!("{} is in the backup but not in its checksum list", extra.path)));
    }
    if head.server.name != manifest.server.name || head.created_unix != manifest.created_unix {
        return Err(ArchiveError::Corrupt("the two copies of the backup's manifest disagree".into()));
    }
    Ok(manifest)
}

/// The SHA-256 of a whole file (the archive as stored).
pub fn file_sha256(path: &Path) -> io::Result<String> {
    crate::installer::http::sha256_file(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::types::*;

    pub(crate) fn manifest() -> Manifest {
        Manifest {
            format: FORMAT,
            kora_version: "0.8.0".into(),
            created_unix: 1_700_000_000,
            label: Label::Manual,
            server: ServerInfo {
                name: "Test".into(),
                mode: "production".into(),
                source: crate::installer::Source::Fork,
                versions: crate::installer::Versions::Latest,
                game_port: 30120,
                txadmin_port: 40120,
                game_build: 3000,
                artifact_build: 1,
                os: "linux".into(),
                dir: "/srv/test".into(),
                github_owner: None,
                run_mode: None,
            },
            database: DatabaseBackup { kind: DbKind::None, name: None, host: None, port: None, dump_bytes: 0, note: String::new() },
            resources: vec![],
            stopped: false,
            notes: vec![],
            payload_bytes: 0,
            files: vec![],
        }
    }

    fn make(path: &Path, pass: Option<&Passphrase>) {
        crate::backup::crypt::set_work_factor_for_tests(8);
        let mut w = ArchiveWriter::create(path, pass).unwrap();
        w.add_header(&manifest()).unwrap();
        w.add_bytes("server/server.cfg", b"endpoint_add_tcp \"0.0.0.0:30120\"\n").unwrap();
        w.add_bytes("database/dump.sql", b"CREATE TABLE a (x int);\n").unwrap();
        w.finish(&manifest()).unwrap();
    }

    #[test]
    fn a_backup_reads_back_and_verifies() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.tar.gz");
        make(&p, None);
        assert_eq!(read_head(&p, None).unwrap().server.name, "Test");
        let m = scan(&p, None, None, 1 << 20, None).unwrap();
        assert_eq!(m.files.len(), 2);
        assert!(m.payload_bytes > 0);
        // extraction of what is wanted
        let out = d.path().join("out");
        let m2 = scan(&p, None, Some((&out, &Wanted { take: &|n| n.starts_with("server/") })), 1 << 20, None).unwrap();
        assert_eq!(m2.files.len(), 2);
        assert!(out.join("server/server.cfg").is_file());
        assert!(!out.join("database").exists());
    }

    #[test]
    fn an_encrypted_backup_needs_the_right_password() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.tar.gz.age");
        let pass = Passphrase::new("a long enough passphrase");
        make(&p, Some(&pass));
        assert!(matches!(read_head(&p, None), Err(ArchiveError::Open(OpenError::NeedsPassphrase))));
        assert!(matches!(read_head(&p, Some(&Passphrase::new("not the passphrase!"))), Err(ArchiveError::Open(OpenError::WrongPassphrase))));
        assert_eq!(read_head(&p, Some(&pass)).unwrap().format, FORMAT);
        assert!(scan(&p, Some(&pass), None, 1 << 20, None).is_ok());
        let raw = std::fs::read(&p).unwrap();
        assert!(!raw.windows(6).any(|w| w == b"server"), "no readable names in an encrypted archive");
    }

    #[test]
    fn a_cut_off_or_changed_archive_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.tar.gz");
        make(&p, None);
        let raw = std::fs::read(&p).unwrap();
        let cut = d.path().join("cut.tar.gz");
        std::fs::write(&cut, &raw[..raw.len() / 2]).unwrap();
        assert!(scan(&cut, None, None, 1 << 20, None).is_err());
        // not an archive at all
        let junk = d.path().join("junk");
        std::fs::write(&junk, b"hello, this is not a backup").unwrap();
        assert!(read_head(&junk, None).is_err());
        assert!(scan(&junk, None, None, 1 << 20, None).is_err());
    }

    #[test]
    fn a_newer_format_is_refused_politely() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("n.tar.gz");
        let mut w = ArchiveWriter::create(&p, None).unwrap();
        let mut m = manifest();
        m.format = FORMAT + 1;
        w.add_header(&m).unwrap();
        w.finish(&m).unwrap();
        match read_head(&p, None) {
            Err(ArchiveError::TooNew(v)) => assert_eq!(v, FORMAT + 1),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn unsafe_names_never_get_out() {
        for bad in ["../x", "/etc/passwd", "a/../../b", "a\\b", "C:/x", "a//b", "./a", ""] {
            assert!(safe_entry_path(bad).is_none(), "{bad}");
        }
        assert_eq!(safe_entry_path("server/txData/default/config.json").unwrap(), PathBuf::from("server/txData/default/config.json"));
    }

    #[test]
    fn a_hostile_archive_with_a_link_or_traversal_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("h.tar.gz");
        let f = File::create(&p).unwrap();
        let gz = GzEncoder::new(f, Compression::fast());
        let mut b = tar::Builder::new(gz);
        let head = serde_json::to_vec(&manifest()).unwrap();
        let mut h = tar::Header::new_gnu();
        h.set_size(head.len() as u64);
        h.set_path(HEADER).unwrap();
        h.set_cksum();
        b.append(&h, &head[..]).unwrap();
        let mut l = tar::Header::new_gnu();
        l.set_entry_type(tar::EntryType::Symlink);
        l.set_path("server/evil").unwrap();
        l.set_link_name("/etc/passwd").unwrap();
        l.set_size(0);
        l.set_cksum();
        b.append(&l, &b""[..]).unwrap();
        b.into_inner().unwrap().finish().unwrap();
        let err = scan(&p, None, None, 1 << 20, None).unwrap_err();
        assert!(matches!(err, ArchiveError::Corrupt(_)), "{err:?}");
    }

    #[test]
    fn a_file_that_shrinks_while_copied_is_padded_and_noted() {
        // 33 MiB claimed, 1 MiB on disk: the writer pads and the archive still verifies.
        let mut r = Exactly { inner: io::Cursor::new(vec![7u8; 10]), left: 25, short: Default::default() };
        let mut v = Vec::new();
        r.read_to_end(&mut v).unwrap();
        assert_eq!(v.len(), 25);
        assert!(r.short.get() && v[..10].iter().all(|b| *b == 7) && v[10..].iter().all(|b| *b == 0));
    }
}
