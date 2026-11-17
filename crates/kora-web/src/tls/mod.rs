//! HTTPS for the panel.
//!
//! # Why, and when
//! The panel speaks plain HTTP on the loopback address (nothing leaves the computer, and a certificate
//! warning there would only scare people) and inside a Tailscale tunnel (already encrypted). On a home
//! network or a public address the session cookie, the sign-in link and everything typed (licence keys)
//! would cross the network in clear. So:
//!
//! * a listener on the loopback address stays plain HTTP unless the person asks otherwise;
//! * any other listener serves HTTPS as soon as there is a certificate, on the SAME port: the first byte of
//!   a connection tells a TLS handshake from plain HTTP, and plain HTTP from another computer gets a redirect
//!   to `https://` (plain HTTP from this computer is still served, so the launch link keeps working);
//! * without a certificate such a listener is plain HTTP and says so loudly ([`plain_http_warning`]);
//!   `--allow-public` refuses to start without HTTPS or a configured trusted proxy ([`rules`]).
//!
//! # Where the certificate comes from, in this order ([`TlsMode::Auto`])
//! 1. one the person provides (`kora panel tls set --cert --key`): the files are watched and re-read when
//!    they change, so a renewal by certbot or a script needs no restart; the key file must not be readable by
//!    other accounts ([`certs::check_key_file`]); HSTS is sent;
//! 2. for Tailscale users, `tailscale cert` for the machine's tailnet name ([`tsnet`]): fetched when the
//!    panel starts and renewed before it expires; needs HTTPS enabled for the tailnet (Tailscale admin
//!    console, DNS page) and MagicDNS; HSTS is sent;
//! 3. a self-signed certificate from a stable local CA ([`selfsigned`]), only when asked for
//!    (`kora panel tls self-signed`): trusted once by the person, no HSTS (it is a certificate the browser
//!    was told to accept, and a mistake would lock the person out of the name for a year).
//!
//! Public domains with ACME (Let's Encrypt) are out of scope: put a reverse proxy in front instead, and
//! tell Kora to trust it ([`crate::remote::proxy`]).
//!
//! # Threat model
//! Network attackers who can read or change traffic between the browser and the panel: TLS 1.2/1.3 with
//! ring's suites (rustls defaults), no client certificates, no session tickets reuse beyond rustls
//! defaults. Private keys (provided, Tailscale, local CA) never appear in the API, logs or audit log. An
//! attacker who can write the provided files could swap the certificate: those files must be private to the
//! person who runs Kora. A certificate that cannot be loaded is NEVER replaced silently by plain HTTP on a
//! listener that already served HTTPS: the old certificate stays until it expires, and the error is shown.
pub mod certs;
pub mod config;
pub mod manager;
pub mod rules;
pub mod selfsigned;
pub mod serve;
pub mod tsnet;

use std::sync::{Arc, RwLock};

use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;

pub use config::{TlsConfig, TlsMode};

/// Where the certificate in use came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CertSource {
    Provided,
    Tailscale,
    SelfSigned,
}

impl CertSource {
    pub fn label(self) -> &'static str {
        match self {
            CertSource::Provided => "a certificate you provided",
            CertSource::Tailscale => "a Tailscale certificate",
            CertSource::SelfSigned => "a self-signed certificate (made by Kora)",
        }
    }
    /// Only certificates the person chose on purpose, or that Tailscale vouches for, earn HSTS.
    pub fn hsts(self) -> bool {
        matches!(self, CertSource::Provided | CertSource::Tailscale)
    }
}

/// What is known about the certificate in use (never the key).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertInfo {
    pub source: CertSource,
    pub names: Vec<String>,
    pub not_before: i64,
    pub not_after: i64,
    pub fingerprint: String,
    /// For a self-signed certificate: the CA's fingerprint (what the person installs and compares).
    pub ca_fingerprint: Option<String>,
}

#[derive(Debug)]
struct Resolver {
    key: RwLock<Option<Arc<CertifiedKey>>>,
}

impl ResolvesServerCert for Resolver {
    fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        self.key.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }
}

/// The certificate in use, swapped without a restart. Cloning shares it.
#[derive(Clone)]
pub struct TlsState {
    resolver: Arc<Resolver>,
    info: Arc<RwLock<Option<CertInfo>>>,
    /// Why the last attempt to load or fetch a certificate failed (shown in Settings and the System check).
    last_error: Arc<RwLock<Option<String>>>,
    server: Arc<rustls::ServerConfig>,
}

impl Default for TlsState {
    fn default() -> Self {
        Self::new()
    }
}

impl TlsState {
    pub fn new() -> Self {
        let resolver = Arc::new(Resolver { key: RwLock::new(None) });
        let mut cfg = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("ring supports the default protocol versions")
            .with_no_client_auth()
            .with_cert_resolver(resolver.clone());
        // The panel's connection loop speaks HTTP/1.1 (WebSocket and SSE both need it).
        cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
        Self { resolver, info: Arc::default(), last_error: Arc::default(), server: Arc::new(cfg) }
    }
    /// Is there a certificate to serve with?
    pub fn active(&self) -> bool {
        self.resolver.key.read().unwrap_or_else(std::sync::PoisonError::into_inner).is_some()
    }
    pub fn info(&self) -> Option<CertInfo> {
        self.info.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }
    pub fn last_error(&self) -> Option<String> {
        self.last_error.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }
    pub fn set_error(&self, e: Option<String>) {
        *self.last_error.write().unwrap_or_else(std::sync::PoisonError::into_inner) = e;
    }
    /// Start serving with `key`.
    pub fn install(&self, key: Arc<CertifiedKey>, info: CertInfo) {
        *self.resolver.key.write().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(key);
        *self.info.write().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(info);
        self.set_error(None);
    }
    /// Stop serving HTTPS (`kora panel tls off`).
    pub fn clear(&self) {
        *self.resolver.key.write().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        *self.info.write().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }
    pub fn acceptor(&self) -> tokio_rustls::TlsAcceptor {
        tokio_rustls::TlsAcceptor::from(self.server.clone())
    }
    /// `Strict-Transport-Security` applies for the certificates that earn it.
    pub fn hsts(&self) -> bool {
        self.info().is_some_and(|i| i.source.hsts())
    }
}

/// The loud warning for a listener that is reachable from other computers and has no certificate.
pub fn plain_http_warning(addr: &str) -> String {
    format!(
        "WARNING: {addr} is reachable from other computers and has NO HTTPS certificate. The session cookie, sign-in links and everything \
you type (licence keys) cross the network unencrypted. Fix it with `kora panel tls self-signed` (then trust it once, the command shows how), \
`kora panel tls set --cert <file> --key <file>`, or use Tailscale or an SSH tunnel."
    )
}

/// The System check line for HTTPS. `exposed`: the panel listens on an address other computers can reach (outside loopback and
/// outside the tailnet), where plain HTTP is a real risk.
pub fn check_item(info: Option<&CertInfo>, error: Option<&str>, exposed: bool, now: i64) -> kora_core::api::toolchain::CheckItem {
    use kora_core::api::toolchain::{CheckItem, CheckState};
    use kora_core::problem::{Action, Problem};
    let item = |state, summary: String, problem: Option<Problem>| CheckItem { id: "https", label: "HTTPS", state, summary, problem };
    match info {
        Some(i) => {
            let days = certs::days_left(i.not_after, now);
            if days <= 14 {
                let fix = match i.source {
                    CertSource::Provided => "Replace the certificate files; the panel reads them again by itself.",
                    _ => "Kora renews it by itself; if it does not, run `kora panel tls renew`.",
                };
                let p = Problem::new("tls.expiring", format!("The panel's HTTPS certificate {}. {fix}", if days < 0 { "has expired".to_string() } else { format!("ends in {days} days") }))
                    .with_action(Action::show_command(&["kora", "panel", "tls", "renew"], false));
                item(CheckState::Problem, format!("{} ({})", if days < 0 { "Certificate expired" } else { "Certificate ending soon" }, i.source.label()), Some(p))
            } else {
                item(CheckState::Ok, format!("On, with {} ({days} days left).", i.source.label()), None)
            }
        }
        None if exposed => {
            let msg = error.map_or(String::new(), |e| format!(" ({e})"));
            let p = Problem::new(
                "tls.missing",
                format!("The panel can be reached from other computers over plain HTTP{msg}: the session cookie, sign-in links and licence keys cross the network unencrypted. Set up HTTPS."),
            )
            .with_action(Action::show_command(&["kora", "panel", "tls", "self-signed"], false));
            item(CheckState::Problem, "Plain HTTP on a reachable address".into(), Some(p))
        }
        None => item(CheckState::Info, "Plain HTTP on this computer (or inside Tailscale): nothing to do.".into(), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(source: CertSource, days: i64) -> CertInfo {
        CertInfo { source, names: vec![], not_before: 0, not_after: days * 86_400, fingerprint: String::new(), ca_fingerprint: None }
    }

    #[test]
    fn the_system_check_line_follows_the_state() {
        use kora_core::api::toolchain::CheckState;
        let ok = check_item(Some(&info(CertSource::Tailscale, 80)), None, true, 0);
        assert!(ok.state == CheckState::Ok && ok.summary.contains("80 days left") && ok.problem.is_none(), "{ok:?}");
        let soon = check_item(Some(&info(CertSource::SelfSigned, 9)), None, true, 0);
        assert!(soon.state == CheckState::Problem && soon.problem.unwrap().message.contains("ends in 9 days"));
        let gone = check_item(Some(&info(CertSource::Provided, -2)), None, true, 0);
        assert!(gone.problem.unwrap().message.contains("has expired"));
        let plain = check_item(None, None, true, 0);
        assert!(plain.state == CheckState::Problem && plain.problem.unwrap().actions.iter().any(|a| format!("{a:?}").contains("self-signed")));
        let local = check_item(None, None, false, 0);
        assert!(local.state == CheckState::Info && local.problem.is_none());
        let err = check_item(None, Some("the key file is readable by others"), true, 0);
        assert!(err.problem.unwrap().message.contains("readable by others"));
    }

    #[test]
    fn a_state_without_a_certificate_serves_nothing() {
        let s = TlsState::new();
        assert!(!s.active() && s.info().is_none() && !s.hsts());
        assert!(plain_http_warning("0.0.0.0:47120").contains("NO HTTPS"));
    }

    #[test]
    fn hsts_is_for_chosen_certificates_only() {
        assert!(CertSource::Provided.hsts() && CertSource::Tailscale.hsts() && !CertSource::SelfSigned.hsts());
    }
}
