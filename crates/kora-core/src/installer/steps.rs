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
use crate::problem::{codes, known, Action, Problem, Tool};
use crate::service::{self, ManagedServer, Platform as SvcPlatform};
use crate::steps::StepResult;
use crate::txadmin::{self, CfxIdentity, DbConnection, ServerConfig};

const CLONE_WORKERS: usize = 4;

pub(crate) fn build(ctx: &Arc<Ctx>) -> Vec<Box<dyn Step>> {
    ctx.plan.iter().map(|p| Box::new(InstallStep { ctx: ctx.clone(), id: p.id, title: p.title }) as Box<dyn Step>).collect()
}

/// The elevated part of one step, in the administrator helper's child (see `system_ops`): `ctx`
/// is an administrator context whose record is a shadow copy, so nothing is written into the
/// server folder. Returns the step's detail and, for the database steps, what the person's process
/// must write.
pub(crate) fn admin_part(ctx: &Arc<Ctx>, undo: bool, step: &str) -> Result<(String, Option<super::system_ops::DbResult>)> {
    let id: &'static str = match step {
        "prepare.db" => "prepare.tools",
        "build.db" => "build.db",
        "system.firewall" => "system.firewall",
        "system.service" => "system.service",
        "system.schedule" => "system.schedule",
        other => return err(format!("'{other}' is not a step Kora's administrator helper does")),
    };
    let st = InstallStep { ctx: ctx.clone(), id, title: "" };
    match (undo, step) {
        (true, "build.db") => st.undo_database(),
        (true, _) => st.undo().map(|d| (d, None)),
        (false, "prepare.db") => st.prepare_db().map(|_| ("The database server is ready".to_string(), None)),
        (false, "build.db") => st.database_elevated(),
        (false, _) => st.exec().map(|d| (d, None)),
    }
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
                // The progress line gets the plain message; the error keeps the whole Problem (if it is one).
                self.ctx.emit(self.id, StepStatus::Failed, e.problem().message, 0.0);
                Err(StepError::Failed(e.0))
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

/// The registered server (other than `dir`) whose records name the same database or database
/// user as `st`. `st` comes from a folder the server's account can change: it must not be able to
/// make `kora delete` drop another server's database.
fn db_used_elsewhere(reg: &Registry, st: &ProvisionState, dir: &Path) -> Option<String> {
    reg.servers.iter().filter(|s| !registry::same_path(&s.dir, dir)).find_map(|s| {
        let other = ProvisionState::load(&super::meta_dir(&s.dir).join("db-provision.json")).ok()?;
        (other.database == st.database || other.user == st.user).then(|| s.name.clone())
    })
}

fn db_client() -> String {
    let ok = |c: &str| crate::sysexe::command(c).arg("--version").output().map(|o| o.status.success()).unwrap_or(false);
    let saved = if cfg!(windows) { windows_db::saved_record().map(|r| r.client).filter(|c| c.is_file()) } else { None };
    pick_db_client(cfg!(windows), crate::elevate::is_elevated(), &ok, saved, &|| windows_client_candidates().into_iter().find(|c| c.is_file()))
}

/// Which database client to run. An elevated Kora on Windows never takes one from PATH (a folder
/// on it may be the person's own, and a `mysql.exe` planted there would run as administrator):
/// only Kora's own MariaDB's client (its record lives in an administrators-only folder) or one
/// in Program Files.
fn pick_db_client(windows: bool, elevated: bool, on_path: &dyn Fn(&str) -> bool, saved: Option<PathBuf>, installed: &dyn Fn() -> Option<PathBuf>) -> String {
    let path_allowed = !(windows && elevated);
    if path_allowed {
        for c in ["mariadb", "mysql"] {
            if on_path(c) {
                return c.into();
            }
        }
    }
    if windows {
        if let Some(c) = saved.or_else(installed) {
            return c.to_string_lossy().into_owned();
        }
    }
    "mysql".into()
}

/// Open GitHub's device-login page in the default browser. Only ever a github.com address, never a shell.
fn open_github_page(url: &str) -> bool {
    if !url.starts_with("https://github.com/") || url.contains(|c: char| c.is_whitespace() || c.is_control() || c == '"') {
        return false;
    }
    let (program, args): (&str, Vec<&str>) = if cfg!(windows) { ("explorer.exe", vec![url]) } else { ("xdg-open", vec![url]) };
    crate::sysexe::command(program).args(args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().is_ok()
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
    login_problem(&admin).map_or(Ok(()), |why| Err(format!("could not log in to the database server as '{}' ({why}). Check the user, password and port, or pick another option.", admin.user)))
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
        "Give the database administrator login, or (from a terminal) run as root with MariaDB unix-socket auth or pass --db-admin-user / --db-admin-host / --db-admin-port with KORA_DB_ADMIN_PASSWORD."
    };
    Err(Problem::new(codes::INSTALL_DB_LOGIN, format!("Could not log in to the database server as '{}'. Kora needs an administrator account to create a NEW database and user. {advice}", admin.user))
        .with_details(why)
        .with_action(Action::change_answers("database"))
        .into())
}

/// No database server, and Kora will not install one with the current answers.
fn db_missing(what_to_do: &str) -> Problem {
    Problem::new(codes::INSTALL_DB_MISSING, format!("No MariaDB or MySQL database server was found on this computer. {what_to_do}"))
        .with_details("From a terminal: --db install (needs administrator rights) or --db skip.")
        .with_action(Action::change_answers("database"))
}

/// A database service that is installed but stopped: the command that starts it.
fn db_stopped(name: &str, start: &[&str]) -> Problem {
    Problem::new(codes::INSTALL_DB_STOPPED, format!("The database server {name} is installed but not running. Start it, then try this step again."))
        .with_action(Action::StartDatabase)
        .with_action(Action::show_command(start, true))
}

/// Installing the database server needs administrator rights.
fn db_needs_admin(req: &InstallRequest, why: String) -> Problem {
    Problem::new(codes::NEEDS_ELEVATION, "Installing the database server needs administrator rights.")
        .with_details(why)
        .with_action(Action::RequestAdmin)
        .with_action(Action::show_command(&crate::api::install::install_command(req), true))
        .with_action(Action::change_answers("database"))
}

/// Git is missing, and the system package manager (administrator rights) is the way to get it.
fn git_needs_admin() -> Problem {
    Problem::new(codes::INSTALL_TOOL_MISSING, "Git is not installed, and installing it here needs administrator rights.")
        .with_action(Action::RequestAdmin)
        .with_action(Action::open_link(known::GIT_URL))
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
    git::owners::check_login_syntax(want).map_err(|m| InstallError::from(Problem::new(codes::GH_OWNER_INVALID, m).with_action(Action::change_answers("github"))))?;
    let all = git::owners::list_owners(r).map_err(|e| {
        InstallError::from(
            Problem::new(codes::GH_UNAVAILABLE, "Kora could not read your GitHub accounts. Nothing was forked.").with_details(e.to_string()).with_action(Action::SignInGithub { scopes: vec![] }),
        )
    })?;
    match git::owners::choose_owner(&all, Some(want)) {
        Ok(_) => Ok(()),
        Err(m) => Err(Problem::new(codes::GH_OWNER_INVALID, format!("{m}\nNothing was forked. Choose another GitHub account to fork into."))
            .with_details("From a terminal: `kora github owners` lists the accounts; pass one with --github-owner.")
            .with_action(Action::change_answers("github"))
            .into()),
    }
}

fn fork_owner(ctx: &Ctx, r: &dyn git::CommandRunner) -> Result<(String, bool)> {
    use git::owners::{check_org, gh_auth, GhAuth};
    let login = match gh_auth(r) {
        GhAuth::LoggedIn { login } => login,
        GhAuth::Missing => return Err(known::gh_missing().into()),
        GhAuth::LoggedOut => return Err(known::github_signed_out().into()),
    };
    match ctx.req.github_owner.as_deref() {
        None => Ok((login, false)),
        Some(o) if o.eq_ignore_ascii_case(&login) => Ok((login, false)),
        Some(o) => {
            let org = check_org(r, o);
            if org.can_fork {
                Ok((org.login, true))
            } else {
                Err(Problem::new(codes::GH_OWNER_INVALID, format!("You cannot fork into '{o}': {}. Choose another GitHub account to fork into.", org.reason.as_deref().unwrap_or("not allowed")))
                    .with_details(format!("From a terminal: `kora github owners` lists the accounts, or leave out --github-owner to fork into {login}."))
                    .with_action(Action::change_answers("github"))
                    .into())
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
            license_key_name: None,
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

/// The account a Linux service runs as (without `KORA_SERVICE_USER`): always this server's own
/// `kora-<name>` system account. `recorded` (from `created.json`, which the server's account can
/// change) is never taken on trust: a forged `"alice"` would make the server run as alice and
/// root `chown -R` the folder to her. Returns (account, create it?, note).
fn linux_service_account(recorded: Option<&str>, expected: &str, runner: &dyn service::CommandRunner, uid_min: u32) -> Result<(String, bool, Option<String>)> {
    use service::account::{is_system_uid, user_status, UserStatus};
    let note =
        recorded.filter(|r| *r != expected).map(|r| format!("Kora's records named the account '{r}' for this server; that is not the account Kora makes for it, so '{expected}' is used instead."));
    match user_status(runner, expected).map_err(|e| InstallError(e.to_string()))? {
        UserStatus::Exists(0) => err(format!("'{expected}' is the root account: refusing to run a server as it. Remove it or choose another server name.")),
        // A system account with this server's name: the one Kora made in an earlier attempt.
        UserStatus::Exists(uid) if is_system_uid(uid, uid_min) => Ok((expected.to_string(), false, note)),
        UserStatus::Exists(uid) => err(format!(
            "'{expected}' already exists as an ordinary user account (uid {uid}), not a system account Kora made: Kora will not run the server as it or give it the server folder. Remove it or choose another server name."
        )),
        UserStatus::Missing => Ok((expected.to_string(), true, note)),
    }
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
    txadmin::txadmin_env(&ctx.req.dir.join("txData"), ctx.req.ports.txadmin).into_iter().map(|(k, v)| format!("{k}={v}\n")).collect()
}

// ---------------------------------------------------------------------------------------------
// verify / exec / undo
// ---------------------------------------------------------------------------------------------

impl InstallStep {
    /// Not elevated, but an administrator helper is there (the Windows panel's worker).
    fn helper(&self) -> Option<&super::system_ops::SharedAdmin> {
        if self.ctx.admin {
            None
        } else {
            self.ctx.hooks.admin.as_ref()
        }
    }

    /// Does this install's database work go to the helper (Kora's own MariaDB login)?
    fn db_helper(&self) -> bool {
        super::system_ops::db_needs_helper(db::Os::current() == db::Os::Windows, self.ctx.admin, self.ctx.hooks.admin.is_some(), db_override(&self.ctx.req))
    }

    /// Have the administrator helper do `step` (see `system_ops`): its lines become this step's
    /// progress, the record it hands back is written here (by the person's own process), and a
    /// failure there is this step's failure, with the same problem.
    fn via_helper(&self, undo: bool, step: &str) -> Result<super::system_ops::StepResult> {
        let ctx = &*self.ctx;
        let Some(h) = self.helper() else { return err("no administrator helper is available") };
        let known = (undo && ctx.system_trusted).then(|| (ctx.req.name.clone(), ctx.req.ports.game, ctx.req.ports.txadmin));
        let args = super::system_ops::step_args(undo, step, &ctx.req.dir, ctx.keep_files, ctx.keep_db, &known);
        ctx.emit(self.id, StepStatus::Running, "Working with administrator rights", 0.2);
        let out = h.0.admin_step(&args, &mut |l| ctx.emit(self.id, StepStatus::Running, l.to_string(), 0.5))?;
        let Some(r) = out.data.and_then(|v| serde_json::from_value::<super::system_ops::StepResult>(v).ok()) else {
            return err(if out.ok { "Kora's administrator helper did not report what it did." } else { "Kora's administrator helper could not do this step." });
        };
        if let Some(c) = &r.created {
            ctx.update_created(|x| *x = c.clone())?;
        }
        for n in &r.notes {
            ctx.note(n.clone());
        }
        if let Some(p) = r.error.clone() {
            return Err(p.into());
        }
        if !out.ok {
            return err("Kora's administrator helper could not do this step.");
        }
        Ok(r)
    }

    fn is_done(&self) -> bool {
        let ctx = &self.ctx;
        match self.id {
            // Done before the engine starts (see `prepare_account`).
            "prepare.account" => true,
            "prepare.dir" => ctx.req.dir.is_dir() && ctx.meta_dir().is_dir(),
            "build.forks" => std::fs::read(forks_path(ctx)).ok().and_then(|b| serde_json::from_slice::<git::ForkState>(&b).ok()).is_some_and(|s| s.is_complete()),
            "build.resources" => match items(ctx) {
                Ok(v) => !v.is_empty() && v.iter().all(item_ok),
                Err(_) => false,
            },
            "build.db" => ProvisionState::load(&db_state_path(ctx)).map(|s| s.granted).unwrap_or(false) && ctx.req.dir.join("database.json").is_file(),
            "build.artifact" => artifact_store(ctx).current_dir().ok().flatten().is_some_and(|d| d.join(ArtPlatform::current().executable()).is_file()),
            "build.config" => ["server.cfg", "permissions.cfg", "secrets.cfg", "txData/default/config.json"].iter().all(|f| ctx.req.dir.join(f).is_file()),
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
                io(crate::fsutil::create_dir_no_follow(&ctx.meta_dir()), "cannot create the server folder")?;
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
        let session = begin_login(&SystemLogin, false).map_err(|e| {
            InstallError::from(Problem::new(codes::GH_SIGN_IN_INCOMPLETE, "Kora could not start the GitHub sign-in.").with_details(e.to_string()).with_action(Action::SignInGithub { scopes: vec![] }))
        })?;
        let opened = open_github_page(&session.url);
        let msg = format!("Sign in to GitHub: {} {} and enter this code: {}  (waiting for you, up to 15 minutes)", if opened { "your browser is opening" } else { "open" }, session.url, session.code);
        ctx.note(format!("GitHub sign-in code {} at {}", session.code, session.url));
        ctx.emit(self.id, StepStatus::Running, msg, 0.5);
        match session.wait() {
            Ok(GhAuth::LoggedIn { login }) => {
                let _ = git::CommandRunner::run(&git::SystemRunner, "gh", &["auth", "setup-git", "--hostname", "github.com"], None);
                ctx.emit(self.id, StepStatus::Running, format!("Signed in to GitHub as {login}"), 0.8);
                Ok(())
            }
            Ok(_) => {
                Err(Problem::new(codes::GH_SIGN_IN_INCOMPLETE, "The GitHub sign-in was not completed (the code expired or was refused).").with_action(Action::SignInGithub { scopes: vec![] }).into())
            }
            Err(e) => Err(Problem::new(codes::GH_SIGN_IN_INCOMPLETE, "The GitHub sign-in did not finish.").with_details(e.to_string()).with_action(Action::SignInGithub { scopes: vec![] }).into()),
        }
    }

    fn prepare_tools(&self) -> Result<String> {
        let ctx = &*self.ctx;
        let r = git::SystemRunner;
        let dev = ctx.req.mode == Mode::Development;
        let mut tools = git::detect(&r);
        // Kora's own copies (no administrator rights, no PATH change): the GitHub CLI wherever it is missing and
        // Git on Windows. They are checked against pinned checksums before anything is unpacked.
        let mut wanted = Vec::new();
        if tools.git.is_none() && crate::tools::pins::pin_here(Tool::Git).is_some() {
            wanted.push(Tool::Git);
        }
        if dev && tools.gh.is_none() {
            wanted.push(Tool::Gh);
        }
        for t in wanted {
            let mut say = |m: &str, f: f32| ctx.emit(self.id, StepStatus::Running, m.to_string(), 0.05 + 0.1 * f);
            crate::tools::install(t, &crate::tools::HttpFetcher(ctx.hooks.http.clone()), &mut say)?;
        }
        tools = git::detect(&r);
        // Linux Git: the system package, which needs administrator rights.
        if tools.git.is_none() {
            let plan = git::plan_bootstrap(git::Os::current(), &tools).map_err(|e| InstallError(e.to_string()))?;
            if !ctx.admin {
                let what: Vec<String> = plan.iter().map(|s| format!("- {}", s.description)).collect();
                let mut p = git_needs_admin().with_details(format!("Kora would:\n{}", what.join("\n")));
                for s in &plan {
                    p.push_action(Action::ShowCommand { command: std::iter::once(s.program.clone()).chain(s.args.iter().cloned()).collect(), elevated: true });
                }
                return Err(p.into());
            }
            for s in &plan {
                ctx.emit(self.id, StepStatus::Running, s.description.clone(), 0.1);
                let args: Vec<&str> = s.args.iter().map(String::as_str).collect();
                let failed = |why: String| InstallError::from(Problem::new(codes::INSTALL_TOOL_MISSING, format!("This did not work: {}.", s.description)).with_details(why));
                let o = git::CommandRunner::run(&r, &s.program, &args, None).map_err(|e| failed(e.to_string()))?;
                if !o.ok() {
                    return Err(failed(o.stderr.trim().to_string()));
                }
            }
            tools = git::detect(&r);
            if tools.git.is_none() {
                return Err(known::git_missing().into());
            }
        }
        if dev {
            if tools.gh.is_none() {
                return Err(Problem::new(codes::GH_MISSING, "Development servers need the GitHub CLI (gh), and it is not installed.")
                    .with_actions(known::gh_missing().actions)
                    .with_action(Action::change_answers("mode"))
                    .into());
            }
            let mut st = git::auth_status(&r).map_err(|e| InstallError(e.to_string()))?;
            if !st.logged_in {
                // Just installed (or never signed in): do the browser sign-in now instead of failing.
                self.sign_in_to_github()?;
                st = git::auth_status(&r).map_err(|e| InstallError(e.to_string()))?;
                if !st.logged_in {
                    return Err(Problem::new(codes::GH_SIGN_IN_INCOMPLETE, "The GitHub sign-in did not finish.").with_action(Action::SignInGithub { scopes: vec![] }).into());
                }
            }
            let missing = st.missing_scopes(false);
            if !missing.is_empty() {
                return Err(known::github_missing_scopes(&missing).into());
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
                return Err(super::ports_busy(&conflicts).into());
            }
        }
        // Database server.
        if ctx.req.db != DbChoice::Skip {
            if self.db_helper() {
                self.via_helper(false, "prepare.db")?;
            } else {
                self.prepare_db()?;
            }
        }
        Ok("git, database and ports are ready".into())
    }

    /// The database server part of `prepare.tools`: install or start it when Kora may, and check
    /// that Kora can log in as its administrator.
    fn prepare_db(&self) -> Result<()> {
        let ctx = &*self.ctx;
        {
            if db_override(&ctx.req) {
                check_admin_login(&admin_conn(&ctx.req))?;
            } else {
                let found = db::detect(&db::SystemRunner, db::Os::current()).map_err(|e| InstallError(e.to_string()))?;
                if found.is_empty() {
                    match ctx.req.db {
                        DbChoice::InstallIfMissing => self.install_db_server()?,
                        _ => return Err(db_missing("Choose \"Set up for me\" to let Kora install one, or skip the database.").into()),
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
        Ok(())
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
            return Err(db_stopped(name, &["net", "start", name]).into());
        }
        Err(db_stopped(name, &["systemctl", "start", name]).into())
    }

    fn install_db_server(&self) -> Result<()> {
        let ctx = &*self.ctx;
        if db::Os::current() == db::Os::Windows {
            if !ctx.admin {
                let why = crate::elevate::not_elevated_db_message(
                    windows_db::MARIADB_VERSION,
                    &windows_db::base_dir().map(|p| p.display().to_string()).unwrap_or_else(|| "%ProgramData%\\Kora\\mariadb".into()),
                );
                return Err(db_needs_admin(&ctx.req, why).into());
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
            _ => return Err(db_missing("Kora cannot install MariaDB on this system: install it yourself, then choose to use the existing database.").into()),
        };
        if !ctx.admin {
            let what: Vec<String> = plan.steps.iter().map(|s| format!("- {}", s.description)).collect();
            return Err(db_needs_admin(&ctx.req, format!("MariaDB is not installed, and installing it needs administrator rights. Kora would:\n{}", what.join("\n"))).into());
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
            git::ForkOutcome::Paused => {
                Err(Problem::new(codes::GH_RATE_LIMITED, "GitHub asked Kora to slow down. The forks made so far are kept: wait a few minutes, then try this step again.").into())
            }
            git::ForkOutcome::Finished if !state.failed.is_empty() => Err(Problem::new(
                codes::INSTALL_FORKS_FAILED,
                format!("{} of your forks could not be made. The others are kept; trying this step again only retries the missing ones.", state.failed.len()),
            )
            .with_details(state.failed.iter().map(|f| format!("{}: {}", f.repo, f.error)).collect::<Vec<_>>().join("\n"))
            .into()),
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

    /// `build.db` by the administrator helper: it creates the database and the login with Kora's
    /// own MariaDB admin login (which never leaves it) and hands back the new login, which is
    /// written here, by the person's own process.
    fn database_via_helper(&self) -> Result<String> {
        let ctx = &*self.ctx;
        let r = self.via_helper(false, "build.db")?;
        let db = r.db.ok_or_else(|| InstallError("Kora's administrator helper did not hand back the new database login.".into()))?;
        let state_path = db_state_path(ctx);
        if db.clear_old_state {
            let _ = std::fs::remove_file(&state_path);
        }
        let (Some(creds), Some(state)) = (db.credentials, db.state) else {
            return err("Kora's administrator helper did not hand back the new database login.");
        };
        io(std::fs::create_dir_all(ctx.meta_dir()), "cannot create state folder")?;
        state.save(&state_path).map_err(|e| InstallError(e.to_string()))?;
        let json = serde_json::to_string_pretty(&creds).map_err(|e| InstallError(e.to_string()))?;
        let path = ctx.req.dir.join("database.json");
        ctx.update_created(|c| c.files.push("database.json".into()))?;
        db::write_secret_file(&db::SystemRunner, db::Os::current(), &path, &json, windows_user().as_deref()).map_err(|e| InstallError(e.to_string()))?;
        ctx.shared.lock().unwrap().db_name = Some(creds.database.clone());
        Ok(r.detail)
    }

    /// `build.db` inside the administrator helper's child: the same work as [`Self::database`],
    /// but nothing is written into the server folder (the state goes to a private temporary file
    /// and comes back with the login).
    fn database_elevated(&self) -> Result<(String, Option<super::system_ops::DbResult>)> {
        let ctx = &*self.ctx;
        let admin = admin_conn(&ctx.req);
        let mut res = super::system_ops::DbResult::default();
        if let Ok(mut old) = ProvisionState::load(&db_state_path(ctx)) {
            ctx.emit(self.id, StepStatus::Running, "Cleaning up an unfinished earlier attempt", 0.1);
            db::rollback(&db::SystemRunner, &admin, &mut old).map_err(|e| InstallError(e.to_string()))?;
            res.clear_old_state = true;
        }
        // A scratch file only administrators can touch (the person's TEMP is theirs to change
        // between the write and the read): Kora's MariaDB folder, under an unguessable name.
        let mut rnd = [0u8; 16];
        getrandom::fill(&mut rnd).map_err(|e| InstallError(format!("no random numbers: {e}")))?;
        let scratch = windows_db::base_dir().filter(|b| b.is_dir()).unwrap_or_else(std::env::temp_dir);
        let tmp = scratch.join(format!("kora-db-state-{}.json", hex::encode(rnd)));
        ctx.emit(self.id, StepStatus::Running, "Creating database and user", 0.4);
        let made = db::provision(&db::SystemRunner, &admin, "127.0.0.1", "127.0.0.1", &tmp);
        let state = ProvisionState::load(&tmp).ok();
        let _ = std::fs::remove_file(&tmp);
        let creds = match made {
            Ok(c) => c,
            Err(e) => {
                // Nothing may stay behind that no record names: drop what this attempt made.
                if let Some(mut st) = state {
                    let _ = db::rollback(&db::SystemRunner, &admin, &mut st);
                }
                return Err(InstallError(e.to_string()));
            }
        };
        ctx.emit(self.id, StepStatus::Running, "Testing the new login", 0.8);
        verify_login(&admin.client, &creds)?;
        let detail = format!("Database {} created for user {}", creds.database, creds.user);
        res.state = state;
        res.credentials = Some(creds);
        Ok((detail, Some(res)))
    }

    fn database(&self) -> Result<String> {
        if self.db_helper() {
            return self.database_via_helper();
        }
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
        db::write_secret_file(&db::SystemRunner, db::Os::current(), &path, &json, windows_user().as_deref()).map_err(|e| InstallError(e.to_string()))?;
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
                ctx.note(format!("txAdmin login (no Cfx.re account was linked): user '{username}', password in {}. Change it after the first login.", f.display()));
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
        if self.helper().is_some() {
            return self.via_helper(false, "system.firewall").map(|r| r.detail);
        }
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
        if self.helper().is_some() {
            let r = self.via_helper(false, "system.service")?;
            if r.grants_left {
                // The opt-in helper registers the service but never changes permissions inside a
                // folder the person controls: the folder's owner (this process) grants them.
                let ctx = &*self.ctx;
                let server = managed_server(ctx, &format!("kora-{}", slug(&ctx.req.name)), "kora.exe")?;
                ctx.emit(self.id, StepStatus::Running, "Letting the service into the server folder", 0.9);
                service::apply_plan(&service::windows_grant_plan(&server), &service::SystemRunner).map_err(|e| InstallError(e.to_string()))?;
            }
            return Ok(r.detail);
        }
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
                let expected = service::account::account_name(&slug(&ctx.req.name));
                let (name, create, note) = linux_service_account(created.service_account.as_deref(), &expected, &rn, service::account::uid_min())?;
                if let Some(n) = note {
                    ctx.note(n);
                }
                (name, create)
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
        }
        if platform == SvcPlatform::Windows {
            server.validate().map_err(|e| InstallError(e.to_string()))?;
            let sid = service::windows_installing_sid(&rn);
            let mut reg = service::windows_register_plan_for(&server, sid.as_deref());
            match service::windows_existing_service(&rn, &server) {
                service::Existing::Missing => {}
                // An earlier attempt registered it and stopped before recording it: use it.
                service::Existing::Ours => reg.steps.retain(|s| !s.command_line().is_some_and(|l| l.starts_with("sc.exe create "))),
                service::Existing::Other(bin) => {
                    return err(format!(
                        "A Windows service called '{}' already exists and runs something else ({bin}). Kora will not change it. Choose another server name, or remove that service yourself.",
                        server.service_name()
                    ))
                }
            }
            tail.extend(reg.steps);
            // In the opt-in helper the person's own process grants the folder rights (see above).
            if !crate::helper::winhelper::in_service() {
                tail.extend(service::windows_grant_plan(&server).steps);
            }
        } else {
            tail.extend(service::install_plan(&server, platform).map_err(|e| InstallError(e.to_string()))?.steps);
        }

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
        // Stage 2: ownership + unit. `chown -R` by path: never once a part of it became a planted link.
        super::check_dir_links(&ctx.req.dir)?;
        let unit = server.unit_path();
        ctx.update_created(|c| c.service_unit = Some(unit))?;
        if platform == SvcPlatform::Linux {
            // Hand the server folder (and only it) to the server's own account, through open folder
            // handles: `chown -R <path>` would follow a link swapped in after the check above.
            let (uid, gid) = crate::userhome::account_ids(&account).ok_or_else(|| InstallError(format!("cannot find the account '{account}'")))?;
            ctx.emit(self.id, StepStatus::Running, &format!("Giving '{account}' ownership of the server folder (and only that folder)"), 0.8);
            crate::fsutil::chown_tree_no_follow(&ctx.req.dir, uid, gid).map_err(|e| InstallError(format!("cannot give '{account}' the server folder: {e}")))?;
        }
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
        if self.helper().is_some() {
            let r = self.via_helper(false, "system.schedule")?;
            // The helper made the task; the schedule's record is the person's file.
            service::save_schedule(&ctx.req.dir, &service::ScheduleConfig::default_for_install(), &service::installing_user()).map_err(|e| InstallError(e.to_string()))?;
            return Ok(r.detail);
        }
        let created = ctx.load_created();
        // Only a label in the unit (the update job itself runs as root); never the recorded name.
        let user = service::account::account_name(&slug(&ctx.req.name));
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
        // In the administrator helper's child the person's own process writes this file.
        if ctx.shadow.is_none() {
            service::save_schedule(&ctx.req.dir, &cfg, &who).map_err(|e| InstallError(e.to_string()))?;
        }
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
            super::check_dir_links(&req.dir)?;
            let plan = crate::userhome::chown_to_kora_plan(std::slice::from_ref(&req.dir), true);
            service::apply_plan(&plan, &service::SystemRunner).map_err(|e| InstallError(e.to_string()))?;
        }
        Ok(format!("Registered '{}' in {}", req.name, registry::registry_path().display()))
    }

    // ---- rollback -------------------------------------------------------------------------

    /// Undo `build.db` inside the administrator helper's child: drop the database and its login
    /// with Kora's own admin login. The state file and the login file are updated by the person's
    /// own process from what this hands back.
    fn undo_database(&self) -> Result<(String, Option<super::system_ops::DbResult>)> {
        let ctx = &*self.ctx;
        if ctx.keep_db {
            return Ok(("Database kept".into(), None));
        }
        let Ok(mut st) = ProvisionState::load(&db_state_path(ctx)) else { return Ok(("No database to remove".into(), None)) };
        let (dbn, usr) = (st.database.clone(), st.user.clone());
        if let Some(other) = db_used_elsewhere(&Registry::load().unwrap_or_default(), &st, &ctx.req.dir) {
            ctx.note(format!("Database '{dbn}' / user '{usr}' were kept: the server '{other}' uses them too (this folder's records name them)."));
            return Ok(("Database kept".into(), None));
        }
        let admin = admin_conn(&ctx.req);
        db::rollback(&db::SystemRunner, &admin, &mut st).map_err(|e| InstallError(format!("could not remove database '{dbn}' / user '{usr}': {e}")))?;
        Ok((format!("Dropped database {dbn} and user {usr}"), Some(super::system_ops::DbResult { state: Some(st), remove_secret_file: true, ..Default::default() })))
    }

    fn undo(&self) -> Result<String> {
        let ctx = &*self.ctx;
        let created = ctx.load_created();
        if !ctx.system_trusted {
            let services = " (`systemctl list-units 'kora-*'` lists Kora's services)";
            let what = match self.id {
                "system.firewall" if created.firewall => Some(("firewall rules", "")),
                "system.service" if created.service_unit.is_some() || created.service_account_created || created.binary.is_some() => {
                    Some(("background service, its system account and the program copy", services))
                }
                "system.schedule" if created.update_timer => Some(("automatic-update schedule", services)),
                _ => None,
            };
            if let Some((what, hint)) = what {
                ctx.note(format!(
                    "Kora left this server's {what} alone: {} is not in Kora's server list, so Kora cannot be sure which system-wide changes belong to it (the folder's own records can be changed by the server's account). Remove them by hand if needed{hint}.",
                    ctx.req.dir.display()
                ));
                return Ok("Left alone (not in the server list; see notes)".into());
            }
        }
        // Not elevated, with the administrator helper: it removes the system-wide things.
        if self.helper().is_some() {
            let any = match self.id {
                "system.firewall" => created.firewall,
                "system.service" => created.service_unit.is_some() || created.service_account_created || created.binary.is_some(),
                "system.schedule" => created.update_timer,
                "build.db" => !ctx.keep_db && self.db_helper() && db_state_path(ctx).is_file(),
                _ => false,
            };
            if any && self.id == "build.db" {
                let r = self.via_helper(true, "build.db")?;
                if let Some(st) = r.db.as_ref().and_then(|d| d.state.as_ref()) {
                    st.save(&db_state_path(ctx)).map_err(|e| InstallError(e.to_string()))?;
                }
                if r.db.as_ref().is_some_and(|d| d.remove_secret_file) {
                    let _ = std::fs::remove_file(ctx.req.dir.join("database.json"));
                }
                return Ok(r.detail);
            }
            if any {
                return self.via_helper(true, self.id).map(|r| r.detail);
            }
        }
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
                if let Some(other) = db_used_elsewhere(&Registry::load().unwrap_or_default(), &st, &ctx.req.dir) {
                    ctx.note(format!("Database '{dbn}' / user '{usr}' were kept: the server '{other}' uses them too (this folder's records name them)."));
                    return Ok("Database kept".into());
                }
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
                let items: Vec<system::Item> = system::items_for(&target, &created, svc_platform(), &rn, ctx.keep_files, used)?.into_iter().filter(|i| wanted.contains(&i.kind)).collect();
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
                // By folder, not by the (possibly forged) recorded name: only this server's entry goes.
                if let Ok(mut reg) = Registry::load() {
                    let n = reg.servers.len();
                    reg.servers.retain(|s| !registry::same_path(&s.dir, &ctx.req.dir));
                    if reg.servers.len() != n {
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
        args: ["--batch", "--skip-column-names", "--protocol=tcp", "-h", &c.host, "-P", &c.port.to_string(), "-u", &c.user, &c.database].iter().map(|s| s.to_string()).collect(),
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

    #[test]
    fn an_elevated_kora_on_windows_never_takes_the_database_client_from_path() {
        let everywhere = |_: &str| true;
        let saved = Some(PathBuf::from(r"C:\ProgramData\Kora\mariadb\11.4\bin\mariadb.exe"));
        assert_eq!(pick_db_client(true, true, &everywhere, saved.clone(), &|| None), r"C:\ProgramData\Kora\mariadb\11.4\bin\mariadb.exe");
        assert_eq!(pick_db_client(true, true, &everywhere, None, &|| Some(PathBuf::from(r"C:\Program Files\MariaDB 11\bin\mysql.exe"))), r"C:\Program Files\MariaDB 11\bin\mysql.exe");
        assert_eq!(pick_db_client(true, false, &everywhere, saved, &|| None), "mariadb", "not elevated: PATH as before");
        assert_eq!(pick_db_client(false, true, &everywhere, None, &|| None), "mariadb", "Linux root: sudo's secure PATH");
    }

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
    fn a_database_another_server_uses_is_never_dropped() {
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a"), t.path().join("b"));
        let st = |db: &str| ProvisionState { database: db.into(), user: db.into(), host: "127.0.0.1".into(), created_database: true, created_user: true, granted: true };
        for (d, db) in [(&a, "kora_aaaa"), (&b, "kora_bbbb")] {
            std::fs::create_dir_all(d.join(".kora")).unwrap();
            st(db).save(&d.join(".kora/db-provision.json")).unwrap();
        }
        let entry = |n: &str, d: &PathBuf| ServerEntry { name: n.into(), dir: d.clone(), mode: Mode::Production, ports: PortsRecord { game: 1, txadmin: 2 }, created_unix: 0 };
        let reg = Registry { servers: vec![entry("A", &a), entry("B", &b)] };
        // A's account forged A's records to name B's database: refused.
        assert_eq!(db_used_elsewhere(&reg, &st("kora_bbbb"), &a).as_deref(), Some("B"));
        // A's own database: fine (A's own records are not "elsewhere").
        assert_eq!(db_used_elsewhere(&reg, &st("kora_aaaa"), &a), None);
    }

    #[test]
    fn a_rerun_install_never_takes_the_recorded_service_account_on_trust() {
        use crate::service::testing::FakeRunner;
        let r = FakeRunner::default();
        r.respond("id -u kora-srv", 1, "");
        // A forged "alice" (or root) in created.json: ignored, this server's own account is made.
        for forged in ["alice", "root", "kora-other"] {
            let (name, create, note) = linux_service_account(Some(forged), "kora-srv", &r, 1000).unwrap();
            assert_eq!((name.as_str(), create), ("kora-srv", true), "{forged}");
            assert!(note.unwrap().contains(forged));
        }
        // An earlier attempt made the account: reused, not created again, no note.
        let r = FakeRunner::default();
        r.respond("id -u kora-srv", 0, "998\n");
        assert_eq!(linux_service_account(Some("kora-srv"), "kora-srv", &r, 1000).unwrap(), ("kora-srv".into(), false, None));
        assert_eq!(linux_service_account(None, "kora-srv", &r, 1000).unwrap(), ("kora-srv".into(), false, None));
        // A person's account (or root) with that name: refused.
        let r = FakeRunner::default();
        r.respond("id -u kora-srv", 0, "1001\n");
        assert!(linux_service_account(None, "kora-srv", &r, 1000).unwrap_err().to_string().contains("ordinary user account"));
        let r = FakeRunner::default();
        r.respond("id -u kora-srv", 0, "0\n");
        assert!(linux_service_account(None, "kora-srv", &r, 1000).unwrap_err().to_string().contains("root account"));
    }
}

/// The install's failures say what happened and what the user can do, never "close this window".
#[cfg(test)]
mod problem_tests {
    use super::*;
    use crate::git::Output;
    use std::time::Duration;

    struct Gh(fn(&str) -> std::io::Result<Output>);
    impl git::CommandRunner for Gh {
        fn run(&self, p: &str, a: &[&str], _: Option<&Path>) -> std::io::Result<Output> {
            (self.0)(&format!("{p} {}", a.join(" ")))
        }
        fn sleep(&self, _: Duration) {}
    }
    fn out(code: i32, stdout: &str) -> std::io::Result<Output> {
        Ok(Output { code, stdout: stdout.into(), stderr: String::new() })
    }

    fn dev_ctx(owner: Option<&str>) -> Arc<Ctx> {
        let d = std::env::temp_dir().join("kora-problem-tests");
        let req = InstallRequest {
            mode: Mode::Development,
            name: "s".into(),
            dir: d,
            hostname: "s".into(),
            license_key: None,
            license_key_name: None,
            cfx_identifier: None,
            ports: Ports::default(),
            txadmin_public: false,
            artifact_url: None,
            game_build: None,
            resources: ResourceSelection::All,
            db: DbChoice::Skip,
            service: false,
            firewall: false,
            github_owner: owner.map(String::from),
            catalog: None,
            dry_run: true,
            db_admin: None,
            start: false,
            app: None,
        };
        new_ctx(&req, &Hooks::default(), false, std::sync::mpsc::channel().0, false, false)
    }

    fn problem_of<T: std::fmt::Debug>(r: Result<T>) -> Problem {
        r.unwrap_err().problem()
    }

    #[test]
    fn gh_missing_signed_out_and_owner_problems_are_told_apart() {
        let ctx = dev_ctx(None);
        let missing = Gh(|_| Err(std::io::Error::new(std::io::ErrorKind::NotFound, "no gh")));
        let p = problem_of(fork_owner(&ctx, &missing));
        assert_eq!(p.code, codes::GH_MISSING);
        assert!(p.actions.contains(&Action::InstallTool { tool: Tool::Gh }), "{p:?}");
        let signed_out = Gh(|_| out(1, ""));
        let p = problem_of(fork_owner(&ctx, &signed_out));
        assert_eq!((p.code.as_str(), p.actions.clone()), (codes::GH_SIGNED_OUT, vec![Action::SignInGithub { scopes: vec![] }]));
        // Accounts cannot be read: sign in again (the panel adds "try this step again").
        let broken = Gh(|_| out(1, "HTTP 401"));
        let p = problem_of(verify_requested_owner(&broken, Some("someone")));
        assert_eq!(p.code, codes::GH_UNAVAILABLE);
        assert!(!p.message.to_lowercase().contains("installer again"), "{}", p.message);
        // An owner that is not one of the user's accounts: change the answer.
        let me_only = Gh(|l| match l {
            "gh api user" => out(0, r#"{"login":"me","id":1}"#),
            l if l.starts_with("gh api user/orgs") => out(0, "[]"),
            _ => out(0, ""),
        });
        let p = problem_of(verify_requested_owner(&me_only, Some("stranger")));
        assert_eq!(p.code, codes::GH_OWNER_INVALID);
        assert!(p.actions.contains(&Action::change_answers("github")));
        assert!(verify_requested_owner(&me_only, Some("me")).is_ok());
    }

    #[test]
    fn tool_and_database_problems_offer_fitting_actions() {
        let p = git_needs_admin();
        assert_eq!(p.code, codes::INSTALL_TOOL_MISSING);
        assert_eq!(p.actions[0], Action::RequestAdmin);
        assert!(!p.actions.contains(&Action::InstallTool { tool: Tool::Gh }), "gh is never installed by the system package manager");

        let p = db_stopped("mariadb", &["systemctl", "start", "mariadb"]);
        assert_eq!(p.actions, vec![Action::StartDatabase, Action::show_command(&["systemctl", "start", "mariadb"], true)]);
        let p = db_missing("Choose another option.");
        assert_eq!((p.code.as_str(), p.actions.clone()), (codes::INSTALL_DB_MISSING, vec![Action::change_answers("database")]));
        let p = db_needs_admin(&dev_ctx(None).req, "why".into());
        assert_eq!(p.code, codes::NEEDS_ELEVATION);
        assert!(matches!(&p.actions[1], Action::ShowCommand { command, elevated: true } if command[0] == "kora" && command[1] == "install"));
    }

    #[test]
    fn a_problem_reads_as_plain_text_in_a_terminal() {
        let e = InstallError::from(known::github_missing_scopes(&["read:org".to_string()]));
        let t = e.to_string();
        assert!(t.starts_with("Your GitHub sign-in does not give Kora") && t.contains("gh auth refresh -h github.com -s read:org"), "{t}");
        assert!(!t.contains("kora-problem"));
        assert_eq!(e.problem().code, codes::GH_MISSING_SCOPES);
        // Through the step engine too.
        let se = StepError::Failed(e.0.clone());
        assert_eq!(InstallError::from(se).problem().code, codes::GH_MISSING_SCOPES);
    }
}
