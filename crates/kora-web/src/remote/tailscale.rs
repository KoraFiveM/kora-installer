//! Who is this peer? Ask the local tailscaled (`whois`). Nothing the request says about itself is used.
//!
//! Linux: HTTP over `/var/run/tailscale/tailscaled.sock`. Anywhere the socket is missing, and on
//! Windows, the `tailscale whois --json` command is used instead. (The Windows localapi needs a
//! password read from tailscaled's own files; the command does that for us.)
use std::net::SocketAddr;

use serde_json::Value;

/// The seam: tests install a fake; the real one is [`SystemWhois`].
pub trait Whois: Send + Sync {
    /// The Tailscale login name (`alice@example.com`) of the machine's user at `peer`.
    /// Blocking; call it inside `spawn_blocking`.
    fn whois(&self, peer: SocketAddr) -> Result<String, String>;
    /// This machine's MagicDNS names (`box.tail1234.ts.net`, `box`), if known. Blocking.
    fn self_names(&self) -> Vec<String> {
        Vec::new()
    }
}

/// No tailscale: every lookup fails.
pub struct NoWhois;
impl Whois for NoWhois {
    fn whois(&self, _peer: SocketAddr) -> Result<String, String> {
        Err("Tailscale is not available on this computer.".into())
    }
}

pub struct SystemWhois;

#[cfg(unix)]
const SOCKET: &str = "/var/run/tailscale/tailscaled.sock";

impl Whois for SystemWhois {
    fn whois(&self, peer: SocketAddr) -> Result<String, String> {
        let body = match localapi(&format!("/localapi/v0/whois?addr={peer}")) {
            Some(Ok(b)) => b,
            Some(Err(e)) => return Err(e),
            None => cli(&["whois", "--json", &peer.ip().to_string()])?,
        };
        parse_whois(&body)
    }
    fn self_names(&self) -> Vec<String> {
        let body = match localapi("/localapi/v0/status") {
            Some(Ok(b)) => b,
            Some(Err(_)) => return Vec::new(),
            None => match cli(&["status", "--json"]) {
                Ok(b) => b,
                Err(_) => return Vec::new(),
            },
        };
        parse_self_names(&body)
    }
}

/// The login name from a whois answer. Tagged machines have no person: they are refused here.
pub fn parse_whois(json: &str) -> Result<String, String> {
    let v: Value = serde_json::from_str(json)
        .map_err(|_| "Tailscale gave an answer Kora could not read.".to_string())?;
    let login = v["UserProfile"]["LoginName"].as_str().unwrap_or("").trim();
    if login.is_empty() || login == "tagged-devices" {
        return Err(
            "Tailscale could not say which person this is (tagged devices have no login).".into(),
        );
    }
    Ok(login.to_string())
}

/// `Self.DNSName` (without the trailing dot) and `Self.HostName` from `status --json`.
pub fn parse_self_names(json: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(d) = v["Self"]["DNSName"].as_str() {
        let d = d.trim().trim_end_matches('.');
        if !d.is_empty() {
            out.push(d.to_string());
        }
    }
    if let Some(h) = v["Self"]["HostName"].as_str() {
        let h = h.trim();
        if !h.is_empty() && !out.iter().any(|o| o.eq_ignore_ascii_case(h)) {
            out.push(h.to_string());
        }
    }
    out
}

/// `None` when there is no localapi socket (try the command); `Some(Err)` when it is there but failed.
#[cfg(unix)]
fn localapi(path: &str) -> Option<Result<String, String>> {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    use std::time::Duration;
    if !std::path::Path::new(SOCKET).exists() {
        return None;
    }
    let run = || -> std::io::Result<String> {
        let mut s = UnixStream::connect(SOCKET)?;
        s.set_read_timeout(Some(Duration::from_secs(3)))?;
        s.set_write_timeout(Some(Duration::from_secs(3)))?;
        // HTTP/1.0: the answer is not chunked and ends when the connection closes.
        write!(
            s,
            "GET {path} HTTP/1.0\r\nHost: local-tailscaled.sock\r\nConnection: close\r\n\r\n"
        )?;
        let mut raw = Vec::new();
        s.take(1 << 20).read_to_end(&mut raw)?;
        let text = String::from_utf8_lossy(&raw).into_owned();
        let (head, body) = text.split_once("\r\n\r\n").unwrap_or((text.as_str(), ""));
        let ok = head
            .lines()
            .next()
            .is_some_and(|l| l.split_whitespace().nth(1) == Some("200"));
        if ok {
            Ok(body.to_string())
        } else {
            Err(std::io::Error::other(format!(
                "tailscaled said: {}",
                head.lines().next().unwrap_or("nothing")
            )))
        }
    };
    Some(run().map_err(|e| format!("Could not ask Tailscale who this is: {e}")))
}

#[cfg(not(unix))]
fn localapi(_path: &str) -> Option<Result<String, String>> {
    None
}

/// Run `tailscale <args>` with a 5 second limit and return its output.
fn cli(args: &[&str]) -> Result<String, String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    let mut c = Command::new("tailscale");
    c.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = c
        .spawn()
        .map_err(|_| "Tailscale is not installed or not running on this computer.".to_string())?;
    let mut out = child.stdout.take().ok_or("no output")?;
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.by_ref().take(1 << 20).read_to_string(&mut s);
        s
    });
    let t0 = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(st)) => {
                let s = reader.join().unwrap_or_default();
                return if st.success() {
                    Ok(s)
                } else {
                    Err("Tailscale does not know this address.".into())
                };
            }
            Ok(None) if t0.elapsed() < Duration::from_secs(5) => {
                std::thread::sleep(Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Tailscale did not answer in time.".into());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whois_json() {
        let j = r#"{"Node":{"Name":"phone.tail.ts.net."},"UserProfile":{"ID":1,"LoginName":"Alice@Example.com","DisplayName":"Alice"}}"#;
        assert_eq!(parse_whois(j).unwrap(), "Alice@Example.com");
        assert!(parse_whois(r#"{"UserProfile":{"LoginName":"tagged-devices"}}"#).is_err());
        assert!(parse_whois("{}").is_err());
        assert!(parse_whois("not json").is_err());
    }

    #[test]
    fn self_names_json() {
        let j = r#"{"Self":{"DNSName":"box.tail1234.ts.net.","HostName":"box"}}"#;
        assert_eq!(
            parse_self_names(j),
            vec!["box.tail1234.ts.net".to_string(), "box".to_string()]
        );
        assert!(parse_self_names("{}").is_empty());
    }
}
