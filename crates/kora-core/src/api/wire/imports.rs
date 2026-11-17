//! Moving a server to this computer: the upload, the look at the file, and the import (a job).
//! The shapes of the preview and the request are in [`crate::backup::import`].

use serde::{Deserialize, Serialize};

use crate::backup::import::ImportSource;

/// `POST /backups/import/upload` answers this: the file is on this computer, waiting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ImportUploaded {
    /// The id to give `inspect` and `import` as `source.upload`.
    pub upload: String,
    pub file_name: String,
    pub bytes: u64,
}

/// `POST /backups/import/inspect`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ImportInspectRequest {
    pub source: ImportSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub passphrase: Option<String>,
}
