//! `kora backup create|list|verify|restore|undo|delete|prune|schedule <server>`: backups of one server
//! and putting them back. The same operations as the panel's Backups tab (`Op::Backup*`).
//!
//! A backup holds secrets (licence key, database login, txAdmin admin). The file is private (owner-only).
//! A password is never taken from the command line (it would show in the process list and the shell
//! history): give it on standard input with `--password-stdin`, or in `KORA_BACKUP_PASSWORD`.

use clap::{Args, Subcommand};
use kora_core::api::wire::BackupCreateRequest;
use kora_core::api::{Backend, SystemBackend};
use kora_core::backup::{BackupEvent, BackupInfo, BackupSchedule, Cadence};
use kora_core::installer::registry::{self, ServerEntry};
use kora_core::problem::plain_text;
use kora_core::space::human;

#[derive(Args, Debug, Clone, Default)]
pub(crate) struct PasswordArgs {
    /// Read the password from the first line of standard input.
    #[arg(long)]
    pub password_stdin: bool,
}

#[derive(Subcommand, Debug)]
pub(crate) enum BackupCmd {
    /// Make a backup of a server (database, settings, txAdmin data, resources).
    Create {
        /// Server name or folder.
        server: String,
        /// Protect the file with a password (see --password-stdin; KORA_BACKUP_PASSWORD also works).
        #[arg(long)]
        encrypt: bool,
        #[command(flatten)]
        pw: PasswordArgs,
        /// Keep every resource in full (a bigger file that needs no internet to restore).
        #[arg(long)]
        everything: bool,
        /// Stop the server while the backup runs (a strictly consistent copy), and start it again.
        #[arg(long)]
        stop_server: bool,
        /// Keep this backup in another folder.
        #[arg(long, value_name = "DIR")]
        folder: Option<String>,
        /// A copy to move the server to another computer: needs a password and is never removed by the retention rules.
        #[arg(long)]
        for_move: bool,
        /// Made by a schedule (what an operating-system task would run).
        #[arg(long, hide = true)]
        scheduled: bool,
    },
    /// List the backups of a server.
    List {
        /// Server name or folder.
        server: String,
    },
    /// Check every file of a backup against its checksums.
    Verify {
        server: String,
        /// The backup's number (see `kora backup list`), or `latest`.
        id: String,
        #[command(flatten)]
        pw: PasswordArgs,
    },
    /// Put a backup back on the server (a safety copy of the server as it is now is made first).
    Restore {
        server: String,
        /// The backup's number, or `latest`.
        id: String,
        #[command(flatten)]
        pw: PasswordArgs,
        /// Do not ask for confirmation.
        #[arg(long)]
        yes: bool,
    },
    /// Undo the last restore (puts the safety copy back).
    Undo {
        server: String,
        #[command(flatten)]
        pw: PasswordArgs,
        /// Do not ask for confirmation.
        #[arg(long)]
        yes: bool,
    },
    /// Delete one backup.
    Delete {
        server: String,
        id: String,
        /// Do not ask for confirmation.
        #[arg(long)]
        yes: bool,
    },
    /// Remove old backups under the retention rules.
    Prune { server: String },
    /// Show or change where backups go, how many are kept, and the schedule.
    Schedule {
        server: String,
        /// Every day at HH:MM (24 hours, this computer's time).
        #[arg(long, value_name = "HH:MM", conflicts_with_all = ["weekly", "off"])]
        daily: Option<String>,
        /// Every week on DAY (mon..sun) at HH:MM.
        #[arg(long, num_args = 2, value_names = ["DAY", "HH:MM"], conflicts_with_all = ["daily", "off"])]
        weekly: Option<Vec<String>>,
        /// Turn scheduled backups off.
        #[arg(long)]
        off: bool,
        /// Keep this many of the newest backups (0 = no limit).
        #[arg(long, value_name = "N")]
        keep_last: Option<u32>,
        /// Also keep the newest backup of each day for this many days (0 = off).
        #[arg(long, value_name = "DAYS")]
        keep_days: Option<u32>,
        /// Keep backups in this folder (outside the server folder).
        #[arg(long, value_name = "DIR", conflicts_with = "default_folder")]
        folder: Option<String>,
        /// Go back to Kora's own backups folder.
        #[arg(long)]
        default_folder: bool,
        /// Stop the server during scheduled backups.
        #[arg(long, value_parser = ["on", "off"])]
        stop: Option<String>,
    },
}

fn fail(m: String) -> i32 {
    eprintln!("error: {m}");
    1
}

fn find(server: &str) -> Result<ServerEntry, String> {
    let reg = registry::Registry::load().map_err(|e| format!("cannot read the server registry: {e}"))?;
    reg.find(server).cloned().ok_or_else(|| format!("no registered server named '{server}' (see `kora status`)"))
}

/// The password, from standard input or the environment (never the command line).
fn password(pw: &PasswordArgs, required: bool) -> Result<Option<String>, String> {
    if pw.password_stdin {
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).map_err(|e| format!("cannot read the password: {e}"))?;
        let p = line.trim_end_matches(['\r', '\n']).to_string();
        return if p.is_empty() { Err("the password on standard input is empty".into()) } else { Ok(Some(p)) };
    }
    if let Some(p) = std::env::var("KORA_BACKUP_PASSWORD").ok().filter(|p| !p.is_empty()) {
        return Ok(Some(p));
    }
    if required {
        return Err("this needs a password: give it with --password-stdin (or in KORA_BACKUP_PASSWORD). Kora never takes it as an argument: it would show in the process list.".into());
    }
    Ok(None)
}

/// A backup number or `latest`.
fn resolve(b: &mut SystemBackend, e: &ServerEntry, id: &str) -> Result<BackupInfo, String> {
    let view = b.backups_view(e).map_err(|m| plain_text(&m))?;
    if id == "latest" {
        return view.backups.into_iter().next().ok_or_else(|| "this server has no backups yet".into());
    }
    view.backups.into_iter().find(|x| x.id == id).ok_or_else(|| format!("there is no backup numbered {id} (see `kora backup list {}`)", e.name))
}

fn show(e: &BackupEvent) {
    println!("  [{:>3}%] {}", e.percent, e.message);
}

fn describe(b: &BackupInfo) -> String {
    let when = kora_core::backup::clock::utc_text(b.created_unix);
    format!(
        "{}  {}  {}  {}{}{}  {}",
        b.id,
        when,
        human(b.bytes),
        if b.encrypted { "password-protected" } else { "not protected" },
        if b.stopped { ", server stopped" } else { "" },
        if b.too_new { ", made by a newer Kora (cannot be restored here)" } else { "" },
        b.label.text()
    )
}

pub(crate) fn run(cmd: BackupCmd) -> i32 {
    let mut b = SystemBackend;
    match cmd {
        BackupCmd::Create { server, encrypt, pw, everything, stop_server, folder, for_move, scheduled } => {
            let e = match find(&server) {
                Ok(e) => e,
                Err(m) => return fail(m),
            };
            if scheduled {
                // What an operating-system task runs: the same as the panel's schedule does.
                let (db, git) = (kora_core::db::SystemRunner, kora_core::git::SystemRunner);
                let tools = kora_core::backup::Tools::system(&db, &git);
                let mut host = kora_core::backup::host::BackendHost::new(&mut b, &e);
                return match kora_core::backup::scheduler::run_scheduled(&e, &mut host, &tools) {
                    kora_core::backup::scheduler::Outcome::Made(id) => {
                        println!("Backup {id} made.");
                        0
                    }
                    kora_core::backup::scheduler::Outcome::Failed(m) => fail(m),
                    kora_core::backup::scheduler::Outcome::NotDue => 0,
                };
            }
            let passphrase = match password(&pw, encrypt || for_move) {
                Ok(p) => p,
                Err(m) => return fail(m),
            };
            println!(
                "Backing up '{}'. The file will hold your licence key, database login and txAdmin admin{}.",
                e.name,
                if passphrase.is_some() { ", locked with your password" } else { "; it is private to your account but not password-protected" }
            );
            let req = BackupCreateRequest { passphrase, everything, stop_server, folder, for_move };
            match b.backup_create(&e, &req, true, &mut |ev| show(&ev)) {
                Ok(r) => {
                    println!("Done: {}", r.path);
                    println!("  {} on disk; {} resources by reference, {} in full.", human(r.info.bytes), r.info.by_reference, r.info.in_full);
                    r.notes.iter().for_each(|n| println!("  Note: {n}"));
                    if r.info.encrypted {
                        println!("  Keep the password safe: it cannot be recovered if it is lost.");
                    }
                    0
                }
                Err(m) => fail(plain_text(&m)),
            }
        }
        BackupCmd::List { server } => {
            let e = match find(&server) {
                Ok(e) => e,
                Err(m) => return fail(m),
            };
            match b.backups_view(&e) {
                Ok(v) => {
                    println!("{}. Backups are kept in {}.", v.summary.text, v.folder);
                    if let Some(w) = &v.summary.warning {
                        println!("Warning: {w}");
                    }
                    if v.backups.is_empty() {
                        println!("Make one with: kora backup create {}", e.name);
                    }
                    v.backups.iter().for_each(|x| println!("  {}", describe(x)));
                    if let Some(u) = v.undo {
                        println!("The last restore can be undone: kora backup undo {} (safety copy {}).", e.name, u.id);
                    }
                    0
                }
                Err(m) => fail(plain_text(&m)),
            }
        }
        BackupCmd::Verify { server, id, pw } => {
            let e = match find(&server) {
                Ok(e) => e,
                Err(m) => return fail(m),
            };
            let info = match resolve(&mut b, &e, &id) {
                Ok(i) => i,
                Err(m) => return fail(m),
            };
            let passphrase = match password(&pw, info.encrypted) {
                Ok(p) => p,
                Err(m) => return fail(m),
            };
            match b.backup_verify(&e, &info.id, passphrase.as_deref(), &mut |ev| show(&ev)) {
                Ok(v) => {
                    println!("{}", v.summary);
                    0
                }
                Err(m) => fail(plain_text(&m)),
            }
        }
        BackupCmd::Restore { server, id, pw, yes } => {
            let e = match find(&server) {
                Ok(e) => e,
                Err(m) => return fail(m),
            };
            let info = match resolve(&mut b, &e, &id) {
                Ok(i) => i,
                Err(m) => return fail(m),
            };
            let passphrase = match password(&pw, info.encrypted) {
                Ok(p) => p,
                Err(m) => return fail(m),
            };
            match b.backup_preview(&e, &info.id, passphrase.as_deref()) {
                Ok(p) => {
                    println!("Restoring backup {} of '{}' will:", info.id, e.name);
                    p.will_change.iter().for_each(|l| println!("  change: {l}"));
                    p.will_keep.iter().for_each(|l| println!("  keep:   {l}"));
                    p.warnings.iter().for_each(|l| println!("  note:   {l}"));
                }
                Err(m) => return fail(plain_text(&m)),
            }
            if !yes && !crate::ui::confirm("Restore this backup now?") {
                println!("Nothing was changed.");
                return 0;
            }
            match b.backup_restore(&e, &info.id, passphrase.as_deref(), &mut |ev| show(&ev)) {
                Ok(r) => {
                    println!("Restore finished. Database {}; {} files put back.", if r.database_restored { "restored" } else { "not touched" }, r.files_restored);
                    r.resources_restored.iter().for_each(|l| println!("  {l}"));
                    r.notes.iter().for_each(|l| println!("  Note: {l}"));
                    r.problems.iter().for_each(|l| println!("  Problem: {l}"));
                    if r.safety_backup.is_some() {
                        println!("To undo this restore: kora backup undo {}", e.name);
                    }
                    i32::from(!r.problems.is_empty())
                }
                Err(m) => fail(plain_text(&m)),
            }
        }
        BackupCmd::Undo { server, pw, yes } => {
            let e = match find(&server) {
                Ok(e) => e,
                Err(m) => return fail(m),
            };
            if !yes && !crate::ui::confirm("Put the server back as it was before the last restore?") {
                println!("Nothing was changed.");
                return 0;
            }
            let passphrase = password(&pw, false).unwrap_or(None);
            match b.backup_undo(&e, passphrase.as_deref(), &mut |ev| show(&ev)) {
                Ok(r) => {
                    println!("Undone. {} files put back.", r.files_restored);
                    r.problems.iter().for_each(|l| println!("  Problem: {l}"));
                    0
                }
                Err(m) => fail(plain_text(&m)),
            }
        }
        BackupCmd::Delete { server, id, yes } => {
            let e = match find(&server) {
                Ok(e) => e,
                Err(m) => return fail(m),
            };
            let info = match resolve(&mut b, &e, &id) {
                Ok(i) => i,
                Err(m) => return fail(m),
            };
            if !yes && !crate::ui::confirm(&format!("Delete backup {}? It cannot be brought back.", info.id)) {
                println!("Nothing was changed.");
                return 0;
            }
            match b.backup_delete(&e, &info.id) {
                Ok(()) => {
                    println!("Deleted backup {}.", info.id);
                    0
                }
                Err(m) => fail(plain_text(&m)),
            }
        }
        BackupCmd::Prune { server } => {
            let e = match find(&server) {
                Ok(e) => e,
                Err(m) => return fail(m),
            };
            match b.backup_prune(&e) {
                Ok(gone) if gone.is_empty() => {
                    println!("Nothing to remove under the current retention rules.");
                    0
                }
                Ok(gone) => {
                    println!("Removed {} old backup{}: {}", gone.len(), if gone.len() == 1 { "" } else { "s" }, gone.join(", "));
                    0
                }
                Err(m) => fail(plain_text(&m)),
            }
        }
        BackupCmd::Schedule { server, daily, weekly, off, keep_last, keep_days, folder, default_folder, stop } => {
            let e = match find(&server) {
                Ok(e) => e,
                Err(m) => return fail(m),
            };
            let mut s = kora_core::backup::settings::load_settings(&e.name);
            let before = s.clone();
            if let Some(t) = daily {
                s.schedule = BackupSchedule { enabled: true, cadence: Cadence::Daily, time: t, weekday: None };
            }
            if let Some(w) = weekly {
                let Some(d) = parse_day(&w[0]) else { return fail(format!("'{}' is not a day: use mon, tue, wed, thu, fri, sat or sun", w[0])) };
                s.schedule = BackupSchedule { enabled: true, cadence: Cadence::Weekly, time: w[1].clone(), weekday: Some(d) };
            }
            if off {
                s.schedule.enabled = false;
            }
            if let Some(n) = keep_last {
                s.retention.keep_last = n;
            }
            if let Some(n) = keep_days {
                s.retention.keep_daily_days = n;
            }
            if let Some(f) = folder {
                s.folder = f;
            }
            if default_folder {
                s.folder.clear();
            }
            if let Some(v) = stop {
                s.stop_for_scheduled = v == "on";
            }
            if s != before {
                match b.backup_settings_set(&e, &s) {
                    Ok(saved) => s = saved,
                    Err(m) => return fail(plain_text(&m)),
                }
                println!("Saved.");
            }
            println!("Folder:    {}", if s.folder.is_empty() { "Kora's own backups folder".to_string() } else { s.folder.clone() });
            println!("Schedule:  {}", describe_schedule(&s.schedule));
            println!("Keeping:   {}", describe_retention(&s.retention));
            println!("Stop the server during scheduled backups: {}", if s.stop_for_scheduled { "yes" } else { "no" });
            if s.schedule.enabled {
                println!("Scheduled backups run while Kora's background service runs (kora panel --background).");
            }
            0
        }
    }
}

fn describe_retention(r: &kora_core::backup::Retention) -> String {
    match (r.keep_last, r.keep_daily_days) {
        (0, 0) => "everything (nothing is removed)".into(),
        (n, 0) => format!("the newest {n}"),
        (0, d) => format!("one per day for {d} days"),
        (n, d) => format!("the newest {n}, and one per day for {d} days"),
    }
}

fn describe_schedule(s: &BackupSchedule) -> String {
    if !s.enabled {
        return "off".into();
    }
    match (s.cadence, s.weekday) {
        (Cadence::Weekly, Some(d)) => format!("every {} at {}", ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"][usize::from(d.min(6))], s.time),
        _ => format!("every day at {}", s.time),
    }
}

/// `mon`..`sun` (or the full name) → 0..=6.
pub(crate) fn parse_day(s: &str) -> Option<u8> {
    let l = s.trim().to_ascii_lowercase();
    ["mon", "tue", "wed", "thu", "fri", "sat", "sun"].iter().position(|d| l.starts_with(d)).map(|i| i as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_and_words() {
        assert_eq!(parse_day("Mon"), Some(0));
        assert_eq!(parse_day("sunday"), Some(6));
        assert_eq!(parse_day("xyz"), None);
        assert_eq!(describe_retention(&kora_core::backup::Retention { keep_last: 7, keep_daily_days: 0 }), "the newest 7");
        assert_eq!(describe_retention(&kora_core::backup::Retention { keep_last: 0, keep_daily_days: 0 }), "everything (nothing is removed)");
        let s = BackupSchedule { enabled: true, cadence: Cadence::Weekly, time: "04:00".into(), weekday: Some(6) };
        assert_eq!(describe_schedule(&s), "every Sunday at 04:00");
        assert_eq!(describe_schedule(&BackupSchedule::default()), "off");
    }

    #[test]
    fn the_password_is_never_an_argument() {
        // there is no flag that takes one: only --password-stdin and the environment
        use clap::CommandFactory;
        #[derive(clap::Parser)]
        struct T {
            #[command(subcommand)]
            c: BackupCmd,
        }
        let cmd = T::command();
        let create = cmd.find_subcommand("create").unwrap();
        assert!(create.get_arguments().all(|a| !a.get_id().as_str().contains("passphrase") && a.get_id().as_str() != "password"));
    }
}
