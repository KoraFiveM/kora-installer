//! `<kora home>/panel.json`: who may sign in remotely, with which role. Tokens are stored only as a
//! salted SHA-256 hash; the file is written with mode 0600 on Unix (on Windows it inherits the
//! permissions of the user's profile folder).
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RemoteConfig {
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
    /// Missing file: the default (remote access off). A file that cannot be read or understood is an
    /// error, never silently replaced.
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(s) if s.trim().is_empty() => Ok(Self::default()),
            Ok(s) => serde_json::from_str(&s).map_err(|e| format!("The remote access settings file {} is not valid: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("Could not read {}: {e}", path.display())),
        }
    }

    /// Write atomically (temp file in the same folder, then rename), mode 0600 on Unix.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        std::fs::create_dir_all(dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
        let tmp = dir.join(format!(".{FILE_NAME}.{}.tmp", random_hex().get(..12).unwrap_or("tmp")));
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        write_private(&tmp, json.as_bytes()).map_err(|e| format!("Could not write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("Could not save {}: {e}", path.display())
        })
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

#[cfg(unix)]
fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
    f.write_all(data)?;
    f.sync_all()
}

#[cfg(not(unix))]
fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    // Windows: the file sits in the user's own profile folder and inherits its access list.
    std::fs::write(path, data)
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
    fn the_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(FILE_NAME);
        let mut c = RemoteConfig::default();
        c.add_user("a", Role::Admin).unwrap();
        c.save(&p).unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        // Saving again (rename over the old file) keeps it private.
        c.save(&p).unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(RemoteConfig::load(&p).unwrap(), c);
        // No temp files left behind.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn missing_is_default_and_corrupt_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(FILE_NAME);
        assert_eq!(RemoteConfig::load(&p).unwrap(), RemoteConfig::default());
        std::fs::write(&p, "{ not json").unwrap();
        assert!(RemoteConfig::load(&p).is_err());
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
