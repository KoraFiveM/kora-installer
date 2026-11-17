//! The service's decision pipeline for ONE request, shared by the Linux and the Windows service:
//!
//! 1. [`super::policy::authorise`] the caller (identity from the OS connection);
//! 2. [`super::op::parse`] the request into a typed [`Op`] (strict allow-list);
//! 3. re-derive and check everything the caller could have forged, on the privileged side, through
//!    [`Host`] (the only place that touches the OS): the folder is the caller's and holds an install
//!    record; the ports come from the folder's install record read here ([`super::op::check_context`]);
//!    the ledger allows the caller to make / remove what the step touches;
//! 4. run the child with arguments made from the typed value, under the operation's time limit;
//! 5. update the ledger when the child succeeded, and write ONE audit line whatever happened.
//!
//! The caller of [`handle`] holds the service's one-at-a-time lock around it.

use std::collections::BTreeMap;

use serde_json::Value;

use super::audit::{self, Outcome};
use super::op::{self, Op, StepId};
use super::ownership;
use super::policy::{self, AccountId, Caller, Policy};
use crate::privsvc::ownership::Ledger;
use crate::privsvc::wire::Rejected;

/// What the service needs from the operating system. Everything the caller could influence is
/// re-read through this, on the privileged side (implementations never follow links).
pub trait Host {
    fn now(&self) -> u64;
    /// The account name for the audit log (best effort, may be empty).
    fn account_name(&self, id: &AccountId) -> String;
    /// Owner of the folder; `Err` when a link is on the way or it cannot be read.
    fn folder_owner(&self, dir: &str) -> Result<Option<AccountId>, String>;
    /// Is this one of the dedicated accounts Kora makes for a server to run as (Linux: `kora`,
    /// `kora-<name>`; made by root, so a user cannot create one)? A folder owned by such an account
    /// is the caller's only when the ledger says the server's service is theirs.
    fn is_server_account(&self, id: &AccountId) -> bool;
    /// The folder's install record: the server's name and ports.
    fn install_record(&self, dir: &str) -> Option<InstallRecord>;
    /// The name of the database Kora made for this server, if it did.
    fn db_name(&self, dir: &str) -> Option<String>;
    fn load_ledger(&self) -> Result<Ledger, String>;
    fn save_ledger(&self, l: &Ledger) -> Result<(), String>;
    fn audit(&self, line: &str);
    fn own_version(&self) -> String;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallRecord {
    pub name: String,
    pub game_port: u16,
    pub txadmin_port: u16,
}

/// What a child produced.
#[derive(Debug, Clone, PartialEq)]
pub struct Ran {
    pub code: i32,
    pub timed_out: bool,
    /// The step's `KORA-ADMIN-RESULT` data.
    pub data: Option<Value>,
}

/// What to run: the typed operation's arguments, the request's secret environment, the time limit.
pub struct RunSpec<'a> {
    pub argv: &'a [String],
    pub env: &'a BTreeMap<String, String>,
    pub limit: std::time::Duration,
}

/// Progress callback: `(stream, line)`.
pub type Progress<'a> = &'a mut dyn FnMut(&str, &str);

/// Runs the child, streaming its output lines to the progress callback.
pub type Run<'a> = &'a dyn Fn(&RunSpec<'_>, Progress<'_>) -> std::io::Result<Ran>;

/// How the request ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    Refused(Rejected),
    Ran(Ran),
    /// The child could not be started.
    CouldNotRun(String),
}

/// Handle one request. See the module documentation.
#[allow(clippy::too_many_arguments)]
pub fn handle(host: &dyn Host, policy: &Result<Policy, String>, caller: &Caller, verb: &str, args: &[String], env: &BTreeMap<String, String>, run: Run<'_>, progress: Progress<'_>) -> Reply {
    let who = caller.id.clone().or_else(|| caller.is_root.then(|| AccountId::uid(0)));
    let name = who.as_ref().map(|i| host.account_name(i)).unwrap_or_default();
    let log = |what: &str, o: Outcome| host.audit(&audit::line(host.now(), who.as_ref(), &name, what, &o));
    if let Err(r) = policy::authorise(caller, policy) {
        log(verb_label(verb), Outcome::Refused("account not allowed"));
        return Reply::Refused(r);
    }
    // After authorise a caller always has an identity (root counts as uid 0).
    let Some(me) = who.clone() else { return Reply::Refused(Rejected("Kora's service could not see who is asking.".into())) };
    let operation = match op::parse(verb, args, env) {
        Ok(o) => o,
        Err(r) => {
            log(verb_label(verb), Outcome::Refused("request not allowed"));
            return Reply::Refused(r);
        }
    };
    let label = operation.label();
    let prepared = match prepare(host, &me, caller.is_root, &operation) {
        Ok(p) => p,
        Err(r) => {
            log(&label, Outcome::Refused("checks failed"));
            return Reply::Refused(r);
        }
    };
    let argv = operation.child_args();
    match run(&RunSpec { argv: &argv, env, limit: operation.time_limit() }, progress) {
        Err(e) => {
            log(&label, Outcome::Failed(-1));
            Reply::CouldNotRun(e.to_string())
        }
        Ok(ran) => {
            if ran.code == 0 && !ran.timed_out {
                if let Err(e) = commit(host, &me, &operation, &prepared, ran.data.as_ref()) {
                    // The thing was made but the record of it could not be written: say so loudly.
                    host.audit(&audit::line(host.now(), Some(&me), &name, &label, &Outcome::Failed(-2)));
                    return Reply::CouldNotRun(format!("The change was made but Kora's ownership record could not be saved: {e}"));
                }
            }
            log(
                &label,
                if ran.timed_out {
                    Outcome::TimedOut
                } else if ran.code == 0 {
                    Outcome::Done
                } else {
                    Outcome::Failed(ran.code)
                },
            );
            Reply::Ran(ran)
        }
    }
}

fn verb_label(verb: &str) -> &str {
    // Only a verb from our own list may appear in the log; anything else is not echoed.
    ["admin-step", "admin-db", "check", "tools", "schedule", "self-update"].into_iter().find(|v| *v == verb).unwrap_or("(unknown)")
}

/// What the checks found out, for the ledger update after the child ran.
#[derive(Debug, Default)]
struct Prepared {
    keys: Vec<String>,
    slug: String,
}

fn prepare(host: &dyn Host, me: &AccountId, is_root: bool, operation: &Op) -> Result<Prepared, Rejected> {
    let mut ports = None;
    let mut p = Prepared::default();
    match operation {
        Op::Step { undo, step, dir, known, .. } => {
            let owner = host.folder_owner(dir).map_err(|e| Rejected(format!("Kora's service will not work on {dir}: {e}.")))?;
            let rec = host.install_record(dir);
            // root may work on any folder with a record (root could do it all anyway); everyone else on their own.
            let by_server_account = owner.as_ref().is_some_and(|o| o != me && host.is_server_account(o));
            if is_root {
                if rec.is_none() {
                    return Err(Rejected(format!("Kora's service will not work on {dir}: it has no Kora install record.")));
                }
            } else {
                ownership::folder_ok(dir, me, owner.as_ref(), rec.is_some(), by_server_account)?;
            }
            // Undoing: the name comes from the person's server list (`known`), not from the folder's record.
            let name = match (undo, known) {
                (true, Some(k)) => k.name.clone(),
                (true, None) => rec.as_ref().map(|r| r.name.clone()).ok_or_else(|| Rejected("Kora's service needs the server's name to undo that.".into()))?,
                (false, _) => rec.as_ref().map(|r| r.name.clone()).ok_or_else(|| Rejected("The server's install record has no name.".into()))?,
            };
            ports = rec.map(|r| (r.game_port, r.txadmin_port));
            p.slug = crate::installer::slug(&name);
            let db = host.db_name(dir);
            p.keys = ownership::keys_for(*step, &p.slug, db.as_deref());
            // A folder handed to the server's own account (a root-run install, or this service's earlier
            // run) is the caller's only when the ledger says the server's service is theirs.
            if by_server_account && !is_root {
                let ledger = host.load_ledger().map_err(|e| Rejected(format!("Kora's service cannot read its ownership records: {e}")))?;
                let key = format!("service:kora-{}", p.slug);
                if ledger.owner(&key).is_none_or(|o| !o.eq_ignore_ascii_case(me.as_str())) {
                    return Err(Rejected(format!("Kora's service will not work on {dir}: it belongs to the server's own account and Kora's records do not show it as yours. An administrator can finish upgrading Kora to fix that.")));
                }
            }
        }
        Op::ScheduleSet { server, .. } | Op::ScheduleOff { server } => {
            p.slug = crate::installer::slug(server);
            p.keys = ownership::keys_for(StepId::Schedule, &p.slug, None);
        }
        _ => {}
    }
    op::check_context(operation, &host.own_version(), ports)?;
    if !p.keys.is_empty() && !is_root {
        let ledger = host.load_ledger().map_err(|e| Rejected(format!("Kora's service cannot read its ownership records: {e}")))?;
        let undo = matches!(operation, Op::Step { undo: true, .. } | Op::ScheduleOff { .. });
        ownership::allow(&ledger, &p.keys, me, undo)?;
    }
    Ok(p)
}

fn commit(host: &dyn Host, me: &AccountId, operation: &Op, p: &Prepared, data: Option<&Value>) -> Result<(), String> {
    let (made, step) = match operation {
        Op::Step { undo, step, .. } => (!undo, Some(*step)),
        Op::ScheduleSet { .. } => (true, Some(StepId::Schedule)),
        Op::ScheduleOff { .. } => (false, Some(StepId::Schedule)),
        _ => return Ok(()),
    };
    let Some(step) = step else { return Ok(()) };
    let mut ledger = host.load_ledger()?;
    let mut keys = p.keys.clone();
    if made && step == StepId::BuildDb {
        // The database name is only known after it was made.
        if let Some(d) = data.and_then(|d| d.pointer("/db/state/database")).and_then(Value::as_str).filter(|d| d.starts_with("kora_")) {
            keys = ownership::keys_for(step, &p.slug, Some(d));
        }
    }
    ownership::note(&mut ledger, &keys, me, made);
    host.save_ledger(&ledger)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Fake {
        owner: Option<AccountId>,
        record: Option<InstallRecord>,
        ledger: RefCell<Ledger>,
        log: RefCell<Vec<String>>,
        db: Option<String>,
    }
    fn me() -> AccountId {
        AccountId::uid(1000)
    }
    fn fake() -> Fake {
        Fake { owner: Some(me()), record: Some(InstallRecord { name: "Main".into(), game_port: 30120, txadmin_port: 40120 }), ledger: Default::default(), log: Default::default(), db: None }
    }
    impl Host for Fake {
        fn now(&self) -> u64 {
            0
        }
        fn account_name(&self, _: &AccountId) -> String {
            "jo".into()
        }
        fn folder_owner(&self, dir: &str) -> Result<Option<AccountId>, String> {
            if dir.contains("link") {
                return Err("a link is on the way".into());
            }
            Ok(self.owner.clone())
        }
        fn is_server_account(&self, id: &AccountId) -> bool {
            *id == AccountId::uid(998)
        }
        fn install_record(&self, _: &str) -> Option<InstallRecord> {
            self.record.clone()
        }
        fn db_name(&self, _: &str) -> Option<String> {
            self.db.clone()
        }
        fn load_ledger(&self) -> Result<Ledger, String> {
            Ok(self.ledger.borrow().clone())
        }
        fn save_ledger(&self, l: &Ledger) -> Result<(), String> {
            *self.ledger.borrow_mut() = l.clone();
            Ok(())
        }
        fn audit(&self, line: &str) {
            self.log.borrow_mut().push(line.to_string());
        }
        fn own_version(&self) -> String {
            "0.8.0".into()
        }
    }

    fn policy() -> Result<Policy, String> {
        let mut p = Policy::new("0.8.0");
        p.allow(me(), "jo").unwrap();
        p.allow(AccountId::uid(1001), "sam").unwrap();
        Ok(p)
    }
    fn caller(id: AccountId) -> Caller {
        Caller { id: Some(id), integrity: None, is_root: false, remote: false }
    }
    fn s(a: &[&str]) -> Vec<String> {
        a.iter().map(|x| x.to_string()).collect()
    }
    fn ok_run(data: Option<Value>) -> impl Fn(&RunSpec<'_>, Progress<'_>) -> std::io::Result<Ran> {
        move |_, _| Ok(Ran { code: 0, timed_out: false, data: data.clone() })
    }
    fn never(_: &RunSpec<'_>, _: Progress<'_>) -> std::io::Result<Ran> {
        panic!("the child must not run")
    }
    const DIR: &str = "/home/jo/kora/main";

    #[test]
    fn a_service_made_for_me_is_recorded_and_logged() {
        let h = fake();
        let r = handle(&h, &policy(), &caller(me()), "admin-step", &s(&["run", "--step", "system.service", "--dir", DIR]), &BTreeMap::new(), &ok_run(None), &mut |_, _| {});
        assert!(matches!(r, Reply::Ran(Ran { code: 0, .. })));
        assert_eq!(h.ledger.borrow().owner("service:kora-main"), Some("uid:1000"));
        let log = h.log.borrow();
        assert_eq!(log.len(), 1);
        assert!(log[0].contains("uid:1000 (jo) | run system.service | done"), "{log:?}");
        assert!(!log[0].contains("/home"), "no paths in the log");
    }

    #[test]
    fn someone_else_cannot_touch_my_things_and_the_child_never_runs() {
        let h = fake();
        h.ledger.borrow_mut().record("service:kora-main".into(), "uid:1000");
        // sam is allowed to use the service, but the folder is jo's
        let r = handle(&h, &policy(), &caller(AccountId::uid(1001)), "admin-step", &s(&["run", "--step", "system.service", "--dir", DIR]), &BTreeMap::new(), &never, &mut |_, _| {});
        assert!(matches!(&r, Reply::Refused(m) if m.0.contains("does not belong")), "{r:?}");
        // ...and even with a folder of his own, the service of that name is jo's
        let mut h2 = fake();
        h2.owner = Some(AccountId::uid(1001));
        h2.ledger.borrow_mut().record("service:kora-main".into(), "uid:1000");
        let r = handle(&h2, &policy(), &caller(AccountId::uid(1001)), "admin-step", &s(&["undo", "--step", "system.service", "--dir", DIR]), &BTreeMap::new(), &never, &mut |_, _| {});
        assert!(matches!(&r, Reply::Refused(m) if m.0.contains("another account")), "{r:?}");
        assert!(h2.log.borrow()[0].contains("refused"));
    }

    #[test]
    fn an_unlisted_account_is_refused_before_anything_is_parsed() {
        let h = fake();
        let r = handle(&h, &policy(), &caller(AccountId::uid(2000)), "admin-step", &s(&["run", "--step", "system.service", "--dir", DIR]), &BTreeMap::new(), &never, &mut |_, _| {});
        assert!(matches!(r, Reply::Refused(_)));
        assert!(h.log.borrow()[0].contains("account not allowed"));
        let r = handle(&h, &Err("bad".into()), &caller(me()), "check", &s(&["--start-database"]), &BTreeMap::new(), &never, &mut |_, _| {});
        assert!(matches!(r, Reply::Refused(_)), "unreadable settings fail closed");
    }

    #[test]
    fn the_ports_come_from_the_record_read_here() {
        let mut h = fake();
        h.record.as_mut().unwrap().game_port = 3389;
        let r = handle(&h, &policy(), &caller(me()), "admin-step", &s(&["run", "--step", "system.firewall", "--dir", DIR]), &BTreeMap::new(), &never, &mut |_, _| {});
        assert!(matches!(&r, Reply::Refused(m) if m.0.contains("3389")), "{r:?}");
        // closing is fine whatever the record says
        let r = handle(
            &h,
            &policy(),
            &caller(me()),
            "admin-step",
            &s(&["undo", "--step", "system.firewall", "--dir", DIR, "--name", "Main", "--game-port", "30120", "--txadmin-port", "40120"]),
            &BTreeMap::new(),
            &ok_run(None),
            &mut |_, _| {},
        );
        assert!(matches!(&r, Reply::Refused(m) if m.0.contains("not show")), "unrecorded rules are not removed through the service: {r:?}");
    }

    #[test]
    fn links_missing_records_and_junk_requests_are_refused() {
        let h = fake();
        let run = |dir: &str, h: &Fake| handle(h, &policy(), &caller(me()), "admin-step", &s(&["run", "--step", "system.service", "--dir", dir]), &BTreeMap::new(), &never, &mut |_, _| {});
        assert!(matches!(run("/home/jo/link/main", &h), Reply::Refused(m) if m.0.contains("link")));
        let mut none = fake();
        none.record = None;
        assert!(matches!(run(DIR, &none), Reply::Refused(m) if m.0.contains("no Kora install record")));
        for (v, a) in [("install", s(&["--dir", DIR])), ("uninstall", s(&[])), ("schedule", s(&["set", "a", "--as-system"])), ("self-update", s(&["--version", "v0.1.0"]))] {
            let r = handle(&h, &policy(), &caller(me()), v, &a, &BTreeMap::new(), &never, &mut |_, _| {});
            assert!(matches!(r, Reply::Refused(_)), "{v}");
        }
        assert!(h.log.borrow().iter().all(|l| !l.contains("--as-system") && !l.contains("/home")));
    }

    #[test]
    fn deleting_removes_the_record_and_a_new_database_is_recorded_under_its_name() {
        let h = fake();
        let data = serde_json::json!({"db": {"state": {"database": "kora_ab12"}}});
        handle(&h, &policy(), &caller(me()), "admin-step", &s(&["run", "--step", "build.db", "--dir", DIR]), &BTreeMap::new(), &ok_run(Some(data)), &mut |_, _| {});
        assert_eq!(h.ledger.borrow().owner("db:kora_ab12"), Some("uid:1000"));
        // a name the child made up that is not Kora's own prefix is not recorded
        let h = fake();
        let evil = serde_json::json!({"db": {"state": {"database": "mysql"}}});
        handle(&h, &policy(), &caller(me()), "admin-step", &s(&["run", "--step", "build.db", "--dir", DIR]), &BTreeMap::new(), &ok_run(Some(evil)), &mut |_, _| {});
        assert!(h.ledger.borrow().owners.is_empty());
        // delete: the server's firewall rules made for me go away from the ledger
        let h = fake();
        h.ledger.borrow_mut().record("firewall:main".into(), "uid:1000");
        let r = handle(
            &h,
            &policy(),
            &caller(me()),
            "admin-step",
            &s(&["undo", "--step", "system.firewall", "--dir", DIR, "--name", "Main", "--game-port", "30120", "--txadmin-port", "40120"]),
            &BTreeMap::new(),
            &ok_run(None),
            &mut |_, _| {},
        );
        assert!(matches!(r, Reply::Ran(_)));
        assert!(h.ledger.borrow().owners.is_empty());
    }

    #[test]
    fn a_failed_or_timed_out_child_changes_no_records() {
        let h = fake();
        let fail = |_: &RunSpec<'_>, _: Progress<'_>| Ok(Ran { code: 1, timed_out: false, data: None });
        handle(&h, &policy(), &caller(me()), "admin-step", &s(&["run", "--step", "system.service", "--dir", DIR]), &BTreeMap::new(), &fail, &mut |_, _| {});
        let late = |_: &RunSpec<'_>, _: Progress<'_>| Ok(Ran { code: 0, timed_out: true, data: None });
        handle(&h, &policy(), &caller(me()), "admin-step", &s(&["run", "--step", "system.service", "--dir", DIR]), &BTreeMap::new(), &late, &mut |_, _| {});
        assert!(h.ledger.borrow().owners.is_empty());
        let log = h.log.borrow();
        assert!(log[0].ends_with("failed (code 1)") && log[1].ends_with("stopped (time limit)"), "{log:?}");
    }

    #[test]
    fn the_child_gets_the_checked_arguments_the_limit_and_only_the_secrets() {
        let h = fake();
        let seen = RefCell::new(None);
        let spy = |r: &RunSpec<'_>, _: Progress<'_>| {
            *seen.borrow_mut() = Some((r.argv.to_vec(), r.env.clone(), r.limit));
            Ok(Ran { code: 0, timed_out: false, data: None })
        };
        let mut env = BTreeMap::new();
        env.insert("CFX_LICENSE_KEY".to_string(), "cfxk_x".to_string());
        handle(&h, &policy(), &caller(me()), "schedule", &s(&["set", "Main", "--daily", "03:00"]), &env, &spy, &mut |_, _| {});
        let (a, e, t) = seen.borrow().clone().unwrap();
        assert_eq!(a, ["--no-elevate", "schedule", "set", "Main", "--as-user", "--daily", "03:00"]);
        assert_eq!(e.len(), 1);
        assert_eq!(t, std::time::Duration::from_secs(180));
        assert_eq!(h.ledger.borrow().owner("task:main"), Some("uid:1000"));
    }

    #[test]
    fn root_is_served_without_the_list_but_still_needs_a_record() {
        let mut h = fake();
        h.owner = Some(AccountId::uid(1001));
        let root = Caller { id: Some(AccountId::uid(0)), integrity: None, is_root: true, remote: false };
        let r = handle(&h, &Err("no file".into()), &root, "admin-step", &s(&["run", "--step", "system.firewall", "--dir", DIR]), &BTreeMap::new(), &ok_run(None), &mut |_, _| {});
        assert!(matches!(r, Reply::Ran(_)), "{r:?}");
        h.record = None;
        assert!(matches!(handle(&h, &Err("x".into()), &root, "admin-step", &s(&["run", "--step", "system.firewall", "--dir", DIR]), &BTreeMap::new(), &never, &mut |_, _| {}), Reply::Refused(_)));
    }

    #[test]
    fn a_folder_handed_to_the_servers_own_account_needs_the_ledger() {
        let mut h = fake();
        h.owner = Some(AccountId::uid(998)); // a root-run install: the folder belongs to `kora`
        let go = |h: &Fake| handle(h, &policy(), &caller(me()), "admin-step", &s(&["run", "--step", "system.firewall", "--dir", DIR]), &BTreeMap::new(), &ok_run(None), &mut |_, _| {});
        assert!(matches!(go(&h), Reply::Refused(m) if m.0.contains("finish upgrading")), "not adopted yet");
        h.ledger.borrow_mut().record("service:kora-main".into(), "uid:1001");
        assert!(matches!(go(&h), Reply::Refused(_)), "someone else's server");
        h.ledger.borrow_mut().record("service:kora-main".into(), "uid:1000");
        assert!(matches!(go(&h), Reply::Ran(_)), "adopted: stays manageable");
    }

    #[test]
    fn the_reach_firewall_operation_is_the_callers_own_game_port_and_is_recorded() {
        let h = fake();
        let go = |h: &Fake, who: AccountId| handle(h, &policy(), &caller(who), "admin-step", &s(&["run", "--step", "reach.firewall", "--dir", DIR]), &BTreeMap::new(), &ok_run(None), &mut |_, _| {});
        assert!(matches!(go(&h, me()), Reply::Ran(Ran { code: 0, .. })));
        assert_eq!(h.ledger.borrow().owner("firewall:main"), Some("uid:1000"), "recorded like the install's firewall step");
        assert!(h.log.borrow()[0].contains("run reach.firewall | done") && !h.log.borrow()[0].contains("/home"));
        // someone else's folder, or rules someone else made
        assert!(matches!(go(&h, AccountId::uid(1001)), Reply::Refused(_)));
        // a forged record with remote desktop as the game port
        let mut bad = fake();
        bad.record = Some(InstallRecord { name: "Main".into(), game_port: 3389, txadmin_port: 40120 });
        assert!(matches!(go(&bad, me()), Reply::Refused(m) if m.0.contains("3389")));
        assert!(bad.ledger.borrow().owners.is_empty());
    }
}
