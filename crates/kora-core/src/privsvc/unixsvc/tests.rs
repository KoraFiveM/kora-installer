use super::*;
use std::os::unix::fs::PermissionsExt;

fn my_uid() -> u32 {
    unsafe { libc::getuid() }
}

struct Env {
    _d: tempfile::TempDir,
    svc: LinuxSvc,
    srv: PathBuf,
    marker: PathBuf,
}

/// A service whose "kora" is a shell script that prints two lines and a result, and leaves a marker.
fn env(allow_me: bool) -> Env {
    let d = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(d.path()).unwrap();
    let marker = root.join("ran");
    let exe = root.join("fake-kora");
    std::fs::write(&exe, format!("#!/bin/sh\necho \"$@\" > {m}\necho hello\necho \"key is $CFX_LICENSE_KEY\"\necho 'KORA-ADMIN-RESULT {{\"detail\":\"made\"}}'\n", m = marker.display())).unwrap();
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    let srv = root.join("srv");
    std::fs::create_dir_all(srv.join(".kora")).unwrap();
    std::fs::write(srv.join(".kora/request.json"), r#"{"name":"Main","ports":{"game":30120,"txadmin":40120}}"#).unwrap();
    let mut p = Policy::new("test");
    if allow_me {
        p.allow(AccountId::uid(my_uid()), "me").unwrap();
    }
    std::fs::write(root.join("service.json"), p.to_text()).unwrap();
    let files = Files { policy: root.join("service.json"), ledger: root.join("owners.json"), audit: root.join("audit.log") };
    Env { svc: LinuxSvc { exe, files, strict_owner: false }, srv, marker, _d: d }
}

fn me() -> Caller {
    Caller { id: Some(AccountId::uid(my_uid())), integrity: None, is_root: false, remote: false }
}

fn ask(e: &Env, caller: Caller, req: &Request) -> Vec<Message> {
    let (a, b) = UnixStream::pair().unwrap();
    let mut a2 = a.try_clone().unwrap();
    let line = serde_json::to_string(req).unwrap() + "\n";
    a2.write_all(line.as_bytes()).unwrap();
    handle_connection(&e.svc, b, caller);
    BufReader::new(a).lines().map_while(Result::ok).filter_map(|l| serde_json::from_str(&l).ok()).collect()
}

fn step(e: &Env, extra: &[&str]) -> Request {
    let mut args: Vec<String> = ["run", "--step", "system.firewall", "--dir"].iter().map(|s| s.to_string()).collect();
    args.push(e.srv.to_string_lossy().into_owned());
    args.extend(extra.iter().map(|s| s.to_string()));
    Request { verb: "admin-step".into(), args, env: BTreeMap::new() }
}

#[test]
fn an_allowed_account_gets_its_step_done_streamed_and_recorded() {
    let e = env(true);
    let mut req = step(&e, &[]);
    req.env.insert("CFX_LICENSE_KEY".into(), "cfxk_topsecret".into());
    let out = ask(&e, me(), &req);
    let lines: Vec<&str> = out.iter().filter_map(|m| if let Message::Progress { line, .. } = m { Some(line.as_str()) } else { None }).collect();
    assert_eq!(lines, ["hello", "key is ***"], "secrets are masked: {out:?}");
    assert!(matches!(out.last(), Some(Message::Result { ok: true, data: Some(d), .. }) if d["detail"] == "made"), "{out:?}");
    let args = std::fs::read_to_string(&e.marker).unwrap();
    assert!(args.starts_with("--no-elevate admin-step run --step system.firewall --dir "), "{args}");
    let ledger = std::fs::read_to_string(e.svc.files.ledger.clone()).unwrap();
    assert!(ledger.contains("firewall:main") && ledger.contains(&format!("uid:{}", my_uid())), "{ledger}");
    let audit = std::fs::read_to_string(&e.svc.files.audit).unwrap();
    assert!(audit.contains("run system.firewall | done") && !audit.contains("cfxk") && !audit.contains(&*e.srv.to_string_lossy()), "{audit}");
}

#[test]
fn another_account_is_refused_and_nothing_runs() {
    let e = env(true);
    let other = Caller { id: Some(AccountId::uid(my_uid() + 1)), ..me() };
    let out = ask(&e, other, &step(&e, &[]));
    assert!(matches!(out.as_slice(), [Message::Error { .. }]), "{out:?}");
    assert!(!e.marker.exists(), "the program must not have run");
    let none = Caller { id: None, ..me() };
    assert!(matches!(ask(&e, none, &step(&e, &[])).as_slice(), [Message::Error { .. }]));
    let e2 = env(false);
    assert!(matches!(ask(&e2, me(), &step(&e2, &[])).as_slice(), [Message::Error { .. }]), "not on the list");
    assert!(!e2.marker.exists());
}

#[test]
fn ping_says_whether_this_account_is_served() {
    let e = env(true);
    let ping = Request { verb: PING.into(), args: vec![], env: BTreeMap::new() };
    assert!(matches!(ask(&e, me(), &ping).as_slice(), [Message::Result { ok: true, version: Some(_), .. }]));
    let other = Caller { id: Some(AccountId::uid(my_uid() + 1)), ..me() };
    assert!(matches!(ask(&e, other, &ping).as_slice(), [Message::Result { ok: false, message: Some(_), .. }]));
}

#[test]
fn junk_and_forbidden_requests_are_refused_without_running() {
    let e = env(true);
    for (verb, args) in [
        ("install", vec!["--dir", "/tmp/x"]),
        ("uninstall", vec!["--yes"]),
        ("admin-step", vec!["run", "--step", "system.service", "--dir", "relative"]),
        ("schedule", vec!["set", "a", "--as-system"]),
    ] {
        let r = Request { verb: verb.into(), args: args.iter().map(|s| s.to_string()).collect(), env: BTreeMap::new() };
        assert!(matches!(ask(&e, me(), &r).as_slice(), [Message::Error { .. }]), "{verb}");
    }
    let mut r = step(&e, &[]);
    r.env.insert("LD_PRELOAD".into(), "/tmp/x.so".into());
    assert!(matches!(ask(&e, me(), &r).as_slice(), [Message::Error { .. }]));
    assert!(!e.marker.exists());
    // not even JSON
    let (a, b) = UnixStream::pair().unwrap();
    (&a).write_all(b"hello\n").unwrap();
    handle_connection(&e.svc, b, me());
    let got: Vec<String> = BufReader::new(a).lines().map_while(Result::ok).collect();
    assert!(got[0].contains("error"));
}

#[test]
fn a_folder_with_a_link_on_the_way_is_refused() {
    let e = env(true);
    let root = e.srv.parent().unwrap().to_path_buf();
    std::os::unix::fs::symlink(&e.srv, root.join("alias")).unwrap();
    assert!(path_has_user_link(&root.join("alias")).unwrap());
    assert!(!path_has_user_link(&e.srv).unwrap());
    let mut req = step(&e, &[]);
    req.args[4] = root.join("alias").to_string_lossy().into_owned();
    let out = ask(&e, me(), &req);
    assert!(matches!(out.as_slice(), [Message::Error { message }] if message.contains("link")), "{out:?}");
    assert!(!e.marker.exists());
}

#[test]
fn a_settings_file_others_can_write_serves_nobody() {
    let mut e = env(true);
    e.svc.strict_owner = true; // the test user is not root: so it is "not root-owned"
    if my_uid() != 0 {
        assert!(e.svc.policy().is_err());
        assert!(matches!(ask(&e, me(), &step(&e, &[])).as_slice(), [Message::Error { .. }]));
        assert!(!e.marker.exists());
    }
}

#[test]
fn over_a_real_socket_the_client_is_identified_by_the_system() {
    let e = env(true);
    let d = tempfile::tempdir().unwrap();
    let sock = d.path().join("service.sock");
    let l = bind(&sock).unwrap();
    let svc: Arc<dyn Svc> = Arc::new(LinuxSvc { exe: e.svc.exe.clone(), files: e.svc.files.clone(), strict_owner: false });
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = Arc::clone(&stop);
    let t = std::thread::spawn(move || serve(svc, l, &s2));
    assert_eq!(probe_unchecked(&sock).unwrap(), env!("CARGO_PKG_VERSION"));
    let mut seen = vec![];
    let a = call_with(&sock, "admin-step", &step(&e, &[]).args, &[], &mut |l| seen.push(l.to_string()), false).unwrap();
    assert!(a.ok && a.data.is_some() && seen == ["hello", "key is"], "{a:?} {seen:?}");
    if my_uid() != 0 {
        let r = call(&sock, PING, &[], &[], &mut |_| {});
        assert!(matches!(r, Err(ClientError::Unavailable(m)) if m.contains("not owned by root")), "a socket another user made gets nothing");
    }
    stop.store(true, Ordering::Relaxed);
    t.join().unwrap();
}

fn probe_unchecked(p: &Path) -> Result<String, ClientError> {
    let a = call_with(p, PING, &[], &[], &mut |_| {}, false)?;
    if a.ok {
        Ok(a.version.unwrap_or_default())
    } else {
        Err(ClientError::Refused(a.message.unwrap_or_default()))
    }
}

#[test]
fn the_program_must_be_in_the_protected_folder() {
    assert!(check_program_trusted(Path::new("/home/jo/.local/bin/kora")).unwrap_err().contains("protected program folder"));
    assert!(check_program_trusted(Path::new("/usr/lib/kora/../../tmp/x")).is_err());
}
