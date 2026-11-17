//! `kora panel remote-actions` and `kora panel user`: who may sign in from other computers.

use clap::Subcommand;

#[derive(Subcommand, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RemoteActionsCmd {
    /// Allow these actions for administrators who sign in with a token.
    On,
    /// Refuse these actions for token sign-ins from other computers (the default).
    Off,
    /// Show whether they are on or off.
    Status,
}

#[derive(Subcommand, Debug)]
pub(crate) enum PanelUserCmd {
    /// Add a user and print their token once (it cannot be shown again).
    Add {
        /// A name for this person (letters, numbers, dots, dashes, underscores).
        name: String,
        /// What they may do: viewer (look only), operator (run servers, updates, logs) or admin (everything).
        #[arg(long, value_parser = ["viewer", "operator", "admin"])]
        role: String,
    },
    /// List the users and their roles.
    List,
    /// Remove a user; their sessions stop working at once.
    Remove {
        /// The user to remove.
        name: String,
    },
}

/// What `kora panel remote-actions on|off|status` prints.
fn remote_actions_message(cmd: RemoteActionsCmd, on: bool) -> &'static str {
    match (cmd, on) {
        (RemoteActionsCmd::Status, true) => "Remote actions are on: administrators who sign in with a token from another computer can delete and unregister servers, uninstall Kora, send console commands, change schedules and update Kora. Turn them off with `kora panel remote-actions off`.",
        (RemoteActionsCmd::Status, false) => "Remote actions are off: administrators who sign in with a token from another computer can install and manage servers, but cannot delete or unregister them, uninstall Kora, send console commands, change schedules or update Kora. Tailscale administrators are not held back by this setting. Turn it on with `kora panel remote-actions on`.",
        (_, true) => "Administrators who sign in with a token from another computer can now delete and unregister servers, uninstall Kora, send console commands, change schedules and update Kora. Deleting still asks for the server's name. Only give the admin role to people you trust.",
        (_, false) => "Remote actions are turned off. Token sign-ins from other computers cannot delete or unregister servers, uninstall Kora, send console commands, change schedules or update Kora (they can still install and manage servers). These all still work on this computer, and for Tailscale administrators.",
    }
}

/// `kora panel remote-actions on|off|status`: the "destructive actions" switch of Settings > Remote
/// access, for a server with no local browser. Saved through the same `Remote::update` the panel uses;
/// a running panel notices the changed file by itself, so no restart is needed.
pub(crate) fn panel_remote_actions(cmd: RemoteActionsCmd, path: &std::path::Path) -> i32 {
    let run = || -> Result<bool, String> {
        let remote = kora_web::remote::Remote::from_file(path.to_path_buf())?;
        match cmd {
            RemoteActionsCmd::Status => Ok(remote.config().allow_remote_destructive),
            RemoteActionsCmd::On | RemoteActionsCmd::Off => remote.update(|cfg| {
                cfg.allow_remote_destructive = cmd == RemoteActionsCmd::On;
                Ok(cfg.allow_remote_destructive)
            }),
        }
    };
    match run() {
        Ok(on) => {
            println!("{}", remote_actions_message(cmd, on));
            if cmd != RemoteActionsCmd::Status {
                println!("A running panel uses the new setting right away. No restart is needed.");
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// `kora panel user ...`: edits `<kora home>/panel.json`. A running panel notices the change by itself.
pub(crate) fn panel_user(cmd: &PanelUserCmd) -> i32 {
    use kora_web::remote::{config, RemoteConfig, Role};
    let path = config::default_path();
    let mut cfg = match RemoteConfig::load(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let result: Result<(), String> = match cmd {
        PanelUserCmd::Add { name, role } => role.parse::<Role>().and_then(|role| {
            let token = cfg.add_user(name, role)?;
            cfg.save(&path)?;
            println!("Added {} ({role}).", name.trim());
            println!("Token (shown once, keep it safe): {token}");
            if !cfg.enabled {
                println!("Remote access is still off. Start the panel with `kora panel --bind <address> --remote`, or turn it on in the panel's Settings.");
            }
            Ok(())
        }),
        PanelUserCmd::List => {
            if cfg.users.is_empty() {
                println!("No users yet. Add one with: kora panel user add <name> --role admin");
            }
            for u in &cfg.users {
                println!("{:<24} {}", u.name, u.role);
            }
            Ok(())
        }
        PanelUserCmd::Remove { name } => {
            if cfg.remove_user(name) {
                cfg.save(&path).map(|()| println!("Removed {name}."))
            } else {
                Err(format!("There is no user named '{name}'."))
            }
        }
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::panel::PanelCmd;
    use crate::Cli;
    use crate::Command;
    use clap::Parser;

    #[test]
    fn panel_remote_actions_switches_the_saved_setting() {
        use kora_web::remote::RemoteConfig;
        let cmd = |a: &[&str]| match Cli::try_parse_from(a).map(|c| c.command) {
            Ok(Some(Command::Panel(p))) => Ok(p.cmd),
            Ok(_) => panic!("not the panel command"),
            Err(e) => Err(e.to_string()),
        };
        assert!(matches!(cmd(&["kora", "panel", "remote-actions", "on"]), Ok(Some(PanelCmd::RemoteActions { cmd: RemoteActionsCmd::On }))));
        assert!(matches!(cmd(&["kora", "panel", "remote-actions", "off"]), Ok(Some(PanelCmd::RemoteActions { cmd: RemoteActionsCmd::Off }))));
        assert!(matches!(cmd(&["kora", "panel", "remote-actions", "status"]), Ok(Some(PanelCmd::RemoteActions { cmd: RemoteActionsCmd::Status }))));
        assert!(cmd(&["kora", "panel", "remote-actions"]).is_err(), "it needs on, off or status");
        assert!(cmd(&["kora", "panel", "remote-actions", "maybe"]).is_err());

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("panel.json");
        let saved = || RemoteConfig::load(&path).unwrap().allow_remote_destructive;
        // Nothing saved yet: off, and asking does not create the file.
        assert_eq!(panel_remote_actions(RemoteActionsCmd::Status, &path), 0);
        assert!(!path.exists());
        // The rest of the settings is kept.
        let mut cfg = RemoteConfig { enabled: true, ..Default::default() };
        cfg.add_user("sam", kora_web::remote::Role::Admin).unwrap();
        cfg.save(&path).unwrap();
        assert_eq!(panel_remote_actions(RemoteActionsCmd::On, &path), 0);
        assert!(saved());
        assert_eq!(panel_remote_actions(RemoteActionsCmd::Status, &path), 0);
        assert!(saved(), "status changes nothing");
        assert_eq!(panel_remote_actions(RemoteActionsCmd::Off, &path), 0);
        assert!(!saved());
        let after = RemoteConfig::load(&path).unwrap();
        assert!(after.enabled && after.user("sam").is_some());
        // A damaged file is reported, never replaced.
        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(panel_remote_actions(RemoteActionsCmd::On, &path), 1);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{not json");

        assert!(remote_actions_message(RemoteActionsCmd::On, true).starts_with("Administrators who sign in with a token from another computer can now delete and unregister servers"));
        assert!(
            remote_actions_message(RemoteActionsCmd::Status, false).contains("can install and manage servers")
                && remote_actions_message(RemoteActionsCmd::Status, false).contains("Tailscale administrators are not held back")
        );
        assert!(remote_actions_message(RemoteActionsCmd::Off, false).starts_with("Remote actions are turned off."));
        assert!(remote_actions_message(RemoteActionsCmd::Status, true).contains("kora panel remote-actions off"));
        assert!(remote_actions_message(RemoteActionsCmd::Status, false).contains("kora panel remote-actions on"));
    }
}
