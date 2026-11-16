//! Paid assets: list, upload + inspect, import job, remove, Downloads folder. Driven through the
//! router with a fake backend; uploads land in a temp server folder.
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, Request, Response};
use kora_core::api::fake::{entry, FakeBackend};
use kora_core::installer::Mode;
use kora_core::paidassets::{AssetsView, EntitlementHint, HintKind, PaidAsset};
use kora_web::remote::{Remote, RemoteConfig, Role};
use kora_web::{router, AppState};
use serde_json::{json, Value};
use tower::ServiceExt;

const ADDR: &str = "127.0.0.1:47120";
const TOKEN: &str = "testtoken";

fn app(fake: &FakeBackend) -> axum::Router {
    let f = fake.clone();
    router(
        AppState::system(TOKEN.into(), ADDR.parse::<SocketAddr>().unwrap())
            .with_backend(Arc::new(move || Box::new(f.clone()))),
    )
}

/// The sample fake with `prod` living in a real temp folder (uploads go to `<dir>/.kora`).
fn fake_in(dir: &Path) -> FakeBackend {
    let f = FakeBackend::sample();
    let mut e = entry("prod", Mode::Production, 30120, 40120);
    e.dir = dir.to_path_buf();
    f.state().servers[0] = e;
    f
}

fn base(method: &str, uri: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(format!("/api/v1{uri}"))
        .header(header::HOST, ADDR)
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header("x-kora", "1")
}

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> Response<Body> {
    let mut b = base(method, uri);
    let body = match body {
        Some(v) => {
            b = b.header(header::CONTENT_TYPE, "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    app.clone().oneshot(b.body(body).unwrap()).await.unwrap()
}

async fn upload(app: &axum::Router, name: &str, bytes: Vec<u8>) -> Response<Body> {
    let r = base("POST", "/servers/prod/assets/inspect")
        .header(header::CONTENT_TYPE, "application/zip")
        .header("x-file-name", name)
        .body(Body::from(bytes))
        .unwrap();
    app.clone().oneshot(r).await.unwrap()
}

async fn json_of(r: Response<Body>) -> Value {
    let b = axum::body::to_bytes(r.into_body(), 1 << 22).await.unwrap();
    if b.is_empty() {
        return Value::Null;
    }
    serde_json::from_slice(&b).unwrap()
}

async fn job(app: &axum::Router, r: Response<Body>) -> Value {
    assert_eq!(r.status(), 202);
    let id = json_of(r).await["job"].as_str().unwrap().to_string();
    for _ in 0..200 {
        let j = json_of(call(app, "GET", &format!("/jobs/{id}"), None).await).await;
        if j["state"] != "running" {
            return j;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("job did not finish");
}

#[tokio::test]
async fn list_shows_assets_hints_and_the_portal_link() {
    let f = FakeBackend::sample();
    f.state().assets.insert(
        "prod".into(),
        AssetsView {
            assets: vec![PaidAsset {
                name: "qs-housing".into(),
                path: "[paid]/qs-housing".into(),
                version: Some("1.2".into()),
                imported_unix: Some(1_700_000_000),
                present: true,
                has_fxap: true,
                backups: 1,
            }],
            hints: vec![EntitlementHint {
                kind: HintKind::Entitlement,
                resource: Some("qs-housing".into()),
                line: "You lack the required entitlement to use qs-housing".into(),
                fix: "Your licence key ...".into(),
            }],
        },
    );
    let a = app(&f);
    let j = json_of(call(&a, "GET", "/servers/prod/assets", None).await).await;
    assert_eq!(
        j["portal_url"],
        "https://portal.cfx.re/assets/granted-assets"
    );
    assert_eq!(j["assets"][0]["name"], "qs-housing");
    assert_eq!(j["assets"][0]["version"], "1.2");
    assert_eq!(j["hints"][0]["kind"], "entitlement");
    assert_eq!(j["hints"][0]["resource"], "qs-housing");
    let empty = json_of(call(&a, "GET", "/servers/dev/assets", None).await).await;
    assert_eq!(empty["assets"], json!([]));
    assert_eq!(
        call(&a, "GET", "/servers/nope/assets", None).await.status(),
        404
    );
}

#[tokio::test]
async fn an_upload_is_streamed_to_a_file_inspected_then_imported() {
    let t = tempfile::tempdir().unwrap();
    let f = fake_in(t.path());
    let a = app(&f);
    let bytes: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
    let r = upload(&a, "qs%20housing.zip", bytes.clone()).await;
    assert_eq!(
        r.status(),
        200,
        "bigger than the 1 MiB JSON limit, still accepted"
    );
    let j = json_of(r).await;
    assert_eq!(j["plan"]["zip_name"], "qs housing.zip");
    assert_eq!(j["plan"]["can_import"], true);
    let id = j["upload"].as_str().unwrap().to_string();
    assert!(
        f.calls().contains(&format!(
            "asset_inspect prod qs housing.zip {}",
            bytes.len()
        )),
        "{:?}",
        f.calls()
    );
    let saved = t
        .path()
        .join(".kora/asset-uploads")
        .join(&id)
        .join("qs housing.zip");
    assert_eq!(
        std::fs::read(&saved).unwrap(),
        bytes,
        "the whole body reached the file"
    );

    let done = job(
        &a,
        call(
            &a,
            "POST",
            "/servers/prod/assets",
            Some(json!({"upload": id, "replace": false})),
        )
        .await,
    )
    .await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["result"]["imported"][0]["name"], "qs housing");
    assert!(f
        .calls()
        .contains(&"asset_import prod qs housing.zip replace=false".to_string()));
    assert!(
        !saved.exists(),
        "the upload is removed after a successful import"
    );
    let again = call(
        &a,
        "POST",
        "/servers/prod/assets",
        Some(json!({"upload": id})),
    )
    .await;
    assert_eq!(again.status(), 404);
}

#[tokio::test]
async fn a_refused_plan_does_not_keep_the_upload() {
    let t = tempfile::tempdir().unwrap();
    let f = fake_in(t.path());
    f.state().asset_plan = Some(Ok(kora_core::paidassets::ImportPlan {
        zip_name: "x.zip".into(),
        can_import: false,
        ..Default::default()
    }));
    let a = app(&f);
    let j = json_of(upload(&a, "x.zip", b"PK..".to_vec()).await).await;
    assert_eq!(j["upload"], Value::Null);
    let left = std::fs::read_dir(t.path().join(".kora/asset-uploads"))
        .unwrap()
        .count();
    assert_eq!(left, 0);
}

#[tokio::test]
async fn bad_requests_are_refused() {
    let t = tempfile::tempdir().unwrap();
    let f = fake_in(t.path());
    let a = app(&f);
    assert_eq!(
        upload(&a, "e.zip", vec![]).await.status(),
        400,
        "empty file"
    );
    assert_eq!(
        call(&a, "POST", "/servers/prod/assets/inspect", Some(json!({})))
            .await
            .status(),
        400
    );
    assert_eq!(
        call(&a, "POST", "/servers/prod/assets", Some(json!({})))
            .await
            .status(),
        400
    );
    assert_eq!(
        call(
            &a,
            "POST",
            "/servers/prod/assets",
            Some(json!({"upload": "../../etc"}))
        )
        .await
        .status(),
        400
    );
    assert_eq!(
        call(
            &a,
            "POST",
            "/servers/prod/assets",
            Some(json!({"upload": "ab".repeat(16)}))
        )
        .await
        .status(),
        404
    );
    let big = base("POST", "/servers/prod/assets/inspect")
        .header(header::CONTENT_TYPE, "application/zip")
        .header(header::CONTENT_LENGTH, (3u64 << 30).to_string())
        .body(Body::from("x"))
        .unwrap();
    assert_eq!(a.clone().oneshot(big).await.unwrap().status(), 413);
    assert!(!f.calls().iter().any(|c| c.starts_with("asset_import")));
}

#[tokio::test]
async fn remove_answers_notes_or_not_found() {
    let f = FakeBackend::sample();
    f.state().assets.insert(
        "prod".into(),
        AssetsView {
            assets: vec![PaidAsset {
                name: "qs-housing".into(),
                path: "[paid]/qs-housing".into(),
                version: None,
                imported_unix: None,
                present: true,
                has_fxap: true,
                backups: 0,
            }],
            hints: vec![],
        },
    );
    let a = app(&f);
    let r = call(&a, "DELETE", "/servers/prod/assets/qs-housing", None).await;
    assert_eq!(r.status(), 200);
    assert!(json_of(r).await["notes"][0]
        .as_str()
        .unwrap()
        .contains("removed"));
    assert_eq!(
        call(&a, "DELETE", "/servers/prod/assets/qs-housing", None)
            .await
            .status(),
        404
    );
    assert_eq!(
        call(&a, "DELETE", "/servers/prod/assets/%5Bpaid%5D", None)
            .await
            .status(),
        400
    );
}

/// The Downloads folder seam is global: one test does everything with it.
#[tokio::test]
async fn downloads_are_listed_and_can_be_inspected_and_imported_locally() {
    let t = tempfile::tempdir().unwrap();
    let dl = t.path().join("Downloads");
    std::fs::create_dir(&dl).unwrap();
    std::fs::write(dl.join("qs-housing.zip"), b"PK-zip").unwrap();
    std::fs::write(dl.join("notes.txt"), b"x").unwrap();
    kora_web::handlers::assets::set_downloads_dir(Some(dl.clone()));
    let f = fake_in(&t.path().join("srv"));
    let a = app(&f);
    let j = json_of(call(&a, "GET", "/assets/downloads", None).await).await;
    assert_eq!(j["found"], true);
    assert_eq!(j["zips"].as_array().unwrap().len(), 1);
    assert_eq!(j["zips"][0]["name"], "qs-housing.zip");
    let p = json_of(
        call(
            &a,
            "POST",
            "/servers/prod/assets/inspect",
            Some(json!({"download": "qs-housing.zip"})),
        )
        .await,
    )
    .await;
    assert_eq!(p["plan"]["zip_name"], "qs-housing.zip");
    assert_eq!(p["download"], "qs-housing.zip");
    for bad in ["../x.zip", "notes.txt", "missing.zip"] {
        let r = call(
            &a,
            "POST",
            "/servers/prod/assets/inspect",
            Some(json!({"download": bad})),
        )
        .await;
        assert_eq!(r.status(), 404, "{bad}");
    }
    let done = job(
        &a,
        call(
            &a,
            "POST",
            "/servers/prod/assets",
            Some(json!({"download": "qs-housing.zip"})),
        )
        .await,
    )
    .await;
    assert_eq!(done["state"], "done", "{done}");
    assert!(
        dl.join("qs-housing.zip").exists(),
        "the user's own download is never deleted"
    );

    // Over remote access, even an admin can't read this computer's Downloads folder.
    let mut cfg = RemoteConfig {
        enabled: true,
        ..Default::default()
    };
    let tok = cfg.add_user("adam", Role::Admin).unwrap();
    let f2 = f.clone();
    let remote = router(
        AppState::system("launch".into(), "100.64.0.5:47120".parse().unwrap())
            .with_backend(Arc::new(move || Box::new(f2.clone())))
            .with_remote(Arc::new(Remote::new(None, cfg))),
    );
    let rq = |method: &str, uri: &str, body: Option<Value>| {
        let mut b = Request::builder()
            .method(method)
            .uri(format!("/api/v1{uri}"))
            .header(header::HOST, "100.64.0.5:47120")
            .header(header::AUTHORIZATION, format!("Bearer {tok}"));
        let body = match body {
            Some(v) => {
                b = b.header(header::CONTENT_TYPE, "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        let mut r = b.body(body).unwrap();
        r.extensions_mut().insert(ConnectInfo(
            "100.64.0.20:51000".parse::<SocketAddr>().unwrap(),
        ));
        r
    };
    let r = remote
        .clone()
        .oneshot(rq("GET", "/assets/downloads", None))
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    let r = remote
        .clone()
        .oneshot(rq(
            "POST",
            "/servers/prod/assets/inspect",
            Some(json!({"download": "qs-housing.zip"})),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    let r = remote
        .clone()
        .oneshot(rq("GET", "/servers/prod/assets", None))
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "the list itself is fine remotely");

    kora_web::handlers::assets::set_downloads_dir(Some(t.path().join("none")));
    let j = json_of(call(&a, "GET", "/assets/downloads", None).await).await;
    assert_eq!(j["zips"], json!([]));
    kora_web::handlers::assets::set_downloads_dir(None);
}
