//! `kora uninstall`: remove the system changes Kora made.

use crate::cmd::install::Printer;
use crate::ui::{print_lines, Terminal};
use std::io::IsTerminal;

use kora_core::installer::{self, registry, RollbackOptions};

pub(crate) fn uninstall(server: Option<String>, dry_run: bool, yes: bool, delete_files: bool, delete_databases: bool) -> i32 {
    use installer::uninstall as un;
    if !yes && !dry_run && !std::io::stdin().is_terminal() {
        eprintln!("error: `kora uninstall` asks before each removal and needs a terminal. Use --dry-run to preview, or --yes to remove the system changes without questions (folders and databases are kept).");
        return 2;
    }
    let all_servers = server.is_none();
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
    let mut delete = |dir: &std::path::Path, o: &RollbackOptions| {
        // In the elevated worker: only folders that are the panel person's (checked here, in the
        // child, right before anything is removed).
        installer::system_ops::check_panel_dir(dir)?;
        installer::rollback(dir, o, &mut Printer { last: None })
    };
    let mut save = |r: &registry::Registry| r.save_or_clean_to(&registry::registry_path());
    let summary =
        if yes { un::run(&mut reg, &opts, &env, &mut un::Defaults, &mut say, &mut delete, &mut save) } else { un::run(&mut reg, &opts, &env, &mut Terminal, &mut say, &mut delete, &mut save) };
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
    // Kora's own shortcuts (per user). Under `sudo` they belong to the person who typed it, not to root.
    let under_sudo = cfg!(unix) && kora_core::userhome::sudo_account().is_some();
    if under_sudo {
        if kora_core::app_install::user::user_status().any() || kora_core::app_install::user::legacy_present().present() {
            println!("\nKora's shortcuts belong to your own account: run `kora app uninstall` without sudo to remove them.");
        }
    } else {
        match kora_core::app_install::user::uninstall_user(dry_run) {
            Ok(lines) if !lines.is_empty() => {
                println!("\n{}", if dry_run { "Kora's shortcuts would be removed:" } else { "Kora's shortcuts were removed:" });
                print_lines("  - ", &lines);
            }
            Ok(_) => {}
            Err(e) => summary.failed.push(format!("Kora's shortcuts: {e}")),
        }
    }
    // Kora's own Git and GitHub CLI copies live in Kora's folder; they go with a full uninstall.
    if opts.only.is_none() {
        let present = kora_core::tools::private_present();
        if !present.is_empty() {
            let what = present.join(" and ");
            if dry_run {
                println!("Would remove Kora's own copies of {what}.");
            } else {
                let q = format!("Remove Kora's own copies of {what}?");
                let go = if yes { un::Asker::ask(&mut un::Defaults, &q, true) } else { un::Asker::ask(&mut Terminal, &q, true) };
                if go {
                    match kora_core::tools::remove_private() {
                        Ok(_) => {
                            println!("Removed Kora's own copies of {what}.");
                            summary.removed.push(format!("Kora's own copies of {what}"));
                        }
                        Err(e) => summary.failed.push(format!("Kora's own copies of {what}: {e}")),
                    }
                } else {
                    summary.kept.push(format!("Kora's own copies of {what}"));
                }
            }
        }
    }
    if all_servers {
        println!();
        if dry_run {
            crate::cmd::keys_cmd::uninstall_note(true, None);
        } else if yes {
            crate::cmd::keys_cmd::uninstall_note(false, None);
        } else {
            crate::cmd::keys_cmd::uninstall_note(false, Some(&mut Terminal));
        }
    }
    if dry_run {
        println!("\nDry run: nothing was changed.");
        if kora_core::app_install::status().installed {
            println!("The Kora app would stay installed (remove it with `kora uninstall --app`).");
        }
        return 0;
    }
    println!("\nRemoved: {}. Kept: {}. Failed: {}.", summary.removed.len(), summary.kept.len(), summary.failed.len());
    if kora_core::app_install::status().installed {
        println!("The Kora app itself (the `kora` command and its shortcuts) is still installed. Remove it with `kora uninstall --app`.");
    }
    for f in &summary.failed {
        eprintln!("  failed: {f}");
    }
    i32::from(!summary.ok())
}
