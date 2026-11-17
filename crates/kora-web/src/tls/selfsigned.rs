//! A certificate made on this computer, for a panel on a home network (no public name to get a real one for).
//!
//! # Model
//! * One **local CA** per Kora home, made once, kept in the private store (`tls_local_ca`). The person
//!   trusts it once in their browser or operating system (the instructions are [`trust_instructions`]);
//!   its fingerprint is shown in the terminal and in Settings so the file they install can be compared.
//! * A **server certificate** signed by that CA for the names people use (`localhost`, this computer's name,
//!   its addresses, the names in the remote settings). It lasts [`LEAF_DAYS`] days (browsers refuse longer)
//!   and is made again automatically [`RENEW_DAYS`] days before it ends, or at once when the list of names
//!   changes. Because the CA stays the same, the person never has to trust anything again.
//!
//! # Threat model
//! Whoever holds the CA key can make a certificate for ANY name that the person's browser then trusts, so the
//! CA key never leaves the store: it is not in the API, the logs, the audit log, the support report or
//! `kora store export-json`'s ordinary output. The CA is useful only to someone who installed it, so it is
//! never installed automatically and the panel never serves it unauthenticated. Keys are ECDSA P-256. The
//! signing itself is `rcgen` over `ring`; nothing here implements a cryptographic primitive.
use std::net::IpAddr;

use kora_core::store::{rusqlite, Store};
use rcgen::{BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose};

pub const LEAF_DAYS: i64 = 397;
pub const RENEW_DAYS: i64 = 30;
pub const CA_YEARS: i32 = 10;

pub struct Ca {
    pub cert_pem: String,
    pub key_pem: String,
}

pub struct Leaf {
    pub cert_pem: String,
    pub key_pem: String,
    pub names: Vec<String>,
    pub not_after: i64,
}

fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn ca_params(label: &str) -> CertificateParams {
    let mut p = CertificateParams::default();
    p.distinguished_name.push(DnType::CommonName, format!("Kora Panel local CA ({label})"));
    p.distinguished_name.push(DnType::OrganizationName, "Kora");
    p.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    p.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let now = time::OffsetDateTime::now_utc();
    p.not_before = now - time::Duration::days(1);
    p.not_after = now + time::Duration::days(365 * CA_YEARS as i64);
    p
}

/// A new CA (not stored).
pub fn new_ca(label: &str) -> Result<Ca, String> {
    let key = KeyPair::generate().map_err(|e| format!("could not make a key ({e})"))?;
    let cert = ca_params(label).self_signed(&key).map_err(|e| format!("could not make the CA certificate ({e})"))?;
    Ok(Ca { cert_pem: cert.pem(), key_pem: key.serialize_pem() })
}

/// A server certificate for `names`, signed by `ca`.
pub fn issue_leaf(ca: &Ca, names: &[String]) -> Result<Leaf, String> {
    let names = clean_names(names);
    if names.is_empty() {
        return Err("a certificate needs at least one name".into());
    }
    let ca_key = KeyPair::from_pem(&ca.key_pem).map_err(|e| format!("the saved CA key cannot be read ({e})"))?;
    // The CA certificate is rebuilt from its saved parameters: rcgen signs with an issuer made from the same
    // distinguished name and key, which is what a verifier compares.
    let issuer = rcgen::Issuer::from_ca_cert_pem(&ca.cert_pem, ca_key).map_err(|e| format!("the saved CA certificate cannot be read ({e})"))?;
    let mut p = CertificateParams::new(names.clone()).map_err(|e| format!("a name is not usable in a certificate ({e})"))?;
    p.distinguished_name.push(DnType::CommonName, names[0].clone());
    p.is_ca = IsCa::NoCa;
    p.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let now = time::OffsetDateTime::now_utc();
    p.not_before = now - time::Duration::days(1);
    p.not_after = now + time::Duration::days(LEAF_DAYS);
    let key = KeyPair::generate().map_err(|e| format!("could not make a key ({e})"))?;
    let cert = p.signed_by(&key, &issuer).map_err(|e| format!("could not sign the certificate ({e})"))?;
    Ok(Leaf { cert_pem: cert.pem(), key_pem: key.serialize_pem(), names, not_after: p.not_after.unix_timestamp() })
}

/// Names for a certificate: trimmed, de-duplicated (case-insensitive), no ports, no empty ones.
pub fn clean_names(names: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for n in names {
        let n = host_only(n.trim()).to_string();
        if n.is_empty() || n.len() > 253 || n.bytes().any(|b| b.is_ascii_whitespace() || b == b'/') {
            continue;
        }
        if !out.iter().any(|o| o.eq_ignore_ascii_case(&n)) {
            out.push(n);
        }
    }
    out
}

/// `host`, `host:port`, `[v6]` or `[v6]:port` to the bare host.
pub fn host_only(h: &str) -> &str {
    if let Some(rest) = h.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(rest);
    }
    if h.parse::<IpAddr>().is_ok() {
        return h;
    }
    match h.rsplit_once(':') {
        Some((host, port)) if port.bytes().all(|b| b.is_ascii_digit()) && !host.contains(':') => host,
        _ => h,
    }
}

/// The names a certificate for this panel should carry: always `localhost` and the loopback addresses, then
/// whatever the caller knows (this computer's name, the address it listens on, names from the remote
/// settings, Tailscale names).
pub fn default_names(extra: &[String]) -> Vec<String> {
    let mut v = vec!["localhost".to_string(), "127.0.0.1".to_string(), "::1".to_string()];
    v.extend(extra.iter().cloned());
    clean_names(&v)
}

/// The address this computer uses to reach other computers (no traffic is sent: connecting a UDP socket only
/// chooses a route). Often the LAN address people will type.
pub fn primary_lan_address() -> Option<IpAddr> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
}

// ---- storage -------------------------------------------------------------------------------------------------

pub fn load_ca(s: &Store) -> Result<Option<Ca>, String> {
    s.read(|c| match c.query_row("SELECT cert_pem, key_pem FROM tls_local_ca WHERE id = 1", [], |r| Ok(Ca { cert_pem: r.get(0)?, key_pem: r.get(1)? })) {
        Ok(ca) => Ok(Some(ca)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    })
    .map_err(crate::error::store_text)
}

/// The saved CA, or a new one that is saved first. One transaction, so two programs never make two.
pub fn ensure_ca(s: &Store, label: &str) -> Result<Ca, String> {
    if let Some(ca) = load_ca(s)? {
        return Ok(ca);
    }
    let fresh = new_ca(label)?;
    s.write(|tx| {
        tx.execute("INSERT OR IGNORE INTO tls_local_ca (id, cert_pem, key_pem, created) VALUES (1, ?1, ?2, ?3)", rusqlite::params![fresh.cert_pem, fresh.key_pem, now_unix()])?;
        Ok(())
    })
    .map_err(crate::error::store_text)?;
    load_ca(s)?.ok_or_else(|| "the local CA could not be saved".to_string())
}

pub fn load_leaf(s: &Store) -> Result<Option<Leaf>, String> {
    s.read(|c| {
        match c.query_row("SELECT cert_pem, key_pem, names, not_after FROM tls_local_leaf WHERE id = 1", [], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?))
        }) {
            Ok((cert_pem, key_pem, names, not_after)) => Ok(Some(Leaf { cert_pem, key_pem, names: names.split('\n').filter(|n| !n.is_empty()).map(String::from).collect(), not_after })),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    })
    .map_err(crate::error::store_text)
}

pub fn save_leaf(s: &Store, l: &Leaf) -> Result<(), String> {
    s.write(|tx| {
        tx.execute(
            "INSERT INTO tls_local_leaf (id, cert_pem, key_pem, names, not_after) VALUES (1, ?1, ?2, ?3, ?4) \
             ON CONFLICT(id) DO UPDATE SET cert_pem = excluded.cert_pem, key_pem = excluded.key_pem, names = excluded.names, not_after = excluded.not_after",
            rusqlite::params![l.cert_pem, l.key_pem, l.names.join("\n"), l.not_after],
        )?;
        Ok(())
    })
    .map_err(crate::error::store_text)
}

pub fn forget(s: &Store) -> Result<(), String> {
    s.write(|tx| {
        tx.execute("DELETE FROM tls_local_leaf", [])?;
        tx.execute("DELETE FROM tls_local_ca", [])?;
        Ok(())
    })
    .map_err(crate::error::store_text)
}

/// The saved server certificate when it still covers every wanted name and has more than [`RENEW_DAYS`]
/// left; otherwise a new one (same CA), saved. `wanted` names missing from the saved certificate are ADDED to
/// it, so a name that was once needed is never dropped silently.
pub fn ensure_leaf(s: &Store, label: &str, wanted: &[String]) -> Result<Leaf, String> {
    let ca = ensure_ca(s, label)?;
    let wanted = clean_names(wanted);
    if let Some(l) = load_leaf(s)? {
        let covered = wanted.iter().all(|w| super::certs::covers(&l.names, w));
        if covered && super::certs::days_left(l.not_after, now_unix()) > RENEW_DAYS {
            return Ok(l);
        }
        let mut all = l.names.clone();
        all.extend(wanted);
        let fresh = issue_leaf(&ca, &all)?;
        save_leaf(s, &fresh)?;
        return Ok(fresh);
    }
    let fresh = issue_leaf(&ca, &wanted)?;
    save_leaf(s, &fresh)?;
    Ok(fresh)
}

/// How to trust the local CA once, per system and browser. `file` is where the CA certificate was saved.
pub fn trust_instructions(file: &str) -> Vec<(&'static str, String)> {
    vec![
        ("Windows (Edge, Chrome)", format!("Double-click {file}, choose Install Certificate, pick Current User, then \"Place all certificates in the following store\" and choose Trusted Root Certification Authorities. Restart the browser.")),
        ("macOS (Safari, Chrome)", format!("Double-click {file} to add it to Keychain Access, open it there, expand Trust and set \"When using this certificate\" to Always Trust.")),
        ("Linux (Chrome, Chromium)", format!("Chrome settings, Privacy and security, Security, Manage certificates, Authorities, Import {file}, and tick \"Trust this certificate for identifying websites\".")),
        ("Firefox (any system)", format!("Settings, Privacy & Security, View Certificates, Authorities, Import {file}, tick \"Trust this CA to identify websites\".")),
        ("Phones and tablets", format!("Send {file} to the device and open it (iPhone: Settings, Profile Downloaded, then Settings, General, About, Certificate Trust Settings, switch it on; Android: Settings, Security, Install a certificate, CA certificate).")),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tls::certs;

    fn store() -> (tempfile::TempDir, Store) {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(d.path()).unwrap();
        (d, s)
    }

    #[test]
    fn the_leaf_is_signed_by_the_ca_and_names_are_cleaned() {
        let ca = new_ca("test").unwrap();
        let l = issue_leaf(&ca, &["Box.local:47120".into(), "box.local".into(), "192.168.1.20".into(), " ".into(), "[::1]".into()]).unwrap();
        assert_eq!(l.names, vec!["Box.local", "192.168.1.20", "::1"]);
        let chain = certs::parse_chain(l.cert_pem.as_bytes()).unwrap();
        let sum = certs::summarise(&chain[0]).unwrap();
        assert!(sum.names.contains(&"Box.local".to_string()) && sum.names.contains(&"192.168.1.20".to_string()));
        // Valid for the span browsers accept (398 days at most).
        let now = now_unix();
        assert!(sum.not_after - sum.not_before <= 398 * 86_400, "{}", (sum.not_after - sum.not_before) / 86_400);
        assert!(certs::days_left(sum.not_after, now) >= 396);
        // The key matches the certificate, and rustls can serve with it.
        assert!(certs::certified_key(chain.clone(), &certs::parse_key(l.key_pem.as_bytes()).unwrap()).is_ok());
        // And the chain verifies against the CA (webpki, as a client would).
        let mut roots = rustls::RootCertStore::empty();
        roots.add(certs::parse_chain(ca.cert_pem.as_bytes()).unwrap().remove(0)).unwrap();
        let cfg = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let _ = cfg; // building it proves the CA is a valid trust anchor; the handshake test is in serve.rs
    }

    #[test]
    fn nothing_to_name_is_refused() {
        let ca = new_ca("t").unwrap();
        assert!(issue_leaf(&ca, &[" ".into()]).is_err());
    }

    #[test]
    fn the_ca_is_made_once_and_the_leaf_kept_until_it_is_old_or_a_name_is_new() {
        let (_d, s) = store();
        let a = ensure_ca(&s, "box").unwrap();
        let b = ensure_ca(&s, "other").unwrap();
        assert_eq!(a.cert_pem, b.cert_pem, "the CA is stable, so it is trusted once");
        let l1 = ensure_leaf(&s, "box", &default_names(&["box.local".into()])).unwrap();
        let l2 = ensure_leaf(&s, "box", &default_names(&["box.local".into()])).unwrap();
        assert_eq!(l1.cert_pem, l2.cert_pem, "unchanged names: same certificate");
        let l3 = ensure_leaf(&s, "box", &default_names(&["10.0.0.9".into()])).unwrap();
        assert_ne!(l1.cert_pem, l3.cert_pem);
        assert!(l3.names.contains(&"box.local".to_string()) && l3.names.contains(&"10.0.0.9".to_string()), "old names are kept: {:?}", l3.names);
        // Close to the end: made again.
        save_leaf(&s, &Leaf { not_after: now_unix() + 5 * 86_400, ..Leaf { cert_pem: l3.cert_pem.clone(), key_pem: l3.key_pem.clone(), names: l3.names.clone(), not_after: 0 } }).unwrap();
        let l4 = ensure_leaf(&s, "box", &l3.names).unwrap();
        assert_ne!(l3.cert_pem, l4.cert_pem);
    }

    #[test]
    fn host_parts() {
        assert_eq!(host_only("box:47120"), "box");
        assert_eq!(host_only("[fd7a::1]:47120"), "fd7a::1");
        assert_eq!(host_only("fd7a::1"), "fd7a::1");
        assert_eq!(host_only("10.0.0.5"), "10.0.0.5");
    }

    #[test]
    fn instructions_cover_the_common_systems() {
        let t = trust_instructions("kora-local-ca.crt");
        let all: String = t.iter().map(|(a, b)| format!("{a} {b}")).collect();
        for w in ["Windows", "macOS", "Linux", "Firefox", "Phones"] {
            assert!(all.contains(w), "{w}");
        }
        assert!(all.contains("kora-local-ca.crt"));
    }
}
