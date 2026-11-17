//! What the administrator service did with administrator rights, in the panel's Activity log (`GET /audit/service`): administrators
//! only, the service's own lines (only this account's unless it administers the machine), newest first, and a plain explanation
//! when the service is not there. Runs against a fake service (no socket).
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, Request};
use kora_core::api::fake::FakeBackend;
use kora_core::privsvc::conn::ClientError;
use kora_core::privsvc::problems::Availability;
use kora_web::elevated::{set_broker_override, Broker, StepOutcome};
use kora_web::remote::{Remote, RemoteConfig, Role};
use kora_web::{router, AppState};
use serde_json::{json, Value};
use tower::ServiceExt;

const BOUND: &str = "100.64.0.5:47120";
const LAUNCH: &str = "launchtoken";
const REMOTE_PEER: &str = "100.64.0.20:51000";

struct Service {
    availability: Availability,
    answer: Result<StepOutcome, ClientError>,
    seen: Mutex<Vec<(String, Vec<String>)>>,
}

impl Broker for Service {
    fn availability(&self) -> Availability {
        self.availability.clone()
    }
    fn call(&self, verb: &str, args: &[String], _: &[(String, String)], _: &mut dyn FnMut(&str)) -> Result<StepOutcome, ClientError> {
        self.seen.lock().unwrap().push((verb.into(), args.to_vec()));
        self.answer.clone()
    }
}

fn answer(admin: bool, lines: &[&str]) -> Result<StepOutcome, ClientError> {
    Ok(StepOutcome { ok: true, code: 0, data: Some(json!({"admin": admin, "lines": lines})) })
}

const LINES: &[&str] = &[
    "2026-11-17T09:00:00Z | uid:1000 (jo) | run system.firewall [my-server] | done",
    "2026-11-17T09:01:00Z | uid:1000 (jo) | backup job set [my-server] | failed (code 1)",
    "not a log line",
    "2026-11-17T09:02:00Z | uid:1000 (jo) | undo system.service | refused: checks failed",
];

fn app() -> (axum::Router, [String; 2]) {
    let mut cfg = RemoteConfig { enabled: true, ..Default::default() };
    let o = cfg.add_user("olly", Role::Operator).unwrap();
    let a = cfg.add_user("adam", Role::Admin).unwrap();
    let st = AppState::system(LAUNCH.into(), BOUND.parse().unwrap()).with_backend(Arc::new(|| Box::new(FakeBackend::sample()))).with_remote(Arc::new(Remote::new(None, cfg)));
    (router(st), [o, a])
}

async fn get(app: &axum::Router, uri: &str, bearer: &str, peer: Option<&str>) -> (u16, Value) {
    let mut r = Request::builder().method("GET").uri(uri).header(header::HOST, BOUND).header(header::AUTHORIZATION, format!("Bearer {bearer}")).header("x-kora", "1").body(Body::empty()).unwrap();
    if let Some(p) = peer {
        r.extensions_mut().insert(ConnectInfo(p.parse::<SocketAddr>().unwrap()));
    }
    let resp = app.clone().oneshot(r).await.unwrap();
    let status = resp.status().as_u16();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 22).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn the_service_log_is_shown_to_administrators_newest_first_and_explained_when_absent() {
    kora_core::installer::registry::use_test_home();
    let (app, [operator, admin]) = app();

    // the service is there and answers with its own lines
    let svc = Arc::new(Service { availability: Availability::Ready, answer: answer(false, LINES), seen: Mutex::new(vec![]) });
    set_broker_override(Some(svc.clone()));
    let (s, v) = get(&app, "/api/v1/audit/service?limit=50", LAUNCH, None).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!((v["available"].as_bool(), v["all_accounts"].as_bool()), (Some(true), Some(false)));
    let e = v["entries"].as_array().unwrap();
    assert_eq!(e.len(), 3, "the junk line is dropped: {v}");
    assert!(e[0]["what"] == "undo system.service" && e[0]["result"] == "refused" && e[0]["detail"] == "checks failed", "newest first: {v}");
    assert!(e[1]["result"] == "failed" && e[1]["detail"] == "code 1" && e[1]["account"] == "jo");
    assert!(e[2]["at"].as_u64().unwrap() < e[0]["at"].as_u64().unwrap());
    assert_eq!(svc.seen.lock().unwrap().clone(), [("audit-read".to_string(), vec!["--lines".to_string(), "50".to_string()])], "one read-only request, bounded");

    // an administrator of the machine sees everyone's
    set_broker_override(Some(Arc::new(Service { availability: Availability::Ready, answer: answer(true, LINES), seen: Mutex::new(vec![]) })));
    assert_eq!(get(&app, "/api/v1/audit/service", LAUNCH, None).await.1["all_accounts"], true);

    // a remote operator may not read it, a remote administrator may
    assert_eq!(get(&app, "/api/v1/audit/service", &operator, Some(REMOTE_PEER)).await.0, 403);
    assert_eq!(get(&app, "/api/v1/audit/service", &admin, Some(REMOTE_PEER)).await.0, 200);

    // the limit is clamped
    let svc = Arc::new(Service { availability: Availability::Ready, answer: answer(false, &[]), seen: Mutex::new(vec![]) });
    set_broker_override(Some(svc.clone()));
    get(&app, "/api/v1/audit/service?limit=99999", LAUNCH, None).await;
    assert_eq!(svc.seen.lock().unwrap()[0].1[1], "500");

    // no service, or one that does not serve this account: an explanation, not an error
    set_broker_override(Some(Arc::new(Service { availability: Availability::Missing, answer: Err(ClientError::Unavailable("x".into())), seen: Mutex::new(vec![]) })));
    let (s, v) = get(&app, "/api/v1/audit/service", LAUNCH, None).await;
    assert_eq!((s, v["available"].as_bool(), v["entries"].as_array().map(Vec::len)), (200, Some(false), Some(0)));
    assert!(v["note"].as_str().unwrap().contains("not installed"));
    set_broker_override(Some(Arc::new(Service { availability: Availability::NotAllowed("ask an administrator".into()), answer: Err(ClientError::Unavailable("x".into())), seen: Mutex::new(vec![]) })));
    assert!(get(&app, "/api/v1/audit/service", LAUNCH, None).await.1["note"].as_str().unwrap().contains("ask an administrator"));

    // a service that stops answering is explained too
    set_broker_override(Some(Arc::new(Service { availability: Availability::Ready, answer: Err(ClientError::Broken("pipe closed".into())), seen: Mutex::new(vec![]) })));
    let (s, v) = get(&app, "/api/v1/audit/service", LAUNCH, None).await;
    assert_eq!((s, v["available"].as_bool()), (200, Some(true)));
    assert!(v["note"].as_str().unwrap().contains("pipe closed"));
    set_broker_override(None);
}
