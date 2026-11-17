//! Answers from an install that failed, kept so the next run does not ask everything again.
//!
//! The store (`saved_answers`; `<kora home>/last-install.json` before v0.8) holds the wizard's answers WITHOUT any secret: the license key is
//! removed and `DbAdmin::password` is never serialised. The step engine separately keeps what was
//! already done inside the server folder (`.kora/plan-state.json`), so running the install again
//! continues where it stopped.

use super::registry::kora_home;
use super::InstallRequest;
use crate::store::{problem_to_io, Store};
use std::path::PathBuf;

/// Which Kora home the answers belong to (`<home>/last-install.json` was the file before v0.8; they are
/// in the store now and this path only names the home).
pub fn path() -> PathBuf {
    kora_home().join("last-install.json")
}

/// Remember `req` (without secrets). Best effort: a failure to save never hides the real error.
pub fn save(req: &InstallRequest) -> std::io::Result<PathBuf> {
    let p = path();
    save_beside(&p, req)?;
    Ok(p)
}

pub(crate) fn save_beside(file: &std::path::Path, req: &InstallRequest) -> std::io::Result<()> {
    let mut r = req.clone();
    r.license_key = None;
    r.catalog = None;
    r.dry_run = false;
    let text = serde_json::to_string(&r).map_err(std::io::Error::other)?;
    let s = Store::open_beside(file).map_err(problem_to_io)?;
    s.write(|tx| {
        tx.execute(
            "INSERT INTO saved_answers (id, req_json, saved_at) VALUES (1, ?1, ?2) ON CONFLICT(id) DO UPDATE SET req_json = excluded.req_json, saved_at = excluded.saved_at",
            rusqlite::params![text, crate::monitor::now_unix() as i64],
        )?;
        Ok(())
    })
    .map_err(problem_to_io)
}

/// The saved answers in the Kora home beside `file`, if there are any and they can be read.
pub(crate) fn load_beside(file: &std::path::Path) -> Option<InstallRequest> {
    let s = Store::open_existing_beside(file).ok()??;
    let text: String = s.read(|c| Ok(c.query_row("SELECT req_json FROM saved_answers WHERE id = 1", [], |r| r.get::<_, String>(0)).ok())).ok()??;
    serde_json::from_str(&text).ok()
}

/// The saved answers, if there are any and they can be read.
pub fn load() -> Option<InstallRequest> {
    load_beside(&path())
}

pub(crate) fn store_beside(file: &std::path::Path, r: &InstallRequest) -> bool {
    let Ok(text) = serde_json::to_string(r) else { return false };
    let Ok(s) = Store::open_beside(file) else { return false };
    s.write(|tx| {
        tx.execute("UPDATE saved_answers SET req_json = ?1 WHERE id = 1", [text])?;
        Ok(())
    })
    .is_ok()
}

/// Forget the saved answers (after a successful install, or when the user starts over).
pub fn clear() {
    if let Ok(Some(s)) = Store::open_existing_beside(&path()) {
        let _ = s.write(|tx| {
            tx.execute("DELETE FROM saved_answers", [])?;
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::installer::{DbAdmin, DbChoice, Mode, Ports, ResourceSelection};

    fn req() -> InstallRequest {
        InstallRequest {
            mode: Mode::Production,
            name: "Dev Den".into(),
            dir: "/tmp/dev-den".into(),
            hostname: "Dev Den".into(),
            license_key: Some("cfxk_SECRETKEY".into()),
            license_key_name: None,
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
    fn saves_answers_but_never_the_license_key_or_the_database_password() {
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("KORA_HOME", home.path());
        save(&req()).unwrap();
        // not a single secret anywhere in the database file or its write-ahead log
        let s = Store::open(home.path()).unwrap();
        let text: String = s.read(|c| Ok(c.query_row("SELECT req_json FROM saved_answers", [], |r| r.get(0))?)).unwrap();
        assert!(!text.contains("SECRETKEY") && !text.contains("hunter2"), "{text}");
        let back = load().unwrap();
        assert_eq!(back.name, "Dev Den");
        assert_eq!(back.license_key, None);
        assert_eq!(back.db_admin.unwrap().password, None);
        clear();
        assert!(load().is_none());
    }
}
