//! Plain-language fixes for the console errors FXServer prints when a paid (escrowed) asset cannot
//! load: the licence key belongs to another Cfx.re account, the files were changed, or the asset
//! could not be fetched.

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HintKind {
    /// "You lack the required entitlement".
    Entitlement,
    /// "Failed to verify protected resource".
    VerifyFailed,
    /// "Resource could not be downloaded".
    DownloadFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntitlementHint {
    pub kind: HintKind,
    /// The asset, when the line names it.
    pub resource: Option<String>,
    /// The console line it was found in (cleaned).
    pub line: String,
    /// What to do about it.
    pub fix: String,
}

/// How many of the newest console lines are looked at.
pub const SCAN_LINES: usize = 600;

fn kind_of(lower: &str) -> Option<HintKind> {
    if lower.contains("lack the required entitlement") {
        Some(HintKind::Entitlement)
    } else if lower.contains("failed to verify protected resource") {
        Some(HintKind::VerifyFailed)
    } else if lower.contains("resource could not be downloaded") {
        Some(HintKind::DownloadFailed)
    } else {
        None
    }
}

fn token_at(line: &str, at: usize) -> Option<&str> {
    let rest = line.get(at..)?.trim_start_matches([' ', ':']);
    let end = rest
        .find(|c: char| c.is_whitespace() || matches!(c, '/' | ',' | ':' | ')' | '('))
        .unwrap_or(rest.len());
    let t = rest[..end]
        .trim_matches(|c: char| matches!(c, '\'' | '"' | '`' | '@' | '[' | ']'))
        .trim_end_matches('.');
    // "to use this resource" and the like are not names.
    let filler = matches!(
        t.to_ascii_lowercase().as_str(),
        "this" | "the" | "a" | "resource" | "could" | "not"
    );
    (!t.is_empty() && !filler && super::valid_asset_name(t)).then_some(t)
}

/// The asset a console line is about, if it names one.
fn resource_of(line: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    for m in [
        "protected resource ",
        "to use ",
        "in resource ",
        "downloaded",
        "@",
    ] {
        for (i, _) in lower.match_indices(m) {
            if let Some(t) = token_at(line, i + m.len()) {
                return Some(t.to_string());
            }
        }
    }
    // Lastly, a quoted name anywhere on the line.
    line.split(['\'', '"'])
        .skip(1)
        .step_by(2)
        .find(|t| super::valid_asset_name(t))
        .map(String::from)
}

/// The fix for `kind`, naming `resource` when known.
pub fn fix_for(kind: HintKind, resource: Option<&str>) -> String {
    let what = resource.map_or_else(|| "this asset".to_string(), String::from);
    match kind {
        HintKind::Entitlement => format!(
            "Your licence key belongs to a different Cfx.re account than the one that bought {what}. Open portal.cfx.re → Assets → Granted Assets and transfer it, or use a key from that account."
        ),
        HintKind::VerifyFailed => format!(
            "{what} could not be verified. Usually its folder was renamed or its hidden .fxap file is missing, or the licence key belongs to a different Cfx.re account than the one that bought it. Download it again from portal.cfx.re → Assets → Granted Assets and import the zip with Kora, without renaming anything."
        ),
        HintKind::DownloadFailed => format!(
            "{what} could not be fetched from Cfx.re. Check that the server can reach the internet and that its licence key belongs to the Cfx.re account that bought it, then restart the server."
        ),
    }
}

/// The hints for `lines` (oldest first): one per kind and asset, newest sighting kept.
pub fn entitlement_hints<S: AsRef<str>>(lines: &[S]) -> Vec<EntitlementHint> {
    let mut out: Vec<EntitlementHint> = Vec::new();
    for l in lines {
        let line = crate::api::logs::clean_line(l.as_ref());
        let lower = line.to_ascii_lowercase();
        let Some(kind) = kind_of(&lower) else {
            continue;
        };
        let resource = resource_of(&line);
        out.retain(|h| !(h.kind == kind && h.resource == resource));
        let fix = fix_for(kind, resource.as_deref());
        let line: String = line.trim().chars().take(300).collect();
        out.push(EntitlementHint {
            kind,
            resource,
            line,
            fix,
        });
    }
    out
}

/// Hints from the newest console output of the server at `dir` (empty when there is none).
pub fn hints_for_server(dir: &Path) -> Vec<EntitlementHint> {
    let Some(src) = crate::console::source(dir) else {
        return Vec::new();
    };
    crate::api::logs::tail(&src, SCAN_LINES)
        .map(|l| entitlement_hints(&l))
        .unwrap_or_default()
}
