//! Updates, snapshots, resources, catalog and per-server settings, driven through the router with
//! a fake backend. The real updater and the catalog download are replaced by seams.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Once};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{header, Request, Response};
use kora_core::addresources::{AddOutcome, AddPlan};
use kora_core::api::fake::FakeBackend;
use kora_core::api::{Status, UpdateBadge};
use kora_core::catalog::Catalog;
use kora_core::update::manual::{
    ChangeStatus, ResourceChange, SnapshotInfo, UpdateEvent, UpdatePreview,
};
use kora_core::update::{SkipReason, UpdateReport, Updated};
use kora_web::handlers::resources::set_catalog_loader;
use kora_web::handlers::updates::set_auto_runner;
use kora_web::{router, AppState};
use serde_json::{json, Value};
use tower::ServiceExt;

const ADDR: &str = "127.0.0.1:47120";
const TOKEN: &str = "testtoken";
const SECRET_URL: &str = "https://discord.com/api/webhooks/123456/SECRETTOKEN";

static AUTO_CALLS: AtomicUsize = AtomicUsize::new(0);

fn seams() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        set_auto_runner(Some(Arc::new(|server: &str| {
            AUTO_CALLS.fetch_add(1, Ordering::SeqCst);
            Ok(UpdateReport {
                server: server.into(),
                ..UpdateReport::default()
            })
        })));
        set_catalog_loader(Some(Arc::new(|| {
            Ok((
                Catalog::embedded().unwrap(),
                Some("Used the bundled catalog.".into()),
            ))
        })));
    });
}

fn app(fake: &FakeBackend) -> axum::Router {
    seams();
    let f = fake.clone();
    router(
        AppState::system(TOKEN.into(), ADDR.parse().unwrap())
            .with_backend(Arc::new(move || Box::new(f.clone()))),
    )
}

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (u16, Value) {
    let b = Request::builder()
        .method(method)
        .uri(format!("/api/v1{uri}"))
        .header(header::HOST, ADDR)
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"));
    let req = match body {
        Some(v) => b
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let r: Response<Body> = app.clone().oneshot(req).await.unwrap();
    let status = r.status().as_u16();
    let bytes = axum::body::to_bytes(r.into_body(), 1 << 22).await.unwrap();
    let v = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, v)
}

/// Start a job (expects 202) and wait for it to finish; returns the job document.
async fn run_job(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> Value {
    let (s, v) = call(app, method, uri, body).await;
    assert_eq!(s, 202, "{v}");
    let id = v["job"].as_str().unwrap().to_string();
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        let (s, j) = call(app, "GET", &format!("/jobs/{id}"), None).await;
        assert_eq!(s, 200);
        if j["state"] != "running" {
            return j;
        }
        assert!(Instant::now() < end, "job did not finish");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn change(id: &str, status: ChangeStatus) -> ResourceChange {
    ResourceChange {
        id: id.into(),
        from: "aaa".into(),
        to: "bbb".into(),
        behind: 2,
        changelog: vec!["Fix".into()],
        needs_restart: false,
        status,
    }
}

// ---- updates ---------------------------------------------------------------------------------

#[tokio::test]
async fn preview_returns_the_changes_and_a_badge() {
    let fake = FakeBackend::sample();
    fake.state().preview = Some(Ok(UpdatePreview {
        resources: vec![change("kora_core", ChangeStatus::Clean)],
        new_release: None,
    }));
    let app = app(&fake);
    let j = run_job(
        &app,
        "POST",
        "/servers/dev/update/preview",
        Some(json!({"scope": "all"})),
    )
    .await;
    assert_eq!(j["state"], "done", "{j}");
    assert_eq!(j["result"]["resources"][0]["status"], "clean");
    assert_eq!(j["result"]["badge"], json!({"kind": "updates", "count": 1}));
    assert!(fake
        .calls()
        .iter()
        .any(|c| c.starts_with("preview dev All")));

    let j = run_job(
        &app,
        "POST",
        "/servers/dev/update/preview",
        Some(json!({"scope": {"resources": ["kora_core"]}})),
    )
    .await;
    assert_eq!(j["state"], "done");
    assert!(fake
        .calls()
        .iter()
        .any(|c| c.contains("Resources([\"kora_core\"])")));
}

#[tokio::test]
async fn preview_failure_is_a_failed_job_and_bad_input_is_refused() {
    let fake = FakeBackend::sample();
    fake.state().preview = Some(Err("Could not reach GitHub.".into()));
    let app = app(&fake);
    let j = run_job(
        &app,
        "POST",
        "/servers/dev/update/preview",
        Some(json!({"scope": "all"})),
    )
    .await;
    assert_eq!(j["state"], "failed");
    assert!(j["error"].to_string().contains("Could not reach GitHub."), "{j}");
    let (s, v) = call(
        &app,
        "POST",
        "/servers/nope/update/preview",
        Some(json!({"scope": "all"})),
    )
    .await;
    assert_eq!((s, v["error"]["code"].as_str()), (404, Some("not_found")));
    let (s, _) = call(
        &app,
        "POST",
        "/servers/dev/update/preview",
        Some(json!({"scope": {"resources": ["../x"]}})),
    )
    .await;
    assert_eq!(s, 400);
    let (s, _) = call(
        &app,
        "POST",
        "/servers/dev/update/preview",
        Some(json!({"scope": 7})),
    )
    .await;
    assert_eq!(s, 400);
}

#[tokio::test]
async fn apply_streams_events_and_returns_the_report() {
    let fake = FakeBackend::sample();
    {
        let mut s = fake.state();
        s.apply_events = vec![
            UpdateEvent::Snapshot("snap1".into()),
            UpdateEvent::Resource {
                id: "kora_core".into(),
                state: "updated".into(),
            },
            UpdateEvent::Health("Server is healthy".into()),
            UpdateEvent::Done,
        ];
        s.apply = Some(Ok(UpdateReport {
            server: "dev".into(),
            updated: vec![Updated {
                id: "kora_core".into(),
                from: "aaa".into(),
                to: "bbb".into(),
            }],
            skipped: vec![("kora_hud".into(), SkipReason::Dirty)],
            ..UpdateReport::default()
        }));
    }
    let app = app(&fake);
    let body = json!({"scope": "all", "restart": "when_quiet", "skip": ["kora_hud"]});
    let j = run_job(&app, "POST", "/servers/dev/update/apply", Some(body)).await;
    assert_eq!(j["state"], "done", "{j}");
    let ev = j["events"].as_array().unwrap();
    assert_eq!(ev[0], json!({"type": "snapshot", "id": "snap1"}));
    assert_eq!(
        ev[1],
        json!({"type": "resource", "id": "kora_core", "state": "updated"})
    );
    assert_eq!(
        ev[2],
        json!({"type": "health", "message": "Server is healthy"})
    );
    assert_eq!(ev[3], json!({"type": "done"}));
    assert_eq!(
        j["result"]["updated"][0],
        json!({"id": "kora_core", "from": "aaa", "to": "bbb"})
    );
    assert_eq!(j["result"]["skipped"][0][0], "kora_hud");
    assert!(j["result"]["skipped"][0][1].is_string());
    assert!(fake
        .calls()
        .iter()
        .any(|c| c.contains("apply dev All restart=WhenQuiet")));

    let (s, _) = call(
        &app,
        "POST",
        "/servers/dev/update/apply",
        Some(json!({"scope": "all", "restart": "sometimes", "skip": []})),
    )
    .await;
    assert_eq!(s, 400);
    let (s, _) = call(
        &app,
        "POST",
        "/servers/dev/update/apply",
        Some(json!({"scope": "all", "restart": "none", "skip": ["a b"]})),
    )
    .await;
    assert_eq!(s, 400);
}

#[tokio::test]
async fn auto_update_runs_the_seam_not_the_real_updater() {
    let fake = FakeBackend::sample();
    let app = app(&fake);
    let before = AUTO_CALLS.load(Ordering::SeqCst);
    let j = run_job(&app, "POST", "/servers/dev/update/auto", None).await;
    assert_eq!(j["state"], "done", "{j}");
    assert_eq!(j["result"]["server"], "dev");
    assert!(AUTO_CALLS.load(Ordering::SeqCst) > before);
    let (s, _) = call(&app, "POST", "/servers/nope/update/auto", None).await;
    assert_eq!(s, 404);
}

#[tokio::test]
async fn snapshots_list_and_rollback() {
    let fake = FakeBackend::sample();
    fake.state().snapshots = vec![SnapshotInfo {
        id: "snap1".into(),
        created_unix: 1700000000,
        resources: vec!["kora_core".into()],
    }];
    let app = app(&fake);
    let (s, v) = call(&app, "GET", "/servers/dev/snapshots", None).await;
    assert_eq!(s, 200);
    assert_eq!(
        v["snapshots"][0],
        json!({"id": "snap1", "created_unix": 1700000000u64, "resources": ["kora_core"]})
    );
    let j = run_job(&app, "POST", "/servers/dev/snapshots/snap1/rollback", None).await;
    assert_eq!(j["state"], "done", "{j}");
    assert!(fake.calls().contains(&"rollback dev snap1".to_string()));
    let (s, v) = call(&app, "POST", "/servers/dev/snapshots/other/rollback", None).await;
    assert_eq!((s, v["error"]["code"].as_str()), (404, Some("not_found")));
    assert_eq!(
        fake.calls()
            .iter()
            .filter(|c| c.starts_with("rollback"))
            .count(),
        1
    );
    let (s, _) = call(&app, "GET", "/servers/nope/snapshots", None).await;
    assert_eq!(s, 404);
}

// ---- resources -------------------------------------------------------------------------------

#[tokio::test]
async fn resources_and_ensure() {
    let fake = FakeBackend::sample();
    let app = app(&fake);
    let (s, v) = call(&app, "GET", "/servers/prod/resources", None).await;
    assert_eq!(
        (s, v),
        (200, json!({"resources": ["kora_lib", "kora_hud"]}))
    );
    let (s, v) = call(
        &app,
        "POST",
        "/servers/prod/ensure",
        Some(json!({"name": "kora_hud", "restart": true})),
    )
    .await;
    assert_eq!((s, v), (200, json!({"message": "ensured kora_hud"})));
    assert!(fake
        .calls()
        .contains(&"ensure prod kora_hud restart=true".to_string()));
    let (s, _) = call(
        &app,
        "POST",
        "/servers/prod/ensure",
        Some(json!({"name": "bad name; rm", "restart": false})),
    )
    .await;
    assert_eq!(s, 400);
    let (s, _) = call(
        &app,
        "POST",
        "/servers/nope/ensure",
        Some(json!({"name": "x", "restart": false})),
    )
    .await;
    assert_eq!(s, 404);
    fake.state().ensure_error = Some("Can't read server.cfg".into());
    let (s, v) = call(
        &app,
        "POST",
        "/servers/prod/ensure",
        Some(json!({"name": "x", "restart": false})),
    )
    .await;
    assert_eq!(s, 422);
    assert_eq!(v["error"]["message"], "Can't read server.cfg");
}

#[tokio::test]
async fn server_catalog_is_snake_case_and_filtered_by_mode() {
    let fake = FakeBackend::sample();
    fake.state().catalog = Some((Catalog::embedded().unwrap(), vec!["kora-lib".into()]));
    let app = app(&fake);
    let (s, v) = call(&app, "GET", "/servers/prod/catalog", None).await;
    assert_eq!(s, 200);
    assert_eq!(v["installed"], json!(["kora-lib"]));
    assert!(v["catalog"]["schema_version"].is_number());
    let list = v["catalog"]["resources"].as_array().unwrap();
    assert!(!list.is_empty());
    assert!(list
        .iter()
        .all(|r| r["kind"] != "internal" && r["dev_only"].is_null() && r["depends_on"].is_array()));
    assert!(v["note"].is_null());
    let (_, d) = call(&app, "GET", "/servers/dev/catalog", None).await;
    assert!(d["note"].is_string());
    let (s, _) = call(&app, "GET", "/servers/nope/catalog", None).await;
    assert_eq!(s, 404);
}

#[tokio::test]
async fn plan_and_add() {
    let fake = FakeBackend::sample();
    fake.state().add_plan = Some(Ok(AddPlan {
        already: vec!["kora-lib".into()],
        ensure: vec!["[kora]".into()],
        ..AddPlan::default()
    }));
    fake.state().add_result = Some(Ok(AddOutcome {
        installed: vec!["kora-hud".into()],
        failed: vec![("ns_garage".into(), "Could not be reached.".into())],
        ensured: vec!["[kora]".into()],
        notes: vec!["Restart the server.".into()],
    }));
    let app = app(&fake);
    let (s, v) = call(
        &app,
        "POST",
        "/servers/prod/resources/plan",
        Some(json!({"ids": ["kora-hud"]})),
    )
    .await;
    assert_eq!(s, 200);
    assert_eq!(v["already"], json!(["kora-lib"]));
    assert_eq!(v["items"], json!([]));
    let (s, _) = call(
        &app,
        "POST",
        "/servers/prod/resources/plan",
        Some(json!({"ids": []})),
    )
    .await;
    assert_eq!(s, 400);
    let (s, _) = call(
        &app,
        "POST",
        "/servers/prod/resources/plan",
        Some(json!({"ids": ["../x"]})),
    )
    .await;
    assert_eq!(s, 400);
    let (s, _) = call(
        &app,
        "POST",
        "/servers/nope/resources/plan",
        Some(json!({"ids": ["a"]})),
    )
    .await;
    assert_eq!(s, 404);

    let j = run_job(
        &app,
        "POST",
        "/servers/prod/resources",
        Some(json!({"ids": ["kora-hud", "ns_garage"]})),
    )
    .await;
    assert_eq!(j["state"], "done", "{j}");
    assert_eq!(j["result"]["installed"], json!(["kora-hud"]));
    assert_eq!(
        j["result"]["failed"],
        json!([["ns_garage", "Could not be reached."]])
    );
    assert!(j["events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["state"] == "installed"));
    let (s, _) = call(&app, "POST", "/servers/prod/resources", Some(json!({}))).await;
    assert_eq!(s, 400);
}

#[tokio::test]
async fn global_catalog_uses_the_loader() {
    let app = app(&FakeBackend::sample());
    let (s, v) = call(&app, "GET", "/catalog", None).await;
    assert_eq!(s, 200);
    assert!(v["schema_version"].is_number() && v["resources"].as_array().unwrap().len() > 10);
    assert!(v["resources"][0]["depends_on"].is_array());
}

// ---- webhook ---------------------------------------------------------------------------------

#[tokio::test]
async fn webhook_never_returns_the_url() {
    let fake = FakeBackend::sample();
    let app = app(&fake);
    let (s, v) = call(&app, "GET", "/servers/prod/webhook", None).await;
    assert_eq!(
        (s, v["configured"].clone(), v["redacted"].clone()),
        (200, json!(false), Value::Null)
    );
    let (s, _) = call(&app, "POST", "/servers/prod/webhook/test", None).await;
    assert_eq!(s, 400);
    let (s, v) = call(
        &app,
        "PUT",
        "/servers/prod/webhook",
        Some(json!({"url": "https://example.com/hook"})),
    )
    .await;
    assert_eq!(s, 400, "{v}");
    let (s, v) = call(
        &app,
        "PUT",
        "/servers/prod/webhook",
        Some(json!({"url": SECRET_URL})),
    )
    .await;
    assert_eq!(s, 200);
    assert_eq!(v["configured"], true);
    assert!(!v.to_string().contains("SECRETTOKEN"), "{v}");
    let (_, v) = call(&app, "GET", "/servers/prod/webhook", None).await;
    assert!(v["configured"] == true && !v.to_string().contains("SECRETTOKEN"));
    let (s, _) = call(&app, "POST", "/servers/prod/webhook/test", None).await;
    assert!(s == 204 || s == 200);
    assert!(fake.calls().contains(&"notify_test prod".to_string()));
    fake.state().notify_test_error = Some("Discord did not accept the message.".into());
    let (s, v) = call(&app, "POST", "/servers/prod/webhook/test", None).await;
    assert_eq!(s, 422);
    assert!(!v.to_string().contains("SECRETTOKEN"));
    let (s, _) = call(&app, "DELETE", "/servers/prod/webhook", None).await;
    assert!(s == 204 || s == 200);
    let (_, v) = call(&app, "GET", "/servers/prod/webhook", None).await;
    assert_eq!(v["configured"], false);
    let (s, _) = call(&app, "GET", "/servers/nope/webhook", None).await;
    assert_eq!(s, 404);
}

// ---- schedule --------------------------------------------------------------------------------

fn daily() -> Value {
    json!({"preset": "daily", "time": "04:00", "weekday": null, "run_as": "installing_user", "enabled": true})
}

#[tokio::test]
async fn schedule_get_set_remove() {
    let fake = FakeBackend::sample();
    let app = app(&fake);
    let (s, v) = call(&app, "GET", "/servers/dev/schedule", None).await;
    assert_eq!(s, 200);
    assert!(v["schedule"].is_null() && v["description"].is_null() && v["run_as_help"].is_string());
    let (s, v) = call(&app, "PUT", "/servers/dev/schedule", Some(daily())).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["schedule"]["time"], "04:00");
    assert!(v["description"].as_str().unwrap().contains("04:00"));
    let (_, v) = call(&app, "GET", "/servers/dev/schedule", None).await;
    assert_eq!(v["schedule"]["preset"], "daily");
    let (s, _) = call(&app, "DELETE", "/servers/dev/schedule", None).await;
    assert!(s == 204 || s == 200);
    assert!(fake.calls().contains(&"schedule_remove dev".to_string()));
}

#[tokio::test]
async fn schedule_validation() {
    let fake = FakeBackend::sample();
    let app = app(&fake);
    let (s, _) = call(&app, "PUT", "/servers/prod/schedule", Some(daily())).await;
    assert_eq!(s, 400, "production servers have no schedule");
    let mut bad = daily();
    bad["time"] = json!("4pm");
    let (s, _) = call(&app, "PUT", "/servers/dev/schedule", Some(bad)).await;
    assert_eq!(s, 400);
    let weekly_no_day = json!({"preset": "weekly", "time": "04:00", "weekday": null, "run_as": "installing_user", "enabled": true});
    let (s, _) = call(&app, "PUT", "/servers/dev/schedule", Some(weekly_no_day)).await;
    assert_eq!(s, 400);
    let (s, _) = call(
        &app,
        "PUT",
        "/servers/dev/schedule",
        Some(json!({"preset": "hourly"})),
    )
    .await;
    assert_eq!(s, 400);
    assert!(!fake.calls().iter().any(|c| c.starts_with("schedule_set")));
}

#[tokio::test]
async fn schedule_needs_elevation_without_changing_anything() {
    let fake = FakeBackend::sample();
    fake.state().elevation = Some(vec!["schedule".into(), "set".into(), "dev".into()]);
    let app = app(&fake);
    let (s, v) = call(&app, "PUT", "/servers/dev/schedule", Some(daily())).await;
    assert_eq!(
        (s, v["error"]["code"].as_str()),
        (403, Some("needs_elevation"))
    );
    assert_eq!(
        v["error"]["command"],
        json!(["kora", "schedule", "set", "dev"])
    );
    assert!(v["error"]["message"]
        .as_str()
        .unwrap()
        .contains("administrator"));
    let (s, v) = call(&app, "DELETE", "/servers/dev/schedule", None).await;
    assert_eq!(
        (s, v["error"]["code"].as_str()),
        (403, Some("needs_elevation"))
    );
    assert!(!fake.calls().iter().any(|c| c.starts_with("schedule_")));
}

// ---- ports -----------------------------------------------------------------------------------

#[tokio::test]
async fn ports_only_when_stopped_and_valid() {
    let fake = FakeBackend::sample();
    let app = app(&fake);
    let (s, v) = call(
        &app,
        "PUT",
        "/servers/dev/ports",
        Some(json!({"game": 30200, "txadmin": 40200})),
    )
    .await;
    assert_eq!(s, 200, "{v}");
    assert!(v["notes"].as_array().is_some());
    assert!(fake.calls().contains(&"ports dev 30200 40200".to_string()));

    for body in [
        json!({"game": 80, "txadmin": 40200}),
        json!({"game": 30200, "txadmin": 70000}),
        json!({"game": 30200, "txadmin": 30200}),
        json!({"game": "a", "txadmin": 1}),
        json!({"game": 30200}),
    ] {
        let (s, _) = call(&app, "PUT", "/servers/dev/ports", Some(body)).await;
        assert_eq!(s, 400);
    }
    let (s, v) = call(
        &app,
        "PUT",
        "/servers/prod/ports",
        Some(json!({"game": 30500, "txadmin": 40500})),
    )
    .await;
    assert_eq!((s, v["error"]["code"].as_str()), (409, Some("conflict")));
    fake.state().status.insert("prod".into(), Status::Stopped);
    fake.state()
        .badges
        .insert("prod".into(), UpdateBadge::UpToDate);
    let (s, _) = call(
        &app,
        "PUT",
        "/servers/prod/ports",
        Some(json!({"game": 30500, "txadmin": 40500})),
    )
    .await;
    assert_eq!(s, 200);
    let (s, _) = call(
        &app,
        "PUT",
        "/servers/nope/ports",
        Some(json!({"game": 30500, "txadmin": 40500})),
    )
    .await;
    assert_eq!(s, 404);
    fake.state().ports_error = Some("Port 30600 is in use.".into());
    let (s, v) = call(
        &app,
        "PUT",
        "/servers/dev/ports",
        Some(json!({"game": 30600, "txadmin": 40600})),
    )
    .await;
    assert_eq!(s, 422);
    assert_eq!(v["error"]["message"], "Port 30600 is in use.");
}
