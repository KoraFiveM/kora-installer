//! The hidden commands of the Windows administrator worker (see `kora_core::helper::worker`):
//!
//! * `kora admin-worker --pipe <name> --parent <pid>`: started elevated by Kora Panel through the
//!   Windows permission prompt; serves the panel's allow-listed requests, then exits.
//! * `kora admin-step run|undo --step <id> --dir <folder>`: one administrator step of an install or
//!   a delete (`kora_core::installer::system_ops`), run by the worker as its child. Prints one
//!   `KORA-ADMIN-RESULT {json}` line at the end.
//!
//! Neither is for people, and neither ever asks for administrator rights itself.

use std::path::{Path, PathBuf};

use clap::Args;
use kora_core::installer::system_ops;

use crate::{AppCmd, Cli, Command, ScheduleCmd};

#[derive(Args, Debug, Clone)]
pub struct AdminStepArgs {
    /// `run` (install) or `undo` (delete).
    #[arg(value_parser = ["run", "undo"])]
    pub action: String,
    /// The install step (see `kora_core::installer::system_ops`).
    #[arg(long, value_parser = ["prepare.db", "build.db", "system.firewall", "system.service", "system.schedule"])]
    pub step: String,
    /// The server folder.
    #[arg(long)]
    pub dir: PathBuf,
    /// Undo: the name in the person's server list.
    #[arg(long, requires_all = ["game_port", "txadmin_port"])]
    pub name: Option<String>,
    #[arg(long)]
    pub game_port: Option<u16>,
    #[arg(long)]
    pub txadmin_port: Option<u16>,
    #[arg(long)]
    pub keep_files: bool,
    #[arg(long)]
    pub keep_db: bool,
}

#[derive(Args, Debug, Clone)]
pub struct AdminWorkerArgs {
    /// The panel's pipe.
    #[arg(long)]
    pub pipe: String,
    /// The panel's process id.
    #[arg(long)]
    pub parent: u32,
}

/// `kora admin-step`: exit code 0 when the step worked; the result line is printed either way.
pub fn admin_step(a: &AdminStepArgs) -> i32 {
    let known = match (&a.name, a.game_port, a.txadmin_port) {
        (Some(n), Some(g), Some(t)) => Some((n.clone(), g, t)),
        _ => None,
    };
    let r = system_ops::run_admin_step(a.action == "undo", &a.step, &a.dir, a.keep_files, a.keep_db, known, &mut |line| println!("{line}"));
    let (code, result) = match r {
        Ok(r) if r.error.is_none() => (0, r),
        Ok(r) => (1, r),
        Err(e) => (1, system_ops::StepResult { error: Some(kora_core::problem::Problem::decode(&e.0)), ..Default::default() }),
    };
    if let Some(p) = &result.error {
        eprintln!("error: {}", p.message);
    }
    match result.to_line() {
        Ok(line) => println!("{line}"),
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    }
    code
}

/// `kora admin-worker`: serve the panel, then exit (never waits for a key: it has no visible window).
pub fn admin_worker(a: &AdminWorkerArgs) -> i32 {
    let args = vec!["--pipe".to_string(), a.pipe.clone(), "--parent".to_string(), a.parent.to_string()];
    match kora_core::helper::winpipe::run_worker(&args, std::sync::Arc::new(worker_validate)) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// The worker's allow-list, checked with this very command-line definition (like the Linux
/// helper's): the arguments must parse as one of the privileged commands the panel uses, and an
/// `admin-step` folder must belong to the person the panel runs as.
pub fn worker_validate(verb: &str, args: &[String]) -> Result<(), String> {
    let sid = kora_core::helper::winpipe::panel_user().map(|u| u.sid.clone()).unwrap_or_default();
    worker_validate_for(verb, args, &|dir| {
        let owner = kora_core::helper::winpipe::path_owner_sid(dir);
        let record = dir.join(".kora").join("request.json").is_file();
        system_ops::check_dir_for(dir, &sid, owner.as_deref(), record)
    })
}

/// [`worker_validate`] with the folder check given (tests).
pub fn worker_validate_for(verb: &str, args: &[String], check_dir: &dyn Fn(&Path) -> Result<(), String>) -> Result<(), String> {
    use clap::Parser;
    let deny = || Err("That command is not one Kora's administrator helper runs.".to_string());
    let argv = ["kora", "--no-elevate", verb].into_iter().map(String::from).chain(args.iter().cloned());
    let Ok(cli) = Cli::try_parse_from(argv) else { return deny() };
    match cli.command {
        Some(Command::AdminStep(a)) if verb == "admin-step" => {
            let allowed = if a.action == "undo" { system_ops::UNDO_STEPS } else { system_ops::RUN_STEPS };
            if !allowed.contains(&a.step.as_str()) {
                return deny();
            }
            check_dir(&a.dir)
        }
        Some(Command::SelfUpdate { version: Some(v), .. })
            if verb == "self-update" && kora_core::app_install::compare_versions(&v, env!("CARGO_PKG_VERSION")) != Some(std::cmp::Ordering::Greater) =>
        {
            Err(format!("Kora's administrator helper only updates Kora to a newer version. To go back to {v}, run `kora self-update --version {v}` yourself from an administrator terminal."))
        }
        Some(Command::SelfUpdate { .. }) if verb == "self-update" => Ok(()),
        Some(Command::Schedule { cmd: ScheduleCmd::Set { .. } | ScheduleCmd::Off { .. } }) if verb == "schedule" => Ok(()),
        Some(Command::App { cmd: AppCmd::Uninstall { .. } }) if verb == "app" => Ok(()),
        Some(Command::Uninstall { .. }) if verb == "uninstall" => Ok(()),
        _ => deny(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(verb: &str, a: &[&str]) -> Result<(), String> {
        let any = |_: &Path| Ok(());
        worker_validate_for(verb, &a.iter().map(|s| s.to_string()).collect::<Vec<_>>(), &any)
    }

    #[test]
    fn the_worker_runs_only_the_listed_administrator_commands() {
        let dir = if cfg!(windows) { r"C:\Users\Jo Smith\kora\my server" } else { "/home/jo/kora/my server" };
        for (verb, args) in [
            ("admin-step", vec!["run", "--step", "system.service", "--dir", dir]),
            ("admin-step", vec!["run", "--step", "prepare.db", "--dir", dir]),
            ("admin-step", vec!["undo", "--step", "system.firewall", "--dir", dir, "--name", "a", "--game-port", "30120", "--txadmin-port", "40120"]),
            ("schedule", vec!["off", "my-server"]),
            ("app", vec!["uninstall", "--yes"]),
            ("uninstall", vec!["--yes"]),
            ("self-update", vec!["--yes"]),
        ] {
            assert!(v(verb, &args).is_ok(), "{verb} {args:?}");
        }
        for (verb, args) in [
            ("install", vec!["--name", "a", "--dir", dir]),
            ("delete", vec!["a", "--yes"]),
            ("service-host", vec!["a"]),
            ("admin-worker", vec!["--pipe", "x", "--parent", "1"]),
            ("admin-step", vec!["undo", "--step", "prepare.db", "--dir", dir]),
            ("admin-step", vec!["run", "--step", "build.config", "--dir", dir]),
            ("admin-step", vec!["run", "--step", "system.service"]),
            ("admin-step", vec!["run", "--step", "system.service", "--dir", dir, "--name", "a"]),
            ("admin-step", vec!["run", "--step", "system.service", "--dir", dir, "; whoami"]),
            ("app", vec!["install"]),
            ("app", vec!["status"]),
            ("schedule", vec!["show", "a"]),
            ("helper", vec!["install"]),
            ("self-update", vec!["--version", "0.0.1", "--yes"]),
            ("cmd.exe", vec!["/c", "whoami"]),
        ] {
            assert!(v(verb, &args).is_err(), "{verb} {args:?}");
        }
    }

    #[test]
    fn admin_step_folders_go_through_the_folder_check() {
        let only = |d: &Path| if d.ends_with("mine") { Ok(()) } else { Err("not yours".to_string()) };
        let a = |d: &str| worker_validate_for("admin-step", &["run".into(), "--step".into(), "system.firewall".into(), "--dir".into(), d.into()], &only);
        assert!(a("/home/jo/mine").is_ok());
        assert_eq!(a("/etc/other"), Err("not yours".into()));
    }

    #[test]
    fn the_hidden_commands_never_ask_for_administrator_rights_themselves() {
        use clap::Parser;
        let kind = |a: &[&str]| crate::command_kind(&Cli::try_parse_from(std::iter::once("kora").chain(a.iter().copied())).unwrap().command);
        assert_eq!(kind(&["admin-worker", "--pipe", "p", "--parent", "1"]), kora_core::elevate::CmdKind::Other);
        assert_eq!(kind(&["admin-step", "run", "--step", "system.service", "--dir", "/x"]), kora_core::elevate::CmdKind::Other);
    }
}
