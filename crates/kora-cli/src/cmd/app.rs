//! `kora app` (the per-user shortcuts) and the removal of the old machine-wide app.

use crate::ui::{print_lines, Terminal};
use std::io::IsTerminal;

use clap::Subcommand;
use kora_core::installer;

#[derive(Subcommand)]
pub(crate) enum AppCmd {
    /// Make the Kora shortcuts that open the panel (per user, no administrator rights; idempotent).
    Install {
        /// Do not ask (nothing is asked; accepted for scripts).
        #[arg(long, short = 'y')]
        yes: bool,
        /// No desktop shortcut (Windows).
        #[arg(long)]
        no_desktop_shortcut: bool,
        /// Accepted and ignored since 0.7.0.
        #[arg(long, hide = true)]
        force: bool,
        /// List every change and make none.
        #[arg(long)]
        dry_run: bool,
    },
    /// Show the shortcuts and whether the old machine-wide app is still here (`key: value` lines).
    Status,
    /// Remove Kora's shortcuts (Start Menu / Desktop on Windows, the applications-menu launcher on Linux). If the
    /// OLD machine-wide app is also installed it is removed too when you run this as administrator; otherwise
    /// `kora uninstall --app` (as administrator) does that. Servers and databases are never touched.
    Uninstall {
        #[arg(long, short = 'y')]
        yes: bool,
        #[arg(long)]
        dry_run: bool,
    },
}

/// `kora app install`: the per-user shortcuts that open the panel. No administrator rights.
pub(crate) fn app_install(no_desktop_shortcut: bool, dry_run: bool) -> i32 {
    use kora_core::app_install::user;
    match user::install_user(&user::UserOptions { desktop_shortcut: !no_desktop_shortcut, dry_run }) {
        Ok(out) => {
            if dry_run {
                println!("Dry run: nothing was changed. Making the Kora shortcuts would:");
            } else if out.changes.is_empty() {
                println!("The Kora shortcuts are already in place ({}).", out.exe.display());
            } else {
                println!("The Kora shortcuts are ready ({}):", out.exe.display());
            }
            print_lines("  - ", &out.changes);
            print_lines("  note: ", &out.notes);
            if user::legacy_present().present() {
                println!("The old machine-wide Kora app is still installed. Remove it with `kora uninstall --app` (as administrator), or use \"Tidy up old shortcuts\" in the panel.");
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

pub(crate) fn app_status() -> i32 {
    use kora_core::app_install::user;
    let st = user::user_status();
    let yn = |b: Option<bool>| b.map_or("n/a", |b| if b { "yes" } else { "no" });
    println!("shortcut_start_menu: {}", yn(st.start_menu));
    println!("shortcut_desktop: {}", yn(st.desktop));
    println!("launcher: {}", yn(st.launcher));
    let legacy = user::legacy_present();
    println!("old_app_present: {}", if legacy.present() { "yes" } else { "no" });
    print_lines("old_app_item: ", &legacy.items);
    print_lines("", &kora_core::app_install::status().lines());
    0
}

/// `kora app uninstall` (`shortcuts`: also Kora's own per-user shortcuts, no administrator rights) and
/// `kora uninstall --app` (only the OLD machine-wide app, which is what "Tidy up old shortcuts" runs as administrator).
pub(crate) fn app_uninstall(yes: bool, dry_run: bool, shortcuts: bool) -> i32 {
    use kora_core::app_install as app;
    let legacy = app::user::legacy_present().present();
    let mine = shortcuts && app::user::user_status().any();
    if !legacy && !mine {
        println!(
            "{} Your servers and databases were not touched.",
            if shortcuts { "There are no Kora shortcuts and no old machine-wide Kora app: nothing to remove." } else { "The old machine-wide Kora app is not installed: nothing to remove." }
        );
        return 0;
    }
    let admin = installer::is_admin() || std::env::var_os("KORA_APP_PREFIX").is_some();
    // The old app can only go with administrator rights. With `--app` that is the whole job; otherwise the
    // shortcuts are still removed and the old app is explained.
    if legacy && !admin && !dry_run && !shortcuts {
        eprintln!("error: removing the old Kora app needs administrator rights: run this as Administrator (Windows) or with sudo (Linux).");
        return 1;
    }
    let remove_legacy = legacy && (admin || dry_run);
    if !yes && !dry_run {
        if !std::io::stdin().is_terminal() {
            eprintln!("error: pass --yes to remove Kora's shortcuts or the old Kora app without a terminal to ask in (or --dry-run to preview).");
            return 2;
        }
        let what = match (mine, remove_legacy) {
            (true, true) => "Remove Kora's shortcuts and the old Kora app (its PATH entry and installed program)?",
            (true, false) => "Remove Kora's shortcuts?",
            _ => "Remove the old Kora app (its shortcuts, PATH entry and installed program)?",
        };
        if !installer::uninstall::Asker::ask(&mut Terminal, &format!("{what} Your servers and databases are not touched."), true) {
            println!("Cancelled. Nothing was changed.");
            return 0;
        }
    }
    let mut code = 0;
    if mine {
        match app::user::uninstall_user(dry_run) {
            Ok(lines) => {
                println!("{}", if dry_run { "Dry run: nothing was changed. Removing Kora's shortcuts would:" } else { "Kora's shortcuts were removed:" });
                print_lines("  - ", &lines);
                crate::cmd::desktop::remove_autostart(dry_run);
            }
            Err(e) => {
                eprintln!("error: {e}");
                code = 1;
            }
        }
    }
    if remove_legacy {
        match app::uninstall_app_opts(dry_run) {
            Ok(lines) => {
                println!("{}", if dry_run { "Dry run: nothing was changed. Removing the old Kora app would:" } else { "The old Kora app was removed:" });
                print_lines("  - ", &lines);
            }
            Err(e) => {
                eprintln!("error: {e}");
                code = 1;
            }
        }
    } else if legacy {
        println!("The old machine-wide Kora app is still installed. Remove it with `kora uninstall --app`, run as Administrator (Windows) or with sudo (Linux).");
    }
    println!("Your servers and databases were not touched.");
    code
}
