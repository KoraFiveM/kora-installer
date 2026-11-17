//! Small platform facts the GUI layer needs, as pure data: what to run for the one administrator action, how
//! the notification identity is registered on Windows, and what to say when WebView2 is missing.

use std::path::{Path, PathBuf};

use crate::bridge::ElevatedAction;

/// The Windows notification identity (AppUserModelID). Toasts from a program that has no registered identity may
/// be dropped by Windows, so the shell registers this one under `HKCU\Software\Classes\AppUserModelId`.
pub const AUMID: &str = "Kora.Desktop";
const AUMID_KEY: &str = r"HKCU\Software\Classes\AppUserModelId\Kora.Desktop";

/// `reg.exe` arguments that register the display name (the first of the two), then the icon (the second).
pub fn aumid_register_args(icon_png: Option<&Path>) -> Vec<Vec<String>> {
    let mut v = vec![["add", AUMID_KEY, "/v", "DisplayName", "/t", "REG_SZ", "/d", "Kora", "/f"].iter().map(|s| s.to_string()).collect::<Vec<_>>()];
    if let Some(p) = icon_png {
        v.push(["add", AUMID_KEY, "/v", "IconUri", "/t", "REG_SZ", "/d", &p.display().to_string(), "/f"].iter().map(|s| s.to_string()).collect());
    }
    v
}

pub fn aumid_unregister_args() -> Vec<String> {
    ["delete", AUMID_KEY, "/f"].iter().map(|s| s.to_string()).collect()
}

/// A program and its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub program: PathBuf,
    pub args: Vec<String>,
}

/// What the shell runs for a fixed administrator action. The page chooses only the action, never the program or
/// the arguments.
///
/// SEAM (privileged-service workstream): `InstallService` is "Install Kora's administrator service", which that
/// workstream ships as `kora service install` (it fetches or finds the installer, checks it and launches the
/// MSI). If its command line differs at merge, change it HERE and nowhere else.
pub fn elevated_command(action: ElevatedAction, kora: &Path) -> Command {
    match action {
        ElevatedAction::InstallService => Command { program: kora.to_path_buf(), args: vec!["service".into(), "install".into()] },
    }
}

/// How to turn Kora's background panel off (the tray's "Quit and turn off Kora's background service").
pub fn stop_service_command(kora: &Path) -> Command {
    Command { program: kora.to_path_buf(), args: vec!["panel".into(), "--remove-service".into()] }
}

/// The command line for `ShellExecuteExW`'s parameters: each argument quoted when needed.
pub fn quote_args(args: &[String]) -> String {
    args.iter().map(|a| if a.is_empty() || a.chars().any(|c| c.is_whitespace() || c == '"') { format!("\"{}\"", a.replace('"', "\\\"")) } else { a.clone() }).collect::<Vec<_>>().join(" ")
}

/// Where a missing WebView2 is fetched (Microsoft's "Evergreen Bootstrapper": a small installer that needs no
/// administrator rights for a per-user install and keeps itself current).
pub const WEBVIEW2_DOWNLOAD: &str = "https://go.microsoft.com/fwlink/p/?LinkId=2124703";

/// What to tell a person whose computer has no WebView2 (or whose Linux has no WebKitGTK). Always ends with the
/// browser route, which needs nothing.
pub fn webview_missing_message(windows: bool) -> Vec<String> {
    if windows {
        vec![
            "Kora's window needs Microsoft Edge WebView2, which this computer does not have yet. Windows 10 and 11 normally include it.".into(),
            format!("You can get it free from Microsoft: {WEBVIEW2_DOWNLOAD}"),
            "Until then Kora opens in your web browser instead, and everything works the same.".into(),
        ]
    } else {
        vec![
            "Kora's window needs WebKitGTK (the package is often called libwebkit2gtk-4.1), which this computer does not have.".into(),
            "Until you install it Kora opens in your web browser instead, and everything works the same.".into(),
        ]
    }
}

/// The `kora` program that sits next to the desktop app (`kora.exe` on Windows); `None` when `exists` says it is not.
pub fn kora_beside(shell_exe: &Path, windows: bool, exists: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
    let name = if windows { "kora.exe" } else { "kora" };
    let candidate = shell_exe.parent()?.join(name);
    exists(&candidate).then_some(candidate)
}

/// The shell's own program name for the browser fallback hint and window title.
pub fn window_title(version: &str) -> String {
    if version.is_empty() {
        "Kora".into()
    } else {
        format!("Kora {version}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_notification_identity_is_registered_per_user() {
        let a = aumid_register_args(Some(Path::new(r"C:\Program Files\Kora\kora.png")));
        assert_eq!(a.len(), 2);
        assert!(a[0].iter().all(|x| !x.contains("HKLM")) && a[0][1].starts_with(r"HKCU\Software\Classes\AppUserModelId\Kora.Desktop"));
        assert_eq!(a[1][3..8], ["IconUri", "/t", "REG_SZ", "/d", r"C:\Program Files\Kora\kora.png"]);
        assert_eq!(aumid_register_args(None).len(), 1);
        assert_eq!(aumid_unregister_args(), ["delete", AUMID_KEY, "/f"]);
    }

    #[test]
    fn the_administrator_action_is_a_fixed_command_the_page_cannot_change() {
        let c = elevated_command(ElevatedAction::InstallService, Path::new(r"C:\Program Files\Kora\kora.exe"));
        assert_eq!(c, Command { program: PathBuf::from(r"C:\Program Files\Kora\kora.exe"), args: vec!["service".into(), "install".into()] });
        assert_eq!(stop_service_command(Path::new("/usr/bin/kora")).args, ["panel", "--remove-service"]);
    }

    #[test]
    fn arguments_are_quoted_for_windows() {
        assert_eq!(quote_args(&["service".into(), "install".into()]), "service install");
        assert_eq!(quote_args(&["a b".into(), "".into(), "say \"hi\"".into()]), "\"a b\" \"\" \"say \\\"hi\\\"\"");
    }

    #[test]
    fn kora_is_found_next_to_the_app() {
        let here = |p: &Path| p == Path::new("/opt/kora/kora");
        assert_eq!(kora_beside(Path::new("/opt/kora/kora-desktop"), false, &here), Some(PathBuf::from("/opt/kora/kora")));
        assert_eq!(kora_beside(Path::new("/elsewhere/kora-desktop"), false, &here), None);
    }

    #[test]
    fn a_missing_webview_is_explained_and_the_browser_is_offered() {
        let w = webview_missing_message(true).join(" ");
        assert!(w.contains("WebView2") && w.contains("LinkId=2124703") && w.contains("web browser instead"), "{w}");
        let l = webview_missing_message(false).join(" ");
        assert!(l.contains("WebKitGTK") && l.contains("web browser instead"), "{l}");
        assert_eq!(window_title("0.8.0"), "Kora 0.8.0");
        assert_eq!(window_title(""), "Kora");
    }
}
