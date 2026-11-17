//! The service's allow-list as a type: [`Op`], parsed strictly by [`parse`] from a wire request
//! (`verb` + `args`, the framing of [`crate::helper::Request`]) and turned back into arguments for
//! the child by [`Op::child_args`]. The service never forwards the caller's text.
//!
//! What is NOT in the list, on purpose: `install` (the install runs as the person, only its
//! administrator steps come here), `uninstall` / `app uninstall` / `helper` (removing Kora itself is
//! done by the MSI / package manager), `schedule ... --as-system` (an update task that runs as the
//! system account on a folder the user controls would be administrator code for that user: the
//! service only makes update tasks that run as the owning user, unelevated), `service-host`,
//! `self-update` to an older release, anything with a licence key on the command line.

use std::collections::BTreeMap;

use crate::privsvc::wire::Rejected;

/// A request line longer than this is refused unread.
pub const MAX_LINE: usize = 16 * 1024;
const MAX_ARGS: usize = 32;
const MAX_ARG: usize = 1024;
const MAX_ENV_VALUE: usize = 2048;
/// The only environment variables a request may set for the child.
pub const SECRET_ENV: &[&str] = &["CFX_LICENSE_KEY", "KORA_DB_ADMIN_PASSWORD"];
/// The wire verbs the service takes (what [`parse`] accepts at all).
pub const VERBS: &[&str] = &["admin-step", "admin-db", "check", "tools", "schedule", "self-update"];
/// Ports are opened only from here up.
pub const FIRST_PORT: u16 = 1024;
/// Never opened, even above 1023: remote administration and database ports.
const REFUSED_PORTS: &[u16] = &[1433, 1434, 3306, 3389, 5432, 5900, 5985, 5986, 47001];

/// Ports the service opens (see the threat model in [`super`]).
pub fn port_ok(port: u16) -> bool {
    port >= FIRST_PORT && !REFUSED_PORTS.contains(&port)
}

/// One installation step (`kora admin-step`, see [`crate::installer::system_ops`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepId {
    PrepareDb,
    BuildDb,
    Firewall,
    Service,
    Schedule,
}

impl StepId {
    pub fn id(self) -> &'static str {
        match self {
            StepId::PrepareDb => "prepare.db",
            StepId::BuildDb => "build.db",
            StepId::Firewall => "system.firewall",
            StepId::Service => "system.service",
            StepId::Schedule => "system.schedule",
        }
    }
    fn from_id(s: &str) -> Option<StepId> {
        [StepId::PrepareDb, StepId::BuildDb, StepId::Firewall, StepId::Service, StepId::Schedule].into_iter().find(|x| x.id() == s)
    }
    /// Steps that can be undone (deleting a server).
    pub fn undoable(self) -> bool {
        self != StepId::PrepareDb
    }
}

/// When the update task runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum When {
    Daily {
        hour: u8,
        minute: u8,
    },
    Weekly {
        day: &'static str,
        hour: u8,
        minute: u8,
    },
    /// Not given: the default (04:00 daily).
    Default,
}

/// The name and ports of the server as the person's server list has them (undo only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Known {
    pub name: String,
    pub game_port: u16,
    pub txadmin_port: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbAction {
    Start,
    Install,
}

/// Everything the service does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// One administrator step of an install (`run`) or a delete (`undo`) for the server in `dir`.
    Step {
        undo: bool,
        step: StepId,
        dir: String,
        known: Option<Known>,
        keep_files: bool,
        keep_db: bool,
    },
    /// Start (or install) Kora's own MariaDB (Windows).
    DbServer(DbAction),
    /// Start whichever database service is installed (the System check's "Start it for me").
    StartDatabase,
    /// Create or change the unattended-update task of the server called `server` on the person's list
    /// (it runs as that person, unelevated).
    ScheduleSet {
        server: String,
        when: When,
    },
    ScheduleOff {
        server: String,
    },
    /// Install Git from the system package manager (Linux only).
    InstallGit,
    /// Apply a verified Kora update (see the self-update slice). `version` only to a newer release.
    UpdateApply {
        version: Option<String>,
    },
}

impl Op {
    /// A short name for the audit log and the progress text: never a path, never a value.
    pub fn label(&self) -> String {
        match self {
            Op::Step { undo, step, known, .. } => format!("{} {}{}", if *undo { "undo" } else { "run" }, step.id(), known.as_ref().map(|k| format!(" [{}]", safe_label(&k.name))).unwrap_or_default()),
            Op::DbServer(DbAction::Start) => "db start".into(),
            Op::DbServer(DbAction::Install) => "db install".into(),
            Op::StartDatabase => "start-database".into(),
            Op::ScheduleSet { server, .. } => format!("schedule set [{}]", safe_label(server)),
            Op::ScheduleOff { server } => format!("schedule off [{}]", safe_label(server)),
            Op::InstallGit => "install git".into(),
            Op::UpdateApply { .. } => "update".into(),
        }
    }

    /// How long one run of this operation may take before the service stops it.
    pub fn time_limit(&self) -> std::time::Duration {
        let mins = match self {
            Op::Step { step: StepId::PrepareDb, .. } | Op::DbServer(DbAction::Install) => 30,
            Op::Step { step: StepId::BuildDb, .. } => 10,
            Op::Step { .. } | Op::ScheduleSet { .. } | Op::ScheduleOff { .. } => 3,
            Op::DbServer(DbAction::Start) | Op::StartDatabase => 5,
            Op::InstallGit => 15,
            Op::UpdateApply { .. } => 20,
        };
        std::time::Duration::from_secs(mins * 60)
    }

    /// The `kora` child's arguments, built from the typed value only.
    pub fn child_args(&self) -> Vec<String> {
        let mut v: Vec<String> = vec!["--no-elevate".into()];
        let s = |x: &str| x.to_string();
        match self {
            Op::Step { undo, step, dir, known, keep_files, keep_db } => {
                v.extend([s("admin-step"), s(if *undo { "undo" } else { "run" }), s("--step"), s(step.id()), s("--dir"), dir.clone()]);
                if let Some(k) = known {
                    v.extend([s("--name"), k.name.clone(), s("--game-port"), k.game_port.to_string(), s("--txadmin-port"), k.txadmin_port.to_string()]);
                }
                if *keep_files {
                    v.push(s("--keep-files"));
                }
                if *keep_db {
                    v.push(s("--keep-db"));
                }
            }
            Op::DbServer(a) => v.extend([s("admin-db"), s(if *a == DbAction::Start { "start" } else { "install" })]),
            Op::StartDatabase => v.extend([s("check"), s("--start-database")]),
            Op::ScheduleSet { server, when } => {
                v.extend([s("schedule"), s("set"), server.clone(), s("--as-user")]);
                match when {
                    When::Daily { hour, minute } => v.extend([s("--daily"), format!("{hour:02}:{minute:02}")]),
                    When::Weekly { day, hour, minute } => v.extend([s("--weekly"), s(day), format!("{hour:02}:{minute:02}")]),
                    When::Default => {}
                }
            }
            Op::ScheduleOff { server } => v.extend([s("schedule"), s("off"), server.clone()]),
            Op::InstallGit => v.extend([s("tools"), s("install"), s("git")]),
            Op::UpdateApply { version } => {
                v.extend([s("self-update"), s("--yes")]);
                if let Some(x) = version {
                    v.extend([s("--version"), x.clone()]);
                }
            }
        }
        v
    }

    /// The server folder this operation works on, if any (the service checks it on its side).
    pub fn dir(&self) -> Option<&str> {
        match self {
            Op::Step { dir, .. } => Some(dir),
            _ => None,
        }
    }
}

fn no<T>(m: &str) -> Result<T, Rejected> {
    Err(Rejected(m.to_string()))
}

/// A printable-safe version of a server name for logs.
fn safe_label(n: &str) -> String {
    n.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ' ')).take(48).collect()
}

/// A server's name on the person's list: what Kora lets people type, nothing that looks like a path.
pub fn name_ok(n: &str) -> bool {
    !n.is_empty() && n.len() <= 64 && !n.starts_with(['.', ' ', '-']) && !n.ends_with(' ') && n.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-'))
}

/// A full folder path as either OS writes it (drive letter or `/`), without `..`, control characters,
/// `\\?\` / UNC forms or a stray quote. The service checks the real folder separately.
pub fn dir_shape_ok(d: &str) -> bool {
    if d.is_empty() || d.len() > MAX_ARG || d.chars().any(|c| c.is_control() || matches!(c, '"' | '<' | '>' | '|' | '*' | '?')) {
        return false;
    }
    let b = d.as_bytes();
    let drive = b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/');
    let unix = d.starts_with('/') && !d.starts_with("//");
    (drive || unix) && !d.split(['/', '\\']).any(|p| p == ".." || p == ".")
}

/// `HH:MM`, 24 hours.
fn hhmm(s: &str) -> Option<(u8, u8)> {
    let (h, m) = s.split_once(':')?;
    if h.len() != 2 || m.len() != 2 {
        return None;
    }
    let (h, m): (u8, u8) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some((h, m))
}

/// `vX.Y.Z` or `X.Y.Z`, optional `-pre` of letters, digits and dots.
pub fn version_ok(v: &str) -> bool {
    let core = v.strip_prefix('v').unwrap_or(v);
    let (nums, pre) = core.split_once('-').map_or((core, None), |(a, b)| (a, Some(b)));
    let parts: Vec<&str> = nums.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|p| !p.is_empty() && p.len() <= 6 && p.bytes().all(|b| b.is_ascii_digit()))
        && pre.is_none_or(|p| !p.is_empty() && p.len() <= 24 && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.'))
}

struct Walk<'a> {
    a: &'a [String],
    i: usize,
}

impl<'a> Walk<'a> {
    fn next(&mut self) -> Option<&'a str> {
        let r = self.a.get(self.i).map(String::as_str);
        self.i += 1;
        r
    }
    fn done(&self) -> bool {
        self.i >= self.a.len()
    }
}

/// Parse a wire request into an [`Op`]. `env` is checked here too: only [`SECRET_ENV`] names, sane values.
pub fn parse(verb: &str, args: &[String], env: &BTreeMap<String, String>) -> Result<Op, Rejected> {
    if args.len() > MAX_ARGS || args.iter().any(|a| a.len() > MAX_ARG || a.contains('\0')) {
        return no("The Kora service refused the request (too many or too long arguments).");
    }
    if env.iter().any(|(k, v)| !SECRET_ENV.contains(&k.as_str()) || v.len() > MAX_ENV_VALUE || v.contains('\0')) {
        return no("The Kora service does not accept that environment setting.");
    }
    let mut w = Walk { a: args, i: 0 };
    let op = match verb {
        "admin-step" => parse_step(&mut w)?,
        "admin-db" => match (w.next(), w.done()) {
            (Some("start"), true) => Op::DbServer(DbAction::Start),
            (Some("install"), true) => Op::DbServer(DbAction::Install),
            _ => return no("The Kora service does not do that with the database server."),
        },
        "check" => match (w.next(), w.done()) {
            (Some("--start-database"), true) => Op::StartDatabase,
            _ => return no("The Kora service does not do that."),
        },
        "tools" => match (w.next(), w.next(), w.done()) {
            (Some("install"), Some("git"), true) => Op::InstallGit,
            _ => return no("The Kora service installs only Git."),
        },
        "schedule" => parse_schedule(&mut w)?,
        "self-update" => parse_update(&mut w)?,
        _ => return no("The Kora service does not run that."),
    };
    if !w.done() {
        return no("The Kora service refused the request (unexpected arguments).");
    }
    Ok(op)
}

fn parse_step(w: &mut Walk) -> Result<Op, Rejected> {
    let undo = match w.next() {
        Some("run") => false,
        Some("undo") => true,
        _ => return no("The Kora service does not do that step."),
    };
    let (mut step, mut dir, mut name, mut game, mut tx, mut keep_files, mut keep_db) = (None, None, None, None, None, false, false);
    while !w.done() {
        let Some(flag) = w.next() else { break };
        match flag {
            "--step" => step = w.next().and_then(StepId::from_id),
            "--dir" => dir = w.next().map(str::to_string),
            "--name" => name = w.next().map(str::to_string),
            "--game-port" => game = w.next().and_then(|p| p.parse::<u16>().ok()),
            "--txadmin-port" => tx = w.next().and_then(|p| p.parse::<u16>().ok()),
            "--keep-files" => keep_files = true,
            "--keep-db" => keep_db = true,
            _ => return no("The Kora service refused the request (unknown option)."),
        }
    }
    let Some(step) = step else { return no("The Kora service does not do that step.") };
    if undo && !step.undoable() {
        return no("The Kora service does not undo that step.");
    }
    let Some(dir) = dir.filter(|d| dir_shape_ok(d)) else { return no("The Kora service needs the server's full folder path.") };
    let known = match (name, game, tx) {
        (None, None, None) => None,
        (Some(n), Some(g), Some(t)) if undo && name_ok(&n) => Some(Known { name: n, game_port: g, txadmin_port: t }),
        _ => return no("The Kora service refused the server's name and ports."),
    };
    Ok(Op::Step { undo, step, dir, known, keep_files: keep_files && undo, keep_db: keep_db && undo })
}

fn parse_schedule(w: &mut Walk) -> Result<Op, Rejected> {
    let action = w.next();
    let Some(server) = w.next().filter(|s| name_ok(s)) else {
        return no("The Kora service takes a server's name from your server list, not a folder.");
    };
    let server = server.to_string();
    match action {
        Some("off") => Ok(Op::ScheduleOff { server }),
        Some("set") => {
            let mut when = When::Default;
            while !w.done() {
                match w.next() {
                    Some("--as-user") => {}
                    Some("--daily") if when == When::Default => {
                        let Some((hour, minute)) = w.next().and_then(hhmm) else { return no("The update time must look like 04:00.") };
                        when = When::Daily { hour, minute };
                    }
                    Some("--weekly") if when == When::Default => {
                        let given = w.next();
                        let day = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"].into_iter().find(|d| Some(*d) == given);
                        let (Some(day), Some((hour, minute))) = (day, w.next().and_then(hhmm)) else { return no("The weekly update time must look like: mon 04:00.") };
                        when = When::Weekly { day, hour, minute };
                    }
                    // `--as-system` and anything else: not through the service (see the module docs).
                    _ => return no("The Kora service only makes update tasks that run as you."),
                }
            }
            Ok(Op::ScheduleSet { server, when })
        }
        _ => no("The Kora service does not do that with update schedules."),
    }
}

fn parse_update(w: &mut Walk) -> Result<Op, Rejected> {
    let mut version = None;
    while !w.done() {
        match w.next() {
            Some("--yes" | "-y") => {}
            Some("--version") => match w.next().filter(|v| version_ok(v)) {
                Some(v) => version = Some(v.to_string()),
                None => return no("That is not a Kora version number."),
            },
            _ => return no("The Kora service refused the update request."),
        }
    }
    Ok(Op::UpdateApply { version })
}

/// Checks that need more than the request: the running version for updates, and for steps that the
/// ports about to be opened are acceptable (`ports`: the game and txAdmin ports from the server's
/// install record, which the service reads itself). Returns the refusal to show.
pub fn check_context(op: &Op, own_version: &str, ports: Option<(u16, u16)>) -> Result<(), Rejected> {
    match op {
        Op::UpdateApply { version: Some(v) } if crate::app_install::compare_versions(v, own_version) != Some(std::cmp::Ordering::Greater) => {
            no("Kora only updates itself to a newer version from here. To go back to an older one, uninstall and install that version.")
        }
        Op::Step { undo: false, step: StepId::Firewall, .. } => match ports {
            Some((g, t)) if port_ok(g) && port_ok(t) && g != t => Ok(()),
            Some((g, t)) => {
                let bad = if !port_ok(g) { g } else { t };
                Err(Rejected(format!("Kora does not open port {bad} in the firewall from the panel: Windows, remote administration or databases use it. Choose other ports for the server (30120 and 40120 are the usual ones).")))
            }
            None => no("The server's install record has no ports."),
        },
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(verb: &str, args: &[&str]) -> Result<Op, Rejected> {
        parse(verb, &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(), &BTreeMap::new())
    }
    const DIR: &str = "/home/jo/kora/my server";

    #[test]
    fn the_listed_operations_parse() {
        assert!(matches!(p("admin-step", &["run", "--step", "system.service", "--dir", DIR]), Ok(Op::Step { undo: false, step: StepId::Service, .. })));
        assert!(p("admin-step", &["run", "--step", "prepare.db", "--dir", r"C:\Users\Jo Smith\kora\a"]).is_ok());
        let u = p("admin-step", &["undo", "--step", "build.db", "--dir", DIR, "--name", "my-server", "--game-port", "30120", "--txadmin-port", "40120", "--keep-files", "--keep-db"]).unwrap();
        assert!(matches!(&u, Op::Step { keep_files: true, keep_db: true, known: Some(k), .. } if k.name == "my-server"));
        assert_eq!(p("admin-db", &["start"]), Ok(Op::DbServer(DbAction::Start)));
        assert_eq!(p("check", &["--start-database"]), Ok(Op::StartDatabase));
        assert_eq!(p("tools", &["install", "git"]), Ok(Op::InstallGit));
        assert_eq!(p("schedule", &["off", "my-server"]), Ok(Op::ScheduleOff { server: "my-server".into() }));
        assert_eq!(p("schedule", &["set", "a", "--daily", "03:30"]), Ok(Op::ScheduleSet { server: "a".into(), when: When::Daily { hour: 3, minute: 30 } }));
        assert_eq!(p("schedule", &["set", "a", "--weekly", "sun", "04:00", "--as-user"]), Ok(Op::ScheduleSet { server: "a".into(), when: When::Weekly { day: "sun", hour: 4, minute: 0 } }));
        assert_eq!(p("self-update", &["--yes"]), Ok(Op::UpdateApply { version: None }));
        assert_eq!(p("self-update", &["--version", "v0.8.0"]), Ok(Op::UpdateApply { version: Some("v0.8.0".into()) }));
    }

    #[test]
    fn everything_else_is_refused() {
        for (verb, args) in [
            ("install", vec!["--name", "a", "--dir", DIR]),
            ("uninstall", vec!["--yes"]),
            ("app", vec!["uninstall"]),
            ("helper", vec!["install"]),
            ("service-host", vec!["a"]),
            ("delete", vec!["a"]),
            ("cmd.exe", vec!["/c", "whoami"]),
            ("", vec![]),
            ("ping", vec![]),
            ("admin-step", vec!["undo", "--step", "prepare.db", "--dir", DIR]),
            ("admin-step", vec!["run", "--step", "build.config", "--dir", DIR]),
            ("admin-step", vec!["run", "--step", "system.service"]),
            ("admin-step", vec!["run", "--step", "system.service", "--dir", "relative/dir"]),
            ("admin-step", vec!["run", "--step", "system.service", "--dir", "/srv/../etc"]),
            ("admin-step", vec!["run", "--step", "system.service", "--dir", r"\\?\C:\x"]),
            ("admin-step", vec!["run", "--step", "system.service", "--dir", r"\\server\share\x"]),
            ("admin-step", vec!["run", "--step", "system.service", "--dir", DIR, "; whoami"]),
            ("admin-step", vec!["run", "--step", "system.service", "--dir", DIR, "--name", "a", "--game-port", "1", "--txadmin-port", "2"]),
            ("admin-step", vec!["undo", "--step", "system.service", "--dir", DIR, "--name", "../x", "--game-port", "30120", "--txadmin-port", "40120"]),
            ("admin-step", vec!["undo", "--step", "system.service", "--dir", DIR, "--name", "a"]),
            ("admin-db", vec!["drop"]),
            ("admin-db", vec!["start", "--other"]),
            ("check", vec![]),
            ("tools", vec!["install", "node"]),
            ("schedule", vec!["show", "a"]),
            ("schedule", vec!["off", "/srv/other"]),
            ("schedule", vec!["off", r"C:\Users\x\srv"]),
            ("schedule", vec!["off", ".."]),
            ("schedule", vec!["set", "a", "--as-system"]),
            ("schedule", vec!["set", "a", "--daily", "25:00"]),
            ("schedule", vec!["set", "a", "--daily", "4:00"]),
            ("schedule", vec!["set", "a", "--daily", "04:00", "--weekly", "mon", "04:00"]),
            ("self-update", vec!["--version", "latest"]),
            ("self-update", vec!["--version", "v1.2.3; rm"]),
            ("self-update", vec!["--check"]),
        ] {
            assert!(p(verb, &args).is_err(), "{verb} {args:?}");
        }
    }

    #[test]
    fn only_the_two_secrets_may_ride_in_the_environment() {
        let args: Vec<String> = ["--start-database"].iter().map(|s| s.to_string()).collect();
        let mut env = BTreeMap::new();
        env.insert("CFX_LICENSE_KEY".to_string(), "cfxk_x".to_string());
        assert!(parse("check", &args, &env).is_ok());
        for bad in ["PATH", "LD_PRELOAD", "KORA_MARIADB_HOME", "KORA_SELF_UPDATE_API", "GIT_SSH_COMMAND", "HOME", "USERPROFILE"] {
            let mut e = BTreeMap::new();
            e.insert(bad.to_string(), "x".to_string());
            assert!(parse("check", &args, &e).is_err(), "{bad}");
        }
        let mut e = BTreeMap::new();
        e.insert("CFX_LICENSE_KEY".to_string(), "a\0b".to_string());
        assert!(parse("check", &args, &e).is_err());
    }

    #[test]
    fn the_child_gets_arguments_made_from_the_typed_value() {
        let op = p("admin-step", &["undo", "--step", "system.firewall", "--dir", DIR, "--keep-db", "--name", "a", "--game-port", "30120", "--txadmin-port", "40120"]).unwrap();
        assert_eq!(op.child_args(), ["--no-elevate", "admin-step", "undo", "--step", "system.firewall", "--dir", DIR, "--name", "a", "--game-port", "30120", "--txadmin-port", "40120", "--keep-db"]);
        // the child's text round-trips to the same operation (what the service runs is what it checked)
        for op in
            [op, Op::ScheduleSet { server: "a".into(), when: When::Weekly { day: "fri", hour: 2, minute: 5 } }, Op::UpdateApply { version: Some("v1.0.0".into()) }, Op::InstallGit, Op::StartDatabase]
        {
            let a = op.child_args();
            assert_eq!(a[0], "--no-elevate");
            assert_eq!(parse(&a[1], &a[2..], &BTreeMap::new()), Ok(op.clone()), "{a:?}");
        }
        let s = Op::ScheduleSet { server: "a".into(), when: When::Default };
        assert!(s.child_args().contains(&"--as-user".to_string()), "update tasks always run as the person");
        let u = Op::UpdateApply { version: None };
        assert_eq!(u.child_args(), ["--no-elevate", "self-update", "--yes"]);
    }

    #[test]
    fn firewall_ports_and_downgrades_are_checked_with_context() {
        let step = p("admin-step", &["run", "--step", "system.firewall", "--dir", DIR]).unwrap();
        assert!(check_context(&step, "0.8.0", Some((30120, 40120))).is_ok());
        assert!(check_context(&step, "0.8.0", Some((3389, 40120))).unwrap_err().0.contains("3389"));
        assert!(check_context(&step, "0.8.0", Some((445, 40120))).is_err());
        assert!(check_context(&step, "0.8.0", Some((30120, 30120))).is_err());
        assert!(check_context(&step, "0.8.0", None).is_err());
        let undo = p("admin-step", &["undo", "--step", "system.firewall", "--dir", DIR]).unwrap();
        assert!(check_context(&undo, "0.8.0", None).is_ok(), "closing is always fine");
        let up = |v: &str| check_context(&Op::UpdateApply { version: Some(v.into()) }, "0.8.0", None);
        assert!(up("v0.8.1").is_ok());
        assert!(up("v0.8.0").is_err() && up("v0.7.0").is_err());
        assert!(check_context(&Op::UpdateApply { version: None }, "0.8.0", None).is_ok());
    }

    #[test]
    fn labels_never_carry_paths_or_secrets() {
        let op = p("admin-step", &["undo", "--step", "system.service", "--dir", DIR, "--name", "my server", "--game-port", "30120", "--txadmin-port", "40120"]).unwrap();
        assert_eq!(op.label(), "undo system.service [my server]");
        assert!(!op.label().contains("/home"));
    }

    #[test]
    fn shapes() {
        assert!(name_ok("My Server-1") && !name_ok("") && !name_ok("../x") && !name_ok("a/b") && !name_ok(".hidden") && !name_ok("a\\b") && !name_ok(&"x".repeat(65)));
        assert!(dir_shape_ok("C:\\Users\\jo\\kora") && dir_shape_ok("D:/kora/a") && dir_shape_ok("/srv/a"));
        assert!(!dir_shape_ok("C:") && !dir_shape_ok("//host/share") && !dir_shape_ok("/a/./b") && !dir_shape_ok("/a\n/b") && !dir_shape_ok("/a\"b"));
        assert!(version_ok("v0.8.0") && version_ok("1.2.3-rc.1") && !version_ok("1.2") && !version_ok("v1.2.3-") && !version_ok("1.2.x"));
        assert!(port_ok(30120) && !port_ok(1023) && !port_ok(3306) && !port_ok(3389));
    }

    #[test]
    fn every_operation_has_a_time_limit() {
        for op in [Op::StartDatabase, Op::InstallGit, Op::UpdateApply { version: None }, Op::DbServer(DbAction::Install)] {
            let t = op.time_limit();
            assert!(t >= std::time::Duration::from_secs(60) && t <= std::time::Duration::from_secs(3600));
        }
    }
}
