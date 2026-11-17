//! Backups of one server, restoring them, and moving a server to another computer.
//!
//! # What a backup is
//! One archive file (`<name>-<id>.tar.gz`, or `.tar.gz.age` when password protected) and a small
//! listing file next to it (`<name>-<id>.json`, secret-free). The archive holds the server's own
//! database as a logical dump, its configuration (`server.cfg`, `secrets.cfg`, permissions, txAdmin's
//! data), Kora's records for it, and its resources: by reference (repository, branch, exact commit) where
//! they can be fetched again, in full where they cannot (changed here, kept-aside changes, unpushed
//! commits, paid assets, anything that is not a git clone). The FXServer build is not in it: it is
//! downloaded again and only its number is recorded.
//!
//! # Threat model
//! The archive contains secrets by necessity: the licence key, the database login, the txAdmin admin.
//! It is therefore created owner-only (0600; on Windows the same protection Kora gives its store), is
//! only ever downloadable by an administrator, and can be protected with a password (see [`crypt`]).
//! Restoring replaces a running server's data, so only an administrator may do it. A backup file may
//! come from anywhere (an import): it is verified before anything is touched, names inside it cannot
//! leave the staging folder, and nothing in it is ever executed.
//!
//! # Consistency
//! The database is one snapshot (`--single-transaction`). Files are copied one by one while the server
//! runs, so two files can be from slightly different moments (a config edited during the copy, a txAdmin
//! database being written). That is "consistent enough" for a game server, and a file seen changing is
//! noted in the manifest. "Stop the server during the backup" gives a strictly consistent copy.
//!
//! # Layout of this module
//! `types` shapes · `crypt` password protection · `archive` the file · `dbdump` the database ·
//! `dest` where backups go (a trait; only a folder today), retention · `settings` per-server settings and
//! the "last backup" line · `create` · `restore` · `scheduler` (runs scheduled backups in the panel's
//! background service) · `import` (move a server to this computer) · `support` (the support bundle).

pub mod archive;
pub mod clock;
pub mod crypt;
pub mod dbdump;
pub mod dest;
pub mod settings;
pub mod types;

pub use types::*;

/// Problem codes of this area.
pub mod codes {
    pub const NEEDS_PASSPHRASE: &str = "backup.needs_passphrase";
    pub const WRONG_PASSPHRASE: &str = "backup.wrong_passphrase";
    pub const CORRUPT: &str = "backup.corrupt";
    pub const TOO_NEW: &str = "backup.too_new";
    pub const DISK_FULL: &str = "backup.disk_full";
    pub const DUMP_TOOL_MISSING: &str = "backup.dump_tool_missing";
    pub const DB_STOPPED: &str = "backup.db_stopped";
    pub const DB_LOGIN: &str = "backup.db_login";
    pub const DEST_UNWRITABLE: &str = "backup.destination_unwritable";
    pub const NOT_FOUND: &str = "backup.not_found";
    pub const BUSY: &str = "backup.busy";
    pub const NO_UNDO: &str = "backup.no_undo";
    pub const WEAK_PASSPHRASE: &str = "backup.weak_passphrase";
}

pub mod create;
pub mod host;
pub mod import;
pub mod resources;
pub mod restore;
pub mod scheduler;
pub mod service;

use crate::db::SystemRunner;
use serde::{Deserialize, Serialize};

/// One step of a backup, restore or import, as a job event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BackupEvent {
    /// A short stable name of the step ("database", "files", ...).
    pub step: String,
    /// Plain words for the person.
    pub message: String,
    /// 0..=100 for the whole operation.
    pub percent: u8,
}

/// Starting and stopping the server around a backup or restore (the panel's backend implements it).
pub trait Host {
    fn running(&mut self) -> bool;
    fn stop(&mut self) -> Result<(), String>;
    fn start(&mut self) -> Result<(), String>;
}

/// Everything that touches the machine, so tests can fake it.
pub struct Tools<'a> {
    pub db: &'a dyn dbdump::StreamRunner,
    pub git: &'a dyn crate::git::CommandRunner,
    /// The database client for queries and loading a dump.
    pub db_client: String,
    /// The dump program, when one was found.
    pub dump_program: Option<String>,
    /// Now, in unix seconds.
    pub now: u64,
}

impl<'a> Tools<'a> {
    /// The real tools: the database client Kora uses everywhere else, and its dump program.
    pub fn system(db: &'a SystemRunner, git: &'a crate::git::SystemRunner) -> Tools<'a> {
        let client = crate::installer::db_client();
        let dump_program = dbdump::dump_program(&client, &dbdump::program_runs);
        Tools { db, git, db_client: client, dump_program, now: crate::monitor::now_unix() }
    }
}

#[cfg(test)]
pub(crate) mod tests_support;
