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
    /// Invite a person: makes the user and prints a one-time link (30 minutes, works once) where they set up a passkey or one-time codes. No token is made.
    #[command(alias = "add")]
    Invite {
        /// A name for this person (letters, numbers, dots, dashes, underscores).
        name: String,
        /// What they may do: viewer (look only), operator (run servers, updates, logs) or admin (everything).
        #[arg(long, value_parser = ["viewer", "operator", "admin"])]
        role: String,
    },
    /// List the users, their roles and how they sign in.
    List,
    /// Show what one user can sign in with (counts and dates, never a secret).
    Credentials {
        /// The user.
        name: String,
    },
    /// A lost phone or laptop: take credentials away, end the user's sessions and print a new one-time link to set up again.
    Reset {
        /// The user.
        name: String,
        /// What to take away: passkeys, code (password and one-time codes), all (also the old token link) or legacy-token (only the old token link, no new link).
        #[arg(long, value_parser = ["passkeys", "code", "all", "legacy-token"], default_value = "all")]
        what: String,
    },
    /// Remove a user; their sessions stop working at once.
    Remove {
        /// The user to remove.
        name: String,
    },
    /// Tokens for scripts (an `Authorization: Bearer` header; never for signing in, never in an address).
    ApiToken {
        #[command(subcommand)]
        cmd: ApiTokenCmd,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ApiTokenCmd {
    /// Make a token and print it once.
    Create {
        /// A name for the script (backup-script).
        name: String,
        /// What it may do: viewer, operator or admin. Deleting and uninstalling still need `kora panel remote-actions on`.
        #[arg(long, value_parser = ["viewer", "operator", "admin"])]
        role: String,
        /// Valid for this many days. Without it the token never expires.
        #[arg(long)]
        days: Option<u32>,
    },
    /// List the tokens (never their secrets).
    List,
    /// Revoke a token by its name or number.
    Revoke {
        /// The token's name or number (see `list`).
        token: String,
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

/// Where a person opens the panel, for the link they are given: the first name in the remote settings, over https when the
/// panel has a certificate. When Kora cannot tell, the link is `<address>/#enrol=...` for the person to complete.
fn enrol_link(cfg: &kora_web::remote::RemoteConfig, secret: &str) -> String {
    let home = kora_core::monitor::panel_service::home();
    let https = {
        let m = kora_web::tls::manager::Manager::new(home, std::sync::Arc::new(kora_web::remote::NoWhois));
        m.reload(false).ok().flatten().is_some()
    };
    let port = kora_web::DEFAULT_PORT;
    match cfg.allowed_hosts.first() {
        Some(h) if h.contains(':') && !h.starts_with('[') => format!("{}://{h}/#enrol={secret}", if https { "https" } else { "http" }),
        Some(h) => format!("{}://{h}:{port}/#enrol={secret}", if https { "https" } else { "http" }),
        None => format!("<the address you open the panel at>/#enrol={secret}"),
    }
}

fn when(unix: i64) -> String {
    if unix == 0 {
        "never".into()
    } else {
        kora_web::tls::certs::format_date(unix)
    }
}

/// `kora panel user ...`. A running panel notices the change by itself.
pub(crate) fn panel_user(cmd: &PanelUserCmd) -> i32 {
    use kora_web::remote::auth::db;
    use kora_web::remote::{config, Remote, Role};
    let path = config::default_path();
    let run = || -> Result<(), String> {
        let remote = Remote::from_file(path.clone())?;
        let home = kora_core::installer::registry::kora_home();
        match cmd {
            PanelUserCmd::Invite { name, role } => {
                let role: Role = role.parse()?;
                let secret = kora_web::remote::auth::invite(&remote, name.trim(), role, "cli", Some(&home))?;
                println!("Added {} ({role}). They have no password, passkey or token yet.", name.trim());
                println!("Give them this link (it works once, for 30 minutes), and they set up a passkey or one-time codes:");
                println!("  {}", enrol_link(&remote.config(), &secret));
                if !remote.config().enabled {
                    println!("Remote access is still off. Start the panel with `kora panel --bind <address> --remote`, or turn it on in the panel's Settings.");
                }
                println!("Passkeys need the panel opened by a NAME over HTTPS (see `kora panel tls`); on a bare address they use one-time codes.");
            }
            PanelUserCmd::List => {
                let cfg = remote.config();
                if cfg.users.is_empty() {
                    println!("No users yet. Add one with: kora panel user invite <name> --role admin");
                }
                let store = remote.auth_store().ok();
                for u in &cfg.users {
                    let s = store.as_ref().and_then(|s| db::state(s, &u.name).ok()).unwrap_or_default();
                    let mut how = Vec::new();
                    if !s.passkeys.is_empty() {
                        how.push(format!("{} passkey{}", s.passkeys.len(), if s.passkeys.len() == 1 { "" } else { "s" }));
                    }
                    if s.totp && s.password {
                        how.push("one-time codes".to_string());
                    }
                    if u.has_token() {
                        how.push("old token link".to_string());
                    }
                    if how.is_empty() {
                        how.push("waiting to set up".to_string());
                    }
                    println!("{:<24} {:<9} {}", u.name, u.role, how.join(", "));
                }
            }
            PanelUserCmd::Credentials { name } => {
                let cfg = remote.config();
                let u = cfg.user(name).ok_or_else(|| format!("There is no user named '{name}'."))?;
                let s = db::state(&remote.auth_store()?, &u.name)?;
                println!("{} ({})", u.name, u.role);
                for p in &s.passkeys {
                    println!("  passkey {:<3} {:<24} for {}  added {}  last used {}", p.id, p.label, p.rp_id, when(p.created), when(p.last_used));
                }
                println!(
                    "  one-time codes: {}{}",
                    if s.totp && s.password { "set up" } else { "not set up" },
                    if s.totp && s.password { format!(" ({} recovery codes left)", s.recovery_left) } else { String::new() }
                );
                println!("  old token link: {}", if u.has_token() { "still works (for scripts)" } else { "none" });
            }
            PanelUserCmd::Reset { name, what } => {
                use kora_core::api::wire::ResetWhat;
                let w = match what.as_str() {
                    "passkeys" => ResetWhat::Passkeys,
                    "code" => ResetWhat::Code,
                    "legacy-token" => ResetWhat::LegacyToken,
                    _ => ResetWhat::All,
                };
                let done = kora_web::remote::auth::reset(&remote, name, w, "cli", Some(&home))?;
                println!("Reset {}. Their signed-in devices stop working within a minute (at once if the panel restarts).", done.name);
                match done.link {
                    Some(secret) => {
                        println!("Give them this link to set up again (it works once, for 30 minutes):");
                        println!("  {}", enrol_link(&remote.config(), &secret));
                    }
                    None => println!("The old token link no longer works."),
                }
            }
            PanelUserCmd::Remove { name } => {
                if remote.update(|c| Ok(c.remove_user(name)))? {
                    println!("Removed {name}. Their passkeys, codes and sessions are gone.");
                } else {
                    return Err(format!("There is no user named '{name}'."));
                }
            }
            PanelUserCmd::ApiToken { cmd } => {
                let store = remote.auth_store()?;
                match cmd {
                    ApiTokenCmd::Create { name, role, days } => {
                        let role: Role = role.parse()?;
                        let token = db::create_api_token(&store, name, role, *days, "cli")?;
                        println!("API token '{}' ({role}{}).", name.trim(), days.map_or(", never expires".to_string(), |d| format!(", expires in {d} days")));
                        println!("Token (shown once, keep it safe): {token}");
                        println!(
                            "Use it as a header: Authorization: Bearer <token>. It is never accepted in an address, and deleting or uninstalling through it needs `kora panel remote-actions on`."
                        );
                    }
                    ApiTokenCmd::List => {
                        let all = db::list_api_tokens(&store)?;
                        if all.is_empty() {
                            println!("No API tokens. Make one with: kora panel user api-token create <name> --role operator");
                        }
                        for t in all {
                            println!("{:<4} {:<24} {:<9} expires {:<11} last used {}", t.id, t.name, t.role, when(t.expires), when(t.last_used));
                        }
                    }
                    ApiTokenCmd::Revoke { token } => {
                        let all = db::list_api_tokens(&store)?;
                        let found = all
                            .iter()
                            .find(|t| t.name.eq_ignore_ascii_case(token) || t.id.to_string() == *token)
                            .ok_or_else(|| format!("There is no API token '{token}' (see `kora panel user api-token list`)."))?;
                        db::revoke_api_token(&store, found.id)?;
                        println!("Revoked '{}'. It stopped working at once.", found.name);
                    }
                }
            }
        }
        Ok(())
    };
    match run() {
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
