//! Moving v0.6.x / v0.7.0 installs to the service layout ("Finish upgrading Kora"), without touching
//! servers or data. Two parts, both safe to run again:
//!
//! * **Administrator part** (`kora service migrate`, run by the package's `postinst`, the MSI, or an
//!   administrator): the old Linux helper is stopped and removed and the account it served is allowed
//!   in the new service; update tasks that still run with the person's FULL administrator token
//!   (`HighestAvailable`, written by 0.6.x) are rewritten to the unelevated form; things Kora made
//!   before the ownership ledger existed are adopted into it ([`adoption`]) for exactly the accounts
//!   that can be shown to own them.
//! * **Per-user part** (`kora service finish-upgrade`, offered by the panel next to "Tidy up old
//!   shortcuts"): the person's own earlier copy of the program is set aside (kept as `.previous`),
//!   its PATH entry removed and the background panel's start-up file re-pointed to the installed program.
//!
//! The decisions here are pure and tested on every OS; the OS layers gather the facts.

use super::policy::AccountId;
use crate::helper::winhelper::Ledger;

// ---- update tasks ----------------------------------------------------------------------------------

/// A Windows update task definition rewritten so it no longer runs with the account's full administrator
/// token: `HighestAvailable` becomes `LeastPrivilege`. `None` when there is nothing to change (already
/// least privilege) or when the task runs as the system account (that is a different decision: the
/// person is told, the task is left as it is).
pub fn unelevate_task_xml(xml: &str) -> Option<String> {
    if !xml.contains("<RunLevel>HighestAvailable</RunLevel>") {
        return None;
    }
    let user = xml.split("<UserId>").nth(1).and_then(|r| r.split("</UserId>").next()).unwrap_or("").trim();
    if user.eq_ignore_ascii_case("SYSTEM") || user == "S-1-5-18" || user.eq_ignore_ascii_case("NT AUTHORITY\\SYSTEM") || user.is_empty() {
        return None;
    }
    Some(xml.replace("<RunLevel>HighestAvailable</RunLevel>", "<RunLevel>LeastPrivilege</RunLevel>"))
}

/// Is this the name of an update task Kora made (`\Kora\Update <name>`)?
pub fn is_update_task(name: &str) -> bool {
    let n = name.trim().trim_matches('"');
    n.strip_prefix("\\Kora\\Update ").or_else(|| n.strip_prefix("Kora\\Update ")).is_some_and(|rest| !rest.is_empty() && !rest.contains(['\\', '/']))
}

// ---- adoption --------------------------------------------------------------------------------------------

/// What the OS layer found for one server of one allowed account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub account: AccountId,
    pub name: String,
    /// The folder is inside the account's own home (links not followed): where a person can make a server folder.
    pub under_home: bool,
    /// Who owns the folder: the account itself, or one of Kora's dedicated server accounts.
    pub owner_is_account: bool,
    pub owner_is_server_account: bool,
    /// What exists on the system now for this server (asked from the system, not from the person's files).
    pub service_exists: bool,
    pub firewall_exists: bool,
    pub task_exists: bool,
    /// The database Kora made for it, when its record says so and the database exists.
    pub database: Option<String>,
}

/// The ledger entries to add: only for a folder the account can be shown to own, only for things that
/// exist now and that no one owns yet. Two accounts claiming the same name both get nothing (the
/// ambiguity is not guessed away), and an existing owner is never replaced.
pub fn adoption(cands: &[Candidate], ledger: &Ledger) -> Vec<(String, AccountId)> {
    let slug = crate::installer::slug;
    let mut out: Vec<(String, AccountId)> = Vec::new();
    let mut claims: std::collections::BTreeMap<String, Vec<&AccountId>> = Default::default();
    for c in cands.iter().filter(|c| c.under_home && (c.owner_is_account || c.owner_is_server_account)) {
        let s = slug(&c.name);
        let mut keys = Vec::new();
        if c.service_exists {
            keys.push(format!("service:kora-{s}"));
        }
        if c.firewall_exists {
            keys.push(format!("firewall:{s}"));
        }
        if c.task_exists {
            keys.push(format!("task:{s}"));
        }
        if let Some(d) = c.database.as_deref().filter(|d| d.starts_with("kora_")) {
            keys.push(format!("db:{d}"));
        }
        for k in keys {
            claims.entry(k).or_default().push(&c.account);
        }
    }
    for (k, who) in claims {
        let mut distinct: Vec<&AccountId> = who;
        distinct.dedup();
        if let [one] = distinct.as_slice() {
            if ledger.owner(&k).is_none() {
                out.push((k, (*one).clone()));
            }
        }
    }
    out
}

// ---- the old Linux helper -------------------------------------------------------------------------------

/// The account the old helper served (`/etc/kora/helper.json`'s uid): it stays allowed in the new service.
pub fn old_helper_account(helper_json: &str) -> Option<AccountId> {
    let uid = serde_json::from_str::<serde_json::Value>(helper_json).ok()?.get("uid")?.as_u64()?;
    let uid = u32::try_from(uid).ok().filter(|u| *u != 0 && *u != u32::MAX)?;
    Some(AccountId::uid(uid))
}

// ---- the person's own copy ----------------------------------------------------------------------------------

/// A user PATH value without `dir` (compared without case, `/` = `\`, trailing separators ignored).
/// `None` when it was not there.
pub fn remove_path_entry(path_value: &str, dir: &str) -> Option<String> {
    let norm = |s: &str| s.trim().replace('/', "\\").trim_end_matches('\\').to_lowercase();
    let want = norm(dir);
    let parts: Vec<&str> = path_value.split(';').collect();
    let kept: Vec<&str> = parts.iter().copied().filter(|p| norm(p) != want).collect();
    (kept.len() != parts.len()).then(|| kept.into_iter().filter(|p| !p.trim().is_empty()).collect::<Vec<_>>().join(";"))
}

/// What "finish upgrading" does for the person.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FinishPlan {
    /// Set the person's own copy aside as `<name>.previous`.
    pub set_aside: Option<std::path::PathBuf>,
    /// Re-point the background panel's start-up file from this program to the installed one.
    pub repoint_from: Option<std::path::PathBuf>,
    /// The folder to take off the person's PATH (Windows).
    pub drop_path: Option<std::path::PathBuf>,
}

impl FinishPlan {
    pub fn is_empty(&self) -> bool {
        *self == FinishPlan::default()
    }
}

/// The plan, given the person's own copy (if there is one), the installed program that is running
/// (never the same file) and whether this is Windows.
pub fn finish_plan(own_copy: Option<&std::path::Path>, installed: &std::path::Path, windows: bool) -> FinishPlan {
    let Some(own) = own_copy.filter(|o| *o != installed && super::op::dir_shape_ok(&o.to_string_lossy())) else { return FinishPlan::default() };
    FinishPlan {
        set_aside: Some(own.to_path_buf()),
        repoint_from: Some(own.to_path_buf()),
        drop_path: windows.then(|| own.to_string_lossy().rsplit_once(['\\', '/']).map(|(dir, _)| std::path::PathBuf::from(dir))).flatten(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const HIGH: &str = r#"<Task><Principals><Principal id="Author"><UserId>PC\jo</UserId><LogonType>InteractiveToken</LogonType><RunLevel>HighestAvailable</RunLevel></Principal></Principals></Task>"#;

    #[test]
    fn an_old_task_loses_its_administrator_token_but_a_system_task_is_left_alone() {
        let fixed = unelevate_task_xml(HIGH).unwrap();
        assert!(fixed.contains("<RunLevel>LeastPrivilege</RunLevel>") && !fixed.contains("HighestAvailable"));
        assert!(fixed.contains("<UserId>PC\\jo</UserId>"), "same account, same schedule");
        assert_eq!(unelevate_task_xml(&fixed), None, "already least privilege");
        let sys = HIGH.replace("PC\\jo", "S-1-5-18");
        assert_eq!(unelevate_task_xml(&sys), None, "the system account's task is a different decision");
        assert_eq!(unelevate_task_xml(&HIGH.replace("<UserId>PC\\jo</UserId>", "")), None, "no user: not guessed");
    }

    #[test]
    fn only_kora_update_tasks_are_touched() {
        assert!(is_update_task("\\Kora\\Update main") && is_update_task("Kora\\Update my-server"));
        for n in ["\\Kora\\", "\\Other\\Update main", "\\Kora\\Update ", "\\Kora\\Update a\\b", "\\Kora\\Update a/../b", "Update main", "\\Microsoft\\Windows\\Update"] {
            assert!(!is_update_task(n), "{n}");
        }
    }

    fn cand(acc: u32, name: &str) -> Candidate {
        Candidate {
            account: AccountId::uid(acc),
            name: name.into(),
            under_home: true,
            owner_is_account: true,
            owner_is_server_account: false,
            service_exists: true,
            firewall_exists: true,
            task_exists: false,
            database: Some("kora_ab12".into()),
        }
    }

    #[test]
    fn what_can_be_shown_to_be_theirs_is_adopted_and_nothing_else() {
        let got = adoption(&[cand(1000, "Main")], &Ledger::default());
        let keys: Vec<&str> = got.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["db:kora_ab12", "firewall:main", "service:kora-main"]);
        assert!(got.iter().all(|(_, a)| *a == AccountId::uid(1000)));
        // root-run installs: the folder belongs to the server's own account, inside the person's home
        let mut c = cand(1000, "Main");
        c.owner_is_account = false;
        c.owner_is_server_account = true;
        assert_eq!(adoption(&[c.clone()], &Ledger::default()).len(), 3);
        // a folder outside their home, or owned by someone else, gives nothing
        c.under_home = false;
        assert!(adoption(&[c], &Ledger::default()).is_empty());
        let mut other = cand(1000, "Main");
        other.owner_is_account = false;
        assert!(adoption(&[other], &Ledger::default()).is_empty());
    }

    #[test]
    fn an_existing_owner_stays_and_a_name_two_accounts_claim_is_not_guessed() {
        let mut l = Ledger::default();
        l.record("service:kora-main".into(), "uid:1001");
        let got = adoption(&[cand(1000, "Main")], &l);
        assert!(!got.iter().any(|(k, _)| k == "service:kora-main"), "never taken over");
        assert!(got.iter().any(|(k, _)| k == "firewall:main"));
        let both = adoption(&[cand(1000, "Main"), cand(1001, "Main")], &Ledger::default());
        assert!(both.is_empty(), "both claim it: nobody gets it: {both:?}");
        let mut forged = cand(1000, "Main");
        forged.database = Some("mysql".into());
        assert!(!adoption(&[forged], &Ledger::default()).iter().any(|(k, _)| k.starts_with("db:")), "only Kora's own database names");
    }

    #[test]
    fn the_old_helper_s_account_stays_allowed() {
        assert_eq!(old_helper_account(r#"{"uid":1000,"gid":1000,"user":"jo","home":"/home/jo","version":"0.7.0"}"#), Some(AccountId::uid(1000)));
        for bad in ["{}", "not json", r#"{"uid":0}"#, r#"{"uid":"1000"}"#, r#"{"uid":99999999999}"#] {
            assert_eq!(old_helper_account(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_path_entry_comes_off_without_case_or_slash_trouble() {
        assert_eq!(remove_path_entry(r"C:\Windows;C:\Users\Jo\AppData\Local\Kora\bin;C:\Tools", r"c:/users/jo/appdata/local/kora/bin/"), Some(r"C:\Windows;C:\Tools".to_string()));
        assert_eq!(remove_path_entry(r"C:\Windows;C:\Tools", r"C:\Kora"), None);
        assert_eq!(remove_path_entry(r"C:\Kora\bin", r"C:\Kora\bin"), Some(String::new()));
    }

    #[test]
    fn finishing_sets_the_own_copy_aside_only_when_it_is_another_file() {
        let own = Path::new(r"C:\Users\Jo\AppData\Local\Kora\bin\kora.exe");
        let sys = Path::new(r"C:\Program Files\Kora\kora.exe");
        let p = finish_plan(Some(own), sys, true);
        assert_eq!(p.set_aside.as_deref(), Some(own));
        assert_eq!(p.repoint_from.as_deref(), Some(own));
        assert_eq!(p.drop_path.as_deref(), Some(Path::new(r"C:\Users\Jo\AppData\Local\Kora\bin")));
        assert!(finish_plan(Some(sys), sys, true).is_empty(), "never the installed program itself");
        assert!(finish_plan(None, sys, true).is_empty());
        assert!(finish_plan(Some(Path::new("kora")), sys, true).is_empty(), "a relative path is not trusted");
        assert_eq!(finish_plan(Some(Path::new("/home/jo/.local/bin/kora")), Path::new("/usr/lib/kora/kora"), false).drop_path, None);
    }
}
