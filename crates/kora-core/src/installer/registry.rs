//! Server registry (`~/.kora/servers.json`, Windows `%LOCALAPPDATA%\Kora\servers.json`) and the
//! per-server `kora-server.json`. Neither file ever contains secrets.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::types::{Mode, Ports};

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
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).unwrap_or_default();
    PathBuf::from(home).join(".kora")
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
    pub fn load_from(path: &Path) -> std::io::Result<Self> {
        match fs::read(path) {
            Ok(b) => serde_json::from_slice(&b).map_err(std::io::Error::other),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }

    pub fn load() -> std::io::Result<Self> {
        Self::load_from(&registry_path())
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(d) = path.parent() {
            fs::create_dir_all(d)?;
        }
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?)?;
        fs::rename(tmp, path)
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&registry_path())
    }

    /// Like [`save_to`](Self::save_to), but when no server is left the (empty) registry file is
    /// deleted, and its folder too if that leaves it empty.
    pub fn save_or_clean_to(&self, path: &Path) -> std::io::Result<()> {
        if !self.servers.is_empty() {
            return self.save_to(path);
        }
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        }
        if let Some(d) = path.parent() {
            let _ = fs::remove_dir(d); // only succeeds when empty
        }
        Ok(())
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
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
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
        let e = ServerEntry {
            name: "a".into(),
            dir: "/x".into(),
            mode: Mode::Production,
            ports: PortsRecord { game: 30120, txadmin: 40120 },
            created_unix: 1,
        };
        r.upsert(e.clone());
        r.upsert(ServerEntry { created_unix: 2, ..e });
        r.save_to(&p).unwrap();
        let r2 = Registry::load_from(&p).unwrap();
        assert_eq!(r2.servers.len(), 1);
        assert_eq!(r2.find("a").unwrap().created_unix, 2);
        assert!(r2.find("/x").is_some());
        let mut r2 = r2;
        assert!(r2.remove("a") && !r2.remove("a"));
        // an emptied registry disappears (with its folder, when nothing else is in it)
        r2.save_or_clean_to(&p).unwrap();
        assert!(!p.exists() && !p.parent().unwrap().exists());
        r2.save_or_clean_to(&p).unwrap(); // already gone: fine
    }

    #[test]
    fn windows_paths_compare_without_case_slashes_or_trailing_separator() {
        assert!(same_path_windows("C:\\Kora\\Srv", "c:/kora/srv/"));
        assert!(same_path_windows("C:\\Kora\\Srv\\", "C:\\KORA\\SRV"));
        assert!(!same_path_windows("C:\\Kora\\Srv", "C:\\Kora\\Srv2"));
        assert!(!same_path_windows("C:\\Kora", "D:\\Kora"));
    }
}
