//! Two commands of the administrator service's follow-up operations (see `kora_core::privsvc::op`):
//!
//! * `kora backup-job set|off <folder>` (hidden): create, replace or remove the operating-system job that runs a server's
//!   scheduled backups (`kora_core::service::backup_job`). Run by the service as root / SYSTEM with the calling person's
//!   account in its environment; never by a person. It opens no store: it works from the server folder's own record.
//! * `kora hand-over <folder>`: give the files in a server's folder to the account the server runs as, after a restore or an
//!   import wrote them as you (`kora_core::backup::handover`). Needs administrator rights: it asks Kora's administrator
//!   service, or does it itself when run as administrator / root.

use std::path::PathBuf;

use clap::{Args, Subcommand};
use kora_core::installer;
use kora_core::service::backup_job;

#[derive(Subcommand, Debug)]
pub(crate) enum BackupJobCmd {
    /// Create or replace the job.
    Set {
        /// The server's full folder.
        dir: PathBuf,
        /// Every day at HH:MM (24 hours, this computer's time).
        #[arg(long, value_name = "HH:MM", conflicts_with = "weekly", required_unless_present = "weekly")]
        daily: Option<String>,
        /// Every week on DAY (mon..sun) at HH:MM.
        #[arg(long, num_args = 2, value_names = ["DAY", "HH:MM"], conflicts_with = "daily")]
        weekly: Option<Vec<String>>,
    },
    /// Remove the job (nothing else).
    Off {
        /// The server's full folder.
        dir: PathBuf,
    },
}

#[derive(Args, Debug)]
pub(crate) struct HandOverArgs {
    /// The server's full folder.
    pub dir: PathBuf,
}

fn fail(m: impl std::fmt::Display) -> i32 {
    eprintln!("error: {m}");
    1
}

/// The server's short name from the folder's own install record (the person's server list is never opened here).
fn slug_of(dir: &std::path::Path) -> Result<String, String> {
    installer::load_request_from(dir).map(|r| installer::slug(&r.name)).ok_or_else(|| format!("{} has no Kora install record", dir.display()))
}

pub(crate) fn backup_job(cmd: BackupJobCmd) -> i32 {
    if !kora_core::elevate::is_elevated() {
        return fail(
            "this changes the operating system's scheduled jobs, which needs administrator rights. It is done for you by Kora's administrator service (`kora backup schedule <server> --daily 03:30`).",
        );
    }
    match cmd {
        BackupJobCmd::Set { dir, daily, weekly } => {
            let slug = match slug_of(&dir) {
                Ok(s) => s,
                Err(e) => return fail(e),
            };
            let (weekday, time) = match (daily, weekly) {
                (Some(t), None) => (None, t),
                (None, Some(w)) => match crate::cmd::backup::parse_day(&w[0]) {
                    Some(d) => (Some(d), w[1].clone()),
                    None => return fail(format!("'{}' is not a day: use mon, tue, wed, thu, fri, sat or sun", w[0])),
                },
                _ => return fail("say when: --daily HH:MM or --weekly DAY HH:MM"),
            };
            let schedule = match backup_job::schedule_from(weekday, &time) {
                Ok(s) => s,
                Err(e) => return fail(e),
            };
            let account = kora_core::service::installing_user();
            match backup_job::set(&slug, &dir, &schedule, &account) {
                Ok(()) => {
                    println!("Scheduled backups of {slug}: {}, as {account}.", schedule.describe());
                    0
                }
                Err(e) => fail(e),
            }
        }
        BackupJobCmd::Off { dir } => {
            let slug = match slug_of(&dir) {
                Ok(s) => s,
                Err(e) => return fail(e),
            };
            match backup_job::remove(&slug) {
                Ok(()) => {
                    println!("Scheduled backups of {slug} are off in the operating system.");
                    0
                }
                Err(e) => fail(e),
            }
        }
    }
}

pub(crate) fn hand_over(a: HandOverArgs) -> i32 {
    use kora_core::backup::handover::{after_write, needed, HandOver};
    if !needed(&a.dir) {
        println!("{}: this server does not run under an account of its own, so there is nothing to hand over.", a.dir.display());
        return 0;
    }
    match after_write(&a.dir) {
        HandOver::NotNeeded => {
            println!("Nothing to hand over.");
            0
        }
        HandOver::Done(m) => {
            println!("{m}");
            0
        }
        HandOver::Failed(m) => fail(m),
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    fn kind(a: &[&str]) -> kora_core::elevate::CmdKind {
        crate::command_kind(&crate::Cli::try_parse_from(std::iter::once("kora").chain(a.iter().copied())).unwrap().command)
    }

    #[test]
    fn neither_asks_for_administrator_rights_by_relaunching() {
        assert_eq!(kind(&["backup-job", "off", "/srv/a"]), kora_core::elevate::CmdKind::Other);
        assert_eq!(kind(&["hand-over", "/srv/a"]), kora_core::elevate::CmdKind::Other);
    }

    #[test]
    fn the_job_needs_a_time() {
        assert!(crate::Cli::try_parse_from(["kora", "backup-job", "set", "/srv/a"]).is_err());
        assert!(crate::Cli::try_parse_from(["kora", "backup-job", "set", "/srv/a", "--daily", "03:30"]).is_ok());
        assert!(crate::Cli::try_parse_from(["kora", "backup-job", "set", "/srv/a", "--weekly", "sun", "03:30"]).is_ok());
        assert!(crate::Cli::try_parse_from(["kora", "backup-job", "set", "/srv/a", "--daily", "03:30", "--weekly", "sun", "03:30"]).is_err());
    }

    #[test]
    fn without_administrator_rights_it_refuses_and_changes_nothing() {
        if kora_core::elevate::is_elevated() {
            return;
        }
        assert_eq!(super::backup_job(super::BackupJobCmd::Off { dir: "/nonexistent".into() }), 1);
    }
}
