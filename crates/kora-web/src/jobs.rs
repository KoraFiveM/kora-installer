//! Job source abstraction + `GET /jobs/{id}` and the SSE stream.
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::{Extension, Json};
use kora_core::api::Op;
use serde_json::{json, Value};
use tokio_stream::wrappers::ReceiverStream;

use crate::error::ApiError;
use crate::remote::{roles, Caller};
use crate::security::Limited;
use crate::AppState;

pub use kora_core::api::JobMessage;

pub trait Jobs: Send + Sync {
    /// `{id, op, server, state, events, result, error}` or `None` when unknown.
    fn get(&self, id: &str) -> Option<Value>;
    /// Past events plus a receiver of later messages (ends with `Done`/`Failed`).
    fn subscribe(&self, id: &str) -> Option<(Vec<Value>, Receiver<JobMessage>)>;
    /// The operation that started the job (`None`: unknown job, or not one of ours).
    fn op(&self, id: &str) -> Option<Op> {
        let v = self.get(id)?;
        let name = v.get("op")?.as_str()?;
        Op::ALL.iter().copied().find(|o| o.name() == name)
    }
}

/// May `caller` read the job started by `op`? This computer: always. A remote caller: only with the role
/// the operation itself needs, and never a local-only operation's job (its events can hold what the
/// operation keeps from remote sessions).
pub(crate) fn may_read(caller: &Caller, op: Option<Op>) -> Result<(), ApiError> {
    if !caller.remote {
        return Ok(());
    }
    let refuse = || ApiError::forbidden("Your account cannot follow this job.");
    let op = op.ok_or_else(refuse)?;
    if roles::local_only(op) {
        return Err(ApiError::forbidden(
            "This job can only be followed on the computer that runs Kora.",
        ));
    }
    match &caller.identity {
        Some(id) if id.role >= roles::min_role(op) => Ok(()),
        _ => Err(refuse()),
    }
}

/// The real source: kora-core's std-thread registry.
impl Jobs for kora_core::api::JobRegistry {
    fn get(&self, id: &str) -> Option<Value> {
        let mut v = serde_json::to_value(kora_core::api::JobRegistry::get(self, id)?).ok()?;
        if let Some(e) = v.get("error").and_then(Value::as_str).map(job_error) {
            v["error"] = e;
        }
        Some(v)
    }
    fn subscribe(&self, id: &str) -> Option<(Vec<Value>, Receiver<JobMessage>)> {
        kora_core::api::JobRegistry::subscribe(self, id)
    }
    fn op(&self, id: &str) -> Option<Op> {
        kora_core::api::JobRegistry::get(self, id).map(|s| s.op)
    }
}

/// The UI reads a job error as `{code, message, command?, kept?, log_path?}`. A job body returns
/// `Err(String)`: plain text becomes `{code: "failed", message}`; a JSON object (for structured
/// failures such as installs or `needs_elevation`) is passed through, with `code` defaulting to `failed`.
pub fn job_error(e: &str) -> Value {
    let e = &crate::error::friendly(e.to_string());
    match serde_json::from_str::<Value>(e) {
        Ok(mut v @ Value::Object(_)) if v.get("message").is_some_and(Value::is_string) => {
            if v.get("code").is_none() {
                v["code"] = json!("failed");
            }
            v
        }
        _ => json!({"code": "failed", "message": e}),
    }
}

/// Job source used when nothing is plugged in: every id is unknown.
pub struct NoJobs;
impl Jobs for NoJobs {
    fn get(&self, _: &str) -> Option<Value> {
        None
    }
    fn subscribe(&self, _: &str) -> Option<(Vec<Value>, Receiver<JobMessage>)> {
        None
    }
}

#[derive(Default)]
struct Entry {
    events: Vec<Value>,
    subs: Vec<Sender<JobMessage>>,
    end: Option<JobMessage>,
}

/// In-memory implementation (tests and demos).
#[derive(Default)]
pub struct MemoryJobs {
    inner: Mutex<HashMap<String, Entry>>,
}

impl MemoryJobs {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn create(&self, id: &str) {
        self.inner
            .lock()
            .unwrap()
            .insert(id.to_string(), Entry::default());
    }
    pub fn push(&self, id: &str, ev: Value) {
        if let Some(e) = self.inner.lock().unwrap().get_mut(id) {
            e.events.push(ev.clone());
            e.subs
                .retain(|s| s.send(JobMessage::Event(ev.clone())).is_ok());
        }
    }
    pub fn finish(&self, id: &str, result: Value) {
        self.end(id, JobMessage::Done(result));
    }
    pub fn fail(&self, id: &str, err: &str) {
        self.end(id, JobMessage::Failed(err.to_string()));
    }
    fn end(&self, id: &str, m: JobMessage) {
        if let Some(e) = self.inner.lock().unwrap().get_mut(id) {
            for s in e.subs.drain(..) {
                let _ = s.send(m.clone());
            }
            e.end = Some(m);
        }
    }
}

impl Jobs for MemoryJobs {
    fn get(&self, id: &str) -> Option<Value> {
        let g = self.inner.lock().unwrap();
        let e = g.get(id)?;
        let (state, result, error) = match &e.end {
            None => ("running", Value::Null, Value::Null),
            Some(JobMessage::Done(r)) => ("done", r.clone(), Value::Null),
            Some(JobMessage::Failed(m)) => ("failed", Value::Null, json!(m)),
            Some(JobMessage::Event(_)) => ("running", Value::Null, Value::Null),
        };
        Some(
            json!({"id": id, "op": "test", "server": null, "state": state,
            "events": e.events, "result": result, "error": error}),
        )
    }
    fn subscribe(&self, id: &str) -> Option<(Vec<Value>, Receiver<JobMessage>)> {
        let mut g = self.inner.lock().unwrap();
        let e = g.get_mut(id)?;
        let (tx, rx) = channel();
        match &e.end {
            Some(m) => {
                let _ = tx.send(m.clone());
            }
            None => e.subs.push(tx),
        }
        Some((e.events.clone(), rx))
    }
}

pub async fn job_get(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let jobs = st.jobs.clone();
    crate::blocking(move || {
        let v = jobs.get(&id).ok_or_else(|| ApiError::not_found("unknown job"))?;
        may_read(&caller, jobs.op(&id))?;
        Ok(Json(v))
    })
    .await
}

pub async fn job_events(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let jobs = st.jobs.clone();
    let c = caller.clone();
    let (past, rx) = crate::blocking(move || {
        if jobs.get(&id).is_none() {
            return Err(ApiError::not_found("unknown job"));
        }
        may_read(&c, jobs.op(&id))?;
        jobs.subscribe(&id)
            .ok_or_else(|| ApiError::not_found("unknown job"))
    })
    .await?;
    let permit = st.stream_permit(&caller)?;
    let (tx, trx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(64);
    // Bridge the blocking std receiver to the async stream on a dedicated thread.
    std::thread::spawn(move || {
        let send = |ev: Event| tx.blocking_send(Ok(ev)).is_ok();
        for v in past {
            if !send(progress(&v)) {
                return;
            }
        }
        loop {
            if tx.is_closed() {
                return;
            }
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(JobMessage::Event(v)) => {
                    if !send(progress(&v)) {
                        return;
                    }
                }
                Ok(JobMessage::Done(v)) => {
                    send(Event::default().event("done").data(v.to_string()));
                    return;
                }
                Ok(JobMessage::Failed(m)) => {
                    send(
                        Event::default()
                            .event("failed")
                            .data(json!({"error": m}).to_string()),
                    );
                    return;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    send(
                        Event::default()
                            .event("failed")
                            .data(json!({"error": "job ended unexpectedly"}).to_string()),
                    );
                    return;
                }
            }
        }
    });
    Ok(Sse::new(Limited::new(ReceiverStream::new(trx), permit))
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}

fn progress(v: &Value) -> Event {
    Event::default().event("progress").data(v.to_string())
}

#[cfg(test)]
mod tests {
    use super::{job_error, may_read};
    use crate::remote::{Caller, Identity, Provider, Role};
    use kora_core::api::Op;
    use serde_json::json;

    fn remote(role: Role) -> Caller {
        Caller {
            remote: true,
            peer: Some("100.64.0.9:5000".parse().unwrap()),
            identity: Some(Identity { name: "x".into(), provider: Provider::Token, role }),
        }
    }

    #[test]
    fn remote_callers_read_only_the_jobs_their_role_could_start() {
        assert!(may_read(&Caller::local(), Some(Op::InstallStart)).is_ok());
        assert!(may_read(&Caller::local(), None).is_ok());
        assert!(may_read(&remote(Role::Viewer), Some(Op::ServerControl)).is_err());
        assert!(may_read(&remote(Role::Operator), Some(Op::ServerControl)).is_ok());
        assert!(may_read(&remote(Role::Operator), Some(Op::InstallStart)).is_err());
        assert!(may_read(&remote(Role::Admin), Some(Op::InstallStart)).is_ok());
        assert!(may_read(&remote(Role::Admin), Some(Op::NodeServerControl)).is_err(), "local-only");
        assert!(may_read(&remote(Role::Admin), None).is_err(), "unknown op");
    }

    #[test]
    fn job_errors_have_the_ui_shape() {
        assert_eq!(job_error("disk full"), json!({"code": "failed", "message": "disk full"}));
        assert_eq!(job_error(r#"{"message":"m","log_path":"/x"}"#), json!({"code": "failed", "message": "m", "log_path": "/x"}));
        assert_eq!(job_error(r#"{"code":"needs_elevation","message":"m","command":["kora"]}"#)["code"], "needs_elevation");
        assert_eq!(job_error("[1]"), json!({"code": "failed", "message": "[1]"}));
    }
}
