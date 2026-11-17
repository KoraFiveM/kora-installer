//! The GitHub loop for a resource, driven through the router with a fake backend: what the handlers check before anything
//! reaches git or GitHub, and that errors arrive as guided problems.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, Response};
use kora_core::api::fake::FakeBackend;
use kora_core::collab::{Acted, Reply};
use kora_core::problem::{Action, Problem};
use kora_web::{router, AppState};
use serde_json::{json, Value};
use tower::ServiceExt;

const ADDR: &str = "127.0.0.1:47120";
const TOKEN: &str = "testtoken";

fn app(fake: &FakeBackend) -> axum::Router {
    let f = fake.clone();
    router(AppState::system(TOKEN.into(), ADDR.parse().unwrap()).with_backend(Arc::new(move || Box::new(f.clone()))))
}

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (u16, Value) {
    let b = Request::builder().method(method).uri(format!("/api/v1{uri}")).header(header::HOST, ADDR).header(header::AUTHORIZATION, format!("Bearer {TOKEN}"));
    let req = match body {
        Some(v) => b.header(header::CONTENT_TYPE, "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let r: Response<Body> = app.clone().oneshot(req).await.unwrap();
    let status = r.status().as_u16();
    let bytes = axum::body::to_bytes(r.into_body(), 1 << 22).await.unwrap();
    (status, if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap_or(Value::Null) })
}

fn acted() -> Reply {
    Reply::Acted(Acted { message: "done".into(), acting_as: Some("sam".into()), url: None })
}

fn collab_calls(f: &FakeBackend) -> usize {
    f.calls().iter().filter(|c| c.starts_with("collab ")).count()
}

#[tokio::test]
async fn what_the_browser_sends_is_checked_before_anything_runs() {
    let fake = FakeBackend::sample();
    fake.state().collab = Some(Ok(acted()));
    let app = app(&fake);
    let base = "/servers/dev/resources/kora_hud";
    for (m, u, b) in [
        ("POST", format!("{base}/prs/0/comment"), json!({"body": "x"})),
        ("POST", format!("{base}/prs/abc/comment"), json!({"body": "x"})),
        ("POST", format!("{base}/prs/12/comment"), json!({"body": "x", "source": "elsewhere"})),
        ("POST", format!("{base}/prs/12/review"), json!({"action": "delete"})),
        ("POST", format!("{base}/prs/12/merge"), json!({"method": "--admin"})),
        ("POST", format!("{base}/new-branch"), json!({"name": "--exec=x"})),
        ("POST", "/servers/dev/resources/..%2F..%2Fetc/commit".to_string(), json!({"message": "x"})),
        ("POST", "/servers/dev/fork".to_string(), json!({"owner": "-bad"})),
        ("POST", "/servers/dev/fork".to_string(), json!({"resources": []})),
        ("POST", "/servers/dev/fork".to_string(), json!({"resources": ["../x"]})),
    ] {
        let (s, v) = call(&app, m, &u, Some(b)).await;
        assert!(s == 400 || s == 404, "{u}: {s} {v}");
    }
    assert_eq!(collab_calls(&fake), 0, "nothing reached the backend");
    // A good one does.
    let (s, v) = call(&app, "POST", &format!("{base}/prs/12/review"), Some(json!({"action": "approve"}))).await;
    assert_eq!((s, v["acting_as"].as_str()), (200, Some("sam")), "{v}");
    assert_eq!(collab_calls(&fake), 1);
}

#[tokio::test]
async fn a_plain_clone_pushing_gets_the_guided_problem_with_the_fix() {
    let fake = FakeBackend::sample();
    let p = Problem::new(kora_core::collab::NEEDS_FORK, "Pushing needs your own copy.").with_action(Action::MakeFork { resource: None });
    fake.state().collab = Some(Err(p.encode()));
    let app = app(&fake);
    let (s, v) = call(&app, "POST", "/servers/prod/resources/kora_hud/push", None).await;
    assert_eq!(s, 422, "{v}");
    assert_eq!(v["error"]["code"], kora_core::collab::NEEDS_FORK);
    assert_eq!(v["error"]["actions"][0]["kind"], "make_fork");
}

#[tokio::test]
async fn the_changes_and_pull_requests_are_served_for_every_server() {
    let fake = FakeBackend::sample();
    fake.state().collab = Some(Err("nothing set up".into()));
    let app = app(&fake);
    for server in ["dev", "prod"] {
        let (s, _) = call(&app, "GET", &format!("/servers/{server}/resources/kora_hud/changes"), None).await;
        assert_ne!(s, 409, "{server}: no longer refused because of how the server follows versions");
    }
    assert_eq!(collab_calls(&fake), 2);
}
