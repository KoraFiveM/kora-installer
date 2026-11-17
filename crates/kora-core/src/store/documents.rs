//! Small Kora-home state files that live in the store as JSON documents.
//!
//! A state file that used to be `<kora home>/<name>.json` and has no shape that needs queries (for example
//! `reach-consent.json`) is kept here under its name, as the same JSON, so it moves into the store without a table or a
//! migration of its own. The import of the old file is the [`super::legacy::EXTRA_HOME_FILES`] entry for it.
//!
//! ```ignore
//! let s = Store::open_home()?;
//! documents::put(&s, "reach-consent", &serde_json::json!({"granted": true}))?;
//! let doc = documents::get(&s, "reach-consent")?;           // Option<Value>
//! documents::update(&s, "reach-consent", |d| { d["granted"] = false.into(); Ok(()) })?;   // read-modify-write in ONE transaction
//! ```
//! Never put a secret in a document that is not meant to be private: the whole store is private to the person who runs
//! Kora, but documents are shown by `kora store export-json`.

use rusqlite::Connection;
use serde_json::Value;

use crate::problem::Problem;

use super::Store;

/// A document name: lower case letters, digits, `-` and `_`, 1 to 60 characters.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 60 && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

fn check(name: &str) -> Result<(), Problem> {
    if valid_name(name) {
        Ok(())
    } else {
        Err(Problem::generic(format!("\"{name}\" is not a valid name for a stored document.")))
    }
}

pub(crate) fn read(c: &Connection, name: &str) -> Result<Option<Value>, Problem> {
    check(name)?;
    match c.query_row("SELECT json FROM documents WHERE name = ?1", [name], |r| r.get::<_, String>(0)) {
        Ok(t) => serde_json::from_str(&t).map(Some).map_err(|e| Problem::generic(format!("The stored document \"{name}\" cannot be read ({e}). Nothing was changed."))),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub(crate) fn write(c: &Connection, name: &str, doc: &Value) -> Result<(), Problem> {
    check(name)?;
    c.execute(
        "INSERT INTO documents (name, json, updated) VALUES (?1, ?2, ?3) ON CONFLICT(name) DO UPDATE SET json = excluded.json, updated = excluded.updated",
        rusqlite::params![name, doc.to_string(), crate::monitor::now_unix() as i64],
    )?;
    Ok(())
}

pub fn get(s: &Store, name: &str) -> Result<Option<Value>, Problem> {
    s.read(|c| read(c, name))
}

pub fn put(s: &Store, name: &str, doc: &Value) -> Result<(), Problem> {
    s.write(|tx| write(tx, name, doc))
}

pub fn remove(s: &Store, name: &str) -> Result<(), Problem> {
    check(name)?;
    s.write(|tx| {
        tx.execute("DELETE FROM documents WHERE name = ?1", [name])?;
        Ok(())
    })
}

/// Read (`Value::Null` when there is none), change and write in ONE transaction.
pub fn update<R>(s: &Store, name: &str, f: impl FnOnce(&mut Value) -> Result<R, Problem>) -> Result<R, Problem> {
    s.write(|tx| {
        let mut doc = read(tx, name)?.unwrap_or(Value::Null);
        let r = f(&mut doc)?;
        write(tx, name, &doc)?;
        Ok(r)
    })
}
