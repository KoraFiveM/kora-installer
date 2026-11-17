//! Changing who may use the service (`kora service allow|deny|list`, and the installers' enrolment of
//! the installing administrator). Administrators only: the settings file is in a protected folder,
//! and the commands relaunch themselves with administrator rights like every other privileged `kora`
//! command.

use std::io;
use std::path::Path;

use super::policy::{AccountId, Policy};
use crate::privsvc::wire::Rejected;

/// The settings as they are (a missing file = nobody yet); a damaged file is an error, never silently replaced.
pub fn load_or_new(path: &Path) -> Result<Policy, String> {
    match std::fs::read_to_string(path) {
        Ok(t) => Policy::parse(&t),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Policy::new(env!("CARGO_PKG_VERSION"))),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

/// Write the settings through a temporary file and a rename; readable by everyone (the panel shows
/// who is allowed), writable by the administrator who wrote it only.
pub fn save(path: &Path, p: &Policy) -> io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("json.tmp");
    let _ = std::fs::remove_file(&tmp);
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o644).open(&tmp)?;
        f.write_all(p.to_text().as_bytes())?;
        f.sync_all()?;
    }
    #[cfg(not(unix))]
    std::fs::write(&tmp, p.to_text())?;
    std::fs::rename(&tmp, path)
}

/// Who `spec` means: an account name (looked up in the system), `uid:N`, or a SID. Root is refused
/// (it can always use the service).
pub fn resolve_account(spec: &str) -> Result<(AccountId, String), Rejected> {
    let spec = spec.trim();
    if let Ok(id) = AccountId::parse(spec) {
        return Ok((id, String::new()));
    }
    #[cfg(unix)]
    if let Some((uid, _)) = crate::userhome::account_ids(spec) {
        if uid == 0 {
            return Err(Rejected("root can always use Kora's service; there is nothing to allow.".into()));
        }
        return Ok((AccountId::uid(uid), spec.to_string()));
    }
    #[cfg(windows)]
    if let Some(sid) = super::winsvc::lookup_account_sid(spec) {
        let id = AccountId::sid(&sid)?;
        return Ok((id, spec.to_string()));
    }
    Err(Rejected(format!("'{spec}' is not an account on this computer. Use the account's name (or uid:1000 / the Windows SID).")))
}

/// `allow`: add the account; returns the text to print.
pub fn allow(path: &Path, spec: &str) -> Result<String, String> {
    let (id, name) = resolve_account(spec).map_err(|e| e.0)?;
    let mut p = load_or_new(path)?;
    let was = p.contains(&id);
    p.allow(id.clone(), &name).map_err(|e| e.0)?;
    save(path, &p).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(if was { format!("{} was already allowed.", label(&id, &name)) } else { format!("{} may now use Kora's administrator service.", label(&id, &name)) })
}

/// `deny`: remove the account.
pub fn deny(path: &Path, spec: &str) -> Result<String, String> {
    let (id, name) = resolve_account(spec).map_err(|e| e.0)?;
    let mut p = load_or_new(path)?;
    if !p.deny(&id) {
        return Ok(format!("{} was not on the list.", label(&id, &name)));
    }
    save(path, &p).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(format!("{} can no longer use Kora's administrator service. Servers it already set up keep working.", label(&id, &name)))
}

/// `list`: one line per account.
pub fn list(path: &Path) -> Result<Vec<String>, String> {
    Ok(load_or_new(path)?.accounts.iter().map(|a| if a.name.is_empty() { a.id.to_string() } else { format!("{} ({})", a.name, a.id) }).collect())
}

fn label(id: &AccountId, name: &str) -> String {
    if name.is_empty() {
        id.to_string()
    } else {
        format!("{name} ({id})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_list_deny_round_trip() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("etc/kora/service.json");
        assert!(list(&f).unwrap().is_empty(), "no file yet = nobody");
        assert!(allow(&f, "uid:1000").unwrap().contains("may now use"));
        assert!(allow(&f, "uid:1000").unwrap().contains("already"));
        assert!(allow(&f, "S-1-5-21-1-2-3-1001").is_ok());
        assert_eq!(list(&f).unwrap().len(), 2);
        assert!(deny(&f, "uid:1000").unwrap().contains("can no longer"));
        assert!(deny(&f, "uid:1000").unwrap().contains("not on the list"));
        assert_eq!(list(&f).unwrap(), ["S-1-5-21-1-2-3-1001"]);
        assert!(super::super::policy::Policy::parse(&std::fs::read_to_string(&f).unwrap()).is_ok());
    }

    #[test]
    fn bad_accounts_and_damaged_files_are_refused_not_replaced() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("service.json");
        for bad in ["", "uid:0", "S-1-5-18", "no-such-account-xyz"] {
            assert!(allow(&f, bad).is_err(), "{bad:?}");
        }
        assert!(!f.exists());
        std::fs::write(&f, "{ broken").unwrap();
        assert!(allow(&f, "uid:1000").is_err());
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "{ broken", "a damaged file is left for an administrator to look at");
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_world_readable_and_not_group_writable() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("service.json");
        allow(&f, "uid:1000").unwrap();
        assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o644);
    }
}
