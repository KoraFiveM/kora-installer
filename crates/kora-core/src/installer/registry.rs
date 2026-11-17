//! Server registry (in the Kora home's database, see [`crate::store`]; it was `servers.json` before v0.8) and
//! the per-server `kora-server.json`. Neither ever contains secrets.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::types::{Mode, Ports, Source, Versions};
use crate::store::Store;

/// Directory holding the registry. `KORA_HOME` overrides (tests, portable installs).
pub fn kora_home() -> PathBuf {
    if let Some(h) = std::env::var_os("KORA_HOME") {
        return PathBuf::from(h);
    }
    refuse_real_home_in_tests();
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

/// Is this executable a Rust test binary? Cargo names them `<crate or test name>-<16 hex digits>`; the shipped
/// program is `kora` / `kora.exe`. (Pure: the file name is given.)
pub fn is_test_binary_name(file_stem: &str) -> bool {
    match file_stem.rsplit_once('-') {
        Some((name, hash)) => !name.is_empty() && hash.len() == 16 && hash.bytes().all(|b| b.is_ascii_hexdigit()),
        None => false,
    }
}

/// The Kora folder of this test process: an empty temporary folder, made once and used from then on (`KORA_HOME`
/// is set to it). For tests that do not need a folder of their own but must not reach the real one. A test that
/// needs its own sets `KORA_HOME` to a `tempfile::tempdir()` itself (and keeps the `TempDir` alive).
pub fn use_test_home() -> PathBuf {
    static HOME: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    if let Some(h) = std::env::var_os("KORA_HOME") {
        return PathBuf::from(h);
    }
    let h = HOME.get_or_init(|| {
        let mut b = [0u8; 4];
        let _ = getrandom::fill(&mut b);
        let p = std::env::temp_dir().join(format!("kora-test-home-{}-{}", std::process::id(), hex::encode(b)));
        let _ = std::fs::create_dir_all(&p);
        p
    });
    std::env::set_var("KORA_HOME", h);
    h.clone()
}

/// Tests must never read or write the real Kora folder (the developer's own saved keys, server list ...). A test
/// binary that asks for the Kora home without `KORA_HOME` set panics here, naming the way out. The unit tests of
/// kora-core itself get [`use_test_home`] automatically (they cannot reach the real folder either way).
/// `KORA_ALLOW_REAL_HOME=1` switches the guard off for a deliberate run (never in CI).
fn refuse_real_home_in_tests() {
    if std::env::var_os("KORA_ALLOW_REAL_HOME").is_some() {
        return;
    }
    if cfg!(test) {
        use_test_home();
        return;
    }
    let exe = std::env::current_exe().ok();
    let stem = exe.as_deref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    if is_test_binary_name(&stem) {
        panic!(
            "this test asked for the real Kora folder (~/.kora or %LOCALAPPDATA%\\Kora). Tests must use a temporary one: call \
             kora_core::installer::registry::use_test_home() first (an empty folder for this test process), or set KORA_HOME to a \
             tempfile::tempdir() path (keep the TempDir alive; tests that share the variable take turns with a lock, see \
             crates/kora-web/tests/keys.rs `rig()`), or pass an explicit path to the `::at(..)` constructor of the store you use."
        );
    }
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

/// One registered server.
///
/// # Adding a field to the server entry
/// The list is stored as JSON per server (the `record` column of the `servers` table), so a new field needs NO
/// database migration: add it here with `#[serde(default)]` (and `skip_serializing_if` when it should not be written
/// when empty) so records written before it existed still read and old `servers.json` files still import. A field a
/// NEWER Kora added survives an older Kora saving the list (unknown keys of an existing record are kept). Only a field
/// that must be LOOKED UP by, not just read back, needs a column: that is a new numbered migration in `store/schema.rs`.
/// A change of meaning (like mapping the old `mode`) belongs in a `#[serde(...)]` conversion or in `read_servers`.
/// Test: `registry::tests::new_fields_need_no_migration`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerEntry {
    pub name: String,
    pub dir: PathBuf,
    /// The old spelling, still written for older versions of Kora (see [`super::style`]).
    pub mode: Mode,
    /// Clone or fork. Missing in records made before 0.8: then it follows `mode`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    /// Released versions or the latest changes. Missing in records made before 0.8: then it follows `mode`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub versions: Option<Versions>,
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
    pub fn load_from(path: &Path) -> crate::problem::Res<Self> {
        let Some(s) = Store::open_existing_beside(path)? else { return Ok(Self::default()) };
        blocked(&s)?;
        s.read(read_servers).map(|servers| Self { servers })
    }

    pub fn load() -> crate::problem::Res<Self> {
        Self::load_from(&registry_path())
    }

    /// Replace the whole list in one transaction. (Two programs that each load, change and save can still
    /// overwrite each other's change: use [`Registry::update_at`] for a change that must not be lost.)
    pub fn save_to(&self, path: &Path) -> crate::problem::Res<()> {
        let s = Store::open_beside(path)?;
        blocked(&s)?;
        s.write(|tx| write_servers(tx, &self.servers))
    }

    pub fn save(&self) -> crate::problem::Res<()> {
        self.save_to(&registry_path())
    }

    /// The old name of [`save_to`](Self::save_to): the registry used to be a file that was deleted when it
    /// became empty. There is no such file any more, so this is the same.
    pub fn save_or_clean_to(&self, path: &Path) -> crate::problem::Res<()> {
        if self.servers.is_empty() && Store::open_existing_beside(path)?.is_none() {
            return Ok(());
        }
        self.save_to(path)
    }

    /// Read, change and write the list inside ONE transaction, so two programs changing it at the same
    /// moment cannot lose each other's change. `f` returns whether it changed anything.
    pub fn update_at<R>(path: &Path, f: impl FnOnce(&mut Registry) -> crate::problem::Res<R>) -> crate::problem::Res<R> {
        let s = Store::open_beside(path)?;
        blocked(&s)?;
        s.write(|tx| {
            let mut reg = Registry { servers: read_servers(tx)? };
            let out = f(&mut reg)?;
            write_servers(tx, &reg.servers)?;
            Ok(out)
        })
    }

    /// [`update_at`](Self::update_at) for the Kora home of this process.
    pub fn update<R>(f: impl FnOnce(&mut Registry) -> crate::problem::Res<R>) -> crate::problem::Res<R> {
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
    let mut st = c.prepare("SELECT name, record FROM servers ORDER BY seq")?;
    let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?.collect::<Result<Vec<_>, _>>()?;
    let mut out = Vec::new();
    for (name, record) in rows {
        // Fields this Kora does not know are ignored; fields the record lacks take their serde defaults.
        let e: ServerEntry =
            serde_json::from_str(&record).map_err(|e| {
                crate::problem::Problem::new(crate::store::codes::STORE_UNREADABLE, format!("Kora's server list has an entry for \"{name}\" it cannot read. Nothing was changed."))
                    .with_details(format!("line {}, column {}", e.line(), e.column()))
            })?;
        out.push(e);
    }
    Ok(out)
}

/// The server list is waiting on an old `servers.json` that could not be imported: say so, never start empty.
fn blocked(s: &Store) -> crate::problem::Res<()> {
    match s.blocked("servers") {
        Some(b) => Err(crate::problem::Problem::new(
            crate::store::codes::STORE_UNREADABLE,
            format!("Kora cannot read its old server list {}; it was left exactly as it was. Fix or move that file and try again.", b.file.display()),
        )
        .with_details(b.why.clone())),
        None => Ok(()),
    }
}

pub(crate) fn read_servers_pub(c: &rusqlite::Connection) -> Result<Vec<ServerEntry>, crate::problem::Problem> {
    read_servers(c)
}

/// Used by the import of the old `servers.json`: replace the list, or add what is not there yet.
pub(crate) fn write_servers_merging(c: &rusqlite::Connection, servers: &[ServerEntry], replace: bool) -> Result<(), crate::problem::Problem> {
    if replace {
        return write_servers(c, servers);
    }
    let have = read_servers(c)?;
    if have.is_empty() {
        return write_servers(c, servers);
    }
    let mut all = have;
    for e in servers {
        if !all.iter().any(|h| h.name == e.name) {
            all.push(e.clone());
        }
    }
    write_servers(c, &all)
}

fn write_servers(tx: &rusqlite::Connection, servers: &[ServerEntry]) -> Result<(), crate::problem::Problem> {
    // What each server's record held, so fields a NEWER Kora added (that this Kora's struct does not have) survive
    // this Kora saving the list.
    let mut kept: std::collections::HashMap<String, serde_json::Map<String, serde_json::Value>> = std::collections::HashMap::new();
    {
        let mut st = tx.prepare("SELECT name, record FROM servers")?;
        for row in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (name, record) = row?;
            if let Ok(serde_json::Value::Object(m)) = serde_json::from_str(&record) {
                kept.insert(name, m);
            }
        }
    }
    // Names are unique: the last of two equal names wins, like `upsert`.
    tx.execute("DELETE FROM servers", [])?;
    let mut st = tx.prepare("INSERT INTO servers (name, seq, dir, record) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(name) DO UPDATE SET seq = excluded.seq, dir = excluded.dir, record = excluded.record")?;
    for (i, e) in servers.iter().enumerate() {
        let mut rec = match serde_json::to_value(e) {
            Ok(serde_json::Value::Object(m)) => m,
            _ => return Err(crate::problem::Problem::generic("Kora could not save the server list.")),
        };
        if let Some(old) = kept.get(&e.name) {
            for (k, v) in old {
                rec.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
        st.execute(rusqlite::params![e.name, i as i64, e.dir.to_string_lossy(), serde_json::Value::Object(rec).to_string()])?;
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
    /// The old spelling, still written for older versions of Kora (see [`super::style`]).
    pub mode: Mode,
    /// Clone or fork. Missing in files made before 0.8: then it follows `mode`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    /// Released versions or the latest changes. Missing in files made before 0.8: then it follows `mode`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub versions: Option<Versions>,
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

impl ServerEntry {
    pub fn source(&self) -> Source {
        super::style(self.mode, self.source, self.versions).0
    }
    pub fn versions(&self) -> Versions {
        super::style(self.mode, self.source, self.versions).1
    }
}

impl ServerFile {
    pub fn source(&self) -> Source {
        super::style(self.mode, self.source, self.versions).0
    }
    pub fn versions(&self) -> Versions {
        super::style(self.mode, self.source, self.versions).1
    }
    pub fn load(dir: &Path) -> crate::problem::Res<Self> {
        serde_json::from_slice(&fs::read(server_file_path(dir))?)
    }
    pub fn save(&self) -> crate::problem::Res<()> {
        let p = server_file_path(&self.dir);
        let tmp = p.with_extension("json.tmp");
        // Both spellings are written; the old one follows the new (see `style`).
        let mut out = self.clone();
        if out.versions.is_some() {
            out.mode = super::compat_mode(out.versions());
        }
        fs::write(&tmp, serde_json::to_vec_pretty(&out)?)?;
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
    fn new_fields_need_no_migration() {
        // a record written by a NEWER Kora, with a field this struct does not have
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("servers.json");
        let e = ServerEntry { source: None, versions: None, name: "a".into(), dir: "/x".into(), mode: Mode::Production, ports: PortsRecord { game: 30120, txadmin: 40120 }, created_unix: 1 };
        Registry { servers: vec![e.clone()] }.save_to(&p).unwrap();
        let s = Store::open(d.path()).unwrap();
        s.write(|tx| {
            let mut v: serde_json::Value = serde_json::from_str(&tx.query_row("SELECT record FROM servers WHERE name = 'a'", [], |r| r.get::<_, String>(0))?).unwrap();
            v["source"] = serde_json::json!("fork");
            tx.execute("UPDATE servers SET record = ?1 WHERE name = 'a'", [v.to_string()])?;
            Ok(())
        })
        .unwrap();
        // this Kora reads it (ignoring the field), changes the list, and the field is still there afterwards
        Registry::update_at(&p, |r| {
            r.servers[0].created_unix = 2;
            Ok(())
        })
        .unwrap();
        let rec: serde_json::Value = serde_json::from_str(&s.read(|c| Ok(c.query_row("SELECT record FROM servers", [], |r| r.get::<_, String>(0))?)).unwrap()).unwrap();
        assert_eq!((rec["source"].as_str(), rec["created_unix"].as_u64()), (Some("fork"), Some(2)));
        // and a record written BEFORE a field existed still reads (the missing field is the serde default)
        s.write(|tx| {
            tx.execute("UPDATE servers SET record = '{\"name\":\"a\",\"dir\":\"/x\",\"mode\":\"production\",\"ports\":{\"game\":1,\"txadmin\":2},\"created_unix\":9}'", [])?;
            Ok(())
        })
        .unwrap();
        assert_eq!(Registry::load_from(&p).unwrap().servers[0].created_unix, 9);
    }

    #[test]
    fn test_binaries_are_told_from_the_program() {
        assert!(is_test_binary_name("api_contract-3abacca8181dc5e1") && is_test_binary_name("kora-996eb47857e055f1"));
        assert!(!is_test_binary_name("kora") && !is_test_binary_name("kora-cli") && !is_test_binary_name("kora-0123456789abcdeg"));
    }

    #[test]
    fn registry_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("sub/servers.json");
        let mut r = Registry::load_from(&p).unwrap();
        assert!(r.servers.is_empty());
        let e = ServerEntry { source: None, versions: None, name: "a".into(), dir: "/x".into(), mode: Mode::Production, ports: PortsRecord { game: 30120, txadmin: 40120 }, created_unix: 1 };
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
