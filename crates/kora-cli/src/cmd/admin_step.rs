//! The hidden commands Kora's administrator service runs as its children (see `kora_core::privsvc`):
//!
//! * `kora admin-step run|undo --step <id> --dir <folder>`: one administrator step of an install or
//!   a delete (`kora_core::installer::system_ops`). Prints one `KORA-ADMIN-RESULT {json}` line at the end.
//! * `kora admin-db start|install`: Kora's own MariaDB on Windows, outside an install.
//!
//! Neither is for people, and neither ever asks for administrator rights itself: the service has
//! already decided (caller, folder, ownership, ports) and starts them with a clean environment.

use std::path::PathBuf;

use clap::Args;
use kora_core::installer::system_ops;

#[derive(Args, Debug, Clone)]
pub struct AdminStepArgs {
    /// `run` (install) or `undo` (delete).
    #[arg(value_parser = ["run", "undo"])]
    pub action: String,
    /// The install step (see `kora_core::installer::system_ops`).
    #[arg(long, value_parser = ["prepare.db", "build.db", "system.firewall", "reach.firewall", "system.service", "system.schedule"])]
    pub step: String,
    /// The server folder.
    #[arg(long)]
    pub dir: PathBuf,
    /// Undo: the name in the person's server list.
    #[arg(long, requires_all = ["game_port", "txadmin_port"])]
    pub name: Option<String>,
    #[arg(long)]
    pub game_port: Option<u16>,
    #[arg(long)]
    pub txadmin_port: Option<u16>,
    #[arg(long)]
    pub keep_files: bool,
    #[arg(long)]
    pub keep_db: bool,
}

#[derive(Args, Debug, Clone)]
pub struct AdminDbArgs {
    /// `start` Kora's own MariaDB, or `install` it when it is not there yet.
    #[arg(value_parser = ["start", "install"])]
    pub action: String,
}

/// `kora admin-db start|install` (Windows, run by the administrator helper).
pub fn admin_db(a: &AdminDbArgs) -> i32 {
    match system_ops::db_server(&a.action, &mut |l| println!("{l}")) {
        Ok(m) => {
            println!("{m}");
            0
        }
        Err(e) => crate::ui::fail(e),
    }
}

/// `kora admin-step`: exit code 0 when the step worked; the result line is printed either way.
pub fn admin_step(a: &AdminStepArgs) -> i32 {
    let known = match (&a.name, a.game_port, a.txadmin_port) {
        (Some(n), Some(g), Some(t)) => Some((n.clone(), g, t)),
        _ => None,
    };
    let r = system_ops::run_admin_step(a.action == "undo", &a.step, &a.dir, a.keep_files, a.keep_db, known, &mut |line| println!("{line}"));
    let (code, result) = match r {
        Ok(r) if r.error.is_none() => (0, r),
        Ok(r) => (1, r),
        Err(e) => (1, system_ops::StepResult { error: Some(e), ..Default::default() }),
    };
    if let Some(p) = &result.error {
        crate::ui::print_error(p);
    }
    match result.to_line() {
        Ok(line) => println!("{line}"),
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    }
    code
}

#[cfg(test)]
mod tests {

    #[test]
    fn the_hidden_commands_never_ask_for_administrator_rights_themselves() {
        use clap::Parser;
        let kind = |a: &[&str]| crate::command_kind(&crate::Cli::try_parse_from(std::iter::once("kora").chain(a.iter().copied())).unwrap().command);
        assert_eq!(kind(&["admin-step", "run", "--step", "system.service", "--dir", "/x"]), kora_core::elevate::CmdKind::Other);
        assert_eq!(kind(&["admin-db", "start"]), kora_core::elevate::CmdKind::Other);
    }
}
