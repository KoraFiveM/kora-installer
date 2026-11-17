//! Making a backup of one server.
//!
//! Order of work (each step reported as a job event): check the destination and space → (optionally)
//! stop the server → dump the database → write the archive (header, dump, server files, Kora's records,
//! resources kept in full, closing manifest) → hand it to the destination → remove old backups under the
//! retention rules → start the server again if Kora stopped it. A failure at any point removes the
//! half-made file, starts the server again, and is recorded as the last failure (shown on the Overview,
//! and for a scheduled backup sent to the person's alerts).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::archive::{self, ArchiveWriter, DUMP, KORA_DIR, RESOURCES_DIR, SERVER_DIR};
use super::crypt::{check_passphrase, Passphrase};
use super::dbdump::{self, DbFailure};
use super::dest::{self, DestError, Destination};
use super::types::*;
use super::{clock, codes, settings, BackupEvent, Host, Tools};
use crate::db::Credentials;
use crate::installer::registry::{ServerEntry, ServerFile};
use crate::problem::{Action, Problem, Tool};

pub struct CreateOptions {
    pub passphrase: Option<Passphrase>,
    pub resources: ResourceChoice,
    /// Stop the server while the backup runs (strictly consistent).
    pub stop_server: bool,
    pub label: Label,
    /// Keep it here instead of in the server's settings folder.
    pub folder: Option<String>,
}

impl Default for CreateOptions {
    fn default() -> Self {
        CreateOptions { passphrase: None, resources: ResourceChoice::Auto, stop_server: false, label: Label::Manual, folder: None }
    }
}

#[derive(Debug)]
pub struct CreateReport {
    pub info: BackupInfo,
    pub path: PathBuf,
    pub notes: Vec<String>,
    /// Backups removed by the retention rules.
    pub pruned: Vec<String>,
}

// ----------------------------------------------------------------------------- one at a time

static RUNNING: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Held while a backup or restore of a server runs: a second one is refused, not queued.
pub struct BusyGuard(String);

impl BusyGuard {
    pub fn take(server: &str) -> Result<BusyGuard, Problem> {
        let mut l = RUNNING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if l.iter().any(|s| s == server) {
            return Err(Problem::new(codes::BUSY, "A backup or restore of this server is already running. Wait for it to finish, then try again."));
        }
        l.push(server.to_string());
        Ok(BusyGuard(server.to_string()))
    }
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        RUNNING.lock().unwrap_or_else(std::sync::PoisonError::into_inner).retain(|s| s != &self.0);
    }
}

// ----------------------------------------------------------------------------- what is where

/// The database as Kora recorded it.
pub enum DbSource {
    /// No database Kora knows about (and why, in words).
    None(String),
    Local(Credentials),
    External(Credentials),
}

pub fn is_local_host(h: &str) -> bool {
    matches!(h.trim_matches(['[', ']']), "127.0.0.1" | "localhost" | "::1")
}

/// Values from `database.json` go to a command line (the database name) and are checked first: the
/// file sits in a folder the server's own account can write.
pub fn credentials_are_sane(c: &Credentials) -> bool {
    let word = |s: &str| !s.is_empty() && s.len() <= 80 && !s.starts_with('-') && s.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '$' | '-' | '.'));
    word(&c.database)
        && word(&c.user)
        && !c.host.starts_with('-')
        && c.host.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | ':' | '[' | ']' | '_'))
        && !c.host.is_empty()
        && c.port != 0
}

pub fn database_of(dir: &Path) -> Result<DbSource, Problem> {
    let p = dir.join("database.json");
    match std::fs::read(&p) {
        Ok(bytes) => {
            let c: Credentials = serde_json::from_slice(&bytes).map_err(|_| {
                Problem::new(codes::CORRUPT, "The database login file of this server (database.json) cannot be read, so its database cannot be backed up.").with_action(Action::CopyDetails)
            })?;
            if !credentials_are_sane(&c) {
                return Err(Problem::new(codes::CORRUPT, "The database login file of this server (database.json) has values Kora will not use, so its database cannot be backed up. Check the file."));
            }
            Ok(if is_local_host(&c.host) { DbSource::Local(c) } else { DbSource::External(c) })
        }
        Err(_) => {
            let own =
                ["secrets.cfg", "server.cfg"].iter().filter_map(|f| std::fs::read_to_string(dir.join(f)).ok()).any(|t| t.lines().any(|l| l.trim_start().starts_with("set mysql_connection_string")));
            Ok(DbSource::None(if own {
                "This server uses a database you set up yourself (Kora was not told to create one). Its data is not in this backup: back that database up separately.".into()
            } else {
                "This server has no database, so there was nothing to dump.".into()
            }))
        }
    }
}

fn db_problem(f: DbFailure, what: &str) -> Problem {
    match f {
        DbFailure::ToolMissing => {
            Problem::new(codes::DUMP_TOOL_MISSING, format!("Kora cannot {what}: the database tools (mariadb-dump / mysqldump) were not found. Installing the MariaDB client tools fixes it."))
                .with_action(Action::InstallTool { tool: Tool::Mariadb })
        }
        DbFailure::NotRunning => Problem::new(codes::DB_STOPPED, format!("Kora cannot {what}: the database server is not running.")).with_action(Action::StartDatabase).with_action(Action::Retry),
        DbFailure::Login => {
            Problem::new(codes::DB_LOGIN, format!("Kora cannot {what}: the database refused this server's login (database.json no longer fits the database). Fix the login, or recreate the database."))
                .with_action(Action::CopyDetails)
        }
        DbFailure::NoSuchDatabase => Problem::new(codes::DB_LOGIN, format!("Kora cannot {what}: the database named for this server does not exist on the database server.")),
        DbFailure::DiskFull => disk_full(&format!("Kora cannot {what}: there is no space left on the disk.")),
        DbFailure::Other(m) => Problem::new(codes::CORRUPT, format!("Kora cannot {what}: the database tool failed.")).with_details(m).with_action(Action::CopyDetails),
    }
}

pub fn problem_for_db(f: DbFailure, what: &str) -> Problem {
    db_problem(f, what)
}

pub fn disk_full(msg: &str) -> Problem {
    Problem::new(codes::DISK_FULL, msg.to_string()).with_action(Action::Retry)
}

fn dest_problem(e: DestError, d: &dyn Destination) -> Problem {
    match e {
        DestError::NotWritable(w) => Problem::new(
            codes::DEST_UNWRITABLE,
            format!("Kora cannot write to {}. Choose another folder in the backup settings, or fix its permissions (or reconnect the drive or share).", d.describe()),
        )
        .with_details(w),
        DestError::Full(w) => disk_full(&format!("There is no space left in {}. Free some space or choose another folder.", d.describe())).with_details(w),
        DestError::Io(w) => Problem::new(codes::DEST_UNWRITABLE, format!("Kora could not save the backup in {}.", d.describe())).with_details(w).with_action(Action::CopyDetails),
    }
}

// ----------------------------------------------------------------------------- walking

/// Left out of the server-folder copy: the FXServer build, cache, crash dumps, logs, the resources (handled
/// separately) and Kora's own `.kora` folder (only a few of its files are kept, see [`KORA_FILES`]).
fn skip_server(rel: &str, is_dir: bool) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    let top = !rel.contains('/');
    if is_dir {
        if top {
            matches!(name, "artifacts" | "resources" | ".kora" | ".git" | "cache" | "crashes" | "logs")
        } else {
            matches!(name, "logs" | "cache" | "crashes")
        }
    } else {
        name.ends_with(".log") || name.ends_with(".part")
    }
}

const KORA_FILES: &[&str] = &["alerts.json", "notify.json", "schedule.json", "db-provision.json", "created.json"];

/// Sum the sizes of the files a tree copy would take.
fn tree_bytes(root: &Path, skip: &dyn Fn(&str, bool) -> bool) -> u64 {
    fn walk(dir: &Path, rel: &str, skip: &dyn Fn(&str, bool) -> bool, total: &mut u64) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let Some(name) = e.file_name().to_str().map(str::to_string) else { continue };
            let child = if rel.is_empty() { name } else { format!("{rel}/{name}") };
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                if !skip(&child, true) {
                    walk(&e.path(), &child, skip, total);
                }
            } else if ft.is_file() && !skip(&child, false) {
                *total += e.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    let mut t = 0;
    walk(root, "", skip, &mut t);
    t
}

// ----------------------------------------------------------------------------- the work

/// Make a backup of `e`. `say` gets one event per step.
pub fn create(e: &ServerEntry, o: &CreateOptions, host: &mut dyn Host, tools: &Tools<'_>, say: &mut dyn FnMut(BackupEvent)) -> Result<CreateReport, Problem> {
    let _busy = BusyGuard::take(&e.name)?;
    if crate::update::lock::is_busy(&e.dir) {
        return Err(Problem::new(codes::BUSY, format!("{}. Try the backup again when it has finished.", crate::update::lock::BUSY)));
    }
    let started = tools.now;
    let result = create_unguarded(e, o, host, tools, say);
    match &result {
        Ok(r) => settings::update_status(&e.name, |s| {
            s.last_ok_unix = Some(r.info.created_unix);
            s.last_error = None;
            s.last_failed_unix = None;
        }),
        Err(p) => {
            let msg = p.message.clone();
            settings::update_status(&e.name, |s| {
                s.last_failed_unix = Some(started);
                s.last_error = Some(msg);
            });
        }
    }
    result
}

/// [`create`] without taking the one-at-a-time guard or recording the outcome: for a caller that holds the
/// guard itself (a restore taking its safety copy).
pub fn create_unguarded(e: &ServerEntry, o: &CreateOptions, host: &mut dyn Host, tools: &Tools<'_>, say: &mut dyn FnMut(BackupEvent)) -> Result<CreateReport, Problem> {
    let mut ev = |step: &str, msg: &str, pct: u8| say(BackupEvent { step: step.into(), message: msg.into(), percent: pct });
    if !e.dir.is_dir() {
        return Err(Problem::new(codes::NOT_FOUND, format!("The folder of {} is not there any more ({}), so there is nothing to back up.", e.name, e.dir.display())));
    }
    if let Some(p) = &o.passphrase {
        check_passphrase_text(p)?;
    }
    let cfg = settings::load_settings(&e.name);
    let root = o.folder.clone().unwrap_or(cfg.folder.clone());
    let destination = dest::LocalFolder::for_server(&root, &e.name);
    // Never inside the server folder: deleting the server would delete its backups.
    if destination.local_dir().is_some_and(|d| d.starts_with(&e.dir)) {
        return Err(Problem::new(
            codes::DEST_UNWRITABLE,
            "Backups cannot be kept inside the server's own folder: deleting the server would delete them too. Choose another folder in the backup settings.",
        ));
    }
    ev("prepare", &format!("Checking {}", destination.describe()), 2);
    destination.prepare().map_err(|x| dest_problem(x, &destination))?;

    // --- what is going in
    ev("inspect", "Looking at what to back up", 5);
    let server_file = ServerFile::load(&e.dir).ok();
    let dbsrc = database_of(&e.dir)?;
    let everything = o.resources == ResourceChoice::Everything;
    let resources_dir = e.dir.join("resources");
    let mut resources: Vec<ResourceBackup> = Vec::new();
    if let Some(sf) = &server_file {
        for rec in &sf.resources {
            if let Some(rb) = super::resources::classify(tools.git, &resources_dir, rec, everything) {
                resources.push(rb);
            }
        }
    }
    let referenced: HashSet<String> = resources.iter().filter(|r| r.stored == StoredAs::Reference).map(|r| r.path.clone()).collect();
    let skip_resources = |rel: &str, is_dir: bool| is_dir && referenced.contains(rel);
    let server_bytes = tree_bytes(&e.dir, &skip_server);
    let resource_bytes = if resources_dir.is_dir() { tree_bytes(&resources_dir, &skip_resources) } else { 0 };
    let db_bytes = match &dbsrc {
        DbSource::Local(c) | DbSource::External(c) => dbdump::data_bytes(tools.db, &tools.db_client, c).unwrap_or(0),
        DbSource::None(_) => 0,
    };
    let needed = dest::space_needed(server_bytes + resource_bytes + db_bytes);
    dest::check_space(destination.free_bytes(), needed, &destination.describe()).map_err(|m| disk_full(&m))?;

    // --- consistency
    let was_running = host.running();
    let stop = o.stop_server && was_running;
    let mut stopped_by_us = false;
    if stop {
        ev("stop", "Stopping the server for a strictly consistent copy", 8);
        host.stop().map_err(|m| Problem::generic(format!("The server could not be stopped for the backup: {m}")))?;
        stopped_by_us = true;
    }
    let outcome = build(e, o, tools, &destination, &mut ev, server_file.as_ref(), dbsrc, resources, &skip_resources, resource_bytes + server_bytes + db_bytes, stop || !was_running);
    if stopped_by_us {
        ev("start", "Starting the server again", 95);
        if let Err(m) = host.start() {
            // The backup itself may have worked; the person must know the server is down.
            return match outcome {
                Ok(mut r) => {
                    r.notes.push(format!("The server was stopped for the backup and could not be started again ({m}). Start it from the server page."));
                    Ok(r)
                }
                Err(p) => Err(p),
            };
        }
    }
    let mut report = outcome?;
    // --- retention
    ev("prune", "Removing old backups", 97);
    let status = settings::load_status(&e.name);
    let protect: Vec<String> = status.undo_id.into_iter().collect();
    for id in dest::plan_prune(&destination.list(), &cfg.retention, tools.now, &protect) {
        if id != report.info.id && destination.delete(&id).is_ok() {
            report.pruned.push(id);
        }
    }
    ev("done", "Backup finished", 100);
    Ok(report)
}

fn check_passphrase_text(p: &Passphrase) -> Result<(), Problem> {
    check_passphrase(p.reveal()).map_err(|m| Problem::new(codes::WEAK_PASSPHRASE, m))
}

#[allow(clippy::too_many_arguments)]
fn build(
    e: &ServerEntry,
    o: &CreateOptions,
    tools: &Tools<'_>,
    destination: &dyn Destination,
    ev: &mut dyn FnMut(&str, &str, u8),
    server_file: Option<&ServerFile>,
    dbsrc: DbSource,
    resources: Vec<ResourceBackup>,
    skip_resources: &dyn Fn(&str, bool) -> bool,
    payload_estimate: u64,
    strictly: bool,
) -> Result<CreateReport, Problem> {
    let created = tools.now;
    let id = unique_id(destination, created);
    let file = dest::archive_name(&e.name, &id, o.passphrase.is_some());
    let staged = destination.staging(&file);
    let dump_tmp = PathBuf::from(format!("{}.sql", staged.display()));
    let cleanup = Cleanup(vec![staged.clone(), dump_tmp.clone()]);
    let mut notes: Vec<String> = Vec::new();

    // --- the database
    let mut database = DatabaseBackup { kind: DbKind::None, name: None, host: None, port: None, dump_bytes: 0, note: String::new() };
    let mut have_dump = false;
    match &dbsrc {
        DbSource::None(why) => database.note = why.clone(),
        DbSource::Local(c) | DbSource::External(c) => {
            let external = matches!(dbsrc, DbSource::External(_));
            ev("database", "Copying the database", 15);
            let Some(prog) = tools.dump_program.as_deref() else {
                return Err(db_problem(DbFailure::ToolMissing, "back up the database"));
            };
            let size = dbdump::dump(tools.db, prog, c, &dump_tmp).map_err(|f| db_problem(f, "back up the database"))?;
            have_dump = true;
            database = DatabaseBackup {
                kind: if external { DbKind::External } else { DbKind::Dumped },
                name: Some(c.database.clone()),
                host: Some(c.host.clone()),
                port: Some(c.port),
                dump_bytes: size,
                note: if external {
                    format!("This server's database is on {} (another computer). It was copied over the network with the saved login.", c.host)
                } else {
                    "A consistent copy of the server's own database (one snapshot, routines, triggers and events included).".into()
                },
            };
        }
    }

    // --- the manifest header
    let sf = server_file;
    let info_server = ServerInfo {
        name: e.name.clone(),
        mode: format!("{:?}", e.mode).to_lowercase(),
        game_port: e.ports.game,
        txadmin_port: e.ports.txadmin,
        game_build: sf.map_or(0, |s| s.game_build),
        artifact_build: sf.map_or(0, |s| s.artifact_build),
        os: if cfg!(windows) { "windows".into() } else { "linux".into() },
        dir: e.dir.display().to_string(),
        github_owner: sf.and_then(|s| s.github_owner.clone()),
    };
    if !strictly {
        notes.push("The server was running while the files were copied. The database is one consistent snapshot; the other files were copied one by one, so two of them can be from slightly different moments.".into());
    }
    let mut manifest = Manifest {
        format: FORMAT,
        kora_version: env!("CARGO_PKG_VERSION").into(),
        created_unix: created,
        label: o.label,
        server: info_server,
        database,
        resources: resources.clone(),
        stopped: strictly,
        notes: notes.clone(),
        payload_bytes: payload_estimate,
        files: vec![],
    };

    // --- the archive
    ev("files", "Writing the backup file", 30);
    let mut w = ArchiveWriter::create(&staged, o.passphrase.as_ref()).map_err(|x| io_problem(x, destination))?;
    let wr = |r: std::io::Result<()>| r.map_err(|x| io_problem(x, destination));
    wr(w.add_header(&manifest))?;
    if have_dump {
        wr(w.add_file(DUMP, &dump_tmp))?;
        let _ = std::fs::remove_file(&dump_tmp);
    }
    wr(w.add_tree(SERVER_DIR, &e.dir, &skip_server))?;
    // Kora's own records for the server
    let entry_json = serde_json::to_vec_pretty(e).map_err(|x| Problem::generic(x.to_string()))?;
    wr(w.add_bytes(&format!("{KORA_DIR}/entry.json"), &entry_json))?;
    for f in KORA_FILES {
        let p = crate::installer::meta_dir(&e.dir).join(f);
        if p.is_file() {
            wr(w.add_file(&format!("{KORA_DIR}/{f}"), &p))?;
        }
    }
    ev("resources", "Adding resources that cannot be downloaded again", 55);
    let resources_dir = e.dir.join("resources");
    if resources_dir.is_dir() {
        wr(w.add_tree(RESOURCES_DIR, &resources_dir, skip_resources))?;
    }
    // sizes per full resource, from what was written
    notes.append(&mut w.notes.clone());
    manifest.notes = notes.clone();
    for rb in manifest.resources.iter_mut().filter(|r| r.stored == StoredAs::Full) {
        rb.bytes = tree_bytes(&resources_dir.join(&rb.path), &|_, _| false);
    }
    ev("seal", if o.passphrase.is_some() { "Locking the file with the password" } else { "Finishing the file" }, 85);
    let (sha, bytes) = w.finish(&manifest).map_err(|x| io_problem(x, destination))?;

    // --- hand over
    let info = BackupInfo {
        id: id.clone(),
        file: file.clone(),
        bytes,
        sha256: sha,
        encrypted: o.passphrase.is_some(),
        created_unix: created,
        label: o.label,
        kora_version: manifest.kora_version.clone(),
        server: e.name.clone(),
        database: manifest.database.kind,
        by_reference: manifest.by_reference() as u32,
        in_full: manifest.in_full() as u32,
        stopped: strictly,
        too_new: false,
    };
    ev("save", "Saving the backup", 90);
    destination.commit(&staged, &info).map_err(|x| dest_problem(x, destination))?;
    cleanup.disarm();
    let path = destination.open(&id).unwrap_or_else(|| staged.clone());
    Ok(CreateReport { info, path, notes, pruned: vec![] })
}

fn io_problem(x: std::io::Error, d: &dyn Destination) -> Problem {
    dest_problem(DestError::from(x), d)
}

/// An id that is not taken at this destination (two backups in the same second get `-2`).
fn unique_id(d: &dyn Destination, at: u64) -> String {
    let base = clock::stamp(at);
    let taken: HashSet<String> = d.list().into_iter().map(|b| b.id).collect();
    if !taken.contains(&base) {
        return base;
    }
    (2..).map(|n| format!("{base}-{n}")).find(|c| !taken.contains(c)).unwrap_or(base)
}

/// Removes half-made files when dropped, unless disarmed.
struct Cleanup(Vec<PathBuf>);

impl Cleanup {
    fn disarm(mut self) {
        self.0.clear();
    }
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        for p in &self.0 {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// The archive's pieces, for the tests and the restore preview: the manifest of an archive.
pub fn read_manifest(path: &Path, pass: Option<&Passphrase>) -> Result<Manifest, archive::ArchiveError> {
    archive::read_head(path, pass)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::backup::dbdump::fake::FakeDb;
    use crate::backup::resources::fake::FakeGit;
    use crate::backup::tests_support::*;

    #[test]
    fn a_backup_holds_the_database_the_config_and_what_cannot_be_fetched() {
        let rig = Rig::new();
        rig.db.tables.borrow_mut().push(("players".into(), vec!["alice".into()]));
        let git = rig.clean_git();
        let tools = rig.tools(&git);
        let mut host = FakeHost::new(true);
        let mut events = vec![];
        let r = create(&rig.entry, &CreateOptions::default(), &mut host, &tools, &mut |e| events.push(e)).unwrap();
        assert!(r.path.is_file() && r.info.bytes > 0);
        assert_eq!(r.info.database, DbKind::Dumped);
        assert!(events.iter().any(|e| e.step == "database") && events.last().unwrap().percent == 100);
        let m = archive::scan(&r.path, None, None, 1 << 30, None).unwrap();
        let names: Vec<&str> = m.files.iter().map(|f| f.path.as_str()).collect();
        for want in
            ["database/dump.sql", "server/server.cfg", "server/secrets.cfg", "server/database.json", "server/txData/default/config.json", "kora/entry.json", "resources/local-only/fxmanifest.lua"]
        {
            assert!(names.contains(&want), "{want} missing in {names:?}");
        }
        // by reference: the clean clone's files are NOT in the archive, its commit is in the manifest
        assert!(!names.iter().any(|n| n.starts_with("resources/core/")));
        let core = m.resources.iter().find(|r| r.id == "core").unwrap();
        assert_eq!((core.stored, core.commit.as_deref()), (StoredAs::Reference, Some("abcdef1234567890abcdef1234567890abcdef12")));
        // a resource with local changes is kept in full, with the reason
        let local = m.resources.iter().find(|r| r.id == "local-only").unwrap();
        assert_eq!(local.stored, StoredAs::Full);
        // the FXServer build and logs are not in it
        assert!(!names.iter().any(|n| n.contains("artifacts") || n.ends_with(".log") || n.contains("/logs/")));
        assert_eq!(m.server.artifact_build, 1234);
        // the running server was not stopped, and the manifest says the copy was not strictly consistent
        assert!(!host.stopped_ever && !m.stopped);
        assert!(m.notes.iter().any(|n| n.contains("running while")));
        // the status line knows
        assert!(crate::backup::settings::load_status(&rig.entry.name).last_ok_unix.is_some());
    }

    #[test]
    fn the_archive_is_private_and_the_password_is_not_in_it() {
        let rig = Rig::new();
        let git = rig.clean_git();
        let tools = rig.tools(&git);
        let r = create(&rig.entry, &CreateOptions::default(), &mut FakeHost::new(false), &tools, &mut |_| {}).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&r.path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        // no command line of the dump ever held the password
        assert!(rig.db.calls.borrow().iter().all(|c| !c.args.join(" ").contains("s3cr3t")));
        // nothing is left in the destination but the archive and its listing
        let leftovers: Vec<_> = std::fs::read_dir(r.path.parent().unwrap()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(leftovers.len(), 2, "{leftovers:?}");
    }

    #[test]
    fn a_password_protected_backup_needs_it_to_read() {
        let rig = Rig::new();
        let git = rig.clean_git();
        let tools = rig.tools(&git);
        let pass = Passphrase::new("a very good passphrase");
        let o = CreateOptions { passphrase: Some(pass.clone()), ..Default::default() };
        let r = create(&rig.entry, &o, &mut FakeHost::new(false), &tools, &mut |_| {}).unwrap();
        assert!(r.info.encrypted && r.info.file.ends_with(".tar.gz.age"));
        assert!(archive::read_head(&r.path, None).is_err());
        assert_eq!(archive::read_head(&r.path, Some(&pass)).unwrap().server.name, rig.entry.name);
        // a weak password is refused before anything is made
        let weak = CreateOptions { passphrase: Some(Passphrase::new("abc")), ..Default::default() };
        let e = create(&rig.entry, &weak, &mut FakeHost::new(false), &tools, &mut |_| {}).unwrap_err();
        assert_eq!(e.code, codes::WEAK_PASSPHRASE);
    }

    #[test]
    fn stopping_the_server_for_a_strict_copy_and_starting_it_again_even_on_failure() {
        let rig = Rig::new();
        let git = rig.clean_git();
        let tools = rig.tools(&git);
        let mut host = FakeHost::new(true);
        let r = create(&rig.entry, &CreateOptions { stop_server: true, ..Default::default() }, &mut host, &tools, &mut |_| {}).unwrap();
        assert!(host.stopped_ever && host.running_now, "stopped for the copy, started again after");
        assert!(r.info.stopped);
        // a failing dump still starts it again, leaves nothing behind and records the failure
        *rig.db.fail_with.borrow_mut() = Some("Can't connect to MySQL server (111)".into());
        let mut host = FakeHost::new(true);
        let e = create(&rig.entry, &CreateOptions { stop_server: true, ..Default::default() }, &mut host, &tools, &mut |_| {}).unwrap_err();
        assert_eq!(e.code, codes::DB_STOPPED);
        assert!(e.actions.contains(&Action::StartDatabase));
        assert!(host.running_now);
        let st = crate::backup::settings::load_status(&rig.entry.name);
        assert!(st.last_failed_unix.is_some() && st.last_error.unwrap().contains("not running"));
        let dir = dest::folder_for("", &rig.entry.name);
        assert_eq!(dest::LocalFolder::new(&dir).list().len(), 1, "only the first backup exists");
        assert!(!std::fs::read_dir(&dir).unwrap().flatten().any(|e| e.file_name().to_string_lossy().ends_with(".part") || e.file_name().to_string_lossy().ends_with(".sql")));
    }

    #[test]
    fn a_missing_dump_tool_says_what_to_install() {
        let rig = Rig::new();
        let git = rig.clean_git();
        let mut tools = rig.tools(&git);
        tools.dump_program = None;
        let e = create(&rig.entry, &CreateOptions::default(), &mut FakeHost::new(false), &tools, &mut |_| {}).unwrap_err();
        assert_eq!(e.code, codes::DUMP_TOOL_MISSING);
        assert!(e.actions.contains(&Action::InstallTool { tool: Tool::Mariadb }));
    }

    #[test]
    fn a_server_without_a_database_is_backed_up_and_says_so() {
        let rig = Rig::new();
        std::fs::remove_file(rig.entry.dir.join("database.json")).unwrap();
        let git = rig.clean_git();
        let tools = rig.tools(&git);
        let r = create(&rig.entry, &CreateOptions::default(), &mut FakeHost::new(false), &tools, &mut |_| {}).unwrap();
        assert_eq!(r.info.database, DbKind::None);
        let m = archive::read_head(&r.path, None).unwrap();
        assert!(m.database.note.contains("no database") || m.database.note.contains("set up yourself"));
        assert!(rig.db.calls.borrow().is_empty(), "no database tool was run");
        // a database the person set up themselves is named as such
        std::fs::write(rig.entry.dir.join("secrets.cfg"), "set mysql_connection_string \"mysql://u:p@db.example/x\"\n").unwrap();
        let r = create(&rig.entry, &CreateOptions::default(), &mut FakeHost::new(false), &tools, &mut |_| {}).unwrap();
        assert!(archive::read_head(&r.path, None).unwrap().database.note.contains("set up yourself"));
    }

    #[test]
    fn an_external_database_is_dumped_over_the_network_and_flagged() {
        let rig = Rig::new();
        let mut c: serde_json::Value = serde_json::from_slice(&std::fs::read(rig.entry.dir.join("database.json")).unwrap()).unwrap();
        c["host"] = "db.example.com".into();
        std::fs::write(rig.entry.dir.join("database.json"), c.to_string()).unwrap();
        let git = rig.clean_git();
        let tools = rig.tools(&git);
        let r = create(&rig.entry, &CreateOptions::default(), &mut FakeHost::new(false), &tools, &mut |_| {}).unwrap();
        assert_eq!(r.info.database, DbKind::External);
        assert!(rig.db.calls.borrow().iter().any(|c| c.args.contains(&"db.example.com".to_string())));
    }

    #[test]
    fn a_tampered_database_login_is_not_passed_to_a_command() {
        let rig = Rig::new();
        let mut c: serde_json::Value = serde_json::from_slice(&std::fs::read(rig.entry.dir.join("database.json")).unwrap()).unwrap();
        c["database"] = "--all-databases".into();
        std::fs::write(rig.entry.dir.join("database.json"), c.to_string()).unwrap();
        let git = rig.clean_git();
        let tools = rig.tools(&git);
        let e = create(&rig.entry, &CreateOptions::default(), &mut FakeHost::new(false), &tools, &mut |_| {}).unwrap_err();
        assert!(e.message.contains("values Kora will not use"));
        assert!(rig.db.calls.borrow().is_empty());
    }

    #[test]
    fn the_destination_must_be_writable_and_outside_the_server() {
        let rig = Rig::new();
        let git = rig.clean_git();
        let tools = rig.tools(&git);
        let inside = CreateOptions { folder: Some(rig.entry.dir.join("backups").display().to_string()), ..Default::default() };
        let e = create(&rig.entry, &inside, &mut FakeHost::new(false), &tools, &mut |_| {}).unwrap_err();
        assert!(e.message.contains("inside the server"));
        let blocker = rig.entry.dir.parent().unwrap().join("a-file");
        std::fs::write(&blocker, b"x").unwrap();
        let bad = CreateOptions { folder: Some(blocker.join("sub").display().to_string()), ..Default::default() };
        assert_eq!(create(&rig.entry, &bad, &mut FakeHost::new(false), &tools, &mut |_| {}).unwrap_err().code, codes::DEST_UNWRITABLE);
    }

    #[test]
    fn two_at_once_are_refused_and_old_ones_are_pruned() {
        let rig = Rig::new();
        let git = rig.clean_git();
        let tools = rig.tools(&git);
        let g = BusyGuard::take(&rig.entry.name).unwrap();
        assert_eq!(create(&rig.entry, &CreateOptions::default(), &mut FakeHost::new(false), &tools, &mut |_| {}).unwrap_err().code, codes::BUSY);
        drop(g);
        let mut s = BackupSettings::default();
        s.retention = Retention { keep_last: 2, keep_daily_days: 0 };
        crate::backup::settings::save_settings(&rig.entry.name, &s).unwrap();
        for i in 0..4u64 {
            let mut t = rig.tools(&git);
            t.now = 1_700_000_000 + i * 100;
            create(&rig.entry, &CreateOptions::default(), &mut FakeHost::new(false), &t, &mut |_| {}).unwrap();
        }
        let left = dest::LocalFolder::for_server("", &rig.entry.name).list();
        assert_eq!(left.len(), 2);
        assert_eq!(left[0].created_unix, 1_700_000_300);
    }

    #[allow(dead_code)]
    fn _types(_: &FakeDb, _: &FakeGit) {}
}
