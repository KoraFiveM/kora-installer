//! The reachability and forwarding operations through the router with a fake backend: the checklist, the
//! saved outside-lookup choices (and that a one-off yes saves nothing), forwarding, and the firewall fix.
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request};
use kora_core::api::fake::FakeBackend;
use kora_web::{router, AppState};
use serde_json::{json, Value};
use tower::ServiceExt;

const TOKEN: &str = "testtoken";
const ADDR: &str = "127.0.0.1:47120";

fn app(fake: &FakeBackend) -> axum::Router {
    // These describe the panel without the administrator service: a machine that has the real one must not be asked.
    kora_web::elevated::set_broker_override(Some(Arc::new(kora_web::elevated::AbsentService)));
    let f = fake.clone();
    router(AppState::system(TOKEN.into(), ADDR.parse().unwrap()).with_backend(Arc::new(move || Box::new(f.clone()))))
}

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (u16, Value) {
    let mut b = Request::builder().method(method).uri(uri).header(header::HOST, ADDR).header(header::AUTHORIZATION, format!("Bearer {TOKEN}"));
    let body = match body {
        Some(v) => {
            b = b.header(header::CONTENT_TYPE, "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let r = app.clone().oneshot(b.body(body).unwrap()).await.unwrap();
    let status = r.status().as_u16();
    let bytes = axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn the_check_answers_one_line_per_layer_and_the_saved_choices() {
    let fake = FakeBackend::sample();
    let app = app(&fake);
    let (st, j) = call(&app, "POST", "/api/v1/servers/prod/reach", Some(json!({}))).await;
    assert_eq!(st, 200, "{j}");
    let ids: Vec<&str> = j["layers"].as_array().unwrap().iter().map(|l| l["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["running", "firewall", "router", "internet"]);
    assert_eq!(j["game_port"], 30120);
    assert_eq!(j["consent"], json!({"cfx_list": false, "ip_echo": false}));
    // Nothing is allowed unless asked: the one-off yes reaches the backend and is not saved.
    let (_, j) = call(&app, "POST", "/api/v1/servers/prod/reach", Some(json!({"cfx_list": true}))).await;
    assert_eq!(j["consent"]["cfx_list"], false);
    assert!(fake.calls().iter().any(|c| c.starts_with("reach_check prod Some(Consent { cfx_list: true, ip_echo: false })")), "{:?}", fake.calls());
    assert!(!fake.calls().iter().any(|c| c.starts_with("reach_consent_set")));
    // Saving is its own step and only changes what it names.
    let (st, j) = call(&app, "PUT", "/api/v1/reach/settings", Some(json!({"ip_echo": true}))).await;
    assert_eq!((st, j), (200, json!({"cfx_list": false, "ip_echo": true})));
    let (_, j) = call(&app, "GET", "/api/v1/reach/settings", None).await;
    assert_eq!(j, json!({"cfx_list": false, "ip_echo": true}));
    let (st, _) = call(&app, "POST", "/api/v1/servers/nope/reach", Some(json!({}))).await;
    assert_eq!(st, 404);
}

#[tokio::test]
async fn forwarding_and_the_firewall_fix() {
    let fake = FakeBackend::sample();
    let app = app(&fake);
    let (st, j) = call(&app, "PUT", "/api/v1/servers/prod/forwarding", Some(json!({"enabled": true}))).await;
    assert_eq!((st, j["enabled"].clone()), (200, json!(true)));
    assert!(fake.calls().contains(&"forwarding_set prod true".to_string()));
    let (_, j) = call(&app, "POST", "/api/v1/servers/prod/reach", Some(json!({}))).await;
    assert_eq!(j["forwarding"]["enabled"], true);
    let (st, j) = call(&app, "POST", "/api/v1/servers/prod/reach/firewall", Some(json!({}))).await;
    assert_eq!(st, 200);
    assert_eq!(j["notes"][0], "Opened port 30120 (TCP and UDP) in the firewall.");
    // A refusal (administrator rights) comes through whole, with its command.
    fake.state().firewall_open_error = Some(
        kora_core::problem::Problem::new(kora_core::problem::codes::NEEDS_ELEVATION, "Changing the firewall needs administrator rights.")
            .with_action(kora_core::problem::Action::ShowCommand { command: vec!["kora".into(), "reach".into(), "prod".into(), "--open-firewall".into()], elevated: true }),
    );
    let (st, j) = call(&app, "POST", "/api/v1/servers/prod/reach/firewall", Some(json!({}))).await;
    assert_eq!(st, 403, "{j}");
    assert_eq!(j["error"]["code"], "needs_elevation");
    assert!(j["error"].to_string().contains("--open-firewall"));
}
