//! `kora update`: preview, snapshot, apply and roll back resource updates.

use crate::ui::confirm;
use std::io::IsTerminal;

use clap::{Args, Parser, ValueEnum};
use kora_core::installer::gitops;

/// What to do when an update needs a restart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum RestartArg {
    /// Apply the files, restart nothing.
    None,
    /// Apply the files without restarting; the server loads them on its next restart or an `ensure` in txAdmin (Kora cannot do that for you without credentials).
    Ensure,
    /// Restart the whole server.
    Full,
    /// Restart only when nobody is playing (default).
    WhenQuiet,
}

#[derive(Args, Debug, Default)]
pub(crate) struct UpdateArgs {
    /// Run unattended (fast-forward clean forks only).
    #[arg(long, conflicts_with_all = ["resource", "all", "preview", "yes", "restart", "rollback", "list_snapshots", "force"])]
    pub(crate) auto: bool,
    /// Server name or id.
    pub(crate) server: String,
    /// Update only this resource (repeatable).
    #[arg(long = "resource", value_name = "ID", conflicts_with = "all")]
    pub(crate) resource: Vec<String>,
    /// Update every resource.
    #[arg(long)]
    pub(crate) all: bool,
    /// Show what would change, then stop.
    #[arg(long, conflicts_with_all = ["rollback", "list_snapshots"])]
    pub(crate) preview: bool,
    /// Do not ask for confirmation.
    #[arg(long, short = 'y')]
    pub(crate) yes: bool,
    /// What to do about restarts.
    #[arg(long, value_enum)]
    pub(crate) restart: Option<RestartArg>,
    /// Put everything back the way a snapshot saved it.
    #[arg(long, value_name = "SNAPSHOT", conflicts_with_all = ["resource", "all", "list_snapshots"])]
    pub(crate) rollback: Option<String>,
    /// List saved snapshots.
    #[arg(long, conflicts_with_all = ["resource", "all"])]
    list_snapshots: bool,
    /// With --rollback: roll back even resources with uncommitted changes of your own (they are thrown away).
    #[arg(long, requires = "rollback")]
    pub(crate) force: bool,
}

pub(crate) fn update(a: UpdateArgs) -> i32 {
    use kora_core::update::manual as m;
    if a.auto {
        match gitops::auto_update(&a.server) {
            Ok(report) => {
                println!("{}", report.summary());
                return m::exit_code(&report);
            }
            // The scheduled run simply skips this time (already written to update.log).
            Err(e) if kora_core::update::lock::is_busy_message(&e.to_string()) => {
                println!("Skipped: {e}");
                return 0;
            }
            Err(e) => {
                eprintln!("error: {e}");
                return 1;
            }
        }
    }
    let server = match m::load_server(&a.server) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    if a.list_snapshots {
        let list = m::snapshots(&server);
        if list.is_empty() {
            println!("No snapshots yet. One is saved before every update.");
        }
        for s in list {
            println!("{}  (unix {})  {}", s.id, s.created_unix, s.resources.join(", "));
        }
        return 0;
    }
    if let Some(id) = &a.rollback {
        let warn = if a.force { " Changes of your own in those resources are thrown away." } else { "" };
        if !a.yes && !confirm(&format!("Put '{}' back the way snapshot {id} saved it? The server restarts if it is running.{warn}", server.name)) {
            println!("Nothing changed.");
            return 0;
        }
        return match m::rollback(&server, id, a.force) {
            Ok(()) => {
                println!("Rolled back to snapshot {id}.");
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        };
    }
    let scope = if a.all || (a.resource.is_empty() && a.preview) {
        m::Scope::All
    } else if a.resource.is_empty() {
        eprintln!("error: say what to update: --resource <id> (repeatable) or --all. Add --preview to only look.");
        return 1;
    } else {
        m::Scope::Resources(a.resource.clone())
    };
    let pre = match m::preview(&server, scope.clone()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    print!("{}", m::render_preview(&pre));
    if a.preview {
        return 0;
    }
    if !pre.resources.iter().any(|c| c.status == m::ChangeStatus::Clean) {
        return if pre.resources.iter().any(|c| matches!(c.status, m::ChangeStatus::Dirty | m::ChangeStatus::Diverged)) { 3 } else { 0 };
    }
    if !a.yes {
        if !std::io::stdin().is_terminal() {
            eprintln!("error: not asking for confirmation here; add --yes to go ahead.");
            return 1;
        }
        if !confirm("Apply these updates? A snapshot is saved first.") {
            println!("Nothing changed.");
            return 0;
        }
    }
    let restart = match a.restart.unwrap_or(RestartArg::WhenQuiet) {
        RestartArg::None => m::RestartPolicy::None,
        RestartArg::Ensure => m::RestartPolicy::Ensure,
        RestartArg::Full => m::RestartPolicy::FullRestart,
        RestartArg::WhenQuiet => m::RestartPolicy::WhenQuiet,
    };
    let mut snapshot = None;
    // Exactly what was just shown, even if something new was published while you read it.
    let opts = m::ApplyOpts { targets: Some(m::planned_targets(&pre)), ..m::ApplyOpts::new(restart) };
    let res = m::apply(&server, scope, opts, &mut |e| match e {
        m::UpdateEvent::Snapshot(id) => {
            println!("Saved snapshot {id}.");
            snapshot = Some(id);
        }
        m::UpdateEvent::Resource { id, state } if state == "updating" => {
            println!("Updating {id}...")
        }
        m::UpdateEvent::Health(h) => println!("{h}."),
        m::UpdateEvent::Progress(p) => println!("{p}..."),
        _ => {}
    });
    match res {
        Ok(report) => {
            print!("{}", m::render_report(&report));
            if let Some(id) = snapshot {
                println!("To undo: kora update {} --rollback {id}", a.server);
            }
            m::exit_code(&report)
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Cli;
    use crate::Command;
    use clap::Parser;

    fn parse_update(args: &[&str]) -> Result<UpdateArgs, clap::Error> {
        let mut v = vec!["kora", "update"];
        v.extend_from_slice(args);
        match Cli::try_parse_from(v)?.command {
            Some(Command::Update(a)) => Ok(a),
            _ => unreachable!(),
        }
    }

    #[test]
    fn update_flags_parse() {
        let a = parse_update(&["main", "--resource", "kora-chat", "--resource", "kora-hud", "--yes", "--restart", "when-quiet"]).unwrap();
        assert_eq!((a.server.as_str(), a.resource.len(), a.yes, a.restart), ("main", 2, true, Some(RestartArg::WhenQuiet)));
        assert_eq!(parse_update(&["main", "--all", "--preview"]).map(|a| (a.all, a.preview)).unwrap(), (true, true));
        assert_eq!(parse_update(&["main", "--all", "--restart", "full"]).unwrap().restart, Some(RestartArg::Full));
        assert_eq!(parse_update(&["main", "--rollback", "123"]).unwrap().rollback.as_deref(), Some("123"));
        assert!(parse_update(&["main", "--list-snapshots"]).unwrap().list_snapshots);
        assert!(parse_update(&["--auto", "/srv/main"]).unwrap().auto);
    }

    #[test]
    fn update_flag_conflicts_are_rejected() {
        for bad in [
            &["main", "--all", "--resource", "x"][..],
            &["main", "--auto", "--all"],
            &["main", "--rollback", "1", "--all"],
            &["main", "--preview", "--rollback", "1"],
            &["main", "--restart", "sometimes"],
            &["--all"],
        ] {
            assert!(parse_update(bad).is_err(), "{bad:?}");
        }
    }
}
