//! The System check: git, the GitHub CLI, the GitHub sign-in, the database and administrator rights, each
//! in plain words with the fix that belongs to it; plus the operations behind the fix buttons (install a
//! tool, start the database). Everything that touches the computer is behind [`ToolchainEnv`], so the web
//! server and the command line share the logic and tests need no real tools.

use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};

use super::install::github::{GithubState, GithubUser};
use crate::db::{self, CommandRunner, CommandSpec, DetectedService};
use crate::problem::{codes, known, Action, Problem, Tool};
use crate::tools::{self, Source, ToolStatus};

/// The database server, as far as it can be seen from here.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DbState {
    /// Some MariaDB / MySQL service is installed.
    pub installed: bool,
    /// At least one is running.
    pub running: bool,
    /// The service to look at (the running one, else the first stopped one).
    pub name: Option<String>,
    pub version: Option<String>,
    /// Looking failed (for example no `systemctl`): not an error for the person.
    pub unknown: bool,
}

/// Administrator rights.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AdminState {
    /// Kora runs as administrator / root right now.
    pub elevated: bool,
    /// Linux: the optional privileged helper is installed, so Kora can make system changes by itself.
    pub helper: bool,
    /// Kora's panel is set up as a background service.
    pub service: bool,
}

/// Everything the System check and its fix buttons touch.
pub trait ToolchainEnv: Send + Sync {
    /// Git and the GitHub CLI.
    fn tools(&self) -> Vec<ToolStatus>;
    /// Install `tool` (Kora's own copy; on Linux Git the system package, which needs administrator rights).
    /// `say(message, fraction 0..1)`. Errors are packed [`Problem`]s.
    fn install(&self, tool: Tool, say: &mut dyn FnMut(&str, f32)) -> Result<ToolStatus, Problem>;
    fn database(&self) -> DbState;
    /// Start the installed database server; `Err` is a packed [`Problem`] (administrator rights needed, or it did not start).
    fn start_database(&self, say: &mut dyn FnMut(&str)) -> Result<String, Problem>;
    fn admin(&self) -> AdminState;
}

/// The real computer.
pub struct SystemToolchain;

impl ToolchainEnv for SystemToolchain {
    fn tools(&self) -> Vec<ToolStatus> {
        [Tool::Git, Tool::Gh].iter().map(|t| tools::status(&crate::git::SystemRunner, *t)).collect()
    }

    fn install(&self, tool: Tool, say: &mut dyn FnMut(&str, f32)) -> Result<ToolStatus, Problem> {
        if tool == Tool::Git && !cfg!(windows) {
            install_system_git(say)?;
        } else if tool == Tool::Git || tool == Tool::Gh {
            tools::install_real(tool, say).map_err(String::from)?;
        } else {
            return Err(Problem::new(codes::TOOLS_UNSUPPORTED, format!("Kora cannot install {} from here.", tool.label())).into());
        }
        let st = tools::status(&crate::git::SystemRunner, tool);
        if !st.installed {
            return Err(Problem::new(codes::INSTALL_TOOL_MISSING, format!("{} was installed but does not start. Try again, or install it yourself.", tool.label()))
                .with_action(Action::Retry)
                .with_action(Action::open_link(if tool == Tool::Git { known::GIT_URL } else { known::GH_URL }))
                .into());
        }
        Ok(st)
    }

    fn database(&self) -> DbState {
        match db::detect(&db::SystemRunner, db::Os::current()) {
            Ok(found) => db_state(&found),
            Err(_) => DbState { unknown: true, ..DbState::default() },
        }
    }

    fn start_database(&self, say: &mut dyn FnMut(&str)) -> Result<String, Problem> {
        let found = db::detect(&db::SystemRunner, db::Os::current()).map_err(|e| Problem::generic(format!("Kora could not look for the database server: {e}")).to_string())?;
        start_database_with(&db::SystemRunner, &found, crate::elevate::is_elevated(), cfg!(windows), Duration::from_secs(45), say).map_err(String::from)
    }

    fn admin(&self) -> AdminState {
        AdminState { elevated: crate::elevate::is_elevated(), helper: crate::privsvc::service_ready(), service: crate::monitor::panel_service::service_installed(crate::service::Platform::current()) }
    }
}

/// Git on Linux: the system package manager, which needs root (the caller checked or went through the helper).
fn install_system_git(say: &mut dyn FnMut(&str, f32)) -> Result<(), Problem> {
    let tools = crate::git::Tools { git: None, gh: None };
    let plan = crate::git::plan_bootstrap(crate::git::Os::current(), &tools).map_err(|e| e.to_string())?;
    if plan.is_empty() {
        return Err(Problem::new(codes::TOOLS_UNSUPPORTED, "Git comes from your system's package manager on this computer.").with_action(Action::open_link(known::GIT_URL)).into());
    }
    if !crate::elevate::is_elevated() {
        return Err(Problem::new(codes::NEEDS_ELEVATION, "Installing Git needs administrator rights.")
            .with_action(Action::InstallAdminService)
            .with_action(Action::show_command(&["kora", "tools", "install", "git"], true))
            .into());
    }
    for (i, s) in plan.iter().enumerate() {
        say(&s.description, i as f32 / plan.len() as f32);
        let args: Vec<&str> = s.args.iter().map(String::as_str).collect();
        let o = crate::git::CommandRunner::run(&crate::git::SystemRunner, &s.program, &args, None).map_err(|e| e.to_string())?;
        if !o.ok() {
            return Err(Problem::new(codes::INSTALL_TOOL_MISSING, format!("This did not work: {}.", s.description)).with_details(o.stderr.trim()).with_action(Action::Retry).into());
        }
    }
    Ok(())
}

/// The database state for a list of detected services.
pub fn db_state(found: &[DetectedService]) -> DbState {
    let pick = found.iter().find(|s| s.running).or_else(|| found.first());
    DbState { installed: !found.is_empty(), running: found.iter().any(|s| s.running), name: pick.map(|s| s.name.clone()), version: pick.and_then(|s| s.version.clone()), unknown: false }
}

/// Start the first stopped database service in `found` and wait until it is running.
/// Needs administrator rights (`elevated`); without them: the `needs_elevation` Problem (on Windows with
/// [`Action::InstallAdminService`] (install the service), also the command to run).
pub fn start_database_with(runner: &dyn CommandRunner, found: &[DetectedService], elevated: bool, windows: bool, wait: Duration, say: &mut dyn FnMut(&str)) -> Result<String, Problem> {
    let Some(svc) = found.iter().find(|s| !s.running) else {
        return match found.first() {
            Some(s) => Ok(format!("The database server {} is already running.", s.name)),
            None => Err(Problem::new(codes::INSTALL_DB_MISSING, "No database server is installed on this computer. Kora can install one when you set up a server.")),
        };
    };
    let name = svc.name.clone();
    let (prog, args): (&str, Vec<&str>) = if windows { ("sc.exe", vec!["start", &name]) } else { ("systemctl", vec!["start", &name]) };
    if !elevated {
        let shown: Vec<&str> = if windows { vec!["net", "start", &name] } else { vec!["systemctl", "start", &name] };
        return Err(Problem::new(codes::NEEDS_ELEVATION, format!("Starting the database server {name} needs administrator rights."))
            .with_action(Action::InstallAdminService)
            .with_action(Action::show_command(&shown, true)));
    }
    say(&format!("Starting the database server {name}"));
    let fail = |why: String| {
        Problem::new(codes::DB_START_FAILED, format!("The database server {name} did not start."))
            .with_details(why)
            .with_action(Action::StartDatabase)
            .with_action(Action::OpenLog { job: None, path: None })
    };
    let out = runner.run(&CommandSpec::new(prog, &args)).map_err(|e| fail(e.to_string()))?;
    // Windows: 1056 = "an instance of the service is already running".
    if !out.ok() && !(windows && out.status == 1056) {
        let why = format!("{}{}", out.stderr, out.stdout);
        return Err(fail(why.lines().filter(|l| !l.trim().is_empty()).take(4).collect::<Vec<_>>().join(" | ")));
    }
    let until = Instant::now() + wait;
    loop {
        let now = db::detect(runner, if windows { db::Os::Windows } else { db::Os::Linux }).unwrap_or_default();
        if now.iter().any(|s| s.name == name && s.running) {
            return Ok(format!("The database server {name} is running."));
        }
        if Instant::now() >= until {
            return Err(fail("it was told to start but is not running yet".into()));
        }
        std::thread::sleep(Duration::from_millis(if cfg!(test) { 1 } else { 500 }));
    }
}

// ----------------------------------------------------------------------------------------------
// The check itself
// ----------------------------------------------------------------------------------------------

/// How one line of the System check stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckState {
    /// Fine.
    Ok,
    /// Something to fix: the `problem` has the fix buttons.
    Problem,
    /// Nothing wrong, just good to know (not installed yet, optional).
    Info,
}

/// One line of the System check.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CheckItem {
    /// `git`, `gh`, `github`, `database`, `admin`, `service`.
    pub id: &'static str,
    pub label: &'static str,
    pub state: CheckState,
    /// The state in plain words.
    pub summary: String,
    /// What to do about it, with the fix buttons (`null` when fine).
    pub problem: Option<Problem>,
}

fn item(id: &'static str, label: &'static str, state: CheckState, summary: impl Into<String>, problem: Option<Problem>) -> CheckItem {
    CheckItem { id, label, state, summary: summary.into(), problem }
}

fn where_from(s: &ToolStatus) -> &'static str {
    match s.source {
        Some(Source::Private) => "Kora's own copy",
        _ => "found on this computer",
    }
}

fn tool_item(id: &'static str, label: &'static str, s: &ToolStatus, missing: Problem) -> CheckItem {
    if s.installed {
        let v = s.version.as_deref().map(|v| format!(" {v}")).unwrap_or_default();
        return item(id, label, CheckState::Ok, format!("Installed{v} ({}).", where_from(s)), None);
    }
    if s.broken {
        let mut p = Problem::new(codes::INSTALL_TOOL_MISSING, format!("{} is on this computer but does not start.", s.label));
        if s.can_install {
            p.push_action(Action::InstallTool { tool: s.tool });
        }
        return item(id, label, CheckState::Problem, "Found, but it does not run.", Some(p));
    }
    item(id, label, CheckState::Problem, "Not installed.", Some(missing))
}

/// The System check from what was found. Pure: the tests feed it states.
pub fn build_check(windows: bool, tools: &[ToolStatus], github: &GithubUser, db: &DbState, admin: &AdminState) -> Vec<CheckItem> {
    let find = |t: Tool| tools.iter().find(|s| s.tool == t);
    let mut items = Vec::new();
    if let Some(s) = find(Tool::Git) {
        items.push(tool_item("git", "Git", s, known::git_missing()));
    }
    let gh_ok = find(Tool::Gh).is_some_and(|s| s.installed);
    if let Some(s) = find(Tool::Gh) {
        items.push(tool_item("gh", "GitHub CLI", s, known::gh_missing()));
    }
    items.push(match github.state {
        GithubState::Ready => item("github", "GitHub sign-in", CheckState::Ok, format!("Signed in{}.", github.user.as_deref().map(|u| format!(" as {u}")).unwrap_or_default()), None),
        GithubState::GhMissing => item("github", "GitHub sign-in", CheckState::Info, "Needs the GitHub CLI first.", None),
        GithubState::SignedOut => item("github", "GitHub sign-in", CheckState::Problem, "Not signed in.", github.problem()),
        GithubState::MissingScopes => item("github", "GitHub sign-in", CheckState::Problem, "Signed in, but without the permissions Kora needs.", github.problem()),
    });
    // Without a working gh the sign-in cannot be looked at.
    if !gh_ok && github.state == GithubState::SignedOut {
        if let Some(last) = items.last_mut() {
            *last = item("github", "GitHub sign-in", CheckState::Info, "Needs the GitHub CLI first.", None);
        }
    }
    items.push(match (db.installed, db.running, db.unknown) {
        (_, _, true) => item("database", "Database", CheckState::Info, "Kora could not look for a database server here.", None),
        (false, _, _) => item("database", "Database", CheckState::Info, "No database server found. Kora can set one up for you when you install a server.", None),
        (true, true, _) => item("database", "Database", CheckState::Ok, format!("{} is running.", db_name(db)), None),
        (true, false, _) => {
            item("database", "Database", CheckState::Problem, format!("{} is installed but not running.", db_name(db)), Some(known::db_stopped(db.name.as_deref().unwrap_or("the database"))))
        }
    });
    let who = if windows { "administrator" } else { "root" };
    items.push(if admin.elevated {
        item("admin", "Administrator rights", CheckState::Ok, format!("Kora is running as {who}."), None)
    } else if admin.helper {
        item("admin", "Administrator rights", CheckState::Ok, "Kora runs with your normal rights, and the administrator helper is installed, so system changes happen without a prompt.", None)
    } else {
        item(
            "admin",
            "Administrator rights",
            CheckState::Info,
            if windows { "Kora runs with your normal rights. When something needs administrator rights, Windows asks you for administrator permission on this computer: once for a whole install, once for other actions." } else { "Kora runs with your normal rights. For the steps that need root it shows you the command to run." },
            None,
        )
    });
    items.push(if admin.service {
        item("service", "Background service", CheckState::Ok, "The panel starts with this computer and keeps running in the background.", None)
    } else {
        item("service", "Background service", CheckState::Info, "Not set up: the panel runs only while it is open.", None)
    });
    items
}

fn db_name(d: &DbState) -> String {
    match (&d.name, &d.version) {
        (Some(n), Some(v)) => format!("{n} {v}"),
        (Some(n), None) => n.clone(),
        _ => "The database server".into(),
    }
}

/// `{items: [...], ok: bool}`; `ok` is true when no line needs fixing.
pub fn check_json(items: &[CheckItem]) -> Value {
    json!({"items": items, "ok": items.iter().all(|i| i.state != CheckState::Problem)})
}

/// Run the whole System check against `env` and the GitHub state.
pub fn system_check(env: &dyn ToolchainEnv, github: &GithubUser) -> Vec<CheckItem> {
    build_check(cfg!(windows), &env.tools(), github, &env.database(), &env.admin())
}

#[cfg(test)]
mod tests;
