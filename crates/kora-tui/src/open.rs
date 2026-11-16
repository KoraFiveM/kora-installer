//! Opening a file or folder with the operating system's default program, safely, plus OSC 8
//! terminal hyperlinks. Command building is pure so it can be tested for every OS.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsKind {
    Windows,
    Other,
}

impl OsKind {
    pub fn current() -> Self {
        if cfg!(windows) {
            OsKind::Windows
        } else {
            OsKind::Other
        }
    }
}

/// A program and its arguments. The path is always ONE argument and never goes through a shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenCommand {
    pub program: String,
    pub args: Vec<OsString>,
}

/// Check `target` is absolute, free of `..` and odd characters, and equal to or inside `root`.
pub fn validate_target(target: &Path, root: &Path) -> Result<(), String> {
    let bad = |m: &str| Err(format!("{m}: {}", target.display()));
    let text = target.to_string_lossy();
    if text.chars().any(|c| c.is_control() || c == '"') {
        return bad("That path has characters that can't be opened safely");
    }
    if !target.is_absolute() || !root.is_absolute() {
        return bad("Only full paths can be opened");
    }
    if target.components().any(|c| c == Component::ParentDir) || root.components().any(|c| c == Component::ParentDir) {
        return bad("Paths containing '..' are not opened");
    }
    if !target.starts_with(root) {
        return bad("That path is outside the server folder");
    }
    Ok(())
}

/// Build the command that opens `target` (a file or folder inside `root`).
pub fn open_command(os: OsKind, target: &Path, root: &Path) -> Result<OpenCommand, String> {
    validate_target(target, root)?;
    let (program, args): (&str, Vec<OsString>) = match os {
        // explorer is not a shell, so `&`, spaces and `%` in the path stay literal. (It exits
        // with code 1 even on success, so callers must not treat its exit status as failure.)
        OsKind::Windows => ("explorer.exe", vec![target.as_os_str().to_owned()]),
        OsKind::Other => ("xdg-open", vec![target.as_os_str().to_owned()]),
    };
    Ok(OpenCommand { program: program.into(), args })
}

/// Command that opens a local `http://localhost:<port>/` or `http://127.0.0.1:<port>/` URL in the default browser.
pub fn open_url_command(os: OsKind, url: &str) -> Result<OpenCommand, String> {
    let ok_host = ["http://localhost:", "http://127.0.0.1:", "http://localhost/", "http://127.0.0.1/"].iter().any(|p| url.starts_with(p));
    if !ok_host || url.chars().any(|c| c.is_control() || c.is_whitespace() || c == '"' || c == '&' || c == '^' || c == '|' || c == '<' || c == '>') {
        return Err(format!("Only the local txAdmin address can be opened: {url}"));
    }
    let program = match os {
        OsKind::Windows => "explorer.exe",
        OsKind::Other => "xdg-open",
    };
    Ok(OpenCommand { program: program.into(), args: vec![OsString::from(url)] })
}

/// What the screens use to open things (fake-able in tests).
pub trait Opener {
    fn open(&mut self, target: &Path, root: &Path) -> Result<(), String>;
    /// Open a (local) web address in the default browser.
    fn open_url(&mut self, url: &str) -> Result<(), String> {
        Err(format!("Open this in your browser: {url}"))
    }
}

pub struct SystemOpener;

fn spawn_quiet(cmd: &OpenCommand) -> bool {
    match std::process::Command::new(&cmd.program).args(&cmd.args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn() {
        Ok(mut c) => {
            std::thread::spawn(move || {
                let _ = c.wait();
            });
            true
        }
        Err(_) => false,
    }
}

impl Opener for SystemOpener {
    fn open_url(&mut self, url: &str) -> Result<(), String> {
        let cmd = open_url_command(OsKind::current(), url)?;
        if spawn_quiet(&cmd) {
            Ok(())
        } else {
            Err(format!("Couldn't open the browser automatically. Open this yourself: {url}"))
        }
    }

    fn open(&mut self, target: &Path, root: &Path) -> Result<(), String> {
        let cmd = open_command(OsKind::current(), target, root)?;
        let child = std::process::Command::new(&cmd.program)
            .args(&cmd.args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        match child {
            Ok(mut c) => {
                // Reap it in the background so it doesn't linger as a zombie.
                std::thread::spawn(move || {
                    let _ = c.wait();
                });
                Ok(())
            }
            Err(_) => Err(format!("Couldn't open it automatically. Open this yourself: {}", target.display())),
        }
    }
}

/// Server folder for a log path: `<dir>/.kora/install.log` -> `<dir>`, otherwise the log's folder.
pub fn root_for_log(log: &Path) -> PathBuf {
    let parent = log.parent().unwrap_or(log);
    if parent.file_name().is_some_and(|n| n == ".kora") {
        parent.parent().unwrap_or(parent).to_path_buf()
    } else {
        parent.to_path_buf()
    }
}

/// `file://` URL for a path (percent-encoded), for OSC 8 links.
pub fn file_url(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file://");
    if !s.starts_with('/') {
        out.push('/');
    }
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~:".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Does this terminal understand OSC 8 hyperlinks? Conservative: unknown terminals (and the old
/// Windows console) get no links rather than stray characters. `env` looks up variables.
pub fn supports_hyperlinks(env: &dyn Fn(&str) -> Option<String>) -> bool {
    if env("NO_HYPERLINKS").is_some() {
        return false;
    }
    if env("FORCE_HYPERLINK").is_some() {
        return true;
    }
    if env("WT_SESSION").is_some() || env("KITTY_WINDOW_ID").is_some() || env("WEZTERM_EXECUTABLE").is_some() {
        return true;
    }
    if env("VTE_VERSION").and_then(|v| v.parse::<u32>().ok()).is_some_and(|v| v >= 5000) {
        return true;
    }
    matches!(env("TERM_PROGRAM").as_deref(), Some("iTerm.app" | "WezTerm" | "vscode" | "ghostty" | "Hyper"))
}

pub fn system_supports_hyperlinks() -> bool {
    supports_hyperlinks(&|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
}

/// Escape sequence that re-draws `text` at (`x`,`y`) as a hyperlink to `url` with the SGR style
/// `sgr` (digits and `;`, e.g. `4;36`), then restores the cursor.
/// Only characters without control codes are allowed in `text` and `url`.
pub fn osc8_overlay(x: u16, y: u16, sgr: &str, text: &str, url: &str) -> Option<String> {
    if text.chars().any(|c| c.is_control()) || url.chars().any(|c| c.is_control()) || !sgr.chars().all(|c| c.is_ascii_digit() || c == ';') {
        return None;
    }
    Some(format!("\x1b7\x1b[{};{}H\x1b[{sgr}m\x1b]8;;{url}\x1b\\{text}\x1b]8;;\x1b\\\x1b[0m\x1b8", y + 1, x + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        if cfg!(windows) { PathBuf::from("C:\\Kora\\my server & co") } else { PathBuf::from("/srv/kora/my server & co") }
    }

    #[test]
    fn only_local_txadmin_urls_are_opened() {
        let c = open_url_command(OsKind::Other, "http://localhost:40120").unwrap();
        assert_eq!((c.program.as_str(), c.args), ("xdg-open", vec![OsString::from("http://localhost:40120")]));
        assert_eq!(open_url_command(OsKind::Windows, "http://127.0.0.1:41120/").unwrap().program, "explorer.exe");
        for bad in ["https://evil.example/", "http://localhost.evil.example/", "http://localhost:1/ & calc", "file:///etc/passwd", "http://localhost:1/\n"] {
            assert!(open_url_command(OsKind::Other, bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn command_keeps_path_as_one_literal_argument() {
        let log = root().join(".kora").join("install.log");
        for (os, prog) in [(OsKind::Windows, "explorer.exe"), (OsKind::Other, "xdg-open")] {
            let c = open_command(os, &log, &root()).unwrap();
            assert_eq!(c.program, prog);
            assert_eq!(c.args, vec![log.as_os_str().to_owned()], "spaces and & stay in one argument");
        }
        assert!(open_command(OsKind::Other, &root(), &root()).is_ok(), "the folder itself is allowed");
    }

    #[test]
    fn command_rejects_unsafe_or_outside_paths() {
        let r = root();
        assert!(open_command(OsKind::Other, &r.join("..").join("etc").join("passwd"), &r).is_err());
        assert!(open_command(OsKind::Other, Path::new(if cfg!(windows) { "C:\\Windows\\notepad.exe" } else { "/bin/sh" }), &r).is_err());
        assert!(open_command(OsKind::Other, Path::new("relative.log"), &r).is_err());
        assert!(open_command(OsKind::Windows, &r.join("a\"b.log"), &r).is_err());
        assert!(open_command(OsKind::Windows, &r.join("a\nb.log"), &r).is_err());
        // a sibling that merely shares a prefix is outside
        let sibling = PathBuf::from(format!("{}-evil", r.display())).join("x.log");
        assert!(open_command(OsKind::Other, &sibling, &r).is_err());
    }

    #[test]
    fn roots_and_urls() {
        assert_eq!(root_for_log(Path::new("/s/x/.kora/install.log")), PathBuf::from("/s/x"));
        assert_eq!(root_for_log(Path::new("/tmp/kora-install-x.log")), PathBuf::from("/tmp"));
        assert_eq!(file_url(Path::new("/srv/a b&c/x.log")), "file:///srv/a%20b%26c/x.log");
        assert_eq!(file_url(Path::new("C:\\Kora\\my-server\\.kora\\install.log")), "file:///C:/Kora/my-server/.kora/install.log");
    }

    #[test]
    fn hyperlink_support_detection() {
        let with = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
        };
        assert!(supports_hyperlinks(&with(&[("WT_SESSION", "x")])));
        assert!(supports_hyperlinks(&with(&[("VTE_VERSION", "6800")])));
        assert!(!supports_hyperlinks(&with(&[("VTE_VERSION", "4000")])));
        assert!(!supports_hyperlinks(&with(&[])), "old console / unknown terminals get no links");
        assert!(!supports_hyperlinks(&with(&[("WT_SESSION", "x"), ("NO_HYPERLINKS", "1")])));
    }

    #[test]
    fn overlay_is_well_formed_and_refuses_control_chars() {
        let s = osc8_overlay(4, 9, "4;36", "C:\\x.log", "file:///C:/x.log").unwrap();
        assert!(s.contains("\x1b[10;5H\x1b[4;36m") && s.contains("\x1b]8;;file:///C:/x.log\x1b\\C:\\x.log\x1b]8;;\x1b\\"));
        assert!(osc8_overlay(0, 0, "0", "a\x1bb", "u").is_none());
        assert!(osc8_overlay(0, 0, "0", "a", "u\x07").is_none());
        assert!(osc8_overlay(0, 0, "0;x", "a", "u").is_none());
    }
}
