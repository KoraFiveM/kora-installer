//! The operating-system side of [`super::migrate`]: gather the facts (from the system, never trusting a
//! person's file for more than naming a folder that is then checked), apply the pure decisions.
//! Run as administrator / root, through `kora service migrate`.

use std::path::{Path, PathBuf};

use super::migrate::{adoption, Candidate};
use super::policy::{AccountId, Policy};
use crate::privsvc::ownership::Ledger;

/// The system tools the migration asks (`schtasks`, `sc`, `reg`): quick questions and small changes, run by the MSI's
/// custom action where nobody can answer or wait, so each one has a short limit (and no input).
#[cfg(windows)]
struct Quick;

#[cfg(windows)]
impl Quick {
    const LIMIT: std::time::Duration = std::time::Duration::from_secs(60);

    fn run(&self, program: &str, args: &[String]) -> std::io::Result<crate::service::CommandOutput> {
        let mut cmd = crate::sysexe::command(program);
        cmd.args(args);
        let out = crate::sysexe::output_within(cmd, Self::LIMIT)?;
        Ok(crate::service::CommandOutput {
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

struct Listed {
    name: String,
    dir: PathBuf,
}

/// What an account looks like to the migration.
///
/// The server list is the one thing read from a person's Kora home on the privileged side, only here and only
/// to find folders to look at (each one is then checked from the system: owner, links, what exists). It is read with
/// [`crate::store::peek`], which opens the person's store read-only and immutable and never creates or changes a file
/// (root's `-wal` / `-shm` there would lock the person out). A missing list means "nothing to adopt", never an error; the migration runs when the
/// package or the MSI is installed, before the store has converted anything.
#[derive(Debug, Clone)]
pub struct Person {
    pub id: AccountId,
    /// Their home folder (Windows: profile folder).
    pub home: PathBuf,
    /// Their Kora folder (`~/.kora`, `%LOCALAPPDATA%\Kora`): the server list is read from its store without writing, see
    /// [`crate::store::peek`].
    pub kora_home: PathBuf,
}

/// The candidates for adoption: for each person, each server on their list, with what is found on the system.
///
/// * `owner_of(dir)`: the folder's owner, links not followed (`None`: unreadable, a link on the way, not a folder);
/// * `is_server_account(id)`: one of Kora's dedicated server accounts;
/// * `exists(kind, slug)`: does the system have `service`, `firewall` or `task` for that server?
pub fn gather(
    people: &[Person],
    owner_of: &dyn Fn(&Path) -> Option<AccountId>,
    is_server_account: &dyn Fn(&AccountId) -> bool,
    exists: &dyn Fn(&str, &str) -> bool,
    require_under_home: bool,
) -> Vec<Candidate> {
    let mut out = Vec::new();
    for p in people {
        for (name, dir) in crate::store::peek::registered_server_folders(&p.kora_home) {
            let e = Listed { name, dir };
            let slug = crate::installer::slug(&e.name);
            let owner = owner_of(&e.dir);
            let owner_is_account = owner.as_ref() == Some(&p.id);
            let owner_is_server_account = owner.as_ref().is_some_and(|o| o != &p.id && is_server_account(o));
            let under_home = !require_under_home || e.dir.starts_with(&p.home);
            let created = crate::installer::load_created_from(&e.dir).unwrap_or_default();
            let database = crate::db::ProvisionState::load(&crate::installer::meta_dir(&e.dir).join("db-provision.json")).ok().map(|s| s.database);
            out.push(Candidate {
                account: p.id.clone(),
                name: e.name,
                under_home,
                owner_is_account,
                owner_is_server_account,
                service_exists: created.service_unit.is_some() && exists("service", &slug),
                firewall_exists: created.firewall && exists("firewall", &slug),
                task_exists: created.update_timer && exists("task", &slug),
                database,
            });
        }
    }
    out
}

/// Adopt into the ledger file; returns how many entries were added.
pub fn adopt_into(ledger_path: &Path, found: &[(String, AccountId)]) -> Result<usize, String> {
    let mut l: Ledger = super::ownership::load(ledger_path)?;
    let n = super::ownership::adopt(&mut l, found);
    if n > 0 {
        l.save(ledger_path).map_err(|e| e.to_string())?;
    }
    Ok(n)
}

/// The whole adoption for the accounts on the list.
pub fn adopt_for(
    policy: &Policy,
    ledger_path: &Path,
    people: &[Person],
    owner_of: &dyn Fn(&Path) -> Option<AccountId>,
    is_server_account: &dyn Fn(&AccountId) -> bool,
    exists: &dyn Fn(&str, &str) -> bool,
    require_under_home: bool,
) -> Result<Vec<(String, AccountId)>, String> {
    let wanted: Vec<Person> = people.iter().filter(|p| policy.contains(&p.id)).cloned().collect();
    let cands = gather(&wanted, owner_of, is_server_account, exists, require_under_home);
    let ledger = super::ownership::load(ledger_path)?;
    let found = adoption(&cands, &ledger);
    adopt_into(ledger_path, &found)?;
    Ok(found)
}

// ---- Linux ---------------------------------------------------------------------------------------------------

#[cfg(unix)]
pub mod unix {
    use super::*;

    const OLD_UNIT_NAME: &str = "kora-helper.service";
    const OLD_UNIT_PATH: &str = "/etc/systemd/system/kora-helper.service";
    const OLD_CONFIG_PATH: &str = "/etc/kora/helper.json";
    const OLD_PROGRAM: &str = "/usr/local/libexec/kora/kora";

    /// What removes the old helper: stop and disable its unit, take away the unit, its copy of Kora and its settings.
    fn old_helper_removal() -> crate::service::Plan {
        use crate::service::{Plan, SystemStep};
        Plan::new(vec![
            SystemStep::run("Stop the old helper", "systemctl", &["disable", "--now", OLD_UNIT_NAME]).tolerant(),
            SystemStep::remove("Remove the old helper service", OLD_UNIT_PATH),
            SystemStep::run("Tell systemd the old service is gone", "systemctl", &["daemon-reload"]).tolerant(),
            SystemStep::remove("Remove the old helper's copy of Kora", OLD_PROGRAM),
            SystemStep::remove_dir_if_empty("Remove the old helper's folder", "/usr/local/libexec/kora"),
            SystemStep::remove("Remove the old helper settings", OLD_CONFIG_PATH),
        ])
    }
    use crate::privsvc::unixsvc::{Files, LinuxSvc};
    use std::os::unix::fs::MetadataExt;

    /// The old (v0.7.0) Linux helper's unit, copy and settings.
    pub fn old_helper_present() -> bool {
        Path::new(OLD_UNIT_PATH).exists() || Path::new(OLD_CONFIG_PATH).exists() || Path::new(OLD_PROGRAM).exists()
    }

    /// Everything the Linux migration does. `say` gets one line per thing done.
    pub fn migrate(files: &Files, dry_run: bool, say: &mut dyn FnMut(&str)) -> Result<(), String> {
        use crate::userhome::lookup_account;
        let mut policy = super::super::manage::load_or_new(&files.policy)?;
        // 1. The old helper: its account stays allowed; its unit, copy and settings go.
        if old_helper_present() {
            if let Some(id) = std::fs::read_to_string(OLD_CONFIG_PATH).ok().and_then(|t| super::super::migrate::old_helper_account(&t)) {
                if !policy.contains(&id) {
                    say(&format!("The account that used the old helper ({id}) may use the new service."));
                    if !dry_run {
                        let name = id.as_str().strip_prefix("uid:").and_then(|n| n.parse().ok()).and_then(lookup_account).map(|a| a.name).unwrap_or_default();
                        policy.allow(id, &name).map_err(|e| e.0)?;
                        super::super::manage::save(&files.policy, &policy).map_err(|e| e.to_string())?;
                    }
                }
            }
            say("Removing the old Kora helper (its service, its copy of Kora and its settings).");
            if !dry_run {
                crate::service::apply_plan(&old_helper_removal(), &crate::service::SystemRunner).map_err(|e| e.to_string())?;
            }
        }
        // 2. Things made before the ownership records existed: adopted for the accounts that can be shown to own them.
        let svc = LinuxSvc { exe: PathBuf::new(), files: files.clone(), strict_owner: false };
        let people: Vec<Person> = policy
            .accounts
            .iter()
            .filter_map(|a| {
                let acct = a.id.as_str().strip_prefix("uid:")?.parse().ok().and_then(lookup_account)?;
                let home = PathBuf::from(&acct.home);
                Some(Person { id: a.id.clone(), kora_home: home.join(".kora"), home })
            })
            .collect();
        let owner_of = |d: &Path| -> Option<AccountId> {
            if crate::privsvc::unixsvc::path_has_user_link(d).ok()? {
                return None;
            }
            let m = std::fs::symlink_metadata(d).ok().filter(|m| m.is_dir())?;
            Some(AccountId::uid(m.uid()))
        };
        let is_server = |id: &AccountId| crate::privsvc::dispatch::Host::is_server_account(&svc, id);
        let exists = |kind: &str, slug: &str| match kind {
            "service" => Path::new(&format!("/etc/systemd/system/kora-{slug}.service")).exists(),
            "task" => Path::new(&format!("/etc/systemd/system/kora-{slug}-update.timer")).exists(),
            _ => true, // a firewall rule set cannot be told cheaply; the folder checks and the record carry it
        };
        if dry_run {
            let cands = gather(&people, &owner_of, &is_server, &exists, true);
            let found = adoption(&cands, &super::super::ownership::load(&files.ledger)?);
            say(&format!("Would record who owns {} existing service(s), firewall rule set(s), update task(s) and database(s).", found.len()));
        } else {
            let found = adopt_for(&policy, &files.ledger, &people, &owner_of, &is_server, &exists, true)?;
            if !found.is_empty() {
                say(&format!("Recorded who owns {} existing service(s), firewall rule set(s), update task(s) and database(s): servers set up the old way stay manageable.", found.len()));
            }
        }
        Ok(())
    }
}

// ---- Windows -------------------------------------------------------------------------------------------------

#[cfg(windows)]
pub mod windows {
    use super::*;

    /// Update tasks that still run with the person's full administrator token become unelevated. Returns the names changed.
    pub fn unelevate_tasks(program_files: &str, dry_run: bool, say: &mut dyn FnMut(&str)) -> Result<Vec<String>, String> {
        let rn = Quick;
        let list = rn.run("schtasks.exe", &["/Query".into(), "/FO".into(), "CSV".into(), "/NH".into()]).map_err(|e| e.to_string())?;
        let mut changed = Vec::new();
        for line in list.stdout.lines() {
            let name = line.split("\",\"").next().unwrap_or("").trim_start_matches('"').to_string();
            if !super::super::migrate::is_update_task(&name) {
                continue;
            }
            let Ok(q) = rn.run("schtasks.exe", &["/Query".into(), "/TN".into(), name.clone(), "/XML".into()]) else { continue };
            let Some(fixed) = q.success().then(|| super::super::migrate::unelevate_task_xml(&q.stdout)).flatten() else { continue };
            say(&format!("The update task {name} no longer runs with full administrator rights."));
            if !dry_run {
                // a definition only administrators can change (never a user's folder), then replace the task
                let dir = Path::new(program_files).join("Kora").join("tasks");
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let file = dir.join(format!("migrate-{}.xml", changed.len()));
                std::fs::write(&file, &fixed).map_err(|e| e.to_string())?;
                let r = rn.run("schtasks.exe", &["/Create".into(), "/TN".into(), name.clone(), "/XML".into(), file.to_string_lossy().into_owned(), "/F".into()]);
                let _ = std::fs::remove_file(&file);
                match r {
                    Ok(o) if o.success() => changed.push(name),
                    Ok(o) => say(&format!("note: could not rewrite {name}: {}", o.stderr.trim())),
                    Err(e) => say(&format!("note: could not rewrite {name}: {e}")),
                }
            } else {
                changed.push(name);
            }
        }
        Ok(changed)
    }

    /// Adoption for the accounts on the list: folders owned by the account, services / rules / tasks that exist.
    pub fn adopt(policy: &Policy, ledger: &Path, dry_run: bool, say: &mut dyn FnMut(&str)) -> Result<(), String> {
        let people: Vec<Person> = policy
            .accounts
            .iter()
            .filter_map(|a| {
                let home = PathBuf::from(crate::privsvc::winapi::profile_dir(a.id.as_str())?);
                Some(Person { id: a.id.clone(), kora_home: home.join("AppData").join("Local").join("Kora"), home })
            })
            .collect();
        let owner_of = |d: &Path| -> Option<AccountId> {
            crate::fsutil::check_links(d).ok()?;
            let o = crate::privsvc::winapi::path_owner_sid(d)?;
            AccountId::sid(&o).ok()
        };
        let rn = Quick;
        let exists = |kind: &str, slug: &str| match kind {
            "service" => rn.run("sc.exe", &["query".into(), format!("kora-{slug}")]).is_ok_and(|o| o.success()),
            "task" => rn.run("schtasks.exe", &["/Query".into(), "/TN".into(), format!("Kora\\Update {slug}")]).is_ok_and(|o| o.success()),
            _ => true,
        };
        if dry_run {
            let cands = gather(&people, &owner_of, &|_| false, &exists, false);
            let found = adoption(&cands, &super::super::ownership::load(ledger)?);
            say(&format!("Would record who owns {} existing service(s), firewall rule set(s), update task(s) and database(s).", found.len()));
            return Ok(());
        }
        let found = adopt_for(policy, ledger, &people, &owner_of, &|_| false, &exists, false)?;
        if !found.is_empty() {
            say(&format!("Recorded who owns {} existing service(s), firewall rule set(s), update task(s) and database(s): servers set up the old way stay manageable.", found.len()));
        }
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    fn me() -> AccountId {
        let d = tempfile::tempdir().unwrap();
        AccountId::uid(std::fs::metadata(d.path()).unwrap().uid())
    }

    #[test]
    fn a_missing_server_list_adopts_nothing_and_is_not_an_error() {
        let d = tempfile::tempdir().unwrap();
        let person = Person { id: me(), home: d.path().to_path_buf(), kora_home: d.path().join(".kora") };
        let none = gather(&[person], &|_| None, &|_| false, &|_, _| true, true);
        assert!(none.is_empty(), "the store may have moved servers.json away");
    }

    #[test]
    fn servers_on_a_persons_list_are_adopted_only_for_what_exists_and_is_theirs() {
        let home = tempfile::tempdir().unwrap();
        let home_path = std::fs::canonicalize(home.path()).unwrap();
        let srv = home_path.join("kora").join("main");
        std::fs::create_dir_all(srv.join(".kora")).unwrap();
        let created = crate::installer::Created { service_unit: Some("/etc/systemd/system/kora-main.service".into()), firewall: true, ..Default::default() };
        std::fs::write(srv.join(".kora/created.json"), serde_json::to_string(&created).unwrap()).unwrap();
        let db = crate::db::ProvisionState { database: "kora_ab12".into(), user: "kora_ab12".into(), host: "127.0.0.1".into(), created_database: true, created_user: true, granted: true };
        std::fs::write(srv.join(".kora/db-provision.json"), serde_json::to_string(&db).unwrap()).unwrap();
        let reg = home_path.join(".kora").join("servers.json");
        std::fs::create_dir_all(reg.parent().unwrap()).unwrap();
        let entry = crate::installer::registry::ServerEntry {
            name: "Main".into(),
            dir: srv.clone(),
            mode: crate::installer::Mode::Production,
            source: None,
            versions: None,
            ports: crate::installer::registry::PortsRecord { game: 30120, txadmin: 40120 },
            created_unix: 1,
        };
        crate::installer::registry::Registry { servers: vec![entry] }.save_to(&reg).unwrap();
        let id = me();
        let mut policy = Policy::new("t");
        policy.allow(id.clone(), "me").unwrap();
        let people = vec![Person { id: id.clone(), home: home_path.clone(), kora_home: reg.parent().unwrap().to_path_buf() }];
        let ledger = home_path.join("owners.json");
        let owner_of = |d: &Path| std::fs::symlink_metadata(d).ok().map(|m| AccountId::uid(m.uid()));
        let exists = |kind: &str, _: &str| kind != "service"; // the service is already gone from this machine
        let found = adopt_for(&policy, &ledger, &people, &owner_of, &|_| false, &exists, true).unwrap();
        let keys: Vec<&str> = found.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["db:kora_ab12", "firewall:main"], "a service that does not exist is not claimed");
        assert_eq!(super::super::ownership::load(&ledger).unwrap().owner("firewall:main"), Some(id.as_str()));
        // again: nothing changes
        assert!(adopt_for(&policy, &ledger, &people, &owner_of, &|_| false, &exists, true).unwrap().is_empty());
        // an account that is not on the list is never adopted for
        let other = Policy::new("t");
        assert!(adopt_for(&other, &home_path.join("o2.json"), &people, &owner_of, &|_| false, &exists, true).unwrap().is_empty());
        // a folder outside the person's home is not theirs to claim
        let outside = vec![Person { home: PathBuf::from("/nonexistent-home"), ..people[0].clone() }];
        assert!(adopt_for(&policy, &home_path.join("o3.json"), &outside, &owner_of, &|_| false, &exists, true).unwrap().is_empty());
    }
}

// ---- the person's own copy (no administrator rights) ---------------------------------------------------------

/// `reg query HKCU\Environment /v Path` output -> (type, value).
pub fn parse_reg_query(out: &str) -> Option<(String, String)> {
    let line = out.lines().find(|l| l.trim_start().to_ascii_lowercase().starts_with("path") && l.contains("REG_"))?;
    let (_, rest) = line.trim().split_once("REG_")?;
    let (kind, value) = rest.split_once(char::is_whitespace)?;
    Some((format!("REG_{kind}"), value.trim().to_string()))
}

/// Where the person's own earlier copy of the program was put by the per-user installers.
pub fn own_copy_path() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(|l| PathBuf::from(l).join("Kora").join("bin").join("kora.exe"))
    } else {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("bin").join("kora"))
    }
    .filter(|p| p.is_file())
}

/// Finish upgrading for this person: set the earlier copy aside, drop it from PATH, re-point the background
/// panel. Only when the running program is the installed one. Returns the lines to show.
pub fn finish_upgrade_user(dry_run: bool) -> Vec<String> {
    let mut said = Vec::new();
    let Ok(me) = std::env::current_exe() else { return said };
    let pf = if cfg!(windows) { Some(crate::privsvc::winapi::system_dirs().program_files) } else { None };
    if !super::paths::program_location_ok(&me.to_string_lossy(), cfg!(windows), pf.as_deref()) {
        return said;
    }
    let plan = super::migrate::finish_plan(own_copy_path().as_deref(), &me, cfg!(windows));
    let Some(own) = plan.set_aside.clone() else { return said };
    said.push(format!("Your earlier copy of Kora ({}) is set aside as .previous; the installed Kora is used from now on.", own.display()));
    if dry_run {
        return said;
    }
    if let Some(from) = &plan.repoint_from {
        crate::app_install::user::repoint_background_panel(from, &me);
    }
    #[cfg(windows)]
    if let Some(dir) = &plan.drop_path {
        let rn = Quick;
        if let Ok(o) = rn.run("reg.exe", &["query".into(), r"HKCU\Environment".into(), "/v".into(), "Path".into()]) {
            if let Some((kind, value)) = parse_reg_query(&o.stdout) {
                if let Some(fixed) = super::migrate::remove_path_entry(&value, &dir.to_string_lossy()) {
                    let _ = rn.run("reg.exe", &["add".into(), r"HKCU\Environment".into(), "/v".into(), "Path".into(), "/t".into(), kind, "/d".into(), fixed, "/f".into()]);
                    said.push("The earlier folder is off your PATH (new terminals pick that up).".into());
                }
            }
        }
    }
    let mut prev = own.clone().into_os_string();
    prev.push(".previous");
    let prev = PathBuf::from(prev);
    let _ = std::fs::remove_file(&prev);
    if let Err(e) = std::fs::rename(&own, &prev) {
        said.push(format!("note: could not set the earlier copy aside ({e}); it can be deleted by hand."));
    }
    said
}

#[cfg(test)]
mod reg_tests {
    use super::*;

    #[test]
    fn the_path_value_is_read_from_reg_query() {
        let out = "\r\nHKEY_CURRENT_USER\\Environment\r\n    Path    REG_EXPAND_SZ    C:\\Users\\Jo\\bin;C:\\Tools\r\n\r\n";
        assert_eq!(parse_reg_query(out), Some(("REG_EXPAND_SZ".into(), r"C:\Users\Jo\bin;C:\Tools".into())));
        assert_eq!(parse_reg_query("ERROR: The system was unable to find the specified registry key or value."), None);
        assert_eq!(parse_reg_query("    Path    REG_SZ    C:\\a b\\c").map(|x| x.1), Some(r"C:\a b\c".into()));
    }
}
