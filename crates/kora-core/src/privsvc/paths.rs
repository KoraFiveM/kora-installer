//! Where the service lives and which files it will trust. Constants and pure checks; the OS layers
//! read the real owner / mode of a file and ask these functions.

use std::path::Path;

// ---- Linux ----------------------------------------------------------------------------------
/// The unit installed by the package.
pub const LINUX_UNIT: &str = "kora.service";
/// Root-owned runtime folder (systemd `RuntimeDirectory=kora`, mode 0755) and the socket in it.
pub const LINUX_SOCKET: &str = "/run/kora/service.sock";
/// Protected settings: who may use the service ([`super::policy::Policy`]).
pub const LINUX_POLICY: &str = "/etc/kora/service.json";
/// The ownership ledger ([`super::ownership`]).
pub const LINUX_LEDGER: &str = "/var/lib/kora/owners.json";
/// The audit log ([`super::audit`]).
pub const LINUX_AUDIT: &str = "/var/log/kora/audit.log";
/// Where the program may be (the package, or the tarball installer on other distributions).
pub const LINUX_PROGRAM_DIRS: &[&str] = &["/usr/lib/kora", "/opt/kora"];

// ---- Windows --------------------------------------------------------------------------------
pub const WINDOWS_SERVICE: &str = "KoraService";
pub const WINDOWS_PIPE: &str = r"\\.\pipe\kora-service";
/// The service keeps its files under `%ProgramFiles%\\Kora\\data`: only administrators can create
/// anything in Program Files, so (unlike `%ProgramData%`) a folder cannot be squatted by a user
/// before the installer runs. (`owners.json` stays next to it, where earlier versions put it.)
pub const WINDOWS_DATA_SUBDIR: &str = "data";

/// The files of the Windows service, from `%ProgramFiles%` as Windows reports it (never from the
/// environment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WinFiles {
    pub exe: String,
    pub data_dir: String,
    pub policy: String,
    pub audit: String,
    pub ledger: String,
}

pub fn windows_files(program_files: &str) -> WinFiles {
    let base = format!("{}\\Kora", program_files.trim_end_matches(['\\', '/']));
    WinFiles {
        exe: format!("{base}\\kora.exe"),
        data_dir: format!("{base}\\{WINDOWS_DATA_SUBDIR}"),
        policy: format!("{base}\\{WINDOWS_DATA_SUBDIR}\\service.json"),
        audit: format!("{base}\\{WINDOWS_DATA_SUBDIR}\\audit.log"),
        ledger: format!("{base}\\owners.json"),
    }
}

/// Could a file or folder with this owner and mode be changed by someone other than root?
/// (Owner root, and neither group nor others may write. A sticky bit does not make it safe.)
pub fn writable_by_others(owner_uid: u32, mode: u32) -> bool {
    owner_uid != 0 || mode & 0o022 != 0
}

/// Is `exe` somewhere the service accepts to run from? Linux: under one of [`LINUX_PROGRAM_DIRS`].
/// Windows: `%ProgramFiles%\Kora\` (`program_files` as Windows reports it, not from the environment).
/// A pure text check (case-insensitive and `/` = `\` on Windows); the folders' real permissions are
/// checked separately ([`writable_by_others`], and on Windows the folder's DACL by the installer).
pub fn program_location_ok(exe: &str, windows: bool, program_files: Option<&str>) -> bool {
    if windows {
        let Some(pf) = program_files.filter(|p| !p.is_empty()) else { return false };
        let norm = |s: &str| s.replace('/', "\\").trim_end_matches('\\').to_lowercase();
        let (e, base) = (norm(exe), format!("{}\\kora", norm(pf)));
        e.starts_with(&format!("{base}\\")) && !e.split('\\').any(|p| p == ".." || p == ".")
    } else {
        let p = Path::new(exe);
        p.is_absolute()
            && !p.components().any(|c| matches!(c, std::path::Component::ParentDir | std::path::Component::CurDir))
            && LINUX_PROGRAM_DIRS.iter().any(|d| p.starts_with(d) && p != Path::new(d))
    }
}

/// Panel side (Linux): is the socket really the service's? It must be owned by root.
pub fn socket_owner_ok(owner_uid: u32) -> bool {
    owner_uid == 0
}

/// The environment of a Linux child: built from nothing. System-only `PATH`, the caller's account
/// (looked up by the service from the OS by uid, never from the request), `KORA_SERVICE=1` (turns on
/// the stricter checks in the steps, see [`crate::privsvc::paths::in_service`]'s Linux twin
/// [`in_service`]). No `KORA_*` override, no `GIT_*`, no `LD_*`.
pub fn linux_child_env(user: &str, uid: u32, gid: u32, home: &str, kora_home: Option<&str>) -> Vec<(String, String)> {
    let mut v = vec![
        ("PATH".to_string(), "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string()),
        ("HOME".to_string(), home.to_string()),
        ("USER".to_string(), user.to_string()),
        ("LOGNAME".to_string(), user.to_string()),
        ("SUDO_USER".to_string(), user.to_string()),
        ("SUDO_UID".to_string(), uid.to_string()),
        ("SUDO_GID".to_string(), gid.to_string()),
        ("LANG".to_string(), "C.UTF-8".to_string()),
        (SERVICE_ENV.to_string(), "1".to_string()),
        ("KORA_HELPER".to_string(), "1".to_string()),
    ];
    if let Some(k) = kora_home {
        v.push(("KORA_HOME".to_string(), k.to_string()));
    }
    v
}

/// Set for every child of the service on both systems.
pub const SERVICE_ENV: &str = "KORA_SERVICE";

/// Is this process a child of the service (the stricter checks apply)?
pub fn in_service() -> bool {
    std::env::var(SERVICE_ENV).is_ok_and(|v| v == "1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_root_owned_unshared_files_are_trusted() {
        assert!(!writable_by_others(0, 0o755));
        assert!(!writable_by_others(0, 0o644));
        assert!(writable_by_others(0, 0o775) && writable_by_others(0, 0o757) && writable_by_others(0, 0o777));
        assert!(writable_by_others(1000, 0o755), "owned by a user");
        assert!(writable_by_others(1000, 0o700));
    }

    #[test]
    fn the_program_must_sit_in_the_protected_folder() {
        // The Linux rule uses the host's idea of an absolute path ("/usr/..." is relative on Windows): Unix only.
        #[cfg(unix)]
        {
            assert!(program_location_ok("/usr/lib/kora/kora", false, None));
            assert!(program_location_ok("/opt/kora/bin/kora", false, None));
            for bad in ["/home/jo/.local/bin/kora", "/usr/lib/kora", "/usr/lib/korax/kora", "/usr/lib/kora/../../../tmp/kora", "kora", "/tmp/usr/lib/kora/kora"] {
                assert!(!program_location_ok(bad, false, None), "{bad}");
            }
        }
        let pf = Some(r"C:\Program Files");
        assert!(program_location_ok(r"C:\Program Files\Kora\kora.exe", true, pf));
        assert!(program_location_ok(r"c:/program files/kora/bin/kora.exe", true, pf));
        for bad in [
            r"C:\Users\jo\AppData\Local\Kora\bin\kora.exe",
            r"C:\Program Files\Koralike\kora.exe",
            r"C:\Program Files\Kora",
            r"C:\Program Files\Kora\..\evil\kora.exe",
            r"C:\Program Files (x86)\Kora\kora.exe",
            r"D:\Program Files\Kora\kora.exe",
        ] {
            assert!(!program_location_ok(bad, true, pf), "{bad}");
        }
        assert!(!program_location_ok(r"C:\Program Files\Kora\kora.exe", true, None), "no trusted Program Files: refuse");
    }

    #[test]
    fn the_windows_files_are_all_under_program_files() {
        let f = windows_files(r"C:\Program Files\");
        assert_eq!(f.exe, r"C:\Program Files\Kora\kora.exe");
        assert_eq!(f.policy, r"C:\Program Files\Kora\data\service.json");
        assert_eq!(f.ledger, r"C:\Program Files\Kora\owners.json");
        for p in [&f.exe, &f.data_dir, &f.policy, &f.audit, &f.ledger] {
            assert!(program_location_ok(p, true, Some(r"C:\Program Files")), "{p}");
        }
    }

    #[test]
    fn the_socket_must_be_roots() {
        assert!(socket_owner_ok(0) && !socket_owner_ok(1000));
    }

    #[test]
    fn the_linux_child_environment_is_made_from_nothing() {
        let v = linux_child_env("jo", 1000, 1000, "/home/jo", Some("/home/jo/.kora"));
        let get = |k: &str| v.iter().find(|(n, _)| n == k).map(|(_, x)| x.as_str());
        assert_eq!(get("HOME"), Some("/home/jo"));
        assert_eq!(get("KORA_HOME"), Some("/home/jo/.kora"));
        assert_eq!(get("KORA_SERVICE"), Some("1"));
        assert!(!get("PATH").unwrap().contains("/home") && !get("PATH").unwrap().contains("::") && !get("PATH").unwrap().starts_with(':'));
        for (k, _) in &v {
            assert!(!k.starts_with("GIT_") && !k.starts_with("LD_") && k != "KORA_MARIADB_HOME" && k != "KORA_SELF_UPDATE_API", "{k}");
        }
    }
}
