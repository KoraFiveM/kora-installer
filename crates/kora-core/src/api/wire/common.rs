//! Shapes shared by many operations: a started job, the job record.

use serde::{Deserialize, Serialize};

/// `202` answer of every job operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct JobStarted {
    pub job: String,
}
