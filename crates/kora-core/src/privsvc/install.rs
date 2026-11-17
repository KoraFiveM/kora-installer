//! Installing and removing the service on Linux without the `.deb` (`kora service install`, for other
//! distributions and for per-user installs that want the service): the same layout the package makes.
//!
//! ```text
//! /usr/lib/kora/kora          the program (root-owned, 0755), verified against the release checksum
//! /etc/systemd/system/kora.service  (the package puts it in /lib/systemd/system)
//! /etc/kora/service.json      who may use it ([`super::policy`])
//! /var/lib/kora/owners.json   who owns what ([`super::ownership`])
//! /var/log/kora/audit.log     [`super::audit`]
//! ```
//!
//! The plans are plain data (tests check them); the CLI applies them with the same machinery as
//! other administrator work, after downloading and verifying the program with the self-update path.

use super::paths;
use crate::service::{Plan, SystemStep};

/// Where the program goes (the first of [`paths::LINUX_PROGRAM_DIRS`]).
pub const PROGRAM_DIR: &str = "/usr/lib/kora";
pub const PROGRAM: &str = "/usr/lib/kora/kora";
/// Where `kora service install` puts the unit (the package uses `/lib/systemd/system/kora.service`).
pub const UNIT_PATH: &str = "/etc/systemd/system/kora.service";

/// The systemd unit (one source of truth: the `.deb` is built from `kora service unit`).
pub fn unit_text() -> String {
    format!(
        "[Unit]\nDescription=Kora administrator service (does a short list of administrator jobs for Kora Panel)\nDocumentation=https://github.com/KoraFiveM/kora-installer/blob/main/SECURITY.md\nAfter=local-fs.target\n\n\
         [Service]\nType=simple\nExecStart={PROGRAM} service serve\nRuntimeDirectory=kora\nRuntimeDirectoryMode=0755\nStateDirectory=kora\nStateDirectoryMode=0755\nLogsDirectory=kora\nLogsDirectoryMode=0750\nUMask=0022\nPrivateTmp=true\nRestart=on-failure\nRestartSec=3\n\n\
         [Install]\nWantedBy=multi-user.target\n"
    )
}

/// Steps before the program is copied in, and after (the caller puts the verified program at
/// [`PROGRAM`] in between, and writes the settings file).
pub fn install_plan() -> (Plan, Plan) {
    let before = Plan::new(vec![
        SystemStep::run("Stop an earlier service (if any)", "systemctl", &["stop", paths::LINUX_UNIT]).tolerant(),
        SystemStep::run("Create Kora's protected program folder", "install", &["-d", "-m", "0755", "-o", "root", "-g", "root", PROGRAM_DIR]),
        SystemStep::run("Create the settings folder", "install", &["-d", "-m", "0755", "-o", "root", "-g", "root", "/etc/kora"]),
        SystemStep::run("Create the folder for the ownership records", "install", &["-d", "-m", "0755", "-o", "root", "-g", "root", "/var/lib/kora"]),
    ]);
    let after = Plan::new(vec![
        SystemStep::write("Add the Kora administrator service", UNIT_PATH, unit_text(), 0o644),
        SystemStep::run("Tell systemd to read the new service", "systemctl", &["daemon-reload"]),
        SystemStep::run("Start the service now and at every boot", "systemctl", &["enable", "--now", paths::LINUX_UNIT]),
    ]);
    (before, after)
}

/// Remove the service. `purge`: also the settings, ownership records and audit log (a package
/// `remove` keeps them, `purge` does not).
pub fn uninstall_plan(purge: bool) -> Plan {
    let mut steps = vec![
        SystemStep::run("Stop the service", "systemctl", &["disable", "--now", paths::LINUX_UNIT]).tolerant(),
        SystemStep::remove("Remove the service", UNIT_PATH),
        SystemStep::run("Tell systemd the service is gone", "systemctl", &["daemon-reload"]).tolerant(),
        SystemStep::remove("Remove the program", PROGRAM),
        SystemStep::remove_dir_if_empty("Remove the program folder", PROGRAM_DIR),
    ];
    if purge {
        steps.extend([
            SystemStep::remove("Remove the list of allowed accounts", paths::LINUX_POLICY),
            SystemStep::remove("Remove the ownership records", paths::LINUX_LEDGER),
            SystemStep::remove("Remove the audit log", paths::LINUX_AUDIT),
            SystemStep::remove_dir_if_empty("Remove the settings folder", "/etc/kora"),
            SystemStep::remove_dir_if_empty("Remove the records folder", "/var/lib/kora"),
            SystemStep::remove_dir_if_empty("Remove the log folder", "/var/log/kora"),
        ]);
    }
    Plan::new(steps)
}

/// The account that ran `sudo` (to be enrolled), from `SUDO_UID` / `PKEXEC_UID`: never root, never
/// anything that is not a plain number.
pub fn enrolling_uid(sudo_uid: Option<&str>, pkexec_uid: Option<&str>) -> Option<u32> {
    [sudo_uid, pkexec_uid].into_iter().flatten().find_map(|s| s.trim().parse::<u32>().ok().filter(|u| *u != 0 && *u != u32::MAX && u.to_string() == s.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::Action;

    #[test]
    fn the_unit_runs_the_protected_program_as_root_with_its_own_runtime_folder() {
        let u = unit_text();
        assert!(u.contains("ExecStart=/usr/lib/kora/kora service serve"));
        assert!(u.contains("RuntimeDirectory=kora") && u.contains("RuntimeDirectoryMode=0755"));
        assert!(!u.contains("User="), "the service is root: it makes system services and accounts");
        assert!(paths::program_location_ok(PROGRAM, false, None), "the service accepts to run from where it is installed");
        assert_eq!(UNIT_PATH.rsplit('/').next(), Some(paths::LINUX_UNIT));
    }

    #[test]
    fn install_makes_root_owned_folders_then_the_unit_and_starts_it() {
        let (before, after) = install_plan();
        let lines: Vec<String> = before.steps.iter().filter_map(|s| s.command_line()).collect();
        assert!(lines.iter().any(|l| l == "install -d -m 0755 -o root -g root /usr/lib/kora"));
        assert!(before.steps[0].ignore_failure, "an earlier service may not exist");
        assert!(after.steps.iter().any(|s| matches!(&s.action, Action::WriteFile { path, contents, mode: Some(0o644) } if path == UNIT_PATH && contents.contains("service serve"))));
        assert!(after.steps.last().and_then(|s| s.command_line()).is_some_and(|l| l == "systemctl enable --now kora.service"));
        assert!(before.needs_admin() && after.needs_admin());
    }

    #[test]
    fn uninstall_leaves_the_records_unless_purging() {
        let keep: Vec<String> = uninstall_plan(false).steps.iter().map(|s| s.description.clone()).collect();
        assert!(!keep.iter().any(|d| d.contains("ownership") || d.contains("allowed accounts")));
        let all = uninstall_plan(true);
        let d: Vec<String> = all.steps.iter().map(|s| s.description.clone()).collect();
        for want in ["Remove the list of allowed accounts", "Remove the ownership records", "Remove the audit log", "Remove the settings folder"] {
            assert!(d.iter().any(|x| x == want), "{want}");
        }
        assert!(all.steps.iter().all(|s| s.command_line().is_none_or(|l| !l.contains("rm -r"))), "no recursive removal");
    }

    #[test]
    fn only_the_sudo_user_is_enrolled() {
        assert_eq!(enrolling_uid(Some("1000"), None), Some(1000));
        assert_eq!(enrolling_uid(None, Some("1001")), Some(1001));
        for bad in ["0", "", "abc", "-1", "01", "4294967295", "1000 ; id"] {
            assert_eq!(enrolling_uid(Some(bad), None), None, "{bad:?}");
        }
        assert_eq!(enrolling_uid(Some("0"), Some("1002")), Some(1002), "root's sudo falls through to the next source");
    }
}
