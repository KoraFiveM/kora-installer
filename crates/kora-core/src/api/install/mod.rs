//! The install wizard as plain functions for the web panel: suggested answers, checks (ports,
//! disk, database login, FiveM builds), turning the UI's request into an [`InstallRequest`] with
//! the same rules as the CLI, and running the installer with progress.
//!
//! The logic is the same as the headless `kora install` (`kora-cli`); the parts that do not
//! depend on the command line are kept here because that crate is a front end. Everything that
//! touches the machine or the network goes through [`InstallEnv`], so tests use a fake.

pub mod github;

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{json, Value};

use crate::artifacts::{self, Platform};
use crate::firewall::ports::{self, PortProber, SystemProber};
use crate::firewall::PortRequest;
use crate::installer::registry::ServerEntry;
use crate::installer::{self, slug, DbAdmin, DbChoice, InstallOutcome, InstallRequest, Mode, Ports, ProgressEvent, ProgressSink, ResourceSelection, Source, StepStatus, Versions};
use crate::problem::{codes, Action, Problem};
use crate::space::{self, DiskInfo, SystemDisk};

/// Everything the install routes need from the machine. [`SystemInstallEnv`] is the real one.
pub trait InstallEnv: Send + Sync {
    fn prober(&self) -> &dyn PortProber;
    fn disk(&self) -> &dyn DiskInfo;
    /// Try a database admin login (`None` = automatic: environment / root socket).
    fn db_login(&self, admin: Option<&DbAdmin>) -> Result<(), Problem>;
    /// The FiveM server builds that can be installed.
    fn artifacts(&self) -> Result<artifacts::ArtifactList, Problem>;
    /// Look a Cfx.re username up on forum.cfx.re (the real lookup unless a test replaces it).
    fn cfx_lookup(&self, name: &str) -> Result<crate::cfx::CfxAccount, Problem> {
        crate::cfx::lookup(name).map_err(Problem::from)
    }
    /// Does this process have administrator / root rights?
    fn is_elevated(&self) -> bool;
    /// Plain reasons the request needs administrator rights (empty = none).
    fn admin_reasons(&self, req: &InstallRequest) -> Vec<String>;
    fn saved_load(&self) -> Option<InstallRequest>;
    fn saved_save(&self, req: &InstallRequest) -> bool;
    fn saved_clear(&self);
    /// Mark a failed step as skipped so the next [`InstallEnv::run`] goes on without it
    /// (see [`installer::skip_step`]: only steps that are safe to skip).
    fn skip_step(&self, req: &InstallRequest, step: &str) -> Result<(), Problem> {
        installer::skip_step(req, step)
    }
    /// Run the installer (a dry run when `req.dry_run`). A failure is a [`Problem`].
    fn run(&self, req: &InstallRequest, sink: &mut dyn ProgressSink) -> Result<InstallOutcome, Problem>;
    /// [`InstallEnv::run`] with an administrator helper for the steps that need one (the Windows
    /// panel's worker, see `installer::system_ops`). Fakes may ignore it.
    fn run_with_admin(&self, req: &InstallRequest, sink: &mut dyn ProgressSink, admin: Option<installer::system_ops::SharedAdmin>) -> Result<InstallOutcome, Problem> {
        let _ = admin;
        self.run(req, sink)
    }
}

/// The real machine.
pub struct SystemInstallEnv;

impl InstallEnv for SystemInstallEnv {
    fn prober(&self) -> &dyn PortProber {
        &SystemProber
    }
    fn disk(&self) -> &dyn DiskInfo {
        &SystemDisk
    }
    fn db_login(&self, admin: Option<&DbAdmin>) -> Result<(), Problem> {
        installer::check_db_admin(admin)
    }
    fn artifacts(&self) -> Result<artifacts::ArtifactList, Problem> {
        let http = installer::Hooks::from_env().http;
        artifacts::list_artifacts(&http, Platform::current()).map_err(Problem::from)
    }
    fn is_elevated(&self) -> bool {
        installer::is_admin()
    }
    fn admin_reasons(&self, req: &InstallRequest) -> Vec<String> {
        crate::elevate::admin_reasons(req)
    }
    fn saved_load(&self) -> Option<InstallRequest> {
        installer::saved::load()
    }
    fn saved_save(&self, req: &InstallRequest) -> bool {
        installer::saved::save(req).is_ok()
    }
    fn saved_clear(&self) {
        installer::saved::clear();
    }
    fn run(&self, req: &InstallRequest, sink: &mut dyn ProgressSink) -> Result<InstallOutcome, Problem> {
        // The raw text: it may be a packed Problem (see `installer::Problem`).
        installer::run(req, sink)
    }
    fn run_with_admin(&self, req: &InstallRequest, sink: &mut dyn ProgressSink, admin: Option<installer::system_ops::SharedAdmin>) -> Result<InstallOutcome, Problem> {
        let hooks = installer::Hooks { admin, ..installer::Hooks::from_env() };
        installer::run_with(req, sink, &hooks)
    }
}

// ---------------------------------------------------------------------------------------------
// The UI's request
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WireDb {
    #[default]
    UseExisting,
    InstallIfMissing,
    Skip,
    /// Container mode: a MariaDB container next to the server (the default there).
    Container,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireResources {
    All,
    Only(Vec<String>),
}

#[derive(Clone, Deserialize)]
pub struct WireDbAdmin {
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub password: Option<String>,
}

/// Retired in 0.7.0: still accepted from an old client, never used.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct WireApp {
    #[serde(default)]
    pub desktop_shortcut: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WirePorts {
    pub game: u16,
    pub txadmin: u16,
}

/// The body of `POST /installs` and `/installs/plan` (the UI's `InstallRequest`). Secrets are
/// read here and never written back out (no `Debug`/`Serialize`).
#[derive(Clone, Deserialize, Default)]
pub struct WireInstall {
    #[serde(default)]
    pub mode: Option<Mode>,
    /// Clone or fork (new in 0.8; wins over `mode`).
    #[serde(default)]
    pub source: Option<Source>,
    /// Released versions or the latest changes (new in 0.8; wins over `mode`).
    #[serde(default)]
    pub versions: Option<Versions>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub dir: String,
    #[serde(default)]
    pub hostname: String,
    #[serde(default)]
    pub license_key: Option<String>,
    /// Use the saved key with this name instead of carrying a key.
    #[serde(default)]
    pub license_key_name: Option<String>,
    /// With a pasted `license_key`: save it under this name (the wizard's default), so the next install can pick it.
    #[serde(default)]
    pub save_license_key_as: Option<String>,
    #[serde(default)]
    pub ports: Option<WirePorts>,
    #[serde(default)]
    pub txadmin_public: bool,
    #[serde(default)]
    pub artifact_url: Option<String>,
    #[serde(default)]
    pub game_build: Option<u32>,
    #[serde(default)]
    pub resources: Option<WireResources>,
    #[serde(default)]
    pub db: Option<WireDb>,
    #[serde(default)]
    pub service: bool,
    #[serde(default)]
    pub firewall: bool,
    #[serde(default)]
    pub github_owner: Option<String>,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub db_admin: Option<WireDbAdmin>,
    #[serde(default)]
    pub start: bool,
    #[serde(default)]
    pub app: Option<WireApp>,
    #[serde(default)]
    pub cfx_identifier: Option<String>,
    /// `service` (default) or `container` (Linux, needs a container engine).
    #[serde(default)]
    pub run_mode: Option<crate::container::RunMode>,
    /// What container mode asks for (engine, address, limits); only read with `run_mode: container`.
    #[serde(default)]
    pub container: Option<crate::container::ContainerRequest>,
}

impl WireDbAdmin {
    pub fn to_admin(&self) -> DbAdmin {
        DbAdmin {
            host: if self.host.trim().is_empty() { "127.0.0.1".into() } else { self.host.trim().to_string() },
            port: self.port.unwrap_or(3306),
            user: if self.user.trim().is_empty() { "root".into() } else { self.user.trim().to_string() },
            password: self.password.clone().filter(|p| !p.is_empty()),
        }
    }
}

fn non_empty(s: &Option<String>) -> Option<String> {
    s.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(String::from)
}

/// Default install folder (see `userhome::default_server_dir`); empty when no home folder can be
/// found (the user then has to choose one).
pub fn default_dir(name: &str) -> String {
    crate::userhome::default_server_dir(&slug(name)).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The wizard's name rules.
pub fn validate_name(name: &str) -> Result<(), Problem> {
    let n = name.trim();
    if n.is_empty() {
        return Err("Please give your server a name.".into());
    }
    if n.chars().count() > 32 {
        return Err("That name is too long. Please use 32 characters or fewer.".into());
    }
    if !n.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '-' || c == '_') {
        return Err("Please use only letters, numbers, spaces, dashes and underscores.".into());
    }
    if slug(n).is_empty() {
        return Err("The name needs at least one letter or number.".into());
    }
    Ok(())
}

/// A resource id as accepted by `--resources` and the wizard (letters, digits, `_ . -`).
fn valid_resource_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// Check the UI's request and turn it into an [`InstallRequest`]: the CLI's `into_request`
/// defaults (folder, hostname, catalogue from `KORA_CATALOG`, no app on a dry run) plus the
/// installer's own validation, so a bad request is refused before anything starts.
pub fn build_request(w: &WireInstall) -> Result<InstallRequest, Problem> {
    validate_name(&w.name)?;
    let name = w.name.trim().to_string();
    let dir_text = w.dir.trim();
    let dir = if dir_text.is_empty() { crate::userhome::default_server_dir(&slug(&name))? } else { PathBuf::from(dir_text) };
    if !dir.is_absolute() {
        return Err("Please give the full path of the folder, for example /home/you/kora/my-server.".into());
    }
    let defaults = Ports::default();
    let ports = w.ports.as_ref().map_or(defaults, |p| Ports { game: p.game, txadmin: p.txadmin });
    for (label, p) in [("game", ports.game), ("txAdmin", ports.txadmin)] {
        if p < 1024 {
            return Err(format!("The {label} port must be a number between 1024 and 65535.").into());
        }
    }
    PortRequest { server: slug(&name), game_port: ports.game, txadmin_port: ports.txadmin, expose_txadmin: w.txadmin_public }.validate().map_err(|e| e.to_string())?;
    let (source, versions) = crate::installer::resolve(w.mode, w.source, w.versions);
    let mode = crate::installer::compat_mode(versions);
    let artifact_url = non_empty(&w.artifact_url);
    if let Some(u) = &artifact_url {
        artifacts::validate_url(u, Platform::current()).map_err(|e| e.to_string())?;
    }
    let github_owner = if source == Source::Fork { non_empty(&w.github_owner) } else { None };
    if let Some(o) = &github_owner {
        if !o.eq_ignore_ascii_case("list") {
            crate::git::owners::check_login_syntax(o)?;
        }
    }
    let resources = match &w.resources {
        None | Some(WireResources::All) => ResourceSelection::All,
        Some(WireResources::Only(ids)) => {
            let ids: Vec<String> = ids.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            if let Some(bad) = ids.iter().find(|i| !valid_resource_id(i)) {
                return Err(format!("'{bad}' is not a resource name (letters, digits, _ . - only).").into());
            }
            ResourceSelection::Only(ids)
        }
    };
    let container = match w.run_mode.unwrap_or_default() {
        crate::container::RunMode::Service => None,
        crate::container::RunMode::Container => {
            if !cfg!(target_os = "linux") {
                return Err("Container mode is only available on Linux.".into());
            }
            let c = w.container.clone().unwrap_or_default();
            if let Some(b) = c.bind.as_deref() {
                if b.parse::<std::net::IpAddr>().is_err() {
                    return Err("The address to publish the game port on must be an IP address (or left empty for all addresses).".into());
                }
            }
            if c.memory_mb.is_some_and(|m| m < 512) {
                return Err("A memory limit under 512 MiB is too small for FXServer.".into());
            }
            Some(c)
        }
    };
    let db_admin = match &w.db_admin {
        None => None,
        Some(a) => {
            let admin = a.to_admin();
            crate::db::validate_host(&admin.host).map_err(|e| e.to_string())?;
            Some(admin)
        }
    };
    Ok(InstallRequest {
        mode,
        source: Some(source),
        versions: Some(versions),
        hostname: non_empty(&Some(w.hostname.clone())).unwrap_or_else(|| name.clone()),
        name,
        dir,
        license_key: non_empty(&w.license_key),
        license_key_name: non_empty(&w.license_key_name),
        cfx_identifier: non_empty(&w.cfx_identifier),
        ports,
        txadmin_public: w.txadmin_public,
        artifact_url,
        game_build: w.game_build,
        resources,
        db: match (w.db, container.is_some()) {
            // In a container the managed database container is the default; on a host it never exists.
            (None, true) => DbChoice::Container,
            (Some(WireDb::Container), true) => DbChoice::Container,
            (Some(WireDb::Container), false) => return Err("A database container needs container mode: choose 'In a container' for how the server runs.".into()),
            (d, _) => match d.unwrap_or_default() {
                WireDb::UseExisting | WireDb::Container => DbChoice::UseExisting,
                WireDb::InstallIfMissing => DbChoice::InstallIfMissing,
                WireDb::Skip => DbChoice::Skip,
            },
        },
        // A container server has no service: no unit, no service account, no administrator rights.
        service: w.service && container.is_none(),
        firewall: w.firewall,
        github_owner,
        catalog: std::env::var("KORA_CATALOG").ok().filter(|c| !c.is_empty()),
        dry_run: w.dry_run,
        db_admin,
        start: w.start,
        // Retired in 0.7.0: an old client may still send `app`; it is ignored.
        app: None,
        container,
    })
}

/// The answers in the UI's shape, WITHOUT any secret (no licence key, no database login at all).
pub fn request_json(r: &InstallRequest) -> Value {
    json!({
        "mode": if r.versions() == Versions::Released { "production" } else { "development" },
        "source": r.source(),
        "versions": r.versions(),
        "name": r.name,
        "dir": r.dir.to_string_lossy(),
        "hostname": r.hostname,
        "ports": {"game": r.ports.game, "txadmin": r.ports.txadmin},
        "txadmin_public": r.txadmin_public,
        "artifact_url": r.artifact_url,
        "game_build": r.game_build,
        "resources": match &r.resources {
            ResourceSelection::All => json!("all"),
            ResourceSelection::Only(v) => json!({"only": v}),
        },
        "db": match r.db {
            DbChoice::UseExisting => "use_existing",
            DbChoice::InstallIfMissing => "install_if_missing",
            DbChoice::Skip => "skip",
            DbChoice::Container => "container",
        },
        "run_mode": if r.container.is_some() { "container" } else { "service" },
        "container": r.container,
        "service": r.service,
        "firewall": r.firewall,
        "github_owner": r.github_owner,
        "start": r.start,
        "license_key_name": r.license_key_name,
        // Not a secret; the wizard puts it back so a resumed install keeps the same txAdmin admin.
        "cfx_identifier": r.cfx_identifier,
    })
}

// ---------------------------------------------------------------------------------------------
// Ports
// ---------------------------------------------------------------------------------------------

/// Game and txAdmin ports of the OTHER servers (also when stopped), leaving out the one being
/// installed (matched by name or folder). Same rule as `ports::registered_ports_except`, over a given list.
pub fn avoid_ports(servers: &[ServerEntry], own_name: &str, own_dir: Option<&Path>) -> Vec<u16> {
    servers
        .iter()
        .filter(|s| (own_name.is_empty() || slug(&s.name) != slug(own_name)) && (own_dir.is_none() || Some(s.dir.as_path()) != own_dir))
        .flat_map(|s| [s.ports.game, s.ports.txadmin])
        .collect()
}

/// One port as the UI shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortLineInfo {
    pub port: u16,
    pub ok: bool,
    pub text: String,
    pub suggestion: Option<u16>,
}

impl PortLineInfo {
    pub fn json(&self) -> Value {
        json!({"port": self.port, "ok": self.ok, "text": self.text, "suggestion": self.suggestion})
    }
}

/// Status of both port boxes with the next free port when one is taken (the wizard's text).
pub fn port_lines(game: u16, txadmin: u16, servers: &[ServerEntry], prober: &dyn PortProber) -> (PortLineInfo, PortLineInfo) {
    let avoid = avoid_ports(servers, "", None);
    let report = ports::check_ports(game, txadmin, &avoid, prober);
    let other_server = |port: u16| servers.iter().find(|s| s.ports.game == port || s.ports.txadmin == port).map(|s| s.name.clone());
    let line = |port: u16, other: u16, st: ports::PortStatus| -> PortLineInfo {
        match st {
            ports::PortStatus::Free => PortLineInfo { port, ok: true, text: format!("{port} is free"), suggestion: None },
            ports::PortStatus::InUse { suggestion } => {
                let why = if port == other {
                    "is the same as the other port".to_string()
                } else if let Some(n) = other_server(port) {
                    format!("is used by your other server '{n}'")
                } else {
                    "is already in use".to_string()
                };
                let text = match suggestion {
                    Some(s) => format!("{port} {why} - {s} is free"),
                    None => {
                        format!("{port} {why} - no free port found nearby, please type another")
                    }
                };
                PortLineInfo { port, ok: false, text, suggestion }
            }
        }
    };
    (line(game, txadmin, report.game), line(txadmin, game, report.txadmin))
}

/// Check the requested ports against this machine (and the other servers). With `auto`, move
/// taken ports to the next free ones and return what changed; without, a taken port is an error
/// naming the port and the next free one. (The CLI's `settle_ports_avoiding`.)
pub fn settle_ports(req: &mut InstallRequest, auto: bool, prober: &dyn PortProber, avoid: &[u16]) -> Result<Vec<String>, Problem> {
    let (game, tx) = (req.ports.game, req.ports.txadmin);
    let report = ports::check_ports(game, tx, avoid, prober);
    if report.all_free() {
        return Ok(vec![]);
    }
    if auto {
        let Some((g, t)) = ports::effective_ports(game, tx, avoid, prober) else {
            return Err("No free port was found near the ones you chose. Pick other ports.".into());
        };
        let mut changes = Vec::new();
        for (old, new) in [(game, g), (tx, t)] {
            if old != new {
                changes.push(format!("Port {old} is in use, using {new}"));
            }
        }
        req.ports.game = g;
        req.ports.txadmin = t;
        return Ok(changes);
    }
    Err(ports::conflict_messages(game, tx, &report, avoid).join(" ").into())
}

/// A finished server (resume/reinstall of the same folder) keeps its ports.
pub fn already_installed(req: &InstallRequest) -> bool {
    req.dir.join("kora-server.json").is_file()
}

// ---------------------------------------------------------------------------------------------
// Defaults, disk, plan
// ---------------------------------------------------------------------------------------------

/// `GET /installs/defaults`: the wizard's first answers and what it offers to resume.
pub fn defaults(env: &dyn InstallEnv, servers: &[ServerEntry]) -> Value {
    let name = "My Kora Server";
    let avoid = avoid_ports(servers, "", None);
    let d = Ports::default();
    let (game, tx) = ports::effective_ports(d.game, d.txadmin, &avoid, env.prober()).unwrap_or((d.game, d.txadmin));
    json!({
        "name": name,
        "dir": default_dir(name),
        "hostname": name,
        "ports": {"game": game, "txadmin": tx},
        "saved": env.saved_load().map(|r| request_json(&r)),
    })
}

/// Free space against what an install needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskReport {
    pub ok: bool,
    pub free_bytes: u64,
    pub needed_bytes: u64,
    pub message: String,
}

impl DiskReport {
    pub fn json(&self) -> Value {
        json!({"ok": self.ok, "free_bytes": self.free_bytes, "needed_bytes": self.needed_bytes, "message": self.message})
    }
}

/// The installer's own pre-flight check (`installer::preflight`), with the numbers.
pub fn disk_check(disk: &dyn DiskInfo, dir: &Path, selected: Option<usize>) -> DiskReport {
    let resuming = dir.join(".kora").is_dir();
    let needs = space::install_needs(dir, space::resource_estimate(selected), resuming);
    let needed: u64 = needs.iter().map(|n| n.bytes).sum();
    let free = disk.free_bytes(dir).ok();
    match space::check(disk, &needs) {
        Ok(()) => {
            let free_bytes = free.unwrap_or(0);
            let message = match free {
                Some(f) => format!("{} free, about {} needed.", space::human(f), space::human(needed)),
                None => format!("About {} is needed. Kora could not read how much is free here.", space::human(needed)),
            };
            DiskReport { ok: true, free_bytes, needed_bytes: needed, message }
        }
        Err(message) => DiskReport { ok: false, free_bytes: free.unwrap_or(0), needed_bytes: needed, message },
    }
}

/// How many resources the request selects (`None` = all), for the disk estimate.
pub fn selection_size(req: &InstallRequest) -> Option<usize> {
    match &req.resources {
        ResourceSelection::All => None,
        ResourceSelection::Only(v) => Some(v.len()),
    }
}

/// The dry-run header: the effective choices. Never includes secrets.
pub fn request_summary(req: &InstallRequest) -> String {
    let resources = match &req.resources {
        ResourceSelection::All => "all".to_string(),
        ResourceSelection::Only(v) => format!("{} chosen ({})", v.len(), v.join(", ")),
    };
    format!(
        "Your choices:\n  name:      {}\n  copies:    {}\n  versions:  {}\n  resources: {}\n  ports:     game {}, txAdmin {}\n  artifact:  {}\n  game build: {}\n  hostname:  {}\n  cfx owner: {}\n",
        req.name,
        match req.source() {
            Source::Clone => "plain clones of the originals".to_string(),
            Source::Fork => format!("your own copies (forks) in {}", req.github_owner.as_deref().unwrap_or("your GitHub account")),
        },
        match req.versions() {
            Versions::Released => "released versions",
            Versions::Latest => "the latest changes",
        },
        resources,
        req.ports.game,
        req.ports.txadmin,
        req.artifact_url.as_deref().unwrap_or("recommended"),
        req.game_build.map_or("default".to_string(), |b| b.to_string()),
        if req.hostname.is_empty() { "(none)" } else { &req.hostname },
        req.cfx_identifier.as_deref().unwrap_or("(none)"),
    )
}

fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// `POST /installs/plan`: what the install would do. Nothing is changed and no secret is returned.
pub fn plan(env: &dyn InstallEnv, req: &InstallRequest, servers: &[ServerEntry]) -> Value {
    let mut live = req.clone();
    live.dry_run = false;
    let admin = env.is_elevated();
    let steps: Vec<Value> = installer::plan_steps(&live, admin).iter().map(|s| json!({"id": s.id, "title": s.title})).collect();
    let needs_admin: Vec<String> = env.admin_reasons(&live).iter().map(|r| capitalised(r)).collect();
    let disk = disk_check(env.disk(), &req.dir, selection_size(req));
    let mut warnings: Vec<String> = Vec::new();
    if !already_installed(req) {
        let avoid = avoid_ports(servers, &req.name, Some(&req.dir));
        let report = ports::check_ports(req.ports.game, req.ports.txadmin, &avoid, env.prober());
        warnings.extend(ports::conflict_messages(req.ports.game, req.ports.txadmin, &report, &avoid));
    }
    if !disk.ok {
        warnings.push(disk.message.clone());
    }
    if req.source() == Source::Fork && req.github_owner.is_none() {
        warnings.push("No GitHub account was chosen to fork into. Kora uses your personal account when it is the only choice, otherwise pick one.".into());
    }
    if !needs_admin.is_empty() && !admin {
        warnings.push("Some of this needs administrator rights, which Kora does not have here. Kora will show the command to run, or you can turn those choices off.".into());
    }
    json!({
        "summary": request_summary(req),
        "steps": steps,
        "needs_admin": needs_admin,
        "elevated": admin,
        "disk": disk.json(),
        "warnings": warnings,
        "notes": warnings,
    })
}

// ---------------------------------------------------------------------------------------------
// Elevation
// ---------------------------------------------------------------------------------------------

/// The `kora install ...` command that does this install from an administrator prompt. It NEVER
/// contains the licence key or the database password: the caller tells the user to set
/// `CFX_LICENSE_KEY` / `KORA_DB_ADMIN_PASSWORD` (which the CLI reads).
pub fn install_command(req: &InstallRequest) -> Vec<String> {
    let mut v: Vec<String> = ["kora", "install", "--name"].iter().map(|s| s.to_string()).collect();
    v.push(req.name.clone());
    v.extend(["--dir".to_string(), req.dir.to_string_lossy().into_owned()]);
    v.extend(["--source".to_string(), if req.source() == Source::Clone { "clone" } else { "fork" }.to_string()]);
    v.extend(["--versions".to_string(), if req.versions() == Versions::Released { "released" } else { "latest" }.to_string()]);
    if !req.hostname.is_empty() && req.hostname != req.name {
        v.extend(["--hostname".to_string(), req.hostname.clone()]);
    }
    v.extend(["--game-port".to_string(), req.ports.game.to_string(), "--txadmin-port".to_string(), req.ports.txadmin.to_string()]);
    if req.txadmin_public {
        v.push("--txadmin-public".into());
    }
    if let Some(c) = &req.cfx_identifier {
        v.extend(["--cfx-identifier".to_string(), c.clone()]);
    }
    if let Some(u) = &req.artifact_url {
        v.extend(["--artifact-url".to_string(), u.clone()]);
    }
    if let Some(b) = req.game_build {
        v.extend(["--game-build".to_string(), b.to_string()]);
    }
    if let ResourceSelection::Only(ids) = &req.resources {
        v.extend(["--resources".to_string(), ids.join(",")]);
    }
    v.extend([
        "--db".to_string(),
        match req.db {
            DbChoice::UseExisting => "existing",
            DbChoice::InstallIfMissing => "install",
            DbChoice::Skip => "skip",
            DbChoice::Container => "container",
        }
        .to_string(),
    ]);
    if let Some(c) = &req.container {
        v.extend(["--run-mode".to_string(), "container".to_string()]);
        if let Some(e) = c.engine {
            v.extend(["--engine".to_string(), e.program().to_string()]);
        }
        if let Some(b) = &c.bind {
            v.extend(["--bind".to_string(), b.clone()]);
        }
        if let Some(m) = c.memory_mb {
            v.extend(["--memory-mb".to_string(), m.to_string()]);
        }
        if let Some(t) = c.cpu_tenths {
            v.extend(["--cpus".to_string(), format!("{}.{}", t / 10, t % 10)]);
        }
    }
    if let Some(a) = &req.db_admin {
        v.extend(["--db-admin-user".to_string(), a.user.clone(), "--db-admin-host".to_string(), a.host.clone(), "--db-admin-port".to_string(), a.port.to_string()]);
    }
    if req.service {
        v.push("--service".into());
    }
    if req.firewall {
        v.push("--firewall".into());
    }
    if let Some(o) = &req.github_owner {
        v.extend(["--github-owner".to_string(), o.clone()]);
    }
    v.push(if req.start { "--start" } else { "--no-start" }.into());
    v.push("--yes".into());
    v
}

/// The answers without anything that needs administrator rights ("Continue without the
/// administrator parts"): no firewall change, no background service (so no update task), and the
/// database only used when it is already there. The same rule as the wizard's `withoutAdmin`.
pub fn without_admin(req: &InstallRequest) -> InstallRequest {
    let mut r = req.clone();
    r.firewall = false;
    r.txadmin_public = false;
    r.service = false;
    if r.db == DbChoice::InstallIfMissing {
        r.db = DbChoice::UseExisting;
    }
    r
}

/// What to tell the user to do before running [`install_command`] (names the variables, never values).
pub fn elevation_message(req: &InstallRequest, reasons: &[String]) -> String {
    let mut m = format!(
        "This install needs administrator rights to {}. Kora is not running as an administrator, so run the command below from an administrator prompt (or turn those choices off).",
        crate::elevate::join_reasons(reasons)
    );
    let mut vars = Vec::new();
    if req.license_key.is_some() {
        vars.push("CFX_LICENSE_KEY (your Cfx.re licence key)");
    }
    if req.db_admin.as_ref().is_some_and(|a| a.password.is_some()) {
        vars.push("KORA_DB_ADMIN_PASSWORD (the database administrator password)");
    }
    if !vars.is_empty() {
        m.push_str(&format!(" For your safety the command does not contain secrets: set {} first.", vars.join(" and ")));
    }
    m
}

// ---------------------------------------------------------------------------------------------
// Running
// ---------------------------------------------------------------------------------------------

fn status_name(s: StepStatus) -> &'static str {
    match s {
        StepStatus::Pending => "pending",
        StepStatus::Running => "running",
        StepStatus::Done => "done",
        StepStatus::Skipped => "skipped",
        StepStatus::Failed => "failed",
    }
}

/// A progress event in the UI's shape (snake_case status).
pub fn progress_json(ev: &ProgressEvent) -> Value {
    json!({"step": ev.step, "title": ev.title, "status": status_name(ev.status), "detail": ev.detail, "percent": ev.percent})
}

/// The plain-text log line the CLI keeps for an event.
pub fn log_line(ev: &ProgressEvent) -> String {
    format!("{:>3}% {:?} {} ({}): {}", ev.percent, ev.status, ev.title, ev.step, ev.detail)
}

struct FnSink<F: FnMut(&ProgressEvent) + Send> {
    f: F,
}

impl<F: FnMut(&ProgressEvent) + Send> ProgressSink for FnSink<F> {
    fn event(&mut self, ev: ProgressEvent) {
        (self.f)(&ev);
    }
}

/// How a finished run ended, ready for the job result / error.
pub struct Finished {
    /// The outcome, or the installer's problem when it failed.
    pub outcome: Result<InstallOutcome, Problem>,
    /// The log file (when one could be written).
    pub log_path: Option<PathBuf>,
    /// After a failure: what was kept on disk (folder, saved answers) and how to carry on.
    pub kept: Vec<String>,
    /// After a failure: the terminal's "how to carry on" text (the panel shows buttons instead).
    pub next: String,
    /// After a failure: the step that failed (`None` = it stopped before the first step).
    pub failed_step: Option<String>,
    /// After a failure: what to tell the user, with the actions that fit ([`failure_problem`]).
    pub problem: Option<Problem>,
}

/// Run the install, calling `on_event` for every progress event, then write the log and keep or
/// forget the saved answers exactly like the CLI does (a dry run saves nothing).
pub fn run_install(env: &dyn InstallEnv, req: &InstallRequest, on_event: impl FnMut(&ProgressEvent) + Send) -> Finished {
    run_install_with(env, req, None, on_event)
}

/// [`run_install`] with an administrator helper for the steps that need one.
pub fn run_install_with(env: &dyn InstallEnv, req: &InstallRequest, admin: Option<installer::system_ops::SharedAdmin>, mut on_event: impl FnMut(&ProgressEvent) + Send) -> Finished {
    let mut log: Vec<String> = Vec::new();
    let mut failed_step: Option<String> = None;
    let result = {
        let log_ref = &mut log;
        let failed_ref = &mut failed_step;
        let mut sink = FnSink {
            f: |ev: &ProgressEvent| {
                log_ref.push(log_line(ev));
                if ev.status == StepStatus::Failed {
                    *failed_ref = Some(ev.step.clone());
                }
                on_event(ev);
            },
        };
        env.run_with_admin(req, &mut sink, admin)
    };
    let done = |outcome, log_path| Finished { outcome, log_path, kept: vec![], next: String::new(), failed_step: None, problem: None };
    if req.dry_run {
        return done(result, None);
    }
    match result {
        Ok(out) => {
            let log_path = write_install_log(req, &log, None);
            env.saved_clear();
            done(Ok(out), log_path)
        }
        Err(msg) => {
            let log_path = write_install_log(req, &log, Some(&msg.one_line()));
            let saved = env.saved_save(req);
            let mut kept = Vec::new();
            if req.dir.join(".kora").is_dir() {
                kept.push(format!("What was already created in {}", req.dir.display()));
            }
            if saved {
                kept.push("Your answers (not the licence key or database password)".to_string());
            }
            let problem = failure_problem(req, msg.clone(), failed_step.as_deref());
            Finished { outcome: Err(msg), log_path, kept, next: failure_next_steps(req, saved), failed_step, problem: Some(problem) }
        }
    }
}

/// The [`Problem`] the panel shows when an install stopped: the installer's own problem (or its
/// text as a generic one) followed by the ways on: retry the failed step (or the whole install when
/// it stopped before the first step), skip the step when it is safe to ([`installer::SKIPPABLE`]),
/// and change the answers.
pub fn failure_problem(req: &InstallRequest, error: Problem, failed_step: Option<&str>) -> Problem {
    let mut p = error;
    match failed_step {
        Some(step) => {
            if p.code == codes::FAILED {
                p.code = codes::INSTALL_STEP_FAILED.to_string();
            }
            p.push_action(Action::RetryStep { step: step.to_string() });
            let skippable = installer::plan_steps(req, false).iter().any(|s| s.id == step && s.skippable);
            if skippable {
                p.push_action(Action::SkipStep { step: step.to_string() });
            }
        }
        None => p.push_action(Action::Retry),
    }
    // Not enough room: trying again cannot help; the way on is another folder (the name-and-folder page).
    if p.message.starts_with("Not enough free disk space") && !p.actions.iter().any(|a| matches!(a, Action::ChangeAnswers { .. })) {
        p.actions.insert(0, Action::change_answers("name"));
    }
    if !p.actions.iter().any(|a| matches!(a, Action::ChangeAnswers { .. })) {
        p.push_action(Action::ChangeAnswers { section: None });
    }
    p
}

/// What is kept after a failed install and how to carry on (the CLI's wording, for the panel).
pub fn failure_next_steps(req: &InstallRequest, saved: bool) -> String {
    if req.dir.join(".kora").is_dir() {
        format!(
            "What was done so far is kept{}.\nTo try again: start the install again with the same answers; it continues where it stopped.\nTo start over: delete the server folder {} (or run `kora delete {}`).",
            if saved { ", and so are your answers (not the licence key or database password)" } else { "" },
            req.dir.display(),
            req.dir.display()
        )
    } else if saved {
        "Nothing was created. Your answers are saved (not the licence key or database password): fix the problem above and try again.".to_string()
    } else {
        "Nothing was created. Fix the problem above and try again.".to_string()
    }
}

/// Where the install log goes: inside the server's state folder when it exists, else the temp dir.
pub fn write_install_log(req: &InstallRequest, lines: &[String], error: Option<&str>) -> Option<PathBuf> {
    let meta = req.dir.join(".kora");
    let path = if meta.is_dir() { meta.join("install.log") } else { std::env::temp_dir().join(format!("kora-install-{}.log", slug(&req.name))) };
    let mut text = lines.join("\n");
    if let Some(e) = error {
        text.push_str(&format!("\nERROR: {e}\n"));
    }
    std::fs::write(&path, text).ok().map(|_| path)
}

/// The outcome as the UI reads it (the password itself is never part of it, only its file).
pub fn outcome_json(o: &InstallOutcome) -> Value {
    serde_json::to_value(o).unwrap_or(Value::Null)
}

// ---------------------------------------------------------------------------------------------
// Artifacts
// ---------------------------------------------------------------------------------------------

/// `GET /artifacts`: builds that are not known to be broken, newest first.
pub fn artifacts_json(list: &artifacts::ArtifactList) -> Value {
    let items: Vec<Value> = list.all.iter().filter(|b| b.broken.is_none()).map(|b| json!({"build": b.build, "url": b.url})).collect();
    json!({"recommended": list.recommended.as_ref().map(|b| b.build), "items": items})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(name: &str) -> WireInstall {
        WireInstall {
            name: name.into(),
            // Absolute on every platform ("/tmp/..." is not absolute on Windows).
            dir: std::env::temp_dir().join("kora-x").to_string_lossy().into_owned(),
            ..Default::default()
        }
    }

    /// The wizard puts the saved answers back and reads `cfx_identifier` from them (the panel's TypeScript
    /// always declared it); it is an account name, not a secret, so a resumed install keeps its txAdmin admin.
    #[test]
    fn saved_answers_keep_the_cfx_identifier_but_never_a_secret() {
        let mut w = wire("Alpha");
        w.cfx_identifier = Some("fivem:4465268".into());
        w.license_key = Some("cfxk_SECRETKEYSECRETKEY".into());
        w.db_admin = Some(WireDbAdmin { host: "".into(), port: None, user: "root".into(), password: Some("hunter2".into()) });
        let v = request_json(&build_request(&w).unwrap());
        assert_eq!(v["cfx_identifier"], "fivem:4465268");
        let text = v.to_string();
        assert!(!text.contains("SECRET") && !text.contains("hunter2"), "{text}");
    }

    #[test]
    fn the_old_mode_and_the_new_fields_both_work_and_the_new_ones_win() {
        let pick = |mode: Option<Mode>, source: Option<Source>, versions: Option<Versions>| {
            let mut w = wire("Alpha");
            w.mode = mode;
            w.source = source;
            w.versions = versions;
            let r = build_request(&w).unwrap();
            (r.source(), r.versions(), r.mode)
        };
        assert_eq!(pick(None, None, None), (Source::Clone, Versions::Released, Mode::Production));
        assert_eq!(pick(Some(Mode::Development), None, None), (Source::Fork, Versions::Latest, Mode::Development));
        assert_eq!(pick(None, Some(Source::Fork), None), (Source::Fork, Versions::Latest, Mode::Development));
        assert_eq!(pick(Some(Mode::Production), Some(Source::Fork), None), (Source::Fork, Versions::Released, Mode::Production));
        assert_eq!(pick(None, Some(Source::Clone), Some(Versions::Latest)), (Source::Clone, Versions::Latest, Mode::Development));
    }

    #[test]
    fn a_fork_owner_only_counts_when_forking_and_the_command_uses_the_new_flags() {
        let mut w = wire("Alpha");
        w.github_owner = Some("my-org".into());
        assert_eq!(build_request(&w).unwrap().github_owner, None, "a clone has no owner to fork into");
        w.source = Some(Source::Fork);
        let r = build_request(&w).unwrap();
        assert_eq!(r.github_owner.as_deref(), Some("my-org"));
        let cmd = install_command(&r);
        assert!(cmd.windows(2).any(|p| p == ["--source", "fork"]) && cmd.windows(2).any(|p| p == ["--versions", "latest"]), "{cmd:?}");
        let v = request_json(&r);
        assert_eq!((v["source"].as_str(), v["versions"].as_str(), v["mode"].as_str()), (Some("fork"), Some("latest"), Some("development")));
    }

    #[test]
    fn the_command_never_carries_secrets() {
        let mut w = wire("Alpha");
        w.license_key = Some("cfxk_SECRETKEYSECRETKEY".into());
        w.db_admin = Some(WireDbAdmin { host: "".into(), port: None, user: "root".into(), password: Some("hunter2".into()) });
        w.service = true;
        w.ports = Some(WirePorts { game: 30125, txadmin: 40125 });
        let r = build_request(&w).unwrap();
        let cmd = install_command(&r).join(" ");
        assert!(!cmd.contains("SECRET") && !cmd.contains("hunter2"), "{cmd}");
        assert!(cmd.contains("--service") && cmd.contains("--game-port 30125") && cmd.contains("--db-admin-user root"), "{cmd}");
        let m = elevation_message(&r, &["register the server as a system service".into()]);
        assert!(m.contains("CFX_LICENSE_KEY") && m.contains("KORA_DB_ADMIN_PASSWORD") && !m.contains("hunter2"));
    }

    #[test]
    fn requests_are_validated_like_the_wizard() {
        assert!(build_request(&wire("")).is_err());
        assert!(build_request(&wire("bad/name")).is_err());
        let mut w = wire("ok");
        w.dir = "relative/dir".into();
        assert!(build_request(&w).is_err());
        let mut w = wire("ok");
        w.ports = Some(WirePorts { game: 30120, txadmin: 30120 });
        assert!(build_request(&w).is_err());
        let mut w = wire("ok");
        w.ports = Some(WirePorts { game: 80, txadmin: 40120 });
        assert!(build_request(&w).is_err());
        let mut w = wire("ok");
        w.artifact_url = Some("https://evil.example/server.zip".into());
        assert!(build_request(&w).is_err());
        let r = build_request(&wire("Fine Name")).unwrap();
        assert_eq!(r.hostname, "Fine Name");
        assert_eq!(r.ports, Ports::default());
    }

    #[test]
    fn a_failed_install_offers_the_ways_on() {
        let mut w = wire("Alpha");
        w.firewall = true;
        let req = build_request(&w).unwrap();
        // A plain error in a step that must not be skipped: retry it, or change the answers.
        let p = failure_problem(&req, "download failed".into(), Some("build.artifact"));
        assert_eq!((p.code.as_str(), p.message.as_str()), (codes::INSTALL_STEP_FAILED, "download failed"));
        assert_eq!(p.actions, vec![Action::RetryStep { step: "build.artifact".into() }, Action::ChangeAnswers { section: None }]);
        // A step that is safe to skip also offers that.
        let p = failure_problem(&req, "no firewall".into(), Some("system.firewall"));
        assert!(p.actions.contains(&Action::SkipStep { step: "system.firewall".into() }));
        // The step's own problem keeps its code and actions first; its own "change answers" is kept.
        let own = Problem::new(codes::INSTALL_DB_LOGIN, "login refused").with_action(Action::change_answers("database"));
        let p = failure_problem(&req, own.clone(), Some("prepare.tools"));
        assert_eq!(p.code, codes::INSTALL_DB_LOGIN);
        assert_eq!(p.actions, vec![Action::change_answers("database"), Action::RetryStep { step: "prepare.tools".into() }]);
        // Stopped before the first step: try the whole install again.
        let p = failure_problem(&req, "ports taken".into(), None);
        assert_eq!((p.code.as_str(), p.actions[0].clone()), (codes::FAILED, Action::Retry));
        // A full disk points at the folder first: retrying cannot help.
        let p = failure_problem(&req, "Not enough free disk space on the disk holding /srv/a: Kora needs about 4.6 GiB.".into(), None);
        assert_eq!(p.actions[0], Action::change_answers("name"), "{:?}", p.actions);
    }

    #[test]
    fn saved_answers_json_has_no_secrets() {
        let mut w = wire("Alpha");
        w.license_key = Some("cfxk_SECRETKEYSECRETKEY".into());
        w.db_admin = Some(WireDbAdmin { host: "h".into(), port: Some(1), user: "u".into(), password: Some("pw".into()) });
        let v = request_json(&build_request(&w).unwrap()).to_string();
        assert!(!v.contains("SECRET") && !v.contains("\"pw\"") && !v.contains("\"license_key\"") && !v.contains("db_admin"), "{v}");
    }
}
