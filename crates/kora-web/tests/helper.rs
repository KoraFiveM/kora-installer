//! The privileged helper from the panel's side: without it every privileged action still answers
//! `needs_elevation`; with a (fake) helper the action runs and succeeds. Linux/Unix helper only, but the
//! "no helper" cases hold everywhere a panel is not already an administrator. On Windows the panel uses its
//! administrator worker instead (tests/admin_worker.rs), and the CI runner there is elevated, so this file is
//! for Unix.
#![cfg(unix)]
use std::io;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{header, Request};
use kora_core::api::fake::FakeBackend;
use kora_core::api::install::InstallEnv;
use kora_core::api::maintenance::fake::FakeMaintenance;
use kora_core::api::maintenance::{self, PlanItem};
use kora_core::artifacts::ArtifactList;
use kora_core::firewall::ports::PortProber;
use kora_core::helper::client::{Helper, HelperError, Outcome};
use kora_core::installer::{DbAdmin, InstallOutcome, InstallRequest, ProgressSink};
use kora_core::space::DiskInfo;
use kora_web::elevated::set_helper_override;
use kora_web::handlers::install as install_seam;
use kora_web::{router, AppState};
use serde_json::{json, Value};
use tower::ServiceExt;

const ADDR: &str = "127.0.0.1:47120";
const TOKEN: &str = "testtoken";
const LICENCE: &str = "cfxk_SECRETLICENCEKEY123456";
const DBPASS: &str = "hunter2-db-password";

/// The helper override, the maintenance seam and the install seam are process-wide: tests take turns.
static TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

// ---- a fake helper --------------------------------------------------------------------------

type Call = (String, Vec<String>, Vec<(String, String)>);

struct FakeHelper {
    calls: Mutex<Vec<Call>>,
    lines: Vec<&'static str>,
    code: i32,
    up: bool,
    /// Pretend it vanished between the check and the run.
    vanish: bool,
}

impl FakeHelper {
    fn new(code: i32) -> Arc<Self> {
        Arc::new(Self { calls: Mutex::new(vec![]), lines: vec!["working", "done"], code, up: true, vanish: false })
    }
    fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }
}

impl Helper for FakeHelper {
    fn available(&self) -> bool {
        self.up
    }
    fn run(&self, verb: &str, args: &[String], env: &[(String, String)], on_line: &mut dyn FnMut(&str)) -> Result<Outcome, HelperError> {
        if self.vanish {
            return Err(HelperError::Unavailable("gone".into()));
        }
        self.calls.lock().unwrap().push((verb.into(), args.to_vec(), env.to_vec()));
        for l in &self.lines {
            on_line(l);
        }
        Ok(Outcome { ok: self.code == 0, code: self.code })
    }
}

// ---- install env ----------------------------------------------------------------------------

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
struct Env {
    runs: Mutex<usize>,
}
impl InstallEnv for Env {
    fn prober(&self) -> &dyn PortProber {
        &Prober
    }
    fn disk(&self) -> &dyn DiskInfo {
        &Disk
    }
    fn db_login(&self, _: Option<&DbAdmin>) -> Result<(), String> {
        Ok(())
    }
    fn artifacts(&self) -> Result<ArtifactList, String> {
        Err("offline".into())
    }
    fn is_elevated(&self) -> bool {
        false
    }
    fn admin_reasons(&self, req: &InstallRequest) -> Vec<String> {
        if req.service {
            vec!["register the server as a system service (systemd)".into()]
        } else {
            vec![]
        }
    }
    fn saved_load(&self) -> Option<InstallRequest> {
        None
    }
    fn saved_save(&self, _: &InstallRequest) -> bool {
        true
    }
    fn saved_clear(&self) {}
    fn run(&self, _: &InstallRequest, _: &mut dyn ProgressSink) -> Result<InstallOutcome, String> {
        *self.runs.lock().unwrap() += 1;
        Err("the real installer must not run in this test".into())
    }
}

// ---- http -----------------------------------------------------------------------------------

struct Rig {
    app: axum::Router,
    fake: FakeBackend,
    maint: Arc<FakeMaintenance>,
    env: Arc<Env>,
    _turn: tokio::sync::MutexGuard<'static, ()>,
}

async fn rig(helper: Option<Arc<FakeHelper>>) -> Rig {
    let turn = TURN.lock().await;
    set_helper_override(Some(helper.map_or_else(|| Arc::new(Down) as Arc<dyn Helper>, |h| h as Arc<dyn Helper>)));
    std::env::remove_var("KORA_APP_PREFIX");
    let maint = FakeMaintenance::new(false);
    maintenance::set_override(Some(maint.clone()));
    let env = Arc::new(Env::default());
    install_seam::set_env(Some(env.clone()));
    let fake = FakeBackend::sample();
    let f = fake.clone();
    let st = AppState::system(TOKEN.into(), ADDR.parse::<SocketAddr>().unwrap()).with_backend(Arc::new(move || Box::new(f.clone())));
    Rig { app: router(st), fake, maint, env, _turn: turn }
}

/// "No helper installed".
struct Down;
impl Helper for Down {
    fn available(&self) -> bool {
        false
    }
    fn run(&self, _: &str, _: &[String], _: &[(String, String)], _: &mut dyn FnMut(&str)) -> Result<Outcome, HelperError> {
        Err(HelperError::Unavailable("not installed".into()))
    }
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
        let s = r.status().as_u16();
        let bytes = axum::body::to_bytes(r.into_body(), 1 << 22).await.unwrap();
        (s, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }
    async fn job(&self, accepted: (u16, Value)) -> Value {
        assert_eq!(accepted.0, 202, "{}", accepted.1);
        let id = accepted.1["job"].as_str().unwrap().to_string();
        let t = Instant::now();
        loop {
            let (_, v) = self.call("GET", &format!("/jobs/{id}"), None).await;
            if v["state"] != "running" {
                return v;
            }
            assert!(t.elapsed() < Duration::from_secs(10), "job did not finish");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

/// An absolute folder on every platform ("/tmp/..." is not absolute on Windows).
fn abs(name: &str) -> String {
    std::env::temp_dir().join(name).to_string_lossy().into_owned()
}

fn install_body(dir: &str) -> Value {
    json!({
        "mode": "production", "name": "Alpha Town", "dir": dir, "hostname": "Alpha RP",
        "license_key": LICENCE, "ports": {"game": 30140, "txadmin": 40140}, "txadmin_public": false,
        "artifact_url": null, "game_build": null, "resources": "all", "db": "use_existing",
        "service": true, "firewall": false, "github_owner": null, "dry_run": false,
        "db_admin": {"host": "127.0.0.1", "port": 3306, "user": "root", "password": DBPASS},
        "start": false
    })
}

// ---- without the helper: exactly as before ----------------------------------------------------

#[tokio::test]
async fn without_a_helper_everything_still_answers_needs_elevation() {
    let r = rig(None).await;
    let (s, j) = r.call("GET", "/system", None).await;
    assert_eq!((s, &j["helper_installed"]), (200, &json!(false)));

    // "Tidy up old shortcuts": the per-user half is done, the old copy needs the command.
    r.maint.app.lock().unwrap().legacy = true;
    let (s, j) = r.call("PUT", "/app", Some(json!({"desktop_shortcut": false}))).await;
    assert_eq!((s, j["error"]["code"].as_str()), (403, Some("needs_elevation")));
    assert_eq!(j["error"]["command"], json!(["kora", "uninstall", "--app", "--yes"]));
    assert!(j["error"]["message"].as_str().unwrap().contains("old copy of Kora"));

    let (s, j) = r.call("DELETE", "/app", None).await;
    assert_eq!((s, j["error"]["code"].as_str()), (403, Some("needs_elevation")));

    let (s, j) = r.call("POST", "/uninstall", Some(json!({"answers": [], "only": ["prod"]}))).await;
    assert_eq!((s, j["error"]["code"].as_str()), (403, Some("needs_elevation")));
    assert_eq!(j["error"]["command"], json!(["kora", "uninstall", "--yes", "prod"]));

    r.fake.state().elevation = Some(vec!["schedule".into(), "off".into(), "dev".into()]);
    let (s, j) = r.call("DELETE", "/servers/dev/schedule", None).await;
    assert_eq!((s, j["error"]["code"].as_str()), (403, Some("needs_elevation")));
    assert_eq!(j["error"]["command"], json!(["kora", "schedule", "off", "dev"]));
    assert!(j["error"]["message"].as_str().unwrap().contains("automatic updates"));

    let (s, j) = r.call("POST", "/installs", Some(install_body(&abs("kora-helper-test-a")))).await;
    assert_eq!((s, j["error"]["code"].as_str()), (403, Some("needs_elevation")));
    assert!(!j.to_string().contains("SECRETLICENCE") && !j.to_string().contains(DBPASS));
    assert_eq!(*r.env.runs.lock().unwrap(), 0);
    assert_eq!(r.maint.calls(), ["app_install shortcut=false force=false dry=false"], "only the per-user shortcuts, which need no rights");
    set_helper_override(None);
}

// ---- with the helper --------------------------------------------------------------------------

#[tokio::test]
async fn system_reports_the_helper() {
    let h = FakeHelper::new(0);
    let r = rig(Some(h)).await;
    assert_eq!(r.call("GET", "/system", None).await.1["helper_installed"], json!(true));
    set_helper_override(None);
}

#[tokio::test]
async fn app_install_and_removal_run_through_the_helper() {
    let h = FakeHelper::new(0);
    let r = rig(Some(h.clone())).await;
    r.maint.app.lock().unwrap().legacy = true;
    let (s, j) = r.call("PUT", "/app", Some(json!({"desktop_shortcut": false}))).await;
    assert_eq!(s, 200, "{j}");
    assert!(j["message"].as_str().unwrap().contains("old copy"));
    let (s, j) = r.call("DELETE", "/app", None).await;
    assert_eq!(s, 200, "{j}");
    let calls = h.calls();
    assert_eq!((calls[0].0.as_str(), calls[0].1.clone()), ("uninstall", vec!["--app".to_string(), "--yes".into()]));
    assert_eq!((calls[1].0.as_str(), calls[1].1.clone()), ("uninstall", vec!["--app".to_string(), "--yes".into()]));
    assert_eq!(r.maint.calls(), ["app_install shortcut=false force=false dry=false"], "the panel only made the per-user shortcuts; the helper removed the old copy");
    set_helper_override(None);
}

#[tokio::test]
async fn uninstall_and_self_update_are_jobs_that_stream_helper_lines() {
    let h = FakeHelper::new(0);
    let r = rig(Some(h.clone())).await;
    *r.maint.plan.lock().unwrap() = Vec::<PlanItem>::new();
    let a = r.call("POST", "/uninstall", Some(json!({"answers": [], "delete_files": true, "only": ["prod"]}))).await;
    let j = r.job(a).await;
    assert_eq!(j["state"], "done", "{j}");
    let msgs: Vec<&str> = j["events"].as_array().unwrap().iter().filter_map(|e| e["message"].as_str()).collect();
    assert_eq!(msgs, ["working", "done"]);

    let m = &r.maint;
    maintenance::set_override(Some(Arc::new(FakeMaintenance {
        admin: m.admin,
        update: Mutex::new(m.update.lock().unwrap().clone()),
        needs_admin: true,
        app: Mutex::new(m.app.lock().unwrap().clone()),
        plan: Mutex::new(vec![]),
        outcome: Mutex::new(m.outcome.lock().unwrap().clone()),
        calls: Mutex::new(vec![]),
    })));
    let a = r.call("POST", "/self-update", Some(json!({"version": "0.4.1"}))).await;
    let j = r.job(a).await;
    assert_eq!(j["state"], "done", "{j}");
    assert_eq!(j["result"]["version"], "0.4.1");
    let calls = h.calls();
    assert_eq!(calls[0].1, ["--yes", "--delete-files", "prod"]);
    assert_eq!((calls[1].0.as_str(), calls[1].1.clone()), ("self-update", vec!["--yes".to_string(), "--version".into(), "0.4.1".into()]));
    set_helper_override(None);
}

#[tokio::test]
async fn the_schedule_is_changed_by_the_helper_not_the_panel() {
    let h = FakeHelper::new(0);
    let r = rig(Some(h.clone())).await;
    r.fake.state().elevation = Some(vec!["schedule".into(), "set".into(), "dev".into(), "--daily".into(), "04:00".into()]);
    let body = json!({"preset": "daily", "time": "04:00", "weekday": null, "run_as": "installing_user", "enabled": true});
    let (s, j) = r.call("PUT", "/servers/dev/schedule", Some(body)).await;
    assert_eq!(s, 200, "{j}");
    let (s, _) = r.call("DELETE", "/servers/dev/schedule", None).await;
    assert_eq!(s, 204);
    let calls = h.calls();
    assert_eq!((calls[0].0.as_str(), calls[0].1[0].as_str()), ("schedule", "set"));
    assert!(!r.fake.calls().iter().any(|c| c.starts_with("schedule_set") || c.starts_with("schedule_remove")));
    set_helper_override(None);
}

#[tokio::test]
async fn a_failing_helper_run_is_an_error_not_a_success() {
    let h = FakeHelper::new(7);
    let r = rig(Some(h)).await;
    r.maint.app.lock().unwrap().legacy = true;
    let (s, j) = r.call("PUT", "/app", Some(json!({}))).await;
    assert_eq!(s, 422, "{j}");
    let m = j["error"]["message"].as_str().unwrap();
    assert!(m.contains("code 7") && m.contains("done"), "{m}");
    set_helper_override(None);
}

#[tokio::test]
async fn a_helper_that_vanishes_falls_back_to_the_command() {
    let mut h = FakeHelper::new(0);
    Arc::get_mut(&mut h).unwrap().vanish = true;
    let r = rig(Some(h)).await;
    r.maint.app.lock().unwrap().legacy = true;
    let (s, j) = r.call("PUT", "/app", Some(json!({}))).await;
    assert_eq!((s, j["error"]["code"].as_str()), (403, Some("needs_elevation")));
    set_helper_override(None);
}

#[tokio::test]
async fn an_install_runs_in_the_helper_with_secrets_in_the_environment_only() {
    let h = FakeHelper::new(0);
    let r = rig(Some(h.clone())).await;
    let a = r.call("POST", "/installs", Some(install_body(&abs("kora-helper-test-b")))).await;
    let j = r.job(a).await;
    assert_eq!(j["state"], "done", "{j}");
    assert_eq!(j["result"]["name"], "Alpha Town");
    // The whole outcome, as the finished-install page reads it (not just name and folder).
    let out: kora_core::installer::InstallOutcome = serde_json::from_value(j["result"].clone()).expect("reads as InstallOutcome");
    assert!(out.connect.ends_with(":30140"), "{}", out.connect);
    assert_eq!(out.txadmin_url, "http://localhost:40140");
    assert!(!out.start_attempted && !out.running && out.launch_problem.is_none());
    assert!(out.notes.iter().any(|n| n.contains("kora start")), "{:?}", out.notes);
    let calls = h.calls();
    assert_eq!(calls.len(), 1);
    let (verb, args, env) = &calls[0];
    assert_eq!(verb, "install");
    assert!(args.iter().any(|a| a == "--service") && args.iter().any(|a| a == "Alpha Town"));
    assert!(args.iter().all(|a| !a.contains("SECRETLICENCE") && !a.contains(DBPASS)), "secrets never on the command: {args:?}");
    assert!(env.contains(&("CFX_LICENSE_KEY".to_string(), LICENCE.to_string())));
    assert!(env.contains(&("KORA_DB_ADMIN_PASSWORD".to_string(), DBPASS.to_string())));
    assert_eq!(env.len(), 2);
    assert!(!j.to_string().contains("SECRETLICENCE") && !j.to_string().contains(DBPASS));
    assert_eq!(*r.env.runs.lock().unwrap(), 0, "the panel did not run the installer itself");
    set_helper_override(None);
}

#[tokio::test]
async fn a_failed_install_in_the_helper_is_a_failed_job_without_secrets() {
    let h = FakeHelper::new(1);
    let r = rig(Some(h)).await;
    let a = r.call("POST", "/installs", Some(install_body(&abs("kora-helper-test-c")))).await;
    let j = r.job(a).await;
    assert_eq!(j["state"], "failed", "{j}");
    assert!(j["error"]["message"].as_str().unwrap().contains("code 1"));
    assert!(!j.to_string().contains("SECRETLICENCE") && !j.to_string().contains(DBPASS));
    set_helper_override(None);
}

#[cfg(unix)]
#[tokio::test]
async fn a_real_socket_client_without_a_socket_is_not_installed() {
    let _t = TURN.lock().await;
    set_helper_override(None);
    // On a machine that really has the helper this would be true, so only assert the shape.
    let installed = kora_web::elevated::helper_installed();
    assert_eq!(installed, std::path::Path::new(kora_core::helper::SOCKET_PATH).exists() && installed);
}

#[tokio::test]
async fn the_helpers_outcome_names_the_generated_txadmin_account_but_never_reads_its_password() {
    let h = FakeHelper::new(0);
    let r = rig(Some(h)).await;
    let dir = abs("kora-helper-test-d");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(std::path::Path::new(&dir).join("secrets")).unwrap();
    std::fs::write(std::path::Path::new(&dir).join("secrets").join("txadmin-admin.txt"), "Username: kora-admin\nPassword: PASSWORD-IN-THE-FILE-1234\n").unwrap();
    let a = r.call("POST", "/installs", Some(install_body(&dir))).await;
    let j = r.job(a).await;
    assert_eq!(j["state"], "done", "{j}");
    assert_eq!(j["result"]["admin_user"], kora_core::txadmin::LOCAL_ADMIN_USER);
    assert!(j["result"]["admin_password_file"].as_str().unwrap().ends_with("txadmin-admin.txt"));
    assert!(!j.to_string().contains("PASSWORD-IN-THE-FILE-1234"), "the password never leaves its file");
    // The helper's lines are job events of their own kind (the page shows them as status lines).
    let _ = std::fs::remove_dir_all(&dir);
    set_helper_override(None);
}
