//! The install wizard: defaults, checks, artifacts, plan, the install job and its log.
//!
//! Each `pub async fn` is mounted by `crate::routes` for the [`Op`] of the same name; the HTTP
//! method and path come from `Op::route`, so only the handler's extractors live here. The logic
//! is in `kora_core::api::install`; everything that touches the machine goes through
//! [`InstallEnv`], which tests replace with [`set_env`].
#![allow(clippy::unused_async)]

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::{Extension, Json};
use kora_core::api::install::{self as core, InstallEnv, SystemInstallEnv, WireDbAdmin, WireInstall};
use kora_core::api::Op;
use kora_core::installer::paused::PausedStore;
use kora_core::installer::{slug, InstallRequest, Mode};
use kora_core::keystore::KeyStore;
use kora_core::problem::{codes, Action, Problem};
use kora_core::profile::ProfileStore;
use serde_json::{json, Value};

use crate::elevated::{AdminAsk, WorkerAdmin};
use crate::remote::Caller;
use crate::{blocking, ApiError, AppState};
use kora_core::helper::worker;
use kora_core::installer::system_ops::SharedAdmin;

static ENV: RwLock<Option<Arc<dyn InstallEnv>>> = RwLock::new(None);

/// Replace the machine seam (tests). `None` goes back to the real machine.
pub fn set_env(e: Option<Arc<dyn InstallEnv>>) {
    *ENV.write().unwrap_or_else(|p| p.into_inner()) = e;
}

/// The machine seam in use.
pub fn env() -> Arc<dyn InstallEnv> {
    ENV.read().unwrap_or_else(|p| p.into_inner()).clone().unwrap_or_else(|| Arc::new(SystemInstallEnv))
}

fn parse<T: for<'de> serde::Deserialize<'de>>(v: Value) -> Result<T, ApiError> {
    serde_json::from_value(v).map_err(|_| ApiError::bad_request("That request was not understood. Reload the page and try again."))
}

fn port_of(v: &Value, key: &str, label: &str) -> Result<u16, ApiError> {
    v.get(key).and_then(Value::as_u64).and_then(|n| u16::try_from(n).ok()).ok_or_else(|| ApiError::bad_request(format!("The {label} port must be a number between 1024 and 65535.")))
}

/// `GET /artifacts` → `{recommended, items: [{build, url}]}` (builds known to be broken are left out).
pub async fn artifacts_list() -> Result<Json<Value>, ApiError> {
    let e = env();
    let list = blocking(move || {
        e.artifacts().map_err(|m| ApiError::new(StatusCode::BAD_GATEWAY, "internal", format!("Could not read the list of FiveM server builds ({m}). Check your internet connection and try again.")))
    })
    .await?;
    Ok(Json(core::artifacts_json(&list)))
}

/// `GET /installs/defaults` → `{name, dir, hostname, ports, saved}`.
pub async fn install_defaults(State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    let e = env();
    let v = blocking(move || {
        let servers = st.backend().list().map_err(ApiError::internal)?;
        let mut v = core::defaults(&*e, &servers);
        // What Kora remembers (never secret): the wizard pre-fills these, still editable.
        let p = ProfileStore::default().load();
        v["profile"] = serde_json::to_value(super::keys::profile_view(&p)).unwrap_or(Value::Null);
        // Saved answers that name a saved key which is gone: say so instead of silently picking nothing.
        if let Some(n) = v["saved"]["license_key_name"].as_str().map(String::from) {
            if !KeyStore::default().exists(&n).unwrap_or(true) {
                v["saved"]["license_key_name"] = Value::Null;
                v["saved_key_missing"] = json!(n);
            }
        }
        // Installs that stopped and can carry on (also after the panel was restarted).
        v["resumable"] = json!(resumable());
        Ok(v)
    })
    .await?;
    Ok(Json(v))
}

/// `DELETE /installs/saved`: forget the answers of an install that stopped part-way.
pub async fn install_saved_clear() -> Result<StatusCode, ApiError> {
    let e = env();
    blocking(move || {
        e.saved_clear();
        Ok(())
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /checks/ports` `{game, txadmin}` → `{game: {port, ok, text, suggestion}, txadmin: {...}}`.
pub async fn check_ports(State(st): State<AppState>, Json(body): Json<Value>) -> Result<Json<Value>, ApiError> {
    let (game, tx) = (port_of(&body, "game", "game")?, port_of(&body, "txadmin", "txAdmin")?);
    let e = env();
    let v = blocking(move || {
        let servers = st.backend().list().map_err(ApiError::internal)?;
        let (g, t) = core::port_lines(game, tx, &servers, e.prober());
        Ok(json!({"game": g.json(), "txadmin": t.json()}))
    })
    .await?;
    Ok(Json(v))
}

/// `POST /checks/disk` `{dir}` → `{ok, free_bytes, needed_bytes, message}`.
pub async fn check_disk(Json(body): Json<Value>) -> Result<Json<Value>, ApiError> {
    let dir = body.get("dir").and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string();
    if dir.is_empty() {
        return Err(ApiError::bad_request("Choose a folder for the server first."));
    }
    let e = env();
    let v = blocking(move || Ok(core::disk_check(e.disk(), std::path::Path::new(&dir), None).json())).await?;
    Ok(Json(v))
}

/// `POST /checks/db-login` `{admin: {user, password, host, port} | null}` → `{ok, message}`.
/// The password is only used for the test; it is never stored, logged or returned.
pub async fn check_db_login(Json(body): Json<Value>) -> Result<Json<Value>, ApiError> {
    let admin = match body.get("admin") {
        None | Some(Value::Null) => None,
        Some(a) => Some(parse::<WireDbAdmin>(a.clone())?.to_admin()),
    };
    if let Some(a) = &admin {
        kora_core::db::validate_host(&a.host).map_err(|_| ApiError::bad_request("That database server address does not look right."))?;
    }
    let e = env();
    let automatic = admin.is_none();
    let local = admin.as_ref().is_none_or(|a| matches!(a.host.as_str(), "127.0.0.1" | "localhost" | "::1"));
    let v = blocking(move || {
        Ok(match e.db_login(admin.as_ref()) {
            Ok(()) => json!({"ok": true, "message": if automatic { "Kora can log in to the database automatically." } else { "Logged in to the database." }}),
            Err(m) => {
                // A failed login is often just "the database is installed but not running": say so, with the
                // "start it for me" button (`problem`, shown by the wizard).
                let db = if local { super::toolchain::env().database() } else { Default::default() };
                let problem = (db.installed && !db.running).then(|| kora_core::problem::known::db_stopped(db.name.as_deref().unwrap_or("the database")).to_json());
                json!({"ok": false, "message": m, "problem": problem})
            }
        })
    })
    .await?;
    Ok(Json(v))
}

/// `POST /checks/cfx` `{username}` → `{ok, id, username, identifier, message}`. Asks forum.cfx.re which
/// account the name belongs to, so the install can make that account the txAdmin admin. Only the name is sent.
pub async fn check_cfx(Json(body): Json<Value>) -> Result<Json<Value>, ApiError> {
    let name = body.get("username").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if name.is_empty() {
        return Err(ApiError::bad_request("Type your Cfx.re username."));
    }
    let e = env();
    let v = blocking(move || {
        Ok(match e.cfx_lookup(&name) {
            Ok(a) => {
                ProfileStore::default().remember_cfx(&a.username, &a.identifier());
                json!({
                    "ok": true, "id": a.id, "username": a.username, "identifier": a.identifier(),
                    "message": format!("Found {} (fivem:{}). This account will be the txAdmin admin.", a.username, a.id),
                })
            }
            Err(m) => json!({"ok": false, "id": null, "username": null, "identifier": null, "message": m}),
        })
    })
    .await?;
    Ok(Json(v))
}

/// `POST /installs/plan`: the same checks as starting, nothing changed, no secrets in the answer.
/// → `{summary, steps, needs_admin, elevated, disk, warnings, notes}`.
pub async fn install_plan(State(st): State<AppState>, Json(body): Json<Value>) -> Result<Json<Value>, ApiError> {
    let wire: WireInstall = parse(body)?;
    let e = env();
    let v = blocking(move || {
        let req = core::build_request(&wire).map_err(ApiError::bad_request)?;
        if let Some(n) = &req.license_key_name {
            // Fail now, at the licence page, rather than after the person has answered everything.
            if !KeyStore::default().exists(n).map_err(super::keys::api)? {
                return Err(super::keys::api(KeyStore::default().get(n).unwrap_err()));
            }
        }
        let servers = st.backend().list().map_err(ApiError::internal)?;
        Ok(with_admin_prompt(core::plan(&*e, &req, &servers), crate::elevated::worker_supported()))
    })
    .await?;
    Ok(Json(v))
}

/// What Windows will do, said up front: `admin_prompt` is true when this install needs
/// administrator rights and Windows will ask for them once, when it starts (the panel's worker).
pub const ADMIN_PROMPT_NOTE: &str = "Windows will ask you once for permission when the install starts: choose Yes. Kora does everything else as you.";

fn with_admin_prompt(mut v: Value, worker: bool) -> Value {
    let needs = v["needs_admin"].as_array().is_some_and(|a| !a.is_empty());
    let prompt = worker && needs && v["elevated"] != json!(true);
    v["admin_prompt"] = json!(prompt);
    if prompt {
        for key in ["warnings", "notes"] {
            if let Some(list) = v[key].as_array_mut() {
                for w in list.iter_mut() {
                    if w.as_str().is_some_and(|s| s.starts_with("Some of this needs administrator rights")) {
                        *w = json!(ADMIN_PROMPT_NOTE);
                    }
                }
            }
        }
    }
    v
}

// ---- the install job's log ------------------------------------------------------------------

const KEEP_LOGS: usize = 20;
const MAX_LOG_LINES: usize = 5000;
const TAIL: usize = 500;

#[derive(Default)]
struct LogEntry {
    path: Option<PathBuf>,
    lines: Vec<String>,
}

#[derive(Default)]
struct Logs {
    by_job: HashMap<String, LogEntry>,
    order: VecDeque<String>,
}

static LOGS: Mutex<Option<Logs>> = Mutex::new(None);

fn with_logs<T>(f: impl FnOnce(&mut Logs) -> T) -> T {
    let mut g = LOGS.lock().unwrap_or_else(|p| p.into_inner());
    f(g.get_or_insert_with(Logs::default))
}

fn log_begin(job: &str) {
    with_logs(|l| {
        l.order.push_back(job.to_string());
        l.by_job.insert(job.to_string(), LogEntry::default());
        while l.order.len() > KEEP_LOGS {
            if let Some(old) = l.order.pop_front() {
                l.by_job.remove(&old);
            }
        }
    });
}

fn log_push(job: &str, line: String) {
    with_logs(|l| {
        if let Some(e) = l.by_job.get_mut(job) {
            if e.lines.len() < MAX_LOG_LINES {
                e.lines.push(line);
            }
        }
    });
}

fn log_finish(job: &str, path: Option<PathBuf>) {
    // The file also holds the error text, so prefer it once it exists.
    let from_file = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| t.lines().map(String::from).collect::<Vec<_>>());
    with_logs(|l| {
        if let Some(e) = l.by_job.get_mut(job) {
            e.path = path;
            if let Some(lines) = from_file {
                e.lines = lines;
            }
        }
    });
}

/// `GET /installs/{job}/log` → `{path, lines}` (the end of the log; `path` is empty until the log is written).
pub async fn install_log(State(st): State<AppState>, Extension(caller): Extension<Caller>, Path(job): Path<String>) -> Result<Json<Value>, ApiError> {
    // The log belongs to the install job: a remote caller needs the role that job needs.
    if caller.remote {
        crate::jobs::may_read(&caller, st.jobs.op(&job))?;
    }
    with_logs(|l| {
        l.by_job.get(&job).map(|e| {
            let start = e.lines.len().saturating_sub(TAIL);
            json!({"path": e.path.as_ref().map(|p| p.to_string_lossy().into_owned()), "lines": &e.lines[start..]})
        })
    })
    .map(Json)
    .ok_or_else(|| ApiError::not_found("There is no install with that id."))
}

// ---- stopped installs that can carry on -----------------------------------------------------

/// How long a stopped install can be continued, and how many are kept.
const KEEP_PAUSED: Duration = Duration::from_secs(6 * 3600);
const MAX_PAUSED: usize = 10;

/// An install that stopped at a failed step. The request still holds the licence key and the
/// database password: they live here, in this process's memory (never written anywhere with a secret
/// in it), so "Try this step again" works without typing them again. When the key is a SAVED one
/// (`license_key_name`), the same stopped install is also written down without any secret
/// (`PausedStore`) and carries on after a panel restart, the key being looked up by name; a pasted key
/// that was not saved is gone when the panel restarts.
struct Paused {
    req: InstallRequest,
    via_helper: bool,
    /// Windows: its administrator steps go to the elevated worker (a new prompt when it carries on).
    windows_admin: bool,
    /// The step that failed (`None`: it stopped before the first step). Only this one may be skipped.
    failed_step: Option<String>,
    /// The progress events of the steps that had finished, replayed first by the next job.
    finished: Vec<Value>,
    at: Instant,
}

static PAUSED: Mutex<Option<HashMap<String, Paused>>> = Mutex::new(None);

fn with_paused<T>(f: impl FnOnce(&mut HashMap<String, Paused>) -> T) -> T {
    let mut g = PAUSED.lock().unwrap_or_else(|p| p.into_inner());
    let m = g.get_or_insert_with(HashMap::new);
    m.retain(|_, p| p.at.elapsed() < KEEP_PAUSED);
    f(m)
}

/// Write the stopped install down when it uses a SAVED key (so it survives a panel restart). The file has
/// no secret (see `kora_core::installer::paused`). A pasted key that was not saved stays in memory only.
fn persist(job: &str, p: &Paused) {
    let store = PausedStore::default();
    store.remove_for_dir(&p.req.dir);
    let _ = store.save(job, &p.req, p.via_helper, p.failed_step.as_deref(), &p.finished);
}

/// Stopped installs that can carry on: `[{job, name, failed_step}]`, in memory or written down.
fn resumable() -> Vec<Value> {
    let mut out: Vec<(String, String, Option<String>)> = with_paused(|m| m.iter().map(|(j, p)| (j.clone(), p.req.name.clone(), p.failed_step.clone())).collect());
    for s in PausedStore::default().list() {
        if !out.iter().any(|(j, _, _)| *j == s.job) {
            out.push((s.job, s.req.name, s.failed_step));
        }
    }
    out.into_iter().map(|(job, name, step)| json!({"job": job, "name": name, "failed_step": step})).collect()
}

/// Tests: forget the stopped installs held in memory, as a panel restart does (what was written down stays).
#[doc(hidden)]
pub fn forget_paused_in_memory() {
    with_paused(|m| m.clear());
}

fn pause(job: &str, p: Paused) {
    persist(job, &p);
    with_paused(|m| {
        // One stopped install per folder: a newer one replaces it.
        m.retain(|_, old| old.req.dir != p.req.dir);
        while m.len() >= MAX_PAUSED {
            let Some(oldest) = m.iter().min_by_key(|(_, v)| v.at).map(|(k, _)| k.clone()) else { break };
            m.remove(&oldest);
        }
        m.insert(job.to_string(), p);
    });
}

/// The last event of every step that finished (done or skipped), in order: what a resumed job shows first.
fn finished_events(events: &[Value]) -> Vec<Value> {
    let mut last: Vec<(String, Value)> = Vec::new();
    for ev in events {
        let Some(step) = ev.get("step").and_then(Value::as_str) else { continue };
        // "Administrator permission" belongs to one run: a resumed run asks again when it needs to.
        if step == ADMIN_STEP {
            continue;
        }
        last.retain(|(s, _)| s != step);
        last.push((step.to_string(), ev.clone()));
    }
    last.into_iter().filter(|(_, ev)| matches!(ev.get("status").and_then(Value::as_str), Some("done" | "skipped"))).map(|(_, ev)| ev).collect()
}

/// The job error of a stopped install: the Problem (`code`, `message`, `details`, `actions`) plus
/// `kept`, `log_path`, `failed_step` and `resume: true` (`POST /installs/{job}/resume` works).
fn stopped_error(job: &str, problem: Problem, kept: &[String], log_path: Option<&std::path::Path>, failed_step: Option<&str>) -> String {
    let path = log_path.map(|p| p.to_string_lossy().into_owned());
    let problem = problem.with_action(Action::OpenLog { job: Some(job.to_string()), path: path.clone() });
    let mut err = problem.to_json();
    err["kept"] = json!(kept);
    err["resume"] = json!(true);
    if let Some(p) = path {
        err["log_path"] = json!(p);
    }
    if let Some(s) = failed_step {
        err["failed_step"] = json!(s);
    }
    err.to_string()
}

/// The install runs in the privileged helper as `kora install ...` (the same command the user would
/// have been asked to run). The licence key and the database admin password go in the environment, never
/// on the command. The helper's output lines become job events and go in the install log.
/// A failure can be retried (the helper's install continues where it stopped), not skipped: the
/// helper's install state belongs to the administrator account.
fn install_via_helper(h: &kora_core::api::JobHandle, id: &str, req: &InstallRequest) -> Result<Value, String> {
    let env = crate::elevated::secret_env(req.license_key.as_deref(), req.db_admin.as_ref().and_then(|a| a.password.as_deref()));
    let r = crate::elevated::privileged(core::install_command(req), env, &mut |line| {
        h.event(json!({"type": "status", "message": line}));
        log_push(id, line.to_string());
    });
    log_finish(id, None);
    match r {
        Ok(_) => {
            PausedStore::default().remove(id);
            serde_json::to_value(outcome_from_record(req)).map_err(|e| e.to_string())
        }
        Err(e) => {
            let mut p = e.to_problem();
            if p.code == "bad_request" || p.code == "internal" {
                p.code = kora_core::problem::codes::FAILED.to_string();
            }
            let p = p.with_action(Action::Retry).with_action(Action::ChangeAnswers { section: None });
            pause(id, Paused { req: req.clone(), via_helper: true, windows_admin: false, failed_step: None, finished: vec![], at: Instant::now() });
            Err(stopped_error(id, p, &[], None, None))
        }
    }
}

/// The outcome of an install the helper ran, read back from what it left in the server folder (the panel
/// never sees the helper's own result): the address, whether it is running, the generated txAdmin account
/// (its NAME only: the password stays in its file, which is never read). Panel-side so that it goes with the
/// helper when the helper is replaced.
pub(crate) fn outcome_from_record(req: &InstallRequest) -> kora_core::installer::InstallOutcome {
    use kora_core::launch;
    let pw_file = req.dir.join("secrets").join("txadmin-admin.txt");
    let local_admin = pw_file.is_file();
    let spec = launch::Spec { name: req.name.clone(), dir: req.dir.clone(), game_port: req.ports.game, txadmin_port: req.ports.txadmin };
    let st = req.start.then(|| launch::status_of(&launch::SystemEnv::default(), &spec));
    let running = st.as_ref().is_some_and(|s| s.running && s.txadmin_up);
    let log_tail = if req.start && !running { kora_core::api::logs::discover(&req.dir).first().and_then(|l| kora_core::api::logs::tail(&l.path, 20).ok()).unwrap_or_default() } else { vec![] };
    let mut notes = vec!["Installed by the Kora helper.".to_string()];
    if !req.start {
        notes.push(format!("The server is installed but not started. Start it with `kora start {}`. txAdmin runs on port {}.", req.name, req.ports.txadmin));
    }
    kora_core::installer::InstallOutcome {
        dir: req.dir.clone(),
        connect: launch::connect_address(req.ports.game),
        txadmin_url: format!("http://localhost:{}", req.ports.txadmin),
        notes,
        name: req.name.clone(),
        start_attempted: req.start,
        running,
        txadmin_up: st.as_ref().is_some_and(|s| s.txadmin_up),
        game_listening: st.as_ref().is_some_and(|s| s.game_listening),
        launch_problem: (req.start && !running).then(|| "The server did not come up yet. The last lines of its log are below; look at the server's Logs tab.".to_string()),
        log_tail,
        admin_user: local_admin.then(|| kora_core::txadmin::LOCAL_ADMIN_USER.to_string()),
        admin_password_file: local_admin.then_some(pw_file),
    }
}

/// The progress row of "Windows is asking for permission" (not an installer step).
pub const ADMIN_STEP: &str = "admin";

/// How an install gets its administrator rights.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdminMode {
    /// It needs none, it has them, or the person chose to go on without them.
    None,
    /// Linux: the whole install runs in the privileged helper.
    Helper,
    /// Windows: the person's own install, with its administrator steps done by the elevated
    /// worker. `eager`: ask before the first step (a new install), else when a step needs it.
    Worker { ask: AdminAsk, eager: bool },
}

/// What the person reads when Windows did not give permission, and what they can do.
fn admin_context(req: &InstallRequest, reasons: &[String]) -> worker::Context {
    let what = if reasons.is_empty() { "do the parts of this install that need administrator rights".to_string() } else { kora_core::elevate::join_reasons(reasons) };
    worker::Context { what, command: core::install_command(req), can_skip_admin: true }
}

fn admin_event(status: &str, detail: &str) -> Value {
    json!({"step": ADMIN_STEP, "title": "Administrator permission", "status": status, "detail": detail, "percent": 0})
}

/// Start the install job (a first run, or a resumed one that replays `finished` first).
fn spawn_install(st: &AppState, e: Arc<dyn InstallEnv>, req: InstallRequest, mode: AdminMode, finished: Vec<Value>) -> Result<Response, ApiError> {
    let name = req.name.clone();
    st.spawn_job(Op::InstallStart, Some(name), move |h| {
        let id = h.id().0.clone();
        log_begin(&id);
        for ev in &finished {
            h.event(ev.clone());
        }
        // Written down (without any secret, and only when the key is a SAVED one) so that a panel that dies
        // mid-install leaves something to carry on from; see `PausedStore::save_in_progress`.
        if !req.dry_run {
            let _ = PausedStore::default().save_in_progress(&id, &req, mode == AdminMode::Helper, &finished);
        }
        if mode == AdminMode::Helper {
            return install_via_helper(h, &id, &req);
        }
        let windows_admin = matches!(mode, AdminMode::Worker { .. });
        let admin = match mode {
            AdminMode::Worker { ask, eager } => {
                let wa = WorkerAdmin::new(ask, admin_context(&req, &e.admin_reasons(&req)));
                if eager {
                    h.event(admin_event(
                        "running",
                        "Windows is asking for permission on the computer that runs Kora. Choose Yes to carry on. If you do not see the prompt, look for a flashing shield icon in the taskbar.",
                    ));
                    log_push(&id, "Asking Windows for administrator permission".into());
                    if let Err(p) = wa.open() {
                        h.event(admin_event("failed", &p.message));
                        log_push(&id, format!("Administrator permission: {}", p.message));
                        log_finish(&id, None);
                        let p = p.with_action(Action::ChangeAnswers { section: None });
                        pause(&id, Paused { req: req.clone(), via_helper: false, windows_admin, failed_step: None, finished: finished.clone(), at: Instant::now() });
                        return Err(stopped_error(&id, p, &[], None, None));
                    }
                    h.event(admin_event("done", "Permission given: Kora does the administrator steps when the install reaches them"));
                    log_push(&id, "Administrator permission given".into());
                }
                Some(SharedAdmin(Arc::new(wa)))
            }
            _ => None,
        };
        let mut events: Vec<Value> = finished.clone();
        let fin = core::run_install_with(&*e, &req, admin, |ev| {
            let v = core::progress_json(ev);
            events.push(v.clone());
            h.event(v);
            log_push(&id, core::log_line(ev));
        });
        log_finish(&id, fin.log_path.clone());
        match fin.outcome {
            Ok(out) => {
                PausedStore::default().remove(&id);
                Ok(core::outcome_json(&out))
            }
            Err(msg) => {
                let problem = fin.problem.unwrap_or_else(|| Problem::decode(&msg));
                if !req.dry_run {
                    pause(&id, Paused { req: req.clone(), via_helper: false, windows_admin, failed_step: fin.failed_step.clone(), finished: finished_events(&events), at: Instant::now() });
                }
                Err(stopped_error(&id, problem, &fin.kept, fin.log_path.as_deref(), fin.failed_step.as_deref()))
            }
        }
    })
}

// ---- starting an install --------------------------------------------------------------------

/// `POST /installs`: body = the UI's install request. Validates like the CLI, then runs the
/// installer as a job: every `ProgressEvent` is a job event (`{step, title, status, detail,
/// percent}`), the result is the outcome (`connect`, `txadmin_url`, `admin_user`,
/// `admin_password_file`, `notes`, ...; never the password). A failure is a job error with the
/// Problem's fields (`code`, `message`, `details`, `actions`) plus `kept`, `log_path`,
/// `failed_step` and `resume` (see [`install_resume`]). Without administrator rights for what the
/// install needs: `needs_elevation` with the `kora install ...` command (no secrets on it).
pub async fn install_start(State(st): State<AppState>, Extension(caller): Extension<Caller>, headers: HeaderMap, Json(body): Json<Value>) -> Result<Response, ApiError> {
    let ask = AdminAsk::of(&caller, &headers);
    let wire: WireInstall = parse(body)?;
    // Pasting a new key (or saving one) is for admins; every role that may install can pick a saved key.
    if wire.license_key.as_deref().is_some_and(|k| !k.trim().is_empty()) || wire.save_license_key_as.is_some() {
        super::keys::require_manage(&caller)?;
    }
    let save_as = wire.save_license_key_as.clone().filter(|n| !n.trim().is_empty());
    let e = env();
    let e2 = e.clone();
    let st2 = st.clone();
    let req = blocking(move || {
        let mut req = core::build_request(&wire).map_err(ApiError::bad_request)?;
        if req.dry_run {
            return Ok((req, AdminMode::None));
        }
        // A saved key is looked up by name here, so the request that is kept and logged never carries it.
        if let Some(n) = req.license_key_name.clone() {
            if req.license_key.is_some() {
                return Err(ApiError::bad_request("Pick one of the saved keys, or paste a new key (not both).").with_action(Action::change_answers("licence")));
            }
            req.license_key = Some(KeyStore::default().get(&n).map_err(super::keys::api)?);
        }
        if req.license_key.is_none() {
            return Err(ApiError::problem(
                StatusCode::BAD_REQUEST,
                Problem::new(codes::INSTALL_LICENCE_MISSING, "A licence key is needed. Get one from the Cfx.re portal: https://portal.cfx.re/servers/registration-keys")
                    .with_action(Action::change_answers("licence"))
                    .with_action(Action::open_link(kora_core::installer::LICENCE_URL)),
            ));
        }
        let servers = st2.backend().list().map_err(ApiError::internal)?;
        if servers.iter().any(|s| slug(&s.name) == slug(&req.name) || s.dir == req.dir) {
            return Err(ApiError::conflict(format!("A server called '{}' (or in that folder) already exists. Choose another name or folder.", req.name)).with_action(Action::change_answers("name")));
        }
        if req.mode == Mode::Development {
            let owner = super::github::env().resolve_owner(req.github_owner.as_deref()).map_err(|m| {
                if Problem::is_encoded(&m) {
                    ApiError::core(m)
                } else {
                    ApiError::bad_request(m).with_action(Action::change_answers("github"))
                }
            })?;
            ProfileStore::default().remember_github_owner(&owner);
            req.github_owner = Some(owner);
        }
        if !core::already_installed(&req) {
            let avoid = core::avoid_ports(&servers, &req.name, Some(&req.dir));
            core::settle_ports(&mut req, false, e2.prober(), &avoid).map_err(|m| ApiError::conflict(format!("{m} Go back and pick other ports.")).with_action(Action::change_answers("ports")))?;
        }
        // The person chose to save a pasted key: it is kept under that name (an already saved key is just
        // reused, saying nothing twice). From here on the install can be written down without the key.
        if let (Some(k), Some(n)) = (req.license_key.clone(), &save_as) {
            let store = KeyStore::default();
            let name = match store.name_of(&k).map_err(super::keys::api)? {
                Some(existing) => existing,
                None => {
                    kora_core::keystore::check_key(&k).map_err(super::keys::api)?;
                    store.add(n, &k).map_err(super::keys::api)?.name
                }
            };
            req.license_key_name = Some(name);
        }
        let reasons = e2.admin_reasons(&req);
        if !reasons.is_empty() && !e2.is_elevated() {
            // Windows: one permission prompt when the install starts (not for a remote session
            // unless the person asked). Linux: the helper when installed, otherwise the command.
            crate::elevated::precheck(ask, &admin_context(&req, &reasons), &core::elevation_message(&req, &reasons))?;
            if crate::elevated::worker_supported() {
                return Ok((req, AdminMode::Worker { ask, eager: true }));
            }
            return Ok((req, AdminMode::Helper));
        }
        Ok((req, AdminMode::None))
    })
    .await?;
    let (req, mode) = req;
    // A new start for this folder replaces any stopped install of it.
    with_paused(|m| m.retain(|_, p| p.req.dir != req.dir));
    PausedStore::default().remove_for_dir(&req.dir);
    spawn_install(&st, e, req, mode, vec![])
}

/// `POST /installs/{job}/resume` `{skip?: step}`: carry on with an install that stopped (its job
/// error has `resume: true`), with the same answers, licence key included (kept in memory, see
/// [`Paused`]). Without `skip` the failed step runs again; with it, that step is skipped first
/// (only steps that are safe to skip, see `kora_core::installer::SKIPPABLE`). Answers `202 {job}`:
/// a new job, which first replays the steps that had finished.
pub async fn install_resume(State(st): State<AppState>, Extension(caller): Extension<Caller>, headers: HeaderMap, Path(job): Path<String>, Json(body): Json<Value>) -> Result<Response, ApiError> {
    let ask = AdminAsk::of(&caller, &headers);
    let without_admin = match body.get("without_admin") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(ApiError::bad_request("That request was not understood. Reload the page and try again.")),
    };
    let skip = match body.get("skip") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Some(_) => return Err(ApiError::bad_request("That request was not understood. Reload the page and try again.")),
    };
    let in_memory = with_paused(|m| m.remove(&job));
    let paused = match in_memory {
        Some(p) => Some(p),
        // After a restart: only an install that used a saved key was written down. Its key is looked up
        // by name now; a key that was renamed or deleted since is said plainly (the file is kept).
        None => {
            let id = job.clone();
            blocking(move || {
                let Some(s) = PausedStore::default().load(&id) else { return Ok(None) };
                let mut req = s.req;
                if let Some(n) = req.license_key_name.clone() {
                    req.license_key = Some(KeyStore::default().get(&n).map_err(super::keys::api)?);
                }
                // Not written down: on Windows its administrator steps go to the worker again (asked when needed).
                let windows_admin = crate::elevated::worker_supported() && !s.via_helper;
                Ok(Some(Paused { req, via_helper: s.via_helper, windows_admin, failed_step: s.failed_step, finished: s.finished, at: Instant::now() }))
            })
            .await?
        }
    };
    let Some(paused) = paused else {
        return Err(ApiError::problem(
            StatusCode::NOT_FOUND,
            Problem::new(
                codes::INSTALL_CANNOT_RESUME,
                "This install can no longer be continued from here (Kora Panel was restarted, or it was continued already). Check your answers and start it again: it carries on where it stopped.",
            )
            .with_action(Action::ChangeAnswers { section: None }),
        ));
    };
    let e = env();
    let id = job.clone();
    let (paused, e) = blocking(move || {
        if without_admin && paused.via_helper {
            let p = Problem::new(codes::INSTALL_CANNOT_RESUME, "This install runs with administrator rights, so it cannot carry on without them from here. Change your answers instead.")
                .with_action(Action::ChangeAnswers { section: None });
            pause(&id, paused);
            return Err(ApiError::problem(StatusCode::CONFLICT, p));
        }
        // A remote session does not make Windows ask on a desktop nobody may be watching.
        if paused.windows_admin && !without_admin && !crate::elevated::may_prompt(ask) {
            let p = kora_core::helper::worker::remote_problem(&admin_context(&paused.req, &e.admin_reasons(&paused.req)));
            pause(&id, paused);
            return Err(crate::elevated::admin_error(p));
        }
        if let Some(step) = &skip {
            let refused = if paused.via_helper {
                Some(
                    Problem::new(codes::INSTALL_CANNOT_RESUME, "This install ran with administrator rights, so its steps cannot be skipped from here. Try it again, or change your answers.")
                        .with_action(Action::Retry)
                        .with_action(Action::ChangeAnswers { section: None }),
                )
            } else if paused.failed_step.as_deref() != Some(step.as_str()) {
                Some(Problem::new(codes::INSTALL_CANNOT_RESUME, "Only the step that failed can be skipped. Try it again, or change your answers.").with_action(Action::ChangeAnswers { section: None }))
            } else {
                e.skip_step(&paused.req, step).err()
            };
            if let Some(p) = refused {
                // Nothing changed: the install can still be retried.
                pause(&id, paused);
                return Err(ApiError::problem(StatusCode::CONFLICT, p));
            }
        }
        Ok((paused, e))
    })
    .await?;
    let Paused { req, via_helper, windows_admin, failed_step, finished, .. } = paused;
    let again = Paused { req: req.clone(), via_helper, windows_admin, failed_step, finished: finished.clone(), at: Instant::now() };
    let (req, mode) = if without_admin {
        (core::without_admin(&req), AdminMode::None)
    } else if via_helper {
        (req, AdminMode::Helper)
    } else if windows_admin {
        (req, AdminMode::Worker { ask, eager: false })
    } else {
        (req, AdminMode::None)
    };
    let r = spawn_install(&st, e, req, mode, finished);
    if r.is_err() {
        // Not started (another job runs for this server): it can still be continued later.
        pause(&job, again);
    } else {
        PausedStore::default().remove(&job);
    }
    r
}
