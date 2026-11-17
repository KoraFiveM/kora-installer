//! How the shell gets a signed-in panel: it runs `kora desktop session --json` (the same decision logic as a
//! bare `kora`: use the panel that runs, else start the background one) and reads ONE line of JSON from it.
//!
//! The line may contain the panel's token (the shell needs it to call the panel's API from Rust). It travels
//! over the child's standard output only: never in a command line, a log, a file or an environment variable of
//! another process. [`Session`] therefore has no `Debug` that prints it.

use serde::Deserialize;

use crate::nav;

/// What `kora desktop session --json` printed.
#[derive(Deserialize, Clone, PartialEq, Eq)]
pub struct Session {
    pub ok: bool,
    /// `running` (it already ran), `started` (set up and started in the background) or `foreground_needed`
    /// (no background service is possible here: the shell must run `kora panel --foreground` itself).
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub port: u16,
    /// `http://127.0.0.1:<port>/#code=...` (a one-time code) to load in the window.
    #[serde(default)]
    pub link: String,
    /// The panel's token (API calls from the shell). Never printed.
    #[serde(default)]
    pub token: String,
    /// A plain sentence when `ok` is false.
    #[serde(default)]
    pub message: String,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("ok", &self.ok)
            .field("mode", &self.mode)
            .field("port", &self.port)
            .field("link", &"<hidden>")
            .field("token", &"<hidden>")
            .field("message", &self.message)
            .finish()
    }
}

/// What to do with the child's output.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Load `link` in the window; call the API on `port` with `token`.
    Ready(Session),
    /// No background panel: run the panel in the foreground, then ask again.
    NeedForeground,
    /// Show this sentence (and offer the browser route or a retry).
    Failed(String),
}

/// Read the output: the LAST non-empty line that is JSON (the CLI may print notes before it).
pub fn parse_output(stdout: &str) -> Outcome {
    let Some(line) = stdout.lines().rev().map(str::trim).find(|l| l.starts_with('{')) else {
        return Outcome::Failed("Kora did not say whether its panel is running.".into());
    };
    let Ok(s) = serde_json::from_str::<Session>(line) else {
        return Outcome::Failed("Kora's answer could not be read. Try again, or update Kora.".into());
    };
    if !s.ok {
        let m = if s.message.trim().is_empty() { "Kora could not start its panel.".to_string() } else { s.message.clone() };
        return Outcome::Failed(m);
    }
    if s.mode == "foreground_needed" {
        return Outcome::NeedForeground;
    }
    if s.port == 0 || s.port < 1024 {
        return Outcome::Failed("Kora's panel gave an unusable port.".into());
    }
    // The link must be the panel's own address with a sign-in part: it is what the window will load.
    let prefix = format!("http://127.0.0.1:{}/", s.port);
    if !s.link.starts_with(&prefix) || nav::decide(&s.link, s.port) != nav::Nav::Allow || !(s.link.contains("#code=") || s.link.contains("#token=")) {
        return Outcome::Failed("Kora's panel gave a link the app will not open.".into());
    }
    Outcome::Ready(s)
}

/// The link opened on `route` (the landing page), keeping the sign-in part.
pub fn link_on_route(link: &str, route: &str) -> String {
    if !nav::safe_route(route) || route == "/" {
        return link.to_string();
    }
    match link.split_once("/#") {
        Some((base, rest)) => format!("{base}{route}#{rest}"),
        None => link.to_string(),
    }
}

/// Wait before asking the panel again: 1, 2, 4, 8, 15, 15 ... seconds.
pub fn backoff_secs(attempt: u32) -> u64 {
    match attempt {
        0 => 1,
        1 => 2,
        2 => 4,
        3 => 8,
        _ => 15,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r##"{"ok":true,"mode":"running","port":47120,"link":"http://127.0.0.1:47120/#code=abcdefghijklmnopqrstuv","token":"tok"}"##;

    #[test]
    fn a_good_answer_is_ready() {
        match parse_output(&format!("Starting ...\n{GOOD}\n")) {
            Outcome::Ready(s) => assert_eq!((s.port, s.mode.as_str(), s.token.as_str()), (47120, "running", "tok")),
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn the_token_and_link_never_show_in_debug_output() {
        let Outcome::Ready(s) = parse_output(GOOD) else { panic!() };
        let d = format!("{s:?}");
        assert!(!d.contains("tok\"") && !d.contains("abcdefgh") && d.contains("<hidden>"), "{d}");
    }

    #[test]
    fn problems_become_sentences() {
        assert_eq!(parse_output(r#"{"ok":false,"message":"This window is running as administrator."}"#), Outcome::Failed("This window is running as administrator.".into()));
        assert!(matches!(parse_output(r#"{"ok":false}"#), Outcome::Failed(m) if m.contains("could not start")));
        assert!(matches!(parse_output(""), Outcome::Failed(_)));
        assert!(matches!(parse_output("{broken"), Outcome::Failed(_)));
        assert_eq!(parse_output(r#"{"ok":true,"mode":"foreground_needed"}"#), Outcome::NeedForeground);
    }

    #[test]
    fn a_link_to_anywhere_but_the_panel_is_refused() {
        for link in [
            "https://evil.example/#code=aaaaaaaaaaaaaaaaaaaa",
            "http://127.0.0.1:47121/#code=aaaaaaaaaaaaaaaaaaaa",
            "http://localhost:47120/#code=aaaaaaaaaaaaaaaaaaaa",
            "http://127.0.0.1:47120/",
            "http://127.0.0.1:47120@evil.example/#code=x",
        ] {
            let out = parse_output(&format!(r#"{{"ok":true,"mode":"running","port":47120,"link":"{link}","token":"t"}}"#));
            assert!(matches!(out, Outcome::Failed(_)), "{link}: {out:?}");
        }
        assert!(matches!(parse_output(r#"{"ok":true,"mode":"running","port":80,"link":"http://127.0.0.1:80/#code=aaaa","token":"t"}"#), Outcome::Failed(_)));
    }

    #[test]
    fn the_link_can_land_on_a_page() {
        assert_eq!(link_on_route("http://127.0.0.1:47120/#code=abc", "/servers/a"), "http://127.0.0.1:47120/servers/a#code=abc");
        assert_eq!(link_on_route("http://127.0.0.1:47120/#code=abc", "/"), "http://127.0.0.1:47120/#code=abc");
        assert_eq!(link_on_route("http://127.0.0.1:47120/#code=abc", "//evil"), "http://127.0.0.1:47120/#code=abc");
    }

    #[test]
    fn waits_grow_then_stop_at_fifteen_seconds() {
        assert_eq!((0..7).map(backoff_secs).collect::<Vec<_>>(), [1, 2, 4, 8, 15, 15, 15]);
    }
}
