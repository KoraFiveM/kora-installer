//! Auto-update scheduler: systemd `.service` + `.timer` and Windows `schtasks`.

use super::{ManagedServer, Platform, Plan, ServiceError, ServiceResult, SystemStep};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeOfDay {
    pub hour: u8,
    pub minute: u8,
}

impl TimeOfDay {
    pub fn new(hour: u8, minute: u8) -> ServiceResult<Self> {
        if hour > 23 || minute > 59 {
            return Err(ServiceError::Invalid(format!("{hour}:{minute} is not a valid time of day")));
        }
        Ok(TimeOfDay { hour, minute })
    }
}

impl std::fmt::Display for TimeOfDay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:02}:{:02}", self.hour, self.minute)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl Weekday {
    fn systemd(self) -> &'static str {
        match self {
            Weekday::Mon => "Mon",
            Weekday::Tue => "Tue",
            Weekday::Wed => "Wed",
            Weekday::Thu => "Thu",
            Weekday::Fri => "Fri",
            Weekday::Sat => "Sat",
            Weekday::Sun => "Sun",
        }
    }
    fn schtasks(self) -> &'static str {
        match self {
            Weekday::Mon => "MON",
            Weekday::Tue => "TUE",
            Weekday::Wed => "WED",
            Weekday::Thu => "THU",
            Weekday::Fri => "FRI",
            Weekday::Sat => "SAT",
            Weekday::Sun => "SUN",
        }
    }
}

/// When auto-update runs (server-local time).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Schedule {
    Daily(TimeOfDay),
    Weekly(Weekday, TimeOfDay),
    /// Chosen days (non-empty) at a time.
    Custom(Vec<Weekday>, TimeOfDay),
}

impl Default for Schedule {
    /// Daily at 04:00.
    fn default() -> Self {
        Schedule::Daily(TimeOfDay { hour: 4, minute: 0 })
    }
}

impl Schedule {
    fn validate(&self) -> ServiceResult<()> {
        match self {
            Schedule::Custom(days, _) if days.is_empty() => {
                Err(ServiceError::Invalid("pick at least one day for a custom schedule".into()))
            }
            _ => Ok(()),
        }
    }

    fn time(&self) -> TimeOfDay {
        match self {
            Schedule::Daily(t) | Schedule::Weekly(_, t) | Schedule::Custom(_, t) => *t,
        }
    }

    /// systemd `OnCalendar=` expression.
    pub fn on_calendar(&self) -> String {
        let t = self.time();
        let time = format!("{:02}:{:02}:00", t.hour, t.minute);
        match self {
            Schedule::Daily(_) => format!("*-*-* {time}"),
            Schedule::Weekly(d, _) => format!("{} *-*-* {time}", d.systemd()),
            Schedule::Custom(days, _) => {
                let list: Vec<_> = days.iter().map(|d| d.systemd()).collect();
                format!("{} *-*-* {time}", list.join(","))
            }
        }
    }

    /// Plain-language summary.
    pub fn describe(&self) -> String {
        match self {
            Schedule::Daily(t) => format!("every day at {t}"),
            Schedule::Weekly(d, t) => format!("every week on {} at {t}", d.systemd()),
            Schedule::Custom(days, t) => {
                let list: Vec<_> = days.iter().map(|d| d.systemd()).collect();
                format!("on {} at {t}", list.join(", "))
            }
        }
    }

    /// Arguments for `schtasks /Create` describing the trigger (`/SC ... /ST ...`).
    fn schtasks_trigger(&self) -> Vec<String> {
        let st = self.time().to_string();
        match self {
            Schedule::Daily(_) => vec!["/SC".into(), "DAILY".into(), "/ST".into(), st],
            Schedule::Weekly(d, _) => {
                vec!["/SC".into(), "WEEKLY".into(), "/D".into(), d.schtasks().into(), "/ST".into(), st]
            }
            Schedule::Custom(days, _) => {
                let list: Vec<_> = days.iter().map(|d| d.schtasks()).collect();
                vec!["/SC".into(), "WEEKLY".into(), "/D".into(), list.join(","), "/ST".into(), st]
            }
        }
    }
}

fn update_unit_name(s: &ManagedServer) -> String {
    format!("kora-update-{}", s.name)
}

fn task_name(s: &ManagedServer) -> String {
    format!("Kora\\Update {}", s.name)
}

fn exec_line(s: &ManagedServer) -> String {
    // The absolute server folder (not the name): root has no registry entry guarantee here.
    format!("{} update --auto {}", super::systemd_quote(&s.kora_exe), super::systemd_quote(&s.dir))
}

/// systemd oneshot service that runs `kora update --auto <server folder>`.
///
/// It runs as root on purpose: restarting the game service (and rolling back) needs systemctl.
/// `kora update` itself runs every git command as the owner of the server folder (`runuser`),
/// so no root-owned files appear in the server. No `PrivateTmp`: found in the dev e2e run that
/// it hides a kora binary or server folder under `/tmp` from the job (status 203/EXEC).
pub fn systemd_update_unit(s: &ManagedServer) -> ServiceResult<String> {
    s.validate()?;
    Ok(format!(
        "[Unit]\n\
         Description=Kora auto-update for {name}\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         Type=oneshot\n\
         WorkingDirectory={dir}\n\
         ExecStart={exec}\n\
         NoNewPrivileges=true\n",
        name = s.name,
        dir = s.dir,
        exec = exec_line(s),
    ))
}

/// systemd timer that triggers the update service.
pub fn systemd_timer_unit(s: &ManagedServer, schedule: &Schedule) -> ServiceResult<String> {
    s.validate()?;
    schedule.validate()?;
    Ok(format!(
        "[Unit]\n\
         Description=Kora auto-update schedule for {name} ({when})\n\
         \n\
         [Timer]\n\
         OnCalendar={cal}\n\
         Persistent=true\n\
         Unit={unit}.service\n\
         \n\
         [Install]\n\
         WantedBy=timers.target\n",
        name = s.name,
        when = schedule.describe(),
        cal = schedule.on_calendar(),
        unit = update_unit_name(s),
    ))
}

fn unit_path(s: &ManagedServer, ext: &str) -> String {
    format!("/etc/systemd/system/{}.{ext}", update_unit_name(s))
}

/// Plan to register the auto-update job.
pub fn scheduler_install_plan(s: &ManagedServer, schedule: &Schedule, platform: Platform) -> ServiceResult<Plan> {
    s.validate()?;
    schedule.validate()?;
    let when = schedule.describe();
    Ok(match platform {
        Platform::Linux => {
            let timer = format!("{}.timer", update_unit_name(s));
            Plan::new(vec![
                SystemStep::write(
                    format!("Create the update job definition for '{}' ({})", s.name, unit_path(s, "service")),
                    &unit_path(s, "service"),
                    systemd_update_unit(s)?,
                    0o644,
                ),
                SystemStep::write(
                    format!("Create the update schedule for '{}': {when} ({})", s.name, unit_path(s, "timer")),
                    &unit_path(s, "timer"),
                    systemd_timer_unit(s, schedule)?,
                    0o644,
                ),
                SystemStep::run("Tell systemd to read the new update job", "systemctl", &["daemon-reload"]),
                SystemStep::run(format!("Turn on the automatic update schedule for '{}'", s.name), "systemctl", &["enable", "--now", &timer]),
            ])
        }
        Platform::Windows => {
            // The absolute folder, not the name: the task runs as SYSTEM, whose %LOCALAPPDATA% has no
            // server registry, so a name would never be found.
            let tr = format!("\"{}\" update --auto \"{}\"", s.kora_exe, s.dir.trim_end_matches(['\\', '/']));
            let mut args = vec!["/Create".into(), "/TN".into(), task_name(s), "/TR".into(), tr];
            args.extend(schedule.schtasks_trigger());
            args.extend(["/RU".into(), s.windows_run_as.clone().unwrap_or_else(|| "SYSTEM".into()), "/RL".into(), "HIGHEST".into(), "/F".into()]);
            Plan::new(vec![SystemStep::run_owned(
                format!("Create the Windows scheduled task '{}' that updates '{}' {when}", task_name(s), s.name),
                "schtasks.exe",
                args,
            )])
        }
    })
}

/// Plan to remove only the update job Kora created.
pub fn scheduler_uninstall_plan(s: &ManagedServer, platform: Platform) -> ServiceResult<Plan> {
    s.validate()?;
    Ok(match platform {
        Platform::Linux => {
            let timer = format!("{}.timer", update_unit_name(s));
            Plan::new(vec![
                SystemStep::run(format!("Turn off the automatic update schedule for '{}'", s.name), "systemctl", &["disable", "--now", &timer]).tolerant(),
                SystemStep::remove(format!("Delete {}", unit_path(s, "timer")), &unit_path(s, "timer")),
                SystemStep::remove(format!("Delete {}", unit_path(s, "service")), &unit_path(s, "service")),
                SystemStep::run("Tell systemd the update job is gone", "systemctl", &["daemon-reload"]),
            ])
        }
        Platform::Windows => Plan::new(vec![SystemStep::run(
            format!("Delete the Windows scheduled task '{}' that Kora created", task_name(s)),
            "schtasks.exe",
            &["/Delete", "/TN", &task_name(s), "/F"],
        )]),
    })
}
