//! The page of one resource.

use serde::{Deserialize, Serialize};

use crate::branches::ResourceBranch;
use crate::installer::Mode;
use crate::resourcestate::RestartInfo;

/// The server the resource is on, as far as the page needs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ResourceServerState {
    pub name: String,
    pub mode: Mode,
    pub running: bool,
    pub console_enabled: bool,
    pub console_waiting_for_restart: bool,
}

/// `GET /servers/{s}/resources/{name}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ResourcePage {
    pub id: String,
    /// Relative to `resources/`.
    pub folder: String,
    pub repository: String,
    /// The branch it normally follows.
    pub default_branch: String,
    /// The release it is pinned to, when the server uses released versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub pinned_commit: Option<String>,
    pub third_party: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub version: Option<String>,
    /// Where its files are: the normal version, a branch, a pull request.
    pub branch: ResourceBranch,
    pub restart: RestartInfo,
    pub server: ResourceServerState,
}
