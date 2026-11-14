//! Install orchestrator. `types` is the stable contract; [`run`] executes an [`InstallRequest`]
//! as a resumable, rollback-able plan on the step engine.
//!
//! Layout of a server directory:
//! ```text
//! <dir>/resources/...        cloned resources (catalog `path`)
//! <dir>/artifacts/<build>/   FXServer build (+ state.json)
//! <dir>/txData/default/      txAdmin profile
//! <dir>/server.cfg permissions.cfg secrets.cfg database.json kora-server.json
//! <dir>/.kora/               plan-state.json, created.json (what rollback may remove), ...
//! ```
//! Steps run on a worker thread and report through a channel; the calling thread forwards events
//! to the [`ProgressSink`], so long steps (parallel clones, downloads) report live.

pub mod gitops;
pub mod host;
pub mod http;
pub mod registry;
mod steps;
pub mod system;
pub mod types;
pub mod uninstall;
pub mod windows_db;
pub mod windows_tools;

pub use types::*;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

use crate::catalog::{self, Catalog, Kind, Resource};
use crate::steps::{rollback_plan, run_plan, Step, StepError};

pub use http::HttpClient;
pub use steps::check_db_admin;
pub(crate) use steps::db_override;
pub use registry::{Registry, ServerEntry, ServerFile};

/// Installer error: plain-language message, safe to show (never contains secrets).
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct InstallError(pub String);

pub type Result<T> = std::result::Result<T, InstallError>;

pub(crate) fn err<T>(m: impl Into<String>) -> Result<T> {
    Err(InstallError(m.into()))
}

impl From<StepError> for InstallError {
    fn from(e: StepError) -> Self {
        match e {
            StepError::Failed(m) => InstallError(m),
            other => InstallError(other.to_string()),
        }
    }
}

/// Environment hooks (network client, admin override). Defaults come from the environment.
#[derive(Debug, Clone, Default)]
pub struct Hooks {
    pub http: HttpClient,
    /// Force the "running as administrator" answer (tests).
    pub assume_admin: Option<bool>,
    /// Free-space provider (tests); `None` = the real disk.
    pub disk: Option<SharedDisk>,
}

/// A shareable [`crate::space::DiskInfo`] (printable so `Hooks` stays `Debug`).
#[derive(Clone)]
pub struct SharedDisk(pub Arc<dyn crate::space::DiskInfo + Send + Sync>);

impl std::fmt::Debug for SharedDisk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SharedDisk")
    }
}

impl Hooks {
    /// `KORA_TEST_ARTIFACT_MIRROR=http://127.0.0.1:PORT` redirects artifact downloads (test seam).
    pub fn from_env() -> Self {
        let mirror = std::env::var("KORA_TEST_ARTIFACT_MIRROR").ok().filter(|s| !s.is_empty());
        Hooks {
            http: HttpClient { allow_http_localhost: mirror.is_some(), artifact_mirror: mirror },
            assume_admin: None,
            disk: None,
        }
    }

    fn disk(&self) -> &dyn crate::space::DiskInfo {
        match &self.disk {
            Some(d) => d.0.as_ref(),
            None => &crate::space::SystemDisk,
        }
    }
}

/// Are we running with administrator / root rights?
pub fn is_admin() -> bool {
    if cfg!(windows) {
        // `net session` needs the Server service (often disabled); the token's integrity level does not.
        std::process::Command::new("whoami")
            .arg("/groups")
            .output()
            .map(|o| groups_show_elevated(&String::from_utf8_lossy(&o.stdout)))
            .unwrap_or(false)
    } else {
        std::process::Command::new("id")
            .arg("-u")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "0")
            .unwrap_or(false)
    }
}

/// `whoami /groups` lists `S-1-16-12288` (High mandatory level) only for an elevated token.
pub fn groups_show_elevated(whoami_groups: &str) -> bool {
    whoami_groups.contains("S-1-16-12288") || whoami_groups.contains("S-1-16-16384")
}

/// Lowercase slug usable for service names, firewall rule names and file names.
pub fn slug(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    let s = s.trim_matches('-');
    let s: String = s.chars().take(48).collect();
    if s.is_empty() { "server".into() } else { s }
}

/// `https://github.com/Owner/name.git` -> `Owner/name`.
pub fn repo_slug(url: &str) -> String {
    url.trim_start_matches("https://github.com/").trim_end_matches(".git").to_string()
}

/// Folder/resource name used in `ensure` lines (last path segment).
pub fn resource_name(r: &Resource) -> &str {
    r.path.rsplit('/').next().unwrap_or(&r.path)
}

use catalog::normalize_id as norm_id;

/// Resolve the user's selection against the catalog, in install order. Never returns internal ones.
/// Whatever a selected resource `dependsOn` (transitively) is added automatically.
pub fn select_resources<'a>(cat: &'a Catalog, sel: &ResourceSelection) -> Result<Vec<&'a Resource>> {
    let ordered = cat.install_order();
    let seeds: Vec<String> = match sel {
        ResourceSelection::All => ordered.iter().filter(|r| matches!(r.kind, Kind::Kora | Kind::Dependency)).map(|r| r.id.clone()).collect(),
        ResourceSelection::Only(ids) => {
            for w in ids.iter().map(|i| norm_id(i)) {
                match cat.find_normalized(&w) {
                    None => return err(format!("unknown resource '{w}' (not in the catalog)")),
                    Some(r) if r.is_internal() => return err(format!("resource '{w}' is not available for public installs")),
                    Some(_) => {}
                }
            }
            // Dependencies + core (pre group) are always part of a custom selection.
            ids.iter()
                .cloned()
                .chain(ordered.iter().filter(|r| !r.is_internal() && (r.kind == Kind::Dependency || r.group == catalog::Group::Pre)).map(|r| r.id.clone()))
                .collect()
        }
    };
    let wanted = cat.with_dependencies(seeds);
    for w in &wanted {
        match cat.find_normalized(w) {
            None => return err(format!("a selected resource needs '{w}', which is not in the catalog")),
            Some(r) if r.is_internal() => return err(format!("a selected resource needs '{w}', which is not available for public installs")),
            Some(_) => {}
        }
    }
    Ok(ordered.into_iter().filter(|r| wanted.contains(&norm_id(&r.id))).collect())
}

/// Load the catalog per the request: explicit path/URL (must be valid), else default URL with
/// embedded fallback. Returns the catalog and an optional note.
pub fn load_catalog(req: &InstallRequest, hooks: &Hooks) -> Result<(Catalog, Option<String>)> {
    match req.catalog.as_deref() {
        Some(src) if src.starts_with("http://") || src.starts_with("https://") => {
            let text = catalog::CatalogSource::fetch(&hooks.http, src).map_err(|e| InstallError(format!("could not fetch catalog {src}: {e}")))?;
            Ok((Catalog::parse_validated(&text).map_err(|e| InstallError(e.to_string()))?, None))
        }
        Some(path) => {
            let text = std::fs::read_to_string(path).map_err(|e| InstallError(format!("could not read catalog {path}: {e}")))?;
            Ok((Catalog::parse_validated(&text).map_err(|e| InstallError(e.to_string()))?, None))
        }
        None => {
            let (c, origin) = catalog::load(&hooks.http, catalog::CATALOG_URL).map_err(|e| InstallError(e.to_string()))?;
            let note = match origin {
                catalog::CatalogOrigin::Remote => None,
                catalog::CatalogOrigin::Embedded(why) => Some(format!("Used the catalog bundled with this installer ({why}).")),
            };
            Ok((c, note))
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Plan description
// ---------------------------------------------------------------------------------------------

/// One planned step, in execution order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedStep {
    pub id: &'static str,
    pub title: &'static str,
    /// Relative share of the overall progress bar.
    pub weight: u8,
    pub needs_admin: bool,
}

/// Steps for a request (also the order of rollback, reversed).
pub fn plan_steps(req: &InstallRequest, _admin: bool) -> Vec<PlannedStep> {
    let mut v = vec![
        PlannedStep { id: "prepare.dir", title: "Create the server folder", weight: 2, needs_admin: false },
        PlannedStep { id: "prepare.catalog", title: "Load the resource catalog", weight: 3, needs_admin: false },
        PlannedStep {
            id: "prepare.tools",
            title: "Check git, GitHub CLI, database server and ports",
            weight: 5,
            needs_admin: false,
        },
    ];
    if req.mode == Mode::Development {
        v.push(PlannedStep { id: "build.forks", title: "Fork the resources into your GitHub account", weight: 30, needs_admin: false });
    }
    v.push(PlannedStep {
        id: "build.resources",
        title: if req.mode == Mode::Production { "Download the resources (pinned releases)" } else { "Clone your forks" },
        weight: 30,
        needs_admin: false,
    });
    if req.db != DbChoice::Skip {
        v.push(PlannedStep { id: "build.db", title: "Create a new database and user", weight: 5, needs_admin: false });
    }
    v.push(PlannedStep { id: "build.artifact", title: "Download and unpack the FiveM server build", weight: 20, needs_admin: false });
    v.push(PlannedStep { id: "build.config", title: "Write txAdmin profile, server.cfg, permissions.cfg and secrets.cfg", weight: 3, needs_admin: false });
    if req.firewall {
        v.push(PlannedStep { id: "system.firewall", title: "Open the game port in the firewall", weight: 2, needs_admin: true });
    }
    if req.service {
        v.push(PlannedStep { id: "system.service", title: "Register the background service", weight: 2, needs_admin: true });
    }
    if req.service && req.mode == Mode::Development {
        v.push(PlannedStep { id: "system.schedule", title: "Schedule daily automatic updates (04:00)", weight: 1, needs_admin: true });
    }
    v.push(PlannedStep { id: "finalize", title: "Save server settings and register the server", weight: 1, needs_admin: false });
    v
}

// ---------------------------------------------------------------------------------------------
// Shared context
// ---------------------------------------------------------------------------------------------

/// What this run created. Persisted at `<dir>/.kora/created.json`; rollback removes only this.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Created {
    pub created_dir: bool,
    /// Relative to `<dir>/resources/`.
    pub resources: Vec<String>,
    pub artifacts: bool,
    /// Relative to `<dir>`.
    pub files: Vec<String>,
    pub service_unit: Option<String>,
    /// Dev mode: the auto-update timer was registered.
    #[serde(default)]
    pub update_timer: bool,
    pub firewall: bool,
    pub registered: bool,
    /// The account the service runs as (an existing one or the one Kora created).
    #[serde(default)]
    pub service_account: Option<String>,
    /// Kora created `service_account` (so delete/uninstall may remove it). Never true for an account that already existed.
    #[serde(default)]
    pub service_account_created: bool,
    /// Owner of the server folder before it was handed to the service account (`uid:gid`, Unix).
    #[serde(default)]
    pub dir_owner: Option<String>,
    /// The root-owned copy of the kora program the service runs (shared between servers).
    #[serde(default)]
    pub binary: Option<String>,
}

#[derive(Default)]
pub(crate) struct Shared {
    pub catalog: Option<Catalog>,
    pub selected: Vec<Resource>,
    pub notes: Vec<String>,
    pub artifact_build: Option<u32>,
    pub db_name: Option<String>,
}

pub(crate) struct Ctx {
    pub req: InstallRequest,
    pub hooks: Hooks,
    pub admin: bool,
    pub plan: Vec<PlannedStep>,
    pub keep_db: bool,
    pub keep_files: bool,
    tx: Mutex<Sender<ProgressEvent>>,
    pub shared: Mutex<Shared>,
    manifest_lock: Mutex<()>,
    emitted: Mutex<HashSet<String>>,
}

impl Ctx {
    pub fn meta_dir(&self) -> PathBuf {
        meta_dir(&self.req.dir)
    }

    pub fn note(&self, n: impl Into<String>) {
        self.shared.lock().unwrap().notes.push(n.into());
    }

    /// Emit an event for `step`; `frac` (0..=1) is progress inside the step.
    pub fn emit(&self, step: &str, status: StepStatus, detail: impl Into<String>, frac: f32) {
        let (mut base, mut weight, mut title) = (0u32, 0u32, "");
        let total: u32 = self.plan.iter().map(|p| p.weight as u32).sum::<u32>().max(1);
        for p in &self.plan {
            if p.id == step {
                weight = p.weight as u32;
                title = p.title;
                break;
            }
            base += p.weight as u32;
        }
        let f = if matches!(status, StepStatus::Done | StepStatus::Skipped) { 1.0 } else { frac.clamp(0.0, 1.0) };
        let percent = (((base as f32 + weight as f32 * f) * 100.0 / total as f32) as u32).min(100) as u8;
        self.emitted.lock().unwrap().insert(step.to_string());
        let _ = self.tx.lock().unwrap().send(ProgressEvent {
            step: step.to_string(),
            title: title.to_string(),
            status,
            detail: detail.into(),
            percent,
        });
    }

    pub fn was_emitted(&self, step: &str) -> bool {
        self.emitted.lock().unwrap().contains(step)
    }

    pub fn load_created(&self) -> Created {
        std::fs::read(self.meta_dir().join("created.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// Read-modify-write the created manifest (thread safe).
    pub fn update_created(&self, f: impl FnOnce(&mut Created)) -> Result<()> {
        let _g = self.manifest_lock.lock().unwrap();
        let mut c = self.load_created();
        f(&mut c);
        std::fs::create_dir_all(self.meta_dir()).map_err(|e| InstallError(format!("cannot write install state: {e}")))?;
        let p = self.meta_dir().join("created.json");
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&c).map_err(|e| InstallError(e.to_string()))?)
            .and_then(|_| std::fs::rename(&tmp, &p))
            .map_err(|e| InstallError(format!("cannot write install state: {e}")))
    }
}

pub(crate) fn meta_dir(dir: &Path) -> PathBuf {
    dir.join(".kora")
}

/// A server's record of what Kora created (`None` when the folder has no readable record).
pub fn load_created_from(dir: &Path) -> Option<Created> {
    std::fs::read(meta_dir(dir).join("created.json")).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

/// The install request saved in a server folder (no secrets), with `dir` corrected to where it is now.
pub fn load_request_from(dir: &Path) -> Option<InstallRequest> {
    let mut r: InstallRequest = serde_json::from_slice(&std::fs::read(meta_dir(dir).join("request.json")).ok()?).ok()?;
    r.dir = dir.to_path_buf();
    Some(r)
}

pub(crate) fn save_created_to(dir: &Path, c: &Created) -> Result<()> {
    let p = meta_dir(dir).join("created.json");
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(c).map_err(|e| InstallError(e.to_string()))?)
        .and_then(|_| std::fs::rename(&tmp, &p))
        .map_err(|e| InstallError(format!("cannot write install state: {e}")))
}

fn new_ctx(req: &InstallRequest, hooks: &Hooks, admin: bool, tx: Sender<ProgressEvent>, keep_db: bool, keep_files: bool) -> Arc<Ctx> {
    Arc::new(Ctx {
        req: req.clone(),
        hooks: hooks.clone(),
        admin,
        plan: plan_steps(req, admin),
        keep_db,
        keep_files,
        tx: Mutex::new(tx),
        shared: Mutex::new(Shared::default()),
        manifest_lock: Mutex::new(()),
        emitted: Mutex::new(HashSet::new()),
    })
}

fn validate_request(req: &InstallRequest) -> Result<()> {
    if req.name.trim().is_empty() {
        return err("server name must not be empty");
    }
    if req.dir.as_os_str().is_empty() {
        return err("server folder must not be empty");
    }
    if req.ports.game < 1024 || req.ports.txadmin < 1024 || req.ports.game == req.ports.txadmin {
        return err("ports must be 1024-65535 and the game and txAdmin ports must differ");
    }
    if req.mode == Mode::Development && req.github_owner.as_deref().is_some_and(|o| o.trim().is_empty()) {
        return err("GitHub owner must not be empty");
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------------------------

/// Run (or resume) an install. `dry_run` executes nothing and emits the full plan.
pub fn run(req: &InstallRequest, sink: &mut dyn ProgressSink) -> Result<InstallOutcome> {
    run_with(req, sink, &Hooks::from_env())
}

pub fn run_with(req: &InstallRequest, sink: &mut dyn ProgressSink, hooks: &Hooks) -> Result<InstallOutcome> {
    validate_request(req)?;
    let admin = hooks.assume_admin.unwrap_or_else(is_admin);
    if req.dry_run {
        return dry_run(req, sink, hooks, admin);
    }
    if req.license_key.as_deref().map(str::trim).unwrap_or("").is_empty() {
        return err("a Cfx.re license key is required (get one at https://portal.cfx.re/servers/registration-keys)");
    }
    preflight(req, hooks, admin)?;
    // Re-check right before starting: a port can be taken since the questions were answered.
    if !req.dir.join("kora-server.json").is_file() {
        let c = crate::firewall::ports::install_conflicts(&req.name, &req.dir, req.ports.game, req.ports.txadmin, &crate::firewall::ports::SystemProber);
        if !c.is_empty() {
            return err(c.join(" "));
        }
    }
    let (tx, rx) = mpsc::channel();
    let ctx = new_ctx(req, hooks, admin, tx, false, false);
    // `ctx` (which owns the sender) moves into the worker so the channel closes when it ends.
    let handle = std::thread::spawn(move || -> Result<Vec<String>> {
        let mut list: Vec<Box<dyn Step>> = steps::build(&ctx);
        let state_path = ctx.meta_dir().join("plan-state.json");
        // The state dir must exist before the first persist; remember if the folder is ours.
        if std::fs::metadata(&ctx.req.dir).is_err() {
            std::fs::create_dir_all(&ctx.req.dir).map_err(|e| InstallError(format!("cannot create {}: {e}", ctx.req.dir.display())))?;
            ctx.update_created(|c| c.created_dir = true)?;
        }
        save_request(&ctx)?;
        run_plan(&mut list, &state_path)?;
        // Steps that were already complete never emitted anything: report them as skipped.
        for p in &ctx.plan {
            if !ctx.was_emitted(p.id) {
                ctx.emit(p.id, StepStatus::Skipped, "Already done", 1.0);
            }
        }
        let notes = std::mem::take(&mut ctx.shared.lock().unwrap().notes);
        Ok(notes)
    });
    for ev in rx {
        sink.event(ev);
    }
    let notes = handle.join().map_err(|_| InstallError("installer worker panicked".into()))??;
    let sf = ServerFile::load(&req.dir).ok();
    let build = sf.as_ref().map(|s| s.artifact_build);
    let mut notes = notes;
    notes.push(format!(
        "The server is installed but not started. {} txAdmin runs on port {} and starts the game server by itself (an admin account is already set up). FXServer is in {}.",
        if req.service { format!("Start it with `kora start {}`.", req.name) } else { format!("Start it with `kora service-host {}`.", req.name) },
        req.ports.txadmin,
        fxserver_dir_text(&req.dir, build)
    ));
    Ok(InstallOutcome {
        dir: req.dir.clone(),
        connect: format!("localhost:{}", req.ports.game),
        txadmin_url: format!("http://localhost:{}", req.ports.txadmin),
        notes,
    })
}

/// `<dir>/artifacts/<build>/` written with the platform's own separators (no `C:\\x/artifacts` mix on Windows).
pub(crate) fn fxserver_dir_text(dir: &Path, build: Option<u32>) -> String {
    let b = build.map(|b| b.to_string()).unwrap_or_else(|| "<build>".into());
    dir.join("artifacts").join(b).display().to_string()
}

/// Everything that can be checked before anything is created: free disk space (server folder,
/// artifact folder, the shared program copy a service needs) and whether the service account will
/// be able to reach the server folder. Plain-language errors; nothing has been changed yet.
pub fn preflight(req: &InstallRequest, hooks: &Hooks, admin: bool) -> Result<()> {
    let resuming = meta_dir(&req.dir).is_dir();
    let selected = match &req.resources {
        ResourceSelection::All => None,
        ResourceSelection::Only(v) => Some(v.len()),
    };
    let mut needs = crate::space::install_needs(&req.dir, crate::space::resource_estimate(selected), resuming);
    if req.service && !resuming {
        if let (Some(root), Ok(exe)) = (crate::service::binary::default_root(crate::service::Platform::current()), std::env::current_exe()) {
            let size = std::fs::metadata(exe).map(|m| m.len()).unwrap_or(0);
            needs.push(crate::space::Need::new("the kora program the background service runs", root, size + 64 * crate::space::MIB));
        }
    }
    crate::space::check(hooks.disk(), &needs).map_err(InstallError)?;
    if req.service && admin {
        if let Some(why) = crate::service::binary::unreachable_reason(&req.dir) {
            return err(format!("Cannot set up the background service: {why} Nothing was changed."));
        }
    }
    Ok(())
}

fn save_request(ctx: &Ctx) -> Result<()> {
    // Never persist the license key.
    let mut r = ctx.req.clone();
    r.license_key = None;
    r.dry_run = false;
    std::fs::create_dir_all(ctx.meta_dir()).map_err(|e| InstallError(e.to_string()))?;
    std::fs::write(
        ctx.meta_dir().join("request.json"),
        serde_json::to_vec_pretty(&r).map_err(|e| InstallError(e.to_string()))?,
    )
    .map_err(|e| InstallError(e.to_string()))
}

fn dry_run(req: &InstallRequest, sink: &mut dyn ProgressSink, hooks: &Hooks, admin: bool) -> Result<InstallOutcome> {
    let plan = plan_steps(req, admin);
    let total: u32 = plan.iter().map(|p| p.weight as u32).sum::<u32>().max(1);
    let mut notes = vec!["Dry run: nothing was changed.".to_string()];
    let (cat, cnote) = match load_catalog(req, hooks) {
        Ok(x) => x,
        Err(e) => {
            notes.push(format!("Catalog could not be loaded ({e}); resource list unavailable."));
            (Catalog { schema_version: 2, resources: vec![] }, None)
        }
    };
    notes.extend(cnote);
    let selected = select_resources(&cat, &req.resources).unwrap_or_default();
    match preflight(req, hooks, admin) {
        Ok(()) => notes.push("Free disk space and service-account access look fine.".into()),
        Err(e) => notes.push(format!("WARNING, a real install would stop here: {e}")),
    }
    if !req.dir.join("kora-server.json").is_file() {
        for c in crate::firewall::ports::install_conflicts(&req.name, &req.dir, req.ports.game, req.ports.txadmin, &crate::firewall::ports::SystemProber) {
            notes.push(format!("WARNING, a real install would stop here: {c}"));
        }
    }
    let mut base = 0u32;
    for p in &plan {
        let detail = match p.id {
            "prepare.dir" => format!("Create {}", req.dir.display()),
            "prepare.catalog" => format!("{} resources selected", selected.len()),
            "prepare.tools" => format!("Needs git{}; ports {} (game) and {} (txAdmin) must be free", if req.mode == Mode::Development { " and a logged-in `gh`" } else { "" }, req.ports.game, req.ports.txadmin),
            "build.forks" => format!(
                "Fork {} repositories into {} (needs `gh auth login`; paced in batches)",
                selected.len(),
                req.github_owner.as_deref().unwrap_or("your GitHub account")
            ),
            "build.resources" => {
                let pinned = selected.iter().filter(|r| !r.versions.is_empty()).count();
                format!("{} resources under {}/resources ({} pinned to releases, {} default-branch head), 4 at a time", selected.len(), req.dir.display(), if req.mode == Mode::Production { pinned } else { 0 }, if req.mode == Mode::Production { selected.len() - pinned } else { selected.len() })
            }
            "build.db" => match req.db {
                DbChoice::InstallIfMissing => "Install MariaDB if missing (admin needed), then create a new database + user with a generated password".into(),
                _ => "Create a new database + user with a generated password (never touches existing data)".into(),
            },
            "build.artifact" => match &req.artifact_url {
                Some(u) => format!("Download {u}"),
                None => "Download the recommended FXServer build".into(),
            },
            "build.config" => "Write txData/default profile, server.cfg, permissions.cfg, secrets.cfg (0600)".into(),
            "system.firewall" => format!("Open {}/tcp+udp (admin needed)", req.ports.game),
            "system.service" => "Create + enable the background service (admin needed)".into(),
            "finalize" => format!("Write kora-server.json and register in {}", registry::registry_path().display()),
            _ => String::new(),
        };
        let marker = if p.needs_admin { " [needs admin]" } else { "" };
        let status = if p.needs_admin && !admin { StepStatus::Skipped } else { StepStatus::Pending };
        sink.event(ProgressEvent {
            step: p.id.to_string(),
            title: format!("{}{marker}", p.title),
            status,
            detail: if p.needs_admin && !admin { format!("{detail}; would be skipped without admin rights and returned as a note") } else { detail },
            percent: ((base * 100) / total).min(100) as u8,
        });
        base += p.weight as u32;
    }
    sink.event(ProgressEvent { step: "plan".into(), title: "Plan complete".into(), status: StepStatus::Done, detail: "dry run".into(), percent: 100 });
    if req.db == DbChoice::InstallIfMissing {
        notes.push("Installing MariaDB would need admin rights (only if none is found).".into());
    }
    Ok(InstallOutcome {
        dir: req.dir.clone(),
        connect: format!("localhost:{}", req.ports.game),
        txadmin_url: format!("http://localhost:{}", req.ports.txadmin),
        notes,
    })
}

/// What `rollback` should remove.
#[derive(Debug, Clone, Copy, Default)]
pub struct RollbackOptions {
    /// Leave the server folder and its files alone.
    pub keep_files: bool,
    /// Leave the database and user alone.
    pub keep_db: bool,
}

/// Roll back an install (or delete a server): removes only what this install created.
/// GitHub forks are never deleted here.
pub fn rollback(dir: &Path, opts: &RollbackOptions, sink: &mut dyn ProgressSink) -> Result<Vec<String>> {
    rollback_with(dir, opts, sink, &Hooks::from_env())
}

pub fn rollback_with(dir: &Path, opts: &RollbackOptions, sink: &mut dyn ProgressSink, hooks: &Hooks) -> Result<Vec<String>> {
    let bytes = std::fs::read(meta_dir(dir).join("request.json"))
        .map_err(|_| InstallError(format!("{} has no Kora install record (.kora/request.json): nothing to roll back", dir.display())))?;
    let mut req: InstallRequest = serde_json::from_slice(&bytes).map_err(|e| InstallError(format!("corrupt install record: {e}")))?;
    req.dir = dir.to_path_buf();
    let admin = hooks.assume_admin.unwrap_or_else(is_admin);
    let (tx, rx) = mpsc::channel();
    let ctx = new_ctx(&req, hooks, admin, tx, opts.keep_db, opts.keep_files);
    let handle = std::thread::spawn(move || -> Result<Vec<String>> {
        let mut list = steps::build(&ctx);
        let state_path = ctx.meta_dir().join("plan-state.json");
        rollback_plan(&mut list, &state_path)?;
        steps::finish_rollback(&ctx)?;
        Ok(std::mem::take(&mut ctx.shared.lock().unwrap().notes))
    });
    for ev in rx {
        sink.event(ev);
    }
    handle.join().map_err(|_| InstallError("rollback worker panicked".into()))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(dir: &Path) -> InstallRequest {
        InstallRequest {
            mode: Mode::Production,
            name: "My Server".into(),
            dir: dir.to_path_buf(),
            hostname: "My Server".into(),
            license_key: None,
            cfx_identifier: None,
            ports: Ports::default(),
            txadmin_public: false,
            artifact_url: None,
            game_build: None,
            resources: ResourceSelection::All,
            db: DbChoice::UseExisting,
            service: true,
            firewall: true,
            github_owner: None,
            catalog: None,
            dry_run: true,
            db_admin: None,
        }
    }

    struct Collect(Vec<ProgressEvent>);
    impl ProgressSink for Collect {
        fn event(&mut self, ev: ProgressEvent) {
            self.0.push(ev);
        }
    }

    #[test]
    fn saved_request_has_no_secrets_and_old_records_load() {
        let mut r = req(Path::new("/tmp/x"));
        r.license_key = Some("cfxk_SECRETSECRETSECRET".into());
        r.db_admin = Some(DbAdmin { host: "h".into(), port: 1, user: "u".into(), password: Some("adminpw-secret".into()) });
        assert!(!format!("{r:?}").contains("adminpw-secret"));
        let json = serde_json::to_string(&r).unwrap();
        assert!(!json.contains("adminpw-secret"));
        // Records written before db_admin existed still load.
        let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
        v.as_object_mut().unwrap().remove("db_admin");
        let old: InstallRequest = serde_json::from_value(v).unwrap();
        assert!(old.db_admin.is_none());
    }

    #[test]
    fn slug_is_safe() {
        assert_eq!(slug("My Server!"), "my-server");
        assert_eq!(slug("!!!"), "server");
        assert_eq!(repo_slug("https://github.com/KoraFiveM/-cfx-default-.git"), "KoraFiveM/-cfx-default-");
    }

    #[test]
    fn selection_never_includes_internal_and_always_core_and_deps() {
        let cat = Catalog::embedded().unwrap();
        let all = select_resources(&cat, &ResourceSelection::All).unwrap();
        assert!(all.iter().all(|r| !r.is_internal()));
        assert!(all.iter().any(|r| r.id == "kora-core"));
        let only = select_resources(&cat, &ResourceSelection::Only(vec!["kora_chat".into()])).unwrap();
        assert!(only.iter().any(|r| r.id == "kora-chat"), "underscores normalise to dashes");
        assert!(only.iter().any(|r| r.id == "kora-core"), "core group always included");
        assert!(only.iter().any(|r| r.kind == Kind::Dependency));
        assert!(!only.iter().any(|r| r.id == "kora-hud"));
        assert!(select_resources(&cat, &ResourceSelection::Only(vec!["nope".into()])).is_err());
        assert!(select_resources(&cat, &ResourceSelection::Only(vec!["kora-scrapyard".into()])).is_err(), "internal is refused");
        // install order is respected
        let orders: Vec<u32> = all.iter().filter_map(|r| r.order).collect();
        assert!(orders.windows(2).all(|w| w[0] < w[1]));
    }

    fn tiny_catalog(extra: &str) -> Catalog {
        let sha = "a".repeat(40);
        let rel = format!(r#""latest":"1.0.0","versions":[{{"version":"1.0.0","tag":"v1.0.0","commit":"{sha}"}}]"#);
        let json = format!(
            r#"{{"schemaVersion":2,"resources":[
              {{"id":"kora-lib","repository":"https://github.com/o/lib.git","path":"[pre]/lib","group":"pre","kind":"kora","branch":"dev","order":1,{rel},"dependsOn":["kora_theme"]}},
              {{"id":"kora-theme","repository":"https://github.com/o/theme.git","path":"[standalone]/theme","group":"standalone","kind":"optional","branch":"dev","order":2,{rel},"dependsOn":["kora-icons"]}},
              {{"id":"kora-icons","repository":"https://github.com/o/icons.git","path":"[standalone]/icons","group":"standalone","kind":"optional","branch":"dev","order":3,{rel}}},
              {{"id":"kora-chat","repository":"https://github.com/o/chat.git","path":"[kora]/chat","group":"kora","kind":"optional","branch":"dev","order":4,{rel}}}
              {extra}]}}"#
        );
        Catalog::parse_validated(&json).unwrap()
    }

    #[test]
    fn dependencies_are_added_automatically_and_transitively() {
        let cat = tiny_catalog("");
        let ids = |s: &ResourceSelection| select_resources(&cat, s).unwrap().iter().map(|r| r.id.clone()).collect::<Vec<_>>();
        // core (pre) is always there; its dependsOn chain (theme -> icons) comes with it; kora-chat does not.
        assert_eq!(ids(&ResourceSelection::Only(vec![])), ["kora-lib", "kora-theme", "kora-icons"]);
        // choosing a dependency's dependent pulls it in, in install order, underscores accepted
        assert_eq!(ids(&ResourceSelection::Only(vec!["kora_chat".into()])), ["kora-lib", "kora-theme", "kora-icons", "kora-chat"]);
        assert_eq!(ids(&ResourceSelection::All).len(), 3, "All = kora|dependency kinds only; optional ones are not offered");
    }

    #[test]
    fn all_selection_includes_optional_dependencies_of_selected_resources() {
        let cat = tiny_catalog(
            r#",{"id":"kora-x","repository":"https://github.com/o/x.git","path":"[kora]/x","group":"kora","kind":"kora","branch":"dev","order":5,"latest":"1.0.0","versions":[{"version":"1.0.0","tag":"v1.0.0","commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}],"dependsOn":["kora-chat"]}"#,
        );
        let ids: Vec<String> = select_resources(&cat, &ResourceSelection::All).unwrap().iter().map(|r| r.id.clone()).collect();
        assert!(ids.contains(&"kora-chat".to_string()), "optional chat is needed by kora-x: {ids:?}");
    }

    #[test]
    fn bad_dependencies_are_rejected_by_validation() {
        let mut cat = tiny_catalog("");
        cat.resources[3].depends_on = vec!["nope".into()];
        cat.resources[0].depends_on = vec!["kora-lib".into()];
        let catalog::CatalogError::Invalid(e) = cat.validate().unwrap_err() else { panic!() };
        assert!(e.iter().any(|m| m.contains("'nope' is not in the catalog")), "{e:?}");
        assert!(e.iter().any(|m| m.contains("depends on itself")), "{e:?}");
        // cycles are tolerated by selection
        let mut cat = tiny_catalog("");
        cat.resources[2].depends_on = vec!["kora-lib".into()];
        assert_eq!(select_resources(&cat, &ResourceSelection::Only(vec![])).unwrap().len(), 3);
    }

    #[test]
    fn embedded_catalog_pulls_theme_in_with_lib_and_old_catalogs_still_parse() {
        let cat = Catalog::embedded().unwrap();
        assert!(cat.find("kora-theme").is_some());
        assert_eq!(cat.find("kora-lib").unwrap().depends_on, ["kora-theme"]);
        let only = select_resources(&cat, &ResourceSelection::Only(vec!["kora_chat".into()])).unwrap();
        assert!(only.iter().any(|r| r.id == "kora-theme"), "kora-lib is always installed, so is its dependency");
        let mut v: serde_json::Value = serde_json::from_str(catalog::EMBEDDED_SNAPSHOT).unwrap();
        for r in v["resources"].as_array_mut().unwrap() {
            r.as_object_mut().unwrap().remove("dependsOn");
        }
        assert!(Catalog::parse_validated(&v.to_string()).is_ok(), "dependsOn is optional");
    }

    #[test]
    fn dry_run_executes_nothing_and_marks_admin_steps() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("srv");
        let mut r = req(&dir);
        r.catalog = Some("/nonexistent/catalog.json".into()); // dry run tolerates it
        let mut sink = Collect(vec![]);
        let hooks = Hooks { assume_admin: Some(false), ..Hooks::default() };
        let out = run_with(&r, &mut sink, &hooks).unwrap();
        assert!(!dir.exists(), "dry run must not create the folder");
        assert!(out.notes.iter().any(|n| n.contains("nothing was changed")));
        let admin: Vec<&ProgressEvent> = sink.0.iter().filter(|e| e.title.contains("[needs admin]")).collect();
        assert_eq!(admin.len(), 2, "firewall + service");
        assert!(admin.iter().all(|e| e.status == StepStatus::Skipped));
        assert!(sink.0.iter().any(|e| e.step == "build.resources"));
        assert_eq!(sink.0.last().unwrap().percent, 100);
        let pct: Vec<u8> = sink.0.iter().map(|e| e.percent).collect();
        assert!(pct.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn dev_plan_has_forks_and_skip_db_drops_db_step() {
        let mut r = req(Path::new("/x"));
        r.mode = Mode::Development;
        r.db = DbChoice::Skip;
        r.service = false;
        r.firewall = false;
        let ids: Vec<&str> = plan_steps(&r, true).iter().map(|p| p.id).collect();
        assert!(ids.contains(&"build.forks") && !ids.contains(&"build.db") && !ids.contains(&"system.service"));
    }

    #[test]
    fn dev_service_plan_schedules_updates_prod_does_not() {
        let mut r = req(Path::new("/x"));
        r.service = true;
        r.mode = Mode::Development;
        let ids: Vec<&str> = plan_steps(&r, true).iter().map(|p| p.id).collect();
        let (svc, sch) = (ids.iter().position(|i| *i == "system.service"), ids.iter().position(|i| *i == "system.schedule"));
        assert!(svc.is_some() && sch > svc);
        r.mode = Mode::Production;
        assert!(!plan_steps(&r, true).iter().any(|p| p.id == "system.schedule"));
    }

    struct TinyDisk(u64);
    impl crate::space::DiskInfo for TinyDisk {
        fn free_bytes(&self, _: &Path) -> std::io::Result<u64> {
            Ok(self.0)
        }
        fn volume_key(&self, _: &Path) -> String {
            "one".into()
        }
    }

    fn hooks_with_disk(free: u64) -> Hooks {
        Hooks { assume_admin: Some(false), disk: Some(SharedDisk(Arc::new(TinyDisk(free)))), ..Hooks::default() }
    }

    #[test]
    fn a_too_small_disk_stops_the_install_before_anything_is_created() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("srv");
        let mut r = req(&dir);
        r.dry_run = false;
        r.license_key = Some("cfxk_x".into());
        r.resources = ResourceSelection::Only(vec!["kora_chat".into()]);
        let e = run_with(&r, &mut Collect(vec![]), &hooks_with_disk(crate::space::GIB)).unwrap_err().to_string();
        assert!(e.contains("Not enough free disk space") && e.contains("only 1.0 GiB is free"), "{e}");
        assert!(e.contains(&*dir.to_string_lossy()) && e.contains("--dir"), "{e}");
        assert!(!dir.exists(), "nothing may be created");
        // The estimate follows the selection: 1 chosen + 10 = 11 resources.
        assert!(e.contains("11 resources"), "{e}");
    }

    #[test]
    fn enough_disk_passes_the_preflight_and_a_resumed_install_needs_only_working_room() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("srv");
        let mut r = req(&dir);
        r.service = false;
        assert!(preflight(&r, &hooks_with_disk(100 * crate::space::GIB), false).is_ok());
        assert!(preflight(&r, &hooks_with_disk(3 * crate::space::GIB), false).is_err());
        std::fs::create_dir_all(dir.join(".kora")).unwrap();
        assert!(preflight(&r, &hooks_with_disk(3 * crate::space::GIB), false).is_ok(), "resume: 2 GiB working room only");
        assert!(preflight(&r, &hooks_with_disk(crate::space::GIB), false).is_err());
    }

    #[test]
    fn dry_run_reports_a_disk_problem_as_a_warning_instead_of_failing() {
        let t = tempfile::tempdir().unwrap();
        let mut r = req(&t.path().join("srv"));
        r.catalog = Some("/nonexistent".into());
        let out = run_with(&r, &mut Collect(vec![]), &hooks_with_disk(1)).unwrap();
        assert!(out.notes.iter().any(|n| n.contains("a real install would stop here") && n.contains("Not enough free disk space")), "{:?}", out.notes);
    }

    #[cfg(unix)]
    #[test]
    fn a_private_parent_folder_is_refused_for_service_installs_before_anything_is_created() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let private = t.path().join("home-me");
        std::fs::create_dir(&private).unwrap();
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o700)).unwrap();
        let dir = private.join("kora/srv");
        let mut r = req(&dir);
        r.dry_run = false;
        r.license_key = Some("cfxk_x".into());
        let hooks = Hooks { assume_admin: Some(true), disk: Some(SharedDisk(Arc::new(TinyDisk(u64::MAX)))), ..Hooks::default() };
        let e = run_with(&r, &mut Collect(vec![]), &hooks).unwrap_err().to_string();
        assert!(e.contains("cannot enter") && e.contains("Nothing was changed"), "{e}");
        assert!(!dir.exists());
    }

    #[test]
    fn real_run_requires_license_key_and_valid_ports() {
        let t = tempfile::tempdir().unwrap();
        let mut r = req(&t.path().join("srv"));
        r.dry_run = false;
        let e = run_with(&r, &mut Collect(vec![]), &Hooks::default()).unwrap_err();
        assert!(e.to_string().contains("license key"));
        assert!(!t.path().join("srv").exists(), "nothing created before validation");
        r.ports.txadmin = r.ports.game;
        assert!(run_with(&r, &mut Collect(vec![]), &Hooks::default()).is_err());
    }

    #[test]
    fn fxserver_note_uses_one_kind_of_separator() {
        let d = Path::new("srv").join("main");
        let t = fxserver_dir_text(&d, Some(38621));
        assert_eq!(t, Path::new("srv").join("main").join("artifacts").join("38621").display().to_string());
        assert!(!(t.contains('/') && t.contains('\\')), "mixed separators: {t}");
        assert!(fxserver_dir_text(&d, None).ends_with("<build>"));
    }

    #[test]
    fn elevation_is_read_from_the_high_integrity_group() {
        let elevated = "Mandatory Label\\High Mandatory Level   Label   S-1-16-12288   Mandatory group";
        let normal = "Mandatory Label\\Medium Mandatory Level   Label   S-1-16-8192   Mandatory group\nBUILTIN\\Administrators  Alias  S-1-5-32-544  Group used for deny only";
        assert!(groups_show_elevated(elevated));
        assert!(!groups_show_elevated(normal), "a non-elevated administrator is not elevated");
        assert!(!groups_show_elevated(""));
    }
}
