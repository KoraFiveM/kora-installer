//! The schema, as forward-only numbered migrations.
//!
//! Rules (the store refuses to start if they are broken, see the tests):
//! * versions are 1, 2, 3 ... with no gaps and are never edited once released: a change is a NEW migration;
//! * there is no "down": the way back to an older Kora is `kora store export-json` plus the store backups
//!   (see `docs/store.md`);
//! * every migration runs inside the one transaction that also records it in `schema_migrations`.

pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub sql: &'static str,
}

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "state files: registry, profile, keys, stopped installs, saved answers, remote access, hub nodes",
        sql: r#"
CREATE TABLE meta (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
) WITHOUT ROWID;

-- servers.json
-- The server entry is stored as JSON (`record`), exactly what servers.json held per server, so a new field on the
-- entry needs NO migration (see `installer::registry`: "adding a field to the server entry"). Only what is looked up
-- by is a column.
CREATE TABLE servers (
    name   TEXT PRIMARY KEY NOT NULL,
    seq    INTEGER NOT NULL,
    dir    TEXT NOT NULL,
    record TEXT NOT NULL
) WITHOUT ROWID;

-- profile.json (one row per remembered field)
CREATE TABLE profile (
    field TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
) WITHOUT ROWID;

-- secrets.json. The names live here; the key VALUES live in their own table (`license_key_material`) so
-- that a later version can move just those into the system keychain (see `store::material`).
CREATE TABLE license_keys (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT NOT NULL,
    name_folded TEXT NOT NULL UNIQUE,
    added       INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE license_key_material (
    key_id   INTEGER PRIMARY KEY NOT NULL REFERENCES license_keys(id) ON DELETE CASCADE,
    material TEXT NOT NULL
);

-- paused-installs/<job>.json. Never holds a licence key (only the name of a saved one).
CREATE TABLE paused_installs (
    job         TEXT PRIMARY KEY NOT NULL,
    dir         TEXT NOT NULL,
    via_helper  INTEGER NOT NULL DEFAULT 0,
    failed_step TEXT,
    req_json    TEXT NOT NULL,
    finished_json TEXT NOT NULL,
    saved_at    INTEGER NOT NULL
) WITHOUT ROWID;

-- last-install.json (one row)
CREATE TABLE saved_answers (
    id       INTEGER PRIMARY KEY CHECK (id = 1),
    req_json TEXT NOT NULL,
    saved_at INTEGER NOT NULL DEFAULT 0
);

-- panel.json
CREATE TABLE remote_settings (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
) WITHOUT ROWID;
CREATE TABLE remote_tailscale_users (
    login TEXT PRIMARY KEY NOT NULL,
    role  TEXT NOT NULL
) WITHOUT ROWID;
CREATE TABLE remote_users (
    seq         INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT NOT NULL,
    name_folded TEXT NOT NULL UNIQUE,
    role        TEXT NOT NULL,
    salt        TEXT NOT NULL,
    hash        TEXT NOT NULL,
    created     INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE remote_hosts (
    seq  INTEGER PRIMARY KEY AUTOINCREMENT,
    host TEXT NOT NULL
);

-- hub.json
CREATE TABLE hub_nodes (
    seq   INTEGER PRIMARY KEY AUTOINCREMENT,
    id    TEXT NOT NULL UNIQUE,
    name  TEXT NOT NULL,
    url   TEXT NOT NULL,
    token TEXT NOT NULL
);
"#,
    },
    Migration {
        version: 2,
        name: "durable jobs",
        sql: r#"
CREATE TABLE jobs (
    id             TEXT PRIMARY KEY NOT NULL,
    op             TEXT NOT NULL,
    server         TEXT,
    state          TEXT NOT NULL,      -- running | done | failed | interrupted
    started        INTEGER NOT NULL,
    ended          INTEGER,
    result_json    TEXT,
    error          TEXT,               -- exactly what the job returned (a packed Problem or a JSON error) or the interruption's Problem
    owner_pid      INTEGER NOT NULL DEFAULT 0,
    owner_instance TEXT NOT NULL DEFAULT ''
);
CREATE INDEX jobs_started ON jobs(started);
CREATE TABLE job_events (
    job_id     TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    n          INTEGER NOT NULL,
    event_json TEXT NOT NULL,
    PRIMARY KEY (job_id, n)
) WITHOUT ROWID;
-- An install that is running right now (and so is not offered as "carry on" yet); a restart that finds its job
-- unfinished turns it into a stopped install that can be continued.
ALTER TABLE paused_installs ADD COLUMN in_progress INTEGER NOT NULL DEFAULT 0;
"#,
    },
    Migration {
        version: 3,
        name: "audit log",
        sql: r#"
-- Append-only: UPDATE and DELETE are refused except by the retention sweep (see `store::audit`).
CREATE TABLE audit_log (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    at         INTEGER NOT NULL,
    op         TEXT NOT NULL,
    server     TEXT,
    actor_kind TEXT NOT NULL,          -- local | remote | cli
    actor_name TEXT,
    provider   TEXT,
    role       TEXT,
    result     TEXT NOT NULL,          -- ok | failed | refused
    detail     TEXT
);
CREATE INDEX audit_at ON audit_log(at);
CREATE TRIGGER audit_no_update BEFORE UPDATE ON audit_log BEGIN SELECT RAISE(ABORT, 'the audit log is append-only'); END;
CREATE TRIGGER audit_delete_only_old BEFORE DELETE ON audit_log
WHEN OLD.at >= CAST(COALESCE((SELECT value FROM meta WHERE key = 'audit_sweep_before'), '0') AS INTEGER)
BEGIN SELECT RAISE(ABORT, 'the audit log is append-only'); END;
"#,
    },
    Migration {
        version: 4,
        name: "documents: small Kora-home state files that move into the store",
        sql: r#"
-- One row per former `<kora home>/<name>.json` file, kept as the JSON it was (see `store::documents` and the
-- `EXTRA_HOME_FILES` list in `store::legacy`): a state file with no table of its own needs no migration to move in.
CREATE TABLE documents (
    name    TEXT PRIMARY KEY NOT NULL,
    json    TEXT NOT NULL,
    updated INTEGER NOT NULL DEFAULT 0
) WITHOUT ROWID;
"#,
    },
];

pub fn latest() -> u32 {
    MIGRATIONS.last().map_or(0, |m| m.version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_consecutive_from_one() {
        for (i, m) in MIGRATIONS.iter().enumerate() {
            assert_eq!(m.version as usize, i + 1, "{}", m.name);
        }
    }

    #[test]
    fn every_migration_applies_on_an_empty_database() {
        let c = rusqlite::Connection::open_in_memory().unwrap();
        for m in MIGRATIONS {
            c.execute_batch(m.sql).unwrap_or_else(|e| panic!("migration {} failed: {e}", m.version));
        }
    }
}
