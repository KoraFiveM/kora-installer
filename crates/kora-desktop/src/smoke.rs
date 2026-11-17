//! The automated check of the CI (`.github/workflows/desktop.yml`): `kora-desktop --smoke <report.json>`.
//!
//! Start against a throw-away Kora home, wait until the window has loaded the panel and the panel's event stream
//! is connected, read the page title, drive one tray action (open the settings page), write a JSON report, and
//! quit. The report says what was seen so that the workflow can assert it. If `KORA_DESKTOP_SMOKE_WAIT_SECOND`
//! is set it also waits for a second launch (single instance) before it quits; `<report>.ready` is written when
//! the window is up so the workflow knows when to start that second launch.

use std::path::PathBuf;
use std::time::{Duration, Instant};

pub struct Smoke {
    report: PathBuf,
    started: Instant,
    loaded: bool,
    asked: bool,
    title: Option<String>,
    drove: bool,
    action_url: Option<String>,
    wait_second: bool,
    pub second_launch: Option<String>,
    pub menu_ids: Vec<String>,
    pub health: String,
}

impl Smoke {
    pub fn new(report: PathBuf) -> Smoke {
        Smoke {
            report,
            started: Instant::now(),
            loaded: false,
            asked: false,
            title: None,
            drove: false,
            action_url: None,
            wait_second: std::env::var_os("KORA_DESKTOP_SMOKE_WAIT_SECOND").is_some(),
            second_launch: None,
            menu_ids: Vec::new(),
            health: String::new(),
        }
    }

    pub fn timed_out(&self) -> bool {
        self.started.elapsed() > Duration::from_secs(120)
    }

    pub fn page_loaded(&mut self, url: &str) {
        if self.drove && url.contains("/settings") {
            self.action_url = Some(url.to_string());
        } else if !self.drove {
            self.loaded = true;
        }
    }

    /// Time to read the page title (once the page is loaded and the stream is connected).
    pub fn should_ask_title(&mut self, connected: bool) -> bool {
        if self.loaded && connected && !self.asked {
            self.asked = true;
            return true;
        }
        false
    }

    /// The title arrived: `true` = now drive the action.
    pub fn got_title(&mut self, title: String) -> bool {
        self.title = Some(title);
        if self.drove {
            return false;
        }
        self.drove = true;
        let _ = std::fs::write(format!("{}.ready", self.report.display()), "ready");
        true
    }

    pub fn finished(&self) -> bool {
        self.action_url.is_some() && (!self.wait_second || self.second_launch.is_some())
    }

    pub fn write(&self, ok: bool, error: &str, url: Option<&str>, window_title: Option<&str>, connected: bool, tray_built: bool) {
        let report = serde_json::json!({
            "ok": ok,
            "error": error,
            "window_title": window_title,
            "page_title": self.title,
            "url_before_action": url,
            "action": "open:/settings",
            "action_url": self.action_url,
            "connected": connected,
            "health": self.health,
            "tray_built": tray_built,
            "menu_ids": self.menu_ids,
            "second_launch": self.second_launch,
        });
        let _ = std::fs::write(&self.report, serde_json::to_vec_pretty(&report).unwrap_or_default());
    }
}
