//! Catalog client: schema v2 types, loader (remote with embedded fallback),
//! validation, selection helpers and version resolution.
//!
//! Network access is behind [`CatalogSource`] so tests stay offline.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Public catalog location.
pub const CATALOG_URL: &str = "https://docs.koraframe.work/resources.json";

/// Snapshot of the catalog embedded in the binary (refreshed by CI).
pub const EMBEDDED_SNAPSHOT: &str = include_str!("../../assets/resources.json");

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("catalog parse error: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("catalog fetch error: {0}")]
    Fetch(String),
    #[error("catalog invalid: {}", .0.join("; "))]
    Invalid(Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Group {
    Pre,
    Kora,
    Standalone,
    CfxDefault,
    DefaultMaps,
    Compat,
    Props,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Kora,
    Dependency,
    Optional,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    pub tag: String,
    pub commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resource {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub repository: String,
    /// Relative to the server's `resources/` dir, e.g. `[kora]/kora_core`.
    pub path: String,
    pub group: Group,
    pub kind: Kind,
    pub branch: String,
    /// Install order; may be absent in older catalogs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<u32>,
    #[serde(default)]
    pub latest: Option<String>,
    #[serde(default)]
    pub versions: Vec<Release>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub schema_version: u32,
    pub resources: Vec<Resource>,
}

/// Prod pins releases; dev tracks the branch head.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Prod,
    Dev,
}

/// What to check out for a resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedVersion {
    /// A pinned release (tag + exact commit).
    Pinned { version: String, tag: String, commit: String },
    /// Head of the branch.
    BranchHead { branch: String },
}

impl Resource {
    /// The release named by `latest`, falling back to the highest listed (first) entry.
    pub fn latest_release(&self) -> Option<&Release> {
        self.latest
            .as_ref()
            .and_then(|l| self.versions.iter().find(|v| &v.version == l))
            .or_else(|| self.versions.first())
    }

    pub fn resolve(&self, mode: Mode) -> ResolvedVersion {
        match (mode, self.latest_release()) {
            (Mode::Prod, Some(r)) => ResolvedVersion::Pinned {
                version: r.version.clone(),
                tag: r.tag.clone(),
                commit: r.commit.clone(),
            },
            _ => ResolvedVersion::BranchHead { branch: self.branch.clone() },
        }
    }

    pub fn is_internal(&self) -> bool {
        self.kind == Kind::Internal
    }
}

fn is_sha(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_safe_path(p: &str) -> bool {
    !p.is_empty()
        && !p.starts_with('/')
        && !p.contains('\\')
        && !p.chars().any(char::is_whitespace)
        && !p.split('/').any(|s| s == "..")
}

/// `https://github.com/<owner>/<repo>.git`
pub fn is_valid_repo_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://github.com/") else { return false };
    let Some(rest) = rest.strip_suffix(".git") else { return false };
    let mut parts = rest.split('/');
    let ok = |s: &str| {
        !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
    };
    matches!((parts.next(), parts.next(), parts.next()), (Some(a), Some(b), None) if ok(a) && ok(b))
}

impl Catalog {
    pub fn parse(json: &str) -> Result<Catalog, CatalogError> {
        Ok(serde_json::from_str(json)?)
    }

    /// Parse and validate.
    pub fn parse_validated(json: &str) -> Result<Catalog, CatalogError> {
        let c = Self::parse(json)?;
        c.validate()?;
        Ok(c)
    }

    pub fn embedded() -> Result<Catalog, CatalogError> {
        Self::parse_validated(EMBEDDED_SNAPSHOT)
    }

    /// Collect every problem rather than stopping at the first.
    pub fn validate(&self) -> Result<(), CatalogError> {
        let mut errs = Vec::new();
        if self.schema_version != 2 {
            errs.push(format!("unsupported schemaVersion {}", self.schema_version));
        }
        let (mut ids, mut paths, mut orders) = (HashSet::new(), HashSet::new(), HashSet::new());
        for r in &self.resources {
            if !ids.insert(r.id.as_str()) {
                errs.push(format!("duplicate id {}", r.id));
            }
            if !paths.insert(r.path.as_str()) {
                errs.push(format!("duplicate path {}", r.path));
            }
            if let Some(o) = r.order {
                if o == 0 || !orders.insert(o) {
                    errs.push(format!("{}: invalid or duplicate order {o}", r.id));
                }
            }
            if !is_valid_repo_url(&r.repository) {
                errs.push(format!("{}: bad repository url {}", r.id, r.repository));
            }
            if !is_safe_path(&r.path) {
                errs.push(format!("{}: unsafe path {}", r.id, r.path));
            }
            if r.branch.is_empty() {
                errs.push(format!("{}: empty branch", r.id));
            }
            for v in &r.versions {
                if !is_sha(&v.commit) {
                    errs.push(format!("{}: bad commit for {}", r.id, v.version));
                }
            }
            if let Some(l) = &r.latest {
                if !r.versions.iter().any(|v| &v.version == l) {
                    errs.push(format!("{}: latest {l} not in versions", r.id));
                }
            }
            if matches!(r.kind, Kind::Kora | Kind::Optional) && r.versions.is_empty() {
                errs.push(format!("{}: kora/optional resource needs a release", r.id));
            }
        }
        if errs.is_empty() { Ok(()) } else { Err(CatalogError::Invalid(errs)) }
    }

    pub fn find(&self, id: &str) -> Option<&Resource> {
        self.resources.iter().find(|r| r.id == id)
    }

    pub fn by_kind(&self, kind: Kind) -> Vec<&Resource> {
        self.resources.iter().filter(|r| r.kind == kind).collect()
    }

    pub fn by_group(&self, group: Group) -> Vec<&Resource> {
        self.resources.iter().filter(|r| r.group == group).collect()
    }

    /// Everything except `internal` resources (what public installs may see).
    pub fn public(&self) -> Vec<&Resource> {
        self.resources.iter().filter(|r| !r.is_internal()).collect()
    }

    /// Resources sorted by install order; those without `order` come last in catalog order.
    pub fn install_order(&self) -> Vec<&Resource> {
        let mut v: Vec<(usize, &Resource)> = self.resources.iter().enumerate().collect();
        v.sort_by_key(|(i, r)| (r.order.unwrap_or(u32::MAX), *i));
        v.into_iter().map(|(_, r)| r).collect()
    }

    /// Public resources in install order.
    pub fn public_install_order(&self) -> Vec<&Resource> {
        self.install_order().into_iter().filter(|r| !r.is_internal()).collect()
    }
}

/// Fetches the catalog JSON text. Implemented over HTTP elsewhere; faked in tests.
pub trait CatalogSource {
    fn fetch(&self, url: &str) -> Result<String, String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogOrigin {
    Remote,
    /// Fell back to the embedded snapshot; carries the reason.
    Embedded(String),
}

/// Load from `url`; on any fetch/parse/validation failure use the embedded snapshot.
pub fn load(source: &dyn CatalogSource, url: &str) -> Result<(Catalog, CatalogOrigin), CatalogError> {
    let remote = source
        .fetch(url)
        .map_err(CatalogError::Fetch)
        .and_then(|t| Catalog::parse_validated(&t));
    match remote {
        Ok(c) => Ok((c, CatalogOrigin::Remote)),
        Err(e) => Ok((Catalog::embedded()?, CatalogOrigin::Embedded(e.to_string()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(Result<String, String>);
    impl CatalogSource for Fake {
        fn fetch(&self, _: &str) -> Result<String, String> {
            self.0.clone()
        }
    }

    #[test]
    fn embedded_is_valid() {
        let c = Catalog::embedded().unwrap();
        assert!(c.resources.len() > 10);
        assert!(c.find("kora-lib").is_some());
    }

    #[test]
    fn remote_then_fallback() {
        let (c, o) = load(&Fake(Ok(EMBEDDED_SNAPSHOT.into())), CATALOG_URL).unwrap();
        assert_eq!(o, CatalogOrigin::Remote);
        assert!(!c.resources.is_empty());
        for bad in [Err("offline".to_string()), Ok("{nope".to_string())] {
            let (_, o) = load(&Fake(bad), CATALOG_URL).unwrap();
            assert!(matches!(o, CatalogOrigin::Embedded(_)));
        }
    }

    #[test]
    fn validation_catches_problems() {
        let mut c = Catalog::embedded().unwrap();
        let dup = c.resources[0].clone();
        c.resources.push(dup);
        c.resources[1].repository = "http://example.com/x".into();
        let CatalogError::Invalid(e) = c.validate().unwrap_err() else { panic!() };
        assert!(e.iter().any(|m| m.contains("duplicate id")));
        assert!(e.iter().any(|m| m.contains("duplicate path")));
        assert!(e.iter().any(|m| m.contains("bad repository")));
    }

    #[test]
    fn order_optional_and_selection() {
        let mut c = Catalog::embedded().unwrap();
        let ord = c.install_order();
        let orders: Vec<_> = ord.iter().filter_map(|r| r.order).collect();
        assert!(orders.windows(2).all(|w| w[0] < w[1]));
        for r in &mut c.resources {
            r.order = None;
        }
        assert!(c.validate().is_ok());
        assert_eq!(c.install_order()[0].id, c.resources[0].id);
        assert!(c.public().iter().all(|r| r.kind != Kind::Internal));
        assert!(!c.by_group(Group::Kora).is_empty());
    }

    #[test]
    fn version_resolution() {
        let c = Catalog::embedded().unwrap();
        let lib = c.find("kora-lib").unwrap();
        assert_eq!(
            lib.resolve(Mode::Prod),
            ResolvedVersion::Pinned {
                version: "0.2.1".into(),
                tag: "v0.2.1".into(),
                commit: "6e5dd02ce6dab6cf7380336d7e6a91b1ed2fd126".into()
            }
        );
        assert_eq!(lib.resolve(Mode::Dev), ResolvedVersion::BranchHead { branch: "dev".into() });
        let dep = c.resources.iter().find(|r| r.versions.is_empty()).unwrap();
        assert!(matches!(dep.resolve(Mode::Prod), ResolvedVersion::BranchHead { .. }));
    }
}
