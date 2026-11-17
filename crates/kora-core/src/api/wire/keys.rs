//! Saved licence keys and what Kora remembers between installs.

use serde::{Deserialize, Serialize};

/// What Kora remembers between installs (nothing secret). `null` forgets a field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Profile {
    pub cfx_username: Option<String>,
    pub cfx_identifier: Option<String>,
    pub github_owner: Option<String>,
}

/// A saved key as the panel shows it: the name and the last four characters, never the key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SavedKeyInfo {
    pub name: String,
    pub last4: String,
    /// Unix seconds (0 = unknown).
    pub added: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SavedKey {
    pub name: String,
    pub last4: String,
    pub added: u64,
    /// Servers whose configuration holds exactly this key.
    pub servers: Vec<String>,
    /// Unfinished installs (and saved answers) that use it.
    pub stopped_installs: u32,
}

/// `GET /keys`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SavedKeys {
    pub keys: Vec<SavedKey>,
    /// false: this sign-in can only pick from the saved names (admins and this computer manage them).
    pub can_manage: bool,
    /// Where the file is (only told to the computer that runs Kora).
    pub path: Option<String>,
}

/// `POST /keys`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct KeyAddRequest {
    pub name: String,
    pub license_key: String,
}

/// `POST /keys` answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct KeyAdded {
    pub key: SavedKeyInfo,
}

/// `PUT /keys/{name}`: rename, put a new key under the name, or both.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct KeyUpdateRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub new_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub license_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct KeyChange {
    pub key: SavedKeyInfo,
    pub notes: Vec<String>,
}

/// `DELETE /keys/{name}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct KeyRemoved {
    pub removed: bool,
    pub notes: Vec<String>,
}

/// `PUT /servers/{s}/licence-key`: a saved key by name, or a new one (optionally saved).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(untagged)]
pub enum ServerKeyRequest {
    Saved {
        name: String,
    },
    Pasted {
        license_key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        save_as: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ServerKeyChanged {
    pub server: String,
    pub last4: String,
    /// The saved key it was set from (null: a pasted key that was not saved).
    pub key_name: Option<String>,
    pub running: bool,
    pub restart_needed: bool,
    pub message: String,
}

/// `PUT /profile`: a field left out stays as it is; `null` forgets it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ProfileUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    pub cfx_username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    pub cfx_identifier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    pub github_owner: Option<String>,
}
