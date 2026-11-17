//! Keeping the game port forwarded for exactly as long as the server runs.
//!
//! The mapping has a lease, so something has to renew it, and something has to remove it when the
//! server stops. That something must live as long as the server, not as long as a browser tab:
//!
//! * [`ServerGuard`] runs inside `kora service-host` (the process the OS service starts, which lives
//!   exactly as long as FXServer does);
//! * [`spawn_panel_keeper`] runs in the panel process (the background panel keeps it alive) and covers
//!   servers started without a service.
//!
//! Both do the same idempotent thing (a mapping asked for twice is one mapping), so having both is
//! harmless. [`Keeper::tick`] is the whole logic and is tested with fake routers.

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{Discover, Granted, MapError, PortMapper, Proto, Wanted, LEASE_SECS};
use crate::reach::settings::{self, ForwardStatus};

/// How often the loops look (renewals happen at half a lease; this only needs to be finer than that).
pub const TICK: Duration = Duration::from_secs(30);
/// After a failure, wait this long before bothering the router again.
pub const RETRY_AFTER: Duration = Duration::from_secs(300);
const PROTOS: [Proto; 2] = [Proto::Tcp, Proto::Udp];

/// What should be forwarded right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Want {
    /// The server's folder: the key of everything.
    pub dir: PathBuf,
    pub wanted: Wanted,
}

struct Active {
    wanted: Wanted,
    mapper: Box<dyn PortMapper>,
    renew_at: Instant,
}

pub type Finder = Box<dyn FnMut() -> Vec<Box<dyn PortMapper>> + Send>;
pub type Record<'a> = &'a mut dyn FnMut(&Path, Option<ForwardStatus>);

pub struct Keeper {
    find: Finder,
    active: HashMap<PathBuf, Active>,
    retry_at: HashMap<PathBuf, Instant>,
    now_unix: fn() -> u64,
}

fn success_text(via: &str, port: u16, permanent: bool) -> String {
    if permanent {
        format!("Your router ({via}) forwards port {port} (TCP and UDP). It keeps the rule until the server stops.")
    } else {
        format!("Your router ({via}) forwards port {port} (TCP and UDP) while the server runs.")
    }
}

/// The most useful reason out of several routers' answers.
fn best_error(errors: Vec<MapError>) -> MapError {
    errors
        .into_iter()
        .max_by_key(|e| match e {
            MapError::Refused(_) => 4,
            MapError::Protocol(_) => 3,
            MapError::Io(_) => 2,
            MapError::Timeout => 1,
            MapError::NotFound => 0,
        })
        .unwrap_or(MapError::NotFound)
}

impl Keeper {
    pub fn new(find: Finder) -> Self {
        Self { find, active: HashMap::new(), retry_at: HashMap::new(), now_unix: crate::monitor::now_unix }
    }

    /// The real network.
    pub fn system() -> Self {
        Self::new(Box::new(|| super::discover(&Discover::default())))
    }

    pub fn is_mapped(&self, dir: &Path) -> bool {
        self.active.contains_key(dir)
    }

    fn remove_both(a: &mut Active) {
        for p in PROTOS {
            let _ = a.mapper.remove(&a.wanted, p);
        }
    }

    fn add_both(m: &mut dyn PortMapper, w: &Wanted) -> Result<Granted, MapError> {
        let mut first = None;
        for p in PROTOS {
            match m.add(w, p, LEASE_SECS) {
                Ok(g) => {
                    first.get_or_insert(g);
                }
                Err(e) => {
                    // All or nothing: a TCP-only forward lets the server list show the server and
                    // every player fail to join.
                    for q in PROTOS {
                        let _ = m.remove(w, q);
                    }
                    return Err(e);
                }
            }
        }
        Ok(first.expect("two protocols"))
    }

    /// One look: make `want` true. `record` receives what to show the person for each server folder.
    pub fn tick(&mut self, now: Instant, want: &[Want], record: Record<'_>) {
        // Stop forwarding what is no longer wanted (server stopped, deleted, option switched off, port changed).
        let gone: Vec<PathBuf> = self.active.iter().filter(|(d, a)| !want.iter().any(|w| &w.dir == *d && w.wanted == a.wanted)).map(|(d, _)| d.clone()).collect();
        for d in gone {
            if let Some(mut a) = self.active.remove(&d) {
                Self::remove_both(&mut a);
            }
            if !want.iter().any(|w| w.dir == d) {
                record(&d, None);
            }
        }
        for w in want {
            if let Some(a) = self.active.get_mut(&w.dir) {
                if now < a.renew_at {
                    continue;
                }
                match Self::add_both(a.mapper.as_mut(), &a.wanted) {
                    Ok(g) => {
                        a.renew_at = now + renew_in(g);
                    }
                    Err(e) => {
                        self.active.remove(&w.dir);
                        self.retry_at.insert(w.dir.clone(), now + RETRY_AFTER);
                        record(&w.dir, Some(self.failed(&format!("Renewing the port forwarding failed. {e}"))));
                    }
                }
                continue;
            }
            if self.retry_at.get(&w.dir).is_some_and(|t| now < *t) {
                continue;
            }
            self.retry_at.remove(&w.dir);
            let mut errors = Vec::new();
            let mut done = None;
            for mut m in (self.find)() {
                match Self::add_both(m.as_mut(), &w.wanted) {
                    Ok(g) => {
                        done = Some((m, g));
                        break;
                    }
                    Err(e) => errors.push(e),
                }
            }
            match done {
                Some((mut m, g)) => {
                    let via = m.name();
                    let external_ip = m.external_ip().ok().map(|i| i.to_string());
                    record(
                        &w.dir,
                        Some(ForwardStatus {
                            at: (self.now_unix)(),
                            ok: true,
                            via: Some(via.to_string()),
                            message: success_text(via, w.wanted.port(), g.lease == 0),
                            external_ip,
                            permanent: g.lease == 0,
                        }),
                    );
                    self.active.insert(w.dir.clone(), Active { wanted: w.wanted.clone(), mapper: m, renew_at: now + renew_in(g) });
                }
                None => {
                    self.retry_at.insert(w.dir.clone(), now + RETRY_AFTER);
                    let e = best_error(errors);
                    record(&w.dir, Some(self.failed(&e.to_string())));
                }
            }
        }
    }

    fn failed(&self, message: &str) -> ForwardStatus {
        ForwardStatus { at: (self.now_unix)(), ok: false, via: None, message: message.to_string(), external_ip: None, permanent: false }
    }

    /// Remove everything this keeper forwarded (the server or the panel is going away).
    pub fn release_all(&mut self, record: Record<'_>) {
        let all: Vec<PathBuf> = self.active.keys().cloned().collect();
        for d in all {
            if let Some(mut a) = self.active.remove(&d) {
                Self::remove_both(&mut a);
            }
            record(&d, None);
        }
    }
}

fn renew_in(g: Granted) -> Duration {
    if g.lease == 0 {
        Duration::from_secs(3600)
    } else {
        Duration::from_secs(u64::from(g.lease / 2).max(30))
    }
}

/// This computer's IPv4 address on the network (the "client" of a mapping).
fn lan_v4() -> Option<std::net::Ipv4Addr> {
    match crate::launch::lan_address()? {
        IpAddr::V4(v4) => Some(v4),
        _ => None,
    }
}

/// What the server in `dir` wants forwarded: nothing unless the person switched it on.
pub fn want_for_dir(dir: &Path, name: &str, game_port: u16) -> Option<Want> {
    want_with(dir, name, game_port, lan_v4())
}

fn want_with(dir: &Path, name: &str, game_port: u16, lan: Option<std::net::Ipv4Addr>) -> Option<Want> {
    if !settings::load_server(dir).forward {
        return None;
    }
    let wanted = Wanted::new(game_port, lan?, name).ok()?;
    Some(Want { dir: dir.to_path_buf(), wanted })
}

fn sleep_unless(stop: &AtomicBool, d: Duration) {
    let end = Instant::now() + d;
    while !stop.load(Ordering::SeqCst) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn record_to_disk(dir: &Path, st: Option<ForwardStatus>) {
    settings::record_status(dir, st);
}

/// Lives inside `kora service-host`: forwards while FXServer runs, removes the mapping when dropped.
pub struct ServerGuard {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ServerGuard {
    /// `dir`: the server folder (its `kora-server.json` says the game port and name).
    pub fn start(dir: PathBuf) -> Self {
        Self::start_with(dir, Keeper::system(), TICK, lan_v4)
    }

    pub fn start_with(dir: PathBuf, mut keeper: Keeper, tick: Duration, lan: fn() -> Option<std::net::Ipv4Addr>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        let thread = std::thread::Builder::new().name("kora-port-forward".into()).spawn(move || {
            while !s2.load(Ordering::SeqCst) {
                let want: Vec<Want> = crate::installer::registry::ServerFile::load(&dir).ok().and_then(|f| want_with(&dir, &f.name, f.ports.game, lan())).into_iter().collect();
                keeper.tick(Instant::now(), &want, &mut |d, st| record_to_disk(d, st));
                sleep_unless(&s2, tick);
            }
            keeper.release_all(&mut |d, st| record_to_disk(d, st));
        });
        Self { stop, thread: thread.ok() }
    }
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Handle of the panel's keeper; dropping or [`stop`](Self::stop) removes what it forwarded.
pub struct PanelKeeper {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl PanelKeeper {
    pub fn stop(mut self) {
        self.stop_inner();
    }
    fn stop_inner(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for PanelKeeper {
    fn drop(&mut self) {
        self.stop_inner();
    }
}

/// Start the panel-side keeper: every [`TICK`] it forwards the game port of each registered server that
/// is running and has the option on.
pub fn spawn_panel_keeper() -> PanelKeeper {
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = stop.clone();
    let thread = std::thread::Builder::new().name("kora-port-forward-panel".into()).spawn(move || {
        let mut keeper = Keeper::system();
        while !s2.load(Ordering::SeqCst) {
            let want = panel_wants();
            keeper.tick(Instant::now(), &want, &mut |d, st| record_to_disk(d, st));
            sleep_unless(&s2, TICK);
        }
        keeper.release_all(&mut |d, st| record_to_disk(d, st));
    });
    PanelKeeper { stop, thread: thread.ok() }
}

fn panel_wants() -> Vec<Want> {
    let Ok(reg) = crate::installer::registry::Registry::load() else { return Vec::new() };
    let env = crate::launch::SystemEnv::default();
    reg.servers
        .iter()
        .filter(|e| settings::load_server(&e.dir).forward)
        .filter(|e| {
            let spec = crate::launch::Spec { name: e.name.clone(), dir: e.dir.clone(), game_port: e.ports.game, txadmin_port: e.ports.txadmin };
            e.dir.is_dir() && crate::launch::status_of(&env, &spec).running
        })
        .filter_map(|e| want_for_dir(&e.dir, &e.name, e.ports.game))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Router {
        maps: Vec<(Proto, u16)>,
        adds: u32,
        refuse: Option<MapError>,
        fail_udp: bool,
    }
    struct Fake(Arc<Mutex<Router>>);
    impl PortMapper for Fake {
        fn name(&self) -> &'static str {
            "UPnP"
        }
        fn gateway(&self) -> std::net::Ipv4Addr {
            "192.168.1.1".parse().unwrap()
        }
        fn external_ip(&mut self) -> Result<std::net::Ipv4Addr, MapError> {
            Ok("203.0.113.5".parse().unwrap())
        }
        fn add(&mut self, w: &Wanted, p: Proto, lease: u32) -> Result<Granted, MapError> {
            let mut r = self.0.lock().unwrap();
            if let Some(e) = r.refuse.clone() {
                return Err(e);
            }
            if r.fail_udp && p == Proto::Udp {
                return Err(MapError::Refused("udp no".into()));
            }
            r.adds += 1;
            r.maps.retain(|m| !(m.0 == p && m.1 == w.port()));
            r.maps.push((p, w.port()));
            Ok(Granted { external_port: w.port(), lease })
        }
        fn remove(&mut self, w: &Wanted, p: Proto) -> Result<(), MapError> {
            self.0.lock().unwrap().maps.retain(|m| !(m.0 == p && m.1 == w.port()));
            Ok(())
        }
    }

    fn keeper(r: &Arc<Mutex<Router>>) -> Keeper {
        let r = r.clone();
        let mut k = Keeper::new(Box::new(move || vec![Box::new(Fake(r.clone())) as Box<dyn PortMapper>]));
        k.now_unix = || 100;
        k
    }
    fn want(dir: &str, port: u16) -> Want {
        Want { dir: dir.into(), wanted: Wanted::new(port, "192.168.1.50".parse().unwrap(), "main").unwrap() }
    }

    #[test]
    fn maps_renews_at_half_a_lease_and_removes_when_no_longer_wanted() {
        let r: Arc<Mutex<Router>> = Arc::default();
        let mut k = keeper(&r);
        let t0 = Instant::now();
        let mut rec: Vec<(PathBuf, Option<ForwardStatus>)> = vec![];
        let w = [want("/srv/a", 30120)];
        k.tick(t0, &w, &mut |d, s| rec.push((d.to_path_buf(), s)));
        assert_eq!(r.lock().unwrap().maps.len(), 2);
        let st = rec[0].1.clone().unwrap();
        assert!(st.ok && st.via.as_deref() == Some("UPnP") && st.external_ip.as_deref() == Some("203.0.113.5"));
        // Nothing to do before half the lease.
        k.tick(t0 + Duration::from_secs(1000), &w, &mut |_, _| panic!("no record"));
        assert_eq!(r.lock().unwrap().adds, 2);
        // Renewed after half.
        k.tick(t0 + Duration::from_secs(1801), &w, &mut |_, _| panic!("renewal is silent"));
        assert_eq!(r.lock().unwrap().adds, 4);
        // The server stopped: removed, status cleared.
        k.tick(t0 + Duration::from_secs(1900), &[], &mut |d, s| rec.push((d.to_path_buf(), s)));
        assert!(r.lock().unwrap().maps.is_empty());
        assert_eq!(rec.last().unwrap().1, None);
    }

    #[test]
    fn never_maps_more_than_the_wanted_port() {
        let r: Arc<Mutex<Router>> = Arc::default();
        let mut k = keeper(&r);
        k.tick(Instant::now(), &[want("/srv/a", 30120)], &mut |_, _| {});
        let m = r.lock().unwrap().maps.clone();
        assert!(m.iter().all(|x| x.1 == 30120), "{m:?}");
        // A changed port replaces the old mapping.
        k.tick(Instant::now(), &[want("/srv/a", 30125)], &mut |_, _| {});
        let m = r.lock().unwrap().maps.clone();
        assert!(m.iter().all(|x| x.1 == 30125) && m.len() == 2, "{m:?}");
    }

    #[test]
    fn a_refusal_is_reported_and_not_hammered() {
        let r: Arc<Mutex<Router>> = Arc::default();
        r.lock().unwrap().refuse = Some(MapError::Refused("read-only".into()));
        let mut k = keeper(&r);
        let t0 = Instant::now();
        let mut rec = vec![];
        k.tick(t0, &[want("/srv/a", 30120)], &mut |_, s| rec.push(s));
        let s = rec[0].clone().unwrap();
        assert!(!s.ok && s.message.contains("read-only"), "{s:?}");
        // Within the retry delay the router is left alone (the adds counter would show it).
        r.lock().unwrap().refuse = None;
        k.tick(t0 + Duration::from_secs(60), &[want("/srv/a", 30120)], &mut |_, _| panic!("quiet"));
        assert_eq!(r.lock().unwrap().adds, 0);
        k.tick(t0 + RETRY_AFTER + Duration::from_secs(1), &[want("/srv/a", 30120)], &mut |_, s| rec.push(s));
        assert!(rec.last().unwrap().as_ref().unwrap().ok);
    }

    #[test]
    fn half_a_mapping_is_rolled_back() {
        let r: Arc<Mutex<Router>> = Arc::default();
        r.lock().unwrap().fail_udp = true;
        let mut k = keeper(&r);
        k.tick(Instant::now(), &[want("/srv/a", 30120)], &mut |_, _| {});
        assert!(r.lock().unwrap().maps.is_empty());
    }

    #[test]
    fn release_all_removes_everything() {
        let r: Arc<Mutex<Router>> = Arc::default();
        let mut k = keeper(&r);
        k.tick(Instant::now(), &[want("/srv/a", 30120), want("/srv/b", 30130)], &mut |_, _| {});
        assert_eq!(r.lock().unwrap().maps.len(), 4);
        let mut cleared = 0;
        k.release_all(&mut |_, s| {
            assert!(s.is_none());
            cleared += 1;
        });
        assert_eq!(cleared, 2);
        assert!(r.lock().unwrap().maps.is_empty());
    }

    #[test]
    fn nothing_is_wanted_unless_switched_on() {
        let d = tempfile::tempdir().unwrap();
        assert!(want_for_dir(d.path(), "main", 30120).is_none());
    }

    #[test]
    fn guard_removes_the_mapping_when_dropped() {
        let d = tempfile::tempdir().unwrap();
        settings::save_server(d.path(), &settings::ServerReach { forward: true, ..Default::default() }).unwrap();
        let sf = crate::installer::registry::ServerFile {
            name: "main".into(),
            mode: crate::installer::Mode::Production,
            dir: d.path().to_path_buf(),
            ports: crate::installer::registry::PortsRecord { game: 30120, txadmin: 40120 },
            game_build: 0,
            artifact_build: 0,
            database: None,
            github_owner: None,
            created_unix: 0,
            kora_version: "t".into(),
            resources: vec![],
        };
        sf.save().unwrap();
        let r: Arc<Mutex<Router>> = Arc::default();
        let g = ServerGuard::start_with(d.path().to_path_buf(), keeper(&r), Duration::from_millis(50), || Some("192.168.1.50".parse().unwrap()));
        for _ in 0..50 {
            if r.lock().unwrap().maps.len() == 2 {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(r.lock().unwrap().maps.len(), 2);
        assert!(settings::load_server(d.path()).status.is_some_and(|s| s.ok));
        drop(g);
        assert!(r.lock().unwrap().maps.is_empty());
        assert!(settings::load_server(d.path()).status.is_none());
        assert!(settings::load_server(d.path()).forward, "the choice stays");
    }
}
