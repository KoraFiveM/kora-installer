//! Change the game and txAdmin ports of an existing server (the manager's "Ports" screen).
//!
//! The server must be stopped first (the caller checks). What is written:
//! * `server.cfg`: the `endpoint_add_tcp` / `endpoint_add_udp` lines get the new game port;
//! * `secrets.env` (Linux service environment): the `TXHOST_TXA_PORT` line, only when it exists there;
//! * `kora-server.json` and the server registry.
//!
//! Firewall rules are NOT changed: whoever opened the old ports has to open the new ones.

use std::path::Path;

use crate::firewall::ports::{install_conflicts, PortProber};
use crate::txadmin;

use super::registry::{PortsRecord, Registry, ServerFile};
use super::types::Ports;

/// What changed, for the screen to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortChange {
    pub old: Ports,
    pub new: Ports,
    /// `server.cfg` endpoint lines were rewritten.
    pub cfg_updated: bool,
    /// The Linux service environment file had a `TXHOST_TXA_PORT` line and it was rewritten.
    pub env_updated: bool,
    pub notes: Vec<String>,
}

/// Lowest port Kora will use (same rule as the installer).
pub const MIN_PORT: u16 = 1024;

/// Rewrite the port inside every quoted `endpoint_add_tcp` / `endpoint_add_udp` address that
/// currently uses `old`. Returns `None` when no endpoint line was found at all.
pub fn rewrite_cfg_game_port(cfg: &str, new: u16) -> Option<String> {
    let mut found = false;
    let mut out = String::with_capacity(cfg.len());
    for line in cfg.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("endpoint_add_tcp") || trimmed.starts_with("endpoint_add_udp") {
            if let (Some(a), Some(b)) = (line.find('"'), line.rfind('"')) {
                if a < b {
                    let addr = &line[a + 1..b];
                    if let Some((host, _)) = addr.rsplit_once(':') {
                        found = true;
                        out.push_str(&line[..a + 1]);
                        out.push_str(&format!("{host}:{new}"));
                        out.push_str(&line[b..]);
                        continue;
                    }
                }
            }
        }
        out.push_str(line);
    }
    found.then_some(out)
}

/// Replace the `TXHOST_TXA_PORT=` line. `None` when the file has no such line.
pub fn rewrite_env_txadmin_port(env: &str, new: u16) -> Option<String> {
    let mut found = false;
    let mut out = String::with_capacity(env.len());
    for line in env.split_inclusive('\n') {
        if line.trim_start().starts_with("TXHOST_TXA_PORT=") {
            found = true;
            let eol = if line.ends_with('\n') { "\n" } else { "" };
            out.push_str(&format!("TXHOST_TXA_PORT={new}{eol}"));
        } else {
            out.push_str(line);
        }
    }
    found.then_some(out)
}

/// Check and apply a port change. `name` is the registry name of the server.
pub fn change_ports(dir: &Path, name: &str, new: Ports, prober: &dyn PortProber) -> Result<PortChange, String> {
    let mut sf = ServerFile::load(dir).map_err(|e| format!("cannot read kora-server.json: {e}"))?;
    let old = Ports { game: sf.ports.game, txadmin: sf.ports.txadmin };
    if new == old {
        return Err("those are already the ports this server uses".into());
    }
    if new.game < MIN_PORT || new.txadmin < MIN_PORT {
        return Err(format!("ports must be {MIN_PORT} or higher"));
    }
    if new.game == new.txadmin {
        return Err("the game port and the txAdmin port must be different".into());
    }
    // The server's own old ports are free for it to keep; other Kora servers and busy ports are not.
    let conflicts = install_conflicts(name, dir, new.game, new.txadmin, prober);
    if !conflicts.is_empty() {
        return Err(conflicts.join(" "));
    }

    let mut notes = Vec::new();

    let cfg_path = dir.join("server.cfg");
    let cfg_updated = if old.game != new.game {
        match std::fs::read_to_string(&cfg_path) {
            Ok(cfg) => match rewrite_cfg_game_port(&cfg, new.game) {
                Some(text) => {
                    txadmin::write_file(&cfg_path, &text, false).map_err(|e| format!("cannot write server.cfg: {e}"))?;
                    true
                }
                None => {
                    notes.push("server.cfg has no endpoint_add lines, so the game port there was not changed. Check it by hand.".into());
                    false
                }
            },
            Err(e) => return Err(format!("cannot read server.cfg: {e}")),
        }
    } else {
        false
    };

    let env_path = dir.join("secrets.env");
    let env_updated = if old.txadmin != new.txadmin {
        match std::fs::read_to_string(&env_path) {
            Ok(env) => match rewrite_env_txadmin_port(&env, new.txadmin) {
                Some(text) => {
                    txadmin::write_file(&env_path, &text, true).map_err(|e| format!("cannot write the service environment: {e}"))?;
                    true
                }
                None => false,
            },
            Err(_) => false,
        }
    } else {
        false
    };

    sf.ports = PortsRecord { game: new.game, txadmin: new.txadmin };
    sf.save().map_err(|e| format!("cannot write kora-server.json: {e}"))?;

    let path = super::registry::registry_path();
    Registry::update_at(&path, |reg| {
        if let Some(entry) = reg.servers.iter_mut().find(|s| s.name == name) {
            entry.ports = PortsRecord { game: new.game, txadmin: new.txadmin };
        }
        Ok(())
    })
    .map_err(|e| format!("cannot write the server registry: {e}"))?;

    notes.push("Firewall rules were not changed. Open the new ports and close the old ones if you opened them.".into());
    notes.push("Start the server again to use the new ports.".into());
    Ok(PortChange { old, new, cfg_updated, env_updated, notes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_both_endpoint_lines_and_keeps_the_host() {
        let cfg = "exec secrets.cfg\n\nendpoint_add_tcp \"0.0.0.0:30120\"\nendpoint_add_udp \"0.0.0.0:30120\"\nsv_maxclients 48\n";
        let out = rewrite_cfg_game_port(cfg, 31120).unwrap();
        assert_eq!(out, "exec secrets.cfg\n\nendpoint_add_tcp \"0.0.0.0:31120\"\nendpoint_add_udp \"0.0.0.0:31120\"\nsv_maxclients 48\n");
    }

    #[test]
    fn no_endpoint_lines_means_none() {
        assert!(rewrite_cfg_game_port("sv_maxclients 48\n", 31120).is_none());
    }

    #[test]
    fn env_txadmin_line_is_replaced_only_when_present() {
        let env = "TXHOST_DATA_PATH=/srv/a/txData\nTXHOST_TXA_PORT=40120\nTXHOST_GAME_NAME=fivem";
        assert_eq!(rewrite_env_txadmin_port(env, 41120).unwrap(), "TXHOST_DATA_PATH=/srv/a/txData\nTXHOST_TXA_PORT=41120\nTXHOST_GAME_NAME=fivem");
        assert!(rewrite_env_txadmin_port("TXHOST_GAME_NAME=fivem\n", 41120).is_none());
    }
}
