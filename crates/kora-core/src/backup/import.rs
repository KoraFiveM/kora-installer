//! "Bring a server from a backup or another machine": import a backup as a NEW server.
//!
//! The same archive as a backup ([`super::archive`]); made on the old computer with "Move this server" (a password
//! is required there, the file travels). On the new computer: [`inspect`] reads only the start of the file (password,
//! version), says what the server is and what would be created, finds clashes (the name, the ports, the folder) and
//! offers the fixes; [`run`] then builds the server in steps, each recorded in `<dir>/.kora/import-state.json`, so a
//! failure is carried on from where it stopped by running the same import again (`start_over` removes what was made
//! first):
//!
//! 1. unpack and check the file (every checksum; nothing is trusted before it proved good);
//! 2. the server folder, with the configuration converted for this computer (folders, line endings, ports);
//! 3. the FXServer build (the number the old server ran, downloaded again; the newest recommended one when that
//!    build is gone);
//! 4. a NEW database and login, the dump loaded into it, and the new login written to `database.json` and
//!    `secrets.cfg` (the old login is never reused: it belongs to the old computer's database);
//! 5. the licence key (from the file, or a saved key picked now);
//! 6. the resources: by reference are fetched at their recorded commits, the others were in the file (a resource
//!    that cannot be fetched is reported, the import still completes: it can be fetched later);
//! 7. registered, so it shows in the list.
//!
//! What does not carry over, and the preview says so: the operating-system service, firewall rules, the update
//! timer, the backup schedule, port forwarding, saved GitHub sign-in, and the FXServer build itself (re-downloaded).
//! The licence key moves with the server, but Cfx.re keys are tied to the account and server address: Kora cannot
//! make Cfx.re accept a key on a new computer, and says so.
//!
//! Threat model: the file may come from anyone. It is verified before anything is placed, unpacked into a staging
//! folder with names checked, nothing in it is executed, and paths it names (a txAdmin folder from another machine)
//! are rewritten, never followed.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::archive::{self, Wanted, DUMP, RESOURCES_DIR, SERVER_DIR};
use super::crypt::{OpenError, Passphrase};
use super::dbdump;
use super::restore::{archive_problem, write_into};
use super::types::*;
use super::{codes, create, BackupEvent, Tools};
use crate::installer::Source;
use crate::installer::registry::{now_unix, PortsRecord, Registry, ResourceRecord, ServerEntry, ServerFile};
use crate::problem::{Action, Problem};

// ----------------------------------------------------------------------------------------- shapes

/// Where the file is: uploaded through the panel, or a path on this computer (local sessions only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ImportSource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub upload: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub path: Option<String>,
}

/// A thing in the way, and what to do about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Clash {
    /// `name`, `game_port`, `txadmin_port`, `folder`.
    pub kind: String,
    pub message: String,
    /// What Kora suggests instead (a name, a port number, a folder), when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub suggestion: Option<String>,
}

/// What to do about a fork owner this computer's GitHub sign-in cannot use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ForkChoice {
    /// Keep the remotes as recorded (the old owner's copies). Fetching them may fail if they are private.
    #[default]
    Keep,
    /// Make my own copies here: fork the original repositories into the account signed in on this computer.
    OwnCopies,
    /// Use the originals: plain clones of the original repositories (the server becomes a clone server).
    Originals,
}

/// The fork owner of the server, and whether this computer's GitHub sign-in can use it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ForkNote {
    pub owner: String,
    /// The signed-in account here is that owner, or may create forks in it.
    pub usable: bool,
    pub signed_in_as: Option<String>,
    pub message: String,
}

/// What Kora asks GitHub on this computer (a fake in tests).
pub trait ForkTools {
    /// Can the signed-in account use `owner` (it is the account, or an organisation it may fork into)?
    /// Returns (usable, the signed-in login).
    fn usable(&self, owner: &str) -> (bool, Option<String>);
    /// Make a fork of the original repository `upstream` in `owner`; returns its address.
    fn fork(&self, upstream: &str, owner: &str) -> Result<String, String>;
}

pub struct SystemForks;

impl ForkTools for SystemForks {
    fn usable(&self, owner: &str) -> (bool, Option<String>) {
        use crate::api::install::github::{GithubEnv, SystemGithub};
        let g = SystemGithub;
        let me = g.auth();
        if !me.signed_in {
            return (false, None);
        }
        let login = me.user.clone();
        if login.as_deref().is_some_and(|l| l.eq_ignore_ascii_case(owner)) {
            return (true, login);
        }
        let ok = g.owners().map(|l| l.iter().any(|o| o.login.eq_ignore_ascii_case(owner) && o.can_fork)).unwrap_or(false);
        (ok, login)
    }
    fn fork(&self, upstream: &str, owner: &str) -> Result<String, String> {
        let slug = crate::installer::repo_slug(upstream);
        let name = slug.rsplit('/').next().unwrap_or(&slug).to_string();
        let r = crate::git::SystemRunner;
        use crate::api::install::github::GithubEnv as _;
        let is_me = crate::api::install::github::SystemGithub.auth().user.as_deref().is_some_and(|l| l.eq_ignore_ascii_case(owner));
        let mut args = vec!["repo", "fork", slug.as_str(), "--clone=false"];
        if !is_me {
            args.extend(["--org", owner]);
        }
        let o = crate::git::CommandRunner::run(&r, "gh", &args, None).map_err(|e| format!("gh could not run ({e})"))?;
        if o.ok() || o.stderr.contains("already exists") {
            Ok(format!("https://github.com/{owner}/{name}.git"))
        } else {
            Err(o.stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or("GitHub refused").trim().to_string())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ImportChoices {
    pub name: String,
    pub dir: String,
    pub game_port: u16,
    pub txadmin_port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ImportPreview {
    pub file_name: String,
    pub encrypted: bool,
    /// A password is needed (and was not given or was wrong): nothing else is known yet.
    pub needs_passphrase: bool,
    pub created_unix: u64,
    pub server: Option<ServerInfo>,
    pub database: Option<DatabaseBackup>,
    pub by_reference: u32,
    pub in_full: u32,
    /// What Kora suggests for this computer.
    pub suggested: Option<ImportChoices>,
    pub clashes: Vec<Clash>,
    /// What will be created, in words.
    pub will_create: Vec<String>,
    /// What does not come along, in words.
    pub wont_carry: Vec<String>,
    pub warnings: Vec<String>,
    /// The saved licence keys (names only) that could be used instead of the one in the file.
    pub saved_keys: Vec<String>,
    /// The licence-key warning, always shown.
    pub licence_note: String,
    /// A fork server whose owner cannot be used here: the guided choice (see [`ForkChoice`]).
    pub fork: Option<ForkNote>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ImportRequest {
    pub source: ImportSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub passphrase: Option<String>,
    pub choices: ImportChoices,
    /// Use a saved licence key instead of the one in the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub licence_key_name: Option<String>,
    /// What to do when the fork owner cannot be used on this computer.
    #[serde(default)]
    pub fork_choice: ForkChoice,
    /// Start the server when it is imported.
    #[serde(default)]
    pub start: bool,
    /// Remove what an earlier, failed import of this server made, and begin again.
    #[serde(default)]
    pub start_over: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ImportResult {
    pub name: String,
    pub dir: String,
    pub resources_fetched: Vec<String>,
    /// Things that did not work, one sentence each (the import still completed).
    pub problems: Vec<String>,
    pub notes: Vec<String>,
    pub started: bool,
    /// Steps to do next on THIS computer, in words (open the firewall, set up the service...).
    pub next_steps: Vec<String>,
    /// How to remove the server from the OLD computer, as instructions (Kora never reaches into another machine).
    pub old_machine_steps: Vec<String>,
}

pub const LICENCE_NOTE: &str = "Your licence key moves with the server, but it is yours, not Kora's: Cfx.re ties a key to your account and the address the server runs on. Kora cannot make Cfx.re accept it on a new computer. If the server refuses to start with a licence error, check the key at portal.cfx.re (Keymaster), change its server IP address there if it has one, or pick a different saved key.";

const WONT_CARRY: &[&str] = &[
    "The operating-system service that starts the server with the computer (set it up again on this computer).",
    "Firewall rules and port forwarding (open the ports again for the new computer; the Overview's \"Can players reach my server?\" helps).",
    "The automatic-update timer and the backup schedule.",
    "Your GitHub sign-in (sign in again on this computer if the resources need it).",
    "The FXServer program itself: it is downloaded again.",
];

// ------------------------------------------------------------------------------------------- places

/// Where imported files are kept while they wait: `<Kora home>/imports/<id>/<file>`.
pub fn imports_dir() -> PathBuf {
    crate::installer::registry::kora_home().join("imports")
}

/// Resolve the source to a file. A path may only be used by a local session (`allow_path`).
pub fn resolve_source(s: &ImportSource, allow_path: bool) -> Result<PathBuf, Problem> {
    if let Some(id) = s.upload.as_deref() {
        if !(16..=64).contains(&id.len()) || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Problem::new(codes::NOT_FOUND, "That upload is not known. Upload the file again."));
        }
        let dir = imports_dir().join(id);
        return std::fs::read_dir(&dir)
            .ok()
            .and_then(|mut d| d.find_map(|x| x.ok().map(|x| x.path()).filter(|p| p.is_file())))
            .ok_or_else(|| Problem::new(codes::NOT_FOUND, "That upload is gone. Upload the file again."));
    }
    if let Some(p) = s.path.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        if !allow_path {
            return Err(Problem::new(codes::NOT_FOUND, "A file on the computer that runs Kora can only be chosen from that computer's own screen. Upload the file instead."));
        }
        let path = PathBuf::from(p);
        if !path.is_file() {
            return Err(Problem::new(codes::NOT_FOUND, format!("There is no file at {p}. Check the path (a file ending in .tar.gz or .tar.gz.age).")));
        }
        return Ok(path);
    }
    Err(Problem::new(codes::NOT_FOUND, "Choose the backup file to bring in."))
}

fn pass(p: Option<&str>) -> Option<Passphrase> {
    p.map(|p| p.trim_end_matches(['\r', '\n'])).filter(|p| !p.is_empty()).map(Passphrase::new)
}

// ---------------------------------------------------------------------------------------- inspect

/// Free ports on this computer, not used by a registered server.
pub trait PortCheck {
    fn is_busy(&self, port: u16) -> bool;
}

pub struct SystemPorts;
impl PortCheck for SystemPorts {
    fn is_busy(&self, port: u16) -> bool {
        !matches!(crate::firewall::check_port(port, true), crate::firewall::PortStatus::Free)
    }
}

fn free_port(start: u16, taken: &[u16], ports: &dyn PortCheck) -> u16 {
    (start..start.saturating_add(200)).find(|p| *p >= 1024 && !taken.contains(p) && !ports.is_busy(*p)).unwrap_or(start)
}

/// A name nobody uses yet: `Name`, `Name 2`, ...
fn free_name(base: &str, used: &[String]) -> String {
    let taken = |n: &str| used.iter().any(|u| crate::installer::slug(u) == crate::installer::slug(n));
    if !taken(base) {
        return base.to_string();
    }
    (2..).map(|i| format!("{base} {i}")).find(|n| !taken(n)).unwrap_or_else(|| base.to_string())
}

/// Look at a file. Reads only its start. `servers`: the servers already registered here.
pub fn inspect(file: &Path, passphrase: Option<&str>, servers: &[ServerEntry], saved_keys: &[String], ports: &dyn PortCheck, forks: &dyn ForkTools, default_dir: &dyn Fn(&str) -> String) -> Result<ImportPreview, Problem> {
    let file_name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut p = ImportPreview {
        file_name,
        encrypted: false,
        needs_passphrase: false,
        created_unix: 0,
        server: None,
        database: None,
        by_reference: 0,
        in_full: 0,
        suggested: None,
        clashes: vec![],
        will_create: vec![],
        wont_carry: WONT_CARRY.iter().map(|s| s.to_string()).collect(),
        warnings: vec![],
        saved_keys: saved_keys.to_vec(),
        licence_note: LICENCE_NOTE.into(),
        fork: None,
    };
    let head = match archive::read_head(file, pass(passphrase).as_ref()) {
        Ok(m) => m,
        Err(archive::ArchiveError::Open(OpenError::NeedsPassphrase)) => {
            p.encrypted = true;
            p.needs_passphrase = true;
            return Ok(p);
        }
        Err(e) => return Err(archive_problem(e)),
    };
    p.encrypted = passphrase.is_some() || file.to_string_lossy().ends_with(".age");
    p.created_unix = head.created_unix;
    p.by_reference = head.by_reference() as u32;
    p.in_full = head.in_full() as u32;
    let used_names: Vec<String> = servers.iter().map(|s| s.name.clone()).collect();
    let taken_ports: Vec<u16> = servers.iter().flat_map(|s| [s.ports.game, s.ports.txadmin]).collect();
    let name = free_name(&head.server.name, &used_names);
    let game = if taken_ports.contains(&head.server.game_port) || ports.is_busy(head.server.game_port) { free_port(head.server.game_port, &taken_ports, ports) } else { head.server.game_port };
    let mut taken2 = taken_ports.clone();
    taken2.push(game);
    let tx = if taken2.contains(&head.server.txadmin_port) || ports.is_busy(head.server.txadmin_port) { free_port(head.server.txadmin_port, &taken2, ports) } else { head.server.txadmin_port };
    let dir = default_dir(&name);
    if name != head.server.name {
        p.clashes.push(Clash { kind: "name".into(), message: format!("A server called \"{}\" is already here.", head.server.name), suggestion: Some(name.clone()) });
    }
    if game != head.server.game_port {
        p.clashes.push(Clash { kind: "game_port".into(), message: format!("The game port {} is already used on this computer.", head.server.game_port), suggestion: Some(game.to_string()) });
    }
    if tx != head.server.txadmin_port {
        p.clashes.push(Clash { kind: "txadmin_port".into(), message: format!("The txAdmin port {} is already used on this computer.", head.server.txadmin_port), suggestion: Some(tx.to_string()) });
    }
    if !dir.is_empty() && Path::new(&dir).exists() && std::fs::read_dir(&dir).map(|mut d| d.next().is_some()).unwrap_or(true) {
        p.clashes.push(Clash { kind: "folder".into(), message: format!("The folder {dir} already exists and is not empty."), suggestion: None });
    }
    p.will_create = vec![
        format!("A new server \"{name}\" in {}", if dir.is_empty() { "a folder you choose".to_string() } else { dir.clone() }),
        format!(
            "Its settings and txAdmin data, with the folders and line endings converted for this computer{}",
            if head.server.os != if cfg!(windows) { "windows" } else { "linux" } {
                format!(" (the backup comes from {})", if head.server.os == "windows" { "Windows" } else { "Linux" })
            } else {
                String::new()
            }
        ),
        match head.database.kind {
            DbKind::Dumped | DbKind::External => format!("A NEW database and login on this computer's database server, filled from the backup ({})", crate::space::human(head.database.dump_bytes)),
            _ => "No database (the old server had none that Kora knew of)".into(),
        },
        format!("{} resources fetched again from GitHub at the saved versions, {} taken from the file", head.by_reference(), head.in_full()),
        format!("FXServer build {} (downloaded again)", head.server.artifact_build),
    ];
    if head.database.kind == DbKind::External {
        p.warnings.push(format!(
            "The old server's database was on another computer ({}). Its data was copied into the file; here it goes into a new database on this computer.",
            head.database.host.clone().unwrap_or_default()
        ));
    }
    if head.database.kind == DbKind::None && !head.database.note.is_empty() {
        p.warnings.push(head.database.note.clone());
    }
    if head.server.os != if cfg!(windows) { "windows" } else { "linux" } {
        p.warnings.push("This server was made on a different operating system. Folders in its settings are rewritten, but a resource that runs a program only one system has (a Linux shell script, a Windows .exe) will not work here.".into());
    }
    if !head.stopped {
        p.warnings.push("The backup was made while the server was running, so it is consistent for the database but a few files may be from slightly different moments.".into());
    }
    if head.server.source == Source::Fork {
        if let Some(owner) = head.server.github_owner.clone() {
            let (usable, me) = forks.usable(&owner);
            let message = if usable {
                format!("This server follows your own copies (forks) in the GitHub account {owner}. You are signed in to GitHub here with access to it, so they can be used as they are.")
            } else {
                match &me {
                    Some(m) => format!("This server follows copies (forks) in the GitHub account {owner}, which the account signed in here ({m}) cannot use. Choose what to do about them below."),
                    None => format!("This server follows copies (forks) in the GitHub account {owner}, and you are not signed in to GitHub on this computer. Sign in, or choose what to do about them below."),
                }
            };
            p.fork = Some(ForkNote { owner, usable, signed_in_as: me, message });
        }
    }
    p.server = Some(head.server);
    p.database = Some(head.database);
    p.suggested = Some(ImportChoices { name, dir, game_port: game, txadmin_port: tx });
    Ok(p)
}

// ------------------------------------------------------------------------------ conversions

/// Convert a settings file made on another computer: the old server folder becomes the new one wherever it
/// appears (written with `/`, `\`, or `\\` inside JSON), the rest of such a path gets `/` separators (accepted on
/// both systems), and the line endings are the target's. Pure.
pub fn convert_text(text: &str, old_dir: &str, new_dir: &str, to_windows: bool) -> String {
    let mut out = text.to_string();
    let old = old_dir.trim_end_matches(['/', '\\']);
    if !old.is_empty() {
        let fwd = old.replace('\\', "/");
        let forms = [regex::escape(&fwd), regex::escape(&fwd.replace('/', "\\")), regex::escape(&fwd.replace('/', "\\\\"))];
        let pattern = format!(r#"(?i)(?:{})((?:\\\\|\\|/)[^\s"',;:*?<>|]*)?"#, forms.join("|"));
        if let Ok(rx) = regex::Regex::new(&pattern) {
            let new_fwd = new_dir.trim_end_matches(['/', '\\']).replace('\\', "/");
            out = rx
                .replace_all(&out, |c: &regex::Captures| {
                    let rest = c.get(1).map_or(String::new(), |m| m.as_str().replace("\\\\", "/").replace('\\', "/"));
                    format!("{new_fwd}{rest}")
                })
                .into_owned();
        }
    }
    let unix = out.replace("\r\n", "\n");
    if to_windows {
        unix.replace('\n', "\r\n")
    } else {
        unix
    }
}

fn is_converted_file(rel: &str) -> bool {
    let l = rel.to_ascii_lowercase();
    l.ends_with(".cfg") || (l.starts_with("txdata/") && (l.ends_with(".json") || l.ends_with(".txt")))
}

fn secrets_cfg(key: &str, creds: Option<&crate::db::Credentials>) -> String {
    let mut s = String::from("# Kora secrets. Restricted permissions, git-ignored, never share.\n");
    s.push_str(&format!("sv_licenseKey \"{key}\"\n"));
    match creds {
        Some(c) => s.push_str(&c.server_cfg_line()),
        None => s.push_str("# No database was set up: add your connection string here:\n# set mysql_connection_string \"mysql://user:password@host/database?charset=utf8mb4\"\n"),
    }
    s
}

// ------------------------------------------------------------------------------------------- run

/// Downloads the FXServer build.
pub trait Fetcher {
    /// Make `build` (or, if it is gone, the recommended one) the current build under `<dir>/artifacts`. Returns the build.
    fn artifact(&self, build: u32, dir: &Path, say: &mut dyn FnMut(&str)) -> Result<u32, String>;
}

pub struct SystemFetcher;

impl Fetcher for SystemFetcher {
    fn artifact(&self, build: u32, dir: &Path, say: &mut dyn FnMut(&str)) -> Result<u32, String> {
        use crate::artifacts::{self, Channel, Platform, Verify};
        let http = crate::installer::HttpClient::default();
        let platform = Platform::current();
        let list = artifacts::list_artifacts(&http, platform).map_err(|e| format!("could not look up server builds: {e}"))?;
        let chosen = list.find(build).or_else(|| list.pick(Channel::Recommended)).ok_or("no server build is available")?;
        let (url, got) = (chosen.url.clone(), chosen.build);
        let store = artifacts::ArtifactStore::new(dir.join("artifacts"));
        let dl = dir.join("artifacts").join(".download");
        std::fs::create_dir_all(&dl).map_err(|e| e.to_string())?;
        let file = dl.join(url.rsplit('/').next().unwrap_or("artifact"));
        let _ = std::fs::remove_file(&file);
        say(&format!("Downloading FXServer build {got} (this can take a few minutes)"));
        artifacts::download_verified(&http, &url, platform, &file, &Verify::default()).map_err(|e| e.to_string())?;
        say(&format!("Unpacking build {got}"));
        match store.swap(got, &file, platform) {
            Ok(()) => {}
            Err(artifacts::ArtifactError::Store(m)) if m.contains("already current") => {}
            Err(e) => return Err(e.to_string()),
        }
        let _ = std::fs::remove_dir_all(&dl);
        Ok(got)
    }
}

/// Creates the new database and login (the admin login is the one install uses).
pub trait Provisioner {
    fn provision(&self, state_path: &Path) -> Result<crate::db::Credentials, String>;
    /// Drop what `provision` made (rollback).
    fn drop_made(&self, state_path: &Path);
}

pub struct SystemProvisioner {
    pub admin: crate::db::AdminConn,
}

impl Provisioner for SystemProvisioner {
    fn provision(&self, state_path: &Path) -> Result<crate::db::Credentials, String> {
        crate::db::provision(&crate::db::SystemRunner, &self.admin, "localhost", "127.0.0.1", state_path).map_err(|e| e.to_string())
    }
    fn drop_made(&self, state_path: &Path) {
        if let Ok(mut st) = crate::db::ProvisionState::load(state_path) {
            let _ = crate::db::rollback(&crate::db::SystemRunner, &self.admin, &mut st);
        }
    }
}

pub struct Env<'a> {
    pub tools: &'a Tools<'a>,
    pub fetcher: &'a dyn Fetcher,
    pub provisioner: Option<&'a dyn Provisioner>,
    /// The key of a saved name.
    pub saved_key: &'a dyn Fn(&str) -> Option<String>,
    pub ports: &'a dyn PortCheck,
    pub forks: &'a dyn ForkTools,
    pub servers: Vec<ServerEntry>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    source_sha: String,
    done: Vec<String>,
    db_name: Option<String>,
}

fn state_path(dir: &Path) -> PathBuf {
    crate::installer::meta_dir(dir).join("import-state.json")
}

fn load_state(dir: &Path) -> State {
    std::fs::read(state_path(dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_state(dir: &Path, s: &State) -> Result<(), Problem> {
    let _ = std::fs::create_dir_all(crate::installer::meta_dir(dir));
    std::fs::write(state_path(dir), serde_json::to_vec_pretty(s).map_err(|e| Problem::generic(e.to_string()))?)
        .map_err(|e| Problem::generic(format!("Kora could not write its progress note in the new folder: {e}")))
}

fn io_problem(what: &str, e: std::io::Error) -> Problem {
    if e.kind() == std::io::ErrorKind::StorageFull {
        return create::disk_full(&format!("There is no space left on the disk while {what}."));
    }
    Problem::generic(format!("Kora could not finish {what}: {e}")).with_action(Action::Retry)
}

/// Run the import. Resumable: a second call with the same file and folder carries on after the last finished step.
pub fn run(req: &ImportRequest, file: &Path, env: &Env<'_>, say: &mut dyn FnMut(BackupEvent)) -> Result<ImportResult, Problem> {
    let mut ev = |step: &str, msg: &str, pct: u8| say(BackupEvent { step: step.into(), message: msg.into(), percent: pct });
    let c = &req.choices;
    crate::api::install::validate_name(&c.name).map_err(Problem::generic)?;
    let dir = PathBuf::from(&c.dir);
    if !dir.is_absolute() {
        return Err(Problem::new(codes::DEST_UNWRITABLE, "Give the full path of the folder for the new server."));
    }
    let pw = pass(req.passphrase.as_deref());
    ev("open", "Opening the file", 2);
    let head = archive::read_head(file, pw.as_ref()).map_err(archive_problem)?;
    // clashes are re-checked now: the world may have changed since the preview
    if env.servers.iter().any(|s| crate::installer::slug(&s.name) == crate::installer::slug(&c.name)) {
        return Err(Problem::new(codes::BUSY, format!("A server called \"{}\" is already here. Pick another name.", c.name)));
    }
    for port in [c.game_port, c.txadmin_port] {
        if port < 1024 || env.servers.iter().any(|s| s.ports.game == port || s.ports.txadmin == port) {
            return Err(Problem::new(codes::BUSY, format!("The port {port} is not available. Pick another.")));
        }
    }
    if c.game_port == c.txadmin_port {
        return Err(Problem::new(codes::BUSY, "The game port and the txAdmin port must be different."));
    }
    let _guard = create::BusyGuard::take(&c.name)?;
    let sha = archive::file_sha256(file).map_err(|e| io_problem("reading the file", e))?;
    let mut st = load_state(&dir);
    let resuming = dir.join(".kora").join("import-state.json").is_file() && st.source_sha == sha && !req.start_over;
    if req.start_over || (dir.exists() && !resuming) {
        if dir.exists() && std::fs::read_dir(&dir).map(|mut d| d.next().is_some()).unwrap_or(false) {
            if !req.start_over {
                return Err(Problem::new(codes::BUSY, format!("The folder {} already exists and is not empty. Choose another folder.", dir.display())));
            }
            rollback(&dir, env);
        }
        st = State { source_sha: sha.clone(), ..State::default() };
    } else if !resuming {
        st = State { source_sha: sha.clone(), ..State::default() };
    }
    let created_dir = !dir.exists();
    std::fs::create_dir_all(&dir).map_err(|e| io_problem("creating the server folder", e))?;
    let stage = dir.join(".kora").join("import-staging");
    let mut result = ImportResult {
        name: c.name.clone(),
        dir: dir.display().to_string(),
        resources_fetched: vec![],
        problems: vec![],
        notes: vec![],
        started: false,
        next_steps: vec![],
        old_machine_steps: vec![],
    };
    let mut manifest: Option<Manifest> = None;
    // What to do about a fork owner this computer cannot use (decided once, the same on every resume).
    let mut resources_plan = head.resources.clone();
    let mut new_source = head.server.source;
    let mut new_owner = head.server.github_owner.clone();
    let mut repointed: Vec<String> = Vec::new();
    if head.server.source == Source::Fork {
        if let Some(old) = head.server.github_owner.clone() {
            let (usable, me) = env.forks.usable(&old);
            if !usable {
                match req.fork_choice {
                    ForkChoice::Keep => {}
                    ForkChoice::Originals => {
                        for rb in resources_plan.iter_mut().filter(|r| !r.upstream.is_empty()) {
                            rb.repository = rb.upstream.clone();
                            repointed.push(rb.id.clone());
                        }
                        new_source = Source::Clone;
                        new_owner = None;
                        result.notes.push("The resources follow the original repositories now (this server is a clone server here). Your old copies were not touched.".into());
                    }
                    ForkChoice::OwnCopies => {
                        let Some(owner) = me else {
                            return Err(Problem::new(codes::NOT_FOUND, "To make your own copies, sign in to GitHub on this computer first (the System check page helps), or choose another option.").with_action(Action::SignInGithub { scopes: vec![] }));
                        };
                        for rb in resources_plan.iter_mut().filter(|r| !r.upstream.is_empty()) {
                            match env.forks.fork(&rb.upstream, &owner) {
                                Ok(url) => {
                                    rb.repository = url;
                                    repointed.push(rb.id.clone());
                                }
                                Err(why) => result.problems.push(format!("{}: your own copy could not be made ({why}); it follows the old owner's copy instead.", rb.id)),
                            }
                        }
                        new_owner = Some(owner);
                    }
                }
            }
        }
    }

    let run = (|| -> Result<(), Problem> {
        // 1. unpack and prove
        if !st.done.iter().any(|d| d == "unpack") || !stage.is_dir() {
            ev("unpack", "Unpacking and checking every file (nothing is placed yet)", 6);
            let _ = std::fs::remove_dir_all(&stage);
            std::fs::create_dir_all(&stage).map_err(|e| io_problem("unpacking", e))?;
            if let Some(free) = crate::space::SystemDisk.free_bytes_of(&dir) {
                if free < head.payload_bytes.saturating_add(512 * crate::space::MIB) {
                    return Err(create::disk_full(&format!("Importing needs about {} of room and only {} is free here.", crate::space::human(head.payload_bytes), crate::space::human(free))));
                }
            }
            let m = archive::scan(file, pw.as_ref(), Some((&stage, &Wanted { take: &|_| true })), head.payload_bytes.saturating_mul(2).saturating_add(256 * crate::space::MIB), None)
                .map_err(archive_problem)?;
            manifest = Some(m);
            st.done.push("unpack".into());
            save_state(&dir, &st)?;
        }
        let m = match &manifest {
            Some(m) => m.clone(),
            None => archive::read_head(file, pw.as_ref()).map_err(archive_problem)?,
        };
        let to_windows = cfg!(windows);

        // 2. the folder and the converted configuration
        if !st.done.iter().any(|d| d == "files") {
            ev("files", "Placing the settings and txAdmin data", 20);
            let server_stage = stage.join(SERVER_DIR);
            for rel in files_below(&server_stage) {
                let top = rel.split('/').next().unwrap_or("");
                if matches!(top, "artifacts" | "resources" | ".kora" | ".git") || matches!(rel.as_str(), "secrets.cfg" | "database.json" | "kora-server.json") {
                    continue;
                }
                let Some(dest) = crate::fsutil::join_inside(&dir, &rel) else { continue };
                let src = server_stage.join(&rel);
                if is_converted_file(&rel) {
                    let text = std::fs::read_to_string(&src).map_err(|e| io_problem("reading a settings file", e))?;
                    let mut converted = convert_text(&text, &m.server.dir, &c.dir, to_windows);
                    if rel == "server.cfg" && c.game_port != m.server.game_port {
                        converted = crate::installer::change_ports::rewrite_cfg_game_port(&converted, c.game_port).unwrap_or(converted);
                    }
                    if let Some(parent) = dest.parent() {
                        std::fs::create_dir_all(parent).map_err(|e| io_problem("placing a settings file", e))?;
                    }
                    std::fs::write(&dest, converted).map_err(|e| io_problem("placing a settings file", e))?;
                } else {
                    write_into(&dest, &src, false).map_err(|e| io_problem("placing a file", e))?;
                }
            }
            st.done.push("files".into());
            save_state(&dir, &st)?;
        }

        // 3. the FXServer build
        if !st.done.iter().any(|d| d == "artifact") {
            ev("artifact", "Getting the server program", 35);
            let mut said = |s: &str| ev("artifact", s, 40);
            let got = env
                .fetcher
                .artifact(m.server.artifact_build, &dir, &mut said)
                .map_err(|e| Problem::new(codes::NOT_FOUND, format!("The server program could not be downloaded: {e}")).with_action(Action::Retry))?;
            if got != m.server.artifact_build && m.server.artifact_build != 0 {
                result.notes.push(format!("FXServer build {} is no longer available; build {got} was used instead.", m.server.artifact_build));
            }
            st.done.push("artifact".into());
            save_state(&dir, &st)?;
        }

        // 4. a new database, the dump loaded, the new login written down
        let mut creds: Option<crate::db::Credentials> = None;
        let dump = stage.join(DUMP);
        if matches!(m.database.kind, DbKind::Dumped | DbKind::External) && dump.is_file() {
            let prov = env.provisioner.ok_or_else(|| {
                Problem::new(codes::DB_STOPPED, "A database server is needed to bring this server's database in. Install or start one (the System check page helps), then try again.")
                    .with_action(Action::StartDatabase)
                    .with_action(Action::Retry)
            })?;
            let ppath = crate::installer::meta_dir(&dir).join("db-provision.json");
            if !st.done.iter().any(|d| d == "database") {
                ev("database", "Creating a new database and login", 50);
                let made = prov.provision(&ppath).map_err(|e| {
                    Problem::new(codes::DB_STOPPED, "Kora could not create the new database. Is the database server running? Kora uses the same database administrator login as an install: on Linux the root account's own login (run Kora as root for the import) or the KORA_DB_ADMIN_USER and KORA_DB_ADMIN_PASSWORD settings; on Windows the login of the MariaDB Kora set up.")
                        .with_details(e)
                        .with_action(Action::StartDatabase)
                        .with_action(Action::Retry)
                })?;
                st.db_name = Some(made.database.clone());
                save_state(&dir, &st)?;
                ev("database", "Loading the database", 58);
                dbdump::restore(env.tools.db, &env.tools.db_client, &made, &dump).map_err(|f| create::problem_for_db(f, "load the database"))?;
                creds = Some(made);
                st.done.push("database".into());
                save_state(&dir, &st)?;
            } else if let Ok(b) = std::fs::read(dir.join("database.json")) {
                creds = serde_json::from_slice(&b).ok();
            }
        }

        // 5. secrets: the licence key and the new database login
        if !st.done.iter().any(|d| d == "secrets") {
            ev("secrets", "Writing the licence key and the database login", 65);
            let from_file = std::fs::read_to_string(stage.join(SERVER_DIR).join("secrets.cfg")).ok().and_then(|t| t.lines().find_map(crate::keystore::parse_key_line));
            let key = match req.licence_key_name.as_deref() {
                Some(n) => (env.saved_key)(n).ok_or_else(|| Problem::new(codes::NOT_FOUND, format!("There is no saved licence key called \"{n}\".")))?,
                None => from_file.ok_or_else(|| Problem::new(codes::NOT_FOUND, "The backup has no licence key and none was chosen. Pick a saved key (or add one in Settings) and try again."))?,
            };
            let runner = crate::db::SystemRunner;
            let os = crate::db::Os::current();
            let user = std::env::var("USERNAME").ok().filter(|u| !u.is_empty());
            if let Some(cr) = &creds {
                crate::db::write_secret_file(&runner, os, &dir.join("database.json"), &serde_json::to_string_pretty(cr).map_err(|e| Problem::generic(e.to_string()))?, user.as_deref())
                    .map_err(|e| Problem::generic(format!("Kora could not write database.json: {e}")))?;
            }
            crate::db::write_secret_file(&runner, os, &dir.join("secrets.cfg"), &secrets_cfg(&key, creds.as_ref()), user.as_deref())
                .map_err(|e| Problem::generic(format!("Kora could not write secrets.cfg: {e}")))?;
            st.done.push("secrets".into());
            save_state(&dir, &st)?;
        }

        // 6. resources
        if !st.done.iter().any(|d| d == "resources") {
            ev("resources", "Placing the resources that were in the file", 70);
            let res_dir = dir.join(RESOURCES_DIR);
            std::fs::create_dir_all(&res_dir).map_err(|e| io_problem("creating the resources folder", e))?;
            for rel in files_below(&stage.join(RESOURCES_DIR)) {
                if let Some(dest) = crate::fsutil::join_inside(&res_dir, &rel) {
                    write_into(&dest, &stage.join(RESOURCES_DIR).join(&rel), false).map_err(|e| io_problem("placing a resource", e))?;
                }
            }
            // resources kept in full: their git history comes along; point `origin` where the choice above says
            for rb in resources_plan.iter().filter(|r| r.stored == StoredAs::Full && repointed.contains(&r.id)) {
                let folder = res_dir.join(&rb.path);
                let _ = env.tools.git.run("git", &["remote", "set-url", "origin", &rb.repository], Some(&folder));
            }
            let refs: Vec<&ResourceBackup> = resources_plan.iter().filter(|r| r.stored == StoredAs::Reference).collect();
            for (i, rb) in refs.iter().enumerate() {
                ev("resources", &format!("Fetching {} ({}/{})", rb.id, i + 1, refs.len()), (72 + i * 18 / refs.len().max(1)) as u8);
                match super::resources::ensure_at_commit(env.tools.git, &res_dir, rb) {
                    Ok(msg) => result.resources_fetched.push(msg),
                    Err(msg) => result.problems.push(format!("{msg}. Fetch it later from the Resources tab; the server is otherwise complete.")),
                }
            }
            // every resource counts as changed ("Restart needed"), and a pull request being tried keeps its title
            super::resources::finish_resource_state(&dir, &resources_plan);
            st.done.push("resources".into());
            save_state(&dir, &st)?;
        }

        // 7. record and register
        ev("register", "Registering the server", 92);
        let mut sf = std::fs::read(stage.join(SERVER_DIR).join("kora-server.json")).ok().and_then(|b| serde_json::from_slice::<ServerFile>(&b).ok()).unwrap_or_else(|| ServerFile {
            name: c.name.clone(),
            mode: if m.server.mode == "development" { crate::installer::Mode::Development } else { crate::installer::Mode::Production },
            source: Some(m.server.source),
            versions: Some(m.server.versions),
            dir: dir.clone(),
            ports: PortsRecord { game: c.game_port, txadmin: c.txadmin_port },
            game_build: m.server.game_build,
            artifact_build: m.server.artifact_build,
            database: None,
            github_owner: new_owner.clone(),
            created_unix: now_unix(),
            kora_version: env!("CARGO_PKG_VERSION").into(),
            resources: Vec::<ResourceRecord>::new(),
        });
        sf.name = c.name.clone();
        sf.source = Some(new_source);
        sf.versions = Some(m.server.versions);
        sf.github_owner = new_owner.clone();
        sf.mode = crate::installer::compat_mode(m.server.versions);
        sf.dir = dir.clone();
        sf.ports = PortsRecord { game: c.game_port, txadmin: c.txadmin_port };
        sf.database = creds.as_ref().map(|c| c.database.clone());
        sf.created_unix = now_unix();
        sf.kora_version = env!("CARGO_PKG_VERSION").into();
        sf.save().map_err(|e| io_problem("saving the server's record", e))?;
        let created = crate::installer::Created {
            created_dir,
            files: vec!["server.cfg".into(), "permissions.cfg".into(), "secrets.cfg".into(), "database.json".into(), "txData".into()],
            artifacts: true,
            registered: true,
            ..Default::default()
        };
        let _ = crate::installer::save_created_to(&dir, &created);
        let entry = ServerEntry { name: c.name.clone(), dir: dir.clone(), mode: sf.mode, source: Some(new_source), versions: Some(m.server.versions), ports: PortsRecord { game: c.game_port, txadmin: c.txadmin_port }, created_unix: now_unix() };
        Registry::update(|r| {
            r.upsert(entry.clone());
            Ok(())
        })
        .map_err(|e| io_problem("registering the server", e))?;
        st.done.push("register".into());
        save_state(&dir, &st)?;
        Ok(())
    })();

    let _ = std::fs::remove_dir_all(&stage);
    if let Err(p) = run {
        let msg = format!("{} Nothing is registered yet. Run the import again with the same file to carry on from where it stopped, or choose \"Start over\" to remove what was made.", p.message);
        return Err(Problem::new(p.code.as_str(), msg).with_details(p.details.unwrap_or_default()).with_actions(p.actions));
    }
    result.next_steps = vec![
        "Open the server's ports on this computer (the Overview's \"Can players reach my server?\" shows how).".into(),
        "If you want it to start with the computer, set up the background service (Settings, or `kora install` options).".into(),
        "Check your licence key at portal.cfx.re if the server says it is not valid here.".into(),
    ];
    result.old_machine_steps = vec![
        format!("When you are sure \"{}\" works here, stop it on the OLD computer (open Kora there, Servers, Stop).", head.server.name),
        "Then remove it there: open the server's Danger zone tab and choose Remove (it also drops its database). Kora never does this for you from here.".into(),
        "Keep the moving file until you are sure everything is right: it holds your licence key and logins, so delete it afterwards.".into(),
    ];
    ev("done", "The server is imported", 100);
    Ok(result)
}

/// Remove what a failed import made: the database and login it created, and the folder (only when it holds an
/// import state, i.e. Kora made it).
pub fn rollback(dir: &Path, env: &Env<'_>) {
    if let (Some(p), true) = (env.provisioner, dir.join(".kora").join("db-provision.json").is_file()) {
        p.drop_made(&dir.join(".kora").join("db-provision.json"));
    }
    if dir.join(".kora").join("import-state.json").is_file() || std::fs::read_dir(dir).map(|mut d| d.next().is_none()).unwrap_or(false) {
        let _ = std::fs::remove_dir_all(dir);
    }
}

fn files_below(root: &Path) -> Vec<String> {
    fn walk(dir: &Path, rel: &str, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let Some(n) = e.file_name().to_str().map(str::to_string) else { continue };
            let child = if rel.is_empty() { n } else { format!("{rel}/{n}") };
            match e.file_type() {
                Ok(t) if t.is_dir() => walk(&e.path(), &child, out),
                Ok(t) if t.is_file() => out.push(child),
                _ => {}
            }
        }
    }
    let mut v = Vec::new();
    walk(root, "", &mut v);
    v.sort();
    v
}

// ------------------------------------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::tests_support::*;
    use std::cell::RefCell;

    const SAMEFORKS: SameOwner = SameOwner;
    struct SameOwner;
    impl ForkTools for SameOwner {
        fn usable(&self, _: &str) -> (bool, Option<String>) {
            (true, Some("old-owner".into()))
        }
        fn fork(&self, _: &str, _: &str) -> Result<String, String> {
            Err("not needed".into())
        }
    }

    struct NoPorts;
    impl PortCheck for NoPorts {
        fn is_busy(&self, _: u16) -> bool {
            false
        }
    }

    /// A GitHub where the signed-in account is `me` and only `me` may be used; forks are recorded.
    struct FakeForks {
        me: Option<&'static str>,
        made: RefCell<Vec<String>>,
    }
    impl ForkTools for FakeForks {
        fn usable(&self, owner: &str) -> (bool, Option<String>) {
            (self.me == Some(owner), self.me.map(str::to_string))
        }
        fn fork(&self, upstream: &str, owner: &str) -> Result<String, String> {
            self.made.borrow_mut().push(upstream.to_string());
            Ok(format!("https://github.com/{owner}/{}", upstream.rsplit('/').next().unwrap()))
        }
    }

    struct FakeFetcher {
        calls: RefCell<Vec<u32>>,
    }
    impl Fetcher for FakeFetcher {
        fn artifact(&self, build: u32, dir: &Path, _say: &mut dyn FnMut(&str)) -> Result<u32, String> {
            self.calls.borrow_mut().push(build);
            std::fs::create_dir_all(dir.join("artifacts/current")).map_err(|e| e.to_string())?;
            Ok(build)
        }
    }

    struct FakeProvisioner {
        fail: bool,
        dropped: RefCell<bool>,
    }
    impl Provisioner for FakeProvisioner {
        fn provision(&self, state_path: &Path) -> Result<crate::db::Credentials, String> {
            if self.fail {
                return Err("connection refused".into());
            }
            std::fs::write(state_path, "{}").map_err(|e| e.to_string())?;
            Ok(crate::db::Credentials { host: "127.0.0.1".into(), port: 3306, database: "kora_new1".into(), user: "kora_new1".into(), password: crate::db::Secret::new("n3w-pass-123") })
        }
        fn drop_made(&self, _: &Path) {
            *self.dropped.borrow_mut() = true;
        }
    }

    fn make_backup(rig: &Rig, git: &crate::backup::resources::fake::FakeGit, pass: &str) -> PathBuf {
        rig.db.tables.borrow_mut().push(("players".into(), vec!["alice".into(), "bob".into()]));
        let o = create::CreateOptions { passphrase: Some(Passphrase::new(pass)), label: Label::Move, ..Default::default() };
        create::create(&rig.entry, &o, &mut FakeHost::new(false), &rig.tools(git), &mut |_| {}).unwrap().path
    }

    fn request(file: &Path, pass: &str, dir: &Path, name: &str) -> ImportRequest {
        ImportRequest {
            source: ImportSource { path: Some(file.display().to_string()), upload: None },
            passphrase: Some(pass.into()),
            choices: ImportChoices { name: name.into(), dir: dir.display().to_string(), game_port: 31120, txadmin_port: 41120 },
            licence_key_name: None,
            fork_choice: ForkChoice::Keep,
            start: false,
            start_over: false,
        }
    }

    #[test]
    fn inspect_says_what_is_in_it_finds_clashes_and_offers_fixes() {
        let rig = Rig::new();
        let git = rig.clean_git();
        let path = make_backup(&rig, &git, "a good passphrase");
        // no password: says so and nothing else
        let p = inspect(&path, None, &[], &[], &NoPorts, &FakeForks { me: Some("old-owner"), made: Default::default() }, &|_| "/srv/new".into()).unwrap();
        assert!(p.needs_passphrase && p.server.is_none());
        // wrong password
        assert_eq!(inspect(&path, Some("wrong wrong wrong"), &[], &[], &NoPorts, &FakeForks { me: Some("old-owner"), made: Default::default() }, &|_| "/srv/new".into()).unwrap_err().code, codes::WRONG_PASSPHRASE);
        // right: this machine already has a server with that name and those ports
        let here =
            vec![ServerEntry { name: rig.entry.name.clone(), dir: "/srv/other".into(), mode: crate::installer::Mode::Production, source: None, versions: None, ports: PortsRecord { game: 30120, txadmin: 40120 }, created_unix: 1 }];
        let p = inspect(&path, Some("a good passphrase"), &here, &["Main".into()], &NoPorts, &FakeForks { me: Some("old-owner"), made: Default::default() }, &|n| format!("/srv/{}", crate::installer::slug(n))).unwrap();
        let kinds: Vec<&str> = p.clashes.iter().map(|c| c.kind.as_str()).collect();
        assert_eq!(kinds, ["name", "game_port", "txadmin_port"]);
        let s = p.suggested.unwrap();
        assert!(s.name.ends_with(" 2") && s.game_port != 30120 && s.txadmin_port != 40120);
        assert_eq!((p.by_reference, p.in_full), (1, 1));
        assert!(p.licence_note.contains("Cfx.re") && p.licence_note.contains("cannot make Cfx.re accept"));
        assert!(p.wont_carry.iter().any(|w| w.contains("service")) && p.wont_carry.iter().any(|w| w.contains("Firewall")));
        assert_eq!(p.saved_keys, vec!["Main".to_string()]);
        // a busy folder is a clash too
        let busy = tempfile::tempdir().unwrap();
        std::fs::write(busy.path().join("x"), b"1").unwrap();
        let p = inspect(&path, Some("a good passphrase"), &[], &[], &NoPorts, &FakeForks { me: Some("old-owner"), made: Default::default() }, &|_| busy.path().display().to_string()).unwrap();
        assert!(p.clashes.iter().any(|c| c.kind == "folder"));
    }

    #[test]
    fn a_server_is_moved_database_files_resources_secrets_and_registration() {
        let rig = Rig::new();
        let mut git = rig.clean_git();
        git.clone_makes_git_dir = true;
        let path = make_backup(&rig, &git, "a good passphrase");
        // the new computer: another folder, a fresh (empty) database
        let new_db = FakeDbOnly::default();
        let tools = Tools { db: &new_db.0, git: &git, db_client: "mariadb".into(), dump_program: None, now: 1_700_000_500 };
        git.say("cat-file -e abcdef1234567890abcdef1234567890abcdef12^{commit}", "");
        git.say("checkout -f -B main abcdef1234567890abcdef1234567890abcdef12", "");
        let fetch = FakeFetcher { calls: RefCell::new(vec![]) };
        let prov = FakeProvisioner { fail: false, dropped: RefCell::new(false) };
        let env = Env { tools: &tools, fetcher: &fetch, provisioner: Some(&prov), saved_key: &|_| None, ports: &NoPorts, forks: &SAMEFORKS, servers: vec![] };
        let newname = format!("{} moved", rig.entry.name);
        let target = rig.tmp.path().join("moved");
        let mut steps = vec![];
        let r = run(&request(&path, "a good passphrase", &target, &newname), &path, &env, &mut |e| steps.push(e.step)).unwrap();
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        // the database came across into the NEW database
        assert_eq!(new_db.0.tables.borrow()[0].1, vec!["alice".to_string(), "bob".to_string()]);
        // the old login is not reused: the new one is in database.json and secrets.cfg, the licence key came from the file
        let dbj = std::fs::read_to_string(target.join("database.json")).unwrap();
        assert!(dbj.contains("kora_new1") && !dbj.contains("s3cr3t-pass"));
        let sec = std::fs::read_to_string(target.join("secrets.cfg")).unwrap();
        assert!(sec.contains("cfxk_FAKEFAKEFAKEFAKE_1234") && sec.contains("n3w-pass-123") && !sec.contains("s3cr3t-pass"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(target.join("secrets.cfg")).unwrap().permissions().mode() & 0o777, 0o600);
        }
        // settings and txAdmin data were placed; ports were rewritten; the old folder name is gone from them
        let cfg = std::fs::read_to_string(target.join("server.cfg")).unwrap();
        assert!(cfg.contains("0.0.0.0:31120") && !cfg.contains("30120"));
        assert!(target.join("txData/default/config.json").is_file());
        assert!(!target.join("txData/default/logs").exists());
        // resources: the kept-in-full one came from the file, the referenced one was fetched at its commit
        assert!(target.join("resources/local-only/fxmanifest.lua").is_file());
        assert!(r.resources_fetched.iter().any(|m| m.contains("core is back at version abcdef12")));
        assert_eq!(fetch.calls.borrow().as_slice(), &[1234], "the recorded FXServer build was asked for");
        // registered, with the new name and ports, and the record points at the new folder
        let reg = Registry::load().unwrap();
        let e = reg.find(&newname).unwrap();
        assert_eq!((e.ports.game, e.dir.clone()), (31120, target.clone()));
        let sf = ServerFile::load(&target).unwrap();
        assert_eq!((sf.name.as_str(), sf.dir.clone(), sf.database.as_deref()), (newname.as_str(), target.clone(), Some("kora_new1")));
        // nothing is left in staging, and the instructions for the old machine are there
        assert!(!target.join(".kora/import-staging").exists());
        assert!(r.old_machine_steps.iter().any(|s| s.contains("OLD computer")));
        assert!(steps.contains(&"database".to_string()) && steps.last().unwrap() == "done");
    }

    #[derive(Default)]
    struct FakeDbOnly(crate::backup::dbdump::fake::FakeDb);

    #[test]
    fn a_failure_is_resumable_from_where_it_stopped_and_start_over_cleans_up() {
        let rig = Rig::new();
        let mut git = rig.clean_git();
        git.clone_makes_git_dir = true;
        let path = make_backup(&rig, &git, "a good passphrase");
        let new_db = FakeDbOnly::default();
        let tools = Tools { db: &new_db.0, git: &git, db_client: "mariadb".into(), dump_program: None, now: 1_700_000_500 };
        let fetch = FakeFetcher { calls: RefCell::new(vec![]) };
        let target = rig.tmp.path().join("resumed");
        let name = format!("{} resumed", rig.entry.name);
        // first attempt: the database server cannot be reached
        let bad = FakeProvisioner { fail: true, dropped: RefCell::new(false) };
        let env = Env { tools: &tools, fetcher: &fetch, provisioner: Some(&bad), saved_key: &|_| None, ports: &NoPorts, forks: &SAMEFORKS, servers: vec![] };
        let err = run(&request(&path, "a good passphrase", &target, &name), &path, &env, &mut |_| {}).unwrap_err();
        assert_eq!(err.code, codes::DB_STOPPED);
        assert!(err.actions.contains(&Action::StartDatabase) && err.message.contains("Nothing is registered yet"));
        assert!(Registry::load().unwrap().find(&name).is_none(), "a failed import is not in the list");
        assert!(target.join("server.cfg").is_file(), "the finished steps are kept");
        assert_eq!(fetch.calls.borrow().len(), 1);
        // second attempt with a working database: carries on, does not download the server program again
        let good = FakeProvisioner { fail: false, dropped: RefCell::new(false) };
        let env = Env { tools: &tools, fetcher: &fetch, provisioner: Some(&good), saved_key: &|_| None, ports: &NoPorts, forks: &SAMEFORKS, servers: vec![] };
        git.say("cat-file -e abcdef1234567890abcdef1234567890abcdef12^{commit}", "");
        git.say("checkout -f -B main abcdef1234567890abcdef1234567890abcdef12", "");
        run(&request(&path, "a good passphrase", &target, &name), &path, &env, &mut |_| {}).unwrap();
        assert_eq!(fetch.calls.borrow().len(), 1, "resumed: the build was not downloaded again");
        assert!(Registry::load().unwrap().find(&name).is_some());
        // an existing non-empty folder that is not a resumable import is refused, never overwritten
        let other = rig.tmp.path().join("occupied");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("precious.txt"), b"mine").unwrap();
        let e = run(&request(&path, "a good passphrase", &other, &format!("{name} b")), &path, &env, &mut |_| {}).unwrap_err();
        assert!(e.message.contains("already exists and is not empty"));
        assert!(other.join("precious.txt").is_file());
    }

    #[test]
    fn a_picked_saved_licence_key_replaces_the_one_in_the_file_and_a_missing_one_is_explained() {
        let rig = Rig::new();
        let mut git = rig.clean_git();
        git.clone_makes_git_dir = true;
        let path = make_backup(&rig, &git, "a good passphrase");
        let new_db = FakeDbOnly::default();
        let tools = Tools { db: &new_db.0, git: &git, db_client: "mariadb".into(), dump_program: None, now: 1_700_000_500 };
        let fetch = FakeFetcher { calls: RefCell::new(vec![]) };
        let prov = FakeProvisioner { fail: false, dropped: RefCell::new(false) };
        let lookup = |n: &str| (n == "Other").then(|| "cfxk_OTHERKEY0123456789_abcdef".to_string());
        let env = Env { tools: &tools, fetcher: &fetch, provisioner: Some(&prov), saved_key: &lookup, ports: &NoPorts, forks: &SAMEFORKS, servers: vec![] };
        git.say("cat-file -e abcdef1234567890abcdef1234567890abcdef12^{commit}", "");
        git.say("checkout -f -B main abcdef1234567890abcdef1234567890abcdef12", "");
        let target = rig.tmp.path().join("keyed");
        let mut req = request(&path, "a good passphrase", &target, &format!("{} keyed", rig.entry.name));
        req.licence_key_name = Some("Other".into());
        run(&req, &path, &env, &mut |_| {}).unwrap();
        let sec = std::fs::read_to_string(target.join("secrets.cfg")).unwrap();
        assert!(sec.contains("cfxk_OTHERKEY0123456789_abcdef") && !sec.contains("FAKEFAKE"));
        let target2 = rig.tmp.path().join("nokey");
        let mut req = request(&path, "a good passphrase", &target2, &format!("{} nokey", rig.entry.name));
        req.licence_key_name = Some("Missing".into());
        assert!(run(&req, &path, &env, &mut |_| {}).unwrap_err().message.contains("no saved licence key called \"Missing\""));
    }

    #[test]
    fn clashes_are_checked_again_at_run_time() {
        let rig = Rig::new();
        let git = rig.clean_git();
        let path = make_backup(&rig, &git, "a good passphrase");
        let new_db = FakeDbOnly::default();
        let tools = Tools { db: &new_db.0, git: &git, db_client: "mariadb".into(), dump_program: None, now: 1 };
        let fetch = FakeFetcher { calls: RefCell::new(vec![]) };
        let here = vec![ServerEntry { name: "Taken".into(), dir: "/x".into(), mode: crate::installer::Mode::Production, source: None, versions: None, ports: PortsRecord { game: 31120, txadmin: 50000 }, created_unix: 1 }];
        let env = Env { tools: &tools, fetcher: &fetch, provisioner: None, saved_key: &|_| None, ports: &NoPorts, forks: &SAMEFORKS, servers: here };
        let t = rig.tmp.path().join("t");
        assert!(run(&request(&path, "a good passphrase", &t, "Taken"), &path, &env, &mut |_| {}).unwrap_err().message.contains("already here"));
        assert!(run(&request(&path, "a good passphrase", &t, "Fresh Name"), &path, &env, &mut |_| {}).unwrap_err().message.contains("port 31120"));
        assert!(!t.exists(), "nothing was made");
    }

    fn moved(rig: &Rig, choice: ForkChoice, forks: &FakeForks, tag: &str) -> (PathBuf, ImportResult, Vec<String>) {
        let mut git = rig.clean_git();
        git.clone_makes_git_dir = true;
        let path = make_backup(rig, &git, "a good passphrase");
        let new_db = FakeDbOnly::default();
        let tools = Tools { db: &new_db.0, git: &git, db_client: "mariadb".into(), dump_program: None, now: 1_700_000_500 };
        git.say("cat-file -e abcdef1234567890abcdef1234567890abcdef12^{commit}", "");
        git.say("checkout -f -B main abcdef1234567890abcdef1234567890abcdef12", "");
        let fetch = FakeFetcher { calls: RefCell::new(vec![]) };
        let prov = FakeProvisioner { fail: false, dropped: RefCell::new(false) };
        let env = Env { tools: &tools, fetcher: &fetch, provisioner: Some(&prov), saved_key: &|_| None, ports: &NoPorts, forks, servers: vec![] };
        let target = rig.tmp.path().join(tag);
        let mut req = request(&path, "a good passphrase", &target, &format!("{} {tag}", rig.entry.name));
        req.fork_choice = choice;
        let r = run(&req, &path, &env, &mut |_| {}).unwrap();
        let log = git.log.borrow().clone();
        (target, r, log)
    }

    #[test]
    fn a_fork_owner_this_computer_cannot_use_is_a_guided_choice_and_each_choice_does_what_it_says() {
        let rig = Rig::new();
        let git = rig.clean_git();
        let path = make_backup(&rig, &git, "a good passphrase");
        // the preview names the owner and says it cannot be used here
        let other = FakeForks { me: Some("someone-else"), made: Default::default() };
        let p = inspect(&path, Some("a good passphrase"), &[], &[], &NoPorts, &other, &|_| "/srv/n".into()).unwrap();
        let note = p.fork.unwrap();
        assert_eq!((note.owner.as_str(), note.usable, note.signed_in_as.as_deref()), ("old-owner", false, Some("someone-else")));
        assert!(note.message.contains("cannot use"));
        let same = FakeForks { me: Some("old-owner"), made: Default::default() };
        assert!(inspect(&path, Some("a good passphrase"), &[], &[], &NoPorts, &same, &|_| "/srv/n".into()).unwrap().fork.unwrap().usable);
        let out = FakeForks { me: None, made: Default::default() };
        assert!(inspect(&path, Some("a good passphrase"), &[], &[], &NoPorts, &out, &|_| "/srv/n".into()).unwrap().fork.unwrap().message.contains("not signed in"));

        // keep: the remotes stay as recorded; the server is still a fork server of the old owner
        let (t, _, log) = moved(&rig, ForkChoice::Keep, &other, "keep");
        assert!(!log.iter().any(|l| l.contains("remote set-url")));
        let sf = ServerFile::load(&t).unwrap();
        assert_eq!((sf.source(), sf.github_owner.as_deref()), (Source::Fork, Some("old-owner")));
        // originals: plain clones of the original repositories, a clone server without an owner
        let (t, r, log) = moved(&rig, ForkChoice::Originals, &other, "orig");
        assert!(log.iter().any(|l| l.contains("remote set-url origin https://github.com/KoraFiveM/local-only")), "{log:?}");
        let sf = ServerFile::load(&t).unwrap();
        assert_eq!((sf.source(), sf.github_owner.clone()), (Source::Clone, None));
        assert!(r.notes.iter().any(|n| n.contains("original repositories")));
        // own copies: forks are made in the account signed in here and the server follows them
        let (t, _, log) = moved(&rig, ForkChoice::OwnCopies, &other, "own");
        assert_eq!(other.made.borrow().len(), 2, "one fork per resource");
        assert!(log.iter().any(|l| l.contains("remote set-url origin https://github.com/someone-else/local-only")), "{log:?}");
        assert!(log.iter().any(|l| l.starts_with("clone -- https://github.com/someone-else/core")), "{log:?}");
        let sf = ServerFile::load(&t).unwrap();
        assert_eq!((sf.source(), sf.github_owner.as_deref()), (Source::Fork, Some("someone-else")));
        // every restored resource needs a restart, and the version setting is kept
        assert!(crate::resourcestate::pending(&t, true).contains(&"core".to_string()));
        assert_eq!(sf.versions(), crate::installer::Versions::Latest);
    }

    #[test]
    fn settings_are_converted_between_windows_and_linux() {
        let cfg = "exec \"C:\\Servers\\kora\\secrets.cfg\"\r\nset sv_x \"C:/Servers/kora/data\"\r\nendpoint_add_tcp \"0.0.0.0:30120\"\r\n";
        let to_linux = convert_text(cfg, "C:\\Servers\\kora", "/home/me/kora", false);
        assert_eq!(to_linux, "exec \"/home/me/kora/secrets.cfg\"\nset sv_x \"/home/me/kora/data\"\nendpoint_add_tcp \"0.0.0.0:30120\"\n");
        let json = r#"{"serverDataPath":"/home/me/kora","cfgPath":"/home/me/kora/server.cfg"}"#;
        let to_win = convert_text(json, "/home/me/kora", "D:\\FiveM\\kora", true);
        assert_eq!(to_win, r#"{"serverDataPath":"D:/FiveM/kora","cfgPath":"D:/FiveM/kora/server.cfg"}"#);
        let win_json = r#"{"serverDataPath":"C:\\Servers\\kora"}"#;
        assert_eq!(convert_text(win_json, "C:\\Servers\\kora", "/srv/kora", false), r#"{"serverDataPath":"/srv/kora"}"#);
        // a path that is not the old folder is left alone
        assert_eq!(convert_text("/home/other/x", "/home/me/kora", "/srv/k", false), "/home/other/x");
    }

    #[test]
    fn only_a_local_session_may_name_a_path_and_an_upload_id_is_checked() {
        let _ = crate::installer::registry::use_test_home();
        assert!(resolve_source(&ImportSource { path: Some("/etc/passwd".into()), upload: None }, false).is_err());
        assert!(resolve_source(&ImportSource { upload: Some("../../etc".into()), path: None }, true).is_err());
        assert!(resolve_source(&ImportSource::default(), true).is_err());
        assert!(resolve_source(&ImportSource { path: Some("/definitely/not/here.tar.gz".into()), upload: None }, true).unwrap_err().message.contains("no file at"));
    }
}
