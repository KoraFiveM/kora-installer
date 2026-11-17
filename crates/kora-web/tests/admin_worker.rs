//! The Windows administrator worker from the panel's side, with a fake broker (no prompt, no
//! Windows): one prompt per install, its steps streamed into the same job, and every way a prompt
//! can fail (declined, nobody answered, the worker stopped) ending in choices, not a dead end.
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
use kora_core::api::maintenance;
use kora_core::artifacts::ArtifactList;
use kora_core::firewall::ports::PortProber;
use kora_core::helper::client::HelperError;
use kora_core::helper::worker::{OpenError, StepOutcome};
use kora_core::installer::system_ops::{SharedAdmin, StepResult};
use kora_core::installer::{DbAdmin, InstallOutcome, InstallRequest, ProgressEvent, ProgressSink, StepStatus};
use kora_core::problem::Problem;
use kora_core::space::DiskInfo;
use kora_web::elevated::{set_broker_override, Broker, Channel};
use kora_web::handlers::install as install_seam;
use kora_web::{router, AppState};
use serde_json::{json, Value};
use tower::ServiceExt;

const ADDR: &str = "127.0.0.1:47121";
const TOKEN: &str = "testtoken";
const LICENCE: &str = "cfxk_SECRETLICENCEKEY123456";

static TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

// ---- a fake broker (instead of the Windows prompt) ------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
enum Answer {
    Yes,
    No,
    Nobody,
    /// Yes, but the worker dies on its first request.
    Crash,
}

#[derive(Default)]
struct Log {
    opens: usize,
    calls: Vec<(String, Vec<String>)>,
    finished: usize,
}

struct FakeBroker {
    answers: Mutex<Vec<Answer>>,
    log: Arc<Mutex<Log>>,
}

impl FakeBroker {
    fn new(answers: &[Answer]) -> Arc<Self> {
        Arc::new(Self { answers: Mutex::new(answers.to_vec()), log: Arc::default() })
    }
    fn log(&self) -> std::sync::MutexGuard<'_, Log> {
        self.log.lock().unwrap()
    }
}

impl Broker for FakeBroker {
    fn supported(&self) -> bool {
        true
    }
    fn open(&self, kora_home: Option<&str>) -> Result<Box<dyn Channel>, OpenError> {
        assert!(kora_home.is_some_and(|h| !h.is_empty()), "the worker is told the person's Kora folder");
        self.log.lock().unwrap().opens += 1;
        let mut a = self.answers.lock().unwrap();
        let next = if a.len() > 1 { a.remove(0) } else { a[0] };
        match next {
            Answer::Yes => Ok(Box::new(FakeChannel { log: self.log.clone(), crash: false })),
            Answer::Crash => Ok(Box::new(FakeChannel { log: self.log.clone(), crash: true })),
            Answer::No => Err(OpenError::Declined),
            Answer::Nobody => Err(OpenError::TimedOut),
        }
    }
}

struct FakeChannel {
    log: Arc<Mutex<Log>>,
    crash: bool,
}

impl Channel for FakeChannel {
    fn run(&mut self, verb: &str, args: &[String], env: &[(String, String)], on_line: &mut dyn FnMut(&str)) -> Result<StepOutcome, HelperError> {
        assert!(env.iter().all(|(_, v)| !v.contains("SECRETLICENCE")), "the licence key is never needed by the worker");
        self.log.lock().unwrap().calls.push((verb.into(), args.to_vec()));
        if self.crash {
            return Err(HelperError::Protocol("Kora's administrator helper stopped answering (the connection closed).".into()));
        }
        on_line("working as administrator");
        let data = (verb == "admin-step").then(|| serde_json::to_value(StepResult { detail: format!("did {}", args[2]), ..Default::default() }).unwrap());
        Ok(StepOutcome { ok: true, code: 0, data })
    }
    fn finish(self: Box<Self>) {
        self.log.lock().unwrap().finished += 1;
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
            v.push("register the server as a Windows service".to_string());
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
        unreachable!("the panel always passes its administrator helper (or none)")
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
    maint: Arc<FakeMaintenance>,
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
    let st = AppState::system(TOKEN.into(), ADDR.parse::<SocketAddr>().unwrap()).with_backend(Arc::new(move || Box::new(f.clone())));
    Rig { app: router(st), fake, maint, env, _turn: turn }
}

impl Rig {
    async fn call_h(&self, method: &str, uri: &str, body: Option<Value>, ask: bool) -> (u16, Value) {
        let mut b = Request::builder().method(method).uri(format!("/api/v1{uri}")).header(header::HOST, ADDR).header(header::AUTHORIZATION, format!("Bearer {TOKEN}"));
        if ask {
            b = b.header("x-kora-admin", "ask");
        }
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
    async fn call(&self, method: &str, uri: &str, body: Option<Value>) -> (u16, Value) {
        self.call_h(method, uri, body, false).await
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

fn admin_details(job: &Value) -> Vec<String> {
    job["events"].as_array().unwrap().iter().filter(|e| e["step"] == "admin").map(|e| format!("{}: {}", e["status"].as_str().unwrap(), e["detail"].as_str().unwrap())).collect()
}

// ---- tests ------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_panel_says_windows_will_ask_and_the_plan_says_it_up_front() {
    let r = rig(FakeBroker::new(&[Answer::Yes])).await;
    let (s, j) = r.call("GET", "/system", None).await;
    assert_eq!((s, &j["admin_prompt"]), (200, &json!(true)), "{j}");
    let (s, j) = r.call("POST", "/installs/plan", Some(install_body(&abs("kora-uac-plan")))).await;
    assert_eq!(s, 200, "{j}");
    assert_eq!(j["admin_prompt"], json!(true));
    let notes = j["warnings"].to_string();
    assert!(notes.contains("Windows will ask you once"), "{notes}");
    assert!(!notes.contains("show the command"), "{notes}");
    assert_eq!(r.env.runs.lock().unwrap().len(), 0, "planning changes nothing");
}

#[tokio::test]
async fn one_prompt_covers_every_administrator_step_of_an_install() {
    let b = FakeBroker::new(&[Answer::Yes]);
    let r = rig(b.clone()).await;
    let (_, j) = r.job(r.call("POST", "/installs", Some(install_body(&abs("kora-uac-one")))).await).await;
    assert_eq!(j["state"], "done", "{j}");
    let admin = admin_details(&j);
    assert!(admin[0].starts_with("running: Windows is asking for permission"), "{admin:?}");
    assert!(admin[1].starts_with("done: Permission given"), "{admin:?}");
    // The worker's lines are part of the same job.
    assert!(j["events"].to_string().contains("working as administrator"));
    let log = b.log();
    assert_eq!(log.opens, 1, "one prompt");
    assert_eq!(log.calls.iter().map(|c| c.1[2].clone()).collect::<Vec<_>>(), ["system.firewall", "system.service"]);
    assert!(log.calls.iter().all(|c| c.0 == "admin-step"));
    assert_eq!(log.finished, 1, "the worker is told to exit when the install is done");
    assert!(!j.to_string().contains("SECRETLICENCE"));
    assert_eq!(*r.env.runs.lock().unwrap(), [(true, true, true)]);
}

#[tokio::test]
async fn a_declined_prompt_offers_ask_again_continue_without_and_the_command() {
    let b = FakeBroker::new(&[Answer::No, Answer::Yes]);
    let r = rig(b.clone()).await;
    let (id, j) = r.job(r.call("POST", "/installs", Some(install_body(&abs("kora-uac-no")))).await).await;
    assert_eq!(j["state"], "failed", "{j}");
    let e = &j["error"];
    assert_eq!(e["code"], "admin.declined", "{e}");
    assert_eq!(e["resume"], json!(true));
    assert_eq!(kinds(e), ["request_admin", "continue_without_admin", "show_command", "change_answers", "open_log"]);
    assert!(e["message"].as_str().unwrap().contains("open the game port in the firewall and register the server as a Windows service"), "{e}");
    assert!(!e.to_string().contains("SECRETLICENCE"));
    assert!(r.env.runs.lock().unwrap().is_empty(), "nothing ran");
    // "Ask again": a new prompt; this time Yes.
    let (_, j2) = r.job(r.call_h("POST", &format!("/installs/{id}/resume"), Some(json!({})), true).await).await;
    assert_eq!(j2["state"], "done", "{j2}");
    assert_eq!(b.log().opens, 2);
}

#[tokio::test]
async fn continue_without_the_administrator_parts_carries_on_as_the_person() {
    let b = FakeBroker::new(&[Answer::No]);
    let r = rig(b.clone()).await;
    let (id, j) = r.job(r.call("POST", "/installs", Some(install_body(&abs("kora-uac-without")))).await).await;
    assert_eq!(j["error"]["code"], "admin.declined");
    let (_, j2) = r.job(r.call("POST", &format!("/installs/{id}/resume"), Some(json!({"without_admin": true}))).await).await;
    assert_eq!(j2["state"], "done", "{j2}");
    assert_eq!(*r.env.runs.lock().unwrap(), [(false, false, false)], "no service, no firewall, no helper");
    assert_eq!(b.log().opens, 1, "not asked again");
    // A wrong body is refused, not guessed.
    let (s, _) = r.call("POST", "/installs/nope/resume", Some(json!({"without_admin": "yes"}))).await;
    assert_eq!(s, 400);
}

#[tokio::test]
async fn nobody_answering_is_a_timeout_with_the_same_choices() {
    let r = rig(FakeBroker::new(&[Answer::Nobody])).await;
    let (_, j) = r.job(r.call("POST", "/installs", Some(install_body(&abs("kora-uac-nobody")))).await).await;
    let e = &j["error"];
    assert_eq!(e["code"], "admin.timed_out", "{e}");
    assert!(e["details"].as_str().unwrap().contains("taskbar"));
    assert_eq!(&kinds(e)[..3], ["request_admin", "continue_without_admin", "show_command"]);
}

#[tokio::test]
async fn a_worker_that_stops_midway_fails_the_step_which_can_be_retried_or_skipped() {
    let b = FakeBroker::new(&[Answer::Crash, Answer::Yes]);
    let r = rig(b.clone()).await;
    let (id, j) = r.job(r.call("POST", "/installs", Some(install_body(&abs("kora-uac-crash")))).await).await;
    assert_eq!(j["state"], "failed", "{j}");
    let e = &j["error"];
    assert_eq!(e["code"], "admin.lost", "{e}");
    assert_eq!(e["failed_step"], "system.firewall");
    assert!(kinds(e).contains(&"retry_step".to_string()) && kinds(e).contains(&"skip_step".to_string()), "the firewall may be skipped: {e}");
    // Skip it (the person's own plan state: allowed, unlike a Linux helper install).
    let (_, j2) = r.job(r.call("POST", &format!("/installs/{id}/resume"), Some(json!({"skip": "system.firewall"}))).await).await;
    assert_eq!(j2["state"], "done", "{j2}");
    assert_eq!(*r.env.skipped.lock().unwrap(), ["system.firewall"]);
    let log = b.log();
    assert_eq!(log.opens, 2, "the retry asked again (lazily, when the service step needed it)");
    assert_eq!(log.calls.last().unwrap().1[2], "system.service");
}

#[tokio::test]
async fn the_schedule_change_asks_once_and_a_no_is_a_guided_answer() {
    let b = FakeBroker::new(&[Answer::Yes, Answer::No]);
    let r = rig(b.clone()).await;
    r.fake.state().elevation = Some(vec!["schedule".into(), "off".into(), "dev".into()]);
    let (s, _) = r.call("DELETE", "/servers/dev/schedule", None).await;
    assert_eq!(s, 204);
    assert_eq!(b.log().calls[0], ("schedule".to_string(), vec!["off".to_string(), "dev".into()]));
    assert_eq!(b.log().finished, 1, "one prompt per action, and the worker exits after it");
    let (s, j) = r.call("DELETE", "/servers/dev/schedule", None).await;
    assert_eq!((s, j["error"]["code"].as_str()), (403, Some("admin.declined")), "{j}");
    assert_eq!(kinds(&j["error"]), ["request_admin", "show_command"], "nothing to continue without here");
    assert!(!r.fake.calls().iter().any(|c| c.starts_with("schedule_remove")), "the panel never does it itself");
}

#[tokio::test]
async fn tidy_up_and_uninstall_use_the_worker() {
    let b = FakeBroker::new(&[Answer::Yes]);
    let r = rig(b.clone()).await;
    r.maint.app.lock().unwrap().legacy = true;
    let (s, j) = r.call("PUT", "/app", Some(json!({"desktop_shortcut": false}))).await;
    assert_eq!(s, 200, "{j}");
    let a = r.call("POST", "/uninstall", Some(json!({"answers": [], "only": ["prod"]}))).await;
    let (_, j) = r.job(a).await;
    assert_eq!(j["state"], "done", "{j}");
    let calls = b.log().calls.clone();
    assert_eq!(calls[0], ("app".to_string(), vec!["uninstall".to_string(), "--yes".into()]));
    assert_eq!(calls[1], ("uninstall".to_string(), vec!["--yes".to_string(), "prod".into()]));
}
