//! Account identities as text: ordinary Windows user SIDs, and the SYSTEM check the panel makes on the service's pipe.

/// The machine account.
pub const SYSTEM_SID: &str = "S-1-5-18";

/// A user SID as Windows writes it, and an ordinary account's: `S-1-5-21-<3 numbers>-<RID>`
/// (local or domain account) or `S-1-12-1-<4 numbers>` (Microsoft Entra ID account). SYSTEM,
/// groups and well-known SIDs are refused.
pub fn valid_user_sid(s: &str) -> bool {
    let Some(rest) = s.strip_prefix("S-1-5-21-").or_else(|| s.strip_prefix("S-1-12-1-")) else { return false };
    let parts: Vec<&str> = rest.split('-').collect();
    parts.len() == 4 && parts.iter().all(|p| !p.is_empty() && p.len() <= 10 && p.bytes().all(|b| b.is_ascii_digit()))
}

/// Panel side: is the pipe really the service's (owned by SYSTEM)?
pub fn check_server_owner(owner_sid: Option<&str>) -> Result<(), String> {
    match owner_sid {
        Some(s) if s.eq_ignore_ascii_case(SYSTEM_SID) => Ok(()),
        Some(s) => Err(format!("the service's pipe belongs to {s}, not to Windows (SYSTEM): another program may be pretending to be the service")),
        None => Err("cannot see who made the service's pipe".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_ordinary_accounts_are_user_sids() {
        for ok in ["S-1-5-21-1-2-3-1001", "S-1-12-1-1-2-3-4", "S-1-5-21-4294967295-1-2-500"] {
            assert!(valid_user_sid(ok), "{ok}");
        }
        for bad in ["", "S-1-5-18", "S-1-5-32-544", "S-1-1-0", "S-1-5-21-1-2-3", "S-1-5-21-1-2-3-4-5", "S-1-5-21-a-2-3-4", "s-1-5-21-1-2-3-4", "S-1-5-21-12345678901-2-3-4"] {
            assert!(!valid_user_sid(bad), "{bad}");
        }
    }

    #[test]
    fn the_pipe_must_be_owned_by_system() {
        assert!(check_server_owner(Some("S-1-5-18")).is_ok());
        assert!(check_server_owner(Some("s-1-5-18")).is_ok());
        assert!(check_server_owner(Some("S-1-5-21-1-2-3-1001")).unwrap_err().contains("pretending"));
        assert!(check_server_owner(None).is_err());
    }
}
