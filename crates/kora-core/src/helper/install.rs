//! Installing and removing the helper (Linux: a systemd unit). Plans are plain data so tests can
//! check them; `kora helper install` applies them with the same machinery as other privileged work.

use super::{HelperConfig, CONFIG_PATH, HELPER_EXE, UNIT_NAME, UNIT_PATH};
use crate::service::{Plan, SystemStep};

/// A passwd entry (just what we need).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub home: String,
}

/// Work out who is installing. `sudo_uid` = `$SUDO_UID`; `lookup` = passwd lookup by uid.
/// Refuses root: the helper serves a normal user account.
pub fn resolve_user(sudo_uid: Option<&str>, kora_home: Option<String>, lookup: &dyn Fn(u32) -> Option<Account>) -> Result<HelperConfig, String> {
    let uid: u32 = sudo_uid
        .and_then(|s| s.trim().parse().ok())
        .ok_or("run this from your own account with sudo (`sudo kora helper install`), so Kora knows which user the helper is for")?;
    if uid == 0 {
        return Err("the helper is for a normal user account, not root; run `sudo kora helper install` from your own account".into());
    }
    let a = lookup(uid).ok_or("cannot find your user account")?;
    Ok(HelperConfig { uid, gid: a.gid, user: a.name, home: a.home, kora_home, version: env!("CARGO_PKG_VERSION").to_string() })
}

/// The systemd unit text. The helper needs full root (it runs `kora install`, which uses the package
/// manager, systemctl, useradd and the firewall), so there is no sandboxing here.
pub fn unit_text() -> String {
    format!(
        "[Unit]\nDescription=Kora privileged helper (runs a few allow-listed kora commands for the Kora Panel)\nAfter=local-fs.target\n\n\
         [Service]\nType=simple\nExecStart={HELPER_EXE} helper serve\nRuntimeDirectory=kora\nRuntimeDirectoryMode=0755\nRestart=on-failure\nRestartSec=3\n\n\
         [Install]\nWantedBy=multi-user.target\n"
    )
}

/// Steps to install: root-owned copy of this program, the user record, the unit, start it.
pub fn install_plan(cfg: &HelperConfig, current_exe: &str) -> Plan {
    let dir = HELPER_EXE.rsplit_once('/').map(|x| x.0).unwrap_or("/usr/local/libexec/kora");
    let json = serde_json::to_string_pretty(cfg).unwrap_or_default() + "\n";
    Plan::new(vec![
        SystemStep::run("Stop an older helper (if any)", "systemctl", &["stop", UNIT_NAME]).tolerant(),
        SystemStep::run("Create the folder for the helper's copy of Kora", "install", &["-d", "-m", "0755", "-o", "root", "-g", "root", dir]),
        SystemStep::copy("Copy Kora to a folder only administrators can change", current_exe, HELPER_EXE, 0o755),
        SystemStep::run("Create the helper settings folder", "install", &["-d", "-m", "0755", "-o", "root", "-g", "root", "/etc/kora"]),
        SystemStep::write("Record which user the helper serves", CONFIG_PATH, json, 0o644),
        SystemStep::write("Add the helper service", UNIT_PATH, unit_text(), 0o644),
        SystemStep::run("Tell systemd to read the new service", "systemctl", &["daemon-reload"]),
        SystemStep::run("Start the helper now and at every boot", "systemctl", &["enable", "--now", UNIT_NAME]),
    ])
}

pub fn uninstall_plan() -> Plan {
    Plan::new(vec![
        SystemStep::run("Stop the helper", "systemctl", &["disable", "--now", UNIT_NAME]).tolerant(),
        SystemStep::remove("Remove the helper service", UNIT_PATH),
        SystemStep::run("Tell systemd the service is gone", "systemctl", &["daemon-reload"]).tolerant(),
        SystemStep::remove("Remove the helper's copy of Kora", HELPER_EXE),
        SystemStep::remove_dir_if_empty("Remove the helper's folder", "/usr/local/libexec/kora"),
        SystemStep::remove("Remove the helper settings", CONFIG_PATH),
        SystemStep::remove_dir_if_empty("Remove the helper settings folder", "/etc/kora"),
    ])
}

/// Look a uid up in the passwd database.
#[cfg(unix)]
pub fn lookup_account(uid: u32) -> Option<Account> {
    let mut buf = vec![0u8; 8192];
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut res: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: buffers are valid for the stated sizes; the strings are copied out before `buf` goes away.
    let rc = unsafe { libc::getpwuid_r(uid, &mut pw, buf.as_mut_ptr().cast(), buf.len(), &mut res) };
    if rc != 0 || res.is_null() {
        return None;
    }
    let s = |p: *const libc::c_char| unsafe { std::ffi::CStr::from_ptr(p) }.to_string_lossy().into_owned();
    Some(Account { name: s(pw.pw_name), uid, gid: pw.pw_gid, home: s(pw.pw_dir) })
}

#[cfg(not(unix))]
pub fn lookup_account(_uid: u32) -> Option<Account> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::Action;

    fn me(uid: u32) -> Option<Account> {
        Some(Account { name: "sam".into(), uid, gid: 1001, home: "/home/sam".into() })
    }

    #[test]
    fn root_and_unknown_users_are_refused() {
        assert!(resolve_user(None, None, &me).is_err());
        assert!(resolve_user(Some("0"), None, &me).is_err());
        assert!(resolve_user(Some("abc"), None, &me).is_err());
        assert!(resolve_user(Some("1000"), None, &|_| None).is_err());
        let c = resolve_user(Some("1000"), Some("/k".into()), &me).unwrap();
        assert_eq!((c.uid, c.gid, c.user.as_str(), c.kora_home.as_deref()), (1000, 1001, "sam", Some("/k")));
    }

    #[test]
    fn the_plan_copies_the_program_to_a_root_owned_place_and_starts_the_unit() {
        let c = resolve_user(Some("1000"), None, &me).unwrap();
        let p = install_plan(&c, "/home/sam/.local/bin/kora");
        assert!(p.steps.iter().any(|s| matches!(&s.action, Action::CopyFile { src, dst, mode: Some(0o755) } if src == "/home/sam/.local/bin/kora" && dst == HELPER_EXE)));
        assert!(p.steps.iter().any(|s| matches!(&s.action, Action::WriteFile { path, contents, .. } if path == CONFIG_PATH && contents.contains("\"uid\": 1000"))));
        assert!(p.steps.iter().any(|s| matches!(&s.action, Action::WriteFile { path, contents, .. } if path == UNIT_PATH && contents.contains("ExecStart=/usr/local/libexec/kora/kora helper serve") && contents.contains("RuntimeDirectory=kora"))));
        assert!(p.steps.last().and_then(|s| s.command_line()).is_some_and(|l| l.contains("enable --now kora-helper.service")));
        assert!(p.needs_admin());
        assert!(uninstall_plan().steps.len() >= 5);
    }
}
