//! Putting a backup back on the SAME server.
//!
//! Order, chosen so a failure leaves as little changed as possible:
//! 1. read the start of the archive (right Kora version? right password? this server? same operating system?);
//! 2. unpack it into a staging folder inside the server's `.kora` folder, checking every file against the
//!    checksums in the archive: nothing in the server is touched until the whole archive has proved good;
//! 3. take a SAFETY BACKUP of the server as it is now (so the restore can be undone); if that fails, stop here;
//! 4. stop the server (it is started again at the end if it was running);
//! 5. load the database (the riskiest step, so before any file changes: a failure here leaves the files alone);
//! 6. put the files back: the same names are overwritten in place (keeping their owner and permissions);
//!    `secrets.cfg` and `database.json` of the running server are KEPT (they hold today's licence key and
//!    database login), and only restored when they are missing;
//! 7. bring the resources back: the ones kept by reference are fetched at their recorded commit, the ones
//!    kept in full replace the folder;
//! 8. start the server again.
//!
//! A step that fails on one resource does not stop the restore: it is listed in the report, with the safe
//! outcome. "Undo this restore" is a restore of the safety backup.

use std::path::{Path, PathBuf};

use super::archive::{self, ArchiveError, Wanted, DUMP, KORA_DIR, RESOURCES_DIR, SERVER_DIR};
use super::crypt::{OpenError, Passphrase};
use super::dbdump::{self, DbFailure};
use super::dest::{self, Destination};
use super::types::*;
use super::{codes, create, settings, BackupEvent, Host, Tools};
use crate::installer::registry::{now_unix, ServerEntry};
use crate::problem::{Action, Problem};

#[derive(Default)]
pub struct RestoreOptions {
    pub passphrase: Option<Passphrase>,
    /// Take a safety backup first (always, except when undoing).
    pub no_safety_backup: bool,
}

/// What a restore would do, in words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RestorePreview {
    pub id: String,
    pub created_unix: u64,
    pub encrypted: bool,
    /// The archive is password protected and no (right) password was given: nothing else is known yet.
    pub needs_passphrase: bool,
    pub server: Option<ServerInfo>,
    pub database: Option<DatabaseBackup>,
    pub resources: Vec<ResourceBackup>,
    pub stopped: bool,
    pub server_running: bool,
    /// What will be replaced.
    pub will_change: Vec<String>,
    /// What stays as it is.
    pub will_keep: Vec<String>,
    pub warnings: Vec<String>,
}

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RestoreReport {
    /// The safety backup taken first (what "Undo" goes back to).
    pub safety_backup: Option<String>,
    pub database_restored: bool,
    pub files_restored: u32,
    /// One sentence per resource that was brought back.
    pub resources_restored: Vec<String>,
    /// Things that did not work, one sentence each, with what was done instead.
    pub problems: Vec<String>,
    pub notes: Vec<String>,
    pub restarted: bool,
}

/// Map a failure to open or read an archive to something a person can act on.
pub fn archive_problem(e: ArchiveError) -> Problem {
    match e {
        ArchiveError::Open(OpenError::NeedsPassphrase) => Problem::new(codes::NEEDS_PASSPHRASE, "This backup is password protected. Enter the password to open it."),
        ArchiveError::Open(OpenError::WrongPassphrase) => {
            Problem::new(codes::WRONG_PASSPHRASE, "That password does not open this backup. Check it and try again. (Kora does not keep passwords, so a lost one cannot be recovered.)")
        }
        ArchiveError::Open(OpenError::NotAnArchive) => Problem::new(codes::CORRUPT, "This file is not a Kora backup, or it is damaged."),
        ArchiveError::Open(OpenError::Io(m)) | ArchiveError::Corrupt(m) => Problem::new(codes::CORRUPT, format!("This backup cannot be used: {m}.")).with_action(Action::CopyDetails),
        ArchiveError::TooNew(v) => {
            Problem::new(codes::TOO_NEW, format!("This backup was made by a newer version of Kora (backup format {v}), which this version cannot read safely. Update Kora, then try again."))
        }
        ArchiveError::Io(x) => match x.kind() {
            std::io::ErrorKind::StorageFull => create::disk_full("There is no space left on the disk to unpack this backup."),
            _ => Problem::new(codes::CORRUPT, "The backup file could not be read.").with_details(x.to_string()).with_action(Action::CopyDetails),
        },
    }
}

fn same_os(m: &Manifest) -> bool {
    m.server.os == if cfg!(windows) { "windows" } else { "linux" }
}

fn check_this_server(e: &ServerEntry, m: &Manifest) -> Result<(), Problem> {
    if m.server.name != e.name {
        return Err(Problem::new(
            codes::CORRUPT,
            format!("This backup is of the server \"{}\", not \"{}\". To use it, restore it as a new server (\"Bring a server from a backup\" in the install wizard).", m.server.name, e.name),
        ));
    }
    if !same_os(m) {
        return Err(Problem::new(
            codes::CORRUPT,
            format!(
                "This backup was made on {}, and this computer is different. Restore it as a new server instead: that converts the paths for this computer.",
                if m.server.os == "windows" { "Windows" } else { "Linux" }
            ),
        ));
    }
    Ok(())
}

/// What restoring `path` would do (reads only the start of the archive).
pub fn preview(e: &ServerEntry, info: &BackupInfo, path: &Path, pass: Option<&Passphrase>, server_running: bool) -> Result<RestorePreview, Problem> {
    let mut p = RestorePreview {
        id: info.id.clone(),
        created_unix: info.created_unix,
        encrypted: info.encrypted,
        needs_passphrase: false,
        server: None,
        database: None,
        resources: vec![],
        stopped: info.stopped,
        server_running,
        will_change: vec![],
        will_keep: vec![],
        warnings: vec![],
    };
    let m = match archive::read_head(path, pass) {
        Ok(m) => m,
        Err(ArchiveError::Open(OpenError::NeedsPassphrase)) => {
            p.needs_passphrase = true;
            return Ok(p);
        }
        Err(other) => return Err(archive_problem(other)),
    };
    check_this_server(e, &m)?;
    p.will_change.push("The server's settings: server.cfg, permissions and the other .cfg files".into());
    p.will_change.push("txAdmin's data (its settings, admins and player records)".into());
    match m.database.kind {
        DbKind::Dumped | DbKind::External => p.will_change.push(format!(
            "The database \"{}\": every table is emptied and replaced by the ones in the backup (anything saved since {} is lost, unless you undo)",
            m.database.name.clone().unwrap_or_default(),
            crate::backup::clock::utc_text(m.created_unix)
        )),
        DbKind::None => p.warnings.push(if m.database.note.is_empty() { "This backup has no database.".into() } else { m.database.note.clone() }),
        DbKind::Failed => p.warnings.push("The database could not be copied when this backup was made, so it is not in it.".into()),
    }
    let (r, f) = (m.by_reference(), m.in_full());
    if r > 0 {
        p.will_change.push(format!("{r} resource{} will be fetched again from {}, at the exact version saved (this needs internet)", if r == 1 { "" } else { "s" }, "GitHub"));
    }
    if f > 0 {
        p.will_change.push(format!("{f} resource{} will be replaced by the copy in the backup", if f == 1 { "" } else { "s" }));
    }
    p.will_keep.push("secrets.cfg and database.json stay as they are now (they hold today's licence key and database login)".into());
    p.will_keep.push("The FXServer program itself (it is not in backups)".into());
    p.will_keep.push("Logs".into());
    if server_running {
        p.warnings.push("The server is running. Kora will stop it for the restore and start it again afterwards.".into());
    }
    if m.server.game_port != e.ports.game || m.server.txadmin_port != e.ports.txadmin {
        p.warnings.push(format!(
            "The backup used ports {} and {}, this server now uses {} and {}. The backup's ports come back with its settings.",
            m.server.game_port, m.server.txadmin_port, e.ports.game, e.ports.txadmin
        ));
    }
    p.warnings.push("A safety copy of the server as it is now is made first, so you can undo this.".into());
    p.server = Some(m.server);
    p.database = Some(m.database);
    p.resources = m.resources;
    p.notes_into_warnings(m.notes);
    Ok(p)
}

impl RestorePreview {
    fn notes_into_warnings(&mut self, notes: Vec<String>) {
        self.warnings.extend(notes);
    }
}

/// Check a whole archive: every file against its checksum (and the file against the listing's). A job.
pub fn verify(path: &Path, info: Option<&BackupInfo>, pass: Option<&Passphrase>, say: &mut dyn FnMut(BackupEvent)) -> Result<Manifest, Problem> {
    say(BackupEvent { step: "verify".into(), message: "Checking the file".into(), percent: 5 });
    if let Some(i) = info {
        let sha = archive::file_sha256(path).map_err(|x| archive_problem(ArchiveError::Io(x)))?;
        if sha != i.sha256 {
            return Err(Problem::new(codes::CORRUPT, "The backup file has changed since it was made (its checksum no longer matches), so it may be damaged. It was not used."));
        }
    }
    say(BackupEvent { step: "verify".into(), message: "Opening it and checking every file".into(), percent: 30 });
    let total = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0).saturating_mul(40).max(1 << 20);
    archive::scan(path, pass, None, total, None).map_err(archive_problem)
}

struct Staging(PathBuf);

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Overwrite `dest` with `src`, keeping an existing file's owner and permissions; a new file is made
/// readable by the server's account (0644, or 0600 for the files that hold secrets).
pub fn write_into(dest: &Path, src: &Path, secret: bool) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if crate::fsutil::is_symlink(dest) {
        return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, format!("{} is a link; Kora does not write through links", dest.display())));
    }
    let existed = dest.exists();
    let mut out = std::fs::OpenOptions::new().write(true).create(true).truncate(true).open(dest)?;
    let mut input = std::fs::File::open(src)?;
    std::io::copy(&mut input, &mut out)?;
    out.flush()?;
    drop(out);
    if !existed {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(dest, std::fs::Permissions::from_mode(if secret { 0o600 } else { 0o644 }));
        }
        #[cfg(not(unix))]
        let _ = secret;
        crate::store::privacy::hand_to_owner_of_parent(dest);
    }
    Ok(())
}

fn is_secret_name(rel: &str) -> bool {
    matches!(rel, "secrets.cfg" | "database.json") || rel.ends_with("/secrets.cfg")
}

/// Every file below `root` (relative paths with `/`).
fn files_below(root: &Path) -> Vec<String> {
    fn walk(dir: &Path, rel: &str, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let Some(n) = e.file_name().to_str().map(str::to_string) else { continue };
            let child = if rel.is_empty() { n } else { format!("{rel}/{n}") };
            match e.file_type() {
                Ok(t) if t.is_dir() => walk(&e.path(), &child, out),
                Ok(t) if t.is_file() => out.push(child),
                _ => {}
            }
        }
    }
    let mut v = Vec::new();
    walk(root, "", &mut v);
    v.sort();
    v
}

/// Restore `archive` onto `e`.
pub fn restore(e: &ServerEntry, archive_path: &Path, o: &RestoreOptions, host: &mut dyn Host, tools: &Tools<'_>, say: &mut dyn FnMut(BackupEvent)) -> Result<RestoreReport, Problem> {
    let _busy = create::BusyGuard::take(&e.name)?;
    if crate::update::lock::is_busy(&e.dir) {
        return Err(Problem::new(codes::BUSY, format!("{}. Try the restore again when it has finished.", crate::update::lock::BUSY)));
    }
    let mut ev = |step: &str, msg: &str, pct: u8| say(BackupEvent { step: step.into(), message: msg.into(), percent: pct });
    ev("open", "Opening the backup", 2);
    let head = archive::read_head(archive_path, o.passphrase.as_ref()).map_err(archive_problem)?;
    check_this_server(e, &head)?;

    // 2. unpack and prove it
    let stage = Staging(e.dir.join(".kora").join(format!("restore-{}", crate::backup::clock::stamp(tools.now))));
    let _ = std::fs::remove_dir_all(&stage.0);
    std::fs::create_dir_all(&stage.0).map_err(|x| archive_problem(ArchiveError::Io(x)))?;
    let free = crate::space::SystemDisk.free_bytes_of(&e.dir);
    if let Some(f) = free {
        if f < head.payload_bytes.saturating_add(128 * crate::space::MIB) {
            return Err(create::disk_full(&format!(
                "Unpacking this backup needs about {} of room next to the server, and only {} is free.",
                crate::space::human(head.payload_bytes),
                crate::space::human(f)
            )));
        }
    }
    ev("unpack", "Unpacking and checking every file (nothing is changed yet)", 10);
    let manifest =
        archive::scan(archive_path, o.passphrase.as_ref(), Some((&stage.0, &Wanted { take: &|_| true })), head.payload_bytes.saturating_mul(2).saturating_add(256 * crate::space::MIB), None)
            .map_err(archive_problem)?;

    // 3. safety backup
    let mut safety: Option<String> = None;
    if !o.no_safety_backup {
        ev("safety", "Making a safety copy of the server as it is now", 30);
        let opts = create::CreateOptions { passphrase: o.passphrase.clone(), resources: ResourceChoice::Auto, stop_server: false, label: Label::BeforeRestore, folder: None };
        let mut quiet = |_: BackupEvent| {};
        match create::create_unguarded(e, &opts, host, tools, &mut quiet) {
            Ok(r) => safety = Some(r.info.id),
            Err(p) => {
                return Err(Problem::new(
                    p.code.as_str(),
                    format!("The restore was not started, because the safety copy of the server as it is now could not be made: {} Nothing was changed.", p.message),
                )
                .with_actions(p.actions));
            }
        }
    }

    // 4. stop
    let was_running = host.running();
    if was_running {
        ev("stop", "Stopping the server", 45);
        host.stop().map_err(|m| Problem::generic(format!("The server could not be stopped for the restore: {m}. Nothing was changed.")))?;
    }
    let mut report = RestoreReport { safety_backup: safety.clone(), database_restored: false, files_restored: 0, resources_restored: vec![], problems: vec![], notes: vec![], restarted: false };
    let result = apply(e, &manifest, &stage.0, tools, &mut ev, &mut report);
    // 8. start again, whatever happened
    if was_running {
        ev("start", "Starting the server again", 95);
        match host.start() {
            Ok(()) => report.restarted = true,
            Err(m) => report.problems.push(format!("The server was stopped for the restore and did not start again ({m}). Start it from the server page.")),
        }
    }
    if let Err(p) = result {
        return Err(Problem::new(
            p.code.as_str(),
            format!("{}{}", p.message, safety.as_ref().map(|_| " A safety copy of the server from before is saved: \"Undo this restore\" puts it back.").unwrap_or_default()),
        )
        .with_actions(p.actions));
    }
    settings::update_status(&e.name, |s| {
        s.undo_id = safety.clone();
    });
    ev("done", "Restore finished", 100);
    Ok(report)
}

fn apply(e: &ServerEntry, m: &Manifest, stage: &Path, tools: &Tools<'_>, ev: &mut dyn FnMut(&str, &str, u8), report: &mut RestoreReport) -> Result<(), Problem> {
    // 5. database
    let dump = stage.join(DUMP);
    if matches!(m.database.kind, DbKind::Dumped | DbKind::External) && dump.is_file() {
        ev("database", "Putting the database back", 55);
        let creds = match create::database_of(&e.dir)? {
            create::DbSource::Local(c) | create::DbSource::External(c) => c,
            create::DbSource::None(_) => match create::database_of(&stage.join(SERVER_DIR)) {
                Ok(create::DbSource::Local(c) | create::DbSource::External(c)) => c,
                _ => return Err(Problem::new(codes::DB_LOGIN, "This server has no database login on record, and the backup has none either, so the database cannot be put back.")),
            },
        };
        dbdump::check_login(tools.db, &tools.db_client, &creds).map_err(|f| create::problem_for_db(f, "put the database back"))?;
        dbdump::restore(tools.db, &tools.db_client, &creds, &dump).map_err(|f| match f {
            DbFailure::Other(_) | DbFailure::Login | DbFailure::NoSuchDatabase | DbFailure::NotRunning | DbFailure::ToolMissing | DbFailure::DiskFull => {
                create::problem_for_db(f, "put the database back")
            }
        })?;
        report.database_restored = true;
    }

    // 6. files
    ev("files", "Putting the configuration back", 70);
    let server_stage = stage.join(SERVER_DIR);
    for rel in files_below(&server_stage) {
        let Some(dest) = crate::fsutil::join_inside(&e.dir, &rel) else {
            report.problems.push(format!("{rel} has a name Kora will not write, so it was skipped."));
            continue;
        };
        // never the build, the resources folder or Kora's own folder, whatever an archive says
        let top = rel.split('/').next().unwrap_or("");
        if matches!(top, "artifacts" | "resources" | ".kora" | ".git") {
            continue;
        }
        if is_secret_name(&rel) && dest.exists() {
            report.notes.push(format!("{rel} was kept as it is now (it holds today's licence key and database login)."));
            continue;
        }
        match write_into(&dest, &server_stage.join(&rel), is_secret_name(&rel)) {
            Ok(()) => report.files_restored += 1,
            Err(x) => report.problems.push(format!("{rel} could not be put back ({x})")),
        }
    }
    // Kora's own small records, only where they are missing
    for f in ["alerts.json", "notify.json"] {
        let src = stage.join(KORA_DIR).join(f);
        let dest = crate::installer::meta_dir(&e.dir).join(f);
        if src.is_file() && !dest.exists() {
            let _ = write_into(&dest, &src, f == "notify.json");
        }
    }
    // the server record must keep pointing at this folder, and the registry at the backup's ports
    if let Ok(mut sf) = crate::installer::ServerFile::load(&e.dir) {
        if sf.dir != e.dir {
            sf.dir = e.dir.clone();
            let _ = sf.save();
        }
    }
    if m.server.game_port != e.ports.game || m.server.txadmin_port != e.ports.txadmin {
        let (g, t, name) = (m.server.game_port, m.server.txadmin_port, e.name.clone());
        let _ = crate::installer::registry::Registry::update(|r| {
            if let Some(s) = r.servers.iter_mut().find(|s| s.name == name) {
                s.ports = crate::installer::registry::PortsRecord { game: g, txadmin: t };
            }
            Ok(())
        });
        report.notes.push(format!("The ports are back to {g} (game) and {t} (txAdmin), as in the backup."));
    }

    // 7. resources
    ev("resources", "Bringing the resources back", 80);
    let resources_dir = e.dir.join(RESOURCES_DIR);
    let _ = std::fs::create_dir_all(&resources_dir);
    for rb in m.resources.iter().filter(|r| r.stored == StoredAs::Full) {
        let staged = stage.join(RESOURCES_DIR).join(&rb.path);
        let Some(dest) = crate::fsutil::join_inside(&resources_dir, &rb.path) else { continue };
        if !staged.is_dir() {
            report.problems.push(format!("{} should be in the backup in full but is not, so it was left as it is.", rb.id));
            continue;
        }
        if dest.is_dir() {
            let _ = std::fs::remove_dir_all(&dest);
        }
        let mut n = 0;
        for rel in files_below(&staged) {
            match write_into(&dest.join(&rel), &staged.join(&rel), false) {
                Ok(()) => n += 1,
                Err(x) => report.problems.push(format!("{}/{rel} could not be put back ({x})", rb.id)),
            }
        }
        report.resources_restored.push(format!("{} was put back from the backup ({n} files)", rb.id));
    }
    // folders in resources/ that no record named (kept in full by the archive too)
    let named: Vec<&str> = m.resources.iter().map(|r| r.path.as_str()).collect();
    for rel in files_below(&stage.join(RESOURCES_DIR)) {
        if named.iter().any(|p| rel == *p || rel.starts_with(&format!("{p}/"))) {
            continue;
        }
        if let Some(dest) = crate::fsutil::join_inside(&resources_dir, &rel) {
            if write_into(&dest, &stage.join(RESOURCES_DIR).join(&rel), false).is_ok() {
                report.files_restored += 1;
            }
        }
    }
    let refs: Vec<&ResourceBackup> = m.resources.iter().filter(|r| r.stored == StoredAs::Reference).collect();
    for (i, rb) in refs.iter().enumerate() {
        ev("resources", &format!("Fetching {} ({}/{})", rb.id, i + 1, refs.len()), (80 + (i * 14 / refs.len().max(1))) as u8);
        match super::resources::ensure_at_commit(tools.git, &resources_dir, rb) {
            Ok(msg) => report.resources_restored.push(msg),
            Err(msg) => report.problems.push(format!("{msg}. It was left as it is.")),
        }
    }
    let _ = now_unix();
    Ok(())
}

impl crate::space::SystemDisk {
    /// Bytes free where `p` is, when it can be told.
    pub fn free_bytes_of(&self, p: &Path) -> Option<u64> {
        crate::space::DiskInfo::free_bytes(self, p).ok()
    }
}

/// "Undo this restore": restore the safety backup taken first. `passphrase` is needed when that copy was
/// protected.
pub fn undo(e: &ServerEntry, destination: &dyn Destination, pass: Option<&Passphrase>, host: &mut dyn Host, tools: &Tools<'_>, say: &mut dyn FnMut(BackupEvent)) -> Result<RestoreReport, Problem> {
    let status = settings::load_status(&e.name);
    let Some(id) = status.undo_id else {
        return Err(Problem::new(codes::NO_UNDO, "There is no restore to undo for this server."));
    };
    let Some(path) = destination.open(&id) else {
        return Err(Problem::new(codes::NO_UNDO, "The safety copy that went with the last restore is gone, so it cannot be undone."));
    };
    let r = restore(e, &path, &RestoreOptions { passphrase: pass.cloned(), no_safety_backup: true }, host, tools, say)?;
    settings::update_status(&e.name, |s| s.undo_id = None);
    Ok(r)
}

/// The destination holding a server's backups under its settings.
pub fn destination_for(server: &str) -> dest::LocalFolder {
    dest::LocalFolder::for_server(&settings::load_settings(server).folder, server)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::tests_support::*;

    fn make(rig: &Rig, git: &crate::backup::resources::fake::FakeGit, pass: Option<&str>) -> (BackupInfo, PathBuf) {
        let tools = rig.tools(git);
        let o = create::CreateOptions { passphrase: pass.map(Passphrase::new), ..Default::default() };
        let r = create::create(&rig.entry, &o, &mut FakeHost::new(false), &tools, &mut |_| {}).unwrap();
        (r.info, r.path)
    }

    #[test]
    fn restore_brings_back_the_database_the_files_and_the_resources() {
        let rig = Rig::new();
        rig.db.tables.borrow_mut().push(("players".into(), vec!["alice".into(), "bob".into()]));
        let git = rig.clean_git();
        let (_, path) = make(&rig, &git, None);
        // damage: change a config, wipe the table, edit a kept-in-full resource, change the licence key
        write(&rig.entry.dir.join("server.cfg"), "# broken\n");
        write(&rig.entry.dir.join("txData/default/config.json"), "{}");
        write(&rig.entry.dir.join("resources/local-only/fxmanifest.lua"), "-- ruined\n");
        write(&rig.entry.dir.join("secrets.cfg"), "sv_licenseKey \"cfxk_NEWKEY\"\n");
        rig.db.tables.borrow_mut()[0].1.clear();
        // the clean resource is now on another commit: restore must fetch the recorded one
        git.say("cat-file -e abcdef1234567890abcdef1234567890abcdef12^{commit}", "");
        git.say("checkout -f -B main abcdef1234567890abcdef1234567890abcdef12", "");
        let mut tools = rig.tools(&git);
        tools.now += 1000;
        let mut host = FakeHost::new(true);
        let mut steps = vec![];
        let r = restore(&rig.entry, &path, &RestoreOptions::default(), &mut host, &tools, &mut |e| steps.push(e.step)).unwrap();
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        assert!(r.database_restored && r.restarted && host.running_now);
        assert_eq!(rig.db.tables.borrow()[0].1, vec!["alice".to_string(), "bob".to_string()], "the row is back");
        assert!(std::fs::read_to_string(rig.entry.dir.join("server.cfg")).unwrap().contains("endpoint_add_tcp"));
        assert!(std::fs::read_to_string(rig.entry.dir.join("txData/default/config.json")).unwrap().contains("serverName"));
        assert!(std::fs::read_to_string(rig.entry.dir.join("resources/local-only/fxmanifest.lua")).unwrap().contains("changed by me"));
        // today's licence key was kept, and the report says so
        assert!(std::fs::read_to_string(rig.entry.dir.join("secrets.cfg")).unwrap().contains("NEWKEY"));
        assert!(r.notes.iter().any(|n| n.contains("secrets.cfg was kept")));
        assert!(r.resources_restored.iter().any(|m| m.contains("core is back at version abcdef12")));
        // a safety copy of the damaged state exists, and the staging folder is gone
        let safety = r.safety_backup.clone().unwrap();
        assert!(destination_for(&rig.entry.name).open(&safety).is_some());
        assert!(!std::fs::read_dir(rig.entry.dir.join(".kora")).unwrap().flatten().any(|e| e.file_name().to_string_lossy().starts_with("restore-")));
        assert!(steps.iter().position(|s| s == "safety").unwrap() < steps.iter().position(|s| s == "database").unwrap());
        // ...and "undo" puts the damaged state back
        rig.db.tables.borrow_mut()[0].1.push("carol".into());
        let u = undo(&rig.entry, &destination_for(&rig.entry.name), None, &mut FakeHost::new(false), &tools, &mut |_| {}).unwrap();
        assert!(u.safety_backup.is_none());
        assert!(std::fs::read_to_string(rig.entry.dir.join("server.cfg")).unwrap().contains("# broken"));
        assert!(settings::load_status(&rig.entry.name).undo_id.is_none());
        assert_eq!(undo(&rig.entry, &destination_for(&rig.entry.name), None, &mut FakeHost::new(false), &tools, &mut |_| {}).unwrap_err().code, codes::NO_UNDO);
    }

    #[test]
    fn a_wrong_password_a_foreign_server_and_a_cut_file_change_nothing() {
        let rig = Rig::new();
        let git = rig.clean_git();
        let (info, path) = make(&rig, &git, Some("a good passphrase"));
        let tools = rig.tools(&git);
        write(&rig.entry.dir.join("server.cfg"), "# mine\n");
        let none = restore(&rig.entry, &path, &RestoreOptions::default(), &mut FakeHost::new(false), &tools, &mut |_| {}).unwrap_err();
        assert_eq!(none.code, codes::NEEDS_PASSPHRASE);
        let wrong =
            restore(&rig.entry, &path, &RestoreOptions { passphrase: Some(Passphrase::new("not the one!!")), ..Default::default() }, &mut FakeHost::new(false), &tools, &mut |_| {}).unwrap_err();
        assert_eq!(wrong.code, codes::WRONG_PASSPHRASE);
        let mut other = rig.entry.clone();
        other.name = "Another".into();
        let foreign =
            restore(&other, &path, &RestoreOptions { passphrase: Some(Passphrase::new("a good passphrase")), ..Default::default() }, &mut FakeHost::new(false), &tools, &mut |_| {}).unwrap_err();
        assert!(foreign.message.contains("restore it as a new server"));
        let cut = path.with_file_name("cut.tar.gz.age");
        let raw = std::fs::read(&path).unwrap();
        std::fs::write(&cut, &raw[..raw.len() - 200]).unwrap();
        assert!(restore(&rig.entry, &cut, &RestoreOptions { passphrase: Some(Passphrase::new("a good passphrase")), ..Default::default() }, &mut FakeHost::new(false), &tools, &mut |_| {}).is_err());
        assert_eq!(std::fs::read_to_string(rig.entry.dir.join("server.cfg")).unwrap(), "# mine\n", "nothing was touched");
        // the listing's checksum catches a changed file without the password
        let mut bad = info.clone();
        bad.sha256 = "0".repeat(64);
        assert_eq!(verify(&path, Some(&bad), None, &mut |_| {}).unwrap_err().code, codes::CORRUPT);
        assert!(verify(&path, Some(&info), Some(&Passphrase::new("a good passphrase")), &mut |_| {}).is_ok());
    }

    #[test]
    fn a_failed_safety_copy_stops_before_anything_changes() {
        let rig = Rig::new();
        let git = rig.clean_git();
        let (_, path) = make(&rig, &git, None);
        write(&rig.entry.dir.join("server.cfg"), "# mine\n");
        *rig.db.fail_with.borrow_mut() = Some("Can't connect to MySQL server (111)".into());
        let e = restore(&rig.entry, &path, &RestoreOptions::default(), &mut FakeHost::new(true), &rig.tools(&git), &mut |_| {}).unwrap_err();
        assert!(e.message.contains("safety copy") && e.message.contains("Nothing was changed"));
        assert_eq!(std::fs::read_to_string(rig.entry.dir.join("server.cfg")).unwrap(), "# mine\n");
    }

    #[test]
    fn a_resource_that_cannot_be_fetched_is_reported_and_the_restore_goes_on() {
        let rig = Rig::new();
        let git = rig.clean_git();
        let (_, path) = make(&rig, &git, None);
        git.fail("cat-file -e abcdef1234567890abcdef1234567890abcdef12^{commit}", "missing");
        git.fail("fetch origin", "could not resolve host: github.com");
        let r = restore(&rig.entry, &path, &RestoreOptions::default(), &mut FakeHost::new(false), &rig.tools(&git), &mut |_| {}).unwrap();
        assert!(r.problems.iter().any(|p| p.contains("core") && p.contains("could not fetch")));
        assert!(r.database_restored);
    }

    #[test]
    fn the_preview_says_what_changes_and_what_stays() {
        let rig = Rig::new();
        let git = rig.clean_git();
        let (info, path) = make(&rig, &git, None);
        let p = preview(&rig.entry, &info, &path, None, true).unwrap();
        assert!(p.will_change.iter().any(|w| w.contains("database")));
        assert!(p.will_change.iter().any(|w| w.contains("fetched again")));
        assert!(p.will_keep.iter().any(|w| w.contains("secrets.cfg")));
        assert!(p.warnings.iter().any(|w| w.contains("stop it")));
        assert_eq!(p.resources.len(), 2);
        // encrypted without a password: only says so
        let (info, path) = make(&rig, &git, Some("a good passphrase"));
        let p = preview(&rig.entry, &info, &path, None, false).unwrap();
        assert!(p.needs_passphrase && p.server.is_none());
        assert!(preview(&rig.entry, &info, &path, Some(&Passphrase::new("a good passphrase")), false).unwrap().server.is_some());
    }

    #[test]
    fn restoring_never_writes_outside_the_server() {
        // a tampered archive naming "server/../../x" is refused at unpacking (see the archive tests); and a name that
        // gets past is stopped here
        assert!(crate::fsutil::join_inside(Path::new("/srv/x"), "../evil").is_none());
        assert!(is_secret_name("secrets.cfg") && is_secret_name("database.json") && !is_secret_name("server.cfg"));
    }
}
