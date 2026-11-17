//! `GET /meta`, sign-in, who is signed in.

use serde::{Deserialize, Serialize};

/// Lowest to highest (`kora_web::remote::Role`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Viewer,
    Operator,
    Admin,
}

/// How a session proved who it is: the launch link, Tailscale, a passkey, a password with a one-time code, the old
/// user token, or (requests only) an API token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Local,
    Tailscale,
    Token,
    Passkey,
    Code,
    ApiToken,
}

/// Who is signed in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Identity {
    pub name: String,
    pub provider: Provider,
    pub role: Role,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum PlatformName {
    Windows,
    Linux,
}

/// `GET /meta`. `identity` is `null` unless the request carries a valid session or token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Meta {
    pub version: String,
    pub api: u32,
    pub platform: PlatformName,
    /// The names of the operations this panel serves (`Op::name`).
    pub ops: Vec<String>,
    /// This computer's name.
    pub node: String,
    pub identity: Option<Identity>,
    /// The request came from another computer.
    pub remote: bool,
    /// Remote sessions may use the destructive actions (delete, uninstall, console input, ...).
    pub allow_remote_destructive: bool,
    /// A token sign-in of a user without a passkey or one-time codes: the page should suggest setting one up.
    pub auth_upgrade: bool,
}

/// `POST /session`: local `{token}` or `{code}`; remote `{token}` or `{}` (Tailscale).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SessionRequest {
    /// The launch token, a sign-in code as pasted, or (remote) a user token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub token: Option<String>,
    /// Local only: a one-time code from a sign-in link (`/#code=...`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub code: Option<String>,
}

/// `POST /session`: the cookie is set; `secret` must come back with every request (`X-Kora-Session`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SessionCreated {
    pub ok: bool,
    pub identity: Identity,
    pub secret: String,
}

/// `POST /session/link`: a one-time sign-in code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SessionLink {
    pub code: String,
    /// Seconds.
    pub expires_in: u64,
}
