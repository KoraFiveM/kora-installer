//! Where the window may go: the navigation allow-list, and the deep-link routes.
//!
//! # Threat model
//!
//! The window shows ONE origin: the panel on `http://127.0.0.1:<its port>`. The panel is first-party code served
//! by our own process, and it is the only thing that is ever given the shell's (tiny, see [`crate::bridge`])
//! privileges. Everything else must never run inside the window, because a page in it could talk to the shell:
//!
//! * a link to any other `http(s)` address (a forum, GitHub, a server's txAdmin on another port) opens in the
//!   person's default browser ([`Nav::External`]) and the window stays where it was;
//! * every other scheme (`file:`, `javascript:`, `data:`, `ftp:`, custom handlers, ...) is refused ([`Nav::Deny`]);
//! * `localhost` and `[::1]` are NOT the panel's origin (a different origin to the browser, and another program
//!   may own that name's port): only the literal `127.0.0.1` and the panel's own port count;
//! * a user-info part (`http://127.0.0.1:47120@evil.example/`), a backslash, a control character or a missing
//!   port is refused, because those are how an address is made to look like the panel's;
//! * the address is parsed here by plain string rules (no URL library), strictly: whatever is not clearly
//!   the panel's is treated as foreign.
//!
//! Pages may also arrive from the shell itself (a notification click, the tray menu, a second launch): those carry
//! a ROUTE, never an address, and [`safe_route`] only lets a plain in-panel path through.

/// What to do with an address the window is asked to load or open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Nav {
    /// The panel's own origin: show it in the window.
    Allow,
    /// Somewhere else on the web: hand it to the default browser (the string is the address, unchanged).
    External(String),
    /// Not allowed anywhere.
    Deny,
}

/// The parts of an `http(s)` address that matter here.
struct Parts<'a> {
    scheme: &'a str,
    host: &'a str,
    port: Option<u16>,
}

fn parse(url: &str) -> Option<Parts<'_>> {
    if url.len() > 4096 || url.chars().any(|c| c.is_control() || c == '\\' || c == ' ') {
        return None;
    }
    let (scheme, rest) = url.split_once("://")?;
    if !matches!(scheme, "http" | "https") {
        return None;
    }
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    // `[::1]:80` is the only bracketed form worth telling apart; it is never the panel's origin.
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if !authority.ends_with(']') => (h, Some(p.parse::<u16>().ok()?)),
        _ => (authority, None),
    };
    if host.is_empty() {
        return None;
    }
    Some(Parts { scheme, host, port })
}

/// Decide for `url`, with the panel on `panel_port`.
pub fn decide(url: &str, panel_port: u16) -> Nav {
    if url == "about:blank" {
        return Nav::Allow;
    }
    // A download the page made from its own data: `blob:http://127.0.0.1:<port>/<uuid>`.
    if let Some(inner) = url.strip_prefix("blob:") {
        return match parse(inner) {
            Some(p) if is_panel(&p, panel_port) => Nav::Allow,
            _ => Nav::Deny,
        };
    }
    match parse(url) {
        Some(p) if is_panel(&p, panel_port) => Nav::Allow,
        Some(_) => Nav::External(url.to_string()),
        None => Nav::Deny,
    }
}

fn is_panel(p: &Parts<'_>, panel_port: u16) -> bool {
    p.scheme == "http" && p.host == "127.0.0.1" && p.port == Some(panel_port)
}

/// Is this a plain in-panel path (`/`, `/servers/my%20server`, `/settings?tab=notifications`) that may be put after
/// the panel's origin? Anything that could change the host (`//evil`, `/\evil`, `http:`) is not.
pub fn safe_route(route: &str) -> bool {
    route.len() <= 512
        && route.starts_with('/')
        && !route.starts_with("//")
        && !route.contains("..")
        && !route.contains("://")
        && route.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b'~' | b'%' | b'?' | b'=' | b'&' | b'#' | b'+'))
}

/// `route` if it is safe, else the start page.
pub fn route_or_home(route: Option<&str>) -> String {
    match route {
        Some(r) if safe_route(r) => r.to_string(),
        _ => "/".to_string(),
    }
}

/// Percent-encode one path segment (a server name).
pub fn encode_segment(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The page of one server.
pub fn server_route(name: &str) -> String {
    format!("/servers/{}", encode_segment(name))
}

/// The address to load in the window for `route` on the panel at `port`.
pub fn panel_url(port: u16, route: &str) -> String {
    format!("http://127.0.0.1:{port}{}", route_or_home(Some(route)))
}

/// Is a bridge request from `uri` (the page that sent it) allowed to be answered at all?
pub fn trusted_page(uri: &str, panel_port: u16) -> bool {
    matches!(decide(uri, panel_port), Nav::Allow) && uri != "about:blank"
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: u16 = 47120;

    #[test]
    fn only_the_panels_own_origin_stays_in_the_window() {
        for ok in [
            "http://127.0.0.1:47120/",
            "http://127.0.0.1:47120",
            "http://127.0.0.1:47120/servers/a%20b?x=1#frag",
            "http://127.0.0.1:47120/#code=abc",
            "about:blank",
            "blob:http://127.0.0.1:47120/5e1f-aa",
        ] {
            assert_eq!(decide(ok, P), Nav::Allow, "{ok}");
        }
    }

    #[test]
    fn other_web_addresses_go_to_the_browser_and_never_into_the_window() {
        for ext in [
            "https://github.com/KoraFiveM/kora",
            "http://127.0.0.1:47121/",
            "http://127.0.0.1/",
            "http://localhost:47120/",
            "http://[::1]:47120/",
            "https://127.0.0.1:47120/",
            "http://127.0.0.1.evil.example:47120/",
            "http://localhost:40120/",
            "http://192.168.1.5:47120/",
        ] {
            assert_eq!(decide(ext, P), Nav::External(ext.to_string()), "{ext}");
        }
    }

    #[test]
    fn disguised_and_dangerous_addresses_are_refused() {
        for bad in [
            "http://127.0.0.1:47120@evil.example/",
            "http://evil.example@127.0.0.1:47120/",
            "http://127.0.0.1:47120\\@evil.example/",
            "http://127.0.0.1:47120 .evil/",
            "javascript:alert(1)",
            "data:text/html,<script>1</script>",
            "file:///C:/Windows/System32/cmd.exe",
            "ftp://127.0.0.1:47120/",
            "ms-msdt:/id",
            "vbscript:x",
            "blob:https://evil.example/uuid",
            "blob:http://127.0.0.1:1/uuid",
            "http://",
            "http://:47120/",
            "http://127.0.0.1:notaport/",
            "http://127.0.0.1:47120\n/",
            "",
            "//evil.example/",
            "/relative",
        ] {
            assert_eq!(decide(bad, P), Nav::Deny, "{bad:?}");
        }
        assert_eq!(decide(&format!("http://127.0.0.1:47120/{}", "a".repeat(5000)), P), Nav::Deny, "absurdly long");
    }

    #[test]
    fn routes_are_plain_in_panel_paths() {
        for ok in ["/", "/install", "/servers/my%20server", "/settings?tab=notifications", "/servers/a-b_c.d"] {
            assert!(safe_route(ok), "{ok}");
        }
        for bad in ["", "install", "//evil.example", "/\\evil", "http://evil", "/a/../b", "/a b", "/<x>", "/x://y", "javascript:1"] {
            assert!(!safe_route(bad), "{bad}");
        }
        assert_eq!(route_or_home(Some("//evil")), "/");
        assert_eq!(route_or_home(None), "/");
        assert_eq!(route_or_home(Some("/install")), "/install");
        assert_eq!(panel_url(P, "/servers/x"), "http://127.0.0.1:47120/servers/x");
        assert_eq!(panel_url(P, "//evil"), "http://127.0.0.1:47120/");
    }

    #[test]
    fn server_names_are_encoded_into_one_segment() {
        assert_eq!(server_route("my server"), "/servers/my%20server");
        assert_eq!(server_route("a/b?c#d"), "/servers/a%2Fb%3Fc%23d");
        assert!(safe_route(&server_route("../../etc ü")));
        assert_eq!(server_route("a.b"), "/servers/a%2Eb", "dots are encoded so that a name can never look like ..");
        assert_eq!(decide(&panel_url(P, &server_route("x y")), P), Nav::Allow);
    }

    #[test]
    fn a_bridge_request_must_come_from_the_panel_page() {
        assert!(trusted_page("http://127.0.0.1:47120/servers/x", P));
        assert!(!trusted_page("https://github.com/", P));
        assert!(!trusted_page("about:blank", P));
        assert!(!trusted_page("http://127.0.0.1:47121/", P));
    }
}
