//! The backup operations the panel and the CLI call, written once over [`Backend`]: the default
//! methods of the trait use these, and a front end's test double overrides the trait methods.

use std::path::PathBuf;

use super::create::{self, CreateOptions};
use super::crypt::Passphrase;
use super::dbdump;
use super::dest::{self, Destination};
use super::restore::{self, RestoreOptions, RestorePreview, RestoreReport};
use super::types::*;
use super::{codes, settings, BackupEvent, Tools};
use crate::api::wire::{BackupCreateRequest, BackupCreated, BackupsView, DatabaseNote, UndoInfo, VerifyResult};
use crate::api::Backend;
use crate::db::SystemRunner;
use crate::installer::registry::ServerEntry;
use crate::problem::Problem;

type Res<T> = Result<T, String>;

fn pack(p: Problem) -> String {
    p.into()
}

fn pass(s: Option<&str>) -> Option<Passphrase> {
    s.map(|p| p.trim_end_matches(['\r', '\n'])).filter(|p| !p.is_empty()).map(Passphrase::new)
}

fn tools_for<'a>(db: &'a SystemRunner, git: &'a crate::git::SystemRunner) -> Tools<'a> {
    Tools::system(db, git)
}

fn destination(e: &ServerEntry) -> dest::LocalFolder {
    restore::destination_for(&e.name)
}

/// A folder the person typed: must be a real, absolute, local path outside the server folder.
pub fn check_folder(e: &ServerEntry, folder: &str) -> Result<(), Problem> {
    let f = folder.trim();
    if f.is_empty() {
        return Ok(());
    }
    let p = std::path::Path::new(f);
    if !p.is_absolute() {
        return Err(Problem::new(codes::DEST_UNWRITABLE, "Give the full path of the folder, starting from the top (for example /mnt/backups or D:\\Backups)."));
    }
    if f.contains('\0') || p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err(Problem::new(codes::DEST_UNWRITABLE, "That folder name is not valid."));
    }
    if p.starts_with(&e.dir) {
        return Err(Problem::new(
            codes::DEST_UNWRITABLE,
            "Backups cannot be kept inside the server's own folder: deleting the server would delete them too. Choose a folder outside it, ideally on another disk.",
        ));
    }
    Ok(())
}

pub fn view<B: Backend + ?Sized>(_b: &mut B, e: &ServerEntry) -> Res<BackupsView> {
    let cfg = settings::load_settings(&e.name);
    let status = settings::load_status(&e.name);
    let d = destination(e);
    let list = d.list();
    let now = crate::monitor::now_unix();
    let summary = settings::summary(&list, &cfg, &status, now);
    let undo = status.undo_id.as_ref().and_then(|id| list.iter().find(|b| &b.id == id)).map(|b| UndoInfo { id: b.id.clone(), created_unix: b.created_unix, encrypted: b.encrypted });
    let (kind, note) = match create::database_of(&e.dir) {
        Ok(create::DbSource::Local(_)) => (DbKind::Dumped, "The server's own database is copied as one consistent snapshot.".to_string()),
        Ok(create::DbSource::External(c)) => (DbKind::External, format!("The database is on {} (another computer). It is copied over the network with the saved login.", c.host)),
        Ok(create::DbSource::None(why)) => (DbKind::None, why),
        Err(p) => (DbKind::Failed, p.message),
    };
    let tool_missing = matches!(kind, DbKind::Dumped | DbKind::External) && dbdump::dump_program(&crate::installer::db_client(), &dbdump::program_runs).is_none();
    let folder = d.local_dir().map(|p| p.display().to_string()).unwrap_or_default();
    Ok(BackupsView {
        backups: list,
        summary,
        folder,
        default_folder: cfg.folder.trim().is_empty(),
        free_bytes: d.free_bytes(),
        undo,
        database: DatabaseNote { kind, note, tool_missing },
        schedule_note: "Scheduled backups run while Kora's background service is running (start it with `kora panel --background`). A backup that was missed is made up when it starts again.".into(),
        settings: cfg,
    })
}

pub fn create_backup<B: Backend + ?Sized>(b: &mut B, e: &ServerEntry, req: &BackupCreateRequest, admin: bool, say: &mut dyn FnMut(BackupEvent)) -> Res<BackupCreated> {
    let folder = match req.folder.as_deref().map(str::trim).filter(|f| !f.is_empty()) {
        Some(f) if !admin => {
            let _ = f;
            return Err(pack(Problem::generic("Only an administrator can choose another folder for a backup.")));
        }
        Some(f) => {
            check_folder(e, f).map_err(pack)?;
            Some(f.to_string())
        }
        None => None,
    };
    let passphrase = pass(req.passphrase.as_deref());
    if req.for_move && passphrase.is_none() {
        return Err(pack(Problem::new(codes::WEAK_PASSPHRASE, "A backup made to move a server needs a password: the file will travel, and it holds your licence key and database login.")));
    }
    let cfg = settings::load_settings(&e.name);
    let o = CreateOptions {
        passphrase,
        resources: if req.everything { ResourceChoice::Everything } else { cfg.resources },
        stop_server: req.stop_server,
        label: if req.for_move { Label::Move } else { Label::Manual },
        folder,
    };
    let (db, git) = (SystemRunner, crate::git::SystemRunner);
    let tools = tools_for(&db, &git);
    let mut host = super::host::BackendHost::new(b, e);
    let r = create::create(e, &o, &mut host, &tools, say).map_err(pack)?;
    Ok(BackupCreated { info: r.info, path: r.path.display().to_string(), notes: r.notes, pruned: r.pruned })
}

pub fn settings_set<B: Backend + ?Sized>(_b: &mut B, e: &ServerEntry, new: &BackupSettings) -> Res<BackupSettings> {
    check_folder(e, &new.folder).map_err(pack)?;
    settings::validate_schedule(&new.schedule).map_err(|m| pack(Problem::generic(m)))?;
    if new.retention.keep_last > 1000 || new.retention.keep_daily_days > 3650 {
        return Err(pack(Problem::generic("Those retention numbers are too large.")));
    }
    let old = settings::load_settings(&e.name);
    let mut new = new.clone();
    new.folder = new.folder.trim().to_string();
    if !new.folder.is_empty() {
        dest::LocalFolder::for_server(&new.folder, &e.name)
            .prepare()
            .map_err(|x| pack(Problem::new(codes::DEST_UNWRITABLE, format!("Kora cannot write to that folder ({x:?}). Check the path, the permissions and that the drive or share is connected."))))?;
    }
    settings::save_settings(&e.name, &new).map_err(pack)?;
    // A newly switched-on or changed schedule starts from now: it does not make up slots from before it existed.
    if new.schedule.enabled && new.schedule != old.schedule {
        let now = crate::monitor::now_unix();
        settings::update_status(&e.name, |s| s.last_scheduled_unix = Some(now));
    }
    Ok(new)
}

fn find(e: &ServerEntry, id: &str) -> Result<(BackupInfo, PathBuf), Problem> {
    let d = destination(e);
    let Some(info) = d.list().into_iter().find(|b| b.id == id) else {
        return Err(Problem::new(codes::NOT_FOUND, "That backup is not there any more."));
    };
    let path = d.open(id).ok_or_else(|| Problem::new(codes::NOT_FOUND, "That backup's file is not there any more."))?;
    Ok((info, path))
}

pub fn verify<B: Backend + ?Sized>(_b: &mut B, e: &ServerEntry, id: &str, p: Option<&str>, say: &mut dyn FnMut(BackupEvent)) -> Res<VerifyResult> {
    let (info, path) = find(e, id).map_err(pack)?;
    let m = restore::verify(&path, Some(&info), pass(p).as_ref(), say).map_err(pack)?;
    Ok(VerifyResult { ok: true, files: m.files.len() as u32, bytes: m.payload_bytes, summary: format!("All {} files match their checksums.", m.files.len()) })
}

pub fn preview<B: Backend + ?Sized>(b: &mut B, e: &ServerEntry, id: &str, p: Option<&str>) -> Res<RestorePreview> {
    let (info, path) = find(e, id).map_err(pack)?;
    if info.too_new {
        return Err(pack(Problem::new(codes::TOO_NEW, "This backup was made by a newer version of Kora. Update Kora first.")));
    }
    let running = b.status(e) == crate::api::Status::Running;
    restore::preview(e, &info, &path, pass(p).as_ref(), running).map_err(pack)
}

pub fn restore_backup<B: Backend + ?Sized>(b: &mut B, e: &ServerEntry, id: &str, p: Option<&str>, say: &mut dyn FnMut(BackupEvent)) -> Res<RestoreReport> {
    let (_, path) = find(e, id).map_err(pack)?;
    let (db, git) = (SystemRunner, crate::git::SystemRunner);
    let tools = tools_for(&db, &git);
    let mut host = super::host::BackendHost::new(b, e);
    restore::restore(e, &path, &RestoreOptions { passphrase: pass(p), no_safety_backup: false }, &mut host, &tools, say).map_err(pack)
}

pub fn undo<B: Backend + ?Sized>(b: &mut B, e: &ServerEntry, p: Option<&str>, say: &mut dyn FnMut(BackupEvent)) -> Res<RestoreReport> {
    let (db, git) = (SystemRunner, crate::git::SystemRunner);
    let tools = tools_for(&db, &git);
    let mut host = super::host::BackendHost::new(b, e);
    restore::undo(e, &destination(e), pass(p).as_ref(), &mut host, &tools, say).map_err(pack)
}

pub fn delete<B: Backend + ?Sized>(_b: &mut B, e: &ServerEntry, id: &str) -> Res<()> {
    let d = destination(e);
    if settings::load_status(&e.name).undo_id.as_deref() == Some(id) {
        settings::update_status(&e.name, |s| s.undo_id = None);
    }
    d.delete(id).map_err(|x| pack(Problem::new(codes::DEST_UNWRITABLE, "The backup could not be removed.").with_details(format!("{x:?}"))))
}

pub fn prune<B: Backend + ?Sized>(_b: &mut B, e: &ServerEntry) -> Res<Vec<String>> {
    let d = destination(e);
    let cfg = settings::load_settings(&e.name);
    let protect: Vec<String> = settings::load_status(&e.name).undo_id.into_iter().collect();
    let mut gone = vec![];
    for id in dest::plan_prune(&d.list(), &cfg.retention, crate::monitor::now_unix(), &protect) {
        if d.delete(&id).is_ok() {
            gone.push(id);
        }
    }
    Ok(gone)
}

pub fn file<B: Backend + ?Sized>(_b: &mut B, e: &ServerEntry, id: &str) -> Res<PathBuf> {
    find(e, id).map(|(_, p)| p).map_err(pack)
}

/// The "last backup" line of every server (reads each server's backup folder).
pub fn overview<B: Backend + ?Sized>(b: &mut B) -> Res<crate::api::wire::BackupsOverview> {
    let now = crate::monitor::now_unix();
    let servers = b
        .list()?
        .iter()
        .map(|e| {
            let list = destination(e).list();
            crate::api::wire::ServerBackupLine { server: e.name.clone(), summary: settings::summary(&list, &settings::load_settings(&e.name), &settings::load_status(&e.name), now) }
        })
        .collect();
    Ok(crate::api::wire::BackupsOverview { servers })
}

// ---- importing a server from a backup (see `super::import`)

fn open_source(src: &super::import::ImportSource, allow_path: bool) -> Result<PathBuf, String> {
    super::import::resolve_source(src, allow_path).map_err(pack)
}

pub fn import_inspect(src: &super::import::ImportSource, passphrase: Option<&str>, allow_path: bool, servers: &[ServerEntry]) -> Res<super::import::ImportPreview> {
    let file = open_source(src, allow_path)?;
    let keys: Vec<String> = crate::keystore::KeyStore::default().list().map(|l| l.into_iter().map(|k| k.name).collect()).unwrap_or_default();
    super::import::inspect(&file, passphrase, servers, &keys, &super::import::SystemPorts, &super::import::SystemForks, &|| crate::installer::check_db_admin(None), &crate::api::install::default_dir).map_err(pack)
}

pub fn import_run<B: Backend + ?Sized>(b: &mut B, req: &super::import::ImportRequest, allow_path: bool, say: &mut dyn FnMut(BackupEvent)) -> Res<super::import::ImportResult> {
    let file = open_source(&req.source, allow_path)?;
    let servers = b.list()?;
    let (db, git) = (SystemRunner, crate::git::SystemRunner);
    let tools = tools_for(&db, &git);
    let given = req.db_admin.as_ref().map(|a| crate::installer::DbAdmin { host: a.host.clone(), port: a.port, user: a.user.clone(), password: a.password.clone() });
    let admin = crate::installer::admin_conn_for(given.as_ref());
    let prov = super::import::SystemProvisioner { admin };
    let keys = crate::keystore::KeyStore::default();
    let saved = |n: &str| keys.get(n).ok();
    let env = super::import::Env { tools: &tools, fetcher: &super::import::SystemFetcher, provisioner: Some(&prov), saved_key: &saved, ports: &super::import::SystemPorts, forks: &super::import::SystemForks, servers };
    let result = super::import::run(req, &file, &env, say).map_err(pack)?;
    // the upload has done its job
    if let Some(id) = req.source.upload.as_deref() {
        let _ = std::fs::remove_dir_all(super::import::imports_dir().join(id));
    }
    if req.start {
        if let Some(e) = b.list()?.into_iter().find(|e| e.name == req.choices.name) {
            let mut r = result;
            r.started = b.control(&e, crate::service::ControlAction::Start).is_ok();
            if !r.started {
                r.problems.push("The server was imported but did not start. Start it from its page to see why.".into());
            }
            return Ok(r);
        }
    }
    Ok(result)
}

/// The backups of servers that were deleted (kept, listed apart, restorable as a new server).
pub fn archived_view() -> crate::api::wire::ArchivedBackups {
    crate::api::wire::ArchivedBackups { servers: settings::archived().into_iter().map(|a| crate::api::wire::ArchivedServer { name: a.name, deleted_unix: a.deleted_unix, backups: a.backups }).collect() }
}
