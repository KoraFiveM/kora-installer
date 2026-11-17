//! Import of the old JSON state files (filled in by a later slice).
use super::Store;
use crate::problem::Problem;

pub fn import_if_needed(_s: &mut Store) -> Result<(), Problem> {
    Ok(())
}

/// Is any old JSON state file (or the stopped-installs folder) in `home`?
pub fn any_legacy_files(home: &std::path::Path) -> bool {
    ["servers.json", "secrets.json", "profile.json", "last-install.json", "panel.json", "hub.json", "paused-installs"].iter().any(|n| std::fs::symlink_metadata(home.join(n)).is_ok())
}
