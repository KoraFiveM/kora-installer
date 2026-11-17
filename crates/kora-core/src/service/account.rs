//! The dedicated, unprivileged account a server's service runs as.
//!
//! Linux: a system user `kora-<name>` (no home, no login shell), created by the installer and
//! removed again on delete/uninstall *only if the installer created it*. Windows: the per-service
//! virtual account `NT SERVICE\kora-<name>` (see [`super::virtual_account`]) needs no creation and
//! disappears with the service, so there is nothing to track.

use super::{CommandRunner, Plan, ServiceError, ServiceResult, SystemStep};

/// Linux user names are limited to 32 characters.
const MAX_USER_LEN: usize = 32;

/// `kora-<slug>`, valid as a Linux account name (lowercase, digits, `-`, at most 32 characters).
pub fn account_name(slug: &str) -> String {
    let mut s = format!("kora-{slug}");
    s.truncate(MAX_USER_LEN);
    s.trim_end_matches('-').to_string()
}

/// Does the account exist, and which uid does it have?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserStatus {
    Exists(u32),
    Missing,
}

/// `id -u <name>` through the runner.
pub fn user_status(runner: &dyn CommandRunner, name: &str) -> ServiceResult<UserStatus> {
    let o = runner.run("id", &["-u".into(), name.into()]).map_err(|e| ServiceError::Command { command: format!("id -u {name}"), detail: e.to_string() })?;
    if !o.success() {
        return Ok(UserStatus::Missing);
    }
    Ok(o.stdout.trim().parse().map(UserStatus::Exists).unwrap_or(UserStatus::Missing))
}

/// Create the system account (Linux only; callers skip this on Windows).
pub fn create_plan(name: &str) -> ServiceResult<Plan> {
    check_name(name)?;
    Ok(Plan::new(vec![SystemStep::run(
        format!("Create the unprivileged system account '{name}' (no home folder, cannot log in) that the server runs as"),
        "useradd",
        &["--system", "--user-group", "--no-create-home", "--home-dir", "/nonexistent", "--shell", "/usr/sbin/nologin", name],
    )]))
}

/// Delete an account the installer created.
pub fn delete_plan(name: &str) -> ServiceResult<Plan> {
    check_name(name)?;
    Ok(Plan::new(vec![SystemStep::run(format!("Delete the system account '{name}' that Kora created for the server"), "userdel", &[name])]))
}

/// First uid of ordinary (login) accounts: `UID_MIN` in `/etc/login.defs`, else 1000.
pub fn uid_min() -> u32 {
    parse_uid_min(&std::fs::read_to_string("/etc/login.defs").unwrap_or_default())
}

pub fn parse_uid_min(login_defs: &str) -> u32 {
    login_defs
        .lines()
        .filter_map(|l| {
            let mut w = l.split_whitespace();
            (w.next() == Some("UID_MIN")).then(|| w.next()?.parse::<u32>().ok()).flatten()
        })
        .next_back()
        .filter(|u| *u > 0)
        .unwrap_or(1000)
}

/// A system account (what `useradd --system` makes): not root, below the ordinary range.
pub fn is_system_uid(uid: u32, uid_min: u32) -> bool {
    uid != 0 && uid < uid_min
}

fn check_name(name: &str) -> ServiceResult<()> {
    let ok = !name.is_empty()
        && name.len() <= MAX_USER_LEN
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if ok && name != "root" {
        Ok(())
    } else {
        Err(ServiceError::Invalid(format!("'{name}' is not a valid service account name")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::testing::FakeRunner;

    #[test]
    fn names_are_prefixed_valid_and_short() {
        assert_eq!(account_name("main"), "kora-main");
        let long = account_name(&"a-".repeat(30));
        assert!(long.len() <= 32 && !long.ends_with('-') && long.starts_with("kora-a-"), "{long}");
        for n in ["kora-main", &long, &account_name("server")] {
            assert!(check_name(n).is_ok(), "{n}");
        }
    }

    #[test]
    fn create_plan_is_exactly_the_documented_useradd() {
        let p = create_plan("kora-main").unwrap();
        assert_eq!(p.steps.len(), 1);
        let line = p.steps[0].command_line().unwrap();
        assert_eq!(line, "useradd --system --user-group --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin kora-main");
        assert!(p.needs_admin());
        assert_eq!(delete_plan("kora-main").unwrap().steps[0].command_line().unwrap(), "userdel kora-main");
    }

    #[test]
    fn root_and_odd_names_are_refused() {
        for bad in ["root", "", "Kora", "-x", "a b", "kora;rm", &"k".repeat(33)] {
            assert!(create_plan(bad).is_err(), "{bad}");
            assert!(delete_plan(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn system_accounts_are_below_uid_min_and_never_root() {
        assert_eq!(parse_uid_min("# x\nUID_MIN\t\t 1000\nUID_MAX 60000\n"), 1000);
        assert_eq!(parse_uid_min("UID_MIN 500"), 500);
        assert_eq!(parse_uid_min(""), 1000);
        assert_eq!(parse_uid_min("UID_MIN nonsense"), 1000);
        assert!(is_system_uid(998, 1000));
        assert!(!is_system_uid(0, 1000), "root");
        assert!(!is_system_uid(1000, 1000), "a person's account");
        assert!(!is_system_uid(65534, 1000), "nobody");
    }

    #[test]
    fn status_reads_id_output() {
        let r = FakeRunner::default();
        r.respond("id -u kora-a", 0, "998\n");
        r.respond("id -u kora-b", 1, "");
        assert_eq!(user_status(&r, "kora-a").unwrap(), UserStatus::Exists(998));
        assert_eq!(user_status(&r, "kora-b").unwrap(), UserStatus::Missing);
    }
}
