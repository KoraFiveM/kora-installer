//! `kora` command line entry point.

use std::io::IsTerminal;
use std::path::PathBuf;

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
    /// Hidden from help; it still parses so an old script gets the explanation instead of a usage error.
    #[arg(long, value_name = "CODE", hide = true)]
    code: Option<String>,
    /// Server name (display + registry name).
    #[arg(long)]
    name: Option<String>,
    /// Server folder (default: ~/kora/<name> in your home folder, also with sudo; /home/kora/<name> when logged in as root; Windows: C:\Kora\<name>, the same as in the panel).
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
    /// Your Cfx.re username (the name on forum.cfx.re). Kora looks up your account id and makes you the txAdmin admin.
    #[arg(long, value_name = "NAME", conflicts_with = "cfx_identifier")]
    cfx_user: Option<String>,
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
    /// Accepted and ignored since 0.7.0 (the install no longer adds a machine-wide Kora app).
    #[arg(long, hide = true)]
    no_app: bool,
    /// Accepted and ignored since 0.7.0 (shortcuts are made by the install scripts and `kora app install`).
    #[arg(long, hide = true)]
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
    /// Linux, with --service: when the server folder is inside a private folder (such as your home
    /// folder) that the service's own account cannot enter, let it pass through exactly those
    /// folders (`chmod o+x`) instead of stopping.
    #[arg(long)]
    fix_permissions: bool,
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
            || self.cfx_user.is_some()
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
            || self.fix_permissions
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
            None => default_dir(&name)?,
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
            // Retired in 0.7.0: the install no longer adds a machine-wide app.
            app: None,
        })
    }
}

/// The folder used when `--dir` is not given: the same one the panel suggests
/// (`kora_core::userhome::default_server_dir`). Windows: `C:\Kora\<name>`. Linux: the home folder of
/// the person installing, also under sudo (`/home/kora/<name>` when logged in as root). Never relative.
fn default_dir(name: &str) -> Result<PathBuf, String> {
    kora_core::userhome::default_server_dir(&installer::slug(name))
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
    #[arg(long, conflicts_with_all = ["resource", "all", "preview", "yes", "restart", "rollback", "list_snapshots", "force"])]
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
    /// With --rollback: roll back even resources with uncommitted changes of your own (they are thrown away).
    #[arg(long, requires = "rollback")]
    force: bool,
}

#[derive(Args, Debug, Default)]
struct AddResourcesArgs {
    /// Server name or folder.
    server: String,
    /// REMOVED in 0.4.0 (setup codes no longer exist): use --resource or `kora panel`.
    /// Hidden from help; it still parses so an old script gets the explanation instead of a usage error.
    #[arg(long, value_name = "CODE", hide = true)]
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
struct AddAssetArgs {
    /// Server name or folder.
    server: String,
    /// The .zip file(s) you downloaded from the Cfx.re Portal (as they are; do not unpack them).
    #[arg(required = true, value_name = "ZIP")]
    zip: Vec<std::path::PathBuf>,
    /// Show what would be imported, then stop.
    #[arg(long)]
    dry_run: bool,
    /// Do not ask for confirmation.
    #[arg(long, short = 'y')]
    yes: bool,
}

#[derive(Args, Debug)]
struct PanelArgs {
    /// Port to listen on. Default: 47120, or the next free one (up to 47140) when something else uses it.
    #[arg(long)]
    port: Option<u16>,
    /// Address to listen on. The default, 127.0.0.1, is this computer only. Anything else needs remote access (--remote or saved settings) and is plain HTTP: use it only on a private network such as Tailscale or WireGuard.
    #[arg(long, default_value = "127.0.0.1")]
    bind: std::net::IpAddr,
    /// Allow remote access for this run (needed to listen on an address other than 127.0.0.1 unless it is already switched on in the settings).
    #[arg(long)]
    remote: bool,
    /// Open the panel to your own devices over Tailscale: it keeps running in the background, also listens on this computer's Tailscale address, and you sign in with your Tailscale account (no link, no tunnel). Run it once; with --foreground it runs in this window instead.
    #[arg(long, conflicts_with_all = ["remove_service", "print_link", "setup", "new_token"])]
    tailscale: bool,
    /// With --tailscale: the Tailscale account (name@example.com) that becomes the panel's admin. Without it Kora uses the account you are connected from, else the one that added this computer to Tailscale.
    #[arg(long, value_name = "LOGIN", requires = "tailscale")]
    admin: Option<String>,
    /// Also listen on this computer's Tailscale addresses, with the saved settings (the background service uses this; `--tailscale` sets it up).
    #[arg(long, hide = true, conflicts_with = "tailscale")]
    listen_tailscale: bool,
    /// Let --bind be a public address or every interface (0.0.0.0). The panel is plain HTTP, so on the internet sign-in tokens could be read on the way: use --tailscale or an SSH tunnel instead.
    #[arg(long)]
    allow_public: bool,
    /// Make a new sign-in token, so links shown before stop working (restarts the background panel).
    #[arg(long, conflicts_with_all = ["install_service", "remove_service", "print_link", "setup", "foreground"])]
    new_token: bool,
    /// Do not open a browser; print the address and sign-in link instead (automatic over SSH or without a desktop).
    #[arg(long)]
    no_open: bool,
    /// Run the panel in this window and stop it when the window closes: do not offer or use the background panel.
    #[arg(long, conflicts_with_all = ["install_service", "remove_service", "print_link", "setup"])]
    foreground: bool,
    /// Make sure the background panel is set up and running, then open it in your browser and print the sign-in link (the install scripts run this). Never reads standard input; on a server with Tailscale it asks once on the terminal whether to use it (KORA_TAILSCALE=1 or 0 answers in advance).
    #[arg(long, conflicts_with_all = ["install_service", "remove_service", "print_link"])]
    setup: bool,
    /// Keep the panel running in the background so monitoring and alerts work while it is closed (starts at sign-in; no administrator rights needed).
    #[arg(long, conflicts_with_all = ["remove_service", "print_link"])]
    install_service: bool,
    /// Stop and remove the background panel installed with --install-service.
    #[arg(long, conflicts_with_all = ["install_service", "print_link"])]
    remove_service: bool,
    /// Print the sign-in link of the background panel.
    #[arg(long, conflicts_with_all = ["install_service", "remove_service"])]
    print_link: bool,
    #[command(subcommand)]
    cmd: Option<PanelCmd>,
}

#[derive(Subcommand, Debug)]
enum PanelCmd {
    /// Manage the people who may sign in to the panel remotely (each has a token and a role).
    User {
        #[command(subcommand)]
        cmd: PanelUserCmd,
    },
    /// Manage who may sign in with their Tailscale account (set it up with `kora panel --tailscale`).
    Tailscale {
        #[command(subcommand)]
        cmd: panel_tailscale::TailscaleCmd,
    },
    /// Restart the background panel (the one that keeps running after you close the window). The sign-in link stays the same; open pages sign in again.
    Restart,
    /// Let people who sign in from another computer with a token do the risky things too: delete and unregister servers, uninstall Kora, send console commands, change schedules and update Kora. Off by default. (Token administrators can already install servers; Tailscale administrators can do all of this without the switch.)
    RemoteActions {
        #[command(subcommand)]
        cmd: RemoteActionsCmd,
    },
}

#[derive(Subcommand, Debug, Clone, Copy, PartialEq, Eq)]
enum RemoteActionsCmd {
    /// Allow these actions for administrators who sign in with a token.
    On,
    /// Refuse these actions for token sign-ins from other computers (the default).
    Off,
    /// Show whether they are on or off.
    Status,
}

#[derive(Subcommand, Debug)]
enum PanelUserCmd {
    /// Add a user and print their token once (it cannot be shown again).
    Add {
        /// A name for this person (letters, numbers, dots, dashes, underscores).
        name: String,
        /// What they may do: viewer (look only), operator (run servers, updates, logs) or admin (everything).
        #[arg(long, value_parser = ["viewer", "operator", "admin"])]
        role: String,
    },
    /// List the users and their roles.
    List,
    /// Remove a user; their sessions stop working at once.
    Remove {
        /// The user to remove.
        name: String,
    },
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
    #[command(after_help = "Download the assets you bought from https://portal.cfx.re/assets/granted-assets (Assets → Granted Assets), signed in with the same Cfx.re account that owns this server's licence key, then pass the .zip files here as they are.")]
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
    /// GitHub helpers for development mode.
    Github {
        #[command(subcommand)]
        cmd: GithubCmd,
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
}

#[derive(Subcommand)]
enum HelperCmd {
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
        Some(Command::SelfUpdate { version: Some(v), .. })
            if verb == "self-update" && kora_core::app_install::compare_versions(&v, env!("CARGO_PKG_VERSION")) != Some(std::cmp::Ordering::Greater) =>
        {
            Err(format!("The helper only updates Kora to a newer version. To go back to {v}, run `sudo kora self-update --version {v}` yourself."))
        }
        Some(Command::SelfUpdate { .. }) if verb == "self-update" => Ok(()),
        Some(Command::Schedule { cmd: ScheduleCmd::Set { .. } | ScheduleCmd::Off { .. } }) if verb == "schedule" => Ok(()),
        Some(Command::App { cmd: AppCmd::Install { .. } | AppCmd::Uninstall { .. } }) if verb == "app" => Ok(()),
        Some(Command::Uninstall { .. }) if verb == "uninstall" => Ok(()),
        _ => deny(),
    }
}

fn helper(cmd: HelperCmd) -> i32 {
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
    /// Make the Kora shortcuts that open the panel (per user, no administrator rights; idempotent).
    Install {
        /// Do not ask (nothing is asked; accepted for scripts).
        #[arg(long, short = 'y')]
        yes: bool,
        /// No desktop shortcut (Windows).
        #[arg(long)]
        no_desktop_shortcut: bool,
        /// Accepted and ignored since 0.7.0.
        #[arg(long, hide = true)]
        force: bool,
        /// List every change and make none.
        #[arg(long)]
        dry_run: bool,
    },
    /// Show the shortcuts and whether the old machine-wide app is still here (`key: value` lines).
    Status,
    /// Remove Kora's shortcuts (Start Menu / Desktop on Windows, the applications-menu launcher on Linux). If the
    /// OLD machine-wide app is also installed it is removed too when you run this as administrator; otherwise
    /// `kora uninstall --app` (as administrator) does that. Servers and databases are never touched.
    Uninstall {
        #[arg(long, short = 'y')]
        yes: bool,
        #[arg(long)]
        dry_run: bool,
    },
}

fn print_lines(prefix: &str, lines: &[String]) {
    for l in lines {
        println!("{prefix}{l}");
    }
}

/// `kora app install`: the per-user shortcuts that open the panel. No administrator rights.
fn app_install(no_desktop_shortcut: bool, dry_run: bool) -> i32 {
    use kora_core::app_install::user;
    match user::install_user(&user::UserOptions { desktop_shortcut: !no_desktop_shortcut, dry_run }) {
        Ok(out) => {
            if dry_run {
                println!("Dry run: nothing was changed. Making the Kora shortcuts would:");
            } else if out.changes.is_empty() {
                println!("The Kora shortcuts are already in place ({}).", out.exe.display());
            } else {
                println!("The Kora shortcuts are ready ({}):", out.exe.display());
            }
            print_lines("  - ", &out.changes);
            print_lines("  note: ", &out.notes);
            if user::legacy_present().present() {
                println!("The old machine-wide Kora app is still installed. Remove it with `kora uninstall --app` (as administrator), or use \"Tidy up old shortcuts\" in the panel.");
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn app_status() -> i32 {
    use kora_core::app_install::user;
    let st = user::user_status();
    let yn = |b: Option<bool>| b.map_or("n/a", |b| if b { "yes" } else { "no" });
    println!("shortcut_start_menu: {}", yn(st.start_menu));
    println!("shortcut_desktop: {}", yn(st.desktop));
    println!("launcher: {}", yn(st.launcher));
    let legacy = user::legacy_present();
    println!("old_app_present: {}", if legacy.present() { "yes" } else { "no" });
    print_lines("old_app_item: ", &legacy.items);
    print_lines("", &kora_core::app_install::status().lines());
    0
}

/// `kora app uninstall` (`shortcuts`: also Kora's own per-user shortcuts, no administrator rights) and
/// `kora uninstall --app` (only the OLD machine-wide app, which is what "Tidy up old shortcuts" runs as administrator).
fn app_uninstall(yes: bool, dry_run: bool, shortcuts: bool) -> i32 {
    use kora_core::app_install as app;
    let legacy = app::user::legacy_present().present();
    let mine = shortcuts && app::user::user_status().any();
    if !legacy && !mine {
        println!(
            "{} Your servers and databases were not touched.",
            if shortcuts { "There are no Kora shortcuts and no old machine-wide Kora app: nothing to remove." } else { "The old machine-wide Kora app is not installed: nothing to remove." }
        );
        return 0;
    }
    let admin = installer::is_admin() || std::env::var_os("KORA_APP_PREFIX").is_some();
    // The old app can only go with administrator rights. With `--app` that is the whole job; otherwise the
    // shortcuts are still removed and the old app is explained.
    if legacy && !admin && !dry_run && !shortcuts {
        eprintln!("error: removing the old Kora app needs administrator rights: run this as Administrator (Windows) or with sudo (Linux).");
        return 1;
    }
    let remove_legacy = legacy && (admin || dry_run);
    if !yes && !dry_run {
        if !std::io::stdin().is_terminal() {
            eprintln!("error: pass --yes to remove Kora's shortcuts or the old Kora app without a terminal to ask in (or --dry-run to preview).");
            return 2;
        }
        let what = match (mine, remove_legacy) {
            (true, true) => "Remove Kora's shortcuts and the old Kora app (its PATH entry and installed program)?",
            (true, false) => "Remove Kora's shortcuts?",
            _ => "Remove the old Kora app (its shortcuts, PATH entry and installed program)?",
        };
        if !installer::uninstall::Asker::ask(&mut Terminal, &format!("{what} Your servers and databases are not touched."), true) {
            println!("Cancelled. Nothing was changed.");
            return 0;
        }
    }
    let mut code = 0;
    if mine {
        match app::user::uninstall_user(dry_run) {
            Ok(lines) => {
                println!("{}", if dry_run { "Dry run: nothing was changed. Removing Kora's shortcuts would:" } else { "Kora's shortcuts were removed:" });
                print_lines("  - ", &lines);
            }
            Err(e) => {
                eprintln!("error: {e}");
                code = 1;
            }
        }
    }
    if remove_legacy {
        match app::uninstall_app_opts(dry_run) {
            Ok(lines) => {
                println!("{}", if dry_run { "Dry run: nothing was changed. Removing the old Kora app would:" } else { "The old Kora app was removed:" });
                print_lines("  - ", &lines);
            }
            Err(e) => {
                eprintln!("error: {e}");
                code = 1;
            }
        }
    } else if legacy {
        println!("The old machine-wide Kora app is still installed. Remove it with `kora uninstall --app`, run as Administrator (Windows) or with sudo (Linux).");
    }
    println!("Your servers and databases were not touched.");
    code
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

/// Shown (exit code 2) when `--code` is used: setup codes are gone.
const CODE_REMOVED: &str = "error: setup codes were removed in 0.4.0. Use the flags instead (see `kora install --help`), `kora add-resources <server> --resource <ID>`, or the panel (`kora panel`).";

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

fn install(args: InstallArgs) -> i32 {
    let mut args = args;
    if args.github_owner.as_deref().is_some_and(|o| o.eq_ignore_ascii_case("list")) {
        return github_owners();
    }
    if args.code.is_some() {
        eprintln!("{CODE_REMOVED}");
        return 2;
    }
    // No options: this is the panel's install page. Options: the scriptable installer below.
    if !args.any_given() {
        if args.catalog.is_some() {
            eprintln!("note: --catalog only applies to an install with options; the panel uses the catalogue it knows (or KORA_CATALOG).");
        }
        return bare::open_panel(bare::Landing::Install, false);
    }
    if let Some(name) = args.cfx_user.take() {
        match kora_core::cfx::lookup(&name) {
            Ok(a) => {
                println!("Cfx.re account {} is {}: it will be the txAdmin admin.", a.username, a.identifier());
                args.cfx_identifier = Some(a.identifier());
            }
            Err(e) => {
                eprintln!("error: {e}");
                return 2;
            }
        }
    }
    let args_auto_ports = args.auto_ports;
    let fix_permissions = args.fix_permissions;
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
    let hooks = installer::Hooks { fix_permissions, ..installer::Hooks::from_env() };
    match installer::run_with(&req, &mut sink, &hooks) {
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
    if action != A::Start {
        // So the panel's monitor does not report this stop as a crash.
        if let Some(e) = registry::Registry::load().ok().and_then(|r| r.servers.into_iter().find(|e| e.name == server)) {
            kora_core::monitor::mark_stop_intent(&e.dir);
        }
    }
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
            // The scheduled run simply skips this time (already written to update.log).
            Err(e) if kora_core::update::lock::is_busy_message(&e.to_string()) => {
                println!("Skipped: {e}");
                return 0;
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
        let warn = if a.force {
            " Changes of your own in those resources are thrown away."
        } else {
            ""
        };
        if !a.yes && !confirm(&format!("Put '{}' back the way snapshot {id} saved it? The server restarts if it is running.{warn}", server.name)) {
            println!("Nothing changed.");
            return 0;
        }
        return match m::rollback(&server, id, a.force) {
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
    // Exactly what was just shown, even if something new was published while you read it.
    let opts = m::ApplyOpts {
        targets: Some(m::planned_targets(&pre)),
        ..m::ApplyOpts::new(restart)
    };
    let res = m::apply(&server, scope, opts, &mut |e| match e {
        m::UpdateEvent::Snapshot(id) => {
            println!("Saved snapshot {id}.");
            snapshot = Some(id);
        }
        m::UpdateEvent::Resource { id, state } if state == "updating" => {
            println!("Updating {id}...")
        }
        m::UpdateEvent::Health(h) => println!("{h}."),
        m::UpdateEvent::Progress(p) => println!("{p}..."),
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

/// The folder of `server` (a registered name or a folder).
fn server_dir(server: &str) -> Result<std::path::PathBuf, String> {
    let reg = registry::Registry::load().unwrap_or_default();
    if let Some(e) = reg.find(server) {
        return Ok(e.dir.clone());
    }
    let p = std::path::Path::new(server);
    if p.join("server.cfg").is_file() || p.join("kora-server.json").is_file() {
        return Ok(p.to_path_buf());
    }
    Err(format!("there is no server called '{server}' (see `kora status`), and it is not a server folder"))
}

fn print_asset_plan(p: &kora_core::paidassets::ImportPlan) {
    use kora_core::paidassets::{human, CATEGORY};
    println!("{}:", p.zip_name);
    for r in &p.resources {
        if !r.has_fxmanifest {
            continue;
        }
        let mut tags = vec![human(r.size)];
        if let Some(v) = &r.version {
            tags.push(format!("version {v}"));
        }
        tags.push(if r.escrowed { "escrowed".into() } else { "not escrowed".into() });
        if r.escrowed {
            tags.push(if r.has_fxap { ".fxap present".into() } else { ".fxap MISSING".into() });
        }
        let what = if r.replaces { "replaces the imported copy (kept as a backup)" } else { "new" };
        println!("  {}  ->  resources/{CATEGORY}/{}  ({}; {what})", r.name, r.name, tags.join(", "));
    }
    for pr in &p.problems {
        println!("  {} {}", if pr.blocking { "[!!]" } else { "note:" }, pr.message);
    }
}

/// `kora add-asset <server> <zip>...`: import paid assets downloaded from the Cfx.re Portal.
fn add_asset(a: &AddAssetArgs) -> i32 {
    use kora_core::paidassets as pa;
    let dir = match server_dir(&a.server) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let plans: Vec<(std::path::PathBuf, pa::ImportPlan)> = a.zip.iter().map(|z| (z.clone(), pa::inspect(&dir, z))).collect();
    for (_, p) in &plans {
        print_asset_plan(p);
    }
    let ok: Vec<&(std::path::PathBuf, pa::ImportPlan)> = plans.iter().filter(|(_, p)| p.can_import).collect();
    if ok.len() < plans.len() {
        eprintln!("{} of {} zip(s) can't be imported; see [!!] above.", plans.len() - ok.len(), plans.len());
    }
    if ok.is_empty() {
        println!("Get your assets from {}", pa::PORTAL_URL);
        return 1;
    }
    println!("The server's licence key must belong to the Cfx.re account that bought these assets.");
    if a.dry_run {
        println!("Dry run: nothing was changed.");
        return 0;
    }
    if !a.yes && !confirm("Import these now?") {
        eprintln!("Cancelled.");
        return 1;
    }
    let mut failed = plans.len() - ok.len();
    for (zip, p) in ok {
        match pa::import(&dir, zip, &pa::ImportOpts::default(), &mut |m| println!("{m}...")) {
            Ok(r) => {
                for i in &r.imported {
                    let v = i.version.as_deref().map(|v| format!(" {v}")).unwrap_or_default();
                    println!("[ok] {}{v}{}", i.name, if i.replaced { " (replaced; the old copy is kept as a backup)" } else { "" });
                }
                for n in &r.ensured {
                    println!("Added `ensure {n}` to server.cfg.");
                }
                for n in &r.notes {
                    println!("note: {n}");
                }
            }
            Err(e) => {
                eprintln!("[!!] {}: {e}", p.zip_name);
                failed += 1;
            }
        }
    }
    if failed > 0 {
        1
    } else {
        0
    }
}

/// `kora add-resources <server> --resource ID...`. Browsing the catalog sections
/// (with per-creator drill-in) is in the panel: a server's Resources tab.
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
        eprintln!("error: say what to add: --resource <ID> (repeatable). To browse the catalog, run `kora` and use the server's Resources tab.");
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
    // Kora's own shortcuts (per user). Under `sudo` they belong to the person who typed it, not to root.
    let under_sudo = cfg!(unix) && kora_core::userhome::sudo_account().is_some();
    if under_sudo {
        if kora_core::app_install::user::user_status().any() || kora_core::app_install::user::legacy_present().present() {
            println!("\nKora's shortcuts belong to your own account: run `kora app uninstall` without sudo to remove them.");
        }
    } else {
        match kora_core::app_install::user::uninstall_user(dry_run) {
            Ok(lines) if !lines.is_empty() => {
                println!("\n{}", if dry_run { "Kora's shortcuts would be removed:" } else { "Kora's shortcuts were removed:" });
                print_lines("  - ", &lines);
            }
            Ok(_) => {}
            Err(e) => summary.failed.push(format!("Kora's shortcuts: {e}")),
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

/// Printed on Linux when `kora panel` is run as root: many rented servers only have the root account.
/// `adduser` and the `sudo` group are right for Debian and Ubuntu, the systems Kora supports. Enabling
/// linger starts the new user's service manager, so the background panel can be set up after `su`.
const ROOT_ONLY_HELP: &[&str] = &[
    "",
    "If this server only has the root account, create a user for Kora first (it asks you to choose a password):",
    "  adduser kora",
    "  usermod -aG sudo kora",
    "  loginctl enable-linger kora",
    "  su - kora",
    "Then, as that user, run:",
    "  kora panel --setup",
];

fn panel(a: &PanelArgs) -> i32 {
    panel_with(a, None)
}

/// [`panel`], opening the browser on `landing` (`/install`) when it runs in this window.
fn panel_with(a: &PanelArgs, landing: Option<&str>) -> i32 {
    use kora_core::monitor::panel_service as svc;
    // SECURITY.md: the panel runs as the installing user, never as administrator/root. A root panel
    // would keep its token and state in root's home and run every action with full rights.
    if kora_core::elevate::is_elevated() && std::env::var_os("KORA_ALLOW_ROOT_PANEL").is_none_or(|v| v != "1") {
        eprintln!("error: Kora Panel does not run as administrator/root: it keeps your servers and your sign-in safe. Run `kora` from your normal account, without sudo. It asks for administrator rights only when an action needs them.");
        if cfg!(target_os = "linux") {
            for line in ROOT_ONLY_HELP {
                eprintln!("{line}");
            }
        }
        return 1;
    }
    let home = svc::home();
    match &a.cmd {
        Some(PanelCmd::User { cmd }) => return panel_user(cmd),
        Some(PanelCmd::Tailscale { cmd }) => return panel_tailscale::command(cmd, &home),
        Some(PanelCmd::Restart) => return panel_restart(&home),
        Some(PanelCmd::RemoteActions { cmd }) => return panel_remote_actions(*cmd, &kora_web::remote::config::default_path()),
        None => {}
    }
    if a.new_token {
        return panel_tailscale::new_token(&home);
    }
    if a.tailscale {
        // Set up who may sign in, then keep it running in the background (or here with --foreground).
        let ts = match panel_tailscale::configure(a.admin.as_deref()) {
            Ok(ts) => ts,
            Err(e) => {
                eprintln!("error: {e}");
                return 1;
            }
        };
        if !a.foreground {
            return panel_tailscale::start_background(a, &home, &ts);
        }
    }
    if a.print_link {
        return match svc::link(&home) {
            Ok(link) => {
                let port = svc::saved_port(&home).unwrap_or(kora_web::DEFAULT_PORT);
                // A panel that is still starting gets a moment, so the link can carry a one-time code.
                let up = kora_web::launch::wait_for_panel(port, std::time::Duration::from_secs(5), std::time::Duration::from_millis(250), &kora_web::launch::SystemPorts);
                println!("{}", one_time_link(&link, port));
                if !up {
                    eprintln!("Note: nothing is answering on port {port} right now. The background panel may still be starting; check it with `systemctl --user status kora-panel` (Linux).");
                } else {
                    eprintln!("The link works once, within 10 minutes. Run this again for a new one.");
                }
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        };
    }
    if a.setup {
        return panel_setup(a, &home);
    }
    let ports = kora_web::launch::SystemPorts;
    let platform = kora_core::service::Platform::current();
    if !a.install_service && !a.remove_service && !a.foreground && may_reuse_running(a) {
        let running = running_panel(&home, &ports, platform);
        let tty = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
        match panel_start_mode(headless_session(), tty, running.is_some()) {
            PanelStart::ShowRunning => {
                if let Some(r) = running {
                    print_running(&r);
                    return 0;
                }
            }
            PanelStart::OfferBackground => {
                if ask_yes_no("This computer has no screen, so Kora Panel would stop when you close this window.\nKeep it running in the background instead? [Y/n] ") {
                    return install_background_panel(a, &home);
                }
            }
            PanelStart::Foreground => {}
        }
    }
    if a.install_service || a.remove_service {
        let runner = kora_core::service::SystemRunner;
        if a.remove_service {
            return match svc::remove(&home, &runner, platform) {
                Ok(()) => {
                    println!("The background panel has been stopped and removed. Monitoring and alerts now only run while `kora panel` is open.");
                    0
                }
                Err(e) => {
                    eprintln!("error: could not remove the background panel: {e}");
                    1
                }
            };
        }
        return install_background_panel(a, &home);
    }
    // Only on this computer can we tell which ports are free; another address keeps the asked-for port.
    let port = match (a.port, a.bind.is_loopback()) {
        (Some(p), _) => p,
        (None, false) => kora_web::DEFAULT_PORT,
        (None, true) => match kora_web::launch::pick_port(kora_web::DEFAULT_PORT, false, &ports) {
            Some(p) => {
                if p != kora_web::DEFAULT_PORT {
                    println!("Port {} is used by another program, so Kora Panel uses port {p}.", kora_web::DEFAULT_PORT);
                }
                p
            }
            None => {
                eprintln!("error: ports {} to {} are all in use. Choose one with --port.", kora_web::DEFAULT_PORT, kora_web::launch::FALLBACK_PORTS.end());
                return 1;
            }
        },
    };
    // A saved token (made by --install-service) keeps the sign-in link the same every time.
    let token = svc::load_token(&home).unwrap_or_else(kora_web::security::random_hex);
    if !a.bind.is_loopback() {
        println!("This panel runs in this window only and stops when you close it.{}", if a.tailscale { "" } else { " With Tailscale, `kora panel --tailscale` keeps one running in the background instead." });
    }
    let opts = kora_web::PanelOptions {
        bind: a.bind,
        port,
        open_browser: !a.no_open,
        remote: a.remote,
        token: Some(token.clone()),
        tailscale: a.tailscale || a.listen_tailscale,
        allow_public: a.allow_public,
        landing: landing.map(str::to_string),
        ..Default::default()
    };
    let pid = std::process::id();
    let pid_file = svc::pid_path(&home);
    let _ = std::fs::create_dir_all(&home);
    let _ = std::fs::write(&pid_file, pid.to_string());
    // So a second `kora panel` shows this one's link instead of failing to listen on the same port.
    if a.bind.is_loopback() {
        let _ = svc::record_running(&home, &svc::RunningPanel { port, pid, token });
    }
    let result = kora_web::run(opts);
    let _ = std::fs::remove_file(&pid_file);
    svc::clear_running(&home, pid);
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// May `kora panel` answer with the link of a panel that already runs? Not when it was asked for a
/// different address (`--bind`, `--remote`, Tailscale): that is a different panel, and answering with the
/// 127.0.0.1 link silently ignored those options.
fn may_reuse_running(a: &PanelArgs) -> bool {
    a.bind.is_loopback() && !a.remote && !a.tailscale && !a.listen_tailscale
}

/// What `kora panel` does when started without options.
#[derive(Debug, PartialEq, Eq)]
enum PanelStart {
    /// A panel is already up: show its link instead of starting a second one.
    ShowRunning,
    /// A server with no screen and nobody to keep the window open: offer the background panel.
    OfferBackground,
    /// Run in this window.
    Foreground,
}

fn panel_start_mode(headless: bool, interactive: bool, already_running: bool) -> PanelStart {
    if already_running {
        PanelStart::ShowRunning
    } else if headless && interactive {
        PanelStart::OfferBackground
    } else {
        PanelStart::Foreground
    }
}

/// Running as root (Linux)?
fn launch_uid_is_root() -> bool {
    kora_web::launch::current_uid() == Some(0)
}

/// No desktop to open a browser on: an SSH session, or no display on Linux.
fn headless_session() -> bool {
    kora_web::launch::is_headless()
}

/// A Kora Panel of ours that is already answering.
#[derive(Debug, PartialEq, Eq)]
enum Running {
    /// The background service (`--install-service` / `--setup`).
    Background { port: u16, link: String },
    /// A `kora panel` running in another window.
    Window { port: u16, link: String },
}

impl Running {
    fn port_and_link(&self) -> (u16, &str) {
        match self {
            Running::Background { port, link } | Running::Window { port, link } => (*port, link),
        }
    }
}

/// Find a running panel: the background service on its saved port, else the one recorded by a
/// `kora panel` window (whose process must still exist).
fn running_panel(home: &std::path::Path, ports: &dyn kora_web::launch::PortCheck, platform: kora_core::service::Platform) -> Option<Running> {
    use kora_core::monitor::panel_service as svc;
    find_running(
        svc::service_installed(platform).then(|| svc::link(home).ok().zip(svc::saved_port(home))).flatten(),
        svc::load_running(home).filter(|r| process_alive(r.pid)),
        ports,
    )
}

/// The decision behind [`running_panel`], without the disk: `service` is the installed service's
/// (link, port), `window` the recorded `kora panel` window.
fn find_running(
    service: Option<(String, u16)>,
    window: Option<kora_core::monitor::panel_service::RunningPanel>,
    ports: &dyn kora_web::launch::PortCheck,
) -> Option<Running> {
    if let Some((link, port)) = service {
        if ports.is_kora_panel(port) {
            return Some(Running::Background { port, link });
        }
    }
    let w = window?;
    ports.is_kora_panel(w.port).then(|| Running::Window { port: w.port, link: w.link() })
}

fn process_alive(pid: u32) -> bool {
    if cfg!(target_os = "linux") {
        std::path::Path::new("/proc").join(pid.to_string()).exists()
    } else {
        true
    }
}

/// Swap the token in a `#token=` link for a one-time code from the panel on `port`, so the long-lived
/// token does not end up in a browser's history. Unchanged when the panel cannot give one (an older
/// version, or not answering).
fn one_time_link(link: &str, port: u16) -> String {
    match link.split_once("#token=") {
        Some((_, token)) => kora_web::launch::fresh_link(port, token),
        None => link.to_string(),
    }
}

fn print_running(r: &Running) {
    let (port, link) = r.port_and_link();
    let link = &one_time_link(link, port);
    match r {
        Running::Background { .. } => println!("Kora Panel is already running in the background."),
        Running::Window { .. } => println!("Kora Panel is already running in another window."),
    }
    println!("Sign in with: {link}");
    if headless_session() {
        println!("On a server with no screen, open a tunnel from your own computer first:  {}", kora_web::launch::ssh_hint(port, &kora_web::launch::real_env));
    }
    if matches!(r, Running::Background { .. }) {
        panel_tailscale::print_tailnet_address(&kora_core::monitor::panel_service::home(), port);
    }
    match r {
        Running::Background { .. } => println!("(`kora panel --remove-service` stops and removes it; `kora panel --foreground` runs a separate one in this window.)"),
        Running::Window { .. } => println!("(It stops when that window closes. `kora panel --install-service` keeps one running in the background instead.)"),
    }
}

/// Ask a yes/no question on the terminal; an empty answer means yes.
fn ask_yes_no(question: &str) -> bool {
    use std::io::Write;
    print!("{question}");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return false;
    }
    !matches!(line.trim().to_lowercase().as_str(), "n" | "no")
}

/// The port for a new background panel: the asked-for one, else the saved one, else the default; when
/// not asked for explicitly and it is taken, the next free one.
fn background_port(a: &PanelArgs, home: &std::path::Path, ports: &dyn kora_web::launch::PortCheck) -> Result<u16, String> {
    background_port_for(a.port, home, ports)
}

/// [`background_port`] with the asked-for port given directly.
fn background_port_for(requested: Option<u16>, home: &std::path::Path, ports: &dyn kora_web::launch::PortCheck) -> Result<u16, String> {
    use kora_core::monitor::panel_service as svc;
    let preferred = requested.or_else(|| svc::saved_port(home)).unwrap_or(kora_web::DEFAULT_PORT);
    // Our own background panel on its saved port is about to be restarted, not "another program":
    // keep the port, so links, bookmarks, tunnels and firewall rules stay right.
    if svc::saved_port(home) == Some(preferred) && ports.is_kora_panel(preferred) {
        return Ok(preferred);
    }
    match kora_web::launch::pick_port(preferred, requested.is_some(), ports) {
        Some(p) => {
            if p != preferred {
                println!("Port {preferred} is used by another program, so Kora Panel uses port {p}.");
            }
            Ok(p)
        }
        None if requested.is_some() => Err(format!("port {preferred} is already in use. Choose another with --port.")),
        None => Err(format!("ports {} to {} are all in use. Choose one with --port.", kora_web::DEFAULT_PORT, kora_web::launch::FALLBACK_PORTS.end())),
    }
}

/// Install and start the background panel (systemd user service on Linux, a Startup entry on Windows).
fn install_background_panel(a: &PanelArgs, home: &std::path::Path) -> i32 {
    let port = match background_port(a, home, &kora_web::launch::SystemPorts) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    match start_background_service(home, port, svc_tailscale(home)) {
        Ok(link) => {
            let ports = kora_web::launch::SystemPorts;
            let link = if kora_web::launch::wait_for_panel(port, SETUP_WAIT, std::time::Duration::from_millis(250), &ports) {
                one_time_link(&link, port)
            } else {
                link
            };
            println!("The Kora Panel now runs in the background (starts at sign-in) and keeps watching your servers.");
            println!("Sign in with: {link}");
            if headless_session() {
                println!("On a server with no screen, open a tunnel from your own computer first:  {}", kora_web::launch::ssh_hint(port, &kora_web::launch::real_env));
            }
            println!("Show the link again any time with `kora panel`; remove it with `kora panel --remove-service`.");
            println!("The link contains your sign-in token: keep it private.");
            0
        }
        Err(e) => {
            eprintln!("error: could not set up the background panel: {e}");
            1
        }
    }
}

/// Does the installed background panel also listen on Tailscale? Kept when it is set up again.
fn svc_tailscale(home: &std::path::Path) -> bool {
    kora_core::monitor::panel_service::saved_tailscale(home)
}

/// Write and start the background service on `port`. Returns the sign-in link.
fn start_background_service(home: &std::path::Path, port: u16, tailscale: bool) -> Result<String, String> {
    use kora_core::monitor::panel_service as svc;
    let runner = kora_core::service::SystemRunner;
    let platform = kora_core::service::Platform::current();
    let exe = std::env::current_exe().map_err(|e| format!("cannot find the kora program to run in the background: {e}"))?;
    let home_env = std::env::var_os("KORA_HOME").map(std::path::PathBuf::from);
    // Windows has no service manager to restart the panel: stop the running one (only when it really is
    // our panel answering on its saved port), else the new one cannot take the port.
    let stop_pid = windows_panel_pid(home, platform);
    let spec = svc::InstallSpec { exe: &exe, port, tailscale, stop_pid, kora_home_env: home_env.as_deref() };
    svc::install(home, &spec, &runner, platform).map_err(|e| e.to_string())
}

/// Windows: the process of the running background panel, only when it really is our panel answering
/// on its saved port. `None` on Linux, where systemd stops it.
fn windows_panel_pid(home: &std::path::Path, platform: kora_core::service::Platform) -> Option<u32> {
    use kora_core::monitor::panel_service as svc;
    (platform == kora_core::service::Platform::Windows)
        .then(|| svc::saved_port(home))
        .flatten()
        .filter(|p| kora_web::launch::PortCheck::is_kora_panel(&kora_web::launch::SystemPorts, *p))
        .and_then(|_| std::fs::read_to_string(svc::pid_path(home)).ok())
        .and_then(|t| t.trim().parse().ok())
}

/// What `kora panel restart` can do.
#[derive(Debug, PartialEq, Eq)]
enum PanelRestart {
    /// The background panel is set up: restart it (this also starts it when it had stopped).
    Background,
    /// Only a `kora panel` in a window runs: it has to be stopped there.
    Window,
    /// Nothing to restart.
    NotRunning,
}

fn panel_restart_mode(background_installed: bool, window_running: bool) -> PanelRestart {
    if background_installed {
        PanelRestart::Background
    } else if window_running {
        PanelRestart::Window
    } else {
        PanelRestart::NotRunning
    }
}

/// `kora panel restart`: restart the background panel and print where to sign in. Token, port and
/// settings stay as they are.
fn panel_restart(home: &std::path::Path) -> i32 {
    use kora_core::monitor::panel_service as svc;
    use kora_core::service::Platform;
    let ports = kora_web::launch::SystemPorts;
    let platform = Platform::current();
    let installed = svc::service_installed(platform);
    let window = !installed && running_panel(home, &ports, platform).is_some();
    match panel_restart_mode(installed, window) {
        PanelRestart::Background => {}
        PanelRestart::Window => {
            eprintln!("error: Kora Panel runs in a window here, not in the background, so it cannot be restarted from another window. Stop it there (Ctrl+C) and run `kora panel` again.");
            eprintln!("(`kora panel --install-service` keeps one running in the background instead.)");
            return 1;
        }
        PanelRestart::NotRunning => {
            eprintln!("error: Kora Panel is not running, so there is nothing to restart. Start it with `kora panel` (or keep it running in the background with `kora panel --install-service`).");
            return 1;
        }
    }
    if platform == Platform::Linux && !user_services_available() {
        eprintln!("error: this session cannot reach the background panel (common after sudo or su). Sign in as the user who runs it (not with sudo) and run `kora panel restart` again.");
        return 1;
    }
    let port = svc::saved_port(home).unwrap_or(kora_web::DEFAULT_PORT);
    if let Err(e) = svc::restart(&kora_core::service::SystemRunner, platform, windows_panel_pid(home, platform)) {
        eprintln!("error: could not restart the background panel: {e}");
        return 1;
    }
    if !kora_web::launch::wait_for_panel(port, SETUP_WAIT, std::time::Duration::from_millis(250), &ports) {
        eprintln!("Kora Panel was restarted, but did not answer within {} seconds.", SETUP_WAIT.as_secs());
        if platform == Platform::Linux {
            eprintln!("See what happened with:  systemctl --user status kora-panel");
        }
        return 1;
    }
    println!("Kora Panel restarted. Pages that were open sign in again.");
    if let Ok(link) = svc::link(home) {
        println!("Sign in with: {}", one_time_link(&link, port));
    }
    if headless_session() {
        println!("On a server with no screen, open a tunnel from your own computer first:  {}", kora_web::launch::ssh_hint(port, &kora_web::launch::real_env));
    }
    panel_tailscale::print_tailnet_address(home, port);
    0
}

/// Can this session use systemd user services? Under `sudo` or `su` there is often no user service
/// manager to talk to. Never waits more than a few seconds.
fn user_services_available() -> bool {
    use kora_web::launch::{run_and_wait, OpenerCommand, OpenerExit};
    // `sudo` drops XDG_RUNTIME_DIR; systemctl needs it to find the user service manager.
    if std::env::var_os("XDG_RUNTIME_DIR").is_none() {
        if let Some(uid) = kora_web::launch::current_uid() {
            let dir = format!("/run/user/{uid}");
            if std::path::Path::new(&dir).is_dir() {
                std::env::set_var("XDG_RUNTIME_DIR", dir);
            }
        }
    }
    let check = OpenerCommand { program: "systemctl".into(), args: vec!["--user".into(), "show-environment".into()] };
    matches!(run_and_wait(&check, std::time::Duration::from_secs(5)), Ok(OpenerExit::Exited(Some(0))))
}

/// How long `--setup` waits for the background panel to answer.
const SETUP_WAIT: std::time::Duration = std::time::Duration::from_secs(15);

/// `kora panel --setup`: never asks anything (stdin may be a pipe). Reuses a running panel, or sets up
/// and starts the background one; then opens the browser where there is a screen and prints the link.
fn panel_setup(a: &PanelArgs, home: &std::path::Path) -> i32 {
    use kora_web::launch;
    let ports = launch::SystemPorts;
    let platform = kora_core::service::Platform::current();
    let headless = headless_session();
    let (port, link) = match running_panel(home, &ports, platform) {
        Some(r) => {
            match r {
                Running::Background { .. } => println!("Kora Panel is already running in the background."),
                Running::Window { .. } => println!("Kora Panel is already running in another window (it stops when that window closes; `kora panel --install-service` keeps one running in the background)."),
            }
            let (p, l) = r.port_and_link();
            (p, l.to_string())
        }
        None => {
            if platform == kora_core::service::Platform::Linux && !user_services_available() {
                print_setup_fallback("this session cannot run background services (common after sudo or su, or in a container)");
                return 1;
            }
            let port = match background_port(a, home, &ports) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("error: {e}");
                    return 1;
                }
            };
            println!("Starting Kora Panel in the background (it also starts by itself after a restart) ...");
            let link = match start_background_service(home, port, svc_tailscale(home)) {
                Ok(l) => l,
                Err(e) => {
                    print_setup_fallback(&format!("the background panel could not be set up: {e}"));
                    return 1;
                }
            };
            if !launch::wait_for_panel(port, SETUP_WAIT, std::time::Duration::from_millis(250), &ports) {
                eprintln!("Kora Panel did not answer within {} seconds.", SETUP_WAIT.as_secs());
                if platform == kora_core::service::Platform::Linux {
                    eprintln!("See what happened with:  systemctl --user status kora-panel");
                }
                eprintln!("When it is up, sign in with: {link}");
                return 1;
            }
            (port, link)
        }
    };
    let link = one_time_link(&link, port);
    let opened = (!headless && !a.no_open).then(|| launch::open_browser(&link));
    for line in launch::ready_message(&link, port, headless, opened.as_ref(), &launch::real_env) {
        println!("{line}");
    }
    panel_tailscale::after_setup(a, home, headless);
    0
}

fn print_setup_fallback(why: &str) {
    eprintln!("Kora Panel could not start in the background: {why}.");
    eprintln!("Start it yourself instead; it runs until you close the window:");
    eprintln!("  kora panel --foreground");
    eprintln!("To keep it running in the background, sign in as the user who should run it (not with sudo) and run:");
    eprintln!("  kora panel --setup");
}

/// What `kora panel remote-actions on|off|status` prints.
fn remote_actions_message(cmd: RemoteActionsCmd, on: bool) -> &'static str {
    match (cmd, on) {
        (RemoteActionsCmd::Status, true) => "Remote actions are on: administrators who sign in with a token from another computer can delete and unregister servers, uninstall Kora, send console commands, change schedules and update Kora. Turn them off with `kora panel remote-actions off`.",
        (RemoteActionsCmd::Status, false) => "Remote actions are off: administrators who sign in with a token from another computer can install and manage servers, but cannot delete or unregister them, uninstall Kora, send console commands, change schedules or update Kora. Tailscale administrators are not held back by this setting. Turn it on with `kora panel remote-actions on`.",
        (_, true) => "Administrators who sign in with a token from another computer can now delete and unregister servers, uninstall Kora, send console commands, change schedules and update Kora. Deleting still asks for the server's name. Only give the admin role to people you trust.",
        (_, false) => "Remote actions are turned off. Token sign-ins from other computers cannot delete or unregister servers, uninstall Kora, send console commands, change schedules or update Kora (they can still install and manage servers). These all still work on this computer, and for Tailscale administrators.",
    }
}

/// `kora panel remote-actions on|off|status`: the "destructive actions" switch of Settings > Remote
/// access, for a server with no local browser. Saved through the same `Remote::update` the panel uses;
/// a running panel notices the changed file by itself, so no restart is needed.
fn panel_remote_actions(cmd: RemoteActionsCmd, path: &std::path::Path) -> i32 {
    let run = || -> Result<bool, String> {
        let remote = kora_web::remote::Remote::from_file(path.to_path_buf())?;
        match cmd {
            RemoteActionsCmd::Status => Ok(remote.config().allow_remote_destructive),
            RemoteActionsCmd::On | RemoteActionsCmd::Off => remote.update(|cfg| {
                cfg.allow_remote_destructive = cmd == RemoteActionsCmd::On;
                Ok(cfg.allow_remote_destructive)
            }),
        }
    };
    match run() {
        Ok(on) => {
            println!("{}", remote_actions_message(cmd, on));
            if cmd != RemoteActionsCmd::Status {
                println!("A running panel uses the new setting right away. No restart is needed.");
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// `kora panel user ...`: edits `<kora home>/panel.json`. A running panel notices the change by itself.
fn panel_user(cmd: &PanelUserCmd) -> i32 {
    use kora_web::remote::{config, RemoteConfig, Role};
    let path = config::default_path();
    let mut cfg = match RemoteConfig::load(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let result: Result<(), String> = match cmd {
        PanelUserCmd::Add { name, role } => role.parse::<Role>().and_then(|role| {
            let token = cfg.add_user(name, role)?;
            cfg.save(&path)?;
            println!("Added {} ({role}).", name.trim());
            println!("Token (shown once, keep it safe): {token}");
            if !cfg.enabled {
                println!("Remote access is still off. Start the panel with `kora panel --bind <address> --remote`, or turn it on in the panel's Settings.");
            }
            Ok(())
        }),
        PanelUserCmd::List => {
            if cfg.users.is_empty() {
                println!("No users yet. Add one with: kora panel user add <name> --role admin");
            }
            for u in &cfg.users {
                println!("{:<24} {}", u.name, u.role);
            }
            Ok(())
        }
        PanelUserCmd::Remove { name } => {
            if cfg.remove_user(name) {
                cfg.save(&path).map(|()| println!("Removed {name}."))
            } else {
                Err(format!("There is no user named '{name}'."))
            }
        }
    };
    match result {
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
        // Bare `kora`, `kora manager` and a flagless `kora install` only open the panel: never elevated.
        None | Some(Command::Manager) => K::Other,
        Some(Command::Install(a)) if !a.any_given() => K::Other,
        Some(Command::Install(a)) => K::Install { dry_run: a.dry_run },
        Some(Command::Update(a)) => K::Update { auto: a.auto },
        Some(Command::Status | Command::Schedule { cmd: ScheduleCmd::Show { .. } }) => K::Status,
        Some(Command::Schedule { .. }) => K::Restart,
        Some(Command::Delete { .. }) => K::Delete,
        // The panel runs as the user; privileged actions report the command to run instead.
        Some(Command::ServiceHost { .. } | Command::Github { .. } | Command::Webhook { .. } | Command::SelfUpdate { .. } | Command::Panel(_)) => K::Other,
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
    if !NO_PAUSE_ON_SUCCESS.load(std::sync::atomic::Ordering::Relaxed)
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
    // Bare `kora` opens the panel (see `bare`); it never asks for administrator rights.
    let tty = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    elevate_if_needed(&cli, &cli.command);
    let command = cli.command;
    // "New version available": background check, interactive runs only (never CI, pipes, services).
    let quiet = matches!(command, Some(Command::ServiceHost { .. } | Command::SelfUpdate { .. } | Command::Helper { .. } | Command::Update(UpdateArgs { auto: true, .. })));
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
        Some(Command::Helper { cmd }) => helper(cmd),
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
    finish(code);
}

mod bare;
mod panel_tailscale;

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
    fn add_asset_arguments_and_help() {
        let parse = |args: &[&str]| {
            let mut v = vec!["kora", "add-asset"];
            v.extend_from_slice(args);
            Cli::try_parse_from(v).map(|c| match c.command {
                Some(Command::AddAsset(a)) => a,
                _ => unreachable!(),
            })
        };
        let a = parse(&["srv", "a.zip", "b.zip", "--dry-run"]).unwrap();
        assert_eq!((a.server.as_str(), a.zip.len(), a.dry_run, a.yes), ("srv", 2, true, false));
        assert!(parse(&["srv", "a.zip", "-y"]).unwrap().yes);
        assert!(parse(&["srv"]).is_err(), "at least one zip");
        let help = <Cli as clap::CommandFactory>::command().find_subcommand_mut("add-asset").unwrap().render_long_help().to_string();
        assert!(help.contains("https://portal.cfx.re/assets/granted-assets"), "{help}");
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
    fn install_without_options_is_the_panel_and_the_old_app_flags_are_ignored() {
        assert!(!InstallArgs::default().any_given());
        assert!(!InstallArgs { no_app: true, no_desktop_shortcut: true, ..Default::default() }.any_given());
        let r = InstallArgs { name: Some("a".into()), ..Default::default() }.into_request().unwrap();
        assert_eq!(r.app, None, "the machine-wide app is retired");
        // still accepted, so an old script keeps working
        assert!(Cli::try_parse_from(["kora", "install", "--name", "a", "--no-app", "--no-desktop-shortcut", "-y"]).is_ok());
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
    fn the_headless_default_folder_is_the_one_the_panel_suggests() {
        // One rule for both: `kora_core::userhome` (Windows: C:\Kora\<name>).
        assert_eq!(default_dir("Alpha Server"), kora_core::userhome::default_server_dir("alpha-server"));
        if cfg!(windows) {
            assert_eq!(default_dir("Alpha Server").unwrap(), PathBuf::from("C:\\Kora\\alpha-server"));
        }
        let r = InstallArgs { name: Some("Alpha".into()), ..Default::default() }.into_request().unwrap();
        assert_eq!(Ok(r.dir), kora_core::userhome::default_server_dir("alpha"));
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
    fn hostname_and_game_build_flags() {
        assert!(InstallArgs { hostname: Some("x".into()), ..Default::default() }.any_given());
        assert!(InstallArgs { cfx_user: Some("JSixty".into()), ..Default::default() }.any_given());
        assert!(Cli::try_parse_from(["kora", "install", "--cfx-user", "JSixty", "--cfx-identifier", "fivem:1"]).is_err(), "the two ways to name the admin cannot be mixed");
        assert!(Cli::try_parse_from(["kora", "install", "--cfx-user", "JSixty"]).is_ok());
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
    fn the_panel_offers_the_background_service_only_where_nobody_keeps_a_window_open() {
        // (headless, interactive, background panel already running)
        assert_eq!(panel_start_mode(true, true, true), PanelStart::ShowRunning, "never start a second one");
        assert_eq!(panel_start_mode(false, true, true), PanelStart::ShowRunning);
        assert_eq!(panel_start_mode(true, true, false), PanelStart::OfferBackground, "a server with no screen");
        assert_eq!(panel_start_mode(true, false, false), PanelStart::Foreground, "scripts and pipes are never asked");
        assert_eq!(panel_start_mode(false, true, false), PanelStart::Foreground, "a desktop opens the browser and runs here");
        assert!(Cli::try_parse_from(["kora", "panel", "--foreground"]).is_ok());
        assert!(Cli::try_parse_from(["kora", "panel", "--foreground", "--install-service"]).is_err());
    }

    #[test]
    fn setup_is_a_plain_flag_and_the_port_is_only_set_when_asked() {
        let panel = |a: &[&str]| {
            let mut v = vec!["kora", "panel"];
            v.extend_from_slice(a);
            match Cli::try_parse_from(v).map(|c| c.command) {
                Ok(Some(Command::Panel(p))) => Ok(p),
                Ok(_) => unreachable!(),
                Err(e) => Err(e),
            }
        };
        let p = panel(&["--setup"]).unwrap();
        assert!(p.setup && p.port.is_none(), "no --port means: pick a free one");
        assert_eq!(panel(&["--setup", "--port", "47200"]).unwrap().port, Some(47200));
        assert!(panel(&["--setup", "--no-open"]).is_ok());
        // A panel for another address is never answered with the link of the one on 127.0.0.1.
        assert!(may_reuse_running(&panel(&[]).unwrap()));
        assert!(!may_reuse_running(&panel(&["--bind", "100.64.0.5", "--remote"]).unwrap()));
        assert!(!may_reuse_running(&panel(&["--remote"]).unwrap()));
        assert!(!may_reuse_running(&panel(&["--tailscale"]).unwrap()));
        assert!(!may_reuse_running(&panel(&["--listen-tailscale"]).unwrap()));
        assert!(panel(&["--admin", "a@b"]).is_err(), "--admin goes with --tailscale");
        assert!(panel(&["--tailscale", "--admin", "a@b"]).is_ok());
        assert!(panel(&["--tailscale", "--foreground"]).is_ok());
        assert!(panel(&["--tailscale", "--listen-tailscale"]).is_err());
        assert!(panel(&["--tailscale", "--setup"]).is_err());
        for clash in ["--foreground", "--install-service", "--remove-service", "--print-link"] {
            assert!(panel(&["--setup", clash]).is_err(), "{clash}");
        }
    }

    #[test]
    fn panel_remote_actions_switches_the_saved_setting() {
        use kora_web::remote::RemoteConfig;
        let cmd = |a: &[&str]| match Cli::try_parse_from(a).map(|c| c.command) {
            Ok(Some(Command::Panel(p))) => Ok(p.cmd),
            Ok(_) => panic!("not the panel command"),
            Err(e) => Err(e.to_string()),
        };
        assert!(matches!(cmd(&["kora", "panel", "remote-actions", "on"]), Ok(Some(PanelCmd::RemoteActions { cmd: RemoteActionsCmd::On }))));
        assert!(matches!(cmd(&["kora", "panel", "remote-actions", "off"]), Ok(Some(PanelCmd::RemoteActions { cmd: RemoteActionsCmd::Off }))));
        assert!(matches!(cmd(&["kora", "panel", "remote-actions", "status"]), Ok(Some(PanelCmd::RemoteActions { cmd: RemoteActionsCmd::Status }))));
        assert!(cmd(&["kora", "panel", "remote-actions"]).is_err(), "it needs on, off or status");
        assert!(cmd(&["kora", "panel", "remote-actions", "maybe"]).is_err());

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("panel.json");
        let saved = || RemoteConfig::load(&path).unwrap().allow_remote_destructive;
        // Nothing saved yet: off, and asking does not create the file.
        assert_eq!(panel_remote_actions(RemoteActionsCmd::Status, &path), 0);
        assert!(!path.exists());
        // The rest of the settings is kept.
        let mut cfg = RemoteConfig { enabled: true, ..Default::default() };
        cfg.add_user("sam", kora_web::remote::Role::Admin).unwrap();
        cfg.save(&path).unwrap();
        assert_eq!(panel_remote_actions(RemoteActionsCmd::On, &path), 0);
        assert!(saved());
        assert_eq!(panel_remote_actions(RemoteActionsCmd::Status, &path), 0);
        assert!(saved(), "status changes nothing");
        assert_eq!(panel_remote_actions(RemoteActionsCmd::Off, &path), 0);
        assert!(!saved());
        let after = RemoteConfig::load(&path).unwrap();
        assert!(after.enabled && after.user("sam").is_some());
        // A damaged file is reported, never replaced.
        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(panel_remote_actions(RemoteActionsCmd::On, &path), 1);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{not json");

        assert!(remote_actions_message(RemoteActionsCmd::On, true).starts_with("Remote administrators can now install, delete and uninstall servers"));
        assert!(remote_actions_message(RemoteActionsCmd::Off, false).starts_with("Remote actions are turned off."));
        assert!(remote_actions_message(RemoteActionsCmd::Status, true).contains("kora panel remote-actions off"));
        assert!(remote_actions_message(RemoteActionsCmd::Status, false).contains("kora panel remote-actions on"));
    }

    #[test]
    fn root_only_help_gives_the_commands_to_continue() {
        let text = ROOT_ONLY_HELP.join("\n");
        for want in ["adduser kora", "usermod -aG sudo kora", "loginctl enable-linger kora", "su - kora", "kora panel --setup"] {
            assert!(text.contains(want), "{want}");
        }
    }

    #[test]
    fn panel_restart_restarts_only_the_background_panel() {
        let cmd = |a: &[&str]| match Cli::try_parse_from(a).map(|c| c.command) {
            Ok(Some(Command::Panel(p))) => Ok(p.cmd),
            Ok(_) => panic!("not the panel command"),
            Err(e) => Err(e.to_string()),
        };
        assert!(matches!(cmd(&["kora", "panel", "restart"]), Ok(Some(PanelCmd::Restart))));
        assert!(cmd(&["kora", "panel", "restart", "now"]).is_err(), "it takes no arguments");
        assert!(matches!(cmd(&["kora", "panel"]), Ok(None)));
        // (background panel set up, a `kora panel` window running)
        assert_eq!(panel_restart_mode(true, false), PanelRestart::Background);
        assert_eq!(panel_restart_mode(true, true), PanelRestart::Background, "the background panel is the one to restart");
        assert_eq!(panel_restart_mode(false, true), PanelRestart::Window, "a window is stopped by hand");
        assert_eq!(panel_restart_mode(false, false), PanelRestart::NotRunning);
    }

    struct Ports(Vec<u16>);
    impl kora_web::launch::PortCheck for Ports {
        fn is_free(&self, p: u16) -> bool {
            !self.0.contains(&p)
        }
        fn is_kora_panel(&self, p: u16) -> bool {
            self.0.contains(&p)
        }
    }

    #[test]
    fn a_running_panel_is_found_whether_in_the_background_or_in_a_window() {
        use kora_core::monitor::panel_service::RunningPanel;
        let window = RunningPanel { port: 47121, pid: 9, token: "cd".into() };
        let service = Some(("http://127.0.0.1:47120/#token=ab".to_string(), 47120));
        assert_eq!(
            find_running(service.clone(), Some(window.clone()), &Ports(vec![47120, 47121])),
            Some(Running::Background { port: 47120, link: "http://127.0.0.1:47120/#token=ab".into() }),
            "the background service wins"
        );
        assert_eq!(
            find_running(service.clone(), Some(window.clone()), &Ports(vec![47121])),
            Some(Running::Window { port: 47121, link: "http://127.0.0.1:47121/#token=cd".into() }),
            "a service that is not answering is not running"
        );
        assert_eq!(find_running(service, Some(window), &Ports(vec![])), None, "stale records are ignored");
        assert_eq!(find_running(None, None, &Ports(vec![47120])), None, "someone else's panel has no link we know");
    }

    #[test]
    fn the_background_port_prefers_the_asked_then_saved_then_default_port() {
        let home = tempfile::tempdir().unwrap();
        let args = |port: Option<u16>| PanelArgs {
            port,
            bind: "127.0.0.1".parse().unwrap(),
            remote: false,
            tailscale: false,
            admin: None,
            listen_tailscale: false,
            allow_public: false,
            new_token: false,
            no_open: true,
            foreground: false,
            setup: true,
            install_service: false,
            remove_service: false,
            print_link: false,
            cmd: None,
        };
        assert_eq!(background_port(&args(None), home.path(), &Ports(vec![])), Ok(47120));
        assert_eq!(background_port(&args(None), home.path(), &Ports(vec![47120, 47121])), Ok(47122));
        assert!(background_port(&args(Some(47120)), home.path(), &Ports(vec![47120])).unwrap_err().contains("47120 is already in use"));
        std::fs::write(kora_core::monitor::panel_service::info_path(home.path()), r#"{"port":47130}"#).unwrap();
        assert_eq!(background_port(&args(None), home.path(), &Ports(vec![])), Ok(47130), "the saved port is kept");
        // `Ports` says 47130 is busy AND a Kora panel: our own panel, restarted on the same port.
        assert_eq!(background_port(&args(None), home.path(), &Ports(vec![47130])), Ok(47130), "our own running panel keeps its port");
        struct Other;
        impl kora_web::launch::PortCheck for Other {
            fn is_free(&self, p: u16) -> bool { p != 47130 }
            fn is_kora_panel(&self, _: u16) -> bool { false }
        }
        assert_eq!(background_port(&args(None), home.path(), &Other), Ok(47120), "someone else's program on the saved port");
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
