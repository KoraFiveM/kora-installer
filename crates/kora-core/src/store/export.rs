//! The way back: write the store's contents as the JSON files Kora v0.7.x used.
//!
//! `kora store export-json` makes a folder holding `servers.json`, `secrets.json`, `profile.json`,
//! `last-install.json`, `panel.json`, `hub.json` and `paused-installs/<job>.json` in exactly the shapes
//! the older Kora reads, so going back to v0.7.x loses nothing that was changed since the upgrade (see
//! `docs/store.md`). Only parts that hold something are written. The folder holds saved licence keys
//! and token hashes: by default it is inside the private store folder, and files are `0600` wherever
//! they are written.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::hub::NodeConfig;
use crate::installer::paused::Stored;
use crate::installer::registry::Registry;
use crate::problem::{Action, Problem};

use super::{codes, Store};

fn failed(what: &str, e: impl std::fmt::Display) -> Problem {
    Problem::new(codes::STORE_UNREADABLE, format!("Kora could not write the old-format files ({what}). Nothing in the store was changed.")).with_details(e.to_string()).with_action(Action::CopyDetails)
}

/// Where `export-json` writes by default: `<kora home>/store/export-json/<unix time>/`.
pub fn default_dir(home: &Path) -> PathBuf {
    super::store_dir(home).join("export-json").join(crate::monitor::now_unix().to_string())
}

fn put(dir: &Path, rel: &str, value: &impl serde::Serialize, out: &mut Vec<PathBuf>) -> Result<(), Problem> {
    let path = dir.join(rel);
    let text = serde_json::to_vec_pretty(value).map_err(|e| failed(rel, e))?;
    crate::fsutil::write_no_follow(&path, &text, 0o600).map_err(|e| failed(rel, e))?;
    out.push(path);
    Ok(())
}

/// Write the old-format files into `dir` (created). Returns the files written.
pub fn export_json(s: &Store, dir: &Path) -> Result<Vec<PathBuf>, Problem> {
    crate::fsutil::create_dir_no_follow(dir).map_err(|e| failed("the folder", e))?;
    super::privacy::tighten_dir(dir);
    let mut out = Vec::new();
    let servers = s.read(crate::installer::registry::read_servers_pub)?;
    if !servers.is_empty() {
        put(dir, "servers.json", &Registry { servers }, &mut out)?;
    }
    let keys = s.read(crate::keystore::export_keys)?;
    // A key whose value is in a keychain this session cannot reach is not in the file: say which, so nothing is lost silently.
    let unreadable = s.read(crate::keystore::unreadable_keys)?;
    if !unreadable.is_empty() {
        let text: String = unreadable.iter().map(|(n, why)| format!("{n}: {why}\n")).collect();
        let path = dir.join("keys-not-exported.txt");
        crate::fsutil::write_no_follow(&path, format!("These saved licence keys could not be read from here, so they are NOT in secrets.json:\n{text}").as_bytes(), 0o600).map_err(|e| failed("keys-not-exported.txt", e))?;
        out.push(path);
    }
    if !keys.is_empty() {
        let doc = json!({"version": 1, "keys": keys.iter().map(|(n, k, a)| json!({"name": n, "key": k, "added": a})).collect::<Vec<_>>()});
        put(dir, "secrets.json", &doc, &mut out)?;
    }
    let profile = s.read(crate::profile::read_profile_pub)?;
    if profile != Default::default() {
        put(dir, "profile.json", &profile, &mut out)?;
    }
    let answers: Option<String> = s.read(|c| Ok(c.query_row("SELECT req_json FROM saved_answers WHERE id = 1", [], |r| r.get::<_, String>(0)).ok()))?;
    if let Some(a) = answers.and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
        put(dir, "last-install.json", &a, &mut out)?;
    }
    if let Some(panel) = s.read(panel_json)? {
        put(dir, "panel.json", &panel, &mut out)?;
    }
    let nodes: Vec<NodeConfig> = s.read(|c| {
        let mut st = c.prepare("SELECT id, name, url, token FROM hub_nodes ORDER BY seq")?;
        let v = st.query_map([], |r| Ok(NodeConfig { id: r.get(0)?, name: r.get(1)?, url: r.get(2)?, token: r.get(3)? }))?.collect::<Result<Vec<_>, _>>()?;
        Ok(v)
    })?;
    if !nodes.is_empty() {
        put(dir, "hub.json", &json!({"nodes": nodes}), &mut out)?;
    }
    for e in super::legacy::EXTRA_HOME_FILES {
        if let Some(doc) = s.read(|c| super::documents::read(c, e.document))? {
            put(dir, e.file, &doc, &mut out)?;
        }
    }
    let paused: Vec<Stored> = crate::installer::paused::PausedStore::list_in(s);
    for p in paused {
        put(dir, &format!("paused-installs/{}.json", p.job), &p, &mut out)?;
    }
    Ok(out)
}

fn panel_json(c: &rusqlite::Connection) -> Result<Option<Value>, Problem> {
    let flag = |k: &str| -> bool { c.query_row("SELECT value FROM remote_settings WHERE key = ?1", [k], |r| r.get::<_, String>(0)).is_ok_and(|v| v == "1") };
    let mut ts = serde_json::Map::new();
    for row in c.prepare("SELECT login, role FROM remote_tailscale_users ORDER BY login")?.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (l, r) = row?;
        ts.insert(l, Value::String(r));
    }
    let mut users = Vec::new();
    for row in c
        .prepare("SELECT name, role, salt, hash, created FROM remote_users ORDER BY seq")?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, i64>(4)?)))?
    {
        let (name, role, salt, hash, created) = row?;
        users.push(json!({"name": name, "role": role, "salt": salt, "hash": hash, "created": created}));
    }
    let hosts: Vec<String> = c.prepare("SELECT host FROM remote_hosts ORDER BY seq")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    let (enabled, tailscale, destructive) = (flag("enabled"), flag("tailscale"), flag("allow_remote_destructive"));
    if !(enabled || tailscale || destructive || !ts.is_empty() || !users.is_empty() || !hosts.is_empty()) {
        return Ok(None);
    }
    Ok(Some(json!({"enabled": enabled, "tailscale": tailscale, "tailscale_users": ts, "users": users, "allowed_hosts": hosts, "allow_remote_destructive": destructive})))
}
