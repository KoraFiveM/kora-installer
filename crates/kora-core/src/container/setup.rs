//! Setting a container server up: shared by the install steps and by importing a backup as a container server.
//!
//! Each function does one stage and returns plain words; the callers decide how to report them. Nothing here
//! writes a secret anywhere but `database.json` (0600, through the same function every database login uses).

use std::path::Path;
use std::time::Duration;

use super::dbc;
use super::detect::{self, ContainerCheck, EngineStatus};
use super::engine::{self, ContainerState, Engine, EngineKind};
use super::lifecycle::Manager;
use super::pins;
use super::record::{ContainerRecord, DbRecord};
use super::{codes, ContainerRequest};
use crate::db::{self, Credentials};
use crate::problem::{Action, Problem};

fn no_engine(check: &ContainerCheck, wanted: Option<EngineKind>) -> Problem {
    let mut p = match (wanted, check.engines.iter().find(|e| wanted.is_none_or(|w| e.kind == w)).and_then(|e| e.problem.clone())) {
        (_, Some(p)) => p,
        (Some(w), None) => Problem::new(codes::NO_ENGINE, format!("{} was chosen to run the server, but it was not found on this computer.", w.label())),
        (None, None) => {
            Problem::new(codes::NO_ENGINE, "Container mode needs Podman or Docker, and neither was found on this computer. Podman (rootless) needs no administrator rights to run servers.")
        }
    };
    p.push_action(Action::open_link("https://podman.io/docs/installation"));
    p.push_action(Action::change_answers("run_mode"));
    p
}

/// Find an engine that can really run a container (smoke run included).
pub fn choose_engine(want: Option<EngineKind>) -> Result<(Engine, Option<EngineStatus>, ContainerCheck), Problem> {
    if !cfg!(target_os = "linux") {
        return Err(Problem::generic("Container mode is only available on Linux."));
    }
    let check = detect::check_system();
    let Some(engine) = check.engine_for(want) else { return Err(no_engine(&check, want)) };
    let status = check.engines.iter().find(|e| e.kind == engine.kind).cloned();
    Ok((engine, status, check))
}

#[cfg(unix)]
fn dir_owner_ids(dir: &Path) -> Option<(u32, u32)> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(dir).ok().map(|m| (m.uid(), m.gid()))
}

#[cfg(not(unix))]
fn dir_owner_ids(_dir: &Path) -> Option<(u32, u32)> {
    None
}

/// The folder's `kora-container.json` for a new container server (not written; the caller saves it). The
/// container runs as the folder's owner and never as root.
pub fn new_record(dir: &Path, slug: &str, engine: &Engine, status: Option<&EngineStatus>, want: &ContainerRequest, with_db: bool) -> Result<ContainerRecord, String> {
    let (uid, gid) = dir_owner_ids(dir).ok_or_else(|| "cannot read who owns the server folder".to_string())?;
    if uid == 0 || gid == 0 {
        return Err("The server folder belongs to root. A container server runs as the folder's owner and never as root: choose a folder in your own home, or change its owner.".into());
    }
    super::spec::check_dir(dir)?;
    let rec = ContainerRecord {
        version: super::record::VERSION,
        engine: engine.kind,
        rootless: engine.rootless,
        image: pins::BASE.reference(),
        container: super::container_name(slug),
        uid,
        gid,
        bind: want.bind.clone(),
        memory_mb: want.memory_mb,
        cpu_tenths: want.cpu_tenths,
        selinux_label: status.is_some_and(|s| s.selinux),
        read_only_root: false,
        host_network: false,
        db: with_db.then(|| DbRecord { container: super::db_container_name(slug), volume: super::volume_name(slug), network: super::network_name(slug), image: pins::MARIADB.reference() }),
    };
    rec.check(slug)?;
    Ok(rec)
}

/// Make the server's database container ready and return its login. The login is written to
/// `database.json` BEFORE the container exists (the volume is initialised with it on the first start), so a retry
/// reuses it; a data volume left over from a deleted server whose login is lost is refused, never reused.
pub fn ensure_database(m: &Manager, slug: &str, progress: &mut dyn FnMut(&str), on_new_login: &mut dyn FnMut(&Credentials)) -> Result<Credentials, Problem> {
    let rec = m.rec;
    let db = rec.db.clone().ok_or_else(|| Problem::generic("this server was not set up with a database container"))?;
    let path = m.dir.join("database.json");
    let reused: Option<Credentials> = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let creds = match reused {
        Some(c) => c,
        None => {
            let vol_exists = Engine::new(rec.engine, rec.rootless).run(m.runner, &engine::volume_inspect_args(&db.volume)).is_ok_and(|o| o.success());
            if vol_exists {
                return Err(Problem::new(
                    codes::NAME_TAKEN,
                    format!(
                        "A database volume called {} is already there, probably kept when an earlier server with this name was deleted. Its login is not known any more, so Kora will not use it. Choose another server name, or remove the volume yourself (`{} volume rm {}`) if you do not need its data.",
                        db.volume,
                        rec.engine().program(),
                        db.volume
                    ),
                )
                .with_action(Action::change_answers("name")));
            }
            let name = dbc::new_names().map_err(|e| Problem::generic(e.to_string()))?;
            let pw = db::generate_password().map_err(|e| Problem::generic(e.to_string()))?;
            let creds = dbc::game_credentials(slug, &name, &name, &pw);
            let json = serde_json::to_string_pretty(&creds).map_err(|e| Problem::generic(e.to_string()))?;
            db::write_secret_file(&db::SystemRunner, db::Os::current(), &path, &json, None).map_err(|e| Problem::generic(e.to_string()))?;
            on_new_login(&creds);
            creds
        }
    };
    if matches!(m.db_state(), Some(ContainerState::Missing)) {
        dbc::create_container(m, &creds, progress)?;
    }
    progress("Starting the database container");
    m.start_db()?;
    progress("Waiting for the database to be ready (the first start takes a little while)");
    let exec = dbc::ExecRunner::new(rec, db::SystemRunner).ok_or_else(|| Problem::generic("no database container"))?;
    dbc::wait_ready(&exec, &creds, Duration::from_secs(180), &std::thread::sleep)?;
    Ok(creds)
}

/// What setting up start-at-boot came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Boot {
    /// One line for the step's detail.
    pub detail: String,
    /// What the person must run (or what to know), for the notes. Empty when everything worked.
    pub notes: Vec<String>,
}

/// Create the game container (not started) and set up start at boot. Docker's restart policy needs nothing
/// more (its daemon is enabled by the system); rootless Podman's restarts come from the stock user unit
/// `podman-restart.service`, which needs lingering to run while nobody is signed in. Both are the person's own
/// account: no administrator rights.
pub fn create_game(m: &Manager, progress: &mut dyn FnMut(&str)) -> Result<Boot, Problem> {
    m.ensure_game_container(progress)?;
    let rec = m.rec;
    let mut detail = format!("{} restarts the server by itself (restart policy) and at boot", rec.engine().label());
    let mut notes = Vec::new();
    if rec.engine == EngineKind::Podman && rec.rootless {
        let sys = crate::service::SystemRunner;
        let mut failed = Vec::new();
        for (program, args) in [("systemctl", vec!["--user", "enable", "podman-restart.service"]), ("loginctl", vec!["enable-linger"])] {
            let args: Vec<String> = args.into_iter().map(String::from).collect();
            let line = engine::command_text(program, &args);
            let ok = crate::service::CommandRunner::run(&sys, program, &args).is_ok_and(|o| o.success());
            if !ok {
                failed.push(line);
            }
        }
        if failed.is_empty() {
            detail = "Podman starts the server at boot (podman-restart.service, lingering on)".into();
        } else {
            notes.push(format!(
                "The server will run, but may not start again after a restart of this computer until you run: {}. (Rootless Podman needs the user unit `podman-restart.service` enabled and lingering on for your account.)",
                failed.join(" ; ")
            ));
            detail = "created; start at boot needs the command in the notes".into();
        }
    }
    Ok(Boot { detail, notes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::lifecycle::tests::server_dir;
    use crate::container::record::tests::sample;
    use crate::service::testing::FakeRunner;

    #[test]
    fn a_new_record_names_this_servers_objects_and_refuses_root_owned_folders() {
        let d = tempfile::tempdir().unwrap();
        let e = Engine::new(EngineKind::Podman, true);
        let want = ContainerRequest { bind: Some("203.0.113.9".into()), memory_mb: Some(2048), ..Default::default() };
        let rec = new_record(d.path(), "main", &e, None, &want, true);
        #[cfg(unix)]
        {
            // the temp folder is the test user's own (not root)
            let me = unsafe { libc::getuid() };
            if me != 0 {
                let rec = rec.unwrap();
                assert_eq!((rec.container.as_str(), rec.db.as_ref().unwrap().container.as_str()), ("kora-main", "kora-main-db"));
                assert_eq!((rec.bind.as_deref(), rec.memory_mb, rec.uid), (Some("203.0.113.9"), Some(2048), me));
                assert!(rec.check("main").is_ok());
            }
        }
        assert!(new_record(Path::new("/"), "main", &e, None, &want, false).is_err());
    }

    #[test]
    fn a_leftover_volume_with_no_login_is_never_reused() {
        let d = server_dir();
        let rec = sample();
        let r = FakeRunner::default();
        r.respond("volume inspect", 0, "kora-main-dbdata\n");
        let m = Manager::new(d.path(), &rec, &r);
        let e = ensure_database(&m, "main", &mut |_| {}, &mut |_| {}).unwrap_err();
        assert!(e.has_code(codes::NAME_TAKEN) && e.message.contains("kora-main-dbdata"), "{e:?}");
        assert!(!d.path().join("database.json").exists());
        assert!(!r.calls().iter().any(|c| c.contains(" create ") || c.contains(" start ")), "{:?}", r.calls());
    }
}
