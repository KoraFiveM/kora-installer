//! `kora uninstall`: remove the system changes Kora made.

use crate::cmd::install::Printer;
use crate::ui::{print_lines, Terminal};
use std::io::IsTerminal;

use kora_core::installer::{self, registry, RollbackOptions};

/// A full uninstall also takes off what the per-user install (`install.ps1` / `install.sh`) put on this
/// computer: the running background panel and its start-up entry, the program with its PATH entry, and the
/// Windows Firewall rules for it. Kora's own records (server list, database, sign-in token) are only deleted
/// when asked (`--delete-files`, or yes at the question). Whatever cannot be removed is said, with the fix.
fn remove_own_copy(dry_run: bool, delete_data: bool, admin: bool, ask: &mut dyn FnMut(&str, bool) -> bool, summary: &mut kora_core::installer::uninstall::Summary) {
    use kora_core::app_install::user;
    use kora_core::monitor::panel_service as svc;
    use kora_core::service::{CommandRunner, Platform, SystemRunner};

    let platform = Platform::current();
    let home = svc::home();
    let me = std::process::id();
    let say = |what: &str, lines: Vec<String>| {
        if !lines.is_empty() {
            println!("\n{what}");
            print_lines("  - ", &lines);
        }
    };

    // 1. The background panel: stop it, and drop its start-up entry (Run key / systemd user unit).
    let panel_pid = svc::load_running(&home).map(|p| p.pid).filter(|p| *p != me);
    let start_up = svc::service_installed(platform);
    if panel_pid.is_some() || start_up {
        let mut lines = vec![];
        if let Some(pid) = panel_pid {
            lines.push(format!("Stop the running Kora Panel (process {pid})"));
        }
        if start_up {
            lines.push("Remove the Kora Panel start-up entry".to_string());
        }
        if dry_run {
            say("Kora Panel would be stopped and removed from start-up:", lines);
        } else {
            if let Some(pid) = panel_pid {
                let (prog, args): (&str, Vec<String>) = if cfg!(windows) { ("taskkill.exe", vec!["/PID".into(), pid.to_string(), "/F".into()]) } else { ("kill", vec![pid.to_string()]) };
                let _ = SystemRunner.run(prog, &args);
            }
            match svc::remove(&home, &SystemRunner, platform) {
                Ok(()) => {
                    say("Kora Panel was stopped and removed from start-up:", lines);
                    summary.removed.push("Kora Panel (background service)".into());
                }
                Err(e) => summary.failed.push(format!("Kora Panel start-up entry: {e}")),
            }
        }
    }

    // 2. Windows Firewall rules for kora.exe (Windows asks to allow it the first time it listens). They
    //    need administrator rights to delete; reading them does not.
    if let Some(ul) = user::UserLayout::from_env().ok().filter(|_| cfg!(windows)) {
        let program = ul.program.display().to_string();
        let rules = |verb: &str| {
            let mut c = kora_core::sysexe::command("netsh");
            c.args(["advfirewall", "firewall", verb, "rule", "name=all", &format!("program={program}")]);
            c.output().ok()
        };
        if rules("show").is_some_and(|o| o.status.success()) {
            if dry_run {
                println!("\nThe Windows Firewall rules for {program} would be removed.");
            } else if admin {
                if rules("delete").is_some_and(|o| o.status.success()) {
                    println!("\nThe Windows Firewall rules for {program} were removed.");
                    summary.removed.push("Windows Firewall rules for kora.exe".into());
                } else {
                    summary.failed.push(format!("Windows Firewall rules for {program}"));
                }
            } else {
                summary.kept.push("Windows Firewall rules for kora.exe".into());
                println!("\nLeft in place: the Windows Firewall rules for {program} (deleting them needs Administrator). In an Administrator terminal run:\n  netsh advfirewall firewall delete rule name=all program=\"{program}\"");
            }
        }
    }

    // 3. The program, its backup and its PATH entry.
    if user::program_present() {
        match user::remove_program(dry_run) {
            Ok(lines) => {
                say(if dry_run { "The Kora program would be removed:" } else { "The Kora program was removed:" }, lines);
                if !dry_run {
                    summary.removed.push("the Kora program and its PATH entry".into());
                }
            }
            Err(e) => summary.failed.push(format!("the Kora program: {e}")),
        }
    }

    // 4. Kora's own records. They hold the server list, saved settings and the panel sign-in, so they are
    //    kept unless asked for.
    if home.is_dir() && std::fs::read_dir(&home).is_ok_and(|mut d| d.next().is_some()) {
        if dry_run {
            println!("\nKora's own records in {} would be kept unless you ask to delete them (--delete-files).", home.display());
        } else if delete_data || ask(&format!("Also delete Kora's own records (server list, settings, database) in {}?", home.display()), false) {
            match std::fs::remove_dir_all(&home) {
                Ok(()) => {
                    println!("\nDeleted {}.", home.display());
                    summary.removed.push(format!("Kora's records in {}", home.display()));
                }
                // The program that is running can only be set aside, not deleted, while it runs.
                Err(e) => {
                    println!("\nCould not delete all of {} ({e}). Delete that folder by hand once Kora has closed.", home.display());
                    summary.failed.push(format!("{}: {e}", home.display()));
                }
            }
        } else {
            summary.kept.push(format!("Kora's records in {}", home.display()));
            println!("\nLeft in place: Kora's records in {}. Delete that folder by hand to remove them too.", home.display());
        }
    }
}

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
    // The per-user program, the background panel and everything that starts or points at it.
    if all_servers && !under_sudo {
        let mut ask = |q: &str, default_yes: bool| if yes { un::Asker::ask(&mut un::Defaults, q, default_yes) } else { un::Asker::ask(&mut Terminal, q, default_yes) };
        remove_own_copy(dry_run, delete_files, admin, &mut ask, &mut summary);
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
