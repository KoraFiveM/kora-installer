//! `kora service ...`: Kora's administrator service (see `kora_core::privsvc`).
//!
//! * `kora service status`: is it installed, running, and does it serve this account?
//! * `kora service list|allow|deny`: who may use it (administrators only; relaunched with sudo / the
//!   Windows permission prompt like other privileged commands).
//! * `kora service serve`: the service itself (started by `kora.service` / the Windows service; not for people).

use clap::Subcommand;
use kora_core::privsvc::manage;
#[cfg(unix)]
use kora_core::privsvc::paths;

#[derive(Subcommand, Debug)]
pub enum ServiceCmd {
    /// Run the service (started by the system; not for people).
    #[command(hide = true)]
    Serve,
    /// Say whether the service is installed and running, and whether it serves your account.
    Status,
    /// List the accounts that may use the service.
    List,
    /// Let accounts use the service (administrators only). Give the accounts' names.
    Allow {
        /// Account names (Windows: DOMAIN\name or a SID; Linux: the user name or uid:1000).
        #[arg(required = true)]
        accounts: Vec<String>,
    },
    /// Install the service on this computer (needs administrator rights; asks once). Linux: downloads this
    /// version's program from the official release, checks its checksum and puts it in /usr/lib/kora.
    /// For Debian and Ubuntu the `.deb` package does the same.
    Install {
        /// Also let this account use the service (the account that ran sudo is always added).
        #[arg(long = "allow", value_name = "ACCOUNT")]
        allow: Vec<String>,
    },
    /// Remove the service (needs administrator rights). Servers it set up keep working.
    Uninstall {
        /// Also remove the list of allowed accounts, the ownership records and the audit log.
        #[arg(long)]
        purge: bool,
    },
    /// Print the systemd unit text (the package is built from it; not for people).
    #[command(hide = true)]
    Unit,
    /// Stop an account using the service (administrators only). Servers it set up keep working.
    Deny { account: String },
}

/// Where the settings file is on this system.
#[cfg(unix)]
fn policy_path() -> std::path::PathBuf {
    paths::LINUX_POLICY.into()
}
#[cfg(windows)]
fn policy_path() -> std::path::PathBuf {
    kora_core::privsvc::winsvc::policy_path()
}

pub fn service(cmd: ServiceCmd) -> i32 {
    let fail = |m: String| {
        eprintln!("error: {m}");
        1
    };
    match cmd {
        ServiceCmd::Serve => {
            #[cfg(unix)]
            let r = kora_core::privsvc::unixsvc::run_service();
            #[cfg(windows)]
            let r = kora_core::privsvc::winsvc::run_service();
            match r {
                Ok(()) => 0,
                Err(e) => fail(e),
            }
        }
        ServiceCmd::Status => {
            #[cfg(unix)]
            {
                println!("installed: {}", if std::path::Path::new(policy_path().as_path()).exists() || std::path::Path::new("/lib/systemd/system/kora.service").exists() { "yes" } else { "no" });
                match kora_core::privsvc::unixsvc::probe(std::path::Path::new(paths::LINUX_SOCKET)) {
                    Ok(v) => {
                        println!("running: yes (version {v})");
                        println!("serves this account: yes");
                    }
                    Err(kora_core::privsvc::unixsvc::ClientError::Refused(why)) => {
                        println!("running: yes");
                        println!("serves this account: no ({why})");
                    }
                    Err(kora_core::privsvc::unixsvc::ClientError::Unavailable(why) | kora_core::privsvc::unixsvc::ClientError::Broken(why)) => {
                        println!("running: no ({why})");
                    }
                }
            }
            #[cfg(windows)]
            {
                use kora_core::privsvc::conn::ClientError;
                println!("installed: {}", if policy_path().exists() { "yes" } else { "no" });
                match kora_core::privsvc::winsvc::probe() {
                    Ok(v) => {
                        println!("running: yes (version {v})");
                        println!("serves this account: yes");
                    }
                    Err(ClientError::Refused(why)) => {
                        println!("running: yes");
                        println!("serves this account: no ({why})");
                    }
                    Err(ClientError::Unavailable(why) | ClientError::Broken(why)) => println!("running: no ({why})"),
                }
            }
            0
        }
        ServiceCmd::List => match manage::list(&policy_path()) {
            Ok(v) if v.is_empty() => {
                println!("No account may use Kora's administrator service yet. Add one with `kora service allow <account>`.");
                0
            }
            Ok(v) => {
                for l in v {
                    println!("{l}");
                }
                0
            }
            Err(e) => fail(e),
        },
        ServiceCmd::Unit => {
            print!("{}", kora_core::privsvc::install::unit_text());
            0
        }
        ServiceCmd::Install { allow } => install(&allow),
        ServiceCmd::Uninstall { purge } => apply_plan_for(kora_core::privsvc::install::uninstall_plan(purge), "Kora's administrator service is removed. Servers it set up keep working."),
        ServiceCmd::Allow { accounts } => {
            let mut code = 0;
            // empty names are skipped: the installer passes optional properties through
            for a in accounts.iter().filter(|a| !a.trim().is_empty()) {
                match manage::allow(&policy_path(), a) {
                    Ok(m) => println!("{m}"),
                    Err(e) => code = fail(e),
                }
            }
            code
        }
        ServiceCmd::Deny { account } => match manage::deny(&policy_path(), &account) {
            Ok(m) => {
                println!("{m}");
                0
            }
            Err(e) => fail(e),
        },
    }
}

fn apply_plan_for(plan: kora_core::service::Plan, done: &str) -> i32 {
    #[cfg(unix)]
    {
        for d in plan.descriptions() {
            println!("- {d}");
        }
        match kora_core::service::apply_plan(&plan, &kora_core::service::SystemRunner) {
            Ok(()) => {
                println!("{done}");
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (plan, done);
        eprintln!("error: on Windows remove Kora in Settings > Apps > Installed apps (its uninstaller removes the service)");
        1
    }
}

/// Linux: program from the verified release, folders, unit, settings, start. Same layout as the `.deb`.
#[cfg(unix)]
fn install(allow: &[String]) -> i32 {
    use kora_core::privsvc::install as inst;
    use kora_core::self_update::{self, release, replace};
    let fail = |m: String| {
        eprintln!("error: {m}");
        1
    };
    if !kora_core::elevate::is_elevated() {
        return fail("installing the service needs administrator rights: run `sudo kora service install`".into());
    }
    let mut accounts: Vec<String> = allow.to_vec();
    if let Some(uid) = inst::enrolling_uid(std::env::var("SUDO_UID").ok().as_deref(), std::env::var("PKEXEC_UID").ok().as_deref()) {
        accounts.push(format!("uid:{uid}"));
    }
    let (before, after) = inst::install_plan();
    for d in before.descriptions() {
        println!("- {d}");
    }
    if let Err(e) = kora_core::service::apply_plan(&before, &kora_core::service::SystemRunner) {
        return fail(e.to_string());
    }
    // The program: the verified release of THIS version (never a file from anywhere else), unless this
    // already is the installed program.
    let running = std::env::current_exe().ok();
    if running.as_deref() != Some(std::path::Path::new(inst::PROGRAM)) {
        println!("- Download Kora {} from the official release and check its checksum", env!("CARGO_PKG_VERSION"));
        let src = self_update::Source::from_env();
        let rels = match self_update::fetch_releases(&src) {
            Ok(r) => r,
            Err(e) => return fail(e.to_string()),
        };
        let Some(rel) = release::pick_tag(&rels, env!("CARGO_PKG_VERSION")) else {
            return fail(format!("there is no published release {} to install the service from (this may be a development build)", env!("CARGO_PKG_VERSION")));
        };
        let Some(asset) = release::this_asset_name() else { return fail("there is no published build of Kora for this system".into()) };
        // The target must not exist as a link; replace() writes next to it in the root-owned folder.
        if let Err(e) = self_update::apply(&src, rel, asset, std::path::Path::new(inst::PROGRAM), &replace::RealFs, replace::Flow::Unix) {
            return fail(e.to_string());
        }
    }
    for a in &accounts {
        match manage::allow(&policy_path(), a) {
            Ok(m) => println!("- {m}"),
            Err(e) => eprintln!("note: {e}"),
        }
    }
    if accounts.is_empty() {
        println!("- No account may use the service yet: run `sudo kora service allow <your account>`.");
    }
    for d in after.descriptions() {
        println!("- {d}");
    }
    if let Err(e) = kora_core::service::apply_plan(&after, &kora_core::service::SystemRunner) {
        return fail(e.to_string());
    }
    println!("Kora's administrator service is installed and running. Kora Panel can now do the administrator parts of installs by itself.");
    0
}

/// Windows: the checked `.msi` from the release, run with the one permission prompt.
#[cfg(windows)]
fn install(allow: &[String]) -> i32 {
    match kora_core::privsvc::wininstall::install(allow, &mut |l| println!("{l}")) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_service_commands_are_classified_so_changes_ask_for_administrator_rights() {
        use clap::Parser;
        let kind = |a: &[&str]| crate::command_kind(&crate::Cli::try_parse_from(std::iter::once("kora").chain(a.iter().copied())).unwrap().command);
        assert_eq!(kind(&["service", "status"]), kora_core::elevate::CmdKind::Other);
        assert_eq!(kind(&["service", "serve"]), kora_core::elevate::CmdKind::Other);
        assert_eq!(kind(&["service", "allow", "jo"]), kora_core::elevate::CmdKind::Uninstall { dry_run: false });
        assert_eq!(kind(&["service", "deny", "jo"]), kora_core::elevate::CmdKind::Uninstall { dry_run: false });
        // sudo on Linux; on Windows the install has its own single prompt and removal is Apps & features
        let own = if cfg!(windows) { kora_core::elevate::CmdKind::Other } else { kora_core::elevate::CmdKind::Uninstall { dry_run: false } };
        assert_eq!(kind(&["service", "install"]), own);
        assert_eq!(kind(&["service", "uninstall", "--purge"]), own);
        assert_eq!(kind(&["service", "unit"]), kora_core::elevate::CmdKind::Other);
    }
}
