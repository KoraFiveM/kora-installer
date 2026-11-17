//! Reading certificates and keys: what a certificate says (names, dates, fingerprint), whether a key
//! belongs to it, and whether the key file is private enough. No cryptography is done here: parsing is
//! `x509-parser` and `rustls-pki-types`, signing keys are `rustls`'s (ring), fingerprints are SHA-256.
use std::path::Path;
use std::sync::Arc;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::sign::CertifiedKey;
use sha2::{Digest, Sha256};

/// What a certificate says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertSummary {
    /// Every DNS name and IP address in the certificate (the Subject Alternative Names).
    pub names: Vec<String>,
    /// Unix seconds.
    pub not_before: i64,
    pub not_after: i64,
    /// SHA-256 of the certificate, upper-case hex pairs separated by colons.
    pub fingerprint: String,
    /// The subject's common name, when there is one.
    pub subject: String,
}

/// `AB:CD:...` for the SHA-256 of `der`.
pub fn fingerprint(der: &[u8]) -> String {
    let d = Sha256::digest(der);
    d.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":")
}

pub fn parse_chain(pem: &[u8]) -> Result<Vec<CertificateDer<'static>>, String> {
    let chain: Result<Vec<_>, _> = CertificateDer::pem_slice_iter(pem).collect();
    let chain = chain.map_err(|e| format!("the certificate file is not a PEM certificate ({e})"))?;
    if chain.is_empty() {
        return Err("the certificate file holds no certificate (it should start with -----BEGIN CERTIFICATE-----)".into());
    }
    Ok(chain)
}

pub fn parse_key(pem: &[u8]) -> Result<PrivateKeyDer<'static>, String> {
    PrivateKeyDer::from_pem_slice(pem).map_err(|_| "the key file holds no private key in PEM form (it should start with -----BEGIN PRIVATE KEY-----, or EC/RSA PRIVATE KEY)".to_string())
}

pub fn summarise(der: &[u8]) -> Result<CertSummary, String> {
    use x509_parser::extensions::GeneralName;
    use x509_parser::prelude::*;
    let (_, cert) = X509Certificate::from_der(der).map_err(|e| format!("the certificate cannot be read ({e})"))?;
    let mut names = Vec::new();
    if let Ok(Some(san)) = cert.subject_alternative_name() {
        for n in &san.value.general_names {
            match n {
                GeneralName::DNSName(d) => names.push((*d).to_string()),
                GeneralName::IPAddress(b) => match b.len() {
                    4 => names.push(std::net::Ipv4Addr::new(b[0], b[1], b[2], b[3]).to_string()),
                    16 => {
                        let mut a = [0u8; 16];
                        a.copy_from_slice(b);
                        names.push(std::net::Ipv6Addr::from(a).to_string());
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }
    let subject = cert.subject().iter_common_name().next().and_then(|cn| cn.as_str().ok()).unwrap_or("").to_string();
    Ok(CertSummary { names, not_before: cert.validity().not_before.timestamp(), not_after: cert.validity().not_after.timestamp(), fingerprint: fingerprint(der), subject })
}

/// A rustls signing key for `chain` + `key`, refused when the key is not the certificate's own.
pub fn certified_key(chain: Vec<CertificateDer<'static>>, key: &PrivateKeyDer<'static>) -> Result<Arc<CertifiedKey>, String> {
    let signing = rustls::crypto::ring::sign::any_supported_type(key).map_err(|_| "this kind of private key is not supported (use an ECDSA P-256/P-384 or RSA key)".to_string())?;
    let ck = CertifiedKey::new(chain, signing);
    ck.keys_match().map_err(|_| "the private key does not belong to this certificate".to_string())?;
    Ok(Arc::new(ck))
}

/// Does `name` (a host name or address, as typed in a browser, with no port) appear in the certificate's names?
/// Wildcards cover one label (`*.example.com` covers `a.example.com`, not `a.b.example.com`).
pub fn covers(names: &[String], name: &str) -> bool {
    let name = name.trim_matches(|c| c == '[' || c == ']').to_ascii_lowercase();
    names.iter().any(|n| {
        let n = n.to_ascii_lowercase();
        if n == name {
            return true;
        }
        match n.strip_prefix("*.") {
            Some(rest) => name.split_once('.').is_some_and(|(first, tail)| !first.is_empty() && tail == rest),
            None => false,
        }
    })
}

/// Days from `now` (unix seconds) to `not_after`; negative once expired.
pub fn days_left(not_after: i64, now: i64) -> i64 {
    (not_after - now).div_euclid(86_400)
}

/// A private key file must not be readable by other accounts. Pure decision on the Unix mode.
pub fn key_mode_problem(mode: u32) -> Option<String> {
    (mode & 0o077 != 0).then(|| format!("is readable by other accounts (permissions {:03o})", mode & 0o777))
}

/// Refuse a key file that other accounts can read. Windows files are checked by their ACL, which Kora does
/// not read: the folder it lives in should be private to you (the documentation says how).
pub fn check_key_file(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(path).map_err(|e| format!("cannot read the key file {} ({e})", path.display()))?;
        if let Some(why) = key_mode_problem(meta.permissions().mode()) {
            return Err(format!("the key file {} {why}. Make it private with: chmod 600 {}", path.display(), path.display()));
        }
    }
    #[cfg(not(unix))]
    {
        std::fs::metadata(path).map_err(|e| format!("cannot read the key file {} ({e})", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod testcerts {
    /// A throwaway self-signed certificate and key (PEM) for `names`, valid for `days`.
    pub fn make(names: &[&str], days: i64) -> (String, String) {
        let mut p = rcgen::CertificateParams::new(names.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap();
        p.not_before = rcgen::date_time_ymd(2020, 1, 1);
        let end = time::OffsetDateTime::now_utc() + time::Duration::days(days);
        p.not_after = end;
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = p.self_signed(&key).unwrap();
        (cert.pem(), key.serialize_pem())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_certificate_is_read_back() {
        let (cert, key) = testcerts::make(&["box.example.com", "192.168.1.20"], 90);
        let chain = parse_chain(cert.as_bytes()).unwrap();
        let s = summarise(&chain[0]).unwrap();
        assert_eq!(s.names, vec!["box.example.com", "192.168.1.20"]);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        assert!((89..=90).contains(&days_left(s.not_after, now)));
        assert_eq!(s.fingerprint.len(), 32 * 3 - 1);
        let k = parse_key(key.as_bytes()).unwrap();
        assert!(certified_key(chain, &k).is_ok());
    }

    #[test]
    fn a_key_that_is_not_the_certificates_is_refused() {
        let (cert, _) = testcerts::make(&["a.example"], 30);
        let (_, other_key) = testcerts::make(&["b.example"], 30);
        let e = certified_key(parse_chain(cert.as_bytes()).unwrap(), &parse_key(other_key.as_bytes()).unwrap()).err().unwrap();
        assert!(e.contains("does not belong"), "{e}");
    }

    #[test]
    fn garbage_is_explained() {
        assert!(parse_chain(b"hello").unwrap_err().contains("no certificate"));
        assert!(parse_key(b"hello").unwrap_err().contains("no private key"));
    }

    #[test]
    fn names_are_matched_like_a_browser_does() {
        let n = vec!["box.example.com".to_string(), "*.lan.example".to_string(), "10.0.0.5".to_string()];
        assert!(covers(&n, "BOX.example.com") && covers(&n, "a.lan.example") && covers(&n, "10.0.0.5"));
        assert!(!covers(&n, "a.b.lan.example") && !covers(&n, "lan.example") && !covers(&n, "evil.com") && !covers(&n, "10.0.0.6"));
        assert!(covers(&["::1".to_string()], "[::1]"));
    }

    #[test]
    fn key_files_other_accounts_can_read_are_refused() {
        assert!(key_mode_problem(0o600).is_none() && key_mode_problem(0o400).is_none());
        for m in [0o644, 0o640, 0o604, 0o666] {
            assert!(key_mode_problem(m).is_some(), "{m:o}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_world_readable_key_file_names_the_fix() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("k.pem");
        std::fs::write(&p, "x").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        let e = check_key_file(&p).unwrap_err();
        assert!(e.contains("chmod 600"), "{e}");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(check_key_file(&p).is_ok());
    }
}
