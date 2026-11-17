//! `kora schedule`: when a development server updates itself.

use std::path::PathBuf;

use clap::Subcommand;
use kora_core::installer::{self, registry};

#[derive(Subcommand, Debug)]
pub(crate) enum ScheduleCmd {
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

/// `kora schedule show|set|off`.
pub(crate) fn schedule(cmd: ScheduleCmd) -> i32 {
    use kora_core::service as svc;
    // Working for Kora Panel's person in the elevated worker: only servers on their list, in a
    // folder that is theirs, and never another Windows account's update task.
    let for_panel = installer::system_ops::panel_user().is_some();
    let find = |server: &str| -> Option<registry::ServerEntry> {
        let reg = registry::Registry::load().unwrap_or_default();
        if for_panel {
            return reg.find(server).cloned();
        }
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

