//! `kora panel --tailscale`: open Kora Panel to your own devices over Tailscale in one step.
//!
//! What it does, and why it is safe:
//! * The panel keeps listening on 127.0.0.1 and ALSO on this computer's Tailscale addresses (never on
//!   every interface), so only devices on your tailnet can reach it. Tailscale encrypts the traffic.
//! * People sign in with their Tailscale account: the panel asks the local Tailscale service who is
//!   connecting (it never trusts anything the browser says) and lets in only the accounts listed in
//!   `panel.json`, each with a role. No link, token or tunnel is needed.
//! * The first admin is the Tailscale account you are connected from (an SSH session over Tailscale),
//!   else the account that added this computer to Tailscale, else the one you name with `--admin`.
//!
//! Everything that asks Tailscale goes through [`kora_web::remote::Whois`]; the decisions are plain
//! functions tested below.

use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::time::Duration;

use clap::Subcommand;
use kora_core::monitor::panel_service as svc;
use kora_web::remote::{config, is_tailnet_ip, RemoteConfig, Role, SystemWhois, TsSelf, Whois};

#[derive(Subcommand, Debug)]
pub enum TailscaleCmd {
    /// Let a Tailscale account sign in to the panel, with a role.
    Allow {
        /// Their Tailscale login, for example name@example.com (GitHub accounts look like name@github).
        login: String,
        /// What they may do: viewer (look only), operator (run servers, updates, logs) or admin (everything).
        #[arg(long, default_value = "operator", value_parser = ["viewer", "operator", "admin"])]
        role: String,
    },
    /// Stop letting a Tailscale account sign in; their sessions end at once.
    Remove {
        /// The Tailscale login to remove.
        login: String,
    },
    /// List the Tailscale accounts that may sign in.
    List,
    /// Turn Tailscale access off: the panel listens on this computer only again.
    Off,
}

/// A Tailscale login as Kora stores it (lower case), if it looks like one.
pub fn normalise_login(login: &str) -> Result<String, String> {
    let l = login.trim().to_ascii_lowercase();
    let ok = l.len() <= 254
        && l.split_once('@').is_some_and(|(a, b)| !a.is_empty() && !b.is_empty())
        && !l.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '"' | '\'' | '<' | '>' | '/' | '\\'));
    if ok {
        Ok(l)
    } else {
        Err(format!("'{}' is not a Tailscale login. It looks like name@example.com (or name@github).", login.trim()))
    }
}

/// Where the first admin comes from, in the order Kora tries.
#[derive(Debug, PartialEq, Eq)]
pub enum AdminSource {
    /// `--admin`.
    Flag,
    /// The Tailscale account of the SSH session running this command.
    SshSession,
    /// The account that added this computer to Tailscale.
    Owner,
    /// Already an admin in the settings: nothing to add.
    Existing,
}

impl AdminSource {
    fn why(&self) -> &'static str {
        match self {
            AdminSource::Flag => "you named it with --admin",
            AdminSource::SshSession => "the Tailscale account you are connected from",
            AdminSource::Owner => "the account that added this computer to Tailscale",
            AdminSource::Existing => "already an admin",
        }
    }
}

/// The SSH client's address when it is a Tailscale address (`SSH_CONNECTION` = client ip, client port,
/// server ip, server port).
pub fn ssh_client_on_tailnet(ssh_connection: Option<&str>) -> Option<SocketAddr> {
    let mut f = ssh_connection?.split_whitespace();
    let ip: IpAddr = f.next()?.parse().ok()?;
    let port: u16 = f.next()?.parse().ok()?;
    is_tailnet_ip(ip).then_some(SocketAddr::new(ip, port))
}

/// Choose the first admin. `None` means Kora cannot tell (a tagged computer reached without Tailscale):
/// ask, or tell the person to use `--admin`.
pub fn choose_admin(
    flag: Option<&str>,
    ssh: Option<SocketAddr>,
    ts: &dyn Whois,
    status: &TsSelf,
    cfg: &RemoteConfig,
) -> Result<Option<(String, AdminSource)>, String> {
    if let Some(f) = flag {
        return Ok(Some((normalise_login(f)?, AdminSource::Flag)));
    }
    if let Some(peer) = ssh {
        if let Ok(login) = ts.whois(peer) {
            if let Ok(l) = normalise_login(&login) {
                return Ok(Some((l, AdminSource::SshSession)));
            }
        }
    }
    if let Some(o) = status.owner.as_deref().and_then(|o| normalise_login(o).ok()) {
        return Ok(Some((o, AdminSource::Owner)));
    }
    if let Some((l, _)) = cfg.tailscale_users.iter().find(|(_, r)| **r == Role::Admin) {
        return Ok(Some((l.clone(), AdminSource::Existing)));
    }
    Ok(None)
}

/// Turn Tailscale access on in `cfg` with `admin` as an admin. Other accounts are left as they are;
/// an account that is already listed keeps the higher of its roles.
pub fn enable(cfg: &mut RemoteConfig, admin: &str) {
    cfg.enabled = true;
    cfg.tailscale = true;
    let entry = cfg.tailscale_users.entry(admin.to_string()).or_insert(Role::Admin);
    *entry = Role::Admin;
}

/// Ready to listen on Tailscale: Tailscale's state and the admin that was set.
pub struct Ready {
    pub status: TsSelf,
    pub admin: String,
}

fn tailscale_status() -> Result<TsSelf, String> {
    let Some(s) = SystemWhois.status() else {
        return Err("Tailscale is not installed or not running on this computer. Install it (https://tailscale.com/download), \
connect it with `sudo tailscale up`, then run `kora panel --tailscale` again."
            .into());
    };
    if !s.running || !s.ips.iter().any(|i| is_tailnet_ip(*i)) {
        return Err("Tailscale is installed but not connected. Run `sudo tailscale up`, then `kora panel --tailscale` again.".into());
    }
    Ok(s)
}

/// `kora panel --tailscale` (the part before the panel starts): check Tailscale, choose the admin and
/// save the settings. Never opens anything to the network by itself.
pub fn configure(admin_flag: Option<&str>) -> Result<Ready, String> {
    let status = tailscale_status()?;
    let path = config::default_path();
    let mut cfg = RemoteConfig::load(&path)?;
    let ssh = ssh_client_on_tailnet(std::env::var("SSH_CONNECTION").ok().as_deref());
    let (admin, source) = match choose_admin(admin_flag, ssh, &SystemWhois, &status, &cfg)? {
        Some(a) => a,
        None => match ask_line("This computer belongs to no Tailscale account (it is tagged), so Kora cannot tell who should be the panel's admin.\nYour Tailscale login (name@example.com): ") {
            Some(l) if !l.trim().is_empty() => (normalise_login(&l)?, AdminSource::Flag),
            _ => return Err("Kora cannot tell which Tailscale account should be the panel's admin. Run it again with --admin you@example.com.".into()),
        },
    };
    enable(&mut cfg, &admin);
    cfg.save(&path)?;
    println!("Tailscale sign-in is on. Admin: {admin} ({}).", source.why());
    if super::launch_uid_is_root() {
        println!("Note: Kora Panel runs as root on this computer (you are signed in as root), so a panel admin can change anything here. Only make people you fully trust admin; give others the operator or viewer role.");
    }
    Ok(Ready { status, admin })
}

/// Start (or restart) the background panel listening on Tailscale too, wait for it, and say where to go.
pub fn start_background(a: &super::PanelArgs, home: &Path, ready: &Ready) -> i32 {
    if cfg!(target_os = "linux") && !super::user_services_available() {
        eprintln!("Kora Panel cannot run in the background from this session (common after sudo or su, or in a container).");
        eprintln!("Run it in a window instead (it stops when the window closes):  kora panel --tailscale --foreground");
        return 1;
    }
    let ports = kora_web::launch::SystemPorts;
    let port = match super::background_port(a, home, &ports) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    println!("Starting Kora Panel in the background, on this computer and on Tailscale ...");
    if let Err(e) = super::start_background_service(home, port, true) {
        eprintln!("error: could not set up the background panel: {e}");
        return 1;
    }
    if !kora_web::launch::wait_for_panel(port, super::SETUP_WAIT, Duration::from_millis(250), &ports) {
        eprintln!("Kora Panel did not answer within {} seconds.", super::SETUP_WAIT.as_secs());
        if cfg!(target_os = "linux") {
            eprintln!("See what happened with:  systemctl --user status kora-panel");
        }
        return 1;
    }
    let listening = ready
        .status
        .ips
        .iter()
        .filter(|i| is_tailnet_ip(**i))
        .any(|ip| wait_for_port(SocketAddr::new(*ip, port), Duration::from_secs(10)));
    open_firewall(port);
    println!();
    match ready.status.url(port) {
        Some(url) if listening => {
            println!("  Kora Panel is open to your Tailscale devices. On any computer or phone signed in");
            println!("  to Tailscale as {}, open:", ready.admin);
            println!();
            println!("    {url}");
            println!();
            println!("  You are signed in with your Tailscale account: no link, password or tunnel needed.");
            if ready.status.dns_name.is_none() {
                println!("  (Turn on MagicDNS in the Tailscale admin console to use a name instead of the address.)");
            }
        }
        _ => {
            eprintln!("Kora Panel runs, but is not answering on its Tailscale address yet. It keeps trying by itself;");
            eprintln!("check `tailscale status`, and `systemctl --user status kora-panel` on Linux.");
        }
    }
    println!();
    println!("Let someone else in:   kora panel tailscale allow name@example.com --role operator");
    println!("Turn it off again:     kora panel tailscale off");
    0
}

/// After `kora panel --setup` (the install scripts): use Tailscale when it is here.
/// Already set up: print the address. Otherwise, on a server with no screen, ask (on the terminal,
/// even under `curl | sh`); `KORA_TAILSCALE=1` says yes without asking, `KORA_TAILSCALE=0` never asks.
pub fn after_setup(a: &super::PanelArgs, home: &Path, headless: bool) {
    if svc::saved_tailscale(home) {
        if let Some(port) = svc::saved_port(home) {
            print_tailnet_address(home, port);
        }
        return;
    }
    let choice = std::env::var("KORA_TAILSCALE").ok();
    if choice.as_deref() == Some("0") {
        return;
    }
    if tailscale_status().is_err() {
        return;
    }
    let yes = match choice.as_deref() {
        Some("1") => true,
        _ if headless => ask_line("Tailscale is running on this computer. Open Kora Panel from your own devices over Tailscale\n(no link or tunnel needed, only your tailnet can reach it)? [Y/n] ")
            .is_some_and(|l| !matches!(l.trim().to_ascii_lowercase().as_str(), "n" | "no")),
        _ => false,
    };
    if !yes {
        println!();
        println!("Tip: Tailscale is running here. `kora panel --tailscale` lets you open the panel from your own devices, no tunnel needed.");
        return;
    }
    match configure(None) {
        Ok(ready) => {
            println!();
            start_background(a, home, &ready);
        }
        Err(e) => eprintln!("Tailscale was not set up: {e}"),
    }
}

/// Print the Tailscale address of the background panel, when it listens there.
pub fn print_tailnet_address(home: &Path, port: u16) {
    if !svc::saved_tailscale(home) {
        return;
    }
    if let Some(url) = SystemWhois.status().filter(|s| s.running).and_then(|s| s.url(port)) {
        println!("From your Tailscale devices (sign in with your Tailscale account): {url}");
    }
}

/// `kora panel tailscale ...`. The running panel notices changes to the settings by itself.
pub fn command(cmd: &TailscaleCmd, home: &Path) -> i32 {
    let path = config::default_path();
    let run = || -> Result<(), String> {
        let mut cfg = RemoteConfig::load(&path)?;
        match cmd {
            TailscaleCmd::Allow { login, role } => {
                let login = normalise_login(login)?;
                let role: Role = role.parse()?;
                cfg.tailscale_users.insert(login.clone(), role);
                cfg.save(&path)?;
                println!("{login} may now sign in with Tailscale ({role}).");
                if !(cfg.enabled && cfg.tailscale && svc::saved_tailscale(home)) {
                    println!("Tailscale access is not on yet: run `kora panel --tailscale`.");
                }
            }
            TailscaleCmd::Remove { login } => {
                let l = login.trim().to_ascii_lowercase();
                if cfg.tailscale_users.remove(&l).is_none() {
                    return Err(format!("{} is not on the list (see `kora panel tailscale list`).", login.trim()));
                }
                cfg.save(&path)?;
                println!("Removed {l}. Their sessions have ended.");
                if cfg.tailscale && !cfg.tailscale_users.values().any(|r| *r == Role::Admin) {
                    println!("Nobody has the admin role over Tailscale now. Add one with: kora panel tailscale allow <login> --role admin");
                }
            }
            TailscaleCmd::List => {
                println!("Tailscale access: {}", if cfg.enabled && cfg.tailscale { "on" } else { "off" });
                if cfg.tailscale_users.is_empty() {
                    println!("Nobody may sign in with Tailscale yet. Run `kora panel --tailscale`.");
                }
                for (l, r) in &cfg.tailscale_users {
                    println!("{l:<40} {r}");
                }
            }
            TailscaleCmd::Off => {
                cfg.tailscale = false;
                if cfg.users.is_empty() {
                    cfg.enabled = false;
                }
                cfg.save(&path)?;
                if svc::saved_tailscale(home) {
                    let port = svc::saved_port(home).unwrap_or(kora_web::DEFAULT_PORT);
                    super::start_background_service(home, port, false)?;
                }
                println!("Tailscale access is off. The panel listens on this computer only. (The list of accounts is kept for next time.)");
            }
        }
        Ok(())
    };
    match run() {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// `kora panel --new-token`: a new sign-in token; restarts the background panel so the old one stops working.
pub fn new_token(home: &Path) -> i32 {
    if let Err(e) = svc::rotate_token(home) {
        eprintln!("error: {e}");
        return 1;
    }
    let platform = kora_core::service::Platform::current();
    if svc::service_installed(platform) {
        let port = svc::saved_port(home).unwrap_or(kora_web::DEFAULT_PORT);
        if let Err(e) = super::start_background_service(home, port, svc::saved_tailscale(home)) {
            eprintln!("error: the new token is saved, but the background panel could not be restarted: {e}");
            return 1;
        }
        println!("New sign-in token made and the background panel restarted: links shown before no longer work.");
        println!("Get a new link with:  kora panel --print-link");
    } else {
        println!("New sign-in token made. A panel running in a window keeps its old link until you restart it.");
    }
    0
}

/// Wait until something accepts connections at `addr`.
fn wait_for_port(addr: SocketAddr, timeout: Duration) -> bool {
    let start = std::time::Instant::now();
    loop {
        if std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok() {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

/// `ufw` rule that lets Tailscale devices reach the panel (only on the Tailscale interface).
pub fn ufw_rule(port: u16) -> Vec<String> {
    ["allow", "in", "on", "tailscale0", "to", "any", "port", &port.to_string(), "proto", "tcp", "comment", "Kora Panel over Tailscale"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// A host firewall can block Tailscale traffic to the panel. With `ufw` active, add a rule for the
/// Tailscale interface only (as root), or say which command to run. Other firewalls: a hint.
fn open_firewall(port: u16) {
    if cfg!(windows) {
        println!("If Windows Firewall asks, allow Kora Panel on private networks. Or, as administrator:");
        println!("  netsh advfirewall firewall add rule name=\"Kora Panel (Tailscale)\" dir=in action=allow protocol=TCP localport={port} remoteip=100.64.0.0/10");
        return;
    }
    let active = std::process::Command::new("ufw")
        .arg("status")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("Status: active"));
    if !active {
        return;
    }
    let rule = ufw_rule(port);
    if super::launch_uid_is_root() {
        let ok = std::process::Command::new("ufw").args(&rule).stdin(std::process::Stdio::null()).output().is_ok_and(|o| o.status.success());
        if ok {
            println!("The firewall (ufw) now lets Tailscale devices reach port {port} (on the Tailscale interface only).");
            return;
        }
    }
    let quoted: Vec<String> = rule.iter().map(|a| if a.contains(' ') { format!("'{a}'") } else { a.clone() }).collect();
    println!("The firewall (ufw) is on. If the page does not load from your devices, run:  sudo ufw {}", quoted.join(" "));
}

/// Ask a question on the terminal, also when stdin is a pipe (`curl | sh`). `None` when there is no terminal.
fn ask_line(question: &str) -> Option<String> {
    use std::io::{IsTerminal, Write};
    #[cfg(unix)]
    {
        use std::io::BufRead;
        if let Ok(tty) = std::fs::OpenOptions::new().read(true).write(true).open("/dev/tty") {
            let mut w = tty.try_clone().ok()?;
            let _ = write!(w, "{question}");
            let _ = w.flush();
            let mut line = String::new();
            std::io::BufReader::new(tty).read_line(&mut line).ok()?;
            return Some(line);
        }
    }
    if !std::io::stdin().is_terminal() {
        return None;
    }
    print!("{question}");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).ok()?;
    Some(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeTs(Option<&'static str>);
    impl Whois for FakeTs {
        fn whois(&self, _peer: SocketAddr) -> Result<String, String> {
            self.0.map(str::to_string).ok_or_else(|| "unknown".to_string())
        }
    }

    fn status(owner: Option<&str>) -> TsSelf {
        TsSelf { running: true, ips: vec!["100.64.0.2".parse().unwrap()], owner: owner.map(str::to_string), ..Default::default() }
    }

    #[test]
    fn logins_are_checked_and_lower_cased() {
        assert_eq!(normalise_login(" Alice@Example.com ").unwrap(), "alice@example.com");
        assert_eq!(normalise_login("octo@github").unwrap(), "octo@github");
        for bad in ["", "alice", "@x", "a@", "a b@x", "a@x/y", "\"a\"@x"] {
            assert!(normalise_login(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_ssh_client_counts_only_from_the_tailnet() {
        assert_eq!(ssh_client_on_tailnet(Some("100.101.1.2 50000 100.64.0.2 22")), Some("100.101.1.2:50000".parse().unwrap()));
        assert_eq!(ssh_client_on_tailnet(Some("fd7a:115c:a1e0::3 50000 x 22")), Some("[fd7a:115c:a1e0::3]:50000".parse().unwrap()));
        assert_eq!(ssh_client_on_tailnet(Some("203.0.113.9 50000 198.51.100.7 22")), None, "a public SSH client is nobody's Tailscale login");
        assert_eq!(ssh_client_on_tailnet(None), None);
        assert_eq!(ssh_client_on_tailnet(Some("garbage")), None);
    }

    #[test]
    fn the_admin_is_the_flag_then_the_ssh_account_then_the_owner() {
        let cfg = RemoteConfig::default();
        let ssh = Some("100.101.1.2:50000".parse().unwrap());
        let ts = FakeTs(Some("Sam@Example.com"));
        let pick = |flag, ssh, ts: &dyn Whois, st: &TsSelf, cfg: &RemoteConfig| choose_admin(flag, ssh, ts, st, cfg).unwrap();
        assert_eq!(pick(Some("Boss@x.io"), ssh, &ts, &status(Some("o@x")), &cfg), Some(("boss@x.io".into(), AdminSource::Flag)));
        assert_eq!(pick(None, ssh, &ts, &status(Some("o@x")), &cfg), Some(("sam@example.com".into(), AdminSource::SshSession)));
        assert_eq!(pick(None, ssh, &FakeTs(None), &status(Some("o@x")), &cfg), Some(("o@x".into(), AdminSource::Owner)), "whois failed");
        assert_eq!(pick(None, None, &ts, &status(Some("o@x")), &cfg), Some(("o@x".into(), AdminSource::Owner)));
        // A tagged computer reached without Tailscale: nobody to pick, unless an admin exists already.
        assert_eq!(pick(None, None, &ts, &status(None), &cfg), None);
        let mut with_admin = RemoteConfig::default();
        with_admin.tailscale_users.insert("a@x".into(), Role::Admin);
        assert_eq!(pick(None, None, &ts, &status(None), &with_admin), Some(("a@x".into(), AdminSource::Existing)));
        assert!(choose_admin(Some("not a login"), None, &ts, &status(None), &cfg).is_err());
    }

    #[test]
    fn enabling_keeps_other_accounts_and_makes_the_admin_an_admin() {
        let mut cfg = RemoteConfig::default();
        cfg.tailscale_users.insert("friend@x".into(), Role::Viewer);
        cfg.tailscale_users.insert("me@x".into(), Role::Operator);
        enable(&mut cfg, "me@x");
        assert!(cfg.enabled && cfg.tailscale);
        assert_eq!(cfg.tailscale_users.get("me@x"), Some(&Role::Admin));
        assert_eq!(cfg.tailscale_users.get("friend@x"), Some(&Role::Viewer), "others are left alone");
        assert!(!cfg.allow_remote_destructive, "destructive actions stay off for remote sessions");
    }

    #[test]
    fn the_ufw_rule_is_for_the_tailscale_interface_only() {
        assert_eq!(ufw_rule(47120).join(" "), "allow in on tailscale0 to any port 47120 proto tcp comment Kora Panel over Tailscale");
    }
}
