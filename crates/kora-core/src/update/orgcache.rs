//! One cheap request that says which repositories of the Kora organisation changed.
//!
//! `GET /orgs/KoraFiveM/repos?per_page=100&sort=pushed` lists every repository with its `pushed_at` in a single call
//! (unauthenticated: 60 requests/hour per IP, so it is used at most once per check). The ETag makes a repeat check cheap
//! (a 304 has no body), but for unauthenticated calls GitHub still counts a 304 against the limit. The answer is only ever a HINT to skip a `git fetch`: a clone is skipped when the organisation
//! still reports the `pushed_at` that was current the last time that very clone was really fetched. Any failure
//! (offline, rate limited, odd answer) yields no hint, so every resource is then fetched as usual.
//! The cache file (`<kora home>/org-cache.json`) holds only public data: the ETag, repository names and push times.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const ORG: &str = "KoraFiveM";
pub const ORG_URL: &str = "https://api.github.com/orgs/KoraFiveM/repos?per_page=100&sort=pushed";

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OrgCache {
    pub etag: Option<String>,
    pub checked_unix: u64,
    /// `owner/name` (lowercase) -> `pushed_at`, as of the last 200/304.
    pub repos: BTreeMap<String, String>,
    /// `<clone path>\n<owner/name>` -> the `pushed_at` that was current when that clone was last fetched.
    pub verified: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrgReply {
    Fresh { etag: Option<String>, body: String },
    NotModified,
    RateLimited,
    Failed(String),
}

pub trait OrgSource {
    fn get(&self, etag: Option<&str>) -> OrgReply;
}

/// The real endpoint (HTTPS, short timeouts: a slow GitHub must not slow the check).
pub struct HttpOrg;

impl OrgSource for HttpOrg {
    fn get(&self, etag: Option<&str>) -> OrgReply {
        let agent = ureq::AgentBuilder::new()
            .redirects(0)
            .timeout_connect(Duration::from_secs(4))
            .timeout(Duration::from_secs(10))
            .user_agent("kora-installer")
            .build();
        let mut req = agent
            .get(ORG_URL)
            .set("Accept", "application/vnd.github+json");
        if let Some(e) = etag {
            req = req.set("If-None-Match", e);
        }
        match req.call() {
            Ok(r) if r.status() == 304 => OrgReply::NotModified,
            Ok(r) if r.status() == 200 => {
                let etag = r.header("etag").map(str::to_string);
                match r.into_string() {
                    Ok(body) => OrgReply::Fresh { etag, body },
                    Err(e) => OrgReply::Failed(e.to_string()),
                }
            }
            Ok(r) => OrgReply::Failed(format!("status {}", r.status())),
            Err(ureq::Error::Status(c, _)) if c == 403 || c == 429 => OrgReply::RateLimited,
            Err(ureq::Error::Status(304, _)) => OrgReply::NotModified,
            Err(e) => OrgReply::Failed(e.to_string()),
        }
    }
}

pub fn cache_path() -> PathBuf {
    crate::installer::registry::kora_home().join("org-cache.json")
}

#[derive(serde::Deserialize)]
struct RepoRow {
    full_name: String,
    pushed_at: Option<String>,
}

fn parse_repos(body: &str) -> Option<BTreeMap<String, String>> {
    let rows: Vec<RepoRow> = serde_json::from_str(body).ok()?;
    Some(
        rows.into_iter()
            .filter_map(|r| Some((r.full_name.to_ascii_lowercase(), r.pushed_at?)))
            .collect(),
    )
}

/// What the organisation says about its repositories, plus the record of what each clone last verified.
pub struct OrgHints {
    cache: OrgCache,
    path: Option<PathBuf>,
    /// True only when this check got a current answer (200, or 304 on a cache we hold).
    live: bool,
}

impl OrgHints {
    /// One request. `path` is where the cache is read from and written back to (None: nothing persisted).
    pub fn load(src: &dyn OrgSource, path: Option<&Path>, now: u64) -> OrgHints {
        let mut cache: OrgCache = path
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let etag = if cache.repos.is_empty() {
            None
        } else {
            cache.etag.clone()
        };
        let live = match src.get(etag.as_deref()) {
            OrgReply::NotModified => !cache.repos.is_empty(),
            OrgReply::Fresh { etag, body } => match parse_repos(&body) {
                Some(repos) => {
                    cache.etag = etag;
                    cache.repos = repos;
                    true
                }
                None => false,
            },
            OrgReply::RateLimited | OrgReply::Failed(_) => false,
        };
        if live {
            cache.checked_unix = now;
        }
        OrgHints {
            cache,
            path: path.map(Path::to_path_buf),
            live,
        }
    }

    pub fn is_live(&self) -> bool {
        self.live
    }

    fn key(clone: &Path, slug: &str) -> String {
        format!("{}\n{}", clone.display(), slug.to_ascii_lowercase())
    }

    fn pushed(&self, slug: &str) -> Option<&String> {
        if !self.live {
            return None;
        }
        self.cache.repos.get(&slug.to_ascii_lowercase())
    }

    /// True when the organisation still reports the push time this clone was last fetched at.
    pub fn is_current(&self, clone: &Path, slug: &str) -> bool {
        match (
            self.pushed(slug),
            self.cache.verified.get(&Self::key(clone, slug)),
        ) {
            (Some(now), Some(then)) => now == then,
            _ => false,
        }
    }

    /// Record that `clone` was really fetched while the organisation reported its current push time.
    pub fn mark_fetched(&mut self, clone: &Path, slug: &str) {
        if let Some(p) = self.pushed(slug).cloned() {
            self.cache.verified.insert(Self::key(clone, slug), p);
        }
    }

    /// Forget a clone whose fetch failed, so the next check fetches it.
    pub fn forget(&mut self, clone: &Path, slug: &str) {
        self.cache.verified.remove(&Self::key(clone, slug));
    }

    pub fn save(&self) {
        let Some(path) = &self.path else { return };
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        // Per process: two Kora processes saving at once never write into the same temp file.
        let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
        if std::fs::write(&tmp, serde_json::to_string(&self.cache).unwrap_or_default()).is_ok()
            && std::fs::rename(&tmp, path).is_err()
        {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Fake {
        replies: RefCell<Vec<OrgReply>>,
        seen_etags: RefCell<Vec<Option<String>>>,
    }
    impl Fake {
        fn new(r: Vec<OrgReply>) -> Fake {
            Fake {
                replies: RefCell::new(r),
                seen_etags: RefCell::new(vec![]),
            }
        }
    }
    impl OrgSource for Fake {
        fn get(&self, etag: Option<&str>) -> OrgReply {
            self.seen_etags.borrow_mut().push(etag.map(str::to_string));
            self.replies.borrow_mut().remove(0)
        }
    }

    const BODY: &str = r#"[{"full_name":"KoraFiveM/kora_core","pushed_at":"2026-11-16T09:38:21Z"},{"full_name":"KoraFiveM/empty","pushed_at":null}]"#;
    fn fresh() -> OrgReply {
        OrgReply::Fresh {
            etag: Some("W/\"e1\"".into()),
            body: BODY.into(),
        }
    }
    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kora-org-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.join("org-cache.json")
    }

    #[test]
    fn ok_answer_is_stored_and_marks_clones() {
        let p = tmp("ok");
        let mut h = OrgHints::load(&Fake::new(vec![fresh()]), Some(&p), 100);
        assert!(h.is_live());
        let clone = Path::new("/srv/a/resources/kora_core");
        assert!(
            !h.is_current(clone, "KoraFiveM/kora_core"),
            "never fetched yet"
        );
        h.mark_fetched(clone, "KoraFiveM/kora_core");
        assert!(h.is_current(clone, "korafivem/KORA_CORE"));
        assert!(
            !h.is_current(
                Path::new("/srv/b/resources/kora_core"),
                "KoraFiveM/kora_core"
            ),
            "another clone has its own record"
        );
        assert!(!h.is_current(clone, "KoraFiveM/unknown"));
        h.save();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("e1") && !text.to_lowercase().contains("token"));
    }

    #[test]
    fn not_modified_reuses_the_saved_answer_and_sends_the_etag() {
        let p = tmp("304");
        let mut h = OrgHints::load(&Fake::new(vec![fresh()]), Some(&p), 100);
        h.mark_fetched(Path::new("/c"), "KoraFiveM/kora_core");
        h.save();
        let f = Fake::new(vec![OrgReply::NotModified]);
        let h2 = OrgHints::load(&f, Some(&p), 200);
        assert_eq!(f.seen_etags.borrow()[0].as_deref(), Some("W/\"e1\""));
        assert!(h2.is_live());
        assert!(h2.is_current(Path::new("/c"), "KoraFiveM/kora_core"));
    }

    #[test]
    fn rate_limited_offline_or_garbage_gives_no_hint() {
        let p = tmp("fail");
        let mut h = OrgHints::load(&Fake::new(vec![fresh()]), Some(&p), 100);
        h.mark_fetched(Path::new("/c"), "KoraFiveM/kora_core");
        h.save();
        for bad in [
            OrgReply::RateLimited,
            OrgReply::Failed("offline".into()),
            OrgReply::Fresh {
                etag: None,
                body: "not json".into(),
            },
        ] {
            let h = OrgHints::load(&Fake::new(vec![bad]), Some(&p), 300);
            assert!(!h.is_live());
            assert!(
                !h.is_current(Path::new("/c"), "KoraFiveM/kora_core"),
                "a stale snapshot must never be trusted"
            );
        }
    }

    #[test]
    fn not_modified_without_a_saved_answer_is_no_hint() {
        let p = tmp("empty");
        let f = Fake::new(vec![OrgReply::NotModified]);
        let h = OrgHints::load(&f, Some(&p), 1);
        assert_eq!(f.seen_etags.borrow()[0], None);
        assert!(!h.is_live());
    }

    #[test]
    fn a_new_push_time_means_fetch_again() {
        let p = tmp("moved");
        let mut h = OrgHints::load(&Fake::new(vec![fresh()]), Some(&p), 1);
        h.mark_fetched(Path::new("/c"), "KoraFiveM/kora_core");
        h.save();
        let moved = OrgReply::Fresh {
            etag: Some("e2".into()),
            body: BODY.replace("09:38:21", "09:40:00"),
        };
        let h2 = OrgHints::load(&Fake::new(vec![moved]), Some(&p), 2);
        assert!(h2.is_live());
        assert!(!h2.is_current(Path::new("/c"), "KoraFiveM/kora_core"));
    }
}
