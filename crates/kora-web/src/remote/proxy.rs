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
    /// How a server's txAdmin is reached through the proxy: a host name template with `{server}` (and optionally `{port}`), like
    /// `tx-{server}.example.com`, when the person gives each server's txAdmin a name of its own in the proxy. Left out: the panel's
    /// own name with the server's gateway port (`example.com:50120`), the proxy listening on that same port. See the module
    /// documentation of [`crate::txgate`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub txadmin_host: Option<String>,
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
        (Self { addresses: addresses.into_iter().map(|a| a.to_canonical()).collect(), honour, salt, hash, txadmin_host: None }, secret)
    }

    /// Is this a usable txAdmin host template? Lower-case letters, digits, `-` and `.`, with `{server}` (required) and `{port}`.
    pub fn valid_txadmin_template(t: &str) -> Result<(), String> {
        let bad = |m: &str| Err(format!("That is not a usable host name template ({m}). Example: tx-{{server}}.example.com"));
        if t.is_empty() || t.len() > 120 {
            return bad("it is empty or too long");
        }
        if !t.contains("{server}") {
            return bad("it must contain {server}, so every server gets its own name");
        }
        let rest = t.replace("{server}", "").replace("{port}", "");
        if rest.contains(['{', '}'])
            || !rest.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.')
            || !(t.starts_with('{') || t.starts_with(|c: char| c.is_ascii_alphanumeric()))
            || !(t.ends_with('}') || t.ends_with(|c: char| c.is_ascii_alphanumeric()))
        {
            return bad("use lower-case letters, digits, '-' and '.', and only {server} and {port} in braces");
        }
        Ok(())
    }

    /// A server's name as a piece of a host name: its short name with `_` as `-`.
    pub fn host_label(slug: &str) -> String {
        slug.replace('_', "-")
    }

    /// The host name the template gives `slug` / the gateway port `port`.
    pub fn txadmin_host_for(&self, slug: &str, port: u16) -> Option<String> {
        self.txadmin_host.as_deref().map(|t| t.replace("{server}", &Self::host_label(slug)).replace("{port}", &port.to_string()))
    }

    /// Does `host` (a `Host` header or an `Origin`'s host, any port) match the template for SOME server? (The server is checked once the
    /// gateway port says which one it is, with [`ProxyConfig::txadmin_host_for`].)
    pub fn matches_txadmin_template(&self, host: &str) -> bool {
        let Some(t) = self.txadmin_host.as_deref() else { return false };
        let name = host.rsplit_once(':').filter(|(_, p)| p.bytes().all(|b| b.is_ascii_digit())).map_or(host, |(n, _)| n).to_ascii_lowercase();
        // Compare around the placeholders: literal pieces in order, anything in the placeholder places.
        let pieces: Vec<&str> = t.split("{server}").flat_map(|p| p.split("{port}")).collect();
        let mut at = 0usize;
        for (i, piece) in pieces.iter().enumerate() {
            match name[at..].find(piece) {
                Some(found) if i > 0 || found == 0 => at += found + piece.len(),
                _ => return false,
            }
        }
        pieces.last().is_some_and(|l| name.ends_with(l))
    }

    /// The address a person opens to reach `slug`'s txAdmin through this proxy: the public scheme the proxy forwarded and either the
    /// server's own host name (template) or the panel's public name with the gateway port.
    pub fn txadmin_origin(&self, secure: bool, panel_host_name: &str, slug: &str, gateway_port: u16) -> String {
        let scheme = if secure { "https" } else { "http" };
        match self.txadmin_host_for(slug, gateway_port) {
            Some(h) => format!("{scheme}://{h}"),
            None => format!("{scheme}://{panel_host_name}:{gateway_port}"),
        }
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

/// Example proxy configuration for ONE server's txAdmin, for Caddy and nginx (the text in `docs/panel.md` is exactly this, checked
/// by a test). txAdmin cannot run under a path prefix, so the proxy gives it a name or a port of its own and forwards to the
/// server's gateway port on the panel's own address (`gateway`, `50120` for txAdmin's usual 40120). `Host` must reach the gateway
/// as the visitor typed it, WebSockets must be passed on, and the same secret as for the panel goes in `X-Kora-Proxy-Secret`.
pub fn txadmin_snippets(secret: &str) -> Vec<(&'static str, String)> {
    vec![
        (
            "Caddy, one host name per server",
            format!("tx-main.example.com {{\n    reverse_proxy 127.0.0.1:50120 {{\n        header_up X-Kora-Proxy-Secret {secret}\n        header_up X-Forwarded-Proto {{scheme}}\n    }}\n}}\n(Caddy passes WebSockets on and adds X-Forwarded-For and Host itself.)"),
        ),
        (
            "Caddy, one port per server",
            format!("example.com:50120 {{\n    reverse_proxy 127.0.0.1:50120 {{\n        header_up X-Kora-Proxy-Secret {secret}\n        header_up X-Forwarded-Proto {{scheme}}\n    }}\n}}"),
        ),
        (
            "nginx, one host name per server",
            format!(
                "server {{\n    listen 443 ssl;\n    server_name tx-main.example.com;\n    # ssl_certificate and ssl_certificate_key as for the panel\n    location / {{\n        proxy_pass http://127.0.0.1:50120;\n        proxy_http_version 1.1;\n        proxy_set_header Host $http_host;\n        proxy_set_header X-Forwarded-For $remote_addr;\n        proxy_set_header X-Forwarded-Proto $scheme;\n        proxy_set_header X-Kora-Proxy-Secret {secret};\n        proxy_set_header Upgrade $http_upgrade;\n        proxy_set_header Connection $connection_upgrade;\n        proxy_buffering off;\n        proxy_read_timeout 1h;\n    }}\n}}"
            ),
        ),
        (
            "nginx, one port per server",
            format!(
                "server {{\n    listen 50120 ssl;\n    server_name example.com;\n    # ssl_certificate and ssl_certificate_key as for the panel\n    location / {{\n        proxy_pass http://127.0.0.1:50120;\n        proxy_http_version 1.1;\n        proxy_set_header Host $http_host;\n        proxy_set_header X-Forwarded-For $remote_addr;\n        proxy_set_header X-Forwarded-Proto $scheme;\n        proxy_set_header X-Kora-Proxy-Secret {secret};\n        proxy_set_header Upgrade $http_upgrade;\n        proxy_set_header Connection $connection_upgrade;\n        proxy_buffering off;\n        proxy_read_timeout 1h;\n    }}\n}}"
            ),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    /// The example configurations in `docs/panel.md` are exactly the ones Kora generates (so the documentation cannot drift from what the
    /// gateway expects), and every one carries what the gateway needs: the secret, the forwarded scheme and address, `Host` as typed,
    /// WebSocket upgrades, and the gateway port of txAdmin's usual 40120.
    #[test]
    fn the_documented_proxy_examples_are_the_ones_kora_generates_and_carry_what_the_gateway_needs() {
        let doc = include_str!("../../../../docs/panel.md");
        let snippets = txadmin_snippets("<secret>");
        assert_eq!(snippets.len(), 4);
        for (name, text) in &snippets {
            assert!(doc.contains(&format!("**{name}**\n\n```\n{text}\n```")), "docs/panel.md must show this example exactly: {name}\n{text}");
            assert!(text.contains("X-Kora-Proxy-Secret <secret>") || text.contains("X-Kora-Proxy-Secret <secret>;"), "{name}: the shared secret");
            assert!(text.contains("127.0.0.1:50120"), "{name}: the server's gateway port on the panel's address");
            assert!(text.contains("X-Forwarded-Proto"), "{name}: the public scheme");
            if name.starts_with("nginx") {
                assert!(
                    text.contains("Host $http_host") && text.contains("X-Forwarded-For $remote_addr") && text.contains("Upgrade $http_upgrade") && text.contains("Connection $connection_upgrade"),
                    "{name}"
                );
                assert!(!text.contains("Host $host;"), "{name}: $host drops the port the visitor typed");
            }
        }
        assert!(snippets.iter().filter(|(n, _)| n.contains("host name")).all(|(_, t)| t.contains("tx-main.example.com")), "the host-name examples match the template `tx-{{server}}.example.com`");
        assert!(snippets.iter().filter(|(n, _)| n.contains("one port")).all(|(_, t)| t.contains("50120") && t.contains("example.com")));
    }

    #[test]
    fn the_txadmin_host_template_is_checked_filled_in_and_matched() {
        for ok in ["tx-{server}.example.com", "{server}.tx.example.com", "tx-{server}-{port}.example.com", "{server}"] {
            assert!(ProxyConfig::valid_txadmin_template(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "example.com",
            "tx-{name}.example.com",
            "TX-{server}.example.com",
            "tx-{server}.example.com/",
            "tx-{server}.example.com:8443",
            "-{server}.x",
            "{server}.",
            "a b-{server}",
            "{{server}}",
        ] {
            assert!(ProxyConfig::valid_txadmin_template(bad).is_err(), "{bad}");
        }
        let (mut c, _) = ProxyConfig::new(vec!["127.0.0.1".parse().unwrap()], vec![]);
        assert_eq!(c.txadmin_host_for("main", 50120), None);
        assert_eq!(c.txadmin_origin(true, "panel.example.com", "main", 50120), "https://panel.example.com:50120", "no template: the panel's name and the gateway port");
        c.txadmin_host = Some("tx-{server}.example.com".into());
        assert_eq!(c.txadmin_host_for("my_server", 50120).as_deref(), Some("tx-my-server.example.com"));
        assert_eq!(c.txadmin_origin(true, "panel.example.com", "main", 50120), "https://tx-main.example.com");
        assert_eq!(c.txadmin_origin(false, "panel.example.com", "main", 50120), "http://tx-main.example.com");
        assert!(c.matches_txadmin_template("tx-main.example.com") && c.matches_txadmin_template("TX-Dev.example.com:8443"));
        for no in ["example.com", "tx-.example.com.evil.net", "evil-tx-main.example.com", "tx-main.example.org", "tx-main.example.com.evil.net"] {
            assert!(!c.matches_txadmin_template(no), "{no}");
        }
        // an old saved configuration has no template and still reads
        let old = r#"{"addresses":["127.0.0.1"],"honour":["for","proto"],"salt":"s","hash":"h"}"#;
        assert_eq!(serde_json::from_str::<ProxyConfig>(old).unwrap().txadmin_host, None);
    }

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
