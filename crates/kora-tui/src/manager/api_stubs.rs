//! TEMPORARY local stand-ins for the APIs other branches deliver in `kora-core`
//! (`update::manual`, `notify`, `service::schedule` get/set/remove, `ServerRecord`).
//!
//! Delete this file when the real ones land: in `manager/mod.rs` change the single line
//! `pub use api_stubs as api;` to re-export the real `kora_core` items under the same names,
//! and fix up the imports in `adapter.rs`. Nothing else in the manager names this file.
//!
//! The shapes are exactly the contract given to the manager milestone; the functions here only
//! say "not available yet" (or keep a per-process in-memory value), they never touch disk.

use std::sync::Mutex;

use kora_core::installer::ServerEntry;

/// The registry record of one server.
pub type ServerRecord = ServerEntry;

pub type Result<T> = std::result::Result<T, String>;

pub mod manual {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Scope {
        All,
        Resources(Vec<String>),
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ChangeStatus {
        Clean,
        Dirty,
        Diverged,
        UpToDate,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ResourceChange {
        pub id: String,
        pub from: String,
        pub to: String,
        pub behind: u32,
        pub changelog: Vec<String>,
        pub needs_restart: bool,
        pub status: ChangeStatus,
    }

    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct UpdatePreview {
        pub resources: Vec<ResourceChange>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ApplyOpts {
        /// Restart the server afterwards when a changed resource needs it.
        pub restart_if_needed: bool,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct UpdateEvent {
        pub resource: String,
        pub message: String,
    }

    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct UpdateReport {
        pub updated: Vec<String>,
        pub failed: Vec<(String, String)>,
        /// Id to give to [`rollback`] to undo this run.
        pub snapshot: Option<String>,
        pub restarted: bool,
    }

    const NOT_YET: &str = "updates from the manager are not available in this build yet";

    pub fn preview(_s: &ServerRecord, _scope: Scope) -> Result<UpdatePreview> {
        Err(NOT_YET.into())
    }

    pub fn apply(_s: &ServerRecord, _scope: Scope, _o: ApplyOpts, _on: &mut dyn FnMut(UpdateEvent)) -> Result<UpdateReport> {
        Err(NOT_YET.into())
    }

    pub fn rollback(_s: &ServerRecord, _snapshot: &str) -> Result<()> {
        Err(NOT_YET.into())
    }
}

pub mod notify {
    use super::*;

    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct NotifyConfig {
        pub discord_webhook: Option<String>,
    }

    static MEM: Mutex<Option<NotifyConfig>> = Mutex::new(None);

    pub fn load(_s: &ServerRecord) -> NotifyConfig {
        MEM.lock().ok().and_then(|g| g.clone()).unwrap_or_default()
    }

    pub fn save(_s: &ServerRecord, c: &NotifyConfig) -> Result<()> {
        if let Ok(mut g) = MEM.lock() {
            *g = Some(c.clone());
        }
        Ok(())
    }

    pub fn send_test(_c: &NotifyConfig) -> Result<()> {
        Err("sending a test message is not available in this build yet".into())
    }
}

pub mod schedule {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Preset {
        Daily,
        Weekly,
        Custom,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum RunAs {
        InstallingUser,
        System,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ScheduleConfig {
        pub preset: Preset,
        pub time: String,
        pub weekday: Option<u8>,
        pub run_as: RunAs,
    }

    static MEM: Mutex<Option<ScheduleConfig>> = Mutex::new(None);

    pub fn get(_s: &ServerRecord) -> Option<ScheduleConfig> {
        MEM.lock().ok().and_then(|g| g.clone())
    }

    pub fn set(_s: &ServerRecord, c: &ScheduleConfig) -> Result<()> {
        if let Ok(mut g) = MEM.lock() {
            *g = Some(c.clone());
        }
        Ok(())
    }

    pub fn remove(_s: &ServerRecord) -> Result<()> {
        if let Ok(mut g) = MEM.lock() {
            *g = None;
        }
        Ok(())
    }
}
