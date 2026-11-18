//! Reachability, port forwarding and txAdmin through the panel. The checklist itself
//! ([`crate::reach::ReachReport`]) is an engine type that already has the wire shape.

use serde::{Deserialize, Serialize};

/// `POST /servers/{s}/reach` and `PUT /reach/settings`: which outside lookups to allow. A field that is left out
/// changes nothing (settings) or keeps the saved choice (a check). A check never saves what it is given.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ReachConsentPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub cfx_list: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub ip_echo: Option<bool>,
}

/// `PUT /servers/{s}/forwarding`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ForwardingRequest {
    pub enabled: bool,
}

/// `POST /servers/{s}/txadmin/ticket`: where to open txAdmin through the panel. The address works once, for a minute.
/// txAdmin has its own sign-in; `username` is the account Kora made at install (never its password).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TxAdminTicket {
    pub url: String,
    /// The panel-side port of this server's txAdmin.
    pub port: u16,
    pub expires_in: u64,
    pub username: Option<String>,
}
