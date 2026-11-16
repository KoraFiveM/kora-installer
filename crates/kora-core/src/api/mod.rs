//! The one way front ends (TUI, CLI, web panel) reach the engine.
//!
//! * [`backend`]: the [`Backend`] trait, [`Status`], [`UpdateBadge`], [`badge_of`].
//! * [`system`]: [`SystemBackend`], the real implementation, plus `with_ensure`, `valid_resource_name`, `read_badge`.
//! * [`adopt`]: finding server folders to add; [`logs`]: log files and their tails.
//! * [`ops`]: the [`Op`] list with names and routes; [`jobs`]: [`JobRegistry`]; [`dto`]: API-only shapes.

pub mod adopt;
pub mod backend;
pub mod dto;
#[cfg(any(test, feature = "fake"))]
pub mod fake;
pub mod jobs;
pub mod logs;
pub mod panel_updates;
pub mod ops;
pub mod system;

pub use backend::{badge_of, Backend, Res, Status, StatusDetail, UpdateBadge};
pub use dto::WebhookView;
pub use jobs::{Conflict, JobHandle, JobId, JobMessage, JobRegistry, JobSnapshot, JobState};
pub use ops::{Method, Op};
pub use system::{read_badge, valid_resource_name, with_ensure, SystemBackend};
