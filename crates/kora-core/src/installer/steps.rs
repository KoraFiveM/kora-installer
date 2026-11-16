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

/// Runs before the step engine (the server folder lives in the `kora` account's home, so the account
/// comes first). Reported as the `prepare.account` step. Idempotent.
pub(crate) fn prepare_account(ctx: &Ctx, runner: &dyn service::CommandRunner) -> Result<()> {
    let id = "prepare.account";
    ctx.emit(id, StepStatus::Running, "Checking the 'kora' user account", 0.0);
    match crate::userhome::ensure_kora_user(runner) {
        Ok(detail) => {
            ctx.emit(id, StepStatus::Done, detail, 1.0);
            Ok(())
        }
        Err(e) => {
            ctx.emit(id, StepStatus::Failed, e.clone(), 0.0);
            err(e)
        }
    }
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
    let ok = |c: &str| crate::sysexe::command(c).arg("--version").output().map(|o| o.status.success()).unwrap_or(false);
    if ok("mariadb") {
        return "mariadb".into();
    }
    if ok("mysql") {
        return "mysql".into();
    }
    if cfg!(windows) {
        if let Some(c) = windows_db::saved_record().map(|r| r.client).filter(|c| c.is_file()).or_else(|| windows_client_candidates().into_iter().find(|c| c.is_file())) {
            return c.to_string_lossy().into_owned();
        }
    }
    "mysql".into()
}

/// winget exits non-zero for "already installed" (0x8A150061) and "no newer version" (0x8A15010D):
/// for our purpose (the tool is there) both are success.
pub(crate) fn winget_code_is_fine(code: i32) -> bool {
    matches!(code as u32, 0 | 0x8A15_0061 | 0x8A15_010D)
}

/// Open GitHub's device-login page in the default browser. Only ever a github.com address, never a shell.
fn open_github_page(url: &str) -> bool {
    if !url.starts_with("https://github.com/") || url.contains(|c: char| c.is_whitespace() || c.is_control() || c == '"') {
        return false;
    }
    let (program, args): (&str, Vec<&str>) = if cfg!(windows) { ("explorer.exe", vec![url]) } else { ("xdg-open", vec![url]) };
    crate::sysexe::command(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_ok()
}

fn winget_available() -> bool {
    crate::sysexe::command("winget").arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

/// Where the MariaDB / MySQL installers put their command line client (Windows).
fn windows_client_candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    for var in ["ProgramFiles", "ProgramFiles(x86)"] {
        let Some(pf) = std::env::var_os(var).map(PathBuf::from) else { continue };
        for vendor in ["MariaDB", "MySQL"] {
            let Ok(rd) = std::fs::read_dir(&pf) else { continue };
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                if n.starts_with(vendor) {
                    let nested = std::fs::read_dir(e.path()).into_iter().flatten().flatten().map(|d| d.path());
                    for dir in std::iter::once(e.path()).chain(nested) {
                        for exe in ["mariadb.exe", "mysql.exe"] {
                            v.push(dir.join("bin").join(exe));
                        }
                    }
                }
            }
        }
    }
    v
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
    // The login of the MariaDB Kora installed itself (Windows), unless the operator chose another.
    if env_pw.is_none() {
        if let Some(saved) = windows_db::saved_admin() {
            return saved.to_conn();
        }
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
pub(crate) fn db_override(req: &InstallRequest) -> bool {
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

/// Once `gh` is signed in: check an explicitly requested owner against the account's real list.
/// Runs before anything is forked (an install started with `--github-owner` while `gh` was missing
/// or signed out could not check it up front). Nothing is created on failure.
fn verify_requested_owner(r: &dyn git::CommandRunner, requested: Option<&str>) -> Result<()> {
    let Some(want) = requested.map(str::trim).filter(|o| !o.is_empty()) else { return Ok(()) };
    git::owners::check_login_syntax(want).map_err(InstallError)?;
    let all = git::owners::list_owners(r).map_err(|e| InstallError(format!("could not read your GitHub accounts ({e}). Run the installer again to retry; your answers are saved.")))?;
    match git::owners::choose_owner(&all, Some(want)) {
        Ok(_) => Ok(()),
        Err(m) => err(format!("{m}\nNothing was forked. Run the installer again with a valid --github-owner (your answers are saved).")),
    }
}

fn fork_owner(ctx: &Ctx, r: &dyn git::CommandRunner) -> Result<(String, bool)> {
    use git::owners::{check_org, gh_auth, GhAuth};
    let login = match gh_auth(r) {
        GhAuth::LoggedIn { login } => login,
        GhAuth::Missing => return err("the GitHub CLI (gh) is not installed"),
        GhAuth::LoggedOut => return err("you are not logged in to GitHub: run `gh auth login` first"),
    };
    match ctx.req.github_owner.as_deref() {
        None => Ok((login, false)),
        Some(o) if o.eq_ignore_ascii_case(&login) => Ok((login, false)),
        Some(o) => {
            let org = check_org(r, o);
            if org.can_fork {
                Ok((org.login, true))
            } else {
                err(format!(
                    "You cannot fork into '{o}': {}. Choose another owner (`kora github owners` lists them) or omit --github-owner to fork into {login}.",
                    org.reason.as_deref().unwrap_or("not allowed")
                ))
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
    /// Install only this subfolder of the repository (a pinned copy, no `.git`).
    subpath: Option<String>,
}

fn items(ctx: &Ctx) -> Result<Vec<CloneItem>> {
    let sh = ctx.shared.lock().unwrap();
    let root = res_dir(ctx);
    let fork_state = if ctx.req.mode == Mode::Development {
        let st = std::fs::read(forks_path(ctx)).ok().and_then(|b| serde_json::from_slice::<git::ForkState>(&b).ok());
        Some(st.ok_or_else(|| InstallError("fork state missing".into()))?)
    } else {
        None
    };
    sh.selected
        .iter()
        .map(|r| {
            let repo = repo_slug(&r.repository);
            // Pinned copies (bridge / creator resources, subpath installs) are never forked, in any mode.
            let commit = match (ctx.req.mode == Mode::Production || r.is_pinned_copy(), r.resolve(catalog::Mode::Prod)) {
                (true, ResolvedVersion::Pinned { commit, .. }) => Some(commit),
                _ => None,
            };
            let rel = r.install_rel();
            // The catalog is checked when loaded; this keeps a bad path from ever leaving `resources/`.
            let dest = crate::fsutil::join_inside(&root, &rel).ok_or_else(|| InstallError(format!("{}: unsafe path {rel}", r.id)))?;
            Ok(CloneItem {
                id: r.id.clone(),
                dest,
                rel,
                // The real fork (its name may differ from the upstream's).
                fork: if r.is_pinned_copy() { None } else { fork_state.as_ref().map(|s| s.fork_of(&repo)) },
                subpath: r.subpath.clone(),
                repo,
                commit,
                branch: r.branch.clone(),
            })
        })
        .collect()
}

#[cfg(test)]
mod fork_sync_tests {
    use super::*;
    use crate::git::{SyncOutcome as O, SyncRecord};

    #[test]
    fn summary_lists_untouched_forks_with_the_safe_fix_and_counts() {
        let mut st = git::ForkState::new("me", false, &[]);
        assert_eq!(fork_sync_report(&st, &|r| r.to_string()), (String::new(), vec![]));
        let rec = |repo: &str, outcome| SyncRecord { repo: repo.into(), branch: "dev".into(), outcome };
        st.sync = vec![rec("a/one", O::Synced { commits: 2 }), rec("a/two", O::UpToDate), rec("a/three", O::Diverged { ahead: 1, behind: 4 }), rec("a/four", O::Ahead { ahead: 2 })];
        let (head, lines) = fork_sync_report(&st, &|r| format!("resources/{}", r.rsplit('/').next().unwrap()));
        assert_eq!(head, "; 1 fork(s) brought up to date, 1 already current, 2 left alone");
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("me/three") && lines[0].contains("cd resources/three && git fetch upstream && git merge upstream/dev") && lines[0].contains("Nothing was changed"), "{}", lines[0]);
        assert!(lines[1].contains("me/four") && lines[1].contains("2 commit(s) of its own") && lines[1].contains("Nothing was changed"));
        // the line names the real fork, not the upstream's name
        st.forks.insert("a/three".into(), "me/three-1".into());
        let (_, lines) = fork_sync_report(&st, &|r| r.to_string());
        assert!(lines[0].starts_with("me/three-1 (dev)"), "{}", lines[0]);
    }

    #[test]
    fn development_clones_come_from_the_recorded_fork() {
        let d = tempfile::tempdir().unwrap();
        let req = InstallRequest {
            mode: Mode::Development,
            name: "s".into(),
            dir: d.path().to_path_buf(),
            hostname: "s".into(),
            license_key: None,
            cfx_identifier: None,
            ports: Ports::default(),
            txadmin_public: false,
            artifact_url: None,
            game_build: None,
            resources: ResourceSelection::All,
            db: DbChoice::Skip,
            service: false,
            firewall: false,
            github_owner: None,
            catalog: None,
            dry_run: true,
            db_admin: None,
            start: false,
            app: None,
        };
        let ctx = new_ctx(&req, &Hooks::default(), false, std::sync::mpsc::channel().0, false, false);
        let rel = r#""latest":"1.0.0","versions":[{"version":"1.0.0","tag":"v1.0.0","commit":"0123456789012345678901234567890123456789"}]"#;
        let cat = Catalog::parse_validated(&format!(
            r#"{{"schemaVersion":3,"resources":[
            {{"id":"kora-core","repository":"https://github.com/KoraFiveM/kora_core.git","path":"[kora]/kora_core","group":"kora","kind":"kora","branch":"dev","order":1,{rel}}},
            {{"id":"kora-lib","repository":"https://github.com/KoraFiveM/kora_lib.git","path":"[kora]/kora_lib","group":"kora","kind":"kora","branch":"dev","order":2,{rel}}}]}}"#
        ))
        .unwrap();
        ctx.shared.lock().unwrap().selected = cat.resources.clone();
        std::fs::create_dir_all(ctx.meta_dir()).unwrap();
        // kora_core's fork got a `-1`; kora_lib's entry is missing (an older forks.json)
        let mut st = git::ForkState::new("me", false, &[]);
        st.forks.insert("KoraFiveM/kora_core".into(), "me/kora_core-1".into());
        std::fs::write(forks_path(&ctx), serde_json::to_vec(&st).unwrap()).unwrap();
        let it = items(&ctx).unwrap();
        let fork = |id: &str| it.iter().find(|i| i.id == id).unwrap().fork.clone();
        assert_eq!(fork("kora-core").as_deref(), Some("me/kora_core-1"));
        assert_eq!(fork("kora-lib").as_deref(), Some("me/kora_lib"));
    }
}

/// Text for the end of the fork step: `"; 2 fork(s) brought up to date, ..."` (empty when no existing
/// fork was adopted) and one line per fork that was left alone, with the safe fix.
fn fork_sync_report(state: &git::ForkState, path_of: &dyn Fn(&str) -> String) -> (String, Vec<String>) {
    if state.sync.is_empty() {
        return (String::new(), vec![]);
    }
    let s = state.sync_summary(path_of);
    (format!("; {}", s.headline()), s.lines)
}

fn item_ok(it: &CloneItem) -> bool {
    if it.subpath.is_some() {
        return it.commit.as_deref().is_some_and(|c| pinned::marker_matches(&it.dest, c));
    }
    if !it.dest.join(".git").exists() {
        return false;
    }
    match &it.commit {
        Some(c) => git::head_commit(&git::SystemRunner, &it.dest).map(|h| h.starts_with(c.as_str()) || c.starts_with(h.as_str())).unwrap_or(false),
        None => true,
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
    pinned::place(&partial, &it.dest, it.subpath.as_deref(), &it.repo, it.commit.as_deref()).map_err(InstallError)
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

/// The kora program a root-run job may execute (see `binary::choose_root_exe`): the recorded copy only
/// when it really is Kora's root-owned pinned copy, else this program when it is safe, else a copy
/// of it that has to be made first.
fn root_exe_for(created: &Created) -> Result<service::binary::RootExe> {
    service::binary::root_exe(created.binary.as_deref()).map_err(InstallError)
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
            // Done before the engine starts (see `prepare_account`).
            "prepare.account" => true,
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
                let sel: Vec<Resource> = select_resources(&cat, &ctx.req.resources, ctx.req.mode == Mode::Development)?.into_iter().cloned().collect();
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
            "prepare.account" => Ok("Already done".into()),
            "prepare.tools" => self.prepare_tools(),
            "build.forks" => self.forks(),
            "build.resources" => self.resources(),
            "build.db" => self.database(),
            "build.artifact" => self.artifact(),
            "build.config" => self.config(),
            "system.firewall" => self.firewall(),
            "system.service" => self.service(),
            "system.schedule" => self.schedule(),
            "system.app" => self.app(),
            "finalize" => self.finalize(),
            "launch" => self.launch(),
            other => err(format!("unknown step {other}")),
        }
    }

    /// `gh auth login --web`: show the one-time code and open the GitHub page, then wait for the user.
    fn sign_in_to_github(&self) -> Result<()> {
        use git::owners::{begin_login, GhAuth, SystemLogin};
        let ctx = &*self.ctx;
        ctx.emit(self.id, StepStatus::Running, "Starting the GitHub sign-in...", 0.2);
        let session = begin_login(&SystemLogin, false).map_err(|e| InstallError(format!("could not start the GitHub sign-in: {e}")))?;
        let opened = open_github_page(&session.url);
        let msg = format!(
            "Sign in to GitHub: {} {} and enter this code: {}  (waiting for you, up to 15 minutes)",
            if opened { "your browser is opening" } else { "open" },
            session.url,
            session.code
        );
        ctx.note(format!("GitHub sign-in code {} at {}", session.code, session.url));
        ctx.emit(self.id, StepStatus::Running, msg, 0.5);
        match session.wait() {
            Ok(GhAuth::LoggedIn { login }) => {
                let _ = git::CommandRunner::run(&git::SystemRunner, "gh", &["auth", "setup-git", "--hostname", "github.com"], None);
                ctx.emit(self.id, StepStatus::Running, format!("Signed in to GitHub as {login}"), 0.8);
                Ok(())
            }
            Ok(_) => err("GitHub sign-in was not completed (the code expired or was refused). Run the installer again to retry."),
            Err(e) => err(format!("GitHub sign-in failed: {e}")),
        }
    }

    fn prepare_tools(&self) -> Result<String> {
        let ctx = &*self.ctx;
        let r = git::SystemRunner;
        let dev = ctx.req.mode == Mode::Development;
        let mut tools = git::detect(&r);
        if tools.git.is_none() || (dev && tools.gh.is_none()) {
            let plan = git::plan_bootstrap(git::Os::current(), &tools).map_err(|e| InstallError(e.to_string()))?;
            let no_winget = cfg!(windows) && !plan.is_empty() && !winget_available();
            if no_winget && !ctx.admin {
                return err(format!(
                    "git{} is missing and this Windows has no winget (Windows Package Manager). Kora can download the official installer{} itself, but that needs administrator rights: right-click Windows Terminal > 'Run as administrator' and run Kora again. Or install {} yourself ({}) and run Kora again.",
                    if dev { " / the GitHub CLI" } else { "" },
                    if dev { "s" } else { "" },
                    if dev { "them" } else { "it" },
                    if dev { "https://git-scm.com/download/win, https://cli.github.com" } else { "https://git-scm.com/download/win" }
                ));
            }
            if no_winget {
                // No winget: the vendors' own installers, downloaded over HTTPS and run only if their SHA-256 matches.
                let tmp = windows_tools::private_tmp_dir()?;
                let mut say = |m: &str| ctx.emit(self.id, StepStatus::Running, m.to_string(), 0.1);
                let mut wanted = Vec::new();
                if tools.git.is_none() {
                    wanted.push(windows_tools::Tool::Git);
                }
                if dev && tools.gh.is_none() {
                    wanted.push(windows_tools::Tool::Gh);
                }
                for t in wanted {
                    windows_tools::install(t, &ctx.hooks.http, &tmp, &mut say).map_err(|e| InstallError(format!("{e}. You can also install {} yourself from {} and run Kora again.", t.label(), t.manual_url())))?;
                }
                let _ = std::fs::remove_dir_all(&tmp);
                tools = git::detect(&r);
                if tools.git.is_none() {
                    return err("git was installed but this program cannot find it yet. Close this window, open a new one and run the install again.");
                }
            }
            if !no_winget && !ctx.admin {
                let what: Vec<String> = plan.iter().map(|s| format!("- {}", s.description)).collect();
                return err(format!(
                    "git{} is missing. Installing it needs administrator rights; re-run as root/Administrator, or install it yourself:\n{}",
                    if dev { " / GitHub CLI" } else { "" },
                    what.join("\n")
                ));
            }
            for s in plan.iter().filter(|_| !no_winget) {
                ctx.emit(self.id, StepStatus::Running, s.description.clone(), 0.1);
                let args: Vec<&str> = s.args.iter().map(String::as_str).collect();
                let o = git::CommandRunner::run(&r, &s.program, &args, None).map_err(|e| InstallError(format!("{}: {e}", s.description)))?;
                if !(o.ok() || (s.program == "winget" && winget_code_is_fine(o.code))) {
                    return err(format!("{} failed: {}", s.description, o.stderr.trim()));
                }
            }
            tools = git::detect(&r);
            if tools.git.is_none() {
                return err("git was installed but this program cannot find it yet. Close this window, open a new one and run the install again.");
            }
        }
        if dev {
            if tools.gh.is_none() {
                return err("the GitHub CLI (gh) is required for development installs");
            }
            let mut st = git::auth_status(&r).map_err(|e| InstallError(e.to_string()))?;
            if !st.logged_in {
                // Just installed (or never signed in): do the browser sign-in now instead of failing.
                self.sign_in_to_github()?;
                st = git::auth_status(&r).map_err(|e| InstallError(e.to_string()))?;
                if !st.logged_in {
                    return err("GitHub sign-in did not finish. Run the installer again to retry; your answers are saved.");
                }
            }
            let missing = st.missing_scopes(false);
            if !missing.is_empty() {
                return err(format!("Your GitHub sign-in lacks the scopes {}. Run `gh auth refresh -h github.com -s {}` and try again.", missing.join(", "), missing.join(",")));
            }
            verify_requested_owner(&r, ctx.req.github_owner.as_deref())?;
        }
        // Ports.
        let preq = PortRequest { server: slug(&ctx.req.name), game_port: ctx.req.ports.game, txadmin_port: ctx.req.ports.txadmin, expose_txadmin: ctx.req.txadmin_public };
        let already_installed = ctx.req.dir.join("kora-server.json").is_file();
        if !already_installed {
            preq.validate().map_err(|e| InstallError(e.to_string()))?;
            let conflicts = firewall::ports::install_conflicts(&ctx.req.name, &ctx.req.dir, preq.game_port, preq.txadmin_port, &firewall::ports::SystemProber);
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
                    self.start_stopped_db(&found[0].name)?;
                } else if cfg!(windows) && windows_db::saved_admin().is_none() {
                    ctx.note(format!(
                        "Using the database server '{}' that is already on this computer. Kora needs its administrator login to create a NEW database and user: pass --db-admin-user (and KORA_DB_ADMIN_PASSWORD), or use --db install to let Kora set up its own private MariaDB.",
                        found[0].name
                    ));
                }
                check_admin_login(&admin_conn(&ctx.req))?;
            }
        }
        Ok("git, database and ports are ready".into())
    }

    /// A detected database service that is not running: start it when Kora owns it (Windows), else say how.
    fn start_stopped_db(&self, name: &str) -> Result<()> {
        let ctx = &*self.ctx;
        if cfg!(windows) {
            if let Some(rec) = windows_db::saved_record().filter(|r| r.service == name) {
                ctx.emit(self.id, StepStatus::Running, format!("Starting Kora's MariaDB service {name}"), 0.5);
                windows_db::ensure_running(&rec)?;
                if let Some(a) = windows_db::saved_admin() {
                    windows_db::wait_for_login(&a.to_conn(), std::time::Duration::from_secs(90))?;
                }
                return Ok(());
            }
            return err(format!("{name} is installed but not running. Start it (as administrator: `net start {name}`) and try again."));
        }
        err(format!("{name} is installed but not running. Start it (systemctl start {name}) and try again."))
    }

    fn install_db_server(&self) -> Result<()> {
        let ctx = &*self.ctx;
        if db::Os::current() == db::Os::Windows {
            if !ctx.admin {
                return err(crate::elevate::not_elevated_db_message(
                    windows_db::MARIADB_VERSION,
                    &windows_db::base_dir().map(|p| p.display().to_string()).unwrap_or_else(|| "%ProgramData%\\Kora\\mariadb".into()),
                ));
            }
            let mut say = |m: &str| ctx.emit(self.id, StepStatus::Running, m.to_string(), 0.5);
            let rec = windows_db::install(&ctx.hooks.http, &mut say)?;
            ctx.note(format!(
                "Kora installed its own MariaDB {} as the Windows service '{}' (127.0.0.1:{}, in {}). `kora uninstall --delete-databases` removes it again.",
                rec.version,
                rec.service,
                rec.port,
                rec.root.display()
            ));
            return Ok(());
        }
        let plan = match db::plan_install(db::Os::current(), &ctx.req.dir, "", 3306, &[]) {
            Ok(p) if db::Os::current() == db::Os::Linux => p,
            _ => return err("Automatic MariaDB installation is not available on this system. Install MariaDB yourself, then use --db existing."),
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
        let repos: Vec<String> = ctx.shared.lock().unwrap().selected.iter().filter(|x| !x.is_pinned_copy()).map(|x| repo_slug(&x.repository)).collect();
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
        // Which branch each fork is compared/synced on, and where its resource folder is.
        let sel: Vec<(String, String, String)> =
            ctx.shared.lock().unwrap().selected.iter().filter(|x| !x.is_pinned_copy()).map(|x| (repo_slug(&x.repository), x.branch.clone(), x.install_rel())).collect();
        for (repo, branch, _) in &sel {
            state.branches.insert(repo.clone(), branch.clone());
        }
        let total = repos.len().max(1);
        let save = |s: &git::ForkState| {
            if let Ok(b) = serde_json::to_vec_pretty(s) {
                let _ = crate::fsutil::write_no_follow(&path, &b, 0o644);
            }
        };
        save(&state);
        ctx.emit(self.id, StepStatus::Running, format!("Looking for forks you already have in {}", state.target_owner), 0.0);
        let mut on_progress = |s: &git::ForkState| {
            save(s);
            let what = s.last_event.clone().unwrap_or_else(|| format!("Checking your forks in {}", s.target_owner));
            ctx.emit(self.id, StepStatus::Running, format!("{what} ({}/{total})", s.done.len()), s.done.len() as f32 / total as f32);
        };
        let cfg = git::ForkConfig { sync_existing: true, cache_path: Some(git::forkfind::default_cache_path()), ..git::ForkConfig::default() };
        let out = git::run_forks(&r, &mut state, &cfg, &mut on_progress).map_err(|e| InstallError(e.to_string()))?;
        match out {
            git::ForkOutcome::Paused => err("GitHub rate limit reached. Progress is saved: run the installer again later to resume forking."),
            git::ForkOutcome::Finished if !state.failed.is_empty() => err(format!(
                "{} forks failed (first: {} - {}). Run again to retry them.",
                state.failed.len(),
                state.failed[0].repo,
                state.failed[0].error
            )),
            git::ForkOutcome::Finished => {
                let rel_of = |repo: &str| sel.iter().find(|(r, _, _)| r == repo).map(|(_, _, rel)| format!("resources/{rel}")).unwrap_or_else(|| repo.to_string());
                let (headline, lines) = fork_sync_report(&state, &rel_of);
                for l in &lines {
                    ctx.emit(self.id, StepStatus::Running, l.clone(), 1.0);
                    ctx.note(l.clone());
                }
                Ok(format!("{} forks ready in {} ({}){headline}", state.done.len(), state.target_owner, state.counts_text()))
            }
        }
    }

    fn resources(&self) -> Result<String> {
        let ctx = &*self.ctx;
        let items = items(ctx)?;
        let total = items.len();
        io(std::fs::create_dir_all(res_dir(ctx)), "cannot create resources folder")?;
        io(layout::create_empty_dirs(&res_dir(ctx)), "cannot create the add-on folders")?;
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
        // Folders, never single resources (see `layout`).
        let ensure: Vec<String> = layout::ensure_for_new(ctx.shared.lock().unwrap().selected.iter().map(|r| r.install_rel()).collect::<Vec<_>>().iter().map(String::as_str));
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
                // The password itself is never put in a note or log: it is in the file, and the final screen shows it.
                ctx.note(format!(
                    "txAdmin login (no Cfx.re account was linked): user '{username}', password in {}. Change it after the first login.",
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
            io(crate::fsutil::write_no_follow(&gi, format!("{existing}{add}").as_bytes(), 0o644), "cannot write .gitignore")?;
        }
        // The live console's helper (a failure here must never fail the install: the console is optional).
        if let Err(e) = crate::console::install(dir) {
            ctx.note(format!("The live console helper could not be added ({e}); you can turn it on later in the manager."));
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

        // Which program does the service run? Not one in a folder ordinary users can write to, and
        // not simply the one `created.json` names (that file is in the server folder; see root_exe_for).
        let exe = std::env::current_exe().map_err(|e| InstallError(format!("cannot find the kora program: {e}")))?;
        let choice = root_exe_for(&created)?;
        let own_copy = match &choice {
            service::binary::RootExe::Ready(p) if *p == exe => None,
            other => Some(other.path().to_path_buf()),
        };
        let kora_exe = choice.path().to_string_lossy().into_owned();

        let server = managed_server(ctx, &account, &kora_exe)?;
        let mut head = Vec::new(); // folder access + account + program copy
        if platform == SvcPlatform::Linux && ctx.hooks.fix_permissions {
            // --fix-permissions: exactly the private folders on the way to the server folder (see preflight).
            let blocked = super::service_blocking_dirs(&ctx.req.dir);
            if !blocked.is_empty() {
                ctx.note(format!(
                    "Let the server's own account pass through {} (chmod o+x) so it can reach the server folder.",
                    blocked.iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join(", ")
                ));
            }
            head.extend(service::binary::traverse_plan(&blocked).steps);
        }
        if create_account {
            head.extend(service::account::create_plan(&account).map_err(|e| InstallError(e.to_string()))?.steps);
        }
        if let service::binary::RootExe::Copy { src, dest } = &choice {
            head.extend(service::binary::install_plan(&src.to_string_lossy(), dest).steps);
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

    /// Dev mode: daily 04:00 `kora update --auto <dir>` as the installing user (their GitHub
    /// sign-in works for private forks); configurable later with `kora schedule`.
    fn schedule(&self) -> Result<String> {
        let ctx = &*self.ctx;
        let created = ctx.load_created();
        let user = created.service_account.clone().unwrap_or_else(|| format!("kora-{}", slug(&ctx.req.name)));
        let exe = root_exe_for(&created)?;
        let server = managed_server(ctx, &user, &exe.path().to_string_lossy())?;
        let cfg = service::ScheduleConfig::default_for_install();
        let who = service::installing_user();
        let mut plan = service::Plan::default();
        if let service::binary::RootExe::Copy { src, dest } = &exe {
            plan.extend(service::binary::install_plan(&src.to_string_lossy(), dest));
        }
        plan.extend(service::schedule_install_plan(&server, &cfg, &who, svc_platform()).map_err(|e| InstallError(e.to_string()))?);
        if !ctx.admin {
            ctx.note(format!("Auto-update schedule NOT created (needs administrator rights). Re-run as root/Administrator to: {}", plan.descriptions().join("; ")));
            return Ok("Skipped (needs admin); see notes".into());
        }
        ctx.update_created(|c| {
            c.update_timer = true;
            if let service::binary::RootExe::Copy { dest, .. } = &exe {
                c.binary = Some(dest.to_string_lossy().into_owned());
            }
        })?;
        service::apply_plan(&plan, &service::SystemRunner).map_err(|e| InstallError(e.to_string()))?;
        service::save_schedule(&ctx.req.dir, &cfg, &who).map_err(|e| InstallError(e.to_string()))?;
        Ok(format!("Auto-update scheduled: {} (change it with `kora schedule set {}`)", service::describe_for(&cfg, &who), server.name))
    }

    /// Last step: start the server and wait until txAdmin answers. A server that does not come up is
    /// reported (detail + outcome), never an error: the install itself succeeded and must not roll back.
    fn launch(&self) -> Result<String> {
        let ctx = &*self.ctx;
        let spec = crate::launch::Spec { name: ctx.req.name.clone(), dir: ctx.req.dir.clone(), game_port: ctx.req.ports.game, txadmin_port: ctx.req.ports.txadmin };
        let report = crate::launch::start(&crate::launch::SystemEnv::default(), &spec, &mut |m, f| ctx.emit("launch", StepStatus::Running, m, f));
        let detail = match report.problem {
            None if report.game_listening => "Your server is running (txAdmin is up and the game port is listening)".to_string(),
            None => "txAdmin is up; the game port is not listening yet".to_string(),
            Some(_) => format!("Not running: {}", report.message),
        };
        ctx.shared.lock().unwrap().launch = Some(report);
        Ok(detail)
    }

    /// The Kora app (command + shortcuts). Machine-wide, so it needs administrator rights, and it is
    /// never fatal: a server that installed fine must not be reported as failed because of a shortcut.
    fn app(&self) -> Result<String> {
        use crate::app_install as app;
        let ctx = &*self.ctx;
        let choice = ctx.req.app.unwrap_or_default();
        let opts = app::AppOptions { desktop_shortcut: choice.desktop_shortcut, force: false, dry_run: false };
        if !ctx.admin {
            let what = app::Running::current()
                .and_then(|run| {
                    let l = app::Layout::from_env(&run.exe)?;
                    Ok(app::plan_install(&app::RealSys::new(), &l, &run, &opts).actions.iter().map(app::Action::describe).collect::<Vec<_>>().join("; "))
                })
                .unwrap_or_default();
            ctx.note(format!("The Kora app was NOT installed (needs administrator rights). Re-run as root/Administrator, or run `kora app install`, to: {what}"));
            return Ok("Skipped (needs admin); see notes".into());
        }
        match app::install_app(&opts) {
            Ok(out) => {
                for n in &out.notes {
                    ctx.note(n.clone());
                }
                if out.changed() {
                    ctx.note(if cfg!(windows) {
                        "Kora is installed as an app: open \"Kora\" from the Start Menu to manage your servers later.".to_string()
                    } else {
                        "Run `kora` any time to manage your servers.".to_string()
                    });
                    Ok(format!("Kora app installed at {}", out.exe.display()))
                } else {
                    Ok("The Kora app is already installed and up to date".into())
                }
            }
            Err(e) => {
                ctx.note(format!("The Kora app could not be installed ({e}). Your server is fine; try `kora app install` later."));
                Ok("Skipped (see notes)".into())
            }
        }
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
                path: r.install_rel(),
                repository: r.repository.clone(),
                branch: r.branch.clone(),
                commit: match (req.mode == Mode::Production || r.is_pinned_copy(), r.resolve(catalog::Mode::Prod)) {
                    (true, ResolvedVersion::Pinned { commit, .. }) => Some(commit),
                    _ => None,
                },
                third_party: r.is_pinned_copy(),
                ..Default::default()
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
        // A server in the `kora` account's home without a service belongs to `kora` (with a service the
        // service step already handed it to the service's own account).
        if ctx.admin && !req.service && ctx.plan.iter().any(|p| p.id == "prepare.account") {
            let plan = crate::userhome::chown_to_kora_plan(std::slice::from_ref(&req.dir), true);
            service::apply_plan(&plan, &service::SystemRunner).map_err(|e| InstallError(e.to_string()))?;
        }
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
                // `created.resources` comes from a file the service account can change: every entry
                // must stay inside `<dir>/resources`, and nothing on the way may be a link.
                let dir = &ctx.req.dir;
                let res = Path::new("resources");
                for rel in &created.resources {
                    let r = res.join(rel);
                    match crate::fsutil::remove_inside(dir, &r) {
                        Ok(_) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                            ctx.note(format!("Kept '{rel}' from Kora's records: {e}."));
                            continue;
                        }
                        Err(e) => return err(format!("cannot remove a resource folder: {e}")),
                    }
                    let name = r.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let _ = crate::fsutil::remove_inside(dir, &r.with_file_name(format!("{name}.kora-partial")));
                    let mut parent = r.parent().map(Path::to_path_buf);
                    while let Some(p) = parent.filter(|p| p != res && p.starts_with(res)) {
                        if !crate::fsutil::remove_empty_dir_inside(dir, &p) {
                            break;
                        }
                        parent = p.parent().map(Path::to_path_buf);
                    }
                }
                if !crate::fsutil::is_symlink(&res_dir(ctx)) {
                    layout::remove_empty_dirs(&res_dir(ctx));
                }
                crate::fsutil::remove_empty_dir_inside(dir, res);
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
                let _ = crate::fsutil::remove_inside(&ctx.req.dir, Path::new("artifacts"));
                Ok("Removed the server build".into())
            }
            "build.config" if !ctx.keep_files => {
                for f in &created.files {
                    if f == "database.json" || f == "kora-server.json" {
                        continue;
                    }
                    // Only inside the server folder, never through a link (the list is in a file other accounts can change).
                    if let Err(e) = crate::fsutil::remove_inside(&ctx.req.dir, Path::new(f)) {
                        if e.kind() == std::io::ErrorKind::PermissionDenied {
                            ctx.note(format!("Kept '{f}' from Kora's records: {e}."));
                        }
                    }
                }
                Ok("Removed generated configuration".into())
            }
            "system.firewall" if created.firewall => {
                let preq = PortRequest { server: slug(&ctx.req.name), game_port: ctx.req.ports.game, txadmin_port: ctx.req.ports.txadmin, expose_txadmin: ctx.req.txadmin_public };
                let rn = service::SystemRunner;
                if let Some(kind) = firewall::detect(svc_platform(), &rn) {
                    let ssh = if svc_platform() == SvcPlatform::Linux { firewall::ssh_ports() } else { vec![] };
                    let (plan, note) = firewall::close_plan_checked(&preq, kind, &rn, &ssh).map_err(|e| InstallError(e.to_string()))?;
                    if let Some(n) = note {
                        ctx.note(n);
                    }
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
            "launch" => {
                let spec = crate::launch::Spec { name: ctx.req.name.clone(), dir: ctx.req.dir.clone(), game_port: ctx.req.ports.game, txadmin_port: ctx.req.ports.txadmin };
                let stopped = crate::launch::stop_detached(&crate::launch::SystemEnv::default(), &spec);
                Ok(if stopped { "Stopped the server".into() } else { "Nothing was running".into() })
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
    let _ = crate::fsutil::remove_inside(&ctx.req.dir, Path::new(".kora"));
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

    /// A fake `gh` that is signed in as `me` (member of org `Team`); records every call.
    struct SignedInGh {
        calls: std::sync::Mutex<Vec<String>>,
    }
    impl git::CommandRunner for SignedInGh {
        fn run(&self, p: &str, a: &[&str], _: Option<&std::path::Path>) -> std::io::Result<git::Output> {
            let line = format!("{p} {}", a.join(" "));
            self.calls.lock().unwrap().push(line.clone());
            let ok = |s: &str| Ok(git::Output { code: 0, stdout: s.into(), stderr: String::new() });
            match line.as_str() {
                "gh auth status --hostname github.com" => ok(""),
                "gh api user" => ok(r#"{"login":"me"}"#),
                l if l.starts_with("gh api user/orgs") => ok(r#"[{"login":"Team"}]"#),
                l if l.starts_with("gh api user/memberships/orgs/") => ok(r#"{"state":"active","role":"member"}"#),
                l if l.starts_with("gh api orgs/") => ok(r#"{"members_can_create_repositories":true}"#),
                _ => Ok(git::Output { code: 1, stdout: String::new(), stderr: "unexpected".into() }),
            }
        }
        fn sleep(&self, _: std::time::Duration) {}
    }

    #[test]
    fn after_sign_in_a_valid_owner_passes_and_none_is_skipped() {
        let g = SignedInGh { calls: Default::default() };
        assert!(verify_requested_owner(&g, Some("team")).is_ok());
        assert!(verify_requested_owner(&g, Some("ME")).is_ok());
        let none = SignedInGh { calls: Default::default() };
        assert!(verify_requested_owner(&none, None).is_ok());
        assert!(none.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn after_sign_in_an_unknown_owner_fails_before_any_fork() {
        let g = SignedInGh { calls: Default::default() };
        let e = verify_requested_owner(&g, Some("nobody")).unwrap_err().to_string();
        assert!(e.contains("'nobody' is not your GitHub account"), "{e}");
        assert!(e.contains("Team") && e.contains("Nothing was forked") && e.contains("--github-owner"), "{e}");
        assert!(!g.calls.lock().unwrap().iter().any(|c| c.contains("repo fork")), "no fork may be attempted");
        assert!(verify_requested_owner(&g, Some("-bad")).is_err());
    }

    #[test]
    fn winget_already_installed_codes_count_as_success() {
        assert!(winget_code_is_fine(0));
        assert!(winget_code_is_fine(0x8A15_0061_u32 as i32), "already installed");
        assert!(winget_code_is_fine(0x8A15_010D_u32 as i32), "no applicable upgrade");
        assert!(!winget_code_is_fine(1));
        assert!(!winget_code_is_fine(0x8A15_0003_u32 as i32));
    }
}
