//! Administrator work for the panel, through ONE seam.
//!
//! The panel never runs elevated. Administrator work goes to Kora's administrator service (see
//! `kora_core::privsvc`): the installer put it in a protected folder, it knows which accounts may use
//! it, and it does a fixed list of validated operations with no prompt, locally or remotely. Without
//! the service (a per-user install) there is no automatic path: the panel answers `needs_elevation`
//! with the choices "Install Kora's administrator service", "continue without the administrator parts"
//! (installs) and "show me the command" for an administrator terminal.
//!
//! `privileged_as` takes the `kora ...` command the handler used to hand to the user: the first word
//! (`kora`) is dropped, the next is the verb, the rest are its arguments. Secrets (licence key,
//! database password) go in `env`, never in the command.

use std::sync::{Arc, Mutex};

use kora_core::installer::system_ops::{AdminRunner, AdminStepOutcome};
use kora_core::privsvc::conn::ClientError;
use kora_core::privsvc::problems::{self as svc_problems, Availability, Context};
use kora_core::problem::{codes, Problem};
use serde_json::{json, Value};

use crate::error::ApiError;
use axum::http::StatusCode;

/// How one request to the service ended.
#[derive(Debug, Clone, PartialEq)]
pub struct StepOutcome {
    pub ok: bool,
    pub code: i32,
    /// The `admin-step` result; may hold a secret (a new database login): never log it.
    pub data: Option<Value>,
}

/// The way to the service (the real one: a socket on Linux, a named pipe on Windows; tests use a fake).
pub trait Broker: Send + Sync {
    /// Is the service there, and does it serve this account?
    fn availability(&self) -> Availability;
    /// Does the service do `verb` at all? (`uninstall` and `app` stay commands for an administrator terminal.)
    fn handles(&self, _verb: &str) -> bool {
        true
    }
    /// One request; `on_line` gets each progress line.
    fn call(&self, verb: &str, args: &[String], env: &[(String, String)], on_line: &mut dyn FnMut(&str)) -> Result<StepOutcome, ClientError>;
}

/// The administrator service: Linux on its Unix socket, Windows on its named pipe.
struct ServiceBroker;

impl Broker for ServiceBroker {
    fn availability(&self) -> Availability {
        #[cfg(unix)]
        let r = kora_core::privsvc::unixsvc::probe(std::path::Path::new(kora_core::privsvc::paths::LINUX_SOCKET));
        #[cfg(windows)]
        let r = kora_core::privsvc::winsvc::probe();
        #[cfg(not(any(unix, windows)))]
        let r: Result<String, ClientError> = Err(ClientError::Unavailable("no administrator service on this system".into()));
        match r {
            Ok(_) => Availability::Ready,
            Err(ClientError::Refused(why)) => Availability::NotAllowed(why),
            Err(_) => Availability::Missing,
        }
    }
    fn handles(&self, verb: &str) -> bool {
        kora_core::privsvc::op::VERBS.contains(&verb)
    }
    fn call(&self, verb: &str, args: &[String], env: &[(String, String)], on_line: &mut dyn FnMut(&str)) -> Result<StepOutcome, ClientError> {
        #[cfg(unix)]
        let r = kora_core::privsvc::unixsvc::call(std::path::Path::new(kora_core::privsvc::paths::LINUX_SOCKET), verb, args, env, on_line);
        #[cfg(windows)]
        let r = kora_core::privsvc::winsvc::call(verb, args, env, on_line);
        #[cfg(not(any(unix, windows)))]
        let r: Result<kora_core::privsvc::conn::Answer, ClientError> = {
            let _ = (verb, args, env, &on_line);
            Err(ClientError::Unavailable("no administrator service on this system".into()))
        };
        r.map(|a| StepOutcome { ok: a.ok, code: a.code, data: a.data })
    }
}

type BrokerSlot = Mutex<Option<Arc<dyn Broker>>>;
static BROKER: BrokerSlot = Mutex::new(None);

/// Test seam: use this broker instead of the real service (`None` = the real one again).
pub fn set_broker_override(b: Option<Arc<dyn Broker>>) {
    *BROKER.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = b;
}

fn broker() -> Arc<dyn Broker> {
    BROKER.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone().unwrap_or_else(|| Arc::new(ServiceBroker))
}

/// Can administrator work be done now (the service answers and serves this account)? (`GET /system` `helper_installed`.)
pub fn service_ready() -> bool {
    broker().availability() == Availability::Ready
}

/// `ready`, `missing` or `not_allowed` (with the service's reason): for `GET /system`.
pub fn service_state() -> Value {
    match broker().availability() {
        Availability::Ready => json!({"state": "ready"}),
        Availability::Missing => json!({"state": "missing"}),
        Availability::NotAllowed(why) => json!({"state": "not_allowed", "reason": why}),
    }
}

/// Does the service do this verb (`self-update`, `schedule`, ...)? `uninstall` and `app` are not among its
/// operations: those stay commands for an administrator terminal.
pub fn can_route(verb: &str) -> bool {
    broker().handles(verb)
}

fn problem_status(p: &Problem) -> StatusCode {
    if [codes::NEEDS_ELEVATION, codes::SERVICE_NOT_ALLOWED].contains(&p.code.as_str()) {
        StatusCode::FORBIDDEN
    } else {
        StatusCode::UNPROCESSABLE_ENTITY
    }
}

/// The answer for a Problem from the service (403 for the "permission" ones).
pub fn admin_error(p: Problem) -> ApiError {
    ApiError::problem(problem_status(&p), p)
}

fn current_user() -> String {
    std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_default()
}

/// Before starting a job that needs administrator rights: `Ok` = it can go ahead (the service is there for
/// this account); `Err` = the choices to give instead (install the service, continue without, the command).
/// A remote session is never turned away for being remote: there is no prompt to answer.
pub fn precheck(ctx: &Context, message: &str) -> Result<(), ApiError> {
    match broker().availability() {
        Availability::Ready => Ok(()),
        a => {
            let mut p = svc_problems::unavailable(&a, ctx, &current_user());
            // The handler's own sentence for the "not installed" case, when it has one worded for the screen.
            if a == Availability::Missing && !message.is_empty() && !message.ends_with(':') {
                p.message = format!("{message} {}", p.message);
            }
            Err(admin_error(p))
        }
    }
}

/// [`precheck`] for a command the service must be able to do (`self-update`, `schedule`, `check`, `tools`):
/// any other verb stays a command for an administrator terminal.
pub fn precheck_verb(ctx: &Context, message: &str) -> Result<(), ApiError> {
    let verb = ctx.command.get(1).map(String::as_str).unwrap_or("");
    if !can_route(verb) {
        return Err(ApiError::needs_elevation(message, ctx.command.clone()));
    }
    precheck(ctx, message)
}

fn lost(e: &ClientError, ctx: &Context) -> Problem {
    let (ClientError::Unavailable(m) | ClientError::Broken(m) | ClientError::Refused(m)) = e;
    svc_problems::lost(m, ctx)
}

/// Run `command` (`["kora", verb, args...]`) with administrator rights, through the service. `what`
/// completes "Kora cannot ...".
pub fn privileged_as(what: &str, message: &str, command: Vec<String>, env: Vec<(String, String)>, on_line: &mut dyn FnMut(&str)) -> Result<Value, ApiError> {
    let ctx = Context { what: what.to_string(), command: command.clone(), can_skip_admin: false };
    let mut parts = command.iter().map(String::as_str);
    let (Some("kora"), Some(verb)) = (parts.next(), parts.next()) else { return Err(ApiError::needs_elevation(message, command)) };
    let verb = verb.to_string();
    let args: Vec<String> = parts.map(str::to_string).collect();
    if !can_route(&verb) {
        return Err(ApiError::needs_elevation(message, command));
    }
    precheck(&ctx, message)?;
    let mut tail: Vec<String> = Vec::new();
    let res = broker().call(&verb, &args, &env, &mut |l| {
        on_line(l);
        tail.push(l.to_string());
        if tail.len() > 20 {
            tail.remove(0);
        }
    });
    match res {
        Ok(o) if o.ok => Ok(json!({"via_service": true, "output": tail})),
        Ok(o) => {
            let last = tail.iter().rev().find(|l| !l.trim().is_empty()).cloned().unwrap_or_default();
            let detail = if last.is_empty() { String::new() } else { format!(" The last thing it said: {last}") };
            Err(ApiError::core(format!("The change could not be made (it stopped with code {}).{detail}", o.code)))
        }
        Err(ClientError::Refused(m)) => Err(ApiError::core(m)),
        Err(e) => Err(admin_error(lost(&e, &ctx))),
    }
}

/// [`privileged_as`] for the toolchain actions, whose sentence names no particular thing.
pub fn privileged_msg(message: &str, command: Vec<String>, env: Vec<(String, String)>, on_line: &mut dyn FnMut(&str)) -> Result<Value, ApiError> {
    privileged_as("do this", message, command, env, on_line)
}

/// Environment for a privileged install: only the names the service accepts, and only when set.
pub fn secret_env(license_key: Option<&str>, db_admin_password: Option<&str>) -> Vec<(String, String)> {
    let mut v = Vec::new();
    if let Some(k) = license_key.filter(|k| !k.is_empty()) {
        v.push(("CFX_LICENSE_KEY".to_string(), k.to_string()));
    }
    if let Some(p) = db_admin_password.filter(|p| !p.is_empty()) {
        v.push(("KORA_DB_ADMIN_PASSWORD".to_string(), p.to_string()));
    }
    v
}

/// Start (`"start"`) or install (`"install"`) Kora's own MariaDB through the service (Windows).
pub fn admin_db(action: &str, on_line: &mut dyn FnMut(&str)) -> Result<Value, ApiError> {
    let what = if action == "install" { "install Kora's own database server" } else { "start Kora's own database server" };
    privileged_as(what, "Starting the database server needs administrator rights. Run this command as administrator:", vec!["kora".into(), "admin-db".into(), action.into()], vec![], on_line)
}

/// The install's (or a delete's) administrator steps through the service: each step is one request.
pub struct ServiceAdmin {
    ctx: Context,
}

impl ServiceAdmin {
    pub fn new(ctx: Context) -> Self {
        Self { ctx }
    }

    /// Is the service there now? (The install checks before its first step, so a missing service is
    /// reported at the start, with the choices, not in the middle.)
    pub fn open(&self) -> Result<(), Problem> {
        match broker().availability() {
            Availability::Ready => Ok(()),
            a => Err(svc_problems::unavailable(&a, &self.ctx, &current_user())),
        }
    }
}

impl AdminRunner for ServiceAdmin {
    fn admin_step(&self, args: &[String], on_line: &mut dyn FnMut(&str)) -> Result<AdminStepOutcome, Problem> {
        self.open()?;
        match broker().call("admin-step", args, &[], on_line) {
            Ok(o) => Ok(AdminStepOutcome { ok: o.ok, data: o.data }),
            Err(ClientError::Refused(m)) => Err(Problem::generic(m)),
            Err(e) => Err(lost(&e, &self.ctx)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_problems_are_403_and_a_lost_service_is_422() {
        for c in [codes::NEEDS_ELEVATION, codes::SERVICE_NOT_ALLOWED] {
            assert_eq!(admin_error(Problem::new(c, "m")).status, StatusCode::FORBIDDEN, "{c}");
        }
        assert_eq!(admin_error(Problem::new(codes::SERVICE_LOST, "m")).status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[test]
    fn only_the_two_secrets_go_in_the_environment() {
        assert_eq!(secret_env(Some("k"), Some("p")).len(), 2);
        assert!(secret_env(Some(""), None).is_empty());
    }
}
