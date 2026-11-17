//! Free-disk-space preflight. Everything that asks the OS goes through [`DiskInfo`], so the
//! decision logic is tested with a fake provider.
//!
//! Estimates (measured from the KoraFiveM repositories: 81 repos, ~6 MiB average on GitHub; a
//! clone keeps the history *and* a checkout, so we count twice that; FXServer is a ~300 MB
//! download that unpacks to ~700 MB):

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

pub const MIB: u64 = 1024 * 1024;
pub const GIB: u64 = 1024 * MIB;
/// Working room on top of the estimates (logs, txAdmin data, caches, git temp files).
pub const HEADROOM: u64 = 2 * GIB;
/// Download + unpacked FXServer build.
pub const ARTIFACT: u64 = GIB;
/// One cloned resource (history + checkout), rounded up.
pub const PER_RESOURCE: u64 = 24 * MIB;
/// Resources assumed when the whole catalog is selected.
pub const ALL_RESOURCES_ESTIMATE: u64 = 70;

/// What the OS can tell us about a disk.
pub trait DiskInfo {
    /// Bytes an unprivileged process can still write on the volume holding `path`
    /// (`path` need not exist yet: the nearest existing parent is used).
    fn free_bytes(&self, path: &Path) -> io::Result<u64>;
    /// Identifies the volume of `path`; equal keys mean the same disk (their needs add up).
    fn volume_key(&self, path: &Path) -> String;
}

/// The real disk (statvfs on Unix, GetDiskFreeSpaceExW on Windows).
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemDisk;

fn nearest_existing(path: &Path) -> PathBuf {
    let mut p = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(path) };
    while !p.exists() {
        match p.parent() {
            Some(q) => p = q.to_path_buf(),
            None => break,
        }
    }
    p
}

impl DiskInfo for SystemDisk {
    fn free_bytes(&self, path: &Path) -> io::Result<u64> {
        free_bytes_os(&nearest_existing(path))
    }

    fn volume_key(&self, path: &Path) -> String {
        volume_key_os(&nearest_existing(path))
    }
}

#[cfg(unix)]
fn free_bytes_os(p: &Path) -> io::Result<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(p.as_os_str().as_bytes()).map_err(|_| io::Error::other("path contains a NUL byte"))?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid NUL-terminated path and `st` a properly sized out-parameter.
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((st.f_bavail as u64).saturating_mul(st.f_frsize as u64))
}

#[cfg(unix)]
fn volume_key_os(p: &Path) -> String {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(p).map(|m| m.dev().to_string()).unwrap_or_else(|_| "?".into())
}

#[cfg(windows)]
fn free_bytes_os(p: &Path) -> io::Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetDiskFreeSpaceExW(dir: *const u16, available: *mut u64, total: *mut u64, free: *mut u64) -> i32;
    }
    let wide: Vec<u16> = p.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let mut available = 0u64;
    // SAFETY: `wide` is NUL-terminated; the two optional out-parameters may be null.
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut available, std::ptr::null_mut(), std::ptr::null_mut()) };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(available)
    }
}

#[cfg(windows)]
fn volume_key_os(p: &Path) -> String {
    let abs = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    match abs.components().next() {
        Some(std::path::Component::Prefix(x)) => x.as_os_str().to_string_lossy().to_ascii_lowercase(),
        _ => "?".into(),
    }
}

/// One thing that needs room, on one path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Need {
    pub what: String,
    pub path: PathBuf,
    pub bytes: u64,
}

impl Need {
    pub fn new(what: impl Into<String>, path: impl Into<PathBuf>, bytes: u64) -> Self {
        Need { what: what.into(), path: path.into(), bytes }
    }
}

/// `1.5 GiB`, `300 MiB`.
pub fn human(bytes: u64) -> String {
    if bytes >= GIB {
        format!("{:.1} GiB", bytes as f64 / GIB as f64)
    } else {
        format!("{} MiB", bytes.div_ceil(MIB))
    }
}

/// Needs of a server install. `resources` = how many resources are expected; `resuming` = the
/// folder already holds a partial install (only working room is required again).
/// The artifact folder lives inside the server folder but is listed on its own so a future split
/// onto another disk is already handled; needs on the same volume are added up by [`check`].
pub fn install_needs(dir: &Path, resources: u64, resuming: bool) -> Vec<Need> {
    if resuming {
        return vec![Need::new("working room", dir, HEADROOM)];
    }
    vec![
        Need::new(format!("{resources} resources"), dir, resources * PER_RESOURCE),
        Need::new("the FXServer build", dir.join("artifacts"), ARTIFACT),
        // Folders the service account writes to (txAdmin data, logs, cache) are inside the server
        // folder (ReadWritePaths), so its room is this headroom.
        Need::new("working room for the server and its service account", dir, HEADROOM),
    ]
}

/// Expected number of resources for the selection size (`None` = everything in the catalog).
pub fn resource_estimate(selected: Option<usize>) -> u64 {
    selected.map_or(ALL_RESOURCES_ESTIMATE, |n| n as u64 + 10)
}

/// Does every volume have room for the sum of the needs placed on it?
/// Err = a plain-language message listing every volume that is too small.
pub fn check(disk: &dyn DiskInfo, needs: &[Need]) -> Result<(), String> {
    if std::env::var_os("KORA_SKIP_SPACE_CHECK").is_some_and(|v| v == "1") {
        return Ok(());
    }
    let mut by_volume: BTreeMap<String, Vec<&Need>> = BTreeMap::new();
    for n in needs {
        by_volume.entry(disk.volume_key(&n.path)).or_default().push(n);
    }
    let mut problems = Vec::new();
    for group in by_volume.values() {
        let total: u64 = group.iter().map(|n| n.bytes).sum();
        let first = group[0];
        let free = match disk.free_bytes(&first.path) {
            Ok(f) => f,
            Err(e) => {
                // Not being able to ask is not a reason to block an install.
                eprintln!("note: could not read the free space of {}: {e}", first.path.display());
                continue;
            }
        };
        if free < total {
            let parts: Vec<String> = group.iter().map(|n| format!("{} for {}", human(n.bytes), n.what)).collect();
            problems.push(format!(
                "Not enough free disk space on the disk holding {}: Kora needs about {} ({}) but only {} is free.",
                first.path.display(),
                human(total),
                parts.join(" + "),
                human(free)
            ));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!("{} Free up some space or choose another folder, then try again. Nothing was changed.", problems.join(" ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fake disks: a path belongs to the volume of its longest matching prefix.
    struct Fake {
        volumes: Vec<(&'static str, &'static str, u64)>, // (prefix, key, free)
        fail: bool,
    }

    impl Fake {
        fn vol(&self, p: &Path) -> &(&'static str, &'static str, u64) {
            self.volumes.iter().filter(|v| p.starts_with(v.0)).max_by_key(|v| v.0.len()).unwrap()
        }
    }

    impl DiskInfo for Fake {
        fn free_bytes(&self, path: &Path) -> io::Result<u64> {
            if self.fail {
                return Err(io::Error::other("boom"));
            }
            Ok(self.vol(path).2)
        }
        fn volume_key(&self, path: &Path) -> String {
            self.vol(path).1.to_string()
        }
    }

    #[test]
    fn enough_room_passes_and_the_message_names_the_numbers_when_short() {
        let needs = install_needs(Path::new("/srv/kora/a"), 54, false);
        let total: u64 = needs.iter().map(|n| n.bytes).sum();
        assert_eq!(total, 54 * 24 * MIB + GIB + 2 * GIB);
        let ok = Fake { volumes: vec![("/", "root", total + 1)], fail: false };
        assert!(check(&ok, &needs).is_ok());
        let small = Fake { volumes: vec![("/", "root", total - 1)], fail: false };
        let e = check(&small, &needs).unwrap_err();
        assert!(e.contains("Not enough free disk space"), "{e}");
        assert!(e.contains("/srv/kora/a") && e.contains("4.3 GiB"), "{e}");
        assert!(e.contains("1.0 GiB for the FXServer build") && e.contains("54 resources"), "{e}");
        assert!(e.contains("Nothing was changed") && !e.contains("--dir"));
    }

    #[test]
    fn needs_on_one_volume_add_up_but_separate_volumes_are_checked_alone() {
        let needs = vec![Need::new("resources", "/data/srv", 3 * GIB), Need::new("the FXServer build", "/data/srv/artifacts", 3 * GIB), Need::new("the kora program", "/opt/kora", 100 * MIB)];
        // same volume: 6 GiB needed, only 5 free -> fails although each need alone would fit
        let one = Fake { volumes: vec![("/", "r", 5 * GIB)], fail: false };
        assert!(check(&one, &needs).is_err());
        // artifact on its own disk: both fit; /opt has plenty
        let two = Fake { volumes: vec![("/", "r", 5 * GIB), ("/data/srv/artifacts", "a", 4 * GIB)], fail: false };
        assert!(check(&two, &needs).is_ok());
        // only the small /opt volume is short -> only it is reported
        let three = Fake { volumes: vec![("/", "r", 50 * GIB), ("/opt", "o", 10 * MIB)], fail: false };
        let e = check(&three, &needs).unwrap_err();
        assert!(e.contains("/opt/kora") && !e.contains("/data/srv"), "{e}");
    }

    #[test]
    fn resuming_needs_only_working_room_and_unreadable_disks_do_not_block() {
        let n = install_needs(Path::new("/x"), 54, true);
        assert_eq!(n.iter().map(|n| n.bytes).sum::<u64>(), HEADROOM);
        let broken = Fake { volumes: vec![("/", "r", 0)], fail: true };
        assert!(check(&broken, &install_needs(Path::new("/x"), 54, false)).is_ok());
        assert_eq!(resource_estimate(None), 70);
        assert_eq!(resource_estimate(Some(5)), 15);
    }

    #[test]
    fn human_sizes() {
        assert_eq!(human(300 * MIB), "300 MiB");
        assert_eq!(human(GIB + GIB / 2), "1.5 GiB");
        assert_eq!(human(1), "1 MiB");
    }

    #[test]
    fn the_real_disk_answers_for_existing_and_not_yet_existing_folders() {
        let t = tempfile::tempdir().unwrap();
        let deep = t.path().join("a/b/c/server");
        SystemDisk.free_bytes(&deep).expect("statvfs / GetDiskFreeSpaceExW answers for the nearest existing parent");
        assert_eq!(SystemDisk.volume_key(&deep), SystemDisk.volume_key(t.path()));
    }
}
