//! The panel's install with Kora's administrator service, with a fake service (no socket): the administrator
//! steps of an install are one request each with no prompt, the plan says so up front, a missing service is
//! reported at the start with the choices (install it / continue without / the command), and a service that
//! stops midway fails that step, which can be retried or skipped.
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
use kora_core::installer::system_ops::{SharedAdmin, StepResult};
use kora_core::installer::{DbAdmin, InstallOutcome, InstallRequest, ProgressEvent, ProgressSink, StepStatus};
use kora_core::privsvc::conn::ClientError;
use kora_core::privsvc::problems::Availability;
use kora_core::problem::Problem;
use kora_core::space::DiskInfo;
use kora_web::elevated::{set_broker_override, Broker, StepOutcome};
use kora_web::handlers::install as install_seam;
use kora_web::{router, AppState};
use serde_json::{json, Value};
use tower::ServiceExt;

const ADDR: &str = "127.0.0.1:47121";
const TOKEN: &str = "testtoken";
const LICENCE: &str = "cfxk_SECRETLICENCEKEY123456";

static TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

// ---- a fake service --------------------------------------------------------------------------------------

#[derive(Default)]
struct Log {
    calls: Vec<(String, Vec<String>)>,
}

struct FakeBroker {
    availability: Availability,
    /// The service stops answering on its first request (then works again).
    crash_first: Mutex<bool>,
    log: Arc<Mutex<Log>>,
}

impl FakeBroker {
    fn ready() -> Arc<Self> {
        Arc::new(Self { availability: Availability::Ready, crash_first: Mutex::new(false), log: Arc::default() })
    }
    fn missing() -> Arc<Self> {
        Arc::new(Self { availability: Availability::Missing, crash_first: Mutex::new(false), log: Arc::default() })
    }
    fn crashing_once() -> Arc<Self> {
        Arc::new(Self { availability: Availability::Ready, crash_first: Mutex::new(true), log: Arc::default() })
    }
    fn log(&self) -> std::sync::MutexGuard<'_, Log> {
        self.log.lock().unwrap()
    }
}

impl Broker for FakeBroker {
    fn availability(&self) -> Availability {
        self.availability.clone()
    }
    fn handles(&self, verb: &str) -> bool {
        kora_core::privsvc::op::VERBS.contains(&verb)
    }
    fn call(&self, verb: &str, args: &[String], env: &[(String, String)], on_line: &mut dyn FnMut(&str)) -> Result<StepOutcome, ClientError> {
        assert!(env.iter().all(|(_, v)| !v.contains("SECRETLICENCE")), "the licence key is never needed by the service's steps");
        self.log.lock().unwrap().calls.push((verb.into(), args.to_vec()));
        if std::mem::take(&mut *self.crash_first.lock().unwrap()) {
            return Err(ClientError::Broken("Kora's administrator service stopped answering.".into()));
        }
        on_line("working as administrator");
        let data = (verb == "admin-step").then(|| serde_json::to_value(StepResult { detail: format!("did {}", args[1]), ..Default::default() }).unwrap());
        Ok(StepOutcome { ok: true, code: 0, data })
    }
}

// ---- install env: a fake installer that does its administrator steps through the helper ----------

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
    /// (service, firewall, had a helper) per run.
    runs: Mutex<Vec<(bool, bool, bool)>>,
    skipped: Mutex<Vec<String>>,
}

fn ev(step: &str, status: StepStatus, detail: &str) -> ProgressEvent {
    ProgressEvent { step: step.into(), title: step.into(), status, detail: detail.into(), percent: 50 }
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
        let mut v = vec![];
        if req.firewall {
            v.push("open the game port in the firewall".to_string());
        }
        if req.service {
            v.push("register the server as a system service".to_string());
        }
        v
    }
    fn saved_load(&self) -> Option<InstallRequest> {
        None
    }
    fn saved_save(&self, _: &InstallRequest) -> bool {
        true
    }
    fn saved_clear(&self) {}
    fn skip_step(&self, _: &InstallRequest, step: &str) -> Result<(), Problem> {
        self.skipped.lock().unwrap().push(step.into());
        Ok(())
    }
    fn run(&self, _: &InstallRequest, _: &mut dyn ProgressSink) -> Result<InstallOutcome, String> {
        unreachable!("the panel always passes its administrator service (or none)")
    }
    fn run_with_admin(&self, req: &InstallRequest, sink: &mut dyn ProgressSink, admin: Option<SharedAdmin>) -> Result<InstallOutcome, String> {
        self.runs.lock().unwrap().push((req.service, req.firewall, admin.is_some()));
        sink.event(ev("build.config", StepStatus::Done, "written"));
        let skipped = self.skipped.lock().unwrap().clone();
        for (on, step) in [(req.firewall, "system.firewall"), (req.service, "system.service")] {
            if !on || skipped.iter().any(|s| s == step) {
                continue;
            }
            let Some(a) = &admin else {
                sink.event(ev(step, StepStatus::Done, "Skipped (needs admin); see notes"));
                continue;
            };
            sink.event(ev(step, StepStatus::Running, "Working with administrator rights"));
            let args = vec!["run".to_string(), "--step".into(), step.into(), "--dir".into(), req.dir.to_string_lossy().into_owned()];
            match a.0.admin_step(&args, &mut |l| sink.event(ev(step, StepStatus::Running, l))) {
                Ok(o) if o.ok => sink.event(ev(step, StepStatus::Done, "done")),
                Ok(_) => return Err("failed".into()),
                Err(p) => {
                    sink.event(ev(step, StepStatus::Failed, &p.message));
                    return Err(p.into());
                }
            }
        }
        Ok(InstallOutcome { name: req.name.clone(), dir: req.dir.clone(), ..Default::default() })
    }
}

// ---- http -----------------------------------------------------------------------------------------

struct Rig {
    app: axum::Router,
    fake: FakeBackend,
    env: Arc<Env>,
    _turn: tokio::sync::MutexGuard<'static, ()>,
}

impl Drop for Rig {
    fn drop(&mut self) {
        set_broker_override(None);
    }
}

async fn rig(broker: Arc<FakeBroker>) -> Rig {
    let turn = TURN.lock().await;
    set_broker_override(Some(broker));
    let maint = FakeMaintenance::new(false);
    maintenance::set_override(Some(maint.clone()));
    let env = Arc::new(Env::default());
    install_seam::set_env(Some(env.clone()));
    let fake = FakeBackend::sample();
    let f = fake.clone();
    kora_core::installer::registry::use_test_home(); // tests never touch the real Kora folder
    let st = AppState::system(TOKEN.into(), ADDR.parse::<SocketAddr>().unwrap()).with_backend(Arc::new(move || Box::new(f.clone())));
    Rig { app: router(st), fake, env, _turn: turn }
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
    async fn job(&self, accepted: (u16, Value)) -> (String, Value) {
        assert_eq!(accepted.0, 202, "{}", accepted.1);
        let id = accepted.1["job"].as_str().unwrap().to_string();
        let t = Instant::now();
        loop {
            let (_, v) = self.call("GET", &format!("/jobs/{id}"), None).await;
            if v["state"] != "running" {
                return (id, v);
            }
            assert!(t.elapsed() < Duration::from_secs(10), "job did not finish");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

fn install_body(dir: &str) -> Value {
    json!({
        "mode": "production", "name": "Alpha Town", "dir": dir, "hostname": "Alpha RP",
        "license_key": LICENCE, "ports": {"game": 30150, "txadmin": 40150}, "txadmin_public": false,
        "artifact_url": null, "game_build": null, "resources": "all", "db": "use_existing",
        "service": true, "firewall": true, "github_owner": null, "dry_run": false, "db_admin": null, "start": false
    })
}

fn abs(name: &str) -> String {
    std::env::temp_dir().join(name).to_string_lossy().into_owned()
}

fn kinds(j: &Value) -> Vec<String> {
    j["actions"].as_array().map(|a| a.iter().map(|x| x["kind"].as_str().unwrap().to_string()).collect()).unwrap_or_default()
}

// ---- tests ------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_plan_says_the_service_does_the_administrator_parts() {
    let r = rig(FakeBroker::ready()).await;
    let (s, j) = r.call("POST", "/installs/plan", Some(install_body(&abs("kora-svc-plan")))).await;
    assert_eq!(s, 200, "{j}");
    assert_eq!(j["admin_service"], json!("ready"));
    let notes = j["warnings"].to_string();
    assert!(!notes.contains("Windows will ask"), "{notes}");
    assert_eq!(r.env.runs.lock().unwrap().len(), 0, "planning changes nothing");
}

#[tokio::test]
async fn every_administrator_step_of_an_install_is_one_request_to_the_service_with_no_prompt() {
    let b = FakeBroker::ready();
    let r = rig(b.clone()).await;
    let (_, j) = r.job(r.call("POST", "/installs", Some(install_body(&abs("kora-svc-one")))).await).await;
    assert_eq!(j["state"], "done", "{j}");
    assert!(!j["events"].as_array().unwrap().iter().any(|e| e["step"] == "admin"), "no permission step any more");
    // The whole outcome, as the finished-install page reads it: the install ran in the person's own process, so it is the full one.
    let out: kora_core::installer::InstallOutcome = serde_json::from_value(j["result"].clone()).expect("reads as InstallOutcome");
    assert_eq!(out.name, "Alpha Town", "{out:?}");
    assert!(j["events"].to_string().contains("working as administrator"), "the service's lines are part of the same job");
    let log = b.log();
    assert_eq!(log.calls.iter().map(|c| c.1[2].clone()).collect::<Vec<_>>(), ["system.firewall", "system.service"]);
    assert!(log.calls.iter().all(|c| c.0 == "admin-step"));
    assert!(!j.to_string().contains("SECRETLICENCE"));
    assert_eq!(*r.env.runs.lock().unwrap(), [(true, true, true)]);
}

#[tokio::test]
async fn without_the_service_the_install_does_not_start_and_offers_the_choices() {
    let b = FakeBroker::missing();
    let r = rig(b.clone()).await;
    let (s, j) = r.call("POST", "/installs", Some(install_body(&abs("kora-svc-none")))).await;
    assert_eq!((s, j["error"]["code"].as_str()), (403, Some("needs_elevation")), "{j}");
    assert_eq!(kinds(&j["error"]), ["install_admin_service", "continue_without_admin", "show_command"]);
    assert!(j["error"]["message"].as_str().unwrap().contains("open the game port in the firewall and register the server as a system service"), "{j}");
    assert!(!j.to_string().contains("SECRETLICENCE"));
    assert!(r.env.runs.lock().unwrap().is_empty() && b.log().calls.is_empty(), "nothing ran");
}

#[tokio::test]
async fn continue_without_the_administrator_parts_carries_on_as_the_person() {
    let b = FakeBroker::missing();
    let r = rig(b.clone()).await;
    // what the wizard sends for "Continue without the administrator parts": the same answers minus those parts
    let mut body = install_body(&abs("kora-svc-without"));
    body["service"] = json!(false);
    body["firewall"] = json!(false);
    let (_, j) = r.job(r.call("POST", "/installs", Some(body)).await).await;
    assert_eq!(j["state"], "done", "{j}");
    assert_eq!(*r.env.runs.lock().unwrap(), [(false, false, false)], "no service, no firewall, no administrator service");
    assert!(b.log().calls.is_empty());
}

#[tokio::test]
async fn a_service_that_stops_midway_fails_the_step_which_can_be_retried_or_skipped() {
    let b = FakeBroker::crashing_once();
    let r = rig(b.clone()).await;
    let (id, j) = r.job(r.call("POST", "/installs", Some(install_body(&abs("kora-svc-crash")))).await).await;
    assert_eq!(j["state"], "failed", "{j}");
    let e = &j["error"];
    assert_eq!(e["code"], "service.lost", "{e}");
    assert_eq!(e["failed_step"], "system.firewall");
    assert!(kinds(e).contains(&"retry_step".to_string()) && kinds(e).contains(&"skip_step".to_string()), "the firewall may be skipped: {e}");
    assert!(!e.to_string().to_lowercase().contains("windows asks"), "no prompt to mention: {e}");
    let (_, j2) = r.job(r.call("POST", &format!("/installs/{id}/resume"), Some(json!({"skip": "system.firewall"}))).await).await;
    assert_eq!(j2["state"], "done", "{j2}");
    assert_eq!(*r.env.skipped.lock().unwrap(), ["system.firewall"]);
    assert_eq!(b.log().calls.last().unwrap().1[2], "system.service");
}

#[tokio::test]
async fn the_schedule_change_goes_to_the_service_and_never_to_the_panel() {
    let b = FakeBroker::ready();
    let r = rig(b.clone()).await;
    r.fake.state().elevation = Some(vec!["schedule".into(), "off".into(), "dev".into()]);
    let (s, _) = r.call("DELETE", "/servers/dev/schedule", None).await;
    assert_eq!(s, 204);
    assert_eq!(b.log().calls[0], ("schedule".to_string(), vec!["off".to_string(), "dev".into()]));
    assert!(!r.fake.calls().iter().any(|c| c.starts_with("schedule_remove")), "the panel never does it itself");
}
