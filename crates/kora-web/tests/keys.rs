//! Saved licence keys through the router: the keys API, the install wizard with a key picked by name,
//! stopped installs that carry on after a panel restart, changing a server's key, the profile cache and
//! the roles. No full key may appear in any answer, event or log.
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, Request};
use kora_core::api::fake::FakeBackend;
use kora_core::api::install::InstallEnv;
use kora_core::artifacts::ArtifactList;
use kora_core::firewall::ports::PortProber;
use kora_core::installer::{DbAdmin, InstallOutcome, InstallRequest, ProgressEvent, ProgressSink, StepStatus};
use kora_core::space::DiskInfo;
use kora_web::handlers::install as install_seam;
use kora_web::remote::{Remote, RemoteConfig, Role};
use kora_web::{router, AppState};
use serde_json::{json, Value};
use tower::ServiceExt;

const ADDR: &str = "127.0.0.1:47120";
const TOKEN: &str = "testtoken";
const MAIN: &str = "cfxk_MAINKEYMAINKEYMAINKEY_1111";
const TEST: &str = "cfxk_TESTKEYTESTKEYTESTKEY_2222";
const PASTED: &str = "cfxk_PASTEDKEYPASTEDKEY_3333";
const SECRETS: [&str; 3] = ["MAINKEYMAINKEY", "TESTKEYTESTKEY", "PASTEDKEYPASTED"];

/// The seams and `KORA_HOME` are process-wide, so the tests take turns.
static TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct Prober;
impl PortProber for Prober {
    fn tcp_free(&self, _: u16) -> bool {
        true
    }
    fn udp_free(&self, _: u16) -> bool {
        true
    }
}
struct Disk;
impl DiskInfo for Disk {
    fn free_bytes(&self, _: &Path) -> io::Result<u64> {
        Ok(100 << 30)
    }
    fn volume_key(&self, _: &Path) -> String {
        "v".into()
    }
}

#[derive(Default)]
struct S {
    fail: Option<String>,
    runs: Vec<InstallRequest>,
    saved: Option<InstallRequest>,
}
struct Env {
    p: Prober,
    d: Disk,
    s: Mutex<S>,
}
impl Env {
    fn st(&self) -> std::sync::MutexGuard<'_, S> {
        self.s.lock().unwrap()
    }
}
impl InstallEnv for Env {
    fn prober(&self) -> &dyn PortProber {
        &self.p
    }
    fn disk(&self) -> &dyn DiskInfo {
        &self.d
    }
    fn db_login(&self, _: Option<&DbAdmin>) -> Result<(), String> {
        Ok(())
    }
    fn artifacts(&self) -> Result<ArtifactList, String> {
        Err("offline".into())
    }
    fn cfx_lookup(&self, name: &str) -> Result<kora_core::cfx::CfxAccount, String> {
        Ok(kora_core::cfx::CfxAccount { id: 4242, username: name.to_string() })
    }
    fn is_elevated(&self) -> bool {
        true
    }
    fn admin_reasons(&self, _: &InstallRequest) -> Vec<String> {
        vec![]
    }
    fn saved_load(&self) -> Option<InstallRequest> {
        self.st().saved.clone()
    }
    fn saved_save(&self, req: &InstallRequest) -> bool {
        let mut r = req.clone();
        r.license_key = None;
        self.st().saved = Some(r);
        true
    }
    fn saved_clear(&self) {
        self.st().saved = None;
    }
    fn run(&self, req: &InstallRequest, sink: &mut dyn ProgressSink) -> Result<InstallOutcome, String> {
        self.st().runs.push(req.clone());
        let ev = |step: &str, status, pct| ProgressEvent { step: step.into(), title: format!("Title of {step}"), status, detail: "detail".into(), percent: pct };
        sink.event(ev("prepare.dir", StepStatus::Done, 30));
        if let Some(m) = self.st().fail.clone() {
            sink.event(ev("build.db", StepStatus::Failed, 50));
            return Err(m);
        }
        sink.event(ev("finalize", StepStatus::Done, 100));
        Ok(InstallOutcome { dir: req.dir.clone(), name: req.name.clone(), ..Default::default() })
    }
}

struct Rig {
    app: axum::Router,
    fake: FakeBackend,
    env: Arc<Env>,
    home: PathBuf,
    _turn: tokio::sync::MutexGuard<'static, ()>,
    _dir: tempfile::TempDir,
}

async fn rig() -> Rig {
    let turn = TURN.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::env::set_var("KORA_HOME", &home);
    install_seam::forget_paused_in_memory();
    let env = Arc::new(Env { p: Prober, d: Disk, s: Mutex::new(S::default()) });
    install_seam::set_env(Some(env.clone()));
    let fake = FakeBackend::sample();
    let f = fake.clone();
    let st = AppState::system(TOKEN.into(), ADDR.parse::<SocketAddr>().unwrap()).with_backend(Arc::new(move || Box::new(f.clone())));
    Rig { app: router(st), fake, env, home, _turn: turn, _dir: dir }
}

impl Rig {
    async fn call(&self, method: &str, uri: &str, body: Option<Value>) -> (u16, Value) {
        let mut b = Request::builder().method(method).uri(format!("/api/v1{uri}")).header(header::HOST, ADDR).header(header::AUTHORIZATION, format!("Bearer {TOKEN}"));
        let body = match body {
            Some(v) => {
                b = b.header(header::CONTENT_TYPE, "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        let r = self.app.clone().oneshot(b.body(body).unwrap()).await.unwrap();
        let status = r.status().as_u16();
        let bytes = axum::body::to_bytes(r.into_body(), 1 << 22).await.unwrap();
        let v = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap_or(Value::Null) };
        no_key(&v);
        (status, v)
    }

    /// Like `call`, expecting 2xx.
    async fn ok(&self, method: &str, uri: &str, body: Option<Value>) -> Value {
        let (s, v) = self.call(method, uri, body).await;
        assert!((200..300).contains(&s), "{method} {uri} -> {s} {v}");
        v
    }

    async fn job(&self, id: &str) -> Value {
        let start = Instant::now();
        loop {
            let (s, v) = self.call("GET", &format!("/jobs/{id}"), None).await;
            assert_eq!(s, 200, "{v}");
            if v["state"] != "running" {
                return v;
            }
            assert!(start.elapsed() < Duration::from_secs(20), "job did not finish: {v}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn add(&self, name: &str, key: &str) {
        self.ok("POST", "/keys", Some(json!({"name": name, "license_key": key}))).await;
    }

    /// Everything in the store's folder (the database, its log and any backups) as lossy text, to look for secrets in.
    fn file(&self) -> String {
        let store = self.home.join("store");
        let mut all = String::new();
        // Fold the log into the database file first, so a key cannot hide in the log only.
        if let Ok(s) = kora_core::store::Store::open(&self.home) {
            let _ = s.read(|c| {
                let _ = c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
                Ok(())
            });
        }
        for e in std::fs::read_dir(&store).unwrap().flatten().filter(|e| e.path().is_file()) {
            all.push_str(&String::from_utf8_lossy(&std::fs::read(e.path()).unwrap()));
        }
        all
    }
}

/// No answer, event or log may carry (part of) a full key; the last four characters are allowed.
fn no_key(v: &Value) {
    let t = v.to_string();
    for s in SECRETS {
        assert!(!t.contains(s), "a key leaked: {t}");
    }
}

fn install_body(dir: &Path, key: Value) -> Value {
    let mut b = json!({
        "mode": "production", "name": "Alpha Town", "dir": dir.to_string_lossy(), "hostname": "Alpha RP",
        "ports": {"game": 30140, "txadmin": 40140}, "txadmin_public": false,
        "resources": "all", "db": "use_existing", "service": false, "firewall": false, "dry_run": false, "start": true
    });
    for (k, v) in key.as_object().unwrap() {
        b[k] = v.clone();
    }
    b
}

fn new_dir(r: &Rig, name: &str) -> PathBuf {
    let d = r._dir.path().join(name);
    std::fs::create_dir_all(d.join(".kora")).unwrap();
    d
}

// ---- the keys API -----------------------------------------------------------------------------

#[tokio::test]
async fn keys_are_added_listed_renamed_and_deleted_and_only_ever_masked() {
    let r = rig().await;
    assert_eq!(r.ok("GET", "/keys", None).await["keys"], json!([]));
    let v = r.ok("POST", "/keys", Some(json!({"name": " Main ", "license_key": MAIN}))).await;
    assert_eq!((v["key"]["name"].as_str(), v["key"]["last4"].as_str()), (Some("Main"), Some("1111")));
    r.add("Test server", TEST).await;
    let list = r.ok("GET", "/keys", None).await;
    assert_eq!(list["keys"].as_array().unwrap().len(), 2);
    assert_eq!(list["can_manage"], true);
    assert!(list["keys"][0].get("license_key").is_none() && list["keys"][0].get("key").is_none());

    // The private database holds the keys.
    assert!(r.file().contains(MAIN));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(kora_core::store::db_path(&r.home)).unwrap().permissions().mode() & 0o777, 0o600);
    }

    // Duplicates: by name (any case) and by value.
    let (s, v) = r.call("POST", "/keys", Some(json!({"name": "main", "license_key": PASTED}))).await;
    assert_eq!((s, v["error"]["code"].as_str()), (409, Some("keys.name_taken")), "{v}");
    let (s, v) = r.call("POST", "/keys", Some(json!({"name": "Again", "license_key": MAIN}))).await;
    assert_eq!((s, v["error"]["code"].as_str()), (409, Some("keys.duplicate")), "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("\"Main\""), "{v}");
    for body in [json!({"name": "", "license_key": PASTED}), json!({"name": "X", "license_key": "short"}), json!({"name": "X"})] {
        let (s, v) = r.call("POST", "/keys", Some(body)).await;
        assert_eq!((s, v["error"]["code"].as_str()), (400, Some("keys.invalid")), "{v}");
    }

    // Rename, replace.
    let v = r.ok("PUT", "/keys/Main", Some(json!({"new_name": "Live"}))).await;
    assert_eq!(v["key"]["name"], "Live");
    let v = r.ok("PUT", "/keys/live", Some(json!({"license_key": PASTED}))).await;
    assert_eq!(v["key"]["last4"], "3333");
    assert!(v["notes"][0].as_str().unwrap().contains("already installed"), "{v}");
    let (s, v) = r.call("PUT", "/keys/live", Some(json!({}))).await;
    assert_eq!((s, v["error"]["code"].as_str()), (400, Some("bad_request")), "{v}");
    let (s, v) = r.call("PUT", "/keys/nope", Some(json!({"new_name": "Z"}))).await;
    assert_eq!((s, v["error"]["code"].as_str()), (404, Some("keys.not_found")), "{v}");

    // Delete says servers keep working.
    let v = r.ok("DELETE", "/keys/live", None).await;
    assert!(v["notes"][0].as_str().unwrap().contains("keep working"), "{v}");
    let (s, _) = r.call("DELETE", "/keys/live", None).await;
    assert_eq!(s, 404);
    assert_eq!(r.ok("GET", "/keys", None).await["keys"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn a_damaged_keys_file_is_a_guided_problem_and_is_never_overwritten() {
    let r = rig().await;
    let junk = format!("{{\"keys\": [{{\"name\": \"Main\", \"key\": \"{MAIN}\"");
    std::fs::write(r.home.join("secrets.json"), &junk).unwrap();
    for (m, u, b) in [("GET", "/keys", None), ("POST", "/keys", Some(json!({"name": "New", "license_key": TEST}))), ("DELETE", "/keys/Main", None)] {
        let (s, v) = r.call(m, u, b).await;
        assert_eq!((s, v["error"]["code"].as_str()), (500, Some("keys.unreadable")), "{m} {u} {v}");
        assert!(v["error"]["message"].as_str().unwrap().contains("no key was lost"), "{v}");
    }
    // The wizard cannot start an install with a named key either, and says why.
    let dir = new_dir(&r, "a");
    let (s, v) = r.call("POST", "/installs", Some(install_body(&dir, json!({"license_key_name": "Main"})))).await;
    assert_eq!((s, v["error"]["code"].as_str()), (500, Some("keys.unreadable")), "{v}");
    assert_eq!(std::fs::read_to_string(r.home.join("secrets.json")).unwrap(), junk);
}

#[tokio::test]
async fn keys_show_which_servers_use_them() {
    let r = rig().await;
    r.add("Main", MAIN).await;
    let d = r._dir.path().join("srv-prod");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("secrets.cfg"), format!("sv_licenseKey \"{MAIN}\"\n")).unwrap();
    r.fake.state().servers[0].dir = d;
    let list = r.ok("GET", "/keys", None).await;
    assert_eq!(list["keys"][0]["servers"], json!(["prod"]), "{list}");
}

// ---- installing with a saved key --------------------------------------------------------------

#[tokio::test]
async fn an_install_can_pick_a_saved_key_by_name_and_the_key_never_shows() {
    let r = rig().await;
    r.add("Main", MAIN).await;
    let dir = new_dir(&r, "byname");
    let (s, v) = r.call("POST", "/installs", Some(install_body(&dir, json!({"license_key_name": "main"})))).await;
    assert_eq!(s, 202, "{v}");
    let id = v["job"].as_str().unwrap().to_string();
    let job = r.job(&id).await;
    assert_eq!(job["state"], "done", "{job}");
    let run = r.env.st().runs[0].clone();
    assert_eq!(run.license_key.as_deref(), Some(MAIN), "the installer got the real key");
    assert_eq!(run.license_key_name.as_deref(), Some("main"));
    let (_, log) = r.call("GET", &format!("/installs/{id}/log"), None).await;
    assert!(log["lines"].as_array().is_some());
    // A name that does not exist, and a name together with a key.
    let (s, v) = r.call("POST", "/installs", Some(install_body(&new_dir(&r, "x"), json!({"license_key_name": "Nope"})))).await;
    assert_eq!((s, v["error"]["code"].as_str()), (404, Some("keys.not_found")), "{v}");
    let (s, v) = r.call("POST", "/installs", Some(install_body(&new_dir(&r, "y"), json!({"license_key_name": "Main", "license_key": PASTED})))).await;
    assert_eq!(s, 400, "{v}");
    // The plan checks the name too, so the wizard says so at the licence page.
    let (s, v) = r.call("POST", "/installs/plan", Some(install_body(&new_dir(&r, "z"), json!({"license_key_name": "Nope"})))).await;
    assert_eq!((s, v["error"]["code"].as_str()), (404, Some("keys.not_found")), "{v}");
}

#[tokio::test]
async fn a_pasted_key_is_saved_when_asked_and_not_otherwise() {
    let r = rig().await;
    // Not saved: no file, the install still gets it.
    let (s, v) = r.call("POST", "/installs", Some(install_body(&new_dir(&r, "p1"), json!({"license_key": PASTED})))).await;
    assert_eq!(s, 202, "{v}");
    r.job(v["job"].as_str().unwrap()).await;
    assert!(!r.home.join("secrets.json").exists());
    // Saved under the name the person gave.
    let mut b = install_body(&new_dir(&r, "p2"), json!({"license_key": PASTED, "save_license_key_as": "Test server"}));
    b["name"] = json!("Beta Town");
    b["ports"] = json!({"game": 30150, "txadmin": 40150});
    let (s, v) = r.call("POST", "/installs", Some(b)).await;
    assert_eq!(s, 202, "{v}");
    r.job(v["job"].as_str().unwrap()).await;
    let keys = r.ok("GET", "/keys", None).await;
    assert_eq!((keys["keys"][0]["name"].as_str(), keys["keys"][0]["last4"].as_str()), (Some("Test server"), Some("3333")));
    assert_eq!(r.env.st().runs[1].license_key_name.as_deref(), Some("Test server"));
    // Pasting an already saved key reuses its name instead of failing or saving twice.
    let mut b = install_body(&new_dir(&r, "p3"), json!({"license_key": PASTED, "save_license_key_as": "Other name"}));
    b["name"] = json!("Gamma Town");
    b["ports"] = json!({"game": 30160, "txadmin": 40160});
    let (_, v) = r.call("POST", "/installs", Some(b)).await;
    r.job(v["job"].as_str().unwrap()).await;
    assert_eq!(r.ok("GET", "/keys", None).await["keys"].as_array().unwrap().len(), 1);
    assert_eq!(r.env.st().runs[2].license_key_name.as_deref(), Some("Test server"));
    // A name that is taken is refused before anything starts.
    let mut b = install_body(&new_dir(&r, "p4"), json!({"license_key": TEST, "save_license_key_as": "test SERVER"}));
    b["name"] = json!("Delta Town");
    let (s, v) = r.call("POST", "/installs", Some(b)).await;
    assert_eq!((s, v["error"]["code"].as_str()), (409, Some("keys.name_taken")), "{v}");
    assert_eq!(r.env.st().runs.len(), 3);
}

#[tokio::test]
async fn a_stopped_install_with_a_saved_key_carries_on_after_a_restart_without_a_secret_on_disk() {
    let r = rig().await;
    r.add("Main", MAIN).await;
    let dir = new_dir(&r, "resume");
    r.env.st().fail = Some("download failed".into());
    let (_, v) =
        r.call("POST", "/installs", Some(install_body(&dir, json!({"license_key_name": "Main", "db_admin": {"host": "127.0.0.1", "port": 3306, "user": "root", "password": "hunter2-db"}})))).await;
    let first = v["job"].as_str().unwrap().to_string();
    let job = r.job(&first).await;
    assert_eq!(job["state"], "failed", "{job}");
    assert_eq!(job["error"]["resume"], true);
    // Written down in the store, without the key or the database password.
    let text: String =
        kora_core::store::Store::open(&r.home).unwrap().read(|c| Ok(c.query_row("SELECT req_json FROM paused_installs WHERE job = ?1", [&first], |row| row.get::<_, String>(0)).unwrap())).unwrap();
    assert!(!text.contains("MAINKEY") && !text.contains("hunter2-db"), "{text}");
    assert!(text.contains("\"license_key_name\":\"Main\""), "{text}");
    assert!(!r.file().contains("hunter2-db"), "not even in the database file");
    // The wizard is told it can carry on.
    let d = r.ok("GET", "/installs/defaults", None).await;
    assert_eq!(d["resumable"][0]["job"], first.as_str(), "{d}");
    assert_eq!(d["saved"]["license_key_name"], "Main", "the saved answers keep the key's name: {d}");

    // The panel restarts: memory is gone, the file is not.
    install_seam::forget_paused_in_memory();
    r.env.st().fail = None;
    let (s, v) = r.call("POST", &format!("/installs/{first}/resume"), Some(json!({}))).await;
    assert_eq!(s, 202, "{v}");
    let second = v["job"].as_str().unwrap().to_string();
    let job = r.job(&second).await;
    assert_eq!(job["state"], "done", "{job}");
    assert_eq!(r.env.st().runs.last().unwrap().license_key.as_deref(), Some(MAIN), "the key was looked up by name");
    assert!(kora_core::installer::paused::PausedStore::at(r.home.join("paused-installs")).load(&first).is_none(), "a continued install is not offered again");
    assert_eq!(r.ok("GET", "/installs/defaults", None).await["resumable"], json!([]));
}

#[tokio::test]
async fn a_stopped_install_follows_a_renamed_key_and_says_so_when_the_key_is_gone() {
    let r = rig().await;
    r.add("Main", MAIN).await;
    let dir = new_dir(&r, "rename");
    r.env.st().fail = Some("download failed".into());
    let (_, v) = r.call("POST", "/installs", Some(install_body(&dir, json!({"license_key_name": "Main"})))).await;
    let first = v["job"].as_str().unwrap().to_string();
    r.job(&first).await;
    // The key is in use by a stopped install: the list says so, a rename follows, a delete warns.
    let list = r.ok("GET", "/keys", None).await;
    assert!(list["keys"][0]["stopped_installs"].as_u64().unwrap() >= 1, "{list}");
    let v = r.ok("PUT", "/keys/Main", Some(json!({"new_name": "Live"}))).await;
    assert!(v["notes"][0].as_str().unwrap().contains("unfinished install"), "{v}");
    install_seam::forget_paused_in_memory();
    r.env.st().fail = None;
    let (s, v) = r.call("POST", &format!("/installs/{first}/resume"), Some(json!({}))).await;
    assert_eq!(s, 202, "{v}");
    r.job(v["job"].as_str().unwrap()).await;

    // Stop again, delete the key, restart the panel.
    r.env.st().fail = Some("download failed".into());
    let (_, v) = r.call("POST", "/installs", Some(install_body(&dir, json!({"license_key_name": "Live"})))).await;
    let again = v["job"].as_str().unwrap().to_string();
    r.job(&again).await;
    let v = r.ok("DELETE", "/keys/Live", None).await;
    assert!(v["notes"].as_array().unwrap().iter().any(|n| n.as_str().unwrap().contains("will need another one")), "{v}");
    install_seam::forget_paused_in_memory();
    let (s, v) = r.call("POST", &format!("/installs/{again}/resume"), Some(json!({}))).await;
    assert_eq!((s, v["error"]["code"].as_str()), (404, Some("keys.not_found")), "{v}");
    assert!(v["error"]["actions"].as_array().unwrap().iter().any(|a| a["kind"] == "change_answers"), "{v}");
    // The saved answers do not silently pick nothing.
    let d = r.ok("GET", "/installs/defaults", None).await;
    assert_eq!((d["saved"]["license_key_name"].is_null(), d["saved_key_missing"].as_str()), (true, Some("Live")), "{d}");
    // Adding the key again under the same name lets it carry on.
    r.add("Live", MAIN).await;
    r.env.st().fail = None;
    let (s, v) = r.call("POST", &format!("/installs/{again}/resume"), Some(json!({}))).await;
    assert_eq!(s, 202, "{v}");
    assert_eq!(r.job(v["job"].as_str().unwrap()).await["state"], "done");
}

#[tokio::test]
async fn a_pasted_key_that_was_not_saved_is_only_in_memory() {
    let r = rig().await;
    let dir = new_dir(&r, "mem");
    r.env.st().fail = Some("download failed".into());
    let (_, v) = r.call("POST", "/installs", Some(install_body(&dir, json!({"license_key": PASTED})))).await;
    let id = v["job"].as_str().unwrap().to_string();
    r.job(&id).await;
    let paused = kora_core::installer::paused::PausedStore::at(r.home.join("paused-installs"));
    assert!(paused.list().is_empty() && kora_core::keystore::KeyStore::at(r.home.join("secrets.json")).list().unwrap().is_empty());
    assert!(!r.file().contains("PASTEDKEY"), "a pasted key that was not saved is nowhere on disk");
    assert!(!r.env.st().saved.as_ref().is_some_and(|s| s.license_key.is_some()));
    // Works while the panel runs ...
    r.env.st().fail = None;
    let (s, v) = r.call("POST", &format!("/installs/{id}/resume"), Some(json!({}))).await;
    assert_eq!(s, 202, "{v}");
    r.job(v["job"].as_str().unwrap()).await;
    // ... and after a restart it cannot, and says what to do.
    r.env.st().fail = Some("again".into());
    let (_, v) = r.call("POST", "/installs", Some(install_body(&dir, json!({"license_key": PASTED})))).await;
    let id = v["job"].as_str().unwrap().to_string();
    r.job(&id).await;
    install_seam::forget_paused_in_memory();
    let (s, v) = r.call("POST", &format!("/installs/{id}/resume"), Some(json!({}))).await;
    assert_eq!((s, v["error"]["code"].as_str()), (404, Some("install.cannot_resume")), "{v}");
}

// ---- changing the key of an installed server ---------------------------------------------------

fn server_with_key(r: &Rig, status: kora_core::api::Status) -> PathBuf {
    let d = r._dir.path().join("srv-dev");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("secrets.cfg"), "# Kora secrets\nsv_licenseKey \"cfxk_OLDOLDOLDOLD_0000\"\nset mysql_connection_string \"mysql://u:p@h/d\"\n").unwrap();
    let mut st = r.fake.state();
    st.servers[1].dir = d.clone();
    st.status.insert("dev".into(), status);
    d
}

#[tokio::test]
async fn change_licence_key_on_a_server_and_say_whether_a_restart_is_needed() {
    let r = rig().await;
    r.add("Main", MAIN).await;
    let d = server_with_key(&r, kora_core::api::Status::Stopped);
    let v = r.ok("PUT", "/servers/dev/licence-key", Some(json!({"name": "Main"}))).await;
    assert_eq!((v["restart_needed"].as_bool(), v["last4"].as_str(), v["key_name"].as_str()), (Some(false), Some("1111"), Some("Main")), "{v}");
    assert!(v["message"].as_str().unwrap().contains("No restart is needed"));
    let cfg = std::fs::read_to_string(d.join("secrets.cfg")).unwrap();
    assert!(cfg.contains(&format!("sv_licenseKey \"{MAIN}\"")) && cfg.contains("mysql_connection_string"), "{cfg}");
    // Running: a restart is needed and offered.
    r.fake.state().status.insert("dev".into(), kora_core::api::Status::Running);
    r.add("Test server", TEST).await;
    let v = r.ok("PUT", "/servers/dev/licence-key", Some(json!({"name": "Test server"}))).await;
    assert_eq!(v["restart_needed"], true);
    assert!(v["message"].as_str().unwrap().contains("restart"));
    // A new key, saved on the way (the answer still shows only the last four).
    let v = r.ok("PUT", "/servers/dev/licence-key", Some(json!({"license_key": PASTED, "save_as": "Third"}))).await;
    assert_eq!((v["key_name"].as_str(), v["last4"].as_str()), (Some("Third"), Some("3333")), "{v}");
    let list = r.ok("GET", "/keys", None).await;
    assert_eq!(list["keys"].as_array().unwrap().len(), 3);
    assert!(list["keys"].as_array().unwrap().iter().any(|k| k["name"] == "Third" && k["servers"] == json!(["dev"])), "{list}");
    // Problems.
    let (s, v) = r.call("PUT", "/servers/dev/licence-key", Some(json!({"name": "Nope"}))).await;
    assert_eq!((s, v["error"]["code"].as_str()), (404, Some("keys.not_found")), "{v}");
    let (s, _) = r.call("PUT", "/servers/dev/licence-key", Some(json!({}))).await;
    assert_eq!(s, 400);
    let (s, _) = r.call("PUT", "/servers/dev/licence-key", Some(json!({"name": "Main", "license_key": PASTED}))).await;
    assert_eq!(s, 400);
    let (s, _) = r.call("PUT", "/servers/ghost/licence-key", Some(json!({"name": "Main"}))).await;
    assert_eq!(s, 404);
    // A server whose configuration has no key line: guided, nothing written.
    std::fs::write(d.join("secrets.cfg"), "set x 1\n").unwrap();
    let (s, v) = r.call("PUT", "/servers/dev/licence-key", Some(json!({"name": "Main"}))).await;
    assert_eq!((s, v["error"]["code"].as_str()), (409, Some("keys.server_config")), "{v}");
    assert_eq!(std::fs::read_to_string(d.join("secrets.cfg")).unwrap(), "set x 1\n");
}

// ---- the profile cache -------------------------------------------------------------------------

#[tokio::test]
async fn the_cfx_name_and_github_owner_are_remembered_in_a_separate_non_secret_file() {
    let r = rig().await;
    let d = r.ok("GET", "/installs/defaults", None).await;
    assert_eq!(d["profile"], json!({"cfx_username": null, "cfx_identifier": null, "github_owner": null}));
    let v = r.ok("POST", "/checks/cfx", Some(json!({"username": "JSixty"}))).await;
    assert_eq!(v["ok"], true);
    let d = r.ok("GET", "/installs/defaults", None).await;
    assert_eq!((d["profile"]["cfx_username"].as_str(), d["profile"]["cfx_identifier"].as_str()), (Some("JSixty"), Some("fivem:4242")), "{d}");
    let p = kora_core::profile::ProfileStore::at(r.home.join("profile.json")).load();
    assert_eq!(p.cfx_username.as_deref(), Some("JSixty"));
    assert!(kora_core::keystore::KeyStore::at(r.home.join("secrets.json")).list().unwrap().is_empty(), "the profile is not in the keys table");
    // Editing and forgetting.
    let v = r.ok("PUT", "/profile", Some(json!({"github_owner": "KoraFiveM", "cfx_username": null, "cfx_identifier": null}))).await;
    assert_eq!(v, json!({"cfx_username": null, "cfx_identifier": null, "github_owner": "KoraFiveM"}));
    assert_eq!(r.ok("GET", "/profile", None).await["github_owner"], "KoraFiveM");
    for bad in [json!({"github_owner": "-bad-"}), json!({"cfx_identifier": "steam:1"}), json!({"cfx_username": "x"}), json!({"cfx_username": 5})] {
        let (s, _) = r.call("PUT", "/profile", Some(bad)).await;
        assert_eq!(s, 400);
    }
}

// ---- roles -----------------------------------------------------------------------------------

const BOUND: &str = "100.64.0.5:47120";
const REMOTE_PEER: &str = "100.64.0.20:51000";

async fn remote(app: &axum::Router, token: &str, method: &str, uri: &str, body: Option<Value>) -> (u16, Value) {
    let mut b = Request::builder().method(method).uri(format!("/api/v1{uri}")).header(header::HOST, BOUND).header(header::AUTHORIZATION, format!("Bearer {token}"));
    let body = match body {
        Some(v) => {
            b = b.header(header::CONTENT_TYPE, "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let mut req = b.body(body).unwrap();
    req.extensions_mut().insert(ConnectInfo(REMOTE_PEER.parse::<SocketAddr>().unwrap()));
    let r = app.clone().oneshot(req).await.unwrap();
    let status = r.status().as_u16();
    let bytes = axum::body::to_bytes(r.into_body(), 1 << 22).await.unwrap();
    let v = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    no_key(&v);
    (status, v)
}

#[tokio::test]
async fn only_admins_manage_keys_other_roles_can_only_pick_saved_names() {
    let r = rig().await;
    r.add("Main", MAIN).await;
    server_with_key(&r, kora_core::api::Status::Stopped);
    let mut cfg = RemoteConfig { enabled: true, ..Default::default() };
    let viewer = cfg.add_user("vera", Role::Viewer).unwrap();
    let operator = cfg.add_user("olly", Role::Operator).unwrap();
    let admin = cfg.add_user("adam", Role::Admin).unwrap();
    let f = r.fake.clone();
    let st = AppState::system(TOKEN.into(), BOUND.parse().unwrap()).with_backend(Arc::new(move || Box::new(f.clone()))).with_remote(Arc::new(Remote::new(None, cfg)));
    let app = router(st);

    // A viewer sees nothing of it.
    assert_eq!(remote(&app, &viewer, "GET", "/keys", None).await.0, 403);
    // An operator sees names and last four, cannot manage, cannot paste, can pick.
    let (s, v) = remote(&app, &operator, "GET", "/keys", None).await;
    assert_eq!(s, 200);
    assert_eq!((v["can_manage"].as_bool(), v["path"].is_null(), v["keys"][0]["last4"].as_str()), (Some(false), true, Some("1111")), "{v}");
    for (m, u, b) in [
        ("POST", "/keys", Some(json!({"name": "X", "license_key": TEST}))),
        ("PUT", "/keys/Main", Some(json!({"new_name": "Y"}))),
        ("DELETE", "/keys/Main", None),
        ("PUT", "/profile", Some(json!({"github_owner": "x"}))),
        ("PUT", "/servers/dev/licence-key", Some(json!({"license_key": TEST}))),
        ("PUT", "/servers/dev/licence-key", Some(json!({"license_key": TEST, "save_as": "Mine"}))),
    ] {
        let (s, v) = remote(&app, &operator, m, u, b).await;
        assert_eq!(s, 403, "{m} {u} {v}");
    }
    let (s, v) = remote(&app, &operator, "PUT", "/servers/dev/licence-key", Some(json!({"name": "Main"}))).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(r.ok("GET", "/keys", None).await["keys"].as_array().unwrap().len(), 1, "nothing was added or removed");
    // An admin manages them.
    let (s, v) = remote(&app, &admin, "POST", "/keys", Some(json!({"name": "Test server", "license_key": TEST}))).await;
    assert_eq!(s, 201, "{v}");
    let (s, v) = remote(&app, &admin, "GET", "/keys", None).await;
    assert_eq!((s, v["can_manage"].as_bool()), (200, Some(true)));
    assert_eq!(remote(&app, &admin, "DELETE", "/keys/Test%20server", None).await.0, 200);
}

#[test]
fn the_handler_level_rule_matches_the_role_table() {
    use kora_web::handlers::keys::may_manage;
    use kora_web::remote::{Caller, Identity};
    let mk = |role| {
        let mut c = Caller::local();
        c.remote = true;
        c.identity = Some(Identity { name: "x".into(), provider: kora_web::remote::Provider::Local, role });
        c
    };
    assert!(may_manage(&Caller::local()));
    assert!(may_manage(&mk(Role::Admin)));
    assert!(!may_manage(&mk(Role::Operator)) && !may_manage(&mk(Role::Viewer)));
}
