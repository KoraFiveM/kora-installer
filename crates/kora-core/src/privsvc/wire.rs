//! The wire: one JSON line in ([`Request`]), progress lines and one result out ([`Message`]). The same
//! framing on the Linux socket and the Windows pipe. Pure data and one helper; the rules are in [`super::op`].

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The version-ping verb: answered with the version and whether this caller is served; runs nothing.
pub const PING: &str = "ping";

/// One request (see [`super`]). `Debug` hides the environment values.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Request {
    pub verb: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request").field("verb", &self.verb).field("args", &self.args).field("env", &self.env.keys().collect::<Vec<_>>()).finish()
    }
}

/// One line of the answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    Progress {
        stream: String,
        line: String,
    },
    Result {
        ok: bool,
        code: i32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<String>,
        /// What the step reported back (`admin-step` only).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        data: Option<serde_json::Value>,
    },
    Error {
        message: String,
    },
}

/// Why a request was refused (plain language; shown to the user).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejected(pub String);

impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Mask every secret value (6+ characters) in `line`.
pub fn mask_secrets(line: &str, secrets: &[&str]) -> String {
    let mut out = line.to_string();
    for s in secrets.iter().filter(|s| s.len() >= 6) {
        out = out.replace(s, "***");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_debug_hides_values_and_secrets_are_masked() {
        let mut r = Request { verb: "install".into(), args: vec![], env: BTreeMap::new() };
        r.env.insert("CFX_LICENSE_KEY".into(), "cfxk_TOPSECRET".into());
        assert!(!format!("{r:?}").contains("TOPSECRET"));
        assert_eq!(mask_secrets("key is cfxk_TOPSECRET ok", &["cfxk_TOPSECRET", "x"]), "key is *** ok");
    }

    #[test]
    fn messages_round_trip() {
        let m = Message::Result { ok: true, code: 0, message: None, version: Some("1".into()), data: None };
        let s = serde_json::to_string(&m).unwrap();
        assert!(s.contains(r#""type":"result""#));
        assert_eq!(serde_json::from_str::<Message>(&s).unwrap(), m);
    }
}
