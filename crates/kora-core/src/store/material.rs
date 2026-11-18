//! The seam for "where do saved licence keys' values live".
//!
//! `keystore` keeps names, order and timestamps in `license_keys` and asks a [`KeyMaterial`] for the
//! secret value of one key. Today that is [`SqliteKeyMaterial`] (the `license_key_material` table inside
//! the same private database, in the same transaction as the name). A later workstream that moves the
//! values into the system keychain implements this trait over the keychain (it can ignore the transaction)
//! and changes the one line in `keystore::material()` that picks the implementation; nothing else in the
//! keystore, the panel or the CLI needs to know. A keychain implementation must also migrate what is in
//! `license_key_material` and then empty that table.

use rusqlite::Connection;

use crate::problem::Problem;

pub trait KeyMaterial {
    /// Store (or replace) the secret for key `id`.
    fn put(&self, c: &Connection, id: i64, secret: &str) -> Result<(), Problem>;
    /// The secret for key `id`, if there is one.
    fn get(&self, c: &Connection, id: i64) -> Result<Option<String>, Problem>;
    /// Forget the secret for key `id`.
    fn delete(&self, c: &Connection, id: i64) -> Result<(), Problem>;
}

/// The values in the `license_key_material` table of the store.
pub struct SqliteKeyMaterial;

impl KeyMaterial for SqliteKeyMaterial {
    fn put(&self, c: &Connection, id: i64, secret: &str) -> Result<(), Problem> {
        c.execute("INSERT INTO license_key_material (key_id, material) VALUES (?1, ?2) ON CONFLICT(key_id) DO UPDATE SET material = excluded.material", rusqlite::params![id, secret])?;
        Ok(())
    }
    fn get(&self, c: &Connection, id: i64) -> Result<Option<String>, Problem> {
        match c.query_row("SELECT material FROM license_key_material WHERE key_id = ?1", [id], |r| r.get::<_, String>(0)) {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    fn delete(&self, c: &Connection, id: i64) -> Result<(), Problem> {
        c.execute("DELETE FROM license_key_material WHERE key_id = ?1", [id])?;
        Ok(())
    }
}
