//! Privileged actions for the panel. Windows: through the short-lived elevated worker (one
//! Windows permission prompt; see the second half of this file). Linux: through the helper when it
//! is installed, otherwise the `needs_elevation` answer with the command to run (exactly as before
//! the helper existed).
//!
//! `privileged` takes the `kora ...` command the handler used to hand to the user. The first word
//! (`kora`) is dropped, the next is the helper verb, the rest are its arguments. Secrets (licence
//! key, database password) go in `env`, never in the command.

use std::sync::{Arc, Mutex};

use kora_core::helper::client::{Helper, HelperError, SocketClient};
use serde_json::{json, Value};

use crate::error::ApiError;
use axum::http::{HeaderMap, StatusCode};
use kora_core::helper::worker::{self, Context, OpenError, Session, StepOutcome};
use kora_core::installer::system_ops::{AdminRunner, AdminStepOutcome};
use kora_core::problem::{codes, Problem};

type Slot = Mutex<Option<Arc<dyn Helper>>>;
static OVERRIDE: Slot = Mutex::new(None);

/// Test seam: use this client instead of the real socket (`None` = the real one again).
pub fn set_helper_override(h: Option<Arc<dyn Helper>>) {
    *OVERRIDE.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = h;
}

fn client() -> Arc<dyn Helper> {
    OVERRIDE.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone().unwrap_or_else(|| Arc::new(SocketClient::default()))
}

/// Is the helper installed and answering? (`GET /system` `helper_installed`.)
pub fn helper_installed() -> bool {
    client().available()
}

const DEFAULT_MESSAGE: &str = "This needs administrator rights. Run this command as administrator:";

/// Run `command` (`["kora", verb, args...]`) with administrator rights.
/// Helper present: runs it there, `on_line` receives each output line, returns
/// `{"via_helper": true, "output": [last lines]}`. No helper: `needs_elevation`.
pub fn privileged(command: Vec<String>, env: Vec<(String, String)>, on_line: &mut dyn FnMut(&str)) -> Result<Value, ApiError> {
    privileged_msg(DEFAULT_MESSAGE, command, env, on_line)
}

/// [`privileged`] with the handler's own `needs_elevation` message.
pub fn privileged_msg(message: &str, command: Vec<String>, env: Vec<(String, String)>, on_line: &mut dyn FnMut(&str)) -> Result<Value, ApiError> {
    let needs = |c: Vec<String>| ApiError::needs_elevation(message, c);
    let h = client();
    let mut parts = command.iter().map(String::as_str);
    if parts.next() != Some("kora") {
        return Err(needs(command));
    }
    let Some(verb) = parts.next().map(str::to_string) else { return Err(needs(command)) };
    let args: Vec<String> = parts.map(str::to_string).collect();
    if !h.available() {
        return Err(needs(command));
    }
    let mut tail: Vec<String> = Vec::new();
    let res = h.run(&verb, &args, &env, &mut |l| {
        on_line(l);
        tail.push(l.to_string());
        if tail.len() > 20 {
            tail.remove(0);
        }
    });
    match res {
        Ok(o) if o.ok => Ok(json!({"via_helper": true, "output": tail})),
        Ok(o) => {
            let last = tail.iter().rev().find(|l| !l.trim().is_empty()).cloned().unwrap_or_default();
            let detail = if last.is_empty() { String::new() } else { format!(" The last thing it said: {last}") };
            Err(ApiError::core(format!("The change could not be made (it stopped with code {}).{detail}", o.code)))
        }
        // The helper went away between the check and the run: fall back to the command.
        Err(HelperError::Unavailable(_)) => Err(needs(command)),
        Err(HelperError::Rejected(m) | HelperError::Protocol(m)) => Err(ApiError::core(m)),
    }
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
    !ask.remote || ask.asked
}

/// Opens worker sessions (the real one shows the Windows prompt; tests use a fake).
pub trait Broker: Send + Sync {
    /// Can this panel ask for administrator rights itself (Windows, not elevated)?
    fn supported(&self) -> bool;
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

struct SystemBroker;

impl Broker for SystemBroker {
    fn supported(&self) -> bool {
        cfg!(windows)
    }
    fn open(&self, kora_home: Option<&str>) -> Result<Box<dyn Channel>, OpenError> {
        kora_core::helper::winpipe::open_session(kora_home, worker::PROMPT_LIMIT).map(|s| Box::new(s) as Box<dyn Channel>)
    }
}

type BrokerSlot = Mutex<Option<Arc<dyn Broker>>>;
static BROKER: BrokerSlot = Mutex::new(None);

/// Test seam: use this broker instead of the real prompt (`None` = the real one again).
pub fn set_broker_override(b: Option<Arc<dyn Broker>>) {
    *BROKER.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = b;
}

fn broker() -> Arc<dyn Broker> {
    BROKER.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone().unwrap_or_else(|| Arc::new(SystemBroker))
}

/// Does this panel ask Windows for administrator rights itself (`GET /system` `admin_prompt`)?
pub fn worker_supported() -> bool {
    broker().supported()
}

fn kora_home() -> String {
    kora_core::installer::registry::kora_home().to_string_lossy().into_owned()
}

fn problem_status(p: &Problem) -> StatusCode {
    if [codes::ADMIN_DECLINED, codes::ADMIN_TIMED_OUT, codes::ADMIN_REMOTE, codes::ADMIN_BUSY, codes::NEEDS_ELEVATION].contains(&p.code.as_str()) {
        StatusCode::FORBIDDEN
    } else {
        StatusCode::UNPROCESSABLE_ENTITY
    }
}

/// The answer for a Problem from asking (403 for the "permission" ones).
pub fn admin_error(p: Problem) -> ApiError {
    ApiError::problem(problem_status(&p), p)
}

/// Before starting a job that needs administrator rights: `Ok` = it can go ahead (the Windows
/// worker may prompt, or the Linux helper is there); `Err` = the answer to give instead (a remote
/// session that did not ask; no way to elevate: the command to run).
pub fn precheck(ask: AdminAsk, ctx: &Context, message: &str) -> Result<(), ApiError> {
    if worker_supported() {
        if may_prompt(ask) {
            return Ok(());
        }
        return Err(admin_error(worker::remote_problem(ctx)));
    }
    if helper_installed() {
        return Ok(());
    }
    let mut e = ApiError::needs_elevation(message, ctx.command.clone());
    if ctx.can_skip_admin {
        e = e.with_action(kora_core::problem::Action::ContinueWithoutAdmin);
    }
    Err(e)
}

/// [`privileged_msg`] that also works on Windows: there `command` runs in the elevated worker
/// after one prompt (if this request may show it). `what` completes "Kora could not get permission to ...".
pub fn privileged_as(ask: AdminAsk, what: &str, message: &str, command: Vec<String>, env: Vec<(String, String)>, on_line: &mut dyn FnMut(&str)) -> Result<Value, ApiError> {
    if !worker_supported() {
        return privileged_msg(message, command, env, on_line);
    }
    let ctx = Context { what: what.to_string(), command: command.clone(), can_skip_admin: false };
    if !may_prompt(ask) {
        return Err(admin_error(worker::remote_problem(&ctx)));
    }
    let mut parts = command.iter().map(String::as_str);
    let (Some("kora"), Some(verb)) = (parts.next(), parts.next()) else { return Err(ApiError::needs_elevation(message, command)) };
    let verb = verb.to_string();
    let args: Vec<String> = parts.map(str::to_string).collect();
    let mut ch = broker().open(Some(&kora_home())).map_err(|e| admin_error(worker::open_problem(&e, &ctx)))?;
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
        Ok(o) if o.ok => Ok(json!({"via_worker": true, "output": tail})),
        Ok(o) => {
            let last = tail.iter().rev().find(|l| !l.trim().is_empty()).cloned().unwrap_or_default();
            let detail = if last.is_empty() { String::new() } else { format!(" The last thing it said: {last}") };
            Err(ApiError::core(format!("The change could not be made (it stopped with code {}).{detail}", o.code)))
        }
        Err(HelperError::Rejected(m)) => Err(ApiError::core(m)),
        Err(e) => Err(admin_error(worker::lost_problem(&e))),
    }
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
            *g = Some(broker().open(Some(&kora_home())).map_err(|e| worker::open_problem(&e, &self.ctx))?);
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
                Err(worker::lost_problem(&e))
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

    #[test]
    fn the_prompt_is_only_shown_for_remote_sessions_that_ask() {
        assert!(may_prompt(AdminAsk { remote: false, asked: false }), "someone at this computer");
        assert!(may_prompt(AdminAsk { remote: false, asked: true }));
        assert!(!may_prompt(AdminAsk { remote: true, asked: false }), "nobody may be there to answer");
        assert!(may_prompt(AdminAsk { remote: true, asked: true }), "the person asked for it");
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
