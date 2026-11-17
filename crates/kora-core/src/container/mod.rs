//! Container mode (Linux): a game server that runs in a container instead of as a systemd service.
//!
//! # What it is
//! One container per server, run by Podman (rootless where possible) or Docker, driven through the
//! engine's command line (`podman` / `docker`, found through [`crate::sysexe`], run through
//! [`crate::service::CommandRunner`]). There is no engine API client and no shell: every command is an
//! argument list, built by the pure functions in [`spec`] and [`engine`] and asserted exactly in tests.
//!
//! * **One mechanism: direct `run`, not quadlet or compose.** Quadlet needs Podman 4.4+ and a different
//!   control path from Docker; compose is another program. Driving `run -d` with a restart policy works the
//!   same on both engines, needs no generated file on disk (nothing to get out of step with the engine), and
//!   makes every command testable as an argument list. Start at boot is the engine's: Docker's restart
//!   policy; for rootless Podman the stock `podman-restart.service` user unit plus user lingering
//!   (`loginctl enable-linger`), which [`detect`] checks and explains.
//! * **The folder is mounted at the same path inside.** `/srv/kora/main` on the host is `/srv/kora/main` in
//!   the container, so every absolute path already written into `server.cfg`, txAdmin's data path, the live
//!   console's command file and the resources' own configuration is valid in both places. Nothing is
//!   rewritten when a server changes mode.
//! * **The FXServer build is not in the image.** Kora downloads and verifies it as always into
//!   `<server>/artifacts/` and the container runs it from there on a pinned Alpine base ([`pins`]). FXServer
//!   carries its own musl loader, so the base only supplies an empty root, certificates and `/tmp`.
//! * **Files stay the person's.** The container runs as the folder owner's uid and gid. Rootless Podman adds
//!   `--userns=keep-id`, so that uid is the same inside and out; Kora can update, back up and delete
//!   everything the server writes without any administrator step. (There is no service account in this mode.)
//! * **The database** is by default a MariaDB container on a private per-server network, its data in a named
//!   volume, its image pinned, its login generated like today's and kept in `database.json`. It is reached by
//!   name from the game container and by `exec` from Kora (backups run the dump inside it, with the password
//!   in the engine client's environment, never on a command line).
//! * **Logs and console:** the engine keeps the container's output; a small follower ([`logs`]) copies it to
//!   `<server>/.kora/console.log` where the Console and Logs tabs read it. The live console's command file
//!   is in the bind-mounted folder, so it works unchanged.
//!
//! # Threat model: what container mode does and does not isolate
//! Isolated: the FXServer process (and the resources' scripts, which are the largest risk on a game server)
//! cannot see the rest of the host's files, processes, other users or the administrator's tools; it runs
//! without capabilities (`--cap-drop ALL`), with `no-new-privileges`, never `--privileged`, never the host
//! network unless the person chooses it for a stated reason, and with the host's user namespace remapped by
//! rootless Podman. Not isolated: the **server folder is bind-mounted read-write**, so everything in it is
//! reachable from inside: `secrets.cfg` (licence key), `database.json` (database login), txAdmin's data
//! (admin hashes), and the whole `.git` history. A malicious resource can therefore read the licence key and
//! the database login exactly as it can today. The published ports are reachable from the network like any
//! open port (the game port on the chosen address; txAdmin's on loopback only, the panel's gateway reaches
//! it there). Docker publishes ports by writing its own firewall rules, which bypass `ufw`/`firewalld`
//! rules; the reachability checklist says so. A **root** engine (Docker) is a larger trust grant than a
//! rootless one: membership of the `docker` group is root-equivalent (a member can start a container that
//! mounts `/`), and Kora says so before offering it. Rootless Podman needs no administrator rights at all.
//! Images are pinned by digest ([`pins`]); a pin only moves with a Kora release.
//!
//! # Layout of this module
//! `pins` images · `engine` the engine, its commands and what they print · `spec` the run commands ·
//! `detect` the System check · `lifecycle` state, start, stop, remove · `logs` console copy ·
//! `dbc` the database container and `exec` runner · `record` what a server's `kora-container.json` says.

pub mod backup;
pub mod dbc;
pub mod detect;
pub mod engine;
pub mod lifecycle;
pub mod logs;
pub mod pins;
pub mod reach;
pub mod record;
pub mod setup;
pub mod spec;

pub use engine::{Engine, EngineKind, EngineRunner};
pub use record::{ContainerRecord, DbRecord};

use serde::{Deserialize, Serialize};

/// How a server runs. Servers made before container mode are all `Service`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    /// A system service (systemd unit / Windows service), or started by hand.
    #[default]
    Service,
    /// A container run by Podman or Docker.
    Container,
}

/// What an install asks for when it chooses container mode. Secret-free.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ContainerRequest {
    /// `podman` or `docker`; `None` = the best one found (rootless Podman first).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<EngineKind>,
    /// Address the game port is published on (`None` = every address).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bind: Option<String>,
    /// Memory limit in MiB (`None` = none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_mb: Option<u32>,
    /// CPU limit in tenths of a core, e.g. 20 = 2.0 cores (`None` = none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_tenths: Option<u32>,
}

/// Slug used in container, network and volume names: lowercase letters, digits, `-`, `_`.
pub fn slug_of(name: &str) -> String {
    crate::installer::slug(name)
}

/// `kora-<slug>`: the game container (and, by analogy, what the service would have been called).
pub fn container_name(slug: &str) -> String {
    format!("kora-{slug}")
}

pub fn db_container_name(slug: &str) -> String {
    format!("kora-{slug}-db")
}

pub fn network_name(slug: &str) -> String {
    format!("kora-{slug}-net")
}

pub fn volume_name(slug: &str) -> String {
    format!("kora-{slug}-dbdata")
}

/// The label every object Kora makes carries: `kora.server=<slug>`. Kora lists, inspects and removes by
/// this label and by exact name, never by pattern on an image or a name prefix.
pub const LABEL_SERVER: &str = "kora.server";
pub const LABEL_ROLE: &str = "kora.role";

/// Problem codes of this area.
pub mod codes {
    pub const NO_ENGINE: &str = "container.no_engine";
    pub const ENGINE_CANNOT_RUN: &str = "container.cannot_run";
    pub const NO_SUBUID: &str = "container.no_subuid";
    pub const CGROUP_V1: &str = "container.cgroup_v1";
    pub const NOT_LINGERING: &str = "container.not_lingering";
    pub const SOCKET_PERMISSION: &str = "container.socket_permission";
    pub const SELINUX: &str = "container.selinux";
    pub const PULL_FAILED: &str = "container.pull_failed";
    pub const START_FAILED: &str = "container.start_failed";
    pub const NAME_TAKEN: &str = "container.name_taken";
    pub const DB_NOT_RUNNING: &str = "container.db_not_running";
}
