//! The support bundle through the router: prepare, read a file, save exactly what was shown, download it, and the
//! limits (administrators only for the bundle; names that are not a saved bundle are refused).
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
    let f = fake.clone();
    router(AppState::system(TOKEN.into(), ADDR.parse().unwrap()).with_backend(Arc::new(move || Box::new(f.clone()))))
}

async fn raw(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (u16, Vec<u8>) {
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
    (status, axum::body::to_bytes(r.into_body(), 1 << 24).await.unwrap().to_vec())
}

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (u16, Value) {
    let (s, b) = raw(app, method, uri, body).await;
    (s, serde_json::from_slice(&b).unwrap_or(Value::Null))
}

#[tokio::test]
async fn prepare_read_save_and_download() {
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("KORA_HOME", home.path());
    let fake = FakeBackend::sample();
    let app = app(&fake);
    let (st, l) = call(&app, "POST", "/api/v1/support/bundle/preview", Some(json!({"server": "prod"}))).await;
    assert_eq!(st, 200, "{l}");
    let id = l["id"].as_str().unwrap().to_string();
    assert!(l["never_uploaded"].as_str().unwrap().contains("never uploads"));
    let names: Vec<&str> = l["files"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"servers/prod/server.cfg") && names.contains(&"README.txt"));
    assert_eq!(l["omitted"][0]["name"], "servers/prod/secrets.cfg");
    // the text of one file, as it will be saved: the licence key is hidden
    let (st, f) = call(&app, "POST", "/api/v1/support/bundle/file", Some(json!({"id": id, "name": "servers/prod/server.cfg"}))).await;
    assert_eq!(st, 200);
    assert!(f["text"].as_str().unwrap().contains("<hidden: licence key>"));
    let (st, _) = call(&app, "POST", "/api/v1/support/bundle/file", Some(json!({"id": id, "name": "../../etc/passwd"}))).await;
    assert_eq!(st, 422);
    let (st, _) = call(&app, "POST", "/api/v1/support/bundle/file", Some(json!({"id": "nope", "name": "about.txt"}))).await;
    assert_eq!(st, 422);
    // save, then download exactly that
    let (st, saved) = call(&app, "POST", "/api/v1/support/bundle", Some(json!({"id": id}))).await;
    assert_eq!(st, 200, "{saved}");
    let name = saved["file_name"].as_str().unwrap();
    assert!(name.starts_with("kora-support-") && name.ends_with(".zip"));
    assert!(std::path::Path::new(saved["path"].as_str().unwrap()).is_file());
    let (st, bytes) = raw(&app, "GET", &format!("/api/v1/support/bundle/{name}"), None).await;
    assert_eq!(st, 200);
    assert_eq!(bytes.len() as u64, saved["bytes"].as_u64().unwrap());
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    assert!(z.by_name("README.txt").is_ok());
    // only a bundle Kora saved can be downloaded
    for bad in ["..%2F..%2Fetc%2Fpasswd", "notes.zip", "kora-support-x.txt"] {
        let (st, _) = raw(&app, "GET", &format!("/api/v1/support/bundle/{bad}"), None).await;
        assert_eq!(st, 404, "{bad}");
    }
    // the short summary for a bug report
    let (st, s) = call(&app, "POST", "/api/v1/support/summary", Some(json!({}))).await;
    assert_eq!(st, 200);
    assert!(s["text"].as_str().unwrap().contains("Kora"));
}
