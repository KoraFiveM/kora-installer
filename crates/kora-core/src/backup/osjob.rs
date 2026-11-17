//! Scheduled backups as a job of the operating system, through the administrator service.
//!
//! When the service is there for this account, a server's backup schedule becomes the job `kora-backup-<slug>` (systemd timer /
//! Task Scheduler task, `service::backup_job`) that runs `kora backup create <folder> --scheduled` as the person, even when
//! the panel is closed, and makes up a slot missed while the computer was off. The panel's own scheduler
//! ([`super::scheduler`]) then stands down for that server (`BackupStatus::os_job`), so a backup never runs twice. When the
//! service is not there, the panel's scheduler keeps the schedule, with its note that it needs the background service.
//!
//! Which is in charge is recorded in Kora's store (`BackupStatus::os_job`), never in the server folder. The privileged side
//! creates the job from the typed operation `Op::BackupJobSet` and does not open the store; this module is the person's side.

use super::settings;
use super::types::{BackupSettings, Cadence};
use crate::installer::registry::ServerEntry;
use crate::installer::slug;
use crate::privsvc::client;
use crate::privsvc::problems::Availability;

const DAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

/// The arguments of the service request for `cfg`'s schedule (`backup-job set <dir> ...`).
pub fn set_args(dir: &std::path::Path, cfg: &BackupSettings) -> Vec<String> {
    let s = &cfg.schedule;
    let mut a = vec!["set".to_string(), dir.to_string_lossy().into_owned()];
    match (s.cadence, s.weekday) {
        (Cadence::Weekly, Some(d)) => a.extend(["--weekly".into(), DAYS[usize::from(d.min(6))].into(), s.time.clone()]),
        _ => a.extend(["--daily".into(), s.time.clone()]),
    }
    a
}

fn ask(args: &[String]) -> Result<(), String> {
    let mut last = String::new();
    match client::call("backup-job", args, &[], &mut |l| last = l.to_string()) {
        Ok(a) if a.ok => Ok(()),
        Ok(_) => Err(if last.is_empty() { "the service could not make the change".into() } else { last }),
        Err(client::ClientError::Refused(m) | client::ClientError::Unavailable(m) | client::ClientError::Broken(m)) => Err(m),
    }
}

/// Make the operating system's job match `cfg` (settings already saved). Never fails the caller: what could not be done is
/// kept in the status and shown in the Backups tab.
pub fn apply(e: &ServerEntry, cfg: &BackupSettings) {
    let before = settings::load_status(&e.name);
    let ready = client::availability() == Availability::Ready;
    let (mut os_job, mut error) = (before.os_job, None::<String>);
    if cfg.schedule.enabled && ready {
        match ask(&set_args(&e.dir, cfg)) {
            Ok(()) => os_job = true,
            Err(m) => {
                if before.os_job {
                    let _ = ask(&["off".into(), e.dir.to_string_lossy().into_owned()]);
                }
                os_job = false;
                error = Some(format!("Kora could not set up the operating-system job ({m}). The panel's own scheduler is used instead."));
            }
        }
    } else if before.os_job {
        if ready {
            match ask(&["off".into(), e.dir.to_string_lossy().into_owned()]) {
                Ok(()) => os_job = false,
                Err(m) => error = Some(format!("Kora could not remove the operating-system job ({m}). It does nothing while scheduled backups are off.")),
            }
            if error.is_some() {
                os_job = false;
            }
        } else {
            os_job = false;
            error = Some(if cfg.schedule.enabled {
                "Kora's administrator service is not available now, so the job of the operating system keeps its earlier time. The panel's own scheduler is used from now on, so a backup may be made twice in a day.".to_string()
            } else {
                "Kora's administrator service is not available now, so the job of the operating system could not be removed. It does nothing while scheduled backups are off.".to_string()
            });
        }
    }
    if os_job != before.os_job || error != before.os_job_error {
        settings::update_status(&e.name, |s| {
            s.os_job = os_job;
            s.os_job_error = error.clone();
        });
    }
}

/// A server is being deleted: take its job away first (its status is forgotten right after). A sentence for the person when it
/// could not be done, with the way to do it.
pub fn remove_for_delete(e: &ServerEntry) -> Option<String> {
    if !settings::load_status(&e.name).os_job {
        return None;
    }
    if client::availability() != Availability::Ready {
        return Some(format!(
            "The scheduled-backup job of the operating system (kora-backup-{}) could not be removed because Kora's administrator service is not available. Remove it with your system's scheduler (Linux: systemctl disable --now kora-backup-{0}.timer, then delete its two files in /etc/systemd/system; Windows: Task Scheduler, Kora folder).",
            slug(&e.name)
        ));
    }
    ask(&["off".into(), e.dir.to_string_lossy().into_owned()]).err().map(|m| format!("The scheduled-backup job of the operating system could not be removed: {m}"))
}

/// Is the job still there? A job removed behind Kora's back (`kora uninstall`, by hand) must not leave the panel's own
/// scheduler standing down for good: the record is corrected and the person told.
pub fn verify_with(e: &ServerEntry, rn: &dyn crate::service::CommandRunner, platform: crate::service::Platform) {
    if settings::load_status(&e.name).os_job && !crate::service::backup_job::exists_with(&slug(&e.name), rn, platform) {
        settings::update_status(&e.name, |s| {
            s.os_job = false;
            s.os_job_error =
                Some("The job of the operating system for the scheduled backups is gone (it was removed, for example by `kora uninstall`). The panel's own scheduler is used again.".into());
        });
    }
}

pub fn verify(e: &ServerEntry) {
    verify_with(e, &crate::service::SystemRunner, crate::service::Platform::current());
}

/// The line in the Backups tab about who runs the schedule.
pub fn note(e: &ServerEntry, status: &super::types::BackupStatus) -> String {
    let base = if status.os_job {
        format!(
            "Scheduled backups run as a job of your operating system (kora-backup-{}), so they happen even when Kora's panel is closed, and one missed while the computer was off is made up when it is next on.",
            slug(&e.name)
        )
    } else {
        "Scheduled backups run while Kora's background service is running (start it with `kora panel --background`). A backup that was missed is made up when it starts again. With Kora's administrator service installed they run as a job of your operating system instead, whether or not the panel is open.".to_string()
    };
    match &status.os_job_error {
        Some(m) => format!("{base} {m}"),
        None => base,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::types::BackupSchedule;
    use crate::privsvc::client::testing::{Scripted, SERIAL};

    fn entry(name: &str) -> ServerEntry {
        ServerEntry {
            source: None,
            versions: None,
            name: name.into(),
            dir: format!("/home/jo/kora/{name}").into(),
            mode: crate::installer::Mode::Production,
            ports: crate::installer::registry::PortsRecord { game: 30120, txadmin: 40120 },
            created_unix: 0,
        }
    }
    fn cfg(enabled: bool, cadence: Cadence, weekday: Option<u8>) -> BackupSettings {
        BackupSettings { schedule: BackupSchedule { enabled, cadence, time: "03:30".into(), weekday }, ..Default::default() }
    }
    fn with_service<T>(svc: std::sync::Arc<Scripted>, f: impl FnOnce() -> T) -> T {
        let _g = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _ = crate::installer::registry::use_test_home();
        client::set_override(Some(svc));
        let r = f();
        client::set_override(None);
        r
    }

    #[test]
    fn the_request_is_a_daily_or_weekly_time_for_the_servers_folder() {
        let e = entry("a");
        assert_eq!(set_args(&e.dir, &cfg(true, Cadence::Daily, None)), ["set", "/home/jo/kora/a", "--daily", "03:30"]);
        assert_eq!(set_args(&e.dir, &cfg(true, Cadence::Weekly, Some(6))), ["set", "/home/jo/kora/a", "--weekly", "sun", "03:30"]);
        // what we send is a request the service accepts
        let a = set_args(&e.dir, &cfg(true, Cadence::Weekly, Some(2)));
        assert!(crate::privsvc::op::parse("backup-job", &a, &Default::default()).is_ok());
    }

    #[test]
    fn with_the_service_the_schedule_becomes_an_os_job_and_the_panel_stands_down() {
        let e = entry("os-on");
        let svc = Scripted::ready(true);
        with_service(svc.clone(), || {
            apply(&e, &cfg(true, Cadence::Daily, None));
            let st = settings::load_status(&e.name);
            assert!(st.os_job && st.os_job_error.is_none());
            // the panel's own scheduler does not take it
            assert!(super::super::scheduler::stands_down(&st));
            // turning it off removes the job
            apply(&e, &cfg(false, Cadence::Daily, None));
            assert!(!settings::load_status(&e.name).os_job);
        });
        let r = svc.requests();
        assert_eq!(r[0].0, "backup-job");
        assert_eq!(r[0].1[0], "set");
        assert_eq!(r[1].1, ["off", "/home/jo/kora/os-on"]);
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn without_the_service_the_panel_keeps_the_schedule_and_nothing_is_asked() {
        let e = entry("os-off");
        let svc = Scripted::missing();
        with_service(svc.clone(), || {
            apply(&e, &cfg(true, Cadence::Daily, None));
            let st = settings::load_status(&e.name);
            assert!(!st.os_job && st.os_job_error.is_none());
            assert!(!super::super::scheduler::stands_down(&st));
            assert!(note(&e, &st).contains("kora panel --background"));
        });
        assert!(svc.requests().is_empty());
    }

    #[test]
    fn a_refusal_falls_back_to_the_panels_scheduler_and_says_why() {
        let e = entry("os-refused");
        let svc = std::sync::Arc::new(Scripted {
            availability: Availability::Ready,
            reply: Err(client::ClientError::Refused("Kora's service will not change the scheduled backups of os-refused: Kora made it for another account on this computer.".into())),
            seen: Default::default(),
        });
        with_service(svc, || {
            apply(&e, &cfg(true, Cadence::Daily, None));
            let st = settings::load_status(&e.name);
            assert!(!st.os_job);
            let n = note(&e, &st);
            assert!(n.contains("another account") && n.contains("own scheduler"), "{n}");
        });
    }

    #[test]
    fn deleting_a_server_removes_its_job_or_says_how() {
        let e = entry("os-del");
        let svc = Scripted::ready(true);
        with_service(svc.clone(), || {
            assert_eq!(remove_for_delete(&e), None, "no job: nothing asked");
            settings::update_status(&e.name, |s| s.os_job = true);
            assert_eq!(remove_for_delete(&e), None);
        });
        assert_eq!(svc.requests(), [("backup-job".to_string(), vec!["off".to_string(), "/home/jo/kora/os-del".to_string()])]);
        with_service(Scripted::missing(), || {
            let m = remove_for_delete(&e).unwrap();
            assert!(m.contains("kora-backup-os-del") && m.contains("systemctl disable"), "{m}");
        });
    }

    #[test]
    fn a_job_that_disappeared_gives_the_schedule_back_to_the_panel() {
        let _ = crate::installer::registry::use_test_home();
        let e = entry("os-gone");
        settings::update_status(&e.name, |s| s.os_job = true);
        let there = crate::service::testing::FakeRunner::default();
        verify_with(&e, &there, crate::service::Platform::Linux);
        assert!(settings::load_status(&e.name).os_job, "found: unchanged");
        let gone = crate::service::testing::FakeRunner::default();
        gone.respond("cat /etc/systemd/system/kora-backup-os-gone.timer", 1, "");
        verify_with(&e, &gone, crate::service::Platform::Linux);
        let st = settings::load_status(&e.name);
        assert!(!st.os_job && st.os_job_error.as_deref().is_some_and(|m| m.contains("is gone")));
    }

    #[test]
    fn the_note_says_who_runs_the_schedule() {
        let e = entry("n");
        let on = crate::backup::BackupStatus { os_job: true, ..Default::default() };
        assert!(note(&e, &on).contains("job of your operating system (kora-backup-n)"));
    }
}
