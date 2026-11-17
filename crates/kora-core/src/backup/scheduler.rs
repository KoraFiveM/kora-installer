//! Scheduled backups, run by the panel's background process.
//!
//! **Why not the operating system's scheduler?** Kora's update schedule is a systemd timer or a Windows task,
//! and creating one needs administrator rights, through a path that is being rewritten (the privileged
//! service). Until a backup task can be created through it, scheduled backups run from the long-lived
//! panel process: **they only run while the panel's background service runs** (`kora panel
//! --background`, or the panel kept open). A slot missed because the panel was not running is made up once,
//! when the panel next starts. The Overview says "Backups are scheduled, but none has been made yet" and
//! warns when the newest backup is older than the schedule implies, so a service that is not running is
//! noticed.
//!
//! The administrator operation that would replace this (for the privileged service's author): "create,
//! replace and remove a per-server recurring job named `kora-backup-<slug>` that runs
//! `<kora> backup create <dir> --scheduled` as the account that owns the Kora home (never root, never the
//! service account), at a given daily/weekly local time, with catch-up after downtime", the same shape as
//! the update timer but with a different command line and its own name; the same `ScheduleConfig` parsing
//! ([`crate::service::parse_time`]) validates its input. `kora backup create --scheduled` already exists and
//! does everything this module does for one server.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::create::{self, CreateOptions};
use super::types::Label;
use super::{settings, Host, Tools};
use crate::installer::registry::ServerEntry;

/// How often the panel looks whether a backup is due.
pub const LOOK_EVERY: Duration = Duration::from_secs(60);

/// What happened to one server on one look.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    NotDue,
    Made(String),
    Failed(String),
}

/// Run the backup of `e` if its schedule says one is due. The slot is marked as taken BEFORE the backup
/// starts, so a failing backup is retried at the next slot, not every minute.
pub fn run_if_due(e: &ServerEntry, host: &mut dyn Host, tools: &Tools<'_>) -> Outcome {
    let cfg = settings::load_settings(&e.name);
    let status = settings::load_status(&e.name);
    if !settings::is_due(&cfg.schedule, &status, tools.now) {
        return Outcome::NotDue;
    }
    settings::update_status(&e.name, |s| s.last_scheduled_unix = Some(tools.now));
    run_scheduled(e, host, tools)
}

/// A scheduled backup of `e` now (what `kora backup create --scheduled` does).
pub fn run_scheduled(e: &ServerEntry, host: &mut dyn Host, tools: &Tools<'_>) -> Outcome {
    let cfg = settings::load_settings(&e.name);
    let o = CreateOptions { passphrase: None, resources: cfg.resources, stop_server: cfg.stop_for_scheduled, label: Label::Scheduled, folder: None };
    match create::create(e, &o, host, tools, &mut |_| {}) {
        Ok(r) => Outcome::Made(r.info.id),
        Err(p) => Outcome::Failed(p.message),
    }
}

/// Where a failure is announced (the panel's alert list, Discord, the generic webhook).
pub trait Announce: Send + Sync {
    fn failed(&self, e: &ServerEntry, message: &str);
    fn recovered(&self, e: &ServerEntry);
}

pub struct Handle(Arc<AtomicBool>);

impl Handle {
    pub fn stop(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Start the thread that looks for due backups every [`LOOK_EVERY`].
pub fn spawn(make: crate::monitor::BackendMaker, announce: Arc<dyn Announce>) -> Handle {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let _ = std::thread::Builder::new().name("kora-backups".into()).spawn(move || {
        while !flag.load(Ordering::Relaxed) {
            let (make, announce) = (make.clone(), announce.clone());
            // A bug in one pass must not end the scheduler.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || look(&*make, &*announce, crate::monitor::now_unix())));
            let mut waited = Duration::ZERO;
            while waited < LOOK_EVERY && !flag.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(500));
                waited += Duration::from_millis(500);
            }
        }
    });
    Handle(stop)
}

fn look(make: &(dyn Fn() -> Box<dyn crate::api::Backend + Send> + Send + Sync), announce: &dyn Announce, now: u64) {
    let all = settings::all_settings();
    if !all.values().any(|s| s.schedule.enabled) {
        return;
    }
    let mut backend = make();
    let Ok(servers) = backend.list() else { return };
    for e in servers {
        let Some(cfg) = all.get(&e.name) else { continue };
        if !cfg.schedule.enabled || !e.dir.is_dir() {
            continue;
        }
        let status = settings::load_status(&e.name);
        if !settings::is_due(&cfg.schedule, &status, now) {
            continue;
        }
        let set = crate::container::backup::ToolSet::for_server(&e.name, &e.dir);
        let mut tools = set.tools();
        tools.now = now;
        let mut host = super::host::BackendHost::new(&mut *backend, &e);
        match run_if_due(&e, &mut host, &tools) {
            Outcome::Failed(m) => announce.failed(&e, &m),
            Outcome::Made(_) => announce.recovered(&e),
            Outcome::NotDue => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::dest::Destination;
    use crate::backup::tests_support::*;
    use crate::backup::types::*;

    fn schedule_on(name: &str) {
        let mut s = BackupSettings::default();
        s.schedule.enabled = true;
        settings::save_settings(name, &s).unwrap();
    }

    #[test]
    fn a_due_backup_runs_once_per_slot_and_is_labelled_scheduled() {
        let rig = Rig::new();
        let git = rig.clean_git();
        schedule_on(&rig.entry.name);
        let tools = rig.tools(&git);
        match run_if_due(&rig.entry, &mut FakeHost::new(true), &tools) {
            Outcome::Made(id) => {
                let list = super::super::restore::destination_for(&rig.entry.name).list();
                assert_eq!(list.len(), 1);
                assert_eq!((list[0].id.clone(), list[0].label), (id, Label::Scheduled));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(run_if_due(&rig.entry, &mut FakeHost::new(true), &tools), Outcome::NotDue, "the same slot is not run twice");
        let mut later = rig.tools(&git);
        later.now += 86_400;
        assert!(matches!(run_if_due(&rig.entry, &mut FakeHost::new(true), &later), Outcome::Made(_)), "the next day's slot is");
    }

    #[test]
    fn a_failing_scheduled_backup_is_reported_and_not_retried_every_minute() {
        let rig = Rig::new();
        let git = rig.clean_git();
        schedule_on(&rig.entry.name);
        *rig.db.fail_with.borrow_mut() = Some("Can't connect to MySQL server (111)".into());
        let tools = rig.tools(&git);
        assert!(matches!(run_if_due(&rig.entry, &mut FakeHost::new(false), &tools), Outcome::Failed(m) if m.contains("not running")));
        assert_eq!(run_if_due(&rig.entry, &mut FakeHost::new(false), &tools), Outcome::NotDue);
        let st = settings::load_status(&rig.entry.name);
        assert!(st.last_failed_unix.is_some());
        let s = settings::summary(&[], &settings::load_settings(&rig.entry.name), &st, tools.now + 60);
        assert!(s.warning.unwrap().contains("last backup failed"));
    }

    #[test]
    fn a_server_without_a_schedule_is_left_alone() {
        let rig = Rig::new();
        let git = rig.clean_git();
        assert_eq!(run_if_due(&rig.entry, &mut FakeHost::new(true), &rig.tools(&git)), Outcome::NotDue);
    }
}
