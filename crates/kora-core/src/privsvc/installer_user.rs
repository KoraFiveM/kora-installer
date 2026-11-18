//! Who started the installer when someone ELSE typed the administrator password? (Windows)
//!
//! A standard account that runs Kora's installer sees a permission prompt and an administrator types their password. The
//! installer then runs as that administrator, so "the account that runs the installer" is the administrator, and the person who
//! actually uses Kora had to be added by hand (`kora service allow <account>`). Windows Installer tells its custom actions which
//! process started the installation (`ClientProcessId`); the elevated side (SYSTEM) asks Windows about that process and, when the
//! answer is sound, allows the person it names.
//!
//! # Threat model
//! The process id is an ARGUMENT, so it is only a question, never an answer: nothing the caller says is believed. The service
//! reads from Windows: the client process's own token (user, elevation, session) and, when that process is elevated (the installer
//! was started with "Run as administrator"), its PARENT's token, which is the program that asked for the elevation (Windows makes the
//! requesting program the parent of an elevated process started through the permission prompt). The decision ([`decide`]) allows a
//! person only when ALL hold: an interactive session (not session 0), an ordinary account (`valid_user_sid`: never SYSTEM, a service,
//! a group or the built-in Administrator group), and the parent and the client are in the same session. When anything is missing
//! or odd, nothing is allowed and today's behaviour stands: the account that runs the installer, plus any `ALLOWUSERS`.
//!
//! Limits, stated plainly: whoever started the installation and got an administrator to approve it can name themselves this way, which
//! is exactly what approving the prompt means; a program that spoofs its parent is still a program that got an administrator to
//! approve it. What this removes is the need to run `kora service allow` by hand for the person at the computer.

use super::identity::valid_user_sid;

/// What Windows says about one process (the thin layer in `privsvc::winclient` reads it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proc {
    /// The process token's user, as `S-1-5-...`.
    pub sid: String,
    /// The token is an elevated administrator token.
    pub elevated: bool,
    /// The Windows session (0 is the services' session: nobody sits there).
    pub session: u32,
    /// The process that started it, when it can be looked at.
    pub parent: Option<Box<Proc>>,
}

/// Why nobody was allowed (said to the person who runs the installer; never a reason to fail the install).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Declined {
    NotInteractive,
    NotAnOrdinaryAccount,
    ParentUnknown,
    SessionsDiffer,
}

impl Declined {
    pub fn text(&self) -> &'static str {
        match self {
            Declined::NotInteractive => "the installer was not started from a signed-in desktop session",
            Declined::NotAnOrdinaryAccount => "the account that started the installer is not an ordinary user account",
            Declined::ParentUnknown => "Windows did not say which program asked for the administrator permission",
            Declined::SessionsDiffer => "the installer and the program that started it are in different Windows sessions",
        }
    }
}

/// The SID to allow for the person who started the installation, or why none.
///
/// * the client is NOT elevated (a standard installer started the install and Windows Installer elevates it itself): the client's own
///   user is the person;
/// * the client IS elevated: it was started through the permission prompt, so the person is the one who owns its parent, provided the
///   parent is not itself elevated (an administrator who simply ran it is already allowed by the installer) and shares the session.
pub fn decide(client: &Proc) -> Result<String, Declined> {
    if client.session == 0 {
        return Err(Declined::NotInteractive);
    }
    let person = if client.elevated {
        let Some(parent) = client.parent.as_deref() else { return Err(Declined::ParentUnknown) };
        if parent.session != client.session {
            return Err(Declined::SessionsDiffer);
        }
        if parent.elevated {
            // An administrator ran it themselves: the account that runs the installer is already allowed.
            return Err(Declined::ParentUnknown);
        }
        parent
    } else {
        client
    };
    if person.session == 0 {
        return Err(Declined::NotInteractive);
    }
    if !valid_user_sid(&person.sid) {
        return Err(Declined::NotAnOrdinaryAccount);
    }
    Ok(person.sid.clone())
}

/// `ClientProcessId` as Windows Installer passes it: a plain number, not 0 and not this process.
pub fn parse_pid(s: &str) -> Option<u32> {
    let t = s.trim();
    t.parse::<u32>().ok().filter(|p| *p != 0 && p.to_string() == t)
}

#[cfg(test)]
mod tests {
    use super::*;

    const JO: &str = "S-1-5-21-1-2-3-1001";
    const ADMIN: &str = "S-1-5-21-1-2-3-500";

    fn p(sid: &str, elevated: bool, session: u32, parent: Option<Proc>) -> Proc {
        Proc { sid: sid.into(), elevated, session, parent: parent.map(Box::new) }
    }

    #[test]
    fn a_standard_installer_names_its_own_user() {
        assert_eq!(decide(&p(JO, false, 2, None)), Ok(JO.to_string()));
    }

    #[test]
    fn an_installer_started_through_the_permission_prompt_names_who_asked_for_it_not_the_administrator() {
        // the elevated installer runs as the administrator who typed the password; the program that asked is Jo's
        let client = p(ADMIN, true, 2, Some(p(JO, false, 2, None)));
        assert_eq!(decide(&client), Ok(JO.to_string()));
    }

    #[test]
    fn nothing_is_allowed_when_the_answer_is_not_sound() {
        // nobody sits in session 0 (services)
        assert_eq!(decide(&p(JO, false, 0, None)), Err(Declined::NotInteractive));
        assert_eq!(decide(&p(ADMIN, true, 0, Some(p(JO, false, 2, None)))), Err(Declined::NotInteractive));
        // elevated and no idea who asked, or the asker is elevated too (an administrator ran it: already allowed), or another session
        assert_eq!(decide(&p(ADMIN, true, 2, None)), Err(Declined::ParentUnknown));
        assert_eq!(decide(&p(ADMIN, true, 2, Some(p(ADMIN, true, 2, None)))), Err(Declined::ParentUnknown));
        assert_eq!(decide(&p(ADMIN, true, 2, Some(p(JO, false, 3, None)))), Err(Declined::SessionsDiffer));
        // the asker in session 0
        assert_eq!(decide(&p(ADMIN, true, 2, Some(p(JO, false, 0, None)))), Err(Declined::SessionsDiffer));
        // never SYSTEM, a service account, a group, or a made-up string
        for bad in ["S-1-5-18", "S-1-5-19", "S-1-5-20", "S-1-5-32-544", "S-1-1-0", "jo", "S-1-5-21-1)(A;;GA;;;WD", ""] {
            assert_eq!(decide(&p(bad, false, 2, None)), Err(Declined::NotAnOrdinaryAccount), "{bad:?}");
            assert_eq!(decide(&p(ADMIN, true, 2, Some(p(bad, false, 2, None)))), Err(Declined::NotAnOrdinaryAccount), "{bad:?} as the asker");
        }
    }

    #[test]
    fn the_process_id_is_a_plain_number() {
        assert_eq!(parse_pid("4321"), Some(4321));
        assert_eq!(parse_pid(" 77 "), Some(77));
        for bad in ["", "0", "-1", "01", "12a", "4294967296", "1 2"] {
            assert_eq!(parse_pid(bad), None, "{bad:?}");
        }
        assert!(Declined::ParentUnknown.text().contains("asked for the administrator permission"));
    }
}
