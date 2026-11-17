//! Import of the old JSON state files (Kora up to v0.7.x) into the store.
//!
//! The files, all in the Kora home: `servers.json`, `secrets.json`, `profile.json`, `last-install.json`,
//! `panel.json`, `hub.json` and `paused-installs/<job>.json`. The v0.6.x shapes are the same as v0.7.0's
//! (`license_key_name` in the saved answers is the only field added, and it is optional), so one reader
//! covers both.
//!
//! # The rules
//! * ONE transaction imports every readable file, and the rows are read back and compared with what was
//!   parsed before it commits. Nothing is deleted: afterwards each file is MOVED to
//!   `<kora home>/store/before-sqlite/` (same names), which is also the way back (see `docs/store.md`).
//! * A file that cannot be read is never lost or replaced. A damaged key, server, remote-access or node
//!   file stays exactly where it was and the part of Kora that depends on it says so (`Store::blocked`)
//!   instead of starting empty; it is tried again every time Kora opens the store, so fixing the file is
//!   enough. A damaged profile, saved-answers or stopped-install file only saves typing: it is moved aside
//!   with the others and Kora carries on.
//! * Idempotent. Each imported file's SHA-256 is recorded in the same transaction. A file that is still
//!   there with the same hash (the process died between commit and move) is only moved. A file with a
//!   DIFFERENT hash (an older Kora ran again after a downgrade) is imported again, replacing that part
//!   of the store, after a backup of the store.
//! * Two programs starting at once cannot import twice: everything is read and decided inside the one
//!   write transaction, which the second waits for, and moving a file that is already gone is fine.
//! * Never through a link: a link where an old file should be is treated as unreadable.
//! * Secrets: error text carries positions, never values; the saved answers and stopped installs are
//!   cleaned of any licence key or database password an old version might have written.

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{codes, meta_get, meta_set, Blocked, Store};
use crate::installer::paused::Stored;
use crate::installer::registry::ServerEntry;
use crate::installer::InstallRequest;
use crate::problem::{Action, Problem};
use crate::profile::Profile;

/// Where moved files go.
pub const ASIDE_DIR: &str = "before-sqlite";

const SINGLE_FILES: [&str; 6] = ["servers.json", "secrets.json", "profile.json", "last-install.json", "panel.json", "hub.json"];
const PAUSED_DIR: &str = "paused-installs";

/// Is any old JSON state file (or the stopped-installs folder) in `home`?
pub fn any_legacy_files(home: &Path) -> bool {
    SINGLE_FILES.iter().chain([&PAUSED_DIR]).any(|n| std::fs::symlink_metadata(home.join(n)).is_ok())
}

pub fn aside_dir(home: &Path) -> PathBuf {
    super::store_dir(home).join(ASIDE_DIR)
}

// ---- what a file turned into -----------------------------------------------------------------------

/// A parsed old file.
enum Parsed {
    Servers(Vec<ServerEntry>),
    Keys(Vec<(String, String, u64)>),
    Profile(Profile),
    Answers(InstallRequest),
    Panel(Panel),
    Hub(Vec<(String, String, String, String)>),
    Paused(Box<Stored>),
}

/// `panel.json`, read loosely (its types live in kora-web).
#[derive(Debug, Default, PartialEq, Eq)]
struct Panel {
    enabled: bool,
    tailscale: bool,
    allow_remote_destructive: bool,
    tailscale_users: Vec<(String, String)>,
    /// name, role, salt, hash, created
    users: Vec<(String, String, String, String, u64)>,
    hosts: Vec<String>,
}

/// The plain-words part of a file problem (no values from the file).
struct Bad {
    why: String,
    details: String,
}

fn bad(why: &str, details: impl Into<String>) -> Bad {
    Bad { why: why.to_string(), details: details.into() }
}

fn position(e: &serde_json::Error) -> String {
    // serde's messages can quote the offending value (a key, a token): only the position is kept.
    format!("not valid at line {}, column {}", e.line(), e.column())
}

fn as_value(bytes: &[u8]) -> Result<Value, Bad> {
    let v = serde_json::from_slice::<Value>(bytes).map_err(|e| bad("the file is damaged", position(&e)))?;
    if !v.is_object() {
        return Err(bad("the file is not in the expected shape", "it is not a JSON object"));
    }
    Ok(v)
}

fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(String::from)
}

fn parse_servers(bytes: &[u8]) -> Result<Vec<ServerEntry>, Bad> {
    #[derive(serde::Deserialize)]
    struct Doc {
        #[serde(default)]
        servers: Vec<ServerEntry>,
    }
    serde_json::from_slice::<Doc>(bytes).map(|d| d.servers).map_err(|e| bad("the file is damaged", position(&e)))
}

fn parse_keys(bytes: &[u8]) -> Result<Vec<(String, String, u64)>, Bad> {
    let v = as_value(bytes)?;
    let version = v.get("version").and_then(Value::as_u64).unwrap_or(1);
    if version > 1 {
        return Err(bad("it was written by a newer Kora", format!("format version {version}, this Kora understands 1")));
    }
    let keys = match v.get("keys") {
        None | Some(Value::Null) => return Ok(vec![]),
        Some(Value::Array(a)) => a,
        Some(_) => return Err(bad("the file is not in the expected shape", "\"keys\" is not a list")),
    };
    let mut out = Vec::new();
    for (i, k) in keys.iter().enumerate() {
        let (Some(name), Some(key)) = (str_of(k, "name"), str_of(k, "key")) else {
            return Err(bad("the file is not in the expected shape", format!("entry {} has no name or key", i + 1)));
        };
        out.push((name, key, k.get("added").and_then(Value::as_u64).unwrap_or(0)));
    }
    Ok(out)
}

/// An old saved-answers/stopped-install request, without anything secret an old version might have written.
fn clean_request(mut r: InstallRequest) -> InstallRequest {
    r.license_key = None;
    r.catalog = None;
    r.dry_run = false;
    if let Some(a) = r.db_admin.as_mut() {
        a.password = None;
    }
    r
}

fn parse_answers(bytes: &[u8]) -> Result<InstallRequest, Bad> {
    serde_json::from_slice::<InstallRequest>(bytes).map(clean_request).map_err(|e| bad("the file is damaged", position(&e)))
}

fn parse_paused(bytes: &[u8]) -> Result<Stored, Bad> {
    let mut s = serde_json::from_slice::<Stored>(bytes).map_err(|e| bad("the file is damaged", position(&e)))?;
    s.req = clean_request(s.req);
    Ok(s)
}

fn parse_profile(bytes: &[u8]) -> Result<Profile, Bad> {
    serde_json::from_slice::<Profile>(bytes).map_err(|e| bad("the file is damaged", position(&e)))
}

fn parse_hub(bytes: &[u8]) -> Result<Vec<(String, String, String, String)>, Bad> {
    let v = as_value(bytes)?;
    let nodes = match v.get("nodes") {
        None | Some(Value::Null) => return Ok(vec![]),
        Some(Value::Array(a)) => a,
        Some(_) => return Err(bad("the file is not in the expected shape", "\"nodes\" is not a list")),
    };
    let mut out: Vec<(String, String, String, String)> = Vec::new();
    for (i, n) in nodes.iter().enumerate() {
        let (Some(id), Some(name), Some(url), Some(token)) = (str_of(n, "id"), str_of(n, "name"), str_of(n, "url"), str_of(n, "token")) else {
            return Err(bad("the file is not in the expected shape", format!("node {} is missing a field", i + 1)));
        };
        if out.iter().any(|o| o.0 == id) {
            continue; // an id repeated by hand: the first one wins
        }
        out.push((id, name, url, token));
    }
    Ok(out)
}

fn parse_panel(bytes: &[u8]) -> Result<Panel, Bad> {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(Panel::default());
    }
    let v = as_value(bytes)?;
    if !v.is_object() {
        return Err(bad("the file is not in the expected shape", "not an object"));
    }
    let role = |r: Option<&Value>, what: &str| -> Result<String, Bad> {
        match r.and_then(Value::as_str).map(str::to_ascii_lowercase) {
            Some(r) if matches!(r.as_str(), "viewer" | "operator" | "admin") => Ok(r),
            _ => Err(bad("the file is not in the expected shape", format!("{what} has an unknown role"))),
        }
    };
    let flag = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
    let mut p = Panel { enabled: flag("enabled"), tailscale: flag("tailscale"), allow_remote_destructive: flag("allow_remote_destructive"), ..Panel::default() };
    if let Some(m) = v.get("tailscale_users").and_then(Value::as_object) {
        for (login, r) in m {
            p.tailscale_users.push((login.clone(), role(Some(r), "a Tailscale user")?));
        }
    }
    if let Some(a) = v.get("users").and_then(Value::as_array) {
        for (i, u) in a.iter().enumerate() {
            let what = format!("user {}", i + 1);
            let (Some(name), Some(salt), Some(hash)) = (str_of(u, "name"), str_of(u, "salt"), str_of(u, "hash")) else {
                return Err(bad("the file is not in the expected shape", format!("{what} is missing a field")));
            };
            let mut name = name;
            // Names are unique without regard to case; a hand-edited duplicate keeps both, the second renamed.
            let mut n = 2;
            while p.users.iter().any(|x| x.0.to_lowercase() == name.to_lowercase()) {
                name = format!("{}-{n}", str_of(u, "name").unwrap_or_default());
                n += 1;
            }
            p.users.push((name, role(u.get("role"), &what)?, salt, hash, u.get("created").and_then(Value::as_u64).unwrap_or(0)));
        }
    }
    if let Some(a) = v.get("allowed_hosts").and_then(Value::as_array) {
        p.hosts = a.iter().filter_map(|h| h.as_str().map(String::from)).collect();
    }
    Ok(p)
}

// ---- reading a file safely -------------------------------------------------------------------------

fn hash_of(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// The bytes of an old file, never through a link and never anything that is not a plain file.
fn read_old(path: &Path) -> Result<Option<Vec<u8>>, Bad> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(bad("the file cannot be opened", e.to_string())),
        Ok(m) if m.file_type().is_symlink() => return Err(bad("it is a link, which Kora does not follow", format!("{} is a symbolic link", path.display()))),
        Ok(m) if !m.is_file() => return Err(bad("it is not a file", format!("{} is not a plain file", path.display()))),
        Ok(m) if m.len() > 64 << 20 => return Err(bad("it is far too big to be one of Kora's files", format!("{} bytes", m.len()))),
        Ok(_) => {}
    }
    let mut f = crate::fsutil::open_no_follow(path, crate::fsutil::Access::Read, None).map_err(|e| bad("the file cannot be opened", e.to_string()))?;
    let mut b = Vec::new();
    std::io::Read::read_to_end(&mut f, &mut b).map_err(|e| bad("the file cannot be read", e.to_string()))?;
    Ok(Some(b))
}

fn domain_of(name: &str) -> Option<&'static str> {
    match name {
        "servers.json" => Some("servers"),
        "secrets.json" => Some("keys"),
        "panel.json" => Some("remote"),
        "hub.json" => Some("hub"),
        _ => None, // profile, saved answers, stopped installs only save typing
    }
}

// ---- the work --------------------------------------------------------------------------------------

/// One old file and what happened to it in this pass.
struct Item {
    /// Name relative to the Kora home (`servers.json`, `paused-installs/<job>.json`).
    rel: String,
    path: PathBuf,
    hash: String,
    parsed: Parsed,
    /// Already imported (its hash is recorded): only needs moving aside.
    done: bool,
    /// Was imported before with different content: replace.
    replace: bool,
}

#[derive(Default)]
struct Scan {
    items: Vec<Item>,
    /// Files that could not be parsed or read: (rel, path, why).
    bad: Vec<(String, PathBuf, Bad)>,
}

fn marker(rel: &str) -> String {
    format!("legacy.{rel}")
}

fn scan(c: &Connection, home: &Path) -> Result<Scan, Problem> {
    let mut out = Scan::default();
    let add = |rel: String, path: PathBuf, bytes: Vec<u8>, parse: &dyn Fn(&[u8]) -> Result<Parsed, Bad>, c: &Connection, out: &mut Scan| -> Result<(), Problem> {
        let hash = hash_of(&bytes);
        let seen = meta_get(c, &marker(&rel))?;
        match parse(&bytes) {
            Ok(parsed) => out.items.push(Item { done: seen.as_deref() == Some(hash.as_str()), replace: seen.is_some() && seen.as_deref() != Some(hash.as_str()), rel, path, hash, parsed }),
            Err(b) => out.bad.push((rel, path, b)),
        }
        Ok(())
    };
    for name in SINGLE_FILES {
        let path = home.join(name);
        match read_old(&path) {
            Ok(None) => {}
            Ok(Some(bytes)) => {
                let parse: &dyn Fn(&[u8]) -> Result<Parsed, Bad> = match name {
                    "servers.json" => &|b| parse_servers(b).map(Parsed::Servers),
                    "secrets.json" => &|b| parse_keys(b).map(Parsed::Keys),
                    "profile.json" => &|b| parse_profile(b).map(Parsed::Profile),
                    "last-install.json" => &|b| parse_answers(b).map(Parsed::Answers),
                    "panel.json" => &|b| parse_panel(b).map(Parsed::Panel),
                    _ => &|b| parse_hub(b).map(Parsed::Hub),
                };
                add(name.to_string(), path, bytes, parse, c, &mut out)?;
            }
            Err(b) => out.bad.push((name.to_string(), path, b)),
        }
    }
    let dir = home.join(PAUSED_DIR);
    if let Ok(md) = std::fs::symlink_metadata(&dir) {
        if md.file_type().is_symlink() || !md.is_dir() {
            out.bad.push((PAUSED_DIR.to_string(), dir.clone(), bad("it is not a plain folder", dir.display().to_string())));
        } else if let Ok(rd) = std::fs::read_dir(&dir) {
            let mut names: Vec<String> = rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
            names.sort();
            for n in names {
                let rel = format!("{PAUSED_DIR}/{n}");
                let path = dir.join(&n);
                match read_old(&path) {
                    Ok(Some(bytes)) => add(rel, path, bytes, &|b| parse_paused(b).map(|s| Parsed::Paused(Box::new(s))), c, &mut out)?,
                    Ok(None) => {}
                    Err(b) => out.bad.push((rel, path, b)),
                }
            }
        }
    }
    Ok(out)
}

/// Run at every open: does nothing unless an old file is there.
pub fn import_if_needed(s: &mut Store) -> Result<(), Problem> {
    s.blocked.clear();
    let home = s.home().to_path_buf();
    if !any_legacy_files(&home) {
        return Ok(());
    }
    // A pass that will REPLACE already-imported data (an older Kora ran again): a backup first. (VACUUM
    // cannot run inside the transaction, so this is decided from a look before it.)
    let peek = s.read(|c| scan(c, &home))?;
    if peek.items.iter().any(|i| i.replace) {
        super::backup::backup_before_import(s)?;
    }
    let mut applied: Vec<(String, PathBuf)> = Vec::new();
    let done = s.write(|tx| {
        // Decided again under the write lock: another process may have done it while we waited.
        let sc = scan(tx, &home)?;
        for it in sc.items.iter().filter(|i| !i.done) {
            apply(tx, it)?;
            verify(tx, it)?;
            meta_set(tx, &marker(&it.rel), &it.hash)?;
        }
        Ok(sc)
    });
    let sc = done?;
    for it in &sc.items {
        applied.push((it.rel.clone(), it.path.clone()));
    }
    // Committed: now move the files aside (a crash from here on only means "move them next time").
    for (rel, path) in &applied {
        move_aside(&home, rel, path);
    }
    let mut blocked = Vec::new();
    for (rel, path, b) in sc.bad {
        match domain_of(&rel) {
            Some(domain) => blocked.push(Blocked { domain, why: b.why, details: b.details, file: path }),
            None => {
                // Only saves typing: kept (moved aside), not imported.
                move_aside(&home, &rel, &path);
            }
        }
    }
    super::privacy::fix_owner(&s.path());
    s.blocked = blocked;
    // The folder of stopped installs is empty now.
    let _ = std::fs::remove_dir(home.join(PAUSED_DIR));
    Ok(())
}

/// Move an old file into the `before-sqlite` folder, never overwriting an earlier one there.
fn move_aside(home: &Path, rel: &str, path: &Path) {
    let aside = aside_dir(home);
    let dest_rel = Path::new(rel);
    let mut dest = aside.join(dest_rel);
    if let Some(parent) = dest.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
        super::privacy::tighten_dir(parent);
    }
    if dest.exists() {
        let stamp = crate::monitor::now_unix();
        let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        dest.set_file_name(format!("{name}.{stamp}"));
    }
    // `rename` (same volume): the file keeps its owner and permissions. A missing file is fine: another
    // Kora process got there first.
    let _ = crate::fsutil::rename_retrying(path, &dest);
}

fn apply(c: &Connection, it: &Item) -> Result<(), Problem> {
    match &it.parsed {
        Parsed::Servers(v) => crate::installer::registry::write_servers_merging(c, v, it.replace),
        Parsed::Keys(v) => crate::keystore::import_keys(c, v, it.replace),
        Parsed::Profile(p) => crate::profile::import_profile(c, p, it.replace),
        Parsed::Answers(r) => {
            let text = serde_json::to_string(r).map_err(|e| Problem::generic(e.to_string()))?;
            // Existing newer answers win unless this is a replacement.
            let sql = if it.replace {
                "INSERT INTO saved_answers (id, req_json, saved_at) VALUES (1, ?1, ?2) ON CONFLICT(id) DO UPDATE SET req_json = excluded.req_json, saved_at = excluded.saved_at"
            } else {
                "INSERT OR IGNORE INTO saved_answers (id, req_json, saved_at) VALUES (1, ?1, ?2)"
            };
            c.execute(sql, rusqlite::params![text, file_time(&it.path)])?;
            Ok(())
        }
        Parsed::Panel(p) => import_panel(c, p, it.replace),
        Parsed::Hub(nodes) => {
            #[cfg(test)]
            if TEST_FAIL_HOME.lock().unwrap().as_deref() == it.path.parent() {
                return Err(Problem::generic("test: the import fails half-way"));
            }
            if it.replace {
                c.execute("DELETE FROM hub_nodes", [])?;
            }
            for (id, name, url, token) in nodes {
                c.execute("INSERT OR IGNORE INTO hub_nodes (id, name, url, token) VALUES (?1, ?2, ?3, ?4)", [id, name, url, token])?;
            }
            Ok(())
        }
        Parsed::Paused(st) => {
            if crate::monitor::now_unix().saturating_sub(st.saved_at) > crate::installer::paused::KEEP_SECS {
                return Ok(()); // too old to carry on: kept aside only
            }
            let req = serde_json::to_string(&st.req).map_err(|e| Problem::generic(e.to_string()))?;
            let fin = serde_json::to_string(&st.finished).map_err(|e| Problem::generic(e.to_string()))?;
            c.execute(
                "INSERT INTO paused_installs (job, dir, via_helper, failed_step, req_json, finished_json, saved_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(job) DO UPDATE SET dir = excluded.dir, via_helper = excluded.via_helper, failed_step = excluded.failed_step, req_json = excluded.req_json, finished_json = excluded.finished_json, saved_at = excluded.saved_at",
                rusqlite::params![st.job, st.req.dir.to_string_lossy(), st.via_helper, st.failed_step, req, fin, st.saved_at as i64],
            )?;
            Ok(())
        }
    }
}

fn file_time(path: &Path) -> i64 {
    std::fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64)
}

fn import_panel(c: &Connection, p: &Panel, replace: bool) -> Result<(), Problem> {
    if replace {
        for t in ["remote_tailscale_users", "remote_users", "remote_hosts", "remote_settings"] {
            c.execute(&format!("DELETE FROM {t}"), [])?;
        }
    }
    for (k, v) in [("enabled", p.enabled), ("tailscale", p.tailscale), ("allow_remote_destructive", p.allow_remote_destructive)] {
        c.execute("INSERT OR IGNORE INTO remote_settings (key, value) VALUES (?1, ?2)", rusqlite::params![k, if v { "1" } else { "0" }])?;
    }
    for (l, r) in &p.tailscale_users {
        c.execute("INSERT OR IGNORE INTO remote_tailscale_users (login, role) VALUES (?1, ?2)", [l, r])?;
    }
    for (name, role, salt, hash, created) in &p.users {
        c.execute(
            "INSERT OR IGNORE INTO remote_users (name, name_folded, role, salt, hash, created) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![name, name.to_lowercase(), role, salt, hash, *created as i64],
        )?;
    }
    for h in &p.hosts {
        if c.query_row("SELECT COUNT(*) FROM remote_hosts WHERE host = ?1", [h], |r| r.get::<_, i64>(0))? == 0 {
            c.execute("INSERT INTO remote_hosts (host) VALUES (?1)", [h])?;
        }
    }
    // The running panel notices a new version of the settings.
    let rev: i64 = meta_get(c, "remote_rev")?.and_then(|v| v.parse().ok()).unwrap_or(0);
    meta_set(c, "remote_rev", &(rev + 1).to_string())
}

/// Read back what was written and compare it with what was parsed. A difference aborts the whole import.
fn verify(c: &Connection, it: &Item) -> Result<(), Problem> {
    let wrong = |what: &str| {
        Problem::new(codes::STORE_IMPORT, format!("Kora could not check that its old {} was copied correctly, so nothing was changed and the old file is untouched.", it.rel))
            .with_details(what.to_string())
            .with_action(Action::CopyDetails)
    };
    let count = |sql: &str| -> Result<usize, Problem> { Ok(c.query_row(sql, [], |r| r.get::<_, i64>(0))? as usize) };
    match &it.parsed {
        Parsed::Servers(v) => {
            let have = crate::installer::registry::read_servers_pub(c)?;
            for e in v {
                // Same name twice in an old file: the last one is the one that counts, like `upsert`.
                let last = v.iter().rev().find(|x| x.name == e.name).unwrap_or(e);
                match have.iter().find(|h| h.name == e.name) {
                    // Merging into existing servers keeps the existing entry; a first import or a replacement is exact.
                    Some(h) if h == last || !it.replace && have.len() > v.len() => {}
                    _ => return Err(wrong("a server entry differs after copying")),
                }
            }
        }
        Parsed::Keys(v) => {
            let have = crate::keystore::export_keys(c)?;
            for (_, key, _) in v {
                if !have.iter().any(|(_, k, _)| k == key) {
                    return Err(wrong("a saved key is missing after copying"));
                }
            }
            if have.len() < v.len() {
                return Err(wrong("fewer keys than before"));
            }
        }
        Parsed::Panel(p) => {
            if count("SELECT COUNT(*) FROM remote_users")? < p.users.len() || count("SELECT COUNT(*) FROM remote_tailscale_users")? < p.tailscale_users.len() {
                return Err(wrong("fewer remote users than before"));
            }
            for u in &p.users {
                let ok: i64 = c.query_row("SELECT COUNT(*) FROM remote_users WHERE name = ?1 AND hash = ?2 AND salt = ?3", [&u.0, &u.3, &u.2], |r| r.get(0))?;
                if ok == 0 {
                    return Err(wrong("a remote user differs after copying"));
                }
            }
        }
        Parsed::Hub(nodes) => {
            for (id, _, _, token) in nodes {
                let ok: i64 = c.query_row("SELECT COUNT(*) FROM hub_nodes WHERE id = ?1 AND token = ?2", [id, token], |r| r.get(0))?;
                if ok == 0 {
                    return Err(wrong("a node differs after copying"));
                }
            }
        }
        Parsed::Profile(p) => {
            let have = crate::profile::read_profile_pub(c)?;
            if it.replace && have != *p {
                return Err(wrong("the profile differs after copying"));
            }
        }
        Parsed::Answers(_) => {
            if count("SELECT COUNT(*) FROM saved_answers")? != 1 {
                return Err(wrong("the saved answers are missing"));
            }
        }
        Parsed::Paused(st) => {
            if crate::monitor::now_unix().saturating_sub(st.saved_at) <= crate::installer::paused::KEEP_SECS {
                let n: i64 = c.query_row("SELECT COUNT(*) FROM paused_installs WHERE job = ?1", [&st.job], |r| r.get(0))?;
                if n != 1 {
                    return Err(wrong("a stopped install is missing"));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
/// The Kora home whose import fails at the hub step (so tests running side by side do not disturb each other).
pub(crate) static TEST_FAIL_HOME: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

#[cfg(test)]
mod tests;
