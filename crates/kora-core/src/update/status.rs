//! `<server>/.kora/update-status.json`: what the last update check/run found, for the manager's badges.
//!
//! Shape (read by the manager, keep stable):
//! `{"checked_at": "<RFC3339 UTC>", "updates": <n>, "attention": <n>, "new_release": "<version>"|null}`
//!
//! * `updates`: resources with an update ready that has NOT been applied (a preview: the clean ones; after a run:
//!   the ones deferred until the server is quiet).
//! * `attention`: resources that need a human (dirty or diverged forks, failed or rolled-back updates, SQL to run).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateStatus {
    pub checked_at: String,
    pub updates: u32,
    pub attention: u32,
    pub new_release: Option<String>,
}

pub fn path(server_dir: &Path) -> PathBuf {
    server_dir.join(".kora").join("update-status.json")
}

/// Seconds since the Unix epoch as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn rfc3339(unix: u64) -> String {
    let days = (unix / 86_400) as i64;
    let rem = unix % 86_400;
    // civil-from-days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

impl UpdateStatus {
    pub fn new(now_unix: u64, updates: usize, attention: usize, new_release: Option<String>) -> Self {
        UpdateStatus { checked_at: rfc3339(now_unix), updates: updates as u32, attention: attention as u32, new_release }
    }
}

/// Write atomically (temp file in the same folder, then rename), so a reader never sees half a file.
pub fn write(server_dir: &Path, st: &UpdateStatus) -> std::io::Result<()> {
    let p = path(server_dir);
    let dir = p.parent().expect("status path has a parent");
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("update-status.json.{}.tmp", std::process::id()));
    std::fs::write(&tmp, serde_json::to_vec_pretty(st).map_err(std::io::Error::other)?)?;
    if let Err(e) = std::fs::rename(&tmp, &p) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// Best effort: a status file that cannot be written never fails an update.
pub fn record(server_dir: &Path, st: &UpdateStatus) {
    let _ = write(server_dir, st);
}

pub fn read(server_dir: &Path) -> Option<UpdateStatus> {
    serde_json::from_slice(&std::fs::read(path(server_dir)).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_known_dates() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(rfc3339(1_793_000_000), "2026-10-26T07:33:20Z");
    }

    #[test]
    fn written_atomically_with_exactly_the_manager_shape() {
        let t = tempfile::tempdir().unwrap();
        write(t.path(), &UpdateStatus::new(1_700_000_000, 2, 1, Some("1.4.0".into()))).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(path(t.path())).unwrap()).unwrap();
        assert_eq!(v, serde_json::json!({"checked_at": "2023-11-14T22:13:20Z", "updates": 2, "attention": 1, "new_release": "1.4.0"}));
        write(t.path(), &UpdateStatus::new(1_700_000_060, 0, 0, None)).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(path(t.path())).unwrap()).unwrap();
        assert_eq!(v["new_release"], serde_json::Value::Null);
        let left: Vec<_> = std::fs::read_dir(t.path().join(".kora")).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(left.len(), 1, "no temp file left behind: {left:?}");
        assert_eq!(read(t.path()).unwrap().updates, 0);
    }
}
