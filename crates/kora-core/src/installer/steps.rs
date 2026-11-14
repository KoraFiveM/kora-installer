//! Concrete install steps (one `InstallStep` type dispatching on the step id).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::registry::{now_unix, PortsRecord, Registry, ResourceRecord, ServerEntry, ServerFile};
use super::*;
use crate::artifacts::{self, ArtifactStore, Channel, Platform as ArtPlatform, Verify};
use crate::catalog::ResolvedVersion;
use crate::db::{self, AdminConn, CommandSpec, Credentials, ProvisionState};
use crate::firewall::{self, PortRequest};
use crate::git;
use crate::service::{self, ManagedServer, Platform as SvcPlatform};
use crate::steps::StepResult;
use crate::txadmin::{self, CfxIdentity, DbConnection, ServerConfig};

const CLONE_WORKERS: usize = 4;

pub(crate) fn build(ctx: &Arc<Ctx>) -> Vec<Box<dyn Step>> {
    ctx.plan
        .iter()
        .map(|p| Box::new(InstallStep { ctx: ctx.clone(), id: p.id, title: p.title }) as Box<dyn Step>)
        .collect()
}

struct InstallStep {
    ctx: Arc<Ctx>,
    id: &'static str,
    title: &'static str,
}

fn fail(e: impl std::fmt::Display) -> StepError {
    StepError::Failed(e.to_string())
}

impl Step for InstallStep {
    fn id(&self) -> &str {
        self.id
    }
    fn description(&self) -> &str {
        self.title
    }

    fn verify(&self) -> StepResult<bool> {
        Ok(self.is_done())
    }

    fn run(&mut self) -> StepResult<()> {
        self.ctx.emit(self.id, StepStatus::Running, self.title, 0.0);
        match self.exec() {
            Ok(detail) => {
                self.ctx.emit(self.id, StepStatus::Done, detail, 1.0);
                Ok(())
            }
            Err(e) => {
                self.ctx.emit(self.id, StepStatus::Failed, e.to_string(), 0.0);
                Err(fail(e))
            }
        }
    }

    fn rollback(&mut self) -> StepResult<()> {
        self.ctx.emit(self.id, StepStatus::Running, "Rolling back", 0.0);
        match self.undo() {
            Ok(d) => {
                self.ctx.emit(self.id, StepStatus::Done, d, 1.0);
                Ok(())
            }
            Err(e) => {
                self.ctx.emit(self.id, StepStatus::Failed, e.to_string(), 0.0);
                Err(fail(e))
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------------------------

fn io<T>(r: std::io::Result<T>, what: &str) -> Result<T> {
    r.map_err(|e| InstallError(format!("{what}: {e}")))
}

fn res_dir(ctx: &Ctx) -> PathBuf {
    ctx.req.dir.join("resources")
}

fn db_state_path(ctx: &Ctx) -> PathBuf {
    ctx.meta_dir().join("db-provision.json")
}

fn db_client() -> String {
    let ok = |c: &str| std::process::Command::new(c).arg("--version").output().map(|o| o.status.success()).unwrap_or(false);
    if ok("mariadb") { "mariadb".into() } else { "mysql".into() }
}

/// Admin connection for provisioning. Preference: `req.db_admin`, then the environment
/// (`KORA_DB_ADMIN_PASSWORD` + optional `KORA_DB_ADMIN_USER`, `KORA_DB_HOST`, `KORA_DB_PORT`),
/// else local root socket auth. The admin password may also come from the environment when
/// `req.db_admin` has none (it is never written to the install record).
fn admin_conn(req: &InstallRequest) -> AdminConn {
    admin_conn_for(req.db_admin.as_ref())
}

fn admin_conn_for(db_admin: Option<&DbAdmin>) -> AdminConn {
    let client = db_client();
    let env_pw = std::env::var("KORA_DB_ADMIN_PASSWORD").ok().filter(|s| !s.is_empty());
    if let Some(a) = db_admin {
        let pw = a.password.clone().filter(|s| !s.is_empty()).or(env_pw);
        return AdminConn { host: a.host.clone(), port: a.port, user: a.user.clone(), password: pw.map(db::Secret::new), client };
    }
    match env_pw {
        Some(p) => AdminConn {
            host: std::env::var("KORA_DB_HOST").unwrap_or_else(|_| "127.0.0.1".into()),
            port: std::env::var("KORA_DB_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(3306),
            user: std::env::var("KORA_DB_ADMIN_USER").unwrap_or_else(|_| "root".into()),
            password: Some(db::Secret::new(p)),
            client,
        },
        None => AdminConn::socket_root(&client),
    }
}

/// True when the admin login was chosen explicitly (request or environment), so the local
/// server detection (which only knows the default install) is skipped.
fn db_override(req: &InstallRequest) -> bool {
    req.db_admin.is_some() || std::env::var("KORA_DB_ADMIN_PASSWORD").map(|s| !s.is_empty()).unwrap_or(false)
}

/// Try a real login with the given admin choice (`None` = environment / root socket).
/// Used by the wizard's "Test login" button. Returns a short plain-language error.
pub fn check_db_admin(db_admin: Option<&DbAdmin>) -> std::result::Result<(), String> {
    let admin = admin_conn_for(db_admin);
    login_problem(&admin).map_or(Ok(()), |why| {
        Err(format!(
            "could not log in to the database server as '{}' ({why}). Check the user, password and port, or pick another option.",
            admin.user
        ))
    })
}

/// `None` = the login works, otherwise the client's first error line.
fn login_problem(admin: &AdminConn) -> Option<String> {
    match db::CommandRunner::run(&db::SystemRunner, &admin.command("SELECT 1")) {
        Err(e) => Some(format!("could not run the database client: {e}")),
        Ok(out) if out.ok() => None,
        Ok(out) => Some(out.stderr.lines().next().unwrap_or("login refused").trim().to_string()),
    }
}

fn check_admin_login(admin: &AdminConn) -> Result<()> {
    let Some(why) = login_problem(admin) else { return Ok(()) };
    let advice = if admin.password.is_some() {
        "Check the admin user, password, address and port you gave (use the database admin step to test them)."
    } else {
        "Run as root with MariaDB unix-socket auth, or give an admin login (--db-admin-user / --db-admin-host / --db-admin-port with KORA_DB_ADMIN_PASSWORD)."
    };
    err(format!(
        "Could not log in to the database server as '{}' ({why}). Kora needs an administrator account to create a NEW database and user. {advice}",
        admin.user
    ))
}

fn windows_user() -> Option<String> {
    cfg!(windows).then(|| std::env::var("USERNAME").unwrap_or_default())
}

fn svc_platform() -> SvcPlatform {
    SvcPlatform::current()
}

fn forks_path(ctx: &Ctx) -> PathBuf {
    ctx.meta_dir().join("forks.json")
}

fn gh_login(r: &dyn git::CommandRunner) -> Result<String> {
    let o = r.run("gh", &["api", "user", "--jq", ".login"], None).map_err(|e| InstallError(format!("could not run gh: {e}")))?;
    if !o.ok() {
        return err("could not read your GitHub login from `gh`; run `gh auth login` first");
    }
    Ok(o.stdout.trim().to_string())
}

fn fork_owner(ctx: &Ctx, r: &dyn git::CommandRunner) -> Result<(String, bool)> {
    let login = gh_login(r)?;
    match ctx.req.github_owner.as_deref() {
        None => Ok((login, false)),
        Some(o) if o.eq_ignore_ascii_case(&login) => Ok((login, false)),
        Some(o) => {
            let exists = git::org_exists(r, o).map_err(|e| InstallError(e.to_string()))?;
            if exists {
                Ok((o.to_string(), true))
            } else {
                err(format!("GitHub organisation '{o}' was not found (or you are not a member). Create it first, or omit --github-owner to fork into {login}."))
            }
        }
    }
}

struct CloneItem {
    id: String,
    rel: String,
    dest: PathBuf,
    /// `owner/name` of the catalog (upstream) repository.
    repo: String,
    commit: Option<String>,
    branch: String,
    /// Dev: `owner/name` of the user's fork.
    fork: Option<String>,
}

fn items(ctx: &Ctx) -> Result<Vec<CloneItem>> {
    let sh = ctx.shared.lock().unwrap();
    let root = res_dir(ctx);
    let fork_owner = if ctx.req.mode == Mode::Development {
        let st = std::fs::read(forks_path(ctx)).ok().and_then(|b| serde_json::from_slice::<git::ForkState>(&b).ok());
        Some(st.map(|s| s.target_owner).ok_or_else(|| InstallError("fork state missing".into()))?)
    } else {
        None
    };
    Ok(sh
        .selected
        .iter()
        .map(|r| {
            let repo = repo_slug(&r.repository);
            let name = repo.rsplit('/').next().unwrap_or("").to_string();
            let commit = match (ctx.req.mode, r.resolve(catalog::Mode::Prod)) {
                (Mode::Production, ResolvedVersion::Pinned { commit, .. }) => Some(commit),
                _ => None,
            };
            CloneItem {
                id: r.id.clone(),
                rel: r.path.clone(),
                dest: root.join(&r.path),
                fork: fork_owner.as_ref().map(|o| format!("{o}/{name}")),
                repo,
                commit,
                branch: r.branch.clone(),
            }
        })
        .collect())
}

fn item_ok(it: &CloneItem) -> bool {
    if !it.dest.join(".git").exists() {
        return false;
    }
    match &it.commit {
        Some(c) => git::head_commit(&git::SystemRunner, &it.dest).map(|h| h.starts_with(c.as_str()) || c.starts_with(h.as_str())).unwrap_or(false),
        None => true,
    }
}

fn prune_empty_parents(root: &Path, mut p: PathBuf) {
    while p != root && p.starts_with(root) {
        if std::fs::remove_dir(&p).is_err() {
            break;
        }
        match p.parent() {
            Some(q) => p = q.to_path_buf(),
            None => break,
        }
    }
}

fn clone_one(ctx: &Ctx, it: &CloneItem) -> Result<()> {
    let r = git::SystemRunner;
    if item_ok(it) {
        return Ok(());
    }
    if it.dest.exists() {
        let ours = ctx.load_created().resources.contains(&it.rel);
        let empty = std::fs::read_dir(&it.dest).map(|mut d| d.next().is_none()).unwrap_or(false);
        if ours || empty {
            io(std::fs::remove_dir_all(&it.dest), "cannot replace an incomplete clone")?;
        } else {
            return err(format!("{} already exists and was not created by Kora; move it away or pick another folder", it.dest.display()));
        }
    }
    let name = it.dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let partial = it.dest.with_file_name(format!("{name}.kora-partial"));
    if partial.exists() {
        io(std::fs::remove_dir_all(&partial), "cannot clear a previous partial clone")?;
    }
    if let Some(p) = it.dest.parent() {
        io(std::fs::create_dir_all(p), "cannot create resource folder")?;
    }
    let rel = it.rel.clone();
    ctx.update_created(|c| {
        if !c.resources.contains(&rel) {
            c.resources.push(rel)
        }
    })?;
    let res = match (&it.fork, &it.commit) {
        (Some(fork), _) => git::clone_from_fork(&r, fork, &it.repo, &partial, Some(&it.branch)),
        (None, Some(c)) => git::clone_pinned(&r, &it.repo, c, &partial),
        (None, None) => git::clone_default_head(&r, &it.repo, &partial),
    };
    if let Err(e) = res {
        let _ = std::fs::remove_dir_all(&partial);
        return err(e.to_string());
    }
    io(std::fs::rename(&partial, &it.dest), "cannot move the clone into place")
}

fn artifact_store(ctx: &Ctx) -> ArtifactStore {
    ArtifactStore::new(ctx.req.dir.join("artifacts"))
}

fn parse_cfx(id: &str) -> Option<CfxIdentity> {
    let n: u64 = id.trim().strip_prefix("fivem:")?.parse().ok()?;
    Some(CfxIdentity { fivem_id: n, username: format!("cfx-{n}") })
}

/// Explicit service account chosen by the operator (`KORA_SERVICE_USER`). It must already exist and
/// is never created or removed by Kora. Without it Kora creates a dedicated `kora-<name>` account.
fn service_user_override() -> Option<String> {
    std::env::var("KORA_SERVICE_USER").ok().filter(|u| !u.is_empty() && u != "root")
}

/// The kora program a service runs: the root-owned copy when one was made, else this program.
fn kora_exe_for(created: &Created) -> String {
    created
        .binary
        .clone()
        .or_else(|| std::env::current_exe().ok().map(|p| p.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "kora".into())
}

fn managed_server(ctx: &Ctx, user: &str, kora_exe: &str) -> Result<ManagedServer> {
    Ok(ManagedServer {
        name: slug(&ctx.req.name),
        dir: ctx.req.dir.to_string_lossy().into_owned(),
        start_command: vec![kora_exe.to_string(), "service-host".into(), ctx.req.dir.to_string_lossy().into_owned()],
        user: user.to_string(),
        env_file: None,
        kora_exe: kora_exe.to_string(),
        windows_run_as: None,
    })
}

/// `uid:gid` of a folder (Unix); used to hand ownership back later.
#[cfg(unix)]
fn owner_of(p: &Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(p).ok().map(|m| format!("{}:{}", m.uid(), m.gid()))
}

#[cfg(not(unix))]
fn owner_of(_p: &Path) -> Option<String> {
    None
}

fn txadmin_env_text(ctx: &Ctx) -> String {
    txadmin::txadmin_env(&ctx.req.dir.join("txData"), ctx.req.ports.txadmin)
        .into_iter()
        .map(|(k, v)| format!("{k}={v}\n"))
        .collect()
}

// ---------------------------------------------------------------------------------------------
// verify / exec / undo
// ---------------------------------------------------------------------------------------------

impl InstallStep {
    fn is_done(&self) -> bool {
        let ctx = &self.ctx;
        match self.id {
            "prepare.dir" => ctx.req.dir.is_dir() && ctx.meta_dir().is_dir(),
            "build.forks" => std::fs::read(forks_path(ctx))
                .ok()
                .and_then(|b| serde_json::from_slice::<git::ForkState>(&b).ok())
                .is_some_and(|s| s.is_complete()),
            "build.resources" => match items(ctx) {
                Ok(v) => !v.is_empty() && v.iter().all(item_ok),
                Err(_) => false,
            },
            "build.db" => {
                ProvisionState::load(&db_state_path(ctx)).map(|s| s.granted).unwrap_or(false) && ctx.req.dir.join("database.json").is_file()
            }
            "build.artifact" => artifact_store(ctx)
                .current_dir()
                .ok()
                .flatten()
                .is_some_and(|d| d.join(ArtPlatform::current().executable()).is_file()),
            "build.config" => ["server.cfg", "permissions.cfg", "secrets.cfg", "txData/default/config.json"]
                .iter()
                .all(|f| ctx.req.dir.join(f).is_file()),
            "system.firewall" => ctx.admin && ctx.load_created().firewall,
            "system.service" => ctx.load_created().service_unit.is_some(),
            "system.schedule" => ctx.load_created().update_timer,
            _ => false,
        }
    }

    fn exec(&self) -> Result<String> {
        let ctx = &*self.ctx;
        match self.id {
            "prepare.dir" => {
                io(std::fs::create_dir_all(ctx.meta_dir()), "cannot create the server folder")?;
                Ok(format!("Using {}", ctx.req.dir.display()))
            }
            "prepare.catalog" => {
                let (cat, note) = load_catalog(&ctx.req, &ctx.hooks)?;
                let sel: Vec<Resource> = select_resources(&cat, &ctx.req.resources)?.into_iter().cloned().collect();
                if sel.is_empty() {
                    return err("no resources selected");
                }
                let n = sel.len();
                if let Some(n) = note {
                    ctx.note(n);
                }
                let mut sh = ctx.shared.lock().unwrap();
                sh.selected = sel;
                sh.catalog = Some(cat);
                Ok(format!("{n} resources selected"))
            }
            "prepare.tools" => self.prepare_tools(),
            "build.forks" => self.forks(),
            "build.resources" => self.resources(),
            "build.db" => self.database(),
            "build.artifact" => self.artifact(),
            "build.config" => self.config(),
            "system.firewall" => self.firewall(),
            "system.service" => self.service(),
            "system.schedule" => self.schedule(),
            "finalize" => self.finalize(),
            other => err(format!("unknown step {other}")),
        }
    }

    fn prepare_tools(&self) -> Result<String> {
        let ctx = &*self.ctx;
        let r = git::SystemRunner;
        let dev = ctx.req.mode == Mode::Development;
        let mut tools = git::detect(&r);
        if tools.git.is_none() || (dev && tools.gh.is_none()) {
            let plan = git::plan_bootstrap(git::Os::current(), &tools).map_err(|e| InstallError(e.to_string()))?;
            if !ctx.admin {
                let what: Vec<String> = plan.iter().map(|s| format!("- {}", s.description)).collect();
                return err(format!(
                    "git{} is missing. Installing it needs administrator rights; re-run as root/Administrator, or install it yourself:\n{}",
                    if dev { " / GitHub CLI" } else { "" },
                    what.join("\n")
                ));
            }
            for s in &plan {
                ctx.emit(self.id, StepStatus::Running, s.description.clone(), 0.1);
                let args: Vec<&str> = s.args.iter().map(String::as_str).collect();
                let o = git::CommandRunner::run(&r, &s.program, &args, None).map_err(|e| InstallError(format!("{}: {e}", s.description)))?;
                if !o.ok() {
                    return err(format!("{} failed: {}", s.description, o.stderr.trim()));
                }
            }
            tools = git::detect(&r);
            if tools.git.is_none() {
                return err("git is still missing after installation");
            }
        }
        if dev {
            if tools.gh.is_none() {
                return err("the GitHub CLI (gh) is required for development installs");
            }
            let st = git::auth_status(&r).map_err(|e| InstallError(e.to_string()))?;
            if !st.logged_in {
                return err("You are not signed in to GitHub. Run `gh auth login` (web/device flow), then run the installer again.");
            }
            let missing = st.missing_scopes(false);
            if !missing.is_empty() {
                return err(format!("Your GitHub sign-in lacks the scopes {}. Run `gh auth refresh -h github.com -s {}` and try again.", missing.join(", "), missing.join(",")));
            }
        }
        // Ports.
        let preq = PortRequest { server: slug(&ctx.req.name), game_port: ctx.req.ports.game, txadmin_port: ctx.req.ports.txadmin, expose_txadmin: ctx.req.txadmin_public };
        let already_installed = ctx.req.dir.join("kora-server.json").is_file();
        if !already_installed {
            let conflicts = firewall::port_conflicts(&preq).map_err(|e| InstallError(e.to_string()))?;
            if !conflicts.is_empty() {
                return err(conflicts.join(" "));
            }
        }
        // Database server.
        if ctx.req.db != DbChoice::Skip {
            if db_override(&ctx.req) {
                check_admin_login(&admin_conn(&ctx.req))?;
            } else {
                let found = db::detect(&db::SystemRunner, db::Os::current()).map_err(|e| InstallError(e.to_string()))?;
                if found.is_empty() {
                    match ctx.req.db {
                        DbChoice::InstallIfMissing => self.install_db_server()?,
                        _ => return err("No MariaDB/MySQL server was found. Re-run with --db install (needs admin) or --db skip."),
                    }
                } else if !found.iter().any(|s| s.running) {
                    return err(format!("{} is installed but not running. Start it (systemctl start {}) and try again.", found[0].name, found[0].name));
                }
                check_admin_login(&admin_conn(&ctx.req))?;
            }
        }
        Ok("git, database and ports are ready".into())
    }

    fn install_db_server(&self) -> Result<()> {
        let ctx = &*self.ctx;
        let plan = match db::plan_install(db::Os::current(), &ctx.req.dir, "", 3306, &[]) {
            Ok(p) if db::Os::current() == db::Os::Linux => p,
            _ => return err("Automatic MariaDB installation is only wired up for Debian/Ubuntu so far. Install MariaDB yourself, then use --db existing."),
        };
        if !ctx.admin {
            let what: Vec<String> = plan.steps.iter().map(|s| format!("- {}", s.description)).collect();
            return err(format!("MariaDB is missing and installing it needs administrator rights. Re-run as root, or install it yourself:\n{}", what.join("\n")));
        }
        for s in &plan.steps {
            ctx.emit(self.id, StepStatus::Running, s.description.clone(), 0.5);
            if let db::StepAction::Run(spec) = &s.action {
                let o = db::CommandRunner::run(&db::SystemRunner, spec).map_err(|e| InstallError(e.to_string()))?;
                if !o.ok() {
                    return err(format!("{} failed: {}", s.description, o.stderr.trim()));
                }
            }
        }
        Ok(())
    }

    fn forks(&self) -> Result<String> {
        let ctx = &*self.ctx;
        let r = git::SystemRunner;
        let repos: Vec<String> = ctx.shared.lock().unwrap().selected.iter().map(|x| repo_slug(&x.repository)).collect();
        let path = forks_path(ctx);
        let mut state = match std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<git::ForkState>(&b).ok()) {
            Some(mut s) => {
                s.requeue_failed();
                s
            }
            None => {
                let (owner, is_org) = fork_owner(ctx, &r)?;
                git::ForkState::new(&owner, is_org, &repos)
            }
        };
        let total = repos.len().max(1);
        let save = |s: &git::ForkState| {
            if let Ok(b) = serde_json::to_vec_pretty(s) {
                let _ = std::fs::write(&path, b);
            }
        };
        save(&state);
        let mut on_progress = |s: &git::ForkState| {
            save(s);
            ctx.emit(self.id, StepStatus::Running, format!("Forked {}/{total} (owner {})", s.done.len(), s.target_owner), s.done.len() as f32 / total as f32);
        };
        let out = git::run_forks(&r, &mut state, &git::ForkConfig::default(), &mut on_progress).map_err(|e| InstallError(e.to_string()))?;
        match out {
            git::ForkOutcome::Paused => err("GitHub rate limit reached. Progress is saved: run the installer again later to resume forking."),
            git::ForkOutcome::Finished if !state.failed.is_empty() => err(format!(
                "{} forks failed (first: {} - {}). Run again to retry them.",
                state.failed.len(),
                state.failed[0].repo,
                state.failed[0].error
            )),
            git::ForkOutcome::Finished => Ok(format!("{} forks ready in {}", state.done.len(), state.target_owner)),
        }
    }

    fn resources(&self) -> Result<String> {
        let ctx = &*self.ctx;
        let items = items(ctx)?;
        let total = items.len();
        io(std::fs::create_dir_all(res_dir(ctx)), "cannot create resources folder")?;
        // A resource whose folder lies inside another resource's clone (e.g. `[default-maps]/kora_phoneshop`
        // inside the `default-maps` repo) must wait until that parent clone has been moved into place.
        let nested = |it: &CloneItem| items.iter().any(|o| o.dest != it.dest && it.dest.starts_with(&o.dest));
        let (nested_items, plain_items): (Vec<&CloneItem>, Vec<&CloneItem>) = items.iter().partition(|it| nested(it));
        let (done, abort) = (AtomicUsize::new(0), AtomicBool::new(false));
        let errors: Mutex<Vec<String>> = Mutex::new(vec![]);
        for phase in [&plain_items, &nested_items] {
            let next = AtomicUsize::new(0);
            let count = phase.len();
            std::thread::scope(|s| {
                for _ in 0..CLONE_WORKERS.min(count.max(1)) {
                    s.spawn(|| loop {
                        let i = next.fetch_add(1, Ordering::SeqCst);
                        if i >= count || abort.load(Ordering::SeqCst) {
                            break;
                        }
                        let it = phase[i];
                        ctx.emit(self.id, StepStatus::Running, format!("Cloning {}", it.id), done.load(Ordering::SeqCst) as f32 / total as f32);
                        match clone_one(ctx, it) {
                            Ok(()) => {
                                let n = done.fetch_add(1, Ordering::SeqCst) + 1;
                                ctx.emit(self.id, StepStatus::Running, format!("Cloned {} ({n}/{total})", it.id), n as f32 / total as f32);
                            }
                            Err(e) => {
                                abort.store(true, Ordering::SeqCst);
                                errors.lock().unwrap().push(format!("{}: {e}", it.id));
                            }
                        }
                    });
                }
            });
        }
        let errs = errors.into_inner().unwrap();
        if !errs.is_empty() {
            return err(format!("could not get every resource (progress is kept; run again to resume):\n{}", errs.join("\n")));
        }
        Ok(format!("{total} resources ready"))
    }

    fn database(&self) -> Result<String> {
        let ctx = &*self.ctx;
        let admin = admin_conn(&ctx.req);
        let state_path = db_state_path(ctx);
        // A previous attempt that never produced usable credentials: drop its objects first.
        if let Ok(mut old) = ProvisionState::load(&state_path) {
            ctx.emit(self.id, StepStatus::Running, "Cleaning up an unfinished earlier attempt", 0.1);
            db::rollback(&db::SystemRunner, &admin, &mut old).map_err(|e| InstallError(e.to_string()))?;
            let _ = std::fs::remove_file(&state_path);
        }
        io(std::fs::create_dir_all(ctx.meta_dir()), "cannot create state folder")?;
        ctx.emit(self.id, StepStatus::Running, "Creating database and user", 0.4);
        let creds = db::provision(&db::SystemRunner, &admin, "127.0.0.1", "127.0.0.1", &state_path).map_err(|e| InstallError(e.to_string()))?;
        let json = serde_json::to_string_pretty(&creds).map_err(|e| InstallError(e.to_string()))?;
        let path = ctx.req.dir.join("database.json");
        ctx.update_created(|c| c.files.push("database.json".into()))?;
        db::write_secret_file(&db::SystemRunner, db::Os::current(), &path, &json, windows_user().as_deref())
            .map_err(|e| InstallError(e.to_string()))?;
        // Prove the new account works over TCP, the way oxmysql will connect.
        ctx.emit(self.id, StepStatus::Running, "Testing the new login", 0.8);
        verify_login(&admin.client, &creds)?;
        ctx.shared.lock().unwrap().db_name = Some(creds.database.clone());
        Ok(format!("Database {} created for user {}", creds.database, creds.user))
    }

    fn artifact(&self) -> Result<String> {
        let ctx = &*self.ctx;
        let platform = ArtPlatform::current();
        let (url, build) = match &ctx.req.artifact_url {
            Some(u) => {
                artifacts::validate_url(u, platform).map_err(|e| InstallError(e.to_string()))?;
                (u.clone(), artifacts::build_from_url(u).unwrap_or(0))
            }
            None => {
                ctx.emit(self.id, StepStatus::Running, "Looking up the recommended build", 0.02);
                let list = artifacts::list_artifacts(&ctx.hooks.http, platform).map_err(|e| InstallError(format!("could not look up server builds: {e}")))?;
                let b = list.pick(Channel::Recommended).ok_or_else(|| InstallError("no recommended server build available".into()))?;
                if let Some(why) = &b.broken {
                    ctx.note(format!("Build {} is marked as problematic: {why}", b.build));
                }
                (b.url.clone(), b.build)
            }
        };
        if let Some(gb) = ctx.req.game_build {
            if let Some(w) = artifacts::game_build_warning(gb, artifacts::KORA_MIN_GAME_BUILD) {
                ctx.note(w);
            }
        }
        let store = artifact_store(ctx);
        let adir = ctx.req.dir.join("artifacts");
        ctx.update_created(|c| c.artifacts = true)?;
        let dl = adir.join(".download");
        io(std::fs::create_dir_all(&dl), "cannot create artifacts folder")?;
        let file = dl.join(url.rsplit('/').next().unwrap_or("artifact"));
        let _ = std::fs::remove_file(&file);
        ctx.emit(self.id, StepStatus::Running, format!("Downloading build {build} (this can take a few minutes)"), 0.1);
        artifacts::download_verified(&ctx.hooks.http, &url, platform, &file, &Verify::default()).map_err(|e| InstallError(e.to_string()))?;
        ctx.emit(self.id, StepStatus::Running, format!("Unpacking build {build}"), 0.7);
        match store.swap(build, &file, platform) {
            Ok(()) => {}
            Err(artifacts::ArtifactError::Store(m)) if m.contains("already current") => {}
            Err(e) => return err(e.to_string()),
        }
        let _ = std::fs::remove_dir_all(&dl);
        ctx.shared.lock().unwrap().artifact_build = Some(build);
        Ok(format!("FXServer build {build} ready"))
    }

    fn config(&self) -> Result<String> {
        let ctx = &*self.ctx;
        let req = &ctx.req;
        let key = req.license_key.clone().unwrap_or_default();
        let master = req.cfx_identifier.as_deref().and_then(parse_cfx);
        if req.cfx_identifier.is_some() && master.is_none() {
            ctx.note("The Cfx identifier was not understood (expected fivem:<number>); no txAdmin master was configured.");
        }
        let ensure: Vec<String> = ctx.shared.lock().unwrap().selected.iter().map(|r| resource_name(r).to_string()).collect();
        let creds: Option<Credentials> = if req.db == DbChoice::Skip {
            None
        } else {
            let b = io(std::fs::read(req.dir.join("database.json")), "cannot read database.json")?;
            Some(serde_json::from_slice(&b).map_err(|e| InstallError(format!("corrupt database.json: {e}")))?)
        };
        let dbc = match &creds {
            Some(c) => DbConnection { user: c.user.clone(), password: txadmin::Secret::new(c.password.expose()), host: c.host.clone(), port: c.port, database: c.database.clone() },
            None => DbConnection { user: "kora_unset".into(), password: txadmin::Secret::new("unset-unset"), host: "127.0.0.1".into(), port: 3306, database: "kora_unset".into() },
        };
        let cfg = ServerConfig {
            name: if req.hostname.trim().is_empty() { req.name.clone() } else { req.hostname.clone() },
            game_port: req.ports.game,
            txadmin_port: req.ports.txadmin,
            max_clients: 48,
            game_build: req.game_build.unwrap_or(artifacts::KORA_MIN_GAME_BUILD),
            license_key: txadmin::Secret::new(key),
            db: dbc,
            master: master.clone(),
            ensure,
        };
        ctx.update_created(|c| {
            for f in ["server.cfg", "permissions.cfg", "secrets.cfg", "txData", "secrets"] {
                if !c.files.iter().any(|x| x == f) {
                    c.files.push(f.into());
                }
            }
        })?;
        let dir = &req.dir;
        if creds.is_some() {
            txadmin::write_server_files(dir, &cfg).map_err(|e| InstallError(e.to_string()))?;
        } else {
            let (s, p) = (txadmin::server_cfg(&cfg), txadmin::permissions_cfg(cfg.master.as_ref()));
            let (s, p) = (s.map_err(|e| InstallError(e.to_string()))?, p.map_err(|e| InstallError(e.to_string()))?);
            let secrets = format!(
                "# Kora secrets. Restricted permissions, git-ignored, never share.\nsv_licenseKey \"{}\"\n# No database was set up: add your connection string here:\n# set mysql_connection_string \"mysql://user:password@host/database?charset=utf8mb4\"\n",
                cfg.license_key.expose()
            );
            txadmin::write_file(&dir.join("secrets.cfg"), &secrets, true).map_err(|e| InstallError(e.to_string()))?;
            txadmin::write_file(&dir.join("server.cfg"), &s, false).map_err(|e| InstallError(e.to_string()))?;
            txadmin::write_file(&dir.join("permissions.cfg"), &p, false).map_err(|e| InstallError(e.to_string()))?;
            ctx.note("No database was created: set mysql_connection_string in secrets.cfg before starting.");
        }
        let txdata = dir.join("txData");
        txadmin::write_profile(&txdata, "default", dir, "server.cfg").map_err(|e| InstallError(e.to_string()))?;
        // txAdmin only auto-starts the game server once an admin exists (verified in txAdmin 8.1.2).
        let seed = match master {
            Some(m) => txadmin::MasterSeed::Cfx(m),
            None => {
                let pw = txadmin::Secret::new(txadmin::random_token(20).map_err(|e| InstallError(e.to_string()))?);
                txadmin::MasterSeed::Local { username: txadmin::LOCAL_ADMIN_USER.into(), password: pw }
            }
        };
        if txadmin::write_admins(&txdata, &seed).map_err(|e| InstallError(e.to_string()))? {
            if let txadmin::MasterSeed::Local { username, password } = &seed {
                let sdir = dir.join("secrets");
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt;
                    io(std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&sdir), "cannot create the secrets folder")?;
                }
                #[cfg(not(unix))]
                io(std::fs::create_dir_all(&sdir), "cannot create the secrets folder")?;
                let f = sdir.join("txadmin-admin.txt");
                txadmin::write_file(&f, &txadmin::local_admin_note(username, password, req.ports.txadmin), true).map_err(|e| InstallError(e.to_string()))?;
                ctx.note(format!(
                    "txAdmin login (no Cfx.re account was linked): user '{username}', password '{}'. Saved in {}. Change it after the first login.",
                    password.expose(),
                    f.display()
                ));
            }
        }
        // .gitignore
        let gi = dir.join(".gitignore");
        let existing = std::fs::read_to_string(&gi).unwrap_or_default();
        let mut add = String::new();
        for e in txadmin::GITIGNORE_ENTRIES.iter().chain([&".kora/"]) {
            if !existing.lines().any(|l| l.trim() == *e) {
                add.push_str(e);
                add.push('\n');
            }
        }
        if !add.is_empty() {
            if !gi.exists() {
                ctx.update_created(|c| c.files.push(".gitignore".into()))?;
            }
            io(std::fs::write(&gi, format!("{existing}{add}")), "cannot write .gitignore")?;
        }
        Ok("server.cfg, permissions.cfg, secrets.cfg and txAdmin profile written".into())
    }

    fn firewall(&self) -> Result<String> {
        let ctx = &*self.ctx;
        let preq = PortRequest { server: slug(&ctx.req.name), game_port: ctx.req.ports.game, txadmin_port: ctx.req.ports.txadmin, expose_txadmin: ctx.req.txadmin_public };
        ctx.note(firewall::router_note(&preq));
        let rn = service::SystemRunner;
        let Some(kind) = firewall::detect(svc_platform(), &rn) else {
            if !ctx.req.txadmin_public {
                ctx.note(format!(
                    "txAdmin (port {}) listens on all interfaces: txAdmin 8.1.2 cannot bind only the panel to loopback. No firewall is active here, so block that port from outside yourself or use it through an SSH tunnel.",
                    ctx.req.ports.txadmin
                ));
            }
            return Ok("No active firewall detected: nothing to open".into());
        };
        let plan = firewall::open_plan(&preq, kind).map_err(|e| InstallError(e.to_string()))?;
        if !ctx.admin {
            ctx.note(format!("Firewall NOT changed (needs administrator rights). Re-run as root/Administrator to: {}", plan.descriptions().join("; ")));
            return Ok("Skipped (needs admin); see notes".into());
        }
        ctx.update_created(|c| c.firewall = true)?;
        service::apply_plan(&plan, &rn).map_err(|e| InstallError(e.to_string()))?;
        Ok(format!("Opened port {} in the firewall", ctx.req.ports.game))
    }

    fn service(&self) -> Result<String> {
        let ctx = &*self.ctx;
        let platform = svc_platform();
        let rn = service::SystemRunner;
        let svc_name = format!("kora-{}", slug(&ctx.req.name));
        let created = ctx.load_created();

        // Which account runs the service?
        let (account, create_account) = match (service_user_override(), platform) {
            (Some(u), _) => (u, false),
            (None, SvcPlatform::Windows) => (svc_name.clone(), false), // NT SERVICE\<service>, made with the service
            (None, SvcPlatform::Linux) => {
                let name = created.service_account.clone().unwrap_or_else(|| service::account::account_name(&slug(&ctx.req.name)));
                match service::account::user_status(&rn, &name).map_err(|e| InstallError(e.to_string()))? {
                    service::account::UserStatus::Exists(0) => return err(format!("'{name}' is the root account: refusing to run a server as it. Remove it or choose another server name.")),
                    // An account Kora made in an earlier attempt of this same install is still ours.
                    service::account::UserStatus::Exists(_) => (name.clone(), false),
                    service::account::UserStatus::Missing => (name, true),
                }
            }
        };

        // Which program does the service run? Not one in a folder ordinary users can write to.
        let exe = std::env::current_exe().map_err(|e| InstallError(format!("cannot find the kora program: {e}")))?;
        let own_copy = match (created.binary.clone(), service::binary::exe_is_service_safe(&exe)) {
            (Some(b), _) => Some(PathBuf::from(b)),
            (None, true) => None,
            (None, false) => {
                let root = service::binary::default_root(platform).ok_or_else(|| InstallError("cannot find a program folder for the service copy of kora (ProgramFiles is not set)".into()))?;
                let sha = service::binary::file_sha256(&exe).map_err(|e| InstallError(format!("cannot read the kora program: {e}")))?;
                Some(service::binary::versioned_path(&root, env!("CARGO_PKG_VERSION"), &sha, platform))
            }
        };
        let kora_exe = own_copy.as_ref().map_or_else(|| exe.to_string_lossy().into_owned(), |p| p.to_string_lossy().into_owned());

        let server = managed_server(ctx, &account, &kora_exe)?;
        let mut head = Vec::new(); // account + program copy
        if create_account {
            head.extend(service::account::create_plan(&account).map_err(|e| InstallError(e.to_string()))?.steps);
        }
        if let Some(dest) = &own_copy {
            if !dest.is_file() {
                head.extend(service::binary::install_plan(&exe.to_string_lossy(), dest).steps);
            }
        }
        let mut tail = Vec::new();
        if platform == SvcPlatform::Linux {
            tail.push(service::SystemStep::write("Save the server's non-secret environment (txAdmin data folder and port)", &server.env_file_path(), txadmin_env_text(ctx), 0o600));
            tail.push(service::SystemStep::run(format!("Give '{account}' ownership of the server folder (and only that folder)"), "chown", &["-R", &format!("{account}:{account}"), &server.dir]));
        }
        tail.extend(service::install_plan(&server, platform).map_err(|e| InstallError(e.to_string()))?.steps);

        if !ctx.admin {
            let all: Vec<String> = head.iter().chain(tail.iter()).map(|s| s.description.clone()).collect();
            ctx.note(format!("Service NOT created (needs administrator rights). Re-run as root/Administrator to: {}", all.join("; ")));
            return Ok("Skipped (needs admin); see notes".into());
        }

        // Stage 1: account and program copy. Recorded as soon as they exist, so delete can remove exactly these.
        service::apply_plan(&service::Plan::new(head), &rn).map_err(|e| InstallError(e.to_string()))?;
        let dir_owner = owner_of(&ctx.req.dir);
        ctx.update_created(|c| {
            c.service_account = Some(account.clone());
            c.service_account_created |= create_account;
            if c.dir_owner.is_none() {
                c.dir_owner = dir_owner;
            }
            if own_copy.is_some() {
                c.binary = own_copy.as_ref().map(|p| p.to_string_lossy().into_owned());
            }
        })?;
        // Stage 2: ownership + unit.
        let unit = server.unit_path();
        ctx.update_created(|c| c.service_unit = Some(unit))?;
        service::apply_plan(&service::Plan::new(tail), &rn).map_err(|e| InstallError(e.to_string()))?;
        if create_account {
            ctx.note(format!("Created the system account '{account}' (no login, no home folder) for the server; `kora delete` removes it again."));
        }
        if let Some(p) = &own_copy {
            ctx.note(format!("The service runs the root-owned copy of kora at {}.", p.display()));
        }
        Ok(format!("Service {} registered (not started), running as '{account}'", server.service_name()))
    }

    /// Dev mode: daily 04:00 `kora update --auto <dir>` timer (systemd) / scheduled task (Windows).
    fn schedule(&self) -> Result<String> {
        let ctx = &*self.ctx;
        let created = ctx.load_created();
        let user = created.service_account.clone().unwrap_or_else(|| format!("kora-{}", slug(&ctx.req.name)));
        let server = managed_server(ctx, &user, &kora_exe_for(&created))?;
        let sched = service::Schedule::default();
        let plan = service::scheduler_install_plan(&server, &sched, svc_platform()).map_err(|e| InstallError(e.to_string()))?;
        if !ctx.admin {
            ctx.note(format!("Auto-update schedule NOT created (needs administrator rights). Re-run as root/Administrator to: {}", plan.descriptions().join("; ")));
            return Ok("Skipped (needs admin); see notes".into());
        }
        ctx.update_created(|c| c.update_timer = true)?;
        service::apply_plan(&plan, &service::SystemRunner).map_err(|e| InstallError(e.to_string()))?;
        Ok(format!("Auto-update scheduled {}", sched.describe()))
    }

    fn finalize(&self) -> Result<String> {
        let ctx = &*self.ctx;
        let req = &ctx.req;
        let build = artifact_store(ctx).state().ok().and_then(|s| s.current).unwrap_or(0);
        let resources: Vec<ResourceRecord> = ctx
            .shared
            .lock()
            .unwrap()
            .selected
            .iter()
            .map(|r| ResourceRecord {
                id: r.id.clone(),
                path: r.path.clone(),
                repository: r.repository.clone(),
                branch: r.branch.clone(),
                commit: match (req.mode, r.resolve(catalog::Mode::Prod)) {
                    (Mode::Production, ResolvedVersion::Pinned { commit, .. }) => Some(commit),
                    _ => None,
                },
            })
            .collect();
        let database = ProvisionState::load(&db_state_path(ctx)).ok().map(|s| s.database);
        let now = now_unix();
        let sf = ServerFile {
            name: req.name.clone(),
            mode: req.mode,
            dir: req.dir.clone(),
            ports: PortsRecord::from(&req.ports),
            game_build: req.game_build.unwrap_or(artifacts::KORA_MIN_GAME_BUILD),
            artifact_build: build,
            database,
            github_owner: req.github_owner.clone(),
            created_unix: now,
            kora_version: env!("CARGO_PKG_VERSION").into(),
            resources,
        };
        ctx.update_created(|c| {
            if !c.files.iter().any(|f| f == "kora-server.json") {
                c.files.push("kora-server.json".into());
            }
            c.registered = true;
        })?;
        io(sf.save(), "cannot write kora-server.json")?;
        let mut reg = io(Registry::load(), "cannot read the server registry")?;
        reg.upsert(ServerEntry { name: req.name.clone(), dir: req.dir.clone(), mode: req.mode, ports: PortsRecord::from(&req.ports), created_unix: now });
        io(reg.save(), "cannot write the server registry")?;
        Ok(format!("Registered '{}' in {}", req.name, registry::registry_path().display()))
    }

    // ---- rollback -------------------------------------------------------------------------

    fn undo(&self) -> Result<String> {
        let ctx = &*self.ctx;
        let created = ctx.load_created();
        match self.id {
            "build.forks" => {
                ctx.note("GitHub forks were left in place; delete them on GitHub if you no longer want them.");
                Ok("Forks left in place".into())
            }
            "build.resources" if !ctx.keep_files => {
                let root = res_dir(ctx);
                for rel in &created.resources {
                    let p = root.join(rel);
                    if p.exists() {
                        io(std::fs::remove_dir_all(&p), "cannot remove a resource folder")?;
                    }
                    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let _ = std::fs::remove_dir_all(p.with_file_name(format!("{name}.kora-partial")));
                    if let Some(parent) = p.parent() {
                        prune_empty_parents(&root, parent.to_path_buf());
                    }
                }
                let _ = std::fs::remove_dir(&root);
                Ok(format!("Removed {} resources", created.resources.len()))
            }
            "build.db" if !ctx.keep_db => {
                let path = db_state_path(ctx);
                let Ok(mut st) = ProvisionState::load(&path) else { return Ok("No database to remove".into()) };
                let (dbn, usr) = (st.database.clone(), st.user.clone());
                let admin = admin_conn(&ctx.req);
                db::rollback(&db::SystemRunner, &admin, &mut st).map_err(|e| {
                    InstallError(format!(
                        "could not remove database '{dbn}' / user '{usr}': {e}. Everything else was already removed. Kora needs the database admin login again to drop them: \
                         set KORA_DB_ADMIN_PASSWORD to the admin password (host/port/user are remembered from the install) or run as root with socket auth, then run `kora delete {}` again.",
                        ctx.req.dir.display()
                    ))
                })?;
                st.save(&path).map_err(|e| InstallError(e.to_string()))?;
                let _ = std::fs::remove_file(ctx.req.dir.join("database.json"));
                Ok(format!("Dropped database {dbn} and user {usr}"))
            }
            "build.db" => {
                if let Ok(st) = ProvisionState::load(&db_state_path(ctx)) {
                    ctx.note(format!("Database '{}' and user '{}' were kept.", st.database, st.user));
                }
                Ok("Database kept".into())
            }
            "build.artifact" if !ctx.keep_files && created.artifacts => {
                let _ = std::fs::remove_dir_all(ctx.req.dir.join("artifacts"));
                Ok("Removed the server build".into())
            }
            "build.config" if !ctx.keep_files => {
                for f in &created.files {
                    if f == "database.json" || f == "kora-server.json" {
                        continue;
                    }
                    let p = ctx.req.dir.join(f);
                    if p.is_dir() {
                        let _ = std::fs::remove_dir_all(&p);
                    } else {
                        let _ = std::fs::remove_file(&p);
                    }
                }
                Ok("Removed generated configuration".into())
            }
            "system.firewall" if created.firewall => {
                let preq = PortRequest { server: slug(&ctx.req.name), game_port: ctx.req.ports.game, txadmin_port: ctx.req.ports.txadmin, expose_txadmin: ctx.req.txadmin_public };
                let rn = service::SystemRunner;
                if let Some(kind) = firewall::detect(svc_platform(), &rn) {
                    let plan = firewall::close_plan(&preq, kind).map_err(|e| InstallError(e.to_string()))?;
                    if ctx.admin {
                        service::apply_plan(&plan, &rn).map_err(|e| InstallError(e.to_string()))?;
                    } else {
                        ctx.note(format!("Firewall rules NOT removed (needs admin): {}", plan.descriptions().join("; ")));
                    }
                }
                Ok("Firewall rules removed".into())
            }
            "system.service" | "system.schedule" => {
                let is_service = self.id == "system.service";
                let wanted: &[system::Kind] = if is_service { &[system::Kind::Service, system::Kind::ServiceAccount, system::Kind::Binary] } else { &[system::Kind::UpdateTimer] };
                let rn = service::SystemRunner;
                let reg = Registry::load().unwrap_or_default();
                let used = created.binary.as_deref().is_some_and(|b| system::binary_used_elsewhere(&reg, b, &[ctx.req.dir.as_path()]));
                let target = system::Target::from_request(&ctx.req);
                let items: Vec<system::Item> = system::items_for(&target, &created, svc_platform(), &rn, ctx.keep_files, used)?
                    .into_iter()
                    .filter(|i| wanted.contains(&i.kind))
                    .collect();
                if items.is_empty() {
                    return Ok("Nothing to undo".into());
                }
                let mut done = Vec::new();
                for item in items {
                    if let Some(n) = &item.note {
                        ctx.note(n.clone());
                    }
                    if !ctx.admin {
                        ctx.note(format!("NOT removed (needs administrator rights): {}", item.plan.descriptions().join("; ")));
                        continue;
                    }
                    service::apply_plan(&item.plan, &rn).map_err(|e| InstallError(format!("{}: {e}", item.title)))?;
                    if system::clears_record(&item) {
                        ctx.update_created(|c| system::clear_record(c, item.kind))?;
                    }
                    done.push(item.title.clone());
                }
                Ok(if done.is_empty() { "Skipped (needs admin)".into() } else { done.join("; ") })
            }
            "finalize" => {
                if let Ok(mut reg) = Registry::load() {
                    if reg.remove(&ctx.req.name) {
                        let _ = reg.save();
                    }
                }
                if !ctx.keep_files {
                    let _ = std::fs::remove_file(registry::server_file_path(&ctx.req.dir));
                }
                Ok("Removed from the server registry".into())
            }
            _ => Ok("Nothing to undo".into()),
        }
    }
}

fn verify_login(client: &str, c: &Credentials) -> Result<()> {
    let spec = CommandSpec {
        program: client.to_string(),
        args: ["--batch", "--skip-column-names", "--protocol=tcp", "-h", &c.host, "-P", &c.port.to_string(), "-u", &c.user, &c.database]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        env: vec![("MYSQL_PWD".into(), c.password.expose().to_string())],
        stdin: Some("SELECT 1;".into()),
    };
    let o = db::CommandRunner::run(&db::SystemRunner, &spec).map_err(|e| InstallError(e.to_string()))?;
    if o.ok() {
        Ok(())
    } else {
        err(format!("The new database login was created but could not connect over TCP ({}).", o.stderr.lines().next().unwrap_or("").trim()))
    }
}

/// After every step rolled back: delete the state folder and, if Kora created the folder, the folder.
pub(crate) fn finish_rollback(ctx: &Ctx) -> Result<()> {
    if ctx.keep_files {
        return Ok(());
    }
    let created = ctx.load_created();
    let _ = std::fs::remove_dir_all(ctx.meta_dir());
    if created.created_dir {
        io(std::fs::remove_dir_all(&ctx.req.dir), "cannot remove the server folder")?;
    } else {
        let _ = std::fs::remove_dir(ctx.req.dir.join("txData"));
    }
    Ok(())
}

#[cfg(test)]
mod db_admin_tests {
    use super::*;

    fn admin(pw: Option<&str>) -> DbAdmin {
        DbAdmin { host: "10.0.0.5".into(), port: 33061, user: "dba".into(), password: pw.map(String::from) }
    }

    #[test]
    fn request_admin_is_preferred_over_environment() {
        // Whatever the environment says, an explicit request wins for host/port/user/password.
        let c = admin_conn_for(Some(&admin(Some("secret"))));
        assert_eq!((c.host.as_str(), c.port, c.user.as_str()), ("10.0.0.5", 33061, "dba"));
        assert_eq!(c.password.as_ref().unwrap().expose(), "secret");
    }

    #[test]
    fn db_admin_password_is_never_serialised_or_printed() {
        let a = admin(Some("hunter2-secret"));
        assert!(!format!("{a:?}").contains("hunter2"));
        let json = serde_json::to_string(&a).unwrap();
        assert!(!json.contains("hunter2") && !json.contains("password"), "{json}");
        let back: DbAdmin = serde_json::from_str(&json).unwrap();
        assert_eq!(back.password, None);
        assert_eq!((back.user.as_str(), back.port), ("dba", 33061));
    }
}
