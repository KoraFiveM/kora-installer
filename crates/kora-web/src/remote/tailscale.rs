//! Who is this peer? Ask the local tailscaled (`whois`). Nothing the request says about itself is used.
//!
//! Linux: HTTP over `/var/run/tailscale/tailscaled.sock`. Anywhere the socket is missing, and on
//! Windows, the `tailscale whois --json` command is used instead. (The Windows localapi needs a
//! password read from tailscaled's own files; the command does that for us.)
use std::net::{IpAddr, SocketAddr};

use serde_json::Value;

/// This machine as Tailscale sees it (`status`): enough to listen on the tailnet and to say which
/// address to open.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TsSelf {
    /// Tailscale is up and signed in (`BackendState` is `Running`).
    pub running: bool,
    /// This machine's Tailscale addresses (`100.x.y.z`, `fd7a:115c:a1e0::...`).
    pub ips: Vec<IpAddr>,
    /// The MagicDNS name without the trailing dot (`box.tail1234.ts.net`), when MagicDNS is on.
    pub dns_name: Option<String>,
    /// The short machine name (`box`).
    pub host_name: Option<String>,
    /// The login of the account that added this machine to the tailnet. `None` for tagged machines
    /// (added with an auth key and tags), which belong to no person.
    pub owner: Option<String>,
}

impl TsSelf {
    /// Names and addresses a browser on the tailnet may use for this machine (for the `Host` check).
    pub fn host_names(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for n in self.dns_name.iter().chain(self.host_name.iter()) {
            if !v.iter().any(|x| x.eq_ignore_ascii_case(n)) {
                v.push(n.clone());
            }
        }
        v.extend(self.ips.iter().map(IpAddr::to_string));
        v
    }
    /// The address to tell people to open: the MagicDNS name when there is one, else the first IPv4.
    pub fn url(&self, port: u16) -> Option<String> {
        if let Some(d) = &self.dns_name {
            return Some(format!("http://{d}:{port}"));
        }
        let ip = self.ips.iter().find(|i| i.is_ipv4()).or(self.ips.first())?;
        Some(match ip {
            IpAddr::V4(v4) => format!("http://{v4}:{port}"),
            IpAddr::V6(v6) => format!("http://[{v6}]:{port}"),
        })
    }
}

/// Is `ip` a Tailscale address (`100.64.0.0/10` or `fd7a:115c:a1e0::/48`)?
pub fn is_tailnet_ip(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            o[0] == 100 && (o[1] & 0xc0) == 64
        }
        IpAddr::V6(v6) => {
            let s = v6.segments();
            s[0] == 0xfd7a && s[1] == 0x115c && s[2] == 0xa1e0
        }
    }
}

/// The seam: tests install a fake; the real one is [`SystemWhois`].
pub trait Whois: Send + Sync {
    /// The Tailscale login name (`alice@example.com`) of the machine's user at `peer`.
    /// Blocking; call it inside `spawn_blocking`.
    fn whois(&self, peer: SocketAddr) -> Result<String, String>;
    /// This machine's MagicDNS names (`box.tail1234.ts.net`, `box`), if known. Blocking.
    fn self_names(&self) -> Vec<String> {
        self.status().map(|s| s.host_names()).unwrap_or_default()
    }
    /// This machine's Tailscale state, or `None` when Tailscale is not installed or not answering. Blocking.
    fn status(&self) -> Option<TsSelf> {
        None
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
    fn status(&self) -> Option<TsSelf> {
        let body = match localapi("/localapi/v0/status") {
            Some(Ok(b)) => b,
            Some(Err(_)) => return None,
            None => cli(&["status", "--json"]).ok()?,
        };
        parse_status(&body)
    }
}

/// Read `status --json` (the same shape as the localapi answer).
pub fn parse_status(json: &str) -> Option<TsSelf> {
    let v: Value = serde_json::from_str(json).ok()?;
    let me = &v["Self"];
    if !me.is_object() {
        return None;
    }
    let ips = me["TailscaleIPs"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str()?.parse::<IpAddr>().ok()).collect())
        .unwrap_or_default();
    let dns_name = me["DNSName"]
        .as_str()
        .map(|d| d.trim().trim_end_matches('.').to_string())
        .filter(|d| d.contains('.'));
    let host_name = me["HostName"].as_str().map(|h| h.trim().to_string()).filter(|h| !h.is_empty());
    let tagged = me["Tags"].as_array().is_some_and(|t| !t.is_empty());
    // `UserID` is a number; the `User` map is keyed by it as a string.
    let owner = if tagged {
        None
    } else {
        me["UserID"].as_u64().and_then(|id| {
            let login = v["User"][id.to_string()]["LoginName"].as_str()?.trim();
            (!login.is_empty() && login != "tagged-devices").then(|| login.to_string())
        })
    };
    Some(TsSelf {
        running: v["BackendState"].as_str() == Some("Running"),
        ips,
        dns_name,
        host_name,
        owner,
    })
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

    #[test]
    fn status_json() {
        let j = r#"{"BackendState":"Running","Self":{"DNSName":"box.tail1234.ts.net.","HostName":"box",
            "TailscaleIPs":["100.101.102.103","fd7a:115c:a1e0::1"],"UserID":42},
            "User":{"42":{"LoginName":"Alice@Example.com"}}}"#;
        let s = parse_status(j).unwrap();
        assert!(s.running);
        assert_eq!(s.owner.as_deref(), Some("Alice@Example.com"));
        assert_eq!(s.ips.len(), 2);
        assert_eq!(s.url(47120).unwrap(), "http://box.tail1234.ts.net:47120");
        assert_eq!(
            s.host_names(),
            vec!["box.tail1234.ts.net", "box", "100.101.102.103", "fd7a:115c:a1e0::1"]
        );
        // Tagged machine (auth key + tags): nobody owns it.
        let j = r#"{"BackendState":"Running","Self":{"HostName":"srv","TailscaleIPs":["100.64.0.9"],
            "UserID":7,"Tags":["tag:server"]},"User":{"7":{"LoginName":"tagged-devices"}}}"#;
        let s = parse_status(j).unwrap();
        assert_eq!(s.owner, None);
        assert_eq!(s.dns_name, None, "a bare host name is not a MagicDNS name");
        assert_eq!(s.url(1).unwrap(), "http://100.64.0.9:1");
        // Installed but signed out.
        let s = parse_status(r#"{"BackendState":"NeedsLogin","Self":{"TailscaleIPs":[]}}"#).unwrap();
        assert!(!s.running && s.ips.is_empty() && s.url(1).is_none());
        assert!(parse_status("{}").is_none() && parse_status("nope").is_none());
    }

    #[test]
    fn tailnet_addresses() {
        for ok in ["100.64.0.1", "100.127.255.254", "fd7a:115c:a1e0::5", "::ffff:100.100.1.1"] {
            assert!(is_tailnet_ip(ok.parse().unwrap()), "{ok}");
        }
        for no in ["100.63.255.255", "100.128.0.1", "10.0.0.1", "8.8.8.8", "fd7a:115c:a1e1::1", "::1"] {
            assert!(!is_tailnet_ip(no.parse().unwrap()), "{no}");
        }
    }
}
