//! Bringing a server in from a backup, through the router with a fake backend: the upload (streamed to disk, name
//! made safe, size limit), looking at the file, and starting the import as a job. The engine itself is tested in
//! `kora_core::backup::import`.
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

async fn send(app: &axum::Router, method: &str, uri: &str, headers: &[(&str, String)], body: Body) -> (u16, Value) {
    let mut b = Request::builder().method(method).uri(uri).header(header::HOST, ADDR).header(header::AUTHORIZATION, format!("Bearer {TOKEN}"));
    for (k, v) in headers {
        b = b.header(*k, v.as_str());
    }
    let r = app.clone().oneshot(b.body(body).unwrap()).await.unwrap();
    let st = r.status().as_u16();
    let bytes = axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap();
    (st, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn upload_then_look_then_import() {
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("KORA_HOME", home.path());
    let fake = FakeBackend::sample();
    let app = app(&fake);
    // the file is the body; a hostile name is made safe
    let data = vec![7u8; 300_000];
    let (st, up) = send(
        &app,
        "POST",
        "/api/v1/backups/import/upload",
        &[("x-file-name", "..%2F..%2Fetc%2Fold-server.tar.gz.age".into()), ("content-type", "application/octet-stream".into()), ("x-kora", "1".into())],
        Body::from(data.clone()),
    )
    .await;
    assert_eq!(st, 200, "{up}");
    assert_eq!(up["file_name"], "old-server.tar.gz.age");
    assert_eq!(up["bytes"], 300_000);
    let saved = home.path().join("imports").join(up["upload"].as_str().unwrap()).join("old-server.tar.gz.age");
    assert_eq!(std::fs::read(&saved).unwrap(), data);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&saved).unwrap().permissions().mode() & 0o777, 0o600, "the uploaded file is private");
    }
    // a name that does not look like a backup falls back to a plain one
    let (_, up2) = send(&app, "POST", "/api/v1/backups/import/upload", &[("x-file-name", "notes.txt".into()), ("x-kora", "1".into())], Body::from("x")).await;
    assert_eq!(up2["file_name"], "backup.tar.gz.age");
    // a file declared too big is refused before it is read
    let (st, _) = send(&app, "POST", "/api/v1/backups/import/upload", &[("content-length", (30u64 << 30).to_string()), ("x-kora", "1".into())], Body::from("x")).await;
    assert_eq!(st, 413);
    // look at it
    let (st, p) = send(
        &app,
        "POST",
        "/api/v1/backups/import/inspect",
        &[("content-type", "application/json".into()), ("x-kora", "1".into())],
        Body::from(json!({"source": {"upload": up["upload"]}, "passphrase": "x"}).to_string()),
    )
    .await;
    assert_eq!(st, 200, "{p}");
    assert_eq!(p["suggested"]["game_port"], 30121);
    assert!(p["licence_note"].as_str().unwrap().contains("Cfx.re"));
    // import it: a job
    let req = json!({"source": {"upload": up["upload"]}, "passphrase": "x", "choices": {"name": "Old server", "dir": "/home/me/kora/old", "game_port": 30121, "txadmin_port": 40121}, "start": false, "start_over": false});
    let (st, j) = send(&app, "POST", "/api/v1/backups/import", &[("content-type", "application/json".into()), ("x-kora", "1".into())], Body::from(req.to_string())).await;
    assert_eq!(st, 202, "{j}");
    assert!(j["job"].is_string());
    for _ in 0..50 {
        if fake.calls().iter().any(|c| c.starts_with("import_run Old server")) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(fake.calls().iter().any(|c| c.starts_with("import_run Old server dir=/home/me/kora/old local=true")), "{:?}", fake.calls());
}
