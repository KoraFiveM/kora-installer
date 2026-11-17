//! Answers from an install that failed, kept so the next run does not ask everything again.
//!
//! `<kora home>/last-install.json` holds the wizard's answers WITHOUT any secret: the license key is
//! removed and `DbAdmin::password` is never serialised. The step engine separately keeps what was
//! already done inside the server folder (`.kora/plan-state.json`), so running the install again
//! continues where it stopped.

use super::registry::kora_home;
use super::InstallRequest;
use std::path::PathBuf;

pub fn path() -> PathBuf {
    kora_home().join("last-install.json")
}

/// Remember `req` (without secrets). Best effort: a failure to save never hides the real error.
pub fn save(req: &InstallRequest) -> std::io::Result<PathBuf> {
    let mut r = req.clone();
    r.license_key = None;
    r.catalog = None;
    r.dry_run = false;
    let p = path();
    let text = serde_json::to_string_pretty(&r).map_err(std::io::Error::other)?;
    // Written to a fresh file and renamed over the old one (a crash never leaves half a file), and never
    // through a link: a failed install can run as root (through the helper) inside the user's own folder.
    crate::fsutil::write_no_follow(&p, text.as_bytes(), 0o644)?;
    Ok(p)
}

/// The saved answers, if there are any and they can be read.
pub fn load() -> Option<InstallRequest> {
    let text = std::fs::read_to_string(path()).ok()?;
    serde_json::from_str(&text).ok()
}

/// Forget the saved answers (after a successful install, or when the user starts over).
pub fn clear() {
    let _ = std::fs::remove_file(path());
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
        let p = save(&req()).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(!text.contains("SECRETKEY") && !text.contains("hunter2"), "{text}");
        let back = load().unwrap();
        assert_eq!(back.name, "Dev Den");
        assert_eq!(back.license_key, None);
        assert_eq!(back.db_admin.unwrap().password, None);
        clear();
        assert!(load().is_none());
    }
}
