//! Choosing the release and the asset: pure functions over the GitHub releases JSON.

use serde::Deserialize;

use super::version::Version;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

impl Release {
    pub fn version(&self) -> Option<Version> {
        Version::parse(&self.tag_name)
    }
}

/// The release asset for this OS, or `None` when this platform has no published build.
pub fn asset_name(windows: bool, linux: bool, x86_64: bool) -> Option<&'static str> {
    match (windows, linux, x86_64) {
        (true, _, true) => Some("kora-windows-x86_64.exe"),
        (_, true, true) => Some("kora-linux-x86_64"),
        _ => None,
    }
}

pub fn this_asset_name() -> Option<&'static str> {
    asset_name(cfg!(windows), cfg!(target_os = "linux"), cfg!(target_arch = "x86_64"))
}

/// Parse `/releases?per_page=N`.
pub fn parse_releases(body: &str) -> Result<Vec<Release>, String> {
    serde_json::from_str(body).map_err(|e| format!("unexpected answer from GitHub: {e}"))
}

/// A pre-release by GitHub's flag or by its tag (`v0.3.0-rc1`): either one is enough.
pub fn is_prerelease(r: &Release) -> bool {
    r.prerelease || r.version().is_some_and(|v| v.is_prerelease())
}

/// Newest published release for a program running `current`: the first non-draft with a
/// parseable tag (GitHub lists newest first). A stable build only moves to stable releases; a
/// pre-release build also sees pre-releases, so a tester gets the next candidate and then the
/// final. `--version` (`pick_tag`) can still pick any release.
pub fn pick_newest<'a>(releases: &'a [Release], current: &Version) -> Option<&'a Release> {
    let any = current.is_prerelease();
    releases.iter().find(|r| !r.draft && r.version().is_some() && (any || !is_prerelease(r)))
}

/// A specific tag (`--version`), with or without the leading `v`.
pub fn pick_tag<'a>(releases: &'a [Release], want: &str) -> Option<&'a Release> {
    let w = Version::parse(want)?;
    releases.iter().find(|r| !r.draft && r.version().as_ref() == Some(&w))
}

/// The binary and its `.sha256`; both must exist.
pub fn select_assets<'a>(rel: &'a Release, name: &str) -> Result<(&'a Asset, &'a Asset), String> {
    let bin = rel.assets.iter().find(|a| a.name == name).ok_or_else(|| format!("release {} has no {name} file", rel.tag_name))?;
    let sum_name = format!("{name}.sha256");
    let sum = rel
        .assets
        .iter()
        .find(|a| a.name == sum_name)
        .ok_or_else(|| format!("release {} has no {sum_name} checksum, so it cannot be verified", rel.tag_name))?;
    Ok((bin, sum))
}

/// Read the expected hash from `<hex>  <name>` / `<hex> *<name>` / bare `<hex>`.
pub fn parse_checksum(text: &str, name: &str) -> Result<String, String> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).ok_or("the checksum file is empty")?;
    let mut it = line.split_whitespace();
    let hash = it.next().unwrap_or("").to_ascii_lowercase();
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("the checksum file does not contain a SHA-256 hash".into());
    }
    if let Some(f) = it.next() {
        if f.trim_start_matches('*') != name {
            return Err(format!("the checksum is for '{f}', not '{name}'"));
        }
    }
    Ok(hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(n: &str) -> serde_json::Value {
        serde_json::json!({"name": n, "browser_download_url": format!("https://github.com/x/{n}"), "size": 5})
    }
    fn json() -> String {
        serde_json::json!([
            {"tag_name": "v0.3.0", "draft": true, "assets": []},
            {"tag_name": "v0.2.2-rc1", "prerelease": true, "assets": [asset("kora-linux-x86_64"), asset("kora-linux-x86_64.sha256")]},
            {"tag_name": "v0.2.1", "assets": [asset("kora-windows-x86_64.exe")]},
        ])
        .to_string()
    }

    #[test]
    fn stable_builds_get_stable_releases_and_drafts_are_skipped() {
        let r = parse_releases(&json()).unwrap();
        let v = |s: &str| Version::parse(s).unwrap();
        assert_eq!(pick_newest(&r, &v("0.2.0")).unwrap().tag_name, "v0.2.1", "a stable build skips pre-releases");
        assert_eq!(pick_newest(&r, &v("0.2.0-rc1")).unwrap().tag_name, "v0.2.2-rc1", "a pre-release build sees them");
        assert_eq!(pick_tag(&r, "0.2.1").unwrap().tag_name, "v0.2.1");
        assert_eq!(pick_tag(&r, "v0.2.2-rc1").unwrap().tag_name, "v0.2.2-rc1", "an explicit version may be a pre-release");
        assert!(pick_tag(&r, "v0.3.0").is_none(), "drafts are never offered");
        assert!(pick_newest(&[], &v("0.2.0")).is_none());
        // A `-rc` tag counts as a pre-release even when GitHub's flag was left off.
        let unflagged = parse_releases(r#"[{"tag_name":"v0.4.0-rc1"},{"tag_name":"v0.3.9"}]"#).unwrap();
        assert_eq!(pick_newest(&unflagged, &v("0.3.0")).unwrap().tag_name, "v0.3.9");
    }

    #[test]
    fn asset_selection() {
        let r = parse_releases(&json()).unwrap();
        let (b, s) = select_assets(&r[1], "kora-linux-x86_64").unwrap();
        assert_eq!((b.name.as_str(), s.name.as_str()), ("kora-linux-x86_64", "kora-linux-x86_64.sha256"));
        assert!(select_assets(&r[1], "kora-windows-x86_64.exe").unwrap_err().contains("no kora-windows"));
        // binary present but checksum missing: refuse
        assert!(select_assets(&r[2], "kora-windows-x86_64.exe").unwrap_err().contains("checksum"));
        assert_eq!(asset_name(true, false, true), Some("kora-windows-x86_64.exe"));
        assert_eq!(asset_name(false, true, true), Some("kora-linux-x86_64"));
        assert_eq!(asset_name(false, true, false), None);
    }

    #[test]
    fn checksum_formats() {
        let h = "AB".repeat(32);
        let l = h.to_lowercase();
        assert_eq!(parse_checksum(&format!("{h}  kora\n"), "kora").unwrap(), l);
        assert_eq!(parse_checksum(&format!("{h} *kora"), "kora").unwrap(), l);
        assert_eq!(parse_checksum(&h, "kora").unwrap(), l);
        assert!(parse_checksum(&format!("{h}  other"), "kora").is_err());
        assert!(parse_checksum("nothex  kora", "kora").is_err());
        assert!(parse_checksum("", "kora").is_err());
    }
}
