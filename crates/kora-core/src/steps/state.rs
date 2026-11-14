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

    /// Atomic save (write temp file, then rename).
    pub fn save(&self, path: &Path) -> StepResult<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn status(&self, id: &str) -> Option<StepStatus> {
        self.steps.iter().find(|r| r.id == id).map(|r| r.status)
    }

    pub fn ensure(&mut self, id: &str) {
        if self.status(id).is_none() {
            self.steps.push(StepRecord {
                id: id.to_string(),
                status: StepStatus::Pending,
                error: None,
            });
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
