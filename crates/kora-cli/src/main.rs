//! `kora` command line entry point.

use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::mpsc;

use clap::{Args, Parser, Subcommand, ValueEnum};
use kora_core::elevate;
use kora_core::firewall::ports::{self, PortProber, SystemProber};
use kora_core::launch;
use kora_core::site::{self, Redeemed};
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
    /// Single-use setup code from the website (e.g. KORA-7F3K-92QX).
    #[arg(long)]
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
    /// If a port is in use, silently use the next free one and say what changed.
    #[arg(long)]
    auto_ports: bool,
    /// Filled from a setup code (not a flag).
    #[arg(skip)]
    hostname: Option<String>,
    /// Filled from a setup code (not a flag).
    #[arg(skip)]
    game_build: Option<u32>,
    /// Do not ask anything (implied by any other option).
    #[arg(long, short = 'y')]
    yes: bool,
}

impl InstallArgs {
    fn any_given(&self) -> bool {
        self.name.is_some()
            || self.dir.is_some()
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

    /// Use the website's choices as defaults: anything given explicitly as a flag wins.
    fn apply_redeemed(&mut self, r: &Redeemed) {
        self.name.get_or_insert_with(|| r.name.clone());
        self.mode.get_or_insert(match r.mode {
            Mode::Production => ModeArg::Prod,
            Mode::Development => ModeArg::Dev,
        });
        self.game_port.get_or_insert(r.ports.game);
        self.txadmin_port.get_or_insert(r.ports.txadmin);
        if self.resources.is_none() {
            self.resources = Some(match &r.resources {
                ResourceSelection::All => "all".to_string(),
                ResourceSelection::Only(ids) => ids.join(","),
            });
        }
        if self.artifact_url.is_none() {
            self.artifact_url = r.artifact_url.clone();
        }
        if self.cfx_identifier.is_none() {
            self.cfx_identifier = r.cfx_identifier.clone();
        }
        self.hostname = r.hostname.clone();
        self.game_build = r.game_build;
    }

    /// True when, after a setup code, nothing is left to ask: license key, folder, a database
    /// admin login (or no database) and, for development mode, the fork owner.
    fn headless_ready(&self, dev: bool) -> bool {
        let db_ok = matches!(self.db, Some(DbArg::Skip))
            || self.db_admin_user.is_some()
            || std::env::var("KORA_DB_ADMIN_PASSWORD").map(|p| !p.is_empty()).unwrap_or(false);
        self.license_key.is_some() && self.dir.is_some() && db_ok && (!dev || self.github_owner.is_some())
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
        })
    }
}

/// The user's home folder: USERPROFILE first on Windows (HOME may point elsewhere under Git Bash),
/// HOME first elsewhere.
fn home_dir(windows: bool, get: &dyn Fn(&str) -> Option<std::ffi::OsString>) -> PathBuf {
    let order = if windows { ["USERPROFILE", "HOME"] } else { ["HOME", "USERPROFILE"] };
    order.iter().find_map(|k| get(k).filter(|v| !v.is_empty())).map(PathBuf::from).unwrap_or_default()
}

#[derive(Subcommand)]
enum Command {
    /// Install a new Kora server (interactive TUI, or headless with options).
    Install(Box<InstallArgs>),
    /// Open the multi-server manager.
    Manager,
    /// Update a server (used by schedulers).
    Update {
        /// Run unattended (fast-forward clean forks only).
        #[arg(long)]
        auto: bool,
        /// Server name or id.
        server: String,
    },
    /// Show registered servers.
    Status,
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
    },
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
    use kora_core::git::{owners, SystemRunner};
    let all = owners::list_owners(&SystemRunner).map_err(|e| format!("could not read your GitHub accounts ({e}). Is `gh` installed and logged in? Run `gh auth login`."))?;
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

fn not_implemented(what: &str) -> i32 {
    eprintln!("kora {what}: not implemented yet");
    2
}

/// Exchange a setup code for the website's choices (and verify the artifact build).
fn redeem_code(code: &str) -> Result<Redeemed, String> {
    site::redeem_and_resolve(code, &site::site_url_from_env(), &installer::HttpClient::default()).map_err(|e| e.to_string())
}

/// Interactive wizard (`None` = the user quit without installing).
fn interactive_install(prefill: Option<Redeemed>) -> Result<Option<InstallRequest>, String> {
    let redeemer: kora_tui::wizard::Redeemer = Box::new(redeem_code);
    kora_tui::run_install_wizard_with(Some(redeemer), prefill).map_err(|e| e.to_string())
}

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

/// Dry-run header: the effective choices (website code + flags merged). Never includes secrets.
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
fn interactive_flow(catalog: Option<String>, prefill: Option<Redeemed>) -> i32 {
    let mut req = match interactive_install(prefill) {
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
            let next = if req.dir.join(".kora").is_dir() {
                format!(
                    "What was done so far is kept.\nTo try again: run `kora install` again; it continues where it stopped.\nTo start over: run `kora delete {}` to remove what was created.",
                    req.dir.display()
                )
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
    let mut redeemed: Option<Redeemed> = None;
    if let Some(code) = args.code.take() {
        match redeem_code(&code) {
            Ok(r) => {
                println!("{}", r.welcome());
                redeemed = Some(r);
            }
            Err(e) => {
                eprintln!("error: {e}");
                return 1;
            }
        }
    }
    let mut headless = args.any_given();
    if let Some(r) = &redeemed {
        let dev = r.mode == Mode::Development;
        if args.headless_ready(dev) {
            headless = true;
        } else if headless && dev && args.github_owner.is_none() {
            eprintln!("error: your setup code is for a development server: pass --github-owner <your GitHub user or organisation> (`kora github owners` lists them; the website does not decide where to fork).");
            return 2;
        }
        args.apply_redeemed(r);
    }
    if !headless {
        if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
            eprintln!(
                "error: `kora install` needs an interactive terminal to ask its questions. \
                 Run it in a terminal, or pass options instead (with a setup code you still need --license-key, --dir and a database admin login: --db-admin-user or --db skip), e.g. `kora install --name \"My Server\" --dir ~/kora/my-server --license-key <key> -y` (see `kora install --help`)."
            );
            return 2;
        }
        let catalog = args.catalog.or_else(|| std::env::var("KORA_CATALOG").ok().filter(|c| !c.is_empty()));
        return interactive_flow(catalog, redeemed);
    }
    let args_auto_ports = args.auto_ports;
    let dev_mode = matches!(args.mode, Some(ModeArg::Dev)) || redeemed.as_ref().is_some_and(|r| r.mode == Mode::Development);
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

fn update(auto: bool, server: &str) -> i32 {
    if !auto {
        eprintln!("note: interactive updates arrive with the manager; running the safe fast-forward-only update.");
    }
    match gitops::auto_update(server) {
        Ok(report) => {
            println!("{}", report.summary());
            i32::from(report.has_problems())
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
        return 0;
    }
    println!("\nRemoved: {}. Kept: {}. Failed: {}.", summary.removed.len(), summary.kept.len(), summary.failed.len());
    for f in &summary.failed {
        eprintln!("  failed: {f}");
    }
    i32::from(!summary.ok())
}

fn command_kind(cmd: &Option<Command>) -> elevate::CmdKind {
    use elevate::CmdKind as K;
    match cmd {
        None => K::Wizard,
        Some(Command::Install(a)) => K::Install { dry_run: a.dry_run },
        Some(Command::Manager) => K::Manager,
        Some(Command::Update { auto, .. }) => K::Update { auto: *auto },
        Some(Command::Status) => K::Status,
        Some(Command::Delete { .. }) => K::Delete,
        Some(Command::ServiceHost { .. } | Command::Github { .. }) => K::Other,
        Some(Command::Start { .. }) => K::Start,
        Some(Command::Stop { .. }) => K::Stop,
        Some(Command::Restart { .. }) => K::Restart,
        Some(Command::Uninstall { dry_run, .. }) => K::Uninstall { dry_run: *dry_run },
    }
}

/// Linux/macOS: why this command needs root (empty = it does not). The wizard decides for itself later.
fn admin_reasons_for_command(cmd: &Option<Command>) -> Vec<String> {
    use kora_core::service::ServiceState;
    let one = |s: &str| vec![s.to_string()];
    match cmd {
        Some(Command::Install(a)) => {
            let headless = a.any_given() || (a.code.is_some() && a.headless_ready(false));
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
        Some(Command::Uninstall { dry_run: false, .. }) => one("remove the services, firewall rules and accounts Kora set up"),
        _ => vec![],
    }
}

/// The arguments for the elevated copy: the original ones, plus the folder when it was only implied
/// (a different account's home folder would otherwise change where the server goes).
fn relaunch_args(cmd: &Option<Command>, original: Vec<std::ffi::OsString>) -> Vec<std::ffi::OsString> {
    let mut args = original;
    if let Some(Command::Install(a)) = cmd {
        if a.dir.is_none() && a.code.is_none() {
            if let Ok(req) = a.as_ref().clone().into_request() {
                args.push("--dir".into());
                args.push(req.dir.into_os_string());
            }
        }
    }
    args
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
fn elevate_if_needed(cli: &Cli) {
    let kind = command_kind(&cli.command);
    let tty = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    if !tty || cli.no_elevate || !(cfg!(windows) && kind.privileged() || cfg!(not(windows))) || elevate::is_elevated() {
        return;
    }
    let reasons = if cfg!(windows) { vec![] } else { admin_reasons_for_command(&cli.command) };
    let env = elevate::Env { windows: cfg!(windows), elevated: false, tty, no_elevate: false };
    let need = elevate::decide(&env, kind, &reasons);
    let args = relaunch_args(&cli.command, std::env::args_os().skip(1).collect());
    if let elevate::Elevation::Exit(code) = elevate::ensure_elevated(&need, &args, &elevate::SystemElevator, &mut |m| println!("{m}")) {
        finish(code);
    }
}

fn main() {
    let cli = Cli::parse();
    elevate_if_needed(&cli);
    let code = match cli.command {
        None => install(InstallArgs::default()),
        Some(Command::Install(a)) => install(*a),
        Some(Command::Manager) => not_implemented("manager"),
        Some(Command::Update { auto, server }) => update(auto, &server),
        Some(Command::Status) => status(),
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
        Some(Command::Uninstall { server, dry_run, yes, delete_files, delete_databases }) => uninstall(server, dry_run, yes, delete_files, delete_databases),
    };
    finish(code);
}

#[cfg(test)]
mod tests {
    use super::*;
    use kora_core::site::ArtifactChoice;

    fn redeemed() -> Redeemed {
        Redeemed {
            mode: Mode::Development,
            name: "Alpha One".into(),
            resources: ResourceSelection::Only(vec!["a".into(), "b".into()]),
            artifact: ArtifactChoice::Build(38621),
            artifact_url: Some("https://runtime.fivem.net/x".into()),
            game_build: Some(3258),
            ports: Ports { game: 31120, txadmin: 41120 },
            hostname: Some("Alpha RP".into()),
            cfx_identifier: Some("fivem:1".into()),
            cfx_username: Some("Sticky".into()),
            github_login: Some("octo".into()),
        }
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

    #[test]
    fn redeemed_values_become_defaults_and_flags_win() {
        let mut a = InstallArgs { game_port: Some(30000), ..Default::default() };
        a.apply_redeemed(&redeemed());
        let r = a.into_request().unwrap();
        assert_eq!(r.mode, Mode::Development);
        assert_eq!((r.name.as_str(), r.hostname.as_str()), ("Alpha One", "Alpha RP"));
        assert_eq!((r.ports.game, r.ports.txadmin), (30000, 41120), "explicit flag wins");
        assert_eq!(r.resources, ResourceSelection::Only(vec!["a".into(), "b".into()]));
        assert_eq!(r.game_build, Some(3258));
        assert_eq!(r.cfx_identifier.as_deref(), Some("fivem:1"));
        assert_eq!(r.artifact_url.as_deref(), Some("https://runtime.fivem.net/x"));
        assert!(r.github_owner.is_none(), "fork owner never comes from the website");
        assert!(r.license_key.is_none());
    }

    #[test]
    fn headless_only_when_everything_secret_is_given() {
        let base = || InstallArgs { license_key: Some("k".into()), dir: Some("/x".into()), db: Some(DbArg::Skip), ..Default::default() };
        assert!(base().headless_ready(false));
        assert!(!base().headless_ready(true), "dev needs --github-owner");
        assert!(InstallArgs { github_owner: Some("me".into()), ..base() }.headless_ready(true));
        assert!(!InstallArgs { license_key: None, ..base() }.headless_ready(false));
        assert!(!InstallArgs { dir: None, ..base() }.headless_ready(false));
        assert!(InstallArgs { db: None, db_admin_user: Some("root".into()), ..base() }.headless_ready(false));
    }

    #[test]
    fn dry_run_summary_shows_choices_but_no_secrets() {
        let mut a = InstallArgs { license_key: Some("cfxk_secret".into()), ..Default::default() };
        a.apply_redeemed(&redeemed());
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
        let e = settle_ports(&mut r, false, &Busy(vec![30120])).unwrap_err();
        assert!(e.contains("Port 30120 (game)") && e.contains("next free port is 30121") && e.contains("--game-port 30121 --txadmin-port 40120") && e.contains("--auto-ports"), "{e}");
        assert_eq!(r.ports.game, 30120, "nothing changed without --auto-ports");
        assert!(settle_ports(&mut r, false, &Busy(vec![])).unwrap().is_empty());
    }

    #[test]
    fn auto_ports_picks_next_free_and_says_so() {
        let mut r = req_with_ports(30120, 40120);
        let changes = settle_ports(&mut r, true, &Busy(vec![30120, 30121, 40120])).unwrap();
        assert_eq!(changes, vec!["Port 30120 is in use, using 30122", "Port 40120 is in use, using 40121"]);
        assert_eq!((r.ports.game, r.ports.txadmin), (30122, 40121));
        let mut r = req_with_ports(65535, 40120);
        assert!(settle_ports(&mut r, true, &Busy(vec![65535])).is_err(), "exhausted");
    }

    #[test]
    fn ports_from_a_setup_code_are_rechecked_locally() {
        let mut a = InstallArgs { auto_ports: true, dir: Some("/nonexistent/kora-ports-test".into()), ..Default::default() };
        a.apply_redeemed(&redeemed());
        let mut r = a.into_request().unwrap();
        let changes = settle_ports(&mut r, true, &Busy(vec![31120])).unwrap();
        assert_eq!(changes, vec!["Port 31120 is in use, using 31121"]);
    }

    #[test]
    fn dry_run_shows_the_effective_ports() {
        let mut r = req_with_ports(30120, 40120);
        settle_ports(&mut r, true, &Busy(vec![30120])).unwrap();
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
