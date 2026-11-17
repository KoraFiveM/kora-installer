//! The shell's own small preferences, in `shell.json` in the shell's state folder.
//!
//! These belong to this device's app (what the window did last, whether the first-time hint was shown,
//! pause/quiet hours), not to Kora's servers or accounts, so they are NOT in the Kora store (which the panel
//! owns and the shell never opens: the shell talks to the panel only through its API). A missing or damaged
//! file simply means the defaults.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::toast::Quiet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Settings {
    /// "Kora keeps running in the tray" has been shown once.
    pub close_hint_shown: bool,
    /// The first run has turned "start with Windows" on (so a person who turns it off is not overruled).
    pub autostart_default_applied: bool,
    /// Notifications are paused until this Unix time.
    pub pause_until: Option<u64>,
    /// Quiet hours, minutes after midnight.
    pub quiet_start: Option<u16>,
    pub quiet_end: Option<u16>,
    pub window_width: Option<u32>,
    pub window_height: Option<u32>,
}

impl Settings {
    pub fn quiet(&self) -> Option<Quiet> {
        match (self.quiet_start, self.quiet_end) {
            (Some(s), Some(e)) if s < 1440 && e < 1440 => Some(Quiet { start: s, end: e }),
            _ => None,
        }
    }

    /// The window size to open at: the saved one when it is sensible, else a comfortable default.
    pub fn window_size(&self) -> (u32, u32) {
        match (self.window_width, self.window_height) {
            (Some(w), Some(h)) if (640..=7680).contains(&w) && (480..=4320).contains(&h) => (w, h),
            _ => (1100, 760),
        }
    }

    pub fn load(dir: &Path) -> Settings {
        std::fs::read(dir.join("shell.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    /// Write it (best effort is the caller's choice: the error says why it failed).
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join("shell.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?)?;
        std::fs::rename(tmp, dir.join("shell.json"))
    }
}

/// The shell's state folder: `%LOCALAPPDATA%\Kora\desktop`, or `$XDG_STATE_HOME/kora/desktop`
/// (`~/.local/state/kora/desktop`). `KORA_DESKTOP_HOME` overrides it (tests, the CI smoke run).
pub fn state_dir(windows: bool, get: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let abs = |k: &str| get(k).filter(|v| !v.is_empty()).map(PathBuf::from).filter(|p| p.is_absolute() || (windows && p.to_string_lossy().chars().nth(1) == Some(':')));
    if let Some(d) = abs("KORA_DESKTOP_HOME") {
        return Some(d);
    }
    if windows {
        let local = abs("LOCALAPPDATA").or_else(|| abs("USERPROFILE").map(|p| PathBuf::from(format!("{}\\AppData\\Local", p.display().to_string().trim_end_matches('\\')))))?;
        Some(PathBuf::from(format!("{}\\Kora\\desktop", local.display().to_string().trim_end_matches('\\'))))
    } else {
        let base = abs("XDG_STATE_HOME").or_else(|| abs("HOME").map(|h| h.join(".local").join("state")))?;
        Some(base.join("kora").join("desktop"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_survive_missing_damaged_and_partial_files() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(Settings::load(d.path()), Settings::default());
        std::fs::write(d.path().join("shell.json"), "{ not json").unwrap();
        assert_eq!(Settings::load(d.path()), Settings::default());
        std::fs::write(d.path().join("shell.json"), r#"{"close_hint_shown":true,"future":1}"#).unwrap();
        let s = Settings::load(d.path());
        assert!(s.close_hint_shown && !s.autostart_default_applied);
    }

    #[test]
    fn it_saves_and_loads() {
        let d = tempfile::tempdir().unwrap();
        let s = Settings { close_hint_shown: true, pause_until: Some(5), quiet_start: Some(1320), quiet_end: Some(420), window_width: Some(900), window_height: Some(700), ..Settings::default() };
        s.save(&d.path().join("nested")).unwrap();
        assert_eq!(Settings::load(&d.path().join("nested")), s);
        assert_eq!(s.quiet(), Some(Quiet { start: 1320, end: 420 }));
        assert_eq!(s.window_size(), (900, 700));
    }

    #[test]
    fn nonsense_sizes_and_quiet_hours_are_ignored() {
        let s = Settings { window_width: Some(10), window_height: Some(100000), quiet_start: Some(5000), quiet_end: Some(1), ..Settings::default() };
        assert_eq!(s.window_size(), (1100, 760));
        assert_eq!(s.quiet(), None);
    }

    #[test]
    fn the_state_folder_follows_each_system() {
        let win = |k: &str| match k {
            "LOCALAPPDATA" => Some(r"C:\Users\Sam\AppData\Local".to_string()),
            _ => None,
        };
        assert_eq!(state_dir(true, &win), Some(PathBuf::from(r"C:\Users\Sam\AppData\Local\Kora\desktop")));
        let profile = |k: &str| (k == "USERPROFILE").then(|| r"C:\Users\Sam".to_string());
        assert_eq!(state_dir(true, &profile), Some(PathBuf::from(r"C:\Users\Sam\AppData\Local\Kora\desktop")));
    }

    // "/home/sam" is a relative path on Windows, so the Linux rules are checked on Unix only.
    #[cfg(unix)]
    #[test]
    fn the_linux_state_folder_follows_xdg() {
        let lin = |k: &str| (k == "HOME").then(|| "/home/sam".to_string());
        assert_eq!(state_dir(false, &lin), Some(PathBuf::from("/home/sam/.local/state/kora/desktop")));
        let xdg = |k: &str| match k {
            "XDG_STATE_HOME" => Some("/st".to_string()),
            "HOME" => Some("/home/sam".to_string()),
            _ => None,
        };
        assert_eq!(state_dir(false, &xdg), Some(PathBuf::from("/st/kora/desktop")));
        let over = |k: &str| (k == "KORA_DESKTOP_HOME").then(|| "/tmp/x".to_string());
        assert_eq!(state_dir(false, &over), Some(PathBuf::from("/tmp/x")));
        assert_eq!(state_dir(false, &|_| None), None);
    }
}
