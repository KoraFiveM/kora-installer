//! `kora helper`: the optional Linux privileged helper and its allow-list of commands.

use crate::cmd::app::AppCmd;
use crate::cmd::schedule::ScheduleCmd;
use crate::{Cli, Command, ToolsCmd};
use clap::{Parser, Subcommand};
use kora_core::elevate;
use kora_core::installer::host;

#[derive(Subcommand)]
pub(crate) enum HelperCmd {
    /// Run the helper (started by its system service; not for people).
    #[command(hide = true)]
    Serve,
    /// Install the helper service for your user account (needs administrator rights; run with sudo).
    Install,
    /// Remove the helper service (needs administrator rights).
    Uninstall,
    /// Say whether the helper is installed and running.
    Status,
}

/// The helper's allow-list, checked with this very command-line definition: the arguments must parse
/// as one of the privileged commands the panel otherwise asks the user to run, and nothing else.
/// The install folder must be one the helper's user could have made themselves (see
/// `kora_core::helper::dirs`).
fn helper_validate(verb: &str, args: &[String]) -> Result<(), String> {
    use kora_core::helper as h;
    let uid = h::server::load_config(std::path::Path::new(h::CONFIG_PATH)).map(|c| c.uid);
    helper_validate_for(verb, args, &|dir| match &uid {
        Ok(u) => h::dirs::check_install_dir(dir, *u, &h::dirs::kora_account()),
        Err(e) => Err(e.clone()),
    })
}

/// [`helper_validate`] with the folder check given (tests).
fn helper_validate_for(verb: &str, args: &[String], check_dir: &dyn Fn(&std::path::Path) -> Result<(), String>) -> Result<(), String> {
    let deny = || Err("That command is not one the helper runs.".to_string());
    let argv = ["kora", "--no-elevate", verb].into_iter().map(String::from).chain(args.iter().cloned());
    let Ok(cli) = Cli::try_parse_from(argv) else { return deny() };
    match cli.command {
        Some(Command::Install(a)) if a.code.is_none() && verb == "install" => {
            // Another catalogue is a list of repositories that root would clone and run setup for.
            if a.catalog.is_some() {
                return Err("The helper does not install from another catalogue (--catalog). Run the install command yourself with sudo to use one.".into());
            }
            match &a.dir {
                Some(dir) => check_dir(dir),
                None => Err("The helper needs the server folder (--dir).".into()),
            }
        }
        // Going back to an older release (which may have known problems) is not something the
        // panel's user may do as root on their own: they run it themselves with sudo.
        Some(Command::SelfUpdate { version: Some(v), .. }) if verb == "self-update" && kora_core::app_install::compare_versions(&v, env!("CARGO_PKG_VERSION")) != Some(std::cmp::Ordering::Greater) => {
            Err(format!("The helper only updates Kora to a newer version. To go back to {v}, run `sudo kora self-update --version {v}` yourself."))
        }
        Some(Command::SelfUpdate { .. }) if verb == "self-update" => Ok(()),
        Some(Command::Schedule { cmd: ScheduleCmd::Set { .. } | ScheduleCmd::Off { .. } }) if verb == "schedule" => Ok(()),
        Some(Command::App { cmd: AppCmd::Install { .. } | AppCmd::Uninstall { .. } }) if verb == "app" => Ok(()),
        Some(Command::Uninstall { .. }) if verb == "uninstall" => Ok(()),
        // Git on Linux (the system package) and starting the database: the only privileged parts of these two.
        Some(Command::Tools { cmd: ToolsCmd::Install { tool: Some(t) } }) if verb == "tools" && t == "git" => Ok(()),
        Some(Command::Check { start_database: true }) if verb == "check" => Ok(()),
        _ => deny(),
    }
}

pub(crate) fn helper(cmd: HelperCmd) -> i32 {
    use kora_core::helper as h;
    let fail = |m: String| {
        eprintln!("error: {m}");
        1
    };
    if cfg!(windows) {
        return fail("the Kora helper is not available on Windows yet; the panel shows the command to run as administrator instead".into());
    }
    let apply = |plan: kora_core::service::Plan| -> i32 {
        for d in plan.descriptions() {
            println!("- {d}");
        }
        match kora_core::service::apply_plan(&plan, &kora_core::service::SystemRunner) {
            Ok(()) => 0,
            Err(e) => fail(e.to_string()),
        }
    };
    match cmd {
        HelperCmd::Serve => match h::server::run(std::sync::Arc::new(helper_validate)) {
            Ok(()) => 0,
            Err(e) => fail(e),
        },
        HelperCmd::Install => {
            let cfg = match h::install::resolve_user(std::env::var("SUDO_UID").ok().as_deref(), std::env::var("KORA_HOME").ok(), &h::install::lookup_account) {
                Ok(c) => c,
                Err(e) => return fail(e),
            };
            let exe = match std::env::current_exe() {
                Ok(e) => e.to_string_lossy().into_owned(),
                Err(e) => return fail(e.to_string()),
            };
            let code = apply(h::install::install_plan(&cfg, &exe));
            if code == 0 {
                println!("The Kora helper is installed for the user '{}'. The panel can now do these actions itself.", cfg.user);
            }
            code
        }
        HelperCmd::Uninstall => {
            let code = apply(h::install::uninstall_plan());
            if code == 0 {
                println!("The Kora helper is removed. The panel will show commands to run as administrator again.");
            }
            code
        }
        HelperCmd::Status => {
            let cfg = h::server::load_config(std::path::Path::new(h::CONFIG_PATH)).ok();
            let client = h::client::SocketClient::default();
            let running = h::client::Helper::available(&client);
            println!("installed: {}", if std::path::Path::new(h::UNIT_PATH).exists() { "yes" } else { "no" });
            println!("running: {}", if running { "yes" } else { "no" });
            if let Some(c) = cfg {
                println!("user: {}", c.user);
            }
            if let Some(v) = client.version() {
                println!("version: {v}");
            }
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_helper_runs_only_the_privileged_commands() {
        let any_dir = |_: &std::path::Path| Ok(());
        let v = |verb: &str, a: &[&str]| helper_validate_for(verb, &a.iter().map(|s| s.to_string()).collect::<Vec<_>>(), &any_dir);
        for (verb, a) in [
            ("install", &["--name", "a", "--dir", "/srv/a", "--mode", "prod", "--service", "--firewall"][..]),
            ("self-update", &["--yes"]),
            ("self-update", &["--yes", "--version", "v99.0.0"]),
            ("schedule", &["set", "dev", "--daily", "04:00", "--as-user"]),
            ("schedule", &["off", "dev"]),
            ("app", &["install", "--yes", "--force"]),
            ("app", &["uninstall", "--yes"]),
            ("uninstall", &["--yes", "--delete-files"]),
            ("uninstall", &["--app", "--yes"]),
        ] {
            assert!(v(verb, a).is_ok(), "{verb} {a:?}");
        }
        for (verb, a) in [
            ("delete", &["srv", "--yes"][..]),
            ("service-host", &["srv"]),
            ("start", &["srv"]),
            ("panel", &[]),
            ("helper", &["install"]),
            ("webhook", &["show", "x"]),
            ("install", &["--code", "KORA-7F3K-92QX"]),
            ("install", &["--name", "a", "--bogus"]),
            ("schedule", &["show", "dev"]),
            ("app", &["status"]),
            ("uninstall", &["--help"]),
            ("self-update", &["--version"]),
            ("--version", &[]),
            ("install", &["--", "x"]),
            // root would clone and set up whatever another catalogue lists
            ("install", &["--name", "a", "--dir", "/srv/a", "--catalog", "https://evil.example/c.json"]),
            ("install", &["--name", "a", "--dir", "/srv/a", "--catalog=/tmp/c.json"]),
            // the folder must be given (so it can be checked)
            ("install", &["--name", "a", "--mode", "prod"]),
            // no going back to an older (or the same) release through the helper
            ("self-update", &["--yes", "--version", "v0.3.4"]),
            ("self-update", &["--yes", "--version", env!("CARGO_PKG_VERSION")]),
            ("self-update", &["--yes", "--version", "nonsense"]),
        ] {
            assert!(v(verb, a).is_err(), "{verb} {a:?}");
        }
        assert!(v("self-update", &["--version", "v0.3.4"]).unwrap_err().contains("sudo kora self-update --version v0.3.4"));
    }

    #[test]
    fn the_helper_installs_only_into_folders_the_check_accepts() {
        let only_home = |d: &std::path::Path| if d.starts_with("/home/sam") { Ok(()) } else { Err(format!("not {}", d.display())) };
        let v = |a: &[&str]| helper_validate_for("install", &a.iter().map(|s| s.to_string()).collect::<Vec<_>>(), &only_home);
        assert!(v(&["--name", "a", "--dir", "/home/sam/kora/a", "--mode", "prod"]).is_ok());
        assert_eq!(v(&["--name", "a", "--dir", "/etc/systemd/system", "--mode", "prod"]), Err("not /etc/systemd/system".into()));
        assert!(v(&["--name", "a", "--dir=/root/x", "--mode", "prod"]).is_err());
    }
}
