//! Tools, the System check and "start the database for me", through the router with fakes (no network,
//! no real git / gh, no real database service, no real helper).
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{header, Request};
use kora_core::api::fake::FakeBackend;
use kora_core::api::install::github::{GithubEnv, GithubUser, LoginFlow, OwnerChoice};
use kora_core::api::toolchain::{AdminState, DbState, ToolchainEnv};
use kora_core::privsvc::conn::ClientError;
use kora_core::privsvc::problems::Availability;
use kora_core::problem::{codes, Action, Problem, Tool};
use kora_core::tools::{Source, ToolStatus};
use kora_web::elevated::{set_broker_override, Broker, StepOutcome};
use kora_web::handlers::{github as gh_seam, toolchain as seam};
use kora_web::{router, AppState};
use serde_json::{json, Value};
use tower::ServiceExt;

const ADDR: &str = "127.0.0.1:47120";
const TOKEN: &str = "testtoken";

/// The seams are process-wide, so the tests take turns.
static TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn status(t: Tool, installed: bool, source: Option<Source>) -> ToolStatus {
    ToolStatus {
        tool: t,
        name: if t == Tool::Git { "git" } else { "gh" },
        label: t.label(),
        installed,
        version: installed.then(|| "2.1.0".to_string()),
        source,
        path: installed.then(|| "/k/tools/x".to_string()),
        broken: false,
        can_install: true,
    }
}

struct Fake {
    installed: Mutex<Vec<Tool>>,
    install_calls: Mutex<Vec<Tool>>,
    install_error: Mutex<Option<Problem>>,
    db: Mutex<DbState>,
    admin: Mutex<AdminState>,
    db_starts: Mutex<u32>,
}

impl Fake {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            installed: Mutex::new(vec![Tool::Git]),
            install_calls: Mutex::new(vec![]),
            install_error: Mutex::new(None),
            db: Mutex::new(DbState { installed: true, running: false, name: Some("mariadb".into()), version: None, unknown: false }),
            admin: Mutex::new(AdminState { elevated: true, helper: false, service: false }),
            db_starts: Mutex::new(0),
        })
    }
}

impl ToolchainEnv for Fake {
    fn tools(&self) -> Vec<ToolStatus> {
        let have = self.installed.lock().unwrap().clone();
        vec![status(Tool::Git, have.contains(&Tool::Git), Some(Source::System)), status(Tool::Gh, have.contains(&Tool::Gh), Some(Source::Private))]
    }
    fn install(&self, tool: Tool, say: &mut dyn FnMut(&str, f32)) -> Result<ToolStatus, String> {
        self.install_calls.lock().unwrap().push(tool);
        say("Downloading the GitHub CLI", 0.5);
        if let Some(p) = self.install_error.lock().unwrap().clone() {
            return Err(p.into());
        }
        self.installed.lock().unwrap().push(tool);
        Ok(status(tool, true, Some(Source::Private)))
    }
    fn database(&self) -> DbState {
        self.db.lock().unwrap().clone()
    }
    fn start_database(&self, say: &mut dyn FnMut(&str)) -> Result<String, String> {
        *self.db_starts.lock().unwrap() += 1;
        say("Starting the database server mariadb");
        self.db.lock().unwrap().running = true;
        Ok("The database server mariadb is running.".into())
    }
    fn admin(&self) -> AdminState {
        *self.admin.lock().unwrap()
    }
}

struct Gh(GithubUser);
impl GithubEnv for Gh {
    fn auth(&self) -> GithubUser {
        self.0.clone()
    }
    fn owners(&self) -> Result<Vec<OwnerChoice>, String> {
        Ok(vec![])
    }
    fn begin_login(&self) -> Result<LoginFlow, String> {
        Err("no".into())
    }
    fn resolve_owner(&self, _: Option<&str>) -> Result<String, String> {
        Err("no".into())
    }
}

struct Rig {
    app: axum::Router,
    fake: Arc<Fake>,
    _turn: tokio::sync::MutexGuard<'static, ()>,
}

async fn rig(github: GithubUser) -> Rig {
    let turn = TURN.lock().await;
    let fake = Fake::new();
    seam::set_env(Some(fake.clone()));
    gh_seam::set_env(Some(Arc::new(Gh(github))));
    set_broker_override(Some(broker(false)));
    let fb = FakeBackend::sample();
    kora_core::installer::registry::use_test_home(); // tests never touch the real Kora folder
    let st = AppState::system(TOKEN.into(), ADDR.parse::<SocketAddr>().unwrap()).with_backend(Arc::new(move || Box::new(fb.clone())));
    Rig { app: router(st), fake, _turn: turn }
}

impl Drop for Rig {
    fn drop(&mut self) {
        seam::set_env(None);
        gh_seam::set_env(None);
        set_broker_override(None);
    }
}

impl Rig {
    async fn call(&self, method: &str, uri: &str) -> (u16, Value) {
        let r = self
            .app
            .clone()
            .oneshot(Request::builder().method(method).uri(format!("/api/v1{uri}")).header(header::HOST, ADDR).header(header::AUTHORIZATION, format!("Bearer {TOKEN}")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let s = r.status().as_u16();
        let bytes = axum::body::to_bytes(r.into_body(), 1 << 22).await.unwrap();
        (s, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }
    async fn job(&self, accepted: (u16, Value)) -> Value {
        assert_eq!(accepted.0, 202, "{}", accepted.1);
        let id = accepted.1["job"].as_str().unwrap().to_string();
        let t = Instant::now();
        loop {
            let (_, v) = self.call("GET", &format!("/jobs/{id}")).await;
            if v["state"] != "running" {
                return v;
            }
            assert!(t.elapsed() < Duration::from_secs(10), "job did not finish");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

#[tokio::test]
async fn tools_status_lists_git_and_gh_with_where_they_come_from() {
    let r = rig(GithubUser::ready("sam")).await;
    let (s, v) = r.call("GET", "/tools").await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["tools"][0]["name"], "git");
    assert_eq!((v["tools"][0]["installed"].clone(), v["tools"][0]["source"].clone()), (json!(true), json!("system")));
    assert_eq!((v["tools"][1]["installed"].clone(), v["tools"][1]["can_install"].clone()), (json!(false), json!(true)));
}

#[tokio::test]
async fn the_system_check_lists_every_line_with_its_fix() {
    let r = rig(GithubUser::gh_missing()).await;
    let (s, v) = r.call("GET", "/system/check").await;
    assert_eq!(s, 200, "{v}");
    let ids: Vec<&str> = v["items"].as_array().unwrap().iter().map(|i| i["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["git", "gh", "github", "database", "admin", "service"]);
    assert_eq!(v["ok"], false);
    let gh = &v["items"][1];
    assert_eq!((gh["state"].clone(), gh["problem"]["actions"][0].clone()), (json!("problem"), json!({"kind": "install_tool", "tool": "gh"})));
    // The stopped database offers "start it for me".
    let db = &v["items"][3];
    assert_eq!(db["problem"]["code"], "install.db_stopped");
    assert_eq!(db["problem"]["actions"], json!([{"kind": "start_database"}]));
    assert_eq!(v["items"][0]["state"], "ok");
}

#[tokio::test]
async fn the_system_check_is_ok_when_everything_is_fine() {
    let r = rig(GithubUser::ready("sam")).await;
    r.fake.installed.lock().unwrap().push(Tool::Gh);
    r.fake.db.lock().unwrap().running = true;
    let (_, v) = r.call("GET", "/system/check").await;
    assert_eq!(v["ok"], true, "{v}");
    assert!(v["items"].as_array().unwrap().iter().all(|i| i["problem"].is_null()));
}

#[tokio::test]
async fn installing_a_tool_is_a_job_with_progress_and_a_result() {
    let r = rig(GithubUser::gh_missing()).await;
    let accepted = r.call("POST", "/tools/gh/install").await;
    let job = r.job(accepted).await;
    assert_eq!(job["state"], "done", "{job}");
    assert_eq!(job["result"], json!({"tool": "gh", "version": "2.1.0", "path": "/k/tools/x", "source": "private"}));
    let ev = job["events"].as_array().unwrap();
    assert!(ev.iter().any(|e| e["type"] == "progress" && e["fraction"] == 0.5 && e["message"].as_str().unwrap().contains("Downloading")), "{ev:?}");
    assert_eq!(*r.fake.install_calls.lock().unwrap(), vec![Tool::Gh]);
    // It is there afterwards.
    let (_, v) = r.call("GET", "/tools").await;
    assert_eq!(v["tools"][1]["installed"], true);
}

#[tokio::test]
async fn a_failed_download_is_a_failed_job_with_retry_and_a_link() {
    let r = rig(GithubUser::gh_missing()).await;
    *r.fake.install_error.lock().unwrap() = Some(
        Problem::new(codes::TOOLS_DOWNLOAD_FAILED, "Kora could not download the GitHub CLI (gh). Check your internet connection and try again.")
            .with_action(Action::Retry)
            .with_action(Action::open_link("https://cli.github.com")),
    );
    let job = r.job(r.call("POST", "/tools/gh/install").await).await;
    assert_eq!(job["state"], "failed");
    assert_eq!(job["error"]["code"], "tools.download_failed");
    assert_eq!(job["error"]["actions"][0], json!({"kind": "retry"}));
    assert!(job["error"]["message"].as_str().unwrap().contains("internet connection"));
}

#[tokio::test]
async fn only_git_and_gh_can_be_installed() {
    let r = rig(GithubUser::ready("sam")).await;
    let (s, _) = r.call("POST", "/tools/node/install").await;
    assert_eq!(s, 404);
    assert!(r.fake.install_calls.lock().unwrap().is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn git_on_linux_needs_root_so_without_the_helper_it_is_the_command_to_run() {
    let r = rig(GithubUser::ready("sam")).await;
    r.fake.installed.lock().unwrap().clear();
    r.fake.admin.lock().unwrap().elevated = false;
    let (s, v) = r.call("POST", "/tools/git/install").await;
    assert_eq!(s, 403, "{v}");
    assert_eq!(v["error"]["code"], "needs_elevation");
    assert!(v["error"]["actions"].as_array().unwrap().contains(&json!({"kind": "show_command", "command": ["kora", "tools", "install", "git"], "elevated": true})), "{v}");
    assert!(r.fake.install_calls.lock().unwrap().is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn git_on_linux_goes_through_the_service_when_it_is_installed() {
    let r = rig(GithubUser::ready("sam")).await;
    let h = broker(true);
    set_broker_override(Some(h.clone()));
    r.fake.admin.lock().unwrap().elevated = false;
    let job = r.job(r.call("POST", "/tools/git/install").await).await;
    set_broker_override(None);
    assert_eq!(job["state"], "done", "{job}");
    assert_eq!(*h.calls.lock().unwrap(), vec![("tools".to_string(), vec!["install".to_string(), "git".to_string()])]);
    assert!(r.fake.install_calls.lock().unwrap().is_empty(), "the service did it, not this process");
}

#[tokio::test]
async fn starting_the_database_is_a_job_when_kora_may_do_it() {
    let r = rig(GithubUser::ready("sam")).await;
    let job = r.job(r.call("POST", "/system/database/start").await).await;
    assert_eq!(job["state"], "done", "{job}");
    assert_eq!(job["result"]["message"], "The database server mariadb is running.");
    assert_eq!(*r.fake.db_starts.lock().unwrap(), 1);
    // Now it runs: a second press is a plain "already running".
    let (s, v) = r.call("POST", "/system/database/start").await;
    assert_eq!((s, v["error"]["code"].as_str()), (409, Some("conflict")), "{v}");
}

#[tokio::test]
async fn starting_a_database_that_is_not_installed_says_so() {
    let r = rig(GithubUser::ready("sam")).await;
    *r.fake.db.lock().unwrap() = DbState::default();
    let (s, v) = r.call("POST", "/system/database/start").await;
    assert_eq!((s, v["error"]["code"].as_str()), (422, Some("install.db_missing")), "{v}");
}

type Calls = Arc<Mutex<Vec<(String, Vec<String>)>>>;

/// The administrator service stand-in: `supported` = it is there for this account.
struct FakeBroker {
    supported: bool,
    calls: Calls,
}
impl Broker for FakeBroker {
    fn availability(&self) -> Availability {
        if self.supported {
            Availability::Ready
        } else {
            Availability::Missing
        }
    }
    fn handles(&self, verb: &str) -> bool {
        kora_core::privsvc::op::VERBS.contains(&verb)
    }
    fn call(&self, verb: &str, args: &[String], _: &[(String, String)], on_line: &mut dyn FnMut(&str)) -> Result<StepOutcome, ClientError> {
        self.calls.lock().unwrap().push((verb.into(), args.to_vec()));
        on_line("The database server mariadb is running.");
        Ok(StepOutcome { ok: true, code: 0, data: None })
    }
}
fn broker(supported: bool) -> Arc<FakeBroker> {
    Arc::new(FakeBroker { supported, calls: Arc::default() })
}

#[tokio::test]
async fn without_administrator_rights_the_start_is_the_elevation_problem() {
    let r = rig(GithubUser::ready("sam")).await;
    // No way to ask (Linux without the helper, or a panel that cannot ask Windows itself).
    set_broker_override(Some(broker(false)));
    r.fake.admin.lock().unwrap().elevated = false;
    let (s, v) = r.call("POST", "/system/database/start").await;
    set_broker_override(None);
    assert_eq!((s, v["error"]["code"].as_str()), (403, Some("needs_elevation")), "{v}");
    assert_eq!(v["error"]["actions"][0], json!({"kind": "install_admin_service"}));
    let shown = v["error"]["actions"].as_array().unwrap().iter().find(|a| a["kind"] == "show_command").unwrap();
    assert_eq!(shown["command"], json!(["kora", "check", "--start-database"]), "{shown}");
    assert_eq!(*r.fake.db_starts.lock().unwrap(), 0, "nothing was started");
}

#[tokio::test]
async fn the_database_start_goes_through_the_service_when_it_is_installed() {
    let r = rig(GithubUser::ready("sam")).await;
    let h = broker(true);
    set_broker_override(Some(h.clone()));
    r.fake.admin.lock().unwrap().elevated = false;
    let job = r.job(r.call("POST", "/system/database/start").await).await;
    set_broker_override(None);
    assert_eq!(job["state"], "done", "{job}");
    assert_eq!(*h.calls.lock().unwrap(), vec![("check".to_string(), vec!["--start-database".to_string()])]);
}
