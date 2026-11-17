//! Start, stop, restart, state and removal of a server's containers.
//!
//! [`Manager`] works on one server folder with an engine behind an [`EngineRunner`], so every command it
//! issues is asserted in tests with a fake runner. The real thing is [`system`]. Nothing here ever matches
//! containers by pattern: Kora acts on the exact names in the folder's `kora-container.json` and finds
//! leftovers only by the `kora.server=<slug>` label, so it can never touch somebody else's containers.
//!
//! "Start" means "make sure the container matches what it should be, then start it": a missing container is
//! created, one whose `kora.spec` label differs from the one the current settings give (a new FXServer build
//! after an update, other ports, new limits, a newer pinned image) is removed and created again. That is safe
//! because a container holds no state of its own.

use std::path::Path;

use super::engine::{self, ContainerState, Engine, EngineError, EngineRunner};
use super::record::ContainerRecord;
use super::spec::{self, GameSpec, STOP_TIMEOUT_SECS};
use crate::problem::Problem;

pub struct Manager<'a> {
    pub dir: &'a Path,
    pub rec: &'a ContainerRecord,
    pub runner: &'a dyn EngineRunner,
}

/// What the real machine uses.
pub fn system() -> crate::service::SystemRunner {
    crate::service::SystemRunner
}

/// The folder's slug (the name the containers carry in their label).
pub fn slug_for_dir(dir: &Path) -> String {
    crate::installer::registry::ServerFile::load(dir).map(|f| crate::installer::slug(&f.name)).unwrap_or_else(|_| {
        // A record always names its container `kora-<slug>`.
        ContainerRecord::load(dir).map(|r| r.container.trim_start_matches("kora-").to_string()).unwrap_or_else(|| "server".into())
    })
}

impl<'a> Manager<'a> {
    pub fn new(dir: &'a Path, rec: &'a ContainerRecord, runner: &'a dyn EngineRunner) -> Self {
        Manager { dir, rec, runner }
    }

    pub fn engine(&self) -> Engine {
        self.rec.engine()
    }

    pub fn slug(&self) -> String {
        self.rec.container.trim_start_matches("kora-").to_string()
    }

    fn fail(&self, what: &str, e: EngineError) -> Problem {
        super::detect::problem_for_failure(&self.engine(), what, &e)
    }

    fn state_of(&self, name: &str) -> ContainerState {
        match self.engine().run(self.runner, &engine::inspect_state_args(name)) {
            Ok(o) => engine::parse_state(&o),
            Err(e) => ContainerState::Other(e.to_string()),
        }
    }

    /// The game container's state.
    pub fn state(&self) -> ContainerState {
        self.state_of(&self.rec.container)
    }

    /// The database container's state (`None`: this server has none).
    pub fn db_state(&self) -> Option<ContainerState> {
        self.rec.db.as_ref().map(|d| self.state_of(&d.container))
    }

    fn label_of(&self, name: &str, label: &str) -> Option<String> {
        let args = vec!["inspect".to_string(), "--type".into(), "container".into(), "--format".into(), format!("{{{{index .Config.Labels \"{label}\"}}}}"), name.to_string()];
        let o = self.engine().run(self.runner, &args).ok().filter(|o| o.success())?;
        let v = o.stdout.trim().to_string();
        (!v.is_empty() && v != "<no value>").then_some(v)
    }

    /// The arguments the game container should be created with right now.
    pub fn desired_game_args(&self) -> Result<Vec<String>, Problem> {
        let sf = crate::installer::registry::ServerFile::load(self.dir).map_err(|e| e.lead("cannot read kora-server.json"))?;
        let store = crate::artifacts::ArtifactStore::new(self.dir.join("artifacts"));
        let st = store.state()?;
        let build = st.current.ok_or_else(|| Problem::generic("this server has no FXServer build installed"))?;
        let build_dir = store.build_dir(build);
        let g = GameSpec {
            slug: &self.slug(),
            dir: self.dir,
            rec: self.rec,
            game_port: sf.ports.game,
            txadmin_port: sf.ports.txadmin,
            build,
            build_dir: &build_dir,
            env: crate::txadmin::txadmin_env(&self.dir.join("txData"), sf.ports.txadmin),
        };
        spec::game_create_args(&g).map_err(Problem::generic)
    }

    fn image_present(&self, image: &str) -> bool {
        self.engine().run(self.runner, &engine::image_inspect_args(image)).is_ok_and(|o| o.success())
    }

    /// Pull `image` when the engine does not have it, passing progress lines on.
    pub fn ensure_image(&self, image: &str, progress: &mut dyn FnMut(&str)) -> Result<(), Problem> {
        if self.image_present(image) {
            return Ok(());
        }
        progress(&format!("Downloading {image}"));
        let e = self.engine();
        let args = engine::pull_args(image);
        let o = self.runner.run_lines(e.program(), &args, &mut |l| progress(l.trim())).map_err(|x| self.fail("download the container image", EngineError::Spawn(e.program().into(), x.to_string())))?;
        if o.success() {
            Ok(())
        } else {
            Err(super::detect::pull_problem(&e, image, &engine::tidy(&o.stderr)))
        }
    }

    pub fn ensure_network(&self) -> Result<(), Problem> {
        let Some(db) = &self.rec.db else { return Ok(()) };
        let e = self.engine();
        if e.run(self.runner, &engine::network_inspect_args(&db.network)).is_ok_and(|o| o.success()) {
            return Ok(());
        }
        e.run_ok(self.runner, &engine::network_create_args(&db.network, &self.slug())).map(|_| ()).map_err(|x| self.fail("create the server's private network", x))
    }

    pub fn ensure_volume(&self) -> Result<(), Problem> {
        let Some(db) = &self.rec.db else { return Ok(()) };
        let e = self.engine();
        if e.run(self.runner, &engine::volume_inspect_args(&db.volume)).is_ok_and(|o| o.success()) {
            return Ok(());
        }
        e.run_ok(self.runner, &engine::volume_create_args(&db.volume, &self.slug())).map(|_| ()).map_err(|x| self.fail("create the database's data volume", x))
    }

    /// Make the game container exist and match the current settings; does not start it.
    pub fn ensure_game_container(&self, progress: &mut dyn FnMut(&str)) -> Result<(), Problem> {
        let want = self.desired_game_args()?;
        let want_label = spec::spec_label_of(&want);
        let name = &self.rec.container;
        let e = self.engine();
        match self.state() {
            ContainerState::Missing => {}
            ContainerState::Other(w) if !w.is_empty() && !w.contains("paused") => {
                return Err(Problem::new(super::codes::START_FAILED, format!("Kora could not tell what state the container '{name}' is in.")).with_details(w))
            }
            _ => {
                if self.label_of(name, "kora.spec") == want_label && want_label.is_some() {
                    return Ok(());
                }
                progress("Updating the container to the current settings");
                e.run_ok(self.runner, &engine::rm_args(name)).map_err(|x| self.fail("replace the server's container", x))?;
            }
        }
        self.ensure_image(&self.rec.image, progress)?;
        self.ensure_network()?;
        e.run_ok(self.runner, &want).map(|_| ()).map_err(|x| self.fail("create the server's container", x))
    }

    /// Start the database container. One that is gone (removed by hand) is made again from the login in
    /// `database.json`; its data is in the volume, so nothing is lost. A volume that is gone too is not made up:
    /// a new empty database would not fit the saved login.
    pub fn start_db(&self) -> Result<(), Problem> {
        let Some(db) = &self.rec.db else { return Ok(()) };
        match self.state_of(&db.container) {
            ContainerState::Running => return Ok(()),
            ContainerState::Missing => {
                let engine = self.engine();
                let volume_here = engine.run(self.runner, &engine::volume_inspect_args(&db.volume)).is_ok_and(|o| o.success());
                let creds: Option<crate::db::Credentials> = std::fs::read(self.dir.join("database.json")).ok().and_then(|b| serde_json::from_slice(&b).ok());
                match (volume_here, creds) {
                    (true, Some(c)) => super::dbc::create_container(self, &c, &mut |_| {})?,
                    _ => {
                        return Err(Problem::new(
                            super::codes::DB_NOT_RUNNING,
                            format!(
                                "This server's database container ({}) is gone and so is its data volume or its login file. Restore a backup, or delete the server and install it again.",
                                db.container
                            ),
                        ))
                    }
                }
            }
            _ => {}
        }
        self.engine().run_ok(self.runner, &engine::start_args(&db.container)).map(|_| ()).map_err(|x| self.fail("start the database container", x))
    }

    /// Start the server: database container first, then the game container (created or replaced if needed).
    pub fn start(&self, progress: &mut dyn FnMut(&str)) -> Result<(), Problem> {
        self.start_db()?;
        self.ensure_game_container(progress)?;
        if self.state() != ContainerState::Running {
            self.engine().run_ok(self.runner, &engine::start_args(&self.rec.container)).map_err(|x| self.fail("start the server's container", x))?;
        }
        Ok(())
    }

    /// Stop the game container, waiting up to [`STOP_TIMEOUT_SECS`] for a clean shutdown. The database keeps running.
    pub fn stop(&self) -> Result<(), Problem> {
        match self.state() {
            ContainerState::Missing | ContainerState::Stopped => Ok(()),
            _ => self.engine().run_ok(self.runner, &engine::stop_args(&self.rec.container, STOP_TIMEOUT_SECS)).map(|_| ()).map_err(|x| self.fail("stop the server's container", x)),
        }
    }

    pub fn restart(&self, progress: &mut dyn FnMut(&str)) -> Result<(), Problem> {
        self.stop()?;
        self.start(progress)
    }

    /// Stop and remove the database container too (a delete, a conversion back).
    pub fn stop_db(&self) -> Result<(), Problem> {
        let Some(db) = &self.rec.db else { return Ok(()) };
        match self.state_of(&db.container) {
            ContainerState::Missing | ContainerState::Stopped => Ok(()),
            _ => self.engine().run_ok(self.runner, &engine::stop_args(&db.container, STOP_TIMEOUT_SECS)).map(|_| ()).map_err(|x| self.fail("stop the database container", x)),
        }
    }

    fn gone(&self, done: &mut Vec<String>, what: &str, args: Vec<String>, exists: bool) -> Result<(), Problem> {
        if !exists {
            return Ok(());
        }
        self.engine().run_ok(self.runner, &args).map(|_| ()).map_err(|x| self.fail(&format!("remove {what}"), x))?;
        done.push(format!("Removed {what}"));
        Ok(())
    }

    /// [`Self::gone`] for a container. Rootless Podman can remove the container and still exit 125 because cleaning up its
    /// network namespace failed ("rootless netns: kill network process: permission denied"): when the container is gone
    /// after a failed `rm`, that is what was asked, so it counts as removed (the engine's complaint is kept as a note).
    fn gone_container(&self, done: &mut Vec<String>, what: &str, name: &str) -> Result<(), Problem> {
        if self.state_of(name) == ContainerState::Missing {
            return Ok(());
        }
        match self.engine().run_ok(self.runner, &engine::rm_args(name)) {
            Ok(_) => {}
            Err(x) if self.state_of(name) == ContainerState::Missing => {
                done.push(format!("Removed {what} (the engine reported a clean-up problem afterwards: {})", x.to_string().lines().next().unwrap_or("").trim()));
                return Ok(());
            }
            Err(x) => return Err(self.fail(&format!("remove {what}"), x)),
        }
        done.push(format!("Removed {what}"));
        Ok(())
    }

    /// Remove the game container only (stopping it first, whatever state it is in).
    pub fn remove_game(&self) -> Result<Vec<String>, Problem> {
        let mut done = Vec::new();
        // A running server is stopped the proper way first (SIGTERM, the same grace period), then removed.
        if self.state() == ContainerState::Running {
            self.stop()?;
        }
        self.gone_container(&mut done, &format!("the container {}", self.rec.container), &self.rec.container)?;
        Ok(done)
    }

    /// Remove the database container and the private network, and (only when `remove_volume`) the data volume.
    pub fn remove_db_side(&self, remove_volume: bool) -> Result<Vec<String>, Problem> {
        let mut done = Vec::new();
        let Some(db) = &self.rec.db else { return Ok(done) };
        let e = self.engine();
        self.gone_container(&mut done, &format!("the database container {}", db.container), &db.container)?;
        let net = e.run(self.runner, &engine::network_inspect_args(&db.network)).is_ok_and(|o| o.success());
        self.gone(&mut done, &format!("the private network {}", db.network), engine::network_rm_args(&db.network), net)?;
        if remove_volume {
            let vol = e.run(self.runner, &engine::volume_inspect_args(&db.volume)).is_ok_and(|o| o.success());
            self.gone(&mut done, &format!("the database's data volume {}", db.volume), engine::volume_rm_args(&db.volume), vol)?;
        }
        Ok(done)
    }

    /// Remove what Kora made for this server: the containers, the network, and (only when `remove_volume`)
    /// the database's data volume. Missing objects are fine. Returns what was removed, in words.
    pub fn remove_all(&self, remove_volume: bool) -> Result<Vec<String>, Problem> {
        let mut done = self.remove_game()?;
        done.extend(self.remove_db_side(remove_volume)?);
        Ok(done)
    }

    /// Everything on the engine that carries this server's label (for the "nothing left" check).
    pub fn leftovers(&self) -> Vec<String> {
        let e = self.engine();
        let slug = self.slug();
        let mut out = Vec::new();
        for kind in ["container", "network", "volume"] {
            if let Ok(o) = e.run(self.runner, &engine::list_server_objects_args(kind, &slug)) {
                if o.success() {
                    out.extend(o.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).map(|l| format!("{kind} {l}")));
                }
            }
        }
        out
    }
}

/// The record of a registered server's folder, when it is a container-mode server.
pub fn record_of(dir: &Path) -> Option<ContainerRecord> {
    ContainerRecord::load(dir)
}

/// The sentence with a capital first letter.
pub fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// A container-mode server found by its registry name: its folder, its (checked) record and its slug.
#[derive(Debug, Clone)]
pub struct Target {
    pub dir: std::path::PathBuf,
    pub rec: ContainerRecord,
    pub slug: String,
}

/// The container target of a registered server. `Ok(None)`: it is not a container-mode server. `Err`: it is,
/// but its record fails [`ContainerRecord::check`] and Kora will not act on it.
pub fn target_of(registry_name: &str, dir: &Path) -> Result<Option<Target>, String> {
    let Some(rec) = ContainerRecord::load(dir) else { return Ok(None) };
    let slug = crate::installer::slug(registry_name);
    rec.check(&slug)?;
    Ok(Some(Target { dir: dir.to_path_buf(), rec, slug }))
}

/// A container server's state in the words the service layer uses. A container that does not exist (removed by
/// hand, or a fresh machine) is a stopped server: starting it creates the container again.
pub fn service_state(t: &Target, runner: &dyn EngineRunner) -> crate::service::ServiceState {
    use crate::service::ServiceState;
    match Manager::new(&t.dir, &t.rec, runner).state() {
        ContainerState::Running => ServiceState::Running,
        ContainerState::Stopped | ContainerState::Missing => ServiceState::Stopped,
        ContainerState::Other(w) => ServiceState::Other(w),
    }
}

/// start / stop / restart of a container server. A start also makes sure the console copy runs.
pub fn control(t: &Target, action: crate::service::ControlAction, runner: &dyn EngineRunner) -> Result<(), Problem> {
    use crate::service::ControlAction;
    let m = Manager::new(&t.dir, &t.rec, runner);
    match action {
        ControlAction::Start => m.start(&mut |_| {})?,
        ControlAction::Stop => m.stop()?,
        ControlAction::Restart => m.restart(&mut |_| {})?,
    }
    if action != ControlAction::Stop {
        super::logs::ensure_follower(&t.dir);
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::container::record::tests::sample;
    use crate::service::testing::FakeRunner;

    impl EngineRunner for FakeRunner {}

    /// A server folder with a registered FXServer build, ready for `desired_game_args`.
    pub fn server_dir() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path();
        let sf = crate::installer::registry::ServerFile {
            name: "Main".into(),
            mode: crate::installer::Mode::Production,
            source: None,
            versions: None,
            dir: dir.to_path_buf(),
            ports: crate::installer::registry::PortsRecord { game: 30120, txadmin: 40120 },
            game_build: 0,
            artifact_build: 38435,
            database: None,
            github_owner: None,
            created_unix: 1,
            kora_version: "t".into(),
            resources: vec![],
        };
        std::fs::write(dir.join("kora-server.json"), serde_json::to_vec(&sf).unwrap()).unwrap();
        std::fs::create_dir_all(dir.join("artifacts")).unwrap();
        std::fs::write(dir.join("artifacts/state.json"), r#"{"current":38435,"previous":null}"#).unwrap();
        d
    }

    // Container mode is a Linux feature: these paths are Unix paths ("/srv/..", "/" separators).
    #[cfg(unix)]
    #[test]
    fn start_creates_a_missing_container_then_starts_it_after_the_database() {
        let d = server_dir();
        let rec = sample();
        let r = FakeRunner::default();
        // the database container exists (stopped); the game container does not; images: present
        r.respond("State.Status}} kora-main-db", 0, "exited\n");
        r.respond("inspect --type container", 125, "");
        r.responses.borrow_mut().last_mut().unwrap().1.stderr = "Error: no such container".into();
        r.respond("network inspect", 0, "kora-main-net\n");
        Manager::new(d.path(), &rec, &r).start(&mut |_| {}).unwrap();
        let calls = r.calls();
        let pos = |needle: &str| calls.iter().position(|c| c.contains(needle)).unwrap_or_else(|| panic!("{needle} not in {calls:#?}"));
        assert!(pos("podman start kora-main-db") < pos("podman create --name kora-main"), "{calls:#?}");
        assert!(pos("podman create --name kora-main ") < calls.len() - 1, "{calls:#?}");
        assert_eq!(calls.last().unwrap(), "podman start kora-main");
        assert!(calls.iter().all(|c| !c.contains("sh -c") && !c.starts_with("sudo")), "{calls:#?}");
    }

    // Container mode is a Linux feature: these paths are Unix paths ("/srv/..", "/" separators).
    #[cfg(unix)]
    #[test]
    fn a_container_with_the_current_spec_is_only_started_and_one_with_an_old_spec_is_replaced() {
        let d = server_dir();
        let rec = sample();
        let want = Manager::new(d.path(), &rec, &FakeRunner::default()).desired_game_args().unwrap();
        let label = spec::spec_label_of(&want).unwrap();

        let same = FakeRunner::default();
        same.respond("--format {{.State.Status}} kora-main", 0, "exited\n");
        same.respond("index .Config.Labels", 0, &format!("{label}\n"));
        Manager::new(d.path(), &rec, &same).start(&mut |_| {}).unwrap();
        let calls = same.calls();
        assert!(calls.iter().any(|c| c == "podman start kora-main"), "{calls:#?}");
        assert!(!calls.iter().any(|c| c.starts_with("podman rm") || c.starts_with("podman create")), "{calls:#?}");

        let old = FakeRunner::default();
        old.respond("--format {{.State.Status}} kora-main", 0, "exited\n");
        old.respond("index .Config.Labels", 0, "0123456789abcdef\n");
        old.respond("network inspect", 0, "kora-main-net\n");
        Manager::new(d.path(), &rec, &old).start(&mut |_| {}).unwrap();
        let calls = old.calls();
        let rm = calls.iter().position(|c| c == "podman rm -f -v kora-main").expect("old container removed");
        let create = calls.iter().position(|c| c.starts_with("podman create --name kora-main ")).expect("new one created");
        assert!(rm < create);
    }

    #[test]
    fn a_missing_database_container_is_made_again_from_its_login_and_volume_but_never_without_them() {
        let d = server_dir();
        let rec = sample();
        let creds = crate::container::dbc::game_credentials("main", "kora_ab12cd34", "kora_ab12cd34", &crate::db::Secret::new("pw-1"));
        std::fs::write(d.path().join("database.json"), serde_json::to_vec(&creds).unwrap()).unwrap();
        let r = FakeRunner::default();
        r.respond("State.Status}} kora-main-db", 125, "");
        r.responses.borrow_mut().last_mut().unwrap().1.stderr = "Error: no such container".into();
        r.respond("volume inspect", 0, "kora-main-dbdata\n");
        r.respond("network inspect", 0, "kora-main-net\n");
        Manager::new(d.path(), &rec, &r).start_db().unwrap();
        let calls = r.calls();
        assert!(calls.iter().any(|c| c.starts_with("podman create --name kora-main-db ")), "{calls:#?}");
        assert!(calls.last().unwrap() == "podman start kora-main-db", "{calls:#?}");
        assert!(!d.path().join(".kora/db-init.env").exists());

        // no volume: a new empty database would not fit the saved login
        let r = FakeRunner::default();
        r.respond("State.Status}} kora-main-db", 125, "");
        r.responses.borrow_mut().last_mut().unwrap().1.stderr = "Error: no such container".into();
        r.respond("volume inspect", 1, "");
        let e = Manager::new(d.path(), &rec, &r).start_db().unwrap_err();
        assert!(e.message.contains("is gone"), "{}", e.message);
        assert!(!r.calls().iter().any(|c| c.contains(" create ")));
    }

    #[test]
    fn removing_a_running_game_container_stops_it_properly_first() {
        let d = server_dir();
        let rec = sample();
        let r = FakeRunner::default();
        r.respond("State.Status}} kora-main\n", 0, "running\n");
        r.respond("State.Status", 0, "running\n");
        Manager::new(d.path(), &rec, &r).remove_game().unwrap();
        let calls = r.calls();
        let stop = calls.iter().position(|c| c == "podman stop -t 30 kora-main").expect("stopped gracefully");
        let rm = calls.iter().position(|c| c == "podman rm -f -v kora-main").expect("then removed");
        assert!(stop < rm, "{calls:#?}");
    }

    #[test]
    fn stop_waits_the_same_thirty_seconds_the_service_does_and_leaves_the_database_running() {
        let d = server_dir();
        let rec = sample();
        let r = FakeRunner::default();
        r.respond("State.Status", 0, "running\n");
        Manager::new(d.path(), &rec, &r).stop().unwrap();
        assert!(r.calls().iter().any(|c| c == "podman stop -t 30 kora-main"));
        assert!(!r.calls().iter().any(|c| c.contains("kora-main-db")));
        // stopping a stopped or missing container is fine and runs nothing
        let r = FakeRunner::default();
        r.respond("State.Status", 0, "exited\n");
        Manager::new(d.path(), &rec, &r).stop().unwrap();
        assert!(!r.calls().iter().any(|c| c.contains(" stop ")));
    }

    #[test]
    fn remove_all_removes_exactly_the_servers_objects_and_keeps_the_volume_unless_asked() {
        let d = server_dir();
        let rec = sample();
        let r = FakeRunner::default();
        r.respond("State.Status", 0, "exited\n");
        let done = Manager::new(d.path(), &rec, &r).remove_all(false).unwrap();
        let calls = r.calls();
        for want in ["podman rm -f -v kora-main", "podman rm -f -v kora-main-db", "podman network rm kora-main-net"] {
            assert!(calls.iter().any(|c| c == want), "{want} in {calls:#?}");
        }
        assert!(!calls.iter().any(|c| c.contains("volume rm")), "the data is kept unless asked");
        assert_eq!(done.len(), 3);
        let r = FakeRunner::default();
        r.respond("State.Status", 0, "exited\n");
        Manager::new(d.path(), &rec, &r).remove_all(true).unwrap();
        assert!(r.calls().iter().any(|c| c == "podman volume rm kora-main-dbdata"));
    }

    #[test]
    fn leftovers_are_found_by_label_only() {
        let d = server_dir();
        let rec = sample();
        let r = FakeRunner::default();
        r.respond("ps -a", 0, "kora-main\n");
        let left = Manager::new(d.path(), &rec, &r).leftovers();
        assert_eq!(left, vec!["container kora-main".to_string()]);
        assert!(r.calls().iter().all(|c| c.contains("label=kora.server=main")), "{:?}", r.calls());
    }
}
