//! Choosing where dev-mode forks go: the personal GitHub account or one of the user's organisations.
//!
//! Everything talks to `gh` through [`CommandRunner`] (tests use a fake). The web login flow uses
//! [`LoginSpawner`] so the one-time code parsing is testable without a real `gh`.

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

use super::{valid_name, CommandRunner, GitError, SystemRunner};

/// Errors from this module (same type as the rest of the git layer).
pub type GhError = GitError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerKind {
    User,
    Org,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owner {
    pub login: String,
    pub kind: OwnerKind,
    pub can_fork: bool,
    /// Plain-language reason when `can_fork` is false.
    pub reason: Option<String>,
    /// Organisation role (`admin` / `member`); `None` for the personal account.
    pub role: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GhAuth {
    /// `gh` is not installed.
    Missing,
    LoggedOut,
    LoggedIn { login: String },
}

const PER_PAGE: usize = 100;
const MAX_PAGES: usize = 30;

fn gh(r: &dyn CommandRunner, args: &[&str]) -> Result<super::Output, GhError> {
    r.run("gh", args, None).map_err(|e| GitError::Spawn(format!("gh {}", args.join(" ")), e.to_string()))
}

/// Is `gh` installed, and is somebody logged in?
pub fn gh_auth(runner: &dyn CommandRunner) -> GhAuth {
    let st = match gh(runner, &["auth", "status", "--hostname", "github.com"]) {
        Ok(o) => o,
        Err(_) => return GhAuth::Missing,
    };
    if !st.ok() {
        return GhAuth::LoggedOut;
    }
    match user_login(runner) {
        Ok(l) => GhAuth::LoggedIn { login: l },
        Err(_) => GhAuth::LoggedOut,
    }
}

fn user_login(r: &dyn CommandRunner) -> Result<String, GhError> {
    let o = gh(r, &["api", "user"])?;
    if !o.ok() {
        return Err(GitError::Command { cmd: "gh api user".into(), code: o.code, stderr: o.stderr.trim().to_string() });
    }
    let v: serde_json::Value = serde_json::from_str(o.stdout.trim()).map_err(|_| GitError::Parse("gh api user".into(), o.stdout.clone()))?;
    v.get("login")
        .and_then(|l| l.as_str())
        .filter(|l| valid_name(l))
        .map(str::to_string)
        .ok_or_else(|| GitError::Parse("gh api user".into(), o.stdout.clone()))
}

/// Logins of the organisations the user belongs to (all pages, unsorted).
fn org_logins(r: &dyn CommandRunner) -> Result<Vec<String>, GhError> {
    let mut out: Vec<String> = Vec::new();
    for page in 1..=MAX_PAGES {
        let path = format!("user/orgs?per_page={PER_PAGE}&page={page}");
        let o = gh(r, &["api", &path])?;
        if !o.ok() {
            return Err(GitError::Command { cmd: format!("gh api {path}"), code: o.code, stderr: o.stderr.trim().to_string() });
        }
        let v: serde_json::Value = serde_json::from_str(o.stdout.trim()).map_err(|_| GitError::Parse(format!("gh api {path}"), o.stdout.clone()))?;
        let arr = v.as_array().ok_or_else(|| GitError::Parse(format!("gh api {path}"), o.stdout.clone()))?;
        for item in arr {
            if let Some(l) = item.get("login").and_then(|l| l.as_str()) {
                if valid_name(l) && !out.iter().any(|x| x.eq_ignore_ascii_case(l)) {
                    out.push(l.to_string());
                }
            }
        }
        if arr.len() < PER_PAGE {
            break;
        }
    }
    Ok(out)
}

fn approval_url(org: &str) -> String {
    format!("https://github.com/organizations/{org}/settings/oauth_application_policy")
}

/// `Some(reason)` when the stderr says the organisation blocks the GitHub CLI.
fn blocked_reason(org: &str, stderr: &str) -> Option<String> {
    let s = stderr.to_ascii_lowercase();
    if s.contains("saml") {
        return Some(format!(
            "This organisation requires single sign-on: authorise the GitHub CLI for it at https://github.com/orgs/{org}/sso, then try again"
        ));
    }
    let is403 = s.contains("403") || s.contains("forbidden");
    if (is403 || s.contains("oauth")) && (s.contains("oauth") || s.contains("third-party") || s.contains("access restrictions") || s.contains("application_policy")) {
        return Some(format!("GitHub CLI is not approved for this organisation (an owner must approve it at {})", approval_url(org)));
    }
    None
}

/// Work out whether `org` (a login the user is listed under) can receive forks.
pub fn check_org(r: &dyn CommandRunner, org: &str) -> Owner {
    let mk = |can_fork: bool, reason: Option<String>, role: Option<String>| Owner { login: org.to_string(), kind: OwnerKind::Org, can_fork, reason, role };
    if !valid_name(org) {
        return mk(false, Some("This organisation name cannot be used".into()), None);
    }
    // Membership: state + role.
    let mpath = format!("user/memberships/orgs/{org}");
    let mut role: Option<String> = None;
    match gh(r, &["api", &mpath]) {
        Ok(o) if o.ok() => {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(o.stdout.trim()) {
                role = v.get("role").and_then(|x| x.as_str()).map(str::to_string);
                if let Some(state) = v.get("state").and_then(|x| x.as_str()) {
                    if state != "active" {
                        return mk(false, Some("Your invitation to this organisation has not been accepted yet - accept it on GitHub, or pick another".into()), role);
                    }
                }
            }
        }
        Ok(o) => {
            if let Some(why) = blocked_reason(org, &o.stderr) {
                return mk(false, Some(why), None);
            }
            if o.stderr.contains("404") || o.stderr.contains("Not Found") {
                return mk(false, Some("You are not a member of this organisation (or it does not exist)".into()), None);
            }
            // other failures: carry on with the organisation check
        }
        Err(_) => {}
    }
    let opath = format!("orgs/{org}");
    let mut members_can_create: Option<bool> = None;
    match gh(r, &["api", &opath]) {
        Ok(o) if o.ok() => {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(o.stdout.trim()) {
                members_can_create = v.get("members_can_create_repositories").and_then(|x| x.as_bool());
            }
        }
        Ok(o) => {
            if let Some(why) = blocked_reason(org, &o.stderr) {
                return mk(false, Some(why), role);
            }
        }
        Err(_) => {}
    }
    let is_admin = role.as_deref() == Some("admin");
    if is_admin || members_can_create != Some(false) {
        // `None` (field hidden by permissions) is assumed to be allowed.
        return mk(true, None, role);
    }
    mk(false, Some("Your organisation does not let members create repositories - ask an owner, or pick another".into()), role)
}

/// The personal account first, then the organisations sorted by name, each with `can_fork`.
pub fn list_owners(runner: &dyn CommandRunner) -> Result<Vec<Owner>, GhError> {
    let login = user_login(runner)?;
    let mut orgs = org_logins(runner)?;
    orgs.sort_by_key(|a| a.to_lowercase());
    let mut v = vec![Owner { login, kind: OwnerKind::User, can_fork: true, reason: None, role: None }];
    for o in orgs {
        v.push(check_org(runner, &o));
    }
    Ok(v)
}

/// Pick the fork target. `requested` is the `--github-owner` value (case-insensitive).
/// Without a request the personal account is used only when it is the sole usable option.
pub fn choose_owner(owners: &[Owner], requested: Option<&str>) -> Result<Owner, String> {
    match requested {
        Some(want) => {
            let want = want.trim();
            match owners.iter().find(|o| o.login.eq_ignore_ascii_case(want)) {
                Some(o) if o.can_fork => Ok(o.clone()),
                Some(o) => Err(format!(
                    "You cannot fork into '{}': {}.\n{}",
                    o.login,
                    o.reason.as_deref().unwrap_or("not allowed"),
                    format_owner_list(owners)
                )),
                None => Err(format!("'{want}' is not your GitHub account or an organisation you belong to.\n{}", format_owner_list(owners))),
            }
        }
        None => {
            let usable: Vec<&Owner> = owners.iter().filter(|o| o.can_fork).collect();
            if usable.len() == 1 {
                return Ok(usable[0].clone());
            }
            Err(format!("Choose where to fork with --github-owner <name>.\n{}", format_owner_list(owners)))
        }
    }
}

/// A legal GitHub login: 1-39 characters of `[A-Za-z0-9-]`, no leading or trailing hyphen.
pub fn check_login_syntax(login: &str) -> Result<(), String> {
    let l = login.trim();
    let legal = !l.is_empty()
        && l.len() <= 39
        && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        && !l.starts_with('-')
        && !l.ends_with('-');
    if legal {
        Ok(())
    } else {
        Err(format!(
            "'{login}' is not a valid GitHub user or organisation name (1-39 letters, digits or hyphens, not starting or ending with a hyphen)."
        ))
    }
}

/// Result of checking `--github-owner` before the install starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnerCheck {
    /// Checked against the signed-in account's real list.
    Verified(String),
    /// `gh` is missing or signed out: the install bootstraps and signs in first, then checks again.
    Deferred(String),
}

/// Up-front check of a requested owner. The name must be syntactically valid. With `gh` ready it is
/// verified now (failing with the list of valid owners); without `gh` it is deferred to the install.
pub fn check_requested_owner(runner: &dyn CommandRunner, requested: &str) -> Result<OwnerCheck, String> {
    check_login_syntax(requested)?;
    let want = requested.trim();
    match gh_auth(runner) {
        GhAuth::Missing | GhAuth::LoggedOut => Ok(OwnerCheck::Deferred(want.to_string())),
        GhAuth::LoggedIn { .. } => {
            let all = list_owners(runner).map_err(|e| format!("could not read your GitHub accounts ({e}). Is `gh` installed and logged in? Run `gh auth login`."))?;
            choose_owner(&all, Some(want)).map(|o| OwnerCheck::Verified(o.login))
        }
    }
}

/// Plain-text list of the choices, unusable ones marked.
pub fn format_owner_list(owners: &[Owner]) -> String {
    let mut s = String::from("Available choices:\n");
    for o in owners {
        let what = match o.kind {
            OwnerKind::User => "your personal account".to_string(),
            OwnerKind::Org => match &o.role {
                Some(r) => format!("organisation, you are {r}"),
                None => "organisation".to_string(),
            },
        };
        if o.can_fork {
            s.push_str(&format!("  {} ({what})\n", o.login));
        } else {
            s.push_str(&format!("  {} ({what}) - NOT USABLE: {}\n", o.login, o.reason.as_deref().unwrap_or("not allowed")));
        }
    }
    s
}

/// Rewrite gh's raw fork failure into something a person can act on.
pub fn friendly_fork_error(stderr: &str, target: &str) -> String {
    if let Some(why) = blocked_reason(target, stderr) {
        return why;
    }
    let s = stderr.to_ascii_lowercase();
    if s.contains("forking is disabled") || s.contains("forks are disabled") || s.contains("not allowed to fork") {
        return format!("'{target}' does not allow forking (an owner must allow it in the organisation settings), or choose another owner");
    }
    if s.contains("403") || s.contains("resource not accessible") || s.contains("must be a member") || s.contains("not permitted") || s.contains("permission") {
        return format!("'{target}' refused the fork - you may not be allowed to create repositories there. Ask an owner of '{target}', or pick another owner");
    }
    stderr.trim().to_string()
}

// ---------------------------------------------------------------------------------------------
// Login (web/device flow)
// ---------------------------------------------------------------------------------------------

/// A running `gh auth login`.
pub trait LoginChild: Send {
    /// Next piece of output (stdout or stderr), waiting up to `timeout`.
    /// `Ok(None)` = nothing yet; `Err(())` = the process closed its output.
    #[allow(clippy::result_unit_err)]
    fn next_output(&mut self, timeout: Duration) -> Result<Option<String>, ()>;
    /// Wait for exit; the exit code.
    fn wait(&mut self) -> i32;
    fn kill(&mut self);
    /// After a successful login: set git up to use gh's credentials and read who is logged in.
    fn after_success(&mut self) -> GhAuth;
}

/// Starts `gh` (the real implementation is [`SystemLogin`]).
pub trait LoginSpawner {
    fn spawn(&self, args: &[String]) -> std::io::Result<Box<dyn LoginChild>>;
}

pub struct LoginSession {
    pub code: String,
    /// Where to enter the code.
    pub url: String,
    child: Box<dyn LoginChild>,
}

impl std::fmt::Debug for LoginSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginSession").field("code", &self.code).field("url", &self.url).finish()
    }
}

impl LoginSession {
    /// Block until `gh` exits, then report who is logged in.
    pub fn wait(mut self) -> Result<GhAuth, GhError> {
        let code = self.child.wait();
        if code == 0 {
            Ok(self.child.after_success())
        } else {
            Ok(GhAuth::LoggedOut)
        }
    }
    pub fn cancel(mut self) {
        self.child.kill();
        let _ = self.child.wait();
    }
}

/// Remove ANSI escape sequences and carriage returns.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\u{1b}' {
            if it.peek() == Some(&'[') {
                it.next();
                for n in it.by_ref() {
                    if ('@'..='~').contains(&n) {
                        break;
                    }
                }
            } else {
                it.next();
            }
        } else if c != '\r' {
            out.push(c);
        }
    }
    out
}

/// Find the one-time code (`ABCD-1234`) and the verification URL in gh's login output.
pub fn parse_login_output(text: &str) -> (Option<String>, Option<String>) {
    let clean = strip_ansi(text);
    let mut code = None;
    let mut url = None;
    for line in clean.lines() {
        if code.is_none() {
            if let Some((_, rest)) = line.split_once("one-time code:") {
                let c = rest.trim().trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-');
                if c.len() >= 6 && c.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                    code = Some(c.to_string());
                }
            }
        }
        if url.is_none() {
            if let Some(i) = line.find("https://") {
                let u: String = line[i..].chars().take_while(|c| !c.is_whitespace()).collect();
                if u.contains("github.com") {
                    url = Some(u);
                }
            }
        }
    }
    (code, url)
}

/// Start `gh auth login --web` and return as soon as the code and URL are known.
pub fn begin_login(spawner: &dyn LoginSpawner, admin_org: bool) -> Result<LoginSession, GhError> {
    begin_login_with(spawner, admin_org, Duration::from_secs(30))
}

pub fn begin_login_with(spawner: &dyn LoginSpawner, admin_org: bool, timeout: Duration) -> Result<LoginSession, GhError> {
    let scopes = super::required_scopes(admin_org).join(",");
    let args: Vec<String> = ["auth", "login", "--web", "-h", "github.com", "-p", "https", "-s", &scopes].iter().map(|s| s.to_string()).collect();
    let mut child = spawner.spawn(&args).map_err(|e| GitError::Spawn(format!("gh {}", args.join(" ")), e.to_string()))?;
    let mut buf = String::new();
    let mut waited = Duration::ZERO;
    let step = Duration::from_millis(200);
    loop {
        let (code, url) = parse_login_output(&buf);
        if let (Some(code), Some(url)) = (code.clone(), url) {
            return Ok(LoginSession { code, url, child });
        }
        if waited >= timeout {
            child.kill();
            return Err(GitError::Parse("gh auth login".into(), "no one-time code was shown in time".into()));
        }
        match child.next_output(step) {
            Ok(Some(chunk)) => buf.push_str(&chunk),
            Ok(None) => waited += step,
            Err(()) => {
                // output closed: it may still have given us everything
                let (code, url) = parse_login_output(&buf);
                if let Some(code) = code {
                    return Ok(LoginSession { code, url: url.unwrap_or_else(|| "https://github.com/login/device".into()), child });
                }
                let exit = child.wait();
                return Err(GitError::Command { cmd: "gh auth login".into(), code: exit, stderr: strip_ansi(buf.trim()) });
            }
        }
    }
}

/// The real `gh`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemLogin;

struct SystemChild {
    child: std::process::Child,
    rx: Receiver<String>,
}

impl LoginSpawner for SystemLogin {
    fn spawn(&self, args: &[String]) -> std::io::Result<Box<dyn LoginChild>> {
        use std::io::Read;
        use std::process::Stdio;
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut c = super::system_command("gh", &args, None);
        c.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        c.env("GIT_TERMINAL_PROMPT", "0").env("NO_COLOR", "1").env("GH_PROMPT_DISABLED", "1");
        let mut child = c.spawn()?;
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        fn pump<R: Read + Send + 'static>(mut r: R, tx: std::sync::mpsc::Sender<String>) {
            std::thread::spawn(move || {
                let mut b = [0u8; 1024];
                while let Ok(n) = r.read(&mut b) {
                    if n == 0 || tx.send(String::from_utf8_lossy(&b[..n]).into_owned()).is_err() {
                        break;
                    }
                }
            });
        }
        if let Some(o) = child.stdout.take() {
            pump(o, tx.clone());
        }
        if let Some(e) = child.stderr.take() {
            pump(e, tx.clone());
        }
        drop(tx);
        Ok(Box::new(SystemChild { child, rx }))
    }
}

impl LoginChild for SystemChild {
    fn next_output(&mut self, timeout: Duration) -> Result<Option<String>, ()> {
        match self.rx.recv_timeout(timeout) {
            Ok(s) => Ok(Some(s)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(()),
        }
    }
    fn wait(&mut self) -> i32 {
        // keep draining so gh never blocks on a full pipe
        while self.rx.try_recv().is_ok() {}
        self.child.wait().ok().and_then(|s| s.code()).unwrap_or(-1)
    }
    fn kill(&mut self) {
        let _ = self.child.kill();
    }
    fn after_success(&mut self) -> GhAuth {
        let r = SystemRunner;
        let _ = r.run("gh", &["auth", "setup-git", "--hostname", "github.com"], None);
        gh_auth(&r)
    }
}

// ---------------------------------------------------------------------------------------------
// Creating an organisation
// ---------------------------------------------------------------------------------------------

/// GitHub offers no API to create an organisation: the user does it in the browser.
pub fn org_creation_url() -> &'static str {
    "https://github.com/account/organizations/new?plan=free"
}

/// Poll until an organisation not in `before` shows up. `sleep` is injected so tests do not wait.
pub fn wait_for_new_org(before: &[String], timeout: Duration, runner: &dyn CommandRunner, sleep: &dyn Fn(Duration)) -> Option<String> {
    let poll = Duration::from_secs(3);
    let mut waited = Duration::ZERO;
    loop {
        if let Ok(now) = org_logins(runner) {
            if let Some(n) = now.into_iter().find(|o| !before.iter().any(|b| b.eq_ignore_ascii_case(o))) {
                return Some(n);
            }
        }
        if waited >= timeout {
            return None;
        }
        sleep(poll);
        waited += poll;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::Output;
    use std::cell::RefCell;
    use std::path::Path;

    struct Fake {
        calls: RefCell<Vec<String>>,
        #[allow(clippy::type_complexity)]
        handler: Box<dyn Fn(&str) -> std::io::Result<Output>>,
    }
    impl Fake {
        fn new(h: impl Fn(&str) -> std::io::Result<Output> + 'static) -> Self {
            Fake { calls: RefCell::new(vec![]), handler: Box::new(h) }
        }
    }
    impl CommandRunner for Fake {
        fn run(&self, p: &str, a: &[&str], _: Option<&Path>) -> std::io::Result<Output> {
            let line = format!("{p} {}", a.join(" "));
            self.calls.borrow_mut().push(line.clone());
            (self.handler)(&line)
        }
        fn sleep(&self, _: Duration) {}
    }
    fn ok(s: &str) -> std::io::Result<Output> {
        Ok(Output { code: 0, stdout: s.into(), stderr: String::new() })
    }
    fn fail(s: &str) -> std::io::Result<Output> {
        Ok(Output { code: 1, stdout: String::new(), stderr: s.into() })
    }

    /// A GitHub where `me` belongs to the given (org, role, members_can_create) list.
    fn github(orgs: Vec<(&'static str, &'static str, Option<bool>)>) -> Fake {
        Fake::new(move |l| {
            if l == "gh auth status --hostname github.com" {
                return ok("");
            }
            if l == "gh api user" {
                return ok(r#"{"login":"me","id":1}"#);
            }
            if let Some(rest) = l.strip_prefix("gh api user/orgs?per_page=100&page=") {
                let page: usize = rest.parse().unwrap();
                let items: Vec<String> = orgs.iter().skip((page - 1) * 100).take(100).map(|o| format!(r#"{{"login":"{}"}}"#, o.0)).collect();
                return ok(&format!("[{}]", items.join(",")));
            }
            if let Some(org) = l.strip_prefix("gh api user/memberships/orgs/") {
                return match orgs.iter().find(|o| o.0 == org) {
                    Some(o) => ok(&format!(r#"{{"state":"active","role":"{}"}}"#, o.1)),
                    None => fail("gh: Not Found (HTTP 404)"),
                };
            }
            if let Some(org) = l.strip_prefix("gh api orgs/") {
                return match orgs.iter().find(|o| o.0 == org) {
                    Some(o) => match o.2 {
                        Some(b) => ok(&format!(r#"{{"login":"{org}","members_can_create_repositories":{b}}}"#)),
                        None => ok(&format!(r#"{{"login":"{org}"}}"#)),
                    },
                    None => fail("Not Found"),
                };
            }
            fail("unexpected")
        })
    }

    #[test]
    fn owner_syntax_table() {
        let long_ok = "a".repeat(39);
        let long_bad = "a".repeat(40);
        for good in ["me", "a", "My-Org-2", "a-b", long_ok.as_str(), " me "] {
            assert!(check_login_syntax(good).is_ok(), "{good:?}");
        }
        for bad in ["", " ", "-me", "me-", "a b", "me/x", "me_x", "mé", "me.x", "me@x", long_bad.as_str()] {
            assert!(check_login_syntax(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn bad_syntax_fails_before_touching_gh() {
        let f = github(vec![]);
        assert!(check_requested_owner(&f, "-bad-").unwrap_err().contains("not a valid GitHub"));
        assert!(f.calls.borrow().is_empty());
    }

    #[test]
    fn gh_ready_and_valid_owner_is_verified() {
        let f = github(vec![("Team", "member", Some(true))]);
        assert_eq!(check_requested_owner(&f, "team").unwrap(), OwnerCheck::Verified("Team".into()));
        assert_eq!(check_requested_owner(&f, "ME").unwrap(), OwnerCheck::Verified("me".into()));
    }

    #[test]
    fn gh_ready_and_unknown_owner_fails_early_with_the_list() {
        let f = github(vec![("Team", "member", Some(true))]);
        let e = check_requested_owner(&f, "nobody").unwrap_err();
        assert!(e.contains("'nobody' is not your GitHub account"), "{e}");
        assert!(e.contains("me (your personal account)") && e.contains("Team"), "{e}");
    }

    #[test]
    fn gh_missing_defers() {
        let f = Fake::new(|_| Err(std::io::Error::new(std::io::ErrorKind::NotFound, "no gh")));
        assert_eq!(check_requested_owner(&f, "me").unwrap(), OwnerCheck::Deferred("me".into()));
    }

    #[test]
    fn gh_signed_out_defers() {
        let f = Fake::new(|_| fail("You are not logged into any GitHub hosts"));
        assert_eq!(check_requested_owner(&f, "me").unwrap(), OwnerCheck::Deferred("me".into()));
    }

    #[test]
    fn personal_first_then_sorted_orgs_with_rules() {
        let f = github(vec![("zeta", "member", Some(true)), ("Alpha", "admin", Some(false)), ("locked", "member", Some(false)), ("hidden", "member", None)]);
        let o = list_owners(&f).unwrap();
        let names: Vec<&str> = o.iter().map(|x| x.login.as_str()).collect();
        assert_eq!(names, ["me", "Alpha", "hidden", "locked", "zeta"]);
        assert_eq!(o[0].kind, OwnerKind::User);
        assert!(o[0].can_fork && o[0].role.is_none());
        assert!(o[1].can_fork, "admin may always");
        assert_eq!(o[1].role.as_deref(), Some("admin"));
        assert!(o[2].can_fork && o[2].reason.is_none(), "hidden field assumed allowed");
        assert!(!o[3].can_fork);
        assert!(o[3].reason.as_deref().unwrap().contains("does not let members create repositories"));
        assert!(o[4].can_fork);
    }

    #[test]
    fn paginates_past_100_orgs() {
        let names: Vec<&'static str> = (0..150).map(|i| &*Box::leak(format!("org{i:03}").into_boxed_str())).collect();
        let f = github(names.iter().map(|n| (*n, "member", Some(true))).collect());
        let o = list_owners(&f).unwrap();
        assert_eq!(o.len(), 151);
        assert_eq!(f.calls.borrow().iter().filter(|c| c.contains("user/orgs?")).count(), 2);
    }

    #[test]
    fn exactly_full_page_asks_for_next() {
        let names: Vec<&'static str> = (0..100).map(|i| &*Box::leak(format!("o{i:03}").into_boxed_str())).collect();
        let f = github(names.iter().map(|n| (*n, "member", Some(true))).collect());
        assert_eq!(org_logins(&f).unwrap().len(), 100);
        assert_eq!(f.calls.borrow().len(), 2);
    }

    #[test]
    fn oauth_restricted_org_is_flagged() {
        let f = Fake::new(|l| {
            if l == "gh api user" {
                ok(r#"{"login":"me"}"#)
            } else if l.starts_with("gh api user/orgs") {
                ok(r#"[{"login":"corp"}]"#)
            } else {
                fail("gh: Although you appear to have the correct authorization credentials, the `corp` organization has enabled OAuth App access restrictions, meaning that data access to third-parties is limited. (HTTP 403)")
            }
        });
        let o = list_owners(&f).unwrap();
        assert!(!o[1].can_fork);
        let r = o[1].reason.as_deref().unwrap();
        assert!(r.contains("GitHub CLI is not approved for this organisation"));
        assert!(r.contains("https://github.com/organizations/corp/settings/oauth_application_policy"));
    }

    #[test]
    fn pending_invitation_cannot_fork() {
        let f = Fake::new(|l| {
            if l == "gh api user" {
                ok(r#"{"login":"me"}"#)
            } else if l.starts_with("gh api user/orgs") {
                ok(r#"[{"login":"corp"}]"#)
            } else if l.contains("memberships") {
                ok(r#"{"state":"pending","role":"member"}"#)
            } else {
                ok(r#"{"members_can_create_repositories":true}"#)
            }
        });
        let o = list_owners(&f).unwrap();
        assert!(!o[1].can_fork);
        assert!(o[1].reason.as_deref().unwrap().contains("invitation"));
    }

    #[test]
    fn garbage_output_is_an_error_not_a_panic() {
        let f = Fake::new(|_| ok("<html>nope</html>"));
        assert!(list_owners(&f).is_err());
        let f = Fake::new(|l| if l == "gh api user" { ok(r#"{"login":"me"}"#) } else { ok(r#"{"not":"array"}"#) });
        assert!(list_owners(&f).is_err());
        let f = Fake::new(|_| ok(r#"{"login":"bad name; rm"}"#));
        assert!(list_owners(&f).is_err());
    }

    #[test]
    fn auth_states() {
        let missing = Fake::new(|_| Err(std::io::Error::new(std::io::ErrorKind::NotFound, "program not found")));
        assert_eq!(gh_auth(&missing), GhAuth::Missing);
        let out = Fake::new(|_| fail("You are not logged into any GitHub hosts. To log in, run: gh auth login"));
        assert_eq!(gh_auth(&out), GhAuth::LoggedOut);
        let inn = Fake::new(|l| if l.starts_with("gh auth status") { ok("Logged in") } else { ok(r#"{"login":"me"}"#) });
        assert_eq!(gh_auth(&inn), GhAuth::LoggedIn { login: "me".into() });
    }

    fn owners() -> Vec<Owner> {
        let mk = |l: &str, k, c: bool| Owner { login: l.into(), kind: k, can_fork: c, reason: if c { None } else { Some("because".into()) }, role: None };
        vec![mk("me", OwnerKind::User, true), mk("a", OwnerKind::Org, true), mk("b", OwnerKind::Org, false)]
    }

    #[test]
    fn choose_owner_validation_messages() {
        let o = owners();
        assert_eq!(choose_owner(&o, Some("A")).unwrap().login, "a");
        let e = choose_owner(&o, Some("nobody")).unwrap_err();
        assert!(e.contains("not your GitHub account") && e.contains("me (") && e.contains("b (organisation) - NOT USABLE: because"));
        let e = choose_owner(&o, Some("b")).unwrap_err();
        assert!(e.contains("cannot fork into 'b'") && e.contains("because"));
        // does not guess among several usable
        let e = choose_owner(&o, None).unwrap_err();
        assert!(e.contains("--github-owner"));
        // personal is the sole usable option
        assert_eq!(choose_owner(&o[..1], None).unwrap().login, "me");
        let only_bad = vec![o[0].clone(), o[2].clone()];
        assert_eq!(choose_owner(&only_bad, None).unwrap().login, "me");
    }

    #[test]
    fn friendly_errors() {
        assert!(friendly_fork_error("HTTP 403: Resource not accessible by integration", "corp").contains("refused the fork"));
        assert!(friendly_fork_error("forking is disabled", "corp").contains("does not allow forking"));
        assert!(friendly_fork_error("OAuth App access restrictions (HTTP 403)", "corp").contains("not approved"));
        assert_eq!(friendly_fork_error("weird\n", "corp"), "weird");
    }

    #[test]
    fn parses_login_output_variants() {
        let a = "! First copy your one-time code: ABCD-1234\r\nOpen this URL to continue in your web browser: https://github.com/login/device\r\n";
        assert_eq!(parse_login_output(a), (Some("ABCD-1234".into()), Some("https://github.com/login/device".into())));
        let b = "\u{1b}[33m!\u{1b}[0m First copy your one-time code: \u{1b}[1mWXYZ-9876\u{1b}[0m\n- Open this URL to continue in your web browser: https://github.com/login/device\n";
        assert_eq!(parse_login_output(b), (Some("WXYZ-9876".into()), Some("https://github.com/login/device".into())));
        assert_eq!(parse_login_output("! First copy your one-time code: AB"), (None, None));
        assert_eq!(parse_login_output(""), (None, None));
    }

    struct FakeChild {
        chunks: Vec<String>,
        exit: i32,
        killed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }
    impl LoginChild for FakeChild {
        fn next_output(&mut self, _: Duration) -> Result<Option<String>, ()> {
            if self.chunks.is_empty() {
                Err(())
            } else {
                Ok(Some(self.chunks.remove(0)))
            }
        }
        fn wait(&mut self) -> i32 {
            self.exit
        }
        fn kill(&mut self) {
            self.killed.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        fn after_success(&mut self) -> GhAuth {
            GhAuth::LoggedIn { login: "me".into() }
        }
    }
    struct FakeSpawner {
        chunks: Vec<String>,
        exit: i32,
        killed: std::sync::Arc<std::sync::atomic::AtomicBool>,
        args: RefCell<Vec<String>>,
    }
    impl LoginSpawner for FakeSpawner {
        fn spawn(&self, args: &[String]) -> std::io::Result<Box<dyn LoginChild>> {
            *self.args.borrow_mut() = args.to_vec();
            Ok(Box::new(FakeChild { chunks: self.chunks.clone(), exit: self.exit, killed: self.killed.clone() }))
        }
    }
    fn spawner(chunks: &[&str], exit: i32) -> FakeSpawner {
        FakeSpawner { chunks: chunks.iter().map(|s| s.to_string()).collect(), exit, killed: Default::default(), args: RefCell::new(vec![]) }
    }

    #[test]
    fn begin_login_splits_across_chunks_and_waits() {
        let sp = spawner(&["! First copy your one-", "time code: ABCD-1234\r\n", "Open this URL to continue in your web browser: https://github.com/login/device\r\n"], 0);
        let s = begin_login(&sp, false).unwrap();
        assert_eq!((s.code.as_str(), s.url.as_str()), ("ABCD-1234", "https://github.com/login/device"));
        let a = sp.args.borrow().join(" ");
        assert_eq!(a, "auth login --web -h github.com -p https -s repo,read:org");
        assert_eq!(s.wait().unwrap(), GhAuth::LoggedIn { login: "me".into() });
    }

    #[test]
    fn begin_login_admin_scope_and_failures() {
        let sp = spawner(&["First copy your one-time code: ABCD-1234\nhttps://github.com/login/device\n"], 1);
        let s = begin_login(&sp, true).unwrap();
        assert!(sp.args.borrow().join(" ").ends_with("repo,read:org,admin:org"));
        assert_eq!(s.wait().unwrap(), GhAuth::LoggedOut, "non-zero exit means not logged in");
        let sp = spawner(&["error: something broke\n"], 1);
        assert!(begin_login(&sp, false).is_err());
        let sp = spawner(&["First copy your one-time code: ABCD-1234\n"], 0);
        let s = begin_login(&sp, false).unwrap();
        assert_eq!(s.url, "https://github.com/login/device");
        let killed = sp.killed.clone();
        s.cancel();
        assert!(killed.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn new_org_detected_with_fake_clock() {
        let polls = std::cell::Cell::new(0);
        let f = Fake::new({
            let n = std::rc::Rc::new(std::cell::Cell::new(0));
            move |_| {
                n.set(n.get() + 1);
                if n.get() < 3 { ok(r#"[{"login":"old"}]"# ) } else { ok(r#"[{"login":"old"},{"login":"fresh"}]"#) }
            }
        });
        let slept = RefCell::new(Duration::ZERO);
        let r = wait_for_new_org(&["OLD".to_string()], Duration::from_secs(60), &f, &|d| {
            polls.set(polls.get() + 1);
            *slept.borrow_mut() += d;
        });
        assert_eq!(r.as_deref(), Some("fresh"));
        assert_eq!(polls.get(), 2);
        assert_eq!(*slept.borrow(), Duration::from_secs(6));
    }

    #[test]
    fn new_org_times_out() {
        let f = Fake::new(|_| ok(r#"[{"login":"old"}]"#));
        let slept = RefCell::new(Duration::ZERO);
        let r = wait_for_new_org(&["old".to_string()], Duration::from_secs(10), &f, &|d| *slept.borrow_mut() += d);
        assert_eq!(r, None);
        assert!(*slept.borrow() >= Duration::from_secs(10));
        assert_eq!(org_creation_url(), "https://github.com/account/organizations/new?plan=free");
    }
}
