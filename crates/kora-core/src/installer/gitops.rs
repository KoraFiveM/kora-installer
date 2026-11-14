//! Glue for `kora update --auto`: a [`GitOps`] adapter over the git module, a service-based
//! health check, and [`auto_update`] which wires the update engine to a registered server.

use std::path::Path;
use std::time::Duration;

use super::registry::{now_unix, Registry, ServerFile};
use super::types::Mode;
use super::{slug, InstallError, Result};
use crate::firewall;
use crate::git::{self, CommandRunner, SystemRunner};
use crate::service::{self, ManagedServer, ServiceManager};
use crate::update::{
    self, GitOps, GitResult, HealthCheck, HealthOutcome, HealthRequest, NoMigrations, QuietConfig, RestartDecision, ResourceClass,
    RunOptions, UpdateError, UpdateReport, UpdateTarget,
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
    fn running(&self) -> bool {
        matches!(firewall::check_port(self.game_port, false), firewall::PortStatus::InUse { .. })
    }
    fn has_service(&self) -> bool {
        Path::new(&self.managed.unit_path()).exists() || cfg!(windows)
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

    fn recover(&self, _req: &HealthRequest) {
        if self.running() && self.has_service() {
            let _ = self.restart();
        }
    }
}

/// Run `kora update --auto <server>`. Development servers only.
pub fn auto_update(server: &str) -> Result<UpdateReport> {
    let reg = Registry::load().map_err(|e| InstallError(format!("cannot read the server registry: {e}")))?;
    let entry = reg.find(server).ok_or_else(|| InstallError(format!("no registered server named '{server}' (see `kora status`)")))?.clone();
    let sf = ServerFile::load(&entry.dir).map_err(|e| InstallError(format!("cannot read {}: {e}", entry.dir.join("kora-server.json").display())))?;
    let managed = ManagedServer {
        name: slug(&entry.name),
        dir: entry.dir.to_string_lossy().into_owned(),
        start_command: vec!["unused".into()],
        user: "kora".into(),
        env_file: None,
        kora_exe: "kora".into(),
        windows_run_as: None,
    };
    let health = ServiceHealth { managed, game_port: sf.ports.game, wait: Duration::from_secs(90) };
    // No player-count hook yet: a stopped server has no players, a running one is unknown (defer).
    let players = if health.running() { None } else { Some(0) };
    update_server(&entry.name, &entry.dir, &sf, &GitCli::default(), &health, players)
}

/// The engine wiring, with injectable git and health (used by tests).
pub fn update_server(
    name: &str,
    dir: &Path,
    sf: &ServerFile,
    git: &dyn GitOps,
    health: &dyn HealthCheck,
    players: Option<u32>,
) -> Result<UpdateReport> {
    if sf.mode != Mode::Development {
        return err_msg("auto-update only supports development servers; production servers move to newer releases manually");
    }
    let targets: Vec<UpdateTarget> = sf
        .resources
        .iter()
        .map(|r| UpdateTarget {
            id: r.id.clone(),
            path: dir.join("resources").join(&r.path),
            upstream_ref: format!("upstream/{}", r.branch),
            class: ResourceClass::from_id(&r.id),
        })
        .collect();
    let plan = update::plan(git, &targets);
    let snap_dir = dir.join("snapshots");
    std::fs::create_dir_all(&snap_dir).map_err(|e| InstallError(e.to_string()))?;
    let snapshot = snap_dir.join("last-update.json");
    let restart: RestartDecision = update::decide_restart(players, 0, &QuietConfig { window: None, max_players: 0 });
    let opts = RunOptions { server: name, snapshot_file: &snapshot, now_unix: now_unix(), restart };
    update::run_update(git, health, &NoMigrations, &plan, &opts).map_err(|e| InstallError(e.to_string()))
}

fn err_msg<T>(m: &str) -> Result<T> {
    Err(InstallError(m.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn sh(dir: &Path, args: &[&str]) {
        let o = Command::new("git").args(args).current_dir(dir).env("GIT_AUTHOR_NAME", "t").env("GIT_AUTHOR_EMAIL", "t@t").env("GIT_COMMITTER_NAME", "t").env("GIT_COMMITTER_EMAIL", "t@t").output().unwrap();
        assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
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
            records.push(ResourceRecord { id: id.into(), path: path.into(), repository: "https://github.com/x/y.git".into(), branch: "dev".into(), commit: None });
        }
        // make the second fork dirty
        std::fs::write(t.path().join("srv/resources/[kora]/kora_hud/a"), "local edit").unwrap();
        let sf = ServerFile {
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
        let rep = update_server("t", &t.path().join("srv"), &sf, &GitCli::default(), &Healthy, Some(0)).unwrap();
        assert_eq!(rep.updated.len(), 1, "{}", rep.summary());
        assert_eq!(rep.updated[0].id, "kora-chat");
        assert!(rep.needs_attention.iter().any(|(id, _)| id == "kora-hud"));
        assert_eq!(std::fs::read_to_string(t.path().join("srv/resources/[kora]/kora_chat/a")).unwrap(), "2");
        assert_eq!(std::fs::read_to_string(t.path().join("srv/resources/[kora]/kora_hud/a")).unwrap(), "local edit");
        let prod = ServerFile { mode: Mode::Production, ..sf };
        assert!(update_server("t", &t.path().join("srv"), &prod, &GitCli::default(), &Healthy, Some(0)).is_err());
    }
}
