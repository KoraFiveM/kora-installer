//! FiveM artifact manager: list, validate, download (verified), extract, and
//! keep the previous build for one-step rollback.
//!
//! All network access goes through [`ArtifactSource`] so everything here is
//! testable offline. Reference behaviour: the old Go installer (`sources.go`,
//! `archive.go`, `archive_7z.go`).
//!
//! Upstream formats (checked 2026-11-14):
//! * JG JSON (`https://artifacts.jgscripts.com/json`): `recommendedArtifact`,
//!   `windowsDownloadLink`, `linuxDownloadLink`, `brokenArtifacts{build: reason}`.
//! * Cfx changelog API (`https://changelogs-live.fivem.net/api/changelog/versions/{win32|linux}/server`):
//!   `recommended`, `latest`, `critical`, `optional` (+ `<name>_download`).
//! * TODO-verify: the legacy `runtime.fivem.net/.../master/` HTML directory listing
//!   now redirects to the docs site, so [`parse_cfx_listing`] accepts either the
//!   changelog JSON or an HTML/text listing with `href`s and tolerates both.

mod extract;
mod store;

pub use extract::{extract_archive, ExtractLimits};
pub use store::{ArtifactStore, StoreState};

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

pub const JG_URL: &str = "https://artifacts.jgscripts.com/json";
pub const CFX_CHANGELOG_WINDOWS: &str = "https://changelogs-live.fivem.net/api/changelog/versions/win32/server";
pub const CFX_CHANGELOG_LINUX: &str = "https://changelogs-live.fivem.net/api/changelog/versions/linux/server";
pub const MAX_DOWNLOAD: u64 = 2 << 30;
/// Minimum game build Kora supports. TODO-verify against the catalog; the txAdmin recipe pins 3751.
pub const KORA_MIN_GAME_BUILD: u32 = 3258;

#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    #[error("invalid artifact URL: {0}")]
    InvalidUrl(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("could not parse artifact listing: {0}")]
    Parse(String),
    #[error("verification failed: {0}")]
    Verify(String),
    #[error("unsafe or unsupported archive: {0}")]
    Archive(String),
    #[error("artifact store: {0}")]
    Store(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, ArtifactError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Windows,
    Linux,
}

impl Platform {
    pub fn current() -> Platform {
        if cfg!(windows) {
            Platform::Windows
        } else {
            Platform::Linux
        }
    }
    /// Executable that must exist at the artifact root after extraction.
    pub fn executable(self) -> &'static str {
        match self {
            Platform::Windows => "FXServer.exe",
            Platform::Linux => "run.sh",
        }
    }
    fn path_prefix(self) -> &'static str {
        match self {
            Platform::Windows => "/artifacts/fivem/build_server_windows/master/",
            Platform::Linux => "/artifacts/fivem/build_proot_linux/master/",
        }
    }
    pub fn changelog_url(self) -> &'static str {
        match self {
            Platform::Windows => CFX_CHANGELOG_WINDOWS,
            Platform::Linux => CFX_CHANGELOG_LINUX,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Recommended,
    Latest,
    All,
}

/// One downloadable build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactBuild {
    pub build: u32,
    pub url: String,
    /// Set when the build is known broken (JG `brokenArtifacts`).
    pub broken: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtifactList {
    pub recommended: Option<ArtifactBuild>,
    pub latest: Option<ArtifactBuild>,
    /// Every known build, newest first, deduplicated by build number.
    pub all: Vec<ArtifactBuild>,
    pub broken: BTreeMap<u32, String>,
}

impl ArtifactList {
    pub fn pick(&self, channel: Channel) -> Option<&ArtifactBuild> {
        match channel {
            Channel::Recommended => self.recommended.as_ref(),
            Channel::Latest => self.latest.as_ref().or_else(|| self.all.first()),
            Channel::All => self.all.first(),
        }
    }
    pub fn find(&self, build: u32) -> Option<&ArtifactBuild> {
        self.all.iter().find(|b| b.build == build)
    }
    fn add(&mut self, b: ArtifactBuild) {
        if !self.all.iter().any(|x| x.build == b.build) {
            self.all.push(b);
        }
    }
    fn finish(&mut self) {
        for b in self.all.iter_mut() {
            b.broken = self.broken.get(&b.build).cloned();
        }
        self.all.sort_by_key(|b| std::cmp::Reverse(b.build));
        for b in [&mut self.recommended, &mut self.latest].into_iter().flatten() {
            b.broken = self.broken.get(&b.build).cloned();
        }
    }
}

/// Network access. Implemented for real HTTP elsewhere; tests use fakes.
pub trait ArtifactSource {
    fn fetch_text(&self, url: &str) -> Result<String>;
    /// Stream the body of `url` into `out`. Implementations should cap at [`MAX_DOWNLOAD`]
    /// and must refuse redirects that leave HTTPS.
    fn download(&self, url: &str, out: &mut dyn Write) -> Result<()>;
}

/// Validate that `url` is an official runtime.fivem.net artifact for `platform`.
pub fn validate_url(url: &str, platform: Platform) -> Result<()> {
    let bad = |m: &str| Err(ArtifactError::InvalidUrl(format!("{m}: {url}")));
    let Some(rest) = url.strip_prefix("https://runtime.fivem.net") else {
        return bad("must be an https://runtime.fivem.net URL");
    };
    if !rest.starts_with('/') || rest.contains(['?', '#', '@', '\\', '%']) || rest.chars().any(|c| c.is_control() || c == ' ') {
        return bad("unexpected characters in URL");
    }
    let Some(tail) = rest.strip_prefix(platform.path_prefix()) else {
        return bad("URL does not match the target platform");
    };
    let mut parts = tail.split('/');
    let (Some(dir), Some(file), None) = (parts.next(), parts.next(), parts.next()) else {
        return bad("unexpected path layout");
    };
    let file_ok = match platform {
        Platform::Linux => file == "fx.tar.xz",
        Platform::Windows => file == "server.zip" || file == "server.7z",
    };
    if !file_ok || build_from_dir(dir).is_none() {
        return bad("unexpected file or build directory");
    }
    Ok(())
}

/// `38621-5412ed...` -> 38621 (hash must be 40 hex chars).
fn build_from_dir(dir: &str) -> Option<u32> {
    let (n, hash) = dir.split_once('-')?;
    if hash.len() != 40 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) || n.is_empty() || n.len() > 9 || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    n.parse().ok()
}

/// Build number from a validated URL.
pub fn build_from_url(url: &str) -> Option<u32> {
    let dir = url.rsplit('/').nth(1)?;
    build_from_dir(dir)
}

fn mk(url: &str, platform: Platform) -> Option<ArtifactBuild> {
    validate_url(url, platform).ok()?;
    Some(ArtifactBuild { build: build_from_url(url)?, url: url.to_string(), broken: None })
}

#[derive(Deserialize)]
struct Jg {
    #[serde(rename = "recommendedArtifact")]
    recommended: Option<serde_json::Value>,
    #[serde(rename = "windowsDownloadLink")]
    windows: Option<String>,
    #[serde(rename = "linuxDownloadLink")]
    linux: Option<String>,
    #[serde(rename = "brokenArtifacts", default)]
    broken: BTreeMap<String, String>,
}

/// Parse the JG-style recommended JSON. Invalid/foreign URLs are an error for the recommended entry.
pub fn parse_jg(json: &str, platform: Platform) -> Result<ArtifactList> {
    let jg: Jg = serde_json::from_str(json).map_err(|e| ArtifactError::Parse(e.to_string()))?;
    let url = match platform {
        Platform::Windows => jg.windows,
        Platform::Linux => jg.linux,
    }
    .ok_or_else(|| ArtifactError::Parse("missing download link for platform".into()))?;
    validate_url(&url, platform)?;
    let b = mk(&url, platform).ok_or_else(|| ArtifactError::Parse("download link has no build number".into()))?;
    if let Some(r) = jg.recommended {
        let s = r.as_str().map(str::to_string).unwrap_or_else(|| r.to_string());
        if s.parse::<u32>().ok() != Some(b.build) {
            return Err(ArtifactError::Parse("recommendedArtifact does not match download link".into()));
        }
    }
    let mut list = ArtifactList { recommended: Some(b.clone()), ..Default::default() };
    list.broken = jg.broken.iter().filter_map(|(k, v)| Some((k.parse().ok()?, v.clone()))).collect();
    list.add(b);
    list.finish();
    Ok(list)
}

/// Parse the Cfx side: changelog JSON (`recommended`/`latest`/... with `*_download`) or, failing
/// that, every `href` in a directory-style listing. Entries failing [`validate_url`] are dropped.
pub fn parse_cfx_listing(body: &str, platform: Platform) -> Result<ArtifactList> {
    let mut list = ArtifactList::default();
    if let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(body) {
        for key in ["recommended", "latest", "optional", "critical"] {
            let Some(url) = map.get(&format!("{key}_download")).and_then(|v| v.as_str()) else { continue };
            let Some(b) = mk(url, platform) else { continue };
            match key {
                "recommended" => list.recommended = Some(b.clone()),
                "latest" => list.latest = Some(b.clone()),
                _ => {}
            }
            list.add(b);
        }
    } else {
        let base = format!("https://runtime.fivem.net{}", platform.path_prefix());
        let file = match platform {
            Platform::Linux => "fx.tar.xz",
            Platform::Windows => "server.zip",
        };
        let mut rest = body;
        while let Some(i) = rest.find("href=\"") {
            rest = &rest[i + 6..];
            let Some(j) = rest.find('"') else { break };
            let dir = rest[..j].trim_start_matches("./").trim_end_matches('/');
            rest = &rest[j..];
            if build_from_dir(dir).is_some() {
                if let Some(b) = mk(&format!("{base}{dir}/{file}"), platform) {
                    list.add(b);
                }
            }
        }
        list.finish();
        list.latest = list.all.first().cloned();
    }
    if list.all.is_empty() {
        return Err(ArtifactError::Parse("no artifacts found".into()));
    }
    list.finish();
    if list.latest.is_none() {
        list.latest = list.all.first().cloned();
    }
    Ok(list)
}

/// Merge both sources. JG wins for `recommended` and `broken`; Cfx supplies `latest`/`all`.
/// Either source failing is tolerated, both failing is an error.
pub fn list_artifacts(src: &dyn ArtifactSource, platform: Platform) -> Result<ArtifactList> {
    let jg = src.fetch_text(JG_URL).and_then(|t| parse_jg(&t, platform));
    let cfx = src.fetch_text(platform.changelog_url()).and_then(|t| parse_cfx_listing(&t, platform));
    match (jg, cfx) {
        (Err(e), Err(_)) => Err(e),
        (Ok(l), Err(_)) => Ok(l),
        (Err(_), Ok(l)) => Ok(l),
        (Ok(jg), Ok(mut l)) => {
            l.recommended = jg.recommended.clone().or(l.recommended);
            l.broken = jg.broken;
            for b in jg.all {
                l.add(b);
            }
            l.finish();
            Ok(l)
        }
    }
}

/// Integrity expectations for a download. Cfx publishes no checksum, so these are optional
/// hooks (e.g. a value cached by the Kora site proxy).
#[derive(Debug, Clone, Default)]
pub struct Verify {
    pub sha256: Option<String>,
    pub size: Option<u64>,
    pub max_size: Option<u64>,
}

struct HashWriter<W: Write> {
    inner: W,
    hash: Sha256,
    n: u64,
    max: u64,
}

impl<W: Write> Write for HashWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.n + buf.len() as u64 > self.max {
            return Err(std::io::Error::other("download exceeds size limit"));
        }
        let k = self.inner.write(buf)?;
        self.hash.update(&buf[..k]);
        self.n += k as u64;
        Ok(k)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Download `url` to `dest` (created exclusively), verifying size/sha256. On any failure
/// `dest` is removed. Returns the sha256 hex of the file.
pub fn download_verified(src: &dyn ArtifactSource, url: &str, platform: Platform, dest: &Path, verify: &Verify) -> Result<String> {
    validate_url(url, platform)?;
    let file = std::fs::OpenOptions::new().write(true).create_new(true).open(dest)?;
    let mut w = HashWriter { inner: file, hash: Sha256::new(), n: 0, max: verify.max_size.unwrap_or(MAX_DOWNLOAD).min(MAX_DOWNLOAD) };
    let res = src.download(url, &mut w).and_then(|_| {
        w.flush()?;
        let sha = hex::encode(w.hash.clone().finalize());
        if let Some(s) = verify.size {
            if s != w.n {
                return Err(ArtifactError::Verify(format!("size mismatch: expected {s}, got {}", w.n)));
            }
        }
        if let Some(e) = &verify.sha256 {
            if !e.eq_ignore_ascii_case(&sha) {
                return Err(ArtifactError::Verify("sha256 mismatch".into()));
            }
        }
        Ok(sha)
    });
    drop(w);
    if res.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    res
}

/// Result of checking an `sv_enforceGameBuild` choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameBuildCheck {
    Ok,
    BelowMinimum { requested: u32, minimum: u32 },
}

pub fn check_game_build(requested: u32, minimum: u32) -> GameBuildCheck {
    if requested < minimum {
        GameBuildCheck::BelowMinimum { requested, minimum }
    } else {
        GameBuildCheck::Ok
    }
}

/// Plain-language warning, if any.
pub fn game_build_warning(requested: u32, minimum: u32) -> Option<String> {
    match check_game_build(requested, minimum) {
        GameBuildCheck::Ok => None,
        GameBuildCheck::BelowMinimum { requested, minimum } => Some(format!(
            "Game build {requested} is older than {minimum}, the minimum Kora supports. Some Kora resources may not work; choose {minimum} or newer."
        )),
    }
}

#[cfg(test)]
mod tests;
