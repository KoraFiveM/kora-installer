//! Folder layout of a server's `resources/` and the matching `ensure [folder]` lines.
//!
//! New installs get `[cfx-default]`, `[compat]`, `[kora]`, `[bridges]`, `[creators]/[<creator>]`
//! (only the ones holding installed resources are ensured) plus the always-present empty
//! `[addons]` and `[streamedassets]/{cars,maps,props,peds}`.
//! `server.cfg` loads folders, never single resources. Existing servers are never rearranged:
//! only the folders a new resource needs are added (see [`append_ensures`]).

use std::path::Path;

/// `ensure` order of the folders of a new install.
pub const ENSURE_ORDER: [&str; 7] = ["[cfx-default]", "[compat]", "[kora]", "[bridges]", "[creators]", "[addons]", "[streamedassets]"];

/// Empty folders every new install gets (relative to `resources/`).
pub const EMPTY_DIRS: [&str; 5] = ["[addons]", "[streamedassets]/[cars]", "[streamedassets]/[maps]", "[streamedassets]/[props]", "[streamedassets]/[peds]"];

/// The `ensure` argument of a resource folder (`[kora]/kora_core` -> `[kora]`). A path without a
/// leading `[folder]` is a loose resource and is ensured by its own name.
pub fn top_folder(rel: &str) -> String {
    let first = rel.split('/').next().unwrap_or(rel);
    first.to_string()
}

/// Folders that are always created on disk (empty) and therefore always ensured.
const ALWAYS_ENSURED: [&str; 2] = ["[addons]", "[streamedassets]"];

/// The folders to ensure for a new install holding `rels`, in load order: the order of
/// [`ENSURE_ORDER`], keeping only folders that hold an installed resource plus the always-created
/// `[addons]` and `[streamedassets]`. Folders of older catalogs (`[pre]`, `[standalone]`, ...)
/// are placed just before `[kora]`, in first-seen order.
pub fn ensure_for_new<'a>(rels: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut legacy: Vec<String> = Vec::new();
    let mut loose: Vec<String> = Vec::new();
    let mut tops: Vec<String> = Vec::new();
    for rel in rels {
        let top = top_folder(rel);
        tops.push(top.clone());
        if !top.starts_with('[') {
            if !loose.contains(&top) {
                loose.push(top);
            }
        } else if !ENSURE_ORDER.contains(&top.as_str()) && !legacy.contains(&top) {
            legacy.push(top);
        }
    }
    let used = |f: &str| tops.iter().any(|t| t == f);
    let mut out: Vec<String> = Vec::new();
    for f in ENSURE_ORDER {
        if f == "[kora]" {
            out.extend(legacy.iter().cloned());
        }
        if ALWAYS_ENSURED.contains(&f) || used(f) {
            out.push(f.to_string());
        }
    }
    out.extend(loose);
    out
}

/// Create the empty folders of the new layout under `resources`.
pub fn create_empty_dirs(resources: &Path) -> std::io::Result<()> {
    for d in EMPTY_DIRS {
        std::fs::create_dir_all(resources.join(d))?;
    }
    Ok(())
}

/// Remove the empty folders of [`create_empty_dirs`] again (rollback). Folders holding anything
/// are left alone.
pub fn remove_empty_dirs(resources: &Path) {
    for d in EMPTY_DIRS.iter().rev() {
        let _ = std::fs::remove_dir(resources.join(d));
    }
    let _ = std::fs::remove_dir(resources.join("[streamedassets]"));
}

fn has_ensure(cfg: &str, name: &str) -> bool {
    cfg.lines().any(|l| {
        let mut w = l.split_whitespace();
        w.next() == Some("ensure")
            && w.next() == Some(name)
            && match w.next() {
                None => true,
                Some(x) => x.starts_with('#'),
            }
    })
}

/// The folders of `folders` that `cfg` does not ensure yet.
pub fn missing_ensures(cfg: &str, folders: &[String]) -> Vec<String> {
    folders.iter().filter(|f| !has_ensure(cfg, f)).cloned().collect()
}

/// `cfg` with an `ensure <folder>` line added for every folder not already ensured, in the order
/// given, or `None` when nothing is missing. Existing lines are never touched or reordered.
pub fn append_ensures(cfg: &str, folders: &[String]) -> Option<String> {
    let mut out = cfg.to_string();
    let mut changed = false;
    for f in folders {
        if has_ensure(&out, f) {
            continue;
        }
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&format!("ensure {f}\n"));
        changed = true;
    }
    changed.then_some(out)
}

/// The folders (in [`ENSURE_ORDER`] order, other ones after) that the given relative paths live in.
pub fn folders_for<'a>(rels: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let tops: Vec<String> = rels.into_iter().map(top_folder).collect();
    let mut out: Vec<String> = ENSURE_ORDER.iter().filter(|f| tops.iter().any(|t| t == *f)).map(|f| f.to_string()).collect();
    for t in tops {
        if !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_install_ensures_folders_in_order() {
        let v = ensure_for_new(["[cfx-default]/a", "[compat]/b", "[kora]/kora_core", "[bridges]/c", "[creators]/[x]/d"]);
        assert_eq!(v, ENSURE_ORDER.map(String::from));
    }

    #[test]
    fn only_kora_resources_skip_unused_folders() {
        let v = ensure_for_new(["[kora]/kora_core", "[kora]/kora_lib"]);
        assert_eq!(v, ["[kora]", "[addons]", "[streamedassets]"]);
    }

    #[test]
    fn compat_resource_ensures_compat() {
        let v = ensure_for_new(["[compat]/x", "[kora]/kora_core"]);
        assert_eq!(v, ["[compat]", "[kora]", "[addons]", "[streamedassets]"]);
    }

    #[test]
    fn older_catalog_folders_load_before_kora() {
        let v = ensure_for_new(["[pre]/kora_lib", "[default-maps]/x", "[standalone]/y", "[kora]/z", "[pre]/other"]);
        assert_eq!(&v[..4], ["[pre]", "[default-maps]", "[standalone]", "[kora]"]);
        assert_eq!(v.last().unwrap(), "[streamedassets]");
        assert!(ensure_for_new(["loose_res"]).contains(&"loose_res".to_string()));
    }

    #[test]
    fn creators_are_one_ensure_not_per_creator() {
        let v = ensure_for_new(["[creators]/[acme]/one", "[creators]/[bob]/two"]);
        assert_eq!(v.iter().filter(|f| f.starts_with("[creators]")).count(), 1);
    }

    #[test]
    fn empty_dirs_are_created() {
        let d = tempfile::tempdir().unwrap();
        create_empty_dirs(d.path()).unwrap();
        for p in EMPTY_DIRS {
            assert!(d.path().join(p).is_dir(), "{p}");
        }
        create_empty_dirs(d.path()).unwrap();
    }

    #[test]
    fn append_only_missing_and_never_reorders() {
        let cfg = "exec x\nensure [kora]\nensure [compat] # hi\n";
        let new = append_ensures(cfg, &["[compat]".into(), "[bridges]".into(), "[creators]".into()]).unwrap();
        assert_eq!(new, "exec x\nensure [kora]\nensure [compat] # hi\nensure [bridges]\nensure [creators]\n");
        assert!(append_ensures(&new, &["[bridges]".into()]).is_none());
        assert_eq!(append_ensures("a", &["[x]".into()]).unwrap(), "a\nensure [x]\n");
        assert!(append_ensures("# ensure [x]\n", &["[x]".into()]).is_some(), "a comment does not count");
    }

    #[test]
    fn folders_for_orders_new_style_first() {
        let v = folders_for(["[creators]/[a]/b", "[bridges]/x", "[pre]/p", "[bridges]/y"]);
        assert_eq!(v, ["[bridges]", "[creators]", "[pre]"]);
    }
}
