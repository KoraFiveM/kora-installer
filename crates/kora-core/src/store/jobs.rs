//! Durable jobs: the rows behind `api::JobRegistry` when it is made with [`crate::api::JobRegistry::durable`].
//!
//! A job record (`jobs`), its events (`job_events`, bounded: the newest [`MAX_EVENTS`] are kept) and its
//! final result or error survive a panel restart, so the Jobs page shows history afterwards. A job that
//! was running when the panel died is marked `interrupted` the next time a panel starts, with a Problem
//! that says what happened and what to do (and `resume: true` when the operation can be continued, as
//! installs can).
//!
//! # Secrets
//! The same rule as the live job stream: events, results and errors never carry a licence key or a
//! password. As a second line of defence [`redact`] runs on everything that is written (key-shaped
//! text and values of fields called `license_key`, `password`, `token`, `secret` are replaced).
//! A pasted, unsaved key lives in the panel's memory only, never here.
//!
//! # Which process owns a running job
//! Every row says which process (pid) and which registry instance started it. A registry that starts
//! marks rows as interrupted only when their process is gone, or when the pid is its own but the
//! instance is another one (the panel was restarted and the pid reused), so two panels sharing one Kora
//! home never break each other's jobs.

use rusqlite::Connection;
use serde_json::Value;

use crate::problem::Problem;

use super::Store;

/// Newest events kept per job.
pub const MAX_EVENTS: usize = 1000;
/// Finished jobs kept (oldest dropped first).
pub const KEEP_JOBS: usize = 300;
/// Finished jobs older than this many seconds are dropped.
pub const KEEP_SECS: u64 = 30 * 24 * 3600;

pub const CODE_INTERRUPTED: &str = "job.interrupted";

/// A job as stored.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredJob {
    pub id: String,
    pub op: String,
    pub server: Option<String>,
    pub state: String,
    pub started: u64,
    pub ended: Option<u64>,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub events: Vec<Value>,
}

/// Keys whose values are never written.
fn secret_name(k: &str) -> bool {
    let k = k.to_ascii_lowercase();
    ["license_key", "licence_key", "password", "token", "secret", "authorization"].iter().any(|s| k.contains(s)) && !k.ends_with("_name") && k != "token_set"
}

/// `cfxk_...` licence keys (and anything shaped like one) inside text.
fn redact_text(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if s[i..].starts_with("cfxk_") {
            let mut j = i + 5;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_' || b[j] == b'-') {
                j += 1;
            }
            out.push_str("<hidden key>");
            i = j;
        } else {
            let ch = s[i..].chars().next().unwrap_or(' ');
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// A copy of `v` with secrets replaced.
pub fn redact(v: &Value) -> Value {
    match v {
        Value::String(s) => Value::String(redact_text(s)),
        Value::Array(a) => Value::Array(a.iter().map(redact).collect()),
        Value::Object(m) => Value::Object(m.iter().map(|(k, x)| (k.clone(), if secret_name(k) && !x.is_null() && !x.is_boolean() { Value::String("<hidden>".into()) } else { redact(x) })).collect()),
        other => other.clone(),
    }
}

fn now() -> i64 {
    crate::monitor::now_unix() as i64
}

pub fn insert(s: &Store, id: &str, op: &str, server: Option<&str>, pid: u32, instance: &str) -> Result<(), Problem> {
    s.write(|tx| {
        tx.execute(
            "INSERT OR REPLACE INTO jobs (id, op, server, state, started, owner_pid, owner_instance) VALUES (?1, ?2, ?3, 'running', ?4, ?5, ?6)",
            rusqlite::params![id, op, server, now(), pid, instance],
        )?;
        prune(tx)?;
        Ok(())
    })
}

/// Append events `first..first+events.len()` (their position in the job), keeping only the newest [`MAX_EVENTS`].
pub fn append_events(s: &Store, id: &str, first: usize, events: &[Value]) -> Result<(), Problem> {
    if events.is_empty() {
        return Ok(());
    }
    s.write(|tx| {
        let mut st = tx.prepare("INSERT OR REPLACE INTO job_events (job_id, n, event_json) VALUES (?1, ?2, ?3)")?;
        for (i, e) in events.iter().enumerate() {
            st.execute(rusqlite::params![id, (first + i) as i64, redact(e).to_string()])?;
        }
        let newest = (first + events.len()) as i64;
        tx.execute("DELETE FROM job_events WHERE job_id = ?1 AND n < ?2", rusqlite::params![id, newest - MAX_EVENTS as i64])?;
        Ok(())
    })
}

/// Record how a job ended. `state`: `done` or `failed`.
pub fn finish(s: &Store, id: &str, state: &str, result: Option<&Value>, error: Option<&str>) -> Result<(), Problem> {
    s.write(|tx| {
        tx.execute(
            "UPDATE jobs SET state = ?2, ended = ?3, result_json = ?4, error = ?5 WHERE id = ?1",
            rusqlite::params![id, state, now(), result.map(|r| redact(r).to_string()), error.map(redact_text)],
        )?;
        Ok(())
    })
}

fn row(c: &Connection, id: &str) -> Result<Option<StoredJob>, Problem> {
    let r = c.query_row("SELECT id, op, server, state, started, ended, result_json, error FROM jobs WHERE id = ?1", [id], |r| {
        Ok(StoredJob {
            id: r.get(0)?,
            op: r.get(1)?,
            server: r.get(2)?,
            state: r.get(3)?,
            started: r.get::<_, i64>(4)? as u64,
            ended: r.get::<_, Option<i64>>(5)?.map(|e| e as u64),
            result: r.get::<_, Option<String>>(6)?.and_then(|t| serde_json::from_str(&t).ok()),
            error: r.get(7)?,
            events: vec![],
        })
    });
    match r {
        Ok(j) => Ok(Some(j)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// One job with its events.
pub fn get(s: &Store, id: &str) -> Result<Option<StoredJob>, Problem> {
    s.read(|c| {
        let Some(mut j) = row(c, id)? else { return Ok(None) };
        let mut st = c.prepare("SELECT event_json FROM job_events WHERE job_id = ?1 ORDER BY n")?;
        j.events = st.query_map([id], |r| r.get::<_, String>(0))?.filter_map(|t| t.ok().and_then(|t| serde_json::from_str(&t).ok())).collect();
        Ok(Some(j))
    })
}

/// The newest jobs first, without their events.
pub fn list(s: &Store, limit: usize) -> Result<Vec<StoredJob>, Problem> {
    s.read(|c| {
        let ids: Vec<String> = c.prepare("SELECT id FROM jobs ORDER BY started DESC, rowid DESC LIMIT ?1")?.query_map([limit as i64], |r| r.get(0))?.collect::<Result<_, _>>()?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(j) = row(c, &id)? {
                out.push(j);
            }
        }
        Ok(out)
    })
}

/// Drop finished jobs beyond the limits (their events go with them).
fn prune(c: &Connection) -> Result<(), Problem> {
    let cutoff = now() - KEEP_SECS as i64;
    c.execute("DELETE FROM jobs WHERE state <> 'running' AND ended IS NOT NULL AND ended < ?1", [cutoff])?;
    c.execute("DELETE FROM jobs WHERE state <> 'running' AND id NOT IN (SELECT id FROM jobs WHERE state <> 'running' ORDER BY started DESC, rowid DESC LIMIT ?1)", [KEEP_JOBS as i64])?;
    Ok(())
}

/// Is a process with this id running? (A seam: tests say who is alive.)
pub fn pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // SAFETY: signal 0 only checks that the process exists and may be signalled.
        let r = unsafe { libc::kill(pid as i32, 0) };
        r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        crate::sysexe::command("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains(&format!("\"{pid}\"")))
            .unwrap_or(true)
    }
}

/// The Problem a person reads for a job that was running when the panel stopped.
pub fn interrupted_problem(op: &str, server: Option<&str>, resumable: bool) -> Problem {
    let what = server.map(|s| format!(" for '{s}'")).unwrap_or_default();
    let message = if op == "install_start" || op == "install_resume" {
        if resumable {
            format!("Kora Panel was closed, or the computer restarted, while this install{what} was running, so it did not finish. Kora kept your answers: open Install and choose Carry on, and it continues from the last step that finished.")
        } else {
            format!(
                "Kora Panel was closed, or the computer restarted, while this install{what} was running, so it did not finish. Start the install again with the same folder and it continues from the last step that finished. If you pasted a licence key and did not save it, paste it again: Kora never writes a pasted key down."
            )
        }
    } else {
        format!("Kora Panel was closed, or the computer restarted, while this was running{what}, so it did not finish. Look at the server's state, then run it again if you still need it.")
    };
    Problem::new(CODE_INTERRUPTED, message)
}

/// The error text stored and shown for an interrupted job (the Problem as JSON, with `resume` when it can carry on).
pub fn interrupted_error(op: &str, server: Option<&str>, resumable: bool) -> String {
    let mut v = interrupted_problem(op, server, resumable).to_json();
    if resumable {
        v["resume"] = Value::Bool(true);
    }
    v.to_string()
}

/// Mark the running jobs whose process is gone as interrupted, and let stopped installs they left carry on.
/// Returns the ids. `alive` says whether a pid is running.
pub fn recover(s: &Store, alive: &dyn Fn(u32) -> bool, my_pid: u32, my_instance: &str) -> Result<Vec<String>, Problem> {
    s.write(|tx| {
        let rows: Vec<(String, String, Option<String>, u32, String)> = tx
            .prepare("SELECT id, op, server, owner_pid, owner_instance FROM jobs WHERE state = 'running'")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, i64>(3)? as u32, r.get(4)?)))?
            .collect::<Result<_, _>>()?;
        let mut done = Vec::new();
        for (id, op, server, pid, instance) in rows {
            let gone = if pid == my_pid { instance != my_instance } else { !alive(pid) };
            if !gone {
                continue;
            }
            let resumable: bool = tx.query_row("SELECT COUNT(*) FROM paused_installs WHERE job = ?1", [&id], |r| r.get::<_, i64>(0))? > 0;
            tx.execute("UPDATE paused_installs SET in_progress = 0 WHERE job = ?1", [&id])?;
            tx.execute("UPDATE jobs SET state = 'interrupted', ended = ?2, error = ?3 WHERE id = ?1", rusqlite::params![id, now(), interrupted_error(&op, server.as_deref(), resumable)])?;
            done.push(id);
        }
        Ok(done)
    })
}
