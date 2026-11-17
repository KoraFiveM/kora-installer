use std::{fs, path::Path};

use serde::{Deserialize, Serialize};

use super::StepResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    Running,
    Done,
    Failed,
    RolledBack,
    /// The user chose to carry on without this step (after it failed). [`super::run_plan`] leaves
    /// it alone from then on; rollback still undoes whatever part of it ran.
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepRecord {
    pub id: String,
    pub status: StepStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Persisted install-plan state (ordered).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanState {
    pub steps: Vec<StepRecord>,
}

impl PlanState {
    pub fn load_or_default(path: &Path) -> StepResult<Self> {
        match fs::read_to_string(path) {
            Ok(s) => Ok(serde_json::from_str(&s)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// Atomic save (temp file, then rename), never through a planted link: root saves this in
    /// `<server>/.kora`, which the server's own account can change.
    pub fn save(&self, path: &Path) -> StepResult<()> {
        crate::fsutil::write_no_follow(path, &serde_json::to_vec_pretty(self)?, 0o644)?;
        Ok(())
    }

    pub fn status(&self, id: &str) -> Option<StepStatus> {
        self.steps.iter().find(|r| r.id == id).map(|r| r.status)
    }

    pub fn ensure(&mut self, id: &str) {
        if self.status(id).is_none() {
            self.steps.push(StepRecord { id: id.to_string(), status: StepStatus::Pending, error: None });
        }
    }

    pub fn set(&mut self, id: &str, status: StepStatus, error: Option<String>) {
        self.ensure(id);
        if let Some(r) = self.steps.iter_mut().find(|r| r.id == id) {
            r.status = status;
            r.error = error;
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// Root saves this in `<server>/.kora`: a link planted at the file or its temporary name is
    /// replaced, never written through.
    #[test]
    fn save_never_writes_through_a_planted_link() {
        let t = tempfile::tempdir().unwrap();
        let victim = t.path().join("victim");
        std::fs::write(&victim, "precious").unwrap();
        let p = t.path().join(".kora/plan-state.json");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&victim, p.with_extension("json.tmp")).unwrap();
        std::os::unix::fs::symlink(&victim, &p).unwrap();
        let mut st = PlanState::default();
        st.set("a", StepStatus::Done, None);
        st.save(&p).unwrap();
        assert_eq!(fs::read_to_string(&victim).unwrap(), "precious");
        assert_eq!(PlanState::load_or_default(&p).unwrap(), st);
    }
}
