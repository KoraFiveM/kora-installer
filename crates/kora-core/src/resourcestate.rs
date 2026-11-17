//! "Restart needed" tracking for the resources of a server.
//!
//! When the files of a resource change (an update, a switch of branch, trying a pull request, going back to the
//! normal version, a rollback) a running server still has the old version loaded. This module remembers, on the
//! server side, which resources changed and whether the resource or the whole server has been restarted since, so the
//! notice survives page reloads and shows for every panel user, the command line included.
//!
//! The method is a counter, not a clock: every event takes the next number, a resource "needs a restart" while its
//! change number is higher than the number of its last restart and of the last restart of the whole server. Two
//! events in the same millisecond can never be mixed up. Times are kept too, only for display.
//!
//! Where it is kept is behind [`StateStore`]. [`FileStore`] (a small JSON file in the server folder) is the plain
//! implementation for now; the database work can implement the trait and move the data at merge time. Nothing here
//! is secret and nothing here decides anything about safety: it only drives a notice.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// File name inside the server folder.
pub const FILE: &str = "kora-resource-state.json";

/// What the notice says (one wording, used by the panel and the command line).
pub const NOTICE: &str = "Restart needed: the running server still has the old version.";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceMark {
    /// Number of the last change of the files.
    pub changed_seq: u64,
    pub changed_unix: u64,
    /// Plain words for what changed ("switched to branch fix-ui").
    #[serde(default)]
    pub why: String,
    /// Number of the last restart of this resource alone.
    #[serde(default)]
    pub restarted_seq: u64,
    #[serde(default)]
    pub restarted_unix: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub version: u32,
    /// The last number handed out.
    #[serde(default)]
    pub clock: u64,
    /// Number of the last start or restart of the whole server.
    #[serde(default)]
    pub server_restarted_seq: u64,
    #[serde(default)]
    pub server_restarted_unix: u64,
    #[serde(default)]
    pub resources: BTreeMap<String, ResourceMark>,
}

/// Where the state lives. One value per server folder.
pub trait StateStore: Send + Sync {
    fn load(&self, server_dir: &Path) -> State;
    fn save(&self, server_dir: &Path, state: &State) -> std::io::Result<()>;
}

/// A JSON file in the server folder. A missing or unreadable file is an empty state (nothing to restart).
#[derive(Debug, Default, Clone, Copy)]
pub struct FileStore;

impl FileStore {
    fn path(dir: &Path) -> PathBuf {
        dir.join(FILE)
    }
}

impl StateStore for FileStore {
    fn load(&self, dir: &Path) -> State {
        std::fs::read_to_string(Self::path(dir)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    fn save(&self, dir: &Path, state: &State) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(state).map_err(std::io::Error::other)?;
        let target = Self::path(dir);
        let tmp = dir.join(format!("{FILE}.tmp"));
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &target)
    }
}

fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn key(id: &str) -> String {
    crate::catalog::normalize_id(id)
}

/// The files of these resources changed. Never fails the caller: a notice that cannot be saved is only a notice lost.
pub fn mark_changed_with(store: &dyn StateStore, dir: &Path, ids: &[String], why: &str, now: u64) {
    if ids.is_empty() {
        return;
    }
    let mut st = store.load(dir);
    st.version = 1;
    for id in ids {
        st.clock += 1;
        let m = st.resources.entry(key(id)).or_default();
        m.changed_seq = st.clock;
        m.changed_unix = now;
        m.why = why.to_string();
    }
    let _ = store.save(dir, &st);
}

/// One resource was restarted (or started) on the running server.
pub fn mark_resource_restarted_with(store: &dyn StateStore, dir: &Path, id: &str, now: u64) {
    let mut st = store.load(dir);
    // Nothing recorded for it: nothing to clear (and no file to write).
    if !st.resources.contains_key(&key(id)) {
        return;
    }
    st.clock += 1;
    let clock = st.clock;
    if let Some(m) = st.resources.get_mut(&key(id)) {
        m.restarted_seq = clock;
        m.restarted_unix = now;
    }
    let _ = store.save(dir, &st);
}

/// The whole server was started or restarted: everything is loaded fresh.
pub fn mark_server_restarted_with(store: &dyn StateStore, dir: &Path, now: u64) {
    let mut st = store.load(dir);
    if st.resources.is_empty() && st.server_restarted_seq == 0 {
        return;
    }
    st.version = 1;
    st.clock += 1;
    st.server_restarted_seq = st.clock;
    st.server_restarted_unix = now;
    let _ = store.save(dir, &st);
}

/// What the panel shows for one resource.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RestartInfo {
    /// The running server still has the old version of this resource.
    pub needed: bool,
    /// What changed, when it is `needed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub why: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub changed_unix: Option<u64>,
    /// The words to show.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub notice: Option<String>,
}

/// Is a restart needed for `id`? A server that is not running has nothing old loaded, so it never is.
pub fn info_with(store: &dyn StateStore, dir: &Path, id: &str, server_running: bool) -> RestartInfo {
    let st = store.load(dir);
    info_of(&st, id, server_running, None)
}

/// `server_started`: when the game server process started, if it could be read. A server that started after the change
/// has loaded the new files, whoever restarted it (txAdmin included).
pub fn info_of(st: &State, id: &str, server_running: bool, server_started: Option<u64>) -> RestartInfo {
    let Some(m) = st.resources.get(&key(id)) else {
        return RestartInfo::default();
    };
    if !server_running || m.changed_seq <= m.restarted_seq.max(st.server_restarted_seq) || server_started.is_some_and(|t| t > m.changed_unix) {
        return RestartInfo::default();
    }
    RestartInfo { needed: true, why: Some(m.why.clone()).filter(|w| !w.is_empty()), changed_unix: Some(m.changed_unix), notice: Some(NOTICE.to_string()) }
}

/// Ids of the resources that need a restart now.
pub fn pending_with(store: &dyn StateStore, dir: &Path, server_running: bool) -> Vec<String> {
    let st = store.load(dir);
    let started = if server_running { crate::monitor::probe::server_started_unix(dir) } else { None };
    st.resources.keys().filter(|id| info_of(&st, id, server_running, started).needed).cloned().collect()
}

// Everyday entry points: the file store and the clock.

pub fn mark_changed(dir: &Path, ids: &[String], why: &str) {
    mark_changed_with(&FileStore, dir, ids, why, now_unix());
}

pub fn mark_resource_restarted(dir: &Path, id: &str) {
    mark_resource_restarted_with(&FileStore, dir, id, now_unix());
}

pub fn mark_server_restarted(dir: &Path) {
    mark_server_restarted_with(&FileStore, dir, now_unix());
}

pub fn info(dir: &Path, id: &str, server_running: bool) -> RestartInfo {
    let started = if server_running { crate::monitor::probe::server_started_unix(dir) } else { None };
    info_of(&FileStore.load(dir), id, server_running, started)
}

pub fn pending(dir: &Path, server_running: bool) -> Vec<String> {
    pending_with(&FileStore, dir, server_running)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn nothing_recorded_means_nothing_needed() {
        let d = dir();
        assert!(!info(d.path(), "kora-lib", true).needed);
        assert!(pending(d.path(), true).is_empty());
    }

    #[test]
    fn a_change_needs_a_restart_until_the_resource_is_restarted() {
        let d = dir();
        mark_changed(d.path(), &["kora-lib".into()], "switched to branch fix");
        let i = info(d.path(), "kora-lib", true);
        assert!(i.needed);
        assert_eq!(i.why.as_deref(), Some("switched to branch fix"));
        assert_eq!(i.notice.as_deref(), Some(NOTICE));
        // Other resources are not affected, ids are compared the way the registry does.
        assert!(!info(d.path(), "other", true).needed);
        mark_resource_restarted(d.path(), "kora_lib");
        assert!(!info(d.path(), "kora-lib", true).needed);
    }

    #[test]
    fn a_new_change_after_a_restart_needs_one_again() {
        let d = dir();
        mark_changed(d.path(), &["a".into()], "one");
        mark_resource_restarted(d.path(), "a");
        mark_changed(d.path(), &["a".into()], "two");
        assert!(info(d.path(), "a", true).needed);
    }

    #[test]
    fn restarting_the_server_clears_everything_before_it_and_nothing_after_it() {
        let d = dir();
        mark_changed(d.path(), &["a".into(), "b".into()], "x");
        mark_server_restarted(d.path());
        assert!(pending(d.path(), true).is_empty());
        mark_changed(d.path(), &["b".into()], "y");
        assert_eq!(pending(d.path(), true), vec!["b".to_string()]);
    }

    #[test]
    fn a_stopped_server_has_nothing_old_loaded() {
        let d = dir();
        mark_changed(d.path(), &["a".into()], "x");
        assert!(!info(d.path(), "a", false).needed);
        // ... but the note comes back if it is running again without a restart through Kora.
        assert!(info(d.path(), "a", true).needed);
    }

    #[test]
    fn survives_reading_it_again_and_a_broken_file() {
        let d = dir();
        mark_changed(d.path(), &["a".into()], "x");
        assert!(FileStore.load(d.path()).resources.contains_key("a"));
        std::fs::write(d.path().join(FILE), "not json").unwrap();
        assert!(!info(d.path(), "a", true).needed);
    }

    #[test]
    fn a_server_that_started_after_the_change_has_the_new_files() {
        let mut st = State { clock: 1, ..Default::default() };
        st.resources.insert("a".into(), ResourceMark { changed_seq: 1, changed_unix: 1000, why: "x".into(), ..Default::default() });
        assert!(info_of(&st, "a", true, None).needed);
        assert!(info_of(&st, "a", true, Some(900)).needed, "started before the change: still old");
        assert!(!info_of(&st, "a", true, Some(1001)).needed, "restarted from txAdmin after the change");
    }

    #[test]
    fn a_custom_store_can_be_used() {
        use std::sync::Mutex;
        #[derive(Default)]
        struct Mem(Mutex<State>);
        impl StateStore for Mem {
            fn load(&self, _: &Path) -> State {
                self.0.lock().unwrap().clone()
            }
            fn save(&self, _: &Path, s: &State) -> std::io::Result<()> {
                *self.0.lock().unwrap() = s.clone();
                Ok(())
            }
        }
        let m = Mem::default();
        mark_changed_with(&m, Path::new("x"), &["a".into()], "w", 5);
        assert!(info_with(&m, Path::new("x"), "a", true).needed);
    }
}
