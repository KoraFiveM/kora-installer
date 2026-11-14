//! Windows without `winget` (Windows Server, 10 LTSC, a fresh 10): install Git for Windows and the
//! GitHub CLI from the vendors' own GitHub releases instead. The release list comes over HTTPS from
//! api.github.com; the installer is only run when its SHA-256 matches the digest GitHub publishes for
//! that asset (or, for the GitHub CLI, the `checksums.txt` of the same release). No checksum, no run.

use std::path::Path;

use serde::Deserialize;

use super::{err, InstallError, Result};
use crate::artifacts::ArtifactSource;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Git,
    Gh,
}

impl Tool {
    pub fn repo(self) -> &'static str {
        match self {
            Tool::Git => "git-for-windows/git",
            Tool::Gh => "cli/cli",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Tool::Git => "Git for Windows",
            Tool::Gh => "the GitHub CLI",
        }
    }
    pub fn manual_url(self) -> &'static str {
        match self {
            Tool::Git => "https://git-scm.com/download/win",
            Tool::Gh => "https://cli.github.com",
        }
    }
    fn wanted(self, name: &str) -> bool {
        match self {
            Tool::Git => name.starts_with("Git-") && name.ends_with("-64-bit.exe"),
            Tool::Gh => name.starts_with("gh_") && name.ends_with("_windows_amd64.msi"),
        }
    }
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    digest: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Release {
    #[serde(default)]
    assets: Vec<Asset>,
}

/// What to download and how to check it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Download {
    pub name: String,
    pub url: String,
    /// Lowercase hex SHA-256, when the release publishes one inline.
    pub sha256: Option<String>,
    /// URL of a `checksums.txt` of the same release, used when `sha256` is `None`.
    pub checksums_url: Option<String>,
}

fn is_github_download(url: &str) -> bool {
    url.starts_with("https://github.com/") && !url.contains("..")
}

/// Pick the installer asset from a `releases/latest` JSON document.
pub fn pick_download(tool: Tool, release_json: &str) -> Result<Download> {
    let rel: Release = serde_json::from_str(release_json).map_err(|e| InstallError(format!("cannot read the {} release list: {e}", tool.label())))?;
    let a = rel
        .assets
        .iter()
        .find(|a| tool.wanted(&a.name))
        .ok_or_else(|| InstallError(format!("the latest {} release has no 64-bit Windows installer", tool.label())))?;
    if !is_github_download(&a.browser_download_url) {
        return err(format!("refusing to download {} from {}: not a github.com release asset", a.name, a.browser_download_url));
    }
    let sha256 = a.digest.as_deref().and_then(|d| d.strip_prefix("sha256:")).map(|h| h.to_ascii_lowercase()).filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()));
    let checksums_url = rel
        .assets
        .iter()
        .find(|c| c.name.ends_with("checksums.txt") || c.name.ends_with("sha256sums.txt"))
        .map(|c| c.browser_download_url.clone())
        .filter(|u| is_github_download(u));
    Ok(Download { name: a.name.clone(), url: a.browser_download_url.clone(), sha256, checksums_url })
}

/// `sha  name` / `sha *name` lines.
pub fn checksum_from_list(list: &str, name: &str) -> Option<String> {
    list.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        let (h, n) = (it.next()?, it.next()?.trim_start_matches('*'));
        (n == name && h.len() == 64).then(|| h.to_ascii_lowercase())
    })
}

/// The command line that installs the downloaded file silently.
pub fn install_command(tool: Tool, file: &Path) -> (String, Vec<String>) {
    let f = file.to_string_lossy().into_owned();
    match tool {
        Tool::Git => (f, vec!["/VERYSILENT".into(), "/NORESTART".into(), "/SUPPRESSMSGBOXES".into()]),
        Tool::Gh => ("msiexec.exe".into(), vec!["/i".into(), f, "/qn".into(), "/norestart".into()]),
    }
}

/// Download, verify and run the installer for `tool`. Needs administrator rights (the caller checks).
pub fn install(tool: Tool, http: &dyn ArtifactSource, tmp: &Path, say: &mut dyn FnMut(&str)) -> Result<()> {
    say(&format!("Looking up the latest {} release on GitHub", tool.label()));
    let json = http.fetch_text(&format!("https://api.github.com/repos/{}/releases/latest", tool.repo())).map_err(|e| InstallError(format!("cannot look up {}: {e}", tool.label())))?;
    let d = pick_download(tool, &json)?;
    let expected = match (&d.sha256, &d.checksums_url) {
        (Some(h), _) => h.clone(),
        (None, Some(u)) => {
            let list = http.fetch_text(u).map_err(|e| InstallError(format!("cannot fetch the checksum list for {}: {e}", d.name)))?;
            checksum_from_list(&list, &d.name).ok_or_else(|| InstallError(format!("the release's checksum list has no entry for {}", d.name)))?
        }
        (None, None) => return err(format!("no SHA-256 is published for {}: refusing to run an unverified installer", d.name)),
    };
    std::fs::create_dir_all(tmp).map_err(|e| InstallError(format!("cannot create {}: {e}", tmp.display())))?;
    let file = tmp.join(&d.name);
    say(&format!("Downloading {}", d.name));
    let mut out = std::fs::File::create(&file).map_err(|e| InstallError(format!("cannot write {}: {e}", file.display())))?;
    http.download(&d.url, &mut out).map_err(|e| InstallError(format!("could not download {}: {e}", d.url)))?;
    drop(out);
    let actual = super::super::service::binary::file_sha256(&file).map_err(|e| InstallError(format!("cannot read {}: {e}", file.display())))?;
    if actual != expected {
        let _ = std::fs::remove_file(&file);
        return err(format!("{} does not match its published SHA-256: it was deleted and NOT run.", d.name));
    }
    say(&format!("Installing {} (Windows may ask you to confirm)", tool.label()));
    let (prog, args) = install_command(tool, &file);
    let status = std::process::Command::new(&prog).args(&args).status().map_err(|e| InstallError(format!("could not run the {} installer: {e}", tool.label())))?;
    let _ = std::fs::remove_file(&file);
    // 0 = ok, 3010 = ok but a restart is pending (msiexec).
    if matches!(status.code(), Some(0) | Some(3010)) {
        Ok(())
    } else {
        err(format!("the {} installer failed (exit {}). Install it yourself from {} and run Kora again.", tool.label(), status.code().map_or("?".into(), |c| c.to_string()), tool.manual_url()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIT_JSON: &str = r#"{"assets":[
      {"name":"Git-2.50.0-32-bit.exe","browser_download_url":"https://github.com/git-for-windows/git/releases/download/v2.50.0.windows.1/Git-2.50.0-32-bit.exe","digest":"sha256:aaaa"},
      {"name":"Git-2.50.0-64-bit.exe","browser_download_url":"https://github.com/git-for-windows/git/releases/download/v2.50.0.windows.1/Git-2.50.0-64-bit.exe","digest":"sha256:ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789"}]}"#;
    const GH_JSON: &str = r#"{"assets":[
      {"name":"gh_2.70.0_checksums.txt","browser_download_url":"https://github.com/cli/cli/releases/download/v2.70.0/gh_2.70.0_checksums.txt"},
      {"name":"gh_2.70.0_windows_amd64.msi","browser_download_url":"https://github.com/cli/cli/releases/download/v2.70.0/gh_2.70.0_windows_amd64.msi"},
      {"name":"gh_2.70.0_windows_arm64.msi","browser_download_url":"https://github.com/cli/cli/releases/download/v2.70.0/gh_2.70.0_windows_arm64.msi"}]}"#;

    #[test]
    fn git_installer_is_the_64_bit_exe_with_its_published_digest() {
        let d = pick_download(Tool::Git, GIT_JSON).unwrap();
        assert_eq!(d.name, "Git-2.50.0-64-bit.exe");
        assert_eq!(d.sha256.as_deref(), Some("abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"));
    }

    #[test]
    fn gh_installer_is_the_amd64_msi_checked_against_the_releases_checksum_list() {
        let d = pick_download(Tool::Gh, GH_JSON).unwrap();
        assert_eq!(d.name, "gh_2.70.0_windows_amd64.msi");
        assert!(d.sha256.is_none() && d.checksums_url.as_deref().unwrap().ends_with("gh_2.70.0_checksums.txt"));
        let list = format!("{h}  gh_2.70.0_windows_amd64.msi\n{o}  gh_2.70.0_windows_arm64.msi\n", h = "a".repeat(64), o = "b".repeat(64));
        assert_eq!(checksum_from_list(&list, "gh_2.70.0_windows_amd64.msi"), Some("a".repeat(64)));
        assert_eq!(checksum_from_list(&list, "other.msi"), None);
    }

    #[test]
    fn downloads_must_come_from_github_and_a_missing_installer_is_an_error() {
        let evil = GIT_JSON.replace("https://github.com/git-for-windows", "https://evil.example/git-for-windows");
        assert!(pick_download(Tool::Git, &evil).unwrap_err().to_string().contains("not a github.com"));
        assert!(pick_download(Tool::Git, r#"{"assets":[]}"#).is_err());
        assert!(pick_download(Tool::Git, "nonsense").is_err());
    }

    #[test]
    fn silent_install_commands() {
        let (p, a) = install_command(Tool::Git, Path::new("C:\\t\\Git.exe"));
        assert_eq!(p, "C:\\t\\Git.exe");
        assert!(a.contains(&"/VERYSILENT".to_string()));
        let (p, a) = install_command(Tool::Gh, Path::new("C:\\t\\gh.msi"));
        assert_eq!(p, "msiexec.exe");
        assert_eq!(a[..2], ["/i".to_string(), "C:\\t\\gh.msi".to_string()]);
    }
}
