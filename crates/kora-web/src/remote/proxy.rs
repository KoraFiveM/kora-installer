//! An explicit, opt-in trusted reverse proxy.
//!
//! # The problem
//! Behind a reverse proxy every visitor reaches the panel from the proxy's own address (usually 127.0.0.1),
//! which the panel would take for the computer's owner. So requests that carry forwarded headers are refused
//! outright ([`crate::security::PROXY_HEADERS`]). That is right by default and wrong for the person who
//! deliberately runs Caddy or nginx in front of the panel to get a public name and a Let's Encrypt
//! certificate (Kora does not do ACME itself).
//!
//! # What this adds, for that configured case only
//! `kora panel proxy trust <address>` saves a [`ProxyConfig`]: the proxy's address(es), which forwarded
//! headers are honoured, and a **shared secret** the proxy must add to every request (`X-Kora-Proxy-Secret`).
//! A request counts as proxied only when BOTH hold: it comes from a listed address, AND it carries the secret
//! (compared in constant time against a stored hash). Then:
//!
//! * the client address is the forwarded one (`X-Forwarded-For`: the right-most entry that is not itself a
//!   listed proxy, so a client cannot choose its own address by sending the header first), and that address
//!   is what the sign-in lockout and the "this computer" checks use;
//! * `X-Forwarded-Proto: https` makes the request count as secure (cookies get `Secure`, an `Origin` must be
//!   `https://`); anything else is plain;
//! * the caller is ALWAYS remote, even if the forwarded address is a loopback one: a proxied visitor is never
//!   the person at the keyboard, so the launch token and local-only operations are never reachable this way;
//! * a Tailscale identity is never taken from a proxied request (the forwarded address is a claim, not a
//!   connection tailscaled can vouch for), and `Tailscale-User-*` headers are refused outright.
//!
//! Without the secret, or from an address that is not listed, nothing changes: forwarded headers still mean
//! "refused". The secret exists because the proxy address alone is not enough when the proxy runs on the same
//! computer (a local program could connect from 127.0.0.1 and claim any client address).
//!
//! The secret is shown once when set, kept only as a salted hash, and rotated with `kora panel proxy trust`
//! again. Turn it off with `kora panel proxy forget`.
use std::net::IpAddr;

use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};

use crate::security::{ct_eq, random_hex};

/// The header the proxy adds with the shared secret.
pub const SECRET_HEADER: &str = "x-kora-proxy-secret";

/// Forwarded headers that can be honoured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Honour {
    /// `X-Forwarded-For`: the client's address.
    For,
    /// `X-Forwarded-Proto`: `https` makes the request secure.
    Proto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyConfig {
    pub addresses: Vec<IpAddr>,
    pub honour: Vec<Honour>,
    pub salt: String,
    /// `sha256(salt || secret)`, hex.
    pub hash: String,
}

/// What the panel concluded about a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Not from a trusted proxy: the ordinary rules (including "proxy headers are refused").
    NotProxied,
    /// From the trusted proxy: this is the visitor.
    Trusted { client: IpAddr, secure: bool },
    /// From the trusted proxy but unusable; the text says what to fix.
    Refused(&'static str),
}

impl ProxyConfig {
    /// A new configuration and the secret (shown once, never stored).
    pub fn new(addresses: Vec<IpAddr>, honour: Vec<Honour>) -> (Self, String) {
        let secret = random_hex();
        let salt = random_hex();
        let hash = crate::remote::config::hash_token(&salt, &secret);
        let honour = if honour.is_empty() { vec![Honour::For, Honour::Proto] } else { honour };
        (Self { addresses: addresses.into_iter().map(|a| a.to_canonical()).collect(), honour, salt, hash }, secret)
    }

    fn lists(&self, ip: IpAddr) -> bool {
        let ip = ip.to_canonical();
        self.addresses.contains(&ip)
    }

    pub fn secret_ok(&self, given: &str) -> bool {
        ct_eq(&crate::remote::config::hash_token(&self.salt, given), &self.hash)
    }

    /// Judge a request from `peer` with these `headers`.
    pub fn judge(&self, peer: IpAddr, headers: &HeaderMap) -> Verdict {
        if !self.lists(peer) {
            return Verdict::NotProxied;
        }
        let given = headers.get(SECRET_HEADER).and_then(|v| v.to_str().ok());
        let Some(given) = given else { return Verdict::NotProxied };
        if !self.secret_ok(given.trim()) {
            return Verdict::Refused("The proxy secret is wrong. Copy it again from `kora panel proxy trust` into the proxy's configuration.");
        }
        if headers.contains_key("tailscale-user-login") || headers.contains_key("tailscale-user-name") {
            return Verdict::Refused("Tailscale identity headers are not accepted through a proxy. Use a passkey to sign in.");
        }
        let mut client = None;
        if self.honour.contains(&Honour::For) {
            // Header values joined by commas; the right-most entry that is not one of our proxies.
            let all: Vec<&str> = headers.get_all("x-forwarded-for").iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(',')).map(str::trim).collect();
            for entry in all.iter().rev() {
                match parse_forwarded_ip(entry) {
                    Some(ip) if self.lists(ip) => continue,
                    Some(ip) => {
                        client = Some(ip);
                        break;
                    }
                    None => return Verdict::Refused("The proxy sent an X-Forwarded-For that is not an address."),
                }
            }
        }
        let Some(client) = client else {
            return Verdict::Refused("The proxy must send X-Forwarded-For with the visitor's address (nginx: proxy_set_header X-Forwarded-For $remote_addr;).");
        };
        let secure = self.honour.contains(&Honour::Proto) && headers.get("x-forwarded-proto").and_then(|v| v.to_str().ok()).is_some_and(|v| v.trim().eq_ignore_ascii_case("https"));
        Verdict::Trusted { client, secure }
    }
}

/// `1.2.3.4`, `1.2.3.4:5678`, `[::1]`, `[::1]:5678` or `::1`.
pub fn parse_forwarded_ip(s: &str) -> Option<IpAddr> {
    let s = s.trim();
    if let Ok(ip) = s.parse::<IpAddr>() {
        return Some(ip);
    }
    if let Some(rest) = s.strip_prefix('[') {
        return rest.split(']').next()?.parse().ok();
    }
    s.rsplit_once(':').and_then(|(h, p)| (p.bytes().all(|b| b.is_ascii_digit()) && !p.is_empty()).then(|| h.parse().ok()).flatten())
}

/// Snippets to paste into the proxy's configuration.
pub fn snippets(secret: &str, panel: &str) -> Vec<(&'static str, String)> {
    vec![
        (
            "Caddy",
            format!("example.com {{\n    reverse_proxy {panel} {{\n        header_up X-Kora-Proxy-Secret {secret}\n        header_up X-Forwarded-Proto {{scheme}}\n    }}\n}}\n(Caddy adds X-Forwarded-For itself.)"),
        ),
        (
            "nginx",
            format!(
                "location / {{\n    proxy_pass http://{panel};\n    proxy_set_header Host $host;\n    proxy_set_header X-Forwarded-For $remote_addr;\n    proxy_set_header X-Forwarded-Proto $scheme;\n    proxy_set_header X-Kora-Proxy-Secret {secret};\n    proxy_http_version 1.1;\n    proxy_set_header Upgrade $http_upgrade;\n    proxy_set_header Connection $connection_upgrade;\n    proxy_buffering off;\n}}"
            ),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn h(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut m = HeaderMap::new();
        for (k, v) in pairs {
            m.append(*k, HeaderValue::from_str(v).unwrap());
        }
        m
    }
    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn cfg() -> (ProxyConfig, String) {
        ProxyConfig::new(vec![ip("127.0.0.1")], vec![])
    }

    #[test]
    fn the_secret_is_stored_hashed_and_never_in_the_config() {
        let (c, secret) = cfg();
        assert!(!serde_json::to_string(&c).unwrap().contains(&secret));
        assert!(c.secret_ok(&secret) && !c.secret_ok("nope") && !c.secret_ok(""));
        let (c2, s2) = cfg();
        assert_ne!(secret, s2);
        assert!(!c2.secret_ok(&secret), "a new configuration has a new secret");
    }

    #[test]
    fn only_the_listed_address_with_the_secret_is_a_proxy() {
        let (c, s) = cfg();
        let good = h(&[(SECRET_HEADER, &s), ("x-forwarded-for", "203.0.113.9"), ("x-forwarded-proto", "https")]);
        assert_eq!(c.judge(ip("127.0.0.1"), &good), Verdict::Trusted { client: ip("203.0.113.9"), secure: true });
        // From another address: not a proxy, whatever the headers say.
        assert_eq!(c.judge(ip("192.168.1.9"), &good), Verdict::NotProxied);
        // No secret: not proxied (so forwarded headers get refused by the ordinary rule).
        assert_eq!(c.judge(ip("127.0.0.1"), &h(&[("x-forwarded-for", "203.0.113.9")])), Verdict::NotProxied);
        // A wrong secret is refused, not silently demoted.
        assert!(matches!(c.judge(ip("127.0.0.1"), &h(&[(SECRET_HEADER, "wrong"), ("x-forwarded-for", "203.0.113.9")])), Verdict::Refused(_)));
        // IPv4-mapped peers match the plain address.
        assert!(matches!(c.judge(ip("::ffff:127.0.0.1"), &good), Verdict::Trusted { .. }));
    }

    #[test]
    fn the_client_cannot_pick_its_own_address() {
        let (c, s) = cfg();
        // The client sent "X-Forwarded-For: 10.0.0.1"; the proxy appended the real address.
        let hd = h(&[(SECRET_HEADER, &s), ("x-forwarded-for", "10.0.0.1, 203.0.113.9")]);
        assert_eq!(c.judge(ip("127.0.0.1"), &hd), Verdict::Trusted { client: ip("203.0.113.9"), secure: false });
        // A chain of our own proxies is skipped.
        let hd = h(&[(SECRET_HEADER, &s), ("x-forwarded-for", "203.0.113.9, 127.0.0.1")]);
        assert_eq!(c.judge(ip("127.0.0.1"), &hd), Verdict::Trusted { client: ip("203.0.113.9"), secure: false });
        // Several header lines are read as one list, from the right.
        let hd = h(&[(SECRET_HEADER, &s), ("x-forwarded-for", "10.9.9.9"), ("x-forwarded-for", "198.51.100.4")]);
        assert_eq!(c.judge(ip("127.0.0.1"), &hd), Verdict::Trusted { client: ip("198.51.100.4"), secure: false });
    }

    #[test]
    fn a_loopback_client_address_is_still_a_remote_visitor() {
        // The verdict only names the client; the guard treats every trusted-proxy request as remote (tested there).
        let (c, s) = cfg();
        let hd = h(&[(SECRET_HEADER, &s), ("x-forwarded-for", "127.0.0.2")]);
        assert_eq!(c.judge(ip("127.0.0.1"), &hd), Verdict::Trusted { client: ip("127.0.0.2"), secure: false });
    }

    #[test]
    fn missing_or_bad_forwarding_is_refused_with_the_fix() {
        let (c, s) = cfg();
        let e = |hd: &HeaderMap| match c.judge(ip("127.0.0.1"), hd) {
            Verdict::Refused(m) => m,
            v => panic!("{v:?}"),
        };
        assert!(e(&h(&[(SECRET_HEADER, &s)])).contains("X-Forwarded-For"));
        assert!(e(&h(&[(SECRET_HEADER, &s), ("x-forwarded-for", "not-an-address")])).contains("not an address"));
        assert!(e(&h(&[(SECRET_HEADER, &s), ("x-forwarded-for", "1.2.3.4"), ("tailscale-user-login", "a@b")])).contains("Tailscale"));
    }

    #[test]
    fn only_the_configured_headers_are_honoured() {
        let (c, s) = ProxyConfig::new(vec![ip("127.0.0.1")], vec![Honour::For]);
        let hd = h(&[(SECRET_HEADER, &s), ("x-forwarded-for", "1.2.3.4"), ("x-forwarded-proto", "https")]);
        assert_eq!(c.judge(ip("127.0.0.1"), &hd), Verdict::Trusted { client: ip("1.2.3.4"), secure: false }, "proto not honoured");
    }

    #[test]
    fn forwarded_addresses_parse_in_every_form() {
        assert_eq!(parse_forwarded_ip("1.2.3.4:5000"), Some(ip("1.2.3.4")));
        assert_eq!(parse_forwarded_ip("[2001:db8::1]:443"), Some(ip("2001:db8::1")));
        assert_eq!(parse_forwarded_ip("2001:db8::1"), Some(ip("2001:db8::1")));
        assert_eq!(parse_forwarded_ip("unknown"), None);
        assert_eq!(parse_forwarded_ip(""), None);
    }

    #[test]
    fn snippets_carry_the_secret_and_the_panel_address() {
        let s = snippets("SECRET", "127.0.0.1:47120");
        assert!(s.iter().all(|(_, t)| t.contains("SECRET") && t.contains("127.0.0.1:47120")));
    }
}
