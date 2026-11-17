//! Saved Cfx.re licence keys: `<kora home>/secrets.json`, picked by name.
//!
//! One plain-text JSON file (not encrypted, like the per-server `secrets.cfg`), readable only by the
//! person who runs Kora: created `0600` on Linux/macOS and, on Windows, with the inherited permissions
//! removed (the same helper the panel token and the per-server `secrets.cfg` use, `monitor::write_private`).
//!
//! The rules the rest of Kora relies on:
//! * Everything that leaves this module for a screen, an API answer, a log or an event is a [`KeyInfo`]:
//!   the name and the LAST FOUR characters. The full key only comes out of [`KeyStore::get`], which is
//!   for writing the server's own config and nothing else.
//! * Writes are atomic (temp file, then rename) and the temp file is private from the moment it exists.
//! * A missing file is an empty store. A file that cannot be read or is damaged is never replaced: every
//!   call says so (a [`Problem`]) and leaves it alone, so keys are never silently lost.
//! * One process-wide lock makes changes safe between requests of the panel. Two different programs
//!   changing the file in the same instant (the panel and a `kora keys` command) is not guarded: the
//!   later write wins whole, never half.
//!
//! Key names: 1 to 40 characters, no control characters, compared without regard to case ("main" and
//! "Main" are the same name). The key itself is checked with the installer's own rule
//! ([`crate::txadmin::license_key_ok`]).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::installer::registry::{kora_home, ServerEntry};
use crate::problem::{codes, Action, Problem};

/// The file in the Kora home.
pub const FILE_NAME: &str = "secrets.json";
const VERSION: u32 = 1;
pub const MAX_NAME: usize = 40;

static LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    name: String,
    key: String,
    #[serde(default)]
    added: u64,
}

impl std::fmt::Debug for Entry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Entry").field("name", &self.name).field("key", &"<redacted>").finish()
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Doc {
    #[serde(default = "one")]
    version: u32,
    #[serde(default)]
    keys: Vec<Entry>,
}

fn one() -> u32 {
    VERSION
}

/// What may be shown about a saved key: never more than its name and last four characters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KeyInfo {
    pub name: String,
    pub last4: String,
    /// Unix seconds when it was saved (0 = unknown).
    pub added: u64,
}

/// The last four characters of `key` (all of it when it is shorter, which a valid key never is).
pub fn last4(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    chars[chars.len().saturating_sub(4)..].iter().collect()
}

fn info(e: &Entry) -> KeyInfo {
    KeyInfo { name: e.name.clone(), last4: last4(&e.key), added: e.added }
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

/// The saved keys file. [`KeyStore::default`] is the one in the Kora home; tests use [`KeyStore::at`].
#[derive(Debug, Clone)]
pub struct KeyStore {
    path: PathBuf,
}

impl Default for KeyStore {
    fn default() -> Self {
        Self { path: kora_home().join(FILE_NAME) }
    }
}

impl KeyStore {
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn unreadable(&self, why: &str, details: String) -> Problem {
        Problem::new(
            codes::KEYS_UNREADABLE,
            format!(
                "Kora cannot read your saved licence keys ({why}). Nothing was changed and no key was lost: the file is still where it was. Fix or move the file {} and try again.",
                self.path.display()
            ),
        )
        .with_details(details)
        .with_action(Action::CopyDetails)
    }

    fn read(&self) -> Result<Doc, Problem> {
        let text = match std::fs::read(&self.path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Doc { version: VERSION, keys: vec![] }),
            Err(e) => return Err(self.unreadable("the file cannot be opened", format!("{}: {e}", self.path.display()))),
        };
        tighten(&self.path);
        // serde's messages can quote the offending value, which could be a key: only the position is kept.
        let doc: Doc =
            serde_json::from_slice(&text).map_err(|e| self.unreadable("the file is damaged", format!("{} is not valid at line {}, column {}", self.path.display(), e.line(), e.column())))?;
        if doc.version > VERSION {
            return Err(self.unreadable("it was written by a newer Kora", format!("{} has format version {}, this Kora understands {VERSION}", self.path.display(), doc.version)));
        }
        Ok(doc)
    }

    fn write(&self, doc: &Doc) -> Result<(), Problem> {
        let text = serde_json::to_string_pretty(doc).map_err(|e| Problem::generic("Kora could not save your licence keys.").with_details(e.to_string()))?;
        // A leftover temp file from a crash could have looser permissions: start from nothing.
        let _ = std::fs::remove_file(self.path.with_extension("tmp"));
        crate::monitor::write_private(&self.path, &text).map_err(|e| {
            Problem::new(codes::FAILED, format!("Kora could not save your licence keys to {}. Check that the folder can be written to.", self.path.display()))
                .with_details(e)
                .with_action(Action::CopyDetails)
        })?;
        hand_to_home_owner(&self.path);
        Ok(())
    }

    /// Run `f` on the keys under the lock; the file is written only when `f` says it changed something.
    fn change<T>(&self, f: impl FnOnce(&mut Vec<Entry>) -> Result<(T, bool), Problem>) -> Result<T, Problem> {
        let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let mut doc = self.read()?;
        let (out, changed) = f(&mut doc.keys)?;
        if changed {
            doc.version = VERSION;
            self.write(&doc)?;
        }
        Ok(out)
    }

    /// The saved keys, safe to show.
    pub fn list(&self) -> Result<Vec<KeyInfo>, Problem> {
        let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        Ok(self.read()?.keys.iter().map(info).collect())
    }

    /// Does a key with this name exist (no key is read out)?
    pub fn exists(&self, name: &str) -> Result<bool, Problem> {
        Ok(self.list()?.iter().any(|k| same(&k.name, name)))
    }

    /// The FULL key. Only for writing the server's own configuration or starting an install: never
    /// put the result in an answer, a log, an event or an error.
    pub fn get(&self, name: &str) -> Result<String, Problem> {
        let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        self.read()?.keys.into_iter().find(|e| same(&e.name, name)).map(|e| e.key).ok_or_else(|| not_found(name))
    }

    /// The name a key value is already saved under, if any.
    pub fn name_of(&self, key: &str) -> Result<Option<String>, Problem> {
        let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let k = key.trim();
        Ok(self.read()?.keys.into_iter().find(|e| e.key == k).map(|e| e.name))
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
            let e = Entry { name, key, added: crate::monitor::now_unix() };
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
        self.change(|keys| {
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
        let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        Ok(self.read()?.keys.iter().map(|e| (info(e), servers_using(&e.key, servers))).collect())
    }

    /// Forget a saved key. Servers installed with it are not touched.
    pub fn remove(&self, name: &str) -> Result<(), Problem> {
        self.change(|keys| {
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

/// Run as root (`sudo kora ...`) inside someone else's Kora folder, a file Kora creates would belong to root and
/// that person could no longer read their own keys: give it to the owner of the folder (best effort; nothing
/// happens off Unix, when not root, or when the folder is root's own).
pub(crate) fn hand_to_home_owner(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if !crate::installer::is_admin() {
            return;
        }
        if let Some(m) = path.parent().and_then(|p| std::fs::metadata(p).ok()) {
            if m.uid() != 0 {
                // `lchown`: the folder belongs to someone else, who could swap `path` for a link to any file
                // while root is here; `chown` would follow it and hand that file over.
                let _ = std::os::unix::fs::lchown(path, Some(m.uid()), Some(m.gid()));
            }
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// A file made by an older tool or copied around may be readable by others: take that away (best effort).
fn tighten(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // On the open file, never through a link (root may be reading inside someone else's folder).
        if let Ok(f) = crate::fsutil::open_no_follow(path, crate::fsutil::Access::Read, None) {
            if f.metadata().is_ok_and(|m| m.permissions().mode() & 0o077 != 0) {
                let _ = f.set_permissions(std::fs::Permissions::from_mode(0o600));
            }
        }
    }
    #[cfg(not(unix))]
    let _ = path;
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
    fn missing_file_is_an_empty_store_and_is_not_created_by_reading() {
        let (d, s) = store();
        assert!(s.list().unwrap().is_empty());
        assert!(!d.path().join("secrets.json").exists());
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
    fn file_is_private_from_creation_and_made_private_again_when_read() {
        use std::os::unix::fs::PermissionsExt;
        let (d, s) = store();
        s.add("Main", K1).unwrap();
        let p = d.path().join("secrets.json");
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&p), 0o600);
        assert!(!d.path().join("secrets.tmp").exists(), "no temp file is left");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        s.list().unwrap();
        assert_eq!(mode(&p), 0o600);
        // A stale temp file with loose permissions is not reused.
        let tmp = d.path().join("secrets.tmp");
        std::fs::write(&tmp, "old").unwrap();
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o666)).unwrap();
        s.add("Second", K2).unwrap();
        assert_eq!(mode(&p), 0o600);
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
    fn a_damaged_file_is_reported_and_never_overwritten() {
        let (d, s) = store();
        let p = d.path().join("secrets.json");
        let junk = format!("{{\"keys\": [{{\"name\": \"Main\", \"key\": \"{K1}\" ");
        std::fs::write(&p, &junk).unwrap();
        for e in [s.list().unwrap_err(), s.add("New", K2).unwrap_err(), s.get("Main").unwrap_err(), s.remove("Main").unwrap_err()] {
            assert_eq!(e.code, codes::KEYS_UNREADABLE);
            assert!(e.message.contains("no key was lost"), "{}", e.message);
            let all = format!("{} {:?}", e.message, e.details);
            assert!(!all.contains("AAAAAAAA"), "no key in an error: {all}");
        }
        assert_eq!(std::fs::read_to_string(&p).unwrap(), junk, "left exactly as it was");
        // A wrong type: serde would quote the value; we must not.
        std::fs::write(&p, format!("{{\"keys\": \"{K1}\"}}")).unwrap();
        let e = s.list().unwrap_err();
        assert!(!format!("{} {:?}", e.message, e.details).contains("AAAAAAAA"));
        // Written by a newer Kora.
        std::fs::write(&p, r#"{"version": 99, "keys": []}"#).unwrap();
        assert_eq!(s.add("New", K2).unwrap_err().code, codes::KEYS_UNREADABLE);
    }

    #[test]
    fn unreadable_path_is_a_problem_not_a_panic() {
        let d = tempfile::tempdir().unwrap();
        // The "file" is a folder.
        std::fs::create_dir(d.path().join("secrets.json")).unwrap();
        let e = KeyStore::at(d.path().join("secrets.json")).list().unwrap_err();
        assert_eq!(e.code, codes::KEYS_UNREADABLE);
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
        let e = Entry { name: "Main".into(), key: K1.into(), added: 1 };
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
