//! Looking at the server from outside, only through services the person agreed to.
//!
//! There is no Kora-hosted service and none is assumed. Two optional outside lookups exist, each
//! behind its own switch in [`super::settings::Consent`] (off by default; the panel says what will be
//! contacted and what it learns before the first use):
//!
//! * [`CfxList`]: Cfx.re's public server list, asked about the server's OWN join code (the part of
//!   the `https://<name>-<code>.users.cfx.re/` address FXServer prints when it registers; txAdmin reads
//!   it the same way, `core/modules/FxRunner/handleFd3Messages.ts`). Cfx.re already knows the server
//!   and its address; Kora only reads what is public. It answers "is it listed?" and "which address
//!   does Cfx.re have?". That is evidence, not proof, that players can connect: the list is fed by the
//!   server's own announcements, and the game uses UDP as well.
//! * [`IpEcho`]: a plain-text "what is my address" website. It learns this connection's address.
//!
//! Neither can say "port 30120 is open" for sure. When no sound answer exists the checklist says
//! "unknown" and explains the manual test, instead of guessing.

use crate::problem::Problem;
use std::io::Read;
use std::net::IpAddr;
use std::time::Duration;

use super::addr::parse_ip_text;

pub const CFX_API: &str = "https://frontend.cfx-services.net/api/servers/single/";
pub const IP_ECHO_URL: &str = "https://api.ipify.org";
const TIMEOUT: Duration = Duration::from_secs(6);
const MAX_BODY: u64 = 2 * 1024 * 1024;

/// What Cfx.re's list says about one join code.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Listing {
    pub code: String,
    /// `ip:port` strings Cfx.re has for the server.
    pub endpoints: Vec<String>,
    pub hostname: Option<String>,
    pub clients: Option<u32>,
    /// When Cfx.re last saw the server (its own timestamp, e.g. `2026-11-17T16:00:41Z`), for "last seen".
    pub last_seen: Option<String>,
}

/// The join code in FXServer's output: the id before `.users.cfx.re` in
/// `https://<name>-<code>.users.cfx.re/`. The newest line wins.
pub fn join_code_from_log(text: &str) -> Option<String> {
    let mut found = None;
    for line in text.lines() {
        let mut from = 0;
        while let Some(i) = line[from..].find(".users.cfx.re") {
            let end = from + i;
            let start = line[..end].rfind(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')).map_or(0, |p| p + 1);
            let host = &line[start..end];
            if let Some((_, code)) = host.rsplit_once('-') {
                if valid_code(code) {
                    found = Some(code.to_string());
                }
            }
            from = end + 1;
        }
    }
    found
}

pub fn valid_code(c: &str) -> bool {
    (6..=12).contains(&c.len()) && c.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

/// `https://frontend.cfx-services.net/api/servers/single/<code>` → [`Listing`]. `None`: not a listing.
pub fn parse_listing(code: &str, body: &str) -> Option<Listing> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let data = v.get("Data")?;
    let endpoints: Vec<String> =
        data.get("connectEndPoints")?.as_array()?.iter().filter_map(|e| e.as_str()).filter(|e| e.len() <= 64 && e.parse::<std::net::SocketAddr>().is_ok()).map(str::to_string).collect();
    Some(Listing {
        code: code.to_string(),
        endpoints,
        hostname: data.get("hostname").and_then(|h| h.as_str()).map(|h| h.chars().filter(|c| !c.is_control()).take(80).collect()),
        clients: data.get("clients").and_then(serde_json::Value::as_u64).and_then(|n| u32::try_from(n).ok()),
        // `lastSeen` is inside `Data` in the single-server answer (verified against a live listing on 2026-11-17: fields hostname, clients, selfReportedClients, connectEndPoints, lastSeen).
        last_seen: v
            .get("Data")
            .and_then(|d| d.get("lastSeen"))
            .and_then(|t| t.as_str())
            .filter(|t| t.len() <= 40 && t.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'-' | b':' | b'T' | b'Z' | b'.' | b'+')))
            .map(str::to_string),
    })
}

/// How a lookup ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup<T> {
    Found(T),
    /// The service answered: there is no such thing.
    Absent,
    /// The service could not be asked (offline, blocked, changed).
    Unavailable(String),
}

pub trait CfxList {
    fn listing(&self, code: &str) -> Lookup<Listing>;
}

pub trait IpEcho {
    fn public_ip(&self) -> Lookup<IpAddr>;
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().redirects(0).timeout(TIMEOUT).timeout_connect(TIMEOUT).user_agent("kora").build()
}

fn read_body(r: ureq::Response) -> Result<String, Problem> {
    let mut s = String::new();
    r.into_reader().take(MAX_BODY).read_to_string(&mut s).map_err(|e| e.to_string())?;
    Ok(s)
}

pub struct SystemCfxList;

impl CfxList for SystemCfxList {
    fn listing(&self, code: &str) -> Lookup<Listing> {
        if !valid_code(code) {
            return Lookup::Absent;
        }
        match agent().get(&format!("{CFX_API}{code}")).call() {
            Ok(r) => match read_body(r) {
                Ok(b) => parse_listing(code, &b).map_or_else(|| Lookup::Unavailable("Cfx.re answered something unexpected".into()), Lookup::Found),
                Err(e) => Lookup::Unavailable(e.one_line()),
            },
            Err(ureq::Error::Status(404, _)) => Lookup::Absent,
            Err(ureq::Error::Status(c, _)) => Lookup::Unavailable(format!("Cfx.re answered with error {c}")),
            Err(ureq::Error::Transport(t)) => Lookup::Unavailable(t.to_string()),
        }
    }
}

pub struct SystemIpEcho;

impl IpEcho for SystemIpEcho {
    fn public_ip(&self) -> Lookup<IpAddr> {
        match agent().get(IP_ECHO_URL).call() {
            Ok(r) => match read_body(r) {
                Ok(b) => parse_ip_text(&b).map_or_else(|| Lookup::Unavailable("the website did not answer with an address".into()), Lookup::Found),
                Err(e) => Lookup::Unavailable(e.one_line()),
            },
            Err(ureq::Error::Status(c, _)) => Lookup::Unavailable(format!("the website answered with error {c}")),
            Err(ureq::Error::Transport(t)) => Lookup::Unavailable(t.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_code_is_read_from_the_nucleus_line() {
        let log = "[ citizen-server-impl] Authenticated with cfx.re Nucleus: https://abcd12-qwerty.users.cfx.re/\nother\n";
        assert_eq!(join_code_from_log(log).as_deref(), Some("qwerty"));
        // The newest wins, colours and text around it do not matter.
        let log = "x https://old-aaaaaa.users.cfx.re/ \n\u{1b}[32mAuthenticated: https://srv-bbbbbb7.users.cfx.re\u{1b}[0m";
        assert_eq!(join_code_from_log(log).as_deref(), Some("bbbbbb7"));
        assert_eq!(join_code_from_log("nothing here"), None);
        assert_eq!(join_code_from_log("https://users.cfx.re/ and .users.cfx.re"), None);
        // Too short, upper case or odd characters are not a code.
        assert_eq!(join_code_from_log("https://a-abc.users.cfx.re/"), None);
        assert_eq!(join_code_from_log("https://a-ABCDEF.users.cfx.re/"), None);
    }

    #[test]
    fn listing_parsing() {
        let body = r#"{"EndPoint":"qwerty","Data":{"hostname":"^5My server","clients":3,"connectEndPoints":["203.0.113.5:30120","not an address"],"lastSeen":"2026-11-17T16:00:41.78Z","players":[{"name":"x"}]}}"#;
        let l = parse_listing("qwerty", body).unwrap();
        assert_eq!(l.endpoints, ["203.0.113.5:30120"]);
        assert_eq!(l.clients, Some(3));
        assert_eq!(l.last_seen.as_deref(), Some("2026-11-17T16:00:41.78Z"));
        assert!(parse_listing("qwerty", "{}").is_none());
        assert!(parse_listing("qwerty", "<html>").is_none());
    }
}
