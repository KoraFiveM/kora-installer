//! The server's own database, dumped and loaded again.
//!
//! * The dump is LOGICAL (SQL text), made by `mariadb-dump` / `mysqldump` with `--single-transaction`
//!   (one consistent snapshot of the InnoDB tables while the game keeps writing), `--routines`,
//!   `--triggers`, `--events`. Only the server's own database is dumped, with the server's own login from
//!   `database.json`: never the administrator's, never other databases.
//! * The password goes in the environment (`MYSQL_PWD`), never on the command line (where any program on
//!   the machine could read it), like the rest of Kora's database code.
//! * Everything runs through [`StreamRunner`], the streaming sibling of [`crate::db::CommandRunner`], so
//!   tests use a fake and no real database is needed. Dumps are written to and loaded from FILES (they can
//!   be gigabytes), never held in memory.
//! * Tables that use MyISAM/Aria (not transactional) are dumped as they are at that moment: "consistent"
//!   is promised only for InnoDB, which is what every FiveM framework uses. The manifest says so when the
//!   dump tool warns about it.

use std::path::Path;

use crate::db::{CommandOutput, CommandSpec, Credentials, DbError, DbResult, SystemRunner};

/// Runs a database tool with its input or output as a file.
pub trait StreamRunner {
    /// Capture everything (small answers only).
    fn run(&self, spec: &CommandSpec) -> DbResult<CommandOutput>;
    /// The tool's standard output goes to `out` (a new file, owner-only); the error output is returned.
    fn run_to_file(&self, spec: &CommandSpec, out: &Path) -> DbResult<CommandOutput>;
    /// The tool reads `input` as its standard input.
    fn run_from_file(&self, spec: &CommandSpec, input: &Path) -> DbResult<CommandOutput>;
}

fn child_for(spec: &CommandSpec) -> std::process::Command {
    let mut cmd = crate::sysexe::command(&spec.program);
    cmd.args(&spec.args);
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    cmd
}

/// Keep what a tool printed on its error channel short and free of statements.
fn tidy_stderr(raw: &[u8]) -> String {
    let s = String::from_utf8_lossy(raw);
    s.chars().take(4000).collect()
}

impl StreamRunner for SystemRunner {
    fn run(&self, spec: &CommandSpec) -> DbResult<CommandOutput> {
        crate::db::CommandRunner::run(self, spec)
    }

    fn run_to_file(&self, spec: &CommandSpec, out: &Path) -> DbResult<CommandOutput> {
        use std::process::Stdio;
        let file = create_private(out)?;
        let child = child_for(spec).stdin(Stdio::null()).stdout(Stdio::from(file)).stderr(Stdio::piped()).spawn().map_err(|e| DbError::Command(format!("{}: {e}", spec.program)))?;
        let o = child.wait_with_output()?;
        Ok(CommandOutput { status: o.status.code().unwrap_or(-1), stdout: String::new(), stderr: tidy_stderr(&o.stderr) })
    }

    fn run_from_file(&self, spec: &CommandSpec, input: &Path) -> DbResult<CommandOutput> {
        use std::process::Stdio;
        let file = std::fs::File::open(input)?;
        let child = child_for(spec).stdin(Stdio::from(file)).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| DbError::Command(format!("{}: {e}", spec.program)))?;
        let o = child.wait_with_output()?;
        Ok(CommandOutput { status: o.status.code().unwrap_or(-1), stdout: String::from_utf8_lossy(&o.stdout).chars().take(4000).collect(), stderr: tidy_stderr(&o.stderr) })
    }
}

/// A new file only its owner can read (a dump holds the game's data).
pub fn create_private(path: &Path) -> std::io::Result<std::fs::File> {
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path)
}

// ---------------------------------------------------------------- what went wrong

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbFailure {
    /// The tool is not installed (or not where Kora looked).
    ToolMissing,
    /// Nothing answers on the database's address: it is not running.
    NotRunning,
    /// The login is refused (the password in `database.json` no longer fits).
    Login,
    /// The database named in the login does not exist.
    NoSuchDatabase,
    /// No room to write.
    DiskFull,
    Other(String),
}

/// Read a failed tool's error channel.
pub fn classify(stderr: &str) -> DbFailure {
    let s = stderr.to_ascii_lowercase();
    if s.contains("no space left") || s.contains("disk full") || s.contains("errcode: 28") || s.contains("there is not enough space") {
        DbFailure::DiskFull
    } else if s.contains("can't connect") || s.contains("connection refused") || s.contains("(2002)") || s.contains("(111)") || s.contains("can not connect") {
        DbFailure::NotRunning
    } else if s.contains("access denied") || s.contains("(1045)") {
        DbFailure::Login
    } else if s.contains("unknown database") || s.contains("(1049)") {
        DbFailure::NoSuchDatabase
    } else {
        // The tools print harmless "Warning: ..." lines first; the line that says why it stopped is the first one that is not a warning.
        let lines = || stderr.lines().map(str::trim).filter(|l| !l.is_empty());
        let line = lines().find(|l| !l.to_ascii_lowercase().starts_with("warning")).or_else(|| lines().next()).unwrap_or("the database tool failed");
        DbFailure::Other(line.chars().take(300).collect())
    }
}

// ---------------------------------------------------------------- the tools

/// The dump tool that goes with this database client: next to it when the client is a full path,
/// otherwise the first of `mariadb-dump`, `mysqldump` that answers. `exists` says whether a program can
/// be run (a path is a file, a bare name answers `--version`).
pub fn dump_program(client: &str, exists: &dyn Fn(&str) -> bool) -> Option<String> {
    let exe = if cfg!(windows) { ".exe" } else { "" };
    let path = Path::new(client);
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        // Prefer the one that matches the client (mariadb → mariadb-dump), then the other.
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
        let order = if stem.starts_with("mysql") { ["mysqldump", "mariadb-dump"] } else { ["mariadb-dump", "mysqldump"] };
        for n in order {
            let cand = dir.join(format!("{n}{exe}")).to_string_lossy().into_owned();
            if exists(&cand) {
                return Some(cand);
            }
        }
    }
    ["mariadb-dump", "mysqldump"].into_iter().find(|n| exists(n)).map(str::to_string)
}

/// `exists` for the real machine.
pub fn program_runs(p: &str) -> bool {
    if Path::new(p).is_absolute() || p.contains(['/', '\\']) {
        return Path::new(p).is_file();
    }
    crate::sysexe::command(p).arg("--version").stdin(std::process::Stdio::null()).output().map(|o| o.status.success()).unwrap_or(false)
}

fn connection_args(c: &Credentials) -> Vec<String> {
    vec!["--protocol=tcp".into(), "-h".into(), c.host.clone(), "-P".into(), c.port.to_string(), "-u".into(), c.user.clone()]
}

fn env_for(c: &Credentials) -> Vec<(String, String)> {
    vec![("MYSQL_PWD".into(), c.password.expose().to_string())]
}

/// The dump command: one database, one snapshot, with routines, triggers and events.
pub fn dump_spec(program: &str, c: &Credentials, no_tablespaces: bool) -> CommandSpec {
    let mut args = connection_args(c);
    args.extend(
        ["--single-transaction", "--quick", "--routines", "--triggers", "--events", "--hex-blob", "--default-character-set=utf8mb4", "--skip-comments", "--no-tablespaces"]
            .iter()
            .map(|s| s.to_string()),
    );
    if !no_tablespaces {
        args.retain(|a| a != "--no-tablespaces");
    }
    args.push(c.database.clone());
    CommandSpec { program: program.to_string(), args, env: env_for(c), stdin: None }
}

/// Dump the server's database into `out`. Returns the size. A MySQL server that wants the PROCESS
/// privilege for tablespaces is retried without them (the dump does not need them).
pub fn dump(r: &dyn StreamRunner, program: &str, c: &Credentials, out: &Path) -> Result<u64, DbFailure> {
    let attempt = |no_ts: bool| -> Result<CommandOutput, DbFailure> {
        let _ = std::fs::remove_file(out);
        r.run_to_file(&dump_spec(program, c, no_ts), out).map_err(|e| match e {
            DbError::Io(io) if io.kind() == std::io::ErrorKind::StorageFull => DbFailure::DiskFull,
            DbError::Io(io) => DbFailure::Other(io.to_string()),
            DbError::Command(m) if m.contains("No such file") || m.contains("cannot find") || m.contains("not found") => DbFailure::ToolMissing,
            other => DbFailure::Other(other.to_string()),
        })
    };
    let mut o = attempt(false)?;
    if !o.ok() && o.stderr.to_ascii_lowercase().contains("process") && o.stderr.to_ascii_lowercase().contains("privilege") {
        o = attempt(true)?;
    }
    if !o.ok() {
        let _ = std::fs::remove_file(out);
        return Err(classify(&o.stderr));
    }
    let size = std::fs::metadata(out).map(|m| m.len()).unwrap_or(0);
    // A dump always ends with this line; a cut-off one does not (the tool died, the disk filled).
    if !dump_is_complete(out) {
        let _ = std::fs::remove_file(out);
        return Err(DbFailure::Other("the database dump was cut off before the end".into()));
    }
    Ok(size)
}

/// Does the file end with the trailer `mariadb-dump`/`mysqldump` always writes ("-- Dump completed" is a comment
/// that `--skip-comments` removes, so the check is on the unconditional restore statements at the end).
pub fn dump_is_complete(path: &Path) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else { return false };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    if len == 0 {
        return false;
    }
    let from = len.saturating_sub(2048);
    let mut tail = Vec::new();
    if f.seek(SeekFrom::Start(from)).is_err() || f.read_to_end(&mut tail).is_err() {
        return false;
    }
    let t = String::from_utf8_lossy(&tail);
    t.contains("SET SQL_MODE=@OLD_SQL_MODE") || t.contains("@OLD_SQL_MODE") || t.contains("Dump completed") || t.contains("SET TIME_ZONE=@OLD_TIME_ZONE")
}

fn client_spec(client: &str, c: &Credentials, sql: Option<&str>) -> CommandSpec {
    let mut args = vec!["--batch".to_string(), "--skip-column-names".into(), "--default-character-set=utf8mb4".into()];
    args.extend(connection_args(c));
    // The client takes a connect timeout; mysqldump/mariadb-dump do NOT ("unknown variable 'connect-timeout=10'").
    args.push("--connect-timeout=10".into());
    args.push(c.database.clone());
    CommandSpec { program: client.to_string(), args, env: env_for(c), stdin: sql.map(str::to_string) }
}

/// Can the server's own login connect right now? `Ok` or why not.
pub fn check_login(r: &dyn StreamRunner, client: &str, c: &Credentials) -> Result<(), DbFailure> {
    match r.run(&client_spec(client, c, Some("SELECT 1;"))) {
        Ok(o) if o.ok() => Ok(()),
        Ok(o) => Err(classify(&o.stderr)),
        Err(DbError::Command(m)) if m.contains("No such file") || m.contains("cannot find") || m.contains("not found") => Err(DbFailure::ToolMissing),
        Err(e) => Err(DbFailure::Other(e.to_string())),
    }
}

/// The size of the database's data, for the space check. `None` when it cannot be asked.
pub fn data_bytes(r: &dyn StreamRunner, client: &str, c: &Credentials) -> Option<u64> {
    let sql = format!("SELECT COALESCE(SUM(data_length + index_length), 0) FROM information_schema.tables WHERE table_schema = {};", crate::db::quote_string(&c.database).ok()?);
    let o = r.run(&client_spec(client, c, Some(&sql))).ok()?;
    o.ok().then(|| o.stdout.trim().parse().ok()).flatten()
}

/// The names of the tables and views in the database.
pub fn list_tables(r: &dyn StreamRunner, client: &str, c: &Credentials) -> Result<Vec<(String, bool)>, DbFailure> {
    let sql = format!("SELECT table_name, table_type FROM information_schema.tables WHERE table_schema = {};", crate::db::quote_string(&c.database).map_err(|e| DbFailure::Other(e.to_string()))?);
    let o = r.run(&client_spec(client, c, Some(&sql))).map_err(|e| DbFailure::Other(e.to_string()))?;
    if !o.ok() {
        return Err(classify(&o.stderr));
    }
    Ok(o.stdout
        .lines()
        .filter_map(|l| {
            let (name, kind) = l.split_once('\t')?;
            Some((name.to_string(), kind.trim().eq_ignore_ascii_case("VIEW")))
        })
        .collect())
}

/// A table name in backticks (any characters MariaDB allows; the backtick is doubled).
fn quote_name(name: &str) -> DbResult<String> {
    if name.is_empty() || name.chars().any(|c| c.is_control()) {
        return Err(DbError::InvalidIdentifier(format!("{name:?}")));
    }
    Ok(format!("`{}`", name.replace('`', "``")))
}

/// SQL that empties the database of tables and views, so a restore brings back exactly what the backup had.
pub fn wipe_sql(tables: &[(String, bool)]) -> DbResult<String> {
    let mut sql = String::from("SET FOREIGN_KEY_CHECKS=0;\n");
    for (name, view) in tables {
        sql.push_str(&format!("DROP {} IF EXISTS {};\n", if *view { "VIEW" } else { "TABLE" }, quote_name(name)?));
    }
    sql.push_str("SET FOREIGN_KEY_CHECKS=1;\n");
    Ok(sql)
}

/// Load a dump into the server's database: first drop what is there, then run the file.
pub fn restore(r: &dyn StreamRunner, client: &str, c: &Credentials, dump_file: &Path) -> Result<(), DbFailure> {
    let tables = list_tables(r, client, c)?;
    if !tables.is_empty() {
        let sql = wipe_sql(&tables).map_err(|e| DbFailure::Other(e.to_string()))?;
        let o = r.run(&client_spec(client, c, Some(&sql))).map_err(|e| DbFailure::Other(e.to_string()))?;
        if !o.ok() {
            return Err(classify(&o.stderr));
        }
    }
    let o = r.run_from_file(&client_spec(client, c, None), dump_file).map_err(|e| match e {
        DbError::Io(io) => DbFailure::Other(io.to_string()),
        other => DbFailure::Other(other.to_string()),
    })?;
    if !o.ok() {
        return Err(classify(&o.stderr));
    }
    Ok(())
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::cell::RefCell;

    /// A database that is a map of table name → rows, understood just enough for the tests: it
    /// "dumps" to a text file and "loads" it back, answering the few queries the code asks.
    #[derive(Default)]
    pub struct FakeDb {
        pub tables: RefCell<Vec<(String, Vec<String>)>>,
        pub calls: RefCell<Vec<CommandSpec>>,
        pub fail_with: RefCell<Option<String>>,
        pub process_privilege_first: RefCell<bool>,
    }

    impl StreamRunner for FakeDb {
        fn run(&self, spec: &CommandSpec) -> DbResult<CommandOutput> {
            self.calls.borrow_mut().push(spec.clone());
            if let Some(m) = self.fail_with.borrow().clone() {
                return Ok(CommandOutput { status: 1, stdout: String::new(), stderr: m });
            }
            let sql = spec.stdin.clone().unwrap_or_default();
            let mut out = String::new();
            if sql.contains("information_schema.tables WHERE") && sql.contains("table_name") {
                for (t, _) in self.tables.borrow().iter() {
                    out.push_str(&format!("{t}\tBASE TABLE\n"));
                }
            } else if sql.contains("SUM(data_length") {
                out = "12345\n".into();
            } else if sql.contains("DROP TABLE") {
                self.tables.borrow_mut().clear();
            }
            Ok(CommandOutput { status: 0, stdout: out, stderr: String::new() })
        }

        fn run_to_file(&self, spec: &CommandSpec, out: &Path) -> DbResult<CommandOutput> {
            self.calls.borrow_mut().push(spec.clone());
            if std::mem::take(&mut *self.process_privilege_first.borrow_mut()) {
                return Ok(CommandOutput {
                    status: 2,
                    stdout: String::new(),
                    stderr: "mysqldump: Error: 'Access denied; you need (at least one of) the PROCESS privilege(s) for this operation' when trying to dump tablespaces".into(),
                });
            }
            if let Some(m) = self.fail_with.borrow().clone() {
                return Ok(CommandOutput { status: 2, stdout: String::new(), stderr: m });
            }
            let mut text = String::from("-- fake dump\n");
            for (t, rows) in self.tables.borrow().iter() {
                text.push_str(&format!("TABLE {t}\n"));
                for r in rows {
                    text.push_str(&format!("ROW {r}\n"));
                }
            }
            text.push_str("/*!40101 SET SQL_MODE=@OLD_SQL_MODE */;\n");
            std::fs::write(out, text)?;
            Ok(CommandOutput::default())
        }

        fn run_from_file(&self, spec: &CommandSpec, input: &Path) -> DbResult<CommandOutput> {
            self.calls.borrow_mut().push(spec.clone());
            if let Some(m) = self.fail_with.borrow().clone() {
                return Ok(CommandOutput { status: 1, stdout: String::new(), stderr: m });
            }
            let text = std::fs::read_to_string(input)?;
            let mut tables: Vec<(String, Vec<String>)> = Vec::new();
            for l in text.lines() {
                if let Some(t) = l.strip_prefix("TABLE ") {
                    tables.push((t.to_string(), Vec::new()));
                } else if let Some(r) = l.strip_prefix("ROW ") {
                    tables.last_mut().unwrap().1.push(r.to_string());
                }
            }
            *self.tables.borrow_mut() = tables;
            Ok(CommandOutput::default())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeDb;
    use super::*;
    use crate::db::Secret;

    pub(crate) fn creds() -> Credentials {
        Credentials { host: "127.0.0.1".into(), port: 3306, database: "kora_abc".into(), user: "kora_abc".into(), password: Secret::new("s3cr3t-pass") }
    }

    #[test]
    fn the_dump_command_has_no_password_and_dumps_only_the_servers_database() {
        let s = dump_spec("mariadb-dump", &creds(), false);
        let line = s.args.join(" ");
        assert!(!line.contains("s3cr3t"), "password on the command line: {line}");
        assert!(s.env.iter().any(|(k, v)| k == "MYSQL_PWD" && v == "s3cr3t-pass"));
        for want in ["--single-transaction", "--routines", "--triggers", "--events", "-u kora_abc"] {
            assert!(line.contains(want), "{want} missing in {line}");
        }
        assert_eq!(s.args.last().unwrap(), "kora_abc");
        assert!(!line.contains("--all-databases") && !line.contains("--databases"));
        assert!(!line.contains("--no-tablespaces"));
        assert!(!line.contains("connect-timeout"), "the dump tools reject it: {line}");
        assert!(dump_spec("x", &creds(), true).args.contains(&"--no-tablespaces".to_string()));
        assert!(!format!("{s:?}").contains("s3cr3t"));
    }

    #[test]
    fn dump_program_follows_the_client() {
        let has = |set: &'static [&'static str]| move |p: &str| set.contains(&p);
        // Slash paths joined with Windows' separator and ".exe" would not match the same strings: Unix only.
        #[cfg(unix)]
        {
            assert_eq!(dump_program("/usr/bin/mariadb", &has(&["/usr/bin/mariadb-dump", "/usr/bin/mysqldump"])).as_deref(), Some("/usr/bin/mariadb-dump"));
            assert_eq!(dump_program("/opt/my/bin/mysql", &has(&["/opt/my/bin/mysqldump", "/opt/my/bin/mariadb-dump"])).as_deref(), Some("/opt/my/bin/mysqldump"));
        }
        assert_eq!(dump_program("mysql", &has(&["mysqldump"])).as_deref(), Some("mysqldump"));
        assert_eq!(dump_program("mariadb", &has(&["mariadb-dump", "mysqldump"])).as_deref(), Some("mariadb-dump"));
        assert_eq!(dump_program("mariadb", &has(&[])), None);
    }

    #[test]
    fn failures_are_told_apart() {
        assert_eq!(classify("ERROR 2002 (HY000): Can't connect to local server through socket"), DbFailure::NotRunning);
        assert_eq!(classify("mysqldump: Got error: 2002: \"Can't connect to MySQL server on '127.0.0.1' (111)\""), DbFailure::NotRunning);
        assert_eq!(classify("ERROR 1045 (28000): Access denied for user 'x'@'localhost' (using password: YES)"), DbFailure::Login);
        assert_eq!(classify("ERROR 1049 (42000): Unknown database 'x'"), DbFailure::NoSuchDatabase);
        assert_eq!(classify("Error 28: No space left on device"), DbFailure::DiskFull);
        assert!(matches!(classify("something odd\nsecond"), DbFailure::Other(m) if m == "something odd"));
        assert!(matches!(classify("Warning: Could not set terminology mode\nmysqldump: Couldn't execute 'x': boom (1234)"), DbFailure::Other(m) if m.contains("boom")), "a warning is not the reason");
    }

    #[test]
    fn dump_then_wipe_and_restore_brings_the_row_back() {
        let d = tempfile::tempdir().unwrap();
        let db = FakeDb::default();
        db.tables.borrow_mut().push(("players".into(), vec!["alice".into(), "bob".into()]));
        let out = d.path().join("dump.sql");
        let n = dump(&db, "mariadb-dump", &creds(), &out).unwrap();
        assert!(n > 0);
        // somebody wipes the table
        db.tables.borrow_mut()[0].1.clear();
        restore(&db, "mariadb", &creds(), &out).unwrap();
        assert_eq!(db.tables.borrow()[0].1, vec!["alice".to_string(), "bob".to_string()]);
        // the existing tables were dropped first
        assert!(db.calls.borrow().iter().any(|c| c.stdin.as_deref().is_some_and(|s| s.contains("DROP TABLE IF EXISTS `players`"))));
        // no call ever carried the password in its arguments
        assert!(db.calls.borrow().iter().all(|c| !c.args.join(" ").contains("s3cr3t")));
    }

    #[test]
    fn a_process_privilege_refusal_is_retried_without_tablespaces() {
        let d = tempfile::tempdir().unwrap();
        let db = FakeDb::default();
        *db.process_privilege_first.borrow_mut() = true;
        db.tables.borrow_mut().push(("t".into(), vec![]));
        dump(&db, "mysqldump", &creds(), &d.path().join("x.sql")).unwrap();
        let calls = db.calls.borrow();
        assert_eq!(calls.len(), 2);
        assert!(!calls[0].args.contains(&"--no-tablespaces".to_string()) && calls[1].args.contains(&"--no-tablespaces".to_string()));
    }

    #[test]
    fn a_failed_dump_leaves_no_file_and_says_why() {
        let d = tempfile::tempdir().unwrap();
        let db = FakeDb::default();
        *db.fail_with.borrow_mut() = Some("Can't connect to MySQL server (111)".into());
        let out = d.path().join("x.sql");
        assert_eq!(dump(&db, "mysqldump", &creds(), &out), Err(DbFailure::NotRunning));
        assert!(!out.exists());
    }

    #[test]
    fn a_cut_off_dump_is_not_accepted() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.sql");
        std::fs::write(&p, "CREATE TABLE a (x int);\nINSERT INTO a VALUES (1);\n").unwrap();
        assert!(!dump_is_complete(&p));
        std::fs::write(&p, "CREATE TABLE a (x int);\n/*!40101 SET SQL_MODE=@OLD_SQL_MODE */;\n").unwrap();
        assert!(dump_is_complete(&p));
    }

    #[test]
    fn wipe_quotes_names() {
        let sql = wipe_sql(&[("a`b".into(), false), ("v".into(), true)]).unwrap();
        assert!(sql.contains("DROP TABLE IF EXISTS `a``b`;") && sql.contains("DROP VIEW IF EXISTS `v`;"));
    }
}
