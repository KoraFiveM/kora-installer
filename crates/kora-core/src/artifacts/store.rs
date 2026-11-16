//! `artifacts/<build>/` layout with a persisted current/previous pointer.

use super::{extract_archive, ArtifactError, ExtractLimits, Platform, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreState {
    pub current: Option<u32>,
    pub previous: Option<u32>,
}

/// Manages `<server>/artifacts/`. Holds at most the current and previous build.
pub struct ArtifactStore {
    root: PathBuf,
}

impl ArtifactStore {
    pub fn new(artifacts_dir: impl Into<PathBuf>) -> Self {
        ArtifactStore { root: artifacts_dir.into() }
    }
    pub fn build_dir(&self, build: u32) -> PathBuf {
        self.root.join(build.to_string())
    }
    /// Directory of the active build, if any.
    pub fn current_dir(&self) -> Result<Option<PathBuf>> {
        Ok(self.state()?.current.map(|b| self.build_dir(b)))
    }
    fn state_path(&self) -> PathBuf {
        self.root.join("state.json")
    }
    pub fn state(&self) -> Result<StoreState> {
        match fs::read(self.state_path()) {
            Ok(b) => serde_json::from_slice(&b).map_err(|e| ArtifactError::Store(e.to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(StoreState::default()),
            Err(e) => Err(e.into()),
        }
    }
    fn save(&self, s: &StoreState) -> Result<()> {
        // Never through a link planted in the server folder (root installs and swaps builds).
        crate::fsutil::write_no_follow(&self.state_path(), &serde_json::to_vec_pretty(s).map_err(|e| ArtifactError::Store(e.to_string()))?, 0o644)?;
        Ok(())
    }

    /// Extract `archive` as `build` and make it current. The old current becomes `previous`;
    /// the build before that is deleted. Nothing changes if extraction fails. Stop the server first.
    pub fn swap(&self, build: u32, archive: &Path, platform: Platform) -> Result<()> {
        fs::create_dir_all(&self.root)?;
        let st = self.state()?;
        if st.current == Some(build) {
            return Err(ArtifactError::Store(format!("build {build} is already current")));
        }
        let staging = self.root.join(format!("{build}.partial"));
        if staging.exists() {
            fs::remove_dir_all(&staging)?;
        }
        fs::create_dir(&staging)?;
        if let Err(e) = extract_archive(archive, &staging, platform, ExtractLimits::default()) {
            let _ = fs::remove_dir_all(&staging);
            return Err(e);
        }
        let target = self.build_dir(build);
        // A leftover dir for this build is only ever the old `previous` (or an orphan): replace it.
        if target.exists() {
            fs::remove_dir_all(&target)?;
        }
        crate::fsutil::rename_retrying(&staging, &target)?;
        let new = StoreState { current: Some(build), previous: st.current };
        self.save(&new)?;
        if let Some(old) = st.previous {
            if Some(old) != new.current && Some(old) != new.previous {
                let _ = fs::remove_dir_all(self.build_dir(old));
            }
        }
        Ok(())
    }

    /// One-step rollback: previous becomes current (and the build we leave becomes `previous`,
    /// so a second call undoes the first). Errors if there is no previous build on disk.
    pub fn rollback(&self) -> Result<u32> {
        let st = self.state()?;
        let prev = st.previous.ok_or_else(|| ArtifactError::Store("no previous build to roll back to".into()))?;
        if !self.build_dir(prev).is_dir() {
            return Err(ArtifactError::Store(format!("previous build {prev} is missing on disk")));
        }
        self.save(&StoreState { current: Some(prev), previous: st.current })?;
        Ok(prev)
    }
}
