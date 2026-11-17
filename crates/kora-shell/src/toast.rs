//! Which events become desktop notifications, and how many at once.
//!
//! The shell does NOT invent its own rules about what is worth a notification: the panel's monitor already
//! raises alerts according to the person's settings per server (Notifications and Monitoring tabs: "stopped",
//! "back up", "disk low", "RAM high", "updates"). A rule that is switched off never produces an alert, so it
//! never produces a toast. The shell only turns NEW alerts into toasts and keeps them calm:
//!
//! * each alert id is shown once;
//! * the first read after the shell starts does not replay old alerts, except an unhandled danger from the last
//!   [`STARTUP_WINDOW_SECS`] (the shell may have been started by a crash recovery);
//! * a burst (several servers down together) is held for [`Policy::batch_secs`] and, above
//!   [`Policy::max_individual`], becomes ONE summary toast;
//! * "Pause notifications" and optional quiet hours silence everything except danger during quiet hours;
//! * events the page reports through the bridge (install finished, restart needed, ...) are deduplicated.
//!
//! Every toast carries a ROUTE: clicking it opens that page of the panel.

use std::collections::HashSet;

use crate::model::{AlertKind, AlertRow, Tone};
use crate::nav;

pub const STARTUP_WINDOW_SECS: u64 = 15 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    pub title: String,
    pub body: String,
    /// A panel page (see [`nav::safe_route`]).
    pub route: String,
    /// A danger: still shown in quiet hours.
    pub urgent: bool,
}

/// A daily window in minutes after midnight (`start` may be later than `end`: 22:00 to 07:00).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quiet {
    pub start: u16,
    pub end: u16,
}

impl Quiet {
    pub fn contains(self, minute_of_day: u16) -> bool {
        if self.start == self.end {
            false
        } else if self.start < self.end {
            (self.start..self.end).contains(&minute_of_day)
        } else {
            minute_of_day >= self.start || minute_of_day < self.end
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    pub quiet: Option<Quiet>,
    /// How long a toast waits for others to join it.
    pub batch_secs: u64,
    /// More than this many at once become one summary.
    pub max_individual: usize,
}

impl Default for Policy {
    fn default() -> Self {
        Self { quiet: None, batch_secs: 5, max_individual: 3 }
    }
}

#[derive(Debug, Default)]
pub struct Notifier {
    policy: Policy,
    seen: HashSet<u64>,
    primed: bool,
    update_announced: Option<String>,
    pause_until: Option<u64>,
    pending: Vec<(u64, Toast)>,
    recent: Vec<(u64, String)>,
}

fn route_for(a: &AlertRow) -> String {
    if a.server.is_empty() {
        "/".into()
    } else {
        nav::server_route(&a.server)
    }
}

/// The toast for one alert, in plain words. The panel's own wording is used for everything but "stopped".
pub fn toast_for_alert(a: &AlertRow) -> Toast {
    let route = route_for(a);
    let urgent = a.tone == Tone::Danger;
    match a.kind {
        AlertKind::Stopped => Toast { title: format!("{} stopped", a.server), body: "It stopped without being asked to. Open Kora to start it again.".into(), route, urgent: true },
        AlertKind::Recovered => Toast { title: format!("{} is back up", a.server), body: "It is running again.".into(), route, urgent: false },
        AlertKind::BackupFailed => Toast { title: format!("{}: backup failed", a.server), body: clip(&a.message, 200), route: format!("{route}?tab=backups"), urgent: true },
        _ => Toast { title: if a.title.is_empty() { "Kora needs a look".into() } else { clip(&a.title, 80) }, body: clip(&a.message, 200), route, urgent },
    }
}

fn clip(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}...", s.chars().take(max.saturating_sub(3)).collect::<String>())
    }
}

impl Notifier {
    pub fn new(policy: Policy) -> Self {
        Self { policy, ..Self::default() }
    }

    pub fn set_policy(&mut self, p: Policy) {
        self.policy = p;
    }

    /// Pause until this time (Unix seconds); `None` turns notifications back on.
    pub fn pause_until(&mut self, until: Option<u64>) {
        self.pause_until = until;
    }

    pub fn paused(&self, now: u64) -> bool {
        self.pause_until.is_some_and(|u| now < u)
    }

    /// The latest alert list from the panel.
    pub fn on_alerts(&mut self, now: u64, alerts: &[AlertRow]) {
        let first = !self.primed;
        self.primed = true;
        for a in alerts {
            if !self.seen.insert(a.id) {
                continue;
            }
            let wanted = match a.kind {
                AlertKind::Recovered => !a.acked,
                _ => a.open(),
            };
            if !wanted {
                continue;
            }
            if first && !(a.tone == Tone::Danger && a.raised + STARTUP_WINDOW_SECS >= now) {
                continue;
            }
            self.pending.push((now, toast_for_alert(a)));
        }
    }

    /// Kora itself has a newer version (`None`: none, or not known).
    pub fn on_update(&mut self, now: u64, latest: Option<&str>) {
        let Some(v) = latest else { return };
        if self.update_announced.as_deref() == Some(v) {
            return;
        }
        self.update_announced = Some(v.to_string());
        self.pending.push((now, Toast { title: format!("Kora {v} is ready"), body: "Open Kora to update. Your servers keep running while it does.".into(), route: "/settings".into(), urgent: false }));
    }

    /// Something the page asked to be told about (the bridge). Repeats within a minute are dropped.
    pub fn on_page_event(&mut self, now: u64, t: Toast) {
        let key = format!("{}\u{1}{}", t.title, t.body);
        self.recent.retain(|(at, _)| now < at + 60);
        if self.recent.iter().any(|(_, k)| *k == key) {
            return;
        }
        self.recent.push((now, key));
        self.pending.push((now, t));
    }

    /// Whatever should be shown now. Call about once a second; `minute_of_day` is the local clock.
    pub fn drain(&mut self, now: u64, minute_of_day: u16) -> Vec<Toast> {
        let Some(&(oldest, _)) = self.pending.first() else { return Vec::new() };
        if now < oldest + self.policy.batch_secs {
            return Vec::new();
        }
        let all: Vec<Toast> = self.pending.drain(..).map(|(_, t)| t).collect();
        if self.paused(now) {
            return Vec::new();
        }
        let quiet = self.policy.quiet.is_some_and(|q| q.contains(minute_of_day));
        let mut shown: Vec<Toast> = all.into_iter().filter(|t| !quiet || t.urgent).collect();
        if shown.len() > self.policy.max_individual {
            let urgent = shown.iter().any(|t| t.urgent);
            let names: Vec<String> = shown.iter().take(3).map(|t| t.title.clone()).collect();
            let body = format!("{}, and {} more. Open Kora to see them.", names.join("; "), shown.len() - names.len());
            shown = vec![Toast { title: format!("{} things need a look in Kora", shown.len()), body: clip(&body, 220), route: "/".into(), urgent }];
        }
        shown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alert(id: u64, server: &str, kind: AlertKind, tone: Tone, raised: u64) -> AlertRow {
        AlertRow { id, server: server.into(), kind, tone, title: format!("t{id}"), message: format!("m{id}"), raised, resolved: None, acked: false }
    }

    const NOW: u64 = 1_000_000;

    #[test]
    fn a_new_alert_becomes_one_toast_that_opens_the_servers_page() {
        let mut n = Notifier::default();
        n.on_alerts(NOW, &[]);
        n.on_alerts(NOW + 10, &[alert(1, "my server", AlertKind::Stopped, Tone::Danger, NOW + 9)]);
        assert!(n.drain(NOW + 11, 600).is_empty(), "held briefly so a burst can join");
        let t = n.drain(NOW + 16, 600);
        assert_eq!(t.len(), 1);
        assert_eq!((t[0].title.as_str(), t[0].route.as_str(), t[0].urgent), ("my server stopped", "/servers/my%20server", true));
        n.on_alerts(NOW + 20, &[alert(1, "my server", AlertKind::Stopped, Tone::Danger, NOW + 9)]);
        assert!(n.drain(NOW + 40, 600).is_empty(), "the same alert is never shown twice");
    }

    #[test]
    fn a_failed_backup_is_an_urgent_toast_that_opens_the_backups_tab() {
        let mut n = Notifier::default();
        n.on_alerts(NOW, &[]);
        n.on_alerts(NOW + 10, &[alert(7, "my server", AlertKind::BackupFailed, Tone::Danger, NOW + 9)]);
        let t = n.drain(NOW + 16, 600);
        assert_eq!(t.len(), 1);
        assert_eq!((t[0].title.as_str(), t[0].route.as_str(), t[0].urgent), ("my server: backup failed", "/servers/my%20server?tab=backups", true));
        assert!(crate::nav::safe_route(&t[0].route));
    }

    #[test]
    fn old_alerts_are_not_replayed_when_the_shell_starts_but_a_fresh_danger_is() {
        let mut n = Notifier::default();
        let old = alert(1, "a", AlertKind::DiskLow, Tone::Warning, NOW - 100);
        let stale = alert(2, "b", AlertKind::Stopped, Tone::Danger, NOW - STARTUP_WINDOW_SECS - 1);
        let fresh = alert(3, "c", AlertKind::Stopped, Tone::Danger, NOW - 60);
        n.on_alerts(NOW, &[old, stale, fresh]);
        let t = n.drain(NOW + 10, 600);
        assert_eq!(t.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), ["c stopped"]);
    }

    #[test]
    fn resolved_and_acknowledged_alerts_stay_quiet_and_back_up_is_a_calm_toast() {
        let mut n = Notifier::default();
        n.on_alerts(NOW, &[]);
        let mut done = alert(1, "a", AlertKind::DiskLow, Tone::Warning, NOW);
        done.resolved = Some(NOW + 1);
        let mut acked = alert(2, "a", AlertKind::RamHigh, Tone::Warning, NOW);
        acked.acked = true;
        let up = alert(3, "a", AlertKind::Recovered, Tone::Success, NOW);
        n.on_alerts(NOW + 5, &[done, acked, up]);
        let t = n.drain(NOW + 20, 600);
        assert_eq!(t.len(), 1);
        assert_eq!((t[0].title.as_str(), t[0].urgent), ("a is back up", false));
    }

    #[test]
    fn the_panels_own_words_are_used_for_other_alerts() {
        let mut n = Notifier::default();
        n.on_alerts(NOW, &[]);
        n.on_alerts(NOW + 1, &[alert(1, "a", AlertKind::DiskLow, Tone::Warning, NOW)]);
        let t = n.drain(NOW + 10, 0);
        assert_eq!((t[0].title.as_str(), t[0].body.as_str(), t[0].urgent), ("t1", "m1", false));
    }

    #[test]
    fn a_burst_becomes_one_summary() {
        let mut n = Notifier::default();
        n.on_alerts(NOW, &[]);
        let burst: Vec<_> = (1..=6).map(|i| alert(i, &format!("s{i}"), AlertKind::Stopped, Tone::Danger, NOW)).collect();
        n.on_alerts(NOW + 1, &burst);
        let t = n.drain(NOW + 10, 600);
        assert_eq!(t.len(), 1, "{t:?}");
        assert_eq!(t[0].title, "6 things need a look in Kora");
        assert!(t[0].body.contains("and 3 more") && t[0].urgent);
        assert_eq!(t[0].route, "/");
        // Three or fewer stay separate.
        n.on_alerts(NOW + 20, &(10..=12).map(|i| alert(i, "x", AlertKind::DiskLow, Tone::Warning, NOW)).collect::<Vec<_>>());
        assert_eq!(n.drain(NOW + 30, 600).len(), 3);
    }

    #[test]
    fn pausing_drops_what_arrives_and_resuming_does_not_replay_it() {
        let mut n = Notifier::default();
        n.on_alerts(NOW, &[]);
        n.pause_until(Some(NOW + 3600));
        assert!(n.paused(NOW + 10) && !n.paused(NOW + 3600));
        n.on_alerts(NOW + 1, &[alert(1, "a", AlertKind::Stopped, Tone::Danger, NOW)]);
        assert!(n.drain(NOW + 10, 600).is_empty());
        n.pause_until(None);
        assert!(n.drain(NOW + 4000, 600).is_empty(), "nothing replayed");
        n.on_alerts(NOW + 4001, &[alert(2, "a", AlertKind::Stopped, Tone::Danger, NOW + 4000)]);
        assert_eq!(n.drain(NOW + 4010, 600).len(), 1);
    }

    #[test]
    fn quiet_hours_keep_only_dangers() {
        let mut n = Notifier::new(Policy { quiet: Some(Quiet { start: 22 * 60, end: 7 * 60 }), ..Policy::default() });
        n.on_alerts(NOW, &[]);
        n.on_alerts(NOW + 1, &[alert(1, "a", AlertKind::DiskLow, Tone::Warning, NOW), alert(2, "b", AlertKind::Stopped, Tone::Danger, NOW)]);
        let t = n.drain(NOW + 10, 23 * 60);
        assert_eq!(t.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), ["b stopped"]);
        n.on_alerts(NOW + 20, &[alert(3, "a", AlertKind::DiskLow, Tone::Warning, NOW)]);
        assert_eq!(n.drain(NOW + 30, 12 * 60).len(), 1, "daytime: shown");
        let q = Quiet { start: 22 * 60, end: 7 * 60 };
        assert!(q.contains(23 * 60) && q.contains(3 * 60) && !q.contains(7 * 60) && !q.contains(12 * 60));
        assert!(Quiet { start: 60, end: 120 }.contains(90) && !Quiet { start: 60, end: 120 }.contains(120));
        assert!(!Quiet { start: 5, end: 5 }.contains(5), "an empty window is never quiet");
    }

    #[test]
    fn an_update_is_announced_once_per_version() {
        let mut n = Notifier::default();
        n.on_update(NOW, None);
        n.on_update(NOW, Some("0.8.0"));
        n.on_update(NOW + 1, Some("0.8.0"));
        let t = n.drain(NOW + 10, 600);
        assert_eq!((t.len(), t[0].route.as_str(), t[0].title.as_str()), (1, "/settings", "Kora 0.8.0 is ready"));
        n.on_update(NOW + 100, Some("0.8.1"));
        assert_eq!(n.drain(NOW + 110, 600).len(), 1);
    }

    #[test]
    fn page_events_are_deduplicated_for_a_minute() {
        let mut n = Notifier::default();
        let t = Toast { title: "Install finished".into(), body: "my server is ready".into(), route: "/servers/my%20server".into(), urgent: false };
        n.on_page_event(NOW, t.clone());
        n.on_page_event(NOW + 2, t.clone());
        assert_eq!(n.drain(NOW + 10, 600).len(), 1);
        n.on_page_event(NOW + 70, t);
        assert_eq!(n.drain(NOW + 80, 600).len(), 1);
    }

    #[test]
    fn long_text_is_clipped() {
        let mut a = alert(1, "a", AlertKind::DiskLow, Tone::Warning, NOW);
        a.message = "x".repeat(500);
        let t = toast_for_alert(&a);
        assert!(t.body.chars().count() <= 200 && t.body.ends_with("..."));
    }
}
