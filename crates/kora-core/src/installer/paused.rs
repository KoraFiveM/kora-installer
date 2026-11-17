//! Stopped installs that can carry on after Kora Panel restarts.
//!
//! A failed install pauses with its answers held in memory (the panel's `Paused`), because they include
//! the licence key. When that key is a SAVED one (see `keystore`) the answers hold only its name, so
//! they can also be written down (the store's `paused_installs` table; `<kora home>/paused-installs/<job>.json`
//! before v0.8). No secret is in the row: the key is removed, and the database administrator password is
//! never serialised (so an install that needs that password asks for it again, or uses the automatic login).
//! After a restart the panel reads the row back and looks the key up by name.
//!
//! A pasted key the person chose not to save is never persisted: that install stays in memory only.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::registry::kora_home;
use super::InstallRequest;
use crate::store::{problem_to_io, Store};

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
    !job.is_empty() && job.len() <= 64 && job.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') && !windows_device_name(job)
}

/// `CON`, `NUL`, `COM1`...: on Windows `<folder>\CON.json` is the console device, not a file, and reading it
/// waits for keyboard input forever. The job id comes from the request address, so such names are refused
/// (real ids are 16 hex digits).
fn windows_device_name(s: &str) -> bool {
    let u = s.to_ascii_uppercase();
    let numbered = |p: &str| u.strip_prefix(p).is_some_and(|n| n.len() == 1 && matches!(n.as_bytes()[0], b'1'..=b'9'));
    matches!(u.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$") || numbered("COM") || numbered("LPT")
}

/// The stopped installs in the store. `dir` is `<kora home>/paused-installs` (the folder they were files
/// in before v0.8): it only says which Kora home. A folder with another name (tests) is its own home.
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

    fn home(&self) -> &Path {
        if self.dir.file_name().is_some_and(|n| n == "paused-installs") {
            self.dir.parent().unwrap_or(&self.dir)
        } else {
            &self.dir
        }
    }

    /// Write a stopped install down. Only installs that use a saved key can be (`Ok(false)` otherwise:
    /// nothing is written). One stopped install per folder: an older one for the same folder goes.
    pub fn save(&self, job: &str, req: &InstallRequest, via_helper: bool, failed_step: Option<&str>, finished: &[Value]) -> std::io::Result<bool> {
        self.save_as(job, req, via_helper, failed_step, finished, false)
    }

    /// An install that is RUNNING now, written down so that a panel that dies leaves something to carry on
    /// from. It is not offered as "carry on" while its job runs ([`list`](Self::list) skips it); the next
    /// panel that finds the job unfinished makes it a stopped install (see `store::jobs::recover`).
    pub fn save_in_progress(&self, job: &str, req: &InstallRequest, via_helper: bool, finished: &[Value]) -> std::io::Result<bool> {
        self.save_as(job, req, via_helper, None, finished, true)
    }

    fn save_as(&self, job: &str, req: &InstallRequest, via_helper: bool, failed_step: Option<&str>, finished: &[Value], in_progress: bool) -> std::io::Result<bool> {
        if !valid_id(job) || req.license_key_name.is_none() {
            return Ok(false);
        }
        let mut r = req.clone();
        r.license_key = None;
        r.catalog = None;
        let req_json = serde_json::to_string(&r).map_err(std::io::Error::other)?;
        let finished_json = serde_json::to_string(finished).map_err(std::io::Error::other)?;
        let dir = r.dir.to_string_lossy().into_owned();
        let s = Store::open(self.home()).map_err(problem_to_io)?;
        s.write(|tx| {
            tx.execute("DELETE FROM paused_installs WHERE dir = ?1 AND job <> ?2", rusqlite::params![dir, job])?;
            tx.execute(
                "INSERT INTO paused_installs (job, dir, via_helper, failed_step, req_json, finished_json, saved_at, in_progress) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(job) DO UPDATE SET dir = excluded.dir, via_helper = excluded.via_helper, failed_step = excluded.failed_step, req_json = excluded.req_json, finished_json = excluded.finished_json, saved_at = excluded.saved_at, in_progress = excluded.in_progress",
                rusqlite::params![job, dir, via_helper, failed_step, req_json, finished_json, now() as i64, in_progress],
            )?;
            Ok(())
        })
        .map_err(problem_to_io)?;
        Ok(true)
    }

    /// A stopped install, if it is still there and not too old (one that cannot be read is skipped).
    pub fn load(&self, job: &str) -> Option<Stored> {
        if !valid_id(job) {
            return None;
        }
        self.all().into_iter().find(|s| s.job == job)
    }

    pub fn remove(&self, job: &str) {
        if let Ok(Some(s)) = Store::open_existing(self.home()) {
            let _ = s.write(|tx| {
                tx.execute("DELETE FROM paused_installs WHERE job = ?1", [job])?;
                Ok(())
            });
        }
    }

    /// Forget stopped installs of this folder (a new start replaces them).
    pub fn remove_for_dir(&self, dir: &Path) {
        if let Ok(Some(s)) = Store::open_existing(self.home()) {
            let d = dir.to_string_lossy().into_owned();
            let _ = s.write(|tx| {
                tx.execute("DELETE FROM paused_installs WHERE dir = ?1", [d])?;
                Ok(())
            });
        }
    }

    /// Every stopped install that is written down and still fresh.
    pub fn list(&self) -> Vec<Stored> {
        self.all()
    }

    /// Every stopped install in an open store (the export of the old-format files).
    pub(crate) fn list_in(s: &Store) -> Vec<Stored> {
        let rows: Vec<(String, bool, Option<String>, String, String, i64)> = s
            .read(|c| {
                let mut st = c.prepare("SELECT job, via_helper, failed_step, req_json, finished_json, saved_at FROM paused_installs ORDER BY saved_at, job")?;
                let v = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?.collect::<Result<Vec<_>, _>>()?;
                Ok(v)
            })
            .unwrap_or_default();
        rows.into_iter()
            .filter_map(|(job, via_helper, failed_step, req, finished, saved_at)| {
                Some(Stored { job, req: serde_json::from_str(&req).ok()?, via_helper, failed_step, finished: serde_json::from_str(&finished).ok()?, saved_at: saved_at as u64 })
            })
            .collect()
    }

    fn all(&self) -> Vec<Stored> {
        let Ok(Some(s)) = Store::open_existing(self.home()) else { return vec![] };
        let cutoff = now().saturating_sub(KEEP_SECS) as i64;
        // Old ones are forgotten when anyone looks (what the file version did when it read one).
        let _ = s.write(|tx| {
            tx.execute("DELETE FROM paused_installs WHERE saved_at < ?1", [cutoff])?;
            Ok(())
        });
        let rows: Vec<(String, bool, Option<String>, String, String, i64)> = s
            .read(|c| {
                let mut st = c.prepare("SELECT job, via_helper, failed_step, req_json, finished_json, saved_at FROM paused_installs WHERE in_progress = 0 ORDER BY saved_at, job")?;
                let v = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?.collect::<Result<Vec<_>, _>>()?;
                Ok(v)
            })
            .unwrap_or_default();
        rows.into_iter()
            .filter_map(|(job, via_helper, failed_step, req, finished, saved_at)| {
                Some(Stored { job, req: serde_json::from_str(&req).ok()?, via_helper, failed_step, finished: serde_json::from_str(&finished).ok()?, saved_at: saved_at as u64 })
            })
            .collect()
    }

    /// How many stopped installs use the saved key with this name.
    pub fn count_key(&self, name: &str) -> usize {
        self.all().iter().filter(|s| s.req.license_key_name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(name))).count()
    }

    /// How many unfinished installs use the saved key `name`, counting the saved answers at `saved` too. A
    /// stopped install leaves both a paused entry and saved answers for the same folder: that is ONE install.
    pub fn count_key_with_saved(&self, saved: &Path, name: &str) -> usize {
        let same = |k: &Option<String>| k.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(name));
        let paused = self.all();
        let n = paused.iter().filter(|s| same(&s.req.license_key_name)).count();
        let extra = read_saved(saved).is_some_and(|r| same(&r.license_key_name) && !paused.iter().any(|s| s.req.dir == r.dir));
        n + usize::from(extra)
    }

    /// The saved key was renamed: follow it. Returns how many stopped installs changed.
    pub fn rename_key(&self, old: &str, new: &str) -> usize {
        let mut n = 0;
        for mut s in self.all() {
            if s.req.license_key_name.as_deref().is_some_and(|k| k.eq_ignore_ascii_case(old)) {
                s.req.license_key_name = Some(new.to_string());
                if let (Ok(text), Ok(st)) = (serde_json::to_string(&s.req), Store::open(self.home())) {
                    let job = s.job.clone();
                    if st
                        .write(|tx| {
                            tx.execute("UPDATE paused_installs SET req_json = ?1 WHERE job = ?2", rusqlite::params![text, job])?;
                            Ok(())
                        })
                        .is_ok()
                    {
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

// ---- the saved answers ----------------------------------------------------------------------------

/// Does the saved answers in the Kora home beside `path` use the saved key `name`?
pub fn saved_answers_use_key(path: &Path, name: &str) -> bool {
    read_saved(path).is_some_and(|r| r.license_key_name.as_deref().is_some_and(|k| k.eq_ignore_ascii_case(name)))
}

/// Follow a renamed key in the saved answers beside `path`. Returns whether they changed.
pub fn rename_key_in_saved_answers(path: &Path, old: &str, new: &str) -> bool {
    let Some(mut r) = read_saved(path) else { return false };
    if !r.license_key_name.as_deref().is_some_and(|k| k.eq_ignore_ascii_case(old)) {
        return false;
    }
    r.license_key_name = Some(new.to_string());
    r.license_key = None;
    super::saved::store_beside(path, &r)
}

fn read_saved(path: &Path) -> Option<InstallRequest> {
    super::saved::load_beside(path)
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
        let st = Store::open(&d.path().join("p")).unwrap();
        let text: String = st.read(|c| Ok(c.query_row("SELECT req_json FROM paused_installs WHERE job = 'job2'", [], |r| r.get(0))?)).unwrap();
        assert!(!text.contains("SECRETKEY") && !text.contains("hunter2"), "{text}");
        let v: Value = serde_json::from_str(&text).unwrap();
        assert!(v["license_key"].is_null(), "{text}");
        // no secret in the database FILE either (the key was never written, so not even a deleted copy is left)
        for f in std::fs::read_dir(crate::store::store_dir(&d.path().join("p"))).unwrap().flatten().filter(|f| f.path().is_file()) {
            let b = std::fs::read(f.path()).unwrap();
            assert!(!b.windows(9).any(|w| w == b"SECRETKEY") && !b.windows(7).any(|w| w == b"hunter2"), "{}", f.path().display());
        }
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
        // Path tricks are refused, and so are Windows device names (`CON.json` is the console there).
        assert!(!s.save("../x", &req("/srv/c", Some("Main")), false, None, &[]).unwrap());
        for dev in ["CON", "nul", "Aux", "PRN", "com1", "LPT9"] {
            assert!(!s.save(dev, &req("/srv/c", Some("Main")), false, None, &[]).unwrap(), "{dev}");
            assert!(s.load(dev).is_none(), "{dev}");
        }
        assert!(valid_id("0123456789abcdef") && valid_id("com10") && valid_id("lpt0") && valid_id("console"));
        assert!(s.load("../a2").is_none());
        // Too old.
        let st = Store::open(d.path()).unwrap();
        st.write(|tx| {
            tx.execute("UPDATE paused_installs SET saved_at = 1 WHERE job = 'a2'", [])?;
            Ok(())
        })
        .unwrap();
        assert!(s.load("a2").is_none());
        let n: i64 = st.read(|c| Ok(c.query_row("SELECT COUNT(*) FROM paused_installs WHERE job = 'a2'", [], |r| r.get(0))?)).unwrap();
        assert_eq!(n, 0, "forgotten when seen");
    }

    #[test]
    fn saved_answers_follow_a_renamed_key_in_the_store() {
        let d = tempfile::tempdir().unwrap();
        let answers = d.path().join("last-install.json");
        let mut r = req("/srv/a", Some("Main"));
        r.license_key = None;
        super::super::saved::save_beside(&answers, &r).unwrap();
        assert!(saved_answers_use_key(&answers, "main"));
        assert!(rename_key_in_saved_answers(&answers, "Main", "Live"));
        assert!(saved_answers_use_key(&answers, "Live") && !saved_answers_use_key(&answers, "Main"));
        assert!(!rename_key_in_saved_answers(&d.path().join("elsewhere/none.json"), "a", "b"));
    }

    #[test]
    fn a_stopped_install_with_both_a_paused_entry_and_saved_answers_counts_once() {
        let d = tempfile::tempdir().unwrap();
        let s = PausedStore::at(d.path().join("p"));
        let saved = d.path().join("last-install.json");
        let mut a = req("/srv/a", Some("Main"));
        a.license_key = None;
        super::super::saved::save_beside(&saved, &a).unwrap();
        assert_eq!(s.count_key_with_saved(&saved, "main"), 1, "saved answers alone");
        s.save("j1", &req("/srv/a", Some("Main")), false, None, &[]).unwrap();
        assert_eq!(s.count_key_with_saved(&saved, "main"), 1, "the same install, paused and saved");
        s.save("j2", &req("/srv/b", Some("Main")), false, None, &[]).unwrap();
        assert_eq!(s.count_key_with_saved(&saved, "main"), 2, "another folder is another install");
        assert_eq!(s.count_key_with_saved(&saved, "other"), 0);
    }
}
