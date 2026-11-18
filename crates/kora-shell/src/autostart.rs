//! "Start with Windows" / "Start when I sign in": the exact contents, with no operating-system call.
//!
//! * Windows: a value `Kora` under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` holding the quoted path
//!   of the shell and `--background` (tray only, no window). Per user, no administrator rights; removing the
//!   value is the whole uninstall. Written and read with `reg.exe` (no extra dependency), see
//!   [`reg_add_args`], [`reg_delete_args`], [`reg_query_args`].
//!   Windows' Settings or Task Manager can switch a Run entry off without deleting it: that is the
//!   `StartupApproved` key, read with [`approved_from_reg_query`].
//! * Linux: `~/.config/autostart/kora-desktop.desktop` (XDG autostart), see [`xdg_entry`].
//!
//! The ordinary installer/packaging side of this is in `docs/desktop-app.md`.

use std::path::{Path, PathBuf};

pub const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
pub const APPROVED_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
pub const VALUE_NAME: &str = "Kora";
/// The argument that starts the shell in the tray with no window.
pub const BACKGROUND_ARG: &str = "--background";

/// The command line stored in the Run value.
pub fn windows_run_value(exe: &Path) -> String {
    format!("\"{}\" {BACKGROUND_ARG}", exe.display())
}

pub fn reg_add_args(exe: &Path) -> Vec<String> {
    ["add", RUN_KEY, "/v", VALUE_NAME, "/t", "REG_SZ", "/d"].iter().map(|s| s.to_string()).chain([windows_run_value(exe), "/f".to_string()]).collect()
}

pub fn reg_delete_args() -> Vec<String> {
    ["delete", RUN_KEY, "/v", VALUE_NAME, "/f"].iter().map(|s| s.to_string()).collect()
}

pub fn reg_query_args() -> Vec<String> {
    ["query", RUN_KEY, "/v", VALUE_NAME].iter().map(|s| s.to_string()).collect()
}

pub fn reg_query_approved_args() -> Vec<String> {
    ["query", APPROVED_KEY, "/v", VALUE_NAME].iter().map(|s| s.to_string()).collect()
}

/// Does `reg query` output show our Run value (and which program does it start)?
pub fn run_value_from_reg_query(output: &str) -> Option<String> {
    output.lines().find_map(|l| {
        let l = l.trim();
        let rest = l.strip_prefix(VALUE_NAME)?;
        let rest = rest.trim_start();
        let rest = rest.strip_prefix("REG_SZ").or_else(|| rest.strip_prefix("REG_EXPAND_SZ"))?;
        Some(rest.trim().to_string())
    })
}

/// `false` when Windows' Settings or Task Manager has switched the entry off (`StartupApproved`: the first
/// byte is `02`/`06` for on, `03`/`07` for off). No output at all (the value does not exist) counts as on.
pub fn approved_from_reg_query(output: &str) -> bool {
    for l in output.lines() {
        let l = l.trim();
        if let Some(rest) = l.strip_prefix(VALUE_NAME) {
            if let Some(hex) = rest.trim_start().strip_prefix("REG_BINARY") {
                let hex = hex.trim();
                if let Some(b) = hex.get(..2).and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    return b & 1 == 0;
                }
            }
        }
    }
    true
}

/// Is autostart on, given both query outputs and the program that is running now?
pub fn windows_enabled(run_query: &str, approved_query: &str, exe: &Path) -> bool {
    run_value_from_reg_query(run_query).is_some_and(|v| v == windows_run_value(exe)) && approved_from_reg_query(approved_query)
}

/// Quote one `Exec=` argument the way the Desktop Entry specification wants.
fn exec_arg(p: &Path) -> String {
    let s = p.display().to_string();
    if s.chars().any(|c| c.is_whitespace() || "\"'\\><~|&;$*?#()`".contains(c)) {
        let mut out = String::from("\"");
        for c in s.chars() {
            if "\"`$\\".contains(c) {
                out.push('\\');
            }
            out.push(c);
        }
        out.push('"');
        out.replace('%', "%%")
    } else {
        s.replace('%', "%%")
    }
}

/// The XDG autostart file.
pub fn xdg_entry(exe: &Path) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=Kora\nComment=Kora tray icon: your FiveM servers at a glance\nExec={} {BACKGROUND_ARG}\nTerminal=false\nX-GNOME-Autostart-enabled=true\nX-Kora-Desktop=1\n",
        exec_arg(exe)
    )
}

/// `~/.config/autostart/kora-desktop.desktop` (or `$XDG_CONFIG_HOME/autostart/...`).
pub fn xdg_path(get: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let abs = |k: &str| get(k).filter(|v| !v.is_empty()).map(PathBuf::from).filter(|p| p.is_absolute());
    let base = abs("XDG_CONFIG_HOME").or_else(|| abs("HOME").map(|h| h.join(".config")))?;
    Some(base.join("autostart").join("kora-desktop.desktop"))
}

/// Our own entry? (Only files we wrote are ever removed.)
pub fn xdg_is_ours(text: &str) -> bool {
    text.lines().any(|l| l.trim() == "X-Kora-Desktop=1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_windows_value_quotes_the_program_and_starts_in_the_tray() {
        let exe = Path::new(r"C:\Program Files\Kora\Kora.exe");
        assert_eq!(windows_run_value(exe), r#""C:\Program Files\Kora\Kora.exe" --background"#);
        let a = reg_add_args(exe);
        assert_eq!(a[..7], ["add", RUN_KEY, "/v", "Kora", "/t", "REG_SZ", "/d"]);
        assert_eq!(a[7], r#""C:\Program Files\Kora\Kora.exe" --background"#);
        assert_eq!(a[8], "/f");
        assert_eq!(reg_delete_args(), ["delete", RUN_KEY, "/v", "Kora", "/f"]);
        assert!(RUN_KEY.starts_with("HKCU"), "per user: no administrator rights");
    }

    #[test]
    fn reg_query_output_is_read_the_way_windows_prints_it() {
        let q = "\r\nHKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Run\r\n    Kora    REG_SZ    \"C:\\Program Files\\Kora\\Kora.exe\" --background\r\n\r\n";
        assert_eq!(run_value_from_reg_query(q).as_deref(), Some(r#""C:\Program Files\Kora\Kora.exe" --background"#));
        assert_eq!(run_value_from_reg_query("ERROR: The system was unable to find the specified registry key or value."), None);
        let on = "HKEY_CURRENT_USER\\...\\Run\r\n    Kora    REG_BINARY    020000000000000000000000\r\n";
        let off = "HKEY_CURRENT_USER\\...\\Run\r\n    Kora    REG_BINARY    030000000000000000000000\r\n";
        assert!(approved_from_reg_query(on) && !approved_from_reg_query(off));
        assert!(approved_from_reg_query("ERROR: The system was unable to find the specified registry key or value."), "no approval record = on");
        let exe = Path::new(r"C:\Program Files\Kora\Kora.exe");
        assert!(windows_enabled(q, on, exe));
        assert!(!windows_enabled(q, off, exe), "switched off in Task Manager");
        assert!(!windows_enabled(q, on, Path::new(r"D:\other\Kora.exe")), "points at another copy");
        assert!(!windows_enabled("", on, exe));
    }

    #[test]
    fn the_linux_entry_is_a_valid_desktop_file() {
        let t = xdg_entry(Path::new("/opt/kora/kora-desktop"));
        assert!(t.starts_with("[Desktop Entry]\nType=Application\n") && t.contains("Exec=/opt/kora/kora-desktop --background\n") && t.contains("Terminal=false"), "{t}");
        assert!(xdg_is_ours(&t) && !xdg_is_ours("[Desktop Entry]\nName=Other\n"));
        let t = xdg_entry(Path::new("/home/a b/100%/kora-desktop"));
        assert!(t.contains("Exec=\"/home/a b/100%%/kora-desktop\" --background"), "{t}");
    }

    // Linux path rules use the host's notion of an absolute path ("/home/sam" is relative on Windows), so this runs on Unix only.
    #[cfg(unix)]
    #[test]
    fn the_linux_path_follows_xdg() {
        let env = |k: &str| match k {
            "HOME" => Some("/home/sam".to_string()),
            _ => None,
        };
        assert_eq!(xdg_path(&env), Some(PathBuf::from("/home/sam/.config/autostart/kora-desktop.desktop")));
        let xdg = |k: &str| match k {
            "XDG_CONFIG_HOME" => Some("/cfg".to_string()),
            "HOME" => Some("/home/sam".to_string()),
            _ => None,
        };
        assert_eq!(xdg_path(&xdg), Some(PathBuf::from("/cfg/autostart/kora-desktop.desktop")));
        assert_eq!(xdg_path(&|_| None), None);
        assert_eq!(xdg_path(&|k| (k == "HOME").then(|| "relative".to_string())), None, "relative homes are not trusted");
    }
}
