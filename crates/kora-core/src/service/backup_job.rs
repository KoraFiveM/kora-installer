//! Scheduled backups as a job of the operating system: a systemd timer (Linux) or a Task Scheduler task
//! (Windows) named `kora-backup-<slug>` / `Kora\Backup <slug>` that runs
//! `<kora> backup create <server folder> --scheduled` **as the person who owns the Kora home**.
//!
//! Threat model. Creating the job needs administrator rights (a unit file in `/etc/systemd/system`, a task for a
//! user), so it is made by the administrator service's child (`kora backup-job set|off`, see
//! `privsvc::op::Op::BackupJobSet`). What the job RUNS is the part that matters:
//! * never root, never the server's own service account: the unit has `User=<the caller's account>` (the child's
//!   environment names the account the service identified from the OS connection, see
//!   `privsvc::paths::linux_child_env`), and Windows runs it with the person's interactive token at the least
//!   privilege (`LeastPrivilege`, no stored password), exactly like the update task;
//! * the program is the one root may run (`binary::root_exe`: a root-owned, pinned copy), never a path from the server
//!   folder, so the person cannot swap what the job executes;
//! * it never takes a passphrase (a scheduled backup cannot ask for one): encrypted scheduled backups do not exist;
//! * the job itself opens the person's own store (`kora backup create` needs the saved settings), which is fine because
//!   it runs as the person. The privileged child that CREATES the job never opens a store.
//!
//! Catch-up after downtime: `Persistent=true` (systemd runs a missed slot at the next boot / start of the timer) and
//! `StartWhenAvailable` (Task Scheduler runs a missed start as soon as possible).

use std::path::Path;

use super::schedule::{parse_time, xml_escape, Schedule, TimeOfDay, Weekday};
use super::{apply_plan, systemd_quote, CommandRunner, ManagedServer, Plan, Platform, ServiceError, ServiceResult, SystemRunner, SystemStep};

/// What the job is: the server (by its short name), the folder it backs up, the program to run and who runs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupJob {
    pub slug: String,
    pub dir: String,
    pub exe: String,
    /// `jo` (Linux) or `DOMAIN\jo` (Windows). Never root, never a system account.
    pub account: String,
}

/// The unit / task name.
pub fn unit_name(slug: &str) -> String {
    format!("kora-backup-{slug}")
}

pub fn task_name(slug: &str) -> String {
    format!("Kora\\Backup {slug}")
}

fn unit_path(slug: &str, ext: &str) -> String {
    format!("/etc/systemd/system/{}.{ext}", unit_name(slug))
}

/// The account the job runs as must be an ordinary account name: a Linux login name that is not root, or a Windows
/// `DOMAIN\name` that is not one of the system accounts.
pub fn account_ok(account: &str, platform: Platform) -> ServiceResult<()> {
    let bad = || ServiceError::Invalid("the account the backup job would run as is not usable (it must be an ordinary user account, never root or a system account)".into());
    if account.is_empty() || account.len() > 128 || account.chars().any(|c| c.is_control() || matches!(c, '"' | '%' | '<' | '>' | '|' | '/' | '\'')) {
        return Err(bad());
    }
    match platform {
        Platform::Linux => {
            let mut it = account.chars();
            let first = it.next().is_some_and(|c| c.is_ascii_lowercase() || c == '_');
            if account == "root" || !first || !account.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '.')) {
                return Err(bad());
            }
        }
        Platform::Windows => {
            let upper = account.to_ascii_uppercase();
            let name = upper.rsplit('\\').next().unwrap_or("");
            if upper.starts_with("NT AUTHORITY")
                || upper.starts_with("NT SERVICE")
                || matches!(name, "SYSTEM" | "LOCAL SERVICE" | "NETWORK SERVICE" | "")
                || account.contains(char::is_whitespace) && !account.contains('\\')
            {
                return Err(bad());
            }
        }
    }
    Ok(())
}

fn managed(job: &BackupJob) -> ManagedServer {
    // Only the shared validation (name, folder, program) is wanted; the `user` field is the server's account and unused.
    ManagedServer { name: job.slug.clone(), dir: job.dir.clone(), start_command: vec![job.exe.clone()], user: "kora".into(), env_file: None, kora_exe: job.exe.clone(), windows_run_as: None }
}

fn validate(job: &BackupJob, platform: Platform) -> ServiceResult<()> {
    managed(job).validate()?;
    account_ok(&job.account, platform)
}

/// `ExecStart` / Task command line arguments, identical on both systems apart from quoting.
fn exec_line(job: &BackupJob) -> String {
    format!("{} backup create {} --scheduled", systemd_quote(&job.exe), systemd_quote(&job.dir))
}

/// The systemd oneshot service: runs as the person (`User=`), no new privileges. systemd sets `HOME` for `User=`, which is
/// where the person's Kora home is found.
pub fn systemd_service_unit(job: &BackupJob) -> ServiceResult<String> {
    validate(job, Platform::Linux)?;
    Ok(format!(
        "[Unit]\n\
         Description=Kora scheduled backup for {name}\n\
         After=network-online.target\n\
         \n\
         [Service]\n\
         Type=oneshot\n\
         User={user}\n\
         WorkingDirectory=/\n\
         ExecStart={exec}\n\
         NoNewPrivileges=true\n\
         Nice=10\n",
        name = job.slug,
        user = job.account,
        exec = exec_line(job),
    ))
}

/// The timer: `Persistent=true` makes a slot missed while the machine was off happen at the next start.
pub fn systemd_timer_unit(job: &BackupJob, schedule: &Schedule) -> ServiceResult<String> {
    validate(job, Platform::Linux)?;
    schedule.validate()?;
    Ok(format!(
        "[Unit]\n\
         Description=Kora backup schedule for {name} ({when})\n\
         \n\
         [Timer]\n\
         OnCalendar={cal}\n\
         Persistent=true\n\
         RandomizedDelaySec=300\n\
         Unit={unit}.service\n\
         \n\
         [Install]\n\
         WantedBy=timers.target\n",
        name = job.slug,
        when = schedule.describe(),
        cal = schedule.on_calendar(),
        unit = unit_name(&job.slug),
    ))
}

/// Task Scheduler definition: the person's interactive token, least privilege, a missed start runs as soon as possible.
pub fn task_xml(job: &BackupJob, schedule: &Schedule) -> ServiceResult<String> {
    validate(job, Platform::Windows)?;
    schedule.validate()?;
    let t = schedule.time();
    let by = match schedule {
        Schedule::Daily(_) => "<ScheduleByDay><DaysInterval>1</DaysInterval></ScheduleByDay>".to_string(),
        Schedule::Weekly(d, _) => format!("<ScheduleByWeek><WeeksInterval>1</WeeksInterval><DaysOfWeek><{} /></DaysOfWeek></ScheduleByWeek>", d.xml()),
        Schedule::Custom(days, _) => {
            let list: String = days.iter().map(|d| format!("<{} />", d.xml())).collect();
            format!("<ScheduleByWeek><WeeksInterval>1</WeeksInterval><DaysOfWeek>{list}</DaysOfWeek></ScheduleByWeek>")
        }
    };
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-16\"?>\n\
         <Task version=\"1.2\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\">\n\
         <RegistrationInfo><Description>{desc}</Description></RegistrationInfo>\n\
         <Triggers><CalendarTrigger><StartBoundary>2024-01-01T{t}:00</StartBoundary><Enabled>true</Enabled>{by}</CalendarTrigger></Triggers>\n\
         <Principals><Principal id=\"Author\"><UserId>{user}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>\n\
         <Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><StartWhenAvailable>true</StartWhenAvailable><ExecutionTimeLimit>PT4H</ExecutionTimeLimit><Enabled>true</Enabled></Settings>\n\
         <Actions Context=\"Author\"><Exec><Command>{cmd}</Command><Arguments>{args}</Arguments></Exec></Actions>\n\
         </Task>\n",
        desc = xml_escape(&format!("Kora scheduled backup for {}", job.slug)),
        user = xml_escape(&job.account),
        cmd = xml_escape(&job.exe),
        args = xml_escape(&format!("backup create \"{}\" --scheduled", job.dir.trim_end_matches(['\\', '/']))),
    ))
}

/// Where the task definition is written for `schtasks /XML` to read: a folder only administrators can change.
fn task_xml_file(slug: &str) -> String {
    let pf = std::env::var("ProgramFiles").ok().filter(|p| !p.trim().is_empty()).unwrap_or_else(|| "C:\\Program Files".into());
    format!("{}\\Kora\\tasks\\backup-{slug}.xml", pf.trim_end_matches(['\\', '/']))
}

/// The plan that makes (or replaces) the job.
pub fn install_plan(job: &BackupJob, schedule: &Schedule, platform: Platform) -> ServiceResult<Plan> {
    validate(job, platform)?;
    schedule.validate()?;
    let when = schedule.describe();
    Ok(match platform {
        Platform::Linux => {
            let timer = format!("{}.timer", unit_name(&job.slug));
            Plan::new(vec![
                SystemStep::write(format!("Create the backup job for '{}' ({})", job.slug, unit_path(&job.slug, "service")), &unit_path(&job.slug, "service"), systemd_service_unit(job)?, 0o644),
                SystemStep::write(
                    format!("Create the backup schedule for '{}': {when} ({})", job.slug, unit_path(&job.slug, "timer")),
                    &unit_path(&job.slug, "timer"),
                    systemd_timer_unit(job, schedule)?,
                    0o644,
                ),
                SystemStep::run("Tell systemd to read the new backup job", "systemctl", &["daemon-reload"]),
                SystemStep::run(format!("Turn on the scheduled backups of '{}'", job.slug), "systemctl", &["enable", "--now", &timer]),
            ])
        }
        Platform::Windows => {
            let xml = task_xml_file(&job.slug);
            Plan::new(vec![
                SystemStep::write(format!("Write the scheduled task definition for the backups of '{}'", job.slug), &xml, task_xml(job, schedule)?, 0o600),
                SystemStep::run_owned(
                    format!("Create the Windows scheduled task '{}' that backs up '{}' {when}, as {}", task_name(&job.slug), job.slug, job.account),
                    "schtasks.exe",
                    vec!["/Create".into(), "/TN".into(), task_name(&job.slug), "/XML".into(), xml.clone(), "/F".into()],
                ),
                SystemStep::remove(format!("Delete the temporary file {xml}"), &xml),
            ])
        }
    })
}

/// The plan that removes only the job Kora made (its own, Kora-named unit files / task).
pub fn uninstall_plan(slug: &str, platform: Platform) -> ServiceResult<Plan> {
    let probe = ManagedServer { name: slug.into(), dir: "/".into(), start_command: vec!["kora".into()], user: "kora".into(), env_file: None, kora_exe: "kora".into(), windows_run_as: None };
    probe.validate()?;
    Ok(match platform {
        Platform::Linux => {
            let timer = format!("{}.timer", unit_name(slug));
            Plan::new(vec![
                SystemStep::run(format!("Turn off the scheduled backups of '{slug}'"), "systemctl", &["disable", "--now", &timer]).tolerant(),
                SystemStep::remove(format!("Delete {}", unit_path(slug, "timer")), &unit_path(slug, "timer")),
                SystemStep::remove(format!("Delete {}", unit_path(slug, "service")), &unit_path(slug, "service")),
                SystemStep::run("Tell systemd the backup job is gone", "systemctl", &["daemon-reload"]),
            ])
        }
        Platform::Windows => {
            Plan::new(vec![SystemStep::run(format!("Delete the Windows scheduled task '{}' that Kora created", task_name(slug)), "schtasks.exe", &["/Delete", "/TN", &task_name(slug), "/F"])])
        }
    })
}

/// Is there a job for `slug` on this system (what Kora created, found by its name)?
pub fn exists_with(slug: &str, rn: &dyn CommandRunner, platform: Platform) -> bool {
    match platform {
        Platform::Linux => rn.run("cat", &[unit_path(slug, "timer")]).is_ok_and(|o| o.success()),
        Platform::Windows => rn.run("schtasks.exe", &["/Query".into(), "/TN".into(), task_name(slug)]).is_ok_and(|o| o.success()),
    }
}

/// The schedule of the job read back from the operating system (for `kora backup schedule` to show).
pub fn read_back(slug: &str, rn: &dyn CommandRunner, platform: Platform) -> Option<super::ScheduleConfig> {
    match platform {
        Platform::Linux => {
            let timer = rn.run("cat", &[unit_path(slug, "timer")]).ok().filter(|o| o.success())?;
            super::parse_timer(&timer.stdout, "")
        }
        Platform::Windows => {
            let o = rn.run("schtasks.exe", &["/Query".into(), "/TN".into(), task_name(slug), "/XML".into()]).ok().filter(|o| o.success())?;
            super::parse_task_xml(&o.stdout)
        }
    }
}

/// `daily HH:MM` / `weekly DAY HH:MM` (as the service request writes them) to a [`Schedule`].
pub fn schedule_from(weekday: Option<u8>, time: &str) -> ServiceResult<Schedule> {
    let t: TimeOfDay = parse_time(time)?;
    Ok(match weekday {
        Some(d) => Schedule::Weekly(Weekday::from_index(d)?, t),
        None => Schedule::Daily(t),
    })
}

/// Create or replace the job for the server in `dir` (needs administrator rights: run by the service's child).
/// `account` is the person it runs as.
pub fn set(slug: &str, dir: &Path, schedule: &Schedule, account: &str) -> ServiceResult<()> {
    let exe = super::binary::root_exe(None).map_err(ServiceError::Other)?;
    let job = BackupJob { slug: slug.into(), dir: dir.to_string_lossy().into_owned(), exe: exe.path().to_string_lossy().into_owned(), account: account.into() };
    let mut plan = Plan::default();
    if let super::binary::RootExe::Copy { src, dest } = &exe {
        plan.extend(super::binary::install_plan(&src.to_string_lossy(), dest));
    }
    plan.extend(install_plan(&job, schedule, Platform::current())?);
    apply_plan(&plan, &SystemRunner)
}

/// Remove the job of `slug` if there is one.
pub fn remove(slug: &str) -> ServiceResult<()> {
    if !exists_with(slug, &SystemRunner, Platform::current()) {
        return Ok(());
    }
    apply_plan(&uninstall_plan(slug, Platform::current())?, &SystemRunner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::testing::FakeRunner;

    fn job(account: &str) -> BackupJob {
        BackupJob { slug: "my-server".into(), dir: "/home/jo/kora/my server".into(), exe: "/opt/kora/0.8.0-abcdef123456/kora".into(), account: account.into() }
    }
    fn daily() -> Schedule {
        Schedule::Daily(TimeOfDay { hour: 3, minute: 30 })
    }

    #[test]
    fn the_service_unit_runs_as_the_person_and_nothing_else() {
        let u = systemd_service_unit(&job("jo")).unwrap();
        assert!(u.contains("User=jo\n") && u.contains("Type=oneshot") && u.contains("NoNewPrivileges=true"), "{u}");
        assert!(u.contains("ExecStart=/opt/kora/0.8.0-abcdef123456/kora backup create \"/home/jo/kora/my server\" --scheduled\n"), "{u}");
        assert!(!u.contains("passphrase") && !u.contains("--encrypt") && !u.contains("root"));
    }

    #[test]
    fn the_timer_catches_up_after_downtime() {
        let t = systemd_timer_unit(&job("jo"), &daily()).unwrap();
        assert!(t.contains("OnCalendar=*-*-* 03:30:00") && t.contains("Persistent=true") && t.contains("Unit=kora-backup-my-server.service"), "{t}");
        let w = systemd_timer_unit(&job("jo"), &Schedule::Weekly(Weekday::Sun, TimeOfDay { hour: 4, minute: 0 })).unwrap();
        assert!(w.contains("OnCalendar=Sun *-*-* 04:00:00"));
        // what Kora writes, Kora can read back
        let cfg = crate::service::parse_timer(&t, "").unwrap();
        assert_eq!((cfg.time.as_str(), cfg.weekday), ("03:30", None));
    }

    #[test]
    fn the_task_is_the_persons_unelevated_and_runs_a_missed_start() {
        let x = task_xml(&job("PC\\jo"), &daily()).unwrap();
        assert!(x.contains("<UserId>PC\\jo</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel>"), "{x}");
        assert!(x.contains("<StartWhenAvailable>true</StartWhenAvailable>") && x.contains("<StartBoundary>2024-01-01T03:30:00</StartBoundary>"));
        assert!(x.contains("backup create &quot;/home/jo/kora/my server&quot; --scheduled"), "{x}");
        assert!(!x.contains("S-1-5-18") && !x.contains("HighestAvailable"));
        let cfg = crate::service::parse_task_xml(&x).unwrap();
        assert_eq!((cfg.time.as_str(), cfg.run_as), ("03:30", crate::service::RunAs::InstallingUser));
    }

    #[test]
    fn it_never_runs_as_root_or_a_system_account_or_something_odd() {
        for bad in ["root", "", "Jo", "jo\nroot", "jo\"", "-jo", "a/b", "jo%x"] {
            assert!(systemd_service_unit(&job(bad)).is_err(), "{bad:?}");
        }
        for bad in ["SYSTEM", "NT AUTHORITY\\SYSTEM", "NT SERVICE\\kora-main", "PC\\LOCAL SERVICE", "", "PC\\jo\n"] {
            assert!(task_xml(&job(bad), &daily()).is_err(), "{bad:?}");
        }
        assert!(task_xml(&job("PC\\jo"), &daily()).is_ok() && task_xml(&job("jo"), &daily()).is_ok());
        let mut j = job("jo");
        j.slug = "../x".into();
        assert!(install_plan(&j, &daily(), Platform::Linux).is_err());
    }

    #[test]
    fn plans_touch_only_kora_named_things() {
        let p = install_plan(&job("jo"), &daily(), Platform::Linux).unwrap();
        let d = p.descriptions().join("|");
        assert!(d.contains("kora-backup-my-server.timer") && p.steps.len() == 4, "{d}");
        let rn = FakeRunner::default();
        apply_plan(&uninstall_plan("my-server", Platform::Linux).unwrap(), &rn).unwrap();
        let calls = rn.calls();
        assert!(calls.contains(&"systemctl disable --now kora-backup-my-server.timer".to_string()), "{calls:?}");
        assert!(calls.iter().filter(|c| c.starts_with("remove ")).all(|c| c.starts_with("remove /etc/systemd/system/kora-backup-my-server.")), "{calls:?}");
        let rn = FakeRunner::default();
        apply_plan(&uninstall_plan("my-server", Platform::Windows).unwrap(), &rn).unwrap();
        assert_eq!(rn.calls(), ["schtasks.exe /Delete /TN Kora\\Backup my-server /F"]);
        let w = install_plan(&job("PC\\jo"), &daily(), Platform::Windows).unwrap();
        assert!(w.descriptions()[1].contains("Kora\\Backup my-server"));
        assert!(uninstall_plan("a b", Platform::Linux).is_err());
    }

    #[test]
    fn the_job_is_found_by_its_name_and_read_back() {
        let rn = FakeRunner::default();
        rn.respond("cat /etc/systemd/system/kora-backup-x.timer", 0, "[Timer]\nOnCalendar=Mon *-*-* 05:15:00\n");
        assert!(exists_with("x", &rn, Platform::Linux));
        let c = read_back("x", &rn, Platform::Linux).unwrap();
        assert_eq!((c.time.as_str(), c.weekday), ("05:15", Some(0)));
        let none = FakeRunner::default();
        none.respond("cat", 1, "");
        assert!(!exists_with("x", &none, Platform::Linux));
    }

    #[test]
    fn the_time_is_validated_like_the_update_schedule() {
        assert!(schedule_from(None, "03:30").is_ok() && schedule_from(Some(6), "04:00").is_ok());
        for bad in ["3:30", "25:00", "03:60", "tea"] {
            assert!(schedule_from(None, bad).is_err(), "{bad}");
        }
        assert!(schedule_from(Some(7), "03:30").is_err());
    }
}
