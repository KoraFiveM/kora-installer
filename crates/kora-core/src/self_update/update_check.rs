//! "A new version is available" notice. Never blocks startup: the network check runs in a
//! background thread (3 s timeout) and only in interactive runs; the answer is cached for 24 h in
//! the Kora config folder. Disabled by `KORA_NO_UPDATE_CHECK=1`, by `CI`, and when not on a TTY.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::net::Net;
use super::release::{parse_releases, pick_newest};
use super::version::Version;
use super::{releases_url, Source};

pub const OPT_OUT_ENV: &str = "KORA_NO_UPDATE_CHECK";
pub const TTL_SECS: u64 = 24 * 60 * 60;
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Cache {
    checked_unix: u64,
    latest: String,
}

pub fn cache_path() -> PathBuf {
    crate::installer::registry::kora_home().join("update-check.json")
}

fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Is the check allowed at all? `get` reads the environment.
pub fn enabled(get: &dyn Fn(&str) -> Option<String>, interactive: bool) -> bool {
    let set = |k: &str| get(k).is_some_and(|v| !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false"));
    interactive && !set(OPT_OUT_ENV) && !set("CI")
}

pub fn enabled_here(interactive: bool) -> bool {
    enabled(&|k| std::env::var(k).ok(), interactive)
}

fn read_cache(path: &Path) -> Option<Cache> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn write_cache(path: &Path, c: &Cache) {
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, serde_json::to_string(c).unwrap_or_default()).is_ok() && std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// The cached newest version, whatever its age (no network).
pub fn cached_latest() -> Option<Version> {
    cached_latest_at(&cache_path())
}

pub fn cached_latest_at(path: &Path) -> Option<Version> {
    Version::parse(&read_cache(path)?.latest)
}

/// Fresh enough to skip the network?
fn fresh(c: &Cache, now: u64) -> bool {
    now.saturating_sub(c.checked_unix) < TTL_SECS && c.checked_unix <= now + 60
}

/// Ask GitHub for the newest release this build would update to (stable only, unless this build is a
/// pre-release) and store it. Blocking, bounded by `net`.
pub fn refresh(src: &Source, net: &Net, path: &Path, now: u64) -> Result<Version, String> {
    let body = net.get_text(&releases_url(&src.api_base), super::net::MAX_TEXT)?;
    let rels = parse_releases(&body)?;
    let v = pick_newest(&rels, &super::running_version()).and_then(|r| r.version()).ok_or("no published release found")?;
    write_cache(path, &Cache { checked_unix: now, latest: v.to_string() });
    Ok(v)
}

/// `Some(version)` when `latest` is newer than `current`.
pub fn newer(current: &Version, latest: Option<Version>) -> Option<Version> {
    latest.filter(|l| l > current)
}

pub fn notice_text(v: &Version) -> String {
    format!("Kora v{v} is available - run: kora self-update")
}

/// One-line notice from the cache alone (for the TUI footer). `None` when disabled, unknown or current.
pub fn notice_line() -> Option<String> {
    if !enabled_here(true) {
        return None;
    }
    let cur = Version::parse(env!("CARGO_PKG_VERSION"))?;
    newer(&cur, cached_latest()).map(|v| notice_text(&v))
}

/// A check started at program start; collect the notice when the command is done.
pub struct Pending {
    cached: Option<Version>,
    rx: Option<mpsc::Receiver<Option<Version>>>,
}

impl Pending {
    /// Starts the background refresh when the cache is stale. Returns `None` when checks are off.
    pub fn start(interactive: bool) -> Option<Pending> {
        if !enabled_here(interactive) {
            return None;
        }
        let path = cache_path();
        let now = now_unix();
        let cache = read_cache(&path);
        let cached = cache.as_ref().and_then(|c| Version::parse(&c.latest));
        if cache.as_ref().is_some_and(|c| fresh(c, now)) {
            return Some(Pending { cached, rx: None });
        }
        let (tx, rx) = mpsc::channel();
        let src = Source::from_env();
        let net = Net { connect_timeout: CHECK_TIMEOUT, read_timeout: CHECK_TIMEOUT, total_timeout: CHECK_TIMEOUT, ..src.net.clone() };
        std::thread::spawn(move || {
            let _ = tx.send(refresh(&src, &net, &path, now).ok());
        });
        Some(Pending { cached, rx: Some(rx) })
    }

    /// The notice line, waiting at most `wait` for a refresh still in flight.
    pub fn finish(self, wait: Duration) -> Option<String> {
        let latest = match self.rx {
            Some(rx) => rx.recv_timeout(wait).ok().flatten().or(self.cached),
            None => self.cached,
        };
        let cur = Version::parse(env!("CARGO_PKG_VERSION"))?;
        newer(&cur, latest).map(|v| notice_text(&v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| pairs.iter().find(|(a, _)| *a == k).map(|(_, v)| v.to_string())
    }

    #[test]
    fn opt_out_and_ci_and_tty() {
        assert!(enabled(&env(&[]), true));
        assert!(!enabled(&env(&[]), false), "non-TTY");
        assert!(!enabled(&env(&[("KORA_NO_UPDATE_CHECK", "1")]), true));
        assert!(!enabled(&env(&[("CI", "true")]), true));
        assert!(enabled(&env(&[("KORA_NO_UPDATE_CHECK", "0")]), true));
        assert!(enabled(&env(&[("CI", "")]), true));
    }

    #[test]
    fn ttl() {
        let c = Cache { checked_unix: 1000, latest: "0.2.2".into() };
        assert!(fresh(&c, 1000 + TTL_SECS - 1));
        assert!(!fresh(&c, 1000 + TTL_SECS));
        assert!(!fresh(&c, 100), "a cache dated in the future is not trusted");
    }

    #[test]
    fn newer_only_when_greater() {
        let cur = Version::parse("0.2.1").unwrap();
        assert!(newer(&cur, Version::parse("0.2.2-rc1")).is_some());
        assert!(newer(&cur, Version::parse("0.2.1")).is_none());
        assert!(newer(&cur, Version::parse("0.2.0")).is_none());
        assert!(newer(&cur, None).is_none());
        assert_eq!(notice_text(&Version::parse("0.2.2").unwrap()), "Kora v0.2.2 is available - run: kora self-update");
    }

    #[test]
    fn cache_roundtrip_and_garbage() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("sub").join("update-check.json");
        assert!(cached_latest_at(&p).is_none());
        write_cache(&p, &Cache { checked_unix: 5, latest: "0.9.0".into() });
        assert_eq!(cached_latest_at(&p).unwrap().to_string(), "0.9.0");
        std::fs::write(&p, "not json").unwrap();
        assert!(cached_latest_at(&p).is_none());
    }
}
