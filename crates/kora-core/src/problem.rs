//! [`Problem`]: an error a person can act on. It says in plain words what happened, can carry the
//! technical details for a "Show details" box, and lists the [`Action`]s the user can take (the
//! panel draws one button per action).
//!
//! # Raising a Problem
//!
//! ```
//! use kora_core::problem::{codes, Action, Problem, Tool};
//!
//! let p = Problem::new(codes::GH_MISSING, "The GitHub CLI (gh) is not installed. Kora needs it for development servers.")
//!     .with_details("`gh --version` could not be run")
//!     .with_action(Action::InstallTool { tool: Tool::Gh })
//!     .with_action(Action::ChangeAnswers { section: Some("mode".into()) });
//! assert_eq!(p.actions.len(), 2);
//! ```
//!
//! # Carrying it through code that only knows `String` errors
//!
//! Most of Kora's errors are still `String`s (`api::Res<T>`, `InstallError`, job errors). A Problem
//! goes through them unchanged: `String::from(problem)` (or [`Problem::encode`]) packs it into a
//! string, and [`Problem::decode`] (or `Problem::from(string)`) unpacks it again. A plain string
//! that never was a Problem decodes to a generic one ([`codes::FAILED`], no actions), so every
//! existing error keeps working; the panel then offers its generic "Try again / Copy details".
//!
//! Code that shows an error in a terminal must use [`plain_text`] (or `Problem::to_string()`), never
//! the packed string itself.
//!
//! # Adding an action
//!
//! 1. Add a variant to [`Action`] (snake_case `kind` on the wire; fields are plain data, never
//!    secrets). [`Action`] is `#[non_exhaustive]`, so other crates already match it with a `_` arm
//!    and nothing else has to change.
//! 2. Optionally give it a terminal hint in `Action::terminal_hint`.
//! 3. In the panel, add a handler for the `kind` in `web/src/lib/problemActions.ts` (an action
//!    without a handler is not shown, so an older panel ignores newer actions safely).

use serde::{Deserialize, Serialize};

/// Stable machine codes. Front ends and tests branch on these; never change an existing one.
/// Group them by area with a dotted prefix (`install.`, `github.`, `admin.`).
pub mod codes {
    /// Anything without a more specific code (also every plain string error).
    pub const FAILED: &str = "failed";
    /// Administrator / root rights are needed and Kora does not have them.
    pub const NEEDS_ELEVATION: &str = "needs_elevation";
    /// Windows: the administrator permission prompt was declined (`helper::worker`).
    pub const ADMIN_DECLINED: &str = "admin.declined";
    /// Windows: nobody answered the administrator permission prompt in time.
    pub const ADMIN_TIMED_OUT: &str = "admin.timed_out";
    /// Windows: a remote session; the prompt would appear on a desktop nobody may be watching.
    pub const ADMIN_REMOTE: &str = "admin.remote";
    /// Windows: an earlier permission prompt is still open.
    pub const ADMIN_BUSY: &str = "admin.busy";
    /// Windows: the administrator worker stopped before it finished.
    pub const ADMIN_LOST: &str = "admin.lost";

    /// An install step failed for a reason that has no code of its own.
    pub const INSTALL_STEP_FAILED: &str = "install.step_failed";
    /// No licence key was given.
    pub const INSTALL_LICENCE_MISSING: &str = "install.licence_missing";
    /// A port was taken after the questions were answered.
    pub const INSTALL_PORTS_BUSY: &str = "install.ports_busy";
    /// A tool (git, gh) is missing and Kora could not install it here.
    pub const INSTALL_TOOL_MISSING: &str = "install.tool_missing";
    /// A tool was just installed but this process cannot see it yet.
    pub const INSTALL_TOOL_NOT_FOUND_YET: &str = "install.tool_not_found_yet";
    /// No database server was found and the answers say not to install one.
    pub const INSTALL_DB_MISSING: &str = "install.db_missing";
    /// The database server is installed but stopped.
    pub const INSTALL_DB_STOPPED: &str = "install.db_stopped";
    /// Kora could not log in to the database server as an administrator.
    pub const INSTALL_DB_LOGIN: &str = "install.db_login_failed";
    /// Some forks could not be made.
    pub const INSTALL_FORKS_FAILED: &str = "install.forks_failed";
    /// The step to retry or skip is not known, not failed, or not safe to skip.
    pub const INSTALL_CANNOT_RESUME: &str = "install.cannot_resume";

    /// A remote sign-in made with a token asked for something that stays off until `remote actions` is on.
    pub const REMOTE_ACTIONS_OFF: &str = "remote.actions_off";
    /// A remote session signed in with a passkey or one-time code asked for a sensitive action without a recent check.
    pub const REMOTE_REAUTH: &str = "remote.reauth_required";
    /// A saved licence key's value is not in the place Kora wrote it to (the system keychain entry is gone).
    pub const KEYS_VALUE_MISSING: &str = "keys.value_missing";
    /// The saved licence keys file (`secrets.json`) cannot be read or is damaged: nothing is changed.
    pub const KEYS_UNREADABLE: &str = "keys.unreadable";
    /// No saved licence key has that name (never saved, renamed or deleted).
    pub const KEYS_NOT_FOUND: &str = "keys.not_found";
    /// A saved licence key already has that name.
    pub const KEYS_NAME_TAKEN: &str = "keys.name_taken";
    /// That licence key is already saved (under another name).
    pub const KEYS_DUPLICATE: &str = "keys.duplicate";
    /// The key name or the key itself is not acceptable.
    pub const KEYS_INVALID: &str = "keys.invalid";
    /// Kora could not find or change the licence key in the server's own configuration.
    pub const KEYS_SERVER_CONFIG: &str = "keys.server_config";

    /// The GitHub CLI (`gh`) is not installed.
    pub const GH_MISSING: &str = "github.gh_missing";
    /// `gh` is installed but not signed in.
    pub const GH_SIGNED_OUT: &str = "github.signed_out";
    /// Signed in, but without the permissions (scopes) Kora needs.
    pub const GH_MISSING_SCOPES: &str = "github.missing_scopes";
    /// The browser sign-in was started but did not finish.
    pub const GH_SIGN_IN_INCOMPLETE: &str = "github.sign_in_incomplete";
    /// The chosen fork owner cannot be used.
    pub const GH_OWNER_INVALID: &str = "github.owner_invalid";
    /// GitHub answered "slow down".
    pub const GH_RATE_LIMITED: &str = "github.rate_limited";
    /// Reading the GitHub accounts failed.
    pub const GH_UNAVAILABLE: &str = "github.unavailable";

    /// Kora's own copy of git or gh could not be downloaded (offline, a proxy, GitHub busy).
    pub const TOOLS_DOWNLOAD_FAILED: &str = "tools.download_failed";
    /// There is no copy of the tool for this kind of computer.
    pub const TOOLS_UNSUPPORTED: &str = "tools.unsupported";
    /// The download did not match the checksum Kora expects, so nothing was installed.
    pub const TOOLS_CHECKSUM: &str = "tools.checksum_mismatch";
    /// Starting the database server did not work.
    pub const DB_START_FAILED: &str = "db.start_failed";
}

/// Ready-made problems that several places raise (the installer and the panel's GitHub routes).
pub mod known {
    use super::{codes, Action, Problem, Tool};

    /// Where the GitHub CLI can be downloaded by hand.
    pub const GH_URL: &str = "https://cli.github.com";
    /// Where Git can be downloaded by hand.
    pub const GIT_URL: &str = "https://git-scm.com/downloads";

    /// `gh` is not installed.
    pub fn gh_missing() -> Problem {
        Problem::new(codes::GH_MISSING, "The GitHub CLI (gh) is not installed. Kora needs it to work with GitHub.")
            .with_action(Action::InstallTool { tool: Tool::Gh })
            .with_action(Action::open_link(GH_URL))
    }

    /// Git is not installed (and Kora has no copy of its own yet).
    pub fn git_missing() -> Problem {
        Problem::new(codes::INSTALL_TOOL_MISSING, "Git is not installed. Kora needs it to download and update resources.")
            .with_action(Action::InstallTool { tool: Tool::Git })
            .with_action(Action::open_link(GIT_URL))
    }

    /// A database server is installed but not running.
    pub fn db_stopped(name: &str) -> Problem {
        Problem::new(codes::INSTALL_DB_STOPPED, format!("The database server {name} is installed but not running.")).with_action(Action::StartDatabase)
    }

    /// `gh` is installed but not signed in.
    pub fn github_signed_out() -> Problem {
        Problem::new(codes::GH_SIGNED_OUT, "You are not signed in to GitHub.").with_action(Action::SignInGithub { scopes: vec![] })
    }

    /// Signed in without the permissions Kora needs.
    pub fn github_missing_scopes(missing: &[String]) -> Problem {
        Problem::new(codes::GH_MISSING_SCOPES, "Your GitHub sign-in does not give Kora all the permissions it needs. Sign in again to give them.")
            .with_details(format!("Missing permissions (scopes): {}", missing.join(", ")))
            .with_action(Action::SignInGithub { scopes: missing.to_vec() })
    }
}

/// A tool Kora can install or point to. `#[non_exhaustive]`: new tools may be added.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Tool {
    Git,
    /// The GitHub CLI.
    Gh,
    Mariadb,
    /// A tool this build does not know (sent by a newer Kora).
    #[serde(other)]
    Unknown,
}

impl Tool {
    pub fn label(self) -> &'static str {
        match self {
            Tool::Git => "Git",
            Tool::Gh => "the GitHub CLI (gh)",
            Tool::Mariadb => "MariaDB",
            Tool::Unknown => "the missing tool",
        }
    }
}

/// Something the user can do about a [`Problem`]. Plain data: the front end decides what each
/// one does (the panel's handler registry; a terminal prints a hint). Never put secrets in here.
///
/// JSON: `{"kind": "<snake_case variant>", ...fields}`. An unknown `kind` reads as
/// [`Action::Unknown`], which front ends do not show.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    /// Do the same thing again (the generic "Try again").
    Retry,
    /// Run this step of a paused install again (`POST /installs/{job}/resume`).
    RetryStep { step: String },
    /// Skip this step of a paused install and carry on. Only offered for steps that are safe to
    /// skip (`installer::PlannedStep::skippable`).
    SkipStep { step: String },
    /// Go back to the install answers; `section` names the wizard page to open (`licence`,
    /// `mode`, `name`, `ports`, `database`, `options`, `github`, ...), `None` = the summary.
    ChangeAnswers {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        section: Option<String>,
    },
    /// Sign in to GitHub (again). `scopes`: the permissions that are missing (empty = a plain sign-in).
    SignInGithub {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        scopes: Vec<String>,
    },
    /// Install a tool for the user.
    InstallTool { tool: Tool },
    /// Ask for administrator permission (the elevated worker / helper).
    RequestAdmin,
    /// Carry on without the parts that need administrator rights.
    ContinueWithoutAdmin,
    /// Show a command the user can run themselves, with a copy button. `elevated`: it must be
    /// run as administrator / root (the panel adds `sudo` on Linux).
    ShowCommand {
        command: Vec<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        elevated: bool,
    },
    /// Open a web page (only `https://` addresses are shown).
    OpenLink { url: String },
    /// Show the log: of a panel job (`job`) or a file on this computer (`path`).
    OpenLog {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        job: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        path: Option<String>,
    },
    /// Copy the message and details (for a bug report).
    CopyDetails,
    /// Start the installed (but stopped) database server for the user.
    StartDatabase,
    /// An action this build does not know (sent by a newer Kora). Never shown.
    #[serde(other)]
    Unknown,
}

impl Action {
    pub fn change_answers(section: &str) -> Self {
        Action::ChangeAnswers { section: Some(section.to_string()) }
    }

    pub fn show_command<S: AsRef<str>>(command: &[S], elevated: bool) -> Self {
        Action::ShowCommand { command: command.iter().map(|s| s.as_ref().to_string()).collect(), elevated }
    }

    pub fn open_link(url: &str) -> Self {
        Action::OpenLink { url: url.to_string() }
    }

    /// The line a terminal prints for this action (`None`: nothing useful to say there).
    fn terminal_hint(&self) -> Option<String> {
        match self {
            Action::ShowCommand { command, elevated } => Some(format!("{}: {}", if *elevated { "Run as administrator" } else { "Run" }, command.join(" "))),
            Action::OpenLink { url } => Some(format!("See {url}")),
            Action::SignInGithub { scopes } if !scopes.is_empty() => Some(format!("Sign in again with more permissions: gh auth refresh -h github.com -s {}", scopes.join(","))),
            Action::SignInGithub { .. } => Some("Sign in to GitHub: gh auth login".into()),
            Action::InstallTool { tool } => Some(format!("Install {}.", tool.label())),
            Action::StartDatabase => Some("Start the database: kora check --start-database".into()),
            Action::RequestAdmin => Some("This needs administrator rights: run it again as administrator (root).".into()),
            _ => None,
        }
    }
}

/// An error a person can act on. See the module documentation.
///
/// JSON: `{"code", "message", "details"?, "actions": [..]}` (`details` left out when empty).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Problem {
    /// Stable machine code ([`codes`]).
    pub code: String,
    /// What happened, in plain words, for anyone. One or two sentences; no stack traces.
    pub message: String,
    /// Technical details (command output, error text) for the "Show details" box.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub details: Option<String>,
    /// What the user can do, best first. Empty: the front end offers its generic choices.
    #[serde(default)]
    pub actions: Vec<Action>,
}

/// Marks a string that holds a packed [`Problem`]. Starts with a control character so no
/// ordinary message can look like one.
const PACKED: &str = "\u{1}kora-problem:";

impl Problem {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self { code: code.to_string(), message: message.into(), details: None, actions: Vec::new() }
    }

    /// A problem with no specific code and no actions (what every plain string error becomes).
    pub fn generic(message: impl Into<String>) -> Self {
        Self::new(codes::FAILED, message)
    }

    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        let d = details.into();
        self.details = (!d.trim().is_empty()).then_some(d);
        self
    }

    /// Add an action (once: an action that is already there is not added again).
    pub fn with_action(mut self, a: Action) -> Self {
        self.push_action(a);
        self
    }

    pub fn with_actions(mut self, actions: impl IntoIterator<Item = Action>) -> Self {
        for a in actions {
            self.push_action(a);
        }
        self
    }

    /// Add an action at the end unless it is already there.
    pub fn push_action(&mut self, a: Action) {
        if !self.actions.contains(&a) {
            self.actions.push(a);
        }
    }

    pub fn has_code(&self, code: &str) -> bool {
        self.code == code
    }

    /// Pack into a string that [`Problem::decode`] turns back into this problem.
    pub fn encode(&self) -> String {
        // Serialising plain strings and enums cannot fail.
        format!("{PACKED}{}", serde_json::to_string(self).unwrap_or_default())
    }

    /// Unpack a string from [`Problem::encode`]; any other string becomes [`Problem::generic`].
    pub fn decode(s: &str) -> Self {
        Self::unpack(s).unwrap_or_else(|| Self::generic(s))
    }

    /// Is `s` a packed problem?
    pub fn is_encoded(s: &str) -> bool {
        Self::unpack(s).is_some()
    }

    fn unpack(s: &str) -> Option<Self> {
        serde_json::from_str(s.strip_prefix(PACKED)?).ok()
    }

    /// The JSON the panel reads.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    /// Read a problem from JSON that has at least `message` (missing `code` = [`codes::FAILED`];
    /// unknown fields are ignored, so job errors with extra fields read fine).
    pub fn from_json(v: &serde_json::Value) -> Option<Self> {
        let message = v.get("message")?.as_str()?.to_string();
        let code = v.get("code").and_then(|c| c.as_str()).unwrap_or(codes::FAILED).to_string();
        let details = v.get("details").and_then(|d| d.as_str()).map(String::from);
        let actions = v.get("actions").and_then(|a| serde_json::from_value(a.clone()).ok()).unwrap_or_default();
        Some(Self { code, message, details, actions })
    }
}

/// For terminals: the message, the details, and a line for each action that needs one.
impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)?;
        if let Some(d) = &self.details {
            write!(f, "\n{d}")?;
        }
        for h in self.actions.iter().filter_map(Action::terminal_hint) {
            write!(f, "\n{h}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Problem {}

/// Packs the problem (see [`Problem::encode`]).
impl From<Problem> for String {
    fn from(p: Problem) -> String {
        p.encode()
    }
}

impl From<String> for Problem {
    fn from(s: String) -> Self {
        Self::decode(&s)
    }
}

impl From<&str> for Problem {
    fn from(s: &str) -> Self {
        Self::decode(s)
    }
}

/// The text to show a person for an error string that may hold a packed [`Problem`].
pub fn plain_text(s: &str) -> String {
    match Problem::unpack(s) {
        Some(p) => p.to_string(),
        None => s.to_string(),
    }
}

/// Just the message of an error string that may hold a packed [`Problem`] (no details, no hints).
pub fn message_of(s: &str) -> String {
    match Problem::unpack(s) {
        Some(p) => p.message,
        None => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Problem {
        Problem::new(codes::GH_MISSING_SCOPES, "Your GitHub sign-in is missing a permission.")
            .with_details("missing: read:org")
            .with_action(Action::SignInGithub { scopes: vec!["read:org".into()] })
            .with_action(Action::RetryStep { step: "prepare.tools".into() })
            .with_action(Action::show_command(&["gh", "auth", "refresh"], false))
    }

    #[test]
    fn the_json_shape_is_stable() {
        let v = sample().to_json();
        assert_eq!(
            v,
            json!({
                "code": "github.missing_scopes",
                "message": "Your GitHub sign-in is missing a permission.",
                "details": "missing: read:org",
                "actions": [
                    {"kind": "sign_in_github", "scopes": ["read:org"]},
                    {"kind": "retry_step", "step": "prepare.tools"},
                    {"kind": "show_command", "command": ["gh", "auth", "refresh"]},
                ],
            })
        );
        let every = vec![
            Action::Retry,
            Action::SkipStep { step: "launch".into() },
            Action::ChangeAnswers { section: None },
            Action::change_answers("database"),
            Action::SignInGithub { scopes: vec![] },
            Action::InstallTool { tool: Tool::Gh },
            Action::RequestAdmin,
            Action::ContinueWithoutAdmin,
            Action::show_command(&["kora", "x"], true),
            Action::open_link("https://cli.github.com"),
            Action::OpenLog { job: Some("j".into()), path: None },
            Action::CopyDetails,
        ];
        assert_eq!(
            serde_json::to_value(&every).unwrap(),
            json!([
                {"kind": "retry"},
                {"kind": "skip_step", "step": "launch"},
                {"kind": "change_answers"},
                {"kind": "change_answers", "section": "database"},
                {"kind": "sign_in_github"},
                {"kind": "install_tool", "tool": "gh"},
                {"kind": "request_admin"},
                {"kind": "continue_without_admin"},
                {"kind": "show_command", "command": ["kora", "x"], "elevated": true},
                {"kind": "open_link", "url": "https://cli.github.com"},
                {"kind": "open_log", "job": "j"},
                {"kind": "copy_details"},
            ])
        );
        let back: Vec<Action> = serde_json::from_value(serde_json::to_value(&every).unwrap()).unwrap();
        assert_eq!(back, every);
    }

    #[test]
    fn json_round_trips_and_unknown_kinds_are_tolerated() {
        let p = sample();
        assert_eq!(serde_json::from_value::<Problem>(p.to_json()).unwrap(), p);
        // A newer Kora's action or tool reads as Unknown instead of failing the whole problem.
        let v = json!({"code": "x", "message": "m", "actions": [{"kind": "teleport", "where": 1}, {"kind": "install_tool", "tool": "node"}]});
        let p: Problem = serde_json::from_value(v).unwrap();
        assert_eq!(p.actions, vec![Action::Unknown, Action::InstallTool { tool: Tool::Unknown }]);
        // No details and no actions: both are optional.
        let p: Problem = serde_json::from_value(json!({"code": "failed", "message": "m"})).unwrap();
        assert_eq!(p, Problem::generic("m"));
        assert!(Problem::generic("m").to_json().get("details").is_none());
    }

    #[test]
    fn strings_carry_a_problem_and_plain_strings_still_work() {
        let p = sample();
        let s: String = p.clone().into();
        assert!(Problem::is_encoded(&s));
        assert_eq!(Problem::decode(&s), p);
        assert_eq!(Problem::from(s.clone()), p);
        // Any old error string is a generic problem with no actions.
        let plain = Problem::decode("disk full");
        assert_eq!((plain.code.as_str(), plain.message.as_str(), plain.actions.len()), (codes::FAILED, "disk full", 0));
        assert!(!Problem::is_encoded("disk full") && !Problem::is_encoded("{\"code\":\"x\",\"message\":\"m\"}"));
        // A broken packed string is shown as it is rather than lost.
        assert_eq!(Problem::decode("\u{1}kora-problem:{oops").message, "\u{1}kora-problem:{oops");
        assert_eq!(message_of(&s), p.message);
        assert_eq!(message_of("plain"), "plain");
    }

    #[test]
    fn terminals_get_readable_text() {
        let t = plain_text(&sample().encode());
        assert_eq!(t, "Your GitHub sign-in is missing a permission.\nmissing: read:org\nSign in again with more permissions: gh auth refresh -h github.com -s read:org\nRun: gh auth refresh");
        assert!(!t.contains("kora-problem") && !t.contains('{'));
        assert_eq!(plain_text("as it was"), "as it was");
    }

    #[test]
    fn from_json_reads_job_errors_with_extra_fields() {
        let v = json!({"message": "m", "kept": ["x"], "log_path": "/l", "actions": [{"kind": "retry"}]});
        let p = Problem::from_json(&v).unwrap();
        assert_eq!((p.code.as_str(), p.actions.clone()), (codes::FAILED, vec![Action::Retry]));
        assert!(Problem::from_json(&json!({"code": "x"})).is_none());
    }

    #[test]
    fn actions_are_added_once_and_empty_details_are_dropped() {
        let p = Problem::generic("m").with_action(Action::Retry).with_action(Action::Retry).with_details("  ");
        assert_eq!((p.actions.len(), p.details), (1, None));
    }
}
