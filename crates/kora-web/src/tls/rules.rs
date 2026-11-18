//! The decisions about HTTPS, as small pure functions (tested on every system): which listener serves TLS,
//! what a start with `--allow-public` needs, what a plain-HTTP visitor on the TLS port is told, which scheme
//! an `Origin` must have, and which cookie and HSTS attributes go out.
use std::net::IpAddr;

/// What a listener does about TLS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Listener {
    /// Plain HTTP only (loopback, or HTTPS turned off on purpose).
    Plain,
    /// HTTPS where there is a certificate; other computers' plain HTTP is redirected.
    Tls,
}

/// Which kind a listener on `ip` is. `tls_off`: the person chose `kora panel tls off`. A loopback listener
/// is plain HTTP by default; a Tailscale address is still TLS-capable (a tailnet name gets a real
/// certificate, and passkeys need a secure origin), but needs no warning when it stays plain.
pub fn listener_kind(ip: IpAddr, tls_off: bool) -> Listener {
    if tls_off || ip.to_canonical().is_loopback() {
        Listener::Plain
    } else {
        Listener::Tls
    }
}

/// Is plain HTTP on `ip` a problem worth a loud warning? Not on loopback, and not inside Tailscale (already
/// encrypted end to end). Yes on a LAN or public address.
pub fn plain_is_risky(ip: IpAddr) -> bool {
    let ip = ip.to_canonical();
    !(ip.is_loopback() || crate::remote::is_tailnet_ip(ip))
}

/// `--allow-public` needs HTTPS, or a reverse proxy that Kora was told to trust (which then does the HTTPS).
pub fn allow_public_check(allow_public: bool, bind: IpAddr, has_https: bool, trusted_proxy: bool) -> Result<(), String> {
    let public_or_all = bind.is_unspecified() || !crate::remote::is_private_address(bind);
    if allow_public && public_or_all && !has_https && !trusted_proxy {
        return Err("--allow-public needs HTTPS: there is no certificate yet. Run `kora panel tls self-signed` or `kora panel tls set --cert <file> --key <file>` first, \
or put a reverse proxy in front and tell Kora to trust it with `kora panel proxy trust <address>` (docs/panel.md, \"HTTPS\")."
            .into());
    }
    Ok(())
}

/// The scheme an `Origin` must carry on this connection.
pub fn origin_scheme(secure: bool) -> &'static str {
    if secure {
        "https://"
    } else {
        "http://"
    }
}

/// Is `origin` (`https://host:port`) acceptable for a connection that is `secure` or not? `host_ok` judges the
/// host part.
pub fn origin_ok(origin: &str, secure: bool, host_ok: impl Fn(&str) -> bool) -> bool {
    let scheme = origin_scheme(secure);
    origin.get(..scheme.len()).is_some_and(|p| p.eq_ignore_ascii_case(scheme)) && host_ok(&origin[scheme.len()..])
}

/// The `Set-Cookie` attributes after `name=value`: always `HttpOnly; SameSite=Strict; Path=/`, plus `Secure`
/// over TLS (a `Secure` cookie is not sent back over plain HTTP, which on loopback would lose the session).
pub fn cookie_attrs(secure: bool) -> &'static str {
    if secure {
        "HttpOnly; SameSite=Strict; Path=/; Secure"
    } else {
        "HttpOnly; SameSite=Strict; Path=/"
    }
}

/// One year; no `includeSubDomains` and no `preload`: the panel does not own the domain around its name.
pub const HSTS: &str = "max-age=31536000";

/// Where a plain-HTTP visitor of the TLS port is sent. `host` is the `Host` header (with the port), `target`
/// the request path and query. `None` when either is not a plain value (nothing is echoed back then).
pub fn redirect_location(host: &str, target: &str) -> Option<String> {
    let plain_host = !host.is_empty() && host.len() <= 255 && host.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':' | b'[' | b']' | b'%'));
    let plain_target = target.starts_with('/') && !target.starts_with("//") && target.len() <= 2048 && target.bytes().all(|b| (0x21..0x7f).contains(&b));
    (plain_host && plain_target).then(|| format!("https://{host}{target}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn loopback_stays_plain_and_everything_else_is_https_capable() {
        assert_eq!(listener_kind(ip("127.0.0.1"), false), Listener::Plain);
        assert_eq!(listener_kind(ip("::1"), false), Listener::Plain);
        assert_eq!(listener_kind(ip("::ffff:127.0.0.1"), false), Listener::Plain);
        for a in ["192.168.1.5", "0.0.0.0", "100.64.0.5", "203.0.113.7"] {
            assert_eq!(listener_kind(ip(a), false), Listener::Tls, "{a}");
            assert_eq!(listener_kind(ip(a), true), Listener::Plain, "{a} with TLS off");
        }
    }

    #[test]
    fn plain_http_is_only_risky_outside_loopback_and_the_tailnet() {
        assert!(!plain_is_risky(ip("127.0.0.1")) && !plain_is_risky(ip("100.101.1.1")) && !plain_is_risky(ip("fd7a:115c:a1e0::3")));
        assert!(plain_is_risky(ip("192.168.1.5")) && plain_is_risky(ip("0.0.0.0")) && plain_is_risky(ip("203.0.113.7")));
    }

    #[test]
    fn allow_public_needs_https_or_a_trusted_proxy() {
        let any = ip("0.0.0.0");
        assert!(allow_public_check(true, any, false, false).unwrap_err().contains("needs HTTPS"));
        assert!(allow_public_check(true, any, true, false).is_ok());
        assert!(allow_public_check(true, any, false, true).is_ok());
        assert!(allow_public_check(true, ip("203.0.113.7"), false, false).is_err());
        // Not asking for public: this rule has nothing to say (bind_plan decides).
        assert!(allow_public_check(false, any, false, false).is_ok());
        assert!(allow_public_check(true, ip("192.168.1.5"), false, false).is_ok(), "a private address is not 'public'");
    }

    #[test]
    fn an_origin_must_match_the_connections_scheme() {
        let ok = |h: &str| h == "box.lan:47120";
        assert!(origin_ok("https://box.lan:47120", true, ok));
        assert!(!origin_ok("http://box.lan:47120", true, ok), "downgrade");
        assert!(!origin_ok("https://box.lan:47120", false, ok), "upgrade on a plain connection is not ours either");
        assert!(origin_ok("HTTP://box.lan:47120", false, ok));
        assert!(!origin_ok("https://evil.example", true, ok));
        assert!(!origin_ok("null", true, ok) && !origin_ok("", false, ok));
    }

    #[test]
    fn cookies_are_secure_over_tls_only() {
        assert!(cookie_attrs(true).contains("Secure") && cookie_attrs(true).contains("HttpOnly") && cookie_attrs(true).contains("SameSite=Strict"));
        assert!(!cookie_attrs(false).contains("Secure"));
    }

    #[test]
    fn redirects_echo_only_plain_values() {
        assert_eq!(redirect_location("box.lan:47120", "/install?x=1").unwrap(), "https://box.lan:47120/install?x=1");
        assert_eq!(redirect_location("[fd7a::1]:47120", "/").unwrap(), "https://[fd7a::1]:47120/");
        for bad in [("evil.com/x", "/"), ("a b", "/"), ("box", "//evil.com"), ("box", "x"), ("box", "/a b"), ("", "/"), ("box\r\nSet-Cookie: x", "/")] {
            assert!(redirect_location(bad.0, bad.1).is_none(), "{bad:?}");
        }
    }
}
