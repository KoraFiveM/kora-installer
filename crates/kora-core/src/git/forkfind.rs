//! Finding the user's existing fork of an upstream repository, whatever it is called.
//!
//! GitHub allows one fork of a repository network per owner, and that fork does not have to carry
//! the upstream's name: GitHub appends `-1` when the owner already has an unrelated repository of
//! that name, and people rename their forks. So forks are found by their PARENT, from one listing
//! of the owner's forks per run ([`list_forks`]). When that listing cannot be read, a shared cache
//! (`<kora home>/forks-cache.json`, upstream -> fork per owner) is tried as a hint, each entry
//! checked against GitHub before it is used, and then the old same-name lookup.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{repo_name, run, valid_name, valid_repo, CommandRunner, GitError, Output, Result};

/// One fork listed for the owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedFork {
    /// `owner/name` of the fork.
    pub slug: String,
    pub default_branch: Option<String>,
    /// `owner/name` of the repository it was forked from.
    pub parent: Option<String>,
    /// The parent's own parent (a fork of a fork of the upstream).
    pub grandparent: Option<String>,
}

/// Every fork the owner has, from one listing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForkIndex {
    forks: Vec<ListedFork>,
}

impl ForkIndex {
    /// The owner's fork of `upstream`: a direct fork first, then a fork of a fork of it.
    pub fn find(&self, upstream: &str) -> Option<&ListedFork> {
        let is = |p: &Option<String>| {
            p.as_deref()
                .is_some_and(|p| p.eq_ignore_ascii_case(upstream))
        };
        self.forks
            .iter()
            .find(|f| is(&f.parent))
            .or_else(|| self.forks.iter().find(|f| is(&f.grandparent)))
    }

    fn add(&mut self, slug: &str, upstream: &str) {
        if self.find(upstream).is_none() {
            self.forks.push(ListedFork {
                slug: slug.to_string(),
                default_branch: None,
                parent: Some(upstream.to_string()),
                grandparent: None,
            });
        }
    }

    pub fn len(&self) -> usize {
        self.forks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.forks.is_empty()
    }
}

const LIST_QUERY: &str = "query($owner: String!, $endCursor: String) { repositoryOwner(login: $owner) { repositories(isFork: true, ownerAffiliations: [OWNER], first: 100, after: $endCursor) { nodes { nameWithOwner defaultBranchRef { name } parent { nameWithOwner parent { nameWithOwner } } } pageInfo { hasNextPage endCursor } } } }";

/// All forks owned by `owner` (user or organisation), every page, in one `gh api graphql --paginate`.
pub fn list_forks(r: &dyn CommandRunner, owner: &str) -> Result<ForkIndex> {
    if !valid_name(owner) {
        return Err(GitError::Invalid("fork owner", owner.to_string()));
    }
    let owner_arg = format!("owner={owner}");
    let query_arg = format!("query={LIST_QUERY}");
    let args = [
        "api",
        "graphql",
        "--paginate",
        "-f",
        owner_arg.as_str(),
        "-f",
        query_arg.as_str(),
    ];
    let o = run(r, "gh", &args, None)?;
    if !o.ok() {
        return Err(GitError::Command {
            cmd: "gh api graphql (list forks)".into(),
            code: o.code,
            stderr: o.stderr.trim().to_string(),
        });
    }
    parse_listing(&o.stdout, owner).ok_or_else(|| {
        GitError::Parse(
            "gh api graphql (list forks)".into(),
            o.stdout.chars().take(200).collect(),
        )
    })
}

/// Parse the (possibly several, concatenated) pages of the listing. `None` when any page is not
/// a usable answer: a partial listing must never be read as "these are all the forks".
pub fn parse_listing(text: &str, owner: &str) -> Option<ForkIndex> {
    let mut forks = Vec::new();
    let mut pages = 0;
    for doc in serde_json::Deserializer::from_str(text).into_iter::<serde_json::Value>() {
        let doc = doc.ok()?;
        if doc.get("errors").is_some_and(|e| !e.is_null()) {
            return None;
        }
        let nodes = doc
            .pointer("/data/repositoryOwner/repositories/nodes")?
            .as_array()?;
        pages += 1;
        let s = |v: &serde_json::Value, p: &str| {
            v.pointer(p).and_then(|x| x.as_str()).map(str::to_string)
        };
        for n in nodes {
            let Some(slug) = s(n, "/nameWithOwner") else {
                continue;
            };
            // A user's listing can include repositories they only collaborate on.
            let mine = slug
                .split_once('/')
                .is_some_and(|(o, _)| o.eq_ignore_ascii_case(owner));
            if !mine || !valid_repo(&slug) {
                continue;
            }
            forks.push(ListedFork {
                slug,
                default_branch: s(n, "/defaultBranchRef/name"),
                parent: s(n, "/parent/nameWithOwner"),
                grandparent: s(n, "/parent/parent/nameWithOwner"),
            });
        }
    }
    (pages > 0).then_some(ForkIndex { forks })
}

/// The fork `gh repo fork` reported: without a terminal it prints the new fork's address on
/// stdout, or `owner/name already exists` on stderr when GitHub handed back an existing fork
/// (with a terminal: `✓ Created fork owner/name` / `! owner/name already exists`).
pub fn parse_fork_output(out: &Output, owner: &str) -> Option<String> {
    let text = strip_ansi(&format!("{}\n{}", out.stdout, out.stderr));
    text.split_whitespace().find_map(|t| {
        let t = t.trim_matches(|c: char| {
            !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | ':'))
        });
        let t = t.strip_prefix("https://github.com/").unwrap_or(t);
        let t = t.trim_end_matches('/').trim_end_matches(".git");
        let mine = t
            .split_once('/')
            .is_some_and(|(o, _)| o.eq_ignore_ascii_case(owner));
        (mine && valid_repo(t)).then(|| t.to_string())
    })
}

/// Drop terminal colour codes (`ESC [ ... letter`).
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\u{1b}' {
            for d in it.by_ref() {
                if d.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Shared cache
// ---------------------------------------------------------------------------------------------

/// Where the shared cache lives.
pub fn default_cache_path() -> PathBuf {
    crate::installer::registry::kora_home().join("forks-cache.json")
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheFile {
    /// owner (lowercase) -> upstream `owner/name` (lowercase) -> fork `owner/name`.
    #[serde(default)]
    owners: BTreeMap<String, BTreeMap<String, String>>,
}

fn read_cache(path: &Path) -> CacheFile {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Upstream -> fork for one owner. Only a hint: every entry is checked before it is trusted.
#[derive(Debug, Default)]
pub struct ForkCache {
    path: Option<PathBuf>,
    owner: String,
    entries: BTreeMap<String, String>,
    /// Changes not written yet (`None` = remove).
    changes: BTreeMap<String, Option<String>>,
}

impl ForkCache {
    pub fn load(path: Option<&Path>, owner: &str) -> ForkCache {
        let owner = owner.to_ascii_lowercase();
        let entries = path
            .map(read_cache)
            .and_then(|mut f| f.owners.remove(&owner))
            .unwrap_or_default();
        ForkCache {
            path: path.map(Path::to_path_buf),
            owner,
            entries,
            changes: BTreeMap::new(),
        }
    }

    pub fn get(&self, upstream: &str) -> Option<&str> {
        self.entries
            .get(&upstream.to_ascii_lowercase())
            .map(String::as_str)
    }

    pub fn put(&mut self, upstream: &str, fork: &str) {
        let k = upstream.to_ascii_lowercase();
        if self.entries.get(&k).map(String::as_str) != Some(fork) {
            self.entries.insert(k.clone(), fork.to_string());
            self.changes.insert(k, Some(fork.to_string()));
        }
    }

    pub fn forget(&mut self, upstream: &str) {
        let k = upstream.to_ascii_lowercase();
        if self.entries.remove(&k).is_some() {
            self.changes.insert(k, None);
        }
    }

    /// Write the changes: re-read the file (another Kora may have written it meanwhile), apply
    /// ours, write a temporary file named after this process and rename it over the cache.
    pub fn save(&mut self) {
        let Some(path) = self.path.clone() else {
            return;
        };
        if self.changes.is_empty() {
            return;
        }
        let mut file = read_cache(&path);
        let mine = file.owners.entry(self.owner.clone()).or_default();
        for (k, v) in std::mem::take(&mut self.changes) {
            match v {
                Some(f) => mine.insert(k, f),
                None => mine.remove(&k),
            };
        }
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let tmp = path.with_file_name(format!("forks-cache.json.{}.tmp", std::process::id()));
        let Ok(body) = serde_json::to_vec_pretty(&file) else {
            return;
        };
        if std::fs::write(&tmp, body).is_err() || std::fs::rename(&tmp, &path).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Finder: listing first, then the verified cache, then the same-name lookup
// ---------------------------------------------------------------------------------------------

/// What exists for one upstream before forking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    Missing,
    /// The owner's fork of the upstream (`slug`), with its default branch when known.
    Present {
        slug: String,
        default_branch: Option<String>,
    },
    /// A repository with the upstream's name exists but is not a fork of it (only reported by the
    /// same-name lookup used when the listing cannot be read).
    Foreign(String),
}

/// Is `slug` really the owner's fork of `upstream`? Follows renames (GitHub answers an old name
/// with the renamed repository). Only a definite 404 is `Missing`; other failures are errors.
pub fn probe_repo(r: &dyn CommandRunner, slug: &str, owner: &str, upstream: &str) -> Result<Found> {
    let path = format!("repos/{slug}");
    let o = run(r, "gh", &["api", &path], None)?;
    if !o.ok() {
        // Only a definite "not found" means the fork is missing. Anything else (rate limit, network
        // error, expired token, server error) must NOT be read as "missing": that would send a fork
        // request for a repo that may already exist.
        if o.stderr.contains("HTTP 404") || o.stderr.contains("Not Found") {
            return Ok(Found::Missing);
        }
        return Err(GitError::Command {
            cmd: format!("gh api {path}"),
            code: o.code,
            stderr: o.stderr,
        });
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&o.stdout) else {
        // An answer we cannot inspect: the repository is there (old behaviour).
        return Ok(Found::Present {
            slug: slug.to_string(),
            default_branch: None,
        });
    };
    let s = |p: &str| v.pointer(p).and_then(|x| x.as_str()).map(str::to_string);
    let real = s("/full_name")
        .filter(|f| valid_repo(f))
        .unwrap_or_else(|| slug.to_string());
    if !real
        .split_once('/')
        .is_some_and(|(o, _)| o.eq_ignore_ascii_case(owner))
    {
        // Moved to another owner: not this owner's fork any more.
        return Ok(Found::Missing);
    }
    let default_branch = s("/default_branch");
    if let Some(is_fork) = v.get("fork").and_then(|f| f.as_bool()) {
        if !is_fork {
            return Ok(Found::Foreign(format!(
                "{real} exists but is not a fork of {upstream}"
            )));
        }
        let parent = s("/parent/full_name");
        let source = s("/source/full_name");
        let matches = |p: &Option<String>| {
            p.as_deref()
                .is_some_and(|p| p.eq_ignore_ascii_case(upstream))
        };
        if (parent.is_some() || source.is_some()) && !matches(&parent) && !matches(&source) {
            let from = parent.or(source).unwrap_or_default();
            return Ok(Found::Foreign(format!(
                "{real} is a fork of {from}, not {upstream}"
            )));
        }
    }
    Ok(Found::Present {
        slug: real,
        default_branch,
    })
}

/// Finds forks for one owner during one run (one listing, refreshed only after a fork request
/// whose answer could not be read).
pub struct ForkFinder {
    owner: String,
    index: Option<ForkIndex>,
    listing_tried: bool,
    cache: ForkCache,
}

impl ForkFinder {
    pub fn new(owner: &str, cache_path: Option<&Path>) -> ForkFinder {
        ForkFinder {
            owner: owner.to_string(),
            index: None,
            listing_tried: false,
            cache: ForkCache::load(cache_path, owner),
        }
    }

    fn relist(&mut self, r: &dyn CommandRunner) -> bool {
        self.listing_tried = true;
        match list_forks(r, &self.owner) {
            Ok(i) => {
                self.index = Some(i);
                true
            }
            Err(_) => false,
        }
    }

    /// The owner's fork of `upstream`, if any.
    pub fn find(&mut self, r: &dyn CommandRunner, upstream: &str) -> Result<Found> {
        if !self.listing_tried {
            self.relist(r);
        }
        if let Some(idx) = &self.index {
            return Ok(match idx.find(upstream) {
                Some(f) => {
                    let found = Found::Present {
                        slug: f.slug.clone(),
                        default_branch: f.default_branch.clone(),
                    };
                    self.cache.put(upstream, &f.slug);
                    found
                }
                None => {
                    self.cache.forget(upstream);
                    Found::Missing
                }
            });
        }
        // No listing: the cached fork, checked first.
        if let Some(slug) = self.cache.get(upstream).map(str::to_string) {
            if valid_repo(&slug) {
                if let Found::Present {
                    slug,
                    default_branch,
                } = probe_repo(r, &slug, &self.owner, upstream)?
                {
                    self.cache.put(upstream, &slug);
                    return Ok(Found::Present {
                        slug,
                        default_branch,
                    });
                }
            }
            self.cache.forget(upstream);
        }
        // Then the old lookup by the upstream's name.
        let same = format!("{}/{}", self.owner, repo_name(upstream));
        let found = probe_repo(r, &same, &self.owner, upstream)?;
        if let Found::Present { slug, .. } = &found {
            self.cache.put(upstream, slug);
        }
        Ok(found)
    }

    /// After `gh repo fork` succeeded: which fork did GitHub create or hand back? Read from gh's
    /// answer, else from a fresh listing, else from the same-name lookup. `None`: not found.
    pub fn after_fork(
        &mut self,
        r: &dyn CommandRunner,
        upstream: &str,
        out: &Output,
    ) -> Option<String> {
        let slug = parse_fork_output(out, &self.owner).or_else(|| {
            if self.relist(r) {
                if let Some(f) = self.index.as_ref().and_then(|i| i.find(upstream)) {
                    return Some(f.slug.clone());
                }
            }
            let same = format!("{}/{}", self.owner, repo_name(upstream));
            match probe_repo(r, &same, &self.owner, upstream) {
                Ok(Found::Present { slug, .. }) => Some(slug),
                _ => None,
            }
        })?;
        self.cache.put(upstream, &slug);
        if let Some(i) = &mut self.index {
            i.add(&slug, upstream);
        }
        Some(slug)
    }

    pub fn save(&mut self) {
        self.cache.save();
    }
}
