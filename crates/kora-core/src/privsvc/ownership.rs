//! Which account owns which server folder, service, update task, firewall rule set and database,
//! and the rules for who may change them. Pure decisions; the OS layers supply the facts (a folder's
//! owner from the OS, read on the privileged side without following links).
//!
//! The record of what the service made for whom is the existing ownership ledger
//! ([`crate::helper::winhelper::Ledger`], `owners.json`): keys `service:kora-<slug>`,
//! `firewall:<slug>`, `task:<slug>`, `db:<name>`, value = the owning [`AccountId`]. This module only
//! adds the service's strict rules and a loader that does not mistake an unreadable file for an
//! empty one.

use std::path::Path;

use super::policy::AccountId;
use crate::helper::winhelper::{ledger_keys, Ledger};
use crate::helper::Rejected;

/// The ledger for the service: a missing file is an empty ledger, an unreadable or damaged one is
/// an error (the service then refuses to change anything rather than forgetting who owns what).
pub fn load(path: &Path) -> Result<Ledger, String> {
    match std::fs::read(path) {
        Ok(b) => serde_json::from_slice(&b).map_err(|e| format!("{} is damaged: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Ledger::default()),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

/// May `caller` use the server folder `dir`? `owner` = the folder's owner as the OS says (`None`:
/// could not be read), `has_record` = it holds Kora's install record, `extra_owners` = accounts
/// that legitimately own a Kora server folder besides the person (Linux: the `kora` service
/// account a server runs as). The folder must belong to the caller; there is no "administrators
/// group" exception (an administrator uses an elevated terminal for folders an elevated Kora made).
pub fn folder_ok(dir: &str, caller: &AccountId, owner: Option<&AccountId>, has_record: bool, extra_owners: &[AccountId]) -> Result<(), Rejected> {
    let no = |why: &str| Err(Rejected(format!("Kora's service will not work on {dir}: {why}.")));
    if !super::op::dir_shape_ok(dir) {
        return no("it is not a full folder path");
    }
    if !has_record {
        return no("it has no Kora install record");
    }
    match owner {
        Some(o) if o == caller || extra_owners.contains(o) => Ok(()),
        Some(_) => no("the folder does not belong to you"),
        None => no("cannot see who owns the folder"),
    }
}

/// What the service may do with the things behind `keys` for `caller`.
/// * making (`undo` = false): nothing in `keys` may be owned by someone else;
/// * removing (`undo` = true): everything must be recorded as the caller's. Things the ledger does
///   not know (made by an older Kora, or by hand) are never removed through the service: the
///   "Finish upgrading Kora" step has an administrator adopt them ([`adopt`]) first.
pub fn allow(ledger: &Ledger, keys: &[String], caller: &AccountId, undo: bool) -> Result<(), Rejected> {
    for k in keys {
        match ledger.owner(k) {
            Some(o) if o.eq_ignore_ascii_case(caller.as_str()) => {}
            Some(_) => return Err(Rejected(format!("Kora's service will not change {}: Kora made it for another account on this computer.", describe(k)))),
            None if undo => {
                return Err(Rejected(format!(
                    "Kora's service only removes what Kora made for your account, and its records do not show {} as yours (it may have been made by an older Kora). An administrator can finish upgrading Kora to fix that.",
                    describe(k)
                )))
            }
            None => {}
        }
    }
    Ok(())
}

/// Record (`made` = true) or forget the things behind `keys`.
pub fn note(ledger: &mut Ledger, keys: &[String], caller: &AccountId, made: bool) {
    for k in keys {
        if made {
            ledger.record(k.clone(), caller.as_str());
        } else {
            ledger.forget(k);
        }
    }
}

/// An administrator's migration: give things made before the service existed to `owner`, only where
/// no one owns them yet. Returns how many were adopted.
pub fn adopt(ledger: &mut Ledger, found: &[(String, AccountId)]) -> usize {
    let mut n = 0;
    for (k, o) in found {
        if ledger.owner(k).is_none() {
            ledger.record(k.clone(), o.as_str());
            n += 1;
        }
    }
    n
}

/// The keys an operation touches (the same ones the Windows steps used).
pub fn keys_for(step: super::op::StepId, slug: &str, db: Option<&str>) -> Vec<String> {
    ledger_keys(step.id(), slug, db)
}

fn describe(k: &str) -> String {
    match k.split_once(':') {
        Some(("service", n)) => format!("the service {n}"),
        Some(("firewall", n)) => format!("the firewall rules of {n}"),
        Some(("task", n)) => format!("the update task of {n}"),
        Some(("db", n)) => format!("the database {n}"),
        _ => k.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::op::StepId;
    use super::*;

    fn me() -> AccountId {
        AccountId::uid(1000)
    }
    fn you() -> AccountId {
        AccountId::uid(1001)
    }

    #[test]
    fn folders_must_belong_to_the_caller_and_hold_a_record() {
        let d = "/home/jo/kora/a";
        assert!(folder_ok(d, &me(), Some(&me()), true, &[]).is_ok());
        assert!(folder_ok(d, &me(), Some(&you()), true, &[]).unwrap_err().0.contains("does not belong"));
        assert!(folder_ok(d, &me(), Some(&AccountId::uid(0)), true, &[]).is_err(), "root-owned is not the person's");
        assert!(folder_ok(d, &me(), Some(&AccountId::uid(998)), true, &[AccountId::uid(998)]).is_ok(), "the server's own kora account");
        assert!(folder_ok(d, &me(), None, true, &[]).is_err());
        assert!(folder_ok(d, &me(), Some(&me()), false, &[]).unwrap_err().0.contains("no Kora install record"));
        assert!(folder_ok("rel/a", &me(), Some(&me()), true, &[]).is_err());
        assert!(folder_ok("/home/jo/../root", &me(), Some(&me()), true, &[]).is_err());
        let sid = AccountId::sid("S-1-5-21-1-2-3-1001").unwrap();
        assert!(folder_ok(r"C:\Users\jo\kora\a", &sid, Some(&sid), true, &[]).is_ok());
        assert!(folder_ok(r"C:\Users\jo\kora\a", &sid, Some(&AccountId::sid("S-1-5-21-1-2-3-1002").unwrap()), true, &[]).is_err());
    }

    #[test]
    fn making_needs_nobody_elses_and_removing_needs_yours() {
        let mut l = Ledger::default();
        let k = keys_for(StepId::Service, "main", None);
        assert_eq!(k, ["service:kora-main"]);
        assert!(allow(&l, &k, &me(), false).is_ok(), "new");
        assert!(allow(&l, &k, &me(), true).is_err(), "unrecorded: not removed through the service");
        note(&mut l, &k, &me(), true);
        assert!(allow(&l, &k, &me(), false).is_ok() && allow(&l, &k, &me(), true).is_ok());
        assert!(allow(&l, &k, &you(), false).unwrap_err().0.contains("another account"));
        assert!(allow(&l, &k, &you(), true).is_err());
        note(&mut l, &k, &me(), false);
        assert!(l.owner("service:kora-main").is_none());
    }

    #[test]
    fn databases_and_firewall_and_task_keys() {
        assert_eq!(keys_for(StepId::BuildDb, "a", Some("kora_ab")), ["db:kora_ab"]);
        assert!(keys_for(StepId::BuildDb, "a", None).is_empty());
        assert_eq!(keys_for(StepId::Firewall, "a", None), ["firewall:a"]);
        assert_eq!(keys_for(StepId::Schedule, "a", None), ["task:a"]);
        assert!(keys_for(StepId::PrepareDb, "a", None).is_empty());
    }

    #[test]
    fn adopting_never_takes_over() {
        let mut l = Ledger::default();
        l.record("service:kora-a".into(), you().as_str());
        let n = adopt(&mut l, &[("service:kora-a".into(), me()), ("task:a".into(), me())]);
        assert_eq!(n, 1);
        assert_eq!(l.owner("service:kora-a"), Some(you().as_str()));
        assert_eq!(l.owner("task:a"), Some(me().as_str()));
    }

    #[test]
    fn a_damaged_ledger_is_an_error_not_an_empty_one() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("owners.json");
        assert!(load(&p).unwrap().owners.is_empty(), "missing = empty");
        std::fs::write(&p, "{ nope").unwrap();
        assert!(load(&p).is_err());
        std::fs::write(&p, r#"{"owners":{"task:a":"uid:1000"}}"#).unwrap();
        assert_eq!(load(&p).unwrap().owner("task:a"), Some("uid:1000"));
    }
}
