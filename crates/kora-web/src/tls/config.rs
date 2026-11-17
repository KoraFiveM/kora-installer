//! The saved HTTPS choice (`panel-tls` document in the store): no secrets, only which source to use and
//! where a provided certificate lives. The local CA's keys are in their own private table.
use kora_core::store::{documents, Store};
use serde::{Deserialize, Serialize};

pub const DOC: &str = "panel-tls";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TlsMode {
    /// Use what is there: a provided certificate, else Tailscale's when the tailnet has HTTPS, else none.
    #[default]
    Auto,
    Provided,
    Tailscale,
    SelfSigned,
    /// Plain HTTP even on other computers' addresses (`kora panel tls off`).
    Off,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TlsConfig {
    pub mode: TlsMode,
    pub cert_path: Option<String>,
    pub key_path: Option<String>,
    /// Extra names for the self-signed certificate (`kora panel tls self-signed --name box.lan`).
    pub names: Vec<String>,
    /// Bumped by `kora panel tls renew` so a running panel notices and fetches or makes a new certificate.
    pub renewed: u64,
}

impl TlsConfig {
    pub fn load(s: &Store) -> Result<Self, String> {
        match documents::get(s, DOC).map_err(kora_core::store::problem_to_string)? {
            None => Ok(Self::default()),
            Some(v) => serde_json::from_value(v).map_err(|e| format!("the saved HTTPS settings cannot be read ({e}); they were left as they were")),
        }
    }
    pub fn save(&self, s: &Store) -> Result<(), String> {
        documents::put(s, DOC, &serde_json::to_value(self).map_err(|e| e.to_string())?).map_err(kora_core::store::problem_to_string)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_default_to_auto() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(d.path()).unwrap();
        assert_eq!(TlsConfig::load(&s).unwrap().mode, TlsMode::Auto);
        let c = TlsConfig { mode: TlsMode::Provided, cert_path: Some("/c.pem".into()), key_path: Some("/k.pem".into()), names: vec!["a".into()], renewed: 0 };
        c.save(&s).unwrap();
        assert_eq!(TlsConfig::load(&s).unwrap(), c);
        assert!(serde_json::to_string(&c).unwrap().contains("\"provided\""));
    }
}
