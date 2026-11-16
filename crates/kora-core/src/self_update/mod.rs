//! Self-update of the `kora` program itself, plus the "new version available" notice.
//!
//! Source of truth is the newest published GitHub release of KoraFiveM/kora-installer,
//! pre-releases included. The new program is downloaded next to the target, verified against
//! the release's `.sha256` BEFORE it is moved or executed, then swapped in atomically
//! (see [`replace`]). Updates the installed app copy when one exists, otherwise the running file.

pub mod cli;
pub mod net;
pub mod release;
pub mod replace;
pub mod update_check;
pub mod version;

use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use net::Net;
use release::{parse_checksum, parse_releases, pick_newest, pick_tag, select_assets, Release};
use replace::{Flow, Fs};
use version::Version;

pub const REPO: &str = "KoraFiveM/kora-installer";
pub const DEFAULT_API: &str = "https://api.github.com";
/// Test seam: a loopback `http://127.0.0.1:PORT` base replacing `https://api.github.com`.
pub const API_ENV: &str = "KORA_SELF_UPDATE_API";

#[derive(Debug, thiserror::Error)]
pub enum SelfUpdateError {
    #[error("could not reach GitHub: {0}")]
    Network(String),
    #[error("{0}")]
    Release(String),
    #[error("there is no published build of Kora for this system")]
    UnsupportedPlatform,
    #[error("the download does not match its checksum (expected {expected}, got {actual}); nothing was changed")]
    ChecksumMismatch { expected: String, actual: String },
    #[error("{0}")]
    Replace(String),
    #[error("{0}")]
    Io(String),
}

#[derive(Debug, Clone)]
pub struct Source {
    pub api_base: String,
    pub net: Net,
}

impl Source {
    pub fn production() -> Source {
        Source { api_base: DEFAULT_API.to_string(), net: Net::default() }
    }

    /// Production, unless [`API_ENV`] names a loopback http base (anything else is ignored).
    pub fn from_env() -> Source {
        Source::from_override(std::env::var(API_ENV).ok().as_deref())
    }

    pub fn from_override(v: Option<&str>) -> Source {
        let mut s = Source::production();
        if let Some(v) = v {
            if net::check_url(v, true).is_ok() && v.starts_with("http://") {
                s.api_base = v.trim_end_matches('/').to_string();
                s.net.allow_loopback = true;
            }
        }
        s
    }
}

pub fn releases_url(api_base: &str) -> String {
    // 30 like install.sh / install.ps1: a run of pre-releases must not hide the latest stable one.
    format!("{}/repos/{REPO}/releases?per_page=30", api_base.trim_end_matches('/'))
}

pub fn fetch_releases(src: &Source) -> Result<Vec<Release>, SelfUpdateError> {
    let body = src.net.get_text(&releases_url(&src.api_base), net::MAX_TEXT).map_err(SelfUpdateError::Network)?;
    parse_releases(&body).map_err(SelfUpdateError::Release)
}

pub fn running_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("package version is semver")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    UpToDate { latest: Version },
    Update(Release),
}

/// Without `want`: only ever move forward, and to stable releases only unless `current` is itself a
/// pre-release. With `want` (`--version`): exactly that release, any direction, pre-releases too.
pub fn decide(current: &Version, releases: &[Release], want: Option<&str>) -> Result<Decision, SelfUpdateError> {
    if let Some(w) = want {
        return pick_tag(releases, w)
            .cloned()
            .map(Decision::Update)
            .ok_or_else(|| SelfUpdateError::Release(format!("there is no published release {w}")));
    }
    let newest = pick_newest(releases, current).ok_or_else(|| SelfUpdateError::Release("no published release was found".into()))?;
    let v = newest.version().expect("picked releases have versions");
    Ok(if &v > current { Decision::Update(newest.clone()) } else { Decision::UpToDate { latest: v } })
}

// ---------------------------------------------------------------------------------------------
// Which file to update
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub exe: PathBuf,
    /// `app.json` beside the installed app copy (version + hash), when the app copy is the target.
    pub state: Option<PathBuf>,
    pub app_copy: bool,
}

/// The Kora app copy, when `kora app install` put one on this machine: `(exe, app state file)`.
pub fn installed_app() -> Option<(PathBuf, PathBuf)> {
    let st = crate::app_install::status();
    if !(st.installed && st.exe_exists) {
        return None;
    }
    let layout = crate::app_install::Layout::from_env(&st.exe).ok()?;
    Some((layout.exe, layout.state))
}

/// The installed app copy when there is one (its recorded version is updated afterwards, so
/// `kora app status` stays truthful), otherwise the running file.
pub fn resolve_target(running: &Path, windows: bool, app: Option<(PathBuf, PathBuf)>) -> Target {
    if let Some((exe, state)) = app {
        return Target { exe, state: Some(state), app_copy: true };
    }
    let exe = if windows { running.to_path_buf() } else { std::fs::canonicalize(running).unwrap_or_else(|_| running.to_path_buf()) };
    Target { exe, state: None, app_copy: false }
}

/// Can files be created next to `exe`? `false` means administrator rights are needed.
pub fn dir_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".kora-write-test-{}", std::process::id()));
    match std::fs::OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------------------------
// Download, verify, replace
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Applied {
    pub target: PathBuf,
    pub backup: Option<PathBuf>,
    pub sha256: String,
}

struct HashWriter<W: Write> {
    inner: W,
    hash: Sha256,
}
impl<W: Write> Write for HashWriter<W> {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(b)?;
        self.hash.update(&b[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

fn temp_in(dir: &Path) -> (PathBuf, std::io::Result<std::fs::File>) {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    let p = dir.join(format!(".kora-update-{}-{nanos}.tmp", std::process::id()));
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600); // owner-only until verified
    let f = o.open(&p);
    (p, f)
}

#[cfg(unix)]
fn finish_permissions(tmp: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    // Plain rwx bits only: a setuid/setgid/sticky bit on the old file is never carried over.
    let mode = std::fs::metadata(target).map(|m| m.permissions().mode() & 0o777).unwrap_or(0o755) | 0o100;
    std::fs::set_permissions(tmp, std::fs::Permissions::from_mode(mode))
}
#[cfg(not(unix))]
fn finish_permissions(_: &Path, _: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Download `asset` of `rel` and its checksum, verify, then swap it in for `target`.
pub fn apply(src: &Source, rel: &Release, asset: &str, target: &Path, fs: &dyn Fs, flow: Flow) -> Result<Applied, SelfUpdateError> {
    let (bin, sum) = select_assets(rel, asset).map_err(SelfUpdateError::Release)?;
    let sum_text = src.net.get_text(&sum.browser_download_url, 4096).map_err(SelfUpdateError::Network)?;
    let expected = parse_checksum(&sum_text, asset).map_err(SelfUpdateError::Release)?;

    let dir = target.parent().ok_or_else(|| SelfUpdateError::Io("the program has no folder".into()))?;
    let (tmp, file) = temp_in(dir);
    let file = file.map_err(|e| SelfUpdateError::Io(format!("cannot write in {}: {e}", dir.display())))?;
    let result = (|| {
        let mut w = HashWriter { inner: file, hash: Sha256::new() };
        let n = src.net.download(&bin.browser_download_url, &mut w, net::MAX_BINARY).map_err(SelfUpdateError::Network)?;
        w.flush().and_then(|_| w.inner.sync_all()).map_err(|e| SelfUpdateError::Io(e.to_string()))?;
        let actual = hex::encode(w.hash.finalize());
        if n == 0 || actual != expected {
            return Err(SelfUpdateError::ChecksumMismatch { expected, actual });
        }
        // verified: only now does it become an executable file and move
        finish_permissions(&tmp, target).map_err(|e| SelfUpdateError::Io(e.to_string()))?;
        let backup = replace::replace(fs, flow, target, &tmp).map_err(|e| SelfUpdateError::Replace(e.message))?;
        Ok(Applied { target: target.to_path_buf(), backup, sha256: actual })
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Keep `app.json` (written by the app install) in step with the new program.
pub fn update_app_state(state: &Path, version: &Version, sha256: &str) -> std::io::Result<()> {
    let mut v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(state)?).map_err(std::io::Error::other)?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    if let Some(o) = v.as_object_mut() {
        o.insert("version".into(), version.to_string().into());
        o.insert("sha256".into(), sha256.into());
        o.insert("installed_unix".into(), now.into());
    }
    let tmp = state.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&v).map_err(std::io::Error::other)?)?;
    crate::fsutil::rename_retrying(&tmp, state)
}

/// Run at the start of every command: remove the `<exe>.old` a Windows self-update left behind
/// (for the running program and the installed app copy). Silent and best effort.
pub fn startup_cleanup() {
    if !cfg!(windows) {
        return;
    }
    if let Ok(me) = std::env::current_exe() {
        replace::cleanup_old(&replace::RealFs, true, &me);
        // the app copy's leftover too (cheap: no status probe at startup)
        if let Ok(l) = crate::app_install::Layout::from_env(&me) {
            if l.exe != me {
                replace::cleanup_old(&replace::RealFs, true, &l.exe);
            }
        }
    }
}

#[cfg(test)]
mod tests;
