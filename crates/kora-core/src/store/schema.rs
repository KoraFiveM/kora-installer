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
    Migration {
        version: 5,
        name: "panel HTTPS (local CA and certificate), remote sign-in (passkeys, one-time codes, API tokens), where each saved key is kept",
        sql: r#"
-- The local certificate authority and server certificate of `kora panel tls self-signed`. PRIVATE: the keys here
-- let someone impersonate the panel to a browser that trusts the CA (see `kora-web::tls::selfsigned`).
CREATE TABLE tls_local_ca (
    id       INTEGER PRIMARY KEY CHECK (id = 1),
    cert_pem TEXT NOT NULL,
    key_pem  TEXT NOT NULL,
    created  INTEGER NOT NULL
);
CREATE TABLE tls_local_leaf (
    id       INTEGER PRIMARY KEY CHECK (id = 1),
    cert_pem TEXT NOT NULL,
    key_pem  TEXT NOT NULL,
    names    TEXT NOT NULL,
    not_after INTEGER NOT NULL
);

-- The certificate `tailscale cert` issued for this computer's tailnet name (kept so a restart does not ask again).
CREATE TABLE tls_tailscale (
    id        INTEGER PRIMARY KEY CHECK (id = 1),
    name      TEXT NOT NULL,
    cert_pem  TEXT NOT NULL,
    key_pem   TEXT NOT NULL,
    not_after INTEGER NOT NULL
);

-- Per remote user: the secrets of the sign-in that replaces the token (see `kora-web::remote::auth`). `user` is the
-- lower-cased name in `remote_users`. PRIVATE: a password hash (argon2id), the TOTP secret, hashed recovery codes.
CREATE TABLE remote_user_auth (
    user            TEXT PRIMARY KEY NOT NULL,
    webauthn_id     BLOB NOT NULL,              -- the WebAuthn user handle (random, never the name)
    password_hash   TEXT,                       -- argon2id PHC string
    totp_secret     BLOB,                       -- 20 random bytes
    totp_confirmed  INTEGER NOT NULL DEFAULT 0,
    totp_last_step  INTEGER NOT NULL DEFAULT 0, -- replay protection: the last accepted 30-second step
    recovery_hashes TEXT NOT NULL DEFAULT '[]', -- JSON list of salted SHA-256 hashes, used ones removed
    failures        INTEGER NOT NULL DEFAULT 0, -- failed password/code attempts in a row (account lockout)
    locked_until    INTEGER NOT NULL DEFAULT 0
) WITHOUT ROWID;

CREATE TABLE remote_passkeys (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    user          TEXT NOT NULL,
    label         TEXT NOT NULL,
    credential_id BLOB NOT NULL UNIQUE,
    passkey_json  TEXT NOT NULL,                -- the credential's public key and counter (webauthn-rs), PRIVATE
    rp_id         TEXT NOT NULL,
    created       INTEGER NOT NULL,
    last_used     INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX remote_passkeys_user ON remote_passkeys(user);

-- One-time enrolment links: only the hash of the link's secret is stored. Single use, short lived.
CREATE TABLE remote_enrolments (
    hash       TEXT PRIMARY KEY NOT NULL,
    user       TEXT NOT NULL,
    purpose    TEXT NOT NULL,                   -- invite | reset | recover
    created_by TEXT NOT NULL,
    expires    INTEGER NOT NULL
) WITHOUT ROWID;

-- API tokens for scripts: a role, never interactive. Only a salted hash is stored; shown once.
CREATE TABLE remote_api_tokens (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    name      TEXT NOT NULL UNIQUE,
    role      TEXT NOT NULL,
    salt      TEXT NOT NULL,
    hash      TEXT NOT NULL,
    created   INTEGER NOT NULL,
    expires   INTEGER NOT NULL DEFAULT 0,       -- 0: never
    last_used INTEGER NOT NULL DEFAULT 0,
    created_by TEXT NOT NULL DEFAULT ''
);

-- Where the VALUE of each saved licence key is kept: the private table (`store`) or the system keychain.
-- A key with no row here is in `license_key_material` (everything saved before this version).
CREATE TABLE license_key_location (
    key_id INTEGER PRIMARY KEY NOT NULL REFERENCES license_keys(id) ON DELETE CASCADE,
    place  TEXT NOT NULL,                       -- store | dpapi | secret-service
    note   TEXT NOT NULL DEFAULT ''
);
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
