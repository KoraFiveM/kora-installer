//! The tiny bridge between the panel page and the shell.
//!
//! The page asks by posting one JSON message (`window.ipc.postMessage(...)`, wrapped by `web/src/lib/shell.ts`
//! as `window.koraShell`); the shell answers by calling `window.koraShell._reply(...)` with the same `id`.
//!
//! # Threat model
//!
//! * Only the panel's own origin may ask ([`crate::nav::trusted_page`]); the GUI layer drops anything else before
//!   it reaches [`parse`].
//! * The list of things that may be asked is the [`Command`] enum and nothing else. There is no way to run a
//!   program, read a file, write a file or pass a path to the shell. Choosing a file or folder returns the
//!   chosen path TO the page (which could ask the panel for it anyway); nothing is done with it by the shell.
//! * Every text field has a length limit; unknown commands and malformed JSON are refused with a plain error.
//! * The one elevated action is a fixed name ([`ElevatedAction::InstallService`]) mapped by the shell to a fixed
//!   command; the page cannot supply a program, an argument or a path. Administrator rights still need the
//!   person to confirm Windows' own prompt.
//! * Notifications can only open a panel route ([`crate::nav::safe_route`]), never an address.
//! * Opening links in the browser re-uses the navigation rules: only `http`/`https`.

use serde::{Deserialize, Serialize};

use crate::nav;

/// Largest message the shell reads.
pub const MAX_MESSAGE: usize = 8 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    /// Who is there, and what can they do (the page uses this to show or hide buttons).
    Hello,
    PickFile {
        #[serde(default)]
        title: Option<String>,
        /// File name endings without the dot (`["zip", "kora"]`); empty = any file.
        #[serde(default)]
        extensions: Vec<String>,
    },
    PickFolder {
        #[serde(default)]
        title: Option<String>,
    },
    /// Open an `http(s)` link in the default browser.
    OpenExternal { url: String },
    Notify {
        title: String,
        #[serde(default)]
        body: String,
        #[serde(default)]
        route: Option<String>,
        #[serde(default)]
        urgent: bool,
    },
    /// Run the one fixed administrator action, with Windows' prompt in front of the app.
    Elevate { action: ElevatedAction },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElevatedAction {
    /// "Install Kora's administrator service" (the installer of the privileged service).
    InstallService,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Request {
    pub id: u32,
    #[serde(flatten)]
    pub command: Command,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeError {
    TooLarge,
    NotJson,
    /// With the id when it could be read, so the page's promise can be rejected.
    Refused(Option<u32>, &'static str),
}

fn short(s: &str, max: usize) -> bool {
    s.chars().count() <= max && !s.chars().any(|c| c == '\0')
}

/// Read and check one message. `trusted` says the sender is the panel page (from [`nav::trusted_page`]).
pub fn parse(raw: &str, trusted: bool) -> Result<Request, BridgeError> {
    if !trusted {
        return Err(BridgeError::Refused(None, "this page may not use the Kora app"));
    }
    if raw.len() > MAX_MESSAGE {
        return Err(BridgeError::TooLarge);
    }
    let id = serde_json::from_str::<serde_json::Value>(raw).map_err(|_| BridgeError::NotJson)?.get("id").and_then(|v| v.as_u64()).and_then(|v| u32::try_from(v).ok());
    let req: Request = serde_json::from_str(raw).map_err(|_| BridgeError::Refused(id, "unknown request"))?;
    let bad = |why| Err(BridgeError::Refused(Some(req.id), why));
    match &req.command {
        Command::Hello | Command::Elevate { .. } => {}
        Command::PickFile { title, extensions } => {
            if title.as_deref().is_some_and(|t| !short(t, 100)) {
                return bad("title too long");
            }
            if extensions.len() > 8 || extensions.iter().any(|e| e.is_empty() || e.len() > 10 || !e.bytes().all(|b| b.is_ascii_alphanumeric())) {
                return bad("unusable file types");
            }
        }
        Command::PickFolder { title } => {
            if title.as_deref().is_some_and(|t| !short(t, 100)) {
                return bad("title too long");
            }
        }
        Command::OpenExternal { url } => {
            if !matches!(nav::decide(url, 0), nav::Nav::External(_)) {
                return bad("only web links can be opened");
            }
        }
        Command::Notify { title, body, route, .. } => {
            if title.trim().is_empty() || !short(title, 100) || !short(body, 300) {
                return bad("notification text missing or too long");
            }
            if route.as_deref().is_some_and(|r| !nav::safe_route(r)) {
                return bad("not a page of Kora");
            }
        }
    }
    Ok(req)
}

/// The answer to a request.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Reply {
    pub id: u32,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Reply {
    pub fn ok(id: u32, value: serde_json::Value) -> Self {
        Self { id, ok: true, value: Some(value), error: None }
    }

    pub fn err(id: u32, why: impl Into<String>) -> Self {
        Self { id, ok: false, value: None, error: Some(why.into()) }
    }

    /// The script that hands the answer to the page. The JSON is embedded as a JavaScript literal, with the two
    /// line-separator characters escaped (valid JSON, not valid in older JavaScript string literals).
    pub fn script(&self) -> String {
        let json = serde_json::to_string(self).unwrap_or_else(|_| "{}".into()).replace('\u{2028}', "\\u2028").replace('\u{2029}', "\\u2029");
        format!("window.koraShell&&window.koraShell._reply({json})")
    }
}

/// What `hello` answers: the version and what this shell can do.
pub fn hello_value(version: &str, windows: bool, elevate: bool) -> serde_json::Value {
    serde_json::json!({
        "shell": "kora-desktop",
        "version": version,
        "os": if windows { "windows" } else { "linux" },
        "features": { "pick_file": true, "pick_folder": true, "open_external": true, "notify": true, "elevate": elevate },
    })
}

/// A message the page may send before the shell is ready is simply refused; this is the injected script that
/// defines `window.koraShell` for the page (before any page script runs, at document start).
pub const INIT_SCRIPT: &str = r#"(function () {
  if (window.koraShell) return;
  var pending = {}, next = 1;
  function ask(cmd) {
    return new Promise(function (resolve, reject) {
      var id = next++;
      pending[id] = { resolve: resolve, reject: reject };
      var msg = Object.assign({ id: id }, cmd);
      try { window.ipc.postMessage(JSON.stringify(msg)); } catch (e) { delete pending[id]; reject(e); }
    });
  }
  window.koraShell = {
    _reply: function (r) {
      var p = pending[r.id]; if (!p) return; delete pending[r.id];
      if (r.ok) p.resolve(r.value); else p.reject(new Error(r.error || 'The Kora app said no.'));
    },
    hello: function () { return ask({ cmd: 'hello' }); },
    pickFile: function (o) { o = o || {}; return ask({ cmd: 'pick_file', title: o.title, extensions: o.extensions || [] }); },
    pickFolder: function (o) { o = o || {}; return ask({ cmd: 'pick_folder', title: o.title }); },
    openExternal: function (url) { return ask({ cmd: 'open_external', url: String(url) }); },
    notify: function (o) { return ask({ cmd: 'notify', title: o.title, body: o.body || '', route: o.route, urgent: !!o.urgent }); },
    elevate: function (action) { return ask({ cmd: 'elevate', action: action }); }
  };
})();"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(raw: &str) -> Request {
        parse(raw, true).unwrap_or_else(|e| panic!("{raw}: {e:?}"))
    }

    fn refused(raw: &str) -> BridgeError {
        parse(raw, true).unwrap_err()
    }

    #[test]
    fn the_allowed_requests_parse() {
        assert_eq!(ok(r#"{"id":1,"cmd":"hello"}"#), Request { id: 1, command: Command::Hello });
        assert_eq!(
            ok(r#"{"id":2,"cmd":"pick_file","title":"Choose a backup","extensions":["zip","kora"]}"#).command,
            Command::PickFile { title: Some("Choose a backup".into()), extensions: vec!["zip".into(), "kora".into()] }
        );
        assert_eq!(ok(r#"{"id":3,"cmd":"pick_folder"}"#).command, Command::PickFolder { title: None });
        assert_eq!(ok(r#"{"id":4,"cmd":"open_external","url":"https://github.com/KoraFiveM/kora"}"#).command, Command::OpenExternal { url: "https://github.com/KoraFiveM/kora".into() });
        assert_eq!(
            ok(r#"{"id":5,"cmd":"notify","title":"Done","body":"x","route":"/servers/a"}"#).command,
            Command::Notify { title: "Done".into(), body: "x".into(), route: Some("/servers/a".into()), urgent: false }
        );
        assert_eq!(ok(r#"{"id":6,"cmd":"elevate","action":"install_service"}"#).command, Command::Elevate { action: ElevatedAction::InstallService });
    }

    #[test]
    fn anything_else_is_refused() {
        assert_eq!(parse(r#"{"id":1,"cmd":"hello"}"#, false), Err(BridgeError::Refused(None, "this page may not use the Kora app")));
        for raw in [
            r#"{"id":1,"cmd":"run","program":"cmd.exe"}"#,
            r#"{"id":1,"cmd":"elevate","action":"run_anything"}"#,
            r#"{"id":1,"cmd":"elevate","action":"uninstall_service"}"#,
            r#"{"id":1,"cmd":"open_external","url":"file:///C:/Windows/notepad.exe"}"#,
            r#"{"id":1,"cmd":"open_external","url":"javascript:alert(1)"}"#,
            r#"{"id":1,"cmd":"notify","title":"x","route":"http://evil.example"}"#,
            r#"{"id":1,"cmd":"notify","title":"  "}"#,
            r#"{"id":1,"cmd":"pick_file","extensions":["../../x"]}"#,
            r#"{"id":1,"cmd":"pick_file","extensions":["a","b","c","d","e","f","g","h","i"]}"#,
        ] {
            assert!(matches!(refused(raw), BridgeError::Refused(Some(1), _)), "{raw}: {:?}", refused(raw));
        }
        assert_eq!(refused("not json"), BridgeError::NotJson);
        assert_eq!(refused(&format!(r#"{{"id":1,"cmd":"hello","pad":"{}"}}"#, "a".repeat(MAX_MESSAGE))), BridgeError::TooLarge);
        assert!(matches!(refused(r#"{"cmd":"hello"}"#), BridgeError::Refused(None, _)), "no id");
        assert!(matches!(refused(&format!(r#"{{"id":1,"cmd":"notify","title":"{}"}}"#, "t".repeat(101))), BridgeError::Refused(Some(1), _)));
    }

    #[test]
    fn replies_are_safe_to_embed_in_a_script() {
        let r = Reply::ok(7, serde_json::json!({"path": "C:\\Users\\a b\\x\u{2028}</script>\"'"}));
        let s = r.script();
        assert!(s.starts_with("window.koraShell&&window.koraShell._reply({"), "{s}");
        assert!(!s.contains('\u{2028}') && s.contains("\\u2028"));
        assert!(s.contains(r#""id":7"#) && s.contains(r#""ok":true"#));
        let e = Reply::err(8, "no").script();
        assert!(e.contains(r#""ok":false"#) && e.contains(r#""error":"no""#) && !e.contains("value"));
    }

    #[test]
    fn hello_tells_the_page_what_exists() {
        let v = hello_value("0.8.0", true, true);
        assert_eq!((v["os"].as_str(), v["features"]["elevate"].as_bool()), (Some("windows"), Some(true)));
        assert_eq!(hello_value("1", false, false)["features"]["elevate"], false);
    }

    #[test]
    fn the_injected_script_exposes_only_the_allowed_calls() {
        for name in ["hello", "pickFile", "pickFolder", "openExternal", "notify", "elevate", "_reply"] {
            assert!(INIT_SCRIPT.contains(&format!("{name}:")), "{name}");
        }
        assert!(INIT_SCRIPT.contains("window.ipc.postMessage"));
    }
}
