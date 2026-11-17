//! After a restore or an import has written files into a server folder as the person running Kora, a server that runs
//! under its own service account must be handed those files again (`installer::handover` does it, as root / SYSTEM).
//! This is the person's side: decide whether it is needed, ask the administrator service (or do it directly when this
//! process is already elevated), and say plainly what happened. When it cannot be done the restore still stands; the
//! person is told, with the command that does it.

use std::path::Path;

use crate::privsvc::client;
use crate::privsvc::problems::Availability;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandOver {
    /// The server does not run under an account of its own.
    NotNeeded,
    /// Done: what to tell the person.
    Done(String),
    /// Could not be done: one sentence that ends with what to do (the command).
    Failed(String),
}

/// Does the server in `dir` run under a service account of its own (Linux account / Windows service)?
pub fn needed(dir: &Path) -> bool {
    let Some(c) = crate::installer::load_created_from(dir) else { return false };
    if cfg!(windows) {
        c.service_unit.is_some()
    } else {
        c.service_unit.is_some() || c.service_account.is_some()
    }
}

/// The command a person runs in an administrator terminal.
pub fn command_for(dir: &Path) -> String {
    if cfg!(windows) {
        format!("kora hand-over \"{}\"   (in a terminal opened as administrator)", dir.display())
    } else {
        format!("sudo kora hand-over '{}'", dir.display())
    }
}

/// Hand the files over. Never fails the caller: the outcome says what to tell the person.
pub fn after_write(dir: &Path) -> HandOver {
    if !needed(dir) {
        return HandOver::NotNeeded;
    }
    let cmd = command_for(dir);
    if crate::elevate::is_elevated() {
        return match crate::installer::handover::run(dir) {
            Ok(crate::installer::handover::Outcome::NotNeeded(_)) => HandOver::NotNeeded,
            Ok(crate::installer::handover::Outcome::Done(m)) => HandOver::Done(m),
            Err(e) => HandOver::Failed(format!("The restored files could not be handed to the server's own account ({e}). Run this and then start the server: {cmd}")),
        };
    }
    match client::availability() {
        Availability::Ready => {}
        Availability::Missing => {
            return HandOver::Failed(format!(
                "The server runs under its own account, and Kora's administrator service is not installed, so the restored files could not be handed to that account. The server may not be able to use them until you run: {cmd}"
            ))
        }
        Availability::NotAllowed(why) => {
            return HandOver::Failed(format!("The server runs under its own account, but Kora's administrator service does not serve your account ({why}), so the restored files could not be handed over. Run: {cmd}"))
        }
    }
    let mut last = String::new();
    match client::call("hand-over", &[dir.to_string_lossy().into_owned()], &[], &mut |l| last = l.to_string()) {
        Ok(a) if a.ok => HandOver::Done("The restored files now belong to the server's own account.".into()),
        Ok(_) => HandOver::Failed(format!("The restored files could not be handed to the server's own account{}. Run: {cmd}", if last.is_empty() { String::new() } else { format!(" ({last})") })),
        Err(client::ClientError::Refused(m)) => HandOver::Failed(format!("The restored files could not be handed to the server's own account: {m} Run: {cmd}")),
        Err(client::ClientError::Unavailable(m) | client::ClientError::Broken(m)) => HandOver::Failed(format!("The restored files could not be handed to the server's own account ({m}). Run: {cmd}")),
    }
}

/// Put the outcome into a report's notes / problems.
pub fn note_into(outcome: HandOver, notes: &mut Vec<String>, problems: &mut Vec<String>) {
    match outcome {
        HandOver::NotNeeded => {}
        HandOver::Done(m) => notes.push(m),
        HandOver::Failed(m) => problems.push(m),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::installer::Created;
    use crate::privsvc::client::testing::{Scripted, SERIAL};

    #[cfg(unix)]
    fn server_with_account() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let meta = d.path().join(".kora");
        std::fs::create_dir_all(&meta).unwrap();
        let c = Created { service_unit: Some("/etc/systemd/system/kora-x.service".into()), service_account: Some("kora-x".into()), ..Default::default() };
        std::fs::write(meta.join("created.json"), serde_json::to_vec(&c).unwrap()).unwrap();
        d
    }

    #[test]
    fn a_server_without_an_account_of_its_own_is_left_alone_and_the_service_is_not_asked() {
        let _g = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let svc = Scripted::ready(true);
        client::set_override(Some(svc.clone()));
        let d = tempfile::tempdir().unwrap();
        assert_eq!(after_write(d.path()), HandOver::NotNeeded);
        client::set_override(None);
        assert!(svc.requests().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn the_service_is_asked_for_exactly_this_folder_and_nothing_else() {
        if crate::elevate::is_elevated() {
            return; // a root test run would do the real thing
        }
        let _g = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let d = server_with_account();
        let svc = Scripted::ready(true);
        client::set_override(Some(svc.clone()));
        let r = after_write(d.path());
        client::set_override(None);
        assert!(matches!(&r, HandOver::Done(m) if m.contains("belong")), "{r:?}");
        assert_eq!(svc.requests(), [("hand-over".to_string(), vec![d.path().to_string_lossy().into_owned()])]);
    }

    #[cfg(unix)]
    #[test]
    fn without_the_service_the_restore_says_so_and_shows_the_command() {
        if crate::elevate::is_elevated() {
            return;
        }
        let _g = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let d = server_with_account();
        client::set_override(Some(Scripted::missing()));
        let r = after_write(d.path());
        client::set_override(None);
        let HandOver::Failed(m) = r else { panic!("{r:?}") };
        assert!(m.contains("administrator service is not installed") && m.contains("sudo kora hand-over '") && m.contains(&d.path().display().to_string()), "{m}");
        let (mut notes, mut problems) = (vec![], vec![]);
        note_into(HandOver::Failed(m.clone()), &mut notes, &mut problems);
        assert_eq!((notes.len(), problems), (0, vec![m]));
    }

    #[cfg(unix)]
    #[test]
    fn a_refusal_or_a_failed_step_is_reported_not_hidden() {
        if crate::elevate::is_elevated() {
            return;
        }
        let _g = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let d = server_with_account();
        client::set_override(Some(Scripted::ready(false)));
        let r = after_write(d.path());
        client::set_override(None);
        assert!(matches!(r, HandOver::Failed(m) if m.contains("could not be handed")));
    }
}
