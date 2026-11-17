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

pub mod change_ports;
pub mod gitops;
pub mod host;
pub mod http;
pub mod layout;
pub mod paused;
pub mod pinned;
pub mod registry;
pub mod saved;
mod steps;
pub mod system;
pub mod system_ops;
pub mod types;
pub mod uninstall;
pub mod windows_db;

pub use types::*;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

use crate::catalog::{self, Catalog, Kind, Resource};
use crate::problem::{codes, Action, Problem};
use crate::steps::{rollback_plan, run_plan, Step, StepError};

pub use http::HttpClient;
pub use registry::{Registry, ServerEntry, ServerFile};
pub use steps::check_db_admin;
pub(crate) use steps::db_override;
pub(crate) use steps::db_client;

/// Installer error: plain-language message, safe to show (never contains secrets).
///
/// The string is either plain text or a packed [`Problem`] (`InstallError::from(problem)`), so a
/// step can say what the user can do about it. `Display` always gives readable text (for the CLI
/// and logs); [`InstallError::problem`] gives the structured form (for the panel).
#[derive(Debug)]
pub struct InstallError(pub String);

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&crate::problem::plain_text(&self.0))
    }
}

impl std::error::Error for InstallError {}

impl InstallError {
    /// The structured form (a generic problem for plain text).
    pub fn problem(&self) -> Problem {
        Problem::decode(&self.0)
    }
}

impl From<Problem> for InstallError {
    fn from(p: Problem) -> Self {
        InstallError(p.encode())
    }
}

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
    /// `--fix-permissions`: when the service account cannot get into the server folder because a
    /// parent folder is private, `chmod o+x` exactly those folders instead of stopping.
    pub fix_permissions: bool,
    /// Not running as administrator, but these steps may be done by an elevated helper (the
    /// panel's Windows worker, see [`system_ops`]). `None`: they are skipped with a note, as before.
    pub admin: Option<system_ops::SharedAdmin>,
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
        Hooks { http: HttpClient { allow_http_localhost: mirror.is_some(), artifact_mirror: mirror }, assume_admin: None, disk: None, fix_permissions: false, admin: None }
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
        crate::sysexe::command("whoami").arg("/groups").output().map(|o| groups_show_elevated(&String::from_utf8_lossy(&o.stdout))).unwrap_or(false)
    } else {
        std::process::Command::new("id").arg("-u").output().map(|o| String::from_utf8_lossy(&o.stdout).trim() == "0").unwrap_or(false)
    }
}

/// `whoami /groups` lists `S-1-16-12288` (High mandatory level) only for an elevated token.
pub fn groups_show_elevated(whoami_groups: &str) -> bool {
    whoami_groups.contains("S-1-16-12288") || whoami_groups.contains("S-1-16-16384")
}

/// Lowercase slug usable for service names, firewall rule names and file names.
pub fn slug(name: &str) -> String {
    let mut s: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    let s = s.trim_matches('-');
    let s: String = s.chars().take(48).collect();
    if s.is_empty() {
        "server".into()
    } else {
        s
    }
}

/// `https://github.com/Owner/name.git` -> `Owner/name`.
pub fn repo_slug(url: &str) -> String {
    url.trim_start_matches("https://github.com/").trim_end_matches(".git").to_string()
}

/// Folder/resource name of a resource (last path segment of where it is installed).
pub fn resource_name(r: &Resource) -> &str {
    let p = r.path.as_str();
    p.rsplit('/').next().unwrap_or(p)
}

use catalog::normalize_id as norm_id;

/// Resolve the user's selection against the catalog, in install order. Never returns internal ones,
/// and never development-only ones unless `dev` (a development server). Whatever a selected resource
/// `dependsOn` (transitively) is added automatically.
pub fn select_resources<'a>(cat: &'a Catalog, sel: &ResourceSelection, dev: bool) -> Result<Vec<&'a Resource>> {
    let ordered = cat.install_order();
    let seeds: Vec<String> = match sel {
        // Compat / bridge / creator resources are never selected by default (only through dependencies),
        // and development-only ones are never selected by default either.
        ResourceSelection::All => ordered.iter().filter(|r| r.res_type.is_none() && !r.dev_only && matches!(r.kind, Kind::Kora | Kind::Dependency)).map(|r| r.id.clone()).collect(),
        ResourceSelection::Only(ids) => {
            for w in ids.iter().map(|i| norm_id(i)) {
                match cat.find_normalized(&w) {
                    None => return err(format!("unknown resource '{w}' (not in the catalog)")),
                    Some(r) if r.is_internal() => return err(format!("resource '{w}' is not available for public installs")),
                    Some(r) if !r.offered_to(dev) => return err(format!("resource '{w}' is only available for development servers")),
                    Some(_) => {}
                }
            }
            // Dependencies + core (pre group) are always part of a custom selection.
            ids.iter()
                .cloned()
                .chain(ordered.iter().filter(|r| r.offered_to(dev) && r.res_type.is_none() && (r.kind == Kind::Dependency || r.group == catalog::Group::Pre)).map(|r| r.id.clone()))
                .collect()
        }
    };
    let wanted = cat.with_dependencies(seeds);
    for w in &wanted {
        match cat.find_normalized(w) {
            None => return err(format!("a selected resource needs '{w}', which is not in the catalog")),
            Some(r) if r.is_internal() => return err(format!("a selected resource needs '{w}', which is not available for public installs")),
            Some(r) if !r.offered_to(dev) => return err(format!("a selected resource needs '{w}', which is only available for development servers")),
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
    /// The user may skip this step after it failed ([`skip_step`]) and the install still gives a
    /// working server. Only steps whose result is optional or can be done later; see [`SKIPPABLE`].
    pub skippable: bool,
}

/// Steps that may be skipped after they failed. Deliberately few: everything the server needs to
/// run (folder, catalogue, tools, resources, database, FXServer, configuration, registration) and
/// the background service (its partial setup is easier to finish than to leave) are not.
/// * `system.firewall`: the server works; the port can be opened by hand (the notes say how).
/// * `system.schedule`: only the daily automatic update; `kora schedule set` adds it later.
/// * `launch`: starting the server; it can be started from the panel at any time.
pub const SKIPPABLE: &[&str] = &["system.firewall", "system.schedule", "launch"];

/// Steps for a request (also the order of rollback, reversed).
pub fn plan_steps(req: &InstallRequest, _admin: bool) -> Vec<PlannedStep> {
    plan_steps_for(req, crate::userhome::kora_user_planned(&req.dir))
}

/// [`plan_steps`] with the "needs the `kora` account" decision given (tests).
pub fn plan_steps_for(req: &InstallRequest, kora_user: bool) -> Vec<PlannedStep> {
    let mut v = Vec::new();
    if kora_user {
        v.push(PlannedStep { id: "prepare.account", title: "Create the 'kora' user account that owns the server folder", weight: 1, needs_admin: true, skippable: false });
    }
    v.extend([
        PlannedStep { id: "prepare.dir", title: "Create the server folder", weight: 2, needs_admin: false, skippable: false },
        PlannedStep { id: "prepare.catalog", title: "Load the resource catalog", weight: 3, needs_admin: false, skippable: false },
        PlannedStep { id: "prepare.tools", title: "Check git, GitHub CLI, database server and ports", weight: 5, needs_admin: false, skippable: false },
    ]);
    if req.mode == Mode::Development {
        v.push(PlannedStep { id: "build.forks", title: "Fork the resources into your GitHub account", weight: 30, needs_admin: false, skippable: false });
    }
    v.push(PlannedStep {
        id: "build.resources",
        title: if req.mode == Mode::Production { "Download the resources (pinned releases)" } else { "Clone your forks" },
        weight: 30,
        needs_admin: false,
        skippable: false,
    });
    if req.db != DbChoice::Skip {
        v.push(PlannedStep { id: "build.db", title: "Create a new database and user", weight: 5, needs_admin: false, skippable: false });
    }
    v.push(PlannedStep { id: "build.artifact", title: "Download and unpack the FiveM server build", weight: 20, needs_admin: false, skippable: false });
    v.push(PlannedStep { id: "build.config", title: "Write txAdmin profile, server.cfg, permissions.cfg and secrets.cfg", weight: 3, needs_admin: false, skippable: false });
    if req.firewall {
        v.push(PlannedStep { id: "system.firewall", title: "Open the game port in the firewall", weight: 2, needs_admin: true, skippable: true });
    }
    if req.service {
        v.push(PlannedStep { id: "system.service", title: "Register the background service", weight: 2, needs_admin: true, skippable: false });
    }
    if req.service && req.mode == Mode::Development {
        v.push(PlannedStep { id: "system.schedule", title: "Schedule daily automatic updates (04:00)", weight: 1, needs_admin: true, skippable: true });
    }
    v.push(PlannedStep { id: "finalize", title: "Save server settings and register the server", weight: 1, needs_admin: false, skippable: false });
    if req.start {
        v.push(PlannedStep { id: "launch", title: "Start your server", weight: 4, needs_admin: false, skippable: true });
    }
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
    pub launch: Option<crate::launch::Report>,
}

pub(crate) struct Ctx {
    pub req: InstallRequest,
    pub hooks: Hooks,
    pub admin: bool,
    pub plan: Vec<PlannedStep>,
    pub keep_db: bool,
    pub keep_files: bool,
    /// `false` when deleting a folder that is not in the server list: its name and ports then come
    /// only from its own records, which the server's account can change, so the root-only system
    /// items (service, schedule, account, firewall) are left alone.
    pub system_trusted: bool,
    tx: Mutex<Sender<ProgressEvent>>,
    pub shared: Mutex<Shared>,
    manifest_lock: Mutex<()>,
    emitted: Mutex<HashSet<String>>,
    /// In the elevated helper's child (`kora admin-step`): the server folder's record lives here
    /// instead of on disk, and is handed back to the person's own process, which writes it (so
    /// every file in the server folder stays theirs). `None`: the normal, on-disk record.
    pub shadow: Option<Mutex<Created>>,
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
        let _ = self.tx.lock().unwrap().send(ProgressEvent { step: step.to_string(), title: title.to_string(), status, detail: detail.into(), percent });
    }

    pub fn was_emitted(&self, step: &str) -> bool {
        self.emitted.lock().unwrap().contains(step)
    }

    pub fn load_created(&self) -> Created {
        if let Some(s) = &self.shadow {
            return s.lock().unwrap().clone();
        }
        std::fs::read(self.meta_dir().join("created.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    /// Read-modify-write the created manifest (thread safe).
    pub fn update_created(&self, f: impl FnOnce(&mut Created)) -> Result<()> {
        if let Some(s) = &self.shadow {
            f(&mut s.lock().unwrap());
            return Ok(());
        }
        let _g = self.manifest_lock.lock().unwrap();
        let mut c = self.load_created();
        f(&mut c);
        save_created_to(&self.req.dir, &c)
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

/// Root writes this into a folder other accounts can change, so never through a planted link
/// (a fresh temporary file renamed into place; see `fsutil::write_no_follow`).
pub(crate) fn save_created_to(dir: &Path, c: &Created) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(c).map_err(|e| InstallError(e.to_string()))?;
    crate::fsutil::write_no_follow(&meta_dir(dir).join("created.json"), &bytes, 0o644).map_err(|e| InstallError(format!("cannot write install state: {e}")))
}

fn new_ctx(req: &InstallRequest, hooks: &Hooks, admin: bool, tx: Sender<ProgressEvent>, keep_db: bool, keep_files: bool) -> Arc<Ctx> {
    new_ctx_trusted(req, hooks, admin, tx, keep_db, keep_files, true)
}

fn new_ctx_trusted(req: &InstallRequest, hooks: &Hooks, admin: bool, tx: Sender<ProgressEvent>, keep_db: bool, keep_files: bool, system_trusted: bool) -> Arc<Ctx> {
    Arc::new(Ctx {
        req: req.clone(),
        hooks: hooks.clone(),
        admin,
        plan: plan_steps(req, admin),
        keep_db,
        keep_files,
        system_trusted,
        tx: Mutex::new(tx),
        shared: Mutex::new(Shared::default()),
        manifest_lock: Mutex::new(()),
        emitted: Mutex::new(HashSet::new()),
        shadow: None,
    })
}

pub(crate) fn validate_request(req: &InstallRequest) -> Result<()> {
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

/// Where a Cfx.re licence key is made.
pub const LICENCE_URL: &str = "https://portal.cfx.re/servers/registration-keys";

/// A port was taken after the questions were answered: pick other ports.
pub(crate) fn ports_busy(conflicts: &[String]) -> Problem {
    Problem::new(codes::INSTALL_PORTS_BUSY, conflicts.join(" ")).with_action(Action::change_answers("ports"))
}

/// The user chose to go on without step `id` after it failed (the panel's "Skip this step"). The
/// next run of the install passes over it. Refused for steps that are not [`SKIPPABLE`] or not in
/// this install's plan. Never touches anything but the install's own `plan-state.json`.
pub fn skip_step(req: &InstallRequest, id: &str) -> std::result::Result<(), Problem> {
    let plan = plan_steps(req, false);
    let Some(step) = plan.iter().find(|p| p.id == id) else {
        return Err(Problem::new(codes::INSTALL_CANNOT_RESUME, format!("This install has no step called '{id}'.")));
    };
    if !step.skippable {
        return Err(Problem::new(codes::INSTALL_CANNOT_RESUME, format!("\"{}\" cannot be skipped: your server needs it. Try it again, or change your answers.", step.title))
            .with_action(Action::RetryStep { step: id.to_string() })
            .with_action(Action::ChangeAnswers { section: None }));
    }
    crate::steps::mark_skipped(&meta_dir(&req.dir).join("plan-state.json"), id)
        .map(|_| ())
        .map_err(|e| Problem::generic("Kora could not save your choice to skip this step.").with_details(e.to_string()))
}

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
        return Err(Problem::new(codes::INSTALL_LICENCE_MISSING, "a Cfx.re license key is required (get one at https://portal.cfx.re/servers/registration-keys)")
            .with_action(Action::change_answers("licence"))
            .with_action(Action::open_link(LICENCE_URL))
            .into());
    }
    check_dir_links(&req.dir)?;
    preflight(req, hooks, admin)?;
    // Re-check right before starting: a port can be taken since the questions were answered.
    if !req.dir.join("kora-server.json").is_file() {
        let c = crate::firewall::ports::install_conflicts(&req.name, &req.dir, req.ports.game, req.ports.txadmin, &crate::firewall::ports::SystemProber);
        if !c.is_empty() {
            return Err(ports_busy(&c).into());
        }
    }
    let (tx, rx) = mpsc::channel();
    let ctx = new_ctx(req, hooks, admin, tx, false, false);
    // `ctx` (which owns the sender) moves into the worker so the channel closes when it ends.
    let handle = std::thread::spawn(move || -> Result<(Vec<String>, Option<crate::launch::Report>)> {
        let mut list: Vec<Box<dyn Step>> = steps::build(&ctx);
        let state_path = ctx.meta_dir().join("plan-state.json");
        // The `kora` account must exist before its home folder (which holds the server folder) is made.
        let kora_user = ctx.plan.iter().any(|p| p.id == "prepare.account");
        if kora_user {
            steps::prepare_account(&ctx, &crate::service::SystemRunner)?;
        }
        // The state dir must exist before the first persist; remember if the folder is ours.
        if std::fs::metadata(&ctx.req.dir).is_err() {
            crate::fsutil::create_dir_no_follow(&ctx.req.dir).map_err(|e| InstallError(format!("cannot create {}: {e}", ctx.req.dir.display())))?;
            ctx.update_created(|c| c.created_dir = true)?;
        }
        // Again now that it exists: a part could have been swapped for a link meanwhile.
        check_dir_links(&ctx.req.dir)?;
        if kora_user {
            // Folders made just now belong to `kora` (and so does the folder a deleted service gives back).
            let plan = crate::userhome::chown_to_kora_plan(&crate::userhome::kora_owned_paths(&ctx.req.dir), false);
            crate::service::apply_plan(&plan, &crate::service::SystemRunner).map_err(|e| InstallError(e.to_string()))?;
        }
        save_request(&ctx)?;
        let state = run_plan(&mut list, &state_path)?;
        // Steps that were already complete (or that the user chose to skip) never emitted anything: report them as skipped.
        for p in &ctx.plan {
            if !ctx.was_emitted(p.id) {
                let why = if state.status(p.id) == Some(crate::steps::StepStatus::Skipped) { "Skipped: you chose to go on without it" } else { "Already done" };
                ctx.emit(p.id, StepStatus::Skipped, why, 1.0);
            }
        }
        let mut sh = ctx.shared.lock().unwrap();
        Ok((std::mem::take(&mut sh.notes), sh.launch.take()))
    });
    for ev in rx {
        sink.event(ev);
    }
    let (notes, launch) = handle.join().map_err(|_| InstallError("installer worker panicked".into()))??;
    let sf = ServerFile::load(&req.dir).ok();
    let build = sf.as_ref().map(|s| s.artifact_build);
    let mut notes = notes;
    if !req.start {
        notes.push(format!(
            "The server is installed but not started. Start it with `kora start {}`. txAdmin runs on port {} and starts the game server by itself (an admin account is already set up). FXServer is in {}.",
            req.name,
            req.ports.txadmin,
            fxserver_dir_text(&req.dir, build)
        ));
    }
    let pw_file = req.dir.join("secrets").join("txadmin-admin.txt");
    // The file only exists when no Cfx identity was linked (a generated local admin).
    let local_admin = pw_file.is_file();
    Ok(InstallOutcome {
        dir: req.dir.clone(),
        connect: crate::launch::connect_address(req.ports.game),
        txadmin_url: format!("http://localhost:{}", req.ports.txadmin),
        notes,
        name: req.name.clone(),
        start_attempted: launch.is_some(),
        running: launch.as_ref().is_some_and(|l| l.running()),
        txadmin_up: launch.as_ref().is_some_and(|l| l.txadmin_up),
        game_listening: launch.as_ref().is_some_and(|l| l.game_listening),
        launch_problem: launch.as_ref().and_then(|l| l.problem.map(|_| l.message.clone())),
        log_tail: launch.as_ref().map(|l| l.log_tail.clone()).unwrap_or_default(),
        admin_user: local_admin.then(|| crate::txadmin::LOCAL_ADMIN_USER.to_string()),
        admin_password_file: local_admin.then_some(pw_file),
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
        let blocked = service_blocking_dirs(&req.dir);
        if !blocked.is_empty() && !hooks.fix_permissions {
            let why = crate::service::binary::unreachable_message(&blocked, &Path::new(crate::userhome::KORA_USER_HOME).join(slug(&req.name)));
            return err(format!("Cannot set up the background service: {why} Nothing was changed."));
        }
    }
    Ok(())
}

/// Root installs by path, so a symbolic link on the way to the server folder that another account
/// could have planted (the panel's user owns the folder above it when installing through the
/// helper; the server's account owns the folder itself on a re-run) would send root's writes, and
/// the final `chown -R`, somewhere else. Refused before anything is done, again once the folder
/// exists, and before the folder is handed to an account. Links root alone controls (`/home` ->
/// `/var/home`) are fine.
pub(crate) fn check_dir_links(dir: &Path) -> Result<()> {
    check_dir_links_with(dir, &crate::fsutil::check_links_default_trust)
}

fn check_dir_links_with(dir: &Path, trust: &dyn Fn(u32, u32) -> bool) -> Result<()> {
    crate::fsutil::check_links_with(dir, trust).map_err(|e| InstallError(format!("Kora will not work in {}: {e}. Nothing more was changed.", dir.display())))
}

/// Folders the service account cannot pass through on the way to the server folder `dir`.
/// `/home/kora` is left out when Kora makes it passable itself (the `prepare.account` step).
pub fn service_blocking_dirs(dir: &Path) -> Vec<PathBuf> {
    let kora_user = crate::userhome::kora_user_planned(dir);
    crate::service::binary::blocking_dirs(dir).into_iter().filter(|d| !(kora_user && d == Path::new(crate::userhome::KORA_USER_HOME))).collect()
}

fn save_request(ctx: &Ctx) -> Result<()> {
    // Never persist the license key.
    let mut r = ctx.req.clone();
    r.license_key = None;
    r.dry_run = false;
    let bytes = serde_json::to_vec_pretty(&r).map_err(|e| InstallError(e.to_string()))?;
    crate::fsutil::write_no_follow(&ctx.meta_dir().join("request.json"), &bytes, 0o644).map_err(|e| InstallError(format!("cannot write install state: {e}")))
}

fn dry_run(req: &InstallRequest, sink: &mut dyn ProgressSink, hooks: &Hooks, admin: bool) -> Result<InstallOutcome> {
    let plan = plan_steps(req, admin);
    let total: u32 = plan.iter().map(|p| p.weight as u32).sum::<u32>().max(1);
    let mut notes = vec!["Dry run: nothing was changed.".to_string()];
    let (cat, cnote) = match load_catalog(req, hooks) {
        Ok(x) => x,
        Err(e) => {
            notes.push(format!("Catalog could not be loaded ({e}); resource list unavailable."));
            (Catalog { schema_version: 3, resources: vec![], creators: vec![] }, None)
        }
    };
    notes.extend(cnote);
    let selected = select_resources(&cat, &req.resources, req.mode == Mode::Development).unwrap_or_default();
    match preflight(req, hooks, admin) {
        Ok(()) => notes.push("Free disk space and service-account access look fine.".into()),
        Err(e) => notes.push(format!("WARNING, a real install would stop here: {e}")),
    }
    if req.service && admin && hooks.fix_permissions {
        let blocked = service_blocking_dirs(&req.dir);
        if !blocked.is_empty() {
            let cmds: Vec<String> = crate::service::binary::traverse_plan(&blocked).steps.iter().filter_map(|s| s.command_line()).collect();
            notes.push(format!("--fix-permissions: before the service is set up Kora would run: {}", cmds.join("; ")));
        }
    }
    if !req.dir.join("kora-server.json").is_file() {
        for c in crate::firewall::ports::install_conflicts(&req.name, &req.dir, req.ports.game, req.ports.txadmin, &crate::firewall::ports::SystemProber) {
            notes.push(format!("WARNING, a real install would stop here: {c}"));
        }
    }
    let mut base = 0u32;
    for p in &plan {
        let detail = match p.id {
            "prepare.account" => format!(
                "Create the user account '{user}' with the home folder {home} if it does not exist yet, let other accounts pass through {home} (chmod o+x), and make '{user}' the owner of {}",
                req.dir.display(),
                user = crate::userhome::KORA_USER,
                home = crate::userhome::KORA_USER_HOME,
            ),
            "prepare.dir" => format!("Create {}", req.dir.display()),
            "prepare.catalog" => format!("{} resources selected", selected.len()),
            "prepare.tools" => {
                format!("Needs git{}; ports {} (game) and {} (txAdmin) must be free", if req.mode == Mode::Development { " and a logged-in `gh`" } else { "" }, req.ports.game, req.ports.txadmin)
            }
            "build.forks" => format!(
                "Fork {} repositories into {}, reusing any forks already there (needs `gh auth login`; paced in batches)",
                selected.len(),
                req.github_owner.as_deref().unwrap_or("your GitHub account")
            ),
            "build.resources" => {
                let pinned = selected.iter().filter(|r| !r.versions.is_empty()).count();
                format!(
                    "{} resources under {}/resources ({} pinned to releases, {} default-branch head), 4 at a time",
                    selected.len(),
                    req.dir.display(),
                    if req.mode == Mode::Production { pinned } else { 0 },
                    if req.mode == Mode::Production { selected.len() - pinned } else { selected.len() }
                )
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
            "launch" => format!("Start FXServer and wait until txAdmin answers on port {}", req.ports.txadmin),
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
        name: req.name.clone(),
        ..Default::default()
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

/// [`rollback`] with the database administrator password given by the caller (the panel asks for it when
/// dropping the database needs a login the install did not keep). Host, port and user are the ones the install
/// remembered, else `127.0.0.1:3306` as `root`. The password is used for this call only.
pub fn rollback_with_db_password(dir: &Path, opts: &RollbackOptions, db_admin_password: Option<&str>, sink: &mut dyn ProgressSink) -> Result<Vec<String>> {
    rollback_inner(dir, opts, db_admin_password, sink, &Hooks::from_env())
}

pub fn rollback_with(dir: &Path, opts: &RollbackOptions, sink: &mut dyn ProgressSink, hooks: &Hooks) -> Result<Vec<String>> {
    rollback_inner(dir, opts, None, sink, hooks)
}

/// The server-list entry for `dir`: its name and ports, which `request.json` must not decide.
fn registered(dir: &Path) -> Option<(String, registry::PortsRecord)> {
    let reg = Registry::load().ok()?;
    reg.servers.iter().find(|s| registry::same_path(&s.dir, dir)).map(|s| (s.name.clone(), s.ports))
}

/// [`rollback_with_db_password`] with an administrator helper for the system-wide parts (the Windows
/// panel's worker, see [`system_ops`]).
pub fn rollback_with_admin(dir: &Path, opts: &RollbackOptions, db_admin_password: Option<&str>, admin: Option<system_ops::SharedAdmin>, sink: &mut dyn ProgressSink) -> Result<Vec<String>> {
    rollback_inner(dir, opts, db_admin_password, sink, &Hooks { admin, ..Hooks::from_env() })
}

fn rollback_inner(dir: &Path, opts: &RollbackOptions, db_admin_password: Option<&str>, sink: &mut dyn ProgressSink, hooks: &Hooks) -> Result<Vec<String>> {
    // The router must not keep forwarding a port to a server that is going away (a no-op unless forwarding was on).
    crate::reach::forward::release_for_dir(dir);
    rollback_known(dir, registered(dir), opts, db_admin_password, sink, hooks)
}

/// `known`: the name and ports from the operator's server list (`None`: the folder is not in it).
fn rollback_known(
    dir: &Path,
    known: Option<(String, registry::PortsRecord)>,
    opts: &RollbackOptions,
    db_admin_password: Option<&str>,
    sink: &mut dyn ProgressSink,
    hooks: &Hooks,
) -> Result<Vec<String>> {
    let bytes = std::fs::read(meta_dir(dir).join("request.json")).map_err(|_| InstallError(format!("{} has no Kora install record (.kora/request.json): nothing to roll back", dir.display())))?;
    let mut req: InstallRequest = serde_json::from_slice(&bytes).map_err(|e| InstallError(format!("corrupt install record: {e}")))?;
    req.dir = dir.to_path_buf();
    // `request.json` is in the server folder, which the server's own account can change: a forged
    // name would make root remove ANOTHER server's service, account and firewall rules. The
    // server list (only the operator can change it) decides instead.
    let system_trusted = known.is_some();
    if let Some((name, ports)) = known {
        req.name = name;
        (req.ports.game, req.ports.txadmin) = (ports.game, ports.txadmin);
    }
    if let Some(pw) = db_admin_password.filter(|p| !p.is_empty()) {
        let a = req.db_admin.get_or_insert_with(|| DbAdmin { host: "127.0.0.1".into(), port: 3306, user: "root".into(), password: None });
        a.password = Some(pw.to_string());
    }
    let admin = hooks.assume_admin.unwrap_or_else(is_admin);
    // A server Kora started in the background must not keep running (or hold files open) while it is deleted.
    let spec = crate::launch::Spec { name: req.name.clone(), dir: dir.to_path_buf(), game_port: req.ports.game, txadmin_port: req.ports.txadmin };
    if crate::launch::stop_detached(&crate::launch::SystemEnv::default(), &spec) {
        sink.event(ProgressEvent { step: "launch".into(), title: "Stop the running server".into(), status: StepStatus::Done, detail: "Stopped the background server".into(), percent: 0 });
    }
    let (tx, rx) = mpsc::channel();
    let ctx = new_ctx_trusted(&req, hooks, admin, tx, opts.keep_db, opts.keep_files, system_trusted);
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
            license_key_name: None,
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
            start: false,
            app: None,
        }
    }

    #[test]
    fn only_optional_steps_can_be_skipped() {
        let t = tempfile::tempdir().unwrap();
        let mut r = req(t.path());
        (r.dry_run, r.start) = (false, true);
        let plan = plan_steps_for(&r, false);
        let skippable: Vec<&str> = plan.iter().filter(|p| p.skippable).map(|p| p.id).collect();
        assert_eq!(skippable, ["system.firewall", "launch"]);
        assert!(plan.iter().all(|p| p.skippable == SKIPPABLE.contains(&p.id)));
        // A step the server needs: refused, with the ways that do help.
        let p = skip_step(&r, "build.artifact").unwrap_err();
        assert_eq!(p.code, codes::INSTALL_CANNOT_RESUME);
        assert!(p.actions.contains(&Action::RetryStep { step: "build.artifact".into() }));
        assert!(skip_step(&r, "no.such.step").is_err());
        assert!(!t.path().join(".kora/plan-state.json").exists(), "nothing written for a refusal");
        // An optional one is recorded, so the next run passes over it.
        std::fs::create_dir_all(t.path().join(".kora")).unwrap();
        skip_step(&r, "system.firewall").unwrap();
        let st = crate::steps::PlanState::load_or_default(&t.path().join(".kora/plan-state.json")).unwrap();
        assert_eq!(st.status("system.firewall"), Some(crate::steps::StepStatus::Skipped));
        // Not part of this install (no schedule without a development service): refused.
        assert!(skip_step(&r, "system.schedule").is_err());
    }

    struct Collect(Vec<ProgressEvent>);
    impl ProgressSink for Collect {
        fn event(&mut self, ev: ProgressEvent) {
            self.0.push(ev);
        }
    }

    /// `kora delete` as root reads `created.json`, which the service account can rewrite: absolute
    /// paths, `..` and a `resources` link must never make root delete anything outside the folder.
    #[cfg(unix)]
    #[test]
    fn delete_obeys_forged_records_only_inside_the_server_folder() {
        let t = tempfile::tempdir().unwrap();
        let victim = t.path().join("victim");
        std::fs::create_dir_all(victim.join("etc")).unwrap();
        std::fs::write(victim.join("etc/passwd"), "precious").unwrap();
        std::fs::write(victim.join("file"), "precious").unwrap();
        let dir = t.path().join("srv");
        std::fs::create_dir_all(dir.join(".kora")).unwrap();
        std::fs::create_dir_all(dir.join("own")).unwrap();
        std::fs::write(dir.join("server.cfg"), "x").unwrap();
        std::os::unix::fs::symlink(&victim, dir.join("resources")).unwrap();
        let mut r = req(&dir);
        (r.dry_run, r.db, r.service, r.firewall) = (false, DbChoice::Skip, false, false);
        std::fs::write(dir.join(".kora/request.json"), serde_json::to_vec(&r).unwrap()).unwrap();
        let abs = victim.join("file").to_string_lossy().into_owned();
        let c = Created { resources: vec!["etc".into(), "../../victim/etc".into(), abs.clone()], files: vec![abs, "../victim/file".into(), "server.cfg".into(), "own".into()], ..Created::default() };
        std::fs::write(dir.join(".kora/created.json"), serde_json::to_vec(&c).unwrap()).unwrap();
        let mut st = crate::steps::PlanState::default();
        for id in ["build.resources", "build.config"] {
            st.set(id, crate::steps::StepStatus::Done, None);
        }
        st.save(&dir.join(".kora/plan-state.json")).unwrap();
        let hooks = Hooks { assume_admin: Some(false), ..Hooks::default() };
        let notes = rollback_with(&dir, &RollbackOptions { keep_files: false, keep_db: true }, &mut Collect(vec![]), &hooks).unwrap();
        assert_eq!(std::fs::read_to_string(victim.join("etc/passwd")).unwrap(), "precious");
        assert_eq!(std::fs::read_to_string(victim.join("file")).unwrap(), "precious");
        assert!(!dir.join("server.cfg").exists() && !dir.join("own").exists(), "what really is inside is removed");
        assert!(notes.iter().any(|n| n.contains("Kept")), "{notes:?}");
    }

    /// Through the helper the panel's user owns the folder above the server folder and could swap a
    /// part of the path for a link to a system folder: root refuses to work there.
    #[cfg(unix)]
    #[test]
    fn a_planted_link_on_the_way_to_the_server_folder_is_refused() {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(t.path().join("real/servers")).unwrap();
        std::os::unix::fs::symlink(t.path().join("real/servers"), t.path().join("servers")).unwrap();
        let roots_view = |uid: u32, mode: u32| uid == 0 && mode & 0o022 == 0;
        let me = std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(t.path()).unwrap());
        if me != 0 {
            let e = check_dir_links_with(&t.path().join("servers/new"), &roots_view).unwrap_err().to_string();
            assert!(e.contains("symbolic link") && e.contains("real path"), "{e}");
        }
        // The real path is fine, so is a link the account itself owns, and a missing folder.
        assert!(check_dir_links_with(&t.path().join("real/servers/new"), &roots_view).is_ok());
        assert!(check_dir_links(&t.path().join("servers/new")).is_ok());
    }

    /// Server A's account rewrites A's `request.json` to name server B. `kora delete A` must then
    /// act on A (the server list's name and ports), never on B's service, account or ports; and a
    /// folder that is not in the list gets no system-wide changes at all.
    #[test]
    fn delete_takes_the_name_from_the_server_list_not_the_folder() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("a");
        std::fs::create_dir_all(dir.join(".kora")).unwrap();
        let mut r = req(&dir);
        (r.dry_run, r.db, r.name) = (false, DbChoice::Skip, "B".into());
        r.ports = Ports { game: 30999, txadmin: 40999 };
        std::fs::write(dir.join(".kora/request.json"), serde_json::to_vec(&r).unwrap()).unwrap();
        let c = Created { service_unit: Some("/etc/systemd/system/kora-b.service".into()), update_timer: true, firewall: true, ..Created::default() };
        let write_state = || {
            std::fs::write(dir.join(".kora/created.json"), serde_json::to_vec(&c).unwrap()).unwrap();
            let mut st = crate::steps::PlanState::default();
            for id in ["system.firewall", "system.service", "system.schedule"] {
                st.set(id, crate::steps::StepStatus::Done, None);
            }
            st.save(&dir.join(".kora/plan-state.json")).unwrap();
        };
        let hooks = Hooks { assume_admin: Some(false), ..Hooks::default() };
        let keep = RollbackOptions { keep_files: true, keep_db: true };
        write_state();
        let known = Some(("A".to_string(), registry::PortsRecord { game: 30120, txadmin: 40120 }));
        let notes = rollback_known(&dir, known, &keep, None, &mut Collect(vec![]), &hooks).unwrap().join("\n");
        assert!(notes.contains("'a'") && !notes.contains("'b'") && !notes.contains("kora-b"), "{notes}");
        // Not in the server list: nothing system-wide, said plainly.
        write_state();
        let notes = rollback_known(&dir, None, &keep, None, &mut Collect(vec![]), &hooks).unwrap().join("\n");
        assert!(notes.contains("not in Kora's server list") && notes.contains("background service") && notes.contains("firewall rules"), "{notes}");
        assert!(!notes.contains("NOT removed"), "no system plan is even prepared: {notes}");
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
        let all = select_resources(&cat, &ResourceSelection::All, false).unwrap();
        assert!(all.iter().all(|r| !r.is_internal()));
        assert!(all.iter().any(|r| r.id == "kora-core"));
        let only = select_resources(&cat, &ResourceSelection::Only(vec!["kora_chat".into()]), false).unwrap();
        assert!(only.iter().any(|r| r.id == "kora-chat"), "underscores normalise to dashes");
        assert!(only.iter().any(|r| r.id == "kora-core"), "core group always included");
        assert!(only.iter().any(|r| r.kind == Kind::Dependency));
        assert!(!only.iter().any(|r| r.id == "kora-hud"));
        assert!(select_resources(&cat, &ResourceSelection::Only(vec!["nope".into()]), false).is_err());
        assert!(select_resources(&cat, &ResourceSelection::Only(vec!["kora-scrapyard".into()]), false).is_err(), "internal is refused");
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
        let ids = |s: &ResourceSelection| select_resources(&cat, s, false).unwrap().iter().map(|r| r.id.clone()).collect::<Vec<_>>();
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
        let ids: Vec<String> = select_resources(&cat, &ResourceSelection::All, false).unwrap().iter().map(|r| r.id.clone()).collect();
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
        assert_eq!(select_resources(&cat, &ResourceSelection::Only(vec![]), false).unwrap().len(), 3);
    }

    fn v3_catalog() -> Catalog {
        let rel = r#""latest":"1.0.0","versions":[{"version":"1.0.0","tag":"v1.0.0","commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]"#;
        let json = format!(
            r#"{{"schemaVersion":3,"creators":[{{"id":"acme","name":"Acme"}}],"resources":[
            {{"id":"kora-lib","repository":"https://github.com/o/lib.git","path":"[kora]/lib","group":"pre","kind":"kora","branch":"dev","order":1,{rel},"dependsOn":["compat-a"]}},
            {{"id":"kora-x","repository":"https://github.com/o/x.git","path":"[kora]/x","group":"kora","kind":"kora","branch":"dev","order":2,{rel}}},
            {{"id":"compat-a","repository":"https://github.com/o/c.git","path":"a","group":"compat","kind":"dependency","branch":"main","order":3,"type":"compat",{rel}}},
            {{"id":"compat-b","repository":"https://github.com/o/cb.git","path":"b","group":"compat","kind":"dependency","branch":"main","order":4,"type":"compat",{rel}}},
            {{"id":"bridge-q","repository":"https://github.com/o/q.git","path":"q","group":"other","kind":"optional","branch":"main","order":5,"type":"bridge",{rel},"dependsOn":["compat-b"]}},
            {{"id":"acme-r","repository":"https://github.com/o/r.git","path":"r","group":"other","kind":"optional","branch":"main","order":6,"type":"creator","creator":"acme",{rel},"dependsOn":["bridge-q"]}}
            ]}}"#
        );
        Catalog::parse_validated(&json).unwrap()
    }

    #[test]
    fn typed_resources_are_never_default_selected_but_dependencies_come_along() {
        let cat = v3_catalog();
        let ids = |sel: &ResourceSelection| -> Vec<String> { select_resources(&cat, sel, false).unwrap().iter().map(|r| r.id.clone()).collect() };
        // kora-lib needs compat-a (a dependency by dependsOn); compat-b, bridge-q, acme-r are not selected
        assert_eq!(ids(&ResourceSelection::All), ["kora-lib", "kora-x", "compat-a"]);
        assert_eq!(ids(&ResourceSelection::Only(vec![])), ["kora-lib", "compat-a"]);
        // picking a creator resource pulls the bridge and its compat transitively
        assert_eq!(ids(&ResourceSelection::Only(vec!["acme-r".into()])), ["kora-lib", "compat-a", "compat-b", "bridge-q", "acme-r"]);
    }

    #[test]
    fn selection_folders_and_ensure_lines_for_a_new_install() {
        let cat = v3_catalog();
        let sel = select_resources(&cat, &ResourceSelection::Only(vec!["acme-r".into()]), false).unwrap();
        let rels: Vec<String> = sel.iter().map(|r| r.install_rel()).collect();
        assert_eq!(rels, ["[kora]/lib", "[compat]/a", "[compat]/b", "[bridges]/q", "[creators]/[acme]/r"]);
        let ensure = layout::ensure_for_new(rels.iter().map(String::as_str));
        // [cfx-default] holds nothing here, so it is not ensured
        assert_eq!(ensure, ["[compat]", "[kora]", "[bridges]", "[creators]", "[addons]", "[streamedassets]"]);
    }

    #[test]
    fn only_kora_selection_does_not_ensure_empty_compat_bridges_creators() {
        let cat = v3_catalog();
        let sel = select_resources(&cat, &ResourceSelection::Only(vec![]), false).unwrap();
        // kora-lib's dependency compat-a comes along, so [compat] is populated; bridges/creators are not
        let rels: Vec<String> = sel.iter().map(|r| r.install_rel()).collect();
        let ensure = layout::ensure_for_new(rels.iter().map(String::as_str));
        assert_eq!(ensure, ["[compat]", "[kora]", "[addons]", "[streamedassets]"]);
        let only_kora = layout::ensure_for_new(["[kora]/lib"]);
        assert_eq!(only_kora, ["[kora]", "[addons]", "[streamedassets]"]);
    }

    #[test]
    fn embedded_catalog_pulls_theme_in_with_lib_and_old_catalogs_still_parse() {
        let cat = Catalog::embedded().unwrap();
        assert!(cat.find("kora-theme").is_some());
        assert_eq!(cat.find("kora-lib").unwrap().depends_on, ["kora-theme"]);
        let only = select_resources(&cat, &ResourceSelection::Only(vec!["kora_chat".into()]), false).unwrap();
        assert!(only.iter().any(|r| r.id == "kora-theme"), "kora-lib is always installed, so is its dependency");
        let mut v: serde_json::Value = serde_json::from_str(catalog::EMBEDDED_SNAPSHOT).unwrap();
        for r in v["resources"].as_array_mut().unwrap() {
            r.as_object_mut().unwrap().remove("dependsOn");
        }
        assert!(Catalog::parse_validated(&v.to_string()).is_ok(), "dependsOn is optional");
    }

    #[test]
    fn the_old_app_step_is_gone_even_when_an_old_request_asks_for_it() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("srv");
        let mut r = req(&dir);
        r.app = Some(crate::app_install::AppChoice::default());
        assert!(!plan_steps(&r, true).iter().any(|p| p.id == "system.app"), "retired in 0.7.0: shortcuts are per-user now");
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

    #[test]
    fn the_kora_account_step_comes_first_and_only_when_planned() {
        let r = req(Path::new("/home/kora/my-server"));
        let with = plan_steps_for(&r, true);
        assert_eq!(with[0].id, "prepare.account");
        assert!(with[0].needs_admin);
        assert!(!plan_steps_for(&r, false).iter().any(|p| p.id == "prepare.account"));
        // the real decision: root and a folder in /home/kora only (see the userhome tests); never for other folders
        assert!(!plan_steps(&req(Path::new("/srv/x")), true).iter().any(|p| p.id == "prepare.account"));
    }

    #[test]
    fn the_account_step_reports_progress_and_only_uses_the_runner() {
        let r = req(Path::new("/home/kora/my-server"));
        let (tx, rx) = mpsc::channel();
        let ctx = new_ctx(&r, &Hooks::default(), true, tx, false, false);
        let fake = crate::service::testing::FakeRunner::default();
        fake.respond("id -u kora", 1, "");
        steps::prepare_account(&ctx, &fake).unwrap();
        drop(ctx);
        let ev: Vec<ProgressEvent> = rx.iter().collect();
        assert!(ev.iter().all(|e| e.step == "prepare.account"));
        assert_eq!(ev.last().unwrap().status, StepStatus::Done);
        assert!(fake.calls().iter().any(|c| c.starts_with("useradd --create-home --home-dir /home/kora")), "{:?}", fake.calls());
        let refused = crate::service::testing::FakeRunner::default();
        refused.respond("id -u kora", 0, "0\n");
        let (tx, rx) = mpsc::channel();
        let ctx = new_ctx(&r, &Hooks::default(), true, tx, false, false);
        assert!(steps::prepare_account(&ctx, &refused).is_err());
        drop(ctx);
        assert_eq!(rx.iter().last().unwrap().status, StepStatus::Failed);
    }

    #[cfg(unix)]
    #[test]
    fn a_private_parent_folder_stops_the_service_install_unless_fix_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("home-sam");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::set_permissions(t.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o750)).unwrap();
        let mut r = req(&home.join("kora").join("srv"));
        r.service = true;
        let disk = |fix| Hooks { assume_admin: Some(true), disk: Some(SharedDisk(Arc::new(TinyDisk(u64::MAX)))), fix_permissions: fix, ..Hooks::default() };
        let e = preflight(&r, &disk(false), true).unwrap_err().to_string();
        assert!(e.contains(&*home.to_string_lossy()) && e.contains("--fix-permissions") && e.contains("/home/kora/"), "{e}");
        assert!(preflight(&r, &disk(true), true).is_ok(), "opted in: the service step fixes it");
        assert!(preflight(&r, &disk(false), false).is_ok(), "not admin: no service account is made");
        assert_eq!(service_blocking_dirs(&r.dir), std::slice::from_ref(&home));
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o755)).unwrap();
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
        assert!(e.contains(&*dir.to_string_lossy()), "{e}");
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
        assert!(e.contains("cannot get into") && e.contains("Nothing was changed"), "{e}");
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

    #[test]
    fn development_only_resources_are_for_development_servers() {
        let extra = r#",{"id":"kora-fft","repository":"https://github.com/o/fft.git","path":"[kora]/kora_fft","group":"kora","kind":"optional","branch":"dev","order":5,"devOnly":true}"#;
        let cat = tiny_catalog(extra);
        let ids = |sel: &ResourceSelection, dev: bool| -> Vec<String> { select_resources(&cat, sel, dev).unwrap().iter().map(|r| r.id.clone()).collect() };
        // Never auto-selected, on either kind of server.
        assert!(!ids(&ResourceSelection::All, true).contains(&"kora-fft".to_string()));
        assert!(!ids(&ResourceSelection::Only(vec![]), true).contains(&"kora-fft".to_string()));
        // A production server is refused by name; a development server gets it with its dependencies.
        let err = select_resources(&cat, &ResourceSelection::Only(vec!["kora_fft".into()]), false).unwrap_err();
        assert!(err.to_string().contains("only available for development servers"), "{err}");
        assert_eq!(ids(&ResourceSelection::Only(vec!["kora_fft".into()]), true), ["kora-lib", "kora-theme", "kora-icons", "kora-fft"]);
    }
}
