//! Catalog client: schema v2/v3 types, loader (remote with embedded fallback),
//! validation, selection helpers and version resolution.
//!
//! Network access is behind [`CatalogSource`] so tests stay offline.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub mod pick;
pub use pick::{PickError, Picker, Section};

/// Public catalog location.
pub const CATALOG_URL: &str = "https://docs.koraframe.work/resources.json";

/// Snapshot of the catalog embedded in the binary (refreshed by CI).
pub const EMBEDDED_SNAPSHOT: &str = include_str!("../../assets/resources.json");

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("catalog parse error: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("catalog needs a newer installer: {0}")]
    Unrecognised(String),
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
    Bridges,
    Creators,
    /// Also the home of any group name this installer does not know yet, so a new catalog group
    /// can never make the whole catalog unreadable (groups only affect display order, not installs).
    #[serde(other)]
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

/// Schema v3 resource type. Absent in the catalog = a normal (Kora / dependency) resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResType {
    Compat,
    Bridge,
    Creator,
}

impl ResType {
    /// Top-level folder (under `resources/`) holding resources of this type.
    pub fn folder(self) -> &'static str {
        match self {
            ResType::Compat => "[compat]",
            ResType::Bridge => "[bridges]",
            ResType::Creator => "[creators]",
        }
    }
}

/// A third-party creator whose resources the catalog lists (schema v3 `creators`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Creator {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    /// May be omitted (schema v3): such a release is pinned by commit only. Empty = no tag.
    #[serde(default, skip_serializing_if = "String::is_empty")]
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
    /// Ids of resources this one needs at runtime (e.g. `kora-lib` needs `kora-theme`). Selection
    /// always includes them (transitively). `-` and `_` are interchangeable, like everywhere else.
    #[serde(default, rename = "dependsOn", skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    /// Schema v3: `compat` | `bridge` | `creator`; absent = default.
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub res_type: Option<ResType>,
    /// Schema v3: creator id for `type: creator`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator: Option<String>,
    /// Schema v3: folder inside the repository that holds the resource, when it is not the repo root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subpath: Option<String>,
    /// Offered only to development servers (never to production ones, never auto-selected).
    /// Absent = false. Installers that predate this field ignore it and would offer the resource
    /// to everyone, so the catalog should publish it only once the installer that understands it
    /// is out.
    #[serde(default, rename = "devOnly", skip_serializing_if = "is_false")]
    pub dev_only: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub schema_version: u32,
    pub resources: Vec<Resource>,
    /// Schema v3; empty for v2 catalogs.
    #[serde(default)]
    pub creators: Vec<Creator>,
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

    /// Bridge and creator resources are other people's repositories: always a pinned clone at the
    /// catalog release, never forked, in production and in development.
    pub fn is_third_party(&self) -> bool {
        matches!(self.res_type, Some(ResType::Bridge | ResType::Creator))
    }

    /// A copy that is never forked and never tracks a branch: bridge / creator resources (someone
    /// else's repository) and anything installed from a `subpath` of a repository (a plain copy of
    /// one subfolder, which cannot be a fork checkout). Always a clone/copy pinned to the catalog commit.
    pub fn is_pinned_copy(&self) -> bool {
        self.is_third_party() || self.subpath.is_some()
    }

    /// Folder of this resource relative to the server's `resources/` dir. A catalog path that
    /// already starts with a `[folder]` is used as is, except that typed resources are forced into
    /// their folder (`[bridges]`, `[creators]/[<creator>]`, `[compat]`).
    pub fn install_rel(&self) -> String {
        let Some(t) = self.res_type else { return self.path.clone() };
        let prefix = match (t, &self.creator) {
            // Creator folders are bracketed too, so FXServer scans inside them: `[creators]/[acme]`.
            (ResType::Creator, Some(c)) => format!("{}/[{c}]", t.folder()),
            _ => t.folder().to_string(),
        };
        if self.path == prefix || self.path.starts_with(&format!("{prefix}/")) {
            return self.path.clone();
        }
        // Drop a leading `[other]/` folder so only the resource's own folder name is kept.
        let rest = match self.path.split_once('/') {
            Some((first, rest)) if first.starts_with('[') => rest,
            _ => self.path.as_str(),
        };
        format!("{prefix}/{rest}")
    }

    pub fn resolve(&self, mode: Mode) -> ResolvedVersion {
        let mode = if self.is_pinned_copy() { Mode::Prod } else { mode };
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

    /// Whether a server of this mode may see and pick the resource. Internal resources never;
    /// development-only resources only on development servers.
    pub fn offered_to(&self, dev: bool) -> bool {
        !self.is_internal() && (dev || !self.dev_only)
    }
}

/// Canonical form used to compare resource ids.
pub fn normalize_id(s: &str) -> String {
    s.trim().to_ascii_lowercase().replace('_', "-")
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

fn is_safe_segment(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".." && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
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
        match serde_json::from_str(json) {
            Ok(c) => Ok(c),
            Err(e) => Err(Self::explain_unrecognised(json).unwrap_or(CatalogError::Parse(e))),
        }
    }

    /// `kind` and `type` decide what gets installed and where, so an unknown value is never guessed:
    /// the catalog is rejected, naming the resource and field so it is clear the installer is too old.
    fn explain_unrecognised(json: &str) -> Option<CatalogError> {
        let v: serde_json::Value = serde_json::from_str(json).ok()?;
        for r in v.get("resources")?.as_array()? {
            let id = r.get("id").and_then(|x| x.as_str()).unwrap_or("?");
            for (field, known) in [
                ("kind", &["kora", "dependency", "optional", "internal"][..]),
                ("type", &["compat", "bridge", "creator"][..]),
            ] {
                if let Some(val) = r.get(field).and_then(|x| x.as_str()) {
                    if !known.contains(&val) {
                        return Some(CatalogError::Unrecognised(format!(
                            "resource '{id}' has an unrecognised {field} '{val}' (known: {}); update the Kora installer",
                            known.join(", ")
                        )));
                    }
                }
            }
        }
        None
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
        if !matches!(self.schema_version, 2 | 3) {
            errs.push(format!("unsupported schemaVersion {}", self.schema_version));
        }
        let (mut ids, mut paths, mut orders) = (HashSet::new(), HashSet::<String>::new(), HashSet::new());
        let mut creator_ids = HashSet::new();
        for c in &self.creators {
            if !creator_ids.insert(c.id.as_str()) {
                errs.push(format!("duplicate creator id {}", c.id));
            }
            if !is_safe_segment(&c.id) || c.id.starts_with('[') {
                errs.push(format!("unsafe creator id {}", c.id));
            }
        }
        for r in &self.resources {
            match (r.res_type, &r.creator) {
                (Some(ResType::Creator), None) => errs.push(format!("{}: type creator needs a creator id", r.id)),
                (Some(ResType::Creator), Some(c)) if !creator_ids.contains(c.as_str()) => {
                    errs.push(format!("{}: unknown creator '{c}'", r.id))
                }
                (Some(ResType::Creator), Some(_)) => {}
                (_, Some(_)) => errs.push(format!("{}: creator is only valid with type creator", r.id)),
                _ => {}
            }
            if let Some(sp) = &r.subpath {
                if !is_safe_path(sp) {
                    errs.push(format!("{}: unsafe subpath {sp}", r.id));
                }
            }
            if r.is_pinned_copy() && r.versions.is_empty() {
                errs.push(format!("{}: a bridge, creator or subpath resource needs a release to pin", r.id));
            }
            if !ids.insert(r.id.as_str()) {
                errs.push(format!("duplicate id {}", r.id));
            }
            if !paths.insert(r.install_rel()) {
                errs.push(format!("duplicate path {}", r.install_rel()));
            }
            if let Some(o) = r.order {
                if o == 0 || !orders.insert(o) {
                    errs.push(format!("{}: invalid or duplicate order {o}", r.id));
                }
            }
            if !is_valid_repo_url(&r.repository) {
                errs.push(format!("{}: bad repository url {}", r.id, r.repository));
            }
            if !is_safe_path(&r.install_rel()) {
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
            // Development-only resources track their branch head, so they need no release.
            if matches!(r.kind, Kind::Kora | Kind::Optional) && r.versions.is_empty() && !r.dev_only {
                errs.push(format!("{}: kora/optional resource needs a release", r.id));
            }
            for d in &r.depends_on {
                match self.find_normalized(d) {
                    None => errs.push(format!("{}: dependsOn '{d}' is not in the catalog", r.id)),
                    Some(t) if t.id == r.id => errs.push(format!("{}: depends on itself", r.id)),
                    Some(t) if t.is_internal() && !r.is_internal() => {
                        errs.push(format!("{}: public resource depends on internal '{}'", r.id, t.id))
                    }
                    Some(_) => {}
                }
            }
        }
        if errs.is_empty() { Ok(()) } else { Err(CatalogError::Invalid(errs)) }
    }

    pub fn find(&self, id: &str) -> Option<&Resource> {
        self.resources.iter().find(|r| r.id == id)
    }

    /// Find by id where `-` and `_` and letter case do not matter (`kora_theme` == `kora-theme`).
    pub fn find_normalized(&self, id: &str) -> Option<&Resource> {
        let n = normalize_id(id);
        self.resources.iter().find(|r| normalize_id(&r.id) == n)
    }

    /// `seeds` plus everything they depend on, transitively (normalised ids). Unknown ids and
    /// cycles are tolerated here (validation reports the former); the result never repeats.
    pub fn with_dependencies(&self, seeds: impl IntoIterator<Item = String>) -> HashSet<String> {
        let mut out: HashSet<String> = HashSet::new();
        let mut queue: Vec<String> = seeds.into_iter().map(|s| normalize_id(&s)).collect();
        while let Some(id) = queue.pop() {
            if !out.insert(id.clone()) {
                continue;
            }
            if let Some(r) = self.find_normalized(&id) {
                queue.extend(r.depends_on.iter().map(|d| normalize_id(d)));
            }
        }
        out
    }

    pub fn by_kind(&self, kind: Kind) -> Vec<&Resource> {
        self.resources.iter().filter(|r| r.kind == kind).collect()
    }

    pub fn by_group(&self, group: Group) -> Vec<&Resource> {
        self.resources.iter().filter(|r| r.group == group).collect()
    }

    /// Resources a server of the given mode may see (`dev` = development server).
    pub fn offered(&self, dev: bool) -> Vec<&Resource> {
        self.resources.iter().filter(|r| r.offered_to(dev)).collect()
    }

    /// Resources sorted by install order; those without `order` come last in catalog order.
    pub fn install_order(&self) -> Vec<&Resource> {
        let mut v: Vec<(usize, &Resource)> = self.resources.iter().enumerate().collect();
        v.sort_by_key(|(i, r)| (r.order.unwrap_or(u32::MAX), *i));
        v.into_iter().map(|(_, r)| r).collect()
    }

    /// Resources a server of the given mode may see, in install order.
    pub fn offered_install_order(&self, dev: bool) -> Vec<&Resource> {
        self.install_order().into_iter().filter(|r| r.offered_to(dev)).collect()
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
    fn embedded_snapshot_is_current_v3() {
        let c = Catalog::embedded().unwrap();
        assert_eq!(c.schema_version, 3);
        assert!(c.resources.iter().any(|r| r.res_type == Some(ResType::Compat)));
        assert!(c.resources.iter().any(|r| r.res_type == Some(ResType::Bridge) && r.group == Group::Bridges));
        assert_eq!(c.section(Section::Bridges, false).len(), 4);
    }

    #[test]
    fn group_is_tolerant_but_kind_and_type_are_strict() {
        let base = |g: &str, k: &str, t: &str| {
            format!(
                r#"{{"schemaVersion":3,"resources":[{{"id":"x","repository":"https://github.com/o/x.git","path":"x","group":"{g}","kind":"{k}","branch":"main"{t}}}]}}"#
            )
        };
        for (g, want) in [("other", Group::Other), ("brand-new-group", Group::Other), ("bridges", Group::Bridges), ("creators", Group::Creators), ("kora", Group::Kora), ("cfx-default", Group::CfxDefault)] {
            assert_eq!(Catalog::parse(&base(g, "kora", "")).unwrap().resources[0].group, want, "{g}");
        }
        for (g, s) in [(Group::Bridges, "\"bridges\""), (Group::CfxDefault, "\"cfx-default\""), (Group::Other, "\"other\"")] {
            assert_eq!(serde_json::to_string(&g).unwrap(), s);
        }
        let e = Catalog::parse(&base("kora", "weird", "")).unwrap_err().to_string();
        assert!(e.contains("'x'") && e.contains("kind") && e.contains("'weird'") && e.contains("update"), "{e}");
        let e = Catalog::parse(&base("kora", "kora", r#","type":"gizmo""#)).unwrap_err().to_string();
        assert!(e.contains("'x'") && e.contains("type") && e.contains("'gizmo'"), "{e}");
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
        assert!(c.offered(false).iter().all(|r| r.kind != Kind::Internal));
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

    fn v3(extra_resources: &str, creators: &str) -> String {
        let rel = r#""latest":"1.0.0","versions":[{"version":"1.0.0","tag":"v1.0.0","commit":"0123456789012345678901234567890123456789"}]"#;
        format!(
            r#"{{"schemaVersion":3,"creators":[{creators}],"resources":[{{"id":"kora-a","repository":"https://github.com/KoraFiveM/a.git","path":"[kora]/a","group":"kora","kind":"kora","branch":"dev",{rel},"dependsOn":["bridge-b"]}}{extra_resources}]}}"#
        )
    }

    #[test]
    fn v3_types_creators_and_install_folders() {
        let rel = r#""latest":"1.0.0","versions":[{"version":"1.0.0","tag":"v1.0.0","commit":"0123456789012345678901234567890123456789"}]"#;
        let extra = format!(
            r#",{{"id":"bridge-b","repository":"https://github.com/o/b.git","path":"bridge_b","group":"other","kind":"optional","branch":"main","type":"bridge",{rel}}},
            {{"id":"c-one","repository":"https://github.com/o/c.git","path":"[creators]/[acme]/c_one","group":"other","kind":"optional","branch":"main","type":"creator","creator":"acme","subpath":"res",{rel}}},
            {{"id":"c-two","repository":"https://github.com/o/d.git","path":"d","group":"other","kind":"optional","branch":"main","type":"creator","creator":"acme",{rel}}},
            {{"id":"cm","repository":"https://github.com/o/e.git","path":"[standalone]/e","group":"compat","kind":"optional","branch":"main","type":"compat",{rel}}}"#
        );
        let c = Catalog::parse_validated(&v3(&extra, r#"{"id":"acme","name":"Acme","url":"https://acme.example","description":"x"}"#)).unwrap();
        assert_eq!(c.creators.len(), 1);
        assert_eq!(c.find("bridge-b").unwrap().install_rel(), "[bridges]/bridge_b");
        assert_eq!(c.find("c-one").unwrap().install_rel(), "[creators]/[acme]/c_one");
        assert_eq!(c.find("c-two").unwrap().install_rel(), "[creators]/[acme]/d");
        assert_eq!(c.find("cm").unwrap().install_rel(), "[compat]/e");
        assert_eq!(c.find("kora-a").unwrap().install_rel(), "[kora]/a");
        assert_eq!(c.find("c-one").unwrap().subpath.as_deref(), Some("res"));
        // third-party is pinned even in dev mode; kora resources track the branch
        assert!(matches!(c.find("bridge-b").unwrap().resolve(Mode::Dev), ResolvedVersion::Pinned { .. }));
        assert!(matches!(c.find("c-one").unwrap().resolve(Mode::Dev), ResolvedVersion::Pinned { .. }));
        assert!(matches!(c.find("cm").unwrap().resolve(Mode::Dev), ResolvedVersion::BranchHead { .. }));
        assert!(matches!(c.find("kora-a").unwrap().resolve(Mode::Dev), ResolvedVersion::BranchHead { .. }));
        // dependsOn works across types
        let deps = c.with_dependencies(["kora-a".to_string()]);
        assert!(deps.contains("bridge-b"));
    }

    #[test]
    fn v3_validation() {
        let rel = r#""latest":"1.0.0","versions":[{"version":"1.0.0","tag":"v1.0.0","commit":"0123456789012345678901234567890123456789"}]"#;
        let creator_res = |creator: &str| {
            format!(
                r#",{{"id":"bridge-b","repository":"https://github.com/o/b.git","path":"b","group":"other","kind":"optional","branch":"main","type":"creator"{creator},{rel}}}"#
            )
        };
        let bad = |json: String| match Catalog::parse_validated(&json) {
            Err(CatalogError::Invalid(e)) => e.join("; "),
            other => panic!("expected invalid, got {other:?}"),
        };
        assert!(bad(v3(&creator_res(""), "")).contains("needs a creator id"));
        assert!(bad(v3(&creator_res(r#","creator":"nobody""#), "")).contains("unknown creator"));
        let no_release = r#",{"id":"bridge-b","repository":"https://github.com/o/b.git","path":"b","group":"other","kind":"optional","branch":"main","type":"bridge","latest":null,"versions":[]}"#;
        assert!(bad(v3(no_release, "")).contains("needs a release"));
    }

    #[test]
    fn versions_may_omit_the_tag_and_compat_may_use_a_subpath() {
        let json = r#"{"schemaVersion":3,"creators":[],"resources":[
            {"id":"ox-lib","repository":"https://github.com/KoraFiveM/compat.git","path":"[compat]/ox_lib","group":"compat","kind":"optional","branch":"main","type":"compat","subpath":"ox_lib","latest":"1.0.0","versions":[{"version":"1.0.0","commit":"0123456789012345678901234567890123456789"}]}]}"#;
        let c = Catalog::parse_validated(json).unwrap();
        let r = c.find("ox-lib").unwrap();
        assert_eq!(r.latest_release().unwrap().tag, "");
        assert!(r.is_pinned_copy() && !r.is_third_party());
        assert!(matches!(r.resolve(Mode::Dev), ResolvedVersion::Pinned { ref commit, .. } if commit.starts_with("0123")));
        assert_eq!(r.install_rel(), "[compat]/ox_lib");
        // no release at all: a pinned copy cannot be installed
        let none = json.replace(r#","latest":"1.0.0","versions":[{"version":"1.0.0","commit":"0123456789012345678901234567890123456789"}]"#, "");
        assert!(Catalog::parse_validated(&none).is_err());
        // and a tag-less release round-trips without inventing a tag
        assert!(!serde_json::to_string(&c).unwrap().contains("\"tag\""));
    }

    #[test]
    fn v2_catalog_still_parses() {
        // The bundled snapshot is v3 now; derive a v2 catalog from it (no types, no creators).
        let mut v: serde_json::Value = serde_json::from_str(EMBEDDED_SNAPSHOT).unwrap();
        v["schemaVersion"] = 2.into();
        v.as_object_mut().unwrap().remove("creators");
        v["resources"].as_array_mut().unwrap().retain(|r| r.get("type").is_none());
        let c = Catalog::parse_validated(&v.to_string()).unwrap();
        assert_eq!(c.schema_version, 2);
        assert!(c.creators.is_empty());
        assert!(c.resources.iter().all(|r| r.res_type.is_none() && !r.is_third_party()));
        assert!(c.resources.iter().all(|r| r.install_rel() == r.path));
    }

    #[test]
    fn dev_only_resources_are_offered_to_development_servers_only() {
        let json = r#"{"schemaVersion":3,"resources":[
          {"id":"kora-x","repository":"https://github.com/o/x.git","path":"[kora]/x","group":"kora","kind":"optional","branch":"dev","devOnly":true},
          {"id":"kora-y","repository":"https://github.com/o/y.git","path":"[kora]/y","group":"kora","kind":"optional","branch":"dev","versions":[{"version":"1.0.0","tag":"v1.0.0","commit":"0123456789012345678901234567890123456789"}],"latest":"1.0.0"}
        ]}"#;
        // A development-only resource tracks its branch, so it may have no release.
        let c = Catalog::parse_validated(json).unwrap();
        let ids = |v: Vec<&Resource>| v.iter().map(|r| r.id.clone()).collect::<Vec<_>>();
        assert!(c.find("kora-x").unwrap().dev_only);
        assert_eq!(ids(c.offered(false)), ["kora-y"]);
        assert_eq!(ids(c.offered(true)), ["kora-x", "kora-y"]);
        assert_eq!(ids(c.offered_install_order(false)), ["kora-y"]);
        // The same resource without the flag is public and needs a release like any optional one.
        let public = json.replace(r#""devOnly":true"#, r#""devOnly":false"#);
        assert!(Catalog::parse_validated(&public).is_err());
    }
}
