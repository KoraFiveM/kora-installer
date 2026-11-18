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
    /// The step failed in a way the person can read (and maybe act on).
    #[error("step failed: {0}")]
    Failed(crate::problem::Problem),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("state error: {0}")]
    State(#[from] serde_json::Error),
}

impl From<StepError> for crate::problem::Problem {
    fn from(e: StepError) -> Self {
        match e {
            StepError::Failed(p) => p,
            StepError::Io(e) => e.into(),
            StepError::State(e) => crate::problem::Problem::from(e).context("saving the install progress"),
        }
    }
}

impl From<crate::problem::Problem> for StepError {
    fn from(p: crate::problem::Problem) -> Self {
        StepError::Failed(p)
    }
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
/// Steps already `Done` (and still verifying) are skipped, which gives resume; so are steps the
/// user chose to skip ([`mark_skipped`]). Running the plan again after a failure is "retry the
/// failed step": it starts at the first step that is not done.
/// On failure the step is marked `Failed` (with the error text) and the error is returned.
pub fn run_plan(steps: &mut [Box<dyn Step>], state_path: &Path) -> StepResult<PlanState> {
    let mut state = PlanState::load_or_default(state_path)?;
    for s in steps.iter() {
        state.ensure(s.id());
    }
    state.save(state_path)?;
    for s in steps.iter_mut() {
        let id = s.id().to_string();
        if state.status(&id) == Some(StepStatus::Skipped) {
            continue;
        }
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

/// Record that the user chose to go on without step `id`: the next [`run_plan`] passes over it.
/// Only a step that has failed (or never ran) can be skipped; a done step stays done. The caller
/// decides which steps are safe to skip.
pub fn mark_skipped(state_path: &Path, id: &str) -> StepResult<PlanState> {
    let mut state = PlanState::load_or_default(state_path)?;
    match state.status(id) {
        Some(StepStatus::Done) => {}
        _ => {
            state.set(id, StepStatus::Skipped, None);
            state.save(state_path)?;
        }
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
