//! What Kora remembers about the person between installs: their Cfx.re name (and the identifier it
//! resolved to) and the GitHub owner they fork into. `<kora home>/profile.json`.
//!
//! Nothing in here is secret (a public forum name, its public id, a public GitHub login), so it has its
//! own table, separate from the saved keys. The wizard pre-fills from it; the person can edit or forget
//! any of it. Anything unreadable is treated as empty (it only saves typing). Before v0.8 this was
//! `profile.json`; the store imports that file once (see `store::legacy`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::installer::registry::kora_home;
use crate::problem::Problem;
use crate::store::Store;

pub const FILE_NAME: &str = "profile.json";

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

/// The profile in the Kora home's store (or in the home beside `path`, for tests): `path` only says which
/// Kora home (`<home>/profile.json` was the file before v0.8; the data is in the database now).
#[derive(Debug, Clone)]
pub struct ProfileStore {
    /// `<kora home>/profile.json`: only names the Kora home.
    legacy: PathBuf,
    /// The database file.
    path: PathBuf,
}

impl Default for ProfileStore {
    fn default() -> Self {
        Self::at(kora_home().join(FILE_NAME))
    }
}

const FIELDS: [&str; 3] = ["cfx_username", "cfx_identifier", "github_owner"];

fn read(c: &rusqlite::Connection) -> Result<Profile, Problem> {
    let mut p = Profile::default();
    let mut st = c.prepare("SELECT field, value FROM profile")?;
    for row in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (k, v) = row?;
        match k.as_str() {
            "cfx_username" => p.cfx_username = Some(v),
            "cfx_identifier" => p.cfx_identifier = Some(v),
            "github_owner" => p.github_owner = Some(v),
            _ => {}
        }
    }
    Ok(p)
}

fn write(c: &rusqlite::Connection, p: &Profile) -> Result<(), Problem> {
    let vals = [&p.cfx_username, &p.cfx_identifier, &p.github_owner];
    for (f, v) in FIELDS.iter().zip(vals) {
        match v {
            Some(v) => {
                c.execute("INSERT INTO profile (field, value) VALUES (?1, ?2) ON CONFLICT(field) DO UPDATE SET value = excluded.value", [f, v.as_str()])?;
            }
            None => {
                c.execute("DELETE FROM profile WHERE field = ?1", [f])?;
            }
        }
    }
    Ok(())
}

pub(crate) fn read_profile_pub(c: &rusqlite::Connection) -> Result<Profile, Problem> {
    read(c)
}

/// Used by the import of the old `profile.json`: replace what is remembered, or fill in what is missing.
pub(crate) fn import_profile(c: &rusqlite::Connection, p: &Profile, replace: bool) -> Result<(), Problem> {
    if replace {
        return write(c, p);
    }
    let mut have = read(c)?;
    have.cfx_username = have.cfx_username.or_else(|| p.cfx_username.clone());
    have.cfx_identifier = have.cfx_identifier.or_else(|| p.cfx_identifier.clone());
    have.github_owner = have.github_owner.or_else(|| p.github_owner.clone());
    write(c, &have)
}

impl ProfileStore {
    pub fn at(path: impl Into<PathBuf>) -> Self {
        let legacy = path.into();
        let path = crate::store::db_path(legacy.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new(".")));
        Self { legacy, path }
    }

    /// Where the profile is saved: the store's database file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What is remembered. It only saves typing, so anything that goes wrong reads as "nothing remembered".
    pub fn load(&self) -> Profile {
        let Ok(Some(s)) = Store::open_existing_beside(&self.legacy) else { return Profile::default() };
        s.read(read).unwrap_or_default()
    }

    /// Change the given fields (in one transaction: two programs changing different fields keep both).
    /// Best effort for the caller: a failure to remember never blocks an install.
    pub fn update(&self, cfx_username: Field, cfx_identifier: Field, github_owner: Field) -> crate::problem::Res<Profile> {
        // Nothing to remember and nothing saved: nothing to do, and nothing is created.
        let only_forgetting = !matches!(cfx_username, Field::Set(_)) && !matches!(cfx_identifier, Field::Set(_)) && !matches!(github_owner, Field::Set(_));
        if only_forgetting && Store::open_existing_beside(&self.legacy)?.is_none() {
            return Ok(Profile::default());
        }
        let s = Store::open_beside(&self.legacy)?;
        s.write(|tx| {
            let mut p = read(tx)?;
            cfx_username.apply(&mut p.cfx_username);
            cfx_identifier.apply(&mut p.cfx_identifier);
            github_owner.apply(&mut p.github_owner);
            write(tx, &p)?;
            Ok(p)
        })
    }

    /// Remember a Cfx.re account that was just found.
    pub fn remember_cfx(&self, username: &str, identifier: &str) {
        let _ = self.update(Field::Set(username.to_string()), Field::Set(identifier.to_string()), Field::Keep);
    }

    /// Remember the GitHub owner an install used.
    pub fn remember_github_owner(&self, owner: &str) {
        let _ = self.update(Field::Keep, Field::Keep, Field::Set(owner.to_string()));
    }

    pub fn forget_all(&self) -> crate::problem::Res<()> {
        let Some(s) = Store::open_existing_beside(&self.legacy)? else { return Ok(()) };
        s.write(|tx| {
            tx.execute("DELETE FROM profile", [])?;
            Ok(())
        })
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
