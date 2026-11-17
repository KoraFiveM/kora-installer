//! What the person chose about reachability.
//!
//! * Per server, `<server>/.kora/reach.json` (like the webhook and alert settings): whether Kora may
//!   ask the router to forward the game port, and the last thing that happened. No secrets.
//! * For the whole computer, `<kora home>/reach-consent.json`: which outside services the person
//!   allowed the reachability check to contact. Both default to "no".

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::installer::registry::kora_home;

/// What the last attempt to forward the port did (written by whichever process maintains the mapping).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ForwardStatus {
    /// Unix seconds.
    pub at: u64,
    pub ok: bool,
    /// `UPnP`, `NAT-PMP` or `PCP` when it worked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub via: Option<String>,
    /// In words for a person.
    pub message: String,
    /// The router's own report of its internet-side address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub external_ip: Option<String>,
    /// The mapping is not removed by the router itself (`lease` 0): only Kora removes it.
    #[serde(default)]
    pub permanent: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerReach {
    /// Ask the router to forward the game port while the server runs. Off unless chosen.
    #[serde(default)]
    pub forward: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ForwardStatus>,
    /// The server's Cfx.re join code, remembered from its log (which rotates).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join_code: Option<String>,
}

pub fn server_path(dir: &Path) -> PathBuf {
    dir.join(".kora").join("reach.json")
}

pub fn load_server(dir: &Path) -> ServerReach {
    std::fs::read(server_path(dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save_server(dir: &Path, s: &ServerReach) -> Result<(), String> {
    let p = server_path(dir);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Cannot save the setting: {e}"))?;
    }
    let text = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    crate::monitor::write_private(&p, &text).map_err(|e| format!("Cannot save the setting: {e}"))
}

/// Remember the outcome without touching the choice (the file is re-read, so a switch made in the
/// meantime is not undone).
pub fn record_status(dir: &Path, st: Option<ForwardStatus>) {
    let mut s = load_server(dir);
    if s.status == st {
        return;
    }
    s.status = st;
    let _ = save_server(dir, &s);
}

/// Which outside services the reachability check may contact. Nothing is contacted by default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Consent {
    /// Ask Cfx.re's public server list about this server's own join code (it learns nothing new: the
    /// server already announces itself there). Gives the address Cfx.re sees.
    #[serde(default)]
    pub cfx_list: bool,
    /// Ask a plain "what is my address" website (`api.ipify.org`) for this connection's public
    /// address. It sees that address and that a request was made, nothing else.
    #[serde(default)]
    pub ip_echo: bool,
}

pub fn consent_path() -> PathBuf {
    kora_home().join("reach-consent.json")
}

pub fn load_consent_at(p: &Path) -> Consent {
    std::fs::read(p).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save_consent_at(p: &Path, c: &Consent) -> Result<(), String> {
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Cannot save the setting: {e}"))?;
    }
    let text = serde_json::to_string_pretty(c).map_err(|e| e.to_string())?;
    crate::monitor::write_private(p, &text).map_err(|e| format!("Cannot save the setting: {e}"))
}

pub fn load_consent() -> Consent {
    load_consent_at(&consent_path())
}

pub fn save_consent(c: &Consent) -> Result<(), String> {
    save_consent_at(&consent_path(), c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_off() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(load_server(d.path()), ServerReach::default());
        assert!(!load_server(d.path()).forward);
        assert_eq!(load_consent_at(&d.path().join("nope.json")), Consent::default());
        std::fs::create_dir_all(d.path().join(".kora")).unwrap();
        std::fs::write(server_path(d.path()), "{ broken").unwrap();
        assert!(!load_server(d.path()).forward);
    }

    #[test]
    fn roundtrip_and_status_keeps_the_choice() {
        let d = tempfile::tempdir().unwrap();
        save_server(d.path(), &ServerReach { forward: true, ..Default::default() }).unwrap();
        record_status(d.path(), Some(ForwardStatus { at: 5, ok: true, via: Some("UPnP".into()), message: "ok".into(), external_ip: None, permanent: false }));
        let s = load_server(d.path());
        assert!(s.forward);
        assert_eq!(s.status.unwrap().via.as_deref(), Some("UPnP"));
        let p = d.path().join("c.json");
        save_consent_at(&p, &Consent { cfx_list: true, ip_echo: false }).unwrap();
        assert_eq!(load_consent_at(&p), Consent { cfx_list: true, ip_echo: false });
    }
}
