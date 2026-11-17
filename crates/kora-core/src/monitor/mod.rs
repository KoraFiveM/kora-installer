//! Monitoring and alerts (panel plan, phase 5). Synchronous: one std thread samples every registered
//! server every [`INTERVAL`], keeps 24 hours of history, and raises alerts.
//!
//! * [`probe`]: everything that touches the machine, behind a trait (tests use a fake).
//! * [`history`]: the in-memory ring and `<server>/.kora/metrics.jsonl`.
//! * [`alerts`]: rules, settings (`<server>/.kora/alerts.json`) and the generic webhook.
//! * [`panel_service`]: the opt-in background service that keeps all of this running.
//!
//! Alerts are kept in memory (the last [`MAX_ALERTS`]); history survives restarts, alerts do not.

pub mod alerts;
pub mod history;
pub mod panel_service;
pub mod probe;

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub use alerts::{Alert, AlertKind, AlertSettings};
pub use probe::{DiskUsage, GameInfo, MemUsage, Probe, ProcUsage, SystemProbe};

use crate::api::{Backend, UpdateBadge};
use crate::installer::registry::ServerEntry;
use crate::notify;

pub const INTERVAL: Duration = Duration::from_secs(15);
/// 24 hours of samples.
pub const CAP: usize = 5760;
pub const KEEP_SECS: u64 = 24 * 3600;
pub const MAX_ALERTS: usize = 200;
/// A stop asked for through Kora counts as intended for this long.
const INTENT_SECS: u64 = 15 * 60;
/// Charts get at most this many points.
const MAX_POINTS: usize = 360;

/// One look at one server. Absent fields were not measurable.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "MetricSample"))]
pub struct Sample {
    /// Unix seconds.
    pub t: u64,
    /// The server is running.
    pub up: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub players: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub max_players: Option<u32>,
    /// Percent of the whole machine's CPU used by the server and what it started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub cpu: Option<f32>,
    /// Megabytes of RAM used by the server and what it started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub ram_mb: Option<u32>,
    /// Percent of this machine's RAM in use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub sys_ram_pct: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub disk_free_mb: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub disk_free_pct: Option<f32>,
    #[serde(default)]
    pub game_up: bool,
    #[serde(default)]
    pub txadmin_up: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub badge: Option<UpdateBadge>,
}

/// The chart range of `GET /servers/{s}/metrics`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
    Hour,
    SixHours,
    Day,
}

impl Range {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "1h" => Some(Range::Hour),
            "6h" => Some(Range::SixHours),
            "24h" => Some(Range::Day),
            _ => None,
        }
    }
    pub fn secs(self) -> u64 {
        match self {
            Range::Hour => 3600,
            Range::SixHours => 6 * 3600,
            Range::Day => KEEP_SECS,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct MetricsView {
    pub samples: Vec<Sample>,
    pub latest: Option<Sample>,
}

/// Makes a [`Backend`] for one sampling pass.
pub type BackendMaker = Arc<dyn Fn() -> Box<dyn Backend + Send> + Send + Sync>;

/// Where raised alerts go besides the panel itself.
pub trait AlertSink: Send + Sync {
    fn deliver(&self, server: &ServerEntry, alert: &Alert, settings: &AlertSettings);
}

/// Discord (the server's existing webhook) and the generic webhook, each on its own short-lived thread.
pub struct RealSink;

impl AlertSink for RealSink {
    fn deliver(&self, server: &ServerEntry, alert: &Alert, settings: &AlertSettings) {
        let (server, alert, settings) = (server.clone(), alert.clone(), settings.clone());
        std::thread::spawn(move || {
            if settings.discord {
                let cfg = notify::load(&server);
                if cfg.discord_webhook.is_some() {
                    let colour = match alert.tone {
                        "danger" => 0xe74c3c,
                        "success" => 0x2ecc71,
                        _ => 0xf1c40f,
                    };
                    let _ = notify::send_alert(&cfg, &format!("{}: {}", server.name, alert.title), &alert.message, colour);
                }
            }
            if let Some(url) = &settings.webhook_url {
                let _ = alerts::post_json(url, &alerts::webhook_payload(&alert), settings.webhook_set_remotely);
            }
        });
    }
}

struct Track {
    history: history::History,
    last_up: Option<bool>,
    /// Conditions that are true right now, and the alert that reported each.
    active: HashMap<AlertKind, u64>,
}

#[derive(Default)]
struct State {
    servers: HashMap<String, Track>,
    alerts: VecDeque<Alert>,
    next_id: u64,
}

struct Inner {
    make: BackendMaker,
    probe: Arc<dyn Probe>,
    sink: Arc<dyn AlertSink>,
    state: Mutex<State>,
    stop: AtomicBool,
}

/// Cheap to clone; clones share everything.
#[derive(Clone)]
pub struct Monitor {
    inner: Arc<Inner>,
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Write a file only its owner can read (settings with secrets, the panel token).
pub(crate) fn write_private(path: &Path, text: &str) -> Result<(), String> {
    std::fs::create_dir_all(path.parent().unwrap_or(Path::new("."))).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("tmp");
    let user = std::env::var("USERNAME").ok().filter(|u| !u.is_empty());
    crate::db::write_secret_file(&crate::db::SystemRunner, crate::db::Os::current(), &tmp, text, user.as_deref()).map_err(|e| e.to_string())?;
    crate::fsutil::rename_retrying(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

fn intent_path(dir: &Path) -> std::path::PathBuf {
    dir.join(".kora").join("stop-intent")
}

/// Say that a stop or restart of this server was asked for through Kora, so the monitor does not
/// call it a crash. Best effort; called by the CLI, the manager and the panel before they stop it.
pub fn mark_stop_intent(dir: &Path) {
    if dir.is_dir() {
        let _ = std::fs::create_dir_all(dir.join(".kora"));
        let _ = std::fs::write(intent_path(dir), now_unix().to_string());
    }
}

fn read_intent(dir: &Path) -> Option<u64> {
    std::fs::read_to_string(intent_path(dir)).ok()?.trim().parse().ok()
}

fn round1(v: f64) -> f32 {
    ((v * 10.0).round() / 10.0) as f32
}

impl Monitor {
    pub fn new(make: BackendMaker, probe: Arc<dyn Probe>, sink: Arc<dyn AlertSink>) -> Self {
        Self { inner: Arc::new(Inner { make, probe, sink, state: Mutex::new(State::default()), stop: AtomicBool::new(false) }) }
    }

    /// The real thing: reads this machine, delivers to Discord and the generic webhook.
    pub fn system(make: BackendMaker) -> Self {
        Self::new(make, Arc::new(SystemProbe::new()), Arc::new(RealSink))
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.inner.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Start the sampling thread (first pass at once, then every [`INTERVAL`]); it ends on [`Monitor::stop`].
    pub fn start(&self) -> std::thread::JoinHandle<()> {
        let me = self.clone();
        std::thread::Builder::new()
            .name("kora-monitor".into())
            .spawn(move || {
                while !me.inner.stop.load(Ordering::Relaxed) {
                    // A bug in one pass must not end monitoring.
                    let m = me.clone();
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || m.tick(now_unix())));
                    let mut waited = Duration::ZERO;
                    while waited < INTERVAL && !me.inner.stop.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(500));
                        waited += Duration::from_millis(500);
                    }
                }
            })
            .expect("spawn monitor thread")
    }

    pub fn stop(&self) {
        self.inner.stop.store(true, Ordering::Relaxed);
    }

    /// One pass over every registered server at time `now` (unix seconds).
    pub fn tick(&self, now: u64) {
        let mut backend = (self.inner.make)();
        let Ok(servers) = backend.list() else { return };
        for e in &servers {
            self.sample_one(&mut *backend, e, now);
        }
        // Servers taken off the list: drop their state and close their alerts.
        let mut st = self.state();
        let gone: Vec<String> = st.servers.keys().filter(|k| !servers.iter().any(|e| &e.name == *k)).cloned().collect();
        for name in gone {
            st.servers.remove(&name);
            for a in st.alerts.iter_mut().filter(|a| a.server == name && a.resolved.is_none() && a.kind != AlertKind::Recovered) {
                a.resolved = Some(now);
            }
        }
    }

    fn sample_one(&self, backend: &mut dyn Backend, e: &ServerEntry, now: u64) {
        let probe = &self.inner.probe;
        if !probe.dir_exists(&e.dir) {
            return;
        }
        let detail = backend.status_detail(e);
        let game = if detail.running { probe.game_info(e.ports.game) } else { None };
        let proc_ = if detail.running { probe.process(e) } else { None };
        let mem = probe.memory();
        let disk = probe.disk(&e.dir);
        let sample = Sample {
            t: now,
            up: detail.running,
            players: game.map(|g| g.players),
            max_players: game.and_then(|g| g.max_players),
            cpu: proc_.and_then(|p| p.cpu_pct).map(|c| round1(f64::from(c))),
            ram_mb: proc_.map(|p| u32::try_from(p.rss_bytes / (1024 * 1024)).unwrap_or(u32::MAX)),
            sys_ram_pct: mem.filter(|m| m.total > 0).map(|m| round1((m.total - m.available.min(m.total)) as f64 / m.total as f64 * 100.0)),
            disk_free_mb: disk.map(|d| d.free / (1024 * 1024)),
            disk_free_pct: disk.filter(|d| d.total > 0).map(|d| round1(d.free as f64 / d.total as f64 * 100.0)),
            game_up: detail.game_listening,
            txadmin_up: detail.txadmin_up,
            badge: backend.badge(e),
        };
        let settings = alerts::load_settings(&e.dir);
        let intent = read_intent(&e.dir);
        let intent_fresh = intent.is_some_and(|t| t + INTENT_SECS >= now && t <= now + 60);

        let mut to_send: Vec<Alert> = Vec::new();
        {
            let mut st = self.state();
            let st = &mut *st;
            let track = st.servers.entry(e.name.clone()).or_insert_with(|| Track { history: history::History::load(&e.dir, now), last_up: None, active: HashMap::new() });
            let facts = alerts::Facts { prev_up: track.last_up, intentional_stop: intent_fresh, disk_total: disk.map(|d| d.total) };
            let changes = alerts::evaluate(&sample, &facts, &settings, &track.active);
            for c in changes {
                match c {
                    alerts::Change::Raise(kind, message) | alerts::Change::Event(kind, message) => {
                        st.next_id += 1;
                        let alert = Alert { id: st.next_id, server: e.name.clone(), kind, tone: kind.tone(), title: kind.title(), message, raised: now, resolved: None, acked: false };
                        if kind != AlertKind::Recovered {
                            track.active.insert(kind, alert.id);
                        }
                        to_send.push(alert.clone());
                        st.alerts.push_back(alert);
                    }
                    alerts::Change::Resolve(kind) => {
                        if let Some(id) = track.active.remove(&kind) {
                            if let Some(a) = st.alerts.iter_mut().find(|a| a.id == id) {
                                a.resolved = Some(now);
                            }
                        }
                    }
                }
            }
            while st.alerts.len() > MAX_ALERTS {
                let pos = st.alerts.iter().position(|a| a.resolved.is_some() || a.acked).unwrap_or(0);
                st.alerts.remove(pos);
            }
            // A stop that was asked for has been seen; an old marker left by a restart is cleaned up.
            let used_up = track.last_up == Some(true) && !sample.up && intent_fresh;
            let stale = sample.up && intent.is_some_and(|t| t + 120 < now);
            if used_up || stale {
                let _ = std::fs::remove_file(intent_path(&e.dir));
            }
            track.last_up = Some(sample.up);
            track.history.push(sample);
        }
        for a in &to_send {
            self.inner.sink.deliver(e, a, &settings);
        }
    }

    /// History for the charts. A server the sampler has not looked at yet is read from its file.
    pub fn metrics(&self, e: &ServerEntry, range: Range, now: u64) -> MetricsView {
        let st = self.state();
        let track = st.servers.get(&e.name);
        let view = |h: &history::History| MetricsView { samples: h.range(now, range.secs(), MAX_POINTS), latest: h.latest().cloned() };
        match track {
            Some(t) => view(&t.history),
            None => view(&history::History::load(&e.dir, now)),
        }
    }

    /// Raise an alert that does not come from sampling (a failed backup): it shows in the panel and goes to
    /// the same Discord / webhook as the others. One open alert per kind and server: a second failure while
    /// the first is open is not announced again.
    pub fn raise_external(&self, e: &ServerEntry, kind: AlertKind, message: String, now: u64) {
        let alert = {
            let mut st = self.state();
            let st = &mut *st;
            if st.alerts.iter().any(|a| a.server == e.name && a.kind == kind && a.resolved.is_none()) {
                return;
            }
            st.next_id += 1;
            let alert = Alert { id: st.next_id, server: e.name.clone(), kind, tone: kind.tone(), title: kind.title(), message, raised: now, resolved: None, acked: false };
            st.alerts.push_back(alert.clone());
            while st.alerts.len() > MAX_ALERTS {
                let pos = st.alerts.iter().position(|a| a.resolved.is_some() || a.acked).unwrap_or(0);
                st.alerts.remove(pos);
            }
            alert
        };
        self.inner.sink.deliver(e, &alert, &alerts::load_settings(&e.dir));
    }

    /// Close the open alerts of `kind` for a server (the condition is over).
    pub fn resolve_external(&self, server: &str, kind: AlertKind, now: u64) {
        for a in self.state().alerts.iter_mut().filter(|a| a.server == server && a.kind == kind && a.resolved.is_none()) {
            a.resolved = Some(now);
        }
    }

    /// Newest first.
    pub fn alerts(&self) -> Vec<Alert> {
        self.state().alerts.iter().rev().cloned().collect()
    }

    /// Acknowledge an alert (it stays in the list; an active condition does not raise again). False when unknown.
    pub fn ack(&self, id: u64) -> bool {
        match self.state().alerts.iter_mut().find(|a| a.id == id) {
            Some(a) => {
                a.acked = true;
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests;
