//! What passes between the browser and txAdmin, as pure functions (so every rule is a unit test):
//! which headers are dropped, how cookies are renamed so two servers' txAdmin sessions never collide
//! and the panel's own cookies never reach txAdmin, and how absolute addresses are made relative.

use axum::http::{header, HeaderMap, HeaderName, HeaderValue};

/// Headers that describe one hop of the connection, never the message. (RFC 9110 7.6.1, plus the
/// ones HTTP/1.1 proxies drop by habit.) `connection` and `upgrade` are handled on their own.
pub const HOP_BY_HOP: &[&str] = &["connection", "keep-alive", "proxy-authenticate", "proxy-authorization", "proxy-connection", "te", "trailer", "transfer-encoding", "upgrade"];

/// Request headers that must never reach txAdmin: its own machine-to-machine authentication (a request
/// carrying `x-txadmin-token` is treated as FXServer's in-game menu: no session needed), and anything
/// that claims to come from a proxy or from an identity provider.
pub fn request_header_dropped(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    HOP_BY_HOP.contains(&n.as_str())
        || n == "x-txadmin-token"
        || n == "x-txadmin-identifiers"
        || n == "x-txadmin-envtoken"
        || n == "cookie"
        || n == "expect"
        || n == "x-kora-session"
        || n == "x-kora"
        || n == "x-kora-admin"
        || n == "authorization"
        || crate::security::PROXY_HEADERS.contains(&n.as_str())
}

/// Response headers that stay behind: hop-by-hop ones and cookies (rewritten separately).
pub fn response_header_dropped(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    HOP_BY_HOP.contains(&n.as_str()) || n == "set-cookie"
}

/// Names listed in `Connection:` are hop-by-hop too.
pub fn connection_tokens(h: &HeaderMap) -> Vec<String> {
    h.get_all(header::CONNECTION).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(',')).map(|t| t.trim().to_ascii_lowercase()).filter(|t| !t.is_empty()).collect()
}

/// Is this a WebSocket handshake? `Connection` lists `upgrade` and `Upgrade` is `websocket` (case-insensitive).
pub fn is_websocket_upgrade(h: &HeaderMap) -> bool {
    connection_tokens(h).iter().any(|t| t == "upgrade") && h.get_all(header::UPGRADE).iter().filter_map(|v| v.to_str().ok()).any(|v| v.trim().eq_ignore_ascii_case("websocket"))
}

/// Does the request ask for any upgrade at all?
pub fn asks_for_upgrade(h: &HeaderMap) -> bool {
    h.contains_key(header::UPGRADE) || connection_tokens(h).iter().any(|t| t == "upgrade")
}

/// The prefix put in front of every txAdmin cookie name on its way to the browser.
pub fn cookie_prefix(gateway_port: u16) -> String {
    format!("kora-tx-{gateway_port}-")
}

/// The name of the gateway's own session cookie. It never has the txAdmin prefix.
pub fn gate_cookie_name(gateway_port: u16) -> String {
    format!("kora_txgate_{gateway_port}")
}

/// The `Cookie` header to send to txAdmin: only the cookies it set itself (they carry the prefix of
/// this gateway), with the prefix removed. The panel's cookies, the gateway's session cookie and
/// whatever else the browser holds for this host are not passed on.
pub fn cookies_for_txadmin(h: &HeaderMap, gateway_port: u16) -> Option<HeaderValue> {
    let prefix = cookie_prefix(gateway_port);
    let mut out: Vec<String> = Vec::new();
    for v in h.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()) {
        for part in v.split(';') {
            let Some((k, val)) = part.trim().split_once('=') else { continue };
            let Some(real) = k.strip_prefix(&prefix) else { continue };
            // A name or value that could end the header or start another one is not forwarded.
            if real.is_empty() || !real.bytes().all(is_cookie_name_byte) || val.bytes().any(|b| b < 0x20 || b == 0x7f || b == b';') {
                continue;
            }
            out.push(format!("{real}={val}"));
        }
    }
    (!out.is_empty()).then(|| HeaderValue::from_str(&out.join("; ")).ok()).flatten()
}

/// txAdmin names its cookie like `tx:<hash>`: letters, digits and a few separators.
fn is_cookie_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':' | b'.' | b'!' | b'#' | b'$' | b'%' | b'&' | b'\'' | b'*' | b'+' | b'^' | b'`' | b'|' | b'~')
}

/// One `Set-Cookie` of txAdmin's answer, made safe for the browser:
///
/// * the name gets this gateway's prefix (two servers never share a cookie, and nothing txAdmin sets
///   can replace the panel's cookie or the gateway's own);
/// * `Domain` is dropped (the cookie stays on exactly this host and port's host);
/// * `Path` is kept only as a plain absolute path, else `/`;
/// * `Secure` is dropped (the gateway speaks plain HTTP like the panel; a Secure cookie would never come back);
/// * `HttpOnly`, `SameSite`, `Max-Age` and `Expires` are kept; `SameSite=None` becomes `Lax`.
///
/// `None`: not a cookie worth passing on.
pub fn set_cookie_for_browser(raw: &str, gateway_port: u16) -> Option<String> {
    let mut parts = raw.split(';');
    let (name, value) = parts.next()?.trim().split_once('=')?;
    let name = name.trim();
    let value = value.trim();
    if name.is_empty() || !name.bytes().all(is_cookie_name_byte) || value.bytes().any(|b| b < 0x20 || b == 0x7f) {
        return None;
    }
    let mut out = format!("{}{name}={value}", cookie_prefix(gateway_port));
    let mut path = "/".to_string();
    let mut samesite = "Lax".to_string();
    let mut rest: Vec<String> = Vec::new();
    for attr in parts {
        let attr = attr.trim();
        let (k, v) = attr.split_once('=').map_or((attr, ""), |(k, v)| (k.trim(), v.trim()));
        match k.to_ascii_lowercase().as_str() {
            "domain" | "secure" | "partitioned" => {}
            "path" => {
                if v.starts_with('/') && !v.contains(';') && v.bytes().all(|b| (0x20..0x7f).contains(&b)) {
                    path = v.to_string();
                }
            }
            "samesite" => {
                samesite = match v.to_ascii_lowercase().as_str() {
                    "strict" => "Strict".into(),
                    _ => "Lax".into(),
                };
            }
            "httponly" => rest.push("HttpOnly".into()),
            "max-age" if v.parse::<i64>().is_ok() => rest.push(format!("Max-Age={v}")),
            "expires" if v.bytes().all(|b| (0x20..0x7f).contains(&b) && b != b';') => rest.push(format!("Expires={v}")),
            _ => {}
        }
    }
    out.push_str(&format!("; Path={path}; SameSite={samesite}"));
    for r in rest {
        out.push_str("; ");
        out.push_str(&r);
    }
    Some(out)
}

/// Make a `Location` from txAdmin relative to the gateway when it points at txAdmin itself (its
/// loopback address, or the host the browser used with txAdmin's own port); anything else is left as it is.
pub fn location_for_browser(location: &str, txadmin_port: u16, browser_host: &str) -> String {
    let Some(rest) = location.strip_prefix("http://").or_else(|| location.strip_prefix("HTTP://")) else {
        return location.to_string();
    };
    let (authority, path) = match rest.find(['/', '?', '#']) {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (name, port) = split_authority(authority);
    let (bname, _) = split_authority(browser_host);
    let local = matches!(name.to_ascii_lowercase().as_str(), "127.0.0.1" | "localhost" | "[::1]") || name.eq_ignore_ascii_case(bname);
    if local && port == Some(txadmin_port) {
        let p = if path.starts_with('/') { path.to_string() } else { format!("/{path}") };
        // `//host/..` would be read as another host.
        return format!("/{}", p.trim_start_matches('/'));
    }
    location.to_string()
}

fn split_authority(a: &str) -> (&str, Option<u16>) {
    if let Some(end) = a.rfind(']') {
        let port = a[end + 1..].strip_prefix(':').and_then(|p| p.parse().ok());
        return (&a[..=end], port);
    }
    match a.rsplit_once(':') {
        Some((n, p)) => (n, p.parse().ok()),
        None => (a, None),
    }
}

/// Name a header for logging without its value.
pub fn name_of(n: &HeaderName) -> String {
    n.as_str().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hm(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.append(HeaderName::from_bytes(k.as_bytes()).unwrap(), HeaderValue::from_str(v).unwrap());
        }
        h
    }

    #[test]
    fn txadmins_machine_authentication_and_proxy_claims_are_dropped() {
        for n in [
            "X-TxAdmin-Token",
            "x-txadmin-identifiers",
            "x-txadmin-envtoken",
            "Cookie",
            "Authorization",
            "X-Forwarded-For",
            "Forwarded",
            "Tailscale-User-Login",
            "X-Kora-Session",
            "Connection",
            "Transfer-Encoding",
            "Upgrade",
            "TE",
            "Expect",
        ] {
            assert!(request_header_dropped(n), "{n}");
        }
        // The web UI's own CSRF header and ordinary headers pass.
        for n in ["x-txadmin-csrftoken", "Content-Type", "Accept", "User-Agent", "Origin", "Referer", "Host", "Content-Length"] {
            assert!(!request_header_dropped(n), "{n}");
        }
    }

    #[test]
    fn only_txadmins_own_cookies_are_passed_and_unprefixed() {
        let h = hm(&[("cookie", "kora_session_47120=SECRET; kora_txgate_50120=GATE; kora-tx-50120-tx:abc=sess1; kora-tx-50121-tx:abc=other; other=1"), ("cookie", "kora-tx-50120-x=2")]);
        let c = cookies_for_txadmin(&h, 50120).unwrap();
        assert_eq!(c.to_str().unwrap(), "tx:abc=sess1; x=2");
        let none = hm(&[("cookie", "kora_session_47120=SECRET")]);
        assert!(cookies_for_txadmin(&none, 50120).is_none());
    }

    #[test]
    fn cookie_injection_through_a_value_is_refused() {
        let h = hm(&[("cookie", "kora-tx-50120-a=ok; kora-tx-50120-b=x\ty")]);
        // a tab is allowed in no cookie value we forward
        assert_eq!(cookies_for_txadmin(&h, 50120).unwrap().to_str().unwrap(), "a=ok");
    }

    #[test]
    fn set_cookie_is_prefixed_and_scoped() {
        let c = set_cookie_for_browser("tx:abc=123; Path=/; Domain=evil.example; Secure; HttpOnly; SameSite=None; Max-Age=3600", 50120).unwrap();
        assert_eq!(c, "kora-tx-50120-tx:abc=123; Path=/; SameSite=Lax; HttpOnly; Max-Age=3600");
        assert!(!c.to_ascii_lowercase().contains("domain") && !c.to_ascii_lowercase().contains("secure"));
        // Strict stays strict; a weird path becomes /.
        let c = set_cookie_for_browser("a=b; SameSite=Strict; Path=evil", 1).unwrap();
        assert_eq!(c, "kora-tx-1-a=b; Path=/; SameSite=Strict");
        // A cookie cannot take the name of the panel's or the gateway's own.
        let c = set_cookie_for_browser("kora_session_47120=x", 50120).unwrap();
        assert!(c.starts_with("kora-tx-50120-kora_session_47120="));
        assert!(set_cookie_for_browser("noequals", 1).is_none());
        assert!(set_cookie_for_browser("bad name=1", 1).is_none());
        assert!(set_cookie_for_browser("a=b\r\nSet-Cookie: x=y", 1).is_none());
        // Removing a cookie keeps working.
        assert_eq!(set_cookie_for_browser("tx:abc=unset; Max-Age=0; Path=/", 5).unwrap(), "kora-tx-5-tx:abc=unset; Path=/; SameSite=Lax; Max-Age=0");
    }

    #[test]
    fn locations_pointing_at_txadmin_become_relative() {
        assert_eq!(location_for_browser("http://127.0.0.1:40120/login", 40120, "10.0.0.5:50120"), "/login");
        assert_eq!(location_for_browser("http://localhost:40120/", 40120, "10.0.0.5:50120"), "/");
        assert_eq!(location_for_browser("http://10.0.0.5:40120/a?b=1#c", 40120, "10.0.0.5:50120"), "/a?b=1#c");
        assert_eq!(location_for_browser("http://127.0.0.1:40120", 40120, "h:1"), "/");
        // another port, another host, relative ones and https are left alone
        assert_eq!(location_for_browser("http://127.0.0.1:30120/x", 40120, "h:1"), "http://127.0.0.1:30120/x");
        assert_eq!(location_for_browser("http://example.com:40120/x", 40120, "h:1"), "http://example.com:40120/x");
        assert_eq!(location_for_browser("/login", 40120, "h:1"), "/login");
        assert_eq!(location_for_browser("https://forum.cfx.re/x", 40120, "h:1"), "https://forum.cfx.re/x");
        // protocol-relative smuggling: never `//host`
        assert_eq!(location_for_browser("http://127.0.0.1:40120//evil.example/x", 40120, "h:1"), "/evil.example/x");
    }

    #[test]
    fn websocket_handshake_detection() {
        assert!(is_websocket_upgrade(&hm(&[("connection", "keep-alive, Upgrade"), ("upgrade", "WebSocket")])));
        assert!(!is_websocket_upgrade(&hm(&[("connection", "keep-alive")])));
        assert!(!is_websocket_upgrade(&hm(&[("connection", "upgrade"), ("upgrade", "h2c")])));
        assert!(asks_for_upgrade(&hm(&[("connection", "upgrade"), ("upgrade", "h2c")])));
        assert!(!asks_for_upgrade(&hm(&[("accept", "*/*")])));
    }
}
