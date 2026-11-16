//! The HTTP seam for talking to a node. Tests plug in a fake; the real one is [`UreqTransport`].
use std::time::Duration;

use serde_json::Value;

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Longest body we read from a node (a server list is a few KB).
const MAX_BODY: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct HttpRequest {
    /// `GET`, `POST`, `PUT` or `DELETE`.
    pub method: &'static str,
    pub url: String,
    /// Sent as `Authorization: Bearer <token>` when set.
    pub token: Option<String>,
    pub body: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    /// `Value::Null` when the body was empty or not JSON.
    pub body: Value,
}

pub trait Transport: Send + Sync {
    /// `Err` only when no HTTP answer arrived (cannot connect, timeout, TLS failure); the text is for people.
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse, String>;
}

/// Real transport: `ureq` with the hub's timeouts, no redirects (a redirect would leak the token).
pub struct UreqTransport;

impl Transport for UreqTransport {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse, String> {
        let agent = ureq::AgentBuilder::new()
            .redirects(0)
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .user_agent("kora-hub")
            .build();
        let mut r = agent
            .request(req.method, &req.url)
            .set("Accept", "application/json");
        if let Some(t) = &req.token {
            r = r.set("Authorization", &format!("Bearer {t}"));
        }
        let out = match &req.body {
            Some(b) => r
                .set("Content-Type", "application/json")
                .send_string(&b.to_string()),
            None if req.method == "POST" || req.method == "PUT" => {
                r.set("Content-Type", "application/json").send_string("{}")
            }
            None => r.call(),
        };
        let resp = match out {
            Ok(r) => r,
            Err(ureq::Error::Status(_, r)) => r,
            Err(ureq::Error::Transport(t)) => return Err(describe(&t)),
        };
        let status = resp.status();
        let mut text = String::new();
        std::io::Read::read_to_string(
            &mut std::io::Read::take(resp.into_reader(), MAX_BODY),
            &mut text,
        )
        .map_err(|e| format!("The node's answer could not be read ({e})."))?;
        Ok(HttpResponse {
            status,
            body: serde_json::from_str(&text).unwrap_or(Value::Null),
        })
    }
}

fn describe(t: &ureq::Transport) -> String {
    let s = t.to_string().to_lowercase();
    if s.contains("timed out") || s.contains("timeout") {
        "The node did not answer in time.".into()
    } else if s.contains("refused") {
        "The node refused the connection (is the panel running there?).".into()
    } else if s.contains("dns") || s.contains("resolve") || s.contains("lookup") {
        "The node's address could not be found.".into()
    } else if s.contains("tls") || s.contains("certificate") || s.contains("handshake") {
        "A secure connection to the node could not be made (check its certificate).".into()
    } else {
        format!("The node could not be reached ({t}).")
    }
}
