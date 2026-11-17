//! `kora service ...`: Kora's administrator service (see `kora_core::privsvc`).
//!
//! * `kora service status`: is it installed, running, and does it serve this account?
//! * `kora service list|allow|deny`: who may use it (administrators only; relaunched with sudo / the
//!   Windows permission prompt like other privileged commands).
//! * `kora service serve`: the service itself (started by `kora.service` / the Windows service; not for people).

use clap::Subcommand;
use kora_core::privsvc::{manage, paths};

#[derive(Subcommand, Debug)]
pub enum ServiceCmd {
    /// Run the service (started by the system; not for people).
    #[command(hide = true)]
    Serve,
    /// Say whether the service is installed and running, and whether it serves your account.
    Status,
    /// List the accounts that may use the service.
    List,
    /// Let an account use the service (administrators only). Give the account's name.
    Allow {
        /// Account name (Windows: DOMAIN\name or a SID; Linux: the user name or uid:1000).
        account: String,
    },
    /// Stop an account using the service (administrators only). Servers it set up keep working.
    Deny { account: String },
}

/// Where the settings file is on this system.
#[cfg(unix)]
fn policy_path() -> std::path::PathBuf {
    paths::LINUX_POLICY.into()
}
#[cfg(not(unix))]
fn policy_path() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".into())).join("Kora").join(paths::WINDOWS_DATA_SUBDIR).join("service.json")
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
            #[cfg(not(unix))]
            let r: Result<(), String> = Err("the Windows service is not wired up in this build".into());
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
            #[cfg(not(unix))]
            println!("installed: no");
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
        ServiceCmd::Allow { account } => match manage::allow(&policy_path(), &account) {
            Ok(m) => {
                println!("{m}");
                0
            }
            Err(e) => fail(e),
        },
        ServiceCmd::Deny { account } => match manage::deny(&policy_path(), &account) {
            Ok(m) => {
                println!("{m}");
                0
            }
            Err(e) => fail(e),
        },
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
    }
}
