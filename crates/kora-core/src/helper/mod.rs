//! The optional privileged helper (panel plan, section 1 "Privileges", phase 4).
//!
//! Some panel actions need administrator rights (installing a server with a service or firewall rule,
//! changing the update schedule, removing Kora's system changes, ...). Without the helper the panel
//! answers `needs_elevation` with the `kora ...` command for the user to run, and that stays true.
//! The user may instead install the helper ONCE (`sudo kora helper install`); the panel then runs
//! those same commands itself.
//!
//! # Design
//!
//! * **Process.** `kora helper serve`, started by an OS service as root. Linux: a systemd unit
//!   (`kora-helper.service`). Windows: NOT implemented in this slice (see below).
//! * **Listener.** Unix socket [`SOCKET_PATH`] (`/run/kora/helper.sock`), created with mode `0660`,
//!   owner root, group = the installing user's PRIMARY group. We do not create a dedicated `kora`
//!   group: that needs `groupadd`, adding the user to it and a fresh login before it works. The
//!   file permission is only the first fence; the real control is the peer check below, so a
//!   shared primary group (`users` on some distros) lets other members connect but not be served.
//!   The directory `/run/kora` is `0755` (systemd `RuntimeDirectory=`).
//! * **Peer check.** Every connection is checked with `SO_PEERCRED`: the peer uid must be root or
//!   the uid recorded in [`HelperConfig`] (`/etc/kora/helper.json`, written at install time from
//!   `SUDO_UID`). Anyone else gets an `error` message and the connection is closed without parsing.
//! * **Protocol.** One connection = one request. The client sends ONE JSON line
//!   `{"verb": "...", "args": [...], "env": {"CFX_LICENSE_KEY": "..."}}` (at most
//!   [`MAX_REQUEST_BYTES`]). The server answers with JSON lines: any number of
//!   `{"type":"progress","stream":"stdout"|"stderr","line":"..."}` and then exactly one
//!   `{"type":"result","ok":bool,"code":int,"message":?}` (or `{"type":"error","message":...}`
//!   when the request was refused). The verb `ping` answers a `result` with the version and is
//!   used for "is the helper there?".
//! * **Allow-list only.** `verb` must be one of [`ALLOWED_VERBS`], the top-level `kora`
//!   subcommands the panel hands to the user today as `needs_elevation` commands: `install`,
//!   `self-update`, `schedule`, `app`, `uninstall`. The helper then calls the injected
//!   [`server::Validator`], which in `kora helper serve` re-parses the arguments with the SAME
//!   clap `Cli` the `kora` binary uses and accepts only the exact privileged invocations (for
//!   example `schedule set|off`, `app install|uninstall`, `install` without `--code`). So the
//!   helper can never run anything the CLI could not, and `delete`, `service-host`, `--code`,
//!   unknown verbs, `--license-key` on the command line and unknown environment names are refused.
//! * **No shell.** The child is the same `kora` binary, spawned with an argument vector
//!   (`--no-elevate <verb> <args>...`), never through a shell: metacharacters are plain argument
//!   text. The binary is a root-owned copy made by `helper install`
//!   (`/usr/local/libexec/kora/kora`); `serve` refuses to start when its own executable or any
//!   parent folder is writable by someone else, otherwise a user could swap the binary and gain
//!   root. The child's environment is rebuilt from scratch (`PATH`, `HOME`, `USER`, `LOGNAME`,
//!   `SUDO_USER`, `SUDO_UID`, `KORA_HOME` of the installing user, as `sudo kora ...` would have),
//!   plus the secrets of the request.
//! * **Secrets.** The licence key and database admin password travel in `env`
//!   (`CFX_LICENSE_KEY`, `KORA_DB_ADMIN_PASSWORD`, nothing else), never on argv. They are not
//!   logged, and their values are masked if the child ever echoes them.
//! * **One at a time.** Privileged runs are serialised by a lock; a second request waits.
//!
//! # Windows
//!
//! Not built in this slice. The planned shape is a named pipe `\\.\pipe\kora-helper` with a DACL
//! for the installing user's SID, run by a service through the existing `kora service-host`
//! machinery, with the same protocol and allow-list. Until then `kora helper install` says it is
//! not available on Windows and the panel keeps answering `needs_elevation` there.

pub mod client;
pub mod install;
pub mod server;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Where the Linux helper listens.
pub const SOCKET_PATH: &str = "/run/kora/helper.sock";
/// Where the installing user is recorded (root-owned, world-readable, no secrets).
pub const CONFIG_PATH: &str = "/etc/kora/helper.json";
/// Planned Windows pipe name (not served yet).
pub const PIPE_NAME: &str = r"\\.\pipe\kora-helper";
/// The root-owned copy of `kora` the helper runs.
pub const HELPER_EXE: &str = "/usr/local/libexec/kora/kora";
pub const UNIT_NAME: &str = "kora-helper.service";
pub const UNIT_PATH: &str = "/etc/systemd/system/kora-helper.service";
/// A request line longer than this is refused.
pub const MAX_REQUEST_BYTES: usize = 64 * 1024;
const MAX_ARGS: usize = 64;
const MAX_ARG_BYTES: usize = 4096;
const MAX_ENV_BYTES: usize = 2048;

/// The only environment variables a request may set for the child.
pub const SECRET_ENV: &[&str] = &["CFX_LICENSE_KEY", "KORA_DB_ADMIN_PASSWORD"];
/// The top-level `kora` subcommands the helper will run (see the module documentation).
pub const ALLOWED_VERBS: &[&str] = &["install", "self-update", "schedule", "app", "uninstall"];
/// Answered directly with the version; runs nothing.
pub const PING: &str = "ping";

/// Written at install time; read by `kora helper serve`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelperConfig {
    /// The installing user: the only non-root uid that is served.
    pub uid: u32,
    pub gid: u32,
    pub user: String,
    pub home: String,
    /// `KORA_HOME` of that user's shell when installing, if it was set.
    #[serde(default)]
    pub kora_home: Option<String>,
    pub version: String,
}

/// One request (see the module documentation). `Debug` hides the environment values.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Request {
    pub verb: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request")
            .field("verb", &self.verb)
            .field("args", &self.args)
            .field("env", &self.env.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// One line of the answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    Progress { stream: String, line: String },
    Result {
        ok: bool,
        code: i32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<String>,
    },
    Error { message: String },
}

/// Why a request was refused (plain language; shown to the user).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejected(pub String);

impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Peer rule: root, or the installing user.
pub fn peer_allowed(peer_uid: u32, allowed_uid: u32) -> bool {
    peer_uid == 0 || peer_uid == allowed_uid
}

/// Everything that can be checked without the CLI: verb on the list, sizes, no NUL bytes, no
/// secrets or removed flags on argv, only the two secret environment names. On success returns the
/// child's argument vector (`--no-elevate <verb> <args>...`).
/// The CLI-side argument check (see [`server::Validator`]).
pub type ValidateFn<'a> = &'a dyn Fn(&str, &[String]) -> Result<(), String>;

pub fn check_request(req: &Request, validate: ValidateFn<'_>) -> Result<Vec<String>, Rejected> {
    let no = |m: &str| Err(Rejected(m.to_string()));
    if !ALLOWED_VERBS.contains(&req.verb.as_str()) {
        return no("The helper does not run that command.");
    }
    if req.args.len() > MAX_ARGS || req.args.iter().any(|a| a.len() > MAX_ARG_BYTES || a.contains('\0')) {
        return no("The helper refused the arguments (too many, too long or invalid).");
    }
    for a in &req.args {
        let name = a.split('=').next().unwrap_or("");
        if matches!(name, "--license-key" | "--code") {
            return no("The helper does not accept that option on the command line.");
        }
    }
    for (k, v) in &req.env {
        if !SECRET_ENV.contains(&k.as_str()) || v.len() > MAX_ENV_BYTES || v.contains('\0') {
            return no("The helper does not accept that environment setting.");
        }
    }
    validate(&req.verb, &req.args).map_err(Rejected)?;
    let mut argv = vec!["--no-elevate".to_string(), req.verb.clone()];
    argv.extend(req.args.iter().cloned());
    Ok(argv)
}

/// Mask every secret value (6+ characters) in `line`.
pub fn mask_secrets(line: &str, secrets: &[&str]) -> String {
    let mut out = line.to_string();
    for s in secrets.iter().filter(|s| s.len() >= 6) {
        out = out.replace(s, "***");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(_: &str, _: &[String]) -> Result<(), String> {
        Ok(())
    }
    fn req(verb: &str, args: &[&str]) -> Request {
        Request { verb: verb.into(), args: args.iter().map(|s| s.to_string()).collect(), env: BTreeMap::new() }
    }

    #[test]
    fn only_listed_verbs_pass() {
        for v in ALLOWED_VERBS {
            assert!(check_request(&req(v, &[]), &ok).is_ok(), "{v}");
        }
        for v in ["delete", "service-host", "start", "stop", "restart", "manager", "panel", "", "rm", "sh", "install ", "Install", "ping"] {
            assert!(check_request(&req(v, &[]), &ok).is_err(), "{v:?}");
        }
    }

    #[test]
    fn metacharacters_are_just_arguments() {
        let r = req("uninstall", &["; rm -rf /", "$(id)", "`id`", "a b", "--yes\n"]);
        let argv = check_request(&r, &ok).unwrap();
        assert_eq!(argv[..2], ["--no-elevate", "uninstall"]);
        assert_eq!(argv[2], "; rm -rf /");
        assert!(check_request(&req("uninstall", &["a\0b"]), &ok).is_err());
    }

    #[test]
    fn secrets_and_removed_flags_never_ride_on_argv() {
        for a in [&["--license-key", "x"][..], &["--license-key=x"], &["--code", "KORA-1"], &["--code=KORA-1"]] {
            assert!(check_request(&req("install", a), &ok).is_err(), "{a:?}");
        }
        let mut r = req("install", &["--name", "a"]);
        r.env.insert("CFX_LICENSE_KEY".into(), "cfxk_secret".into());
        r.env.insert("KORA_DB_ADMIN_PASSWORD".into(), "pw".into());
        let argv = check_request(&r, &ok).unwrap();
        assert!(argv.iter().all(|a| !a.contains("cfxk_secret") && a != "pw"));
        r.env.insert("LD_PRELOAD".into(), "/tmp/x.so".into());
        assert!(check_request(&r, &ok).is_err(), "only the two secret names are accepted");
        let mut r = req("install", &[]);
        r.env.insert("PATH".into(), "/tmp".into());
        assert!(check_request(&r, &ok).is_err());
    }

    #[test]
    fn the_validator_has_the_last_word() {
        let deny = |_: &str, _: &[String]| Err("no".to_string());
        assert_eq!(check_request(&req("app", &["status"]), &deny), Err(Rejected("no".into())));
    }

    #[test]
    fn peer_rule() {
        assert!(peer_allowed(0, 1000) && peer_allowed(1000, 1000));
        assert!(!peer_allowed(1001, 1000) && !peer_allowed(65534, 1000));
    }

    #[test]
    fn request_debug_hides_values_and_secrets_are_masked() {
        let mut r = req("install", &[]);
        r.env.insert("CFX_LICENSE_KEY".into(), "cfxk_TOPSECRET".into());
        assert!(!format!("{r:?}").contains("TOPSECRET"));
        assert_eq!(mask_secrets("key is cfxk_TOPSECRET ok", &["cfxk_TOPSECRET", "x"]), "key is *** ok");
    }

    #[test]
    fn messages_round_trip() {
        let m = Message::Result { ok: true, code: 0, message: None, version: Some("1".into()) };
        let s = serde_json::to_string(&m).unwrap();
        assert!(s.contains(r#""type":"result""#));
        assert_eq!(serde_json::from_str::<Message>(&s).unwrap(), m);
    }
}

#[cfg(all(test, unix))]
mod socket_tests;
