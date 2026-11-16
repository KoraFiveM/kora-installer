//! A client for one node's `/api/v1`. It reads the node's `/meta` first and only offers actions the node
//! lists in `ops` (version skew: v1 is additive only, so unknown fields are ignored and unknown ops refused).
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::transport::{HttpRequest, Transport};

/// What a node says about itself (`GET /meta`, no login needed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meta {
    pub api: u32,
    pub version: String,
    pub platform: String,
    pub node: String,
    pub ops: Vec<String>,
}

impl Meta {
    pub fn supports(&self, op: &str) -> bool {
        self.ops.iter().any(|o| o == op)
    }
    fn parse(v: &Value) -> Result<Meta, RemoteError> {
        let api = v.get("api").and_then(Value::as_u64).ok_or_else(|| {
            RemoteError::Invalid("That address does not look like a Kora panel.".into())
        })?;
        let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let ops = v
            .get("ops")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|o| o.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        Ok(Meta {
            api: api as u32,
            version: text("version"),
            platform: text("platform"),
            node: text("node"),
            ops,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteError {
    /// No answer (cannot connect, timeout, TLS).
    Unreachable(String),
    /// 401/403: the token was refused (or is not allowed to do this).
    Unauthorized(String),
    /// The node is too old (or restricted) to offer this action.
    Unsupported(String),
    /// The node answered with an error status; `code` and `message` are the node's own.
    Status {
        status: u16,
        code: String,
        message: String,
    },
    /// The answer was not what a Kora panel sends.
    Invalid(String),
    /// A job on the node failed or timed out.
    JobFailed(String),
}

impl std::fmt::Display for RemoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RemoteError::Unreachable(m)
            | RemoteError::Unauthorized(m)
            | RemoteError::Unsupported(m)
            | RemoteError::Invalid(m)
            | RemoteError::JobFailed(m) => f.write_str(m),
            RemoteError::Status { message, .. } => f.write_str(message),
        }
    }
}
impl std::error::Error for RemoteError {}

pub struct RemoteBackend {
    base: String,
    token: String,
    transport: Arc<dyn Transport>,
    meta: Mutex<Option<Meta>>,
    /// How long to wait between job polls, and for a job in total.
    pub poll_every: Duration,
    pub job_timeout: Duration,
}

impl RemoteBackend {
    /// `url` is the checked panel address (no trailing slash), without `/api/v1`.
    pub fn new(url: &str, token: &str, transport: Arc<dyn Transport>) -> Self {
        Self {
            base: url.trim_end_matches('/').to_string(),
            token: token.to_string(),
            transport,
            meta: Mutex::new(None),
            poll_every: Duration::from_millis(500),
            job_timeout: Duration::from_secs(15 * 60),
        }
    }

    fn call(
        &self,
        method: &'static str,
        path: &str,
        body: Option<Value>,
        auth: bool,
    ) -> Result<Value, RemoteError> {
        let req = HttpRequest {
            method,
            url: format!("{}/api/v1{path}", self.base),
            token: auth.then(|| self.token.clone()),
            body,
        };
        let r = self
            .transport
            .send(&req)
            .map_err(RemoteError::Unreachable)?;
        if (200..300).contains(&r.status) {
            return Ok(r.body);
        }
        let code = r.body["error"]["code"].as_str().unwrap_or("").to_string();
        let message = r.body["error"]["message"].as_str().map(String::from);
        Err(match r.status {
            401 => RemoteError::Unauthorized("The node refused the token. Create a new one on that panel with 'kora panel user add'.".into()),
            403 => RemoteError::Unauthorized(message.unwrap_or_else(|| "That node does not allow this with the token you gave.".into())),
            s => RemoteError::Status { status: s, code, message: message.unwrap_or_else(|| format!("The node answered with an error (HTTP {s}).")) },
        })
    }

    /// `GET /meta`, cached after the first success.
    pub fn meta(&self) -> Result<Meta, RemoteError> {
        if let Some(m) = self.meta.lock().unwrap().clone() {
            return Ok(m);
        }
        let m = Meta::parse(&self.call("GET", "/meta", None, false)?)?;
        if m.api < 1 {
            return Err(RemoteError::Invalid(
                "That address does not look like a Kora panel.".into(),
            ));
        }
        *self.meta.lock().unwrap() = Some(m.clone());
        Ok(m)
    }

    /// Refuse with a clear message when the node does not list `op`.
    pub fn require(&self, op: &str) -> Result<Meta, RemoteError> {
        let m = self.meta()?;
        if m.supports(op) {
            Ok(m)
        } else {
            Err(RemoteError::Unsupported(format!(
                "That node (Kora {}) does not support this yet. Update it to use this from here.",
                if m.version.is_empty() {
                    "?"
                } else {
                    &m.version
                }
            )))
        }
    }

    pub fn servers_list(&self) -> Result<Vec<Value>, RemoteError> {
        self.require("servers_list")?;
        let v = self.call("GET", "/servers", None, true)?;
        Ok(v["servers"].as_array().cloned().unwrap_or_default())
    }

    pub fn server_get(&self, server: &str) -> Result<Value, RemoteError> {
        self.require("server_get")?;
        self.call("GET", &format!("/servers/{}", enc(server)), None, true)
    }

    pub fn jobs_get(&self, id: &str) -> Result<Value, RemoteError> {
        self.require("jobs_get")?;
        self.call("GET", &format!("/jobs/{}", enc(id)), None, true)
    }

    /// Start/stop/restart and wait for the node's job. `on_event` gets each new job event.
    /// Returns the job's result.
    pub fn server_control(
        &self,
        server: &str,
        action: &str,
        on_event: &mut dyn FnMut(&Value),
    ) -> Result<Value, RemoteError> {
        self.require("server_control")?;
        self.require("jobs_get")?;
        let started = self.call(
            "POST",
            &format!("/servers/{}/control", enc(server)),
            Some(json!({ "action": action })),
            true,
        )?;
        self.wait_job(&started, on_event)
    }

    /// Check for updates on a node's server (a read-only job). Returns the job's result.
    pub fn update_preview(
        &self,
        server: &str,
        scope: Value,
        on_event: &mut dyn FnMut(&Value),
    ) -> Result<Value, RemoteError> {
        self.require("update_preview")?;
        self.require("jobs_get")?;
        let started = self.call(
            "POST",
            &format!("/servers/{}/update/preview", enc(server)),
            Some(scope),
            true,
        )?;
        self.wait_job(&started, on_event)
    }

    fn wait_job(
        &self,
        started: &Value,
        on_event: &mut dyn FnMut(&Value),
    ) -> Result<Value, RemoteError> {
        let id = started["job"]
            .as_str()
            .ok_or_else(|| RemoteError::Invalid("The node did not start the job.".into()))?
            .to_string();
        let t0 = Instant::now();
        let mut seen = 0usize;
        loop {
            let job = self.jobs_get(&id)?;
            if let Some(evs) = job["events"].as_array() {
                for e in evs.iter().skip(seen) {
                    on_event(e);
                }
                seen = seen.max(evs.len());
            }
            match job["state"].as_str() {
                Some("done") => return Ok(job["result"].clone()),
                Some("failed") => {
                    let e = &job["error"];
                    let msg = e["message"]
                        .as_str()
                        .or_else(|| e.as_str())
                        .unwrap_or("The job failed on the node.");
                    return Err(RemoteError::JobFailed(msg.to_string()));
                }
                _ => {}
            }
            if t0.elapsed() > self.job_timeout {
                return Err(RemoteError::JobFailed(
                    "The node is taking too long; check it directly.".into(),
                ));
            }
            std::thread::sleep(self.poll_every);
        }
    }
}

/// Percent-encode one path segment.
fn enc(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::transport::HttpResponse;

    struct Fake(Mutex<Vec<(String, HttpResponse)>>, Mutex<Vec<HttpRequest>>);
    impl Transport for Fake {
        fn send(&self, r: &HttpRequest) -> Result<HttpResponse, String> {
            self.1.lock().unwrap().push(r.clone());
            let q = self.0.lock().unwrap();
            let i = q
                .iter()
                .position(|(u, _)| r.url.ends_with(u.as_str()))
                .ok_or("no route")?;
            Ok(q[i].1.clone())
        }
    }
    fn ok(v: Value) -> HttpResponse {
        HttpResponse {
            status: 200,
            body: v,
        }
    }

    #[test]
    fn skew_refuses_unknown_ops_and_sends_token_only_when_needed() {
        let f = Arc::new(Fake(
            Mutex::new(vec![
                (
                    "/api/v1/meta".into(),
                    ok(
                        json!({"api":1,"version":"0.3.9","ops":["meta","servers_list"],"future":true}),
                    ),
                ),
                (
                    "/api/v1/servers".into(),
                    ok(json!({"servers":[{"name":"a"}]})),
                ),
            ]),
            Mutex::default(),
        ));
        let r = RemoteBackend::new("http://h:1/", "tok", f.clone());
        assert_eq!(r.servers_list().unwrap().len(), 1);
        let e = r.server_control("a", "start", &mut |_| {}).unwrap_err();
        assert!(matches!(e, RemoteError::Unsupported(_)), "{e}");
        let reqs = f.1.lock().unwrap();
        assert_eq!(reqs[0].token, None);
        assert_eq!(reqs[1].token.as_deref(), Some("tok"));
        assert!(!reqs.iter().any(|q| q.method == "POST"));
    }
}
