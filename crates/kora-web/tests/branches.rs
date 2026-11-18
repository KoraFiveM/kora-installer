//! Branches and pull requests of a Development server's resources, driven through the router with a
//! fake backend (nothing touches git or GitHub).

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{header, Request, Response};
use kora_core::api::fake::FakeBackend;
use kora_core::branches::{BulkItem, BulkPreview, BulkResult, BulkRow, BulkStatus, Choices, PrChoices, Reply, ResourceBranch, State, SwitchResult};
use kora_core::git::branches::{codes, Branches, Changes, Kind, Outcome, PrSource, PullRequest, Stash, Status, Where};
use kora_core::problem::{known, Action, Problem};
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

async fn run_job(app: &axum::Router, uri: &str, body: Value) -> Value {
    let (s, v) = call(app, "POST", uri, Some(body)).await;
    assert_eq!(s, 202, "{v}");
    let id = v["job"].as_str().unwrap().to_string();
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        let (_, j) = call(app, "GET", &format!("/jobs/{id}"), None).await;
        if j["state"] != "running" {
            return j;
        }
        assert!(Instant::now() < end, "job did not finish");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn where_(kind: Kind, branch: &str) -> Where {
    Where { kind, branch: Some(branch.into()), pr: None, changed_files: vec![], changed_count: 0 }
}

fn row(id: &str) -> ResourceBranch {
    ResourceBranch {
        id: id.into(),
        folder: id.into(),
        state: State::Default,
        branch: Some("dev".into()),
        pr: None,
        pr_title: None,
        default_branch: "dev".into(),
        label: "the normal version (dev)".into(),
        changed_count: 0,
        stashes: 0,
        note: None,
        problem: None,
    }
}

fn switched(status: Status) -> Reply {
    Reply::Switched(Box::new(SwitchResult {
        resource: "kora_hud".into(),
        outcome: Outcome {
            status,
            now: where_(Kind::Branch, "feature"),
            stashed: Some(Stash { index: 0, message: "kept aside".into(), branch: "dev".into() }),
            saved_branch: None,
            title: None,
            message: "Now on branch feature.".into(),
        },
        needs_restart: status == Status::Switched,
        restart_note: Some("restart".into()),
    }))
}

#[tokio::test]
async fn the_overview_and_the_choices_are_served_for_every_server() {
    let fake = FakeBackend::sample();
    fake.state().branches = Some(Ok(Reply::Overview(vec![row("kora_hud")])));
    let app = app(&fake);
    let (s, v) = call(&app, "GET", "/servers/dev/branches", None).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["resources"][0]["label"], "the normal version (dev)");
    // A server that follows released versions gets the same operations (the panel warns when a resource leaves its release).
    let (s, _) = call(&app, "GET", "/servers/prod/branches", None).await;
    assert_eq!(s, 200);
    assert_eq!(fake.calls().iter().filter(|c| c.starts_with("branches prod")).count(), 1);

    fake.state().branches = Some(Ok(Reply::Choices(Box::new(Choices {
        resource: row("kora_hud"),
        branches: Branches { current: Some("dev".into()), default_branch: "dev".into(), local: vec!["dev".into()], fork: vec!["dev".into(), "x".into()], upstream: vec!["dev".into()] },
        stashes: vec![],
        notes: vec![],
    }))));
    let (s, v) = call(&app, "GET", "/servers/dev/resources/kora_hud/branches?refresh=1", None).await;
    assert_eq!(s, 200);
    assert_eq!(v["branches"]["fork"][1], "x");
    assert!(fake.calls().iter().any(|c| c.contains("Choices") && c.contains("refresh: true")), "{:?}", fake.calls());
    let (s, _) = call(&app, "GET", "/servers/dev/resources/kora_hud/branches", None).await;
    assert_eq!(s, 200);
    assert!(fake.calls().iter().any(|c| c.contains("Choices") && c.contains("refresh: false")));
}

#[tokio::test]
async fn pull_requests_come_back_labelled_and_sign_in_problems_keep_their_actions() {
    let fake = FakeBackend::sample();
    let pr = PullRequest {
        number: 5,
        title: "Fix the HUD".into(),
        author: "sam".into(),
        source: PrSource::Upstream,
        repo: "KoraFiveM/kora_hud".into(),
        branch: "fix".into(),
        from: Some("sam/kora_hud".into()),
        draft: false,
        url: "https://github.com/KoraFiveM/kora_hud/pull/5".into(),
        updated: "2026-01-01T00:00:00Z".into(),
        created: "2025-12-01T00:00:00Z".into(),
        review: String::new(),
        checks: kora_core::collab::Checks::None,
    };
    fake.state().branches = Some(Ok(Reply::Prs(PrChoices { prs: vec![pr], notes: vec![] })));
    let app = app(&fake);
    let (s, v) = call(&app, "GET", "/servers/dev/resources/kora_hud/prs", None).await;
    assert_eq!(s, 200);
    assert_eq!((v["prs"][0]["source"].as_str(), v["prs"][0]["author"].as_str()), (Some("upstream"), Some("sam")));
    fake.state().branches = Some(Err(known::github_signed_out()));
    let (s, v) = call(&app, "GET", "/servers/dev/resources/kora_hud/prs", None).await;
    assert_eq!(s, 422);
    assert_eq!(v["error"]["code"], "github.signed_out");
    assert_eq!(v["error"]["actions"][0]["kind"], "sign_in_github");
}

#[tokio::test]
async fn switching_checks_what_it_is_given_and_passes_the_choice_on() {
    let fake = FakeBackend::sample();
    fake.state().branches = Some(Ok(switched(Status::NeedsChoice)));
    let app = app(&fake);
    let u = "/servers/dev/resources/kora_hud/branch";
    for bad in [
        json!({}),
        json!({"action": "nope"}),
        json!({"action": "branch", "branch": "-x"}),
        json!({"action": "branch", "branch": "a b"}),
        json!({"action": "branch", "branch": "a..b"}),
        json!({"action": "branch", "branch": "ok", "remote": "evil"}),
        json!({"action": "pr", "pr": "12; rm -rf /"}),
        json!({"action": "pr", "pr": "https://example.com/x/y/pull/1"}),
        json!({"action": "pr", "pr": 0}),
        json!({"action": "pr", "pr": 5, "source": "elsewhere"}),
        json!({"action": "branch", "branch": "ok", "changes": "yolo"}),
    ] {
        let (s, v) = call(&app, "POST", u, Some(bad.clone())).await;
        assert_eq!(s, 400, "{bad}: {v}");
    }
    let (s, _) = call(&app, "POST", "/servers/dev/resources/..%2Fx/branch", Some(json!({"action": "default"}))).await;
    assert!(s == 400 || s == 404, "{s}");
    assert_eq!(fake.calls().iter().filter(|c| c.starts_with("branches dev")).count(), 0, "bad requests never reach the backend");

    let (s, v) = call(&app, "POST", u, Some(json!({"action": "branch", "branch": "feature", "remote": "upstream"}))).await;
    assert_eq!((s, v["status"].as_str()), (200, Some("needs_choice")));
    assert_eq!(v["stashed"]["index"], 0);
    assert!(fake.calls().last().unwrap().contains(r#"Branch { name: "feature", remote: Some("upstream") }"#) && fake.calls().last().unwrap().contains("changes: Ask"));
    let (s, _) = call(&app, "POST", u, Some(json!({"action": "pr", "pr": "https://github.com/KoraFiveM/kora_hud/pull/45", "source": "upstream", "changes": "stash"}))).await;
    assert_eq!(s, 200);
    let last = fake.calls().last().unwrap().clone();
    assert!(last.contains("Pr {") && last.contains("changes: Stash") && last.contains("Upstream"), "{last}");
    let (s, _) = call(&app, "POST", u, Some(json!({"action": "pr", "pr": 12, "changes": "discard"}))).await;
    assert_eq!(s, 200);
    assert!(fake.calls().last().unwrap().contains(r#"input: "12""#) && fake.calls().last().unwrap().contains("Discard"));
    let (s, _) = call(&app, "POST", u, Some(json!({"action": "default"}))).await;
    assert_eq!(s, 200);
    assert!(fake.calls().last().unwrap().contains("Default"));
    assert_eq!(Changes::default(), Changes::Ask);
}

#[tokio::test]
async fn failures_arrive_as_guided_problems_and_a_running_update_is_a_conflict() {
    let fake = FakeBackend::sample();
    let app = app(&fake);
    let u = "/servers/dev/resources/kora_hud/branch";
    let closed = Problem::new(codes::PR_CLOSED, "Pull request #5 has already been merged.").with_action(Action::open_link("https://github.com/o/n/pull/5"));
    fake.state().branches = Some(Err(closed));
    let (s, v) = call(&app, "POST", u, Some(json!({"action": "pr", "pr": 5}))).await;
    assert_eq!((s, v["error"]["code"].as_str()), (422, Some(codes::PR_CLOSED)));
    assert_eq!(v["error"]["actions"][0]["kind"], "open_link");
    let busy = Problem::new(codes::BUSY, "An update is running for this server right now.");
    fake.state().branches = Some(Err(busy));
    let (s, v) = call(&app, "POST", u, Some(json!({"action": "default"}))).await;
    assert_eq!((s, v["error"]["code"].as_str()), (409, Some(codes::BUSY)));
}

#[tokio::test]
async fn kept_aside_changes_are_brought_back_by_index() {
    let fake = FakeBackend::sample();
    fake.state().branches = Some(Ok(switched(Status::Switched)));
    let app = app(&fake);
    let u = "/servers/dev/resources/kora_hud/stash/restore";
    for bad in [json!({}), json!({"index": -1}), json!({"index": "0"}), json!({"index": 5000})] {
        assert_eq!(call(&app, "POST", u, Some(bad)).await.0, 400);
    }
    let (s, v) = call(&app, "POST", u, Some(json!({"index": 2}))).await;
    assert_eq!((s, v["needs_restart"].as_bool()), (200, Some(true)));
    assert!(fake.calls().last().unwrap().contains("RestoreStash") && fake.calls().last().unwrap().contains("index: 2"));
}

#[tokio::test]
async fn bulk_preview_and_apply_are_jobs_with_progress() {
    let fake = FakeBackend::sample();
    fake.state().branches = Some(Ok(Reply::BulkPreview(BulkPreview {
        branch: "feature".into(),
        rows: vec![BulkRow { id: "kora_hud".into(), has: true, from: Some("origin".into()), already: false, now: "the normal version (dev)".into(), changed_count: 0, note: None }],
        have: 1,
        missing: 0,
        notes: vec![],
    })));
    let app = app(&fake);
    assert_eq!(call(&app, "POST", "/servers/dev/branches/preview", Some(json!({"branch": "-x"}))).await.0, 400);
    let j = run_job(&app, "/servers/dev/branches/preview", json!({"branch": "feature"})).await;
    assert_eq!(j["state"], "done", "{j}");
    assert_eq!(j["result"]["have"], 1);
    assert!(fake.calls().iter().any(|c| c.contains("BulkPreview") && c.contains("fetch: true")));

    fake.state().branches = Some(Ok(Reply::BulkApply(BulkResult {
        items: vec![BulkItem { id: "kora_hud".into(), status: BulkStatus::NeedsChoice, message: "changes".into(), stashed: None, files: vec!["a.lua".into()], problem: None }],
        switched: 0,
        needs_choice: 1,
        failed: 0,
        skipped: 0,
        needs_restart: false,
        restart_note: None,
    })));
    for bad in [json!({"action": "pr", "pr": 5}), json!({"action": "branch", "branch": "ok", "resources": []}), json!({"action": "branch", "branch": "ok", "resources": ["a b"]})] {
        assert_eq!(call(&app, "POST", "/servers/dev/branches/apply", Some(bad)).await.0, 400);
    }
    let j = run_job(&app, "/servers/dev/branches/apply", json!({"action": "branch", "branch": "feature", "resources": ["kora_hud"], "changes": "stash"})).await;
    assert_eq!(j["state"], "done", "{j}");
    assert_eq!(j["result"]["items"][0]["status"], "needs_choice");
    let last = fake.calls().last().unwrap().clone();
    assert!(last.contains("BulkApply") && last.contains(r#"Some(["kora_hud"])"#) && last.contains("Stash"), "{last}");
    let j = run_job(&app, "/servers/dev/branches/apply", json!({"action": "default"})).await;
    assert_eq!(j["state"], "done");
}
