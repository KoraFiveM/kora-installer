use super::*;
use std::cell::{Cell, RefCell};

/// Scripted machine: time only moves when the code sleeps.
struct Fake {
    clock: Cell<Duration>,
    svc: RefCell<ServiceState>,
    svc_fail: bool,
    spawn_fail: Option<String>,
    spawned: RefCell<Vec<u32>>,
    service_starts: Cell<u32>,
    killed: RefCell<Vec<u32>>,
    pid_file: RefCell<Option<u32>>,
    alive: RefCell<Vec<u32>>,
    /// The spawned process dies at this time.
    dies_at: Option<Duration>,
    tx_after: Option<Duration>,
    game_after: Option<Duration>,
    /// Ports busy from the start (another program).
    busy: Vec<u16>,
    /// (time, text) appended to the log.
    log: Vec<(Duration, String)>,
    /// Log content that already exists before the start.
    old_log: String,
}

fn secs(s: u64) -> Duration {
    Duration::from_secs(s)
}

impl Fake {
    fn new() -> Self {
        Fake {
            clock: Cell::new(Duration::ZERO),
            svc: RefCell::new(ServiceState::NotInstalled),
            svc_fail: false,
            spawn_fail: None,
            spawned: RefCell::new(vec![]),
            service_starts: Cell::new(0),
            killed: RefCell::new(vec![]),
            pid_file: RefCell::new(None),
            alive: RefCell::new(vec![]),
            dies_at: None,
            tx_after: Some(secs(5)),
            game_after: Some(secs(9)),
            busy: vec![],
            log: vec![],
            old_log: String::new(),
        }
    }
    fn started_at(&self) -> Duration {
        Duration::ZERO
    }
    fn running_now(&self) -> bool {
        let t = self.clock.get();
        (!self.spawned.borrow().is_empty() || *self.svc.borrow() == ServiceState::Running) && !matches!(self.dies_at, Some(d) if t >= d)
    }
}

fn spec() -> Spec {
    Spec { name: "alpha".into(), dir: PathBuf::from("/srv/alpha"), game_port: 31120, txadmin_port: 41120 }
}

impl LaunchEnv for Fake {
    fn service_state(&self, _: &Spec) -> ServiceState {
        self.svc.borrow().clone()
    }
    fn service_start(&self, _: &Spec) -> Result<(), String> {
        if self.svc_fail {
            return Err("access denied".into());
        }
        self.service_starts.set(self.service_starts.get() + 1);
        *self.svc.borrow_mut() = ServiceState::Running;
        Ok(())
    }
    fn spawn_detached(&self, _: &Spec, _: &Path) -> Result<u32, String> {
        if let Some(e) = &self.spawn_fail {
            return Err(e.clone());
        }
        self.spawned.borrow_mut().push(4242);
        self.alive.borrow_mut().push(4242);
        Ok(4242)
    }
    fn pid_alive(&self, _: &Spec, pid: u32) -> bool {
        if pid == 4242 && !self.spawned.borrow().is_empty() {
            return self.running_now() && !self.killed.borrow().contains(&pid);
        }
        self.alive.borrow().contains(&pid) && !self.killed.borrow().contains(&pid)
    }
    fn txadmin_answers(&self, port: u16) -> bool {
        if self.busy.contains(&port) {
            return false;
        }
        self.running_now() && self.tx_after.is_some_and(|a| self.clock.get() >= a)
    }
    fn port_listening(&self, port: u16) -> bool {
        self.busy.contains(&port) || (port == 31120 && self.running_now() && self.game_after.is_some_and(|a| self.clock.get() >= a))
    }
    fn log_len(&self, _: &Path) -> u64 {
        self.old_log.len() as u64
    }
    fn read_log(&self, _: &Path, from: u64) -> String {
        let mut all = self.old_log.clone();
        for (t, s) in &self.log {
            if self.clock.get() >= *t {
                all.push_str(s);
            }
        }
        all[(from as usize).min(all.len())..].to_string()
    }
    fn read_pid_file(&self, _: &Spec) -> Option<u32> {
        *self.pid_file.borrow()
    }
    fn write_pid_file(&self, _: &Spec, pid: u32) {
        *self.pid_file.borrow_mut() = Some(pid);
    }
    fn remove_pid_file(&self, _: &Spec) {
        *self.pid_file.borrow_mut() = None;
    }
    fn kill_tree(&self, pid: u32) -> bool {
        self.killed.borrow_mut().push(pid);
        true
    }
    fn sleep(&self, d: Duration) {
        self.clock.set(self.clock.get() + d);
    }
    fn now(&self) -> Duration {
        self.clock.get() + self.started_at()
    }
}

fn run(f: &Fake) -> (Report, Vec<String>) {
    let mut seen: Vec<String> = vec![];
    let r = start(f, &spec(), &mut |m, _| {
        if seen.last().map(String::as_str) != Some(m) {
            seen.push(m.to_string())
        }
    });
    (r, seen)
}

#[test]
fn starts_the_registered_service_instead_of_a_process() {
    let f = Fake::new();
    *f.svc.borrow_mut() = ServiceState::Stopped;
    let (r, seen) = run(&f);
    assert!(r.running(), "{r:?}");
    assert_eq!(r.method, Method::Service);
    assert_eq!(f.service_starts.get(), 1);
    assert!(f.spawned.borrow().is_empty(), "no detached process when a service exists");
    assert!(r.game_listening);
    assert_eq!(seen, ["Starting txAdmin...", "Waiting for the game server..."]);
}

#[test]
fn a_refused_service_start_is_a_plain_failure() {
    let mut f = Fake::new();
    f.svc_fail = true;
    *f.svc.borrow_mut() = ServiceState::Stopped;
    let (r, _) = run(&f);
    assert!(!r.running());
    assert_eq!(r.problem, Some(Problem::StartFailed));
    assert!(r.message.contains("access denied"));
}

#[test]
fn starts_detached_and_records_the_pid() {
    let f = Fake::new();
    let (r, _) = run(&f);
    assert!(r.running(), "{r:?}");
    assert_eq!((r.method, r.pid), (Method::Detached, Some(4242)));
    assert_eq!(*f.pid_file.borrow(), Some(4242));
    assert!(r.txadmin_up && r.game_listening);
    assert_eq!(f.spawned.borrow().len(), 1);
}

#[test]
fn txadmin_up_but_game_port_slow_is_reported_as_not_listening_yet() {
    let mut f = Fake::new();
    f.game_after = None;
    let (r, _) = run(&f);
    assert!(r.running());
    assert!(r.txadmin_up && !r.game_listening);
    assert!(r.message.contains("not listening yet"));
    assert!(f.clock.get() >= secs(5) + GAME_GRACE, "waited the whole grace period");
}

#[test]
fn nothing_answering_times_out_after_two_minutes() {
    let mut f = Fake::new();
    f.tx_after = None;
    f.game_after = None;
    f.log = vec![(secs(1), "booting...\nstill booting\n".into())];
    let (r, _) = run(&f);
    assert_eq!(r.problem, Some(Problem::Timeout));
    assert!(!r.running() && !r.txadmin_up);
    assert!(f.clock.get() >= TXADMIN_TIMEOUT && f.clock.get() < TXADMIN_TIMEOUT + secs(3));
    assert_eq!(r.log_tail.last().map(String::as_str), Some("still booting"));
}

#[test]
fn a_rejected_licence_is_not_success_even_though_txadmin_is_up() {
    let mut f = Fake::new();
    f.game_after = None;
    f.log = vec![(secs(7), "[txAdmin] started\nCould not authenticate server license key: Invalid license\n".into())];
    let (r, _) = run(&f);
    assert_eq!(r.problem, Some(Problem::LicenceRejected));
    assert!(!r.running());
    assert!(r.txadmin_up, "txAdmin itself did come up");
    assert_eq!(r.message, "Your licence key was rejected. Check it at portal.cfx.re, then run kora start");
    assert!(r.log_tail.iter().any(|l| l.contains("Invalid license")));
}

#[test]
fn old_licence_errors_in_the_log_do_not_count_for_a_new_start() {
    let mut f = Fake::new();
    f.old_log = "Invalid license key from an earlier run\n".into();
    let (r, _) = run(&f);
    assert!(r.running(), "{r:?}");
}

#[test]
fn a_crash_within_ten_seconds_shows_the_last_log_lines() {
    let mut f = Fake::new();
    f.tx_after = None;
    f.dies_at = Some(secs(3));
    f.log = vec![(secs(1), "line1\nline2\nSegmentation fault\n".into())];
    let (r, _) = run(&f);
    assert_eq!(r.problem, Some(Problem::Crashed));
    assert_eq!(r.message, "The server stopped right after starting. The last lines of its log are below; the full log is in the panel's Logs tab.");
    assert_eq!(r.log_tail, ["line1", "line2", "Segmentation fault"]);
    assert_eq!(*f.pid_file.borrow(), None, "the dead pid is forgotten");
}

#[test]
fn a_port_clash_at_start_names_the_problem_and_starts_nothing() {
    let mut f = Fake::new();
    f.busy = vec![31120];
    let (r, _) = run(&f);
    assert_eq!(r.problem, Some(Problem::PortInUse));
    assert!(f.spawned.borrow().is_empty());
}

#[test]
fn a_bind_error_in_the_log_is_a_port_clash() {
    let mut f = Fake::new();
    f.tx_after = None;
    f.dies_at = Some(secs(2));
    f.log = vec![(secs(1), "Error: listen EADDRINUSE: address already in use 0.0.0.0:41120\n".into())];
    let (r, _) = run(&f);
    assert_eq!(r.problem, Some(Problem::PortInUse));
    assert!(!r.log_tail.is_empty());
    assert_eq!(r.message, "The server stopped because port 41120 is already used by another program. Change the port in the panel's Ports tab, or stop the other program.");
}

#[test]
fn a_port_clash_names_a_port_only_when_the_log_line_has_one_of_ours() {
    let s = spec();
    assert!(port_clash_message("ok 31120\nbind: Address already in use [::]:31120\n", &s).contains("because port 31120 is already used"));
    // Another number on the line (or our port on another line) is not taken for the port.
    let m = port_clash_message("listening on 41120\nError 10048: failed to bind 0.0.0.0:8080\n", &s);
    assert!(m.contains("one of its ports (31120 for players, 41120 for txAdmin)") && !m.contains("8080"), "{m}");
    assert!(m.ends_with("Change the port in the panel's Ports tab, or stop the other program."));
}

#[test]
fn already_running_starts_no_second_instance() {
    let f = Fake::new();
    f.alive.borrow_mut().push(777);
    *f.pid_file.borrow_mut() = Some(777);
    let mut f = f;
    f.tx_after = Some(Duration::ZERO);
    f.game_after = Some(Duration::ZERO);
    // 777 is alive, but running_now() (the fake's txAdmin) needs something spawned: make it answer.
    f.spawned.borrow_mut().push(777);
    let (r, _) = run(&f);
    assert_eq!(r.method, Method::AlreadyRunning);
    assert!(r.running());
    assert_eq!(f.spawned.borrow().len(), 1, "nothing new spawned");
    assert_eq!(f.service_starts.get(), 0);
}

#[test]
fn a_running_service_is_not_started_again() {
    let f = Fake::new();
    *f.svc.borrow_mut() = ServiceState::Running;
    let (r, _) = run(&f);
    assert_eq!(r.method, Method::AlreadyRunning);
    assert_eq!(f.service_starts.get(), 0);
}

#[test]
fn a_stale_pid_file_is_cleaned_up_and_the_server_starts() {
    let f = Fake::new();
    *f.pid_file.borrow_mut() = Some(31337); // not alive
    let (r, _) = run(&f);
    assert!(r.running(), "{r:?}");
    assert_eq!(r.method, Method::Detached);
    assert_eq!(*f.pid_file.borrow(), Some(4242), "replaced by the new pid");
}

#[test]
fn a_server_stuck_on_a_rejected_licence_is_restarted_not_left_alone() {
    let mut f = Fake::new();
    f.alive.borrow_mut().push(555);
    *f.pid_file.borrow_mut() = Some(555);
    f.game_after = None;
    f.old_log = "Could not authenticate server license key\n".into();
    let (r, _) = run(&f);
    assert_eq!(*f.killed.borrow(), [555], "the old process was stopped");
    assert_eq!(f.spawned.borrow().len(), 1, "and a new one started");
    assert!(r.method == Method::Detached);
}

#[test]
fn a_spawn_error_is_reported() {
    let mut f = Fake::new();
    f.spawn_fail = Some("FXServer.exe is missing".into());
    let (r, _) = run(&f);
    assert_eq!(r.problem, Some(Problem::StartFailed));
    assert!(r.message.contains("missing"));
    assert_eq!(*f.pid_file.borrow(), None);
}

#[test]
fn stop_detached_kills_the_recorded_pid_and_clears_the_file() {
    let f = Fake::new();
    f.alive.borrow_mut().push(900);
    *f.pid_file.borrow_mut() = Some(900);
    assert!(stop_detached(&f, &spec()));
    assert_eq!(*f.killed.borrow(), [900]);
    assert_eq!(*f.pid_file.borrow(), None);
    assert!(!stop_detached(&f, &spec()), "nothing left to stop");
}

#[test]
fn log_line_matchers() {
    assert!(licence_rejected("x\nCould not authenticate server license key.\n"));
    assert!(licence_rejected("Invalid license key"));
    assert!(!licence_rejected("Server license key authentication succeeded"));
    assert!(port_clash("bind: Address already in use"));
    assert!(!port_clash("all fine"));
    assert_eq!(last_lines("a\n\nb\nc\n", 2), ["b", "c"]);
    assert_eq!(last_lines("\x1b[38;5;66m[ svadhesive] \x1b[0m\x1b[0mAuthenticating\n", 5), ["[ svadhesive] Authenticating"]);
}

#[test]
fn command_lines_are_quoted_for_paths_with_spaces() {
    assert_eq!(command_line_text(r"C:\Program Files\Kora\FXServer.exe", &[]), r#""C:\Program Files\Kora\FXServer.exe""#);
    assert_eq!(command_line_text("kora", &["start".into(), "my server".into(), String::new(), r#"a"b"#.into()]), r#"kora start "my server" "" "a\"b""#);
}
