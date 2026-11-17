//! `kora audit` (who did what) and `kora store ...` (Kora's own data file: status, backup, the way back to
//! the old JSON files), and the audit row every state-changing `kora` command leaves.
//!
//! The audit rows of the CLI are written from the parsed command line ([`audit_start`], [`CliAudit::finish`]):
//! the command's name path (`keys add`) says which operation it is, a `<server>` argument says which server;
//! nothing else from the command line is kept (no values, so no key can leak). Reads, dry runs and
//! `--preview`-like options leave no row. A failing audit never stops a command.

use std::path::PathBuf;

use clap::{Args, CommandFactory, Subcommand};
use kora_core::api::Op;
use kora_core::store::audit::{self, actor, result, New, Query};
use kora_core::store::{self, backup, export, Store};

#[derive(Args)]
pub struct AuditArgs {
    /// Only this server.
    #[arg(long)]
    pub server: Option<String>,
    /// Only this operation (for example server_control, key_add, update_apply).
    #[arg(long)]
    pub op: Option<String>,
    /// Only whose name contains this (a Tailscale login, a user name, an account name).
    #[arg(long)]
    pub actor: Option<String>,
    /// Only this result: ok, failed, refused or started.
    #[arg(long)]
    pub result: Option<String>,
    /// Only the last this many days.
    #[arg(long)]
    pub days: Option<u64>,
    /// How many rows to show (the newest first).
    #[arg(long, default_value_t = 50)]
    pub limit: usize,
    /// Print JSON, one row per line.
    #[arg(long)]
    pub json: bool,
}

#[derive(Subcommand)]
pub enum StoreCmd {
    /// Show where Kora's data file is, its version, what is in it and what backups there are.
    Status,
    /// Make a consistent copy of Kora's data file (saved licence keys included: keep it private).
    Backup {
        /// Where to put the copy (default: the store's backups folder).
        #[arg(long)]
        to: Option<PathBuf>,
    },
    /// Write the old JSON files (the way back to Kora 0.7): everything changed since the upgrade comes with you.
    ExportJson {
        /// The folder to write them in (default: inside the private store folder).
        #[arg(long)]
        to: Option<PathBuf>,
    },
}

fn say(p: kora_core::problem::Problem) -> i32 {
    let s: String = p.into();
    eprintln!("error: {}", kora_core::problem::plain_text(&s));
    1
}

fn when(at: u64) -> String {
    // Seconds since the epoch as `YYYY-MM-DD HH:MM:SS` (UTC), without a date library.
    let days = (at / 86_400) as i64;
    let secs = at % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}", secs / 3600, secs % 3600 / 60, secs % 60)
}

/// `kora audit`.
pub fn audit(a: AuditArgs) -> i32 {
    let s = match Store::open_home() {
        Ok(s) => s,
        Err(p) => return say(p),
    };
    let since = a.days.map(|d| kora_core::monitor::now_unix().saturating_sub(d * 86_400));
    let rows = match audit::list(&s, &Query { limit: a.limit, server: a.server, op: a.op, actor: a.actor, result: a.result, since, ..Default::default() }) {
        Ok(r) => r,
        Err(p) => return say(p),
    };
    if a.json {
        for r in &rows {
            println!("{}", serde_json::to_string(r).unwrap_or_default());
        }
        return 0;
    }
    if rows.is_empty() {
        println!("Nothing in the audit log yet (it records what is changed, from the panel and from `kora` commands).");
        return 0;
    }
    println!("{:<20} {:<9} {:<22} {:<16} WHO", "WHEN (UTC)", "RESULT", "WHAT", "SERVER");
    for r in rows {
        let who = match (r.actor_kind.as_str(), r.actor_name.as_deref(), r.provider.as_deref(), r.role.as_deref()) {
            ("remote", n, p, role) => format!("{} (remote, {}, {})", n.unwrap_or("unknown"), p.unwrap_or("?"), role.unwrap_or("?")),
            ("cli", n, _, _) => format!("{} (kora command)", n.unwrap_or("unknown")),
            (_, n, _, _) => format!("{} (this computer)", n.unwrap_or("unknown")),
        };
        println!("{:<20} {:<9} {:<22} {:<16} {}{}", when(r.at), r.result, r.op, r.server.unwrap_or_default(), who, r.detail.map(|d| format!("  [{d}]")).unwrap_or_default());
    }
    0
}

/// `kora store ...`.
pub fn store(cmd: StoreCmd) -> i32 {
    let home = kora_core::installer::registry::kora_home();
    match cmd {
        StoreCmd::Status => {
            let s = match Store::open(&home) {
                Ok(s) => s,
                Err(p) => return say(p),
            };
            println!("Kora's data file: {}", s.path().display());
            println!("Format version:   {} (this Kora understands up to {})", s.schema_version().unwrap_or(0), store::schema::latest());
            if let Ok(m) = std::fs::metadata(s.path()) {
                println!("Size:             {} KiB", m.len() / 1024);
            }
            let counts: Vec<(&str, i64)> = ["servers", "license_keys", "paused_installs", "hub_nodes", "remote_users", "jobs", "audit_log"]
                .iter()
                .map(|t| (*t, s.read(|c| Ok(c.query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get::<_, i64>(0))?)).unwrap_or(-1)))
                .collect();
            println!(
                "Holds:            {} servers, {} saved keys, {} stopped installs, {} nodes, {} remote users, {} jobs, {} audit rows",
                counts[0].1, counts[1].1, counts[2].1, counts[3].1, counts[4].1, counts[5].1, counts[6].1
            );
            let backups = backup::list(&home);
            println!("Backups:          {} in {}", backups.len(), backup::backups_dir(&home).display());
            let aside = store::legacy::aside_dir(&home);
            if aside.is_dir() {
                println!("Old files:        the JSON files Kora 0.7 used were moved to {}", aside.display());
            }
            0
        }
        StoreCmd::Backup { to } => {
            let s = match Store::open(&home) {
                Ok(s) => s,
                Err(p) => return say(p),
            };
            match backup::backup_now(&s, to.as_deref()) {
                Ok(p) => {
                    println!("Saved a copy of Kora's data file to {}.", p.display());
                    println!("It holds your saved licence keys: keep it as private as the original.");
                    0
                }
                Err(p) => say(p),
            }
        }
        StoreCmd::ExportJson { to } => {
            let s = match Store::open(&home) {
                Ok(s) => s,
                Err(p) => return say(p),
            };
            let dir = to.unwrap_or_else(|| export::default_dir(&home));
            match export::export_json(&s, &dir) {
                Ok(files) => {
                    println!("Wrote {} file(s) in the format Kora 0.7 uses to {}:", files.len(), dir.display());
                    for f in files {
                        println!("  {}", f.display());
                    }
                    println!("To go back to Kora 0.7: install it, then copy these files into {} (see docs/store.md). They hold your saved licence keys: keep them private.", home.display());
                    0
                }
                Err(p) => say(p),
            }
        }
    }
}

// ---- the audit row of a `kora` command ---------------------------------------------------------------

/// The operation a command path stands for, `None` for what only reads or only runs the panel itself.
fn op_of(path: &[&str]) -> Option<Op> {
    Some(match path {
        ["install"] => Op::InstallStart,
        ["update"] => Op::UpdateApply,
        ["add-resources"] => Op::ResourcesAdd,
        ["add-asset"] => Op::AssetImport,
        ["self-update"] => Op::SelfUpdateApply,
        ["webhook", "set"] => Op::WebhookSet,
        ["webhook", "remove"] => Op::WebhookRemove,
        ["delete"] => Op::ServerDelete,
        ["schedule", "set"] => Op::ScheduleSet,
        ["schedule", "off"] => Op::ScheduleRemove,
        ["app", "install"] => Op::AppInstall,
        ["app", "uninstall"] => Op::AppUninstall,
        ["tools", "install"] => Op::ToolInstall,
        ["keys", "add"] => Op::KeyAdd,
        ["keys", "replace"] | ["keys", "rename"] => Op::KeyUpdate,
        ["keys", "remove"] => Op::KeyRemove,
        ["keys", "use"] => Op::ServerKeySet,
        ["profile", "forget"] => Op::ProfileSet,
        ["branch", "switch"] | ["branch", "default"] | ["pr", "checkout"] => Op::BranchSwitch,
        ["branch", "all"] => Op::BranchBulkApply,
        ["branch", "restore"] => Op::StashRestore,
        ["start"] | ["stop"] | ["restart"] => Op::ServerControl,
        ["uninstall"] => Op::UninstallRun,
        ["panel", "user", "add"] => Op::UserAdd,
        ["panel", "user", "remove"] => Op::UserRemove,
        ["panel", "tailscale", "allow"] | ["panel", "tailscale", "remove"] | ["panel", "tailscale", "off"] | ["panel", "remote-actions", "on"] | ["panel", "remote-actions", "off"] => Op::RemoteSet,
        ["panel", "restart"] => Op::PanelRestart,
        _ => return None,
    })
}

fn flag(m: &clap::ArgMatches, id: &str) -> bool {
    m.try_get_one::<bool>(id).ok().flatten().copied().unwrap_or(false)
}

/// Written when the command starts and finished with its exit code. See the module docs.
pub struct CliAudit {
    op: Option<Op>,
    server: Option<String>,
}

fn os_user() -> Option<String> {
    ["SUDO_USER", "USER", "USERNAME", "LOGNAME"].iter().find_map(|k| std::env::var(k).ok().filter(|v| !v.trim().is_empty()))
}

fn write(op: Op, server: Option<&str>, res: &str, detail: Option<String>) {
    audit::record_quiet(
        &kora_core::installer::registry::kora_home(),
        &New { op: op.name().to_string(), server: server.map(String::from), actor_kind: actor::CLI.into(), actor_name: os_user(), provider: None, role: None, result: res.into(), detail },
    );
}

/// Work out from the command line (already known to parse) whether this command changes something, and note its start.
pub fn audit_start<C: CommandFactory>() -> CliAudit {
    let none = CliAudit { op: None, server: None };
    let Ok(top) = C::command().try_get_matches_from(std::env::args_os()) else { return none };
    let mut path: Vec<String> = Vec::new();
    let mut m = &top;
    while let Some((name, sub)) = m.subcommand() {
        path.push(name.to_string());
        m = sub;
    }
    let path_refs: Vec<&str> = path.iter().map(String::as_str).collect();
    let Some(mut op) = op_of(&path_refs) else { return none };
    // Looking, planning and trying are not changes.
    if flag(m, "dry_run") || flag(m, "preview") || flag(m, "list_snapshots") || flag(m, "check") {
        return none;
    }
    if path_refs == ["update"] {
        if flag(m, "rollback") {
            op = Op::SnapshotRollback;
        } else if flag(m, "auto") {
            op = Op::UpdateAuto;
        }
    }
    // `kora install` with nothing after it only opens the panel's install page.
    if path_refs == ["install"] && !m.ids().any(|id| m.value_source(id.as_str()) == Some(clap::parser::ValueSource::CommandLine)) {
        return none;
    }
    let server = m.try_get_one::<String>("server").ok().flatten().cloned();
    write(op, server.as_deref(), result::STARTED, None);
    CliAudit { op: Some(op), server }
}

impl CliAudit {
    pub fn finish(self, code: i32) {
        if let Some(op) = self.op {
            write(op, self.server.as_deref(), if code == 0 { result::OK } else { result::FAILED }, (code != 0).then(|| format!("exit code {code}")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_written_in_utc() {
        assert_eq!(when(0), "1970-01-01 00:00:00");
        assert_eq!(when(1_700_000_000), "2023-11-14 22:13:20");
        assert_eq!(when(951_782_400), "2000-02-29 00:00:00");
    }

    #[test]
    fn changing_commands_are_known_and_reading_ones_are_not() {
        assert_eq!(op_of(&["keys", "add"]), Some(Op::KeyAdd));
        assert_eq!(op_of(&["stop"]), Some(Op::ServerControl));
        assert_eq!(op_of(&["keys", "list"]), None);
        assert_eq!(op_of(&["status"]), None);
        assert_eq!(op_of(&["audit"]), None);
        assert_eq!(op_of(&["store", "backup"]), None);
    }
}
