//! The audit log through the real router: who did what is recorded for state-changing requests, a refusal is
//! recorded as such, reads and "just looking" posts are not, nothing secret is written, and only admins read it.
use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, Request};
use kora_core::api::fake::FakeBackend;
use kora_core::store::audit::{self, Query};
use kora_core::store::Store;
use kora_web::remote::{Remote, RemoteConfig, Role};
use kora_web::{router, AppState};
use serde_json::{json, Value};
use tower::ServiceExt;

const BOUND: &str = "100.64.0.5:47120";
const LAUNCH: &str = "launchtoken";
const REMOTE_PEER: &str = "100.64.0.20:51000";

struct World {
    app: axum::Router,
    home: tempfile::TempDir,
    tokens: [String; 3],
}

fn world() -> World {
    world_with(FakeBackend::sample())
}

fn world_with(fake: FakeBackend) -> World {
    let home = tempfile::tempdir().unwrap();
    let mut cfg = RemoteConfig { enabled: true, ..Default::default() };
    let v = cfg.add_user("vera", Role::Viewer).unwrap();
    let o = cfg.add_user("olly", Role::Operator).unwrap();
    let a = cfg.add_user("adam", Role::Admin).unwrap();
    let st = AppState::system(LAUNCH.into(), BOUND.parse().unwrap())
        .with_backend(Arc::new(move || Box::new(fake.clone())))
        .with_remote(Arc::new(Remote::new(None, cfg)))
        .with_audit(home.path().to_path_buf());
    World { app: router(st), home, tokens: [v, o, a] }
}

async fn send(w: &World, method: &str, uri: &str, bearer: &str, peer: Option<&str>, body: Option<Value>) -> (u16, Value) {
    let mut b = Request::builder().method(method).uri(uri).header(header::HOST, BOUND).header(header::AUTHORIZATION, format!("Bearer {bearer}"));
    let body = match body {
        Some(v) => {
            b = b.header(header::CONTENT_TYPE, "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let mut r = b.header("x-kora", "1").body(body).unwrap();
    if let Some(p) = peer {
        r.extensions_mut().insert(ConnectInfo(p.parse::<SocketAddr>().unwrap()));
    }
    let resp = w.app.clone().oneshot(r).await.unwrap();
    let status = resp.status().as_u16();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 22).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn rows(w: &World) -> Vec<audit::Row> {
    kora_web::audit::flush();
    audit::list(&Store::open(w.home.path()).unwrap(), &Query { limit: 100, ..Default::default() }).unwrap()
}

#[tokio::test]
async fn changes_are_recorded_with_who_and_how_it_went_and_reads_are_not() {
    let w = world();
    let [viewer, operator, admin] = w.tokens.clone();
    // reads and looking are not recorded
    assert_eq!(send(&w, "GET", "/api/v1/servers", &operator, Some(REMOTE_PEER), None).await.0, 200);
    assert!(send(&w, "POST", "/api/v1/checks/ports", &admin, Some(REMOTE_PEER), Some(json!({"game": 30120, "txadmin": 40120}))).await.0 / 100 != 5);
    assert!(rows(&w).is_empty(), "{:?}", rows(&w));
    // an operator restarts a server (allowed), a viewer tries (refused), this computer stops one
    let (s, _) = send(&w, "POST", "/api/v1/servers/prod/control", &operator, Some(REMOTE_PEER), Some(json!({"action": "restart"}))).await;
    assert!(s == 200 || s == 202, "{s}");
    let (s, _) = send(&w, "POST", "/api/v1/servers/prod/control", &viewer, Some(REMOTE_PEER), Some(json!({"action": "restart"}))).await;
    assert_eq!(s, 403);
    let (s, _) = send(&w, "POST", "/api/v1/servers/prod/control", LAUNCH, None, Some(json!({"action": "stop"}))).await;
    assert!(s == 200 || s == 202, "{s}");
    let all = rows(&w);
    let by = |who: &str| all.iter().filter(|r| r.actor_name.as_deref() == Some(who)).collect::<Vec<_>>();
    let olly = by("olly");
    assert!(
        !olly.is_empty()
            && olly.iter().all(|r| r.op == "server_control"
                && r.server.as_deref() == Some("prod")
                && r.actor_kind == "remote"
                && r.provider.as_deref() == Some("token")
                && r.role.as_deref() == Some("operator")),
        "{olly:?}"
    );
    let vera = by("vera");
    assert_eq!(vera.len(), 1);
    assert_eq!((vera[0].result.as_str(), vera[0].role.as_deref()), ("refused", Some("viewer")));
    assert!(all.iter().any(|r| r.actor_kind == "local" && r.op == "server_control"), "{all:?}");
    // nothing from the request body (the action) or any secret
    let text = serde_json::to_string(&all).unwrap();
    assert!(!text.contains(&operator) && !text.contains(&admin) && !text.contains(LAUNCH) && !text.contains("restart"), "{text}");
}

#[tokio::test]
async fn only_an_admin_can_read_the_log() {
    let w = world();
    let [viewer, operator, admin] = w.tokens.clone();
    send(&w, "POST", "/api/v1/servers/prod/control", &operator, Some(REMOTE_PEER), Some(json!({"action": "restart"}))).await;
    for t in [&viewer, &operator] {
        let (s, _) = send(&w, "GET", "/api/v1/audit", t, Some(REMOTE_PEER), None).await;
        assert_eq!(s, 403);
    }
    let (s, v) = send(&w, "GET", "/api/v1/audit?limit=10", &admin, Some(REMOTE_PEER), None).await;
    assert_eq!(s, 200, "{v}");
    assert!(v["entries"].as_array().unwrap().iter().any(|e| e["actor_name"] == "olly" && e["op"] == "server_control"), "{v}");
    assert_eq!(v["retention_days"], 365);
    let (_, v) = send(&w, "GET", "/api/v1/audit?actor=nobody", &admin, Some(REMOTE_PEER), None).await;
    assert!(v["entries"].as_array().unwrap().is_empty());
    // this computer sees it too
    assert_eq!(send(&w, "GET", "/api/v1/audit", LAUNCH, None, None).await.0, 200);
}

#[tokio::test]
async fn a_slow_job_leaves_a_started_row_and_a_row_for_how_it_ended() {
    let w = world();
    let [_, _, admin] = w.tokens.clone();
    let (s, v) = send(&w, "POST", "/api/v1/servers/prod/update/apply", &admin, Some(REMOTE_PEER), Some(json!({}))).await;
    assert!(s == 202 || s == 200, "{s} {v}");
    // wait for the job's end row
    let mut got = vec![];
    for _ in 0..100 {
        got = rows(&w);
        if got.iter().any(|r| r.op == "update_apply" && (r.result == "ok" || r.result == "failed")) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let mine: Vec<_> = got.iter().filter(|r| r.op == "update_apply").collect();
    assert!(mine.iter().any(|r| r.result == "started" && r.detail.as_deref().is_some_and(|d| d.starts_with("job "))), "{got:?}");
    assert!(mine.iter().any(|r| r.result == "ok" || r.result == "failed"), "{got:?}");
    assert!(mine.iter().all(|r| r.server.as_deref() == Some("prod") && r.actor_name.as_deref() == Some("adam")), "{mine:?}");
}

#[tokio::test]
async fn nothing_is_recorded_when_the_panel_has_no_audit_home() {
    // the default (tests, embedders): no disk is touched
    let fake = FakeBackend::sample();
    let st = AppState::system(LAUNCH.into(), BOUND.parse().unwrap()).with_backend(Arc::new(move || Box::new(fake.clone())));
    let app = router(st);
    let r = Request::builder()
        .method("POST")
        .uri("/api/v1/servers/prod/control")
        .header(header::HOST, BOUND)
        .header(header::AUTHORIZATION, format!("Bearer {LAUNCH}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({"action": "stop"}).to_string()))
        .unwrap();
    let resp = app.oneshot(r).await.unwrap();
    assert!(resp.status().is_success());
}

fn world_with_registry(home: &std::path::Path, registry: kora_core::api::JobRegistry, tokens: &RemoteConfig) -> axum::Router {
    let fake = FakeBackend::sample();
    let st = AppState::system(LAUNCH.into(), BOUND.parse().unwrap())
        .with_backend(Arc::new(move || Box::new(fake.clone())))
        .with_registry(registry)
        .with_remote(Arc::new(Remote::new(None, tokens.clone())))
        .with_audit(home.to_path_buf());
    router(st)
}

#[tokio::test]
async fn the_jobs_list_survives_a_restart_and_shows_an_interrupted_job_with_its_guidance() {
    let home = tempfile::tempdir().unwrap();
    let mut cfg = RemoteConfig { enabled: true, ..Default::default() };
    let operator = cfg.add_user("olly", Role::Operator).unwrap();
    let admin = cfg.add_user("adam", Role::Admin).unwrap();
    let w = World {
        app: world_with_registry(home.path(), kora_core::api::JobRegistry::durable_in(home.path().to_path_buf()), &cfg),
        home: tempfile::tempdir().unwrap(),
        tokens: [String::new(), operator.clone(), admin.clone()],
    };
    let (s, v) = send(&w, "POST", "/api/v1/servers/prod/update/apply", &admin, Some(REMOTE_PEER), Some(json!({}))).await;
    assert!(s == 202 || s == 200, "{s} {v}");
    let id = v["job"].as_str().unwrap().to_string();
    for _ in 0..100 {
        let (_, j) = send(&w, "GET", &format!("/api/v1/jobs/{id}"), &admin, Some(REMOTE_PEER), None).await;
        if j["state"] == "done" || j["state"] == "failed" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    }
    // an install that was running when the "panel" died: a running row owned by a process that is gone
    Store::open(home.path())
        .unwrap()
        .write(|tx| {
            tx.execute(
                "INSERT INTO jobs (id, op, server, state, started, owner_pid, owner_instance) VALUES ('deadbeefdeadbeef', 'install_start', 'Staging', 'running', 1700000000, 4000000, 'gone')",
                [],
            )?;
            Ok(())
        })
        .unwrap();

    // a NEW panel starts on the same Kora folder
    let w2 = World {
        app: world_with_registry(home.path(), kora_core::api::JobRegistry::durable_with(home.path().to_path_buf(), &|_| false), &cfg),
        home: tempfile::tempdir().unwrap(),
        tokens: [String::new(), operator.clone(), admin.clone()],
    };
    let (s, list) = send(&w2, "GET", "/api/v1/jobs?limit=10", &admin, Some(REMOTE_PEER), None).await;
    assert_eq!(s, 200, "{list}");
    let jobs = list["jobs"].as_array().unwrap();
    let old = jobs.iter().find(|j| j["id"] == id.as_str()).expect("the finished job is still listed");
    assert!(old["state"] == "done" || old["state"] == "failed", "{old}");
    let dead = jobs.iter().find(|j| j["id"] == "deadbeefdeadbeef").expect("the interrupted job is listed");
    assert_eq!(dead["state"], "interrupted");
    assert_eq!(dead["error"]["code"], "job.interrupted");
    assert!(dead["error"]["message"].as_str().unwrap().contains("did not finish"), "{dead}");
    // the single-job route reads it too, with the same guidance
    let (s, one) = send(&w2, "GET", "/api/v1/jobs/deadbeefdeadbeef", &admin, Some(REMOTE_PEER), None).await;
    assert_eq!((s, one["state"].as_str()), (200, Some("interrupted")), "{one}");
    // a remote operator does not see jobs of operations above their role (an install is admin-only)
    let (_, list) = send(&w2, "GET", "/api/v1/jobs", &operator, Some(REMOTE_PEER), None).await;
    assert!(list["jobs"].as_array().unwrap().iter().all(|j| j["op"] != "install_start"), "{list}");
}

#[tokio::test]
async fn acting_on_github_is_recorded_with_the_account_that_acted_and_never_the_text() {
    use kora_core::collab::{Acted, Reply};
    let fake = FakeBackend::sample();
    fake.state().collab = Some(Ok(Reply::Acted(Acted { message: "Pull request #12 was merged (squash).".into(), acting_as: Some("sam-rp".into()), url: None })));
    let w = world_with(fake);
    let [_viewer, operator, admin] = w.tokens.clone();
    let uri = "/api/v1/servers/dev/resources/kora_hud/prs/12";
    // An operator may not merge or comment (refused, and recorded as refused).
    let (s, _) = send(&w, "POST", &format!("{uri}/merge"), &operator, Some(REMOTE_PEER), Some(json!({"method": "squash"}))).await;
    assert_eq!(s, 403);
    // An admin over a token is held back from the destructive ones unless remote actions are on; a comment is fine.
    let (s, _) = send(&w, "POST", &format!("{uri}/comment"), &admin, Some(REMOTE_PEER), Some(json!({"body": "TOPSECRETTEXT looks fine"}))).await;
    assert_eq!(s, 200);
    let (s, _) = send(&w, "POST", &format!("{uri}/merge"), LAUNCH, None, Some(json!({"method": "squash"}))).await;
    assert_eq!(s, 200);
    let all = rows(&w);
    let merge: Vec<_> = all.iter().filter(|r| r.op == "pr_merge").collect();
    assert!(merge.iter().any(|r| r.result == "refused" && r.actor_name.as_deref() == Some("olly")), "{all:?}");
    let ok = merge.iter().find(|r| r.result == "ok").expect("the merge is in the log");
    assert!(ok.detail.as_deref().unwrap_or("").contains("GitHub account sam-rp"), "{ok:?}");
    let comment = all.iter().find(|r| r.op == "pr_comment" && r.result == "ok").expect("the comment is in the log");
    assert!(comment.detail.as_deref().unwrap_or("").contains("sam-rp"));
    // Nothing of what was written reaches the log.
    assert!(!serde_json::to_string(&all).unwrap().contains("TOPSECRETTEXT"));
}
