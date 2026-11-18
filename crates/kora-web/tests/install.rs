//! The install wizard and GitHub routes, driven through the router with fakes (no network, no
//! `gh`, no installer, no real ports or disks).
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{header, Request, Response};
use kora_core::api::fake::FakeBackend;
use kora_core::api::install::github::{GithubEnv, GithubUser, LoginFlow, OwnerChoice};
use kora_core::api::install::InstallEnv;
use kora_core::artifacts::{ArtifactBuild, ArtifactList};
use kora_core::feedback::PrPlan;
use kora_core::firewall::ports::PortProber;
use kora_core::installer::{DbAdmin, InstallOutcome, InstallRequest, ProgressEvent, ProgressSink, StepStatus};
use kora_core::space::DiskInfo;
use kora_web::handlers::{github as gh_seam, install as install_seam};
use kora_web::{router, AppState};
use serde_json::{json, Value};
use tower::ServiceExt;

const ADDR: &str = "127.0.0.1:47120";
const TOKEN: &str = "testtoken";
const LICENCE: &str = "cfxk_SECRETLICENCEKEY123456";
const DBPASS: &str = "hunter2-db-password";

/// The seams are process-wide, so the tests that use them take turns.
static TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

// ---- fakes ----------------------------------------------------------------------------------

struct Prober(Vec<u16>);
impl PortProber for Prober {
    fn tcp_free(&self, p: u16) -> bool {
        !self.0.contains(&p)
    }
    fn udp_free(&self, p: u16) -> bool {
        !self.0.contains(&p)
    }
}

struct Disk(u64);
impl DiskInfo for Disk {
    fn free_bytes(&self, _: &Path) -> io::Result<u64> {
        Ok(self.0)
    }
    fn volume_key(&self, _: &Path) -> String {
        "v".into()
    }
}

#[derive(Default)]
struct S {
    elevated: bool,
    reasons: Vec<String>,
    db_ok: bool,
    saved: Option<InstallRequest>,
    fail: Option<String>,
    /// The step that fails (default `build.db`).
    fail_step: Option<&'static str>,
    runs: Vec<InstallRequest>,
    artifacts: Option<Result<ArtifactList, kora_core::problem::Problem>>,
}

struct Env {
    prober: Prober,
    disk: Disk,
    s: Mutex<S>,
}

impl Env {
    fn new() -> Arc<Env> {
        Arc::new(Env { prober: Prober(vec![30125]), disk: Disk(100 << 30), s: Mutex::new(S { elevated: true, db_ok: true, ..Default::default() }) })
    }
    fn st(&self) -> std::sync::MutexGuard<'_, S> {
        self.s.lock().unwrap()
    }
}

impl InstallEnv for Env {
    fn prober(&self) -> &dyn PortProber {
        &self.prober
    }
    fn disk(&self) -> &dyn DiskInfo {
        &self.disk
    }
    fn db_login(&self, admin: Option<&DbAdmin>) -> Result<(), kora_core::problem::Problem> {
        if self.st().db_ok && admin.is_none_or(|a| a.password.as_deref() != Some("wrong")) {
            Ok(())
        } else {
            Err("could not log in to the database server as 'root' (access denied). Check the user, password and port.".into())
        }
    }
    fn artifacts(&self) -> Result<ArtifactList, kora_core::problem::Problem> {
        self.st().artifacts.clone().unwrap_or_else(|| Err("offline".into()))
    }
    fn is_elevated(&self) -> bool {
        self.st().elevated
    }
    fn admin_reasons(&self, req: &InstallRequest) -> Vec<String> {
        if req.service {
            self.st().reasons.clone()
        } else {
            vec![]
        }
    }
    fn saved_load(&self) -> Option<InstallRequest> {
        self.st().saved.clone()
    }
    fn saved_save(&self, req: &InstallRequest) -> bool {
        self.st().saved = Some(req.clone());
        true
    }
    fn saved_clear(&self) {
        self.st().saved = None;
    }
    fn run(&self, req: &InstallRequest, sink: &mut dyn ProgressSink) -> Result<InstallOutcome, kora_core::problem::Problem> {
        self.st().runs.push(req.clone());
        let ev = |step: &str, status, pct| ProgressEvent { step: step.into(), title: format!("Title of {step}"), status, detail: "detail".into(), percent: pct };
        sink.event(ev("prepare.dir", StepStatus::Running, 10));
        sink.event(ev("prepare.dir", StepStatus::Done, 30));
        let (fail, step) = {
            let st = self.st();
            (st.fail.clone(), st.fail_step.unwrap_or("build.db"))
        };
        if let Some(m) = fail {
            // Like the installer: a step the user chose to skip is passed over.
            let state = kora_core::steps::PlanState::load_or_default(&req.dir.join(".kora/plan-state.json")).unwrap_or_default();
            if state.status(step) == Some(kora_core::steps::StepStatus::Skipped) {
                sink.event(ev(step, StepStatus::Skipped, 90));
            } else {
                sink.event(ev(step, StepStatus::Failed, 50));
                return Err(m.into());
            }
        }
        sink.event(ev("finalize", StepStatus::Done, 100));
        Ok(InstallOutcome {
            dir: req.dir.clone(),
            connect: format!("localhost:{}", req.ports.game),
            txadmin_url: format!("http://localhost:{}", req.ports.txadmin),
            notes: vec!["a note".into()],
            name: req.name.clone(),
            admin_user: Some("admin".into()),
            admin_password_file: Some(req.dir.join("secrets").join("txadmin-admin.txt")),
            ..Default::default()
        })
    }
}

type Approval = Result<Option<String>, kora_core::problem::Problem>;

/// A machine without `gh`.
struct MissingGh;
impl GithubEnv for MissingGh {
    fn auth(&self) -> GithubUser {
        GithubUser::gh_missing()
    }
    fn owners(&self) -> Result<Vec<OwnerChoice>, kora_core::problem::Problem> {
        Err("gh not found".into())
    }
    fn begin_login(&self) -> Result<LoginFlow, kora_core::problem::Problem> {
        Err(kora_core::problem::known::gh_missing())
    }
    fn resolve_owner(&self, _: Option<&str>) -> Result<String, kora_core::problem::Problem> {
        Err(kora_core::problem::known::gh_missing())
    }
}

struct Gh {
    user: Mutex<Option<String>>,
    release: Mutex<Option<mpsc::Receiver<Approval>>>,
}
impl GithubEnv for Gh {
    fn auth(&self) -> GithubUser {
        match self.user.lock().unwrap().clone() {
            Some(u) => GithubUser::ready(&u),
            None => GithubUser::signed_out(),
        }
    }
    fn owners(&self) -> Result<Vec<OwnerChoice>, kora_core::problem::Problem> {
        Ok(vec![
            OwnerChoice { login: "sam".into(), is_org: false, can_fork: true, reason: None },
            OwnerChoice { login: "acme".into(), is_org: true, can_fork: false, reason: Some("members cannot create repositories".into()) },
        ])
    }
    fn begin_login(&self) -> Result<LoginFlow, kora_core::problem::Problem> {
        let rx = self.release.lock().unwrap().take().ok_or("no sign-in prepared")?;
        Ok(LoginFlow { code: "ABCD-1234".into(), url: "https://github.com/login/device".into(), wait: Box::new(move || rx.recv().map_err(|_| "closed".to_string())?) })
    }
    fn resolve_owner(&self, requested: Option<&str>) -> Result<String, kora_core::problem::Problem> {
        match requested {
            Some(o) => Ok(o.to_string()),
            None => Ok("sam".into()),
        }
    }
}

// ---- http helpers ---------------------------------------------------------------------------

struct Rig {
    app: axum::Router,
    fake: FakeBackend,
    env: Arc<Env>,
    _turn: tokio::sync::MutexGuard<'static, ()>,
    /// The Kora folder of this test (what the install remembers goes here, never into the real one).
    _home: tempfile::TempDir,
}

async fn rig() -> Rig {
    let turn = TURN.lock().await;
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("KORA_HOME", home.path());
    let env = Env::new();
    install_seam::set_env(Some(env.clone()));
    gh_seam::set_env(Some(Arc::new(Gh { user: Mutex::new(Some("sam".into())), release: Mutex::new(None) })));
    let fake = FakeBackend::sample();
    let f = fake.clone();
    let st = AppState::system(TOKEN.into(), ADDR.parse::<SocketAddr>().unwrap()).with_backend(Arc::new(move || Box::new(f.clone())));
    Rig { app: router(st), fake, env, _turn: turn, _home: home }
}

fn tmp(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("kora-web-install-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
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
        let r: Response<Body> = self.app.clone().oneshot(b.body(body).unwrap()).await.unwrap();
        let status = r.status().as_u16();
        let bytes = axum::body::to_bytes(r.into_body(), 1 << 22).await.unwrap();
        (status, if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap_or(Value::Null) })
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
}

fn install_body(dir: &Path) -> Value {
    json!({
        "mode": "production", "name": "Alpha Town", "dir": dir.to_string_lossy(), "hostname": "Alpha RP",
        "license_key": LICENCE, "ports": {"game": 30140, "txadmin": 40140}, "txadmin_public": false,
        "artifact_url": null, "game_build": null, "resources": "all", "db": "use_existing",
        "service": false, "firewall": false, "github_owner": null, "dry_run": false,
        "db_admin": {"host": "127.0.0.1", "port": 3306, "user": "root", "password": DBPASS},
        "start": true
    })
}

fn no_secrets(v: &Value) {
    let t = v.to_string();
    assert!(!t.contains("SECRETLICENCE") && !t.contains(DBPASS), "secret leaked: {t}");
}

// ---- defaults, saved, checks ----------------------------------------------------------------

#[tokio::test]
async fn defaults_avoid_registered_ports_and_saved_answers_hold_no_secrets() {
    let r = rig().await;
    let (s, v) = r.call("GET", "/installs/defaults", None).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["name"], "My Kora Server");
    assert_eq!(v["hostname"], "My Kora Server");
    assert!(v["dir"].as_str().unwrap().ends_with("my-kora-server"));
    // prod uses 30120/40120 and dev 30130/40130, so the defaults move on.
    assert_eq!(v["ports"], json!({"game": 30121, "txadmin": 40121}));
    assert!(v["saved"].is_null());

    let mut req = kora_core::api::install::build_request(&serde_json::from_value(install_body(&tmp("saved"))).unwrap()).unwrap();
    req.license_key = Some(LICENCE.into()); // a leaky store would hand this out
    r.env.st().saved = Some(req);
    let (_, v) = r.call("GET", "/installs/defaults", None).await;
    assert_eq!(v["saved"]["name"], "Alpha Town");
    assert_eq!(v["saved"]["db"], "use_existing");
    no_secrets(&v);
    assert!(v["saved"].get("license_key").is_none() && v["saved"].get("db_admin").is_none());

    let (s, _) = r.call("DELETE", "/installs/saved", None).await;
    assert_eq!(s, 204);
    assert!(r.env.st().saved.is_none());
    let (_, v) = r.call("GET", "/installs/defaults", None).await;
    assert!(v["saved"].is_null());
}

#[tokio::test]
async fn port_check_gives_the_next_free_port_and_knows_other_servers() {
    let r = rig().await;
    let (s, v) = r.call("POST", "/checks/ports", Some(json!({"game": 30140, "txadmin": 40140}))).await;
    assert_eq!(s, 200);
    assert_eq!(v["game"], json!({"port": 30140, "ok": true, "text": "30140 is free", "suggestion": null}));
    assert!(v["txadmin"]["ok"].as_bool().unwrap());

    let (_, v) = r.call("POST", "/checks/ports", Some(json!({"game": 30125, "txadmin": 40130}))).await;
    assert_eq!(v["game"]["ok"], false);
    assert_eq!(v["game"]["suggestion"], 30126);
    assert!(v["game"]["text"].as_str().unwrap().contains("already in use"));
    assert_eq!(v["txadmin"]["ok"], false);
    assert!(v["txadmin"]["text"].as_str().unwrap().contains("your other server 'dev'"), "{v}");

    let (_, v) = r.call("POST", "/checks/ports", Some(json!({"game": 31000, "txadmin": 31000}))).await;
    assert!(v["txadmin"]["text"].as_str().unwrap().contains("same as the other port"), "{v}");
    let (s, _) = r.call("POST", "/checks/ports", Some(json!({"game": "x"}))).await;
    assert_eq!(s, 400);
}

#[tokio::test]
async fn disk_check_compares_free_space_with_what_the_install_needs() {
    let r = rig().await;
    let dir = tmp("disk");
    let (s, v) = r.call("POST", "/checks/disk", Some(json!({"dir": dir.to_string_lossy()}))).await;
    assert_eq!(s, 200);
    assert_eq!(v["ok"], true);
    assert_eq!(v["free_bytes"], 100u64 << 30);
    assert!(v["needed_bytes"].as_u64().unwrap() > (1u64 << 30));
    let r2 = Env { prober: Prober(vec![]), disk: Disk(1 << 30), s: Mutex::new(S::default()) };
    install_seam::set_env(Some(Arc::new(r2)));
    let (_, v) = r.call("POST", "/checks/disk", Some(json!({"dir": dir.to_string_lossy()}))).await;
    assert_eq!(v["ok"], false, "{v}");
    assert!(!v["message"].as_str().unwrap().is_empty());
    let (s, _) = r.call("POST", "/checks/disk", Some(json!({"dir": "  "}))).await;
    assert_eq!(s, 400);
}

#[tokio::test]
async fn database_login_test_never_echoes_the_password() {
    let r = rig().await;
    let (s, v) = r.call("POST", "/checks/db-login", Some(json!({"admin": null}))).await;
    assert_eq!((s, &v["ok"]), (200, &json!(true)));
    let (_, v) = r.call("POST", "/checks/db-login", Some(json!({"admin": {"user": "root", "password": DBPASS, "host": "127.0.0.1", "port": 3306}}))).await;
    assert_eq!(v["ok"], true);
    no_secrets(&v);
    let (_, v) = r.call("POST", "/checks/db-login", Some(json!({"admin": {"user": "root", "password": "wrong", "host": "127.0.0.1", "port": 3306}}))).await;
    assert_eq!(v["ok"], false);
    assert!(!v.to_string().contains("wrong\""));
    let (s, _) = r.call("POST", "/checks/db-login", Some(json!({"admin": {"user": "root", "host": "bad host;", "port": 1}}))).await;
    assert_eq!(s, 400);
}

/// A computer whose database server is installed but stopped.
struct StoppedDb;
impl kora_core::api::toolchain::ToolchainEnv for StoppedDb {
    fn tools(&self) -> Vec<kora_core::tools::ToolStatus> {
        vec![]
    }
    fn install(&self, _: kora_core::problem::Tool, _: &mut dyn FnMut(&str, f32)) -> Result<kora_core::tools::ToolStatus, kora_core::problem::Problem> {
        Err("no".into())
    }
    fn database(&self) -> kora_core::api::toolchain::DbState {
        kora_core::api::toolchain::DbState { installed: true, running: false, name: Some("mariadb".into()), version: None, unknown: false }
    }
    fn start_database(&self, _: &mut dyn FnMut(&str)) -> Result<String, kora_core::problem::Problem> {
        Err("no".into())
    }
    fn admin(&self) -> kora_core::api::toolchain::AdminState {
        Default::default()
    }
}

#[tokio::test]
async fn a_failed_database_login_offers_to_start_a_stopped_database() {
    let r = rig().await;
    kora_web::handlers::toolchain::set_env(Some(Arc::new(StoppedDb)));
    let (_, v) = r.call("POST", "/checks/db-login", Some(json!({"admin": {"user": "root", "password": "wrong", "host": "127.0.0.1", "port": 3306}}))).await;
    assert_eq!(v["ok"], false);
    assert_eq!(v["problem"]["code"], "install.db_stopped");
    assert_eq!(v["problem"]["actions"], json!([{"kind": "start_database"}]));
    // A database on another computer is not ours to start.
    let (_, v) = r.call("POST", "/checks/db-login", Some(json!({"admin": {"user": "root", "password": "wrong", "host": "db.example.com", "port": 3306}}))).await;
    assert!(v["problem"].is_null(), "{v}");
    kora_web::handlers::toolchain::set_env(None);
}

#[tokio::test]
async fn artifacts_are_listed_without_broken_builds() {
    let r = rig().await;
    let b = |n: u32, broken: Option<&str>| ArtifactBuild { build: n, url: format!("https://runtime.fivem.net/x/{n}"), broken: broken.map(String::from) };
    r.env.st().artifacts =
        Some(Ok(ArtifactList { recommended: Some(b(17000, None)), latest: Some(b(17200, None)), all: vec![b(17200, None), b(17100, Some("crashes")), b(17000, None)], ..Default::default() }));
    let (s, v) = r.call("GET", "/artifacts", None).await;
    assert_eq!(s, 200);
    assert_eq!(v["recommended"], 17000);
    assert_eq!(v["items"].as_array().unwrap().len(), 2);
    assert_eq!(v["items"][0]["build"], 17200);
    r.env.st().artifacts = Some(Err("no network".into()));
    let (s, v) = r.call("GET", "/artifacts", None).await;
    assert_eq!(s, 502);
    assert!(v["error"]["message"].as_str().unwrap().contains("internet"));
}

// ---- plan -----------------------------------------------------------------------------------

// Without a helper this answers "run this command" on Linux. On Windows the panel asks Windows itself
// (tests/admin_worker.rs covers that), so the answer here is different by design.
#[cfg(not(windows))]
#[tokio::test]
async fn plan_validates_like_the_cli_and_returns_no_secrets() {
    let r = rig().await;
    r.env.st().reasons = vec!["register the server as a system service (systemd)".into()];
    r.env.st().elevated = false;
    let mut body = install_body(&tmp("plan"));
    body["service"] = json!(true);
    let (s, v) = r.call("POST", "/installs/plan", Some(body)).await;
    assert_eq!(s, 200, "{v}");
    no_secrets(&v);
    assert!(v["summary"].as_str().unwrap().contains("Alpha Town"));
    assert!(v["steps"].as_array().unwrap().iter().any(|s| s["id"] == "system.service"));
    assert_eq!(v["needs_admin"], json!(["Register the server as a system service (systemd)"]));
    assert_eq!(v["disk"]["ok"], true);
    assert!(v["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("administrator")));

    let mut port_clash = install_body(&tmp("plan2"));
    port_clash["ports"] = json!({"game": 30125, "txadmin": 40140});
    let (_, v) = r.call("POST", "/installs/plan", Some(port_clash)).await;
    assert!(v["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("30125")), "{v}");

    for (field, value) in [("name", json!("bad/name")), ("dir", json!("relative/dir")), ("ports", json!({"game": 30120, "txadmin": 30120})), ("artifact_url", json!("https://evil.example/a.zip"))] {
        let mut b = install_body(&tmp("plan3"));
        b[field] = value;
        let (s, v) = r.call("POST", "/installs/plan", Some(b)).await;
        assert_eq!(s, 400, "{field}: {v}");
        no_secrets(&v);
    }
    let (s, _) = r.call("POST", "/installs/plan", Some(json!({"name": 5}))).await;
    assert_eq!(s, 400);
}

// ---- install --------------------------------------------------------------------------------

#[tokio::test]
async fn install_runs_as_a_job_with_progress_events_and_a_safe_outcome() {
    let r = rig().await;
    let dir = tmp("run");
    std::fs::create_dir_all(dir.join(".kora")).unwrap();
    r.env.st().saved = Some(kora_core::api::install::build_request(&serde_json::from_value(install_body(&dir)).unwrap()).unwrap());
    let (s, v) = r.call("POST", "/installs", Some(install_body(&dir))).await;
    assert_eq!(s, 202, "{v}");
    let id = v["job"].as_str().unwrap().to_string();
    let job = r.job(&id).await;
    assert_eq!(job["state"], "done", "{job}");
    assert_eq!(job["op"], "install_start");
    let events = job["events"].as_array().unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0], json!({"step": "prepare.dir", "title": "Title of prepare.dir", "status": "running", "detail": "detail", "percent": 10}));
    assert_eq!(events[2]["status"], "done");
    let res = &job["result"];
    assert_eq!(res["connect"], "localhost:30140");
    assert_eq!(res["txadmin_url"], "http://localhost:40140");
    assert_eq!(res["admin_user"], "admin");
    assert!(res["admin_password_file"].as_str().unwrap().ends_with("txadmin-admin.txt"));
    no_secrets(&job);
    // what the installer was given (secrets only in memory), and the saved answers are gone
    let run = r.env.st().runs[0].clone();
    assert_eq!(run.license_key.as_deref(), Some(LICENCE));
    assert_eq!(run.db_admin.unwrap().password.as_deref(), Some(DBPASS));
    assert_eq!(run.hostname, "Alpha RP");
    assert!(r.env.st().saved.is_none());

    let (s, log) = r.call("GET", &format!("/installs/{id}/log"), None).await;
    assert_eq!(s, 200);
    assert!(log["path"].as_str().unwrap().ends_with("install.log"));
    assert!(log["lines"].as_array().unwrap().iter().any(|l| l.as_str().unwrap().contains("prepare.dir")));
    no_secrets(&log);
    let (s, _) = r.call("GET", "/installs/nope/log", None).await;
    assert_eq!(s, 404);
}

#[tokio::test]
async fn a_failed_install_reports_what_was_kept_and_saves_the_answers_without_secrets() {
    let r = rig().await;
    let dir = tmp("fail");
    std::fs::create_dir_all(dir.join(".kora")).unwrap();
    r.env.st().fail = Some("could not log in to the database server as 'root'.".into());
    let (_, v) = r.call("POST", "/installs", Some(install_body(&dir))).await;
    let id = v["job"].as_str().unwrap().to_string();
    let job = r.job(&id).await;
    assert_eq!(job["state"], "failed");
    // this branch's registry keeps the raw string; the integrator's passes a JSON object through
    // The jobs route returns the error in the UI's shape: `{code, message, kept, log_path}`.
    let err: Value = job["error"].clone();
    assert_eq!(err["code"], "install.step_failed");
    assert_eq!((err["failed_step"].as_str(), err["resume"].as_bool()), (Some("build.db"), Some(true)));
    assert!(err["message"].as_str().unwrap().contains("could not log in to the database"));
    assert!(err["kept"].as_array().unwrap().len() >= 2, "{err}");
    let log_path = err["log_path"].as_str().unwrap().to_string();
    assert!(std::fs::read_to_string(&log_path).unwrap().contains("ERROR: could not log in"));
    no_secrets(&job);
    let saved = r.env.st().saved.clone().expect("answers kept");
    assert_eq!(saved.name, "Alpha Town");
    let (_, log) = r.call("GET", &format!("/installs/{id}/log"), None).await;
    assert!(log["lines"].as_array().unwrap().iter().any(|l| l.as_str().unwrap().starts_with("ERROR")));
    // the next defaults offer them again, without secrets
    let (_, d) = r.call("GET", "/installs/defaults", None).await;
    assert_eq!(d["saved"]["name"], "Alpha Town");
    no_secrets(&d);
}

/// Retry runs the failed step again in a new job, with the same answers and the licence key that
/// was only ever kept in memory; nothing is typed again.
#[tokio::test]
async fn a_stopped_install_can_retry_its_step_without_retyping_anything() {
    let r = rig().await;
    let dir = tmp("retry");
    std::fs::create_dir_all(dir.join(".kora")).unwrap();
    r.env.st().fail = Some("download failed".into());
    let (_, v) = r.call("POST", "/installs", Some(install_body(&dir))).await;
    let first = v["job"].as_str().unwrap().to_string();
    let job = r.job(&first).await;
    let err = &job["error"];
    assert_eq!(err["actions"][0], json!({"kind": "retry_step", "step": "build.db"}));
    assert!(err["actions"].as_array().unwrap().contains(&json!({"kind": "change_answers"})), "{err}");
    assert!(!err["actions"].as_array().unwrap().iter().any(|a| a["kind"] == "skip_step"), "build.db is not optional: {err}");
    assert!(err["actions"].as_array().unwrap().iter().any(|a| a["kind"] == "open_log" && a["job"] == first.as_str()), "{err}");
    no_secrets(&job);
    // Skipping a step that is not optional is refused, and the install can still be retried.
    let (s, v) = r.call("POST", &format!("/installs/{first}/resume"), Some(json!({"skip": "build.db"}))).await;
    assert_eq!((s, v["error"]["code"].as_str()), (409, Some("install.cannot_resume")), "{v}");
    // Retry: fails again, as a new job that first shows the steps already done.
    let (s, v) = r.call("POST", &format!("/installs/{first}/resume"), Some(json!({}))).await;
    assert_eq!(s, 202, "{v}");
    let second = v["job"].as_str().unwrap().to_string();
    assert_ne!(second, first);
    let job = r.job(&second).await;
    assert_eq!(job["state"], "failed");
    assert_eq!(job["events"][0]["step"], "prepare.dir");
    assert_eq!(job["events"][0]["status"], "done");
    // The first job cannot be continued twice.
    let (s, v) = r.call("POST", &format!("/installs/{first}/resume"), Some(json!({}))).await;
    assert_eq!((s, v["error"]["code"].as_str()), (404, Some("install.cannot_resume")));
    assert_eq!(v["error"]["actions"], json!([{"kind": "change_answers"}]));
    // The problem is fixed: retrying the second one finishes the install.
    r.env.st().fail = None;
    let (s, v) = r.call("POST", &format!("/installs/{second}/resume"), Some(json!({}))).await;
    assert_eq!(s, 202, "{v}");
    let job = r.job(v["job"].as_str().unwrap()).await;
    assert_eq!(job["state"], "done", "{job}");
    no_secrets(&job);
    let runs = r.env.st().runs.clone();
    assert_eq!(runs.len(), 3);
    for run in &runs {
        assert_eq!(run.license_key.as_deref(), Some(LICENCE), "the key is kept for retries");
        assert_eq!(run.db_admin.as_ref().unwrap().password.as_deref(), Some(DBPASS));
    }
}

/// An optional step (here: starting the server) that failed can be skipped; the install finishes without it.
#[tokio::test]
async fn an_optional_failed_step_can_be_skipped() {
    let r = rig().await;
    let dir = tmp("skip");
    std::fs::create_dir_all(dir.join(".kora")).unwrap();
    {
        let mut st = r.env.st();
        st.fail = Some("the server did not start".into());
        st.fail_step = Some("launch");
    }
    let (_, v) = r.call("POST", "/installs", Some(install_body(&dir))).await;
    let id = v["job"].as_str().unwrap().to_string();
    let job = r.job(&id).await;
    let actions = job["error"]["actions"].as_array().unwrap().clone();
    assert!(actions.contains(&json!({"kind": "skip_step", "step": "launch"})), "{actions:?}");
    let (s, v) = r.call("POST", &format!("/installs/{id}/resume"), Some(json!({"skip": "launch"}))).await;
    assert_eq!(s, 202, "{v}");
    let job = r.job(v["job"].as_str().unwrap()).await;
    assert_eq!(job["state"], "done", "{job}");
    assert!(job["events"].as_array().unwrap().iter().any(|e| e["step"] == "launch" && e["status"] == "skipped"));
    let st = kora_core::steps::PlanState::load_or_default(&dir.join(".kora/plan-state.json")).unwrap();
    assert_eq!(st.status("launch"), Some(kora_core::steps::StepStatus::Skipped));
    let (s, _) = r.call("POST", "/installs/nope/resume", Some(json!({}))).await;
    assert_eq!(s, 404);
    let (s, _) = r.call("POST", &format!("/installs/{id}/resume"), Some(json!({"skip": 5}))).await;
    assert_eq!(s, 400);
}

#[tokio::test]
async fn install_refuses_bad_requests_before_starting() {
    let r = rig().await;
    let dir = tmp("refuse");
    let mut b = install_body(&dir);
    b["license_key"] = Value::Null;
    let (s, v) = r.call("POST", "/installs", Some(b)).await;
    assert_eq!(s, 400);
    assert!(v["error"]["message"].as_str().unwrap().contains("licence key"));
    let mut b = install_body(&dir);
    b["name"] = json!("prod");
    let (s, v) = r.call("POST", "/installs", Some(b)).await;
    assert_eq!((s, v["error"]["code"].as_str()), (409, Some("conflict")));
    let mut b = install_body(&dir);
    b["ports"] = json!({"game": 30125, "txadmin": 40140});
    let (s, v) = r.call("POST", "/installs", Some(b)).await;
    assert_eq!((s, v["error"]["code"].as_str()), (409, Some("conflict")), "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("30126"));
    let mut b = install_body(&dir);
    b["ports"] = json!({"game": 30120, "txadmin": 40140});
    let (s, _) = r.call("POST", "/installs", Some(b)).await;
    assert_eq!(s, 409, "port of another server");
    let mut b = install_body(&dir);
    b["name"] = json!("x/y");
    assert_eq!(r.call("POST", "/installs", Some(b)).await.0, 400);
    assert!(r.env.st().runs.is_empty(), "nothing may have started");
}

// Without a helper this answers "run this command" on Linux. On Windows the panel asks Windows itself
// (tests/admin_worker.rs covers that), so the answer here is different by design.
#[cfg(not(windows))]
#[tokio::test]
async fn install_needing_administrator_rights_answers_needs_elevation_without_secrets() {
    let r = rig().await;
    r.env.st().elevated = false;
    r.env.st().reasons = vec!["register the server as a system service (systemd)".into()];
    let mut b = install_body(&tmp("elev"));
    b["service"] = json!(true);
    b["firewall"] = json!(true);
    b["resources"] = json!({"only": ["kora_hud", "ox_lib"]});
    b["app"] = json!({"desktop_shortcut": false}); // an old UI may still send it: ignored
    let (s, v) = r.call("POST", "/installs", Some(b.clone())).await;
    assert_eq!(s, 403, "{v}");
    assert_eq!(v["error"]["code"], "needs_elevation");
    let cmd: Vec<String> = v["error"]["command"].as_array().unwrap().iter().map(|c| c.as_str().unwrap().to_string()).collect();
    assert_eq!(&cmd[..2], ["kora", "install"]);
    for want in ["--name", "Alpha Town", "--service", "--firewall", "--game-port", "30140", "--resources", "kora_hud,ox_lib", "--db", "existing", "--db-admin-user", "--start"] {
        assert!(cmd.iter().any(|c| c == want), "missing {want} in {cmd:?}");
    }
    no_secrets(&v);
    let msg = v["error"]["message"].as_str().unwrap();
    assert!(msg.contains("CFX_LICENSE_KEY") && msg.contains("KORA_DB_ADMIN_PASSWORD"), "{msg}");
    assert!(r.env.st().runs.is_empty());
    // a dry run changes nothing, so it never needs elevation
    b["dry_run"] = json!(true);
    let (s, v) = r.call("POST", "/installs", Some(b.clone())).await;
    assert_eq!(s, 202);
    r.job(v["job"].as_str().unwrap()).await;
    // elevated: it goes ahead
    r.env.st().elevated = true;
    b["dry_run"] = json!(false);
    assert_eq!(r.call("POST", "/installs", Some(b)).await.0, 202);
}

#[tokio::test]
async fn development_installs_resolve_the_github_owner() {
    let r = rig().await;
    let dir = tmp("dev");
    let mut b = install_body(&dir);
    b["mode"] = json!("development");
    let (s, v) = r.call("POST", "/installs", Some(b)).await;
    assert_eq!(s, 202, "{v}");
    r.job(v["job"].as_str().unwrap()).await;
    let run = r.env.st().runs[0].clone();
    assert_eq!(run.github_owner.as_deref(), Some("sam"));
}

// ---- GitHub ---------------------------------------------------------------------------------

#[tokio::test]
async fn github_status_and_owners() {
    let r = rig().await;
    let (s, v) = r.call("GET", "/github/status", None).await;
    assert_eq!((s, v), (200, json!({"signed_in": true, "user": "sam", "state": "ready", "missing_scopes": [], "problem": null})));
    let (s, v) = r.call("GET", "/github/owners", None).await;
    assert_eq!(s, 200);
    assert_eq!(v["owners"][0], json!({"login": "sam", "kind": "user", "can_fork": true, "reason": null}));
    assert_eq!(v["owners"][1]["kind"], "org");
    gh_seam::set_env(Some(Arc::new(Gh { user: Mutex::new(None), release: Mutex::new(None) })));
    let (_, v) = r.call("GET", "/github/status", None).await;
    assert_eq!((v["signed_in"].clone(), v["state"].clone()), (json!(false), json!("signed_out")));
    assert_eq!(v["problem"]["actions"], json!([{"kind": "sign_in_github"}]));
    let (s, v) = r.call("GET", "/github/owners", None).await;
    assert_eq!((s, v["error"]["code"].as_str()), (403, Some("github.signed_out")));
    assert_eq!(v["error"]["actions"], json!([{"kind": "sign_in_github"}]));
    // gh missing is told apart from signed out, with "install it" offered.
    gh_seam::set_env(Some(Arc::new(MissingGh)));
    let (_, v) = r.call("GET", "/github/status", None).await;
    assert_eq!((v["state"].clone(), v["problem"]["code"].clone()), (json!("gh_missing"), json!("github.gh_missing")));
    assert_eq!(v["problem"]["actions"][0], json!({"kind": "install_tool", "tool": "gh"}));
    let (s, v) = r.call("GET", "/github/owners", None).await;
    assert_eq!((s, v["error"]["code"].as_str()), (403, Some("github.gh_missing")));
}

#[tokio::test]
async fn github_login_answers_with_the_code_and_completes_when_approved() {
    let r = rig().await;
    let (tx, rx) = mpsc::channel();
    gh_seam::set_env(Some(Arc::new(Gh { user: Mutex::new(None), release: Mutex::new(Some(rx)) })));
    let (s, v) = r.call("POST", "/github/login", None).await;
    assert_eq!(s, 202, "{v}");
    assert_eq!(v["code"], "ABCD-1234");
    assert_eq!(v["url"], "https://github.com/login/device");
    let id = v["job"].as_str().unwrap().to_string();
    // still waiting: a second sign-in is refused
    let (s, v2) = r.call("POST", "/github/login", None).await;
    assert_eq!((s, v2["error"]["code"].as_str()), (409, Some("conflict")));
    assert_eq!(r.call("GET", &format!("/jobs/{id}"), None).await.1["state"], "running");
    tx.send(Ok(Some("sam".into()))).unwrap();
    let job = r.job(&id).await;
    assert_eq!(job["state"], "done");
    assert_eq!(job["result"], json!({"user": "sam"}));

    // a failed sign-in fails the job and lets the next one start
    let (tx, rx) = mpsc::channel();
    gh_seam::set_env(Some(Arc::new(Gh { user: Mutex::new(None), release: Mutex::new(Some(rx)) })));
    let (s, v) = r.call("POST", "/github/login", None).await;
    assert_eq!(s, 202, "{v}");
    tx.send(Err("GitHub did not confirm the sign-in.".into())).unwrap();
    let job = r.job(v["job"].as_str().unwrap()).await;
    assert_eq!(job["state"], "failed");
    assert!(job["error"]["message"].as_str().unwrap().contains("did not confirm"));
}

#[tokio::test]
async fn issues_need_a_server_a_title_and_a_github_sign_in() {
    let r = rig().await;
    let ok = json!({"resource": "kora_hud", "title": "  It crashes  ", "text": "Steps..."});
    let (s, v) = r.call("POST", "/servers/nope/issues", Some(ok.clone())).await;
    assert_eq!((s, v["error"]["code"].as_str()), (404, Some("not_found")));
    let (s, _) = r.call("POST", "/servers/dev/issues", Some(json!({"resource": "kora_hud", "title": "  ", "text": "x"}))).await;
    assert_eq!(s, 400);
    let (s, _) = r.call("POST", "/servers/dev/issues", Some(json!({"resource": "bad name!", "title": "t"}))).await;
    assert_eq!(s, 400);
    let (s, v) = r.call("POST", "/servers/dev/issues", Some(ok.clone())).await;
    assert_eq!(s, 403, "{v}");
    r.fake.state().signed_in = true;
    let (s, v) = r.call("POST", "/servers/dev/issues", Some(ok)).await;
    assert_eq!(s, 200, "{v}");
    assert!(v["url"].as_str().unwrap().starts_with("https://github.com/"));
    assert!(r.fake.state().calls.iter().any(|c| c == "issue dev kora_hud [It crashes] [Steps...]"));
    r.fake.state().issue_result = Some(Err("GitHub said no.".into()));
    let (s, v) = r.call("POST", "/servers/dev/issues", Some(json!({"resource": "kora_hud", "title": "t", "text": ""}))).await;
    assert_eq!(s, 422);
    assert_eq!(v["error"]["message"], "GitHub said no.");
}

fn plan() -> PrPlan {
    PrPlan {
        resource: "kora_hud".into(),
        upstream: "KoraFiveM/kora_hud".into(),
        base: "dev".into(),
        head_owner: "sam".into(),
        head_branch: "fix".into(),
        ahead: 2,
        dir: "/srv/dev/resources/[kora]/kora_hud".into(),
    }
}

#[tokio::test]
async fn pull_requests_recompute_the_plan_and_refuse_a_changed_one() {
    let r = rig().await;
    r.fake.state().signed_in = true;
    r.fake.state().pr_plan = Some(Ok(plan()));
    let (s, v) = r.call("POST", "/servers/dev/pr/plan", Some(json!({"resource": "kora_hud"}))).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["upstream"], "KoraFiveM/kora_hud");
    assert_eq!(v["dir"], "/srv/dev/resources/[kora]/kora_hud");
    assert_eq!(v["ahead"], 2);
    let (s, _) = r.call("POST", "/servers/nope/pr/plan", Some(json!({"resource": "kora_hud"}))).await;
    assert_eq!(s, 404);

    // the plan exactly as the server gave it: goes ahead
    let (s, v) = r.call("POST", "/servers/dev/pr", Some(json!({"plan": plan(), "title": " Fix it ", "text": "body"}))).await;
    assert_eq!(s, 200, "{v}");
    assert!(v["url"].as_str().unwrap().contains("/pull/"));
    assert!(r.fake.state().calls.iter().any(|c| c == "pr_open kora_hud [Fix it] [body]"));

    // changed by the client: refused, nothing pushed
    let before = r.fake.state().calls.iter().filter(|c| c.starts_with("pr_open")).count();
    for tamper in [json!({"dir": "/etc"}), json!({"upstream": "evil/repo"}), json!({"head_branch": "main"}), json!({"head_owner": "mallory"}), json!({"base": "master"}), json!({"ahead": 99})] {
        let mut p = serde_json::to_value(plan()).unwrap();
        for (k, v) in tamper.as_object().unwrap() {
            p[k] = v.clone();
        }
        let (s, v) = r.call("POST", "/servers/dev/pr", Some(json!({"plan": p, "title": "t", "text": ""}))).await;
        assert_eq!(s, 400, "{tamper}: {v}");
    }
    assert_eq!(r.fake.state().calls.iter().filter(|c| c.starts_with("pr_open")).count(), before);
    let (s, _) = r.call("POST", "/servers/dev/pr", Some(json!({"plan": plan(), "title": "", "text": ""}))).await;
    assert_eq!(s, 400);
    let (s, _) = r.call("POST", "/servers/dev/pr", Some(json!({"title": "t"}))).await;
    assert_eq!(s, 400);
    // not signed in
    r.fake.state().signed_in = false;
    let (s, _) = r.call("POST", "/servers/dev/pr", Some(json!({"plan": plan(), "title": "t", "text": ""}))).await;
    assert_eq!(s, 403);
    // the resource has nothing to send any more
    r.fake.state().signed_in = true;
    r.fake.state().pr_plan = Some(Err("kora_hud has no commits to propose.".into()));
    let (s, v) = r.call("POST", "/servers/dev/pr", Some(json!({"plan": plan(), "title": "t", "text": ""}))).await;
    assert_eq!(s, 422);
    assert!(v["error"]["message"].as_str().unwrap().contains("no commits"));
}
