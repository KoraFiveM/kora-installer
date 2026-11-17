//! **Not wired up in v0.7.0** (moved to a follow-up release): nothing in the panel or the CLI
//! starts or talks to this service yet. What IS used now: the ownership [`Ledger`] and
//! [`port_allowed`] (by the worker's elevated steps), and [`in_service`] (always false until the
//! service exists).
//!
//! The opt-in persistent Windows helper: a Windows service (`KoraHelper`, LocalSystem) that does a
//! SHORT list of administrator steps for ONE Windows account without a permission prompt, so Kora
//! Panel can install and remove servers when nobody is at the desktop (remote sessions).
//!
//! Off by default. Someone at the computer turns it on (Settings, or `kora helper install`) with ONE
//! Windows permission prompt, through the short-lived worker ([`super::worker`]); turning it off is
//! the same route. Every decision is a pure function in this file (tested on every OS); the Windows
//! calls are in [`super::winpipe`] (`serve_helper`, `open_helper`).
//!
//! # Shape
//!
//! * **Program.** The installer copies the running `kora.exe` (while the worker holds it locked
//!   against renaming and writing) to [`helper_dir`] = `%ProgramFiles%\Kora\helper\kora.exe`, a
//!   folder whose permissions it resets to SYSTEM and Administrators (full) and Users (read and
//!   run), owner Administrators, nothing inherited. It checks the copy's SHA-256 against the
//!   source. The service runs `"<that path>" helper serve` (quoted: no unquoted-path problem). DLLs
//!   are looked up in that same admin-only folder first.
//! * **Who it serves.** [`ServiceConfig`] (`helper.json` next to the copy) records the installing
//!   account's SID, taken from the panel's token by the worker, never from a name. Each connection
//!   is identified from the client's token (impersonation at identification level), and served
//!   only when its user SID is that SID and its integrity is medium or higher ([`check_client`]).
//! * **Pipe.** `\\.\pipe\kora-helper`, created by the service as the first instance (a squatter
//!   makes the service fail instead of sharing the name), remote clients rejected, and the rule
//!   [`pipe_sddl`]: owner SYSTEM, network logons denied, the one user and SYSTEM may read and write.
//!   The panel checks that the pipe's OWNER is SYSTEM before it sends anything ([`check_server_owner`]):
//!   only a SYSTEM process (or an administrator with the restore privilege) can create an object
//!   owned by SYSTEM, so the person's own malware cannot pretend to be the helper and collect the
//!   licence key or database password. The panel opens the pipe for identification only.
//! * **Protocol.** The worker's ([`super::worker::serve`]), with three differences: the Kora folder is
//!   fixed at install time (a `session` naming another one is refused, [`session_home_ok`]); the
//!   verbs are [`SERVICE_VERBS`] and every request must pass [`route`] AND the validator in
//!   `kora-cli` (`service_validate`); and privileged runs are one at a time across connections.
//!   Children get [`SERVICE_ENV`]`=1`, which turns on the extra checks in
//!   `installer::system_ops::run_admin_step` (ownership ledger, firewall ports, folder grants left
//!   to the person).
//! * **Versions.** The panel uses the helper only when `ping` reports exactly its own version.
//!   When the helper is older, the panel asks it to update itself ([`refresh_allowed`]): the
//!   service downloads THAT release from Kora's GitHub releases, checks it against the release's
//!   SHA-256 (the self-update path), replaces its own copy and restarts. Until it matches, the
//!   panel uses the Windows prompt as before.
//! * **Robustness.** Starts at boot (`start= auto`), restarted by Windows after a crash
//!   (`sc failure`), and the panel falls back to the prompt whenever the helper does not answer.
//!   Logs (`helper.log` next to the copy, admin-only, capped) hold verbs, the step id and the
//!   outcome, never arguments or environment values.
//!
//! # Threat model (the stronger attacker)
//!
//! With the helper on, ANY program running as the installing user (including malware that the
//! person ran by mistake) can ask for the allow-listed operations without a prompt, with folders
//! and records it crafted itself. So the allow-list and the argument checks are the only barrier
//! between that user and SYSTEM. What the service does, and why it is safe against that:
//!
//! | operation | through the service | why |
//! |---|---|---|
//! | `admin-step run/undo system.firewall` | yes | opens/closes inbound ports for one server; ports below 1024 and remote-administration ports ([`port_allowed`]) refused; undo only for rules the ledger says this user made |
//! | `admin-step run/undo system.service` | yes | registers `kora-<name>` running Kora's admin-only program copy as the service's own virtual account, with only ordinary-user privileges (`sc privs`: no impersonation, which would lead to SYSTEM); the folder rights are granted by the person's own process, so SYSTEM never changes permissions inside a folder the user controls (junction tricks); undo only for the user's own services (ledger); the program copy is never deleted by the service |
//! | `admin-step run/undo build.db`, `run prepare.db`, `admin-db start/install`, `check --start-database` | yes | Kora's own MariaDB: pinned download checksum, admin-only folder, database and user names Kora generates; drop only for databases the ledger says this user made |
//! | `admin-step run/undo system.schedule`, `schedule set/off` | **no** (prompt) | the update task runs Kora's update on a folder the user controls, as SYSTEM or with the person's FULL administrator token (`HighestAvailable`): a crafted folder would turn that into administrator code |
//! | `self-update`, `app`, `uninstall` | **no** (prompt) | replace or remove machine-wide programs and everything Kora made: rare, destructive, and not needed remotely |
//! | `helper install/uninstall` | **no** (prompt, local only) | turning the helper on or off always needs someone at the computer |
//! | `helper refresh --version V` | yes | only to a NEWER version than the helper's own, downloaded from Kora's releases and checked against the release checksum: the caller cannot choose the file |
//!
//! Folders: an `admin-step` folder must hold a Kora install record and be OWNED by the helper's
//! user (not by Administrators, unlike the prompt path), must contain no link on the way, and the
//! steps never write into it. The ownership ledger ([`Ledger`], `%ProgramFiles%\Kora\owners.json`,
//! admin-only) records which Windows account each service, firewall rule set, update task and
//! database was made for, so on a computer shared by several accounts one account's programs
//! cannot remove another's.
//!
//! Residual risk, in the words the opt-in screen uses ([`RISK_TEXT`]).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::service::{Plan, SystemStep};

/// The Windows service name. Deliberately NOT `kora-<something>`: server services are
/// `kora-<name>`, and no server name can produce this one.
pub const SERVICE_NAME: &str = "KoraHelper";
pub const DISPLAY_NAME: &str = "Kora helper (opt-in, for Kora Panel)";
/// The helper's pipe.
pub const PIPE: &str = super::PIPE_NAME;
/// Set for the service's children: the stricter checks of the persistent helper apply.
pub const SERVICE_ENV: &str = "KORA_HELPER_SERVICE";
/// The top-level `kora` subcommands the service runs at all ([`route`] narrows them further).
pub const SERVICE_VERBS: &[&str] = &["admin-step", "admin-db", "check", "helper"];
/// The machine account.
pub const SYSTEM_SID: &str = "S-1-5-18";
/// `SECURITY_MANDATORY_MEDIUM_RID`.
pub const MEDIUM_INTEGRITY: u32 = 0x2000;
/// How long the panel waits for the helper to answer before using the prompt instead.
pub const CONNECT_WAIT_MS: u32 = 2000;

/// What the opt-in screen says about the risk (two or three plain sentences).
pub const RISK_TEXT: &str = "Kora's helper is a small Windows service that does a few administrator jobs for your Windows account without asking each time, so installs and removals work when you use Kora from another device. \
While it is on, any program running as you, including a harmful one, could also ask it to open firewall ports for a server, register a Kora server service in a folder you own, or create and remove Kora's own databases; it never runs other programs as administrator and only you can use it. \
Turn it off here at any time.";

/// What the helper does (the list Settings shows).
pub const DOES: &[&str] = &[
    "Open and close the firewall ports of your servers",
    "Register and remove your servers' Windows services",
    "Set up, start and use Kora's own database server (MariaDB)",
    "Keep itself up to date with Kora (official releases only)",
];
/// What still asks for permission at the computer.
pub const DOES_NOT: &[&str] = &["Automatic update schedules", "Updating or removing Kora itself", "Turning the helper on or off"];

/// Recorded at install time (`helper.json` next to the program copy; admin-only folder, no secrets).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceConfig {
    /// The only account served (`S-1-5-21-...`).
    pub sid: String,
    pub user: String,
    pub domain: String,
    /// That account's Kora folder (fixed: the panel cannot point the helper at another one).
    pub kora_home: String,
    pub version: String,
}

impl ServiceConfig {
    /// The whole environment of every child (built from nothing, see
    /// [`super::worker::elevated_child_env`]): system folders, who "you" is, the fixed Kora
    /// folder, service mode.
    pub fn child_env(&self, sys: &super::worker::SystemDirs, profile: Option<&str>) -> Vec<(String, String)> {
        let u = super::winpipe::PanelUser { sid: self.sid.clone(), domain: self.domain.clone(), name: self.user.clone(), admin_member: false };
        let mut v = super::worker::elevated_child_env(sys, &u, profile, Some(&self.kora_home));
        v.push((SERVICE_ENV.into(), "1".into()));
        v
    }
}

/// Is this process a child of the persistent helper (the stricter checks apply)?
pub fn in_service() -> bool {
    std::env::var(SERVICE_ENV).is_ok_and(|v| v == "1")
}

/// A user SID as Windows writes it, and an ordinary account's: `S-1-5-21-<3 numbers>-<RID>`
/// (local or domain account) or `S-1-12-1-<4 numbers>` (Microsoft Entra ID account). SYSTEM,
/// groups and well-known SIDs are refused.
pub fn valid_user_sid(s: &str) -> bool {
    let Some(rest) = s.strip_prefix("S-1-5-21-").or_else(|| s.strip_prefix("S-1-12-1-")) else { return false };
    let parts: Vec<&str> = rest.split('-').collect();
    parts.len() == 4 && parts.iter().all(|p| !p.is_empty() && p.len() <= 10 && p.bytes().all(|b| b.is_ascii_digit()))
}

/// The pipe's security descriptor: owner SYSTEM (the panel checks it), network logons denied,
/// the one user and SYSTEM may read and write; protected (nothing inherited).
pub fn pipe_sddl(user_sid: &str) -> Result<String, String> {
    if !valid_user_sid(user_sid) {
        return Err(format!("'{user_sid}' is not an ordinary Windows account"));
    }
    Ok(format!("O:SYG:SYD:P(D;;GA;;;NU)(A;;GRGW;;;{user_sid})(A;;GA;;;SY)"))
}

/// Service side: may this client be served? `client_sid` from its token; `integrity` = the RID of
/// its mandatory label.
pub fn check_client(client_sid: Option<&str>, integrity: Option<u32>, cfg: &ServiceConfig) -> Result<(), String> {
    match client_sid {
        Some(s) if s.eq_ignore_ascii_case(&cfg.sid) => {}
        Some(_) => return Err("Kora's helper only works for the Windows account that turned it on.".into()),
        None => return Err("Kora's helper could not see who is asking.".into()),
    }
    match integrity {
        Some(i) if i >= MEDIUM_INTEGRITY => Ok(()),
        _ => Err("Kora's helper does not take requests from restricted (low integrity) programs.".into()),
    }
}

/// Panel side: is the pipe really the helper's (owned by SYSTEM)?
pub fn check_server_owner(owner_sid: Option<&str>) -> Result<(), String> {
    match owner_sid {
        Some(s) if s.eq_ignore_ascii_case(SYSTEM_SID) => Ok(()),
        Some(s) => Err(format!("the helper's pipe belongs to {s}, not to Windows (SYSTEM): another program may be pretending to be the helper")),
        None => Err("cannot see who made the helper's pipe".into()),
    }
}

/// `%ProgramFiles%\Kora\helper`.
pub fn helper_dir(program_files: &Path) -> PathBuf {
    program_files.join("Kora").join("helper")
}
pub fn helper_exe(dir: &Path) -> PathBuf {
    dir.join("kora.exe")
}
pub fn config_path(dir: &Path) -> PathBuf {
    dir.join("helper.json")
}
pub fn log_path(dir: &Path) -> PathBuf {
    dir.join("helper.log")
}
/// `%ProgramFiles%\Kora\owners.json` (see [`Ledger`]).
pub fn ledger_path(program_files: &Path) -> PathBuf {
    program_files.join("Kora").join("owners.json")
}

/// `%ProgramFiles%` as SYSTEM sees it (services get the machine environment, not a user's).
pub fn program_files() -> Option<PathBuf> {
    std::env::var_os("ProgramFiles").filter(|p| !p.is_empty()).map(PathBuf::from)
}

/// The service's command line: the program path quoted (spaces in `Program Files`).
pub fn bin_path(exe: &Path) -> Result<String, String> {
    let e = exe.to_string_lossy();
    if e.contains('"') || e.chars().any(char::is_control) || !exe.is_absolute() && !e.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return Err(format!("{e} cannot be used as the helper's program path"));
    }
    Ok(format!("\"{e}\" helper serve"))
}

/// `icacls` arguments that make the helper folder administrators-only (users may read and run).
pub fn folder_acl_args(dir: &Path) -> Vec<String> {
    vec![dir.to_string_lossy().into_owned(), "/inheritance:r".into(), "/grant:r".into(), "*S-1-5-18:(OI)(CI)F".into(), "*S-1-5-32-544:(OI)(CI)F".into(), "*S-1-5-32-545:(OI)(CI)RX".into()]
}

/// Steps to turn the helper on (run elevated), in two parts: up to the program copy, and the rest.
/// The caller checks the copy's checksum in between (see `kora-cli`'s `helper install`), so the
/// service never starts a file that differs from the program the person approved.
pub fn install_plan(program_files: &Path, src_exe: &str, cfg: &ServiceConfig) -> Result<(Plan, Plan), String> {
    let dir = helper_dir(program_files);
    let exe = helper_exe(&dir);
    let d = dir.to_string_lossy().into_owned();
    let json = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())? + "\n";
    let copy = Plan::new(vec![
        SystemStep::run("Stop an earlier helper (if any)", "sc.exe", &["stop", SERVICE_NAME]).tolerant(),
        SystemStep::run("Remove an earlier helper (if any)", "sc.exe", &["delete", SERVICE_NAME]).tolerant(),
        SystemStep::copy("Copy Kora to the helper's folder (only administrators can change it)", src_exe, &exe.to_string_lossy(), 0o755),
    ]);
    let rest = Plan::new(vec![
        SystemStep::write("Record which Windows account the helper serves", &config_path(&dir).to_string_lossy(), json, 0o644),
        SystemStep::run_owned("Only administrators may change the helper's folder (everyone may read it)", "icacls.exe", folder_acl_args(&dir)),
        SystemStep::run_owned("The helper's folder belongs to the Administrators group", "icacls.exe", vec![d.clone(), "/setowner".into(), "*S-1-5-32-544".into(), "/T".into(), "/C".into()]),
        SystemStep::run_owned(
            "Add the helper service (starts with Windows, runs as the system account)",
            "sc.exe",
            vec![
                "create".into(),
                SERVICE_NAME.into(),
                "binPath=".into(),
                bin_path(&exe)?,
                "start=".into(),
                "auto".into(),
                "obj=".into(),
                "LocalSystem".into(),
                "DisplayName=".into(),
                DISPLAY_NAME.into(),
            ],
        ),
        SystemStep::run_owned(
            "Describe the helper service",
            "sc.exe",
            vec![
                "description".into(),
                SERVICE_NAME.into(),
                format!("Does a few administrator jobs for Kora Panel for {}\\{} only. Turn it off in Kora's Settings or with 'kora helper uninstall'.", cfg.domain, cfg.user),
            ],
        ),
        SystemStep::run_owned(
            "Restart the helper if it stops unexpectedly",
            "sc.exe",
            vec!["failure".into(), SERVICE_NAME.into(), "reset=".into(), "3600".into(), "actions=".into(), "restart/2000/restart/5000/restart/30000".into()],
        ),
        SystemStep::run("Treat a non-zero exit as a crash so the restart rule applies", "sc.exe", &["failureflag", SERVICE_NAME, "1"]),
        SystemStep::run("Start the helper", "sc.exe", &["start", SERVICE_NAME]),
    ]);
    Ok((copy, rest))
}

/// Steps to turn the helper off and remove every trace (run elevated). The folder itself is
/// removed by the caller afterwards (with retries: the program file stays locked for a moment
/// after the service stops).
pub fn uninstall_plan(program_files: &Path) -> Plan {
    let dir = helper_dir(program_files);
    let f = |p: PathBuf| p.to_string_lossy().into_owned();
    Plan::new(vec![
        SystemStep::run("Stop the helper", "sc.exe", &["stop", SERVICE_NAME]).tolerant(),
        SystemStep::run("Remove the helper service", "sc.exe", &["delete", SERVICE_NAME]).tolerant(),
        SystemStep::remove("Remove the helper's settings", &f(config_path(&dir))),
        SystemStep::remove("Remove the helper's log", &f(log_path(&dir))),
    ])
}

/// Where a request goes when the helper is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// The helper does it, no prompt.
    Helper,
    /// It needs the Windows prompt (someone at the computer); the reason, in plain words.
    Prompt(String),
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
}

/// Which requests the helper does (see the table in the module documentation). The panel uses
/// this to choose; the service checks it again for every request.
pub fn route(verb: &str, args: &[String]) -> Route {
    let schedule = || Route::Prompt("Kora's helper does not set up automatic update schedules: they run Kora's update with administrator rights on a folder your account can change.".into());
    match verb {
        "admin-step" => match (args.first().map(String::as_str), flag(args, "--step")) {
            (Some("run"), Some("prepare.db" | "build.db" | "system.firewall" | "system.service")) => Route::Helper,
            (Some("undo"), Some("build.db" | "system.firewall" | "system.service")) => Route::Helper,
            (_, Some("system.schedule")) => schedule(),
            _ => Route::Prompt("Kora's helper does not do that step.".into()),
        },
        "admin-db" => Route::Helper,
        "check" if args.iter().any(|a| a == "--start-database") => Route::Helper,
        "helper" if args.first().is_some_and(|a| a == "refresh") => Route::Helper,
        "helper" => Route::Prompt("Turning Kora's helper on or off needs someone at this computer.".into()),
        "schedule" => schedule(),
        "self-update" | "app" | "uninstall" => Route::Prompt("Kora's helper does not update or remove Kora itself.".into()),
        _ => Route::Prompt("Kora's helper does not do that.".into()),
    }
}

/// Ports the helper opens in the firewall: not the privileged range, and not the usual
/// remote-administration and database ports (a harmful program could otherwise expose them).
pub fn port_allowed(port: u16) -> bool {
    const REFUSED: &[u16] = &[1433, 1434, 3306, 3389, 5432, 5900, 5985, 5986, 47001];
    port >= 1024 && !REFUSED.contains(&port)
}

/// A `session` line's Kora folder against the recorded one (Windows paths: case-insensitive, `/` = `\`).
pub fn session_home_ok(given: Option<&str>, recorded: &str) -> bool {
    match given {
        None => true,
        Some(g) => super::worker::same_exe(Path::new(g), Path::new(recorded)),
    }
}

/// May the helper update itself to `want` (it runs `own`)? Only to a newer release.
pub fn refresh_allowed(want: &str, own: &str) -> bool {
    crate::app_install::compare_versions(want, own) == Some(std::cmp::Ordering::Greater)
}

/// What the panel should do with a helper that answered `ping` with `version`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionState {
    /// Same version: use it.
    Current,
    /// Older: ask it to update itself; use the prompt meanwhile.
    Older,
    /// Newer than the panel (the panel was not restarted after an update, or went back): use the prompt.
    Newer,
}

pub fn version_state(helper: &str, panel: &str) -> VersionState {
    match crate::app_install::compare_versions(helper, panel) {
        Some(std::cmp::Ordering::Equal) => VersionState::Current,
        Some(std::cmp::Ordering::Less) => VersionState::Older,
        Some(std::cmp::Ordering::Greater) => VersionState::Newer,
        None if helper == panel => VersionState::Current,
        None => VersionState::Newer,
    }
}

// ---------------------------------------------------------------------------------------------
// Ownership ledger
// ---------------------------------------------------------------------------------------------

/// Which Windows account each system-wide thing was made for: `"service:kora-main"`,
/// `"firewall:main"`, `"task:main"`, `"db:kora_ab12"` → SID. Written by the elevated step children
/// (worker and helper) after they made something; read by the helper before it removes anything.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ledger {
    #[serde(default)]
    pub owners: BTreeMap<String, String>,
}

impl Ledger {
    pub fn load(path: &Path) -> Ledger {
        std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    /// Write via a temporary file and a rename (no half-written ledger).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?)?;
        std::fs::rename(&tmp, path)
    }

    pub fn owner(&self, key: &str) -> Option<&str> {
        self.owners.get(key).map(String::as_str)
    }

    pub fn record(&mut self, key: String, sid: &str) {
        self.owners.insert(key, sid.to_string());
    }

    pub fn forget(&mut self, key: &str) {
        self.owners.remove(key);
    }
}

/// The ledger keys of one step for the server named `slug` (`db` = the database name, `build.db`).
pub fn ledger_keys(step: &str, slug: &str, db: Option<&str>) -> Vec<String> {
    match step {
        "system.service" => vec![format!("service:kora-{slug}")],
        "system.firewall" => vec![format!("firewall:{slug}")],
        "system.schedule" => vec![format!("task:{slug}")],
        "build.db" => db.map(|d| vec![format!("db:{d}")]).unwrap_or_default(),
        _ => vec![],
    }
}

/// Helper mode: may `caller` change the things behind `keys`? Making something: no other account
/// may have it already. Removing: it must be recorded as the caller's.
pub fn ledger_allows(ledger: &Ledger, keys: &[String], caller: &str, undo: bool) -> Result<(), String> {
    for k in keys {
        match ledger.owner(k) {
            Some(o) if o.eq_ignore_ascii_case(caller) => {}
            Some(_) => return Err(format!("Kora's helper will not change {}: Kora made it for another Windows account on this computer.", describe_key(k))),
            None if undo => {
                return Err(format!(
                    "Kora's helper only removes what Kora made for your Windows account, and its records do not show {} as yours (it may have been made by an older Kora). Ask for administrator permission at the computer instead.",
                    describe_key(k)
                ))
            }
            None => {}
        }
    }
    Ok(())
}

fn describe_key(k: &str) -> String {
    match k.split_once(':') {
        Some(("service", n)) => format!("the Windows service {n}"),
        Some(("firewall", n)) => format!("the firewall rules of {n}"),
        Some(("task", n)) => format!("the update task of {n}"),
        Some(("db", n)) => format!("the database {n}"),
        _ => k.to_string(),
    }
}

/// The status Settings and `kora helper status` show.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Status {
    /// The service is registered.
    pub installed: bool,
    /// It answered `ping`.
    pub running: bool,
    pub version: Option<String>,
    /// `DOMAIN\user` it serves (from its settings file).
    pub user: Option<String>,
    /// It serves the account this panel runs as.
    pub for_me: bool,
}

#[cfg(test)]
mod tests;
