//! Shapes shared by many operations: a started job, the job record, the error envelope.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::JobState;
use crate::problem::{Action, Problem};

/// `202` answer of every job operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct JobStarted {
    pub job: String,
}

/// What a failed job (or a failed request) carries: a [`Problem`] plus the extra fields some failures add.
/// On the wire a [`Problem`] and these fields are one flat object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ErrorBody {
    /// Stable machine code (`failed`, `needs_elevation`, `github.signed_out`, ...), see `problem::codes`.
    pub code: String,
    /// What happened, in plain words, for anyone.
    pub message: String,
    /// Technical details for "Show details".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub details: Option<String>,
    /// What the user can do, best first. An action kind this page does not know is not shown.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<Action>>", optional))]
    pub actions: Vec<Action>,
    /// `needs_elevation`: the command to run as administrator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub command: Option<Vec<String>>,
    /// The message as a catalogue id plus parameters, for a front end that translates (`Problem::i18n`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub i18n: Option<crate::problem::MessageRef>,
}

impl From<Problem> for ErrorBody {
    fn from(p: Problem) -> Self {
        ErrorBody { code: p.code, message: p.message, details: p.details, actions: p.actions, command: None, i18n: p.i18n }
    }
}

/// Every error answer of the API: `{"error": {...}}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ErrorEnvelope {
    pub error: ErrorBody,
}

/// The error of a failed job: an [`ErrorBody`] plus what installs add.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct JobError {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub details: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<Action>>", optional))]
    pub actions: Vec<Action>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub command: Option<Vec<String>>,
    /// The message as a catalogue id plus parameters, for a front end that translates (`Problem::i18n`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub i18n: Option<crate::problem::MessageRef>,
    /// Install failures: what was kept on disk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub kept: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub log_path: Option<String>,
    /// Install failures: the step that failed (absent: it stopped before the first step).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub failed_step: Option<String>,
    /// Install failures: `POST /installs/{job}/resume` can carry on with the same answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub resume: Option<bool>,
}

/// `GET /jobs/{id}`. `E` is the type of the job's events and `R` of its result (see the operation's entry in `OpTypes`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct JobRecord<R = Value, E = Value> {
    pub id: String,
    /// The operation that started it (`Op::name`).
    pub op: String,
    pub server: Option<String>,
    pub state: JobState,
    pub events: Vec<E>,
    pub result: Option<R>,
    pub error: Option<JobError>,
    /// Unix seconds.
    pub started: u64,
    /// Unix seconds; `null` while it runs.
    pub ended: Option<u64>,
}

/// One job in `GET /jobs`: this run's and, after a restart, the remembered ones (newest first, no events).
/// `state` can be `interrupted`: the panel stopped while it ran, and `error` says what to do (`resume` when an install can carry on).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct JobSummary {
    pub id: String,
    pub op: String,
    pub server: Option<String>,
    pub state: JobState,
    pub events: Vec<Value>,
    #[cfg_attr(feature = "ts", ts(as = "Option<serde_json::Value>"))]
    pub result: Option<Value>,
    pub error: Option<JobError>,
    pub started: u64,
    pub ended: Option<u64>,
}

/// `?limit=` of `GET /jobs` (default 50, at most 300).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct JobsQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub limit: Option<u32>,
}

/// `GET /jobs`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct JobsList {
    pub jobs: Vec<JobSummary>,
}

/// Data of the `failed` event of `GET /jobs/{id}/events`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct JobFailedData {
    /// A plain message, or a packed problem as JSON text.
    pub error: String,
}
