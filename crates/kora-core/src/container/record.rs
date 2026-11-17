//! `<server folder>/kora-container.json`: what a server's container mode is, secret-free.
//!
//! It sits next to `kora-server.json`, so it travels with the folder and with backups (the backup records
//! the run mode and an import recreates it, see `backup`). No password is ever in it: the database login
//! lives in `database.json` as it always did. A server with no such file is a service-mode server, which is
//! every server made before container mode: nothing to migrate, and removing the file (after removing the
//! containers) turns a server back into a service-mode one.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::engine::EngineKind;

pub const FILE: &str = "kora-container.json";

/// The managed database container next to the game container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbRecord {
    /// Container name (`kora-<slug>-db`).
    pub container: String,
    /// Named volume holding the data (`kora-<slug>-dbdata`).
    pub volume: String,
    /// The private network both containers join (`kora-<slug>-net`).
    pub network: String,
    /// The pinned image reference it was made from.
    pub image: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerRecord {
    /// Format version of this file.
    pub version: u32,
    pub engine: EngineKind,
    pub rootless: bool,
    /// Pinned image reference of the game container's base.
    pub image: String,
    /// Container name (`kora-<slug>`).
    pub container: String,
    /// The account the container runs as (the folder's owner), so files it writes stay manageable.
    pub uid: u32,
    pub gid: u32,
    /// Address the game port is published on; `None` = every address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_mb: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_tenths: Option<u32>,
    /// Mount the folder with a private SELinux label (`:Z`): set when the engine reports SELinux enforcing.
    #[serde(default)]
    pub selinux_label: bool,
    /// Read-only root file system (the folder and `/tmp` stay writable). Off by default: whether FXServer
    /// tolerates it is checked by the CI smoke test, not assumed.
    #[serde(default)]
    pub read_only_root: bool,
    /// Host network instead of published ports. Never the default; only when the person chose it.
    #[serde(default)]
    pub host_network: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub db: Option<DbRecord>,
}

pub const VERSION: u32 = 1;

impl ContainerRecord {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join(FILE)
    }

    /// The record of this folder; `None` when it has none (a service-mode server) or cannot be read.
    pub fn load(dir: &Path) -> Option<Self> {
        let b = std::fs::read(Self::path(dir)).ok()?;
        serde_json::from_slice(&b).ok()
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        // Never through a link planted in the server folder.
        let text = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        crate::fsutil::write_no_follow(&Self::path(dir), &text, 0o644)
    }

    pub fn remove(dir: &Path) -> std::io::Result<()> {
        match std::fs::remove_file(Self::path(dir)) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }

    pub fn engine(&self) -> super::engine::Engine {
        super::engine::Engine::new(self.engine, self.rootless)
    }

    /// `in a container (podman, rootless)`: what `kora status` and the panel say.
    pub fn describe(&self) -> String {
        format!("in a container ({})", self.engine().label())
    }
}

/// Is this folder a container-mode server?
pub fn is_container(dir: &Path) -> bool {
    ContainerRecord::path(dir).is_file()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn sample() -> ContainerRecord {
        ContainerRecord {
            version: VERSION,
            engine: EngineKind::Podman,
            rootless: true,
            image: super::super::pins::BASE.reference(),
            container: "kora-main".into(),
            uid: 1000,
            gid: 1000,
            bind: None,
            memory_mb: None,
            cpu_tenths: None,
            selinux_label: false,
            read_only_root: false,
            host_network: false,
            db: Some(DbRecord { container: "kora-main-db".into(), volume: "kora-main-dbdata".into(), network: "kora-main-net".into(), image: super::super::pins::MARIADB.reference() }),
        }
    }

    #[test]
    fn a_record_round_trips_and_names_the_mode_in_words() {
        let d = tempfile::tempdir().unwrap();
        assert!(!is_container(d.path()));
        assert!(ContainerRecord::load(d.path()).is_none());
        let r = sample();
        r.save(d.path()).unwrap();
        assert!(is_container(d.path()));
        assert_eq!(ContainerRecord::load(d.path()), Some(r.clone()));
        assert_eq!(r.describe(), "in a container (podman, rootless)");
        ContainerRecord::remove(d.path()).unwrap();
        assert!(!is_container(d.path()));
        ContainerRecord::remove(d.path()).unwrap();
    }

    #[test]
    fn a_newer_file_with_extra_fields_still_reads_and_the_file_has_no_secret_fields() {
        let d = tempfile::tempdir().unwrap();
        let mut v = serde_json::to_value(sample()).unwrap();
        v["something_new"] = serde_json::json!(1);
        std::fs::write(ContainerRecord::path(d.path()), v.to_string()).unwrap();
        assert!(ContainerRecord::load(d.path()).is_some());
        let text = serde_json::to_string(&sample()).unwrap().to_ascii_lowercase();
        assert!(!text.contains("password") && !text.contains("secret") && !text.contains("licen"));
    }
}
