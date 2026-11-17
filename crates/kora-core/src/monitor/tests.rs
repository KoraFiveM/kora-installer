//! Sampler tests: a fake backend, a fake machine and a recording sink, so nothing leaves the process.
use std::path::Path;
use std::sync::Mutex;

use super::*;
use crate::api::fake::FakeBackend;
use crate::api::Status;
use crate::installer::registry::{PortsRecord, ServerEntry};
use crate::installer::Mode;

#[derive(Default)]
struct Machine {
    players: Option<GameInfo>,
    proc_: Option<ProcUsage>,
    disk: Option<DiskUsage>,
    mem: Option<MemUsage>,
    gone: bool,
}

#[derive(Default)]
struct FakeProbe(Mutex<Machine>);
impl Probe for FakeProbe {
    fn dir_exists(&self, _: &Path) -> bool {
        !self.0.lock().unwrap().gone
    }
    fn game_info(&self, _: u16) -> Option<GameInfo> {
        self.0.lock().unwrap().players
    }
    fn process(&self, _: &ServerEntry) -> Option<ProcUsage> {
        self.0.lock().unwrap().proc_
    }
    fn disk(&self, _: &Path) -> Option<DiskUsage> {
        self.0.lock().unwrap().disk
    }
    fn memory(&self) -> Option<MemUsage> {
        self.0.lock().unwrap().mem
    }
}

#[derive(Default)]
struct Recorder(Mutex<Vec<(String, AlertKind, AlertSettings)>>);
impl AlertSink for Recorder {
    fn deliver(&self, server: &ServerEntry, a: &Alert, s: &AlertSettings) {
        self.0.lock().unwrap().push((server.name.clone(), a.kind, s.clone()));
    }
}

struct Rig {
    dir: tempfile::TempDir,
    fake: FakeBackend,
    probe: Arc<FakeProbe>,
    sink: Arc<Recorder>,
    mon: Monitor,
    entry: ServerEntry,
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let entry = ServerEntry { name: "prod".into(), dir: dir.path().to_path_buf(), mode: Mode::Production, ports: PortsRecord { game: 30120, txadmin: 40120 }, created_unix: 0 };
    let fake = FakeBackend::new();
    fake.state().servers.push(entry.clone());
    fake.state().status.insert("prod".into(), Status::Running);
    let probe = Arc::new(FakeProbe::default());
    let sink = Arc::new(Recorder::default());
    let f = fake.clone();
    let mon = Monitor::new(Arc::new(move || Box::new(f.clone())), probe.clone(), sink.clone());
    Rig { dir, fake, probe, sink, mon, entry }
}

#[test]
fn a_sample_collects_status_players_cpu_ram_and_disk() {
    let r = rig();
    *r.probe.0.lock().unwrap() = Machine {
        players: Some(GameInfo { players: 12, max_players: Some(48) }),
        proc_: Some(ProcUsage { cpu_pct: Some(12.34), rss_bytes: 3 << 30 }),
        disk: Some(DiskUsage { free: 50 << 30, total: 100 << 30 }),
        mem: Some(MemUsage { total: 16 << 30, available: 4 << 30 }),
        gone: false,
    };
    r.fake.state().badges.insert("prod".into(), UpdateBadge::Updates(2));
    r.mon.tick(1_000);
    let v = r.mon.metrics(&r.entry, Range::Hour, 1_000);
    let s = v.latest.expect("a sample");
    assert_eq!(v.samples.len(), 1);
    assert_eq!((s.up, s.players, s.max_players, s.cpu, s.ram_mb), (true, Some(12), Some(48), Some(12.3), Some(3072)));
    assert_eq!((s.sys_ram_pct, s.disk_free_mb, s.disk_free_pct), (Some(75.0), Some(51_200), Some(50.0)));
    assert!(s.game_up && s.txadmin_up);
    assert_eq!(s.badge, Some(UpdateBadge::Updates(2)));
    assert!(r.sink.0.lock().unwrap().is_empty(), "a healthy server raises nothing");
}

#[test]
fn a_stopped_server_has_no_player_or_process_numbers() {
    let r = rig();
    r.fake.state().status.insert("prod".into(), Status::Stopped);
    *r.probe.0.lock().unwrap() = Machine { players: Some(GameInfo { players: 3, max_players: None }), proc_: Some(ProcUsage { cpu_pct: None, rss_bytes: 1 }), ..Machine::default() };
    r.mon.tick(10);
    let s = r.mon.metrics(&r.entry, Range::Hour, 10).latest.unwrap();
    assert!(!s.up && s.players.is_none() && s.ram_mb.is_none() && !s.game_up);
}

#[test]
fn a_crash_raises_once_and_coming_back_says_so() {
    let r = rig();
    r.mon.tick(100);
    r.fake.state().status.insert("prod".into(), Status::Stopped);
    r.mon.tick(115);
    r.mon.tick(130);
    r.mon.tick(145);
    let kinds: Vec<AlertKind> = r.sink.0.lock().unwrap().iter().map(|x| x.1).collect();
    assert_eq!(kinds, vec![AlertKind::Stopped], "one alert while it stays down");
    let a = r.mon.alerts();
    assert_eq!((a.len(), a[0].tone, a[0].resolved), (1, "danger", None));
    r.fake.state().status.insert("prod".into(), Status::Running);
    r.mon.tick(160);
    let a = r.mon.alerts();
    assert_eq!(a.len(), 2);
    assert_eq!(a[0].kind, AlertKind::Recovered);
    assert_eq!(a[1].resolved, Some(160), "the stopped alert is closed");
    // And a second crash raises again.
    r.fake.state().status.insert("prod".into(), Status::Stopped);
    r.mon.tick(175);
    assert_eq!(r.sink.0.lock().unwrap().iter().filter(|x| x.1 == AlertKind::Stopped).count(), 2);
}

#[test]
fn a_stop_asked_for_through_kora_is_not_a_crash() {
    let r = rig();
    r.mon.tick(now_unix());
    mark_stop_intent(r.dir.path());
    r.fake.state().status.insert("prod".into(), Status::Stopped);
    r.mon.tick(now_unix() + 15);
    assert!(r.mon.alerts().is_empty());
    assert!(!intent_path(r.dir.path()).exists(), "the marker is used up");
    // Later it starts and dies by itself: that one counts.
    r.fake.state().status.insert("prod".into(), Status::Running);
    r.mon.tick(now_unix() + 30);
    r.fake.state().status.insert("prod".into(), Status::Stopped);
    r.mon.tick(now_unix() + 45);
    assert_eq!(r.mon.alerts().len(), 1);
}

#[test]
fn an_old_marker_does_not_hide_a_crash() {
    let r = rig();
    std::fs::create_dir_all(r.dir.path().join(".kora")).unwrap();
    std::fs::write(intent_path(r.dir.path()), "100").unwrap();
    r.mon.tick(10_000);
    r.fake.state().status.insert("prod".into(), Status::Stopped);
    r.mon.tick(10_015);
    assert_eq!(r.mon.alerts().len(), 1);
}

#[test]
fn a_server_that_was_already_down_when_the_panel_started_is_not_a_crash() {
    let r = rig();
    r.fake.state().status.insert("prod".into(), Status::Stopped);
    r.mon.tick(1);
    r.mon.tick(16);
    assert!(r.mon.alerts().is_empty());
}

#[test]
fn disk_ram_and_update_alerts_debounce_until_they_clear() {
    let r = rig();
    r.fake.state().badges.insert("prod".into(), UpdateBadge::NeedsAttention);
    *r.probe.0.lock().unwrap() = Machine { disk: Some(DiskUsage { free: 2 << 30, total: 500 << 30 }), mem: Some(MemUsage { total: 16 << 30, available: 1 << 30 }), ..Machine::default() };
    for t in [100, 115, 130] {
        r.mon.tick(t);
    }
    let mut kinds: Vec<String> = r.sink.0.lock().unwrap().iter().map(|x| format!("{:?}", x.1)).collect();
    kinds.sort();
    assert_eq!(kinds, ["DiskLow", "RamHigh", "UpdateAttention"]);
    // Everything clears, then the disk fills up again: only that one is raised a second time.
    r.fake.state().badges.insert("prod".into(), UpdateBadge::UpToDate);
    *r.probe.0.lock().unwrap() = Machine { disk: Some(DiskUsage { free: 300 << 30, total: 500 << 30 }), mem: Some(MemUsage { total: 16 << 30, available: 8 << 30 }), ..Machine::default() };
    r.mon.tick(145);
    assert!(r.mon.alerts().iter().all(|a| a.resolved == Some(145)));
    r.probe.0.lock().unwrap().disk = Some(DiskUsage { free: 1 << 30, total: 500 << 30 });
    r.mon.tick(160);
    assert_eq!(r.sink.0.lock().unwrap().len(), 4);
}

#[test]
fn settings_switch_rules_off_and_travel_with_the_delivery() {
    let r = rig();
    let set = AlertSettings { rules: alerts::Rules { stopped: false, ..Default::default() }, webhook_url: Some("https://example.test/h".into()), ..AlertSettings::default() };
    alerts::save_settings(r.dir.path(), &set).unwrap();
    r.mon.tick(100);
    r.fake.state().status.insert("prod".into(), Status::Stopped);
    r.mon.tick(115);
    assert!(r.mon.alerts().is_empty(), "the stopped rule is off");
    r.probe.0.lock().unwrap().disk = Some(DiskUsage { free: 1 << 30, total: 10 << 30 });
    r.mon.tick(130);
    let sent = r.sink.0.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].2.webhook_url.as_deref(), Some("https://example.test/h"));
}

#[test]
fn ack_keeps_the_alert_but_does_not_raise_it_again() {
    let r = rig();
    r.mon.tick(100);
    r.fake.state().status.insert("prod".into(), Status::Stopped);
    r.mon.tick(115);
    let id = r.mon.alerts()[0].id;
    assert!(r.mon.ack(id));
    assert!(!r.mon.ack(9999));
    r.mon.tick(130);
    let a = r.mon.alerts();
    assert_eq!((a.len(), a[0].acked), (1, true));
}

#[test]
fn history_is_reloaded_after_a_restart() {
    let r = rig();
    for t in [100, 115, 130] {
        r.mon.tick(t);
    }
    // A new monitor (the panel restarted) reads the file.
    let again = Monitor::new(
        Arc::new({
            let f = r.fake.clone();
            move || Box::new(f.clone())
        }),
        r.probe.clone(),
        r.sink.clone(),
    );
    let v = again.metrics(&r.entry, Range::Hour, 140);
    assert_eq!(v.samples.len(), 3);
    assert_eq!(v.latest.unwrap().t, 130);
}

#[test]
fn a_missing_folder_is_skipped_and_forgotten_servers_close_their_alerts() {
    let r = rig();
    r.mon.tick(100);
    r.fake.state().status.insert("prod".into(), Status::Stopped);
    r.mon.tick(115);
    assert_eq!(r.mon.alerts().len(), 1);
    r.fake.state().servers.clear();
    r.mon.tick(130);
    assert_eq!(r.mon.alerts()[0].resolved, Some(130));
    r.fake.state().servers.push(r.entry.clone());
    r.probe.0.lock().unwrap().gone = true;
    r.mon.tick(145);
    assert!(r.mon.metrics(&r.entry, Range::Hour, 145).samples.len() <= 2);
}

#[test]
fn the_generic_webhook_gets_json_and_errors_do_not_leak_the_address() {
    use std::io::{Read, Write};
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let t = std::thread::spawn(move || {
        let (mut c, _) = l.accept().unwrap();
        c.set_read_timeout(Some(std::time::Duration::from_secs(3))).unwrap();
        let mut got = Vec::new();
        let mut buf = vec![0u8; 4096];
        // Headers and body may arrive in separate packets.
        while !got.ends_with(b"}") {
            match c.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => got.extend_from_slice(&buf[..n]),
            }
        }
        let _ = c.write_all(b"HTTP/1.0 204 No Content\r\n\r\n");
        String::from_utf8_lossy(&got).into_owned()
    });
    let a = Alert { id: 1, server: "prod".into(), kind: AlertKind::Stopped, tone: "danger", title: "Server stopped", message: "m".into(), raised: 5, resolved: None, acked: false };
    alerts::post_json(&format!("http://127.0.0.1:{port}/secret-path"), &alerts::webhook_payload(&a), false).unwrap();
    let req = t.join().unwrap();
    assert!(req.starts_with("POST /secret-path "), "{req}");
    assert!(req.contains(r#""server":"prod""#) && req.contains(r#""kind":"stopped""#) && req.contains(r#""severity":"danger""#), "{req}");
    // Port 0 never answers. (A port freed by an earlier listener could be taken by another test
    // running at the same time, and then answer.)
    let e = alerts::post_json("http://127.0.0.1:0/secret-path", &serde_json::json!({}), false).unwrap_err();
    assert!(e.contains("Could not reach") && !e.contains("secret-path") && !e.contains("127.0.0.1"), "{e}");
}

#[test]
fn a_failed_backup_raises_one_alert_that_closes_when_a_backup_works() {
    let r = rig();
    r.mon.raise_external(&r.entry, AlertKind::BackupFailed, "The scheduled backup did not work: the disk is full".into(), 100);
    r.mon.raise_external(&r.entry, AlertKind::BackupFailed, "again".into(), 200);
    let list = r.mon.alerts();
    assert_eq!(list.len(), 1, "a second failure while the first is open is not announced again");
    assert_eq!((list[0].kind, list[0].tone, list[0].title), (AlertKind::BackupFailed, "danger", "Backup failed"));
    assert!(list[0].resolved.is_none());
    // it went to the same sink as the other alerts (Discord / the generic webhook)
    assert_eq!(r.sink.0.lock().unwrap().len(), 1);
    r.mon.resolve_external("prod", AlertKind::BackupFailed, 300);
    assert_eq!(r.mon.alerts()[0].resolved, Some(300));
    // a later failure is a new alert
    r.mon.raise_external(&r.entry, AlertKind::BackupFailed, "a week later".into(), 400);
    assert_eq!(r.mon.alerts().len(), 2);
}
