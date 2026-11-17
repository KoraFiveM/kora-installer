//! The database container: creating it, waiting for it, and running database tools inside it.
//!
//! * **Credentials** are generated exactly like today's ([`crate::db::generate_name`],
//!   [`crate::db::generate_password`]) and written to `database.json` and `secrets.cfg` by the same function,
//!   with the host set to the container's name on the server's private network (the game container reaches it
//!   there; nothing is published to the host). The image creates the database and the login on its first start
//!   from an env file Kora writes (0600) just before `create` and deletes right after: no password is ever on a
//!   command line, in a job event, or in a log.
//! * **Tools run inside** ([`ExecRunner`]): every database tool of the backup code goes through the
//!   [`StreamRunner`] seam; this one turns a tool command into `<engine> exec -i -e MYSQL_PWD <container> <tool>
//!   ...`. The password stays in the engine client's environment (`-e NAME` forwards it by name), so it is on no
//!   command line here either. Inside the container the login connects to `127.0.0.1:3306`
//!   ([`tool_credentials`]).
//! * **The generated root password** of the image is only valid for `root@localhost` inside the container and is
//!   printed in that container's own log; Kora never uses it and never reads that log.

use std::path::Path;
use std::time::Duration;

use super::engine::{self, Engine};
use super::lifecycle::Manager;
use super::record::ContainerRecord;
use super::spec;
use crate::backup::dbdump::{self, DbFailure, StreamRunner};
use crate::db::{CommandOutput, CommandSpec, Credentials, DbResult, Secret};
use crate::problem::{Context, Problem};

/// The client and dump programs inside the official MariaDB image.
pub const CLIENT: &str = "mariadb";
pub const DUMP: &str = "mariadb-dump";

/// A [`StreamRunner`] that runs every tool inside the database container.
pub struct ExecRunner<R> {
    pub engine: Engine,
    pub container: String,
    pub inner: R,
}

/// A runner behind a reference is a runner.
impl<T: StreamRunner + ?Sized> StreamRunner for &T {
    fn run(&self, spec: &CommandSpec) -> DbResult<CommandOutput> {
        (**self).run(spec)
    }
    fn run_to_file(&self, spec: &CommandSpec, out: &Path) -> DbResult<CommandOutput> {
        (**self).run_to_file(spec, out)
    }
    fn run_from_file(&self, spec: &CommandSpec, input: &Path) -> DbResult<CommandOutput> {
        (**self).run_from_file(spec, input)
    }
}

impl<R: StreamRunner> ExecRunner<R> {
    pub fn new(rec: &ContainerRecord, inner: R) -> Option<Self> {
        rec.db.as_ref().map(|d| ExecRunner { engine: rec.engine(), container: d.container.clone(), inner })
    }

    /// The same tool, run through the engine. A host path to the program (`/usr/bin/mariadb`) becomes its
    /// bare name: the container has its own.
    pub fn wrap(&self, spec: &CommandSpec) -> CommandSpec {
        let tool = Path::new(&spec.program).file_name().and_then(|n| n.to_str()).unwrap_or(&spec.program).to_string();
        let env_names: Vec<&str> = spec.env.iter().map(|(k, _)| k.as_str()).collect();
        CommandSpec { program: self.engine.program().to_string(), args: engine::exec_args(&self.container, &env_names, &tool, &spec.args), env: spec.env.clone(), stdin: spec.stdin.clone() }
    }
}

impl<R: StreamRunner> StreamRunner for ExecRunner<R> {
    fn run(&self, spec: &CommandSpec) -> DbResult<CommandOutput> {
        self.inner.run(&self.wrap(spec))
    }
    fn run_to_file(&self, spec: &CommandSpec, out: &Path) -> DbResult<CommandOutput> {
        self.inner.run_to_file(&self.wrap(spec), out)
    }
    fn run_from_file(&self, spec: &CommandSpec, input: &Path) -> DbResult<CommandOutput> {
        self.inner.run_from_file(&self.wrap(spec), input)
    }
}

impl<R: StreamRunner> crate::db::CommandRunner for ExecRunner<R> {
    fn run(&self, spec: &CommandSpec) -> DbResult<CommandOutput> {
        StreamRunner::run(self, spec)
    }
}

/// The login as seen from inside the database container.
pub fn tool_credentials(c: &Credentials) -> Credentials {
    Credentials { host: "127.0.0.1".into(), port: spec::DB_PORT, database: c.database.clone(), user: c.user.clone(), password: c.password.clone() }
}

/// What the game server (and `secrets.cfg`) connect to.
pub fn game_credentials(slug: &str, database: &str, user: &str, password: &Secret) -> Credentials {
    Credentials { host: spec::db_host(slug), port: spec::DB_PORT, database: database.into(), user: user.into(), password: password.clone() }
}

/// A new database name and login (not password) in Kora's own style.
pub fn new_names() -> DbResult<String> {
    crate::db::generate_name(&Default::default())
}

fn db_problem(f: DbFailure) -> Problem {
    Problem::new(super::codes::DB_NOT_RUNNING, "The database container did not become ready.").with_details(format!("{f:?}"))
}

/// Wait until the server's own login works inside the database container, up to `timeout` (the first start
/// creates the database and can take a minute on a slow disk). `sleep` is a seam for tests.
pub fn wait_ready(runner: &dyn StreamRunner, c: &Credentials, timeout: Duration, sleep: &dyn Fn(Duration)) -> Result<(), Problem> {
    let poll = Duration::from_secs(2);
    let mut waited = Duration::ZERO;
    loop {
        match dbdump::check_login(runner, CLIENT, &tool_credentials(c)) {
            Ok(()) => return Ok(()),
            Err(DbFailure::Login) => return Err(Problem::new(
                super::codes::DB_NOT_RUNNING,
                "The database container is running but refused the server's login. The data volume may belong to an older install; remove it (delete the server and keep nothing) or restore a backup.",
            )
            .with_details("access denied")),
            Err(f) if waited >= timeout => return Err(db_problem(f)),
            Err(_) => {
                sleep(poll);
                waited += poll;
            }
        }
    }
}

/// Create the network, the volume and the database container (not yet started), with the login in `creds`.
/// The env file is written 0600 next to the server's records and removed again even when `create` fails.
pub fn create_container(m: &Manager, creds: &Credentials, progress: &mut dyn FnMut(&str)) -> Result<(), Problem> {
    let rec = m.rec;
    let db = rec.db.as_ref().ok_or_else(|| Problem::generic("this server has no database container"))?;
    m.ensure_image(&db.image, progress)?;
    m.ensure_network()?;
    m.ensure_volume()?;
    let meta = m.dir.join(".kora");
    std::fs::create_dir_all(&meta).map_err(|e| Problem::from(e).context(&format!("creating {}", meta.display())))?;
    let env_path = meta.join("db-init.env");
    let _ = std::fs::remove_file(&env_path);
    let text = spec::db_env_file(&creds.database, &creds.user, creds.password.expose());
    crate::fsutil::write_no_follow(&env_path, text.as_bytes(), 0o600).context("writing the database's start-up file")?;
    struct Remove<'p>(&'p Path);
    impl Drop for Remove<'_> {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(self.0);
        }
    }
    let _guard = Remove(&env_path);
    let path = env_path.to_str().ok_or_else(|| Problem::generic("the server folder's name is not valid text"))?;
    let args = spec::db_create_args(&m.slug(), rec, path).map_err(Problem::generic)?;
    progress("Creating the database container");
    m.engine().run_ok(m.runner, &args).map(|_| ()).map_err(|e| super::detect::problem_for_failure(&m.engine(), "create the database container", &e))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::container::lifecycle::tests::server_dir;
    use crate::container::record::tests::sample;
    use crate::db::SystemRunner;
    #[cfg(unix)]
    use crate::service::testing::FakeRunner;
    use std::cell::RefCell;

    struct Recording(RefCell<Vec<CommandSpec>>, RefCell<Vec<CommandOutput>>);
    impl StreamRunner for Recording {
        fn run(&self, spec: &CommandSpec) -> DbResult<CommandOutput> {
            self.0.borrow_mut().push(spec.clone());
            Ok(self.1.borrow_mut().pop().unwrap_or_default())
        }
        fn run_to_file(&self, spec: &CommandSpec, _: &Path) -> DbResult<CommandOutput> {
            self.run(spec)
        }
        fn run_from_file(&self, spec: &CommandSpec, _: &Path) -> DbResult<CommandOutput> {
            self.run(spec)
        }
    }

    fn creds() -> Credentials {
        game_credentials("main", "kora_ab12cd34", "kora_ab12cd34", &Secret::new("hunter2-secret"))
    }

    #[test]
    fn tools_run_through_exec_with_the_password_only_in_the_environment() {
        let rec = sample();
        let inner = Recording(Default::default(), Default::default());
        let r = ExecRunner::new(&rec, &inner).unwrap();
        let dump = dbdump::dump_spec("/usr/bin/mariadb-dump", &tool_credentials(&creds()), false);
        let w = r.wrap(&dump);
        assert_eq!(w.program, "podman");
        let line = w.args.join(" ");
        assert!(line.starts_with("exec -i -e MYSQL_PWD kora-main-db mariadb-dump --protocol=tcp -h 127.0.0.1 -P 3306 -u kora_ab12cd34 "), "{line}");
        assert!(line.ends_with("kora_ab12cd34"), "database name last: {line}");
        assert!(!line.contains("hunter2-secret") && !w.program.contains("hunter2"), "no password in the argument list");
        assert_eq!(w.env, vec![("MYSQL_PWD".to_string(), "hunter2-secret".to_string())], "it travels in the engine client's environment");
        assert!(!format!("{w:?}").contains("hunter2"), "Debug never shows it");
        let mut none = rec.clone();
        none.db = None;
        assert!(ExecRunner::new(&none, &inner).is_none());
    }

    #[test]
    fn a_sql_statement_goes_in_on_stdin_through_exec() {
        let rec = sample();
        let inner = Recording(Default::default(), Default::default());
        let r = ExecRunner::new(&rec, &inner).unwrap();
        dbdump::check_login(&r, CLIENT, &tool_credentials(&creds())).unwrap();
        let seen = inner.0.borrow();
        assert_eq!(seen[0].program, "podman");
        assert_eq!(seen[0].stdin.as_deref(), Some("SELECT 1;"));
        assert!(seen[0].args.contains(&"mariadb".to_string()));
        let _ = SystemRunner; // the real runner type satisfies the seam
    }

    #[test]
    fn the_game_connects_by_container_name_and_tools_by_loopback() {
        let c = creds();
        assert_eq!(c.host, "kora-main-db");
        assert_eq!(c.port, 3306);
        assert!(c.server_cfg_line().contains("@kora-main-db:3306/kora_ab12cd34"));
        let t = tool_credentials(&c);
        assert_eq!((t.host.as_str(), t.port), ("127.0.0.1", 3306));
    }

    // Container mode is Linux-only (`Facts::system` offers no engine elsewhere) and its folders are POSIX paths.
    #[cfg(unix)]
    #[test]
    fn creating_the_database_container_writes_the_env_file_privately_and_removes_it() {
        let d = server_dir();
        let rec = sample();
        let r = FakeRunner::default();
        // image missing → pull; network/volume missing → create
        r.respond("image inspect", 1, "");
        r.respond("network inspect", 1, "");
        r.respond("volume inspect", 1, "");
        let m = Manager::new(d.path(), &rec, &r);
        create_container(&m, &creds(), &mut |_| {}).unwrap();
        let calls = r.calls();
        let pos = |n: &str| calls.iter().position(|c| c.contains(n)).unwrap_or_else(|| panic!("{n} in {calls:#?}"));
        assert!(pos("pull docker.io/library/mariadb@sha256:") < pos("network create"));
        assert!(pos("network create") < pos("volume create") && pos("volume create") < pos("create --name kora-main-db"));
        let create = calls.iter().find(|c| c.contains("create --name kora-main-db")).unwrap();
        assert!(!create.contains("hunter2-secret"), "{create}");
        assert!(create.contains("--env-file ") && create.contains(".kora/db-init.env"));
        assert!(!d.path().join(".kora/db-init.env").exists(), "the file with the password is gone again");
        assert!(calls.iter().all(|c| !c.contains("hunter2-secret")));
    }

    #[test]
    fn waiting_retries_while_it_starts_and_stops_at_a_refused_login() {
        let rec = sample();
        let _ = rec;
        let n = RefCell::new(0u32);
        struct Flaky<'a>(&'a RefCell<u32>);
        impl StreamRunner for Flaky<'_> {
            fn run(&self, _: &CommandSpec) -> DbResult<CommandOutput> {
                *self.0.borrow_mut() += 1;
                if *self.0.borrow() < 3 {
                    Ok(CommandOutput { status: 1, stdout: String::new(), stderr: "ERROR 2002 (HY000): Can't connect to MySQL server".into() })
                } else {
                    Ok(CommandOutput::default())
                }
            }
            fn run_to_file(&self, s: &CommandSpec, _: &Path) -> DbResult<CommandOutput> {
                self.run(s)
            }
            fn run_from_file(&self, s: &CommandSpec, _: &Path) -> DbResult<CommandOutput> {
                self.run(s)
            }
        }
        let slept = RefCell::new(0u32);
        wait_ready(&Flaky(&n), &creds(), Duration::from_secs(60), &|_| *slept.borrow_mut() += 1).unwrap();
        assert_eq!((*n.borrow(), *slept.borrow()), (3, 2));

        let denied = Recording(Default::default(), RefCell::new(vec![CommandOutput { status: 1, stdout: String::new(), stderr: "ERROR 1045 (28000): Access denied for user".into() }]));
        let e = wait_ready(&denied, &creds(), Duration::from_secs(60), &|_| panic!("no waiting on a refused login")).unwrap_err();
        assert!(e.message.contains("refused"));

        let never = Recording(Default::default(), RefCell::new(vec![CommandOutput { status: 1, stdout: String::new(), stderr: "Can't connect".into() }; 40]));
        assert!(wait_ready(&never, &creds(), Duration::from_secs(4), &|_| {}).is_err(), "gives up after the timeout");
    }
}
