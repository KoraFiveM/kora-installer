//! Every operation in `Op::ALL` has a working route: it is mounted with the right method and is not a
//! stub. (What each one answers is tested next to its handler.)
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request};
use kora_core::api::fake::FakeBackend;
use kora_core::api::maintenance::{self, fake::FakeMaintenance};
use kora_core::api::{Method, Op};
use kora_web::{router, AppState};
use serde_json::Value;
use tower::ServiceExt;

const TOKEN: &str = "testtoken";
const ADDR: &str = "127.0.0.1:47120";

/// Operations other agents are implementing in parallel right now (everything in servers.rs, console.rs,
/// system.rs, updates.rs, resources.rs, settings.rs, install.rs and github.rs): they may still answer
/// 501 `not_implemented`.
///
/// MUST BE EMPTY AT MERGE (`allowed_501_is_empty_at_merge` below checks it; it is ignored until then).
/// The maintenance, jobs, meta and session operations are never allowed here.
const ALLOWED_501: &[Op] = &[
    Op::SystemInfo,
    Op::ServersList,
    Op::ServerGet,
    Op::ServerControl,
    Op::ServerDelete,
    Op::ServerUnregister,
    Op::ServersScan,
    Op::ServersAdopt,
    Op::ServerOpenFolder,
    Op::LogsList,
    Op::LogsTail,
    Op::ConsoleStream,
    Op::ConsoleSend,
    Op::ConsoleEnable,
    Op::ResourcesList,
    Op::ResourceEnsure,
    Op::UpdatePreview,
    Op::UpdateApply,
    Op::UpdateAuto,
    Op::SnapshotsList,
    Op::SnapshotRollback,
    Op::CatalogForServer,
    Op::ResourcesPlan,
    Op::ResourcesAdd,
    Op::WebhookGet,
    Op::WebhookSet,
    Op::WebhookRemove,
    Op::WebhookTest,
    Op::ScheduleGet,
    Op::ScheduleSet,
    Op::ScheduleRemove,
    Op::PortsSet,
    Op::IssueReport,
    Op::PrPlan,
    Op::PrOpen,
    Op::GithubStatus,
    Op::GithubLogin,
    Op::GithubOwners,
    Op::CatalogGet,
    Op::ArtifactsList,
    Op::InstallDefaults,
    Op::InstallSavedClear,
    Op::CheckPorts,
    Op::CheckDisk,
    Op::CheckDbLogin,
    Op::InstallPlan,
    Op::InstallStart,
    Op::InstallLog,
    Op::Events,
];

fn uri_for(op: Op) -> String {
    let path = op
        .route()
        .1
        .replace("{s}", "prod")
        .replace("{id}", "fakeid")
        .replace("{file}", "server.log")
        .replace("{job}", "fakejob");
    format!("/api/v1{path}")
}

#[tokio::test]
async fn every_op_has_a_real_route() {
    // Maintenance operations must never touch the machine: they run against a fake.
    maintenance::set_override(Some(FakeMaintenance::new(false)));
    let fake = FakeBackend::sample();
    let st = AppState::system(TOKEN.into(), ADDR.parse().unwrap())
        .with_backend(Arc::new(move || Box::new(fake.clone())));
    let app = router(st);
    let mut bad = Vec::new();
    for op in Op::ALL {
        let method = match op.route().0 {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
        };
        let req = Request::builder()
            .method(method)
            .uri(uri_for(*op))
            .header(header::HOST, ADDR)
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(if method == "GET" || method == "DELETE" {
                Body::empty()
            } else {
                Body::from("{}")
            })
            .unwrap();
        let r = app.clone().oneshot(req).await.unwrap();
        let status = r.status().as_u16();
        // Streams (console, events) never finish: look at the head only.
        let is_stream = r
            .headers()
            .get(header::CONTENT_TYPE)
            .is_some_and(|v| v.to_str().unwrap_or("").starts_with("text/event-stream"));
        let (code, message) = if is_stream {
            (String::new(), String::new())
        } else {
            let bytes = axum::body::to_bytes(r.into_body(), 1 << 22).await.unwrap();
            let j: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            (
                j["error"]["code"].as_str().unwrap_or("").to_string(),
                j["error"]["message"].as_str().unwrap_or("").to_string(),
            )
        };
        let allowed = ALLOWED_501.contains(op);
        let stub = status == 501 || code == "not_implemented";
        let missing = status == 405 || (status == 404 && message == "unknown API route");
        if missing || (stub && !allowed) {
            bad.push(format!(
                "{} {} {} -> {status} {code} {message}",
                op.name(),
                method,
                uri_for(*op)
            ));
        }
    }
    maintenance::set_override(None);
    assert!(
        bad.is_empty(),
        "operations without a working route:\n{}",
        bad.join("\n")
    );
}

#[test]
fn maintenance_ops_are_never_allowed_to_be_stubs() {
    for op in [
        Op::SelfUpdateCheck,
        Op::SelfUpdateApply,
        Op::AppStatus,
        Op::AppInstall,
        Op::AppUninstall,
        Op::UninstallPlan,
        Op::UninstallRun,
        Op::Meta,
        Op::SessionCreate,
        Op::JobsGet,
        Op::JobEvents,
    ] {
        assert!(!ALLOWED_501.contains(&op), "{op:?}");
    }
}

#[test]
#[ignore = "only meaningful once every handler is implemented (run with --ignored at merge)"]
fn allowed_501_is_empty_at_merge() {
    assert!(
        ALLOWED_501.is_empty(),
        "ALLOWED_501 must be empty at merge: {ALLOWED_501:?}"
    );
}
