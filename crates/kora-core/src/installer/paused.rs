//! Stopped installs that can carry on after Kora Panel restarts.
//!
//! A failed install pauses with its answers held in memory (the panel's `Paused`), because they include
//! the licence key. When that key is a SAVED one (see `keystore`) the answers hold only its name, so
//! they can also be written down: `<kora home>/paused-installs/<job>.json`. No secret is in the file:
//! the key is removed, and the database administrator password is never serialised (so an install that
//! needs that password asks for it again, or uses the automatic login). After a restart the panel reads
//! the file back and looks the key up by name.
//!
//! A pasted key the person chose not to save is never persisted: that install stays in memory only.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::registry::kora_home;
use super::InstallRequest;

/// How long a stopped install can be continued after a restart.
pub const KEEP_SECS: u64 = 3 * 24 * 3600;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stored {
    pub job: String,
    /// The answers, WITHOUT the licence key (`license_key_name` says which saved key).
    pub req: InstallRequest,
    pub via_helper: bool,
    pub failed_step: Option<String>,
    /// The progress events of the steps that had finished.
    pub finished: Vec<Value>,
    pub saved_at: u64,
}

fn valid_id(job: &str) -> bool {
    !job.is_empty() && job.len() <= 64 && job.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

#[derive(Debug, Clone)]
pub struct PausedStore {
    dir: PathBuf,
}

impl Default for PausedStore {
    fn default() -> Self {
        Self { dir: kora_home().join("paused-installs") }
    }
}

impl PausedStore {
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn file(&self, job: &str) -> Option<PathBuf> {
        valid_id(job).then(|| self.dir.join(format!("{job}.json")))
    }

    /// Write a stopped install down. Only installs that use a saved key can be (`Ok(false)` otherwise:
    /// nothing is written). One stopped install per folder: an older file for the same folder goes.
    pub fn save(&self, job: &str, req: &InstallRequest, via_helper: bool, failed_step: Option<&str>, finished: &[Value]) -> std::io::Result<bool> {
        let (Some(path), Some(_)) = (self.file(job), req.license_key_name.as_ref()) else { return Ok(false) };
        let mut r = req.clone();
        r.license_key = None;
        r.catalog = None;
        let item = Stored { job: job.to_string(), req: r, via_helper, failed_step: failed_step.map(String::from), finished: finished.to_vec(), saved_at: now() };
        for old in self.all() {
            if old.req.dir == item.req.dir && old.job != job {
                self.remove(&old.job);
            }
        }
        std::fs::create_dir_all(&self.dir)?;
        let text = serde_json::to_string_pretty(&item).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text)?;
        crate::fsutil::rename_retrying(&tmp, &path)?;
        Ok(true)
    }

    /// A stopped install, if it is still there and not too old (a file that cannot be read is skipped).
    pub fn load(&self, job: &str) -> Option<Stored> {
        let path = self.file(job)?;
        let item: Stored = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
        if now().saturating_sub(item.saved_at) > KEEP_SECS {
            let _ = std::fs::remove_file(&path);
            return None;
        }
        (item.job == job).then_some(item)
    }

    pub fn remove(&self, job: &str) {
        if let Some(p) = self.file(job) {
            let _ = std::fs::remove_file(p);
        }
    }

    /// Forget stopped installs of this folder (a new start replaces them).
    pub fn remove_for_dir(&self, dir: &Path) {
        for s in self.all() {
            if s.req.dir == dir {
                self.remove(&s.job);
            }
        }
    }

    fn all(&self) -> Vec<Stored> {
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return vec![] };
        rd.flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                self.load(name.strip_suffix(".json")?)
            })
            .collect()
    }

    /// How many stopped installs use the saved key with this name.
    pub fn count_key(&self, name: &str) -> usize {
        self.all().iter().filter(|s| s.req.license_key_name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(name))).count()
    }

    /// The saved key was renamed: follow it. Returns how many stopped installs changed.
    pub fn rename_key(&self, old: &str, new: &str) -> usize {
        let mut n = 0;
        for mut s in self.all() {
            if s.req.license_key_name.as_deref().is_some_and(|k| k.eq_ignore_ascii_case(old)) {
                s.req.license_key_name = Some(new.to_string());
                if let (Some(p), Ok(text)) = (self.file(&s.job), serde_json::to_string_pretty(&s)) {
                    let tmp = p.with_extension("json.tmp");
                    if std::fs::write(&tmp, text).is_ok() && crate::fsutil::rename_retrying(&tmp, &p).is_ok() {
                        n += 1;
                    }
                }
            }
        }
        n
    }
}

fn now() -> u64 {
    crate::monitor::now_unix()
}

// ---- the saved answers (last-install.json) ------------------------------------------------------

/// Does the saved answers file at `path` use the saved key `name`?
pub fn saved_answers_use_key(path: &Path, name: &str) -> bool {
    read_saved(path).is_some_and(|r| r.license_key_name.as_deref().is_some_and(|k| k.eq_ignore_ascii_case(name)))
}

/// Follow a renamed key in the saved answers at `path`. Returns whether the file changed.
pub fn rename_key_in_saved_answers(path: &Path, old: &str, new: &str) -> bool {
    let Some(mut r) = read_saved(path) else { return false };
    if !r.license_key_name.as_deref().is_some_and(|k| k.eq_ignore_ascii_case(old)) {
        return false;
    }
    r.license_key_name = Some(new.to_string());
    r.license_key = None;
    let Ok(text) = serde_json::to_string_pretty(&r) else { return false };
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).is_ok() && crate::fsutil::rename_retrying(&tmp, path).is_ok()
}

fn read_saved(path: &Path) -> Option<InstallRequest> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::installer::{DbAdmin, DbChoice, Mode, Ports, ResourceSelection};

    fn req(dir: &str, key_name: Option<&str>) -> InstallRequest {
        InstallRequest {
            mode: Mode::Production,
            name: "Dev Den".into(),
            dir: dir.into(),
            hostname: "Dev Den".into(),
            license_key: Some("cfxk_SECRETKEYSECRET".into()),
            license_key_name: key_name.map(String::from),
            cfx_identifier: None,
            ports: Ports { game: 30120, txadmin: 40120 },
            txadmin_public: false,
            artifact_url: None,
            game_build: None,
            resources: ResourceSelection::All,
            db: DbChoice::InstallIfMissing,
            service: true,
            firewall: true,
            github_owner: None,
            catalog: None,
            dry_run: false,
            db_admin: Some(DbAdmin { host: "127.0.0.1".into(), port: 3306, user: "root".into(), password: Some("hunter2".into()) }),
            start: true,
            app: None,
        }
    }

    #[test]
    fn only_saved_key_installs_are_written_and_never_with_a_secret() {
        let d = tempfile::tempdir().unwrap();
        let s = PausedStore::at(d.path().join("p"));
        let ev = vec![serde_json::json!({"step": "prepare.git", "status": "done"})];
        // A pasted key that was not saved: nothing on disk.
        assert!(!s.save("job1", &req("/srv/a", None), false, Some("build.clone"), &ev).unwrap());
        assert!(!d.path().join("p").exists());
        assert!(s.save("job2", &req("/srv/a", Some("Main")), false, Some("build.clone"), &ev).unwrap());
        let text = std::fs::read_to_string(d.path().join("p/job2.json")).unwrap();
        assert!(!text.contains("SECRETKEY") && !text.contains("hunter2"), "{text}");
        let v: Value = serde_json::from_str(&text).unwrap();
        assert!(v["req"]["license_key"].is_null(), "{text}");
        let back = s.load("job2").unwrap();
        assert_eq!(back.req.license_key, None);
        assert_eq!(back.req.license_key_name.as_deref(), Some("Main"));
        assert_eq!(back.req.db_admin.unwrap().password, None);
        assert_eq!(back.failed_step.as_deref(), Some("build.clone"));
        assert_eq!(back.finished, ev);
    }

    #[test]
    fn one_per_folder_expiry_bad_ids_and_key_references() {
        let d = tempfile::tempdir().unwrap();
        let s = PausedStore::at(d.path());
        s.save("a1", &req("/srv/a", Some("Main")), false, None, &[]).unwrap();
        s.save("a2", &req("/srv/a", Some("Main")), false, None, &[]).unwrap();
        s.save("b1", &req("/srv/b", Some("Test")), true, None, &[]).unwrap();
        assert!(s.load("a1").is_none() && s.load("a2").is_some(), "a newer one replaces the older for the same folder");
        assert_eq!((s.count_key("main"), s.count_key("Test"), s.count_key("x")), (1, 1, 0));
        assert_eq!(s.rename_key("Main", "Live"), 1);
        assert_eq!(s.load("a2").unwrap().req.license_key_name.as_deref(), Some("Live"));
        s.remove_for_dir(Path::new("/srv/b"));
        assert!(s.load("b1").is_none());
        // Path tricks are refused.
        assert!(!s.save("../x", &req("/srv/c", Some("Main")), false, None, &[]).unwrap());
        assert!(s.load("../a2").is_none());
        // Too old.
        let mut old: Stored = serde_json::from_slice(&std::fs::read(d.path().join("a2.json")).unwrap()).unwrap();
        old.saved_at = 1;
        std::fs::write(d.path().join("a2.json"), serde_json::to_string(&old).unwrap()).unwrap();
        assert!(s.load("a2").is_none());
        assert!(!d.path().join("a2.json").exists());
    }

    #[test]
    fn saved_answers_follow_a_renamed_key() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("last-install.json");
        let mut r = req("/srv/a", Some("Main"));
        r.license_key = None;
        std::fs::write(&p, serde_json::to_string(&r).unwrap()).unwrap();
        assert!(saved_answers_use_key(&p, "main"));
        assert!(rename_key_in_saved_answers(&p, "Main", "Live"));
        assert!(saved_answers_use_key(&p, "Live") && !saved_answers_use_key(&p, "Main"));
        assert!(!rename_key_in_saved_answers(&d.path().join("none.json"), "a", "b"));
    }
}
