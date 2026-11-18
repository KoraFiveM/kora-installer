//! Servers, console and system handlers, driven through the router with a fake backend.
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request, Response};
use kora_core::api::fake::{entry, FakeBackend};
use kora_core::api::Status;
use kora_core::installer::Mode;
use kora_web::{router, AppState};
use serde_json::{json, Value};
use tower::ServiceExt;

const ADDR: &str = "127.0.0.1:47120";
const TOKEN: &str = "testtoken";

fn app(fake: &FakeBackend) -> axum::Router {
    let f = fake.clone();
    kora_core::installer::registry::use_test_home(); // tests never touch the real Kora folder
                                                     // The panel without the administrator service: a machine that has the real one installed must not be asked.
    kora_web::elevated::set_broker_override(Some(Arc::new(kora_web::elevated::AbsentService)));
    router(AppState::system(TOKEN.into(), ADDR.parse::<SocketAddr>().unwrap()).with_backend(Arc::new(move || Box::new(f.clone()))))
}

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> Response<Body> {
    let mut b = Request::builder().method(method).uri(format!("/api/v1{uri}")).header(header::HOST, ADDR).header(header::AUTHORIZATION, format!("Bearer {TOKEN}")).header("x-kora", "1");
    let body = match body {
        Some(v) => {
            b = b.header(header::CONTENT_TYPE, "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    app.clone().oneshot(b.body(body).unwrap()).await.unwrap()
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

fn managed_server(fake: &FakeBackend, name: &str, mode: Mode, dir: &Path) {
    std::fs::create_dir_all(dir.join(".kora")).unwrap();
    std::fs::write(dir.join(".kora/request.json"), "{}").unwrap();
    std::fs::write(dir.join("server.cfg"), "ensure chat\n").unwrap();
    let mut e = entry(name, mode, 30150, 40150);
    e.dir = dir.to_path_buf();
    fake.state().servers.push(e);
}

#[tokio::test]
async fn system_info_has_the_fields_the_ui_reads() {
    let a = app(&FakeBackend::sample());
    let r = call(&a, "GET", "/system", None).await;
    assert_eq!(r.status(), 200);
    let j = json_of(r).await;
    assert_eq!(j["platform"], if cfg!(windows) { "windows" } else { "linux" });
    assert_eq!(j["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(j["admin_service"]["state"], "missing");
    assert!(j["elevated"].is_boolean() && j["admin"] == j["elevated"]);
    assert!(j["headless"].is_boolean());
    assert!(j["kora_home"].is_string());
}

#[tokio::test]
async fn list_and_get() {
    let f = FakeBackend::sample();
    let a = app(&f);
    let j = json_of(call(&a, "GET", "/servers", None).await).await;
    let s = j["servers"].as_array().unwrap();
    assert_eq!(s.len(), 2);
    assert_eq!(s[0]["name"], "prod");
    assert_eq!(s[0]["status"], "running");
    assert_eq!(s[0]["mode"], "production");
    assert_eq!(s[0]["ports"], json!({"game": 30120, "txadmin": 40120}));
    assert_eq!(s[0]["badge"], json!({"kind": "up_to_date"}));
    assert_eq!(s[0]["managed"], false);
    assert_eq!(s[1]["badge"], Value::Null);
    let g = json_of(call(&a, "GET", "/servers/prod", None).await).await;
    assert_eq!(g["running"], true);
    assert_eq!(g["console_enabled"], false);
    assert_eq!(g["status_detail"]["txadmin_up"], true);
    let r = call(&a, "GET", "/servers/nope", None).await;
    assert_eq!(r.status(), 404);
    f.state().list_error = Some("broken".into());
    assert_eq!(call(&a, "GET", "/servers", None).await.status(), 500);
}

#[tokio::test]
async fn control_runs_a_job_and_reports_the_status() {
    let f = FakeBackend::sample();
    let a = app(&f);
    let r = call(&a, "POST", "/servers/dev/control", Some(json!({"action": "start"}))).await;
    let j = job(&a, r).await;
    assert_eq!(j["state"], "done");
    assert_eq!(j["result"], json!({"status": "running"}));
    assert!(f.calls().contains(&"Start dev".to_string()));
    let r = call(&a, "POST", "/servers/prod/control", Some(json!({"action": "stop"}))).await;
    assert_eq!(job(&a, r).await["result"], json!({"status": "stopped"}));
    assert_eq!(call(&a, "POST", "/servers/nope/control", Some(json!({"action": "start"}))).await.status(), 404);
    assert_eq!(call(&a, "POST", "/servers/dev/control", Some(json!({"action": "explode"}))).await.status(), 400);
    f.state().control_error = Some("Port 30130 is in use.".into());
    let r = call(&a, "POST", "/servers/dev/control", Some(json!({"action": "restart"}))).await;
    let j = job(&a, r).await;
    assert_eq!(j["state"], "failed");
    // The job record carries the message as a plain string (the registry's format).
    let msg = j["error"].as_str().or_else(|| j["error"]["message"].as_str()).unwrap();
    assert!(msg.contains("Port 30130"), "{j}");
    f.state().status.insert("dev".into(), Status::Missing);
    assert_eq!(call(&a, "POST", "/servers/dev/control", Some(json!({"action": "start"}))).await.status(), 409);
}

#[tokio::test]
async fn delete_checks_name_managed_and_running() {
    let d = tempfile::tempdir().unwrap();
    let f = FakeBackend::sample();
    managed_server(&f, "mine", Mode::Development, &d.path().join("mine"));
    let a = app(&f);
    let body = |n: &str| Some(json!({"confirm_name": n, "keep_files": true, "keep_db": false}));
    assert_eq!(call(&a, "DELETE", "/servers/mine", body("wrong")).await.status(), 400);
    // Adopted (no install record): only removal from the list.
    assert_eq!(call(&a, "DELETE", "/servers/dev", body("dev")).await.status(), 400);
    f.state().status.insert("mine".into(), Status::Running);
    assert_eq!(call(&a, "DELETE", "/servers/mine", body("mine")).await.status(), 409);
    f.state().status.insert("mine".into(), Status::Stopped);
    assert_eq!(call(&a, "DELETE", "/servers/nope", body("nope")).await.status(), 404);
    let r = call(&a, "DELETE", "/servers/mine", body("mine")).await;
    let j = job(&a, r).await;
    assert_eq!(j["state"], "done");
    assert_eq!(j["result"], json!({"notes": ["Deleted."]}));
    assert!(f.calls().contains(&"delete mine keep_files=true keep_db=false".to_string()));
}

#[tokio::test]
async fn a_failed_database_drop_asks_for_the_admin_password_and_a_retry_with_it_works() {
    let d = tempfile::tempdir().unwrap();
    let f = FakeBackend::sample();
    managed_server(&f, "mine", Mode::Production, &d.path().join("mine"));
    let a = app(&f);
    f.state().delete_error = Some(
        kora_core::problem::Problem::new(
            kora_core::problem::codes::DB_NEEDS_PASSWORD,
            "Everything else was removed, but the database 'kora_1' / user 'kora_1' could not be deleted: Kora needs the database administrator password. Enter it and try again, or keep the database and delete the server without it.",
        )
        .with_details("command failed: database client exited with status 1\nOn the command line: set KORA_DB_ADMIN_PASSWORD to the admin password, then run `kora delete /x` again."),
    );
    let first = json!({"confirm_name": "mine", "keep_files": false, "keep_db": false});
    let r = call(&a, "DELETE", "/servers/mine", Some(first)).await;
    let j = job(&a, r).await;
    assert_eq!(j["state"], "failed");
    assert_eq!(j["error"]["code"], "needs_db_password", "{j}");
    let msg = j["error"]["message"].as_str().unwrap();
    assert!(msg.contains("database 'kora_1'") && msg.contains("administrator password"), "{msg}");
    assert!(!msg.contains("KORA_DB_ADMIN_PASSWORD") && !msg.contains("kora delete"), "panel wording, not command-line wording: {msg}");
    // Retry with the password: it is used, and never echoed back or recorded.
    let retry = json!({"confirm_name": "mine", "keep_files": false, "keep_db": false, "db_admin_password": "s3cret-pw"});
    let r = call(&a, "DELETE", "/servers/mine", Some(retry)).await;
    let j = job(&a, r).await;
    assert_eq!(j["state"], "done", "{j}");
    assert!(!j.to_string().contains("s3cret-pw"));
    let calls = f.calls();
    assert!(calls.contains(&"delete mine keep_files=false keep_db=false db_password=set".to_string()), "{calls:?}");
    assert!(!calls.iter().any(|c| c.contains("s3cret-pw")));
}

#[tokio::test]
async fn unregister_only_for_adopted_servers() {
    let d = tempfile::tempdir().unwrap();
    let f = FakeBackend::sample();
    managed_server(&f, "mine", Mode::Production, d.path());
    let a = app(&f);
    assert_eq!(call(&a, "DELETE", "/servers/mine/registration", None).await.status(), 400);
    assert_eq!(call(&a, "DELETE", "/servers/dev/registration", None).await.status(), 204);
    assert_eq!(call(&a, "DELETE", "/servers/dev/registration", None).await.status(), 404);
    assert!(f.calls().contains(&"unregister dev".to_string()));
}

#[tokio::test]
async fn scan_and_adopt() {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join("games/found/.kora")).unwrap();
    std::fs::create_dir_all(d.path().join("games/plain")).unwrap();
    let f = FakeBackend::sample();
    f.state().roots = vec![d.path().to_path_buf()];
    let a = app(&f);
    let r = call(&a, "POST", "/servers/scan", None).await;
    let j = job(&a, r).await;
    assert_eq!(j["state"], "done");
    let c = j["result"]["candidates"].as_array().unwrap();
    assert_eq!(c.len(), 1);
    assert_eq!(c[0]["name"], "found");
    assert_eq!(c[0]["mode"], "production");

    let dir = d.path().join("games/found");
    let r = call(&a, "POST", "/servers/adopt", Some(json!({"dir": dir}))).await;
    assert_eq!(r.status(), 200);
    let s = json_of(r).await;
    assert_eq!((s["name"].as_str().unwrap(), s["managed"].clone()), ("found", json!(false)));
    assert_eq!(f.state().servers.len(), 3);
    // Again: already registered.
    let r = call(&a, "POST", "/servers/adopt", Some(json!({"dir": dir}))).await;
    assert_eq!(r.status(), 400);
    assert!(json_of(r).await["error"]["message"].as_str().unwrap().contains("already"));
    // Not a server, not a folder, not absolute.
    let plain = d.path().join("games/plain");
    assert_eq!(call(&a, "POST", "/servers/adopt", Some(json!({"dir": plain}))).await.status(), 400);
    assert_eq!(call(&a, "POST", "/servers/adopt", Some(json!({"dir": "/no/such/dir"}))).await.status(), 400);
    assert_eq!(call(&a, "POST", "/servers/adopt", Some(json!({"dir": "relative"}))).await.status(), 400);
}

#[tokio::test]
async fn servers_carry_the_address_players_on_the_network_use() {
    let f = FakeBackend::sample();
    let a = app(&f);
    let list = json_of(call(&a, "GET", "/servers", None).await).await;
    for sv in list["servers"].as_array().unwrap() {
        let port = sv["ports"]["game"].as_u64().unwrap();
        match sv["connect"].as_str() {
            // Network address and the server's own game port, never localhost or the txAdmin port.
            Some(c) => {
                assert!(c.ends_with(&format!(":{port}")), "{c}");
                assert!(!c.starts_with("127.") && !c.starts_with("localhost"), "{c}");
            }
            None => assert!(sv["connect"].is_null()),
        }
    }
    let one = json_of(call(&a, "GET", "/servers/prod", None).await).await;
    assert!(one.get("connect").is_some(), "{one}");
}

#[tokio::test]
async fn open_folder_uses_the_servers_own_folder_and_refuses_when_headless() {
    use kora_web::handlers::{servers::set_opener, system::seam::set_headless};
    let d = tempfile::tempdir().unwrap();
    let f = FakeBackend::sample();
    managed_server(&f, "mine", Mode::Production, d.path());
    let a = app(&f);
    let opened: Arc<Mutex<Vec<std::path::PathBuf>>> = Arc::default();
    let o = opened.clone();
    set_opener(Some(Box::new(move |p| {
        o.lock().unwrap().push(p.to_path_buf());
        Ok(())
    })));
    set_headless(Some(true));
    let r = call(&a, "POST", "/servers/mine/open-folder", None).await;
    assert_eq!(r.status(), 400);
    // The refusal gives the path, so the person can copy it and go there themselves.
    let msg = json_of(r).await["error"]["message"].as_str().unwrap().to_string();
    assert!(msg.contains(&d.path().display().to_string()), "{msg}");
    assert!(opened.lock().unwrap().is_empty());
    set_headless(Some(false));
    assert_eq!(call(&a, "POST", "/servers/mine/open-folder", Some(json!({"dir": "/etc"}))).await.status(), 204);
    assert_eq!(*opened.lock().unwrap(), vec![d.path().to_path_buf()]);
    assert_eq!(call(&a, "POST", "/servers/nope/open-folder", None).await.status(), 404);
    // "prod" is registered with a folder that does not exist.
    assert_eq!(call(&a, "POST", "/servers/prod/open-folder", None).await.status(), 409);
    set_headless(None);
    set_opener(None);
}

#[tokio::test]
async fn logs_list_and_tail_only_known_files() {
    let d = tempfile::tempdir().unwrap();
    let f = FakeBackend::sample();
    managed_server(&f, "mine", Mode::Production, d.path());
    let body: String = (0..10).map(|i| format!("\u{1b}[31mline {i}\u{1b}[0m\n")).collect();
    std::fs::write(d.path().join(".kora/server.log"), body).unwrap();
    std::fs::write(d.path().join("secret.txt"), "top secret\n").unwrap();
    let a = app(&f);
    let j = json_of(call(&a, "GET", "/servers/mine/logs", None).await).await;
    let files = j["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["name"], "server.log");
    assert!(files[0]["size"].as_u64().unwrap() > 0);
    let j = json_of(call(&a, "GET", "/servers/mine/logs/server.log?lines=3", None).await).await;
    assert_eq!(j["file"], "server.log");
    // Colour codes are kept for the panel to show; the text itself is unchanged.
    assert_eq!(j["lines"], json!(["\u{1b}[31mline 7\u{1b}[0m", "\u{1b}[31mline 8\u{1b}[0m", "\u{1b}[31mline 9\u{1b}[0m"]));
    for bad in ["secret.txt", "..%2Fsecret.txt", "%2Fetc%2Fpasswd", "..%5Csecret.txt", "update.log"] {
        let r = call(&a, "GET", &format!("/servers/mine/logs/{bad}"), None).await;
        assert!(matches!(r.status().as_u16(), 400 | 404), "{bad}: {}", r.status());
    }
    assert_eq!(call(&a, "GET", "/servers/nope/logs", None).await.status(), 404);
}

#[tokio::test]
async fn console_enable_then_send() {
    let d = tempfile::tempdir().unwrap();
    let f = FakeBackend::sample();
    managed_server(&f, "mine", Mode::Production, d.path());
    let a = app(&f);
    let send = |c: &str| call(&a, "POST", "/servers/mine/console", Some(json!({"command": c})));
    // Validation first.
    assert_eq!(send("").await.status(), 400);
    assert_eq!(send("say a\nsay b").await.status(), 400);
    assert_eq!(send(&"x".repeat(1001)).await.status(), 400);
    // Not running.
    let r = send("status").await;
    assert_eq!(r.status(), 409);
    assert!(json_of(r).await["error"]["message"].as_str().unwrap().contains("not running"));
    f.state().status.insert("mine".into(), Status::Running);
    // Running, but no helper yet.
    let r = send("status").await;
    assert_eq!(r.status(), 409);
    assert!(json_of(r).await["error"]["message"].as_str().unwrap().contains("live console"));
    assert_eq!(call(&a, "PUT", "/servers/mine/console/enabled", Some(json!({"enabled": true}))).await.status(), 204);
    assert_eq!(call(&a, "PUT", "/servers/mine/console/enabled", Some(json!({"enabled": true}))).await.status(), 204);
    let g = json_of(call(&a, "GET", "/servers/mine", None).await).await;
    assert_eq!(g["console_enabled"], true);
    assert_eq!(send("  say hi ").await.status(), 204);
    assert_eq!(std::fs::read_to_string(d.path().join(".kora/console.in")).unwrap(), "say hi\n");
    assert_eq!(call(&a, "PUT", "/servers/nope/console/enabled", None).await.status(), 404);
}

async fn next_chunk(body: &mut axum::body::BodyDataStream) -> String {
    use tokio_stream::StreamExt;
    let c = tokio::time::timeout(Duration::from_secs(5), body.next()).await.expect("timed out").unwrap().unwrap();
    String::from_utf8_lossy(&c).into_owned()
}

#[tokio::test]
async fn resource_control_queues_a_fixed_command() {
    let d = tempfile::tempdir().unwrap();
    let f = FakeBackend::sample();
    managed_server(&f, "mine", Mode::Production, d.path());
    f.state().resources = vec!["kora_core".into(), "kora_console".into()];
    let a = app(&f);
    let ctl = |n: &str, action: &str| {
        let path = format!("/servers/mine/resources/{n}/control");
        let body = json!({"action": action});
        let a = &a;
        async move { call(a, "POST", &path, Some(body)).await }
    };
    // Validation first.
    assert_eq!(ctl("kora_core", "explode").await.status(), 400);
    assert_eq!(ctl("kora_console", "stop").await.status(), 400);
    assert_eq!(ctl("not-installed-here", "start").await.status(), 404);
    assert_eq!(ctl("kora_core", "start").await.status(), 409, "the server is stopped");
    f.state().status.insert("mine".into(), Status::Running);
    let r = ctl("kora_core", "start").await;
    assert_eq!(r.status(), 409);
    assert!(json_of(r).await["error"]["message"].as_str().unwrap().contains("live console"));
    assert_eq!(call(&a, "PUT", "/servers/mine/console/enabled", Some(json!({"enabled": true}))).await.status(), 204);
    for action in ["start", "stop", "restart"] {
        let r = ctl("kora_core", action).await;
        assert_eq!(r.status(), 200, "{action}");
        assert!(json_of(r).await["message"].as_str().unwrap().contains(action));
    }
    // Restart is `ensure`: it also starts a resource that was not running.
    assert_eq!(std::fs::read_to_string(d.path().join(".kora/console.in")).unwrap(), "start kora_core\nstop kora_core\nensure kora_core\n");
    assert_eq!(ctl("kora_console", "restart").await.status(), 200);
    assert_eq!(call(&a, "POST", "/servers/nope/resources/x/control", Some(json!({"action": "start"}))).await.status(), 404);
}

#[tokio::test]
async fn resource_control_sends_the_folder_name_fxserver_knows() {
    let d = tempfile::tempdir().unwrap();
    let f = FakeBackend::sample();
    managed_server(&f, "mine", Mode::Production, d.path());
    // The list shows the id `kora-lib`; FXServer only knows the folder `kora_lib`.
    f.state().resources = vec!["kora-lib".into(), "helper".into()];
    f.state().resource_folders.insert("kora-lib".into(), "kora_lib".into());
    f.state().resource_folders.insert("helper".into(), "kora_console".into());
    f.state().status.insert("mine".into(), Status::Running);
    let a = app(&f);
    assert_eq!(call(&a, "PUT", "/servers/mine/console/enabled", Some(json!({"enabled": true}))).await.status(), 204);
    let ctl = |n: &str, action: &str| {
        let path = format!("/servers/mine/resources/{n}/control");
        let body = json!({"action": action});
        let a = &a;
        async move { call(a, "POST", &path, Some(body)).await }
    };
    let r = ctl("kora-lib", "restart").await;
    assert_eq!(r.status(), 200);
    assert_eq!(json_of(r).await["message"], "Asked the server to restart kora_lib. The Console tab shows what happened.");
    assert_eq!(ctl("kora_lib", "start").await.status(), 404, "resources are picked by the id the list shows");
    assert_eq!(ctl("helper", "stop").await.status(), 400, "the console helper is protected under any id");
    assert_eq!(std::fs::read_to_string(d.path().join(".kora/console.in")).unwrap(), "ensure kora_lib\n");
}

#[tokio::test]
async fn commands_are_refused_until_the_new_console_helper_is_loaded() {
    let d = tempfile::tempdir().unwrap();
    let f = FakeBackend::sample();
    managed_server(&f, "mine", Mode::Production, d.path());
    f.state().resources = vec!["kora_core".into()];
    f.state().status.insert("mine".into(), Status::Running);
    let a = app(&f);
    // Enabled while the server runs: the server keeps writing its log and never loads the helper.
    assert_eq!(call(&a, "PUT", "/servers/mine/console/enabled", Some(json!({"enabled": true}))).await.status(), 204);
    let token = std::fs::read_to_string(d.path().join(".kora/console.in.pending")).unwrap().trim().to_string();
    std::thread::sleep(Duration::from_millis(30));
    let log = d.path().join(".kora/console.log");
    std::fs::write(&log, "still the old process\n").unwrap();
    for (path, body) in [("/servers/mine/console", json!({"command": "status"})), ("/servers/mine/resources/kora_core/control", json!({"action": "restart"}))] {
        let r = call(&a, "POST", path, Some(body)).await;
        assert_eq!(r.status(), 409, "{path}");
        assert_eq!(json_of(r).await["error"]["message"], "The live console starts working after the server's next restart. Restart the server, then try again.");
    }
    assert!(!d.path().join(".kora/console.in").exists(), "nothing was queued");
    // After the restart the helper says it is there.
    std::fs::write(&log, format!("[script:kora_console] Kora live console ready {token}\n")).unwrap();
    assert_eq!(call(&a, "POST", "/servers/mine/console", Some(json!({"command": "status"}))).await.status(), 204);
    assert_eq!(call(&a, "POST", "/servers/mine/resources/kora_core/control", Some(json!({"action": "restart"}))).await.status(), 200);
    assert_eq!(std::fs::read_to_string(d.path().join(".kora/console.in")).unwrap(), "status\nensure kora_core\n");
}

#[tokio::test]
async fn console_stream_replays_then_follows() {
    let d = tempfile::tempdir().unwrap();
    let f = FakeBackend::sample();
    managed_server(&f, "mine", Mode::Production, d.path());
    let log = d.path().join(".kora/console.log");
    std::fs::write(&log, "one\ntwo\n").unwrap();
    let a = app(&f);
    let r = call(&a, "GET", "/servers/mine/console/stream", None).await;
    assert_eq!(r.status(), 200);
    assert!(r.headers()[header::CONTENT_TYPE].to_str().unwrap().starts_with("text/event-stream"));
    let mut body = r.into_body().into_data_stream();
    let mut got = String::new();
    while !got.contains("two") {
        got += &next_chunk(&mut body).await;
    }
    assert!(got.contains("event: line") && got.contains(r#"{"line":"one"}"#), "{got}");
    use std::io::Write;
    let mut fh = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
    write!(fh, "three\npart").unwrap();
    let mut got = String::new();
    while !got.contains("three") {
        got += &next_chunk(&mut body).await;
    }
    assert!(!got.contains("part"), "a half-written line waits for its end");
    writeln!(fh, "ial").unwrap();
    let mut got = String::new();
    while !got.contains("partial") {
        got += &next_chunk(&mut body).await;
    }
    assert_eq!(call(&a, "GET", "/servers/nope/console/stream", None).await.status(), 404);
}

#[tokio::test]
async fn events_send_the_server_list_first() {
    let f = FakeBackend::sample();
    let a = app(&f);
    let r = call(&a, "GET", "/events", None).await;
    assert_eq!(r.status(), 200);
    let mut body = r.into_body().into_data_stream();
    let mut got = String::new();
    while !got.contains("\n\n") {
        got += &next_chunk(&mut body).await;
    }
    assert!(got.starts_with("event: servers\ndata: "), "{got}");
    let data = got.lines().find_map(|l| l.strip_prefix("data: ")).unwrap();
    let v: Value = serde_json::from_str(data).unwrap();
    assert_eq!(v["servers"].as_array().unwrap().len(), 2);
    assert_eq!(v, json_of(call(&a, "GET", "/servers", None).await).await);
}
