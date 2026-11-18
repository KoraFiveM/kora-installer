//! Where backups go, and how many are kept.
//!
//! [`Destination`] is deliberately small so a remote one (S3-compatible storage, SFTP) can be added later
//! without touching the code that makes or restores backups: an archive is always BUILT in
//! [`Destination::staging_dir`] (a local folder; for a folder destination that is the folder itself) and
//! then handed over with [`Destination::commit`]. Only a folder is built now.
//!
//! The default folder is Kora's own backups folder in the Kora home, which is outside every server
//! folder, so deleting a server does not delete its backups. It is on the same disk as the Kora home,
//! so Kora says plainly that a copy on another disk (or a share) is the one that survives a dead disk.

use std::io;
use std::path::{Path, PathBuf};

use super::types::{BackupInfo, Label, Retention};
use crate::space::DiskInfo;

#[derive(Debug, PartialEq, Eq)]
pub enum DestError {
    /// The folder cannot be created or written to (permissions, read-only, a share that is gone).
    NotWritable(String),
    /// No room.
    Full(String),
    Io(String),
}

impl From<io::Error> for DestError {
    fn from(e: io::Error) -> Self {
        match e.kind() {
            io::ErrorKind::StorageFull => DestError::Full(e.to_string()),
            io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem => DestError::NotWritable(e.to_string()),
            _ => DestError::Io(e.to_string()),
        }
    }
}

pub trait Destination {
    /// Words for the person: "the folder /srv/backups".
    fn describe(&self) -> String;
    /// A real folder on this computer (`None` for a remote destination).
    fn local_dir(&self) -> Option<&Path>;
    /// Bytes free where the archive will be built; `None` when it cannot be told.
    fn free_bytes(&self) -> Option<u64>;
    /// Create what is needed and prove a file can be written.
    fn prepare(&self) -> Result<(), DestError>;
    /// Where to build an archive called `file` (owner-only, local).
    fn staging(&self, file: &str) -> PathBuf;
    /// Make the finished archive (at `staged`) and its listing file visible.
    fn commit(&self, staged: &Path, info: &BackupInfo) -> Result<(), DestError>;
    /// The backups here, newest first.
    fn list(&self) -> Vec<BackupInfo>;
    /// A local path to read the archive `id` from (a remote destination would download it first).
    fn open(&self, id: &str) -> Option<PathBuf>;
    /// Remove a backup (archive and listing file).
    fn delete(&self, id: &str) -> Result<(), DestError>;
}

/// A folder on this computer, a subfolder per server.
pub struct LocalFolder {
    dir: PathBuf,
}

/// The folder name of a server's backups.
pub fn server_folder_name(server: &str) -> String {
    crate::installer::slug(server)
}

/// Kora's own backups folder: `<Kora home>/server-backups`.
pub fn default_root() -> PathBuf {
    crate::installer::registry::kora_home().join("server-backups")
}

/// `root/<server>`; `root` empty = the default root.
pub fn folder_for(root: &str, server: &str) -> PathBuf {
    let base = if root.trim().is_empty() { default_root() } else { PathBuf::from(root.trim()) };
    base.join(server_folder_name(server))
}

impl LocalFolder {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        LocalFolder { dir: dir.into() }
    }

    /// The destination for a server's settings.
    pub fn for_server(root: &str, server: &str) -> Self {
        Self::new(folder_for(root, server))
    }

    fn info_path(&self, file: &str) -> PathBuf {
        self.dir.join(listing_name(file))
    }
}

/// `x.tar.gz` / `x.tar.gz.age` → `x.json`.
pub fn listing_name(archive: &str) -> String {
    let stem = archive.strip_suffix(".tar.gz.age").or_else(|| archive.strip_suffix(".tar.gz")).unwrap_or(archive);
    format!("{stem}.json")
}

/// The archive file name for a backup.
pub fn archive_name(server: &str, id: &str, encrypted: bool) -> String {
    format!("{}-{id}.tar.gz{}", server_folder_name(server), if encrypted { ".age" } else { "" })
}

impl Destination for LocalFolder {
    fn describe(&self) -> String {
        format!("the folder {}", self.dir.display())
    }

    fn local_dir(&self) -> Option<&Path> {
        Some(&self.dir)
    }

    fn free_bytes(&self) -> Option<u64> {
        crate::space::SystemDisk.free_bytes(&self.dir).ok()
    }

    fn prepare(&self) -> Result<(), DestError> {
        if crate::fsutil::is_symlink(&self.dir) {
            return Err(DestError::NotWritable(format!("{} is a link; choose the real folder", self.dir.display())));
        }
        // The default folder is made private (owner-only, and on Windows with its own access list).
        let home = crate::installer::registry::kora_home();
        if self.dir.starts_with(default_root()) {
            let root = default_root();
            crate::store::privacy::ensure_private_dir(&home, &root).map_err(|p| DestError::NotWritable(p.message))?;
            std::fs::create_dir_all(&self.dir)?;
            crate::store::privacy::tighten_dir(&self.dir);
        } else {
            std::fs::create_dir_all(&self.dir)?;
            crate::store::privacy::tighten_dir(&self.dir);
        }
        // Prove it: a file that is made and removed again.
        let probe = self.dir.join(format!(".kora-write-test-{}", std::process::id()));
        std::fs::write(&probe, b"ok").map_err(DestError::from)?;
        let _ = std::fs::remove_file(&probe);
        Ok(())
    }

    fn staging(&self, file: &str) -> PathBuf {
        self.dir.join(format!("{file}.part"))
    }

    fn commit(&self, staged: &Path, info: &BackupInfo) -> Result<(), DestError> {
        let final_path = self.dir.join(&info.file);
        crate::fsutil::rename_retrying(staged, &final_path)?;
        let json = serde_json::to_vec_pretty(info).map_err(|e| DestError::Io(e.to_string()))?;
        let tmp = self.dir.join(format!("{}.tmp", listing_name(&info.file)));
        let mut f = super::dbdump::create_private(&tmp).or_else(|_| {
            let _ = std::fs::remove_file(&tmp);
            super::dbdump::create_private(&tmp)
        })?;
        io::Write::write_all(&mut f, &json)?;
        drop(f);
        crate::fsutil::rename_retrying(&tmp, &self.info_path(&info.file))?;
        harden(&final_path);
        Ok(())
    }

    fn list(&self) -> Vec<BackupInfo> {
        let mut out: Vec<BackupInfo> = Vec::new();
        for e in std::fs::read_dir(&self.dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let Some(info) = std::fs::read(&p).ok().and_then(|b| serde_json::from_slice::<BackupInfo>(&b).ok()) else { continue };
            // a listing whose archive is gone is not a backup
            if self.dir.join(&info.file).is_file() && info.file.ends_with(".gz") | info.file.ends_with(".age") {
                out.push(info);
            }
        }
        out.sort_by(|a, b| b.created_unix.cmp(&a.created_unix).then(b.id.cmp(&a.id)));
        out
    }

    fn open(&self, id: &str) -> Option<PathBuf> {
        self.list().into_iter().find(|b| b.id == id).map(|b| self.dir.join(b.file))
    }

    fn delete(&self, id: &str) -> Result<(), DestError> {
        let Some(info) = self.list().into_iter().find(|b| b.id == id) else { return Ok(()) };
        // the listing first, so a half-deleted backup never looks like a good one
        let _ = std::fs::remove_file(self.info_path(&info.file));
        match std::fs::remove_file(self.dir.join(&info.file)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

/// Owner-only access for a finished archive. Unix: 0600 from creation (nothing to do). Windows: remove
/// inherited access so only the person who runs Kora can read it, even in a shared folder.
fn harden(path: &Path) {
    if crate::db::Os::current() == crate::db::Os::Windows {
        if let Some(user) = crate::winuser::name() {
            if let Ok(spec) = crate::db::windows_acl_command(path, &user) {
                let _ = crate::db::CommandRunner::run(&crate::db::SystemRunner, &spec);
            }
        }
    }
}

// ----------------------------------------------------------------------------------- space

/// What a backup is expected to need, in bytes of the destination's disk.
pub fn space_needed(payload_estimate: u64) -> u64 {
    // The archive is compressed (smaller than the payload), but it is built before it is known how much
    // smaller: ask for the payload plus room to work.
    payload_estimate.saturating_add(256 * crate::space::MIB)
}

/// `Ok` when `free` covers `needed`; otherwise the words to show.
pub fn check_space(free: Option<u64>, needed: u64, where_: &str) -> Result<(), String> {
    match free {
        Some(f) if f < needed => {
            Err(format!("{where_} has {} free, and this backup may need about {}. Free some space, or choose a folder on a bigger disk.", crate::space::human(f), crate::space::human(needed)))
        }
        _ => Ok(()),
    }
}

// ----------------------------------------------------------------------------------- retention

const DAY: u64 = 86_400;
/// A safety copy taken before a restore is kept at least this long.
const SAFETY_KEEP_DAYS: u64 = 30;

/// Which backups to remove under `r`. Only backups Kora made on a schedule or on request are counted;
/// a safety copy is removed after [`SAFETY_KEEP_DAYS`] days; a copy made for moving the server and the
/// one `protect`ed (what "undo" goes back to) are never removed. Nothing is removed when both limits are
/// off. The newest backup is always kept.
pub fn plan_prune(list: &[BackupInfo], r: &Retention, now: u64, protect: &[String]) -> Vec<String> {
    if r.keep_last == 0 && r.keep_daily_days == 0 {
        return vec![];
    }
    let mut counted: Vec<&BackupInfo> = list.iter().filter(|b| matches!(b.label, Label::Manual | Label::Scheduled)).collect();
    counted.sort_by(|a, b| b.created_unix.cmp(&a.created_unix).then(b.id.cmp(&a.id)));
    let mut keep: std::collections::HashSet<&str> = std::collections::HashSet::new();
    if let Some(newest) = counted.first() {
        keep.insert(&newest.id);
    }
    if r.keep_last > 0 {
        for b in counted.iter().take(r.keep_last as usize) {
            keep.insert(&b.id);
        }
    }
    if r.keep_daily_days > 0 {
        let mut days_seen = std::collections::HashSet::new();
        for b in &counted {
            if now.saturating_sub(b.created_unix) <= u64::from(r.keep_daily_days) * DAY && days_seen.insert(b.created_unix / DAY) {
                keep.insert(&b.id);
            }
        }
    }
    let mut out: Vec<String> = counted.iter().filter(|b| !keep.contains(b.id.as_str())).map(|b| b.id.clone()).collect();
    out.extend(list.iter().filter(|b| b.label == Label::BeforeRestore && now.saturating_sub(b.created_unix) > SAFETY_KEEP_DAYS * DAY).map(|b| b.id.clone()));
    out.retain(|id| !protect.contains(id));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::types::DbKind;

    fn info(id: &str, at: u64, label: Label) -> BackupInfo {
        BackupInfo {
            id: id.into(),
            file: format!("srv-{id}.tar.gz"),
            bytes: 10,
            sha256: "00".into(),
            encrypted: false,
            created_unix: at,
            label,
            kora_version: "0.8.0".into(),
            server: "Srv".into(),
            database: DbKind::None,
            by_reference: 0,
            in_full: 0,
            stopped: false,
            too_new: false,
        }
    }

    #[test]
    fn a_folder_destination_lists_commits_and_deletes() {
        let d = tempfile::tempdir().unwrap();
        let dest = LocalFolder::new(d.path().join("bk/srv"));
        dest.prepare().unwrap();
        let name = archive_name("Srv", "20240101-000000", false);
        let staged = dest.staging(&name);
        std::fs::write(&staged, b"data").unwrap();
        let mut i = info("20240101-000000", 100, Label::Manual);
        i.file = name.clone();
        dest.commit(&staged, &i).unwrap();
        assert_eq!(dest.list().len(), 1);
        assert_eq!(dest.open("20240101-000000").unwrap(), d.path().join("bk/srv").join(&name));
        assert!(!staged.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let m = std::fs::metadata(d.path().join("bk/srv").join(listing_name(&name))).unwrap().permissions().mode() & 0o777;
            assert_eq!(m, 0o600, "the listing is private too");
        }
        dest.delete("20240101-000000").unwrap();
        assert!(dest.list().is_empty());
        assert!(std::fs::read_dir(d.path().join("bk/srv")).unwrap().next().is_none());
    }

    #[test]
    fn an_unwritable_folder_is_reported() {
        let d = tempfile::tempdir().unwrap();
        let file = d.path().join("a-file");
        std::fs::write(&file, b"x").unwrap();
        // a folder cannot be made below a file
        let dest = LocalFolder::new(file.join("sub"));
        assert!(matches!(dest.prepare(), Err(DestError::NotWritable(_) | DestError::Io(_))));
    }

    #[test]
    fn a_missing_archive_hides_its_listing() {
        let d = tempfile::tempdir().unwrap();
        let dest = LocalFolder::new(d.path());
        std::fs::write(d.path().join("srv-1.json"), serde_json::to_vec(&info("1", 1, Label::Manual)).unwrap()).unwrap();
        assert!(dest.list().is_empty());
    }

    #[test]
    fn names() {
        assert_eq!(archive_name("My Server!", "20240101-000000", false), "my-server-20240101-000000.tar.gz");
        assert_eq!(archive_name("a", "1", true), "a-1.tar.gz.age");
        assert_eq!(listing_name("a-1.tar.gz.age"), "a-1.json");
        assert_eq!(listing_name("a-1.tar.gz"), "a-1.json");
    }

    #[test]
    fn space_words() {
        assert!(check_space(Some(10), 1_000_000_000, "The folder").unwrap_err().contains("free"));
        assert!(check_space(Some(2_000_000_000), 1_000_000_000, "x").is_ok());
        assert!(check_space(None, u64::MAX, "x").is_ok(), "unknown space is not a reason to refuse");
    }

    #[test]
    fn keep_the_last_n() {
        let list: Vec<_> = (1..=10).map(|i| info(&format!("b{i:02}"), i * 1000, Label::Scheduled)).collect();
        let gone = plan_prune(&list, &Retention { keep_last: 3, keep_daily_days: 0 }, 20_000, &[]);
        assert_eq!(gone.len(), 7);
        assert!(!gone.contains(&"b10".to_string()) && !gone.contains(&"b08".to_string()) && gone.contains(&"b07".to_string()));
        assert!(plan_prune(&list, &Retention { keep_last: 0, keep_daily_days: 0 }, 20_000, &[]).is_empty(), "no limits: nothing is removed");
    }

    #[test]
    fn keep_one_per_day_for_d_days() {
        let now = 40 * DAY;
        let mut list = vec![];
        for day in 30..40u64 {
            list.push(info(&format!("d{day}a"), day * DAY + 100, Label::Scheduled));
            list.push(info(&format!("d{day}b"), day * DAY + 5000, Label::Scheduled));
        }
        list.push(info("old", 5 * DAY, Label::Manual));
        let gone = plan_prune(&list, &Retention { keep_last: 0, keep_daily_days: 3 }, now, &[]);
        // the newest of each of the last 3 days survives (days 37..39); the older ones and "old" go
        assert!(gone.contains(&"old".to_string()) && gone.contains(&"d39a".to_string()) && gone.contains(&"d35b".to_string()));
        assert!(!gone.contains(&"d39b".to_string()) && !gone.contains(&"d38b".to_string()) && !gone.contains(&"d37b".to_string()));
    }

    #[test]
    fn safety_copies_and_move_exports_are_handled_apart() {
        let now = 100 * DAY;
        let list = vec![
            info("m1", 99 * DAY, Label::Manual),
            info("m2", 98 * DAY, Label::Manual),
            info("safe-new", 99 * DAY, Label::BeforeRestore),
            info("safe-old", 10 * DAY, Label::BeforeRestore),
            info("move", DAY, Label::Move),
        ];
        let gone = plan_prune(&list, &Retention { keep_last: 1, keep_daily_days: 0 }, now, &[]);
        assert_eq!(gone, vec!["m2".to_string(), "safe-old".to_string()]);
        // the copy "undo" needs is never removed
        let gone = plan_prune(&list, &Retention { keep_last: 1, keep_daily_days: 0 }, now, &["safe-old".to_string()]);
        assert_eq!(gone, vec!["m2".to_string()]);
    }
}
