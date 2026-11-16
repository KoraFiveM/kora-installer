//! `kora` command line entry point.

use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::mpsc;

use clap::{Args, Parser, Subcommand, ValueEnum};
use kora_core::elevate;
use kora_core::firewall::ports::{self, PortProber, SystemProber};
use kora_core::launch;
use kora_core::installer::{
    self, gitops, host, registry, DbAdmin, DbChoice, InstallRequest, Mode, Ports, ProgressEvent, ProgressSink, ResourceSelection,
    RollbackOptions, StepStatus,
};

#[derive(Parser)]
#[command(name = "kora", version, about = "Kora installer and server manager")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// Do not ask Windows/sudo for administrator permission (for developers and CI).
    #[arg(long, global = true)]
    no_elevate: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum ModeArg {
    Prod,
    Dev,
}

#[derive(Clone, Copy, ValueEnum)]
enum DbArg {
    Existing,
    Install,
    Skip,
}

#[derive(Args, Default, Clone)]
struct InstallArgs {
    /// REMOVED in 0.4.0 (setup codes no longer exist): use the flags below or `kora panel`.
    #[arg(long, value_name = "CODE")]
    code: Option<String>,
    /// Server name (display + registry name).
    #[arg(long)]
    name: Option<String>,
    /// Server folder.
    #[arg(long)]
    dir: Option<PathBuf>,
    /// prod = pinned release clones, dev = forks on your GitHub account.
    #[arg(long, value_enum)]
    mode: Option<ModeArg>,
    /// Cfx.re license key (prefer the CFX_LICENSE_KEY environment variable).
    #[arg(long, env = "CFX_LICENSE_KEY", hide_env_values = true)]
    license_key: Option<String>,
    #[arg(long)]
    game_port: Option<u16>,
    #[arg(long)]
    txadmin_port: Option<u16>,
    /// Also open the txAdmin port in the firewall (default: txAdmin stays private).
    #[arg(long)]
    txadmin_public: bool,
    /// Cfx.re account that becomes the txAdmin master, e.g. fivem:123456 (default: a generated local admin).
    #[arg(long)]
    cfx_identifier: Option<String>,
    /// Official runtime.fivem.net artifact URL (default: the recommended build).
    #[arg(long)]
    artifact_url: Option<String>,
    /// `all` or a comma separated list of resource ids (dependencies and core are always included).
    #[arg(long)]
    resources: Option<String>,
    /// existing = use the local MariaDB, install = install it if missing, skip = no database.
    #[arg(long, value_enum)]
    db: Option<DbArg>,
    /// Install and enable the OS service (needs admin).
    #[arg(long)]
    service: bool,
    /// Open the game port in the OS firewall (needs admin).
    #[arg(long)]
    firewall: bool,
    /// Dev mode: GitHub user or organisation to fork into (`list` shows the choices).
    #[arg(long)]
    github_owner: Option<String>,
    /// Catalog file or URL (default: the public catalog with an embedded fallback). Env: KORA_CATALOG.
    #[arg(long, hide = true)]
    catalog: Option<String>,
    /// Database admin user used once to create the new database + user (password: KORA_DB_ADMIN_PASSWORD).
    #[arg(long)]
    db_admin_user: Option<String>,
    /// Database server address for the admin login (default 127.0.0.1).
    #[arg(long)]
    db_admin_host: Option<String>,
    /// Database server port for the admin login (default 3306).
    #[arg(long)]
    db_admin_port: Option<u16>,
    /// Start the server when the install finishes and wait until txAdmin answers (the interactive installer does this by default).
    #[arg(long)]
    start: bool,
    /// Do not start the server after installing (start it later with `kora start <name>`).
    #[arg(long, conflicts_with = "start")]
    no_start: bool,
    /// Show the full plan and change nothing.
    #[arg(long)]
    dry_run: bool,
    /// Do not install the Kora app (the `kora` command and Start Menu shortcut) next to the server.
    #[arg(long)]
    no_app: bool,
    /// With the Kora app: no desktop shortcut (Windows).
    #[arg(long)]
    no_desktop_shortcut: bool,
    /// If a port is in use, silently use the next free one and say what changed.
    #[arg(long)]
    auto_ports: bool,
    /// Server hostname shown in the server list (default: the server name).
    #[arg(long)]
    hostname: Option<String>,
    /// Pin the FiveM game build (e.g. 3258); default: the build the server ships with.
    #[arg(long, value_name = "N")]
    game_build: Option<u32>,
    /// Do not ask anything (implied by any other option).
    #[arg(long, short = 'y')]
    yes: bool,
}

impl InstallArgs {
    fn any_given(&self) -> bool {
        self.name.is_some()
            || self.dir.is_some()
            || self.hostname.is_some()
            || self.game_build.is_some()
            || self.mode.is_some()
            || self.license_key.is_some()
            || self.game_port.is_some()
            || self.txadmin_port.is_some()
            || self.txadmin_public
            || self.cfx_identifier.is_some()
            || self.artifact_url.is_some()
            || self.resources.is_some()
            || self.db.is_some()
            || self.service
            || self.firewall
            || self.github_owner.is_some()
            || self.db_admin_user.is_some()
            || self.db_admin_host.is_some()
            || self.db_admin_port.is_some()
            || self.dry_run
            || self.start
            || self.no_start
            || self.auto_ports
            || self.yes
    }

    fn into_request(self) -> Result<InstallRequest, String> {
        let db_admin = if self.db_admin_user.is_some() || self.db_admin_host.is_some() || self.db_admin_port.is_some() {
            Some(DbAdmin {
                host: self.db_admin_host.unwrap_or_else(|| "127.0.0.1".into()),
                port: self.db_admin_port.unwrap_or(3306),
                user: self.db_admin_user.unwrap_or_else(|| "root".into()),
                password: std::env::var("KORA_DB_ADMIN_PASSWORD").ok().filter(|p| !p.is_empty()),
            })
        } else {
            None
        };
        let catalog = self.catalog.or_else(|| std::env::var("KORA_CATALOG").ok().filter(|c| !c.is_empty()));
        let name = self.name.unwrap_or_else(|| "kora".into());
        let dir = match self.dir {
            Some(d) => d,
            None => {
                home_dir(cfg!(windows), &|k| std::env::var_os(k)).join("kora").join(installer::slug(&name))
            }
        };
        let resources = match self.resources.as_deref().map(str::trim) {
            None | Some("") | Some("all") => ResourceSelection::All,
            Some(list) => ResourceSelection::Only(list.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()),
        };
        let defaults = Ports::default();
        Ok(InstallRequest {
            mode: match self.mode.unwrap_or(ModeArg::Prod) {
                ModeArg::Prod => Mode::Production,
                ModeArg::Dev => Mode::Development,
            },
            hostname: self.hostname.clone().unwrap_or_else(|| name.clone()),
            name,
            dir,
            license_key: self.license_key,
            cfx_identifier: self.cfx_identifier,
            txadmin_public: self.txadmin_public,
            ports: Ports { game: self.game_port.unwrap_or(defaults.game), txadmin: self.txadmin_port.unwrap_or(defaults.txadmin) },
            artifact_url: self.artifact_url,
            game_build: self.game_build,
            resources,
            db: match self.db.unwrap_or(DbArg::Existing) {
                DbArg::Existing => DbChoice::UseExisting,
                DbArg::Install => DbChoice::InstallIfMissing,
                DbArg::Skip => DbChoice::Skip,
            },
            service: self.service,
            firewall: self.firewall,
            github_owner: self.github_owner,
            catalog,
            dry_run: self.dry_run,
            db_admin,
            // Headless installs start nothing unless asked (CI and scripts keep their behaviour).
            start: self.start && !self.no_start,
            // Real installs include the app by default; a dry run lists nothing it would not change anyway.
            app: (!self.no_app && !self.dry_run).then_some(kora_core::app_install::AppChoice { desktop_shortcut: !self.no_desktop_shortcut }),
        })
    }
}

/// The user's home folder: USERPROFILE first on Windows (HOME may point elsewhere under Git Bash),
/// HOME first elsewhere.
fn home_dir(windows: bool, get: &dyn Fn(&str) -> Option<std::ffi::OsString>) -> PathBuf {
    let order = if windows { ["USERPROFILE", "HOME"] } else { ["HOME", "USERPROFILE"] };
    order.iter().find_map(|k| get(k).filter(|v| !v.is_empty())).map(PathBuf::from).unwrap_or_default()
}

/// What to do when an update needs a restart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum RestartArg {
    /// Apply the files, restart nothing.
    None,
    /// Apply the files without restarting; the server loads them on its next restart or an `ensure` in txAdmin (Kora cannot do that for you without credentials).
    Ensure,
    /// Restart the whole server.
    Full,
    /// Restart only when nobody is playing (default).
    WhenQuiet,
}

#[derive(Args, Debug, Default)]
struct UpdateArgs {
    /// Run unattended (fast-forward clean forks only).
    #[arg(long, conflicts_with_all = ["resource", "all", "preview", "yes", "restart", "rollback", "list_snapshots"])]
    auto: bool,
    /// Server name or id.
    server: String,
    /// Update only this resource (repeatable).
    #[arg(long = "resource", value_name = "ID", conflicts_with = "all")]
    resource: Vec<String>,
    /// Update every resource.
    #[arg(long)]
    all: bool,
    /// Show what would change, then stop.
    #[arg(long, conflicts_with_all = ["rollback", "list_snapshots"])]
    preview: bool,
    /// Do not ask for confirmation.
    #[arg(long, short = 'y')]
    yes: bool,
    /// What to do about restarts.
    #[arg(long, value_enum)]
    restart: Option<RestartArg>,
    /// Put everything back the way a snapshot saved it.
    #[arg(long, value_name = "SNAPSHOT", conflicts_with_all = ["resource", "all", "list_snapshots"])]
    rollback: Option<String>,
    /// List saved snapshots.
    #[arg(long, conflicts_with_all = ["resource", "all"])]
    list_snapshots: bool,
}

#[derive(Args, Debug, Default)]
struct AddResourcesArgs {
    /// Server name or folder.
    server: String,
    /// REMOVED in 0.4.0 (setup codes no longer exist): use --resource or `kora panel`.
    #[arg(long, value_name = "CODE")]
    code: Option<String>,
    /// Catalog resource id to add (repeatable). Dependencies are added automatically.
    #[arg(long = "resource", value_name = "ID")]
    resource: Vec<String>,
    /// Show what would be added, then stop.
    #[arg(long)]
    dry_run: bool,
    /// Do not ask for confirmation.
    #[arg(long, short = 'y')]
    yes: bool,
}

#[derive(Args, Debug)]
struct PanelArgs {
    /// Port to listen on (127.0.0.1 only).
    #[arg(long, default_value_t = kora_web::DEFAULT_PORT)]
    port: u16,
    /// Do not open a browser; print the address and sign-in link instead (automatic over SSH or without a desktop).
    #[arg(long)]
    no_open: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Install a new Kora server (interactive TUI, or headless with options).
    Install(Box<InstallArgs>),
    /// Open the multi-server manager.
    Manager,
    /// Open the Kora Panel: install, manage and monitor servers in your web browser (local only).
    Panel(PanelArgs),
    /// Update a server: preview, snapshot, apply and roll back (`--auto` is for schedulers).
    Update(UpdateArgs),
    /// Add compatibility, bridge or creator resources (or any catalog resource) to an installed server.
    AddResources(AddResourcesArgs),
    /// Show registered servers.
    Status,
    /// Update the kora program itself to the newest release (checksum-verified).
    SelfUpdate {
        /// Only say whether a newer version exists.
        #[arg(long)]
        check: bool,
        /// Do not ask for confirmation.
        #[arg(long, short = 'y')]
        yes: bool,
        /// Install exactly this release (also allows going back to an older one), e.g. v0.2.1.
        #[arg(long, value_name = "vX.Y.Z")]
        version: Option<String>,
    },
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
    /// The Kora app: the `kora` command / Start Menu shortcut that reopens the manager.
    App {
        #[command(subcommand)]
        cmd: AppCmd,
    },
    /// GitHub helpers for development mode.
    Github {
        #[command(subcommand)]
        cmd: GithubCmd,
    },
    /// Run a server's FXServer (used by the Windows service and supervisors).
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
        /// Remove only the Kora app (shortcuts, PATH entry, the installed copy); servers and databases are never touched.
        #[arg(long)]
        app: bool,
    },
}

#[derive(Subcommand, Debug)]
enum ScheduleCmd {
    /// Show the current schedule in plain language.
    Show {
        /// Registry name, or the server folder.
        server: String,
    },
    /// Change the schedule (needs administrator rights; Kora asks for them).
    #[command(after_help = kora_core::service::RUN_AS_HELP)]
    Set {
        /// Registry name, or the server folder.
        server: String,
        /// Every day at HH:MM (24 hours, this computer's time). The default is 04:00.
        #[arg(long, value_name = "HH:MM", conflicts_with = "weekly")]
        daily: Option<String>,
        /// Every week on DAY (mon..sun) at HH:MM.
        #[arg(long, num_args = 2, value_names = ["DAY", "HH:MM"], conflicts_with = "daily")]
        weekly: Option<Vec<String>>,
        /// Run as you, the person who installed (default): your GitHub sign-in works for private forks.
        #[arg(long, conflicts_with = "as_system")]
        as_user: bool,
        /// Run as the system account: works when nobody is logged in, but cannot fetch private forks.
        #[arg(long)]
        as_system: bool,
    },
    /// Turn automatic updates off.
    Off {
        /// Registry name, or the server folder.
        server: String,
    },
}

#[derive(Subcommand)]
enum WebhookCmd {
    /// Save a Discord webhook for a server. Prefer --stdin or $KORA_DISCORD_WEBHOOK so the URL stays out of shell history.
    Set {
        /// Server name or folder.
        server: String,
        /// The webhook URL (https://discord.com/api/webhooks/<id>/<token>).
        url: Option<String>,
        /// Read the URL from standard input.
        #[arg(long)]
        stdin: bool,
        /// Send a test message after saving.
        #[arg(long)]
        test: bool,
    },
    /// Send a test message.
    Test { server: String },
    /// Show whether a webhook is set (the token is hidden).
    Show { server: String },
    /// Delete the saved webhook.
    Remove { server: String },
}

fn webhook(cmd: WebhookCmd) -> i32 {
    use kora_core::notify;
    let find = |s: &str| -> Result<registry::ServerEntry, String> {
        let reg = registry::Registry::load().map_err(|e| format!("cannot read the server registry: {e}"))?;
        reg.find(s).cloned().ok_or_else(|| format!("no registered server named '{s}' (see `kora status`)"))
    };
    let fail = |m: String| {
        eprintln!("error: {m}");
        1
    };
    let test = |srv: &registry::ServerEntry, cfg: &notify::NotifyConfig| match notify::send_test(cfg, &srv.name) {
        Ok(()) => {
            println!("Test message sent.");
            0
        }
        Err(e) => fail(format!("test message failed: {e}")),
    };
    match cmd {
        WebhookCmd::Set { server, url, stdin, test: do_test } => {
            let srv = match find(&server) {
                Ok(s) => s,
                Err(e) => return fail(e),
            };
            let env = std::env::var(notify::ENV_VAR).ok();
            let url = match notify::resolve_url_input(url.as_deref(), stdin, env.as_deref(), &mut std::io::stdin().lock()) {
                Ok(u) => u,
                Err(notify::NotifyError::NotConfigured) => return fail(format!("give a URL, use --stdin, or set ${}", notify::ENV_VAR)),
                Err(e) => return fail(e.to_string()),
            };
            let cfg = notify::NotifyConfig { discord_webhook: Some(url) };
            if let Err(e) = notify::save(&srv, &cfg) {
                return fail(e.to_string());
            }
            println!("Saved Discord webhook for '{}': {}", srv.name, notify::redact(cfg.discord_webhook.as_deref().unwrap_or("")));
            if do_test {
                return test(&srv, &cfg);
            }
            println!("Run `kora webhook test {}` to check it.", srv.name);
            0
        }
        WebhookCmd::Test { server } => match find(&server) {
            Ok(srv) => {
                let cfg = notify::load(&srv);
                if cfg.discord_webhook.is_none() {
                    return fail(format!("no webhook set for '{}'; use `kora webhook set`", srv.name));
                }
                test(&srv, &cfg)
            }
            Err(e) => fail(e),
        },
        WebhookCmd::Show { server } => match find(&server) {
            Ok(srv) => {
                match notify::load(&srv).discord_webhook {
                    Some(u) => println!("{}: {}", srv.name, notify::redact(&u)),
                    None => println!("{}: no webhook set", srv.name),
                }
                0
            }
            Err(e) => fail(e),
        },
        WebhookCmd::Remove { server } => match find(&server) {
            Ok(srv) => match notify::save(&srv, &notify::NotifyConfig::default()) {
                Ok(()) => {
                    println!("Removed the webhook for '{}'.", srv.name);
                    0
                }
                Err(e) => fail(e.to_string()),
            },
            Err(e) => fail(e),
        },
    }
}

#[derive(Subcommand)]
enum AppCmd {
    /// Install or update the Kora app (idempotent).
    Install {
        /// Do not ask (nothing is asked today; accepted for scripts).
        #[arg(long, short = 'y')]
        yes: bool,
        /// No desktop shortcut (Windows).
        #[arg(long)]
        no_desktop_shortcut: bool,
        /// Replace a newer installed copy with this program.
        #[arg(long)]
        force: bool,
        /// List every change and make none.
        #[arg(long)]
        dry_run: bool,
    },
    /// Show whether the Kora app is installed (`key: value` lines).
    Status,
    /// Remove the Kora app. Servers and databases are never touched.
    Uninstall {
        #[arg(long, short = 'y')]
        yes: bool,
        #[arg(long)]
        dry_run: bool,
    },
}

/// What a bare `kora` does in a terminal: manage the servers you have, or set up the first one.
fn default_command(registered: usize) -> Command {
    if registered > 0 {
        Command::Manager
    } else {
        Command::Install(Box::default())
    }
}

fn registered_servers() -> usize {
    registry::Registry::load().map(|r| r.servers.len()).unwrap_or(0)
}

fn print_lines(prefix: &str, lines: &[String]) {
    for l in lines {
        println!("{prefix}{l}");
    }
}

fn app_install(force: bool, no_desktop_shortcut: bool, dry_run: bool) -> i32 {
    use kora_core::app_install as app;
    if !dry_run && !installer::is_admin() && std::env::var_os("KORA_APP_PREFIX").is_none() {
        eprintln!("error: installing the Kora app needs administrator rights: run this as Administrator (Windows) or with sudo (Linux).");
        return 1;
    }
    match app::install_app(&app::AppOptions { desktop_shortcut: !no_desktop_shortcut, force, dry_run }) {
        Ok(out) => {
            if dry_run {
                println!("Dry run: nothing was changed. The Kora app install would:");
            } else if out.changed() {
                println!("The Kora app is installed ({}):", out.exe.display());
            } else {
                println!("The Kora app is already installed and up to date ({}).", out.exe.display());
            }
            print_lines("  - ", &out.changes);
            print_lines("  note: ", &out.notes);
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn app_status() -> i32 {
    print_lines("", &kora_core::app_install::status().lines());
    0
}

fn app_uninstall(yes: bool, dry_run: bool) -> i32 {
    use kora_core::app_install as app;
    if !dry_run && !installer::is_admin() && std::env::var_os("KORA_APP_PREFIX").is_none() {
        eprintln!("error: removing the Kora app needs administrator rights: run this as Administrator (Windows) or with sudo (Linux).");
        return 1;
    }
    if !yes && !dry_run {
        if !std::io::stdin().is_terminal() {
            eprintln!("error: pass --yes to remove the Kora app without a terminal to ask in (or --dry-run to preview).");
            return 2;
        }
        if !installer::uninstall::Asker::ask(&mut Terminal, "Remove the Kora app (shortcuts, PATH entry, the installed program)? Your servers and databases are not touched.", true) {
            println!("Cancelled. Nothing was changed.");
            return 0;
        }
    }
    match app::uninstall_app_opts(dry_run) {
        Ok(lines) => {
            println!("{}", if dry_run { "Dry run: nothing was changed. Removing the Kora app would:" } else { "The Kora app was removed:" });
            print_lines("  - ", &lines);
            println!("Your servers and databases were not touched.");
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

#[derive(Subcommand)]
enum GithubCmd {
    /// List the GitHub accounts and organisations your forks can go into.
    Owners,
}

/// Print the fork targets (`kora github owners`, `--github-owner list`).
fn github_owners() -> i32 {
    use kora_core::git::{owners, SystemRunner};
    match owners::list_owners(&SystemRunner) {
        Ok(o) => {
            print!("{}", owners::format_owner_list(&o));
            0
        }
        Err(e) => {
            eprintln!("error: could not read your GitHub accounts ({e}). Is `gh` installed and logged in? Run `gh auth login`.");
            1
        }
    }
}

/// Check `--github-owner` against the real list; with no value, only the personal account is
/// chosen automatically, and only when it is the sole usable option.
fn resolve_github_owner(requested: Option<&str>) -> Result<String, String> {
    resolve_github_owner_with(&kora_core::git::SystemRunner, requested)
}

fn resolve_github_owner_with(runner: &dyn kora_core::git::CommandRunner, requested: Option<&str>) -> Result<String, String> {
    use kora_core::git::owners;
    if let Some(want) = requested {
        // A named owner needs no `gh` yet: the install sets `gh` up and signs in, then re-checks it
        // (before any fork) in the prerequisites step.
        return match owners::check_requested_owner(runner, want)? {
            owners::OwnerCheck::Verified(l) => Ok(l),
            owners::OwnerCheck::Deferred(l) => {
                println!("GitHub CLI is not installed or not signed in yet: '{l}' will be checked after the sign-in during setup.");
                Ok(l)
            }
        };
    }
    let all = owners::list_owners(runner).map_err(|e| format!("could not read your GitHub accounts ({e}). Is `gh` installed and logged in? Run `gh auth login`."))?;
    owners::choose_owner(&all, requested).map(|o| o.login)
}

/// Plain printing sink.
struct Printer {
    last: Option<(String, String)>,
}

impl ProgressSink for Printer {
    fn event(&mut self, ev: ProgressEvent) {
        let key = (ev.step.clone(), format!("{:?}{}", ev.status, ev.detail));
        if self.last.as_ref() == Some(&key) {
            return;
        }
        self.last = Some(key);
        let mark = match ev.status {
            StepStatus::Pending => "[ ]",
            StepStatus::Running => "[..]",
            StepStatus::Done => "[ok]",
            StepStatus::Skipped => "[--]",
            StepStatus::Failed => "[!!]",
        };
        println!("{:>3}% {mark} {} ({}): {}", ev.percent, ev.title, ev.step, ev.detail);
    }
}

/// `kora manager`: the server list. "Install a new server" runs the install wizard, then comes back.
fn manager() -> i32 {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        eprintln!("error: `kora manager` needs an interactive terminal. `kora status` lists your servers without one.");
        return 2;
    }
    loop {
        match kora_tui::manager::run_manager() {
            Ok(kora_tui::manager::Exit::Quit) => return 0,
            // The wizard reports its own errors and cancellation; either way the manager comes back.
            Ok(kora_tui::manager::Exit::InstallNew) => {
                install(InstallArgs::default());
            }
            // Issues, pull requests and forking need GitHub: gh's own sign-in gets the terminal (it shows
            // its one-time code and address), then the manager comes back. Kora never sees the token.
            Ok(kora_tui::manager::Exit::GithubLogin) => {
                println!("Signing in to GitHub. Follow the steps gh shows below.");
                match kora_core::git::auth_login_interactive() {
                    Ok(()) => println!("Signed in. Opening the manager again..."),
                    Err(e) => eprintln!("Could not sign in to GitHub ({e}). Opening the manager again..."),
                }
            }
            // Changing the schedule needs administrator rights: the terminal is back to normal here,
            // so the permission prompt (sudo / UAC) can be answered, then the manager comes back.
            Ok(kora_tui::manager::Exit::Elevate(args)) => {
                println!("Changing the update schedule needs administrator permission. Approve the prompt to continue.");
                let os: Vec<std::ffi::OsString> = args.iter().map(Into::into).collect();
                match elevate::relaunch_elevated(&os) {
                    Ok(()) => println!("Done. Opening the manager again..."),
                    Err(elevate::ElevateError::Declined) => println!("{}", elevate::declined_message()),
                    Err(e) => eprintln!("error: {e}. Run this yourself: sudo kora {}", args.join(" ")),
                }
            }
            Err(e) => {
                eprintln!("error: {e}");
                return 1;
            }
        }
    }
}

/// Interactive wizard (`None` = the user quit without installing).
fn interactive_install() -> Result<Option<InstallRequest>, String> {
    kora_tui::run_install_wizard().map_err(|e| e.to_string())
}

/// Shown (exit code 2) when `--code` is used: setup codes are gone.
const CODE_REMOVED: &str = "error: setup codes were removed in 0.4.0. Use the flags instead (see `kora install --help`), `kora add-resources <server> --resource <ID>`, or the panel (`kora panel`).";

/// Sends every event to the progress screen and keeps a plain-text copy for the log file.
struct ChannelSink {
    tx: mpsc::Sender<ProgressEvent>,
    log: Vec<String>,
}

impl ProgressSink for ChannelSink {
    fn event(&mut self, ev: ProgressEvent) {
        self.log.push(format!("{:>3}% {:?} {} ({}): {}", ev.percent, ev.status, ev.title, ev.step, ev.detail));
        let _ = self.tx.send(ev); // the screen may already be gone (cancelled)
    }
}

/// A finished server (resume/reinstall of the same folder) keeps its ports.
fn already_installed(req: &InstallRequest) -> bool {
    req.dir.join("kora-server.json").is_file()
}

/// Check the requested ports against this machine (and the other Kora servers on it). With
/// `auto`, move taken ports to the next free ones and return what changed; without, a taken port
/// is an error naming the port, the next free port and how to apply it.
fn settle_ports(req: &mut InstallRequest, auto: bool, prober: &dyn PortProber) -> Result<Vec<String>, String> {
    let avoid = ports::registered_ports_except(&req.name, Some(&req.dir));
    settle_ports_avoiding(req, auto, prober, &avoid)
}

/// `settle_ports` with the other servers' ports given (tests pass none, so this machine's servers don't matter).
fn settle_ports_avoiding(req: &mut InstallRequest, auto: bool, prober: &dyn PortProber, avoid: &[u16]) -> Result<Vec<String>, String> {
    let avoid = avoid.to_vec();
    let (game, tx) = (req.ports.game, req.ports.txadmin);
    let report = ports::check_ports(game, tx, &avoid, prober);
    if report.all_free() {
        return Ok(vec![]);
    }
    if auto {
        let Some((g, t)) = ports::effective_ports(game, tx, &avoid, prober) else {
            return Err("no free port was found near the requested ones: choose other ports with --game-port and --txadmin-port".into());
        };
        let mut changes = Vec::new();
        for (old, new) in [(game, g), (tx, t)] {
            if old != new {
                changes.push(format!("Port {old} is in use, using {new}"));
            }
        }
        req.ports.game = g;
        req.ports.txadmin = t;
        return Ok(changes);
    }
    let mut text = ports::conflict_messages(game, tx, &report, &avoid).join(" ");
    if let Some((g, t)) = ports::effective_ports(game, tx, &avoid, prober) {
        text.push_str(&format!(" Run the command again with --game-port {g} --txadmin-port {t}, or add --auto-ports to let Kora pick free ports."));
    }
    Err(text)
}

/// Dry-run header: the effective choices (from the flags). Never includes secrets.
fn request_summary(req: &InstallRequest) -> String {
    use kora_core::installer::{Mode, ResourceSelection};
    let resources = match &req.resources {
        ResourceSelection::All => "all".to_string(),
        ResourceSelection::Only(v) => format!("{} chosen ({})", v.len(), v.join(", ")),
    };
    format!(
        "Your choices:\n  name:      {}\n  mode:      {}\n  resources: {}\n  ports:     game {}, txAdmin {}\n  artifact:  {}\n  game build: {}\n  hostname:  {}\n  cfx owner: {}\n",
        req.name,
        if req.mode == Mode::Production { "production" } else { "development" },
        resources,
        req.ports.game,
        req.ports.txadmin,
        req.artifact_url.as_deref().unwrap_or("recommended"),
        req.game_build.map_or("default".to_string(), |b| b.to_string()),
        if req.hostname.is_empty() { "(none)" } else { &req.hostname },
        req.cfx_identifier.as_deref().unwrap_or("(none)"),
    )
}

fn print_outcome(out: &installer::InstallOutcome, dry: bool) {
    println!();
    println!("{}", if dry { "Plan (nothing was changed):" } else { "Install finished." });
    println!("  folder:  {}", out.dir.display());
    println!("  connect: {}", out.connect);
    println!("  txAdmin: {}", out.txadmin_url);
    if out.start_attempted {
        if out.running {
            println!("  server:  RUNNING - txAdmin is up on {}{}", out.txadmin_url, if out.game_listening { ", the game port is listening" } else { " (the game port is not listening yet)" });
        } else if let Some(p) = &out.launch_problem {
            println!("  server:  NOT RUNNING - {p}");
            if out.txadmin_up {
                println!("  txAdmin is up on {} (the game server itself did not start)", out.txadmin_url);
            }
            for l in &out.log_tail {
                println!("    | {l}");
            }
        }
    }
    if let (Some(u), Some(f)) = (&out.admin_user, &out.admin_password_file) {
        println!("  txAdmin login: user '{u}', password in {}", f.display());
    }
    for n in &out.notes {
        println!("  note: {n}");
    }
}

/// Where the install log goes: inside the server's state folder when it exists, else the temp dir.
fn write_install_log(req: &InstallRequest, lines: &[String], error: Option<&str>) -> Option<PathBuf> {
    let meta = req.dir.join(".kora");
    let path = if meta.is_dir() {
        meta.join("install.log")
    } else {
        std::env::temp_dir().join(format!("kora-install-{}.log", installer::slug(&req.name)))
    };
    let mut text = lines.join("\n");
    if let Some(e) = error {
        text.push_str(&format!("\nERROR: {e}\n"));
    }
    std::fs::write(&path, text).ok().map(|_| path)
}

/// Wizard -> installer thread + progress screen -> outcome / error screen.
fn interactive_flow(catalog: Option<String>, no_app: bool, no_desktop_shortcut: bool) -> i32 {
    let mut req = match interactive_install() {
        Ok(Some(r)) => r,
        Ok(None) => {
            // The wizard's goodbye screen already waited for a key: don't ask twice.
            ALREADY_PAUSED.store(true, std::sync::atomic::Ordering::Relaxed);
            println!("Cancelled. Nothing was changed.");
            return 0;
        }
        Err(e) => {
            eprintln!("error: the interactive screen could not be shown: {e}");
            return 1;
        }
    };
    if catalog.is_some() {
        req.catalog = catalog;
    }
    if no_app {
        req.app = None;
    } else if no_desktop_shortcut {
        req.app = Some(kora_core::app_install::AppChoice { desktop_shortcut: false });
    }
    let (tx, rx) = mpsc::channel();
    let worker_req = req.clone();
    let worker = std::thread::spawn(move || {
        let mut sink = ChannelSink { tx, log: Vec::new() };
        let r = installer::run(&worker_req, &mut sink);
        (r, sink.log)
    });
    let shown = kora_tui::run_install_progress(rx);
    if let Err(e) = &shown {
        eprintln!("{e}. Waiting for the current step to finish so nothing is left half-done...");
    }
    let (result, log) = match worker.join() {
        Ok(v) => v,
        Err(_) => {
            eprintln!("error: the installer stopped unexpectedly (internal error).");
            return 1;
        }
    };
    match result {
        Ok(out) if shown.is_ok() => {
            if let Err(e) = kora_tui::show_outcome(&out) {
                eprintln!("note: could not show the final screen: {e}");
            }
            let _ = write_install_log(&req, &log, None);
            installer::saved::clear();
            print_outcome(&out, false);
            0
        }
        Ok(out) => {
            print_outcome(&out, false);
            130
        }
        Err(e) => {
            let msg = e.to_string();
            let path = write_install_log(&req, &log, Some(&msg));
            // Keep the answers (never the secrets) so the next run does not ask everything again.
            let saved = installer::saved::save(&req).is_ok();
            let next = if req.dir.join(".kora").is_dir() {
                format!(
                    "What was done so far is kept{}.\nTo try again: run `kora install` again; it continues where it stopped.\nTo start over: run `kora delete {}` to remove what was created.",
                    if saved { ", and so are your answers (not the license key or database password)" } else { "" },
                    req.dir.display()
                )
            } else if saved {
                "Nothing was created. Your answers are saved (not the license key or database password): fix the problem above and run `kora install` again.".to_string()
            } else {
                "Nothing was created. Fix the problem above and run `kora install` again.".to_string()
            };
            if shown.is_ok() {
                if let Err(e2) = kora_tui::show_failure(&msg, &next, path.as_deref()) {
                    eprintln!("note: could not show the error screen: {e2}");
                }
            }
            eprintln!("\nerror: {msg}\n{next}");
            if let Some(p) = path {
                eprintln!("Log: {}", p.display());
            }
            1
        }
    }
}

fn install(args: InstallArgs) -> i32 {
    let mut args = args;
    if args.github_owner.as_deref().is_some_and(|o| o.eq_ignore_ascii_case("list")) {
        return github_owners();
    }
    if args.code.is_some() {
        eprintln!("{CODE_REMOVED}");
        return 2;
    }
    let headless = args.any_given();
    if !headless {
        if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
            eprintln!(
                "error: `kora install` needs an interactive terminal to ask its questions. \
                 Run it in a terminal, or pass options instead e.g. `kora install --name \"My Server\" --dir ~/kora/my-server --license-key <key> -y` (see `kora install --help`)."
            );
            return 2;
        }
        let catalog = args.catalog.or_else(|| std::env::var("KORA_CATALOG").ok().filter(|c| !c.is_empty()));
        return interactive_flow(catalog, args.no_app, args.no_desktop_shortcut);
    }
    let args_auto_ports = args.auto_ports;
    let dev_mode = matches!(args.mode, Some(ModeArg::Dev));
    if dev_mode && !args.dry_run {
        match resolve_github_owner(args.github_owner.as_deref()) {
            Ok(l) => args.github_owner = Some(l),
            Err(e) => {
                eprintln!("error: {e}");
                return 2;
            }
        }
    }
    let req = match args.into_request() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let mut req = req;
    let auto = args_auto_ports;
    if !already_installed(&req) {
        match settle_ports(&mut req, auto, &SystemProber) {
            Ok(changes) => changes.iter().for_each(|c| println!("{c}")),
            // A dry run still shows the plan: the plan notes flag the conflict.
            Err(_) if req.dry_run => {}
            Err(e) => {
                eprintln!("error: {e}");
                return 1;
            }
        }
    }
    let dry = req.dry_run;
    if dry {
        print!("{}", request_summary(&req));
    }
    let mut sink = Printer { last: None };
    match installer::run(&req, &mut sink) {
        Ok(out) => {
            print_outcome(&out, dry);
            0
        }
        Err(e) => {
            eprintln!("\nerror: {e}");
            if !dry && req.dir.join(".kora").is_dir() {
                eprintln!("The install state is saved in {}: run the same command again to resume, or `kora delete {}` to remove what was created.", req.dir.join(".kora").display(), req.dir.display());
            }
            1
        }
    }
}

fn status() -> i32 {
    let reg = match registry::Registry::load() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: cannot read the server registry: {e}");
            return 1;
        }
    };
    if reg.servers.is_empty() {
        println!("No Kora servers registered ({}). Install one with `kora install`.", registry::registry_path().display());
        return 0;
    }
    println!("{:<20} {:<5} {:<7} {:<7} {:<9} {:<26} folder", "NAME", "MODE", "GAME", "TXADMIN", "STATE", "PORTS");
    for s in &reg.servers {
        let spec = launch::Spec { name: s.name.clone(), dir: s.dir.clone(), game_port: s.ports.game, txadmin_port: s.ports.txadmin };
        let st = launch::status_of(&launch::SystemEnv::default(), &spec);
        let state = if !s.dir.is_dir() {
            "missing"
        } else if st.running {
            "running"
        } else {
            "stopped"
        };
        let ports = format!("game {}, txAdmin {}", if st.game_listening { "listening" } else { "closed" }, if st.txadmin_up { "up" } else { "down" });
        let mode = if s.mode == Mode::Production { "prod" } else { "dev" };
        println!("{:<20} {:<5} {:<7} {:<7} {:<9} {:<26} {}", s.name, mode, s.ports.game, s.ports.txadmin, state, ports, s.dir.display());
    }
    0
}

fn control(server: &str, action: kora_core::service::ControlAction) -> i32 {
    use kora_core::service::{ControlAction as A, ServiceState};
    let (verb, past) = match action {
        A::Start => ("start", "Started"),
        A::Stop => ("stop", "Stopped"),
        A::Restart => ("restart", "Restarted"),
    };
    let has_service = !matches!(host::service_state(server), Ok(ServiceState::NotInstalled) | Err(_));
    // Without a background service Kora runs FXServer itself (detached, pid in .kora/server.pid).
    if !has_service {
        if matches!(action, A::Stop | A::Restart) {
            match launch::stop_named_detached(server) {
                Ok(true) => println!("Stopped '{server}'."),
                Ok(false) if action == A::Stop => {
                    println!("'{server}' was not running (nothing started by Kora).");
                    return 0;
                }
                Ok(false) => {}
                Err(e) => {
                    eprintln!("error: could not {verb} '{server}': {e}");
                    return 1;
                }
            }
            if action == A::Stop {
                return 0;
            }
        }
        return start_and_report(server, past);
    }
    if action == A::Start {
        return start_and_report(server, past);
    }
    match host::service_control(server, action) {
        Ok(()) => {
            println!("{past} '{server}'.");
            0
        }
        Err(e) => {
            eprintln!("error: could not {verb} '{server}': {e}");
            1
        }
    }
}

/// Start (service or detached), show progress lines and the result.
fn start_and_report(server: &str, past: &str) -> i32 {
    let mut last = String::new();
    let mut progress = |m: &str, _f: f32| {
        if m != last {
            println!("{m}");
            last = m.to_string();
        }
    };
    match launch::start_named(server, &mut progress) {
        Ok(r) => {
            if r.running() {
                println!("{} '{server}': txAdmin is up{}.", if r.method == launch::Method::AlreadyRunning { "Already running" } else { past }, if r.game_listening { ", the game port is listening" } else { ", the game port is not listening yet" });
                0
            } else {
                eprintln!("error: '{server}' did not start properly: {}", r.message);
                for l in &r.log_tail {
                    eprintln!("  | {l}");
                }
                1
            }
        }
        Err(e) => {
            eprintln!("error: could not start '{server}': {e}");
            1
        }
    }
}

/// `kora schedule show|set|off`.
fn schedule(cmd: ScheduleCmd) -> i32 {
    use kora_core::service as svc;
    let find = |server: &str| -> Option<registry::ServerEntry> {
        let reg = registry::Registry::load().unwrap_or_default();
        reg.find(server).cloned().or_else(|| {
            let dir = PathBuf::from(server);
            let sf = registry::ServerFile::load(&dir).ok()?;
            Some(registry::ServerEntry { name: sf.name, dir, mode: sf.mode, ports: registry::PortsRecord { game: sf.ports.game, txadmin: sf.ports.txadmin }, created_unix: 0 })
        })
    };
    let missing = |server: &str| {
        eprintln!("error: '{server}' is not a registered Kora server or server folder (see `kora status`)");
        1
    };
    match cmd {
        ScheduleCmd::Show { server } => {
            let Some(rec) = find(&server) else { return missing(&server) };
            match svc::get(&rec) {
                Some(cfg) => {
                    println!("{}: {}", rec.name, svc::describe(&cfg));
                    if cfg.run_as == svc::RunAs::System {
                        println!("note: the system account has no GitHub sign-in, so forks of private repositories cannot be updated. Switch with: kora schedule set {} --as-user", rec.name);
                    }
                }
                None => println!("{}: no automatic updates are scheduled. Turn them on with: kora schedule set {} --daily 04:00", rec.name, rec.name),
            }
            0
        }
        ScheduleCmd::Set { server, daily, weekly, as_user: _, as_system } => {
            let Some(rec) = find(&server) else { return missing(&server) };
            let (preset, time, weekday) = match (daily, weekly) {
                (_, Some(w)) => {
                    let day = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"].iter().position(|d| w[0].to_lowercase().starts_with(d));
                    let Some(day) = day.filter(|_| w[0].len() >= 3) else {
                        eprintln!("error: '{}' is not a weekday (use mon, tue, wed, thu, fri, sat or sun)", w[0]);
                        return 1;
                    };
                    (svc::Preset::Weekly, w[1].clone(), Some(day as u8))
                }
                (Some(t), None) => (svc::Preset::Daily, t, None),
                (None, None) => {
                    eprintln!("error: say when: --daily HH:MM or --weekly DAY HH:MM");
                    return 1;
                }
            };
            let cfg = svc::ScheduleConfig { preset, time, weekday, run_as: if as_system { svc::RunAs::System } else { svc::RunAs::InstallingUser }, enabled: true };
            if let Err(e) = cfg.validate() {
                eprintln!("error: {e}");
                return 1;
            }
            match svc::set(&rec, &cfg) {
                Ok(()) => {
                    println!("{}: {}", rec.name, svc::describe(&cfg));
                    0
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    1
                }
            }
        }
        ScheduleCmd::Off { server } => {
            let Some(rec) = find(&server) else { return missing(&server) };
            match svc::remove(&rec) {
                Ok(()) => {
                    println!("{}: automatic updates are off", rec.name);
                    0
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    1
                }
            }
        }
    }
}

fn update(a: UpdateArgs) -> i32 {
    use kora_core::update::manual as m;
    if a.auto {
        match gitops::auto_update(&a.server) {
            Ok(report) => {
                println!("{}", report.summary());
                return m::exit_code(&report);
            }
            Err(e) => {
                eprintln!("error: {e}");
                return 1;
            }
        }
    }
    let server = match m::load_server(&a.server) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    if a.list_snapshots {
        let list = m::snapshots(&server);
        if list.is_empty() {
            println!("No snapshots yet. One is saved before every update.");
        }
        for s in list {
            println!("{}  (unix {})  {}", s.id, s.created_unix, s.resources.join(", "));
        }
        return 0;
    }
    if let Some(id) = &a.rollback {
        if !a.yes && !confirm(&format!("Put '{}' back the way snapshot {id} saved it?", server.name)) {
            println!("Nothing changed.");
            return 0;
        }
        return match m::rollback(&server, id) {
            Ok(()) => {
                println!("Rolled back to snapshot {id}.");
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        };
    }
    let scope = if a.all || (a.resource.is_empty() && a.preview) {
        m::Scope::All
    } else if a.resource.is_empty() {
        eprintln!("error: say what to update: --resource <id> (repeatable) or --all. Add --preview to only look.");
        return 1;
    } else {
        m::Scope::Resources(a.resource.clone())
    };
    let pre = match m::preview(&server, scope.clone()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    print!("{}", m::render_preview(&pre));
    if a.preview {
        return 0;
    }
    if !pre.resources.iter().any(|c| c.status == m::ChangeStatus::Clean) {
        return if pre.resources.iter().any(|c| matches!(c.status, m::ChangeStatus::Dirty | m::ChangeStatus::Diverged)) { 3 } else { 0 };
    }
    if !a.yes {
        if !std::io::stdin().is_terminal() {
            eprintln!("error: not asking for confirmation here; add --yes to go ahead.");
            return 1;
        }
        if !confirm("Apply these updates? A snapshot is saved first.") {
            println!("Nothing changed.");
            return 0;
        }
    }
    let restart = match a.restart.unwrap_or(RestartArg::WhenQuiet) {
        RestartArg::None => m::RestartPolicy::None,
        RestartArg::Ensure => m::RestartPolicy::Ensure,
        RestartArg::Full => m::RestartPolicy::FullRestart,
        RestartArg::WhenQuiet => m::RestartPolicy::WhenQuiet,
    };
    let mut snapshot = None;
    let res = m::apply(&server, scope, m::ApplyOpts { restart, skip: Vec::new() }, &mut |e| match e {
        m::UpdateEvent::Snapshot(id) => {
            println!("Saved snapshot {id}.");
            snapshot = Some(id);
        }
        m::UpdateEvent::Resource { id, state } if state == "updating" => println!("Updating {id}..."),
        m::UpdateEvent::Health(h) => println!("{h}."),
        _ => {}
    });
    match res {
        Ok(report) => {
            print!("{}", m::render_report(&report));
            if let Some(id) = snapshot {
                println!("To undo: kora update {} --rollback {id}", a.server);
            }
            m::exit_code(&report)
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn confirm(question: &str) -> bool {
    use std::io::Write;
    print!("{question} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).is_ok() && matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// `kora add-resources <server> --resource ID...`. Browsing the catalog sections
/// (with per-creator drill-in) is in the manager: `kora manager` > Add resources.
fn add_resources(a: AddResourcesArgs) -> i32 {
    if a.code.is_some() {
        eprintln!("{CODE_REMOVED}");
        return 2;
    }
    use kora_core::addresources as add;
    use kora_core::git::SystemRunner;
    let mut server = match kora_core::update::manual::load_server(&a.server) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let ids: Vec<String> = if a.resource.is_empty() {
        eprintln!("error: say what to add: --resource <ID> (repeatable). To browse the catalog, open `kora manager` and choose Add resources, or use `kora panel`.");
        return 2;
    } else {
        a.resource.clone()
    };
    let (cat, note) = match add::load_catalog() {
        Ok(x) => x,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    if let Some(n) = note {
        println!("note: {n}");
    }
    let plan = match add::plan(&cat, &server, &ids) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    for id in &plan.already {
        println!("{id} is already installed.");
    }
    if plan.is_empty() {
        println!("Nothing to add.");
        return 0;
    }
    println!("Will add ({} mode):", if server.mode == Mode::Development { "development" } else { "production" });
    for i in &plan.items {
        let how = if i.third_party { "pinned, never forked" } else if i.fork { "your fork" } else if i.commit.is_some() { "pinned" } else { "branch head" };
        println!("  {}{}  ->  resources/{}  ({how})", i.id, if i.dependency { " (needed by another)" } else { "" }, i.rel);
    }
    println!("server.cfg gets `ensure` lines for: {}", plan.ensure.join(", "));
    if a.dry_run {
        println!("Dry run: nothing was changed.");
        return 0;
    }
    if !a.yes && !confirm("Add these now?") {
        eprintln!("Cancelled.");
        return 1;
    }
    if plan.items.iter().any(|i| i.fork) && !kora_core::git::is_signed_in(&SystemRunner) {
        println!("Forking needs you to be signed in to GitHub.");
        if let Err(e) = kora_core::git::auth_login_interactive() {
            eprintln!("error: could not sign in to GitHub ({e}).");
            return 1;
        }
    }
    let cloner = add::GitCloner(&SystemRunner);
    match add::execute(&cloner, &mut server, &plan, &mut |m| println!("{m}...")) {
        Ok(out) => {
            for id in &out.installed {
                println!("[ok] {id}");
            }
            for (id, e) in &out.failed {
                eprintln!("[!!] {id}: {e}");
            }
            for n in &out.notes {
                println!("note: {n}");
            }
            if !out.ensured.is_empty() {
                println!("Added to server.cfg: {}", out.ensured.iter().map(|f| format!("ensure {f}")).collect::<Vec<_>>().join(", "));
            }
            if !out.installed.is_empty() {
                println!("Restart the server to load the new resources.");
            }
            i32::from(!out.ok())
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn delete(server: &str, keep_files: bool, keep_db: bool, yes: bool) -> i32 {
    let reg = registry::Registry::load().unwrap_or_default();
    let (name, dir) = match reg.find(server) {
        Some(e) => (e.name.clone(), e.dir.clone()),
        None => (server.to_string(), PathBuf::from(server)),
    };
    if !dir.join(".kora").is_dir() {
        eprintln!("error: '{server}' is not a Kora server folder or registered server");
        return 1;
    }
    if !yes {
        println!("This removes: {}{}; the registry entry. GitHub forks are never deleted.", if keep_files { "(files kept)" } else { "the server folder and everything in it" }, if keep_db { "" } else { "; the database and its user" });
        println!("Type the server name ({name}) to confirm:");
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        if line.trim() != name {
            eprintln!("Cancelled.");
            return 1;
        }
    }
    let mut sink = Printer { last: None };
    match installer::rollback(&dir, &RollbackOptions { keep_files, keep_db }, &mut sink) {
        Ok(notes) => {
            for n in notes {
                println!("note: {n}");
            }
            println!("Deleted '{name}'.");
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// Asks on the terminal; an empty answer takes the default.
struct Terminal;

impl installer::uninstall::Asker for Terminal {
    fn ask(&mut self, question: &str, default_yes: bool) -> bool {
        use std::io::Write;
        print!("{question} {} ", if default_yes { "[Y/n]" } else { "[y/N]" });
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() {
            return false;
        }
        match line.trim().to_ascii_lowercase().as_str() {
            "" => default_yes,
            "y" | "yes" => true,
            _ => false,
        }
    }
}

fn uninstall(server: Option<String>, dry_run: bool, yes: bool, delete_files: bool, delete_databases: bool) -> i32 {
    use installer::uninstall as un;
    if !yes && !dry_run && !std::io::stdin().is_terminal() {
        eprintln!("error: `kora uninstall` asks before each removal and needs a terminal. Use --dry-run to preview, or --yes to remove the system changes without questions (folders and databases are kept).");
        return 2;
    }
    let mut reg = match registry::Registry::load() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: cannot read the server registry: {e}");
            return 1;
        }
    };
    let admin = installer::is_admin();
    if !admin && !dry_run {
        eprintln!("note: not running as administrator/root: removing services, firewall rules and accounts will be refused. Re-run with sudo / as Administrator.");
    }
    let runner = kora_core::service::SystemRunner;
    let env = un::Env { platform: kora_core::service::Platform::current(), runner: &runner, admin };
    let opts = un::Options { dry_run, only: server, delete_files, delete_databases };
    let mut say = |s: &str| println!("{s}");
    let mut delete = |dir: &std::path::Path, o: &RollbackOptions| installer::rollback(dir, o, &mut Printer { last: None });
    let mut save = |r: &registry::Registry| r.save_or_clean_to(&registry::registry_path());
    let summary = if yes {
        un::run(&mut reg, &opts, &env, &mut un::Defaults, &mut say, &mut delete, &mut save)
    } else {
        un::run(&mut reg, &opts, &env, &mut Terminal, &mut say, &mut delete, &mut save)
    };
    let mut summary = summary;
    {
        let base = installer::windows_db::base_dir();
        let record = base.as_ref().and_then(|b| installer::windows_db::load_record(b).map(|r| (r, b.clone())));
        let mut say = |s: &str| println!("{s}");
        let mut remove = |r: &installer::windows_db::Record, b: &std::path::Path| installer::windows_db::remove(r, b, &mut |m| println!("  {m}"));
        if yes {
            un::mariadb_cleanup(&reg, &opts, admin, record, &mut un::Defaults, &mut say, &mut remove, &mut summary);
        } else {
            un::mariadb_cleanup(&reg, &opts, admin, record, &mut Terminal, &mut say, &mut remove, &mut summary);
        }
    }
    if dry_run {
        println!("\nDry run: nothing was changed.");
        if kora_core::app_install::status().installed {
            println!("The Kora app would stay installed (remove it with `kora uninstall --app`).");
        }
        return 0;
    }
    println!("\nRemoved: {}. Kept: {}. Failed: {}.", summary.removed.len(), summary.kept.len(), summary.failed.len());
    if kora_core::app_install::status().installed {
        println!("The Kora app itself (the `kora` command and its shortcuts) is still installed. Remove it with `kora uninstall --app`.");
    }
    for f in &summary.failed {
        eprintln!("  failed: {f}");
    }
    i32::from(!summary.ok())
}

fn panel(a: &PanelArgs) -> i32 {
    let opts = kora_web::PanelOptions { port: a.port, open_browser: !a.no_open, ..Default::default() };
    match kora_web::run(opts) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn command_kind(cmd: &Option<Command>) -> elevate::CmdKind {
    use elevate::CmdKind as K;
    match cmd {
        None => K::Wizard,
        Some(Command::Install(a)) => K::Install { dry_run: a.dry_run },
        Some(Command::Manager) => K::Manager,
        Some(Command::Update(a)) => K::Update { auto: a.auto },
        Some(Command::Status | Command::Schedule { cmd: ScheduleCmd::Show { .. } }) => K::Status,
        Some(Command::Schedule { .. }) => K::Restart,
        Some(Command::Delete { .. }) => K::Delete,
        // The panel runs as the user; privileged actions report the command to run instead.
        Some(Command::ServiceHost { .. } | Command::Github { .. } | Command::Webhook { .. } | Command::SelfUpdate { .. } | Command::Panel(_)) => K::Other,
        // Writes into the server folder, like a manual update: same privilege rules.
        Some(Command::AddResources(_)) => K::Update { auto: false },
        Some(Command::Start { .. }) => K::Start,
        Some(Command::Stop { .. }) => K::Stop,
        Some(Command::Restart { .. }) => K::Restart,
        Some(Command::Uninstall { dry_run, .. }) => K::Uninstall { dry_run: *dry_run },
        Some(Command::App { cmd }) => match cmd {
            AppCmd::Install { dry_run, .. } | AppCmd::Uninstall { dry_run, .. } => K::Uninstall { dry_run: *dry_run },
            AppCmd::Status => K::Status,
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
        Some(Command::Start { server } | Command::Stop { server } | Command::Restart { server }) => {
            match host::service_state(server) {
                Ok(ServiceState::NotInstalled) | Err(_) => vec![],
                Ok(_) => one("control the server's background service"),
            }
        }
        Some(Command::Delete { server, .. }) => {
            let reg = registry::Registry::load().unwrap_or_default();
            let dir = reg.find(server).map(|e| e.dir.clone()).unwrap_or_else(|| PathBuf::from(server));
            match installer::load_created_from(&dir) {
                Some(c) if c.service_unit.is_some() || c.firewall || c.update_timer => one("remove the server's background service and firewall rules"),
                _ => vec![],
            }
        }
        Some(Command::Schedule { cmd: ScheduleCmd::Set { .. } | ScheduleCmd::Off { .. } }) => one("change the automatic-update schedule"),
        Some(Command::Uninstall { dry_run: false, app: true, .. }) => one("remove the Kora app (the command, shortcuts and the installed program)"),
        Some(Command::Uninstall { dry_run: false, .. }) => one("remove the services, firewall rules and accounts Kora set up"),
        Some(Command::App { cmd: AppCmd::Install { dry_run: false, .. } | AppCmd::Uninstall { dry_run: false, .. } }) => one("install or remove the Kora app (the command, shortcuts and the installed program)"),
        _ => vec![],
    }
}

/// The arguments for the elevated copy: the original ones, plus the folder when it was only implied
/// (a different account's home folder would otherwise change where the server goes).
fn relaunch_args(cmd: &Option<Command>, original: Vec<std::ffi::OsString>) -> Vec<std::ffi::OsString> {
    let mut args = original;
    if let Some(Command::Install(a)) = cmd {
        if a.dir.is_none() {
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
static ALREADY_PAUSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn finish(code: i32) -> ! {
    let (si, so) = (std::io::stdin().is_terminal(), std::io::stdout().is_terminal());
    if !ALREADY_PAUSED.load(std::sync::atomic::Ordering::Relaxed)
        && elevate::should_pause(cfg!(windows), elevate::owns_console(), elevate::was_relaunched(), si, so) {
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
    if std::env::args_os().any(|a| a == "--license-key" || a.to_string_lossy().starts_with("--license-key=")) {
        eprintln!("Note: a licence key on the command line is visible to other programs on this computer (process list) and stays in your shell history. Prefer the CFX_LICENSE_KEY environment variable.");
    }
    // Before parsing: `--version`/`--help` exit inside the parser and must clean up too.
    kora_core::self_update::startup_cleanup();
    let cli = Cli::parse();
    // Bare `kora` in a terminal: the manager when servers exist, the install wizard otherwise.
    let tty = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let bare = match &cli.command {
        None if tty => Some(default_command(registered_servers())),
        _ => None,
    };
    elevate_if_needed(&cli, if bare.is_some() { &bare } else { &cli.command });
    let command = if bare.is_some() { bare } else { cli.command };
    // "New version available": background check, interactive runs only (never CI, pipes, services).
    let quiet = matches!(command, Some(Command::ServiceHost { .. } | Command::SelfUpdate { .. } | Command::Update(UpdateArgs { auto: true, .. })));
    let pending = if quiet { None } else { kora_core::self_update::update_check::Pending::start(tty) };
    let code = match command {
        None => install(InstallArgs::default()),
        Some(Command::Install(a)) => install(*a),
        Some(Command::Manager) => manager(),
        Some(Command::Panel(a)) => panel(&a),
        Some(Command::Update(a)) => update(a),
        Some(Command::AddResources(a)) => add_resources(a),
        Some(Command::Status) => status(),
        Some(Command::SelfUpdate { check, yes, version }) => kora_core::self_update::cli::run(&kora_core::self_update::cli::Options { check, yes, version }),
        Some(Command::Webhook { cmd }) => webhook(cmd),
        Some(Command::Schedule { cmd }) => schedule(cmd),
        Some(Command::Github { cmd: GithubCmd::Owners }) => github_owners(),
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
        Some(Command::Uninstall { app: true, yes, dry_run, .. }) => app_uninstall(yes, dry_run),
        Some(Command::Uninstall { server, dry_run, yes, delete_files, delete_databases, app: false }) => uninstall(server, dry_run, yes, delete_files, delete_databases),
        Some(Command::App { cmd }) => match cmd {
            AppCmd::Install { force, no_desktop_shortcut, dry_run, .. } => app_install(force, no_desktop_shortcut, dry_run),
            AppCmd::Status => app_status(),
            AppCmd::Uninstall { yes, dry_run } => app_uninstall(yes, dry_run),
        },
    };
    if let Some(line) = pending.and_then(|p| p.finish(std::time::Duration::from_secs(2))) {
        println!("\n{line}");
    }
    finish(code);
}

#[cfg(test)]
mod parity;

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_add(args: &[&str]) -> Result<AddResourcesArgs, clap::Error> {
        let mut v = vec!["kora", "add-resources"];
        v.extend_from_slice(args);
        match Cli::try_parse_from(v)?.command {
            Some(Command::AddResources(a)) => Ok(a),
            _ => unreachable!(),
        }
    }

    #[test]
    fn add_resources_arguments() {
        let a = parse_add(&["srv", "--resource", "a", "--resource", "b", "--dry-run"]).unwrap();
        assert_eq!((a.server.as_str(), a.resource.len(), a.dry_run, a.code.is_none()), ("srv", 2, true, true));
        let a = parse_add(&["srv", "--code", "KORA-7F3K-92QX", "-y"]).unwrap();
        assert!(a.code.is_some() && a.yes, "--code still parses (it fails at run time with exit 2)");
        assert_eq!(add_resources(a), 2, "setup codes were removed");
        assert!(parse_add(&[]).is_err(), "the server is required");
    }

    fn parse_update(args: &[&str]) -> Result<UpdateArgs, clap::Error> {
        let mut v = vec!["kora", "update"];
        v.extend_from_slice(args);
        match Cli::try_parse_from(v)?.command {
            Some(Command::Update(a)) => Ok(a),
            _ => unreachable!(),
        }
    }

    #[test]
    fn bare_kora_opens_the_manager_with_servers_and_the_wizard_without() {
        for (registered, manager) in [(0, false), (1, true), (2, true), (50, true)] {
            match default_command(registered) {
                Command::Manager => assert!(manager, "{registered} servers"),
                Command::Install(a) => assert!(!manager && !a.any_given(), "{registered} servers: the plain wizard"),
                _ => panic!("unexpected default command"),
            }
        }
    }

    #[test]
    fn update_flags_parse() {
        let a = parse_update(&["main", "--resource", "kora-chat", "--resource", "kora-hud", "--yes", "--restart", "when-quiet"]).unwrap();
        assert_eq!((a.server.as_str(), a.resource.len(), a.yes, a.restart), ("main", 2, true, Some(RestartArg::WhenQuiet)));
        assert_eq!(parse_update(&["main", "--all", "--preview"]).map(|a| (a.all, a.preview)).unwrap(), (true, true));
        assert_eq!(parse_update(&["main", "--all", "--restart", "full"]).unwrap().restart, Some(RestartArg::Full));
        assert_eq!(parse_update(&["main", "--rollback", "123"]).unwrap().rollback.as_deref(), Some("123"));
        assert!(parse_update(&["main", "--list-snapshots"]).unwrap().list_snapshots);
        assert!(parse_update(&["--auto", "/srv/main"]).unwrap().auto);
    }

    #[test]
    fn update_flag_conflicts_are_rejected() {
        for bad in [
            &["main", "--all", "--resource", "x"][..],
            &["main", "--auto", "--all"],
            &["main", "--rollback", "1", "--all"],
            &["main", "--preview", "--rollback", "1"],
            &["main", "--restart", "sometimes"],
            &["--all"],
        ] {
            assert!(parse_update(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn webhook_subcommands_parse() {
        let p = |a: &[&str]| Cli::try_parse_from(std::iter::once("kora").chain(a.iter().copied()));
        assert!(p(&["webhook", "set", "alpha", "--stdin", "--test"]).is_ok());
        assert!(p(&["webhook", "set", "alpha", "https://discord.com/api/webhooks/1/t"]).is_ok());
        assert!(p(&["webhook", "test", "alpha"]).is_ok());
        assert!(p(&["webhook", "show", "alpha"]).is_ok());
        assert!(p(&["webhook", "remove", "alpha"]).is_ok());
        assert!(p(&["webhook", "set"]).is_err());
        assert!(p(&["webhook"]).is_err());
    }

    #[test]
    fn real_installs_include_the_app_unless_told_not_to_and_dry_runs_do_not() {
        let app = |a: InstallArgs| a.into_request().unwrap().app;
        assert_eq!(app(InstallArgs::default()), Some(kora_core::app_install::AppChoice { desktop_shortcut: true }));
        assert_eq!(app(InstallArgs { no_desktop_shortcut: true, ..Default::default() }), Some(kora_core::app_install::AppChoice { desktop_shortcut: false }));
        assert_eq!(app(InstallArgs { no_app: true, ..Default::default() }), None);
        assert_eq!(app(InstallArgs { dry_run: true, ..Default::default() }), None);
        assert!(!InstallArgs { no_app: true, ..Default::default() }.any_given(), "--no-app alone still opens the wizard");
    }

    #[test]
    fn app_commands_are_classified_for_elevation() {
        use elevate::CmdKind as K;
        let parse = |a: &[&str]| Cli::try_parse_from(std::iter::once("kora").chain(a.iter().copied())).unwrap().command;
        assert_eq!(command_kind(&parse(&["app", "install", "--yes"])), K::Uninstall { dry_run: false });
        assert_eq!(command_kind(&parse(&["app", "install", "--dry-run"])), K::Uninstall { dry_run: true });
        assert_eq!(command_kind(&parse(&["app", "status"])), K::Status);
        assert!(matches!(parse(&["uninstall", "--app", "-y"]), Some(Command::Uninstall { app: true, yes: true, .. })));
    }

    #[test]
    fn home_dir_prefers_the_platform_variable() {
        let env = |k: &str| match k {
            "HOME" => Some("/c/Users/Git Bash Home".into()),
            "USERPROFILE" => Some("C:\\Users\\Real".into()),
            _ => None,
        };
        assert_eq!(home_dir(true, &env), PathBuf::from("C:\\Users\\Real"));
        assert_eq!(home_dir(false, &env), PathBuf::from("/c/Users/Git Bash Home"));
        let only_home = |k: &str| (k == "HOME").then(|| "/home/me".into());
        assert_eq!(home_dir(true, &only_home), PathBuf::from("/home/me"), "falls back");
        let empty = |k: &str| (k == "USERPROFILE").then(|| "".into());
        assert_eq!(home_dir(true, &empty), PathBuf::new());
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
    fn hostname_and_game_build_flags() {
        assert!(InstallArgs { hostname: Some("x".into()), ..Default::default() }.any_given());
        assert!(InstallArgs { game_build: Some(3258), ..Default::default() }.any_given());
        let r = InstallArgs { name: Some("Alpha".into()), ..Default::default() }.into_request().unwrap();
        assert_eq!((r.hostname.as_str(), r.game_build), ("Alpha", None), "hostname defaults to the name");
        let r = InstallArgs { name: Some("Alpha".into()), hostname: Some("Alpha RP".into()), game_build: Some(3258), ..Default::default() }.into_request().unwrap();
        assert_eq!((r.hostname.as_str(), r.game_build), ("Alpha RP", Some(3258)));
    }

    #[test]
    fn install_code_is_rejected_with_exit_2() {
        assert_eq!(install(InstallArgs { code: Some("KORA-7F3K-92QX".into()), ..Default::default() }), 2);
    }

    /// Fake `gh`: `None` = not installed, `Some(false)` = signed out, `Some(true)` = signed in as `me` (org `Team`).
    struct FakeGh(Option<bool>, std::sync::Mutex<Vec<String>>);
    impl kora_core::git::CommandRunner for FakeGh {
        fn run(&self, p: &str, a: &[&str], _: Option<&std::path::Path>) -> std::io::Result<kora_core::git::Output> {
            use kora_core::git::Output;
            let line = format!("{p} {}", a.join(" "));
            self.1.lock().unwrap().push(line.clone());
            let ok = |s: &str| Ok(Output { code: 0, stdout: s.into(), stderr: String::new() });
            match self.0 {
                None => Err(std::io::Error::new(std::io::ErrorKind::NotFound, "gh not found")),
                Some(false) => Ok(Output { code: 1, stdout: String::new(), stderr: "not logged in".into() }),
                Some(true) => match line.as_str() {
                    "gh auth status --hostname github.com" => ok(""),
                    "gh api user" => ok(r#"{"login":"me"}"#),
                    l if l.starts_with("gh api user/orgs") => ok(r#"[{"login":"Team"}]"#),
                    l if l.starts_with("gh api user/memberships/orgs/") => ok(r#"{"state":"active","role":"member"}"#),
                    l if l.starts_with("gh api orgs/") => ok(r#"{"members_can_create_repositories":true}"#),
                    _ => Ok(Output { code: 1, stdout: String::new(), stderr: "unexpected".into() }),
                },
            }
        }
        fn sleep(&self, _: std::time::Duration) {}
    }

    #[test]
    fn github_owner_syntax_is_rejected_immediately() {
        for bad in ["", "-a", "a-", "a b", "a/b", &"x".repeat(40)] {
            let g = FakeGh(Some(true), Default::default());
            assert!(resolve_github_owner_with(&g, Some(bad)).is_err(), "{bad:?}");
            assert!(g.1.lock().unwrap().is_empty(), "gh is not consulted for {bad:?}");
        }
    }

    #[test]
    fn github_owner_is_verified_when_gh_is_ready() {
        let g = FakeGh(Some(true), Default::default());
        assert_eq!(resolve_github_owner_with(&g, Some("team")).unwrap(), "Team");
        let e = resolve_github_owner_with(&g, Some("nobody")).unwrap_err();
        assert!(e.contains("not your GitHub account") && e.contains("Team"), "{e}");
    }

    #[test]
    fn github_owner_is_deferred_when_gh_is_missing_or_signed_out() {
        for state in [None, Some(false)] {
            let g = FakeGh(state, Default::default());
            assert_eq!(resolve_github_owner_with(&g, Some("someone")).unwrap(), "someone", "install proceeds to the gh bootstrap step");
            assert!(!g.1.lock().unwrap().iter().any(|c| c.contains("repo fork")));
        }
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

    #[test]
    fn dry_run_summary_shows_choices_but_no_secrets() {
        let a = InstallArgs {
            license_key: Some("cfxk_secret".into()),
            name: Some("Alpha One".into()),
            mode: Some(ModeArg::Dev),
            resources: Some("a,b".into()),
            game_port: Some(31120),
            txadmin_port: Some(41120),
            artifact_url: Some("https://runtime.fivem.net/x".into()),
            game_build: Some(3258),
            cfx_identifier: Some("fivem:1".into()),
            ..Default::default()
        };
        let text = request_summary(&a.into_request().unwrap());
        for want in ["Alpha One", "development", "2 chosen (a, b)", "game 31120, txAdmin 41120", "https://runtime.fivem.net/x", "3258", "fivem:1"] {
            assert!(text.contains(want), "missing {want}: {text}");
        }
        assert!(!text.contains("cfxk_secret"));
    }

    struct Busy(Vec<u16>);
    impl PortProber for Busy {
        fn tcp_free(&self, p: u16) -> bool {
            !self.0.contains(&p)
        }
        fn udp_free(&self, _: u16) -> bool {
            true
        }
    }

    fn req_with_ports(game: u16, tx: u16) -> InstallRequest {
        let mut a = InstallArgs { game_port: Some(game), txadmin_port: Some(tx), dir: Some("/nonexistent/kora-ports-test".into()), ..Default::default() };
        a.name = Some("kora-ports-test".into());
        a.into_request().unwrap()
    }

    #[test]
    fn busy_port_fails_with_a_message_naming_port_and_next_free() {
        let mut r = req_with_ports(30120, 40120);
        let e = settle_ports_avoiding(&mut r, false, &Busy(vec![30120]), &[]).unwrap_err();
        assert!(e.contains("Port 30120 (game)") && e.contains("next free port is 30121") && e.contains("--game-port 30121 --txadmin-port 40120") && e.contains("--auto-ports"), "{e}");
        assert_eq!(r.ports.game, 30120, "nothing changed without --auto-ports");
        assert!(settle_ports_avoiding(&mut r, false, &Busy(vec![]), &[]).unwrap().is_empty());
    }

    #[test]
    fn auto_ports_picks_next_free_and_says_so() {
        let mut r = req_with_ports(30120, 40120);
        let changes = settle_ports_avoiding(&mut r, true, &Busy(vec![30120, 30121, 40120]), &[]).unwrap();
        assert_eq!(changes, vec!["Port 30120 is in use, using 30122", "Port 40120 is in use, using 40121"]);
        assert_eq!((r.ports.game, r.ports.txadmin), (30122, 40121));
        let mut r = req_with_ports(65535, 40120);
        assert!(settle_ports_avoiding(&mut r, true, &Busy(vec![65535]), &[]).is_err(), "exhausted");
    }

    #[test]
    fn ports_from_flags_are_rechecked_locally() {
        let a = InstallArgs { auto_ports: true, game_port: Some(31120), txadmin_port: Some(41120), dir: Some("/nonexistent/kora-ports-test".into()), ..Default::default() };
        let mut r = a.into_request().unwrap();
        let changes = settle_ports_avoiding(&mut r, true, &Busy(vec![31120]), &[]).unwrap();
        assert_eq!(changes, vec!["Port 31120 is in use, using 31121"]);
    }

    #[test]
    fn dry_run_shows_the_effective_ports() {
        let mut r = req_with_ports(30120, 40120);
        settle_ports_avoiding(&mut r, true, &Busy(vec![30120]), &[]).unwrap();
        assert!(request_summary(&r).contains("game 30121, txAdmin 40120"));
    }

    #[test]
    fn headless_start_is_off_unless_asked() {
        let mk = |a: &[&str]| {
            let mut v = vec!["kora", "install", "-y"];
            v.extend_from_slice(a);
            Cli::try_parse_from(v)
        };
        let req = |a: &[&str]| match mk(a).unwrap().command {
            Some(Command::Install(i)) => i.into_request().unwrap().start,
            _ => unreachable!(),
        };
        assert!(!req(&[]), "existing smoke/CI behaviour is unchanged");
        assert!(req(&["--start"]));
        assert!(!req(&["--no-start"]));
        assert!(mk(&["--start", "--no-start"]).is_err());
    }
}
