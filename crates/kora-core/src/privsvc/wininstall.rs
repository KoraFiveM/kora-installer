//! Windows: install Kora's administrator service for a per-user install (`kora service install`, the
//! panel's "Install Kora's administrator service"): download this version's `.msi` from the official
//! release, check it against the release checksum, and run it with the Windows permission prompt.
//! This is the ONE prompt that remains: someone at the computer approves the installer itself.
//!
//! Between "checked" and "run" the file cannot change: it is opened without write or delete sharing
//! BEFORE its checksum is computed (so the hash is of the very bytes `msiexec` will read) and kept
//! open until `msiexec` has finished. `msiexec` is started by its full path under the system folder
//! Windows reports, never through `PATH`.

use std::io::{Read, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::self_update::{self, net, release};

/// The release asset.
pub const MSI_ASSET: &str = "kora-windows-x86_64.msi";
/// msiexec exit codes that mean "installed": 0, and 3010 (a restart is advised).
pub fn msi_success(code: u32) -> bool {
    code == 0 || code == 3010
}

/// msiexec's arguments: quiet apart from a progress bar, the file as a quoted path, the account to
/// allow as a public property (the MSI also allows the account that runs the installer).
pub fn msi_params(msi: &str, allow_sid: Option<&str>) -> String {
    let mut p = format!("/i \"{msi}\" /passive /norestart");
    if let Some(s) = allow_sid.filter(|s| crate::privsvc::identity::valid_user_sid(s)) {
        p.push_str(&format!(" ALLOWUSERS=\"{s}\""));
    }
    p
}

fn say(on: &mut dyn FnMut(&str), s: &str) {
    on(s);
}

/// Run the whole install. `allow`: accounts (names or SIDs) that may use the service; the account
/// running this program is always added.
pub fn install(allow: &[String], on: &mut dyn FnMut(&str)) -> Result<(), String> {
    let me = super::winsvc::current_user_sid();
    let sid = me.clone().or_else(|| allow.iter().find_map(|a| crate::privsvc::identity::valid_user_sid(a).then(|| a.clone())));
    let src = self_update::Source::from_env();
    let rels = self_update::fetch_releases(&src).map_err(|e| e.to_string())?;
    let rel = release::pick_tag(&rels, env!("CARGO_PKG_VERSION"))
        .ok_or_else(|| format!("there is no published release {} to install the service from (this may be a development build)", env!("CARGO_PKG_VERSION")))?;
    let (bin, sum) = release::select_assets(rel, MSI_ASSET).map_err(|e| e.to_string())?;
    let expected = release::parse_checksum(&src.net.get_text(&sum.browser_download_url, 4096)?, MSI_ASSET)?;

    // A private folder of the person's own, the installer written exclusively.
    let dir: PathBuf = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).ok_or("LOCALAPPDATA is not set")?.join("Kora").join("tmp");
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let path = dir.join(format!("kora-{}-{}.msi", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0)));
    say(on, &format!("Downloading the Kora installer {}", rel.tag_name));
    {
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        src.net.download(&bin.browser_download_url, &mut f, net::MAX_BINARY)?;
        f.flush().and_then(|_| f.sync_all()).map_err(|e| e.to_string())?;
    }
    let result = (|| {
        // Lock FIRST (share read only), then hash what we hold.
        let mut locked = std::fs::OpenOptions::new().read(true).share_mode(1).open(&path).map_err(|e| format!("cannot open the installer: {e}"))?;
        let mut h = Sha256::new();
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = locked.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
        }
        let actual = hex::encode(h.finalize());
        if actual != expected {
            return Err(format!("the installer does not match its checksum (expected {expected}, got {actual}). Nothing was installed."));
        }
        say(on, "Checksum OK. Windows will ask for administrator permission once: choose Yes.");
        let system_root = crate::privsvc::winapi::system_dirs().system_root;
        let msiexec = format!(r"{}\System32\msiexec.exe", system_root.trim_end_matches('\\'));
        let mut allow_sid = sid.clone();
        // an explicitly named account other than the caller is allowed by the MSI through ALLOWUSERS too
        let extra: Vec<String> = allow.iter().filter_map(|a| super::manage::resolve_account(a).ok().map(|(id, _)| id.to_string())).filter(|s| Some(s) != me.as_ref()).collect();
        if allow_sid.is_none() {
            allow_sid = extra.first().cloned();
        }
        let code = super::winsvc::run_elevated_wait(&msiexec, &msi_params(&path.to_string_lossy(), allow_sid.as_deref()), Duration::from_secs(30 * 60))?;
        drop(locked);
        if !msi_success(code) {
            return Err(format!("the installer stopped with code {code}"));
        }
        // more than one account: the rest through the service's own command, now that it exists
        for a in &extra {
            if Some(a) != allow_sid.as_ref() {
                say(on, &format!("To allow {a} too, run `kora service allow {a}` as an administrator."));
            }
        }
        Ok(())
    })();
    let _ = std::fs::remove_file(&path);
    result?;
    say(on, "Kora's administrator service is installed. Kora Panel can now do the administrator parts of installs by itself.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_installer_runs_quietly_from_a_quoted_path_and_names_only_a_real_account() {
        assert_eq!(
            msi_params(r"C:\Users\Jo Smith\AppData\Local\Kora\tmp\k.msi", Some("S-1-5-21-1-2-3-1001")),
            r#"/i "C:\Users\Jo Smith\AppData\Local\Kora\tmp\k.msi" /passive /norestart ALLOWUSERS="S-1-5-21-1-2-3-1001""#
        );
        assert!(!msi_params("k.msi", Some("S-1-5-32-544")).contains("ALLOWUSERS"), "a group is never enrolled");
        assert!(!msi_params("k.msi", Some("x\" /quiet")).contains("ALLOWUSERS"));
        assert!(msi_success(0) && msi_success(3010) && !msi_success(1603) && !msi_success(1602));
    }
}
