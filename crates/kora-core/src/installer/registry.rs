//! Server registry (in the Kora home's database, see [`crate::store`]; it was `servers.json` before v0.8) and
//! the per-server `kora-server.json`. Neither ever contains secrets.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::types::{Mode, Ports};
use crate::store::{problem_to_io, Store};

/// Directory holding the registry. `KORA_HOME` overrides (tests, portable installs).
pub fn kora_home() -> PathBuf {
    if let Some(h) = std::env::var_os("KORA_HOME") {
        return PathBuf::from(h);
    }
    if cfg!(windows) {
        if let Some(l) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(l).join("Kora");
        }
    }
    // HOME as before (under a plain `sudo` that is root's: the relaunch pins KORA_HOME first, see
    // `elevate`), then the account's passwd entry; never a relative `.kora`.
    let home =
        std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).filter(|h| !h.is_empty()).map(PathBuf::from).or_else(crate::userhome::real_user_home).unwrap_or_else(std::env::temp_dir);
    home.join(".kora")
}

/// Same folder? Windows paths ignore case, accept `/` for `\\` and a trailing separator
/// (`C:\\Kora\\Srv` = `c:/kora/srv/`); elsewhere paths are compared exactly.
pub fn same_path(a: &Path, b: &Path) -> bool {
    if !cfg!(windows) {
        return a == b;
    }
    same_path_windows(&a.to_string_lossy(), &b.to_string_lossy())
}

/// The Windows rule of [`same_path`] on plain strings (testable on every OS).
pub fn same_path_windows(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.replace('/', "\\").trim_end_matches('\\').to_lowercase();
    norm(a) == norm(b)
}

pub fn registry_path() -> PathBuf {
    kora_home().join("servers.json")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerEntry {
    pub name: String,
    pub dir: PathBuf,
    pub mode: Mode,
    pub ports: PortsRecord,
    pub created_unix: u64,
}

/// `Ports` without needing `PartialEq` on the contract type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortsRecord {
    pub game: u16,
    pub txadmin: u16,
}

impl From<&Ports> for PortsRecord {
    fn from(p: &Ports) -> Self {
        Self { game: p.game, txadmin: p.txadmin }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registry {
    #[serde(default)]
    pub servers: Vec<ServerEntry>,
}

impl Registry {
    /// The servers in the store of the Kora folder that holds `path` (the file name is only used to find
    /// the folder: the data lives in the database, see [`crate::store`]). Nothing registered: empty.
    pub fn load_from(path: &Path) -> std::io::Result<Self> {
        let Some(s) = Store::open_existing_beside(path).map_err(problem_to_io)? else { return Ok(Self::default()) };
        s.read(|c| read_servers(c)).map(|servers| Self { servers }).map_err(problem_to_io)
    }

    pub fn load() -> std::io::Result<Self> {
        Self::load_from(&registry_path())
    }

    /// Replace the whole list in one transaction. (Two programs that each load, change and save can still
    /// overwrite each other's change: use [`Registry::update_at`] for a change that must not be lost.)
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        let s = Store::open_beside(path).map_err(problem_to_io)?;
        s.write(|tx| write_servers(tx, &self.servers)).map_err(problem_to_io)
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&registry_path())
    }

    /// The old name of [`save_to`](Self::save_to): the registry used to be a file that was deleted when it
    /// became empty. There is no such file any more, so this is the same.
    pub fn save_or_clean_to(&self, path: &Path) -> std::io::Result<()> {
        if self.servers.is_empty() && Store::open_existing_beside(path).map_err(problem_to_io)?.is_none() {
            return Ok(());
        }
        self.save_to(path)
    }

    /// Read, change and write the list inside ONE transaction, so two programs changing it at the same
    /// moment cannot lose each other's change. `f` returns whether it changed anything.
    pub fn update_at<R>(path: &Path, f: impl FnOnce(&mut Registry) -> std::io::Result<R>) -> std::io::Result<R> {
        let s = Store::open_beside(path).map_err(problem_to_io)?;
        let mut inner: Option<std::io::Error> = None;
        let r = s.write(|tx| {
            let mut reg = Registry { servers: read_servers(tx)? };
            match f(&mut reg) {
                Ok(out) => {
                    write_servers(tx, &reg.servers)?;
                    Ok(out)
                }
                Err(e) => {
                    let p = crate::problem::Problem::generic(e.to_string());
                    inner = Some(e);
                    Err(p)
                }
            }
        });
        match r {
            Ok(v) => Ok(v),
            Err(p) => Err(inner.unwrap_or_else(|| problem_to_io(p))),
        }
    }

    /// [`update_at`](Self::update_at) for the Kora home of this process.
    pub fn update<R>(f: impl FnOnce(&mut Registry) -> std::io::Result<R>) -> std::io::Result<R> {
        Self::update_at(&registry_path(), f)
    }

    /// Insert or replace by name.
    pub fn upsert(&mut self, e: ServerEntry) {
        self.servers.retain(|s| s.name != e.name);
        self.servers.push(e);
    }

    pub fn remove(&mut self, name: &str) -> bool {
        let n = self.servers.len();
        self.servers.retain(|s| s.name != name);
        n != self.servers.len()
    }

    /// Find by registry name or by directory.
    pub fn find(&self, name_or_dir: &str) -> Option<&ServerEntry> {
        self.servers
            .iter()
            .find(|s| s.name == name_or_dir)
            .or_else(|| self.servers.iter().find(|s| super::slug(&s.name) == name_or_dir))
            .or_else(|| self.servers.iter().find(|s| same_path(&s.dir, Path::new(name_or_dir))))
    }
}


// ---- the `servers` table ---------------------------------------------------------------------

fn read_servers(c: &rusqlite::Connection) -> Result<Vec<ServerEntry>, crate::problem::Problem> {
    let mut st = c.prepare("SELECT name, dir, mode, game_port, txadmin_port, created_unix FROM servers ORDER BY seq")?;
    let rows = st.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?, r.get::<_, i64>(4)?, r.get::<_, i64>(5)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (name, dir, mode, game, txadmin, created) = row?;
        let mode: Mode = serde_json::from_value(serde_json::Value::String(mode.clone()))
            .map_err(|_| crate::problem::Problem::new(crate::store::codes::STORE_UNREADABLE, format!("Kora's server list has an unknown mode \"{mode}\" for \"{name}\". Nothing was changed.")))?;
        out.push(ServerEntry { name, dir: PathBuf::from(dir), mode, ports: PortsRecord { game: game as u16, txadmin: txadmin as u16 }, created_unix: created as u64 });
    }
    Ok(out)
}

pub(crate) fn mode_text(m: Mode) -> String {
    match serde_json::to_value(m) {
        Ok(serde_json::Value::String(s)) => s,
        _ => "production".into(),
    }
}

fn write_servers(tx: &rusqlite::Connection, servers: &[ServerEntry]) -> Result<(), crate::problem::Problem> {
    // Names are unique: the last of two equal names wins, like `upsert`.
    tx.execute("DELETE FROM servers", [])?;
    let mut st = tx.prepare("INSERT INTO servers (name, seq, dir, mode, game_port, txadmin_port, created_unix) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT(name) DO UPDATE SET seq = excluded.seq, dir = excluded.dir, mode = excluded.mode, game_port = excluded.game_port, txadmin_port = excluded.txadmin_port, created_unix = excluded.created_unix")?;
    for (i, e) in servers.iter().enumerate() {
        st.execute(rusqlite::params![e.name, i as i64, e.dir.to_string_lossy(), mode_text(e.mode), e.ports.game as i64, e.ports.txadmin as i64, e.created_unix as i64])?;
    }
    Ok(())
}

/// One installed resource, as recorded in `kora-server.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceRecord {
    pub id: String,
    /// Relative to `<dir>/resources/`.
    pub path: String,
    pub repository: String,
    pub branch: String,
    /// Pinned commit (production releases); `None` when tracking a branch head.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// A pinned copy: a bridge / creator resource (somebody else's repository) or a subpath install.
    /// Never a fork, in any mode, so the development updater leaves it alone.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub third_party: bool,
    /// Where it came from when it is not a git clone: `"portal"` for a paid (Cfx.re escrowed) asset
    /// imported from a zip ([`crate::paidassets`]). Updaters and catalog tools leave those alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Version from the asset's `fxmanifest.lua`, when it has one (paid assets).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// When it was imported (paid assets).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_unix: Option<u64>,
}

impl ResourceRecord {
    /// A paid asset imported from a Cfx.re Portal zip: not a git clone, never updated or forked.
    pub fn is_portal(&self) -> bool {
        self.source.as_deref() == Some(crate::paidassets::SOURCE)
    }
}

/// `<dir>/kora-server.json`: config and install result. Secret-free.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerFile {
    pub name: String,
    pub mode: Mode,
    pub dir: PathBuf,
    pub ports: PortsRecord,
    pub game_build: u32,
    pub artifact_build: u32,
    pub database: Option<String>,
    pub github_owner: Option<String>,
    pub created_unix: u64,
    pub kora_version: String,
    pub resources: Vec<ResourceRecord>,
}

pub fn server_file_path(dir: &Path) -> PathBuf {
    dir.join("kora-server.json")
}

impl ServerFile {
    pub fn load(dir: &Path) -> std::io::Result<Self> {
        serde_json::from_slice(&fs::read(server_file_path(dir))?).map_err(std::io::Error::other)
    }
    pub fn save(&self) -> std::io::Result<()> {
        let p = server_file_path(&self.dir);
        let tmp = p.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?)?;
        fs::rename(tmp, p)
    }
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("sub/servers.json");
        let mut r = Registry::load_from(&p).unwrap();
        assert!(r.servers.is_empty());
        let e = ServerEntry { name: "a".into(), dir: "/x".into(), mode: Mode::Production, ports: PortsRecord { game: 30120, txadmin: 40120 }, created_unix: 1 };
        r.upsert(e.clone());
        r.upsert(ServerEntry { created_unix: 2, ..e });
        r.save_to(&p).unwrap();
        let r2 = Registry::load_from(&p).unwrap();
        assert_eq!(r2.servers.len(), 1);
        assert_eq!(r2.find("a").unwrap().created_unix, 2);
        assert!(r2.find("/x").is_some());
        let mut r2 = r2;
        assert!(r2.remove("a") && !r2.remove("a"));
        r2.save_or_clean_to(&p).unwrap();
        assert!(Registry::load_from(&p).unwrap().servers.is_empty());
        r2.save_or_clean_to(&p).unwrap(); // saving an empty list again: fine
    }

    #[test]
    fn windows_paths_compare_without_case_slashes_or_trailing_separator() {
        assert!(same_path_windows("C:\\Kora\\Srv", "c:/kora/srv/"));
        assert!(same_path_windows("C:\\Kora\\Srv\\", "C:\\KORA\\SRV"));
        assert!(!same_path_windows("C:\\Kora\\Srv", "C:\\Kora\\Srv2"));
        assert!(!same_path_windows("C:\\Kora", "D:\\Kora"));
    }
}
