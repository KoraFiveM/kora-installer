//! The support bundle: what is in it (shown before it is saved), one file's text, and where it was saved.
//! Nothing here is ever uploaded; every text has been through `support::redact`.

use serde::{Deserialize, Serialize};

use crate::support::bundle::{BundleFile, Omitted};

/// `POST /support/bundle/preview` and `POST /support/summary`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SupportRequest {
    /// Only this server's files (left out: every server).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub server: Option<String>,
}

/// What a bundle holds, built and kept for a few minutes so that what is saved is exactly what was shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BundleListing {
    pub id: String,
    pub files: Vec<BundleFile>,
    /// Left out altogether, and why.
    pub omitted: Vec<Omitted>,
    pub total_bytes: u64,
    /// Where it will be saved, on the computer that runs Kora.
    pub folder: String,
    /// "Kora never uploads this…", said by the program, not the page.
    pub never_uploaded: String,
}

/// `POST /support/bundle/file`: one file's (cleaned) text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BundleFileRequest {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BundleFileText {
    pub name: String,
    pub text: String,
}

/// `POST /support/bundle`: save what was previewed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BundleCreateRequest {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BundleSaved {
    pub file_name: String,
    /// The full path on the computer that runs Kora.
    pub path: String,
    pub bytes: u64,
    pub folder: String,
}

/// `POST /support/summary`: the short cleaned text an issue report may attach (never the file).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SupportSummary {
    pub text: String,
}
