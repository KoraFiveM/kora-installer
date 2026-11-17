//! Monitoring routes: metrics, alerts, acknowledging and per-server alert settings. The sampler runs
//! against a fake machine and a recording sink: nothing here touches the real machine or network.
use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, Response};
use kora_core::api::fake::FakeBackend;
use kora_core::api::Status;
use kora_core::installer::registry::{PortsRecord, ServerEntry};
use kora_core::installer::Mode;
use kora_core::monitor::alerts::{Alert, AlertSettings};
use kora_core::monitor::{AlertSink, DiskUsage, GameInfo, MemUsage, Monitor, Probe, ProcUsage};
use kora_web::handlers::monitor as handler;
use kora_web::{router, AppState};
use serde_json::{json, Value};
use tower::ServiceExt;

const ADDR: &str = "127.0.0.1:47120";
const TOKEN: &str = "testtoken";
const SECRET_URL: &str = "https://hooks.example.test/very/secret/abc123?key=zzz";

struct Machine;
impl Probe for Machine {
    fn dir_exists(&self, _: &Path) -> bool {
        true
    }
    fn game_info(&self, _: u16) -> Option<GameInfo> {
        Some(GameInfo { players: 7, max_players: Some(32) })
    }
    fn process(&self, _: &ServerEntry) -> Option<ProcUsage> {
        Some(ProcUsage { cpu_pct: Some(5.0), rss_bytes: 2 << 30 })
    }
    fn disk(&self, _: &Path) -> Option<DiskUsage> {
        Some(DiskUsage { free: 200 << 30, total: 500 << 30 })
    }
    fn memory(&self) -> Option<MemUsage> {
        Some(MemUsage { total: 16 << 30, available: 8 << 30 })
    }
}

struct Sink;
impl AlertSink for Sink {
    fn deliver(&self, _: &ServerEntry, _: &Alert, _: &AlertSettings) {}
}

struct Rig {
    app: axum::Router,
    fake: FakeBackend,
    mon: Monitor,
    // Keeps the temp dir alive; some tests read the saved settings.
    dir: tempfile::TempDir,
}

/// The monitor lives in one slot per process: tests that use it take turns.
static TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn rig(install: bool) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let fake = FakeBackend::new();
    {
        let mut s = fake.state();
        s.servers.push(ServerEntry {
            source: None,
            versions: None,
            name: "prod".into(),
            dir: dir.path().to_path_buf(),
            mode: Mode::Production,
            ports: PortsRecord { game: 30120, txadmin: 40120 },
            created_unix: 0,
        });
        s.status.insert("prod".into(), Status::Running);
    }
    let f = fake.clone();
    let make: kora_web::BackendFactory = Arc::new(move || Box::new(f.clone()));
    let mon = Monitor::new(make.clone(), Arc::new(Machine), Arc::new(Sink));
    handler::install(install.then(|| mon.clone()));
    let st = AppState::system(TOKEN.into(), ADDR.parse().unwrap()).with_backend(make);
    Rig { app: router(st), fake, mon, dir }
}

fn req(method: &str, uri: &str, body: Option<Value>) -> Request<Body> {
    let b = Request::builder().method(method).uri(uri).header(header::HOST, ADDR).header(header::AUTHORIZATION, format!("Bearer {TOKEN}")).header("x-kora", "1");
    match body {
        Some(v) => b.header(header::CONTENT_TYPE, "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    }
}

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (u16, String) {
    let r: Response<Body> = app.clone().oneshot(req(method, uri, body)).await.unwrap();
    let status = r.status().as_u16();
    let bytes = axum::body::to_bytes(r.into_body(), 1 << 22).await.unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

async fn json_call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (u16, Value) {
    let (s, t) = call(app, method, uri, body).await;
    (s, serde_json::from_str(&t).unwrap_or(Value::Null))
}

fn now() -> u64 {
    kora_core::monitor::now_unix()
}

#[tokio::test]
async fn metrics_return_samples_and_the_latest() {
    let _t = TURN.lock().await;
    let r = rig(true);
    let n = now();
    r.mon.tick(n - 30);
    r.mon.tick(n - 15);
    r.mon.tick(n);
    let (st, v) = json_call(&r.app, "GET", "/api/v1/servers/prod/metrics?range=1h", None).await;
    assert_eq!(st, 200);
    assert_eq!(v["samples"].as_array().unwrap().len(), 3);
    let l = &v["latest"];
    assert_eq!((l["up"].as_bool(), l["players"].as_u64(), l["max_players"].as_u64(), l["ram_mb"].as_u64()), (Some(true), Some(7), Some(32), Some(2048)));
    assert_eq!(l["disk_free_mb"].as_u64(), Some(204_800));
    // The other ranges and the default work; unknown ones are refused.
    for q in ["", "?range=6h", "?range=24h"] {
        let (st, v) = json_call(&r.app, "GET", &format!("/api/v1/servers/prod/metrics{q}"), None).await;
        assert_eq!((st, v["samples"].as_array().map(Vec::len)), (200, Some(3)), "{q}");
    }
    let (st, v) = json_call(&r.app, "GET", "/api/v1/servers/prod/metrics?range=1y", None).await;
    assert_eq!((st, v["error"]["code"].as_str()), (400, Some("bad_request")));
    let (st, v) = json_call(&r.app, "GET", "/api/v1/servers/nope/metrics", None).await;
    assert_eq!((st, v["error"]["code"].as_str()), (404, Some("not_found")));
    handler::install(None);
}

#[tokio::test]
async fn without_a_monitor_everything_is_empty() {
    let _t = TURN.lock().await;
    let r = rig(false);
    let (st, v) = json_call(&r.app, "GET", "/api/v1/servers/prod/metrics", None).await;
    assert_eq!((st, v["samples"].as_array().map(Vec::len), v["latest"].is_null()), (200, Some(0), true));
    let (st, v) = json_call(&r.app, "GET", "/api/v1/alerts", None).await;
    assert_eq!((st, v["alerts"].as_array().map(Vec::len)), (200, Some(0)));
    let (st, _) = json_call(&r.app, "POST", "/api/v1/alerts/1/ack", None).await;
    assert_eq!(st, 404);
}

#[tokio::test]
async fn a_crash_shows_up_in_the_alert_list_and_can_be_acknowledged() {
    let _t = TURN.lock().await;
    let r = rig(true);
    let n = now();
    r.mon.tick(n - 30);
    r.fake.state().status.insert("prod".into(), Status::Stopped);
    r.mon.tick(n - 15);
    let (st, v) = json_call(&r.app, "GET", "/api/v1/alerts", None).await;
    assert_eq!(st, 200);
    let a = &v["alerts"][0];
    assert_eq!((a["server"].as_str(), a["kind"].as_str(), a["tone"].as_str(), a["acked"].as_bool()), (Some("prod"), Some("stopped"), Some("danger"), Some(false)));
    assert!(a["resolved"].is_null() && a["title"].is_string() && a["message"].is_string() && a["raised"].is_u64());
    let id = a["id"].as_u64().unwrap();
    let (st, _) = call(&r.app, "POST", &format!("/api/v1/alerts/{id}/ack"), None).await;
    assert_eq!(st, 204);
    let (_, v) = json_call(&r.app, "GET", "/api/v1/alerts", None).await;
    assert_eq!(v["alerts"][0]["acked"].as_bool(), Some(true));
    let (st, v) = json_call(&r.app, "POST", "/api/v1/alerts/99999/ack", None).await;
    assert_eq!((st, v["error"]["code"].as_str()), (404, Some("not_found")));
    let (st, v) = json_call(&r.app, "POST", "/api/v1/alerts/abc/ack", None).await;
    assert_eq!((st, v["error"]["code"].as_str()), (400, Some("bad_request")));
    handler::install(None);
}

#[tokio::test]
async fn alert_settings_have_defaults_update_partially_and_never_return_the_webhook() {
    let _t = TURN.lock().await;
    let r = rig(false);
    let (st, v) = json_call(&r.app, "GET", "/api/v1/servers/prod/alerts", None).await;
    assert_eq!(st, 200);
    assert_eq!((v["enabled"].as_bool(), v["disk_min_gb"].as_f64(), v["disk_min_percent"].as_f64(), v["ram_max_percent"].as_f64()), (Some(true), Some(5.0), Some(5.0), Some(90.0)));
    assert_eq!(v["rules"]["stopped"].as_bool(), Some(true));
    assert_eq!(v["webhook"]["configured"].as_bool(), Some(false));

    let (st, put) = call(&r.app, "PUT", "/api/v1/servers/prod/alerts", Some(json!({"disk_min_gb": 20, "rules": {"ram": false}, "webhook_url": SECRET_URL}))).await;
    assert_eq!(st, 200, "{put}");
    let (_, got) = call(&r.app, "GET", "/api/v1/servers/prod/alerts", None).await;
    for text in [&put, &got] {
        assert!(!text.contains("abc123") && !text.contains("zzz") && !text.contains("secret"), "the address leaked: {text}");
        assert!(text.contains("https://hooks.example.test/***"), "{text}");
    }
    let v: Value = serde_json::from_str(&got).unwrap();
    assert_eq!((v["disk_min_gb"].as_f64(), v["rules"]["ram"].as_bool(), v["rules"]["disk"].as_bool(), v["webhook"]["configured"].as_bool()), (Some(20.0), Some(false), Some(true), Some(true)));
    // A later update without the address keeps it; an empty one removes it.
    let (st, v) = json_call(&r.app, "PUT", "/api/v1/servers/prod/alerts", Some(json!({"discord": false}))).await;
    assert_eq!((st, v["webhook"]["configured"].as_bool(), v["discord"].as_bool()), (200, Some(true), Some(false)));
    let (_, v) = json_call(&r.app, "PUT", "/api/v1/servers/prod/alerts", Some(json!({"webhook_url": ""}))).await;
    assert_eq!(v["webhook"]["configured"].as_bool(), Some(false));
    // On disk the file is owner-only.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(r.dir.path().join(".kora/alerts.json")).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0);
    }
}

#[tokio::test]
async fn alert_settings_errors() {
    let _t = TURN.lock().await;
    let r = rig(false);
    for body in [
        json!({"ram_max_percent": 500}),
        json!({"disk_min_percent": -1}),
        json!({"webhook_url": "not a url"}),
        json!({"webhook_url": "ftp://x/y"}),
        json!({"bogus": true}),
        json!({"rules": {"stopped": "yes"}}),
    ] {
        let (st, v) = json_call(&r.app, "PUT", "/api/v1/servers/prod/alerts", Some(body.clone())).await;
        assert_eq!((st, v["error"]["code"].as_str()), (400, Some("bad_request")), "{body}");
        assert!(!v["error"]["message"].as_str().unwrap().contains("not a url"));
    }
    let (st, v) = json_call(&r.app, "GET", "/api/v1/servers/nope/alerts", None).await;
    assert_eq!((st, v["error"]["code"].as_str()), (404, Some("not_found")));
    let (st, _) = json_call(&r.app, "PUT", "/api/v1/servers/nope/alerts", Some(json!({}))).await;
    assert_eq!(st, 404);
}

/// A remote admin sets the alert address: it must not point into this machine or its network.
#[tokio::test]
async fn remote_admins_cannot_aim_alerts_at_the_private_network() {
    use axum::extract::ConnectInfo;
    use kora_web::remote::{Remote, RemoteConfig, Role};
    let _t = TURN.lock().await;
    let r = rig(false);
    let mut cfg = RemoteConfig { enabled: true, ..Default::default() };
    let admin = cfg.add_user("adam", Role::Admin).unwrap();
    let f = r.fake.clone();
    let make: kora_web::BackendFactory = Arc::new(move || Box::new(f.clone()));
    let app = router(AppState::system(TOKEN.into(), ADDR.parse().unwrap()).with_backend(make).with_remote(Arc::new(Remote::new(None, cfg))));
    let remote_put = |url: &str| {
        let mut q = Request::builder()
            .method("PUT")
            .uri("/api/v1/servers/prod/alerts")
            .header(header::HOST, ADDR)
            .header(header::AUTHORIZATION, format!("Bearer {admin}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({"webhook_url": url}).to_string()))
            .unwrap();
        q.extensions_mut().insert(ConnectInfo("100.64.0.20:5000".parse::<std::net::SocketAddr>().unwrap()));
        let app = app.clone();
        async move {
            let r = app.oneshot(q).await.unwrap();
            let st = r.status().as_u16();
            let b = axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap();
            (st, String::from_utf8_lossy(&b).into_owned())
        }
    };
    for bad in [
        "http://127.0.0.1:47120/api/v1/servers",
        "http://10.0.0.5/hook",
        "http://169.254.169.254/latest/meta-data/",
        "http://100.100.100.100/",
        "http://[::1]:8080/",
        "http://0.0.0.0/",
        // Web address rules read `\` as `/`: this one goes to 127.0.0.1, not to hooks.example.com.
        "http://127.0.0.1:8080\\@hooks.example.com/",
        "http://[64:ff9b::7f00:1]/",
    ] {
        let (st, body) = remote_put(bad).await;
        assert_eq!(st, 400, "{bad}: {body}");
        assert!(body.contains("public internet address") || body.contains("backslash"), "{body}");
    }
    let saved = || kora_core::monitor::alerts::load_settings(r.dir.path());
    let (st, body) = remote_put("https://93.184.216.34/hook").await;
    assert_eq!(st, 200, "{body}");
    // Remembered, so every send looks the address up again and refuses inside addresses.
    assert!(saved().webhook_set_remotely);
    // On this computer a LAN address is the owner's choice.
    let (st, body) = call(&app, "PUT", "/api/v1/servers/prod/alerts", Some(json!({"webhook_url": "http://192.168.1.50/hook"}))).await;
    assert_eq!(st, 200, "{body}");
    assert!(!saved().webhook_set_remotely, "set here: sent as before");
    // Other remote changes still work while that address stays as it is.
    let mut q = req("PUT", "/api/v1/servers/prod/alerts", Some(json!({"enabled": true})));
    q.headers_mut().insert(header::AUTHORIZATION, format!("Bearer {admin}").parse().unwrap());
    q.extensions_mut().insert(ConnectInfo("100.64.0.20:5000".parse::<std::net::SocketAddr>().unwrap()));
    assert_eq!(app.clone().oneshot(q).await.unwrap().status(), 200);
    assert!(!saved().webhook_set_remotely, "the address did not change");
}
