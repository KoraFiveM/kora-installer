//! `kora` command line entry point.

use std::io::IsTerminal;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use kora_core::elevate;
use kora_core::installer::{self, host, registry};

use crate::cmd::app::{app_install, app_status, app_uninstall, AppCmd};
use crate::cmd::desktop::DesktopCmd;
use crate::cmd::github::{github_owners, GithubCmd};
use crate::cmd::helper::{helper, HelperCmd};
use crate::cmd::install::{install, InstallArgs};
use crate::cmd::panel::{panel, PanelArgs};
use crate::cmd::reach::ReachArgs;
use crate::cmd::resources::{add_asset, add_resources, AddAssetArgs, AddResourcesArgs};
use crate::cmd::schedule::{schedule, ScheduleCmd};
use crate::cmd::server::{control, delete, status};
use crate::cmd::store::{AuditArgs, StoreCmd};
use crate::cmd::toolchain::ToolsCmd;
use crate::cmd::uninstall::uninstall;
use crate::cmd::update::{update, UpdateArgs};
use crate::cmd::webhook::{webhook, WebhookCmd};
use crate::cmd::{admin_worker, bare, branch, keys_cmd, resource, toolchain};

#[derive(Parser)]
#[command(name = "kora", version, about = "Kora installer and server manager")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// Do not ask Windows/sudo for administrator permission (for developers and CI).
    #[arg(long, global = true)]
    no_elevate: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Install a new Kora server. With no options this opens the panel's install page in your browser; with options it installs without asking (for scripts).
    Install(Box<InstallArgs>),
    /// Open Kora Panel in your browser (the same as running `kora` with nothing after it).
    Manager,
    /// Open the Kora Panel: install, manage and monitor servers in your web browser (this computer only unless remote access is set up).
    Panel(PanelArgs),
    /// Update a server: preview, snapshot, apply and roll back (`--auto` is for schedulers).
    Update(UpdateArgs),
    /// Add compatibility, bridge or creator resources (or any catalog resource) to an installed server.
    AddResources(AddResourcesArgs),
    /// Import paid (Cfx.re escrowed) assets from the .zip files you downloaded from the Cfx.re Portal.
    #[command(
        after_help = "Download the assets you bought from https://portal.cfx.re/assets/granted-assets (Assets → Granted Assets), signed in with the same Cfx.re account that owns this server's licence key, then pass the .zip files here as they are."
    )]
    AddAsset(AddAssetArgs),
    /// Show registered servers.
    Status,
    /// Update the kora program itself to the newest stable release (checksum-verified).
    SelfUpdate {
        /// Only say whether a newer version exists.
        #[arg(long)]
        check: bool,
        /// Do not ask for confirmation.
        #[arg(long, short = 'y')]
        yes: bool,
        /// Install exactly this release (also an older one or a pre-release), e.g. v0.2.1.
        #[arg(long, value_name = "vX.Y.Z")]
        version: Option<String>,
    },
    /// Can players reach a server? A checklist (running, firewall, router, internet) and the fixes.
    Reach(ReachArgs),
    /// Discord notifications for update reports.
    Webhook {
        #[command(subcommand)]
        cmd: WebhookCmd,
    },
    /// Delete a server: removes what the installer created (files, database + user, registry entry).
    Delete {
        /// Registry name, or the server folder.
        server: String,
        /// Keep the server folder and files.
        #[arg(long)]
        keep_files: bool,
        /// Keep the database and its user.
        #[arg(long)]
        keep_db: bool,
        /// Do not ask for confirmation.
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Show or change when a development server updates itself.
    Schedule {
        #[command(subcommand)]
        cmd: ScheduleCmd,
    },
    /// The Kora shortcuts (Start Menu / desktop launcher) that open the panel, and tidying up the old machine-wide app.
    App {
        #[command(subcommand)]
        cmd: AppCmd,
    },
    /// Who changed what (from the panel and from `kora` commands), newest first.
    Audit(AuditArgs),
    /// Kora's own data file: where it is, a backup or restore of it, and the way back to the old JSON files.
    Store {
        #[command(subcommand)]
        cmd: StoreCmd,
    },
    /// Saved Cfx.re licence keys: list, add, rename, replace, remove, and change a server's key.
    Keys {
        #[command(subcommand)]
        cmd: keys_cmd::KeysCmd,
    },
    /// What Kora remembers between installs (your Cfx.re name and GitHub owner).
    Profile {
        #[command(subcommand)]
        cmd: keys_cmd::ProfileCmd,
    },
    /// Development servers: see and change which branch each resource is on, and go back to the normal version.
    Branch {
        #[command(subcommand)]
        cmd: branch::BranchCmd,
    },
    /// Development servers: list and try pull requests on a resource.
    Pr {
        #[command(subcommand)]
        cmd: branch::PrCmd,
    },
    /// One resource: what it is, which version it is on, and whether a restart is needed.
    Resource {
        #[command(subcommand)]
        cmd: resource::ResourceCmd,
    },
    /// GitHub helpers for development mode.
    Github {
        #[command(subcommand)]
        cmd: GithubCmd,
    },
    /// Git and the GitHub CLI: which copy Kora uses, and installing its own (no administrator rights needed).
    Tools {
        #[command(subcommand)]
        cmd: ToolsCmd,
    },
    /// Check this computer: Git, the GitHub CLI, the GitHub sign-in, the database and administrator rights, each with what to do about it.
    Check {
        /// Start the database server if it is installed but not running (needs administrator rights).
        #[arg(long)]
        start_database: bool,
    },
    /// Run a server's FXServer (used by the Windows service and supervisors).
    #[command(hide = true)]
    ServiceHost {
        /// Registry name, or the server folder (absolute path).
        name: String,
    },
    /// Start a server's background service.
    Start {
        /// Registry name.
        server: String,
    },
    /// Stop a server's background service.
    Stop {
        /// Registry name.
        server: String,
    },
    /// Restart a server's background service.
    Restart {
        /// Registry name.
        server: String,
    },
    /// Remove the system changes Kora made (services, update timers, firewall rules, service account, server list entry).
    /// Asks before each item; server folders and databases are kept unless you say otherwise.
    Uninstall {
        /// Only this server (registry name or folder); default: every registered server.
        server: Option<String>,
        /// Show everything that would be removed and change nothing.
        #[arg(long)]
        dry_run: bool,
        /// Do not ask: remove the system changes, keep server folders and databases.
        #[arg(long, short = 'y')]
        yes: bool,
        /// Also delete the server folders (only what Kora created in them).
        #[arg(long)]
        delete_files: bool,
        /// Also drop the databases and database users Kora created.
        #[arg(long)]
        delete_databases: bool,
        /// Remove only the OLD machine-wide Kora app (its shortcuts, PATH entry, the installed copy); servers and databases are never touched. (`kora app uninstall` removes Kora's own shortcuts.)
        #[arg(long)]
        app: bool,
    },
    /// The optional privileged helper: lets the panel do the few things that need administrator rights.
    Helper {
        #[command(subcommand)]
        cmd: HelperCmd,
    },
    /// Windows: Kora Panel's short-lived administrator helper (started through the permission prompt).
    #[command(hide = true)]
    AdminWorker(admin_worker::AdminWorkerArgs),
    /// One administrator step of an install or delete, run by that helper.
    #[command(hide = true)]
    AdminStep(admin_worker::AdminStepArgs),
    /// What the Kora desktop app (tray icon and window) asks of kora. Run by the app, never by a person.
    #[command(hide = true)]
    Desktop {
        #[command(subcommand)]
        cmd: DesktopCmd,
    },
    /// Start (or install) Kora's own MariaDB on Windows, run by that helper.
    #[command(hide = true)]
    AdminDb(admin_worker::AdminDbArgs),
}

fn command_kind(cmd: &Option<Command>) -> elevate::CmdKind {
    use elevate::CmdKind as K;
    match cmd {
        // Bare `kora`, `kora manager` and a flagless `kora install` only open the panel: never elevated.
        None | Some(Command::Manager) => K::Other,
        Some(Command::Install(a)) if !a.any_given() => K::Other,
        Some(Command::Install(a)) => K::Install { dry_run: a.dry_run },
        Some(Command::Update(a)) => K::Update { auto: a.auto },
        Some(Command::Status | Command::Schedule { cmd: ScheduleCmd::Show { .. } }) => K::Status,
        Some(Command::Schedule { .. }) => K::Restart,
        Some(Command::Delete { .. }) => K::Delete,
        // The panel runs as the user; privileged actions report the command to run instead.
        Some(
            Command::ServiceHost { .. }
            | Command::Github { .. }
            | Command::Tools { .. }
            | Command::Check { .. }
            | Command::Keys { .. }
            | Command::Audit(_)
            | Command::Store { .. }
            | Command::Profile { .. }
            | Command::Branch { .. }
            | Command::Pr { .. }
            | Command::Resource { .. }
            | Command::Webhook { .. }
            | Command::Desktop { .. }
            | Command::Reach(_)
            | Command::SelfUpdate { .. }
            | Command::Panel(_),
        ) => K::Other,
        // Already elevated (the worker) or run by it: never relaunched.
        Some(Command::AdminWorker(_) | Command::AdminStep(_) | Command::AdminDb(_)) => K::Other,
        Some(Command::Helper { cmd: HelperCmd::Install | HelperCmd::Uninstall }) => K::Uninstall { dry_run: false },
        Some(Command::Helper { .. }) => K::Other,
        // Writes into the server folder, like a manual update: same privilege rules.
        Some(Command::AddResources(_)) => K::Update { auto: false },
        Some(Command::AddAsset(_)) => K::Update { auto: false },
        Some(Command::Start { .. }) => K::Start,
        Some(Command::Stop { .. }) => K::Stop,
        Some(Command::Restart { .. }) => K::Restart,
        Some(Command::Uninstall { dry_run, .. }) => K::Uninstall { dry_run: *dry_run },
        Some(Command::App { cmd }) => match cmd {
            // The per-user shortcuts need no administrator rights; the old machine-wide app, if there, is
            // removed only when this is already run as administrator (`kora uninstall --app` asks for it).
            AppCmd::Install { .. } | AppCmd::Uninstall { .. } | AppCmd::Status => K::Status,
        },
    }
}

/// Linux: why this command needs root (empty = it does not). The wizard decides for itself later.
fn admin_reasons_for_command(cmd: &Option<Command>) -> Vec<String> {
    use kora_core::service::ServiceState;
    let one = |s: &str| vec![s.to_string()];
    match cmd {
        Some(Command::Install(a)) => {
            let headless = a.any_given();
            if !headless {
                return vec![];
            }
            a.as_ref().clone().into_request().map(|r| elevate::admin_reasons(&r)).unwrap_or_default()
        }
        Some(Command::Start { server } | Command::Stop { server } | Command::Restart { server }) => match host::service_state(server) {
            Ok(ServiceState::NotInstalled) | Err(_) => vec![],
            Ok(_) => one("control the server's background service"),
        },
        Some(Command::Delete { server, .. }) => {
            let reg = registry::Registry::load().unwrap_or_default();
            let dir = reg.find(server).map(|e| e.dir.clone()).unwrap_or_else(|| PathBuf::from(server));
            match installer::load_created_from(&dir) {
                Some(c) if c.service_unit.is_some() || c.firewall || c.update_timer => one("remove the server's background service and firewall rules"),
                _ => vec![],
            }
        }
        Some(Command::Schedule { cmd: ScheduleCmd::Set { .. } | ScheduleCmd::Off { .. } }) => one("change the automatic-update schedule"),
        Some(Command::Uninstall { dry_run: false, app: true, .. }) => one("remove the old Kora app (the command, shortcuts and the installed program)"),
        Some(Command::Uninstall { dry_run: false, .. }) => one("remove the services, firewall rules and accounts Kora set up"),
        Some(Command::Helper { cmd: HelperCmd::Install | HelperCmd::Uninstall }) => one("install or remove the Kora helper service"),
        _ => vec![],
    }
}

/// The arguments for the elevated copy: the original ones, plus the folder when it was only implied
/// (decided here, before `sudo` changes HOME to root's). Only for an install with options: a bare
/// `kora install` is the wizard, and an extra `--dir` would turn it into an install without questions.
fn relaunch_args(cmd: &Option<Command>, original: Vec<std::ffi::OsString>) -> Vec<std::ffi::OsString> {
    let mut args = original;
    if let Some(Command::Install(a)) = cmd {
        if a.dir.is_none() && a.any_given() {
            if let Ok(req) = a.as_ref().clone().into_request() {
                args.push("--dir".into());
                args.push(req.dir.into_os_string());
            }
        }
    }
    args
}

/// Remove `--license-key <k>` / `--license-key=<k>` from `args`, returning the key. Pure.
fn strip_license_key_arg(args: Vec<std::ffi::OsString>) -> (Vec<std::ffi::OsString>, Option<String>) {
    let mut out = Vec::with_capacity(args.len());
    let mut key = None;
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        let text = a.to_string_lossy().into_owned();
        if text == "--license-key" {
            if let Some(v) = it.next() {
                key = Some(v.to_string_lossy().into_owned());
                continue;
            }
        } else if let Some(v) = text.strip_prefix("--license-key=") {
            key = Some(v.to_string());
            continue;
        }
        out.push(a);
    }
    (out, key)
}

/// Window kept open (Windows) so an elevated or double-clicked console never flashes and vanishes.
static NO_PAUSE_ON_SUCCESS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn finish(code: i32) -> ! {
    let (si, so) = (std::io::stdin().is_terminal(), std::io::stdout().is_terminal());
    if !NO_PAUSE_ON_SUCCESS.load(std::sync::atomic::Ordering::Relaxed) && elevate::should_pause(cfg!(windows), elevate::owns_console(), elevate::was_relaunched(), si, so) {
        println!("\nPress Enter to close this window.");
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
    }
    std::process::exit(code);
}

/// Ask for administrator permission before anything else happens (see `kora_core::elevate`).
/// `effective` = what will run (a bare `kora` resolves to the manager or the wizard); the relaunch
/// keeps the original arguments so the elevated copy decides the same way.
fn elevate_if_needed(cli: &Cli, effective: &Option<Command>) {
    let kind = command_kind(effective);
    let tty = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    if !tty || cli.no_elevate || !(cfg!(windows) && kind.privileged() || cfg!(not(windows))) || elevate::is_elevated() {
        return;
    }
    let reasons = if cfg!(windows) { vec![] } else { admin_reasons_for_command(effective) };
    let env = elevate::Env { windows: cfg!(windows), elevated: false, tty, no_elevate: false };
    let need = elevate::decide(&env, kind, &reasons);
    let args = relaunch_args(&cli.command, std::env::args_os().skip(1).collect());
    // A licence key typed on the command line would be visible in the elevated copy's process listing (and in the
    // PowerShell helper's): hand it over through the environment instead (`sudo` keeps CFX_LICENSE_KEY; so does Start-Process).
    let (args, key) = strip_license_key_arg(args);
    if let Some(k) = key {
        std::env::set_var("CFX_LICENSE_KEY", k);
    }
    if let elevate::Elevation::Exit(code) = elevate::ensure_elevated(&need, &args, &elevate::SystemElevator, &mut |m| println!("{m}")) {
        finish(code);
    }
}

fn main() {
    // An elevated child of the Windows worker or helper: drop every override variable first.
    kora_core::helper::worker::scrub_overrides();
    // Saved licence keys may live in the system keychain (never touched by tests: see `store::keychain`).
    kora_core::store::keychain::enable_system_keychain();
    if std::env::args_os().any(|a| a == "--license-key" || a.to_string_lossy().starts_with("--license-key=")) {
        eprintln!(
            "Note: a licence key on the command line is visible to other programs on this computer (process list) and stays in your shell history. Prefer the CFX_LICENSE_KEY environment variable."
        );
    }
    // Before parsing: `--version`/`--help` exit inside the parser and must clean up too.
    kora_core::self_update::startup_cleanup();
    let cli = Cli::parse();
    // Bare `kora` opens the panel (see `bare`); it never asks for administrator rights.
    let tty = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    elevate_if_needed(&cli, &cli.command);
    let audit = crate::cmd::store::audit_start::<Cli>();
    let command = cli.command;
    // "New version available": background check, interactive runs only (never CI, pipes, services).
    let quiet = matches!(
        command,
        Some(
            Command::ServiceHost { .. }
                | Command::SelfUpdate { .. }
                | Command::Helper { .. }
                | Command::AdminWorker(_)
                | Command::AdminStep(_)
                | Command::AdminDb(_)
                | Command::Update(UpdateArgs { auto: true, .. })
        )
    );
    // The worker has a hidden window and its step children no terminal: never wait for a key.
    match &command {
        Some(Command::AdminWorker(a)) => std::process::exit(admin_worker::admin_worker(a)),
        Some(Command::AdminStep(a)) => std::process::exit(admin_worker::admin_step(a)),
        Some(Command::AdminDb(a)) => std::process::exit(admin_worker::admin_db(a)),
        _ => {}
    }
    let pending = if quiet { None } else { kora_core::self_update::update_check::Pending::start(tty) };
    // Already running as root for these: repair update jobs an older Kora pointed at a program other
    // accounts can change (`~/.local/bin/kora`). Does nothing when not root or nothing is wrong.
    let repairs = match &command {
        Some(Command::Install(a)) => !a.dry_run,
        Some(Command::Update(_) | Command::Schedule { .. }) => true,
        _ => false,
    };
    if repairs {
        kora_core::service::repair_root_update_units(&mut |line| println!("{line}"));
    }
    let code = match command {
        Some(Command::Install(a)) => install(*a),
        None | Some(Command::Manager) => {
            let code = bare::open_panel(bare::Landing::Home, false);
            if code == 0 {
                // A shortcut's window should vanish, not wait for a key.
                NO_PAUSE_ON_SUCCESS.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            code
        }
        Some(Command::Panel(a)) => panel(&a),
        Some(Command::Update(a)) => update(a),
        Some(Command::AddResources(a)) => add_resources(a),
        Some(Command::AddAsset(a)) => add_asset(&a),
        Some(Command::Status) => status(),
        Some(Command::SelfUpdate { check, yes, version }) => kora_core::self_update::cli::run(&kora_core::self_update::cli::Options { check, yes, version }),
        Some(Command::Webhook { cmd }) => webhook(cmd),
        Some(Command::Desktop { cmd }) => cmd::desktop::run(cmd),
        Some(Command::Reach(a)) => cmd::reach::run(a),
        Some(Command::Branch { cmd }) => branch::branch(cmd),
        Some(Command::Pr { cmd }) => branch::pr(cmd),
        Some(Command::Resource { cmd }) => resource::resource(cmd),
        Some(Command::Schedule { cmd }) => schedule(cmd),
        Some(Command::Github { cmd: GithubCmd::Owners }) => github_owners(),
        Some(Command::Tools { cmd: ToolsCmd::Status }) => toolchain::run_tools_status(),
        Some(Command::Tools { cmd: ToolsCmd::Install { tool } }) => toolchain::run_tools_install(tool.as_deref()),
        Some(Command::Check { start_database }) => toolchain::run_check(start_database),
        Some(Command::Keys { cmd }) => keys_cmd::keys(cmd),
        Some(Command::Audit(a)) => crate::cmd::store::audit(a),
        Some(Command::Store { cmd }) => crate::cmd::store::store(cmd),
        Some(Command::Profile { cmd }) => keys_cmd::profile(cmd),
        Some(Command::Delete { server, keep_files, keep_db, yes }) => delete(&server, keep_files, keep_db, yes),
        Some(Command::ServiceHost { name }) => match host::service_host(&name) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        },
        Some(Command::Start { server }) => control(&server, kora_core::service::ControlAction::Start),
        Some(Command::Stop { server }) => control(&server, kora_core::service::ControlAction::Stop),
        Some(Command::Restart { server }) => control(&server, kora_core::service::ControlAction::Restart),
        Some(Command::Helper { cmd }) => helper(cmd),
        Some(Command::AdminWorker(_) | Command::AdminStep(_) | Command::AdminDb(_)) => unreachable!("handled above"),
        Some(Command::Uninstall { app: true, yes, dry_run, .. }) => app_uninstall(yes, dry_run, false),
        Some(Command::Uninstall { server, dry_run, yes, delete_files, delete_databases, app: false }) => uninstall(server, dry_run, yes, delete_files, delete_databases),
        Some(Command::App { cmd }) => match cmd {
            AppCmd::Install { no_desktop_shortcut, dry_run, .. } => app_install(no_desktop_shortcut, dry_run),
            AppCmd::Status => app_status(),
            AppCmd::Uninstall { yes, dry_run } => app_uninstall(yes, dry_run, true),
        },
    };
    if let Some(line) = pending.and_then(|p| p.finish(std::time::Duration::from_secs(2))) {
        println!("\n{line}");
    }
    audit.finish(code);
    finish(code);
}

mod cmd;
mod ui;

#[cfg(test)]
mod parity;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_kora_and_the_panel_aliases_never_elevate_but_install_with_options_still_may() {
        use elevate::CmdKind as K;
        let kind = |a: &[&str]| command_kind(&Cli::try_parse_from(std::iter::once("kora").chain(a.iter().copied())).unwrap().command);
        for opens_panel in [&[][..], &["manager"], &["install"], &["install", "--no-app"], &["install", "--catalog", "x"]] {
            let k = kind(opens_panel);
            assert!(matches!(k, K::Other) && !k.privileged(), "{opens_panel:?}");
        }
        assert!(kind(&["install", "--name", "a"]).privileged(), "an install with options is the scriptable installer");
        assert!(!kind(&["app", "install"]).privileged(), "the per-user shortcuts need no administrator rights");
    }

    #[test]
    fn the_helper_command_is_classified() {
        use elevate::CmdKind as K;
        let parse = |a: &[&str]| Cli::try_parse_from(std::iter::once("kora").chain(a.iter().copied())).unwrap().command;
        assert_eq!(command_kind(&parse(&["helper", "install"])), K::Uninstall { dry_run: false });
        assert_eq!(command_kind(&parse(&["helper", "status"])), K::Other);
        assert_eq!(command_kind(&parse(&["helper", "serve"])), K::Other);
    }

    #[test]
    fn app_commands_are_classified_for_elevation() {
        use elevate::CmdKind as K;
        let parse = |a: &[&str]| Cli::try_parse_from(std::iter::once("kora").chain(a.iter().copied())).unwrap().command;
        assert_eq!(command_kind(&parse(&["app", "install", "--yes"])), K::Status, "per-user shortcuts: no administrator rights");
        assert_eq!(command_kind(&parse(&["app", "uninstall", "--yes"])), K::Status, "Kora's own shortcuts: no administrator rights, and never run as root's home");
        assert_eq!(command_kind(&parse(&["app", "uninstall", "--dry-run"])), K::Status);
        assert_eq!(command_kind(&parse(&["uninstall", "--app", "--yes"])), K::Uninstall { dry_run: false }, "the old machine-wide app needs them");
        assert_eq!(command_kind(&parse(&["app", "status"])), K::Status);
        assert!(matches!(parse(&["uninstall", "--app", "-y"]), Some(Command::Uninstall { app: true, yes: true, .. })));
    }

    #[test]
    fn the_implied_folder_is_absolute_and_decided_before_the_relaunch() {
        // No --dir: the folder comes from kora_core::userhome and is never relative.
        let r = InstallArgs { name: Some("Alpha".into()), ..Default::default() }.into_request().unwrap();
        assert!(r.dir.is_absolute() && r.dir.ends_with("alpha"), "{:?}", r.dir);
        let parse = |a: &[&str]| Cli::try_parse_from(std::iter::once("kora").chain(a.iter().copied())).unwrap().command;
        let os = |a: &[&str]| a.iter().map(std::ffi::OsString::from).collect::<Vec<_>>();
        // an install with options and no --dir: the resolved folder is added
        let cmd = parse(&["install", "--name", "Alpha", "--service"]);
        let args = relaunch_args(&cmd, os(&["install", "--name", "Alpha", "--service"]));
        assert_eq!(args[..4], os(&["install", "--name", "Alpha", "--service"])[..]);
        assert_eq!(args[4], "--dir");
        assert_eq!(PathBuf::from(&args[5]), r.dir);
        // --dir given: left alone
        let cmd = parse(&["install", "--name", "Alpha", "--dir", "/srv/a"]);
        assert_eq!(relaunch_args(&cmd, os(&["install", "--name", "Alpha", "--dir", "/srv/a"])), os(&["install", "--name", "Alpha", "--dir", "/srv/a"]));
        // bare `kora install` is the wizard: an extra --dir would make it skip the questions
        let cmd = parse(&["install"]);
        assert_eq!(relaunch_args(&cmd, os(&["install"])), os(&["install"]));
        // --fix-permissions is an install option (no questions) and is passed on as it is
        let cmd = parse(&["install", "--fix-permissions", "--dir", "/srv/a"]);
        assert!(matches!(&cmd, Some(Command::Install(a)) if a.fix_permissions && a.any_given()));
    }

    /// The whole `kora --help` tree (every subcommand, long help, 100 columns, no colour).
    fn full_help() -> String {
        fn walk(cmd: &mut clap::Command, path: &str, out: &mut String) {
            let rendered = cmd.clone().term_width(100).max_term_width(100).color(clap::ColorChoice::Never).render_long_help().to_string();
            out.push_str(&format!("===== {path} =====\n{rendered}\n"));
            let names: Vec<String> = cmd.get_subcommands().map(|c| c.get_name().to_string()).filter(|n| n != "help").collect();
            for n in names {
                if let Some(sub) = cmd.find_subcommand_mut(&n) {
                    walk(sub, &format!("{path} {n}"), out);
                }
            }
        }
        use clap::CommandFactory;
        let mut cmd = Cli::command().disable_help_subcommand(true);
        cmd.build();
        let mut out = String::new();
        walk(&mut cmd, "kora", &mut out);
        out
    }

    /// Compares `kora --help` (all subcommands) with `tests/golden/help.txt`.
    /// After an intended change to flags or help text, regenerate it with
    /// `KORA_UPDATE_GOLDEN=1 cargo test -p kora-cli help_tree_matches_golden`.
    #[test]
    fn help_tree_matches_golden() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/help.txt");
        let actual = full_help();
        if std::env::var_os("KORA_UPDATE_GOLDEN").is_some_and(|v| !v.is_empty() && v != "0") {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &actual).unwrap();
            return;
        }
        let golden = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
        assert!(golden == actual, "`kora --help` changed. If intended, regenerate: KORA_UPDATE_GOLDEN=1 cargo test -p kora-cli help_tree_matches_golden");
    }

    #[test]
    fn the_licence_key_is_taken_off_the_relaunch_command_line() {
        let v = |a: &[&str]| a.iter().map(std::ffi::OsString::from).collect::<Vec<_>>();
        let (rest, key) = strip_license_key_arg(v(&["install", "--license-key", "cfxk_a", "--dir", "/x"]));
        assert_eq!((rest, key.as_deref()), (v(&["install", "--dir", "/x"]), Some("cfxk_a")));
        let (rest, key) = strip_license_key_arg(v(&["install", "--license-key=cfxk_b", "-y"]));
        assert_eq!((rest, key.as_deref()), (v(&["install", "-y"]), Some("cfxk_b")));
        let (rest, key) = strip_license_key_arg(v(&["install", "--dir", "/x"]));
        assert_eq!((rest.len(), key), (3, None));
    }
}
