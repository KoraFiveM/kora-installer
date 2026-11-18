//! The audit log: an append-only record of what was changed, when, by whom.
//!
//! One row per state-changing operation: when, which [`crate::api::Op`] (by name), which server, who
//! (the person on this computer, a remote identity with its provider and role, or a `kora` command) and
//! the result. Never a secret and never a request body: `detail` is a short note Kora wrote itself (a
//! job id, an error CODE), cut to [`MAX_DETAIL`] characters and passed through the same redaction as job
//! events ([`super::jobs::redact`]).
//!
//! # Append-only
//! The table refuses `UPDATE` and `DELETE` with triggers (see the schema). The one exception is the
//! retention sweep ([`sweep`]): it lifts the delete guard for rows OLDER than a cut-off, in its own
//! transaction, and puts the guard back before it commits. Retention: [`RETENTION_DAYS`] days and at most
//! [`MAX_ROWS`] rows. This is a record for the person who runs Kora, not tamper-proof evidence: someone
//! who can open the database file can change it. It is private to that person like the rest of the store.
//!
//! Writing is best effort ([`record_quiet`]): a failing audit never stops the operation it describes.

use std::path::Path;

use rusqlite::{Connection, ToSql};
use serde::Serialize;

use crate::problem::Problem;

use super::{meta_set, Store};

pub const RETENTION_DAYS: u64 = 365;
pub const MAX_ROWS: usize = 100_000;
pub const MAX_DETAIL: usize = 200;

/// Who did it.
pub mod actor {
    pub const LOCAL: &str = "local";
    pub const REMOTE: &str = "remote";
    pub const CLI: &str = "cli";
}

/// How it ended.
pub mod result {
    pub const OK: &str = "ok";
    pub const FAILED: &str = "failed";
    /// Not allowed (the role was too low, remote actions were off, ...).
    pub const REFUSED: &str = "refused";
    /// A slow operation was started; a later row says how it ended.
    pub const STARTED: &str = "started";
}

/// A row to write.
#[derive(Debug, Clone, Default)]
pub struct New {
    pub op: String,
    pub server: Option<String>,
    pub actor_kind: String,
    pub actor_name: Option<String>,
    pub provider: Option<String>,
    pub role: Option<String>,
    pub result: String,
    pub detail: Option<String>,
}

/// A row as stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Row {
    pub id: i64,
    pub at: u64,
    pub op: String,
    pub server: Option<String>,
    pub actor_kind: String,
    pub actor_name: Option<String>,
    pub provider: Option<String>,
    pub role: Option<String>,
    pub result: String,
    pub detail: Option<String>,
}

/// What to look for.
#[derive(Debug, Clone, Default)]
pub struct Query {
    /// At most this many (clamped to 1..=500).
    pub limit: usize,
    /// Only rows older than this id (for "show more").
    pub before: Option<i64>,
    pub server: Option<String>,
    pub op: Option<String>,
    /// Part of the actor's name.
    pub actor: Option<String>,
    pub result: Option<String>,
    /// Only rows at or after this time (unix seconds).
    pub since: Option<u64>,
}

fn clean(s: &str, max: usize) -> String {
    let t: String = s.chars().filter(|c| !c.is_control()).take(max).collect();
    super::jobs::redact(&serde_json::Value::String(t)).as_str().unwrap_or_default().to_string()
}

/// Write one row.
pub fn record(s: &Store, n: &New) -> Result<(), Problem> {
    s.write(|tx| {
        tx.execute(
            "INSERT INTO audit_log (at, op, server, actor_kind, actor_name, provider, role, result, detail) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                crate::monitor::now_unix() as i64,
                clean(&n.op, 60),
                n.server.as_deref().map(|v| clean(v, 120)),
                clean(&n.actor_kind, 12),
                n.actor_name.as_deref().map(|v| clean(v, 80)),
                n.provider.as_deref().map(|v| clean(v, 20)),
                n.role.as_deref().map(|v| clean(v, 20)),
                clean(&n.result, 12),
                n.detail.as_deref().map(|v| clean(v, MAX_DETAIL)),
            ],
        )?;
        // Now and then (every 200th row) the old ones are swept.
        let id = tx.last_insert_rowid();
        if id % 200 == 0 {
            sweep_in(tx, RETENTION_DAYS, MAX_ROWS)?;
        }
        Ok(())
    })
}

/// [`record`] in the Kora home, never failing the caller.
pub fn record_quiet(home: &Path, n: &New) {
    if let Ok(s) = Store::open(home) {
        let _ = record(&s, n);
    }
}

fn row_of(r: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    Ok(Row {
        id: r.get(0)?,
        at: r.get::<_, i64>(1)? as u64,
        op: r.get(2)?,
        server: r.get(3)?,
        actor_kind: r.get(4)?,
        actor_name: r.get(5)?,
        provider: r.get(6)?,
        role: r.get(7)?,
        result: r.get(8)?,
        detail: r.get(9)?,
    })
}

/// The newest rows first.
pub fn list(s: &Store, q: &Query) -> Result<Vec<Row>, Problem> {
    s.read(|c| {
        let mut sql = String::from("SELECT id, at, op, server, actor_kind, actor_name, provider, role, result, detail FROM audit_log WHERE 1 = 1");
        let mut args: Vec<Box<dyn ToSql>> = Vec::new();
        let mut add = |clause: &str, v: Box<dyn ToSql>| {
            args.push(v);
            sql.push_str(&format!(" AND {clause} ?{}", args.len()));
        };
        if let Some(b) = q.before {
            add("id <", Box::new(b));
        }
        if let Some(v) = &q.server {
            add("server =", Box::new(v.clone()));
        }
        if let Some(v) = &q.op {
            add("op =", Box::new(v.clone()));
        }
        if let Some(v) = &q.result {
            add("result =", Box::new(v.clone()));
        }
        if let Some(v) = q.since {
            add("at >=", Box::new(v as i64));
        }
        if let Some(v) = &q.actor {
            // Part of the name, ignoring case; `%` and `_` in the search are plain characters.
            let like = format!("%{}%", v.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
            args.push(Box::new(like));
            sql.push_str(&format!(" AND actor_name LIKE ?{} ESCAPE '\\'", args.len()));
        }
        args.push(Box::new(q.limit.clamp(1, 500) as i64));
        sql.push_str(&format!(" ORDER BY id DESC LIMIT ?{}", args.len()));
        let mut st = c.prepare(&sql)?;
        let rows = st.query_map(rusqlite::params_from_iter(args.iter().map(|a| a.as_ref())), row_of)?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
}

/// Delete what is past retention. Returns how many rows went.
pub fn sweep(s: &Store, days: u64, max_rows: usize) -> Result<usize, Problem> {
    s.write(|tx| sweep_in(tx, days, max_rows))
}

fn sweep_in(c: &Connection, days: u64, max_rows: usize) -> Result<usize, Problem> {
    let now = crate::monitor::now_unix() as i64;
    let mut cutoff = now - (days as i64) * 24 * 3600;
    // Too many rows: everything older than the (max_rows)th newest goes as well.
    if let Ok(at) = c.query_row("SELECT at FROM audit_log ORDER BY id DESC LIMIT 1 OFFSET ?1", [max_rows as i64], |r| r.get::<_, i64>(0)) {
        cutoff = cutoff.max(at + 1);
    }
    meta_set(c, "audit_sweep_before", &cutoff.to_string())?;
    let n = c.execute("DELETE FROM audit_log WHERE at < ?1", [cutoff])?;
    // The guard goes back on before the transaction ends.
    meta_set(c, "audit_sweep_before", "0")?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let h = tempfile::tempdir().unwrap();
        let s = Store::open(h.path()).unwrap();
        (h, s)
    }

    fn row(op: &str, server: Option<&str>, who: &str, result: &str) -> New {
        New {
            op: op.into(),
            server: server.map(String::from),
            actor_kind: actor::REMOTE.into(),
            actor_name: Some(who.into()),
            provider: Some("tailscale".into()),
            role: Some("admin".into()),
            result: result.into(),
            detail: None,
        }
    }

    #[test]
    fn rows_are_written_and_listed_newest_first_with_filters() {
        let (_h, s) = store();
        record(&s, &row("server_control", Some("live"), "sam@example.com", result::OK)).unwrap();
        record(&s, &row("update_apply", Some("live"), "pat", result::STARTED)).unwrap();
        record(&s, &row("key_add", None, "pat", result::REFUSED)).unwrap();
        let all = list(&s, &Query { limit: 50, ..Default::default() }).unwrap();
        assert_eq!(all.iter().map(|r| r.op.as_str()).collect::<Vec<_>>(), ["key_add", "update_apply", "server_control"]);
        assert_eq!((all[2].actor_name.as_deref(), all[2].provider.as_deref(), all[2].role.as_deref(), all[2].result.as_str()), (Some("sam@example.com"), Some("tailscale"), Some("admin"), "ok"));
        assert!(all[0].at > 0);
        let by_server = list(&s, &Query { limit: 50, server: Some("live".into()), ..Default::default() }).unwrap();
        assert_eq!(by_server.len(), 2);
        assert_eq!(list(&s, &Query { limit: 50, actor: Some("SAM".into()), ..Default::default() }).unwrap().len(), 1);
        assert_eq!(list(&s, &Query { limit: 50, actor: Some("%".into()), ..Default::default() }).unwrap().len(), 0, "wildcards are plain characters");
        assert_eq!(list(&s, &Query { limit: 50, result: Some("refused".into()), ..Default::default() }).unwrap().len(), 1);
        let page = list(&s, &Query { limit: 1, before: Some(all[0].id), ..Default::default() }).unwrap();
        assert_eq!(page[0].op, "update_apply");
    }

    #[test]
    fn nothing_secret_is_written_and_text_is_cut() {
        let (_h, s) = store();
        let mut n = row("key_add", None, "pat", result::FAILED);
        n.detail = Some(format!("the key cfxk_SECRETSECRETSECRET_1234 failed\nnew line {}", "x".repeat(500)));
        record(&s, &n).unwrap();
        let r = &list(&s, &Query { limit: 5, ..Default::default() }).unwrap()[0];
        let d = r.detail.as_deref().unwrap();
        assert!(!d.contains("SECRETSECRET") && d.contains("<hidden: licence key>") && !d.contains('\n') && d.chars().count() <= MAX_DETAIL, "{d}");
    }

    #[test]
    fn it_is_append_only() {
        let (_h, s) = store();
        record(&s, &row("server_control", Some("a"), "pat", result::OK)).unwrap();
        let upd = s.write(|tx| {
            tx.execute("UPDATE audit_log SET result = 'ok2'", [])?;
            Ok(())
        });
        assert!(upd.is_err(), "no editing");
        let del = s.write(|tx| {
            tx.execute("DELETE FROM audit_log", [])?;
            Ok(())
        });
        assert!(del.is_err(), "no deleting");
        // switching the guard off by hand (outside the sweep) for the newest rows still does not help
        let sneaky = s.write(|tx| {
            meta_set(tx, "audit_sweep_before", "1")?;
            tx.execute("DELETE FROM audit_log", [])?;
            Ok(())
        });
        assert!(sneaky.is_err());
        assert_eq!(list(&s, &Query { limit: 5, ..Default::default() }).unwrap().len(), 1);
    }

    #[test]
    fn retention_removes_old_rows_and_only_those_and_puts_the_guard_back() {
        let (_h, s) = store();
        // an old row, written by hand (the API never writes the past)
        s.write(|tx| {
            tx.execute("INSERT INTO audit_log (at, op, actor_kind, result) VALUES (?1, 'old_thing', 'local', 'ok')", [crate::monitor::now_unix() as i64 - 400 * 24 * 3600])?;
            Ok(())
        })
        .unwrap();
        record(&s, &row("fresh", None, "pat", result::OK)).unwrap();
        assert_eq!(sweep(&s, RETENTION_DAYS, MAX_ROWS).unwrap(), 1);
        let left = list(&s, &Query { limit: 10, ..Default::default() }).unwrap();
        assert_eq!(left.iter().map(|r| r.op.as_str()).collect::<Vec<_>>(), ["fresh"]);
        let after = s.write(|tx| {
            tx.execute("DELETE FROM audit_log", [])?;
            Ok(())
        });
        assert!(after.is_err(), "append-only again after the sweep");
    }

    #[test]
    fn too_many_rows_are_trimmed_oldest_first() {
        let (_h, s) = store();
        s.write(|tx| {
            for i in 0..30 {
                tx.execute("INSERT INTO audit_log (at, op, actor_kind, result) VALUES (?1, ?2, 'local', 'ok')", rusqlite::params![crate::monitor::now_unix() as i64 - 1000 + i, format!("op{i:02}")])?;
            }
            Ok(())
        })
        .unwrap();
        sweep(&s, RETENTION_DAYS, 10).unwrap();
        let left = list(&s, &Query { limit: 100, ..Default::default() }).unwrap();
        assert_eq!(left.len(), 10);
        assert_eq!(left[0].op, "op29");
        assert_eq!(left[9].op, "op20");
    }
}
