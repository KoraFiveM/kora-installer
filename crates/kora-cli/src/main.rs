//! `kora` command line entry point.

use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::mpsc;

use clap::{Args, Parser, Subcommand, ValueEnum};
use kora_core::firewall::{check_port, PortStatus};
use kora_core::installer::{
    self, gitops, host, registry, DbAdmin, DbChoice, InstallRequest, Mode, Ports, ProgressEvent, ProgressSink, ResourceSelection,
    RollbackOptions, StepStatus,
};

#[derive(Parser)]
#[command(name = "kora", version, about = "Kora installer and server manager")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
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

#[derive(Args, Default)]
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
    /// Dev mode: GitHub user or organisation to fork into.
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
    /// Show the full plan and change nothing.
    #[arg(long)]
    dry_run: bool,
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
                let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default();
                home.join("kora").join(installer::slug(&name))
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
            hostname: name.clone(),
            name,
            dir,
            license_key: self.license_key,
            cfx_identifier: self.cfx_identifier,
            txadmin_public: self.txadmin_public,
            ports: Ports { game: self.game_port.unwrap_or(defaults.game), txadmin: self.txadmin_port.unwrap_or(defaults.txadmin) },
            artifact_url: self.artifact_url,
            game_build: None,
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
        })
    }
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
    /// Remove Kora system changes (service, timers, firewall rules, registry entry).
    Uninstall,
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

/// Interactive wizard (`None` = the user quit without installing).
fn interactive_install() -> Result<Option<InstallRequest>, String> {
    kora_tui::run_install_wizard().map_err(|e| e.to_string())
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

fn print_outcome(out: &installer::InstallOutcome, dry: bool) {
    println!();
    println!("{}", if dry { "Plan (nothing was changed):" } else { "Install finished." });
    println!("  folder:  {}", out.dir.display());
    println!("  connect: {}", out.connect);
    println!("  txAdmin: {}", out.txadmin_url);
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
fn interactive_flow(catalog: Option<String>) -> i32 {
    let mut req = match interactive_install() {
        Ok(Some(r)) => r,
        Ok(None) => {
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
                    "What was done so far is kept. Run `kora install` again to continue where it stopped, or `kora delete {}` to remove what was created.",
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
    let headless = args.any_given();
    if let Some(c) = &args.code {
        eprintln!("note: setup code {c} noted; redeeming codes needs the website API (not available yet), using your options instead.");
    }
    if !headless {
        if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
            eprintln!(
                "error: `kora install` needs an interactive terminal to ask its questions. \
                 Run it in a terminal, or pass options instead, e.g. `kora install --name \"My Server\" --dir ~/kora/my-server --license-key <key> -y` (see `kora install --help`)."
            );
            return 2;
        }
        let catalog = args.catalog.or_else(|| std::env::var("KORA_CATALOG").ok().filter(|c| !c.is_empty()));
        return interactive_flow(catalog);
    }
    let req = match args.into_request() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let dry = req.dry_run;
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
    println!("{:<20} {:<5} {:<7} {:<7} {:<9} folder", "NAME", "MODE", "GAME", "TXADMIN", "STATE");
    for s in &reg.servers {
        let svc = host::service_state(&s.name).ok();
        let state = if !s.dir.is_dir() {
            "missing"
        } else if svc == Some(kora_core::service::ServiceState::Running) {
            "running"
        } else if svc == Some(kora_core::service::ServiceState::Stopped) {
            "stopped"
        } else if matches!(check_port(s.ports.game, false), PortStatus::InUse { .. }) {
            "running"
        } else {
            "stopped"
        };
        let mode = if s.mode == Mode::Production { "prod" } else { "dev" };
        println!("{:<20} {:<5} {:<7} {:<7} {:<9} {}", s.name, mode, s.ports.game, s.ports.txadmin, state, s.dir.display());
    }
    0
}

fn control(server: &str, action: kora_core::service::ControlAction) -> i32 {
    use kora_core::service::ControlAction as A;
    let (verb, past) = match action {
        A::Start => ("start", "Started"),
        A::Stop => ("stop", "Stopped"),
        A::Restart => ("restart", "Restarted"),
    };
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

fn main() {
    let cli = Cli::parse();
    let code = match cli.command {
        None => install(InstallArgs::default()),
        Some(Command::Install(a)) => install(*a),
        Some(Command::Manager) => not_implemented("manager"),
        Some(Command::Update { auto, server }) => update(auto, &server),
        Some(Command::Status) => status(),
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
        Some(Command::Uninstall) => not_implemented("uninstall"),
    };
    std::process::exit(code);
}
