//! `kora install`: the scriptable installer (with no options it opens the panel instead).

use crate::cmd::github::{github_owners, resolve_github_owner};
use std::path::PathBuf;

use clap::{Args, Parser, ValueEnum};
use kora_core::firewall::ports::{self, PortProber, SystemProber};
use kora_core::installer::{self, host, registry, DbAdmin, DbChoice, InstallRequest, Mode, Ports, ProgressEvent, ProgressSink, ResourceSelection, StepStatus};

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum ModeArg {
    Prod,
    Dev,
}

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum DbArg {
    Existing,
    Install,
    Skip,
}

#[derive(Args, Default, Clone)]
pub(crate) struct InstallArgs {
    /// REMOVED in 0.4.0 (setup codes no longer exist): use the flags below or `kora panel`.
    /// Hidden from help; it still parses so an old script gets the explanation instead of a usage error.
    #[arg(long, value_name = "CODE", hide = true)]
    pub(crate) code: Option<String>,
    /// Server name (display + registry name).
    #[arg(long)]
    pub(crate) name: Option<String>,
    /// Server folder (default: ~/kora/<name> in your home folder, also with sudo; /home/kora/<name> when logged in as root; Windows: C:\Kora\<name>, the same as in the panel).
    #[arg(long)]
    pub(crate) dir: Option<PathBuf>,
    /// prod = pinned release clones, dev = forks on your GitHub account.
    #[arg(long, value_enum)]
    pub(crate) mode: Option<ModeArg>,
    /// Cfx.re license key (prefer the CFX_LICENSE_KEY environment variable).
    #[arg(long, env = "CFX_LICENSE_KEY", hide_env_values = true)]
    pub(crate) license_key: Option<String>,
    /// Use the saved licence key with this name (see `kora keys list`) instead of passing a key.
    #[arg(long, alias = "licence-key-name", value_name = "NAME")]
    pub(crate) license_key_name: Option<String>,
    /// Also save the key from CFX_LICENSE_KEY under this name, so the next install can pick it with --license-key-name.
    #[arg(long, alias = "save-licence-key", value_name = "NAME")]
    pub(crate) save_license_key: Option<String>,
    #[arg(long)]
    pub(crate) game_port: Option<u16>,
    #[arg(long)]
    pub(crate) txadmin_port: Option<u16>,
    /// Also open the txAdmin port in the firewall (default: txAdmin stays private).
    #[arg(long)]
    pub(crate) txadmin_public: bool,
    /// Cfx.re account that becomes the txAdmin master, e.g. fivem:123456 (default: a generated local admin).
    #[arg(long)]
    pub(crate) cfx_identifier: Option<String>,
    /// Your Cfx.re username (the name on forum.cfx.re). Kora looks up your account id and makes you the txAdmin admin.
    #[arg(long, value_name = "NAME", conflicts_with = "cfx_identifier")]
    pub(crate) cfx_user: Option<String>,
    /// Official runtime.fivem.net artifact URL (default: the recommended build).
    #[arg(long)]
    pub(crate) artifact_url: Option<String>,
    /// `all` or a comma separated list of resource ids (dependencies and core are always included).
    #[arg(long)]
    pub(crate) resources: Option<String>,
    /// existing = use the local MariaDB, install = install it if missing, skip = no database.
    #[arg(long, value_enum)]
    pub(crate) db: Option<DbArg>,
    /// Install and enable the OS service (needs admin).
    #[arg(long)]
    pub(crate) service: bool,
    /// Open the game port in the OS firewall (needs admin).
    #[arg(long)]
    pub(crate) firewall: bool,
    /// Dev mode: GitHub user or organisation to fork into (`list` shows the choices).
    #[arg(long)]
    pub(crate) github_owner: Option<String>,
    /// Catalog file or URL (default: the public catalog with an embedded fallback). Env: KORA_CATALOG.
    #[arg(long, hide = true)]
    pub(crate) catalog: Option<String>,
    /// Database admin user used once to create the new database + user (password: KORA_DB_ADMIN_PASSWORD).
    #[arg(long)]
    pub(crate) db_admin_user: Option<String>,
    /// Database server address for the admin login (default 127.0.0.1).
    #[arg(long)]
    pub(crate) db_admin_host: Option<String>,
    /// Database server port for the admin login (default 3306).
    #[arg(long)]
    pub(crate) db_admin_port: Option<u16>,
    /// Start the server when the install finishes and wait until txAdmin answers (the interactive installer does this by default).
    #[arg(long)]
    pub(crate) start: bool,
    /// Do not start the server after installing (start it later with `kora start <name>`).
    #[arg(long, conflicts_with = "start")]
    pub(crate) no_start: bool,
    /// Show the full plan and change nothing.
    #[arg(long)]
    pub(crate) dry_run: bool,
    /// Accepted and ignored since 0.7.0 (the install no longer adds a machine-wide Kora app).
    #[arg(long, hide = true)]
    pub(crate) no_app: bool,
    /// Accepted and ignored since 0.7.0 (shortcuts are made by the install scripts and `kora app install`).
    #[arg(long, hide = true)]
    pub(crate) no_desktop_shortcut: bool,
    /// If a port is in use, silently use the next free one and say what changed.
    #[arg(long)]
    pub(crate) auto_ports: bool,
    /// Server hostname shown in the server list (default: the server name).
    #[arg(long)]
    pub(crate) hostname: Option<String>,
    /// Pin the FiveM game build (e.g. 3258); default: the build the server ships with.
    #[arg(long, value_name = "N")]
    pub(crate) game_build: Option<u32>,
    /// Linux, with --service: when the server folder is inside a private folder (such as your home
    /// folder) that the service's own account cannot enter, let it pass through exactly those
    /// folders (`chmod o+x`) instead of stopping.
    #[arg(long)]
    pub(crate) fix_permissions: bool,
    /// Do not ask anything (implied by any other option).
    #[arg(long, short = 'y')]
    pub(crate) yes: bool,
}

impl InstallArgs {
    pub(crate) fn any_given(&self) -> bool {
        self.name.is_some()
            || self.dir.is_some()
            || self.hostname.is_some()
            || self.game_build.is_some()
            || self.mode.is_some()
            || self.license_key.is_some()
            || self.license_key_name.is_some()
            || self.save_license_key.is_some()
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

    pub(crate) fn into_request(self) -> Result<InstallRequest, String> {
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
            license_key_name: self.license_key_name,
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

/// Plain printing sink.
pub(crate) struct Printer {
    pub(crate) last: Option<(String, String)>,
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
pub(crate) const CODE_REMOVED: &str =
    "error: setup codes were removed in 0.4.0. Use the flags instead (see `kora install --help`), `kora add-resources <server> --resource <ID>`, or the panel (`kora panel`).";

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

pub(crate) fn install(args: InstallArgs) -> i32 {
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
        return crate::bare::open_panel(crate::bare::Landing::Install, false);
    }
    if let Some(name) = args.cfx_user.take() {
        match kora_core::cfx::lookup(&name) {
            Ok(a) => {
                println!("Cfx.re account {} is {}: it will be the txAdmin admin.", a.username, a.identifier());
                kora_core::profile::ProfileStore::default().remember_cfx(&a.username, &a.identifier());
                args.cfx_identifier = Some(a.identifier());
            }
            Err(e) => {
                eprintln!("error: {e}");
                return 2;
            }
        }
    }
    // A saved key by name, or a pasted one to save: the request then names it (a name is not a secret).
    match crate::keys_cmd::resolve_install_key(args.license_key.take(), args.license_key_name.as_deref(), args.save_license_key.as_deref()) {
        Ok((key, name)) => {
            if args.save_license_key.is_some() && args.license_key_name.is_none() {
                println!("Saved the licence key as \"{}\" for next time.", name.as_deref().unwrap_or_default());
            }
            args.license_key = key;
            args.license_key_name = name;
        }
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    }
    let args_auto_ports = args.auto_ports;
    let fix_permissions = args.fix_permissions;
    let dev_mode = matches!(args.mode, Some(ModeArg::Dev));
    if dev_mode && !args.dry_run {
        match resolve_github_owner(args.github_owner.as_deref()) {
            Ok(l) => {
                kora_core::profile::ProfileStore::default().remember_github_owner(&l);
                args.github_owner = Some(l);
            }
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
                eprintln!(
                    "The install state is saved in {}: run the same command again to resume, or `kora delete {}` to remove what was created.",
                    req.dir.join(".kora").display(),
                    req.dir.display()
                );
            }
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Cli;
    use crate::Command;
    use clap::Parser;

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
            license_key_name: None,
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
