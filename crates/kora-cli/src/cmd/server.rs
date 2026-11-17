//! `kora status`, `start`, `stop`, `restart` and `delete`: looking after one registered server.

use crate::cmd::install::Printer;
use std::path::PathBuf;

use kora_core::installer::{self, host, registry, RollbackOptions, Source, Versions};
use kora_core::launch;

pub(crate) fn status() -> i32 {
    let reg = match registry::Registry::load() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: cannot read the server registry: {e}");
            return 1;
        }
    };
    if reg.servers.is_empty() {
        println!("No Kora servers registered ({}). Install one with `kora install`.", registry::registry_path().display());
        return 0;
    }
    println!("{:<20} {:<15} {:<7} {:<7} {:<9} {:<26} folder", "NAME", "COPIES, VERSIONS", "GAME", "TXADMIN", "STATE", "PORTS");
    for s in &reg.servers {
        let spec = launch::Spec { name: s.name.clone(), dir: s.dir.clone(), game_port: s.ports.game, txadmin_port: s.ports.txadmin };
        let st = launch::status_of(&launch::SystemEnv::default(), &spec);
        let state = if !s.dir.is_dir() {
            "missing"
        } else if st.running {
            "running"
        } else {
            "stopped"
        };
        let ports = format!("game {}, txAdmin {}", if st.game_listening { "listening" } else { "closed" }, if st.txadmin_up { "up" } else { "down" });
        let style = format!("{}, {}", if s.source() == Source::Clone { "clone" } else { "fork" }, if s.versions() == Versions::Released { "released" } else { "latest" });
        println!("{:<20} {:<15} {:<7} {:<7} {:<9} {:<26} {}", s.name, style, s.ports.game, s.ports.txadmin, state, ports, s.dir.display());
        if s.dir.is_dir() {
            for l in crate::cmd::resource::attention_lines(&s.name, st.running) {
                println!("    {l}");
            }
        }
    }
    0
}

pub(crate) fn control(server: &str, action: kora_core::service::ControlAction) -> i32 {
    use kora_core::service::{ControlAction as A, ServiceState};
    let (verb, past) = match action {
        A::Start => ("start", "Started"),
        A::Stop => ("stop", "Stopped"),
        A::Restart => ("restart", "Restarted"),
    };
    if action != A::Start {
        // So the panel's monitor does not report this stop as a crash.
        if let Some(e) = registry::Registry::load().ok().and_then(|r| r.servers.into_iter().find(|e| e.name == server)) {
            kora_core::monitor::mark_stop_intent(&e.dir);
        }
    }
    let has_service = !matches!(host::service_state(server), Ok(ServiceState::NotInstalled) | Err(_));
    // Without a background service Kora runs FXServer itself (detached, pid in .kora/server.pid).
    if !has_service {
        if matches!(action, A::Stop | A::Restart) {
            match launch::stop_named_detached(server) {
                Ok(true) => println!("Stopped '{server}'."),
                Ok(false) if action == A::Stop => {
                    println!("'{server}' was not running (nothing started by Kora).");
                    return 0;
                }
                Ok(false) => {}
                Err(e) => {
                    eprintln!("error: could not {verb} '{server}': {e}");
                    return 1;
                }
            }
            if action == A::Stop {
                return 0;
            }
        }
        return start_and_report(server, past);
    }
    if action == A::Start {
        return start_and_report(server, past);
    }
    match host::service_control(server, action) {
        Ok(()) => {
            println!("{past} '{server}'.");
            0
        }
        Err(e) => {
            eprintln!("error: could not {verb} '{server}': {e}");
            1
        }
    }
}

/// Start (service or detached), show progress lines and the result.
fn start_and_report(server: &str, past: &str) -> i32 {
    let mut last = String::new();
    let mut progress = |m: &str, _f: f32| {
        if m != last {
            println!("{m}");
            last = m.to_string();
        }
    };
    match launch::start_named(server, &mut progress) {
        Ok(r) => {
            if r.running() {
                println!(
                    "{} '{server}': txAdmin is up{}.",
                    if r.method == launch::Method::AlreadyRunning { "Already running" } else { past },
                    if r.game_listening { ", the game port is listening" } else { ", the game port is not listening yet" }
                );
                0
            } else {
                eprintln!("error: '{server}' did not start properly: {}", r.message);
                for l in &r.log_tail {
                    eprintln!("  | {l}");
                }
                1
            }
        }
        Err(e) => {
            eprintln!("error: could not start '{server}': {e}");
            1
        }
    }
}

pub(crate) fn delete(server: &str, keep_files: bool, keep_db: bool, yes: bool) -> i32 {
    let reg = registry::Registry::load().unwrap_or_default();
    let (name, dir) = match reg.find(server) {
        Some(e) => (e.name.clone(), e.dir.clone()),
        None => (server.to_string(), PathBuf::from(server)),
    };
    if !dir.join(".kora").is_dir() {
        eprintln!("error: '{server}' is not a Kora server folder or registered server");
        return 1;
    }
    if !yes {
        println!(
            "This removes: {}{}; the registry entry. GitHub forks are never deleted.",
            if keep_files { "(files kept)" } else { "the server folder and everything in it" },
            if keep_db { "" } else { "; the database and its user" }
        );
        println!("Type the server name ({name}) to confirm:");
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        if line.trim() != name {
            eprintln!("Cancelled.");
            return 1;
        }
    }
    let mut sink = Printer { last: None };
    match installer::rollback(&dir, &RollbackOptions { keep_files, keep_db }, &mut sink) {
        Ok(notes) => {
            for n in notes {
                println!("note: {n}");
            }
            println!("Deleted '{name}'.");
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}
