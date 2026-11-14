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
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(20))
        .timeout_read(Duration::from_secs(60))
        .user_agent("kora-installer")
        .build()
}

impl HttpClient {
    fn check(&self, url: &str) -> Result<(), String> {
        if url.starts_with("https://") {
            return Ok(());
        }
        let local = url.starts_with("http://127.0.0.1") || url.starts_with("http://localhost");
        if self.allow_http_localhost && local {
            return Ok(());
        }
        Err(format!("refusing non-HTTPS URL: {url}"))
    }

    fn get(&self, url: &str) -> Result<ureq::Response, String> {
        self.check(url)?;
        agent().get(url).call().map_err(|e| e.to_string())
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
