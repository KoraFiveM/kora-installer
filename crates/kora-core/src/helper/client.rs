//! The helper's client side (used by the panel). Everything the panel needs is behind the
//! [`Helper`] trait so tests can swap in a fake.

use super::Message;

/// How a privileged run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub ok: bool,
    pub code: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelperError {
    /// Nobody is listening (helper not installed or stopped).
    Unavailable(String),
    /// The helper answered but refused the request (plain-language reason).
    Rejected(String),
    /// The conversation broke off.
    Protocol(String),
}

impl std::fmt::Display for HelperError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HelperError::Unavailable(m) | HelperError::Rejected(m) | HelperError::Protocol(m) => f.write_str(m),
        }
    }
}

pub trait Helper: Send + Sync {
    /// Is a helper installed and answering?
    fn available(&self) -> bool;
    /// Run one allow-listed command; `on_line` gets each output line.
    fn run(&self, verb: &str, args: &[String], env: &[(String, String)], on_line: &mut dyn FnMut(&str)) -> Result<Outcome, HelperError>;
}

/// Talks to the real helper over its socket.
#[derive(Debug, Clone)]
pub struct SocketClient {
    pub path: std::path::PathBuf,
}

impl Default for SocketClient {
    fn default() -> Self {
        SocketClient { path: super::SOCKET_PATH.into() }
    }
}

/// Parse one answer line.
pub fn parse_message(line: &str) -> Result<Message, HelperError> {
    serde_json::from_str(line.trim()).map_err(|_| HelperError::Protocol("The Kora helper sent something unexpected.".into()))
}

#[cfg(unix)]
impl SocketClient {
    fn converse(&self, req: &super::Request, timeout: Option<std::time::Duration>, on_line: &mut dyn FnMut(&str)) -> Result<(Outcome, Option<String>), HelperError> {
        use std::io::{BufRead, BufReader, Write};
        let mut s = std::os::unix::net::UnixStream::connect(&self.path).map_err(|e| HelperError::Unavailable(format!("The Kora helper is not running ({e}).")))?;
        s.set_read_timeout(timeout).ok();
        let mut line = serde_json::to_string(req).map_err(|e| HelperError::Protocol(e.to_string()))?;
        line.push('\n');
        s.write_all(line.as_bytes()).map_err(|e| HelperError::Unavailable(format!("The Kora helper did not accept the request ({e}).")))?;
        for l in BufReader::new(&s).lines() {
            let l = l.map_err(|e| HelperError::Protocol(format!("The Kora helper stopped answering ({e}).")))?;
            match parse_message(&l)? {
                Message::Progress { line, .. } => on_line(&line),
                Message::Result { ok, code, version, .. } => return Ok((Outcome { ok, code }, version)),
                Message::Error { message } => return Err(HelperError::Rejected(message)),
                Message::Hello { .. } => {}
            }
        }
        Err(HelperError::Protocol("The Kora helper closed the connection before it finished.".into()))
    }
}

#[cfg(unix)]
impl Helper for SocketClient {
    fn available(&self) -> bool {
        let ping = super::Request { verb: super::PING.into(), ..Default::default() };
        self.converse(&ping, Some(std::time::Duration::from_secs(3)), &mut |_| {}).is_ok()
    }

    fn run(&self, verb: &str, args: &[String], env: &[(String, String)], on_line: &mut dyn FnMut(&str)) -> Result<Outcome, HelperError> {
        let req = super::Request { verb: verb.into(), args: args.to_vec(), env: env.iter().cloned().collect() };
        self.converse(&req, None, on_line).map(|(o, _)| o)
    }
}

#[cfg(unix)]
impl SocketClient {
    /// The helper's version, when it answers.
    pub fn version(&self) -> Option<String> {
        let ping = super::Request { verb: super::PING.into(), ..Default::default() };
        self.converse(&ping, Some(std::time::Duration::from_secs(3)), &mut |_| {}).ok().and_then(|(_, v)| v)
    }
}

#[cfg(not(unix))]
impl Helper for SocketClient {
    fn available(&self) -> bool {
        false
    }
    fn run(&self, _: &str, _: &[String], _: &[(String, String)], _: &mut dyn FnMut(&str)) -> Result<Outcome, HelperError> {
        Err(HelperError::Unavailable("The Kora helper is not available on Windows yet.".into()))
    }
}

#[cfg(not(unix))]
impl SocketClient {
    pub fn version(&self) -> Option<String> {
        None
    }
}
