//! The tray icon's state and its menu, built from what the panel reports.
//!
//! [`health`] gives the colour and words, [`build_menu`] the menu as plain data ([`Entry`]) and [`Action`] what a
//! click means. The GUI layer maps entries to native menu items and calls [`decode`] on the clicked id.
//!
//! What "Quit" does is stated in the menu itself ([`QUIT_NOTE`]): the game servers and the background panel keep
//! running; only "Quit and turn off Kora's background service" stops the panel.

use crate::model::{AlertKind, ServerStatus, Snapshot, Tone};
use crate::nav;

/// The tray's status at a glance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    /// The panel is not answering (starting, stopped, or the shell is reconnecting).
    Offline,
    /// The panel answers and no servers exist yet.
    NoServers,
    /// Every server runs and nothing needs doing.
    AllRunning,
    /// Something is worth a look: a server stopped on purpose, a warning, an update.
    Notice,
    /// Something needs attention: a server stopped unexpectedly, a folder is missing, a danger alert.
    Attention,
}

/// The icon colour for a [`Health`] (the GUI layer holds one image for each).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconKind {
    Grey,
    Green,
    Amber,
    Red,
}

impl Health {
    pub fn icon(self) -> IconKind {
        match self {
            Health::Offline | Health::NoServers => IconKind::Grey,
            Health::AllRunning => IconKind::Green,
            Health::Notice => IconKind::Amber,
            Health::Attention => IconKind::Red,
        }
    }
}

/// Alerts that are still open and are not just "it came back".
fn open_problems(s: &Snapshot) -> impl Iterator<Item = &crate::model::AlertRow> {
    s.alerts.iter().filter(|a| a.open() && a.kind != AlertKind::Recovered && a.tone != Tone::Success)
}

pub fn health(s: &Snapshot) -> Health {
    if !s.connected {
        return Health::Offline;
    }
    if s.servers.is_empty() {
        return if open_problems(s).next().is_some() { Health::Attention } else { Health::NoServers };
    }
    if s.servers.iter().any(|v| v.status == ServerStatus::Missing) || open_problems(s).any(|a| a.tone == Tone::Danger) {
        return Health::Attention;
    }
    if s.servers.iter().any(|v| v.status != ServerStatus::Running) || open_problems(s).next().is_some() || s.update.is_some() {
        return Health::Notice;
    }
    Health::AllRunning
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The hover text (Windows shows at most 127 characters; this stays well under).
pub fn tooltip(s: &Snapshot) -> String {
    let running = s.servers.iter().filter(|v| v.status == ServerStatus::Running).count();
    let total = s.servers.len();
    let line = match health(s) {
        Health::Offline => "Kora is starting or not reachable".to_string(),
        Health::NoServers => "Kora: no servers yet".to_string(),
        Health::AllRunning => {
            if total == 1 {
                "Kora: your server is running".to_string()
            } else {
                format!("Kora: all {total} servers are running")
            }
        }
        Health::Notice | Health::Attention => {
            let mut parts = Vec::new();
            if running < total {
                parts.push(format!("{running} of {total} servers running"));
            }
            let n = open_problems(s).count();
            if n > 0 {
                parts.push(format!("{} to look at", plural(n, "thing", "things")));
            }
            if s.update.is_some() {
                parts.push("an update is ready".to_string());
            }
            if parts.is_empty() {
                parts.push("something needs a look".to_string());
            }
            format!("Kora: {}", parts.join(", "))
        }
    };
    line.chars().take(120).collect()
}

/// What a click on the menu does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Show the window on this route.
    Open(String),
    Control {
        server: String,
        verb: Verb,
    },
    /// Open the server's txAdmin in the default browser.
    OpenTxAdmin {
        server: String,
    },
    CheckUpdates,
    /// Pause notifications for this many minutes (`0` = resume now).
    PauseNotifications(u32),
    ToggleAutostart,
    Quit,
    QuitAndStopService,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Start,
    Stop,
    Restart,
}

impl Verb {
    /// The word the panel's `POST /servers/{s}/control` expects.
    pub fn api_word(self) -> &'static str {
        match self {
            Verb::Start => "start",
            Verb::Stop => "stop",
            Verb::Restart => "restart",
        }
    }
}

/// An id for a native menu item; [`decode`] is its inverse. Server names go last so they may hold anything.
pub fn encode(a: &Action) -> String {
    match a {
        Action::Open(r) => format!("open:{r}"),
        Action::Control { server, verb } => format!("srv:{}:{server}", verb.api_word()),
        Action::OpenTxAdmin { server } => format!("txadmin:{server}"),
        Action::CheckUpdates => "updates".into(),
        Action::PauseNotifications(m) => format!("pause:{m}"),
        Action::ToggleAutostart => "autostart".into(),
        Action::Quit => "quit".into(),
        Action::QuitAndStopService => "quit-stop".into(),
    }
}

pub fn decode(id: &str) -> Option<Action> {
    if let Some(r) = id.strip_prefix("open:") {
        return nav::safe_route(r).then(|| Action::Open(r.to_string()));
    }
    if let Some(rest) = id.strip_prefix("srv:") {
        let (verb, server) = rest.split_once(':')?;
        let verb = match verb {
            "start" => Verb::Start,
            "stop" => Verb::Stop,
            "restart" => Verb::Restart,
            _ => return None,
        };
        return (!server.is_empty()).then(|| Action::Control { server: server.to_string(), verb });
    }
    if let Some(s) = id.strip_prefix("txadmin:") {
        return (!s.is_empty()).then(|| Action::OpenTxAdmin { server: s.to_string() });
    }
    if let Some(m) = id.strip_prefix("pause:") {
        return m.parse().ok().map(Action::PauseNotifications);
    }
    match id {
        "updates" => Some(Action::CheckUpdates),
        "autostart" => Some(Action::ToggleAutostart),
        "quit" => Some(Action::Quit),
        "quit-stop" => Some(Action::QuitAndStopService),
        _ => None,
    }
}

/// A menu entry as plain data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Item {
        id: String,
        label: String,
        enabled: bool,
    },
    Check {
        id: String,
        label: String,
        checked: bool,
    },
    /// A line of text that does nothing when clicked.
    Info(String),
    Submenu {
        label: String,
        items: Vec<Entry>,
    },
    Separator,
}

/// Shown under "Quit" so nobody thinks it stops their servers.
pub const QUIT_NOTE: &str = "Quit closes this icon. Your servers and Kora's background service keep running.";

/// What the menu depends on besides the panel's data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuOptions {
    pub autostart: bool,
    pub paused: bool,
    /// "Start with Windows" or "Start with your desktop".
    pub windows: bool,
    /// How many servers the menu lists before pointing at the window.
    pub max_servers: usize,
}

impl Default for MenuOptions {
    fn default() -> Self {
        Self { autostart: false, paused: false, windows: cfg!(windows), max_servers: 12 }
    }
}

fn item(action: &Action, label: impl Into<String>) -> Entry {
    Entry::Item { id: encode(action), label: label.into(), enabled: true }
}

fn status_word(s: ServerStatus) -> &'static str {
    match s {
        ServerStatus::Running => "running",
        ServerStatus::Stopped => "stopped",
        ServerStatus::Missing => "folder missing",
        ServerStatus::Unknown => "unknown",
    }
}

pub fn build_menu(s: &Snapshot, o: &MenuOptions) -> Vec<Entry> {
    let mut m = vec![item(&Action::Open("/".into()), "Open Kora"), Entry::Info(tooltip(s)), Entry::Separator];
    if s.connected {
        for v in s.servers.iter().take(o.max_servers) {
            let mut items = vec![item(&Action::Open(nav::server_route(&v.name)), "Open in Kora"), Entry::Separator];
            let ctl = |verb: Verb, label: &str, enabled: bool| Entry::Item { id: encode(&Action::Control { server: v.name.clone(), verb }), label: label.into(), enabled };
            match v.status {
                ServerStatus::Running => {
                    items.push(ctl(Verb::Restart, "Restart", true));
                    items.push(ctl(Verb::Stop, "Stop", true));
                }
                ServerStatus::Stopped => items.push(ctl(Verb::Start, "Start", true)),
                ServerStatus::Missing | ServerStatus::Unknown => {
                    items.push(ctl(Verb::Start, "Start", false));
                }
            }
            if v.ports.txadmin != 0 && v.status == ServerStatus::Running {
                items.push(item(&Action::OpenTxAdmin { server: v.name.clone() }, "Open txAdmin"));
            }
            m.push(Entry::Submenu { label: format!("{} ({})", v.name, status_word(v.status)), items });
        }
        if s.servers.len() > o.max_servers {
            m.push(Entry::Info(format!("and {} more in the Kora window", s.servers.len() - o.max_servers)));
        }
        if !s.servers.is_empty() {
            m.push(Entry::Separator);
        }
    } else {
        m.push(Entry::Info("Waiting for Kora's panel ...".into()));
        m.push(Entry::Separator);
    }
    let update_label = match &s.update {
        Some(v) => format!("Update Kora to {v} ..."),
        None => "Check for updates".to_string(),
    };
    m.push(match &s.update {
        Some(_) => item(&Action::Open("/settings".into()), update_label),
        None => item(&Action::CheckUpdates, update_label),
    });
    m.push(if o.paused {
        item(&Action::PauseNotifications(0), "Turn notifications back on")
    } else {
        Entry::Submenu {
            label: "Pause notifications".into(),
            items: vec![item(&Action::PauseNotifications(60), "For 1 hour"), item(&Action::PauseNotifications(8 * 60), "For 8 hours"), item(&Action::PauseNotifications(24 * 60), "Until tomorrow")],
        }
    });
    let start_label = if o.windows { "Start with Windows" } else { "Start when I sign in" };
    m.push(Entry::Check { id: encode(&Action::ToggleAutostart), label: start_label.into(), checked: o.autostart });
    m.push(Entry::Separator);
    m.push(Entry::Info(QUIT_NOTE.into()));
    m.push(item(&Action::Quit, "Quit"));
    m.push(item(&Action::QuitAndStopService, "Quit and turn off Kora's background service"));
    m
}

/// Every clickable id in a menu (for tests and for the CI smoke report).
pub fn ids(entries: &[Entry]) -> Vec<String> {
    let mut out = Vec::new();
    for e in entries {
        match e {
            Entry::Item { id, .. } | Entry::Check { id, .. } => out.push(id.clone()),
            Entry::Submenu { items, .. } => out.extend(ids(items)),
            Entry::Info(_) | Entry::Separator => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AlertRow, PortsRow, ServerRow};

    fn srv(name: &str, status: ServerStatus) -> ServerRow {
        ServerRow { name: name.into(), status, ports: PortsRow { game: 30120, txadmin: 40120 } }
    }

    fn alert(id: u64, kind: AlertKind, tone: Tone) -> AlertRow {
        AlertRow { id, server: "a".into(), kind, tone, title: "t".into(), message: "m".into(), raised: 100, resolved: None, acked: false }
    }

    fn snap(servers: Vec<ServerRow>, alerts: Vec<AlertRow>) -> Snapshot {
        Snapshot { connected: true, servers, alerts, update: None }
    }

    #[test]
    fn health_follows_what_the_panel_reports() {
        assert_eq!(health(&Snapshot::default()), Health::Offline);
        assert_eq!(health(&snap(vec![], vec![])), Health::NoServers);
        assert_eq!(health(&snap(vec![srv("a", ServerStatus::Running), srv("b", ServerStatus::Running)], vec![])), Health::AllRunning);
        assert_eq!(health(&snap(vec![srv("a", ServerStatus::Running), srv("b", ServerStatus::Stopped)], vec![])), Health::Notice, "stopped on purpose is a notice");
        assert_eq!(health(&snap(vec![srv("a", ServerStatus::Stopped)], vec![alert(1, AlertKind::Stopped, Tone::Danger)])), Health::Attention, "stopped unexpectedly");
        assert_eq!(health(&snap(vec![srv("a", ServerStatus::Missing)], vec![])), Health::Attention);
        assert_eq!(health(&snap(vec![srv("a", ServerStatus::Running)], vec![alert(2, AlertKind::DiskLow, Tone::Warning)])), Health::Notice);
        let mut done = alert(3, AlertKind::Stopped, Tone::Danger);
        done.resolved = Some(200);
        let mut acked = alert(4, AlertKind::RamHigh, Tone::Danger);
        acked.acked = true;
        assert_eq!(
            health(&snap(vec![srv("a", ServerStatus::Running)], vec![done, acked, alert(5, AlertKind::Recovered, Tone::Success)])),
            Health::AllRunning,
            "resolved, acknowledged and 'back up' alerts are not problems"
        );
        let mut upd = snap(vec![srv("a", ServerStatus::Running)], vec![]);
        upd.update = Some("0.8.0".into());
        assert_eq!(health(&upd), Health::Notice);
        assert_eq!(Health::Attention.icon(), IconKind::Red);
        assert_eq!(Health::AllRunning.icon(), IconKind::Green);
        assert_eq!(Health::Offline.icon(), IconKind::Grey);
    }

    #[test]
    fn the_tooltip_says_it_in_words() {
        assert_eq!(tooltip(&snap(vec![srv("a", ServerStatus::Running), srv("b", ServerStatus::Running)], vec![])), "Kora: all 2 servers are running");
        assert_eq!(tooltip(&snap(vec![srv("a", ServerStatus::Running)], vec![])), "Kora: your server is running");
        let t = tooltip(&snap(vec![srv("a", ServerStatus::Running), srv("b", ServerStatus::Stopped)], vec![alert(1, AlertKind::Stopped, Tone::Danger)]));
        assert_eq!(t, "Kora: 1 of 2 servers running, 1 thing to look at");
        assert_eq!(tooltip(&Snapshot::default()), "Kora is starting or not reachable");
    }

    #[test]
    fn menu_ids_round_trip_and_hostile_ids_do_not_decode() {
        let actions = [
            Action::Open("/servers/a%20b".into()),
            Action::Control { server: "my: server".into(), verb: Verb::Restart },
            Action::Control { server: "x".into(), verb: Verb::Stop },
            Action::OpenTxAdmin { server: "a:b".into() },
            Action::CheckUpdates,
            Action::PauseNotifications(60),
            Action::ToggleAutostart,
            Action::Quit,
            Action::QuitAndStopService,
        ];
        for a in actions {
            assert_eq!(decode(&encode(&a)), Some(a.clone()), "{a:?}");
        }
        for bad in ["", "open://evil", "open:http://evil", "srv:explode:x", "srv:start:", "srv:start", "pause:abc", "txadmin:", "rm -rf"] {
            assert_eq!(decode(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_menu_lists_each_server_with_what_it_can_do() {
        let s = snap(vec![srv("one", ServerStatus::Running), srv("two", ServerStatus::Stopped), srv("three", ServerStatus::Missing)], vec![]);
        let menu = build_menu(&s, &MenuOptions { windows: true, ..MenuOptions::default() });
        let ids = ids(&menu);
        for want in ["open:/", "srv:restart:one", "srv:stop:one", "txadmin:one", "srv:start:two", "srv:start:three", "updates", "pause:60", "autostart", "quit", "quit-stop"] {
            assert!(ids.iter().any(|i| i == want), "missing {want}: {ids:?}");
        }
        assert!(!ids.contains(&"txadmin:two".to_string()), "txAdmin only runs with the server");
        assert!(!ids.contains(&"srv:stop:two".to_string()));
        let missing_start = menu.iter().find_map(|e| match e {
            Entry::Submenu { label, items } if label.starts_with("three") => items.iter().find(|i| matches!(i, Entry::Item { id, .. } if id == "srv:start:three")),
            _ => None,
        });
        assert!(matches!(missing_start, Some(Entry::Item { enabled: false, .. })), "a missing folder cannot be started");
        for id in &ids {
            assert!(decode(id).is_some(), "every id the menu makes must decode: {id}");
        }
        let text = format!("{menu:?}");
        assert!(text.contains("Start with Windows") && text.contains(QUIT_NOTE) && text.contains("Quit and turn off Kora's background service"), "{text}");
    }

    #[test]
    fn the_menu_has_a_sensible_shape_offline_paused_with_an_update_and_with_many_servers() {
        let off = build_menu(&Snapshot::default(), &MenuOptions::default());
        assert!(format!("{off:?}").contains("Waiting for Kora"));
        assert!(ids(&off).contains(&"quit".to_string()), "Quit is always there");
        let mut s = snap(vec![srv("a", ServerStatus::Running)], vec![]);
        s.update = Some("0.8.0".into());
        let m = build_menu(&s, &MenuOptions { paused: true, autostart: true, windows: false, max_servers: 12 });
        let t = format!("{m:?}");
        assert!(t.contains("Update Kora to 0.8.0") && t.contains("Turn notifications back on") && t.contains("Start when I sign in"), "{t}");
        assert!(ids(&m).contains(&"pause:0".to_string()) && !ids(&m).contains(&"pause:60".to_string()));
        let many = snap((0..20).map(|i| srv(&format!("s{i}"), ServerStatus::Running)).collect(), vec![]);
        let m = build_menu(&many, &MenuOptions::default());
        assert!(format!("{m:?}").contains("and 8 more in the Kora window"));
    }
}
