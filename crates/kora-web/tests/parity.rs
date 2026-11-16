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
const ALLOWED_501: &[Op] = &[];

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
    offline::install();
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
        let t0 = std::time::Instant::now();
        let r = app.clone().oneshot(req).await.unwrap();
        // Offline fakes answer at once; a slow route means something real ran (network, gh, ...).
        if t0.elapsed() > std::time::Duration::from_secs(2) {
            bad.push(format!("{} took {:?}: it touched the real machine or network", op.name(), t0.elapsed()));
        }
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
    offline::remove();
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
fn allowed_501_is_empty_at_merge() {
    assert!(
        ALLOWED_501.is_empty(),
        "ALLOWED_501 must be empty at merge: {ALLOWED_501:?}"
    );
}

/// Offline stand-ins for every seam that would otherwise reach the machine, the network or `gh`.
mod offline {
    use std::io;
    use std::path::Path;
    use std::sync::Arc;

    use kora_core::api::install::github::{GithubEnv, GithubUser, LoginFlow, OwnerChoice};
    use kora_core::api::install::InstallEnv;
    use kora_core::artifacts::ArtifactList;
    use kora_core::firewall::ports::PortProber;
    use kora_core::installer::{DbAdmin, InstallOutcome, InstallRequest, ProgressSink};
    use kora_core::space::DiskInfo;
    use kora_web::handlers::{github, hub, install, resources, servers, system, updates};

    struct Env;
    impl PortProber for Env {
        fn tcp_free(&self, _: u16) -> bool {
            true
        }
        fn udp_free(&self, _: u16) -> bool {
            true
        }
    }
    impl DiskInfo for Env {
        fn free_bytes(&self, _: &Path) -> io::Result<u64> {
            Ok(100 << 30)
        }
        fn volume_key(&self, _: &Path) -> String {
            "v".into()
        }
    }
    impl InstallEnv for Env {
        fn prober(&self) -> &dyn PortProber {
            self
        }
        fn disk(&self) -> &dyn DiskInfo {
            self
        }
        fn db_login(&self, _: Option<&DbAdmin>) -> Result<(), String> {
            Ok(())
        }
        fn artifacts(&self) -> Result<ArtifactList, String> {
            Err("offline".into())
        }
        fn is_elevated(&self) -> bool {
            true
        }
        fn admin_reasons(&self, _: &InstallRequest) -> Vec<String> {
            vec![]
        }
        fn saved_load(&self) -> Option<InstallRequest> {
            None
        }
        fn saved_save(&self, _: &InstallRequest) -> bool {
            true
        }
        fn saved_clear(&self) {}
        fn run(&self, _: &InstallRequest, _: &mut dyn ProgressSink) -> Result<InstallOutcome, String> {
            Err("offline".into())
        }
    }
    impl GithubEnv for Env {
        fn auth(&self) -> GithubUser {
            GithubUser { signed_in: false, user: None }
        }
        fn owners(&self) -> Result<Vec<OwnerChoice>, String> {
            Ok(vec![])
        }
        fn begin_login(&self) -> Result<LoginFlow, String> {
            Err("offline".into())
        }
        fn resolve_owner(&self, _: Option<&str>) -> Result<String, String> {
            Err("offline".into())
        }
    }

    pub fn install() {
        install::set_env(Some(Arc::new(Env)));
        github::set_env(Some(Arc::new(Env)));
        resources::set_catalog_loader(Some(Arc::new(|| Err("offline".into()))));
        updates::set_auto_runner(Some(Arc::new(|_| Err("offline".into()))));
        servers::set_opener(Some(Box::new(|_| Ok(()))));
        system::seam::set_headless(Some(true));
        // Hub: an empty nodes file that does not exist, and a transport that never reaches the network.
        hub::set_store_path(Some(std::env::temp_dir().join(format!("kora-parity-hub-{}.json", std::process::id()))));
        hub::set_transport(Some(Arc::new(NoNet)));
    }

    pub fn remove() {
        install::set_env(None);
        github::set_env(None);
        resources::set_catalog_loader(None);
        updates::set_auto_runner(None);
        servers::set_opener(None);
        system::seam::set_headless(None);
        hub::set_store_path(None);
        hub::set_transport(None);
    }

    struct NoNet;
    impl kora_core::hub::Transport for NoNet {
        fn send(&self, _: &kora_core::hub::HttpRequest) -> Result<kora_core::hub::HttpResponse, String> {
            Err("offline".into())
        }
    }
}
