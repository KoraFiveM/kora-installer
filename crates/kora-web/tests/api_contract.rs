//! The contract between the Rust description of the API (`kora_core::api::spec`, the source of the
//! generated TypeScript) and what the real routes answer.
//!
//! For every operation that has a case below, the real handler runs (fake backend, no network) and
//! * the request body of the case must read as the declared request type,
//! * the answer must read as the declared response type AND write back to the very same JSON (so a
//!   field the type does not know, or a `null` where the type leaves the field out, fails here),
//! * a job's events and result must read as the declared event and result types, a failed job's error
//!   as `JobError`.
//! Every error answer of every operation must read as `ErrorEnvelope`.
//!
//! An operation without a case is listed in `UNVERIFIED` with the reason; the list is printed by the
//! test and a new operation must be in one place or the other.
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request, Response};
use kora_core::api::fake::FakeBackend;
use kora_core::api::spec::{spec, Body as SpecBody, Resp};
use kora_core::api::wire::{ErrorEnvelope, JobError};
use kora_core::api::{Method, Op};
use kora_web::remote::Remote;
use kora_web::{router, AppState};
use serde_json::{json, Value};
use tower::ServiceExt;

const ADDR: &str = "127.0.0.1:47120";
const TOKEN: &str = "testtoken";

struct Case {
    op: Op,
    /// `{s}`, `{id}`, ... in the route, in order of appearance.
    params: &'static [(&'static str, &'static str)],
    query: &'static str,
    body: Option<Value>,
}

fn case(op: Op, params: &'static [(&'static str, &'static str)], body: Option<Value>) -> Case {
    Case { op, params, query: "", body }
}

fn cases() -> Vec<Case> {
    let s = &[("s", "prod")];
    vec![
        case(Op::Meta, &[], None),
        case(Op::SessionCreate, &[], Some(json!({"token": TOKEN}))),
        case(Op::SystemInfo, &[], None),
        case(Op::RemoteGet, &[], None),
        case(Op::RemoteSet, &[], Some(json!({"allow_remote_destructive": false}))),
        case(Op::UsersList, &[], None),
        case(Op::UserAdd, &[], Some(json!({"name": "sam", "role": "operator"}))),
        case(Op::ServersList, &[], None),
        case(Op::ServerGet, s, None),
        case(Op::ServerControl, s, Some(json!({"action": "restart"}))),
        case(Op::ServersScan, &[], None),
        case(Op::ResourcesList, s, None),
        case(Op::ResourceEnsure, s, Some(json!({"name": "kora_hud", "restart": false}))),
    ]
}

/// Operations the contract test does not run yet, and why. Shrinks as cases are added.
const UNVERIFIED: &[(Op, &str)] = &[
    (Op::SessionLogout, "204, nothing to read"),
    (Op::SessionLink, "needs a local session"),
    (Op::PanelRestart, "would restart the process"),
    (Op::UserRemove, "204, nothing to read"),
    (Op::ServerUnregister, "204"),
    (Op::ServerDelete, "needs a managed server folder"),
    (Op::ServersAdopt, "needs a real server folder"),
    (Op::ServerOpenFolder, "opens a window"),
    (Op::LogsList, "needs a server folder with logs"),
    (Op::LogsTail, "needs a server folder with logs"),
    (Op::ConsoleStream, "stream"),
    (Op::ConsoleSend, "204"),
    (Op::ConsoleEnable, "204"),
    (Op::ResourceControl, "needs a console helper on a running server"),
    (Op::NodeServerControl, "needs a registered remote node"),
];

fn app() -> axum::Router {
    kora_core::api::maintenance::set_override(Some(kora_core::api::maintenance::fake::FakeMaintenance::new(false)));
    let fake = FakeBackend::sample();
    let st = AppState::system(TOKEN.into(), ADDR.parse::<SocketAddr>().unwrap()).with_backend(Arc::new(move || Box::new(fake.clone()))).with_remote(Arc::new(Remote::memory()));
    router(st)
}

fn uri(op: Op, params: &[(&str, &str)], query: &str) -> String {
    let mut path = op.route().1.to_string();
    for (k, v) in params {
        path = path.replace(&format!("{{{k}}}"), v);
    }
    assert!(!path.contains('{'), "{}: a path parameter has no value in the case", op.name());
    let q = if query.is_empty() { String::new() } else { format!("?{query}") };
    format!("/api/v1{path}{q}")
}

async fn send(app: &axum::Router, method: Method, uri: &str, body: Option<&Value>) -> Response<Body> {
    let mut b = Request::builder().method(method.as_str()).uri(uri).header(header::HOST, ADDR).header(header::AUTHORIZATION, format!("Bearer {TOKEN}")).header("x-kora", "1");
    let body = match body {
        Some(v) => {
            b = b.header(header::CONTENT_TYPE, "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    app.clone().oneshot(b.body(body).unwrap()).await.unwrap()
}

async fn json_of(r: Response<Body>) -> Value {
    let b = axum::body::to_bytes(r.into_body(), 1 << 22).await.unwrap();
    if b.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&b).unwrap_or_else(|e| panic!("not JSON: {e}: {}", String::from_utf8_lossy(&b)))
    }
}

#[tokio::test]
async fn real_answers_match_the_declared_types() {
    let app = app();
    let mut problems = Vec::new();
    let mut checked = 0;
    for c in cases() {
        let sp = spec(c.op);
        let name = c.op.name();
        if let (Some(b), SpecBody::Json(t)) = (&c.body, &sp.body) {
            if let Err(e) = t.check(b) {
                problems.push(format!("{name}: the case's request body: {e}"));
            }
        }
        let r = send(&app, c.op.route().0, &uri(c.op, c.params, c.query), c.body.as_ref()).await;
        let status = r.status().as_u16();
        let v = json_of(r).await;
        if !(200..300).contains(&status) {
            problems.push(format!("{name}: the case answered {status}: {v}"));
            continue;
        }
        match (&sp.response, c.op.is_job()) {
            (Resp::Job, _) | (Resp::Json(_), true) => {
                if status != 202 {
                    problems.push(format!("{name}: a job answers 202, not {status}"));
                }
                let id = v["job"].as_str().unwrap_or_default().to_string();
                if let Resp::Json(t) = &sp.response {
                    if let Err(e) = t.check(&v) {
                        problems.push(format!("{name}: start answer: {e}"));
                    }
                }
                let rec = wait_job(&app, &id).await;
                let jt = sp.job.clone().unwrap_or_default();
                for ev in rec["events"].as_array().cloned().unwrap_or_default() {
                    if let Some(t) = &jt.events {
                        if let Err(e) = t.check(&ev) {
                            problems.push(format!("{name}: job event {ev}: {e}"));
                        }
                    }
                }
                if rec["state"] == "done" {
                    if let Some(t) = &jt.result {
                        if let Err(e) = t.check(&rec["result"]) {
                            problems.push(format!("{name}: job result: {e}"));
                        }
                    }
                } else if rec["state"] == "failed" {
                    problems.push(format!("{name}: the job failed: {}", rec["error"]));
                }
            }
            (Resp::Json(t), false) => {
                if let Err(e) = t.check(&v) {
                    problems.push(format!("{name}: {e}"));
                }
            }
            (Resp::NoContent, _) => {
                if !v.is_null() {
                    problems.push(format!("{name}: declared to answer nothing but answered {v}"));
                }
            }
            (Resp::Stream(..) | Resp::Untyped, _) => {}
        }
        checked += 1;
    }
    println!("contract: {checked} operations run against their declared types; unverified: {}", UNVERIFIED.len());
    assert!(problems.is_empty(), "the API does not match its declared types:\n{}", problems.join("\n"));
}

/// The job record (`GET /jobs/{id}`) reads as `JobRecord`, a failed job's error as `JobError`.
async fn wait_job(app: &axum::Router, id: &str) -> Value {
    for _ in 0..400 {
        let r = send(app, Method::Get, &format!("/api/v1/jobs/{id}"), None).await;
        let v = json_of(r).await;
        if v["state"] != "running" {
            let rec = serde_json::from_value::<kora_core::api::wire::JobRecord<Value, Value>>(v.clone());
            assert!(rec.is_ok(), "job record: {rec:?}");
            if !v["error"].is_null() {
                let e: Result<JobError, _> = serde_json::from_value(v["error"].clone());
                assert!(e.is_ok(), "job error {}: {e:?}", v["error"]);
            }
            return v;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("job {id} did not finish");
}

/// Every error answer of every operation has the declared envelope.
#[tokio::test]
async fn every_error_answer_has_the_declared_shape() {
    let app = app();
    let mut bad = Vec::new();
    for op in Op::ALL {
        if matches!(op, Op::ConsoleStream | Op::Events | Op::JobEvents) {
            continue;
        }
        let params: Vec<(String, String)> = kora_core::api::tsgen::path_params(op.route().1).into_iter().map(|p| (p.to_string(), "nope".to_string())).collect();
        let mut path = op.route().1.to_string();
        for (k, v) in &params {
            path = path.replace(&format!("{{{k}}}"), v);
        }
        let body = if matches!(op.route().0, Method::Get | Method::Delete) { None } else { Some(json!({"unexpected": 1})) };
        let r = send(&app, op.route().0, &format!("/api/v1{path}"), body.as_ref()).await;
        if r.status().is_success() {
            continue;
        }
        let status = r.status();
        let ct = r.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        let bytes = axum::body::to_bytes(r.into_body(), 1 << 22).await.unwrap();
        // Axum's own extractor rejections (a body it cannot read) are plain text, not ours.
        if !ct.starts_with("application/json") {
            continue;
        }
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        let t = kora_core::api::spec::Ty::of::<ErrorEnvelope>();
        if let Err(e) = t.check(&v) {
            bad.push(format!("{} -> {status}: {e}", op.name()));
        }
    }
    assert!(bad.is_empty(), "error answers that are not the declared envelope:\n{}", bad.join("\n"));
}

#[test]
fn every_operation_is_verified_or_listed_as_unverified() {
    let have: Vec<Op> = cases().iter().map(|c| c.op).collect();
    let skip: Vec<Op> = UNVERIFIED.iter().map(|(o, _)| *o).collect();
    let mut missing = Vec::new();
    for op in Op::ALL {
        let typed = {
            let s = spec(*op);
            matches!(s.body, SpecBody::Json(_) | SpecBody::Raw(_)) || matches!(s.response, Resp::Json(_) | Resp::Stream(..)) || s.job.as_ref().is_some_and(|j| j.events.is_some() || j.result.is_some())
        };
        // Untyped operations have nothing to verify yet; the generator lists them.
        if typed && !have.contains(op) && !skip.contains(op) {
            missing.push(op.name());
        }
        assert!(!(have.contains(op) && skip.contains(op)), "{} is both verified and listed as unverified", op.name());
    }
    assert!(missing.is_empty(), "typed operations with neither a contract case nor an UNVERIFIED entry: {missing:?}");
}
