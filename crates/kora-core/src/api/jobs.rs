//! Slow operations as jobs: run on a std thread, collect events, keep the result for a while.
//! No async runtime: a web server polls [`JobRegistry::get`] or reads a [`JobRegistry::subscribe`]
//! channel (from `spawn_blocking`, or a small bridge thread).

use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ops::Op;

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

/// All jobs of this process. Cheap to clone; clones share the same jobs.
#[derive(Clone, Default)]
pub struct JobRegistry {
    inner: Arc<Mutex<Inner>>,
}

/// Given to the job body to report progress.
pub struct JobHandle {
    reg: JobRegistry,
    id: JobId,
}

impl JobHandle {
    pub fn id(&self) -> &JobId {
        &self.id
    }

    /// Append an event (kept for replay and sent to live subscribers).
    pub fn event(&self, value: Value) {
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

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Run `f` as a job. Read-only ops ([`Op::is_read_only_job`]) may overlap with anything; any other
    /// op is refused while another mutating job of the same server is running.
    pub fn spawn<F>(&self, op: Op, server: Option<String>, f: F) -> Result<JobId, Conflict>
    where
        F: FnOnce(&JobHandle) -> Result<Value, String> + Send + 'static,
    {
        self.spawn_with(op, server, !op.is_read_only_job(), f)
    }

    /// Like [`spawn`](Self::spawn) with the "mutating" decision made by the caller.
    pub fn spawn_with<F>(&self, op: Op, server: Option<String>, mutating: bool, f: F) -> Result<JobId, Conflict>
    where
        F: FnOnce(&JobHandle) -> Result<Value, String> + Send + 'static,
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
            let snap = JobSnapshot { id: id.clone(), op, server, state: JobState::Running, events: vec![], result: None, error: None };
            g.jobs.insert(id.0.clone(), Job { snap, mutating, finished_at: None, seq, subscribers: vec![] });
        }
        let reg = self.clone();
        let handle = JobHandle { reg: self.clone(), id: id.clone() };
        let spawned = std::thread::Builder::new().name(format!("job-{}", op.name())).spawn(move || {
            let out = match catch_unwind(AssertUnwindSafe(|| f(&handle))) {
                Ok(r) => r,
                Err(p) => Err(panic_text(p)),
            };
            reg.finish(&handle.id, out);
        });
        if let Err(e) = spawned {
            self.finish(&id, Err(format!("could not start the operation: {e}")));
        }
        Ok(id)
    }

    fn finish(&self, id: &JobId, out: Result<Value, String>) {
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

    pub fn get(&self, id: &str) -> Option<JobSnapshot> {
        self.lock().jobs.get(id).map(|j| j.snap.clone())
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
        let j = g.jobs.get_mut(id)?;
        let (tx, rx) = channel();
        let past = j.snap.events.clone();
        match j.snap.state {
            JobState::Running => j.subscribers.push(tx),
            JobState::Done => {
                let _ = tx.send(JobMessage::Done(j.snap.result.clone().unwrap_or(Value::Null)));
            }
            JobState::Failed => {
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
    fn gated() -> (Sender<()>, impl FnOnce(&JobHandle) -> Result<Value, String> + Send + 'static) {
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
        assert_eq!((s.state, s.error.as_deref(), s.result), (JobState::Failed, Some("nope"), None));
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
        assert_eq!(rx.recv_timeout(T).unwrap(), JobMessage::Failed("bad".into()));
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
}
