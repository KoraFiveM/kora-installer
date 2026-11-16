//! Real HTTP implementations of the network traits (`CatalogSource`, `ArtifactSource`).

use std::io::Write;
use std::time::{Duration, Instant};

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

/// Agent for big downloads: no overall cap (the caller enforces a size-scaled deadline) and `stall` as the
/// longest silence tolerated on the connection.
fn download_agent(stall: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .redirects(0)
        .timeout_connect(Duration::from_secs(20))
        .timeout_read(stall)
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
        self.get_with(&agent(), url, None)
    }

    /// Like [`get`](Self::get) with a chosen agent and an optional `Range: bytes=<start>-`. A redirect may go to
    /// ANY host (the caller verifies a pinned SHA-256), but every hop must be HTTPS and absolute.
    fn get_with(&self, agent: &ureq::Agent, url: &str, range_from: Option<u64>) -> Result<ureq::Response, String> {
        let mut current = url.to_string();
        for _ in 0..=MAX_REDIRECTS {
            self.check(&current)?;
            let mut req = agent.get(&current);
            if let Some(from) = range_from {
                req = req.set("Range", &format!("bytes={from}-"));
            }
            let resp = match req.call() {
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

// ------------------------------------------------------------ robust, hash-pinned downloads

/// Tuning for [`HttpClient::download_verified`].
#[derive(Debug, Clone)]
pub struct FetchConfig {
    /// No bytes for this long = the source is stalled: give up on it (keep the partial file).
    pub stall: Duration,
    /// Attempts per source before the next source is tried (a second pass resumes the partial file).
    pub passes: usize,
    /// Slowest speed we still wait for: the overall deadline is `size / min_rate` (at least `min_deadline`).
    pub min_rate: u64,
    pub min_deadline: Duration,
    /// Smallest gap between progress reports.
    pub report_every: Duration,
}

impl Default for FetchConfig {
    fn default() -> Self {
        Self { stall: Duration::from_secs(30), passes: 2, min_rate: 150 * 1024, min_deadline: Duration::from_secs(10 * 60), report_every: Duration::from_millis(500) }
    }
}

/// A progress report for a running download.
#[derive(Debug, Clone, PartialEq)]
pub struct FetchProgress {
    pub bytes: u64,
    pub total: Option<u64>,
    /// Bytes per second since this attempt began.
    pub rate: f64,
    pub eta: Option<Duration>,
}

impl FetchProgress {
    /// `"Downloading MariaDB 45% (12.3 MB/s, about 5 s left)"`.
    pub fn line(&self, what: &str) -> String {
        let mb = self.rate / 1_048_576.0;
        match self.total.filter(|t| *t > 0) {
            Some(t) => {
                let eta = self.eta.map(|e| format!(", about {} s left", e.as_secs().max(1))).unwrap_or_default();
                format!("Downloading {what} {}% ({mb:.1} MB/s{eta})", (self.bytes.saturating_mul(100) / t).min(100))
            }
            None => format!("Downloading {what} {:.0} MB ({mb:.1} MB/s)", self.bytes as f64 / 1_048_576.0),
        }
    }
}

pub fn sha256_file(path: &std::path::Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut h = Sha256::new();
    let mut f = std::fs::File::open(path)?;
    let mut buf = [0u8; 65536];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Plain-language short name of a URL's host for messages.
fn host_of(url: &str) -> &str {
    url.split_once("://").map_or(url, |(_, r)| r).split(['/', '?', '#']).next().unwrap_or(url)
}

impl HttpClient {
    /// One attempt at `url`, appending to `dest` when the server honours a Range request.
    fn download_once(&self, url: &str, dest: &std::path::Path, cfg: &FetchConfig, progress: &mut dyn FnMut(&FetchProgress)) -> Result<(), String> {
        let have = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
        let agent = download_agent(cfg.stall);
        let resp = self.get_with(&agent, url, (have > 0).then_some(have))?;
        let (mut written, total, append) = match resp.status() {
            206 => {
                // `Content-Range: bytes <from>-<to>/<total>`: must continue exactly where we stopped.
                let cr = resp.header("content-range").unwrap_or("").to_string();
                let from = cr.strip_prefix("bytes ").and_then(|r| r.split('-').next()).and_then(|n| n.trim().parse::<u64>().ok());
                if from != Some(have) {
                    let _ = std::fs::remove_file(dest);
                    return Err("the server resumed at the wrong place; starting over".into());
                }
                (have, cr.rsplit('/').next().and_then(|t| t.trim().parse::<u64>().ok()), true)
            }
            200 => (0, resp.header("content-length").and_then(|t| t.parse::<u64>().ok()), false),
            416 => {
                let _ = std::fs::remove_file(dest);
                return Err("the server could not resume the partial file; starting over".into());
            }
            c => return Err(format!("the server answered HTTP {c}")),
        };
        if total.is_some_and(|t| t > MAX_DOWNLOAD) {
            return Err("the file is larger than the allowed size".into());
        }
        let mut f = if append {
            std::fs::OpenOptions::new().append(true).open(dest)
        } else {
            std::fs::File::create(dest)
        }
        .map_err(|e| format!("cannot write {}: {e}", dest.display()))?;
        let start = Instant::now();
        let start_bytes = written;
        let deadline = total.map(|t| start + (Duration::from_secs(t / cfg.min_rate.max(1))).max(cfg.min_deadline));
        let mut last_report = Instant::now().checked_sub(cfg.report_every).unwrap_or_else(Instant::now);
        let mut reader = resp.into_reader();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = match std::io::Read::read(&mut reader, &mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                // A signal (for example a child process ending) interrupts the read: just read again.
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) if matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock) => {
                    return Err(format!("no data for {} seconds (stalled)", cfg.stall.as_secs()));
                }
                Err(e) => return Err(format!("the connection broke after {} MB: {e}", written / 1_048_576)),
            };
            f.write_all(&buf[..n]).map_err(|e| format!("cannot write {}: {e}", dest.display()))?;
            written += n as u64;
            if written > MAX_DOWNLOAD {
                return Err("the file is larger than the allowed size".into());
            }
            if deadline.is_some_and(|d| Instant::now() > d) {
                return Err("too slow: the download would take far too long".into());
            }
            if last_report.elapsed() >= cfg.report_every {
                last_report = Instant::now();
                progress(&Self::progress(written, start_bytes, total, start));
            }
        }
        f.flush().ok();
        progress(&Self::progress(written, start_bytes, total, start));
        if let Some(t) = total.filter(|t| *t != written) {
            return Err(format!("the download ended early ({written} of {t} bytes)"));
        }
        Ok(())
    }

    fn progress(written: u64, start_bytes: u64, total: Option<u64>, start: Instant) -> FetchProgress {
        let secs = start.elapsed().as_secs_f64().max(0.001);
        let rate = (written - start_bytes) as f64 / secs;
        let eta = total.filter(|_| rate > 1.0).map(|t| Duration::from_secs_f64(t.saturating_sub(written) as f64 / rate));
        FetchProgress { bytes: written, total, rate, eta }
    }

    /// Download one of `sources` (tried in order, each up to `cfg.passes` times, resuming a partial file) into
    /// `dest` and accept it ONLY if its SHA-256 equals `sha256`. A wrong hash deletes the file and aborts (it is
    /// not retried: a different file is not a transient fault). Returns the URL that delivered the file.
    /// Redirects may lead to any host; trust comes from the hash. Every hop must still be HTTPS.
    pub fn download_verified(&self, sources: &[String], dest: &std::path::Path, sha256: &str, cfg: &FetchConfig, progress: &mut dyn FnMut(&FetchProgress), note: &mut dyn FnMut(&str)) -> Result<String, String> {
        let want = sha256.to_ascii_lowercase();
        let mut problems = Vec::new();
        let _ = std::fs::remove_file(dest);
        for url in sources {
            for pass in 1..=cfg.passes.max(1) {
                match self.download_once(url, dest, cfg, progress) {
                    Ok(()) => {
                        let got = sha256_file(dest).map_err(|e| format!("cannot read the download back: {e}"))?;
                        if got == want {
                            return Ok(url.clone());
                        }
                        let _ = std::fs::remove_file(dest);
                        return Err(format!("the file from {} does not match the expected SHA-256 checksum, so it was deleted and nothing was installed", host_of(url)));
                    }
                    Err(e) => {
                        note(&format!("Download from {} failed ({e}){}", host_of(url), if pass < cfg.passes { "; trying again" } else { "; trying another source" }));
                        problems.push(format!("{}: {e}", host_of(url)));
                    }
                }
            }
        }
        let _ = std::fs::remove_file(dest);
        Err(format!("could not download the file from any source ({}). Check your internet connection and try again.", problems.join("; ")))
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

    // ---- hash-pinned, resumable downloads against a local mock server ----

    use std::sync::{Arc, Mutex};

    type Handler = Arc<dyn Fn(&str, &mut std::net::TcpStream) + Send + Sync>;

    /// Thread-per-connection server; `h(request_head, stream)` writes the whole answer.
    fn mock(h: Handler) -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for c in l.incoming().flatten() {
                let h = h.clone();
                std::thread::spawn(move || {
                    let mut c = c;
                    let mut b = [0u8; 4096];
                    let n = c.read(&mut b).unwrap_or(0);
                    h(&String::from_utf8_lossy(&b[..n]), &mut c);
                });
            }
        });
        port
    }

    fn payload() -> Vec<u8> {
        (0..300_000u32).map(|i| (i % 251) as u8).collect()
    }
    fn sha(b: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
    }
    fn range_start(req: &str) -> Option<usize> {
        req.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("range: bytes=").map(|r| r.trim_end_matches('-').parse().unwrap()))
    }
    fn ok_full(c: &mut std::net::TcpStream, body: &[u8]) {
        let _ = c.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).as_bytes());
        let _ = c.write_all(body);
    }
    fn fast_cfg() -> FetchConfig {
        FetchConfig { stall: Duration::from_millis(700), report_every: Duration::ZERO, ..Default::default() }
    }
    fn client() -> HttpClient {
        HttpClient { allow_http_localhost: true, ..Default::default() }
    }
    fn run(srcs: Vec<String>, want: &str, dest: &std::path::Path) -> (Result<String, String>, Vec<FetchProgress>, Vec<String>) {
        let mut prog = Vec::new();
        let mut notes = Vec::new();
        let r = client().download_verified(&srcs, dest, want, &fast_cfg(), &mut |p| prog.push(p.clone()), &mut |m| notes.push(m.to_string()));
        (r, prog, notes)
    }

    #[test]
    fn pinned_hash_ok_and_progress_is_reported() {
        let body = payload();
        let b = body.clone();
        let port = mock(Arc::new(move |_, c| ok_full(c, &b)));
        let d = tempfile::tempdir().unwrap();
        let dest = d.path().join("f.zip");
        let (r, prog, _) = run(vec![format!("http://127.0.0.1:{port}/a.zip")], &sha(&body), &dest);
        assert_eq!(r.unwrap(), format!("http://127.0.0.1:{port}/a.zip"));
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        let last = prog.last().unwrap();
        assert_eq!((last.bytes, last.total), (300_000, Some(300_000)));
        assert!(last.line("MariaDB").starts_with("Downloading MariaDB 100% ("), "{}", last.line("MariaDB"));
    }

    #[test]
    fn a_wrong_hash_aborts_and_deletes_the_file_without_trying_other_sources() {
        let body = payload();
        let hits = Arc::new(Mutex::new(0));
        let (b, h) = (body.clone(), hits.clone());
        let port = mock(Arc::new(move |_, c| {
            *h.lock().unwrap() += 1;
            ok_full(c, &b)
        }));
        let d = tempfile::tempdir().unwrap();
        let dest = d.path().join("f.zip");
        let u = format!("http://127.0.0.1:{port}/a.zip");
        let (r, _, _) = run(vec![u.clone(), u], &"0".repeat(64), &dest);
        assert!(r.unwrap_err().contains("does not match"));
        assert!(!dest.exists());
        assert_eq!(*hits.lock().unwrap(), 1);
    }

    #[test]
    fn a_redirect_to_another_host_is_followed_but_never_downgraded() {
        let body = payload();
        let b = body.clone();
        let target = mock(Arc::new(move |_, c| ok_full(c, &b)));
        // Different host name (localhost vs 127.0.0.1), like the mirror-routed endpoint.
        let front = mock(Arc::new(move |_, c| {
            let _ = c.write_all(format!("HTTP/1.1 302 Found\r\nLocation: http://localhost:{target}/m/a.zip\r\nContent-Length: 0\r\n\r\n").as_bytes());
        }));
        let d = tempfile::tempdir().unwrap();
        let dest = d.path().join("f.zip");
        let (r, _, _) = run(vec![format!("http://127.0.0.1:{front}/rest")], &sha(&body), &dest);
        assert!(r.is_ok(), "{r:?}");
        // A hop to plain http on a real host is refused (HTTPS-only outside the localhost test seam).
        let evil = mock(Arc::new(|_, c| {
            let _ = c.write_all(b"HTTP/1.1 302 Found\r\nLocation: http://mirror.example/a.zip\r\nContent-Length: 0\r\n\r\n");
        }));
        let (r, _, _) = run(vec![format!("http://127.0.0.1:{evil}/rest")], &sha(&body), &dest);
        assert!(r.unwrap_err().contains("refusing non-HTTPS"));
        assert!(!dest.exists());
    }

    #[test]
    fn a_stalled_source_falls_back_to_the_next() {
        let body = payload();
        let b = body.clone();
        let stuck = mock(Arc::new(|_, c| {
            let _ = c.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 300000\r\n\r\nabc");
            std::thread::sleep(Duration::from_secs(4));
        }));
        let good = mock(Arc::new(move |_, c| ok_full(c, &b)));
        let d = tempfile::tempdir().unwrap();
        let dest = d.path().join("f.zip");
        let (r, _, notes) = run(vec![format!("http://127.0.0.1:{stuck}/a.zip"), format!("http://127.0.0.1:{good}/a.zip")], &sha(&body), &dest);
        assert_eq!(r.unwrap(), format!("http://127.0.0.1:{good}/a.zip"));
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        assert!(notes.iter().any(|n| n.contains("stalled") && n.contains("trying again")), "{notes:?}");
        assert!(notes.iter().any(|n| n.contains("trying another source")), "{notes:?}");
    }

    #[test]
    fn a_truncated_response_is_resumed_with_a_range_request() {
        let body = payload();
        let b = body.clone();
        let ranges = Arc::new(Mutex::new(Vec::new()));
        let r2 = ranges.clone();
        let port = mock(Arc::new(move |req, c| match range_start(req) {
            None => {
                // Promise everything, deliver 100 000 bytes, hang up.
                let _ = c.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", b.len()).as_bytes());
                let _ = c.write_all(&b[..100_000]);
            }
            Some(from) => {
                r2.lock().unwrap().push(from);
                let rest = &b[from..];
                let _ = c.write_all(format!("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {from}-{}/{}\r\nContent-Length: {}\r\n\r\n", b.len() - 1, b.len(), rest.len()).as_bytes());
                let _ = c.write_all(rest);
            }
        }));
        let d = tempfile::tempdir().unwrap();
        let dest = d.path().join("f.zip");
        let (r, _, notes) = run(vec![format!("http://127.0.0.1:{port}/a.zip")], &sha(&body), &dest);
        assert!(r.is_ok(), "{r:?} {notes:?}");
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        assert_eq!(*ranges.lock().unwrap(), vec![100_000]);
    }

    #[test]
    fn every_source_failing_gives_a_plain_message() {
        let port = mock(Arc::new(|_, c| {
            let _ = c.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
        }));
        let d = tempfile::tempdir().unwrap();
        let (r, _, _) = run(vec![format!("http://127.0.0.1:{port}/a.zip")], &"0".repeat(64), &d.path().join("f.zip"));
        assert!(r.unwrap_err().contains("internet connection"));
    }
}
