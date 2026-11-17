//! Glue for `kora update --auto`: a [`GitOps`] adapter over the git module, a service-based
//! health check, and [`auto_update`] which wires the update engine to a registered server.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::registry::{now_unix, Registry, ServerFile};
use super::types::Versions;
use super::{slug, InstallError, Result};
use crate::firewall;
use crate::git::{self, CommandRunner, SystemRunner};
use crate::service::{self, ManagedServer, ServiceManager};
use crate::update::{
    self, FetchMode, GitOps, GitResult, HealthCheck, HealthOutcome, HealthRequest, NoMigrations, QuietConfig, ResourceClass, RestartDecision, RunOptions, UpdateError, UpdateReport, UpdateTarget,
};

/// `GitOps` implemented by shelling out to `git` through the git module's runner.
pub struct GitCli<R: CommandRunner = SystemRunner>(pub R);

impl Default for GitCli<SystemRunner> {
    fn default() -> Self {
        GitCli(SystemRunner)
    }
}

fn gerr(e: impl std::fmt::Display) -> UpdateError {
    UpdateError::Git(e.to_string())
}

impl<R: CommandRunner> GitCli<R> {
    fn git(&self, repo: &Path, args: &[&str]) -> GitResult<git::Output> {
        let o = self.0.run("git", args, Some(repo)).map_err(gerr)?;
        if o.ok() {
            Ok(o)
        } else {
            Err(UpdateError::Git(format!("git {} failed: {}", args.join(" "), o.stderr.trim())))
        }
    }
    fn count(&self, repo: &Path, range: &str) -> GitResult<u32> {
        let o = self.git(repo, &["rev-list", "--count", range])?;
        o.stdout.trim().parse().map_err(|_| UpdateError::Git(format!("unexpected rev-list output {:?}", o.stdout)))
    }
}

/// One fetch. A connection that goes quiet for 30 s is abandoned, so one dead remote cannot hold the whole check.
fn fetch_with(r: &dyn CommandRunner, repo: &Path, remote: &str) -> GitResult<()> {
    let args = ["-c", "http.lowSpeedLimit=1000", "-c", "http.lowSpeedTime=30", "fetch", remote];
    let o = r.run("git", &args, Some(repo)).map_err(gerr)?;
    if o.ok() {
        Ok(())
    } else {
        Err(UpdateError::Git(format!("git fetch {remote} failed: {}", o.stderr.trim())))
    }
}

/// Fetches made by this process, so an apply right after a preview does not fetch the same repositories again.
const RECENT_FETCH: Duration = Duration::from_secs(120);
type RecentMap = std::sync::Mutex<std::collections::HashMap<PathBuf, std::time::Instant>>;
fn recent_map() -> &'static RecentMap {
    static M: std::sync::OnceLock<RecentMap> = std::sync::OnceLock::new();
    M.get_or_init(Default::default)
}
fn recently_fetched(p: &Path) -> bool {
    recent_map().lock().unwrap_or_else(|e| e.into_inner()).get(p).is_some_and(|t| t.elapsed() < RECENT_FETCH)
}
fn note_fetched(p: &Path, r: &GitResult<()>) {
    let mut m = recent_map().lock().unwrap_or_else(|e| e.into_inner());
    match r {
        Ok(()) => {
            m.insert(p.to_path_buf(), std::time::Instant::now());
        }
        Err(_) => {
            m.remove(p);
        }
    }
}

impl<R: CommandRunner> GitOps for GitCli<R> {
    fn fetch(&self, repo: &Path) -> GitResult<()> {
        self.git(repo, &["fetch", "upstream"]).map(|_| ())
    }
    fn head_commit(&self, repo: &Path, rev: &str) -> GitResult<String> {
        if rev == "HEAD" {
            return git::head_commit(&self.0, repo).map_err(gerr);
        }
        Ok(self.git(repo, &["rev-parse", rev])?.stdout.trim().to_string())
    }
    fn fast_forward(&self, repo: &Path, upstream: &str) -> GitResult<()> {
        self.git(repo, &["merge", "--ff-only", upstream]).map(|_| ())
    }
    fn reset_to(&self, repo: &Path, commit: &str) -> GitResult<()> {
        git::reset_to_commit(&self.0, repo, commit).map_err(gerr)
    }
    fn is_dirty(&self, repo: &Path) -> GitResult<bool> {
        Ok(!self.git(repo, &["status", "--porcelain"])?.stdout.trim().is_empty())
    }
    fn is_diverged(&self, repo: &Path, upstream: &str) -> GitResult<bool> {
        Ok(self.count(repo, &format!("{upstream}..HEAD"))? > 0)
    }
    fn behind_count(&self, repo: &Path, upstream: &str) -> GitResult<u32> {
        self.count(repo, &format!("HEAD..{upstream}"))
    }
    fn is_ancestor(&self, repo: &Path, ancestor: &str, descendant: &str) -> GitResult<bool> {
        let o = self.0.run("git", &["merge-base", "--is-ancestor", ancestor, descendant], Some(repo)).map_err(gerr)?;
        match o.code {
            0 => Ok(true),
            1 => Ok(false),
            _ => Err(UpdateError::Git(format!("could not compare {ancestor} with {descendant}: {}", o.stderr.trim()))),
        }
    }
    fn fetch_origin(&self, repo: &Path) -> GitResult<()> {
        self.git(repo, &["fetch", "origin"]).map(|_| ())
    }
    fn current_branch(&self, repo: &Path) -> GitResult<Option<String>> {
        git::branches::current(&self.0, repo).map_err(gerr)
    }
    fn switch_branch(&self, repo: &Path, branch: &str) -> GitResult<()> {
        git::branches::switch_to(&self.0, repo, branch).map_err(gerr)
    }
    fn has_ref(&self, repo: &Path, rev: &str) -> bool {
        self.git(repo, &["rev-parse", "--verify", "--quiet", &format!("{rev}^{{commit}}")]).is_ok()
    }
    fn remote_slug(&self, repo: &Path, remote: &str) -> Option<String> {
        self.git(repo, &["remote", "get-url", remote]).ok().and_then(|o| git::slug_of_url(o.stdout.trim()))
    }
    fn fetch_many(&self, targets: &[&UpdateTarget], mode: FetchMode) -> Vec<GitResult<()>> {
        // Development: `fetch upstream`. Production: the target is a pinned commit; a checkout that already has it needs no network.
        let is_commit = |r: &str| r.len() >= 7 && r.chars().all(|c| c.is_ascii_hexdigit());
        let recent = |p: &Path| mode == FetchMode::AllowRecent && recently_fetched(p);
        let mut out: Vec<Option<GitResult<()>>> = targets.iter().map(|_| None).collect();
        let mut jobs: Vec<(usize, &'static str, Box<dyn CommandRunner + Send>)> = Vec::new();
        for (i, t) in targets.iter().enumerate() {
            let remote = if is_commit(&t.upstream_ref) {
                if self.has_ref(&t.path, &t.upstream_ref) {
                    out[i] = Some(Ok(()));
                    continue;
                }
                "origin"
            } else {
                "upstream"
            };
            if recent(&t.path) {
                out[i] = Some(Ok(()));
                continue;
            }
            match self.0.fork() {
                Some(r) => jobs.push((i, remote, r)),
                // A runner that cannot be copied runs on this thread.
                None => {
                    let r = self.git(&t.path, &["fetch", remote]).map(|_| ());
                    note_fetched(&t.path, &r);
                    out[i] = Some(r);
                }
            }
        }
        let paths: Vec<&Path> = targets.iter().map(|t| t.path.as_path()).collect();
        let done = update::par_map(jobs, update::FETCH_PARALLEL, |(i, remote, r)| (i, fetch_with(&r, paths[i], remote)));
        for (i, r) in done {
            note_fetched(paths[i], &r);
            out[i] = Some(r);
        }
        out.into_iter().map(|r| r.unwrap_or(Ok(()))).collect()
    }
    fn move_pinned(&self, repo: &Path, commit: &str) -> GitResult<()> {
        git::move_pinned(&self.0, repo, commit).map(|_| ()).map_err(gerr)
    }
    fn sync_forks(&self, targets: &[UpdateTarget], progress: &mut dyn FnMut(usize, usize)) -> Vec<(String, git::SyncOutcome)> {
        // Needs gh and a signed-in user; otherwise the forks are simply left as they are.
        let slug = |repo: &Path, remote: &str| self.git(repo, &["remote", "get-url", remote]).ok().and_then(|o| git::slug_of_url(o.stdout.trim()));
        let jobs: Vec<git::SyncJob> = targets
            .iter()
            .filter_map(|t| {
                let branch = t.upstream_ref.strip_prefix("upstream/")?.to_string();
                let (fork, upstream) = (slug(&t.path, "origin")?, slug(&t.path, "upstream")?);
                // Not a fork of someone else's repository (the user works on the original): nothing to sync.
                (!fork.eq_ignore_ascii_case(&upstream)).then(|| git::SyncJob { id: t.id.clone(), fork, upstream, branch })
            })
            .collect();
        if jobs.is_empty() || !git::is_signed_in(&self.0) {
            return Vec::new();
        }
        git::sync_forks_paced(&self.0, &jobs, &git::ForkConfig { sync_existing: true, ..git::ForkConfig::default() }, progress)
    }
    fn log_subjects(&self, repo: &Path, from: &str, to: &str, max: usize) -> Vec<String> {
        let range = format!("{from}..{to}");
        let n = format!("-{max}");
        match self.git(repo, &["log", "--format=%s", &n, &range]) {
            Ok(o) => o.stdout.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect(),
            Err(_) => Vec::new(),
        }
    }
}

/// Runs every command as `user` (via `runuser`) with `HOME` set to a folder that user owns.
/// Used when the update runs as root (systemd timer) so git never creates root-owned files in
/// a server folder owned by the service account, and never trips git's "dubious ownership" check.
#[derive(Clone)]
pub struct AsUser {
    pub user: String,
    pub home: PathBuf,
}

impl CommandRunner for AsUser {
    fn run(&self, program: &str, args: &[&str], cwd: Option<&Path>) -> std::io::Result<git::Output> {
        let home = format!("HOME={}", self.home.display());
        let mut a: Vec<&str> = vec!["-u", &self.user, "--", "env", &home, program];
        a.extend_from_slice(args);
        SystemRunner.run("runuser", &a, cwd)
    }
    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }
    fn fork(&self) -> Option<Box<dyn CommandRunner + Send>> {
        Some(Box::new(self.clone()))
    }
}

/// Adds `-c safe.directory=*` to every `git` call. The Windows scheduled task runs as SYSTEM in a
/// folder owned by the person who installed, which git otherwise refuses ("dubious ownership").
pub struct SafeDirectory<R: CommandRunner>(pub R);

impl<R: CommandRunner> CommandRunner for SafeDirectory<R> {
    fn run(&self, program: &str, args: &[&str], cwd: Option<&Path>) -> std::io::Result<git::Output> {
        if program == "git" {
            let mut a: Vec<&str> = vec!["-c", "safe.directory=*"];
            a.extend_from_slice(args);
            return self.0.run(program, &a, cwd);
        }
        self.0.run(program, args, cwd)
    }
    fn sleep(&self, d: Duration) {
        self.0.sleep(d)
    }
    fn fork(&self) -> Option<Box<dyn CommandRunner + Send>> {
        self.0.fork().map(|r| Box::new(SafeDirectory(r)) as Box<dyn CommandRunner + Send>)
    }
}

/// Name of the owner of `dir` when it is not the current user (Unix only; needs root to switch).
#[cfg(unix)]
fn foreign_owner(dir: &Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let uid = std::fs::metadata(dir).ok()?.uid();
    let me = String::from_utf8_lossy(&std::process::Command::new("id").arg("-u").output().ok()?.stdout).trim().parse::<u32>().ok()?;
    if me != 0 || uid == 0 {
        return None;
    }
    let o = std::process::Command::new("id").args(["-nu", &uid.to_string()]).output().ok()?;
    let name = String::from_utf8_lossy(&o.stdout).trim().to_string();
    (o.status.success() && !name.is_empty()).then_some(name)
}
#[cfg(not(unix))]
fn foreign_owner(_: &Path) -> Option<String> {
    None
}

/// The home folder to give git when the folder owner is the person the schedule runs "as" (the
/// installing user, `KORA_UPDATE_AS` in the update unit): their `gh` / git credentials live there,
/// so private forks can be fetched. Any other owner (e.g. the service account) gets the server folder.
fn credentials_home(owner: &str) -> Option<PathBuf> {
    if std::env::var("KORA_UPDATE_AS").ok().as_deref() != Some(owner) {
        return None;
    }
    let o = std::process::Command::new("getent").args(["passwd", owner]).output().ok()?;
    let line = String::from_utf8_lossy(&o.stdout).into_owned();
    let home = line.trim().split(':').nth(5)?;
    (o.status.success() && !home.is_empty()).then(|| PathBuf::from(home))
}

/// Number of players online from the server's own `/players.json` (None when unreachable).
pub fn online_players(port: u16) -> Option<u32> {
    use std::io::{Read, Write};
    let mut c = std::net::TcpStream::connect_timeout(&std::net::SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_secs(3)).ok()?;
    c.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    c.write_all(format!("GET /players.json HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\r\n").as_bytes()).ok()?;
    let mut buf = String::new();
    c.read_to_string(&mut buf).ok()?;
    let body = buf.split_once("\r\n\r\n")?.1;
    serde_json::from_str::<Vec<serde_json::Value>>(body).ok().map(|v| v.len() as u32)
}

/// Test seam (documented): when `KORA_TEST_FAIL_HEALTH` is set, the health check after an update
/// reports the update as unhealthy (after a real restart), so the automatic rollback to the
/// recorded commits can be exercised end to end. Never set in production.
pub const FAIL_HEALTH_ENV: &str = "KORA_TEST_FAIL_HEALTH";

struct FaultInjecting<'a>(&'a dyn HealthCheck);

impl HealthCheck for FaultInjecting<'_> {
    fn check(&self, req: &HealthRequest) -> HealthOutcome {
        let real = self.0.check(req);
        match real {
            HealthOutcome::Healthy if std::env::var_os(FAIL_HEALTH_ENV).is_some_and(|v| !v.is_empty()) => {
                HealthOutcome::Unhealthy { reason: format!("injected failure ({FAIL_HEALTH_ENV})"), failed: req.updated.clone() }
            }
            other => other,
        }
    }
    fn recover(&self, req: &HealthRequest) {
        self.0.recover(req)
    }
}

/// Health check backed by the OS service: restarts it and waits for the game port to come up.
/// A server that is not running needs no restart. A running server that Kora does not manage as
/// a service cannot be restarted by Kora: it is left alone and reported healthy.
pub struct ServiceHealth {
    pub managed: ManagedServer,
    pub game_port: u16,
    pub wait: Duration,
}

impl ServiceHealth {
    pub(crate) fn running(&self) -> bool {
        matches!(firewall::check_port(self.game_port, false), firewall::PortStatus::InUse { .. })
    }
    fn has_service(&self) -> bool {
        if cfg!(windows) {
            return self.has_service_via(&service::SystemRunner, service::Platform::Windows);
        }
        Path::new(&self.managed.unit_path()).exists()
    }
    /// Windows: ask the service manager. A server started by hand (no `--service`) has none, and
    /// "restart failed" for it would wrongly roll every update back.
    fn has_service_via(&self, rn: &dyn service::CommandRunner, platform: service::Platform) -> bool {
        !matches!(ServiceManager::new(platform, rn).status(&self.managed), Ok(service::ServiceState::NotInstalled))
    }
    fn restart(&self) -> std::result::Result<(), String> {
        let rn = service::SystemRunner;
        ServiceManager::new(service::Platform::current(), &rn).restart(&self.managed).map_err(|e| e.to_string())
    }
}

impl HealthCheck for ServiceHealth {
    fn check(&self, req: &HealthRequest) -> HealthOutcome {
        if !self.running() || !req.full_restart || !self.has_service() {
            return HealthOutcome::Healthy;
        }
        if let Err(e) = self.restart() {
            return HealthOutcome::Unhealthy { reason: format!("restart failed: {e}"), failed: req.updated.clone() };
        }
        let deadline = std::time::Instant::now() + self.wait;
        while std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_secs(2));
            if self.running() {
                return HealthOutcome::Healthy;
            }
        }
        HealthOutcome::Unhealthy { reason: "server did not come back after the update".into(), failed: req.updated.clone() }
    }

    /// Only after a full restart: anything else never restarted the server, so it still runs the
    /// old code and players are not kicked for nothing.
    fn recover(&self, req: &HealthRequest) {
        if req.full_restart && self.running() && self.has_service() {
            let _ = self.restart();
        }
    }
}

/// Run `kora update --auto <server>`. Development servers only. `server` is a registered name
/// or the absolute server folder (what the systemd timer passes). The report is appended to
/// `<dir>/update.log`.
pub fn auto_update(server: &str) -> Result<UpdateReport> {
    let as_dir = Path::new(server);
    let (name, dir) = if as_dir.is_absolute() && as_dir.join("kora-server.json").is_file() {
        let sf = ServerFile::load(as_dir).map_err(|e| InstallError(format!("cannot read {}: {e}", as_dir.join("kora-server.json").display())))?;
        (sf.name, as_dir.to_path_buf())
    } else {
        let reg = Registry::load().map_err(|e| InstallError(format!("cannot read the server registry: {e}")))?;
        let entry = reg.find(server).ok_or_else(|| InstallError(format!("no registered server named '{server}' (see `kora status`)")))?;
        (entry.name.clone(), entry.dir.clone())
    };
    // A server on released versions moves between releases (the catalogue decides), through the same engine as the
    // manual update; it takes the update lock itself.
    if let Some(sf) = ServerFile::load(&dir).ok().filter(|sf| sf.versions() == Versions::Released) {
        let res = update::manual::apply(&sf, update::manual::Scope::All, update::manual::ApplyOpts::new(update::manual::RestartPolicy::WhenQuiet), &mut |_| {});
        append_log(&dir, &res.as_ref().map(UpdateReport::summary).unwrap_or_else(|e| format!("update failed: {e}")));
        if let Ok(report) = &res {
            let record = super::registry::ServerEntry { source: sf.source, versions: sf.versions, name: name.clone(), dir: dir.clone(), mode: sf.mode, ports: sf.ports, created_unix: sf.created_unix };
            crate::notify::after_update(&record, report);
        }
        return res;
    }
    let res = with_git(&dir, |git| {
        // Read the server file under the lock, so a concurrent manual update is never overwritten.
        auto_update_in(&name, &dir, git, &|sf| service_health(&name, &dir, sf), &|sf, h| {
            if h.running() {
                online_players(sf.ports.game)
            } else {
                Some(0)
            }
        })
    });
    if let Ok((report, sf)) = &res {
        let record = super::registry::ServerEntry { source: sf.source, versions: sf.versions, name: name.clone(), dir: dir.clone(), mode: sf.mode, ports: sf.ports, created_unix: sf.created_unix };
        crate::notify::after_update(&record, report);
    }
    res.map(|(r, _)| r)
}

/// The locked part of [`auto_update`], with injectable git, health and player count: takes the
/// server's update lock (busy: logs that this run was skipped and returns the busy error), runs
/// [`update_server`], then writes `update.log` and the status file.
pub fn auto_update_in<H: HealthCheck>(
    name: &str,
    dir: &Path,
    git: &dyn GitOps,
    health: &dyn Fn(&ServerFile) -> H,
    players: &dyn Fn(&ServerFile, &H) -> Option<u32>,
) -> Result<(UpdateReport, ServerFile)> {
    let _lock = match update::lock::acquire(dir) {
        Ok(l) => l,
        Err(e) => {
            append_log(dir, &format!("skipped: {e}"));
            return Err(InstallError(e.to_string()));
        }
    };
    let res = ServerFile::load(dir).map_err(|e| InstallError(format!("cannot read {}: {e}", dir.join("kora-server.json").display()))).and_then(|sf| {
        let h = health(&sf);
        // Running server: ask it for its player count (restart-needing updates wait for 0 players);
        // a server that is not running has none. Asked after the fork sync, which can take minutes.
        update_server(name, dir, &sf, git, &FaultInjecting(&h), &|| players(&sf, &h)).map(|r| (r, sf))
    });
    let line = match &res {
        Ok((r, _)) => r.summary(),
        Err(e) => format!("update failed: {e}"),
    };
    append_log(dir, &line);
    if let Ok((report, _)) = &res {
        crate::update::status::record(dir, &crate::update::manual::apply_status(report, now_unix(), &None));
    }
    res
}

/// Service-backed health check for a registered server.
pub(crate) fn service_health(name: &str, dir: &Path, sf: &ServerFile) -> ServiceHealth {
    let managed = ManagedServer {
        name: slug(name),
        dir: dir.to_string_lossy().into_owned(),
        start_command: vec!["unused".into()],
        user: "kora".into(),
        env_file: None,
        kora_exe: "kora".into(),
        windows_run_as: None,
    };
    ServiceHealth { managed, game_port: sf.ports.game, wait: Duration::from_secs(90) }
}

/// Run `f` with the command runner appropriate for who owns `dir` (the same choice as [`with_git`]): for work that needs
/// `git` and `gh` themselves rather than the update engine's view of them.
pub(crate) fn with_runner<T>(dir: &Path, f: impl FnOnce(&dyn CommandRunner) -> T) -> T {
    match foreign_owner(dir) {
        Some(user) => f(&AsUser { home: credentials_home(&user).unwrap_or_else(|| dir.to_path_buf()), user }),
        None if cfg!(windows) => f(&SafeDirectory(SystemRunner)),
        None => f(&SystemRunner),
    }
}

/// Run `f` with the git adapter appropriate for who owns `dir` (service account / SYSTEM / plain).
pub(crate) fn with_git<T>(dir: &Path, f: impl FnOnce(&dyn GitOps) -> T) -> T {
    match foreign_owner(dir) {
        Some(user) => f(&GitCli(AsUser { home: credentials_home(&user).unwrap_or_else(|| dir.to_path_buf()), user })),
        None if cfg!(windows) => f(&GitCli(SafeDirectory(SystemRunner))),
        None => f(&GitCli::default()),
    }
}

/// Root (the update timer) appends here, in a folder the service account can change: never through
/// a planted link or hard link (see `fsutil::open_no_follow`).
fn append_log(dir: &Path, text: &str) {
    use std::io::Write;
    if let Ok(mut f) = crate::fsutil::open_no_follow(&dir.join("update.log"), crate::fsutil::Access::Append, Some(0o644)) {
        let _ = writeln!(f, "=== unix {} ===\n{}", now_unix(), text.trim_end());
    }
}

/// The engine wiring, with injectable git and health (used by tests). Does not take the update lock:
/// [`auto_update_in`] does. `players` is asked after the forks were synced.
pub fn update_server(name: &str, dir: &Path, sf: &ServerFile, git: &dyn GitOps, health: &dyn HealthCheck, players: &dyn Fn() -> Option<u32>) -> Result<UpdateReport> {
    if sf.versions() != Versions::Latest {
        return err_msg("this server follows released versions: its automatic update moves between releases (see `kora_core::update::manual::apply`), not along a branch");
    }
    let targets: Vec<UpdateTarget> = sf
        .resources
        .iter()
        .filter(|r| !r.third_party && !super::pinned::is_subpath_copy(&dir.join("resources").join(&r.path)))
        .map(|r| UpdateTarget { id: r.id.clone(), path: dir.join("resources").join(&r.path), upstream_ref: format!("upstream/{}", r.branch), class: ResourceClass::from_id(&r.id) })
        .collect();
    // Bring the user's forks on GitHub up to date first (fast-forward only), then the local checkouts.
    let fork_sync = git.sync_forks(&targets, &mut |_, _| {});
    let mut plan = update::plan(git, &targets);
    // A version an earlier automatic rollback undid is not tried again every night.
    let held = update::apply_holds(dir, &mut plan);
    // Same place and format as manual snapshots, so `kora update --rollback` and the panel list them.
    let now = now_unix();
    let snap_dir = update::snapshot_dir(dir);
    let snapshot = snap_dir.join(format!("{}.json", update::new_snapshot_id(&snap_dir, now)));
    let restart: RestartDecision = update::decide_restart(players(), 0, &QuietConfig { window: None, max_players: 0 });
    let opts = RunOptions { server: name, snapshot_file: &snapshot, now_unix: now, restart };
    let mut report = update::run_update(git, health, &NoMigrations, &plan, &opts).map_err(|e| InstallError(e.to_string()))?;
    update::add_fork_sync(&mut report, fork_sync);
    report.held = held.iter().map(|(id, c)| (id.clone(), update::held_text(c))).collect();
    update::record_holds(dir, &plan, &report);
    update::prune_snapshots(dir, update::KEEP_SNAPSHOTS);
    Ok(report)
}

fn err_msg<T>(m: &str) -> Result<T> {
    Err(InstallError(m.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(unused_imports)]
    use crate::installer::Mode;

    /// The update timer runs as root; `update.log` is in a folder the service account owns.
    #[cfg(unix)]
    #[test]
    fn the_update_log_is_never_written_through_a_planted_link() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("srv");
        std::fs::create_dir_all(&dir).unwrap();
        append_log(&dir, "first");
        append_log(&dir, "second");
        let log = std::fs::read_to_string(dir.join("update.log")).unwrap();
        assert!(log.contains("first") && log.contains("second"), "{log}");
        std::fs::remove_file(dir.join("update.log")).unwrap();
        let victim = t.path().join("nologin");
        std::os::unix::fs::symlink(&victim, dir.join("update.log")).unwrap();
        append_log(&dir, "x");
        assert!(!victim.exists(), "root must not create the link's target");
    }
    use std::process::Command;

    fn sh(dir: &Path, args: &[&str]) {
        let o = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    }

    #[test]
    fn a_hand_started_windows_server_has_no_service_so_no_restart_is_attempted() {
        use crate::service::testing::FakeRunner;
        let managed = ManagedServer {
            name: "main".into(),
            dir: "C:\\kora\\main".into(),
            start_command: vec!["unused".into()],
            user: "kora".into(),
            env_file: None,
            kora_exe: "kora".into(),
            windows_run_as: None,
        };
        let h = ServiceHealth { managed, game_port: 30120, wait: Duration::from_secs(1) };
        let r = FakeRunner::default();
        r.respond("sc.exe query", 1060, "[SC] EnumQueryServicesStatus:OpenService FAILED 1060");
        assert!(!h.has_service_via(&r, service::Platform::Windows));
        let r = FakeRunner::default();
        r.respond("sc.exe query", 0, "STATE              : 4  RUNNING");
        assert!(h.has_service_via(&r, service::Platform::Windows));
    }

    #[test]
    fn update_syncs_only_behind_forks_when_signed_in_and_never_forces() {
        use std::cell::RefCell;
        struct Gh {
            calls: RefCell<Vec<String>>,
            signed_in: bool,
        }
        impl CommandRunner for Gh {
            fn run(&self, p: &str, a: &[&str], _: Option<&Path>) -> std::io::Result<git::Output> {
                let line = format!("{p} {}", a.join(" "));
                self.calls.borrow_mut().push(line.clone());
                let ok = |s: &str| Ok(git::Output { code: 0, stdout: s.into(), stderr: String::new() });
                if line.starts_with("gh auth status") {
                    return Ok(git::Output { code: if self.signed_in { 0 } else { 1 }, ..Default::default() });
                }
                if line.starts_with("git remote get-url origin") {
                    return ok("https://github.com/me/res.git\n");
                }
                if line.starts_with("git remote get-url upstream") {
                    return ok("https://github.com/KoraFiveM/res.git\n");
                }
                if line.contains("/compare/") {
                    return ok(r#"{"ahead_by":0,"behind_by":3}"#);
                }
                ok("")
            }
            fn sleep(&self, _: Duration) {}
        }
        let t = |id: &str| UpdateTarget { id: id.into(), path: PathBuf::from("/x"), upstream_ref: "upstream/dev".into(), class: ResourceClass::Other };
        let g = GitCli(Gh { calls: RefCell::new(vec![]), signed_in: true });
        let out = g.sync_forks(&[t("res")], &mut |_, _| {});
        assert_eq!(out, vec![("res".to_string(), git::SyncOutcome::Synced { commits: 3 })]);
        let calls = g.0.calls.borrow();
        assert!(calls.contains(&"gh repo sync me/res --branch dev".to_string()), "{calls:?}");
        assert!(calls.iter().all(|c| !c.contains("--force")));
        drop(calls);
        // signed out: no GitHub call beyond the auth check
        let g = GitCli(Gh { calls: RefCell::new(vec![]), signed_in: false });
        assert!(g.sync_forks(&[t("res")], &mut |_, _| {}).is_empty());
        assert!(g.0.calls.borrow().iter().all(|c| !c.contains("/compare/") && !c.contains("repo sync")));
    }

    #[test]
    fn safe_directory_flag_is_added_to_git_only() {
        use std::cell::RefCell;
        struct Rec(RefCell<Vec<String>>);
        impl CommandRunner for Rec {
            fn run(&self, p: &str, a: &[&str], _: Option<&Path>) -> std::io::Result<git::Output> {
                self.0.borrow_mut().push(format!("{p} {}", a.join(" ")));
                Ok(git::Output { code: 0, stdout: String::new(), stderr: String::new() })
            }
            fn sleep(&self, _: Duration) {}
        }
        let r = SafeDirectory(Rec(RefCell::new(vec![])));
        r.run("git", &["status"], None).unwrap();
        r.run("gh", &["auth", "status"], None).unwrap();
        assert_eq!(*r.0 .0.borrow(), vec!["git -c safe.directory=* status", "gh auth status"]);
    }

    #[test]
    fn git_adapter_against_local_repos() {
        let t = tempfile::tempdir().unwrap();
        let up = t.path().join("up");
        std::fs::create_dir(&up).unwrap();
        sh(&up, &["init", "-q", "-b", "dev"]);
        std::fs::write(up.join("a"), "1").unwrap();
        sh(&up, &["add", "."]);
        sh(&up, &["commit", "-qm", "one"]);
        let fork = t.path().join("fork");
        sh(t.path(), &["clone", "-q", up.to_str().unwrap(), fork.to_str().unwrap()]);
        sh(&fork, &["remote", "add", "upstream", up.to_str().unwrap()]);
        let g = GitCli::default();
        g.fetch(&fork).unwrap();
        assert!(!g.is_dirty(&fork).unwrap());
        assert_eq!(g.behind_count(&fork, "upstream/dev").unwrap(), 0);
        std::fs::write(up.join("a"), "2").unwrap();
        sh(&up, &["commit", "-qam", "two"]);
        g.fetch(&fork).unwrap();
        assert_eq!(g.behind_count(&fork, "upstream/dev").unwrap(), 1);
        assert!(!g.is_diverged(&fork, "upstream/dev").unwrap());
        let before = g.head_commit(&fork, "HEAD").unwrap();
        g.fast_forward(&fork, "upstream/dev").unwrap();
        assert_ne!(g.head_commit(&fork, "HEAD").unwrap(), before);
        g.reset_to(&fork, &before).unwrap();
        assert_eq!(g.head_commit(&fork, "HEAD").unwrap(), before);
        std::fs::write(fork.join("b"), "x").unwrap();
        assert!(g.is_dirty(&fork).unwrap());
    }

    struct Healthy;
    impl HealthCheck for Healthy {
        fn check(&self, _: &HealthRequest) -> HealthOutcome {
            HealthOutcome::Healthy
        }
        fn recover(&self, _: &HealthRequest) {}
    }

    #[test]
    fn update_server_fast_forwards_clean_fork_and_skips_dirty() {
        use crate::installer::registry::{PortsRecord, ResourceRecord};
        let t = tempfile::tempdir().unwrap();
        let mut records = vec![];
        for (id, path) in [("kora-chat", "[kora]/kora_chat"), ("kora-hud", "[kora]/kora_hud")] {
            let up = t.path().join(format!("up-{id}"));
            std::fs::create_dir(&up).unwrap();
            sh(&up, &["init", "-q", "-b", "dev"]);
            std::fs::write(up.join("a"), "1").unwrap();
            sh(&up, &["add", "."]);
            sh(&up, &["commit", "-qm", "one"]);
            let fork = t.path().join("srv/resources").join(path);
            std::fs::create_dir_all(fork.parent().unwrap()).unwrap();
            sh(t.path(), &["clone", "-q", up.to_str().unwrap(), fork.to_str().unwrap()]);
            sh(&fork, &["remote", "add", "upstream", up.to_str().unwrap()]);
            std::fs::write(up.join("a"), "2").unwrap();
            sh(&up, &["commit", "-qam", "two"]);
            records.push(ResourceRecord {
                id: id.into(),
                path: path.into(),
                repository: "https://github.com/x/y.git".into(),
                branch: "dev".into(),
                commit: None,
                third_party: false,
                ..Default::default()
            });
        }
        // make the second fork dirty
        std::fs::write(t.path().join("srv/resources/[kora]/kora_hud/a"), "local edit").unwrap();
        let sf = ServerFile {
            source: None,
            versions: None,
            name: "t".into(),
            mode: Mode::Development,
            dir: t.path().join("srv"),
            ports: PortsRecord { game: 1, txadmin: 2 },
            game_build: 1,
            artifact_build: 1,
            database: None,
            github_owner: None,
            created_unix: 0,
            kora_version: "0".into(),
            resources: records,
        };
        let rep = update_server("t", &t.path().join("srv"), &sf, &GitCli::default(), &Healthy, &|| Some(0)).unwrap();
        assert_eq!(rep.updated.len(), 1, "{}", rep.summary());
        assert_eq!(rep.updated[0].id, "kora-chat");
        assert!(rep.needs_attention.iter().any(|(id, _)| id == "kora-hud"));
        assert_eq!(std::fs::read_to_string(t.path().join("srv/resources/[kora]/kora_chat/a")).unwrap(), "2");
        assert_eq!(std::fs::read_to_string(t.path().join("srv/resources/[kora]/kora_hud/a")).unwrap(), "local edit");
        let prod = ServerFile { mode: Mode::Production, ..sf };
        assert!(update_server("t", &t.path().join("srv"), &prod, &GitCli::default(), &Healthy, &|| Some(0)).is_err());
        // The automatic run saves its snapshot where `kora update --rollback` and the panel look.
        let snaps = update::list_snapshots(&t.path().join("srv"));
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].1.entries[0].id, "kora-chat");
        assert!(!t.path().join("srv/snapshots").exists());
    }

    #[test]
    fn auto_update_skips_and_logs_when_another_update_runs() {
        use crate::installer::registry::PortsRecord;
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("srv");
        std::fs::create_dir_all(&dir).unwrap();
        let sf = ServerFile {
            source: None,
            versions: None,
            name: "t".into(),
            mode: Mode::Development,
            dir: dir.clone(),
            ports: PortsRecord { game: 1, txadmin: 2 },
            game_build: 1,
            artifact_build: 1,
            database: None,
            github_owner: None,
            created_unix: 0,
            kora_version: "0".into(),
            resources: vec![],
        };
        sf.save().unwrap();
        let held = update::lock::acquire(&dir).unwrap();
        let asked = std::cell::Cell::new(false);
        let e = auto_update_in("t", &dir, &GitCli::default(), &|_| Healthy, &|_, _| {
            asked.set(true);
            Some(0)
        })
        .unwrap_err();
        assert!(update::lock::is_busy_message(&e.to_string()), "{e}");
        assert!(!asked.get(), "nothing was looked at");
        let log = std::fs::read_to_string(dir.join("update.log")).unwrap();
        assert!(log.contains("skipped: Another update is running for this server"), "{log}");
        drop(held);
        let (r, _) = auto_update_in("t", &dir, &GitCli::default(), &|_| Healthy, &|_, _| Some(0)).unwrap();
        assert!(!r.has_problems());
        assert!(crate::update::status::read(&dir).is_some());
    }
}
