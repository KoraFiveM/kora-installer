//! The hub, end to end: a hub panel (driven through `router()`) talks over real HTTP on 127.0.0.1 to a
//! node panel (also `kora_web::router`, with a `FakeBackend`). Also version skew, a node that is down,
//! secrecy of the token and bad addresses.
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request, Response};
use kora_core::api::fake::FakeBackend;
use kora_core::hub::{HttpRequest, HttpResponse, NodeStore, Transport};
use kora_web::handlers::hub;
use kora_web::{router, AppState};
use serde_json::{json, Value};
use tower::ServiceExt;

const HUB_ADDR: &str = "127.0.0.1:47120";
const HUB_TOKEN: &str = "hubtoken";
const NODE_TOKEN: &str = "node-secret-token";

/// The seams are process-wide, so these tests run one at a time.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct Setup {
    _dir: tempfile::TempDir,
    store: NodeStore,
    app: axum::Router,
    _guard: tokio::sync::MutexGuard<'static, ()>,
}

async fn setup(local: &FakeBackend) -> Setup {
    let guard = SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hub.json");
    hub::set_store_path(Some(path.clone()));
    hub::set_transport(None);
    let f = local.clone();
    let app = router(
        AppState::system(HUB_TOKEN.into(), HUB_ADDR.parse::<SocketAddr>().unwrap())
            .with_backend(Arc::new(move || Box::new(f.clone()))),
    );
    Setup {
        _dir: dir,
        store: NodeStore::at(path),
        app,
        _guard: guard,
    }
}

/// A node panel on a free loopback port. Returns its base URL.
async fn start_node(fake: &FakeBackend) -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let f = fake.clone();
    let app = router(
        AppState::system(NODE_TOKEN.into(), addr)
            .with_backend(Arc::new(move || Box::new(f.clone()))),
    );
    tokio::spawn(async move {
        axum::serve(l, app).await.unwrap();
    });
    format!("http://{addr}")
}

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> Response<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(format!("/api/v1{uri}"))
        .header(header::HOST, HUB_ADDR)
        .header(header::AUTHORIZATION, format!("Bearer {HUB_TOKEN}"));
    let body = match body {
        Some(v) => {
            b = b.header(header::CONTENT_TYPE, "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    app.clone().oneshot(b.body(body).unwrap()).await.unwrap()
}

async fn text_of(r: Response<Body>) -> String {
    String::from_utf8(
        axum::body::to_bytes(r.into_body(), 1 << 22)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap()
}
async fn json_of(r: Response<Body>) -> Value {
    let t = text_of(r).await;
    if t.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&t).unwrap()
    }
}

async fn wait_job(app: &axum::Router, r: Response<Body>) -> Value {
    assert_eq!(r.status(), 202);
    let id = json_of(r).await["job"].as_str().unwrap().to_string();
    for _ in 0..400 {
        let j = json_of(call(app, "GET", &format!("/jobs/{id}"), None).await).await;
        if j["state"] != "running" {
            return j;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("job did not finish");
}

async fn add(app: &axum::Router, name: &str, url: &str, token: &str) -> Response<Body> {
    call(
        app,
        "POST",
        "/nodes",
        Some(json!({"name": name, "url": url, "token": token})),
    )
    .await
}

#[tokio::test]
async fn hub_and_node_end_to_end() {
    let local = FakeBackend::new();
    local.state().servers.push(kora_core::api::fake::entry(
        "hub-local",
        kora_core::installer::Mode::Production,
        30120,
        40120,
    ));
    let s = setup(&local).await;
    let node = FakeBackend::sample();
    let url = start_node(&node).await;

    let r = add(&s.app, "Basement", &url, NODE_TOKEN).await;
    assert_eq!(r.status(), 201);
    let body = text_of(r).await;
    assert!(
        !body.contains(NODE_TOKEN),
        "the add answer must not echo the token: {body}"
    );
    let j: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(j["node"]["token_set"], true);
    assert_eq!(j["node"]["server_count"], 2);
    let id = j["node"]["id"].as_str().unwrap().to_string();

    // The token is stored, 0600 on Unix, and never returned by any hub route.
    assert_eq!(s.store.load().unwrap()[0].token, NODE_TOKEN);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(s.store.path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let list = text_of(call(&s.app, "GET", "/nodes", None).await).await;
    assert!(!list.contains(NODE_TOKEN));
    let list: Value = serde_json::from_str(&list).unwrap();
    let n = &list["nodes"][0];
    assert_eq!(
        (
            n["reachable"].clone(),
            n["server_count"].clone(),
            n["api"].clone()
        ),
        (json!(true), json!(2), json!(1))
    );
    assert_eq!(n["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(n["can_control"], true);
    assert_eq!(n["insecure"], false);
    assert!(n["last_error"].is_null());

    // Node servers carry the actions the node supports.
    let ns = json_of(call(&s.app, "GET", &format!("/nodes/{id}/servers"), None).await).await;
    let names: Vec<_> = ns["servers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, ["prod", "dev"]);
    assert_eq!(
        ns["servers"][0]["actions"],
        json!(["start", "stop", "restart"])
    );

    // The fleet: local first, then the node's, each tagged.
    let f = json_of(call(&s.app, "GET", "/fleet", None).await).await;
    assert_eq!(f["partial"], false);
    let rows: Vec<(String, String)> = f["servers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| {
            (
                x["node_name"].as_str().unwrap().into(),
                x["name"].as_str().unwrap().into(),
            )
        })
        .collect();
    assert_eq!(rows[0], ("This panel".into(), "hub-local".into()));
    assert!(rows.contains(&("Basement".into(), "prod".into())));
    assert!(f["servers"][0]["node_id"].is_null());

    // Control a node's server: the hub job completes with the node's own result.
    let r = call(
        &s.app,
        "POST",
        &format!("/nodes/{id}/servers/dev/control"),
        Some(json!({"action": "start"})),
    )
    .await;
    let j = wait_job(&s.app, r).await;
    assert_eq!(j["state"], "done", "{j}");
    assert_eq!(j["result"]["status"], "running");
    assert!(
        node.calls().contains(&"Start dev".to_string()),
        "{:?}",
        node.calls()
    );
    assert!(local.calls().iter().all(|c| !c.contains("Start")));
    assert!(j["events"].as_array().unwrap().iter().any(|e| e["message"]
        .as_str()
        .is_some_and(|m| m.contains("Basement"))));

    // Errors from the node come back as ours.
    let r = call(
        &s.app,
        "POST",
        &format!("/nodes/{id}/servers/nope/control"),
        Some(json!({"action": "start"})),
    )
    .await;
    let j = wait_job(&s.app, r).await;
    assert_eq!(j["state"], "failed");
    assert!(
        j["error"]["message"].as_str().unwrap().contains("nope"),
        "{j}"
    );
    let r = call(
        &s.app,
        "POST",
        &format!("/nodes/{id}/servers/dev/control"),
        Some(json!({"action": "explode"})),
    )
    .await;
    assert_eq!(r.status(), 400);

    // Remove.
    assert_eq!(
        call(&s.app, "DELETE", &format!("/nodes/{id}"), None)
            .await
            .status(),
        204
    );
    assert_eq!(
        call(&s.app, "DELETE", &format!("/nodes/{id}"), None)
            .await
            .status(),
        404
    );
    assert_eq!(
        call(&s.app, "GET", &format!("/nodes/{id}/servers"), None)
            .await
            .status(),
        404
    );
    assert_eq!(
        call(
            &s.app,
            "POST",
            &format!("/nodes/{id}/servers/dev/control"),
            Some(json!({"action": "stop"}))
        )
        .await
        .status(),
        404
    );
    assert!(s.store.load().unwrap().is_empty());
}

#[tokio::test]
async fn a_wrong_token_or_a_non_kora_address_is_not_saved() {
    let s = setup(&FakeBackend::new()).await;
    let url = start_node(&FakeBackend::sample()).await;
    let r = add(&s.app, "N", &url, "wrong-token").await;
    assert_eq!(r.status(), 502);
    let j = json_of(r).await;
    assert_eq!(j["error"]["code"], "node_unauthorized");
    // (Never a 401: the UI would think this panel signed us out.) Port 1 is closed:
    let r = add(&s.app, "N", "http://127.0.0.1:1", "t").await;
    assert_eq!(r.status(), 502);
    assert_eq!(json_of(r).await["error"]["code"], "node_unreachable");
    assert!(s.store.load().unwrap().is_empty());
}

#[tokio::test]
async fn bad_input_is_rejected_before_any_request() {
    let s = setup(&FakeBackend::new()).await;
    struct Boom;
    impl Transport for Boom {
        fn send(&self, _: &HttpRequest) -> Result<HttpResponse, String> {
            panic!("no request may be made for bad input");
        }
    }
    hub::set_transport(Some(Arc::new(Boom)));
    for (name, url, token) in [
        ("N", "ftp://host", "t"),
        ("N", "host:47120", "t"),
        ("N", "http://user:pw@host", "t"),
        ("N", "javascript:alert(1)", "t"),
        ("", "http://h:1", "t"),
        ("N", "http://h:1", ""),
        ("N", "http://h:1", "has space"),
    ] {
        let r = add(&s.app, name, url, token).await;
        assert_eq!(r.status(), 400, "{name} {url} {token}");
        assert_eq!(json_of(r).await["error"]["code"], "bad_request");
    }
    hub::set_transport(None);
}

/// A transport that stands in for a node of any version.
struct Script {
    ops: Vec<&'static str>,
    calls: Mutex<Vec<(String, String)>>,
}
impl Transport for Script {
    fn send(&self, r: &HttpRequest) -> Result<HttpResponse, String> {
        self.calls
            .lock()
            .unwrap()
            .push((r.method.to_string(), r.url.clone()));
        let ok = |b: Value| {
            Ok(HttpResponse {
                status: 200,
                body: b,
            })
        };
        if r.url.ends_with("/api/v1/meta") {
            return ok(
                json!({"api": 1, "version": "0.3.1", "platform": "linux", "node": "old", "ops": self.ops, "unknown_future_field": {"x": 1}}),
            );
        }
        if r.url.ends_with("/api/v1/servers") {
            return ok(json!({"servers": [{"name": "old-one", "status": "running", "extra": 1}]}));
        }
        Ok(HttpResponse {
            status: 404,
            body: json!({"error": {"code": "not_found", "message": "unknown API route"}}),
        })
    }
}

#[tokio::test]
async fn an_older_node_is_only_offered_what_it_supports() {
    let s = setup(&FakeBackend::new()).await;
    let t = Arc::new(Script {
        ops: vec!["meta", "servers_list", "server_get"],
        calls: Mutex::default(),
    });
    hub::set_transport(Some(t.clone()));
    // 203.0.113.0/24 is a documentation range: plain http to it is flagged.
    let r = add(&s.app, "Old", "http://203.0.113.9:47120", "tok").await;
    assert_eq!(r.status(), 201);
    let j = json_of(r).await;
    assert_eq!(j["node"]["insecure"], true);
    assert!(j["node"]["warning"].as_str().unwrap().contains("https"));
    assert_eq!(j["node"]["can_control"], false);
    let id = j["node"]["id"].as_str().unwrap().to_string();

    let f = json_of(call(&s.app, "GET", "/fleet", None).await).await;
    let old = f["servers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == "old-one")
        .unwrap();
    assert_eq!(old["actions"], json!([]));

    let r = call(
        &s.app,
        "POST",
        &format!("/nodes/{id}/servers/old-one/control"),
        Some(json!({"action": "stop"})),
    )
    .await;
    assert_eq!(r.status(), 422);
    let j = json_of(r).await;
    assert_eq!(j["error"]["code"], "node_unsupported");
    assert!(j["error"]["message"].as_str().unwrap().contains("0.3.1"));
    assert!(
        !t.calls.lock().unwrap().iter().any(|(m, _)| m == "POST"),
        "nothing may be sent for an unsupported action"
    );
    hub::set_transport(None);
}

#[tokio::test]
async fn a_node_that_is_down_gives_a_partial_fleet() {
    let local = FakeBackend::sample();
    let s = setup(&local).await;
    let url = start_node(&FakeBackend::sample()).await;
    assert_eq!(add(&s.app, "Up", &url, NODE_TOKEN).await.status(), 201);
    // A second node that nothing listens on (saved directly, as if it went down later).
    s.store.add("Down", "http://127.0.0.1:1", "x").unwrap();

    let f = json_of(call(&s.app, "GET", "/fleet", None).await).await;
    assert_eq!(f["partial"], true);
    let nodes = f["nodes"].as_array().unwrap();
    let up = nodes.iter().find(|n| n["name"] == "Up").unwrap();
    let down = nodes.iter().find(|n| n["name"] == "Down").unwrap();
    assert_eq!(up["reachable"], true);
    assert_eq!(down["reachable"], false);
    assert!(down["last_error"].as_str().unwrap().len() > 5);
    assert!(down["server_count"].is_null());
    let from = |who: &str| {
        f["servers"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|x| x["node_name"] == who)
            .count()
    };
    assert_eq!((from("This panel"), from("Up"), from("Down")), (2, 2, 0));

    // Listing nodes still works, and listing the dead node's servers says why.
    let down_id = down["id"].as_str().unwrap();
    let r = call(&s.app, "GET", &format!("/nodes/{down_id}/servers"), None).await;
    assert_eq!(r.status(), 502);
    assert_eq!(json_of(r).await["error"]["code"], "node_unreachable");
}
