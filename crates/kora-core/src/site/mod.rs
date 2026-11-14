//! Setup-code client for the Kora website (`POST /api/installer/redeem`).
//!
//! The website lets a user prepare an install (mode, name, resources, ports...) and hands out a
//! single-use code such as `KORA-7F3K-92QX`. [`redeem`] turns that code back into a [`Redeemed`]
//! set of answers. The response never contains secrets: the license key, the database admin login
//! and (for development mode) the GitHub organisation to fork into are still asked locally.
//!
//! Safety rules enforced here: HTTPS only (plain HTTP just for `localhost` / `127.0.0.1`, used by
//! tests), no redirects, strict timeouts, a response size cap, and defensive validation of every
//! field of the reply (the site is trusted but the network is not).

use std::io::Read;
use std::time::Duration;

use serde::Deserialize;

use crate::artifacts::{self, ArtifactSource, Platform};
use crate::installer::{InstallRequest, Mode, Ports, ResourceSelection};

pub const DEFAULT_SITE_URL: &str = "https://koraframe.work";
/// Alphabet of the code characters (no 0/O/1/I/L).
pub const CODE_ALPHABET: &str = "23456789ABCDEFGHJKMNPQRSTUVWXYZ";
const MAX_BODY: usize = 64 * 1024;
const MAX_RESOURCES: usize = 200;

/// Plain-language failures. `Display` is meant to be shown to the user as is.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SiteError {
    #[error("That setup code doesn't look right. Codes look like KORA-7F3K-92QX (letters and numbers, no 0, O, 1, I or L). {0}")]
    BadCode(String),
    #[error("That setup code is not valid: it may have expired (codes last 30 minutes), was already used, or was mistyped. Create a new one on the Kora website, or continue without a code.")]
    InvalidOrExpired,
    #[error("Too many attempts. Please wait a while and try again, or continue without a code.")]
    RateLimited,
    #[error("Could not reach the Kora website ({0}). Check your internet connection, or continue without a code and answer the questions here instead (run `kora install` without --code).")]
    Network(String),
    #[error("The Kora website had a problem (HTTP {0}). Try again in a few minutes, or continue without a code and answer the questions here instead.")]
    Server(u16),
    #[error("The Kora website sent an answer this installer does not understand ({0}). Update the installer, or continue without a code.")]
    BadResponse(String),
    #[error("The website address is not allowed: {0}")]
    BadUrl(String),
    #[error("Your setup code asks for FiveM server build {0}, which is not an official build we can verify. Pick the recommended build on the website, or continue without a code.")]
    UnknownBuild(u32),
}

pub type Result<T> = std::result::Result<T, SiteError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactChoice {
    Recommended,
    Build(u32),
}

/// What the website decided for this install. No secrets are in here by construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redeemed {
    pub mode: Mode,
    pub name: String,
    pub resources: ResourceSelection,
    pub artifact: ArtifactChoice,
    /// Validated runtime.fivem.net URL for `ArtifactChoice::Build`, filled by [`Redeemed::resolve_artifact`].
    pub artifact_url: Option<String>,
    pub game_build: Option<u32>,
    pub ports: Ports,
    pub hostname: Option<String>,
    pub cfx_identifier: Option<String>,
    pub cfx_username: Option<String>,
    /// The linked GitHub login. Only a suggestion: the fork owner is always confirmed by the user.
    pub github_login: Option<String>,
}

impl Redeemed {
    /// Turn `ArtifactChoice::Build` into a validated official download URL using the artifact list.
    /// `Recommended` needs nothing (the installer picks it later).
    pub fn resolve_artifact(&mut self, src: &dyn ArtifactSource, platform: Platform) -> Result<()> {
        let ArtifactChoice::Build(b) = self.artifact else { return Ok(()) };
        let list = artifacts::list_artifacts(src, platform).map_err(|e| SiteError::Network(e.to_string()))?;
        let found = list.find(b).ok_or(SiteError::UnknownBuild(b))?;
        artifacts::validate_url(&found.url, platform).map_err(|_| SiteError::UnknownBuild(b))?;
        self.artifact_url = Some(found.url.clone());
        Ok(())
    }

    /// Copy the decided values into a request (everything except secrets, folder and database).
    pub fn apply_to(&self, req: &mut InstallRequest) {
        req.mode = self.mode;
        req.name = self.name.clone();
        req.hostname = self.hostname.clone().unwrap_or_else(|| self.name.clone());
        req.resources = self.resources.clone();
        req.artifact_url = self.artifact_url.clone();
        req.game_build = self.game_build;
        req.ports = self.ports.clone();
        if self.cfx_identifier.is_some() {
            req.cfx_identifier = self.cfx_identifier.clone();
        }
    }

    /// Friendly greeting for the wizard.
    pub fn welcome(&self) -> String {
        match &self.cfx_username {
            Some(u) => format!("Welcome, {u}! Your choices from the website were loaded."),
            None => "Welcome! Your choices from the website were loaded.".to_string(),
        }
    }
}

/// Client settings (tests shorten the timeouts).
#[derive(Debug, Clone)]
pub struct Options {
    pub connect_timeout: Duration,
    pub timeout: Duration,
    pub max_body: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self { connect_timeout: Duration::from_secs(8), timeout: Duration::from_secs(15), max_body: MAX_BODY }
    }
}

/// `KORA_SITE_URL` or the default site.
pub fn site_url_from_env() -> String {
    std::env::var("KORA_SITE_URL").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| DEFAULT_SITE_URL.into())
}

/// Normalise a typed code: case-insensitive, dashes/spaces optional, optional `KORA` prefix.
/// Returns the canonical `KORA-XXXX-XXXX`.
pub fn normalize_code(input: &str) -> Result<String> {
    let mut s: String = input.chars().filter(|c| !matches!(c, '-' | ' ' | '_') && !c.is_control()).collect::<String>().to_ascii_uppercase();
    if s.is_empty() {
        return Err(SiteError::BadCode("Nothing was entered.".into()));
    }
    if s.len() == 12 && s.starts_with("KORA") {
        s.drain(..4);
    }
    if s.len() != 8 {
        return Err(SiteError::BadCode("It should have 8 letters and numbers after KORA-.".into()));
    }
    if let Some(bad) = s.chars().find(|c| !CODE_ALPHABET.contains(*c)) {
        return Err(SiteError::BadCode(format!("The character '{bad}' is not used in codes.")));
    }
    Ok(format!("KORA-{}-{}", &s[..4], &s[4..]))
}

/// HTTPS only; `http://` only for the exact hosts `localhost` / `127.0.0.1` (tests). Returns the
/// base without a trailing slash.
pub fn check_base_url(base: &str) -> Result<String> {
    let base = base.trim().trim_end_matches('/');
    let bad = |m: &str| Err(SiteError::BadUrl(format!("{m}: {base}")));
    let (https, rest) = if let Some(r) = base.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = base.strip_prefix("http://") {
        (false, r)
    } else {
        return bad("it must start with https://");
    };
    let (authority, path) = rest.split_once('/').map_or((rest, ""), |(a, p)| (a, p));
    if authority.is_empty() || authority.contains(['@', '\\', ' ', '?', '#']) || path.contains(['?', '#']) {
        return bad("unexpected characters");
    }
    let host = authority.rsplit_once(':').map_or(authority, |(h, p)| if p.bytes().all(|b| b.is_ascii_digit()) { h } else { authority });
    if !https && !matches!(host, "localhost" | "127.0.0.1") {
        return bad("plain http:// is only allowed for localhost");
    }
    Ok(base.to_string())
}

fn agent(o: &Options) -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(o.connect_timeout).timeout(o.timeout).redirects(0).user_agent("kora-installer").build()
}

/// Redeem `code` against `base_url` with default options.
pub fn redeem(code: &str, base_url: &str) -> Result<Redeemed> {
    redeem_with(code, base_url, &Options::default())
}

pub fn redeem_with(code: &str, base_url: &str, opts: &Options) -> Result<Redeemed> {
    let code = normalize_code(code)?;
    let base = check_base_url(base_url)?;
    let url = format!("{base}/api/installer/redeem");
    let body = serde_json::json!({ "code": code }).to_string();
    let resp = match agent(opts).post(&url).set("Content-Type", "application/json").set("Accept", "application/json").send_string(&body) {
        Ok(r) => r,
        Err(ureq::Error::Status(404, _)) => return Err(SiteError::InvalidOrExpired),
        Err(ureq::Error::Status(429, _)) => return Err(SiteError::RateLimited),
        Err(ureq::Error::Status(c, _)) => return Err(SiteError::Server(c)),
        Err(ureq::Error::Transport(t)) => return Err(SiteError::Network(t.to_string())),
    };
    if resp.status() != 200 {
        // 3xx (redirects are disabled) and other 2xx are not part of the contract.
        return Err(SiteError::Server(resp.status()));
    }
    let mut buf = Vec::new();
    resp.into_reader().take(opts.max_body as u64 + 1).read_to_end(&mut buf).map_err(|e| SiteError::Network(e.to_string()))?;
    if buf.len() > opts.max_body {
        return Err(SiteError::BadResponse("the answer is too large".into()));
    }
    parse_response(&buf)
}

/// Redeem and resolve the artifact build in one go (what the front ends call).
pub fn redeem_and_resolve(code: &str, base_url: &str, src: &dyn ArtifactSource) -> Result<Redeemed> {
    let mut r = redeem(code, base_url)?;
    r.resolve_artifact(src, Platform::current())?;
    Ok(r)
}

#[derive(Deserialize)]
struct RawResponse {
    config: RawConfig,
    #[serde(default)]
    cfx: Option<RawCfx>,
    #[serde(default)]
    github: Option<RawGithub>,
}

#[derive(Deserialize)]
struct RawConfig {
    mode: String,
    name: String,
    resources: serde_json::Value,
    artifact: RawArtifact,
    #[serde(rename = "gameBuild", default)]
    game_build: Option<u64>,
    ports: RawPorts,
    #[serde(default)]
    hostname: Option<String>,
}

#[derive(Deserialize)]
struct RawArtifact {
    kind: String,
    #[serde(default)]
    build: Option<String>,
}

#[derive(Deserialize)]
struct RawPorts {
    game: u64,
    txadmin: u64,
}

#[derive(Deserialize)]
struct RawCfx {
    #[serde(default)]
    identifier: Option<String>,
    #[serde(default)]
    username: Option<String>,
}

#[derive(Deserialize)]
struct RawGithub {
    #[serde(default)]
    login: Option<String>,
}

/// Printable text only (the values end up on a terminal).
fn clean(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max).collect::<String>().trim().to_string()
}

fn clean_opt(s: Option<String>, max: usize) -> Option<String> {
    s.map(|s| clean(&s, max)).filter(|s| !s.is_empty())
}

fn bad<T>(m: &str) -> Result<T> {
    Err(SiteError::BadResponse(m.into()))
}

/// Local name rules (see the wizard): letters, digits, space, dash, underscore, max 32.
fn local_name(site_name: &str) -> String {
    let mapped: String = site_name.chars().map(|c| if c.is_ascii_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { ' ' }).collect();
    let squeezed = mapped.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut: String = squeezed.chars().take(32).collect();
    let cut = cut.trim().to_string();
    if cut.chars().any(|c| c.is_ascii_alphanumeric()) {
        cut
    } else {
        "My Kora Server".into()
    }
}

fn parse_response(body: &[u8]) -> Result<Redeemed> {
    let raw: RawResponse = serde_json::from_slice(body).map_err(|e| SiteError::BadResponse(e.to_string()))?;
    let c = raw.config;
    let mode = match c.mode.as_str() {
        "production" => Mode::Production,
        "development" => Mode::Development,
        _ => return bad("unknown mode"),
    };
    let name = clean(&c.name, 48);
    if name.is_empty() {
        return bad("missing server name");
    }
    let resources = match &c.resources {
        serde_json::Value::String(s) if s == "all" => ResourceSelection::All,
        serde_json::Value::Array(a) => {
            if a.len() > MAX_RESOURCES {
                return bad("too many resources");
            }
            let mut ids = Vec::new();
            for v in a {
                let Some(id) = v.as_str() else { return bad("resource ids must be text") };
                if id.is_empty() || id.len() > 64 || !id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) || id.starts_with(['-', '.']) {
                    return bad("unexpected resource id");
                }
                ids.push(id.to_string());
            }
            ResourceSelection::Only(ids)
        }
        _ => return bad("unknown resources value"),
    };
    let artifact = match (c.artifact.kind.as_str(), c.artifact.build) {
        ("recommended", _) => ArtifactChoice::Recommended,
        ("build", Some(b)) if (4..=6).contains(&b.len()) && b.bytes().all(|x| x.is_ascii_digit()) => ArtifactChoice::Build(b.parse().map_err(|_| SiteError::BadResponse("bad build".into()))?),
        _ => return bad("unknown artifact choice"),
    };
    let game_build = match c.game_build {
        None => None,
        Some(g) => Some(u32::try_from(g).map_err(|_| SiteError::BadResponse("bad game build".into()))?),
    };
    let port = |p: u64| u16::try_from(p).ok().filter(|p| *p >= 1024);
    let (Some(game), Some(txadmin)) = (port(c.ports.game), port(c.ports.txadmin)) else { return bad("ports out of range") };
    if game == txadmin {
        return bad("the two ports are equal");
    }
    let cfx = raw.cfx.unwrap_or(RawCfx { identifier: None, username: None });
    let cfx_identifier = clean_opt(cfx.identifier, 64).filter(|i| i.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '-')));
    Ok(Redeemed {
        mode,
        name: local_name(&name),
        resources,
        artifact,
        artifact_url: None,
        game_build,
        ports: Ports { game, txadmin },
        hostname: clean_opt(c.hostname, 64),
        cfx_identifier,
        cfx_username: clean_opt(cfx.username, 64),
        github_login: clean_opt(raw.github.and_then(|g| g.login), 64).filter(|l| l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')),
    })
}

#[cfg(test)]
mod tests;
