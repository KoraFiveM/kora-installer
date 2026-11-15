//! Real HTTP implementations of the network traits (`CatalogSource`, `ArtifactSource`).

use std::io::Write;
use std::time::Duration;

use crate::artifacts::{ArtifactError, ArtifactSource, MAX_DOWNLOAD};
use crate::catalog::CatalogSource;

/// Blocking HTTP client over `ureq` (rustls). HTTPS only unless `allow_http_localhost` is set
/// (test seam for a local artifact double).
#[derive(Debug, Clone, Default)]
pub struct HttpClient {
    /// Permit plain `http://127.0.0.1` / `http://localhost` URLs (tests only).
    pub allow_http_localhost: bool,
    /// Test seam: replace `https://runtime.fivem.net` with this base URL when downloading
    /// (the URL is still validated as an official artifact URL first).
    pub artifact_mirror: Option<String>,
}

fn agent() -> ureq::Agent {
    // Redirects are followed by hand (see `HttpClient::get`) so each hop can be checked.
    ureq::AgentBuilder::new()
        .redirects(0)
        .timeout_connect(Duration::from_secs(20))
        .timeout_read(Duration::from_secs(60))
        // Overall cap per request: a server that trickles bytes cannot hold the installer forever.
        .timeout(Duration::from_secs(60 * 60))
        .user_agent("kora-installer")
        .build()
}

/// Most redirects followed (GitHub release assets and runtime.fivem.net need one or two).
pub const MAX_REDIRECTS: usize = 5;

/// Absolute `Location` target for a redirect from `base` (absolute URLs and absolute paths only).
pub fn resolve_redirect(base: &str, location: &str) -> Result<String, String> {
    if location.chars().any(|c| c.is_control() || c == ' ') {
        return Err("redirect target has unexpected characters".into());
    }
    if location.starts_with("https://") || location.starts_with("http://") {
        return Ok(location.to_string());
    }
    if let Some(path) = location.strip_prefix('/').filter(|_| !location.starts_with("//")) {
        let (scheme, rest) = base.split_once("://").ok_or("bad base URL")?;
        let host = rest.split('/').next().unwrap_or("");
        return Ok(format!("{scheme}://{host}/{path}"));
    }
    Err(format!("unsupported redirect target: {location}"))
}

impl HttpClient {
    fn check(&self, url: &str) -> Result<(), String> {
        if url.starts_with("https://") {
            return Ok(());
        }
        // Exact host: `http://localhost.evil.example` must not pass.
        let host = url.strip_prefix("http://").map(|r| r.split(['/', '?', '#']).next().unwrap_or("")).map(|a| a.rsplit_once(':').map_or(a, |(h, _)| h));
        let local = matches!(host, Some("127.0.0.1") | Some("localhost"));
        if self.allow_http_localhost && local {
            return Ok(());
        }
        Err(format!("refusing non-HTTPS URL: {url}"))
    }

    /// GET with at most [`MAX_REDIRECTS`] redirects, each re-checked: HTTPS only (never a downgrade to
    /// http, except the localhost test seam) and, for hops, an absolute URL.
    fn get(&self, url: &str) -> Result<ureq::Response, String> {
        let mut current = url.to_string();
        for _ in 0..=MAX_REDIRECTS {
            self.check(&current)?;
            let resp = match agent().get(&current).call() {
                Ok(r) => r,
                Err(ureq::Error::Status(c, r)) if (300..400).contains(&c) => r,
                Err(e) => return Err(e.to_string()),
            };
            if !(300..400).contains(&resp.status()) {
                return Ok(resp);
            }
            let loc = resp.header("location").ok_or_else(|| format!("redirect without a Location from {current}"))?;
            current = resolve_redirect(&current, loc)?;
        }
        Err(format!("too many redirects from {url}"))
    }
}

impl CatalogSource for HttpClient {
    fn fetch(&self, url: &str) -> Result<String, String> {
        self.get(url)?.into_string().map_err(|e| e.to_string())
    }
}

impl ArtifactSource for HttpClient {
    fn fetch_text(&self, url: &str) -> Result<String, ArtifactError> {
        self.get(url)
            .map_err(ArtifactError::Network)?
            .into_string()
            .map_err(|e| ArtifactError::Network(e.to_string()))
    }

    fn download(&self, url: &str, out: &mut dyn Write) -> Result<(), ArtifactError> {
        let mapped;
        let url = match (&self.artifact_mirror, url.strip_prefix("https://runtime.fivem.net")) {
            (Some(base), Some(rest)) => {
                mapped = format!("{}{rest}", base.trim_end_matches('/'));
                mapped.as_str()
            }
            _ => url,
        };
        let resp = self.get(url).map_err(ArtifactError::Network)?;
        let mut reader = resp.into_reader();
        let mut limited = std::io::Read::take(&mut reader, MAX_DOWNLOAD + 1);
        let n = std::io::copy(&mut limited, out).map_err(|e| ArtifactError::Network(e.to_string()))?;
        if n > MAX_DOWNLOAD {
            return Err(ArtifactError::Network("download exceeds size limit".into()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// One-shot server answering every request with `answer(port)`.
    fn serve(answer: impl Fn(u16) -> String + Send + 'static) -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for c in l.incoming().flatten().take(8) {
                let mut c = c;
                let mut b = [0u8; 2048];
                let _ = c.read(&mut b);
                let _ = c.write_all(answer(port).as_bytes());
            }
        });
        port
    }

    #[test]
    fn redirect_targets_are_absolute_and_clean() {
        assert_eq!(resolve_redirect("https://a.example/x/y", "/z").unwrap(), "https://a.example/z");
        assert_eq!(resolve_redirect("https://a.example/x", "https://b.example/q").unwrap(), "https://b.example/q");
        assert!(resolve_redirect("https://a.example/x", "//evil.example/x").is_err());
        assert!(resolve_redirect("https://a.example/x", "relative").is_err());
        assert!(resolve_redirect("https://a.example/x", "/a\r\nSet-Cookie: x").is_err());
    }

    #[test]
    fn an_https_source_cannot_redirect_to_plain_http() {
        let c = HttpClient { allow_http_localhost: true, ..Default::default() };
        // The test server is plain http on localhost (allowed); it redirects to a non-local http host: refused.
        let port = serve(|_| "HTTP/1.1 302 Found\r\nLocation: http://evil.example/payload\r\nContent-Length: 0\r\n\r\n".into());
        let e = c.get(&format!("http://127.0.0.1:{port}/x")).err().unwrap();
        assert!(e.contains("refusing non-HTTPS"), "{e}");
    }

    #[test]
    fn redirect_loops_are_cut_off() {
        let c = HttpClient { allow_http_localhost: true, ..Default::default() };
        let port = serve(|p| format!("HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{p}/again\r\nContent-Length: 0\r\n\r\n"));
        let e = c.get(&format!("http://127.0.0.1:{port}/x")).err().unwrap();
        assert!(e.contains("too many redirects"), "{e}");
    }

    #[test]
    fn localhost_lookalike_hosts_are_not_local() {
        let c = HttpClient { allow_http_localhost: true, ..Default::default() };
        assert!(c.check("http://localhost:8080/x").is_ok() && c.check("http://127.0.0.1/x").is_ok());
        assert!(c.check("http://localhost.evil.example/x").is_err());
        assert!(c.check("http://127.0.0.1.evil.example/x").is_err());
        assert!(c.check("http://localhost@evil.example/x").is_err());
    }

    #[test]
    fn a_normal_answer_still_works() {
        let c = HttpClient { allow_http_localhost: true, ..Default::default() };
        let port = serve(|_| "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi".into());
        assert_eq!(c.fetch(&format!("http://127.0.0.1:{port}/x")).unwrap(), "hi");
    }
}
