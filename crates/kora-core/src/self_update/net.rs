//! HTTPS client for self-update: host allow-list checked on the first URL and on every redirect,
//! size cap, timeouts. Plain HTTP is only accepted for loopback when the test seam is on.

use std::io::{Read, Write};
use std::time::{Duration, Instant};

// Shared with the installer's hardened client: bounded hops, absolute-URL-only redirect resolution.
use crate::installer::http::{resolve_redirect, MAX_REDIRECTS};

pub const ALLOWED_HOSTS: &[&str] =
    &["api.github.com", "github.com", "objects.githubusercontent.com", "release-assets.githubusercontent.com"];

/// Largest program we will download.
pub const MAX_BINARY: u64 = 100 * 1024 * 1024;
pub const MAX_TEXT: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Net {
    /// Accept `http://127.0.0.1` / `http://localhost` (tests, local release server).
    pub allow_loopback: bool,
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    /// Upper bound for one whole download.
    pub total_timeout: Duration,
}

impl Default for Net {
    fn default() -> Self {
        Net { allow_loopback: false, connect_timeout: Duration::from_secs(10), read_timeout: Duration::from_secs(30), total_timeout: Duration::from_secs(600) }
    }
}

/// `(scheme, host)` of an absolute URL; rejects user-info and empty hosts.
pub fn split_url(url: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = url.split_once("://")?;
    let auth = rest.split(['/', '?', '#']).next()?;
    if auth.is_empty() || auth.contains('@') || auth.contains('\\') {
        return None;
    }
    let host = if auth.starts_with('[') { auth.split(']').next()?.trim_start_matches('[') } else { auth.split(':').next()? };
    if host.is_empty() {
        return None;
    }
    Some((scheme, host))
}

pub fn check_url(url: &str, allow_loopback: bool) -> Result<(), String> {
    let (scheme, host) = split_url(url).ok_or_else(|| format!("refusing malformed URL: {url}"))?;
    let host = host.to_ascii_lowercase();
    match scheme {
        "https" if ALLOWED_HOSTS.contains(&host.as_str()) => Ok(()),
        "http" if allow_loopback && (host == "127.0.0.1" || host == "localhost") => Ok(()),
        "https" => Err(format!("refusing to download from '{host}': not a GitHub host")),
        _ => Err(format!("refusing non-HTTPS URL: {url}")),
    }
}

impl Net {
    fn agent(&self) -> ureq::Agent {
        ureq::AgentBuilder::new()
            .redirects(0)
            .timeout_connect(self.connect_timeout)
            .timeout_read(self.read_timeout)
            .user_agent("kora-installer-self-update")
            .build()
    }

    /// GET with manual redirect following; every hop must pass [`check_url`].
    fn get(&self, url: &str) -> Result<ureq::Response, String> {
        let agent = self.agent();
        let mut cur = url.to_string();
        for _ in 0..=MAX_REDIRECTS {
            check_url(&cur, self.allow_loopback)?;
            let resp = match agent.get(&cur).set("Accept", "application/vnd.github+json, application/octet-stream").call() {
                Ok(r) => r,
                Err(ureq::Error::Status(c, _)) => return Err(format!("the server answered HTTP {c}")),
                Err(ureq::Error::Transport(t)) => return Err(format!("network error: {t}")),
            };
            if (300..400).contains(&resp.status()) {
                let loc = resp.header("location").ok_or("redirect without a location")?;
                cur = resolve_redirect(&cur, loc)?;
                continue;
            }
            return Ok(resp);
        }
        Err("too many redirects".into())
    }

    pub fn get_text(&self, url: &str, max: u64) -> Result<String, String> {
        let mut buf = Vec::new();
        self.download(url, &mut buf, max)?;
        String::from_utf8(buf).map_err(|_| "the answer is not text".to_string())
    }

    /// Stream the body into `out`, at most `max` bytes, within the total timeout.
    pub fn download(&self, url: &str, out: &mut dyn Write, max: u64) -> Result<u64, String> {
        let resp = self.get(url)?;
        if let Some(len) = resp.header("content-length").and_then(|v| v.parse::<u64>().ok()) {
            if len > max {
                return Err(format!("the file is too large ({len} bytes, limit {max})"));
            }
        }
        let mut r = resp.into_reader();
        let start = Instant::now();
        let mut buf = [0u8; 64 * 1024];
        let mut total = 0u64;
        loop {
            if start.elapsed() > self.total_timeout {
                return Err("the download took too long".into());
            }
            let n = r.read(&mut buf).map_err(|e| format!("download interrupted: {e}"))?;
            if n == 0 {
                return Ok(total);
            }
            total += n as u64;
            if total > max {
                return Err(format!("the file is larger than the {max}-byte limit"));
            }
            out.write_all(&buf[..n]).map_err(|e| format!("cannot write the download: {e}"))?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn allow_list() {
        for ok in [
            "https://github.com/KoraFiveM/kora-installer/releases/download/v1/kora",
            "https://objects.githubusercontent.com/x",
            "https://release-assets.githubusercontent.com/x?y=1",
            "https://api.github.com/repos/x",
            "https://GitHub.com:443/x",
        ] {
            assert!(check_url(ok, false).is_ok(), "{ok}");
        }
        for bad in [
            "http://github.com/x",
            "https://evil.com/x",
            "https://github.com.evil.com/x",
            "https://user@github.com/x",
            "https://github.com@evil.com/x",
            "https://evilgithub.com/x",
            "ftp://github.com/x",
            "github.com/x",
            "http://127.0.0.1:1/x",
        ] {
            assert!(check_url(bad, false).is_err(), "{bad}");
        }
        assert!(check_url("http://127.0.0.1:9/x", true).is_ok());
        assert!(check_url("http://localhost:9/x", true).is_ok());
        assert!(check_url("http://evil.com/x", true).is_err());
    }
}
