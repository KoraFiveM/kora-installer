//! Administrator work for the panel, through ONE seam.
//!
//! The panel never runs elevated. Administrator work goes to Kora's administrator service (see
//! `kora_core::privsvc`): the installer put it in a protected folder, it knows which accounts may use
//! it, and it does a fixed list of validated operations with no prompt, locally or remotely. Without
//! the service (a per-user install) there is no automatic path: the panel answers `needs_elevation`
//! with the choices "Install Kora's administrator service", "continue without the administrator parts"
//! (installs) and "show me the command" for an administrator terminal.
//!
//! Windows still starts the short-lived UAC worker until the Windows service lands; that path (and
//! `may_prompt`, `AdminAsk`, the `admin.*` problems) goes away with it.
//!
//! `privileged_as` takes the `kora ...` command the handler used to hand to the user: the first word
//! (`kora`) is dropped, the next is the verb, the rest are its arguments. Secrets (licence key,
//! database password) go in `env`, never in the command.

use std::sync::{Arc, Mutex};

use kora_core::helper::client::HelperError;
use serde_json::{json, Value};

use crate::error::ApiError;
use axum::http::{HeaderMap, StatusCode};
use kora_core::helper::worker::{self, Context, OpenError, Session, StepOutcome};
use kora_core::installer::system_ops::{AdminRunner, AdminStepOutcome};
use kora_core::privsvc::problems::{self as svc_problems, Availability};
use kora_core::problem::{codes, Problem};

/// Is the administrator service there and serving this account? (`GET /system` `helper_installed`.)
pub fn helper_installed() -> bool {
    worker_supported()
}

/// Environment for a privileged install: only the names the helper accepts, and only when set.
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

// ---- Windows: the elevated worker (one prompt per install or action) --------------------------
//
// The panel never runs elevated. On Windows it asks Windows (the normal permission prompt) to start
// its own program as a short-lived administrator worker, which does the allow-listed work and exits
// (see `kora_core::helper::worker` for the design and the threat model). Whether the prompt may be
// shown depends on who asks ([`AdminAsk`]): someone at this computer, yes; a remote session only when
// the person asked for it, because nobody may be at the computer to answer it.

/// Header a page sends when the person chose "Ask for administrator permission" (`ask`).
pub const ASK_HEADER: &str = "x-kora-admin";

/// Who asks for administrator work, as far as the prompt is concerned.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AdminAsk {
    /// Not at this computer: another device, or a remote sign-in (token, Tailscale) even through a local proxy.
    pub remote: bool,
    /// The person explicitly asked for the Windows prompt.
    pub asked: bool,
}

impl AdminAsk {
    pub fn of(caller: &crate::remote::Caller, headers: &HeaderMap) -> Self {
        let remote = caller.remote || caller.identity.as_ref().is_some_and(|i| i.provider != crate::remote::Provider::Local);
        let asked = headers.get(ASK_HEADER).and_then(|v| v.to_str().ok()).is_some_and(|v| v.eq_ignore_ascii_case("ask"));
        Self { remote, asked }
    }
    /// A request from this computer, the person not asking explicitly (tests, internal callers).
    pub fn local() -> Self {
        Self::default()
    }
}

/// May the Windows prompt be shown for this request? Not for a remote session unless asked.
pub fn may_prompt(ask: AdminAsk) -> bool {
    !broker().prompts() || !ask.remote || ask.asked
}

/// Opens administrator sessions (the real ones: the service on Linux, the UAC prompt on Windows
/// for now; tests use a fake).
pub trait Broker: Send + Sync {
    /// Can the panel get administrator work done at all (the service is there and serves this
    /// account, or on Windows the prompt can be shown)?
    fn supported(&self) -> bool;
    /// The same, with the reason when not.
    fn availability(&self) -> Availability {
        if self.supported() {
            Availability::Ready
        } else {
            Availability::Missing
        }
    }
    /// Does a request show a permission prompt on this computer (the Windows worker)? The service never does.
    fn prompts(&self) -> bool {
        true
    }
    /// Does this broker do `verb` at all? The service has a short list; the old worker took more.
    fn handles(&self, _verb: &str) -> bool {
        true
    }
    fn open(&self, kora_home: Option<&str>) -> Result<Box<dyn Channel>, OpenError>;
}

/// One open worker session.
pub trait Channel: Send {
    fn run(&mut self, verb: &str, args: &[String], env: &[(String, String)], on_line: &mut dyn FnMut(&str)) -> Result<StepOutcome, HelperError>;
    /// Say goodbye (the worker exits).
    fn finish(self: Box<Self>);
}

impl Channel for Session {
    fn run(&mut self, verb: &str, args: &[String], env: &[(String, String)], on_line: &mut dyn FnMut(&str)) -> Result<StepOutcome, HelperError> {
        Session::run(self, verb, args, env, on_line)
    }
    fn finish(self: Box<Self>) {
        Session::finish(*self);
    }
}

/// The administrator service: Linux on its Unix socket, Windows on its named pipe.
struct ServiceBroker;

impl ServiceBroker {
    fn probe() -> Availability {
        use kora_core::privsvc::conn::ClientError;
        #[cfg(unix)]
        let r = kora_core::privsvc::unixsvc::probe(std::path::Path::new(kora_core::privsvc::paths::LINUX_SOCKET));
        #[cfg(windows)]
        let r = kora_core::privsvc::winsvc::probe();
        match r {
            Ok(_) => Availability::Ready,
            Err(ClientError::Refused(why)) => Availability::NotAllowed(why),
            Err(_) => Availability::Missing,
        }
    }
}

impl Broker for ServiceBroker {
    fn supported(&self) -> bool {
        Self::probe() == Availability::Ready
    }
    fn availability(&self) -> Availability {
        Self::probe()
    }
    fn prompts(&self) -> bool {
        false
    }
    fn handles(&self, verb: &str) -> bool {
        kora_core::privsvc::op::VERBS.contains(&verb)
    }
    fn open(&self, _kora_home: Option<&str>) -> Result<Box<dyn Channel>, OpenError> {
        Ok(Box::new(ServiceChannel))
    }
}

/// One request per connection to the service; `finish` has nothing to close.
struct ServiceChannel;

impl Channel for ServiceChannel {
    fn run(&mut self, verb: &str, args: &[String], env: &[(String, String)], on_line: &mut dyn FnMut(&str)) -> Result<StepOutcome, HelperError> {
        use kora_core::privsvc::conn::ClientError;
        #[cfg(unix)]
        let r = kora_core::privsvc::unixsvc::call(std::path::Path::new(kora_core::privsvc::paths::LINUX_SOCKET), verb, args, env, on_line);
        #[cfg(windows)]
        let r = kora_core::privsvc::winsvc::call(verb, args, env, on_line);
        match r {
            Ok(a) => Ok(StepOutcome { ok: a.ok, code: a.code, data: a.data }),
            Err(ClientError::Unavailable(m)) => Err(HelperError::Unavailable(m)),
            Err(ClientError::Refused(m)) => Err(HelperError::Rejected(m)),
            Err(ClientError::Broken(m)) => Err(HelperError::Protocol(m)),
        }
    }
    fn finish(self: Box<Self>) {}
}

/// No administrator path at all (a platform build without either).
#[cfg(not(any(unix, windows)))]
struct NoBroker;
#[cfg(not(any(unix, windows)))]
impl Broker for NoBroker {
    fn supported(&self) -> bool {
        false
    }
    fn open(&self, _: Option<&str>) -> Result<Box<dyn Channel>, OpenError> {
        Err(OpenError::Unsupported)
    }
}

fn default_broker() -> Arc<dyn Broker> {
    #[cfg(any(unix, windows))]
    return Arc::new(ServiceBroker);
    #[cfg(not(any(unix, windows)))]
    return Arc::new(NoBroker);
}

type BrokerSlot = Mutex<Option<Arc<dyn Broker>>>;
static BROKER: BrokerSlot = Mutex::new(None);

/// Test seam: use this broker instead of the real prompt (`None` = the real one again).
pub fn set_broker_override(b: Option<Arc<dyn Broker>>) {
    *BROKER.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = b;
}

fn broker() -> Arc<dyn Broker> {
    BROKER.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone().unwrap_or_else(default_broker)
}

/// Can administrator work be done now (the service answers for this account; on Windows, the prompt)?
/// (`GET /system` `admin_prompt` is the prompt-only part: [`prompts`].)
pub fn worker_supported() -> bool {
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

/// Does an administrator request show a permission prompt on this computer (the old Windows worker)?
pub fn prompts() -> bool {
    broker().prompts()
}

/// Does the administrator path take this verb (`self-update`, `schedule`, ...)? `uninstall` and `app`
/// are not among the service's operations: those stay commands for an administrator terminal.
pub fn can_route(verb: &str) -> bool {
    broker().handles(verb)
}

fn kora_home() -> String {
    kora_core::installer::registry::kora_home().to_string_lossy().into_owned()
}

fn problem_status(p: &Problem) -> StatusCode {
    if [codes::ADMIN_DECLINED, codes::ADMIN_TIMED_OUT, codes::ADMIN_REMOTE, codes::ADMIN_BUSY, codes::NEEDS_ELEVATION, codes::SERVICE_NOT_ALLOWED].contains(&p.code.as_str()) {
        StatusCode::FORBIDDEN
    } else {
        StatusCode::UNPROCESSABLE_ENTITY
    }
}

/// The answer for a Problem from asking (403 for the "permission" ones).
pub fn admin_error(p: Problem) -> ApiError {
    ApiError::problem(problem_status(&p), p)
}

fn current_user() -> String {
    std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_default()
}

/// Before starting a job that needs administrator rights: `Ok` = it can go ahead (the service is
/// there for this account, or the Windows prompt may be shown); `Err` = the choices to give instead
/// (install the service, continue without, the command to run). A remote session needs no prompt
/// with the service, so it is never turned away for being remote.
pub fn precheck(ask: AdminAsk, ctx: &Context, message: &str) -> Result<(), ApiError> {
    match broker().availability() {
        Availability::Ready => {
            if may_prompt(ask) {
                Ok(())
            } else {
                Err(admin_error(worker::remote_problem(ctx)))
            }
        }
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
pub fn precheck_verb(ask: AdminAsk, ctx: &Context, message: &str) -> Result<(), ApiError> {
    let verb = ctx.command.get(1).map(String::as_str).unwrap_or("");
    if !can_route(verb) {
        return Err(ApiError::needs_elevation(message, ctx.command.clone()));
    }
    precheck(ask, ctx, message)
}

fn lost_problem(e: &HelperError, ctx: &Context) -> Problem {
    if prompts() {
        worker::lost_problem(e)
    } else {
        svc_problems::lost(&e.to_string(), ctx)
    }
}

fn open_problem(e: &OpenError, ctx: &Context) -> Problem {
    worker::open_problem(e, ctx)
}

/// Run `command` (`["kora", verb, args...]`) with administrator rights: the service (or, on Windows
/// for now, the worker after one prompt if this request may show it). `what` completes "Kora cannot ...".
pub fn privileged_as(ask: AdminAsk, what: &str, message: &str, command: Vec<String>, env: Vec<(String, String)>, on_line: &mut dyn FnMut(&str)) -> Result<Value, ApiError> {
    let ctx = Context { what: what.to_string(), command: command.clone(), can_skip_admin: false };
    let mut parts = command.iter().map(String::as_str);
    let (Some("kora"), Some(verb)) = (parts.next(), parts.next()) else { return Err(ApiError::needs_elevation(message, command)) };
    let verb = verb.to_string();
    let args: Vec<String> = parts.map(str::to_string).collect();
    if !can_route(&verb) {
        return Err(ApiError::needs_elevation(message, command));
    }
    precheck(ask, &ctx, message)?;
    let mut ch = broker().open(Some(&kora_home())).map_err(|e| admin_error(open_problem(&e, &ctx)))?;
    let mut tail: Vec<String> = Vec::new();
    let res = ch.run(&verb, &args, &env, &mut |l| {
        on_line(l);
        tail.push(l.to_string());
        if tail.len() > 20 {
            tail.remove(0);
        }
    });
    ch.finish();
    match res {
        Ok(o) if o.ok => Ok(json!({"via_service": true, "output": tail})),
        Ok(o) => {
            let last = tail.iter().rev().find(|l| !l.trim().is_empty()).cloned().unwrap_or_default();
            let detail = if last.is_empty() { String::new() } else { format!(" The last thing it said: {last}") };
            Err(ApiError::core(format!("The change could not be made (it stopped with code {}).{detail}", o.code)))
        }
        Err(HelperError::Rejected(m)) => Err(ApiError::core(m)),
        Err(e) => Err(admin_error(lost_problem(&e, &ctx))),
    }
}

/// `privileged_as` for a caller that is always at this computer (toolchain actions).
pub fn privileged_msg(message: &str, command: Vec<String>, env: Vec<(String, String)>, on_line: &mut dyn FnMut(&str)) -> Result<Value, ApiError> {
    privileged_as(AdminAsk::local(), "do this", message, command, env, on_line)
}

/// Windows: start (`"start"`) or install (`"install"`) Kora's own MariaDB through the worker, one
/// prompt (for the panel's "Start database for me"). Elsewhere: the `needs_elevation` answer.
pub fn admin_db(ask: AdminAsk, action: &str, on_line: &mut dyn FnMut(&str)) -> Result<Value, ApiError> {
    let what = if action == "install" { "install Kora's own database server" } else { "start Kora's own database server" };
    privileged_as(ask, what, "Starting the database server needs administrator rights. Run this command as administrator:", vec!["kora".into(), "admin-db".into(), action.into()], vec![], on_line)
}

/// The install's (or a delete's) administrator steps through one worker session: opened on first
/// use (or up front with [`WorkerAdmin::open`]), reused for every later step, closed when dropped.
pub struct WorkerAdmin {
    ask: AdminAsk,
    ctx: Context,
    ch: Mutex<Option<Box<dyn Channel>>>,
}

impl WorkerAdmin {
    pub fn new(ask: AdminAsk, ctx: Context) -> Self {
        Self { ask, ctx, ch: Mutex::new(None) }
    }

    fn ensure(&self, g: &mut Option<Box<dyn Channel>>) -> Result<(), Problem> {
        if g.is_none() {
            if !may_prompt(self.ask) {
                return Err(worker::remote_problem(&self.ctx));
            }
            *g = Some(broker().open(Some(&kora_home())).map_err(|e| open_problem(&e, &self.ctx))?);
        }
        Ok(())
    }

    /// Ask now (the install asks before its first step, so the person answers while they watch).
    pub fn open(&self) -> Result<(), Problem> {
        let mut g = self.ch.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        self.ensure(&mut g)
    }
}

impl AdminRunner for WorkerAdmin {
    fn admin_step(&self, args: &[String], on_line: &mut dyn FnMut(&str)) -> Result<AdminStepOutcome, Problem> {
        let mut g = self.ch.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        self.ensure(&mut g)?;
        let Some(ch) = g.as_mut() else { return Err(worker::open_problem(&OpenError::Unsupported, &self.ctx)) };
        match ch.run("admin-step", args, &[], on_line) {
            Ok(o) => Ok(AdminStepOutcome { ok: o.ok, data: o.data }),
            Err(HelperError::Rejected(m)) => Err(Problem::generic(m)),
            Err(e) => {
                // Gone: the next step (a retry) asks again.
                *g = None;
                Err(lost_problem(&e, &self.ctx))
            }
        }
    }
}

impl Drop for WorkerAdmin {
    fn drop(&mut self) {
        if let Some(ch) = self.ch.get_mut().unwrap_or_else(std::sync::PoisonError::into_inner).take() {
            ch.finish();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A broker that shows a prompt (the old Windows worker) or not (the service).
    struct Fake(bool);
    impl Broker for Fake {
        fn supported(&self) -> bool {
            true
        }
        fn prompts(&self) -> bool {
            self.0
        }
        fn open(&self, _: Option<&str>) -> Result<Box<dyn Channel>, OpenError> {
            Err(OpenError::Unsupported)
        }
    }
    static BROKER_TURN: Mutex<()> = Mutex::new(());

    #[test]
    fn the_service_never_turns_a_remote_session_away_but_the_prompt_does() {
        let _t = BROKER_TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        set_broker_override(Some(Arc::new(Fake(false))));
        assert!(may_prompt(AdminAsk { remote: true, asked: false }), "no prompt to answer: nobody needs to be at the computer");
        set_broker_override(None);
    }

    #[test]
    fn the_prompt_is_only_shown_for_remote_sessions_that_ask() {
        let _t = BROKER_TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        set_broker_override(Some(Arc::new(Fake(true))));
        assert!(may_prompt(AdminAsk { remote: false, asked: false }), "someone at this computer");
        assert!(may_prompt(AdminAsk { remote: false, asked: true }));
        assert!(!may_prompt(AdminAsk { remote: true, asked: false }), "nobody may be there to answer");
        assert!(may_prompt(AdminAsk { remote: true, asked: true }), "the person asked for it");
        set_broker_override(None);
    }

    #[test]
    fn remote_sign_ins_count_as_remote_even_through_a_local_proxy() {
        use crate::remote::{Caller, Identity, Provider, Role};
        let mut h = HeaderMap::new();
        assert_eq!(AdminAsk::of(&Caller::local(), &h), AdminAsk { remote: false, asked: false });
        let ts = Caller { remote: false, peer: None, identity: Some(Identity { name: "sam@".into(), provider: Provider::Tailscale, role: Role::Admin }) };
        assert!(AdminAsk::of(&ts, &h).remote);
        let far = Caller { remote: true, ..Caller::local() };
        assert!(AdminAsk::of(&far, &h).remote);
        h.insert(ASK_HEADER, "ask".parse().unwrap());
        assert_eq!(AdminAsk::of(&far, &h), AdminAsk { remote: true, asked: true });
        h.insert(ASK_HEADER, "yes please".parse().unwrap());
        assert!(!AdminAsk::of(&far, &h).asked, "only the exact value");
    }

    #[test]
    fn permission_problems_are_403_and_a_lost_worker_is_422() {
        for c in [codes::ADMIN_DECLINED, codes::ADMIN_TIMED_OUT, codes::ADMIN_REMOTE, codes::ADMIN_BUSY] {
            assert_eq!(admin_error(Problem::new(c, "m")).status, StatusCode::FORBIDDEN, "{c}");
        }
        assert_eq!(admin_error(Problem::new(codes::ADMIN_LOST, "m")).status, StatusCode::UNPROCESSABLE_ENTITY);
    }
}
