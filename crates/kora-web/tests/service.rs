//! Kora's administrator service from the panel's side (fake service, no socket): without it every
//! privileged action answers `needs_elevation` with the choices (install the service, continue without,
//! the command); with it the actions it does run as jobs, and the ones that stay commands for an
//! administrator terminal (`uninstall`, `app`) still say so. Unix only (the Windows panel uses its
//! worker until the Windows service lands: tests/admin_worker.rs).
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
use kora_core::api::maintenance;
use kora_core::api::maintenance::fake::FakeMaintenance;
use kora_core::artifacts::ArtifactList;
use kora_core::firewall::ports::PortProber;
use kora_core::installer::{DbAdmin, InstallOutcome, InstallRequest, ProgressSink};
use kora_core::privsvc::conn::ClientError;
use kora_core::privsvc::problems::Availability;
use kora_core::space::DiskInfo;
use kora_web::elevated::{set_broker_override, Broker, StepOutcome};
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

/// The service as the panel sees it: no prompt, a short list of verbs, one connection per request.
struct FakeService {
    helper: Option<Arc<FakeHelper>>,
}

impl Broker for FakeService {
    fn availability(&self) -> Availability {
        if self.helper.as_ref().is_some_and(|h| h.up) {
            Availability::Ready
        } else {
            Availability::Missing
        }
    }
    fn handles(&self, verb: &str) -> bool {
        kora_core::privsvc::op::VERBS.contains(&verb)
    }
    fn call(&self, verb: &str, args: &[String], env: &[(String, String)], on_line: &mut dyn FnMut(&str)) -> Result<StepOutcome, ClientError> {
        let Some(h) = &self.helper else { return Err(ClientError::Unavailable("not installed".into())) };
        if h.vanish {
            return Err(ClientError::Unavailable("gone".into()));
        }
        h.calls.lock().unwrap().push((verb.into(), args.to_vec(), env.to_vec()));
        for l in &h.lines {
            on_line(l);
        }
        Ok(StepOutcome { ok: h.code == 0, code: h.code, data: None })
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
    set_broker_override(Some(Arc::new(FakeService { helper })));
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

// ---- without the service: the choices ---------------------------------------------------------------

fn kinds(e: &Value) -> Vec<String> {
    e["actions"].as_array().unwrap().iter().map(|a| a["kind"].as_str().unwrap().to_string()).collect()
}

#[tokio::test]
async fn without_the_service_everything_answers_needs_elevation_with_the_choices() {
    let r = rig(None).await;
    let (s, j) = r.call("GET", "/system", None).await;
    assert_eq!((s, &j["helper_installed"], &j["admin_service"]["state"], &j["admin_prompt"]), (200, &json!(false), &json!("missing"), &Value::Null), "no prompt exists any more");

    // "Tidy up old shortcuts": the per-user half is done, the old copy needs the command (never the service's job).
    r.maint.app.lock().unwrap().legacy = true;
    let (s, j) = r.call("PUT", "/app", Some(json!({"desktop_shortcut": false}))).await;
    assert_eq!((s, j["error"]["code"].as_str()), (403, Some("needs_elevation")));
    assert_eq!(j["error"]["command"], json!(["kora", "uninstall", "--app", "--yes"]));
    assert!(j["error"]["message"].as_str().unwrap().contains("old copy of Kora"));

    let (s, j) = r.call("POST", "/uninstall", Some(json!({"answers": [], "only": ["prod"]}))).await;
    assert_eq!((s, j["error"]["code"].as_str()), (403, Some("needs_elevation")));
    assert_eq!(j["error"]["command"], json!(["kora", "uninstall", "--yes", "prod"]));

    r.fake.state().elevation = Some(vec!["schedule".into(), "off".into(), "dev".into()]);
    let (s, j) = r.call("DELETE", "/servers/dev/schedule", None).await;
    assert_eq!((s, j["error"]["code"].as_str()), (403, Some("needs_elevation")));
    assert_eq!(j["error"]["command"], json!(["kora", "schedule", "off", "dev"]));
    assert_eq!(kinds(&j["error"]), ["install_admin_service", "show_command"], "no prompt to ask for any more");
    assert!(j["error"]["message"].as_str().unwrap().contains("administrator service"));

    let (s, j) = r.call("POST", "/installs", Some(install_body(&abs("kora-service-test-a")))).await;
    assert_eq!((s, j["error"]["code"].as_str()), (403, Some("needs_elevation")));
    assert_eq!(kinds(&j["error"])[..2], ["install_admin_service", "continue_without_admin"], "{j}");
    assert!(!j.to_string().contains("SECRETLICENCE") && !j.to_string().contains(DBPASS));
    assert_eq!(*r.env.runs.lock().unwrap(), 0);
    assert_eq!(r.maint.calls(), ["app_install shortcut=false force=false dry=false"], "only the per-user shortcuts, which need no rights");
    set_broker_override(None);
}

// ---- with the service --------------------------------------------------------------------------------

#[tokio::test]
async fn system_reports_the_service() {
    let h = FakeHelper::new(0);
    let r = rig(Some(h)).await;
    let j = r.call("GET", "/system", None).await.1;
    assert_eq!((&j["helper_installed"], &j["admin_service"]["state"], &j["admin_prompt"]), (&json!(true), &json!("ready"), &Value::Null), "no prompt exists any more");
    set_broker_override(None);
}

#[tokio::test]
async fn removing_kora_s_old_copy_and_everything_it_set_up_stays_an_administrator_terminal_command() {
    let h = FakeHelper::new(0);
    let r = rig(Some(h.clone())).await;
    r.maint.app.lock().unwrap().legacy = true;
    let (s, j) = r.call("PUT", "/app", Some(json!({"desktop_shortcut": false}))).await;
    assert_eq!((s, j["error"]["code"].as_str()), (403, Some("needs_elevation")), "{j}");
    let (s, j) = r.call("DELETE", "/app", None).await;
    assert_eq!((s, j["error"]["code"].as_str()), (403, Some("needs_elevation")), "{j}");
    let (s, j) = r.call("POST", "/uninstall", Some(json!({"answers": [], "delete_files": true, "only": ["prod"]}))).await;
    assert_eq!((s, j["error"]["code"].as_str()), (403, Some("needs_elevation")), "{j}");
    assert_eq!(j["error"]["command"], json!(["kora", "uninstall", "--yes", "--delete-files", "prod"]), "the whole command, files included");
    assert!(h.calls().is_empty(), "the service is never asked for what is not on its list");
    set_broker_override(None);
}

#[tokio::test]
async fn self_update_is_a_job_that_streams_the_services_lines() {
    let h = FakeHelper::new(0);
    let r = rig(Some(h.clone())).await;
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
    let msgs: Vec<&str> = j["events"].as_array().unwrap().iter().filter_map(|e| e["message"].as_str()).collect();
    assert_eq!(msgs, ["working", "done"]);
    let calls = h.calls();
    assert_eq!((calls[0].0.as_str(), calls[0].1.clone()), ("self-update", vec!["--yes".to_string(), "--version".into(), "0.4.1".into()]));
    set_broker_override(None);
}

#[tokio::test]
async fn the_schedule_is_changed_by_the_service_not_the_panel() {
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
    set_broker_override(None);
}

#[tokio::test]
async fn an_update_task_that_runs_as_the_system_account_stays_a_command() {
    let h = FakeHelper::new(0);
    let r = rig(Some(h.clone())).await;
    r.fake.state().elevation = Some(vec!["schedule".into(), "set".into(), "dev".into(), "--as-system".into()]);
    let body = json!({"preset": "daily", "time": "04:00", "weekday": null, "run_as": "system", "enabled": true});
    let (s, j) = r.call("PUT", "/servers/dev/schedule", Some(body)).await;
    assert_eq!((s, j["error"]["code"].as_str()), (403, Some("needs_elevation")), "{j}");
    assert!(h.calls().is_empty());
    set_broker_override(None);
}

#[tokio::test]
async fn a_failing_service_run_is_an_error_not_a_success() {
    let h = FakeHelper::new(7);
    let r = rig(Some(h)).await;
    r.fake.state().elevation = Some(vec!["schedule".into(), "off".into(), "dev".into()]);
    let (s, j) = r.call("DELETE", "/servers/dev/schedule", None).await;
    assert_eq!(s, 422, "{j}");
    let m = j["error"]["message"].as_str().unwrap();
    assert!(m.contains("code 7") && m.contains("done"), "{m}");
    set_broker_override(None);
}

#[tokio::test]
async fn a_service_that_stops_midway_says_so_and_can_be_tried_again() {
    let mut h = FakeHelper::new(0);
    Arc::get_mut(&mut h).unwrap().vanish = true;
    let r = rig(Some(h)).await;
    r.fake.state().elevation = Some(vec!["schedule".into(), "off".into(), "dev".into()]);
    let (s, j) = r.call("DELETE", "/servers/dev/schedule", None).await;
    assert_eq!((s, j["error"]["code"].as_str()), (422, Some("service.lost")), "{j}");
    assert!(!j["error"]["message"].as_str().unwrap().to_lowercase().contains("windows"));
    set_broker_override(None);
}

#[cfg(unix)]
#[tokio::test]
async fn the_real_socket_client_without_a_service_says_missing() {
    let _t = TURN.lock().await;
    set_broker_override(None);
    // On a machine that really has the service this would be "ready": only assert the shape.
    let state = kora_web::elevated::service_state();
    assert!(["ready", "missing", "not_allowed"].contains(&state["state"].as_str().unwrap()));
}
