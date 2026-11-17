//! Handing a server folder to the account the server runs as, AFTER files the person wrote went into it (a restore, an
//! import): the same ownership the install gave a new server, so the background service can still read and write what
//! was just put there.
//!
//! * Linux: the folder is handed to the server's dedicated `kora-<name>` account with
//!   [`crate::fsutil::chown_tree_no_follow`] (through open folder handles, never by path, never through a link).
//! * Windows: the folder's inheritable `NT SERVICE\kora-<name>` rights and the explicit grants on the files that do not
//!   inherit (`secrets.cfg`, `database.json`, `txData\admins.json`) are applied again: a restore replaces those files.
//!
//! Threat model. This runs as root / SYSTEM in the administrator service's child, for a folder a person controls. So:
//! * the account is NEVER taken from the request or from the folder's own records as written: the folder's install record
//!   (`.kora/request.json`) gives the server's name, and the account is the one Kora makes for that name
//!   ([`service::account::account_name`]); a `created.json` that names another account makes the step refuse. A person
//!   who edits those files can therefore not choose who receives their files, or anything else's;
//! * the account must exist and be a system account (not root, below the ordinary users' range), the same rule the install
//!   applies, so a person's own account or `nobody` can never be the target;
//! * the folder is checked for links on the way first, and the walk itself does not follow any;
//! * nothing is read from the folder beyond the two small records.

use std::path::Path;

use super::{check_dir_links, load_created_from, load_request_from, slug};
use crate::service::{self, account, CommandRunner, ManagedServer, Plan, Platform, SystemStep};

/// What the step did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The server does not run under an account of its own: nothing to hand over.
    NotNeeded(String),
    Done(String),
}

/// The plan for `dir`, or why not. Pure apart from what `rn` and `ids` answer (tests fake both).
pub fn plan_for(dir: &Path, platform: Platform, rn: &dyn CommandRunner, ids: &dyn Fn(&str) -> Option<(u32, u32)>, uid_min: u32) -> Result<(Plan, Outcome), String> {
    let req = load_request_from(dir).ok_or("the folder has no Kora install record, so Kora does not know which account the server runs as")?;
    let created = load_created_from(dir).unwrap_or_default();
    let slug = slug(&req.name);
    let none = |why: &str| Ok((Plan::default(), Outcome::NotNeeded(why.to_string())));
    match platform {
        Platform::Linux => {
            if created.service_unit.is_none() && created.service_account.is_none() {
                return none("this server does not run as an account of its own");
            }
            let expected = account::account_name(&slug);
            if created.service_account.as_deref().is_some_and(|r| r != expected) {
                return Err(format!("the folder's records name a different account than the one Kora makes for this server ('{expected}'), so nothing was handed over"));
            }
            let uid = match account::user_status(rn, &expected).map_err(|e| e.to_string())? {
                account::UserStatus::Missing => return Err(format!("the server's account '{expected}' does not exist")),
                account::UserStatus::Exists(uid) if account::is_system_uid(uid, uid_min) => uid,
                account::UserStatus::Exists(_) => return Err(format!("'{expected}' is not a system account Kora made for this server, so nothing was handed over")),
            };
            let (uid, gid) = ids(&expected).filter(|(u, _)| *u == uid).ok_or_else(|| format!("cannot find the account '{expected}'"))?;
            let plan = Plan::new(vec![SystemStep::chown_tree(format!("Give '{expected}' ownership of the restored server files (and only the server folder)"), &dir.to_string_lossy(), uid, gid)]);
            Ok((plan, Outcome::Done(format!("The server's files now belong to '{expected}'."))))
        }
        Platform::Windows => {
            if created.service_unit.is_none() {
                return none("this server does not run as a Windows service");
            }
            let exe = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| "kora.exe".into());
            let server = ManagedServer {
                name: slug.clone(),
                dir: dir.to_string_lossy().into_owned(),
                start_command: vec![exe.clone()],
                user: "kora".into(),
                env_file: None,
                kora_exe: exe,
                windows_run_as: None,
            };
            server.validate().map_err(|e| e.to_string())?;
            let plan = service::windows_grant_plan(&server);
            Ok((plan, Outcome::Done(format!("The server's service account has its rights on the restored files again ({}).", service::virtual_account(&server.service_name())))))
        }
    }
}

/// Do it (as root / SYSTEM). The folder must be free of links first.
pub fn run(dir: &Path) -> Result<Outcome, String> {
    check_dir_links(dir).map_err(|e| e.to_string())?;
    let (plan, outcome) = plan_for(dir, Platform::current(), &service::SystemRunner, &|n| crate::userhome::account_ids(n), account::uid_min())?;
    service::apply_plan(&plan, &service::SystemRunner).map_err(|e| e.to_string())?;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::installer::Created;
    use crate::service::testing::FakeRunner;

    fn folder(created: Option<&Created>) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let meta = d.path().join(".kora");
        std::fs::create_dir_all(&meta).unwrap();
        let req = crate::installer::tests_request("My Server", d.path());
        std::fs::write(meta.join("request.json"), serde_json::to_vec(&req).unwrap()).unwrap();
        if let Some(c) = created {
            std::fs::write(meta.join("created.json"), serde_json::to_vec(c).unwrap()).unwrap();
        }
        d
    }
    fn ids(n: &str) -> Option<(u32, u32)> {
        (n == "kora-my-server").then_some((998, 997))
    }
    fn account_998() -> FakeRunner {
        let r = FakeRunner::default();
        r.respond("id -u kora-my-server", 0, "998\n");
        r
    }

    #[test]
    fn a_server_with_its_own_account_has_its_folder_handed_to_it_by_open_handles() {
        let c = Created { service_unit: Some("/etc/systemd/system/kora-my-server.service".into()), service_account: Some("kora-my-server".into()), ..Default::default() };
        let d = folder(Some(&c));
        let (plan, out) = plan_for(d.path(), Platform::Linux, &account_998(), &ids, 1000).unwrap();
        assert!(matches!(out, Outcome::Done(m) if m.contains("kora-my-server")));
        assert_eq!(plan.steps.len(), 1);
        let rn = FakeRunner::default();
        service::apply_plan(&plan, &rn).unwrap();
        assert_eq!(rn.calls(), [format!("chown -R -- 998:997 {}", d.path().display())], "the no-follow walk, once, for this folder only");
    }

    #[test]
    fn a_server_without_an_account_of_its_own_needs_nothing() {
        let d = folder(None);
        let (plan, out) = plan_for(d.path(), Platform::Linux, &FakeRunner::default(), &ids, 1000).unwrap();
        assert!(plan.steps.is_empty() && matches!(out, Outcome::NotNeeded(_)));
    }

    #[test]
    fn the_account_is_never_one_the_folders_own_records_choose() {
        let forged = Created { service_unit: Some("x".into()), service_account: Some("alice".into()), ..Default::default() };
        let d = folder(Some(&forged));
        let e = plan_for(d.path(), Platform::Linux, &account_998(), &ids, 1000).unwrap_err();
        assert!(e.contains("different account"), "{e}");
        // the right name, but it is an ordinary person's account or root
        let ok = Created { service_unit: Some("x".into()), service_account: Some("kora-my-server".into()), ..Default::default() };
        let d = folder(Some(&ok));
        for (uid, why) in [("1000", "not a system account"), ("0", "not a system account")] {
            let r = FakeRunner::default();
            r.respond("id -u kora-my-server", 0, &format!("{uid}\n"));
            let e = plan_for(d.path(), Platform::Linux, &r, &|_| Some((uid.parse().unwrap(), 1)), 1000).unwrap_err();
            assert!(e.contains(why), "{uid}: {e}");
        }
        let gone = FakeRunner::default();
        gone.respond("id -u", 1, "");
        assert!(plan_for(d.path(), Platform::Linux, &gone, &ids, 1000).unwrap_err().contains("does not exist"));
    }

    #[test]
    fn a_folder_without_an_install_record_is_refused() {
        let d = tempfile::tempdir().unwrap();
        assert!(plan_for(d.path(), Platform::Linux, &FakeRunner::default(), &ids, 1000).unwrap_err().contains("no Kora install record"));
    }

    #[test]
    fn windows_applies_the_services_rights_again() {
        let c = Created { service_unit: Some("kora-my-server".into()), ..Default::default() };
        let d = folder(Some(&c));
        let (plan, out) = plan_for(d.path(), Platform::Windows, &FakeRunner::default(), &ids, 1000).unwrap();
        let lines: Vec<String> = plan.steps.iter().filter_map(|s| s.command_line()).collect();
        assert!(lines.iter().all(|l| l.starts_with("icacls.exe ") && l.contains("NT SERVICE\\kora-my-server")), "{lines:?}");
        assert!(lines.iter().any(|l| l.contains("secrets.cfg")) && lines.iter().any(|l| l.contains("database.json")) && lines.iter().any(|l| l.contains("admins.json")));
        assert!(matches!(out, Outcome::Done(_)));
        let none = folder(None);
        assert!(plan_for(none.path(), Platform::Windows, &FakeRunner::default(), &ids, 1000).unwrap().0.steps.is_empty());
    }
}
