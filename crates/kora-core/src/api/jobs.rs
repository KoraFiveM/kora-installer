//! Slow operations as jobs: run on a std thread, collect events, keep the result for a while.
//! No async runtime: a web server polls [`JobRegistry::get`] or reads a [`JobRegistry::subscribe`]
//! channel (from `spawn_blocking`, or a small bridge thread).
//!
//! [`JobRegistry::new`] keeps everything in memory (tests, demos). [`JobRegistry::durable`] also writes
//! every job, its (bounded) events and its end to the Kora home's store (see [`crate::store::jobs`]), so a
//! panel restart keeps the history, and a job that was running when the panel died is marked
//! `interrupted` with a guided problem the next time a panel starts. A failing journal never stops a job:
//! the job runs on and is simply not remembered.

use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ops::Op;
use crate::problem::Problem;

/// Why a job failed: a [`Problem`] plus the extra fields some failures add (an install adds `kept`,
/// `log_path`, `failed_step`, `resume`). A job body returns `Err(JobFailure)`; a plain [`Problem`],
/// `String` or `&str` converts into one. It is kept (and stored, see [`crate::store::jobs`]) as the
/// JSON text of one flat object: the Problem's fields, then the extras.
#[derive(Debug, Clone, PartialEq)]
pub struct JobFailure {
    pub problem: Problem,
    pub extra: serde_json::Map<String, Value>,
}

impl JobFailure {
    pub fn new(problem: Problem) -> Self {
        Self { problem, extra: serde_json::Map::new() }
    }
    /// Add an extra field (never overrides the Problem's own fields).
    pub fn with(mut self, key: &str, value: Value) -> Self {
        if !matches!(key, "code" | "message" | "details" | "actions") {
            self.extra.insert(key.to_string(), value);
        }
        self
    }
    /// The flat JSON object the panel and the store use.
    pub fn to_json(&self) -> Value {
        let mut v = self.problem.to_json();
        if let Value::Object(m) = &mut v {
            // The wire shape leaves `actions` out when there are none (as job errors always did).
            if self.problem.actions.is_empty() {
                m.remove("actions");
            }
            for (k, x) in &self.extra {
                m.insert(k.clone(), x.clone());
            }
        }
        v
    }
    /// The text kept in `JobSnapshot::error` and the store.
    pub fn to_text(&self) -> String {
        self.to_json().to_string()
    }
}

impl From<Problem> for JobFailure {
    fn from(p: Problem) -> Self {
        Self::new(p)
    }
}
impl From<String> for JobFailure {
    fn from(s: String) -> Self {
        Self::new(Problem::generic(s))
    }
}
impl From<&str> for JobFailure {
    fn from(s: &str) -> Self {
        Self::new(Problem::generic(s))
    }
}

/// What a job body returns.
pub type JobResult = Result<Value, JobFailure>;

/// Finished jobs kept at most this many (oldest dropped first).
pub const MAX_FINISHED: usize = 200;
/// Finished jobs older than this are dropped.
pub const KEEP_FINISHED: Duration = Duration::from_secs(6 * 3600);
/// Events stored per job (later ones are still sent to subscribers).
pub const MAX_EVENTS: usize = 5000;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct JobId(pub String);

impl std::fmt::Display for JobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Running,
    Done,
    Failed,
    /// The panel stopped while it was running (found after a restart).
    Interrupted,
}

/// Another mutating job is already running for this server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub server: String,
    pub running: JobId,
}

impl std::fmt::Display for Conflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "another operation is already running for '{}' (job {})", self.server, self.running)
    }
}

impl std::error::Error for Conflict {}

/// What a subscriber receives.
#[derive(Debug, Clone, PartialEq)]
pub enum JobMessage {
    Event(Value),
    Done(Value),
    Failed(String),
}

fn op_name<S: serde::Serializer>(op: &Op, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(op.name())
}

/// `GET /jobs/{id}`.
#[derive(Debug, Clone, Serialize)]
pub struct JobSnapshot {
    pub id: JobId,
    #[serde(serialize_with = "op_name")]
    pub op: Op,
    pub server: Option<String>,
    pub state: JobState,
    pub events: Vec<Value>,
    pub result: Option<Value>,
    pub error: Option<String>,
    /// Unix seconds.
    pub started: u64,
    pub ended: Option<u64>,
}

struct Job {
    snap: JobSnapshot,
    mutating: bool,
    finished_at: Option<Instant>,
    seq: u64,
    subscribers: Vec<Sender<JobMessage>>,
}

#[derive(Default)]
struct Inner {
    jobs: HashMap<String, Job>,
    next_seq: u64,
}

/// Events not yet written, per job.
struct Buf {
    pending: Vec<Value>,
    /// Position of the first pending event.
    next: usize,
    last_flush: Instant,
}

/// The durable side of a registry (see the module docs).
struct Journal {
    home: PathBuf,
    instance: String,
    pid: u32,
    bufs: Mutex<HashMap<String, Buf>>,
}

/// Events are written at once when the job has been quiet for this long, else in small batches.
const FLUSH_AFTER: Duration = Duration::from_millis(300);
const FLUSH_COUNT: usize = 50;

impl Journal {
    fn store(&self) -> Option<crate::store::Store> {
        crate::store::Store::open(&self.home).ok()
    }

    fn begin(&self, id: &str, op: Op, server: Option<&str>) {
        if let Some(s) = self.store() {
            let _ = crate::store::jobs::insert(&s, id, op.name(), server, self.pid, &self.instance);
        }
        self.bufs.lock().unwrap_or_else(|p| p.into_inner()).insert(id.to_string(), Buf { pending: vec![], next: 0, last_flush: Instant::now() - FLUSH_AFTER });
    }

    fn event(&self, id: &str, v: &Value) {
        let batch = {
            let mut g = self.bufs.lock().unwrap_or_else(|p| p.into_inner());
            let Some(b) = g.get_mut(id) else { return };
            b.pending.push(v.clone());
            if b.pending.len() >= FLUSH_COUNT || b.last_flush.elapsed() >= FLUSH_AFTER {
                b.last_flush = Instant::now();
                let first = b.next;
                b.next += b.pending.len();
                Some((first, std::mem::take(&mut b.pending)))
            } else {
                None
            }
        };
        if let Some((first, events)) = batch {
            if let Some(s) = self.store() {
                let _ = crate::store::jobs::append_events(&s, id, first, &events);
            }
        }
    }

    fn end(&self, id: &str, state: &str, result: Option<&Value>, error: Option<&str>) {
        let rest = self.bufs.lock().unwrap_or_else(|p| p.into_inner()).remove(id);
        if let Some(s) = self.store() {
            if let Some(b) = rest {
                let _ = crate::store::jobs::append_events(&s, id, b.next, &b.pending);
            }
            let _ = crate::store::jobs::finish(&s, id, state, result, error);
        }
    }
}

fn op_from_name(name: &str) -> Option<Op> {
    Op::ALL.iter().copied().find(|o| o.name() == name)
}

fn snapshot_of(j: crate::store::jobs::StoredJob) -> Option<JobSnapshot> {
    let state = match j.state.as_str() {
        "running" => JobState::Running, // another live panel's job (a dead owner's was already marked interrupted)
        "done" => JobState::Done,
        "failed" => JobState::Failed,
        _ => JobState::Interrupted,
    };
    Some(JobSnapshot { id: JobId(j.id), op: op_from_name(&j.op)?, server: j.server, state, events: j.events, result: j.result, error: j.error, started: j.started, ended: j.ended })
}

fn unix_now() -> u64 {
    crate::monitor::now_unix()
}

/// All jobs of this process. Cheap to clone; clones share the same jobs.
#[derive(Clone, Default)]
pub struct JobRegistry {
    inner: Arc<Mutex<Inner>>,
    journal: Option<Arc<Journal>>,
}

/// Given to the job body to report progress.
pub struct JobHandle {
    reg: JobRegistry,
    id: JobId,
}

/// Is this process alive? A seam for [`JobRegistry::durable_with`].
pub type Alive = dyn Fn(u32) -> bool;

impl JobHandle {
    pub fn id(&self) -> &JobId {
        &self.id
    }

    /// Append an event (kept for replay and sent to live subscribers).
    pub fn event(&self, value: Value) {
        // Written down BEFORE anyone can see it, so whatever a viewer saw is also in the store if the panel dies now.
        if let Some(j) = &self.reg.journal {
            j.event(&self.id.0, &value);
        }
        let mut g = self.reg.lock();
        if let Some(j) = g.jobs.get_mut(&self.id.0) {
            if j.snap.events.len() < MAX_EVENTS {
                j.snap.events.push(value.clone());
            }
            j.subscribers.retain(|s| s.send(JobMessage::Event(value.clone())).is_ok());
        }
    }
}

fn new_id() -> String {
    let mut b = [0u8; 8];
    if getrandom::fill(&mut b).is_err() {
        // Extremely unlikely; fall back to time so ids still differ.
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        b.copy_from_slice(&(t as u64).to_le_bytes());
    }
    hex::encode(b)
}

fn panic_text(p: Box<dyn std::any::Any + Send>) -> String {
    let m = p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_else(|| "unknown".into());
    format!("the operation crashed: {m}")
}

impl JobRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// A registry that also keeps its jobs in the store of this process's Kora home. On creation it marks
    /// the jobs of dead processes as interrupted.
    pub fn durable() -> Self {
        Self::durable_in(crate::installer::registry::kora_home())
    }

    pub fn durable_in(home: PathBuf) -> Self {
        Self::durable_with(home, &crate::store::jobs::pid_alive)
    }

    /// [`durable_in`](Self::durable_in) with the "is this process alive" check given (tests).
    pub fn durable_with(home: PathBuf, alive: &Alive) -> Self {
        let mut instance = [0u8; 8];
        let _ = getrandom::fill(&mut instance);
        let journal = Journal { home, instance: hex::encode(instance), pid: std::process::id(), bufs: Mutex::new(HashMap::new()) };
        if let Some(s) = journal.store() {
            let _ = crate::store::jobs::recover(&s, alive, journal.pid, &journal.instance);
        }
        Self { inner: Arc::default(), journal: Some(Arc::new(journal)) }
    }

    /// The jobs started by a PREVIOUS run that are still known (newest first, without events): what a Jobs
    /// page shows after a restart, together with this run's own. At most `limit`.
    pub fn history(&self, limit: usize) -> Vec<JobSnapshot> {
        let mut all: Vec<JobSnapshot> = self
            .list()
            .into_iter()
            .map(|mut j| {
                j.events.clear();
                j
            })
            .collect();
        if let Some(s) = self.journal.as_ref().and_then(|j| j.store()) {
            for j in crate::store::jobs::list(&s, limit).unwrap_or_default() {
                if !all.iter().any(|a| a.id.0 == j.id) {
                    if let Some(snap) = snapshot_of(j) {
                        all.push(snap);
                    }
                }
            }
        }
        all.sort_by_key(|a| std::cmp::Reverse(a.started));
        all.truncate(limit);
        all
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Run `f` as a job. Read-only ops ([`Op::is_read_only_job`]) may overlap with anything; any other
    /// op is refused while another mutating job of the same server is running.
    pub fn spawn<F>(&self, op: Op, server: Option<String>, f: F) -> Result<JobId, Conflict>
    where
        F: FnOnce(&JobHandle) -> JobResult + Send + 'static,
    {
        self.spawn_with(op, server, !op.is_read_only_job(), f)
    }

    /// Like [`spawn`](Self::spawn) with the "mutating" decision made by the caller.
    pub fn spawn_with<F>(&self, op: Op, server: Option<String>, mutating: bool, f: F) -> Result<JobId, Conflict>
    where
        F: FnOnce(&JobHandle) -> JobResult + Send + 'static,
    {
        let id = JobId(new_id());
        {
            let mut g = self.lock();
            if mutating {
                if let Some(name) = &server {
                    let busy = g.jobs.values().find(|j| j.mutating && j.snap.state == JobState::Running && j.snap.server.as_deref() == Some(name.as_str()));
                    if let Some(j) = busy {
                        return Err(Conflict { server: name.clone(), running: j.snap.id.clone() });
                    }
                }
            }
            Self::prune(&mut g);
            g.next_seq += 1;
            let seq = g.next_seq;
            let snap = JobSnapshot { id: id.clone(), op, server, state: JobState::Running, events: vec![], result: None, error: None, started: unix_now(), ended: None };
            g.jobs.insert(id.0.clone(), Job { snap, mutating, finished_at: None, seq, subscribers: vec![] });
        }
        if let Some(j) = &self.journal {
            let server = self.lock().jobs.get(&id.0).and_then(|j| j.snap.server.clone());
            j.begin(&id.0, op, server.as_deref());
        }
        let reg = self.clone();
        let handle = JobHandle { reg: self.clone(), id: id.clone() };
        let spawned = std::thread::Builder::new().name(format!("job-{}", op.name())).spawn(move || {
            let out = match catch_unwind(AssertUnwindSafe(|| f(&handle))) {
                Ok(r) => r,
                Err(p) => Err(panic_text(p).into()),
            };
            reg.finish(&handle.id, out);
        });
        if let Err(e) = spawned {
            self.finish(&id, Err(format!("could not start the operation: {e}").into()));
        }
        Ok(id)
    }

    fn finish(&self, id: &JobId, out: JobResult) {
        let out = out.map_err(|f| f.to_text());
        // Written down BEFORE anyone is told it ended, so whoever sees "done" can rely on the store having it.
        if let Some(journal) = &self.journal {
            match &out {
                Ok(v) => journal.end(&id.0, "done", Some(v), None),
                Err(e) => journal.end(&id.0, "failed", None, Some(e)),
            }
        }
        let mut g = self.lock();
        let Some(j) = g.jobs.get_mut(&id.0) else { return };
        let msg = match out {
            Ok(v) => {
                j.snap.state = JobState::Done;
                j.snap.result = Some(v.clone());
                JobMessage::Done(v)
            }
            Err(e) => {
                j.snap.state = JobState::Failed;
                j.snap.error = Some(e.clone());
                JobMessage::Failed(e)
            }
        };
        j.finished_at = Some(Instant::now());
        j.snap.ended = Some(unix_now());
        for s in j.subscribers.drain(..) {
            let _ = s.send(msg.clone());
        }
    }

    fn prune(g: &mut Inner) {
        let now = Instant::now();
        g.jobs.retain(|_, j| j.finished_at.is_none_or(|t| now.duration_since(t) < KEEP_FINISHED));
        let mut finished: Vec<(u64, String)> = g.jobs.iter().filter(|(_, j)| j.finished_at.is_some()).map(|(k, j)| (j.seq, k.clone())).collect();
        if finished.len() >= MAX_FINISHED {
            finished.sort();
            let drop = finished.len() + 1 - MAX_FINISHED;
            for (_, k) in finished.into_iter().take(drop) {
                g.jobs.remove(&k);
            }
        }
    }

    /// A job of this run, or (with a journal) one remembered from before.
    pub fn get(&self, id: &str) -> Option<JobSnapshot> {
        if let Some(j) = self.lock().jobs.get(id).map(|j| j.snap.clone()) {
            return Some(j);
        }
        let s = self.journal.as_ref()?.store()?;
        crate::store::jobs::get(&s, id).ok()?.and_then(snapshot_of)
    }

    /// All jobs, oldest first.
    pub fn list(&self) -> Vec<JobSnapshot> {
        let g = self.lock();
        let mut v: Vec<(u64, JobSnapshot)> = g.jobs.values().map(|j| (j.seq, j.snap.clone())).collect();
        v.sort_by_key(|(s, _)| *s);
        v.into_iter().map(|(_, s)| s).collect()
    }

    /// The events so far, and a channel that carries the rest and then `Done` / `Failed`.
    /// For a finished job the channel already holds the final message.
    pub fn subscribe(&self, id: &str) -> Option<(Vec<Value>, Receiver<JobMessage>)> {
        let mut g = self.lock();
        let Some(j) = g.jobs.get_mut(id) else {
            drop(g);
            // Not of this run: replay what was remembered.
            let snap = self.get(id)?;
            let (tx, rx) = channel();
            match snap.state {
                JobState::Done => {
                    let _ = tx.send(JobMessage::Done(snap.result.clone().unwrap_or(Value::Null)));
                }
                // another live panel's job: its end is not ours to report (the stream just closes)
                JobState::Running => {}
                _ => {
                    let _ = tx.send(JobMessage::Failed(snap.error.clone().unwrap_or_default()));
                }
            }
            return Some((snap.events, rx));
        };
        let (tx, rx) = channel();
        let past = j.snap.events.clone();
        match j.snap.state {
            JobState::Running => j.subscribers.push(tx),
            JobState::Done => {
                let _ = tx.send(JobMessage::Done(j.snap.result.clone().unwrap_or(Value::Null)));
            }
            JobState::Failed | JobState::Interrupted => {
                let _ = tx.send(JobMessage::Failed(j.snap.error.clone().unwrap_or_default()));
            }
        }
        Some((past, rx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::mpsc::RecvTimeoutError;

    const T: Duration = Duration::from_secs(5);

    /// A job body that blocks until `gate` is sent to.
    fn gated() -> (Sender<()>, impl FnOnce(&JobHandle) -> JobResult + Send + 'static) {
        let (tx, rx) = channel::<()>();
        (tx, move |_h: &JobHandle| {
            let _ = rx.recv_timeout(T);
            Ok(json!("gated"))
        })
    }

    fn wait_done(reg: &JobRegistry, id: &JobId) -> JobSnapshot {
        let (_, rx) = reg.subscribe(&id.0).unwrap();
        loop {
            match rx.recv_timeout(T).unwrap() {
                JobMessage::Event(_) => {}
                _ => return reg.get(&id.0).unwrap(),
            }
        }
    }

    #[test]
    fn runs_and_records_result_and_events_in_order() {
        let reg = JobRegistry::new();
        let id = reg
            .spawn(Op::UpdateApply, Some("a".into()), |h| {
                h.event(json!(1));
                h.event(json!(2));
                h.event(json!(3));
                Ok(json!({"ok": true}))
            })
            .unwrap();
        let s = wait_done(&reg, &id);
        assert_eq!(s.state, JobState::Done);
        assert_eq!(s.events, [json!(1), json!(2), json!(3)]);
        assert_eq!(s.result, Some(json!({"ok": true})));
        assert_eq!(s.error, None);
        assert_eq!(s.op, Op::UpdateApply);
        assert_eq!(s.server.as_deref(), Some("a"));
    }

    #[test]
    fn failure_is_recorded() {
        let reg = JobRegistry::new();
        let id = reg.spawn(Op::ServerControl, Some("a".into()), |_| Err("nope".into())).unwrap();
        let s = wait_done(&reg, &id);
        assert_eq!((s.state, s.error.as_deref(), s.result), (JobState::Failed, Some(r#"{"code":"failed","message":"nope"}"#), None));
    }

    #[test]
    fn a_panic_becomes_failed() {
        let reg = JobRegistry::new();
        let id = reg.spawn(Op::ServerControl, Some("a".into()), |_| panic!("kaboom")).unwrap();
        let s = wait_done(&reg, &id);
        assert_eq!(s.state, JobState::Failed);
        assert!(s.error.unwrap().contains("kaboom"));
        // The slot is free again.
        assert!(reg.spawn(Op::ServerControl, Some("a".into()), |_| Ok(Value::Null)).is_ok());
    }

    #[test]
    fn late_subscriber_gets_a_replay_then_the_rest() {
        let reg = JobRegistry::new();
        let (go, body) = {
            let (tx, rx) = channel::<()>();
            (tx, move |h: &JobHandle| {
                h.event(json!("early"));
                let _ = rx.recv_timeout(T);
                h.event(json!("late"));
                Ok(json!("end"))
            })
        };
        let id = reg.spawn(Op::UpdateApply, Some("a".into()), body).unwrap();
        // Wait until the first event is stored.
        while reg.get(&id.0).unwrap().events.is_empty() {
            std::thread::yield_now();
        }
        let (past, rx) = reg.subscribe(&id.0).unwrap();
        assert_eq!(past, [json!("early")]);
        go.send(()).unwrap();
        assert_eq!(rx.recv_timeout(T).unwrap(), JobMessage::Event(json!("late")));
        assert_eq!(rx.recv_timeout(T).unwrap(), JobMessage::Done(json!("end")));
        // And a subscriber after the end sees everything at once.
        let (past, rx) = reg.subscribe(&id.0).unwrap();
        assert_eq!(past, [json!("early"), json!("late")]);
        assert_eq!(rx.recv_timeout(T).unwrap(), JobMessage::Done(json!("end")));
        assert!(matches!(rx.recv_timeout(Duration::from_millis(50)), Err(RecvTimeoutError::Disconnected)));
    }

    #[test]
    fn failed_job_notifies_subscribers() {
        let reg = JobRegistry::new();
        let (gate, body) = gated();
        let id = reg.spawn(Op::ServerDelete, Some("a".into()), move |h| body(h).and(Err("bad".into()))).unwrap();
        let (_, rx) = reg.subscribe(&id.0).unwrap();
        gate.send(()).unwrap();
        assert_eq!(rx.recv_timeout(T).unwrap(), JobMessage::Failed(r#"{"code":"failed","message":"bad"}"#.into()));
    }

    #[test]
    fn one_mutating_job_per_server() {
        let reg = JobRegistry::new();
        let (gate, body) = gated();
        let first = reg.spawn(Op::ServerControl, Some("a".into()), body).unwrap();
        let err = reg.spawn(Op::UpdateApply, Some("a".into()), |_| Ok(Value::Null)).unwrap_err();
        assert_eq!(err, Conflict { server: "a".into(), running: first.clone() });
        assert!(err.to_string().contains("'a'"));
        // Another server is fine; a read-only job on the same server is exempt.
        let (g2, b2) = gated();
        assert!(reg.spawn(Op::ServerControl, Some("b".into()), b2).is_ok());
        let (g3, b3) = gated();
        assert!(reg.spawn(Op::UpdatePreview, Some("a".into()), b3).is_ok());
        // spawn_with can override the decision.
        assert!(reg.spawn_with(Op::UpdateApply, Some("a".into()), false, |_| Ok(Value::Null)).is_ok());
        // Jobs without a server never conflict.
        assert!(reg.spawn(Op::ServersScan, None, |_| Ok(Value::Null)).is_ok());
        gate.send(()).unwrap();
        wait_done(&reg, &first);
        assert!(reg.spawn(Op::UpdateApply, Some("a".into()), |_| Ok(Value::Null)).is_ok());
        let _ = (g2.send(()), g3.send(()));
    }

    #[test]
    fn finished_jobs_are_capped() {
        let reg = JobRegistry::new();
        let mut first = None;
        for i in 0..(MAX_FINISHED + 20) {
            let id = reg.spawn(Op::ServersScan, None, move |_| Ok(json!(i))).unwrap();
            wait_done(&reg, &id);
            first.get_or_insert(id);
        }
        assert!(reg.list().len() <= MAX_FINISHED);
        assert!(reg.get(&first.unwrap().0).is_none(), "the oldest was dropped");
    }

    #[test]
    fn unknown_ids_and_snapshot_json() {
        let reg = JobRegistry::new();
        assert!(reg.get("nope").is_none() && reg.subscribe("nope").is_none());
        let id = reg.spawn(Op::UpdatePreview, Some("a".into()), |_| Ok(json!(7))).unwrap();
        assert_eq!(id.0.len(), 16);
        let s = wait_done(&reg, &id);
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["op"], "update_preview");
        assert_eq!(v["state"], "done");
        assert_eq!(v["id"], id.0.as_str());
        assert_eq!(v["result"], 7);
        assert!(v["error"].is_null());
    }

    #[test]
    fn clones_share_jobs() {
        let a = JobRegistry::new();
        let b = a.clone();
        let id = a.spawn(Op::ServersScan, None, |_| Ok(Value::Null)).unwrap();
        wait_done(&b, &id);
        assert!(b.get(&id.0).is_some());
    }

    // ---- durable jobs -------------------------------------------------------------------------

    fn home() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn alive_none(_: u32) -> bool {
        false
    }

    #[test]
    fn a_finished_job_is_still_there_after_a_restart_with_its_events_result_and_error() {
        let h = home();
        let reg = JobRegistry::durable_in(h.path().to_path_buf());
        let ok = reg
            .spawn(Op::UpdateApply, Some("a".into()), |j| {
                j.event(json!({"step": "one", "status": "done"}));
                j.event(json!({"step": "two", "status": "done"}));
                Ok(json!({"updated": 2}))
            })
            .unwrap();
        wait_done(&reg, &ok);
        let bad = reg.spawn(Op::ServerControl, Some("b".into()), |_| Err("the server did not start".into())).unwrap();
        wait_done(&reg, &bad);
        drop(reg);

        let after = JobRegistry::durable_in(h.path().to_path_buf());
        let s = after.get(&ok.0).unwrap();
        assert_eq!((s.state, s.op, s.server.as_deref()), (JobState::Done, Op::UpdateApply, Some("a")));
        assert_eq!(s.events, [json!({"step": "one", "status": "done"}), json!({"step": "two", "status": "done"})]);
        assert_eq!(s.result, Some(json!({"updated": 2})));
        assert!(s.started > 0 && s.ended.is_some());
        let b = after.get(&bad.0).unwrap();
        assert_eq!((b.state, b.error.as_deref()), (JobState::Failed, Some(r#"{"code":"failed","message":"the server did not start"}"#)));
        // the event stream of an old job replays and ends
        let (past, rx) = after.subscribe(&ok.0).unwrap();
        assert_eq!(past.len(), 2);
        assert_eq!(rx.recv_timeout(T).unwrap(), JobMessage::Done(json!({"updated": 2})));
        let hist = after.history(10);
        assert_eq!(hist.len(), 2);
        assert!(hist.iter().all(|j| j.events.is_empty()), "history rows are light");
        assert!(after.get("nope").is_none());
    }

    #[test]
    fn a_job_that_was_running_when_the_panel_died_is_interrupted_with_a_guided_problem() {
        let h = home();
        {
            let reg = JobRegistry::durable_in(h.path().to_path_buf());
            let (_gate, body) = gated();
            let id = reg.spawn(Op::UpdateApply, Some("a".into()), move |j| {
                j.event(json!({"step": "x", "status": "running"}));
                body(j)
            });
            let id = id.unwrap();
            while reg.get(&id.0).unwrap().events.is_empty() {
                std::thread::yield_now();
            }
            // the "panel dies": nothing more is written. Another registry (the next panel) opens the store.
            let store = crate::store::Store::open(h.path()).unwrap();
            // pretend the owner is another, now dead, process
            store
                .write(|tx| {
                    tx.execute("UPDATE jobs SET owner_pid = 999999, owner_instance = 'old'", [])?;
                    Ok(())
                })
                .unwrap();
            let next = JobRegistry::durable_with(h.path().to_path_buf(), &alive_none);
            let s = next.get(&id.0).unwrap();
            assert_eq!(s.state, JobState::Interrupted);
            let e: Value = serde_json::from_str(s.error.as_deref().unwrap()).unwrap();
            assert_eq!(e["code"], "job.interrupted");
            assert!(e["message"].as_str().unwrap().contains("did not finish"));
            assert!(e.get("resume").is_none(), "an update is not resumable");
            assert!(!s.events.is_empty(), "what it had shown is kept");
            let (_, rx) = next.subscribe(&id.0).unwrap();
            assert!(matches!(rx.recv_timeout(T).unwrap(), JobMessage::Failed(_)));
        }
    }

    #[test]
    fn a_running_job_of_another_live_panel_is_left_alone() {
        let h = home();
        let reg = JobRegistry::durable_in(h.path().to_path_buf());
        let (gate, body) = gated();
        let id = reg.spawn(Op::UpdateApply, Some("a".into()), body).unwrap();
        // the job belongs to ANOTHER panel process that is alive (a second panel on the same Kora home)
        let store = crate::store::Store::open(h.path()).unwrap();
        store
            .write(|tx| {
                tx.execute("UPDATE jobs SET owner_pid = ?1 WHERE id = ?2", rusqlite::params![std::process::id() as i64 + 1, id.0])?;
                Ok(())
            })
            .unwrap();
        let third = JobRegistry::durable_with(h.path().to_path_buf(), &|pid| pid == std::process::id() + 1);
        assert_eq!(third.get(&id.0).unwrap().state, JobState::Running, "its owner is alive");
        gate.send(()).unwrap();
        wait_done(&reg, &id);
    }

    #[test]
    fn an_installs_stopped_state_carries_on_after_the_interruption() {
        let h = home();
        let paused = crate::installer::paused::PausedStore::at(h.path().join("paused-installs"));
        let reg = JobRegistry::durable_in(h.path().to_path_buf());
        let (_gate, body) = gated();
        let id = reg.spawn(Op::InstallStart, Some("Dev Den".into()), body).unwrap();
        let mut req: crate::installer::InstallRequest = serde_json::from_value(json!({
            "mode": "production", "name": "Dev Den", "dir": "/srv/dev-den", "hostname": "Dev Den", "license_key_name": "Main",
            "ports": {"game": 30120, "txadmin": 40120}, "txadmin_public": false, "resources": "All", "db": "InstallIfMissing",
            "service": true, "firewall": true, "dry_run": false, "start": true
        }))
        .unwrap();
        req.license_key = Some("cfxk_SECRET_SECRET_SECRET".into());
        assert!(paused.save_in_progress(&id.0, &req, false, &[]).unwrap());
        assert!(paused.list().is_empty(), "not offered while it runs");
        let store = crate::store::Store::open(h.path()).unwrap();
        store
            .write(|tx| {
                tx.execute("UPDATE jobs SET owner_pid = 999999, owner_instance = 'old'", [])?;
                Ok(())
            })
            .unwrap();
        let next = JobRegistry::durable_with(h.path().to_path_buf(), &alive_none);
        let s = next.get(&id.0).unwrap();
        assert_eq!(s.state, JobState::Interrupted);
        let e: Value = serde_json::from_str(s.error.as_deref().unwrap()).unwrap();
        assert_eq!(e["resume"], true);
        assert!(e["message"].as_str().unwrap().contains("Carry on"));
        let list = paused.list();
        assert_eq!(list.len(), 1, "now it can be continued");
        assert_eq!(list[0].req.license_key, None, "never the key");
    }

    #[test]
    fn events_are_bounded_and_secrets_never_reach_the_rows() {
        let h = home();
        let reg = JobRegistry::durable_in(h.path().to_path_buf());
        let id = reg
            .spawn(Op::InstallStart, Some("a".into()), |j| {
                for i in 0..(crate::store::jobs::MAX_EVENTS + 250) {
                    j.event(json!({"step": "chatty", "n": i}));
                }
                j.event(json!({"step": "x", "detail": "key cfxk_ABCDEFGHIJKLMNOP_1234 was used", "license_key": "cfxk_ABCDEFGHIJKLMNOP_1234", "db_admin": {"password": "hunter2"}}));
                Ok(json!({"admin_password": "hunter2", "ok": true}))
            })
            .unwrap();
        wait_done(&reg, &id);
        let next = JobRegistry::durable_in(h.path().to_path_buf());
        let s = next.get(&id.0).unwrap();
        assert_eq!(s.events.len(), crate::store::jobs::MAX_EVENTS, "the newest are kept");
        assert_eq!(s.events.last().unwrap()["step"], "x");
        assert_eq!(s.events[0]["n"], 251 - 1 + 1, "the oldest were dropped");
        let text = serde_json::to_string(&s).unwrap();
        assert!(!text.contains("hunter2") && !text.contains("ABCDEFGHIJKLMNOP"), "{text}");
        assert_eq!(s.result.unwrap()["ok"], true);
        // and not in the database file either
        let bytes: Vec<u8> = std::fs::read_dir(crate::store::store_dir(h.path())).unwrap().flatten().filter(|e| e.path().is_file()).flat_map(|e| std::fs::read(e.path()).unwrap()).collect();
        assert!(!bytes.windows(7).any(|w| w == b"hunter2") && !bytes.windows(16).any(|w| w == b"ABCDEFGHIJKLMNOP"));
    }

    #[test]
    fn old_finished_jobs_are_dropped_beyond_the_limit() {
        let h = home();
        let s = crate::store::Store::open(h.path()).unwrap();
        for i in 0..(crate::store::jobs::KEEP_JOBS + 20) {
            crate::store::jobs::insert(&s, &format!("job{i:04}"), "servers_scan", None, 1, "x").unwrap();
            crate::store::jobs::finish(&s, &format!("job{i:04}"), "done", Some(&json!(i)), None).unwrap();
        }
        let all = crate::store::jobs::list(&s, 10_000).unwrap();
        assert!(all.len() <= crate::store::jobs::KEEP_JOBS + 1, "{}", all.len());
        assert!(crate::store::jobs::get(&s, "job0000").unwrap().is_none());
        assert!(crate::store::jobs::get(&s, &format!("job{:04}", crate::store::jobs::KEEP_JOBS + 19)).unwrap().is_some());
    }

    #[test]
    fn a_broken_store_never_stops_a_job() {
        // the Kora home is a file: nothing can be opened there
        let h = home();
        let file = h.path().join("not-a-folder");
        std::fs::write(&file, "x").unwrap();
        let reg = JobRegistry::durable_in(file);
        let id = reg.spawn(Op::ServersScan, None, |_| Ok(json!(1))).unwrap();
        assert_eq!(wait_done(&reg, &id).state, JobState::Done);
    }
}
