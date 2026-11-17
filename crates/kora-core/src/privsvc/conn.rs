//! The transport-independent part of the service: serving one connection (a request line in,
//! progress lines and one result out) and the client's side of the same conversation. The Linux
//! socket ([`super::unixsvc`]) and the Windows named pipe (`super::winsvc`) only supply the byte
//! streams and the caller's identity.

use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::dispatch::{self, Host, Progress, Ran, Reply, RunSpec};
use super::policy::{Caller, Policy};
use crate::helper::{Message, Request, PING};

/// What one served connection needs besides the OS facts.
pub trait Svc: Send + Sync {
    fn host(&self) -> &dyn Host;
    /// The current settings (read at every connection).
    fn policy(&self) -> Result<Policy, String>;
    /// The service must exit once this answer is sent (an update was installed: the service manager starts the new program).
    fn restart_requested(&self) -> bool {
        super::update::restart_requested()
    }
    /// Run the child for `caller`.
    fn run(&self, caller: &Caller, spec: &RunSpec<'_>, progress: Progress<'_>, cancel: &Arc<AtomicBool>) -> std::io::Result<Ran>;
}

fn send(out: &mut dyn Write, m: &Message) -> bool {
    let mut line = serde_json::to_string(m).unwrap_or_default();
    line.push('\n');
    out.write_all(line.as_bytes()).and_then(|_| out.flush()).is_ok()
}

/// Serve one request on an already identified connection. `reader` must stop at
/// [`super::op::MAX_LINE`] + 1 bytes (the transport caps it); a request that is too long, not
/// valid JSON or not ended by a newline is refused unread further.
pub fn handle_stream(svc: &dyn Svc, reader: &mut dyn BufRead, out: &mut dyn Write, caller: Caller) {
    let mut line = String::new();
    let n = reader.read_line(&mut line);
    if n.is_err() || line.len() > super::op::MAX_LINE || !line.ends_with('\n') {
        send(out, &Message::Error { message: "The request was not understood.".into() });
        return;
    }
    let Ok(req) = serde_json::from_str::<Request>(line.trim()) else {
        send(out, &Message::Error { message: "The request was not understood.".into() });
        return;
    };
    let policy = svc.policy();
    if req.verb == PING {
        let answer = super::policy::authorise(&caller, &policy);
        send(out, &Message::Result { ok: answer.is_ok(), code: 0, message: answer.err().map(|r| r.0), version: Some(env!("CARGO_PKG_VERSION").into()), data: None });
        return;
    }
    let secrets: Vec<String> = req.env.values().cloned().collect();
    let cancel = Arc::new(AtomicBool::new(false));
    let mut alive = true;
    let mut data = None;
    let run = |spec: &RunSpec<'_>, progress: Progress<'_>| svc.run(&caller, spec, progress, &cancel);
    let mut progress = |stream_name: &str, text: &str| {
        if stream_name == "stdout" {
            if let Some(v) = crate::helper::worker::result_line(text) {
                data = Some(v);
                return;
            }
        }
        if alive {
            let masked = crate::helper::mask_secrets(text, &secrets.iter().map(String::as_str).collect::<Vec<_>>());
            alive = send(out, &Message::Progress { stream: stream_name.to_string(), line: masked });
            if !alive {
                cancel.store(true, Ordering::SeqCst);
            }
        }
    };
    let reply = dispatch::handle(svc.host(), &policy, &caller, &req.verb, &req.args, &req.env, &run, &mut progress);
    let m = match reply {
        Reply::Refused(r) => Message::Error { message: r.0 },
        Reply::CouldNotRun(e) => Message::Error { message: format!("Kora's service could not run that: {e}") },
        Reply::Ran(r) if r.timed_out => Message::Result { ok: false, code: -1, message: Some("The step took too long and was stopped.".into()), version: None, data: None },
        Reply::Ran(r) => Message::Result { ok: r.code == 0, code: r.code, message: None, version: None, data: r.data.or(data) },
    };
    send(out, &m);
}

// ---- the client's side ---------------------------------------------------------------------------

/// How a request ended, from the client's side.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    pub ok: bool,
    pub code: i32,
    pub message: Option<String>,
    pub version: Option<String>,
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientError {
    /// Nobody is listening, or it is not the real service.
    Unavailable(String),
    /// The service answered with a refusal.
    Refused(String),
    Broken(String),
}

/// Send one request and read the answer, handing each progress line to `on_line`.
pub fn converse(reader: &mut dyn BufRead, writer: &mut dyn Write, verb: &str, args: &[String], env: &[(String, String)], on_line: &mut dyn FnMut(&str)) -> Result<Answer, ClientError> {
    let req = Request { verb: verb.into(), args: args.to_vec(), env: env.iter().cloned().collect::<BTreeMap<_, _>>() };
    let mut line = serde_json::to_string(&req).map_err(|e| ClientError::Broken(e.to_string()))?;
    line.push('\n');
    writer.write_all(line.as_bytes()).and_then(|_| writer.flush()).map_err(|e| ClientError::Broken(e.to_string()))?;
    loop {
        let mut l = String::new();
        let n = reader.read_line(&mut l).map_err(|e| ClientError::Broken(format!("Kora's administrator service stopped answering ({e}).")))?;
        if n == 0 {
            return Err(ClientError::Broken("Kora's administrator service stopped answering.".into()));
        }
        match serde_json::from_str::<Message>(l.trim()) {
            Ok(Message::Progress { line, .. }) => on_line(&line),
            Ok(Message::Result { ok, code, message, version, data }) => return Ok(Answer { ok, code, message, version, data }),
            Ok(Message::Error { message }) => return Err(ClientError::Refused(message)),
            _ => return Err(ClientError::Broken("Kora's administrator service sent something unexpected.".into())),
        }
    }
}

/// The version when the service serves this account, else why not.
pub fn interpret_ping(a: Result<Answer, ClientError>) -> Result<String, ClientError> {
    let a = a?;
    if a.ok {
        Ok(a.version.unwrap_or_default())
    } else {
        Err(ClientError::Refused(a.message.unwrap_or_else(|| "Kora's administrator service is not turned on for this account.".into())))
    }
}
