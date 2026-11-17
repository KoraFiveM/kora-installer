//! This computer and this panel: `GET /system`, remote access settings, panel users.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::session::{PlatformName, Role};

/// `GET /system`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SystemInfo {
    pub version: String,
    pub platform: PlatformName,
    /// Kora runs with administrator rights (root).
    pub elevated: bool,
    /// Same as `elevated` (the panel reads this name).
    pub admin: bool,
    pub helper_installed: bool,
    /// Windows: the panel asks Windows for administrator permission itself (one prompt per install or action).
    pub admin_prompt: bool,
    pub headless: bool,
    /// The browser is on the same machine as Kora (enables "Open folder").
    pub local: bool,
    pub kora_home: String,
    /// The panel can restart itself ("Restart now" after a self-update).
    pub can_restart: bool,
}

/// `POST /panel/restart`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PanelRestarting {
    pub restarting: bool,
}

/// A person who can sign in from another computer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PanelUser {
    pub name: String,
    pub role: Role,
    /// Unix seconds.
    pub created: u64,
}

/// `GET /users`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct UsersList {
    pub users: Vec<PanelUser>,
}

/// `POST /users`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct UserAddRequest {
    pub name: String,
    pub role: Role,
}

/// `POST /users` answer: the token is shown this once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct NewUser {
    pub name: String,
    pub role: Role,
    pub token: String,
}

/// `GET /remote` (local sessions only). Token hashes are never sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RemoteSettings {
    pub enabled: bool,
    /// On for this run because of `--remote`, whatever the saved setting says.
    pub enabled_by_flag: bool,
    pub bind: String,
    pub port: u16,
    /// Where Tailscale devices open the panel, while it also listens there (`kora panel --tailscale`).
    pub tailnet_url: Option<String>,
    pub tailscale: bool,
    pub tailscale_users: BTreeMap<String, Role>,
    pub allowed_hosts: Vec<String>,
    pub allow_remote_destructive: bool,
    pub users: Vec<PanelUser>,
    pub saved_to: Option<String>,
}

/// `PUT /remote`: what is left out stays as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct RemoteSettingsUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub tailscale: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub tailscale_users: Option<BTreeMap<String, Role>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub allowed_hosts: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub allow_remote_destructive: Option<bool>,
}
