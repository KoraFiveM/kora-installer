//! `kora tools status|install` and `kora check`: the command-line side of the panel's System check
//! (`Op::ToolsStatus`, `Op::ToolInstall`, `Op::SystemCheck`, `Op::DbStart`). The logic lives in
//! `kora_core::api::toolchain`; this only prints.

use clap::Subcommand;
use kora_core::api::install::github::{GithubEnv, SystemGithub};
use kora_core::api::toolchain::{system_check, CheckState, SystemToolchain, ToolchainEnv};
use kora_core::problem::{plain_text, Tool};
use kora_core::tools::Source;

fn tool_from(name: &str) -> Option<Tool> {
    match name {
        "git" => Some(Tool::Git),
        "gh" => Some(Tool::Gh),
        _ => None,
    }
}

fn describe(s: &kora_core::tools::ToolStatus) -> String {
    let label = if s.tool == Tool::Git { "Git" } else { "GitHub CLI" };
    if !s.installed {
        return if s.broken { format!("{label}: found, but it does not run") } else { format!("{label}: not installed") };
    }
    let from = match s.source {
        Some(Source::Private) => "Kora's own copy",
        _ => "on this computer",
    };
    let at = s.path.as_deref().map(|p| format!(" ({p})")).unwrap_or_default();
    format!("{label}: {} ({from}){at}", s.version.as_deref().unwrap_or("installed"))
}

/// `kora tools status`
pub fn tools_status(env: &dyn ToolchainEnv) -> i32 {
    let all = env.tools();
    for s in &all {
        println!("{}", describe(s));
    }
    i32::from(all.iter().any(|s| !s.installed))
}

/// `kora tools install [git|gh|all]`: Kora's own copy, checked against a pinned checksum; no
/// administrator rights (Git on Linux comes from the system package and needs root).
pub fn tools_install(env: &dyn ToolchainEnv, which: Option<&str>) -> i32 {
    let wanted: Vec<Tool> = match which.unwrap_or("all") {
        "all" => env.tools().iter().filter(|s| !s.installed && s.can_install).map(|s| s.tool).collect(),
        name => match tool_from(name) {
            Some(t) => vec![t],
            None => {
                eprintln!("error: Kora can install `git` or `gh` (or `all`), not `{name}`.");
                return 2;
            }
        },
    };
    if wanted.is_empty() {
        println!("Nothing to install: Git and the GitHub CLI are already here.");
        return 0;
    }
    let mut code = 0;
    for t in wanted {
        let mut last = String::new();
        match env.install(t, &mut |m, _f| {
            if m != last {
                println!("{m}");
                last = m.to_string();
            }
        }) {
            Ok(s) => println!("{}", describe(&s)),
            Err(e) => {
                eprintln!("error: {}", plain_text(&e));
                code = 1;
            }
        }
    }
    code
}

/// `kora check [--start-database]`: the System check in words. Exit code 1 when something needs fixing.
pub fn check(env: &dyn ToolchainEnv, github: &dyn GithubEnv, start_database: bool) -> i32 {
    if start_database {
        return match env.start_database(&mut |m| println!("{m}")) {
            Ok(m) => {
                println!("{m}");
                0
            }
            Err(e) => {
                eprintln!("error: {}", plain_text(&e));
                1
            }
        };
    }
    let items = system_check(env, &github.auth());
    let mut fix = 0;
    for i in &items {
        let mark = match i.state {
            CheckState::Ok => "ok  ",
            CheckState::Info => "info",
            CheckState::Problem => {
                fix += 1;
                "FIX "
            }
        };
        println!("[{mark}] {}: {}", i.label, i.summary);
        if let Some(p) = &i.problem {
            for l in plain_text(&p.encode()).lines().skip(1) {
                println!("       {l}");
            }
        }
    }
    if fix > 0 {
        println!(
            "\n{fix} thing{} to fix. `kora tools install` sets up Git and the GitHub CLI; `kora check --start-database` starts the database; `gh auth login` signs in to GitHub.",
            if fix == 1 { "" } else { "s" }
        );
    }
    i32::from(fix > 0)
}

pub fn run_tools_status() -> i32 {
    tools_status(&SystemToolchain)
}
pub fn run_tools_install(which: Option<&str>) -> i32 {
    tools_install(&SystemToolchain, which)
}
pub fn run_check(start_database: bool) -> i32 {
    check(&SystemToolchain, &SystemGithub, start_database)
}

#[derive(Subcommand)]
pub(crate) enum ToolsCmd {
    /// Show which Git and GitHub CLI Kora uses (the one on this computer, or its own copy).
    Status,
    /// Download Kora's own copy of Git (Windows) and the GitHub CLI, checked against a pinned checksum. No administrator rights, nothing else on the computer changes.
    Install {
        /// `git`, `gh` or `all` (the default: whatever is missing).
        #[arg(value_name = "TOOL")]
        tool: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use kora_core::api::install::github::GithubUser;
    use kora_core::api::toolchain::{AdminState, DbState};
    use kora_core::tools::ToolStatus;
    use std::sync::Mutex;

    struct Fake {
        git_ok: Mutex<bool>,
        installs: Mutex<Vec<Tool>>,
    }
    fn st(t: Tool, ok: bool) -> ToolStatus {
        ToolStatus {
            tool: t,
            name: if t == Tool::Git { "git" } else { "gh" },
            label: t.label(),
            installed: ok,
            version: ok.then(|| "1.0".into()),
            source: ok.then_some(Source::Private),
            path: None,
            broken: false,
            can_install: true,
        }
    }
    impl ToolchainEnv for Fake {
        fn tools(&self) -> Vec<ToolStatus> {
            vec![st(Tool::Git, *self.git_ok.lock().unwrap()), st(Tool::Gh, false)]
        }
        fn install(&self, tool: Tool, say: &mut dyn FnMut(&str, f32)) -> Result<ToolStatus, String> {
            say("Downloading", 0.5);
            self.installs.lock().unwrap().push(tool);
            Ok(st(tool, true))
        }
        fn database(&self) -> DbState {
            DbState::default()
        }
        fn start_database(&self, _: &mut dyn FnMut(&str)) -> Result<String, String> {
            Ok("started".into())
        }
        fn admin(&self) -> AdminState {
            AdminState::default()
        }
    }
    struct Gh;
    impl GithubEnv for Gh {
        fn auth(&self) -> GithubUser {
            GithubUser::gh_missing()
        }
        fn owners(&self) -> Result<Vec<kora_core::api::install::github::OwnerChoice>, String> {
            Ok(vec![])
        }
        fn begin_login(&self) -> Result<kora_core::api::install::github::LoginFlow, String> {
            Err("no".into())
        }
        fn resolve_owner(&self, _: Option<&str>) -> Result<String, String> {
            Err("no".into())
        }
    }

    #[test]
    fn install_all_installs_only_what_is_missing() {
        let f = Fake { git_ok: Mutex::new(true), installs: Mutex::new(vec![]) };
        assert_eq!(tools_install(&f, None), 0);
        assert_eq!(*f.installs.lock().unwrap(), vec![Tool::Gh]);
    }

    #[test]
    fn a_tool_name_is_checked() {
        let f = Fake { git_ok: Mutex::new(true), installs: Mutex::new(vec![]) };
        assert_eq!(tools_install(&f, Some("node")), 2);
        assert_eq!(tools_install(&f, Some("git")), 0);
        assert_eq!(*f.installs.lock().unwrap(), vec![Tool::Git]);
    }

    #[test]
    fn status_and_check_exit_codes_say_whether_something_is_missing() {
        let f = Fake { git_ok: Mutex::new(true), installs: Mutex::new(vec![]) };
        assert_eq!(tools_status(&f), 1, "gh is missing");
        assert_eq!(check(&f, &Gh, false), 1, "gh is a thing to fix");
        assert_eq!(check(&f, &Gh, true), 0, "start-database goes through the env");
    }
}
