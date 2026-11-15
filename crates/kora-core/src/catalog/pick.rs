//! Pure selection rules for optional catalog sections (compat, bridges, creators): what is
//! included, what is blocked from being un-selected, and what finally gets installed.
//!
//! Rules: nothing is selected by default; whatever a selected resource `dependsOn` is included
//! transitively (across types); a resource that another selected one needs cannot be deselected.

use std::collections::{BTreeSet, HashSet};

use super::{normalize_id, Catalog, Creator, ResType, Resource};

/// A browsable section of the catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Compat,
    Bridges,
    Creators,
}

impl Section {
    pub const ALL: [Section; 3] = [Section::Compat, Section::Bridges, Section::Creators];

    pub fn title(self) -> &'static str {
        match self {
            Section::Compat => "Compatibility",
            Section::Bridges => "Bridges",
            Section::Creators => "Creators",
        }
    }

    fn res_type(self) -> ResType {
        match self {
            Section::Compat => ResType::Compat,
            Section::Bridges => ResType::Bridge,
            Section::Creators => ResType::Creator,
        }
    }
}

impl Catalog {
    /// Public resources of a section, in install order.
    pub fn section(&self, s: Section) -> Vec<&Resource> {
        self.public_install_order()
            .into_iter()
            .filter(|r| r.res_type == Some(s.res_type()))
            .collect()
    }

    /// Creators that have at least one public resource, in catalog order.
    pub fn creators_with_resources(&self) -> Vec<&Creator> {
        self.creators
            .iter()
            .filter(|c| !self.creator_resources(&c.id).is_empty())
            .collect()
    }

    pub fn creator_resources(&self, creator: &str) -> Vec<&Resource> {
        self.public_install_order()
            .into_iter()
            .filter(|r| {
                r.res_type == Some(ResType::Creator) && r.creator.as_deref() == Some(creator)
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PickError {
    #[error("'{0}' is not in the catalog")]
    Unknown(String),
    #[error("'{0}' is not available")]
    Internal(String),
    #[error("'{0}' is already installed")]
    Installed(String),
    #[error("'{id}' is needed by {}", .by.join(", "))]
    Needed { id: String, by: Vec<String> },
}

/// Selection state over a catalog. `installed` ids count as satisfied dependencies and cannot be picked.
#[derive(Debug, Clone)]
pub struct Picker<'a> {
    cat: &'a Catalog,
    picked: BTreeSet<String>,
    installed: HashSet<String>,
}

impl<'a> Picker<'a> {
    pub fn new(cat: &'a Catalog, installed: impl IntoIterator<Item = String>) -> Self {
        Self {
            cat,
            picked: BTreeSet::new(),
            installed: installed.into_iter().map(|i| normalize_id(&i)).collect(),
        }
    }

    /// Start from explicit ids (e.g. the list a website code returned). Unknown or internal ids are an error.
    pub fn with_ids(
        cat: &'a Catalog,
        installed: impl IntoIterator<Item = String>,
        ids: &[String],
    ) -> Result<Self, PickError> {
        let mut p = Self::new(cat, installed);
        for id in ids {
            p.check_pickable(id)?;
            if !p.is_installed(id) {
                p.picked.insert(normalize_id(id));
            }
        }
        Ok(p)
    }

    fn check_pickable(&self, id: &str) -> Result<&'a Resource, PickError> {
        let r = self
            .cat
            .find_normalized(id)
            .ok_or_else(|| PickError::Unknown(id.to_string()))?;
        if r.is_internal() {
            return Err(PickError::Internal(id.to_string()));
        }
        Ok(r)
    }

    pub fn is_installed(&self, id: &str) -> bool {
        self.installed.contains(&normalize_id(id))
    }

    pub fn is_picked(&self, id: &str) -> bool {
        self.picked.contains(&normalize_id(id))
    }

    /// Ids (as in the catalog) of explicitly picked resources that need `id`, directly or not.
    pub fn needed_by(&self, id: &str) -> Vec<String> {
        let n = normalize_id(id);
        self.picked
            .iter()
            .filter(|p| **p != n)
            .filter(|p| self.cat.with_dependencies([(*p).clone()]).contains(&n))
            .map(|p| {
                self.cat
                    .find_normalized(p)
                    .map_or_else(|| p.clone(), |r| r.id.clone())
            })
            .collect()
    }

    /// Picked, or pulled in by something picked, or already installed.
    pub fn is_checked(&self, id: &str) -> bool {
        self.is_installed(id) || self.is_picked(id) || !self.needed_by(id).is_empty()
    }

    /// Toggle one resource. Returns the new checked state. A resource that another selected one
    /// needs (or one that is already installed) cannot be unchecked.
    pub fn toggle(&mut self, id: &str) -> Result<bool, PickError> {
        self.check_pickable(id)?;
        if self.is_installed(id) {
            return Err(PickError::Installed(id.to_string()));
        }
        let n = normalize_id(id);
        let by = self.needed_by(id);
        if !by.is_empty() {
            return Err(PickError::Needed {
                id: id.to_string(),
                by,
            });
        }
        if self.picked.remove(&n) {
            Ok(false)
        } else {
            self.picked.insert(n);
            Ok(true)
        }
    }

    /// Select everything in `ids` (skips internal and installed ones).
    pub fn select_all<'i>(&mut self, ids: impl IntoIterator<Item = &'i str>) {
        for id in ids {
            if self.check_pickable(id).is_ok() && !self.is_installed(id) {
                self.picked.insert(normalize_id(id));
            }
        }
    }

    /// Deselect everything in `ids` that is not blocked; returns the blocked ones.
    pub fn deselect_all<'i>(&mut self, ids: impl IntoIterator<Item = &'i str>) -> Vec<PickError> {
        let ids: Vec<&str> = ids.into_iter().collect();
        // Remove all first so two items in `ids` that need each other do not block themselves.
        let before = self.picked.clone();
        for id in &ids {
            self.picked.remove(&normalize_id(id));
        }
        let mut blocked = Vec::new();
        for id in &ids {
            let n = normalize_id(id);
            if !before.contains(&n) {
                continue;
            }
            let by = self.needed_by(id);
            if !by.is_empty() {
                self.picked.insert(n);
                blocked.push(PickError::Needed {
                    id: (*id).to_string(),
                    by,
                });
            }
        }
        blocked
    }

    /// Are all of `ids` checked (for the select-all toggle)?
    pub fn all_checked<'i>(&self, ids: impl IntoIterator<Item = &'i str>) -> bool {
        let mut any = false;
        for id in ids {
            any = true;
            if !self.is_checked(id) {
                return false;
            }
        }
        any
    }

    /// Everything to install: the picks plus their dependencies, minus what is installed, in install order.
    pub fn resolved(&self) -> Vec<&'a Resource> {
        let wanted = self.cat.with_dependencies(self.picked.iter().cloned());
        self.cat
            .install_order()
            .into_iter()
            .filter(|r| {
                let n = normalize_id(&r.id);
                wanted.contains(&n) && !self.installed.contains(&n)
            })
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.picked.is_empty()
    }

    /// The explicitly picked ids (as written in the catalog), in catalog order.
    pub fn picked_ids(&self) -> Vec<String> {
        self.cat
            .resources
            .iter()
            .filter(|r| self.picked.contains(&normalize_id(&r.id)))
            .map(|r| r.id.clone())
            .collect()
    }

    pub fn picked_count(&self) -> usize {
        self.picked.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cat() -> Catalog {
        let rel = r#""versions":[{"version":"1.0.0","tag":"v1.0.0","commit":"0123456789012345678901234567890123456789"}],"latest":"1.0.0""#;
        let r = |id: &str, extra: &str| {
            let kind = if id == "hidden" {
                "internal"
            } else {
                "optional"
            };
            format!(
                r#"{{"id":"{id}","repository":"https://github.com/o/{id}.git","path":"{id}","group":"other","kind":"{kind}","branch":"main",{rel}{extra}}}"#
            )
        };
        let json = format!(
            r#"{{"schemaVersion":3,"creators":[{{"id":"acme","name":"Acme"}},{{"id":"empty","name":"Empty"}}],"resources":[{},{},{},{},{}]}}"#,
            r("compat-a", r#","type":"compat""#),
            r("bridge-x", r#","type":"bridge","dependsOn":["compat-a"]"#),
            r(
                "acme-one",
                r#","type":"creator","creator":"acme","dependsOn":["bridge-x"]"#
            ),
            r("acme-two", r#","type":"creator","creator":"acme""#),
            r("hidden", r#","type":"creator","creator":"acme""#),
        );
        Catalog::parse_validated(&json).unwrap()
    }

    fn ids(v: Vec<&Resource>) -> Vec<String> {
        v.into_iter().map(|r| r.id.clone()).collect()
    }

    #[test]
    fn sections_and_creators() {
        let c = cat();
        assert_eq!(ids(c.section(Section::Compat)), ["compat-a"]);
        assert_eq!(ids(c.section(Section::Bridges)), ["bridge-x"]);
        assert_eq!(
            ids(c.creator_resources("acme")),
            ["acme-one", "acme-two"],
            "internal is hidden"
        );
        let cs: Vec<&str> = c
            .creators_with_resources()
            .iter()
            .map(|c| c.id.as_str())
            .collect();
        assert_eq!(
            cs,
            ["acme"],
            "a creator without public resources is not listed"
        );
    }

    #[test]
    fn nothing_is_selected_by_default() {
        let c = cat();
        let p = Picker::new(&c, []);
        assert!(p.is_empty() && p.resolved().is_empty());
    }

    #[test]
    fn dependencies_are_included_transitively() {
        let c = cat();
        let mut p = Picker::new(&c, []);
        assert_eq!(p.toggle("acme-one"), Ok(true));
        assert_eq!(ids(p.resolved()), ["compat-a", "bridge-x", "acme-one"]);
        assert!(p.is_checked("compat-a") && !p.is_picked("compat-a"));
        assert!(!p.is_checked("acme-two"));
    }

    #[test]
    fn needed_dependency_cannot_be_deselected() {
        let c = cat();
        let mut p = Picker::new(&c, []);
        p.toggle("acme-one").unwrap();
        let e = p.toggle("bridge-x").unwrap_err();
        assert_eq!(
            e,
            PickError::Needed {
                id: "bridge-x".into(),
                by: vec!["acme-one".into()]
            }
        );
        assert!(p.toggle("compat-a").is_err());
        // picked explicitly too, still blocked while needed
        p.select_all(["bridge-x"]);
        assert!(p.toggle("bridge-x").is_err());
        // drop the parent: the dependencies are free again
        assert_eq!(p.toggle("acme-one"), Ok(false));
        assert_eq!(p.toggle("bridge-x"), Ok(false));
        assert!(p.resolved().is_empty());
    }

    #[test]
    fn select_all_and_deselect_all_respect_blocks() {
        let c = cat();
        let mut p = Picker::new(&c, []);
        p.select_all(["acme-one", "acme-two"]);
        assert!(p.all_checked(["acme-one", "acme-two"]));
        assert_eq!(p.resolved().len(), 4);
        // bridge-x is needed by acme-one which is part of the same deselect: allowed together
        let blocked = p.deselect_all(["acme-one", "acme-two", "bridge-x"]);
        assert!(blocked.is_empty());
        assert!(p.resolved().is_empty());
        p.select_all(["acme-one", "bridge-x"]);
        let blocked = p.deselect_all(["bridge-x"]);
        assert_eq!(blocked.len(), 1);
        assert!(p.is_picked("bridge-x"));
    }

    #[test]
    fn installed_ones_are_satisfied_and_skipped() {
        let c = cat();
        let mut p = Picker::new(&c, ["compat_a".to_string()]);
        assert!(p.is_checked("compat-a"));
        assert_eq!(
            p.toggle("compat-a"),
            Err(PickError::Installed("compat-a".into()))
        );
        p.toggle("bridge-x").unwrap();
        assert_eq!(ids(p.resolved()), ["bridge-x"]);
    }

    #[test]
    fn unknown_and_internal_are_rejected() {
        let c = cat();
        let mut p = Picker::new(&c, []);
        assert!(matches!(p.toggle("nope"), Err(PickError::Unknown(_))));
        assert!(matches!(p.toggle("hidden"), Err(PickError::Internal(_))));
        assert!(Picker::with_ids(&c, [], &["acme-two".into(), "nope".into()]).is_err());
        let p = Picker::with_ids(&c, [], &["acme-two".into()]).unwrap();
        assert_eq!(ids(p.resolved()), ["acme-two"]);
    }
}
