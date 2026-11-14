//! Step engine: an install plan is an ordered list of idempotent steps whose
//! status is persisted, which enables resume and rollback.
//!
//! Contract for a [`Step`]:
//! * `verify` reports whether the desired outcome already holds (cheap, no side effects).
//! * `run` must be idempotent: running it again after success or partial failure is safe.
//! * `rollback` removes only what Kora itself created.

mod state;

pub use state::{PlanState, StepRecord, StepStatus};

use std::path::Path;

/// Error type for the step engine.
#[derive(Debug, thiserror::Error)]
pub enum StepError {
    #[error("step failed: {0}")]
    Failed(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("state error: {0}")]
    State(#[from] serde_json::Error),
}

pub type StepResult<T> = Result<T, StepError>;

/// One idempotent unit of work in an install plan.
pub trait Step {
    /// Stable unique id (used as the persisted key).
    fn id(&self) -> &str;
    /// Plain-language description shown to the user.
    fn description(&self) -> &str;
    /// Is the desired outcome already in place?
    fn verify(&self) -> StepResult<bool>;
    /// Perform the step. Must be idempotent.
    fn run(&mut self) -> StepResult<()>;
    /// Undo what this step created. Must be safe if `run` never completed.
    fn rollback(&mut self) -> StepResult<()>;
}

/// Run `steps` in order, persisting status to `state_path` after every transition.
/// Steps already `Done` (and still verifying) are skipped, which gives resume.
/// On failure the step is marked `Failed` and the error is returned.
pub fn run_plan(steps: &mut [Box<dyn Step>], state_path: &Path) -> StepResult<PlanState> {
    let mut state = PlanState::load_or_default(state_path)?;
    for s in steps.iter() {
        state.ensure(s.id());
    }
    state.save(state_path)?;
    for s in steps.iter_mut() {
        let id = s.id().to_string();
        if state.status(&id) == Some(StepStatus::Done) && s.verify()? {
            continue;
        }
        if s.verify()? {
            state.set(&id, StepStatus::Done, None);
            state.save(state_path)?;
            continue;
        }
        state.set(&id, StepStatus::Running, None);
        state.save(state_path)?;
        match s.run() {
            Ok(()) => state.set(&id, StepStatus::Done, None),
            Err(e) => {
                state.set(&id, StepStatus::Failed, Some(e.to_string()));
                state.save(state_path)?;
                return Err(e);
            }
        }
        state.save(state_path)?;
    }
    Ok(state)
}

/// Roll back steps in reverse order, for those that started (not `Pending`/`RolledBack`).
pub fn rollback_plan(steps: &mut [Box<dyn Step>], state_path: &Path) -> StepResult<PlanState> {
    let mut state = PlanState::load_or_default(state_path)?;
    for s in steps.iter_mut().rev() {
        let id = s.id().to_string();
        match state.status(&id) {
            None | Some(StepStatus::Pending) | Some(StepStatus::RolledBack) => continue,
            _ => {}
        }
        s.rollback()?;
        state.set(&id, StepStatus::RolledBack, None);
        state.save(state_path)?;
    }
    Ok(state)
}

#[cfg(test)]
mod tests;
