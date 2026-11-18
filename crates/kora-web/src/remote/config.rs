//! Who may sign in remotely, with which role, kept in the Kora home's private database (the `remote_*`
//! tables of `kora_core::store`; `<kora home>/panel.json` before v0.8, imported once). Tokens are stored
//! only as a salted SHA-256 hash. Every change made through [`RemoteConfig::update_at`] (what the panel
//! uses) is one transaction that reads and writes together, so a change made by `kora panel user add`
//! while the panel runs cannot be lost. [`RemoteConfig::save`] after a [`RemoteConfig::load`] (what the
//! CLI does) refuses, with a plain message, when somebody else changed the settings in between.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use kora_core::store::{self, rusqlite, Store};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::roles::Role;
use crate::security::{ct_eq, random_hex};

pub const FILE_NAME: &str = "panel.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenUser {
    pub name: String,
    pub role: Role,
    /// Random per user, hex.
    pub salt: String,
    /// `sha256(salt || token)`, hex. The token itself is never stored.
    pub hash: String,
    /// Unix seconds.
    #[serde(default)]
    pub created: u64,
}

impl TokenUser {
    /// The old token link still works for this user.
    pub fn has_token(&self) -> bool {
        !self.hash.is_empty()
    }
}

/// Which version of the stored settings a [`RemoteConfig`] was loaded from (0: none). Not part of the
/// settings: ignored by comparisons, copies and serialisation.
#[derive(Debug, Default)]
pub struct Rev(AtomicU64);

impl Clone for Rev {
    fn clone(&self) -> Self {
        Rev(AtomicU64::new(self.0.load(Ordering::Relaxed)))
    }
}
impl PartialEq for Rev {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl Eq for Rev {}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RemoteConfig {
    #[serde(skip)]
    pub loaded_rev: Rev,
    /// Remote access on. (`kora panel --remote` turns it on for one run without saving.)
    pub enabled: bool,
    /// Let people on the tailnet in, by their Tailscale login (asked of the local tailscaled).
    pub tailscale: bool,
    /// Tailscale login name (lower case) to role. Anyone not listed is refused.
    pub tailscale_users: BTreeMap<String, Role>,
    /// Per-user tokens.
    pub users: Vec<TokenUser>,
    /// Extra `Host` names people will use (MagicDNS names, a DNS name, an address). Port is the panel's.
    pub allowed_hosts: Vec<String>,
    /// Allow destructive actions from remote sessions (admins only; still needs the typed name).
    pub allow_remote_destructive: bool,
    /// Interactive remote users must sign in with a passkey or one-time codes: the old token link is refused
    /// for people (API tokens for scripts are unaffected). Off by default so an upgrade breaks nothing.
    pub require_strong_auth: bool,
    /// A reverse proxy in front of the panel that Kora has been told to trust (`remote::proxy`).
    pub trusted_proxy: Option<super::proxy::ProxyConfig>,
}

pub fn default_path() -> PathBuf {
    kora_core::installer::registry::kora_home().join(FILE_NAME)
}

pub fn hash_token(salt: &str, token: &str) -> String {
    let mut h = Sha256::new();
    h.update(salt.as_bytes());
    h.update([0u8]);
    h.update(token.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// A user name: letters, digits, `.`, `_`, `-`; 1 to 40 characters.
pub fn valid_user_name(name: &str) -> Result<(), String> {
    let ok = !name.is_empty() && name.len() <= 40 && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    if ok {
        Ok(())
    } else {
        Err("A user name uses letters, numbers, dots, dashes and underscores, up to 40 characters.".into())
    }
}

/// A host name or address someone will type in the browser (no scheme, path or spaces).
pub fn valid_host(h: &str) -> Result<(), String> {
    let bad = h.is_empty() || h.len() > 255 || h.bytes().any(|b| b.is_ascii_whitespace() || matches!(b, b'/' | b'\\' | b'@' | b'?' | b'#' | b'"' | b'\'' | b'<' | b'>'));
    if bad || h.contains("://") {
        Err(format!("'{h}' is not a host name or address. Use just the name, for example box.example.ts.net."))
    } else {
        Ok(())
    }
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl RemoteConfig {
    /// Nothing saved yet: the default (remote access off). Settings that cannot be read (or an old `panel.json`
    /// that could not be imported) are an error, never silently replaced.
    pub fn load(path: &Path) -> Result<Self, String> {
        let Some(s) = Store::open_existing_beside(path).map_err(crate::error::store_text)? else {
            let nothing = Self::default();
            nothing.loaded_rev.0.store(1, Ordering::Relaxed); // version 0 of the settings
            return Ok(nothing);
        };
        blocked(&s)?;
        s.read(read_cfg).map_err(crate::error::store_text)
    }

    /// The version of the saved settings now (0: none saved).
    pub fn stored_rev(path: &Path) -> Result<u64, String> {
        let Some(s) = Store::open_existing_beside(path).map_err(crate::error::store_text)? else { return Ok(0) };
        s.read(rev_of).map_err(crate::error::store_text)
    }

    /// Save these settings (replacing what is saved). Refuses, saving nothing, when the settings changed since
    /// this value was loaded (see the module docs).
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let s = Store::open_beside(path).map_err(crate::error::store_text)?;
        blocked(&s)?;
        let base = self.loaded_rev.0.load(Ordering::Relaxed);
        let new_rev = s
            .write(|tx| {
                let now = rev_of(tx)?;
                if base != 0 && now + 1 != base {
                    return Err(kora_core::problem::Problem::generic(
                        "The remote access settings were changed by another Kora window or command while you were editing them, so nothing was saved. Run the command again.",
                    ));
                }
                write_cfg(tx, self)
            })
            .map_err(crate::error::store_text)?;
        self.loaded_rev.0.store(new_rev + 1, Ordering::Relaxed);
        Ok(())
    }

    /// Read, change and write the settings in ONE transaction (nothing is lost if another program changes them
    /// at the same moment). Nothing is written when `f` fails.
    pub fn update_at<R>(path: &Path, f: impl FnOnce(&mut RemoteConfig) -> Result<R, String>) -> Result<R, String> {
        Self::update_at_cfg(path, f).map(|(r, _)| r)
    }

    /// [`update_at`](Self::update_at), also giving back the settings as saved.
    pub fn update_at_cfg<R>(path: &Path, f: impl FnOnce(&mut RemoteConfig) -> Result<R, String>) -> Result<(R, RemoteConfig), String> {
        let s = Store::open_beside(path).map_err(crate::error::store_text)?;
        blocked(&s)?;
        let mut inner: Option<String> = None;
        let r = s.write(|tx| {
            let mut cfg = read_cfg(tx)?;
            match f(&mut cfg) {
                Ok(out) => {
                    let rev = write_cfg(tx, &cfg)?;
                    cfg.loaded_rev.0.store(rev + 1, Ordering::Relaxed);
                    Ok((out, cfg))
                }
                Err(e) => {
                    inner = Some(e.clone());
                    Err(kora_core::problem::Problem::generic(e))
                }
            }
        });
        r.map_err(|p| inner.unwrap_or_else(|| crate::error::store_text(p)))
    }

    pub fn user(&self, name: &str) -> Option<&TokenUser> {
        self.users.iter().find(|u| u.name.eq_ignore_ascii_case(name))
    }

    /// Add a user and return the new token (shown once, never stored).
    pub fn add_user(&mut self, name: &str, role: Role) -> Result<String, String> {
        let name = name.trim();
        valid_user_name(name)?;
        if self.user(name).is_some() {
            return Err(format!("There is already a user named '{name}'."));
        }
        let token = random_hex();
        let salt = random_hex();
        let hash = hash_token(&salt, &token);
        self.users.push(TokenUser { name: name.to_string(), role, salt, hash, created: now() });
        Ok(token)
    }

    /// Add a user with NO token: they sign in with a passkey or one-time codes, set up through an enrolment link.
    pub fn add_invited_user(&mut self, name: &str, role: Role) -> Result<(), String> {
        let name = name.trim();
        valid_user_name(name)?;
        if self.user(name).is_some() {
            return Err(format!("There is already a user named '{name}'."));
        }
        self.users.push(TokenUser { name: name.to_string(), role, salt: String::new(), hash: String::new(), created: now() });
        Ok(())
    }

    pub fn remove_user(&mut self, name: &str) -> bool {
        let before = self.users.len();
        self.users.retain(|u| !u.name.eq_ignore_ascii_case(name));
        self.users.len() != before
    }

    /// Find the user a token belongs to. Every user is hashed and compared in constant time so the
    /// time taken does not depend on which user (or none) matched.
    pub fn user_for_token(&self, token: &str) -> Option<&TokenUser> {
        let mut found = None;
        for u in &self.users {
            // A user without a token (invited, or the token was ended) has an empty hash: never a match.
            if !u.has_token() {
                continue;
            }
            if ct_eq(&hash_token(&u.salt, token), &u.hash) {
                found = Some(u);
            }
        }
        found
    }

    pub fn tailscale_role(&self, login: &str) -> Option<Role> {
        let login = login.to_ascii_lowercase();
        self.tailscale_users.iter().find(|(k, _)| k.to_ascii_lowercase() == login).map(|(_, r)| *r)
    }
}

fn blocked(s: &Store) -> Result<(), String> {
    match s.blocked("remote") {
        Some(b) => Err(format!("The remote access settings file {} could not be read ({}); it was left exactly as it was. Fix or move it and try again.", b.file.display(), b.why)),
        None => Ok(()),
    }
}

fn rev_of(c: &rusqlite::Connection) -> Result<u64, kora_core::problem::Problem> {
    Ok(store::meta_get(c, "remote_rev")?.and_then(|v| v.parse().ok()).unwrap_or(0))
}

fn role_text(r: Role) -> &'static str {
    r.as_str()
}

fn role_from(s: &str) -> Result<Role, kora_core::problem::Problem> {
    s.parse::<Role>().map_err(|_| kora_core::problem::Problem::generic(format!("The saved remote access settings have an unknown role \"{s}\". Nothing was changed.")))
}

fn read_cfg(c: &rusqlite::Connection) -> Result<RemoteConfig, kora_core::problem::Problem> {
    let mut cfg = RemoteConfig::default();
    let flag = |k: &str| -> Result<bool, kora_core::problem::Problem> {
        match c.query_row("SELECT value FROM remote_settings WHERE key = ?1", [k], |r| r.get::<_, String>(0)) {
            Ok(v) => Ok(v == "1"),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
            Err(e) => Err(e.into()),
        }
    };
    cfg.enabled = flag("enabled")?;
    cfg.tailscale = flag("tailscale")?;
    cfg.allow_remote_destructive = flag("allow_remote_destructive")?;
    cfg.require_strong_auth = flag("require_strong_auth")?;
    cfg.trusted_proxy = match c.query_row("SELECT value FROM remote_settings WHERE key = 'trusted_proxy'", [], |r| r.get::<_, String>(0)) {
        Ok(v) => Some(serde_json::from_str(&v).map_err(|e| kora_core::problem::Problem::generic(format!("The saved trusted proxy setting cannot be read ({e}). Nothing was changed.")))?),
        Err(rusqlite::Error::QueryReturnedNoRows) => None,
        Err(e) => return Err(e.into()),
    };
    let mut st = c.prepare("SELECT login, role FROM remote_tailscale_users ORDER BY login")?;
    for row in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (l, r) = row?;
        cfg.tailscale_users.insert(l, role_from(&r)?);
    }
    let mut st = c.prepare("SELECT name, role, salt, hash, created FROM remote_users ORDER BY seq")?;
    for row in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, i64>(4)?)))? {
        let (name, role, salt, hash, created) = row?;
        cfg.users.push(TokenUser { name, role: role_from(&role)?, salt, hash, created: created as u64 });
    }
    let mut st = c.prepare("SELECT host FROM remote_hosts ORDER BY seq")?;
    for row in st.query_map([], |r| r.get::<_, String>(0))? {
        cfg.allowed_hosts.push(row?);
    }
    cfg.loaded_rev.0.store(rev_of(c)? + 1, Ordering::Relaxed);
    Ok(cfg)
}

/// Replace what is saved; returns the new version.
fn write_cfg(c: &rusqlite::Connection, cfg: &RemoteConfig) -> Result<u64, kora_core::problem::Problem> {
    for (k, v) in [("enabled", cfg.enabled), ("tailscale", cfg.tailscale), ("allow_remote_destructive", cfg.allow_remote_destructive), ("require_strong_auth", cfg.require_strong_auth)] {
        c.execute("INSERT INTO remote_settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value", rusqlite::params![k, if v { "1" } else { "0" }])?;
    }
    match &cfg.trusted_proxy {
        Some(p) => {
            c.execute(
                "INSERT INTO remote_settings (key, value) VALUES ('trusted_proxy', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [serde_json::to_string(p).map_err(|e| kora_core::problem::Problem::generic(e.to_string()))?],
            )?;
        }
        None => {
            c.execute("DELETE FROM remote_settings WHERE key = 'trusted_proxy'", [])?;
        }
    }
    c.execute("DELETE FROM remote_tailscale_users", [])?;
    for (l, r) in &cfg.tailscale_users {
        c.execute("INSERT INTO remote_tailscale_users (login, role) VALUES (?1, ?2) ON CONFLICT(login) DO UPDATE SET role = excluded.role", rusqlite::params![l, role_text(*r)])?;
    }
    c.execute("DELETE FROM remote_users", [])?;
    for u in &cfg.users {
        c.execute(
            "INSERT INTO remote_users (name, name_folded, role, salt, hash, created) VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT(name_folded) DO UPDATE SET name = excluded.name, role = excluded.role, salt = excluded.salt, hash = excluded.hash, created = excluded.created",
            rusqlite::params![u.name, u.name.to_lowercase(), role_text(u.role), u.salt, u.hash, u.created as i64],
        )?;
    }
    // Whoever is no longer a user loses every credential, enrolment link and passkey with the name.
    for t in ["remote_user_auth", "remote_passkeys", "remote_enrolments"] {
        c.execute(&format!("DELETE FROM {t} WHERE user NOT IN (SELECT name_folded FROM remote_users)"), [])?;
    }
    c.execute("DELETE FROM remote_hosts", [])?;
    for h in &cfg.allowed_hosts {
        c.execute("INSERT INTO remote_hosts (host) VALUES (?1)", [h])?;
    }
    let rev = rev_of(c)? + 1;
    store::meta_set(c, "remote_rev", &rev.to_string())?;
    Ok(rev)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_hashed_and_found() {
        let mut c = RemoteConfig::default();
        let t = c.add_user("alice", Role::Operator).unwrap();
        let t2 = c.add_user("bob", Role::Viewer).unwrap();
        assert_ne!(t, t2);
        assert!(!serde_json::to_string(&c).unwrap().contains(&t), "token must not be stored");
        assert_eq!(c.user_for_token(&t).unwrap().name, "alice");
        assert_eq!(c.user_for_token(&t2).unwrap().name, "bob");
        assert!(c.user_for_token("nope").is_none());
        assert!(c.add_user("ALICE", Role::Admin).is_err());
        assert!(c.add_user("bad name", Role::Admin).is_err());
        assert!(c.remove_user("Alice"));
        assert!(c.user_for_token(&t).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn the_store_is_private_and_settings_round_trip() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(FILE_NAME);
        let mut c = RemoteConfig { enabled: true, tailscale: true, allowed_hosts: vec!["box.example.com".into(), "a.b".into()], ..Default::default() };
        c.tailscale_users.insert("sam@example.com".into(), Role::Admin);
        c.add_user("a", Role::Admin).unwrap();
        c.add_user("b", Role::Viewer).unwrap();
        c.save(&p).unwrap();
        let db = store::db_path(dir.path());
        assert_eq!(std::fs::metadata(&db).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(RemoteConfig::load(&p).unwrap(), c);
        c.save(&p).unwrap();
        assert_eq!(RemoteConfig::load(&p).unwrap(), c);
    }

    #[test]
    fn saving_what_was_loaded_before_somebody_elses_change_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(FILE_NAME);
        let mut mine = RemoteConfig::load(&p).unwrap();
        // the panel (or another command) adds a user meanwhile
        RemoteConfig::update_at(&p, |c| c.add_user("theirs", Role::Viewer).map(|_| ())).unwrap();
        mine.add_user("mine", Role::Admin).unwrap();
        let e = mine.save(&p).unwrap_err();
        assert!(e.contains("changed by another Kora"), "{e}");
        let now = RemoteConfig::load(&p).unwrap();
        assert!(now.user("theirs").is_some() && now.user("mine").is_none());
        // loading again and changing is fine
        let mut again = RemoteConfig::load(&p).unwrap();
        again.add_user("mine", Role::Admin).unwrap();
        again.save(&p).unwrap();
        again.add_user("third", Role::Viewer).unwrap();
        again.save(&p).unwrap();
        assert_eq!(RemoteConfig::load(&p).unwrap().users.len(), 3);
    }

    #[test]
    fn updates_in_threads_all_land() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(FILE_NAME);
        let hs: Vec<_> = (0..6)
            .map(|i| {
                let p = p.clone();
                std::thread::spawn(move || RemoteConfig::update_at(&p, |c| c.add_user(&format!("u{i}"), Role::Viewer).map(|_| ())).unwrap())
            })
            .collect();
        for h in hs {
            h.join().unwrap();
        }
        assert_eq!(RemoteConfig::load(&p).unwrap().users.len(), 6);
    }

    #[test]
    fn nothing_saved_is_default() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(FILE_NAME);
        assert_eq!(RemoteConfig::load(&p).unwrap(), RemoteConfig::default());
        assert!(!store::db_path(dir.path()).exists(), "reading creates nothing");
    }

    #[test]
    fn host_names_are_checked() {
        assert!(valid_host("box.tail1234.ts.net").is_ok());
        assert!(valid_host("100.64.0.5").is_ok());
        assert!(valid_host("http://x").is_err());
        assert!(valid_host("a b").is_err());
        assert!(valid_host("").is_err());
    }
}
