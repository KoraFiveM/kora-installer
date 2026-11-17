//! Saved Cfx.re licence keys, picked by name. They live in the Kora home's private database
//! ([`crate::store`]; `<kora home>/secrets.json` before v0.8, which the store imports once).
//!
//! Not encrypted (like the per-server `secrets.cfg`), but as private as the file was: the database folder is
//! `0700` and its files `0600` from creation on Linux/macOS, and on Windows the inherited permissions are
//! removed from the folder ([`crate::store::privacy`]). The key VALUES are in their own table behind the
//! [`KeyMaterial`] trait ([`material()`] is the one place that picks the implementation), so that a later
//! version can keep them in the system keychain instead.
//!
//! The rules the rest of Kora relies on:
//! * Everything that leaves this module for a screen, an API answer, a log or an event is a [`KeyInfo`]:
//!   the name and the LAST FOUR characters. The full key only comes out of [`KeyStore::get`], which is
//!   for writing the server's own config and nothing else.
//! * Every change is ONE database transaction that reads, checks and writes together, so the panel and a
//!   `kora keys` command running at the same moment cannot overwrite each other's change (the old file
//!   could not promise that).
//! * A store that cannot be read (or an old `secrets.json` that cannot be imported) is never replaced: every
//!   call says so (a [`Problem`]) and leaves it alone, so keys are never silently lost.
//!
//! Key names: 1 to 40 characters, no control characters, compared without regard to case ("main" and
//! "Main" are the same name). The key itself is checked with the installer's own rule
//! ([`crate::txadmin::license_key_ok`]).

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::installer::registry::{kora_home, ServerEntry};
use crate::problem::{codes, Action, Problem};
use crate::store::keychain::{self, Chosen, Place};
use crate::store::material::KeyMaterial;
use crate::store::Store;

/// The old file in the Kora home. Its name still says which Kora home a [`KeyStore::at`] path means.
pub const FILE_NAME: &str = "secrets.json";
pub const MAX_NAME: usize = 40;

/// Where the key VALUES live: per key, the private table, the system keychain or Windows' protected storage (see
/// [`crate::store::keychain`]).
fn material() -> Chosen {
    Chosen::new()
}

#[derive(Clone)]
struct Entry {
    /// Row id; 0 for a key that is not saved yet.
    id: i64,
    name: String,
    key: String,
    added: u64,
    /// Where the value is kept.
    place: Place,
    /// Why the value cannot be read now (it was in a place this session cannot reach or that lost it), in words naming the key.
    /// The key is then empty here and is never written back as empty.
    lost: Option<String>,
}

impl std::fmt::Debug for Entry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Entry").field("name", &self.name).field("key", &"<redacted>").finish()
    }
}

/// What may be shown about a saved key: never more than its name and last four characters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KeyInfo {
    pub name: String,
    pub last4: String,
    /// Unix seconds when it was saved (0 = unknown).
    pub added: u64,
    /// Where the value is kept, in words ("Kora's private file", "your system keychain").
    pub kept: String,
    /// The value cannot be read from here right now: why, and what to do (the key is never shown as empty or lost silently).
    pub problem: Option<String>,
}

/// The last four characters of `key` (all of it when it is shorter, which a valid key never is).
pub fn last4(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    chars[chars.len().saturating_sub(4)..].iter().collect()
}

fn info(e: &Entry) -> KeyInfo {
    KeyInfo { name: e.name.clone(), last4: last4(&e.key), added: e.added, kept: e.place.label().to_string(), problem: e.lost.clone() }
}

/// Trim and check a key name.
pub fn check_name(name: &str) -> Result<String, Problem> {
    let n = name.trim();
    let bad = |why: &str| Problem::new(codes::KEYS_INVALID, why).with_action(Action::change_answers("licence"));
    if n.is_empty() {
        return Err(bad("Give the key a name you will recognise, for example \"Main\" or \"Test server\"."));
    }
    if n.chars().count() > MAX_NAME {
        return Err(bad("That name is too long. Use up to 40 characters."));
    }
    if n.chars().any(char::is_control) {
        return Err(bad("A key name can only have normal letters, numbers, spaces and punctuation."));
    }
    Ok(n.to_string())
}

/// Trim and check a licence key. The message never repeats the key.
pub fn check_key(key: &str) -> Result<String, Problem> {
    let k = key.trim();
    if k.is_empty() {
        return Err(Problem::new(codes::KEYS_INVALID, "Paste the licence key first. You can get one at https://portal.cfx.re/servers/registration-keys.")
            .with_action(Action::open_link(crate::installer::LICENCE_URL)));
    }
    if !crate::txadmin::license_key_ok(k) {
        return Err(Problem::new(
            codes::KEYS_INVALID,
            "That does not look like a Cfx.re licence key. It is 8 to 64 letters, numbers, dashes or underscores, with no spaces. Copy it again from portal.cfx.re.",
        )
        .with_action(Action::open_link(crate::installer::LICENCE_URL)));
    }
    Ok(k.to_string())
}

fn same(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// The saved keys of one Kora home. [`KeyStore::default`] is the one of this process; tests use [`KeyStore::at`].
#[derive(Debug, Clone)]
pub struct KeyStore {
    /// `<kora home>/secrets.json`: only names the Kora home (see the module docs).
    legacy: PathBuf,
    /// The database file the keys are in.
    path: PathBuf,
}

impl Default for KeyStore {
    fn default() -> Self {
        Self::at(kora_home().join(FILE_NAME))
    }
}

fn load_entries(c: &rusqlite::Connection) -> Result<Vec<Entry>, Problem> {
    let mut st = c.prepare("SELECT id, name, added FROM license_keys ORDER BY id")?;
    let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))?.collect::<Result<Vec<_>, _>>()?;
    let mut out = Vec::with_capacity(rows.len());
    for (id, name, added) in rows {
        let (place, _) = keychain::location(c, id)?;
        // A value that cannot be read (kept where this session cannot reach, or gone from there) leaves the key empty and says
        // why; it is never an error for the whole list, and never written back as empty.
        let (key, lost) = match material().get(c, id) {
            Ok(v) => (v.unwrap_or_default(), None),
            Err(p) if keychain::is_unavailable(&p) => (String::new(), Some(p.message.replace("{key}", &format!("\"{name}\"")))),
            Err(e) => return Err(e),
        };
        out.push(Entry { id, name, key, added: added as u64, place, lost });
    }
    Ok(out)
}

/// Write the difference between `before` and `after` (inside the caller's transaction).
fn persist(c: &rusqlite::Connection, before: &[Entry], after: &[Entry]) -> Result<(), Problem> {
    for b in before {
        if !after.iter().any(|a| a.id == b.id) {
            material().delete(c, b.id)?;
            c.execute("DELETE FROM license_keys WHERE id = ?1", [b.id])?;
        }
    }
    for a in after {
        if a.id == 0 {
            c.execute("INSERT INTO license_keys (name, name_folded, added) VALUES (?1, ?2, ?3)", rusqlite::params![a.name, a.name.to_lowercase(), a.added as i64])?;
            material().put(c, c.last_insert_rowid(), &a.key)?;
        } else if let Some(b) = before.iter().find(|b| b.id == a.id) {
            if b.name != a.name {
                c.execute("UPDATE license_keys SET name = ?1, name_folded = ?2 WHERE id = ?3", rusqlite::params![a.name, a.name.to_lowercase(), a.id])?;
            }
            if b.key != a.key {
                material().put(c, a.id, &a.key)?;
            }
        }
    }
    Ok(())
}

/// Every saved key as (name, key, added): what the import of the old file verifies against.
pub(crate) fn export_keys(c: &rusqlite::Connection) -> Result<Vec<(String, String, u64)>, Problem> {
    Ok(load_entries(c)?.into_iter().filter(|e| e.lost.is_none()).map(|e| (e.name, e.key, e.added)).collect())
}

/// Keys whose value cannot be read from here (so an export cannot carry them): name and why.
pub(crate) fn unreadable_keys(c: &rusqlite::Connection) -> Result<Vec<(String, String)>, Problem> {
    Ok(load_entries(c)?.into_iter().filter_map(|e| e.lost.map(|why| (e.name, why))).collect())
}

/// Used by the import of the old `secrets.json`: replace the saved keys, or add the ones that are not there.
/// A name that is already taken by a DIFFERENT key gets " (2)", " (3)" ... so no key is ever dropped.
pub(crate) fn import_keys(c: &rusqlite::Connection, keys: &[(String, String, u64)], replace: bool) -> Result<(), Problem> {
    let before = load_entries(c)?;
    let mut after: Vec<Entry> = if replace { Vec::new() } else { before.clone() };
    for (name, key, added) in keys {
        if after.iter().any(|e| same(&e.name, name) && e.key == *key) {
            continue;
        }
        let mut unique = name.clone();
        let mut n = 2;
        while after.iter().any(|e| same(&e.name, &unique)) {
            unique = format!("{name} ({n})");
            n += 1;
        }
        after.push(Entry { id: 0, name: unique, key: key.clone(), added: *added, place: Place::Store, lost: None });
    }
    persist(c, &before, &after)
}

impl KeyStore {
    pub fn at(path: impl Into<PathBuf>) -> Self {
        let legacy = path.into();
        let path = crate::store::db_path(legacy.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new(".")));
        Self { legacy, path }
    }

    /// Where the keys are saved: the store's database file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn unreadable(&self, why: &str, details: String) -> Problem {
        Problem::new(
            codes::KEYS_UNREADABLE,
            format!(
                "Kora cannot read your saved licence keys ({why}). Nothing was changed and no key was lost: the file is still where it was. Fix or move the file {} and try again.",
                self.legacy.display()
            ),
        )
        .with_details(details)
        .with_action(Action::CopyDetails)
    }

    /// A store problem about the keys is told as the keys problem it is.
    fn blocked(&self, s: &Store) -> Result<(), Problem> {
        match s.blocked("keys") {
            Some(why) => Err(self.unreadable(&why.why, why.details.clone())),
            None => Ok(()),
        }
    }

    /// Read-only: `None` when there is no store yet (nothing saved).
    fn open_read(&self) -> Result<Option<Store>, Problem> {
        match Store::open_existing_beside(&self.legacy)? {
            Some(s) => {
                self.blocked(&s)?;
                Ok(Some(s))
            }
            None => Ok(None),
        }
    }

    fn read(&self) -> Result<Vec<Entry>, Problem> {
        match self.open_read()? {
            Some(s) => s.read(load_entries),
            None => Ok(vec![]),
        }
    }

    /// Run `f` on the keys inside one transaction; the database is written only when `f` says it changed something.
    fn change<T>(&self, f: impl FnOnce(&mut Vec<Entry>) -> Result<(T, bool), Problem>) -> Result<T, Problem> {
        let s = Store::open_beside(&self.legacy)?;
        self.blocked(&s)?;
        let out = s.write(|tx| {
            let before = load_entries(tx)?;
            let mut keys = before.clone();
            let (out, changed) = f(&mut keys)?;
            if changed {
                persist(tx, &before, &keys)?;
            }
            Ok(out)
        })?;
        // After the commit: keychain items no key points to any more (a removed key, a rolled-back write) are deleted.
        keychain::sweep(&s);
        Ok(out)
    }

    /// Where the keys are kept and what happens to new ones (Settings, `kora keys keychain status`).
    pub fn keychain_status(&self) -> Result<keychain::Status, Problem> {
        match self.open_read()? {
            Some(s) => keychain::status(&s),
            None => keychain::status(&Store::open_beside(&self.legacy)?),
        }
    }

    /// Turn the keychain on (when there is one) and move the saved keys in.
    pub fn keychain_on(&self) -> Result<keychain::Moved, Problem> {
        let s = Store::open_beside(&self.legacy)?;
        self.blocked(&s)?;
        keychain::migrate_in(&s)
    }

    /// Turn the keychain off and move every key back into Kora's private file.
    pub fn keychain_off(&self) -> Result<keychain::Moved, Problem> {
        let s = Store::open_beside(&self.legacy)?;
        self.blocked(&s)?;
        keychain::migrate_out(&s)
    }

    /// Like [`change`](Self::change) for something that needs a key to exist already (rename, replace, forget): with no
    /// store yet there is no such key, and nothing is created to say so.
    fn change_existing<T>(&self, name: &str, f: impl FnOnce(&mut Vec<Entry>) -> Result<(T, bool), Problem>) -> Result<T, Problem> {
        if self.open_read()?.is_none() {
            return Err(not_found(name));
        }
        self.change(f)
    }

    /// The saved keys, safe to show.
    pub fn list(&self) -> Result<Vec<KeyInfo>, Problem> {
        Ok(self.read()?.iter().map(info).collect())
    }

    /// Does a key with this name exist (no key is read out)?
    pub fn exists(&self, name: &str) -> Result<bool, Problem> {
        Ok(self.list()?.iter().any(|k| same(&k.name, name)))
    }

    /// The FULL key. Only for writing the server's own configuration or starting an install: never
    /// put the result in an answer, a log, an event or an error.
    pub fn get(&self, name: &str) -> Result<String, Problem> {
        let e = self.read()?.into_iter().find(|e| same(&e.name, name)).ok_or_else(|| not_found(name))?;
        match e.lost {
            // Never an empty key handed to an install: a guided problem naming the key.
            Some(why) => Err(Problem::new(codes::KEYS_VALUE_MISSING, why).with_action(Action::change_answers("licence"))),
            None => Ok(e.key),
        }
    }

    /// The name a key value is already saved under, if any.
    pub fn name_of(&self, key: &str) -> Result<Option<String>, Problem> {
        let k = key.trim();
        Ok(self.read()?.into_iter().find(|e| e.key == k).map(|e| e.name))
    }

    /// Save a new key.
    pub fn add(&self, name: &str, key: &str) -> Result<KeyInfo, Problem> {
        let (name, key) = (check_name(name)?, check_key(key)?);
        self.change(|keys| {
            if let Some(e) = keys.iter().find(|e| same(&e.name, &name)) {
                return Err(name_taken(&e.name));
            }
            if let Some(e) = keys.iter().find(|e| e.key == key) {
                return Err(duplicate(&e.name));
            }
            let e = Entry { id: 0, name, key, added: crate::monitor::now_unix(), place: Place::Store, lost: None };
            let i = info(&e);
            keys.push(e);
            Ok((i, true))
        })
    }

    /// Give a saved key another name.
    pub fn rename(&self, old: &str, new: &str) -> Result<KeyInfo, Problem> {
        self.update(old, Some(new), None)
    }

    /// Put a different key under an existing name (servers already installed keep the key they have).
    pub fn replace(&self, name: &str, key: &str) -> Result<KeyInfo, Problem> {
        self.update(name, None, Some(key))
    }

    /// Rename and/or replace in one step: either both happen or neither.
    pub fn update(&self, name: &str, new_name: Option<&str>, new_key: Option<&str>) -> Result<KeyInfo, Problem> {
        let new_name = new_name.map(check_name).transpose()?;
        let new_key = new_key.map(check_key).transpose()?;
        self.change_existing(name, |keys| {
            let Some(i) = keys.iter().position(|e| same(&e.name, name)) else { return Err(not_found(name)) };
            if let Some(n) = &new_name {
                if keys.iter().enumerate().any(|(j, e)| j != i && same(&e.name, n)) {
                    return Err(name_taken(n));
                }
            }
            if let Some(k) = &new_key {
                if let Some((_, e)) = keys.iter().enumerate().find(|(j, e)| *j != i && e.key == *k) {
                    return Err(duplicate(&e.name));
                }
            }
            let changed = new_name.as_deref().is_some_and(|n| n != keys[i].name) || new_key.as_deref().is_some_and(|k| k != keys[i].key);
            if let Some(n) = new_name {
                keys[i].name = n;
            }
            if let Some(k) = new_key {
                keys[i].key = k;
            }
            Ok((info(&keys[i]), changed))
        })
    }

    /// Every saved key (safe to show) with the servers whose configuration holds exactly that key.
    pub fn list_with_servers(&self, servers: &[ServerEntry]) -> Result<Vec<(KeyInfo, Vec<String>)>, Problem> {
        Ok(self.read()?.iter().map(|e| (info(e), servers_using(&e.key, servers))).collect())
    }

    /// Forget a saved key. Servers installed with it are not touched.
    pub fn remove(&self, name: &str) -> Result<(), Problem> {
        self.change_existing(name, |keys| {
            let Some(i) = keys.iter().position(|e| same(&e.name, name)) else { return Err(not_found(name)) };
            keys.remove(i);
            Ok(((), true))
        })
    }
}

fn not_found(name: &str) -> Problem {
    Problem::new(codes::KEYS_NOT_FOUND, format!("There is no saved licence key called \"{name}\" (it may have been renamed or deleted). Pick another saved key or add the key again."))
        .with_action(Action::change_answers("licence"))
}

fn name_taken(name: &str) -> Problem {
    Problem::new(codes::KEYS_NAME_TAKEN, format!("A saved key is already called \"{name}\". Choose another name.")).with_action(Action::change_answers("licence"))
}

fn duplicate(name: &str) -> Problem {
    Problem::new(codes::KEYS_DUPLICATE, format!("That key is already saved as \"{name}\". Pick it from the list instead of adding it again.")).with_action(Action::change_answers("licence"))
}

// ---- the key inside a server's own configuration ----------------------------------------------

/// The config files that can hold `sv_licenseKey`, in the order Kora looks (the installer writes `secrets.cfg`).
const SERVER_FILES: [&str; 2] = ["secrets.cfg", "server.cfg"];

/// The value of an `sv_licenseKey` line (`sv_licenseKey "x"`, `sv_licenseKey x`, optionally after `set`).
/// `None` for any other line (comments included).
pub fn parse_key_line(line: &str) -> Option<String> {
    let t = line.trim();
    if t.starts_with('#') || t.starts_with("//") {
        return None;
    }
    let mut parts = t.splitn(2, char::is_whitespace);
    let mut first = parts.next()?;
    let mut rest = parts.next()?;
    if first.eq_ignore_ascii_case("set") {
        let mut p = rest.trim_start().splitn(2, char::is_whitespace);
        first = p.next()?;
        rest = p.next()?;
    }
    if !first.eq_ignore_ascii_case("sv_licenseKey") {
        return None;
    }
    let v = rest.trim();
    // A trailing `# comment` after a bare value is not part of the key.
    let v = if let Some(r) = v.strip_prefix('"') { r.split('"').next().unwrap_or("") } else { v.split_whitespace().next().unwrap_or("") };
    (!v.is_empty()).then(|| v.to_string())
}

/// `text` with the first `sv_licenseKey` line holding `key` (quoted, same layout as the installer's).
/// `None` when there is no such line.
pub fn replace_key_line(text: &str, key: &str) -> Option<String> {
    let mut done = false;
    let mut out = String::with_capacity(text.len() + 8);
    for line in text.split_inclusive('\n') {
        if !done && parse_key_line(line).is_some() {
            done = true;
            let eol = if line.ends_with("\r\n") {
                "\r\n"
            } else if line.ends_with('\n') {
                "\n"
            } else {
                ""
            };
            let t = line.trim_start();
            let indent = &line[..line.len() - t.len()];
            let set = if t.get(..4).is_some_and(|p| p.eq_ignore_ascii_case("set ")) { "set " } else { "" };
            out.push_str(&format!("{indent}{set}sv_licenseKey \"{key}\"{eol}"));
        } else {
            out.push_str(line);
        }
    }
    done.then_some(out)
}

/// The key a server's configuration holds, if it can be read (used to say which saved key a server uses).
pub fn server_key(dir: &Path) -> Option<String> {
    SERVER_FILES.iter().find_map(|f| {
        let text = std::fs::read_to_string(dir.join(f)).ok()?;
        text.lines().find_map(parse_key_line)
    })
}

/// Which servers (by name) hold exactly this key.
pub fn servers_using(key: &str, servers: &[ServerEntry]) -> Vec<String> {
    servers.iter().filter(|s| server_key(&s.dir).as_deref() == Some(key)).map(|s| s.name.clone()).collect()
}

fn config_problem(what: &str, details: String) -> Problem {
    Problem::new(codes::KEYS_SERVER_CONFIG, what.to_string()).with_details(details).with_action(Action::CopyDetails)
}

/// Replace the contents of an existing config file IN PLACE (same file, so its owner, permissions and on
/// Windows its access list, including the grant the server's service account was given, all stay as they are:
/// a new file renamed over it would be owned by whoever runs Kora and the service could no longer read it).
/// Only the bytes from the first difference onwards are written, and the file is never emptied first, so
/// a crash or power cut at the wrong moment cannot lose the file's other lines (the database connection
/// string lives here too): with a new key of the same length, the usual case, only the key's own bytes are
/// touched. If writing fails half-way the old contents are put back.
fn rewrite_in_place(path: &Path, old: &str, new: &str) -> std::io::Result<()> {
    use std::io::{Seek, SeekFrom, Write};
    let mut f = crate::fsutil::open_no_follow(path, crate::fsutil::Access::ReadWrite, None)?;
    let from = changed_from(old.as_bytes(), new.as_bytes());
    let put = |f: &mut std::fs::File, text: &str| -> std::io::Result<()> {
        f.seek(SeekFrom::Start(from as u64))?;
        f.write_all(&text.as_bytes()[from..])?;
        f.set_len(text.len() as u64)?;
        f.sync_all()
    };
    put(&mut f, new).inspect_err(|_| {
        let _ = put(&mut f, old);
    })
}

/// Where `old` and `new` first differ (their common length when one is the start of the other).
fn changed_from(old: &[u8], new: &[u8]) -> usize {
    old.iter().zip(new).take_while(|(a, b)| a == b).count()
}

/// Write `key` into the server's own configuration, where the key lives today (`secrets.cfg`, else
/// `server.cfg`). The key is validated first. The file keeps its other lines, its owner and its permissions.
pub fn set_server_key(dir: &Path, key: &str) -> Result<(), Problem> {
    let key = check_key(key)?;
    for f in SERVER_FILES {
        let path = dir.join(f);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(config_problem("Kora could not read the server's configuration, so the key was not changed.", format!("{}: {e}", path.display()))),
        };
        if let Some(new) = replace_key_line(&text, &key) {
            return rewrite_in_place(&path, &text, &new).map_err(|e| {
                config_problem(
                    "Kora could not write the new key into the server's configuration (the account that runs Kora may not be allowed to change that file). The old key is still in place.",
                    format!("{}: {e}", path.display()),
                )
            });
        }
    }
    Err(config_problem(
        "Kora could not find where this server's licence key is set, so nothing was changed. Servers Kora installed keep it in secrets.cfg; edit that file by hand for other setups.",
        format!("no sv_licenseKey line in {} or {}", dir.join("secrets.cfg").display(), dir.join("server.cfg").display()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const K1: &str = "cfxk_AAAAAAAAAAAAAAAA_1111";
    const K2: &str = "cfxk_BBBBBBBBBBBBBBBB_2222";

    fn store() -> (tempfile::TempDir, KeyStore) {
        let d = tempfile::tempdir().unwrap();
        let s = KeyStore::at(d.path().join("secrets.json"));
        (d, s)
    }

    #[test]
    fn missing_store_is_empty_and_is_not_created_by_reading() {
        let (d, s) = store();
        assert!(s.list().unwrap().is_empty());
        assert!(!crate::store::db_path(d.path()).exists() && !d.path().join("secrets.json").exists());
        assert_eq!(s.get("x").unwrap_err().code, codes::KEYS_NOT_FOUND);
    }

    #[test]
    fn add_list_get_masks_everything_but_the_last_four() {
        let (_d, s) = store();
        let i = s.add("  Main ", K1).unwrap();
        assert_eq!(i.name, "Main");
        assert_eq!(i.last4, "1111");
        let listed = s.list().unwrap();
        let shown = serde_json::to_string(&listed).unwrap() + &format!("{listed:?}");
        assert!(!shown.contains("AAAAAAAA"), "{shown}");
        assert_eq!(s.get("main").unwrap(), K1, "names ignore case");
        assert_eq!(s.name_of(K1).unwrap().as_deref(), Some("Main"));
        assert_eq!(s.name_of(K2).unwrap(), None);
    }

    #[cfg(unix)]
    #[test]
    fn the_database_is_as_private_as_secrets_json_was() {
        use std::os::unix::fs::PermissionsExt;
        let (d, s) = store();
        s.add("Main", K1).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let dir = crate::store::store_dir(d.path());
        assert_eq!(mode(&dir), 0o700);
        let mut files = 0;
        for f in std::fs::read_dir(&dir).unwrap().flatten().filter(|f| f.path().is_file()) {
            files += 1;
            assert_eq!(mode(&f.path()), 0o600, "{}", f.path().display());
        }
        assert!(files >= 1);
        // a copy made too open is closed again when Kora next opens it
        let db = crate::store::db_path(d.path());
        std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o644)).unwrap();
        s.list().unwrap();
        assert_eq!(mode(&db), 0o600);
        // the key is in the file and nowhere else is it handed out
        let bytes = std::fs::read(&db).unwrap();
        let wal = std::fs::read(crate::store::privacy::side_file(&db, "-wal")).unwrap_or_default();
        assert!(bytes.windows(K1.len()).any(|w| w == K1.as_bytes()) || wal.windows(K1.len()).any(|w| w == K1.as_bytes()));
    }

    #[test]
    fn a_forgotten_key_is_wiped_from_the_file() {
        let (d, s) = store();
        s.add("Main", K1).unwrap();
        s.remove("Main").unwrap();
        // fold the write-ahead log into the main file, then look
        let st = Store::open(d.path()).unwrap();
        st.read(|c| {
            c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
            Ok(())
        })
        .unwrap();
        let bytes = std::fs::read(crate::store::db_path(d.path())).unwrap();
        assert!(!bytes.windows(K1.len()).any(|w| w == K1.as_bytes()), "secure_delete should have zeroed it");
    }

    #[test]
    fn two_stores_in_one_process_see_each_others_changes() {
        let d = tempfile::tempdir().unwrap();
        let a = KeyStore::at(d.path().join("secrets.json"));
        let b = KeyStore::at(d.path().join("secrets.json"));
        a.add("One", K1).unwrap();
        b.add("Two", K2).unwrap();
        assert_eq!(a.list().unwrap().len(), 2);
        b.remove("One").unwrap();
        assert_eq!(a.list().unwrap().len(), 1);
        // order of saving is kept
        a.add("Three", "cfxk_CCCCCCCCCCCCCCCC_3333").unwrap();
        assert_eq!(a.list().unwrap().iter().map(|k| k.name.as_str()).collect::<Vec<_>>(), ["Two", "Three"]);
    }

    #[test]
    fn names_and_duplicates() {
        let (_d, s) = store();
        s.add("Main", K1).unwrap();
        assert_eq!(s.add("MAIN", K2).unwrap_err().code, codes::KEYS_NAME_TAKEN);
        let e = s.add("Other", K1).unwrap_err();
        assert_eq!(e.code, codes::KEYS_DUPLICATE);
        assert!(e.message.contains("\"Main\""), "says where it is saved: {}", e.message);
        for bad in ["", "   ", "a\nb", &"x".repeat(41)] {
            assert_eq!(s.add(bad, K2).unwrap_err().code, codes::KEYS_INVALID, "{bad:?}");
        }
        for bad in ["", "short", "has space in it 1234", "cfxk_ä_ä_ä_ä_ä"] {
            let e = s.add("Ok", bad).unwrap_err();
            assert_eq!(e.code, codes::KEYS_INVALID, "{bad:?}");
            assert!(!e.message.contains(bad) || bad.is_empty());
        }
        assert_eq!(s.list().unwrap().len(), 1);
    }

    #[test]
    fn rename_replace_remove() {
        let (_d, s) = store();
        s.add("Main", K1).unwrap();
        s.add("Test server", K2).unwrap();
        assert_eq!(s.rename("main", "Live").unwrap().name, "Live");
        assert_eq!(s.rename("Live", "test SERVER").unwrap_err().code, codes::KEYS_NAME_TAKEN);
        assert_eq!(s.rename("Live", "LIVE").unwrap().name, "LIVE", "changing only the case is allowed");
        assert_eq!(s.rename("Nope", "X").unwrap_err().code, codes::KEYS_NOT_FOUND);
        let new = "cfxk_CCCCCCCCCCCCCCCC_3333";
        assert_eq!(s.replace("live", new).unwrap().last4, "3333");
        assert_eq!(s.replace("live", K2).unwrap_err().code, codes::KEYS_DUPLICATE);
        assert_eq!(s.get("live").unwrap(), new);
        s.remove("LIVE").unwrap();
        assert_eq!(s.get("live").unwrap_err().code, codes::KEYS_NOT_FOUND);
        assert_eq!(s.remove("live").unwrap_err().code, codes::KEYS_NOT_FOUND);
        assert_eq!(s.list().unwrap().len(), 1);
    }

    #[test]
    fn concurrent_adds_all_land() {
        let (_d, s) = store();
        let hs: Vec<_> = (0..8)
            .map(|i| {
                let s = s.clone();
                std::thread::spawn(move || s.add(&format!("Key {i}"), &format!("cfxk_thread_key_number_{i:04}")).unwrap())
            })
            .collect();
        for h in hs {
            h.join().unwrap();
        }
        assert_eq!(s.list().unwrap().len(), 8);
    }

    #[test]
    fn debug_never_prints_a_key() {
        let e = Entry { id: 1, name: "Main".into(), key: K1.into(), added: 1, place: Place::Store, lost: None };
        assert!(!format!("{e:?}").contains("AAAAAAAA"));
    }

    #[test]
    fn key_lines_are_read_and_replaced_in_place() {
        assert_eq!(parse_key_line("sv_licenseKey \"abc12345\""), Some("abc12345".into()));
        assert_eq!(parse_key_line("  set sv_licensekey abc12345 # mine"), Some("abc12345".into()));
        assert_eq!(parse_key_line("# sv_licenseKey \"abc12345\""), None);
        assert_eq!(parse_key_line("sv_hostname \"x\""), None);
        let text = "# Kora secrets\r\nsv_licenseKey \"old_old_old\"\r\nset mysql_connection_string \"mysql://u:p@h/d\"\r\n";
        let out = replace_key_line(text, K1).unwrap();
        assert_eq!(out, format!("# Kora secrets\r\nsv_licenseKey \"{K1}\"\r\nset mysql_connection_string \"mysql://u:p@h/d\"\r\n"));
        assert_eq!(replace_key_line("sv_hostname \"x\"\n", K1), None);
    }

    #[test]
    fn a_key_change_only_touches_the_bytes_that_differ() {
        assert_eq!(changed_from(b"abc KEY1 tail", b"abc KEY2 tail"), 7);
        assert_eq!(changed_from(b"abc", b"abcdef"), 3);
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("secrets.cfg");
        // Longer, shorter and same-length replacements all leave the other lines as they were.
        for old in ["short", "a_much_longer_old_key_than_the_new_one_is_by_far_0123456789", K2] {
            std::fs::write(&p, format!("# top\nsv_licenseKey \"{old}\"\nset mysql_connection_string \"x\"\n")).unwrap();
            set_server_key(d.path(), K1).unwrap();
            assert_eq!(std::fs::read_to_string(&p).unwrap(), format!("# top\nsv_licenseKey \"{K1}\"\nset mysql_connection_string \"x\"\n"));
        }
    }

    #[test]
    fn server_key_is_changed_where_it_lives() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("secrets.cfg"), "sv_licenseKey \"old_old_old_1\"\nset mysql_connection_string \"x\"\n").unwrap();
        assert_eq!(server_key(d.path()).as_deref(), Some("old_old_old_1"));
        set_server_key(d.path(), K1).unwrap();
        assert_eq!(server_key(d.path()).as_deref(), Some(K1));
        let t = std::fs::read_to_string(d.path().join("secrets.cfg")).unwrap();
        assert!(t.contains("mysql_connection_string"), "other lines kept: {t}");
        // Not replaced by a new file: the owner, the mode and (Windows) the service account's grant stay.
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            let p = d.path().join("secrets.cfg");
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o640)).unwrap();
            let ino = std::fs::metadata(&p).unwrap().ino();
            set_server_key(d.path(), K2).unwrap();
            let m = std::fs::metadata(&p).unwrap();
            assert_eq!((m.ino(), m.permissions().mode() & 0o777), (ino, 0o640));
            set_server_key(d.path(), K1).unwrap();
        }
        // Key only in server.cfg (an adopted server).
        let d2 = tempfile::tempdir().unwrap();
        std::fs::write(d2.path().join("server.cfg"), "sv_licenseKey old_old_old_2\nsv_maxclients 32\n").unwrap();
        set_server_key(d2.path(), K2).unwrap();
        assert_eq!(server_key(d2.path()).as_deref(), Some(K2));
        // Nowhere: a guided problem, nothing written.
        let d3 = tempfile::tempdir().unwrap();
        std::fs::write(d3.path().join("server.cfg"), "sv_maxclients 32\n").unwrap();
        let e = set_server_key(d3.path(), K1).unwrap_err();
        assert_eq!(e.code, codes::KEYS_SERVER_CONFIG);
        assert!(!d3.path().join("secrets.cfg").exists());
        // A bad key never reaches the file.
        assert_eq!(set_server_key(d.path(), "bad key").unwrap_err().code, codes::KEYS_INVALID);
        assert_eq!(server_key(d.path()).as_deref(), Some(K1));
    }

    #[test]
    fn servers_using_a_key() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("secrets.cfg"), format!("sv_licenseKey \"{K1}\"\n")).unwrap();
        let mk = |name: &str, dir: &Path| ServerEntry {
            source: None,
            versions: None,
            name: name.into(),
            dir: dir.to_path_buf(),
            mode: crate::installer::Mode::Production,
            ports: crate::installer::registry::PortsRecord { game: 30120, txadmin: 40120 },
            created_unix: 0,
        };
        let list = vec![mk("a", d.path()), mk("b", &d.path().join("nowhere"))];
        assert_eq!(servers_using(K1, &list), vec!["a".to_string()]);
        assert!(servers_using(K2, &list).is_empty());
    }
}
