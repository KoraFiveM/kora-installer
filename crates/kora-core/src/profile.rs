//! What Kora remembers about the person between installs: their Cfx.re name (and the identifier it
//! resolved to) and the GitHub owner they fork into. `<kora home>/profile.json`.
//!
//! Nothing in this file is secret (a public forum name, its public id, a public GitHub login), so it is
//! a normal file, separate from `secrets.json`. The wizard pre-fills from it; the person can edit or
//! forget any of it. A damaged file is treated as empty (it only saves typing) and replaced on the next save.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::installer::registry::kora_home;

pub const FILE_NAME: &str = "profile.json";

static LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    /// The Cfx.re name as the forum spells it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cfx_username: Option<String>,
    /// `fivem:<id>`, what the name resolved to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cfx_identifier: Option<String>,
    /// The GitHub user or organisation development servers are forked into.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_owner: Option<String>,
}

/// A field in a change: leave it, set it, or forget it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Field {
    #[default]
    Keep,
    Set(String),
    Forget,
}

impl Field {
    fn apply(self, slot: &mut Option<String>) {
        match self {
            Field::Keep => {}
            Field::Set(v) => *slot = Some(v),
            Field::Forget => *slot = None,
        }
    }
}

/// The file in the Kora home (or elsewhere, for tests).
#[derive(Debug, Clone)]
pub struct ProfileStore {
    path: PathBuf,
}

impl Default for ProfileStore {
    fn default() -> Self {
        Self { path: kora_home().join(FILE_NAME) }
    }
}

impl ProfileStore {
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Profile {
        std::fs::read(&self.path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    /// Change the given fields. Best effort for the caller: a failure to remember never blocks an install.
    pub fn update(&self, cfx_username: Field, cfx_identifier: Field, github_owner: Field) -> std::io::Result<Profile> {
        let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let mut p = self.load();
        cfx_username.apply(&mut p.cfx_username);
        cfx_identifier.apply(&mut p.cfx_identifier);
        github_owner.apply(&mut p.github_owner);
        if let Some(d) = self.path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let text = serde_json::to_string_pretty(&p).map_err(std::io::Error::other)?;
        // Never through a link: `kora install` can run as root (through the helper) inside the user's own folder.
        crate::fsutil::write_no_follow(&self.path, text.as_bytes(), 0o644)?;
        crate::keystore::hand_to_home_owner(&self.path);
        Ok(p)
    }

    /// Remember a Cfx.re account that was just found.
    pub fn remember_cfx(&self, username: &str, identifier: &str) {
        let _ = self.update(Field::Set(username.to_string()), Field::Set(identifier.to_string()), Field::Keep);
    }

    /// Remember the GitHub owner an install used.
    pub fn remember_github_owner(&self, owner: &str) {
        let _ = self.update(Field::Keep, Field::Keep, Field::Set(owner.to_string()));
    }

    pub fn forget_all(&self) -> std::io::Result<()> {
        let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        match std::fs::remove_file(&self.path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembers_edits_and_forgets() {
        let d = tempfile::tempdir().unwrap();
        let s = ProfileStore::at(d.path().join("profile.json"));
        assert_eq!(s.load(), Profile::default());
        s.remember_cfx("JSixty", "fivem:42");
        s.remember_github_owner("KoraFiveM");
        let p = s.load();
        assert_eq!((p.cfx_username.as_deref(), p.cfx_identifier.as_deref(), p.github_owner.as_deref()), (Some("JSixty"), Some("fivem:42"), Some("KoraFiveM")));
        let p = s.update(Field::Forget, Field::Forget, Field::Keep).unwrap();
        assert_eq!((p.cfx_username, p.cfx_identifier, p.github_owner.as_deref()), (None, None, Some("KoraFiveM")));
        s.forget_all().unwrap();
        s.forget_all().unwrap();
        assert_eq!(s.load(), Profile::default());
    }

    #[test]
    fn a_damaged_file_is_just_empty_and_is_replaced_on_save() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("profile.json");
        std::fs::write(&p, "{ nope").unwrap();
        let s = ProfileStore::at(&p);
        assert_eq!(s.load(), Profile::default());
        s.remember_github_owner("x-org");
        assert_eq!(s.load().github_owner.as_deref(), Some("x-org"));
    }
    /// `kora install` can run as root through the helper, inside the user's own folder: a link the user left at
    /// the file (or at its temporary name) must be replaced, never written through.
    #[cfg(unix)]
    #[test]
    fn saving_never_writes_through_a_link() {
        let d = tempfile::tempdir().unwrap();
        let victim = d.path().join("victim.txt");
        std::fs::write(&victim, "keep me").unwrap();
        let p = d.path().join("profile.json");
        for planted in [p.clone(), p.with_extension("json.tmp")] {
            std::os::unix::fs::symlink(&victim, &planted).unwrap();
            let s = ProfileStore::at(&p);
            s.remember_github_owner("x-org");
            assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep me", "{planted:?}");
            assert_eq!(s.load().github_owner.as_deref(), Some("x-org"));
            let _ = std::fs::remove_file(&planted);
            let _ = std::fs::remove_file(&p);
        }
    }
}
