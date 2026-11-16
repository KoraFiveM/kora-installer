//! Auto-update scheduler: systemd `.service` + `.timer` and Windows `schtasks`.

use super::{apply_plan, CommandRunner, ManagedServer, Platform, Plan, ServiceError, ServiceResult, SystemRunner, SystemStep};
use crate::installer::registry::ServerEntry;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
    /// 0 = Monday .. 6 = Sunday.
    pub fn from_index(i: u8) -> ServiceResult<Self> {
        Ok(match i {
            0 => Weekday::Mon,
            1 => Weekday::Tue,
            2 => Weekday::Wed,
            3 => Weekday::Thu,
            4 => Weekday::Fri,
            5 => Weekday::Sat,
            6 => Weekday::Sun,
            _ => return Err(ServiceError::Invalid(format!("weekday {i} is out of range (0 = Monday .. 6 = Sunday)"))),
        })
    }
    pub fn index(self) -> u8 {
        self as u8
    }
    /// English name for plain-language text.
    pub fn long_name(self) -> &'static str {
        ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"][self as usize]
    }
    fn xml(self) -> &'static str {
        self.long_name()
    }
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
}


// ---------------------------------------------------------------------------------------------
// Configuration model (what the manager and `kora schedule` use)
// ---------------------------------------------------------------------------------------------

/// A server as the registry records it.
pub type ServerRecord = ServerEntry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    Daily,
    Weekly,
    /// A chosen time, optionally on one chosen weekday.
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunAs {
    /// The person who installed: their `gh`/`git` sign-in works, so private forks can be fetched.
    InstallingUser,
    /// The machine account: always on, but has no GitHub sign-in (private forks cannot be fetched).
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleConfig {
    pub preset: Preset,
    /// `HH:MM`, 24 hours, server local time.
    pub time: String,
    /// 0 = Monday .. 6 = Sunday (Weekly / Custom).
    pub weekday: Option<u8>,
    pub run_as: RunAs,
    pub enabled: bool,
}

impl ScheduleConfig {
    /// The installer default: every day at 04:00, as the installing user.
    pub fn default_for_install() -> Self {
        ScheduleConfig { preset: Preset::Daily, time: "04:00".into(), weekday: None, run_as: RunAs::InstallingUser, enabled: true }
    }

    /// Check the time format, the weekday range and that Weekly has a weekday.
    pub fn validate(&self) -> ServiceResult<()> {
        self.to_schedule().map(|_| ())
    }

    /// The trigger this configuration describes.
    pub fn to_schedule(&self) -> ServiceResult<Schedule> {
        let time = parse_time(&self.time)?;
        let day = self.weekday.map(Weekday::from_index).transpose()?;
        Ok(match (self.preset, day) {
            (Preset::Daily, _) => Schedule::Daily(time),
            (Preset::Weekly, Some(d)) => Schedule::Weekly(d, time),
            (Preset::Weekly, None) => return Err(ServiceError::Invalid("a weekly schedule needs a weekday".into())),
            (Preset::Custom, Some(d)) => Schedule::Weekly(d, time),
            (Preset::Custom, None) => Schedule::Daily(time),
        })
    }
}

/// `HH:MM` (24 h) exactly; "4:00" and "04:00:00" are rejected so the stored value is canonical.
pub fn parse_time(s: &str) -> ServiceResult<TimeOfDay> {
    let bad = || ServiceError::Invalid(format!("'{s}' is not a time: use 24-hour HH:MM, like 04:00"));
    let b = s.as_bytes();
    if b.len() != 5 || b[2] != b':' || !b.iter().enumerate().all(|(i, c)| i == 2 || c.is_ascii_digit()) {
        return Err(bad());
    }
    TimeOfDay::new(s[0..2].parse().map_err(|_| bad())?, s[3..5].parse().map_err(|_| bad())?).map_err(|_| bad())
}

/// The account name of the person running Kora (the one who ran the installer, even through
/// `sudo`): `DOMAIN\user` on Windows.
pub fn installing_user() -> String {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    if cfg!(windows) {
        let user = env("USERNAME").unwrap_or_else(|| "user".into());
        return match env("USERDOMAIN") {
            Some(d) => format!("{d}\\{user}"),
            None => user,
        };
    }
    env("SUDO_USER").filter(|u| u != "root").or_else(|| env("USER")).or_else(|| env("LOGNAME")).unwrap_or_else(|| "root".into())
}

/// Plain language: "Every day at 04:00, as you (jamie)".
pub fn describe(cfg: &ScheduleConfig) -> String {
    describe_for(cfg, &installing_user())
}

/// [`describe`] with the installing user's name given.
pub fn describe_for(cfg: &ScheduleConfig, user: &str) -> String {
    if !cfg.enabled {
        return "Off: this server is not updated automatically".into();
    }
    let when = match cfg.to_schedule() {
        Ok(Schedule::Daily(t)) => format!("Every day at {t}"),
        Ok(Schedule::Weekly(d, t)) => format!("Every {} at {t}", d.long_name()),
        Ok(Schedule::Custom(days, t)) => format!("On {} at {t}", days.iter().map(|d| d.long_name()).collect::<Vec<_>>().join(", ")),
        Err(_) => format!("Invalid schedule ({})", cfg.time),
    };
    match cfg.run_as {
        RunAs::InstallingUser => format!("{when}, as you ({user})"),
        RunAs::System => format!("{when}, as the system account"),
    }
}

/// Help text for the two choices; shown by `kora schedule --help` and the manager.
pub const RUN_AS_HELP: &str = "\
Run as you (recommended): the update uses YOUR GitHub / git sign-in, so forks of private repositories work. \
On Windows it runs only while you are logged in (no password is stored); if the PC was off or you were \
signed out at that time, it runs at your next login. On Linux it runs from the system timer as the folder's \
owner and also catches up after downtime.\n\
Run as the system account: runs even when nobody is logged in (good for always-on servers), but the system \
account has no GitHub sign-in, so forks of PRIVATE repositories cannot be fetched.";

// ---------------------------------------------------------------------------------------------
// Stored configuration (<server>/.kora/schedule.json is the source of truth)
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Stored {
    #[serde(flatten)]
    config: ScheduleConfig,
    /// Who "you" is (the installing user) when the job was created.
    #[serde(default)]
    user: String,
}

fn stored_path(dir: &Path) -> PathBuf {
    crate::installer::meta_dir(dir).join("schedule.json")
}

fn load_stored(dir: &Path) -> Option<Stored> {
    serde_json::from_slice(&std::fs::read(stored_path(dir)).ok()?).ok()
}

/// Record the schedule the installer created (installer step; the OS job is made by its plan).
pub fn save_schedule(dir: &Path, cfg: &ScheduleConfig, user: &str) -> ServiceResult<()> {
    save_stored(dir, &Stored { config: cfg.clone(), user: user.to_string() })
}

/// Written as root into a folder the service account can change: never through a planted link.
fn save_stored(dir: &Path, st: &Stored) -> ServiceResult<()> {
    crate::fsutil::write_no_follow(&stored_path(dir), &serde_json::to_vec_pretty(st).map_err(std::io::Error::other)?, 0o644)?;
    Ok(())
}

/// Record in `created.json` that Kora made the job (so delete/uninstall remove it), if that file
/// exists; `copy` = the program copy made for it (removed again with the server).
fn mark_created(dir: &Path, on: bool, copy: Option<&Path>) {
    let p = crate::installer::meta_dir(dir).join("created.json");
    if let Ok(b) = std::fs::read(&p) {
        if let Ok(mut v) = serde_json::from_slice::<serde_json::Value>(&b) {
            v["update_timer"] = serde_json::Value::Bool(on);
            if let Some(c) = copy {
                v["binary"] = serde_json::Value::String(c.to_string_lossy().into_owned());
            }
            if let Ok(out) = serde_json::to_vec_pretty(&v) {
                let _ = crate::fsutil::write_no_follow(&p, &out, 0o644);
            }
        }
    }
}

/// The program named in the server's `created.json`, if any. Only a hint (see `binary::choose_root_exe`).
fn recorded_binary(dir: &Path) -> Option<String> {
    crate::installer::load_created_from(dir).and_then(|c| c.binary)
}

/// The ManagedServer the scheduler needs, from a registry entry. The kora program is the one a
/// root job may run ([`super::binary::root_exe`]): never simply the path a server folder's records
/// name (anyone who can write that folder could otherwise choose what root runs). When a copy would
/// have to be made first, this program is named here; [`set`] makes the copy before using it.
pub fn managed_for(rec: &ServerRecord) -> ManagedServer {
    let exe = super::binary::root_exe(recorded_binary(&rec.dir).as_deref())
        .map(|r| r.path().to_string_lossy().into_owned())
        .or_else(|_| std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).map_err(|_| ()))
        .unwrap_or_else(|_| "kora".into());
    managed_with_exe(rec, exe)
}

/// For reading or removing the job: only its name matters, so the kora program is not looked at
/// (finding the root program can mean hashing it, too slow for every status refresh).
fn managed_for_lookup(rec: &ServerRecord) -> ManagedServer {
    managed_with_exe(rec, "kora".into())
}

fn managed_with_exe(rec: &ServerRecord, exe: String) -> ManagedServer {
    let dir = rec.dir.to_string_lossy().into_owned();
    ManagedServer {
        name: crate::installer::slug(&rec.name),
        dir,
        start_command: vec![exe.clone()],
        user: "kora".into(),
        env_file: None,
        kora_exe: exe,
        windows_run_as: None,
    }
}

/// What is stored for the server, checked against the OS: `None` when there is no schedule;
/// `enabled: false` when it is switched off or its OS job has gone missing. A schedule made by an
/// older Kora (no stored file) is read back from the OS job; those ran as SYSTEM.
pub fn get(server: &ServerRecord) -> Option<ScheduleConfig> {
    get_with(&managed_for_lookup(server), &server.dir, &SystemRunner, Platform::current())
}

pub fn get_with(s: &ManagedServer, dir: &Path, rn: &dyn CommandRunner, platform: Platform) -> Option<ScheduleConfig> {
    let os = os_job(s, rn, platform);
    match load_stored(dir) {
        Some(st) => {
            let mut c = st.config;
            if c.enabled && os.is_none() {
                c.enabled = false; // the job was deleted behind our back
            }
            Some(c)
        }
        None => os,
    }
}

/// Read the job Kora registered from the OS (also how an older, SYSTEM-created schedule is found).
fn os_job(s: &ManagedServer, rn: &dyn CommandRunner, platform: Platform) -> Option<ScheduleConfig> {
    match platform {
        Platform::Linux => {
            let timer = rn.run("cat", &[unit_path(s, "timer")]).ok().filter(|o| o.success())?;
            let service = rn.run("cat", &[unit_path(s, "service")]).ok().map(|o| o.stdout).unwrap_or_default();
            parse_timer(&timer.stdout, &service)
        }
        Platform::Windows => {
            let o = rn.run("schtasks.exe", &["/Query".into(), "/TN".into(), task_name(s), "/XML".into()]).ok().filter(|o| o.success())?;
            parse_task_xml(&o.stdout)
        }
    }
}

/// Parse a Kora systemd timer (and its service, for who it runs as).
pub fn parse_timer(timer: &str, service: &str) -> Option<ScheduleConfig> {
    let cal = timer.lines().find_map(|l| l.trim().strip_prefix("OnCalendar="))?.trim();
    let (day, time) = match cal.rsplit_once(' ')? {
        (before, t) if before.ends_with("*-*-*") => (before.trim_end_matches("*-*-*").trim(), t),
        _ => return None,
    };
    let time = time.get(0..5)?.to_string();
    parse_time(&time).ok()?;
    let weekday = day.split(',').next().filter(|d| !d.is_empty()).and_then(|d| ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"].iter().position(|n| *n == d)).map(|i| i as u8);
    let multi = day.contains(',');
    let run_as = if service.lines().any(|l| l.trim().starts_with("Environment=KORA_UPDATE_AS=")) { RunAs::InstallingUser } else { RunAs::System };
    let preset = match (weekday, multi) {
        (None, _) => Preset::Daily,
        (Some(_), false) => Preset::Weekly,
        (Some(_), true) => Preset::Custom,
    };
    Some(ScheduleConfig { preset, time, weekday, run_as, enabled: true })
}

/// Parse `schtasks /Query /XML` output (not localised, unlike the text formats).
pub fn parse_task_xml(xml: &str) -> Option<ScheduleConfig> {
    let tag = |name: &str| -> Option<&str> {
        let open = format!("<{name}>");
        let start = xml.find(&open)? + open.len();
        let end = xml[start..].find(&format!("</{name}>"))? + start;
        Some(&xml[start..end])
    };
    let boundary = tag("StartBoundary")?;
    let time = boundary.split('T').nth(1)?.get(0..5)?.to_string();
    parse_time(&time).ok()?;
    let user = tag("UserId").unwrap_or("").trim();
    let system = user.eq_ignore_ascii_case("SYSTEM") || user == "S-1-5-18" || user.eq_ignore_ascii_case("NT AUTHORITY\\SYSTEM");
    let days = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
    let found: Vec<u8> = days.iter().enumerate().filter(|(_, d)| xml.contains(&format!("<{d} />")) || xml.contains(&format!("<{d}/>"))).map(|(i, _)| i as u8).collect();
    let (preset, weekday) = match found.as_slice() {
        [] => (Preset::Daily, None),
        [d] => (Preset::Weekly, Some(*d)),
        [d, ..] => (Preset::Custom, Some(*d)),
    };
    Some(ScheduleConfig { preset, time, weekday, run_as: if system { RunAs::System } else { RunAs::InstallingUser }, enabled: true })
}

/// Create or replace the job. Needs administrator rights: when not elevated, asks for them
/// (`sudo` / UAC) and re-runs `kora schedule set` elevated.
pub fn set(server: &ServerRecord, cfg: &ScheduleConfig) -> ServiceResult<()> {
    cfg.validate()?;
    if !cfg.enabled {
        return remove(server);
    }
    if !crate::elevate::is_elevated() {
        return relaunch(&set_args(server, cfg));
    }
    let exe = super::binary::root_exe(recorded_binary(&server.dir).as_deref()).map_err(ServiceError::Other)?;
    let s = managed_with_exe(server, exe.path().to_string_lossy().into_owned());
    let user = installing_user();
    let mut plan = Plan::default();
    if let super::binary::RootExe::Copy { src, dest } = &exe {
        plan.extend(super::binary::install_plan(&src.to_string_lossy(), dest));
    }
    plan.extend(schedule_install_plan(&s, cfg, &user, Platform::current())?);
    apply_plan(&plan, &SystemRunner)?;
    save_stored(&server.dir, &Stored { config: cfg.clone(), user })?;
    mark_created(&server.dir, true, matches!(exe, super::binary::RootExe::Copy { .. }).then(|| exe.path()));
    Ok(())
}

/// Remove the job Kora created (nothing else) and the stored setting.
pub fn remove(server: &ServerRecord) -> ServiceResult<()> {
    let s = managed_for_lookup(server);
    if os_job(&s, &SystemRunner, Platform::current()).is_none() && load_stored(&server.dir).is_none() {
        return Ok(()); // nothing of ours
    }
    if !crate::elevate::is_elevated() {
        return relaunch(&["schedule".into(), "off".into(), server.dir.to_string_lossy().into_owned()]);
    }
    if os_job(&s, &SystemRunner, Platform::current()).is_some() {
        apply_plan(&scheduler_uninstall_plan(&s, Platform::current())?, &SystemRunner)?;
    }
    let _ = std::fs::remove_file(stored_path(&server.dir));
    mark_created(&server.dir, false, None);
    Ok(())
}

fn relaunch(args: &[String]) -> ServiceResult<()> {
    let os: Vec<std::ffi::OsString> = args.iter().map(Into::into).collect();
    crate::elevate::relaunch_elevated(&os).map_err(|e| ServiceError::Other(e.to_string()))
}

/// The `kora` arguments that make the same change (used to re-run elevated).
pub fn set_args(server: &ServerRecord, cfg: &ScheduleConfig) -> Vec<String> {
    let mut a = vec!["schedule".to_string(), "set".into(), server.dir.to_string_lossy().into_owned()];
    match (cfg.preset, cfg.weekday) {
        (Preset::Daily, _) | (_, None) => a.extend(["--daily".into(), cfg.time.clone()]),
        (_, Some(d)) => a.extend(["--weekly".into(), Weekday::from_index(d).map(|d| d.systemd().to_string()).unwrap_or_default(), cfg.time.clone()]),
    }
    a.push(if cfg.run_as == RunAs::System { "--as-system" } else { "--as-user" }.into());
    a
}

// ---------------------------------------------------------------------------------------------
// Units and plans
// ---------------------------------------------------------------------------------------------

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
/// It stays a root service on purpose, for both choices: restarting the game service (and rolling
/// back) needs systemctl, and the server folder may belong to the service account. `kora update`
/// runs every git command as the owner of the server folder (`runuser`), so no root-owned files
/// appear. For [`RunAs::InstallingUser`] the unit also records `KORA_UPDATE_AS=<user>`: when that
/// person owns the folder, git runs with THEIR home, so their `gh` / git credentials are used and
/// private forks work. No `PrivateTmp`: it hides a kora binary under `/tmp` (status 203/EXEC).
pub fn systemd_update_unit_as(s: &ManagedServer, run_as: RunAs, user: &str) -> ServiceResult<String> {
    s.validate()?;
    let env = match run_as {
        RunAs::InstallingUser => {
            if user.is_empty() || user.chars().any(|c| c.is_control() || c.is_whitespace() || c == '"' || c == '%') {
                return Err(ServiceError::Invalid("the installing user's name is not usable in a systemd unit".into()));
            }
            format!("Environment=KORA_UPDATE_AS={user}\n")
        }
        RunAs::System => String::new(),
    };
    Ok(format!(
        "[Unit]\n\
         Description=Kora auto-update for {name}\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         Type=oneshot\n\
         WorkingDirectory={dir}\n\
         {env}\
         ExecStart={exec}\n\
         NoNewPrivileges=true\n",
        name = s.name,
        dir = s.dir,
        exec = exec_line(s),
    ))
}

/// The update service as the old (SYSTEM-era) Kora wrote it.
pub fn systemd_update_unit(s: &ManagedServer) -> ServiceResult<String> {
    systemd_update_unit_as(s, RunAs::System, "")
}

/// systemd timer that triggers the update service. `Persistent=true`: a run missed while the
/// machine was off happens at next boot; up to 10 minutes of random delay avoids every server
/// hitting GitHub at the same second.
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
         RandomizedDelaySec=600\n\
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

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// Task Scheduler definition. XML (not `/SC` flags) because only XML can say `StartWhenAvailable`
/// (run at next opportunity after a missed time) and the no-password "interactive token" logon.
pub fn task_xml(s: &ManagedServer, schedule: &Schedule, run_as: RunAs, user: &str) -> ServiceResult<String> {
    s.validate()?;
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
    let principal = match run_as {
        RunAs::System => "<UserId>S-1-5-18</UserId><RunLevel>HighestAvailable</RunLevel>".to_string(),
        RunAs::InstallingUser => {
            if user.is_empty() || user.chars().any(|c| c.is_control()) {
                return Err(ServiceError::Invalid("the installing user's name is not usable".into()));
            }
            format!("<UserId>{}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>HighestAvailable</RunLevel>", xml_escape(user))
        }
    };
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-16\"?>\n\
         <Task version=\"1.2\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\">\n\
         <RegistrationInfo><Description>{desc}</Description></RegistrationInfo>\n\
         <Triggers><CalendarTrigger><StartBoundary>2024-01-01T{t}:00</StartBoundary><Enabled>true</Enabled>{by}</CalendarTrigger></Triggers>\n\
         <Principals><Principal id=\"Author\">{principal}</Principal></Principals>\n\
         <Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><StartWhenAvailable>true</StartWhenAvailable><ExecutionTimeLimit>PT4H</ExecutionTimeLimit><Enabled>true</Enabled></Settings>\n\
         <Actions Context=\"Author\"><Exec><Command>{cmd}</Command><Arguments>{args}</Arguments></Exec></Actions>\n\
         </Task>\n",
        desc = xml_escape(&format!("Kora auto-update for {}", s.name)),
        cmd = xml_escape(&s.kora_exe),
        args = xml_escape(&format!("update --auto \"{}\"", s.dir.trim_end_matches(['\\', '/']))),
    ))
}

/// Where the task definition is written for `schtasks /XML` to read. NOT the server folder: other
/// accounts can change files there, and could swap the definition (and so what runs as SYSTEM)
/// between the write and the read. `%ProgramFiles%\Kora\tasks` only administrators can change.
fn task_xml_path(s: &ManagedServer) -> String {
    let pf = std::env::var("ProgramFiles").ok().filter(|p| !p.trim().is_empty()).unwrap_or_else(|| "C:\\Program Files".into());
    format!("{}\\Kora\\tasks\\update-{}.xml", pf.trim_end_matches(['\\', '/']), s.name)
}

/// Plan to (re)register the job for `cfg` (installing user named `user`).
pub fn schedule_install_plan(s: &ManagedServer, cfg: &ScheduleConfig, user: &str, platform: Platform) -> ServiceResult<Plan> {
    let schedule = cfg.to_schedule()?;
    scheduler_install_plan_as(s, &schedule, cfg.run_as, user, platform)
}

/// The system-account plan (what Kora versions before the configurable schedule created).
pub fn scheduler_install_plan(s: &ManagedServer, schedule: &Schedule, platform: Platform) -> ServiceResult<Plan> {
    scheduler_install_plan_as(s, schedule, RunAs::System, "", platform)
}

pub fn scheduler_install_plan_as(s: &ManagedServer, schedule: &Schedule, run_as: RunAs, user: &str, platform: Platform) -> ServiceResult<Plan> {
    s.validate()?;
    schedule.validate()?;
    let when = schedule.describe();
    let who = match run_as {
        RunAs::InstallingUser => format!("as {user}"),
        RunAs::System => "as the system account".into(),
    };
    Ok(match platform {
        Platform::Linux => {
            let timer = format!("{}.timer", update_unit_name(s));
            Plan::new(vec![
                SystemStep::write(
                    format!("Create the update job definition for '{}' ({})", s.name, unit_path(s, "service")),
                    &unit_path(s, "service"),
                    systemd_update_unit_as(s, run_as, user)?,
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
            let xml = task_xml_path(s);
            Plan::new(vec![
                SystemStep::write(format!("Write the scheduled task definition for '{}'", s.name), &xml, task_xml(s, schedule, run_as, user)?, 0o600),
                SystemStep::run_owned(
                    format!("Create the Windows scheduled task '{}' that updates '{}' {when}, {who}", task_name(s), s.name),
                    "schtasks.exe",
                    vec!["/Create".into(), "/TN".into(), task_name(s), "/XML".into(), xml.clone(), "/F".into()],
                ),
                SystemStep::remove(format!("Delete the temporary file {xml}"), &xml),
            ])
        }
    })
}

/// Plan to remove only the update job Kora created (its own, Kora-named unit files / task).
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `schedule get` (the panel's status refresh) and `off` only need the job's name: the root
    /// program, which may mean hashing this one, is not worked out for them.
    #[test]
    fn reading_the_schedule_does_not_look_for_the_root_program() {
        let rec = ServerEntry {
            name: "My Server".into(),
            dir: "/srv/my".into(),
            mode: crate::installer::Mode::Production,
            ports: crate::installer::registry::PortsRecord { game: 30120, txadmin: 40120 },
            created_unix: 0,
        };
        let s = managed_for_lookup(&rec);
        assert_eq!((s.name.as_str(), s.kora_exe.as_str()), ("my-server", "kora"));
        assert_eq!(unit_path(&s, "timer"), unit_path(&managed_for(&rec), "timer"));
        let rn = super::super::testing::FakeRunner::default();
        rn.respond("cat /etc/systemd/system/kora-update-my-server.timer", 0, "[Timer]\nOnCalendar=*-*-* 05:30:00\n");
        let cfg = get_with(&s, Path::new("/nonexistent"), &rn, Platform::Linux).unwrap();
        assert_eq!(cfg.time, "05:30");
    }
}
