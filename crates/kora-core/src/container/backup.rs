//! Backups of a container server: the database tools run inside the database container.
//!
//! [`ToolSet::for_server`] answers "which tools make this server's backup": the host's own database client and
//! dump program for a service-mode server (exactly as before), or, for a container server with a database
//! container, [`ExecRunner`](super::dbc::ExecRunner) with the image's own `mariadb` / `mariadb-dump`. The
//! dump therefore needs no database client on the host at all. The login in `database.json` names the container
//! on the private network; for the tools inside it, [`database_of`](crate::backup::create::database_of) maps it
//! to `127.0.0.1:3306` (see [`dbc::tool_credentials`](super::dbc::tool_credentials)).
//!
//! The folder's `kora-container.json` is part of every backup (it is next to `kora-server.json`), so a restore on
//! the same machine keeps the mode and a move to another machine knows what the server was (see `backup::import`).

use std::path::Path;

use super::dbc::{self, ExecRunner};
use super::record::ContainerRecord;
use crate::backup::dbdump::StreamRunner;
use crate::backup::Tools;

/// The tools of one server's backup, owned so a [`Tools`] can borrow from them.
pub struct ToolSet {
    db: Box<dyn StreamRunner>,
    git: crate::git::SystemRunner,
    client: String,
    dump: Option<String>,
    pub container_db: bool,
}

impl ToolSet {
    /// The host's tools.
    pub fn host() -> ToolSet {
        let client = crate::installer::db_client();
        let dump = crate::backup::dbdump::dump_program(&client, &crate::backup::dbdump::program_runs);
        ToolSet { db: Box::new(crate::db::SystemRunner), git: crate::git::SystemRunner, client, dump, container_db: false }
    }

    /// The tools inside the database container of the server whose record is `rec`.
    pub fn in_container(rec: &ContainerRecord) -> Option<ToolSet> {
        let exec = ExecRunner::new(rec, crate::db::SystemRunner)?;
        Some(ToolSet { db: Box::new(exec), git: crate::git::SystemRunner, client: dbc::CLIENT.into(), dump: Some(dbc::DUMP.into()), container_db: true })
    }

    /// For the server `registry_name` in `dir`: the container's tools when it has a database container and its
    /// record is its own, else the host's.
    pub fn for_server(registry_name: &str, dir: &Path) -> ToolSet {
        match super::lifecycle::target_of(registry_name, dir) {
            Ok(Some(t)) => Self::in_container(&t.rec).unwrap_or_else(Self::host),
            _ => Self::host(),
        }
    }

    pub fn tools(&self) -> Tools<'_> {
        Tools { db: self.db.as_ref(), git: &self.git, db_client: self.client.clone(), dump_program: self.dump.clone(), now: crate::monitor::now_unix() }
    }
}

#[cfg(test)]
mod tests {
    use super::super::record::tests::sample;
    use super::*;

    #[test]
    fn a_container_server_with_a_database_container_uses_the_images_tools() {
        let rec = sample();
        let set = ToolSet::in_container(&rec).unwrap();
        let t = set.tools();
        assert_eq!((t.db_client.as_str(), t.dump_program.as_deref()), ("mariadb", Some("mariadb-dump")));
        assert!(set.container_db);
        let mut none = rec;
        none.db = None;
        assert!(ToolSet::in_container(&none).is_none(), "no database container: the host's tools");
    }
}
