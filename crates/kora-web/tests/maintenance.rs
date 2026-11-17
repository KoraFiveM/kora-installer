//! Self-update, the Kora app and uninstall: handlers driven through the router with a fake machine.
#![allow(clippy::await_holding_lock)]
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{header, Request};
use kora_core::api::fake::FakeBackend;
use kora_core::api::maintenance::fake::FakeMaintenance;
use kora_core::api::maintenance::{self, PlanItem, UninstallOutcome};
use kora_web::{router, AppState};
use serde_json::{json, Value};
use tower::ServiceExt;

const TOKEN: &str = "testtoken";
const ADDR: &str = "127.0.0.1:47120";
static LOCK: Mutex<()> = Mutex::new(());

/// The maintenance seam is process-wide: tests take turns.
fn setup(admin: bool) -> (axum::Router, Arc<FakeMaintenance>, MutexGuard<'static, ()>) {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var("KORA_APP_PREFIX");
    let m = FakeMaintenance::new(admin);
    maintenance::set_override(Some(m.clone()));
    let fake = FakeBackend::sample();
    let st = AppState::system(TOKEN.into(), ADDR.parse().unwrap()).with_backend(Arc::new(move || Box::new(fake.clone())));
    (router(st), m, g)
}

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (u16, Value) {
    let mut b = Request::builder().method(method).uri(uri).header(header::HOST, ADDR).header(header::AUTHORIZATION, format!("Bearer {TOKEN}"));
    let body = match body {
        Some(j) => {
            b = b.header(header::CONTENT_TYPE, "application/json");
            Body::from(j.to_string())
        }
        None => Body::empty(),
    };
    let r = app.clone().oneshot(b.body(body).unwrap()).await.unwrap();
    let s = r.status().as_u16();
    let bytes = axum::body::to_bytes(r.into_body(), 1 << 22).await.unwrap();
    (s, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn finish(app: &axum::Router, id: &str) -> Value {
    let t = Instant::now();
    loop {
        let (_, j) = call(app, "GET", &format!("/api/v1/jobs/{id}"), None).await;
        if j["state"] != "running" {
            return j;
        }
        assert!(t.elapsed() < Duration::from_secs(10), "job did not finish");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn self_update_check_reports_versions() {
    let (app, _m, _g) = setup(false);
    let (s, j) = call(&app, "GET", "/api/v1/self-update", None).await;
    assert_eq!(s, 200);
    assert_eq!((j["current"].as_str(), j["latest"].as_str()), (Some("0.4.0"), Some("0.4.1")));
    assert_eq!((&j["available"], &j["update_available"]), (&json!(true), &json!(true)));
    assert_eq!(j["versions"], json!(["0.4.1", "0.4.0"]));
}

#[tokio::test]
async fn self_update_check_failure_is_a_plain_error() {
    let (app, m, _g) = setup(false);
    *m.update.lock().unwrap() = Err("could not reach GitHub: offline".into());
    let (s, j) = call(&app, "GET", "/api/v1/self-update", None).await;
    assert_eq!(s, 422);
    assert!(j["error"]["message"].as_str().unwrap().contains("could not reach GitHub"));
}

#[tokio::test]
async fn self_update_apply_runs_a_job() {
    let (app, m, _g) = setup(false);
    let (s, j) = call(&app, "POST", "/api/v1/self-update", Some(json!({"version": "v0.4.1"}))).await;
    assert_eq!(s, 202);
    let done = finish(&app, j["job"].as_str().unwrap()).await;
    assert_eq!(done["state"], "done");
    assert_eq!(done["result"]["version"], "v0.4.1");
    assert!(m.calls().contains(&"self_update_apply v0.4.1".to_string()));
    // No body at all is fine too.
    let (s, _) = call(&app, "POST", "/api/v1/self-update", None).await;
    assert_eq!(s, 202);
}

// Without a helper this answers "run this command" on Linux. On Windows the panel asks Windows itself
// (tests/admin_worker.rs covers that), so the answer here is different by design.
#[cfg(not(windows))]
#[tokio::test]
async fn self_update_apply_validates_and_elevates() {
    let (app, _m, _g) = setup(false);
    let (s, _) = call(&app, "POST", "/api/v1/self-update", Some(json!({"version": "x; rm -rf /"}))).await;
    assert_eq!(s, 400);
    let (app, mut_m, _g2) = {
        drop(_g);
        let (a, m, g) = setup(false);
        (a, m, g)
    };
    // Rebuild with a fake that cannot write the program folder.
    let m2 = Arc::new(FakeMaintenance { needs_admin: true, ..clone_fake(&mut_m) });
    maintenance::set_override(Some(m2));
    let (s, j) = call(&app, "POST", "/api/v1/self-update", Some(json!({"version": "0.4.1"}))).await;
    assert_eq!(s, 403);
    assert_eq!(j["error"]["code"], "needs_elevation");
    assert_eq!(j["error"]["command"], json!(["kora", "self-update", "--yes", "--version", "0.4.1"]));
}

#[cfg(not(windows))]
fn clone_fake(m: &FakeMaintenance) -> FakeMaintenance {
    FakeMaintenance {
        admin: m.admin,
        update: Mutex::new(m.update.lock().unwrap().clone()),
        needs_admin: m.needs_admin,
        app: Mutex::new(m.app.lock().unwrap().clone()),
        plan: Mutex::new(m.plan.lock().unwrap().clone()),
        outcome: Mutex::new(m.outcome.lock().unwrap().clone()),
        calls: Mutex::new(vec![]),
    }
}

#[tokio::test]
async fn app_status_shape() {
    let (app, m, _g) = setup(false);
    let (s, j) = call(&app, "GET", "/api/v1/app", None).await;
    assert_eq!(s, 200);
    assert_eq!(
        j,
        json!({"installed": false, "version": null, "path": null, "desktop_shortcut": false, "supported": true,
               "legacy": false, "legacy_items": [], "shortcuts": false})
    );
    m.app.lock().unwrap().installed = true;
    m.app.lock().unwrap().path = Some("/opt/kora/kora".into());
    m.app.lock().unwrap().legacy = true;
    m.app.lock().unwrap().legacy_items = vec!["Delete /opt/kora/bin/kora".into()];
    let (_, j) = call(&app, "GET", "/api/v1/app", None).await;
    assert_eq!(j["installed"], true);
    assert_eq!(j["path"], "/opt/kora/kora");
    assert_eq!(j["legacy"], true);
    assert_eq!(j["legacy_items"], json!(["Delete /opt/kora/bin/kora"]));
}

#[tokio::test]
async fn tidy_up_with_nothing_old_only_makes_the_shortcuts_and_needs_no_administrator() {
    for admin in [false, true] {
        let (app, m, _g) = setup(admin);
        let (s, j) = call(&app, "PUT", "/api/v1/app", Some(json!({"desktop_shortcut": false, "dry_run": false}))).await;
        assert_eq!(s, 200, "{j}");
        assert!(j["message"].as_str().unwrap().contains("shortcuts"), "{j}");
        assert_eq!(m.calls(), ["app_install shortcut=false force=false dry=false"], "no old copy: nothing is removed");
    }
}

#[tokio::test]
async fn tidy_up_as_admin_makes_the_shortcuts_first_then_removes_the_old_copy() {
    let (app, m, _g) = setup(true);
    m.app.lock().unwrap().legacy = true;
    m.app.lock().unwrap().legacy_items = vec!["Delete the old program".into()];
    let (s, j) = call(&app, "PUT", "/api/v1/app", Some(json!({}))).await;
    assert_eq!(s, 200, "{j}");
    assert!(j["message"].as_str().unwrap().contains("old copy"), "{j}");
    assert_eq!(m.calls(), ["app_install shortcut=true force=false dry=false", "app_uninstall dry=false"], "new shortcuts BEFORE the old ones go");
    // a preview changes nothing and says what would go
    let (s, j) = call(&app, "PUT", "/api/v1/app", Some(json!({"dry_run": true}))).await;
    assert_eq!(s, 200);
    let text = j["message"].as_str().unwrap();
    assert!(text.contains("Nothing was changed") && text.contains("Delete the old program"), "{j}");
    assert_eq!(m.calls().last().unwrap(), "app_install shortcut=true force=false dry=true");
}

// Without a helper this answers "run this command" on Linux. On Windows the panel asks Windows itself
// (tests/admin_worker.rs covers that), so the answer here is different by design.
#[cfg(not(windows))]
#[tokio::test]
async fn tidy_up_without_administrator_rights_makes_the_shortcuts_and_asks_for_the_command() {
    let (app, m, _g) = setup(false);
    m.app.lock().unwrap().legacy = true;
    let (s, j) = call(&app, "PUT", "/api/v1/app", Some(json!({"desktop_shortcut": true}))).await;
    assert_eq!(s, 403, "{j}");
    assert_eq!(j["error"]["code"], "needs_elevation");
    assert_eq!(j["error"]["command"], json!(["kora", "uninstall", "--app", "--yes"]));
    assert!(j["error"]["message"].as_str().unwrap().contains("old ones keep working"), "{j}");
    assert_eq!(m.calls(), ["app_install shortcut=true force=false dry=false"], "the per-user half is done; the old copy is untouched");
}

#[tokio::test]
async fn app_removal_as_admin() {
    let (app, m, _g) = setup(true);
    let (s, j) = call(&app, "DELETE", "/api/v1/app", Some(json!({"dry_run": true}))).await;
    assert_eq!(s, 200);
    assert!(j["message"].as_str().unwrap().contains("Nothing was changed"));
    let (s, _) = call(&app, "DELETE", "/api/v1/app", Some(json!({"dry_run": false}))).await;
    assert_eq!(s, 200);
    assert_eq!(m.calls(), ["app_uninstall dry=true", "app_uninstall dry=false"]);
}

#[tokio::test]
async fn app_removal_without_administrator_rights_gives_the_command() {
    let (app, m, _g) = setup(false);
    let (s, j) = call(&app, "DELETE", "/api/v1/app", Some(json!({"dry_run": false}))).await;
    assert_eq!(s, 403);
    assert_eq!(j["error"]["command"], json!(["kora", "uninstall", "--app", "--yes"]));
    assert!(m.calls().is_empty(), "nothing may run without elevation");
}

#[tokio::test]
async fn uninstall_plan_has_safe_defaults() {
    let (app, _m, _g) = setup(false);
    let (s, j) = call(&app, "POST", "/api/v1/uninstall/plan", None).await;
    assert_eq!(s, 200);
    let items = j["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!((items[0]["kind"].as_str(), items[0]["remove"].as_bool()), (Some("system"), Some(true)));
    assert_eq!((items[1]["kind"].as_str(), items[1]["remove"].as_bool()), (Some("folder"), Some(false)));
    let (s, j) = call(&app, "POST", "/api/v1/uninstall/plan", Some(json!({"server": "nope"}))).await;
    assert_eq!(s, 404, "{j}");
}

#[tokio::test]
async fn uninstall_run_job_and_errors() {
    let (app, m, _g) = setup(true);
    let body = json!({"answers": [{"item": "prod/service", "remove": true}, {"item": "prod/folder", "remove": false}], "delete_files": false, "delete_databases": false, "only": ["prod"]});
    let (s, j) = call(&app, "POST", "/api/v1/uninstall", Some(body.clone())).await;
    assert_eq!(s, 202, "{j}");
    let done = finish(&app, j["job"].as_str().unwrap()).await;
    assert_eq!(done["state"], "done");
    assert!(done["result"]["notes"].is_array());
    assert!(m.calls()[0].starts_with("uninstall_run answers=[(\"prod/service\", true), (\"prod/folder\", false)]"));
    // Unknown server.
    let (s, _) = call(&app, "POST", "/api/v1/uninstall", Some(json!({"only": ["ghost"]}))).await;
    assert_eq!(s, 404);
    // Failures are reported as a failed job.
    *m.outcome.lock().unwrap() = Ok(UninstallOutcome { failed: vec!["prod: service: refused".into()], ..Default::default() });
    let (_, j) = call(&app, "POST", "/api/v1/uninstall", Some(body)).await;
    let done = finish(&app, j["job"].as_str().unwrap()).await;
    assert_eq!(done["state"], "failed");
}

// Without a helper this answers "run this command" on Linux. On Windows the panel asks Windows itself
// (tests/admin_worker.rs covers that), so the answer here is different by design.
#[cfg(not(windows))]
#[tokio::test]
async fn uninstall_run_needs_elevation_without_admin() {
    let (app, m, _g) = setup(false);
    let (s, j) = call(&app, "POST", "/api/v1/uninstall", Some(json!({"answers": [], "delete_files": true, "delete_databases": false, "only": ["prod"]}))).await;
    assert_eq!(s, 403);
    assert_eq!(j["error"]["code"], "needs_elevation");
    assert_eq!(j["error"]["command"], json!(["kora", "uninstall", "--yes", "--delete-files", "prod"]));
    assert!(m.calls().is_empty());
}

#[tokio::test]
async fn bad_bodies_are_400() {
    let (app, _m, _g) = setup(true);
    let r = app
        .clone()
        .oneshot(Request::builder().method("POST").uri("/api/v1/uninstall").header(header::HOST, ADDR).header(header::AUTHORIZATION, format!("Bearer {TOKEN}")).body(Body::from("{nope")).unwrap())
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
    let _ = PlanItem { item: String::new(), description: String::new(), kind: "system", remove: true };
}
