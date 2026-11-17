use super::*;
use std::path::PathBuf;

#[test]
fn pipe_names_are_random_looking_and_checked() {
    let n = pipe_name(4321, &[0xab; 16]);
    assert_eq!(n, format!(r"\\.\pipe\kora-admin-4321-{}", "ab".repeat(16)));
    assert!(valid_pipe_name(&n));
    for bad in [
        r"\\.\pipe\kora-helper",
        r"\\server\pipe\kora-admin-1-abababababababababababababababab",
        r"\\.\pipe\kora-admin--abababababababababababababababab",
        r"\\.\pipe\kora-admin-1-ABABABABABABABABABABABABABABABAB",
        r"\\.\pipe\kora-admin-1-abab",
        r"\\.\pipe\kora-admin-1-ababababababababababababababababx",
        r"\\.\pipe\kora-admin-x1-abababababababababababababababab",
    ] {
        assert!(!valid_pipe_name(bad), "{bad}");
    }
}

#[test]
fn the_worker_command_line_holds_no_secret_and_parses_back() {
    let n = pipe_name(77, &[1; 16]);
    let a = worker_args(&n, 77);
    assert_eq!(a[0], WORKER_COMMAND);
    assert_eq!(parse_worker_args(&a[1..]).unwrap(), (n.clone(), 77));
    assert!(parse_worker_args(&["--pipe".into(), n.clone()]).is_err(), "no parent");
    assert!(parse_worker_args(&["--pipe".into(), r"\\.\pipe\other".into(), "--parent".into(), "1".into()]).is_err());
    assert!(parse_worker_args(&["--pipe".into(), n, "--parent".into(), "0".into()]).is_err());
}

#[test]
fn the_pipe_is_for_elevated_administrators_and_system_only() {
    // No "Everyone", "Users", "Authenticated Users" or the user's own SID: only elevated tokens get in.
    for sid in ["WD", "BU", "AU", "IU", "AN", "NU"] {
        assert!(!PIPE_SDDL.contains(&format!(";;;{sid})")), "{sid}");
    }
    assert!(PIPE_SDDL.starts_with("D:P"), "protected: nothing inherited");
    assert!(PIPE_SDDL.contains("(A;;GRGW;;;BA)") && PIPE_SDDL.contains("(A;;GRGW;;;SY)"));
    assert!(PIPE_SDDL.contains("(A;;RC;;;OW)"), "the owner cannot rewrite the DACL");
}

#[test]
fn exe_paths_compare_like_windows_does() {
    let p = |s: &str| PathBuf::from(s);
    assert!(same_exe(&p(r"C:\Users\Jo Smith\AppData\Local\Kora\kora.exe"), &p(r"c:\users\jo smith\appdata\local\kora\KORA.EXE")));
    assert!(same_exe(&p(r"\\?\C:\Program Files\Kora\kora.exe"), &p(r"C:\Program Files\Kora\kora.exe")));
    assert!(same_exe(&p("C:/k/kora.exe"), &p(r"C:\k\kora.exe")));
    assert!(!same_exe(&p(r"C:\k\kora.exe"), &p(r"C:\k\kora2.exe")));
    assert!(!same_exe(&p(""), &p("")));
}

#[test]
fn peers_must_be_exactly_the_processes_we_expect() {
    let own: FileId = (0xabcd, 42);
    assert!(check_server_peer(10, 10, Some(own), Some(own), "x").is_ok(), "the same file, whatever the paths look like");
    assert!(check_server_peer(10, 11, Some(own), Some(own), "x").is_err(), "another process owns the pipe");
    assert!(check_server_peer(10, 10, Some((0xabcd, 43)), Some(own), r"C:\evil\kora.exe").unwrap_err().contains("evil"), "pid reused by another program, or a copy");
    assert!(check_server_peer(10, 10, Some((0xabce, 42)), Some(own), "x").is_err(), "same index on another volume");
    assert!(check_server_peer(10, 10, None, Some(own), "x").is_err());
    assert!(check_server_peer(10, 10, Some(own), None, "x").is_err());
    assert!(check_client_peer(5, 5).is_ok());
    assert!(check_client_peer(5, 6).is_err() && check_client_peer(0, 0).is_err());
}

#[test]
fn session_lines_accept_only_a_full_kora_folder() {
    assert_eq!(parse_session(&[]).unwrap(), None);
    let home = if cfg!(windows) { r"C:\Users\Jo Smith\.kora" } else { "/home/jo smith/.kora" };
    assert_eq!(parse_session(&["--kora-home".into(), home.into()]).unwrap().as_deref(), Some(home));
    for bad in ["relative", "/a/../b", "a\0b"] {
        assert!(parse_session(&["--kora-home".into(), bad.into()]).is_err(), "{bad}");
    }
    assert!(parse_session(&["--other".into(), home.into()]).is_err());
}

#[test]
fn result_lines_are_recognised() {
    assert_eq!(result_line(r#"KORA-ADMIN-RESULT {"a":1}"#), Some(serde_json::json!({"a": 1})));
    assert_eq!(result_line("KORA-ADMIN-RESULT not json"), None);
    assert_eq!(result_line("hello"), None);
}

#[test]
fn every_failure_to_ask_offers_a_way_on() {
    let ctx = Context { what: "install the server".into(), command: vec!["kora".into(), "install".into()], can_skip_admin: true };
    for (e, code) in
        [(OpenError::Declined, codes::ADMIN_DECLINED), (OpenError::TimedOut, codes::ADMIN_TIMED_OUT), (OpenError::Busy, codes::ADMIN_BUSY), (OpenError::Failed("x".into()), codes::ADMIN_LOST)]
    {
        let p = open_problem(&e, &ctx);
        assert_eq!(p.code, code);
        assert_eq!(p.actions[0], Action::RequestAdmin, "{code}: ask again first");
        assert!(p.actions.contains(&Action::ContinueWithoutAdmin), "{code}");
        assert!(p.actions.iter().any(|a| matches!(a, Action::ShowCommand { elevated: true, .. })), "{code}");
    }
    let no_skip = Context { can_skip_admin: false, ..ctx.clone() };
    assert!(!open_problem(&OpenError::Declined, &no_skip).actions.contains(&Action::ContinueWithoutAdmin));
    let r = remote_problem(&ctx);
    assert_eq!(r.code, codes::ADMIN_REMOTE);
    assert!(r.message.contains("another device") && r.actions[0] == Action::RequestAdmin);
    assert_eq!(shell_execute_error(1223), OpenError::Declined);
    assert!(matches!(shell_execute_error(2), OpenError::Failed(_)));
}

// ---- the protocol over a real byte stream (Unix socket pair) --------------------------------

#[cfg(unix)]
mod stream {
    use super::*;
    use std::sync::{Arc, Mutex};

    type Call = (Vec<String>, Vec<(String, String)>);

    #[derive(Default)]
    struct Fake {
        calls: Mutex<Vec<Call>>,
        code: i32,
    }

    impl Executor for Fake {
        fn run(&self, argv: &[String], env: &[(String, String)], on_line: &mut dyn FnMut(&str, &str)) -> io::Result<i32> {
            self.calls.lock().unwrap().push((argv.to_vec(), env.to_vec()));
            on_line("stdout", "working");
            if let Some((_, v)) = env.iter().find(|(k, _)| k == "CFX_LICENSE_KEY") {
                on_line("stderr", &format!("echo {v}"));
            }
            on_line("stdout", r#"KORA-ADMIN-RESULT {"password":"s3cret-db"}"#);
            Ok(self.code)
        }
    }

    fn validator(verb: &str, args: &[String]) -> Result<(), String> {
        match (verb, args.first().map(String::as_str)) {
            ("admin-step", Some("firewall-open" | "service-install")) | ("schedule", Some("off")) => Ok(()),
            _ => Err("That command is not one the administrator helper runs.".into()),
        }
    }

    fn rig(fake: Arc<Fake>) -> (Session, std::thread::JoinHandle<()>) {
        use std::os::unix::net::UnixStream;
        let (a, b) = UnixStream::pair().unwrap();
        let worker = std::thread::spawn(move || {
            let mut r = io::BufReader::new(b.try_clone().unwrap());
            let mut w = b;
            let base = vec![("USERNAME".to_string(), "jo".to_string())];
            serve(&mut r, &mut w, &validator, fake.as_ref(), &base, &|_| {}).unwrap();
        });
        let s = Session::begin(Box::new(io::BufReader::new(a.try_clone().unwrap())), Box::new(a), Some("/home/jo/.kora")).unwrap();
        (s, worker)
    }

    #[test]
    fn one_session_runs_several_steps_and_ends_with_bye() {
        let fake = Arc::new(Fake::default());
        let (mut s, worker) = rig(fake.clone());
        assert_eq!(s.worker_pid, std::process::id());
        let mut lines = Vec::new();
        let out = s.run("admin-step", &["firewall-open".into(), "--dir".into(), "/srv/a b".into()], &[], &mut |l| lines.push(l.to_string())).unwrap();
        assert!(out.ok);
        assert_eq!(out.data, Some(serde_json::json!({"password": "s3cret-db"})), "the result line becomes data");
        assert_eq!(lines, ["working"], "and is never shown");
        let out = s.run("admin-step", &["service-install".into()], &[("CFX_LICENSE_KEY".into(), "cfxk_TOPSECRET".into())], &mut |l| lines.push(l.to_string())).unwrap();
        assert!(out.ok);
        assert!(lines.iter().all(|l| !l.contains("TOPSECRET")), "{lines:?}");
        s.finish();
        worker.join().unwrap();
        let calls = fake.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, ["--no-elevate", "admin-step", "firewall-open", "--dir", "/srv/a b"]);
        assert!(calls[0].1.contains(&("USERNAME".into(), "jo".into())) && calls[0].1.contains(&("KORA_HOME".into(), "/home/jo/.kora".into())));
        assert!(calls[1].0.iter().all(|a| !a.contains("TOPSECRET")), "secrets never on argv");
    }

    #[test]
    fn anything_off_the_allow_list_is_refused_and_the_session_goes_on() {
        let fake = Arc::new(Fake::default());
        let (mut s, worker) = rig(fake.clone());
        for (verb, args) in [
            ("install", vec!["--name", "x"]),
            ("delete", vec!["x", "--yes"]),
            ("service-host", vec!["x"]),
            ("cmd.exe", vec!["/c", "whoami"]),
            ("admin-step", vec!["run", "cmd.exe"]),
            ("admin-worker", vec![]),
            ("schedule", vec!["set", "x"]),
        ] {
            let a: Vec<String> = args.iter().map(|x| x.to_string()).collect();
            assert!(matches!(s.run(verb, &a, &[], &mut |_| {}), Err(HelperError::Rejected(_))), "{verb} {args:?}");
        }
        let bad_env = s.run("schedule", &["off".into(), "x".into()], &[("PATH".into(), "C:\\evil".into())], &mut |_| {});
        assert!(matches!(bad_env, Err(HelperError::Rejected(_))));
        assert!(s.run("schedule", &["off".into(), "x".into()], &[], &mut |_| {}).unwrap().ok, "still serving after refusals");
        s.finish();
        worker.join().unwrap();
        assert_eq!(fake.calls.lock().unwrap().len(), 1, "only the allowed one ran");
    }

    #[test]
    fn a_failing_step_reports_its_code() {
        let fake = Arc::new(Fake { code: 4, ..Default::default() });
        let (mut s, worker) = rig(fake);
        let out = s.run("schedule", &["off".into(), "x".into()], &[], &mut |_| {}).unwrap();
        assert_eq!((out.ok, out.code), (false, 4));
        drop(s); // the panel goes away without bye: the worker ends too
        worker.join().unwrap();
    }

    #[test]
    fn a_dead_worker_is_reported_not_waited_for() {
        use std::os::unix::net::UnixStream;
        let (a, b) = UnixStream::pair().unwrap();
        let w = std::thread::spawn(move || {
            let mut b = b;
            let _ = send(&mut b, &Message::Hello { version: env!("CARGO_PKG_VERSION").into(), pid: 1 });
            let mut r = io::BufReader::new(b.try_clone().unwrap());
            let mut line = String::new();
            let _ = r.read_line(&mut line); // session
            let _ = send(&mut b, &Message::Result { ok: true, code: 0, message: None, version: None, data: None });
            line.clear();
            let _ = r.read_line(&mut line); // the request; then "crash"
        });
        let mut s = Session::begin(Box::new(io::BufReader::new(a.try_clone().unwrap())), Box::new(a), None).unwrap();
        let e = s.run("schedule", &["off".into(), "x".into()], &[], &mut |_| {}).unwrap_err();
        assert!(matches!(e, HelperError::Protocol(_)), "{e:?}");
        w.join().unwrap();
        assert_eq!(lost_problem(&e).code, codes::ADMIN_LOST);
    }

    #[test]
    fn a_worker_of_another_version_or_without_hello_is_not_used() {
        use std::os::unix::net::UnixStream;
        for first in [Message::Hello { version: "0.0.1".into(), pid: 1 }, Message::Progress { stream: "stdout".into(), line: "hi".into() }] {
            let (a, mut b) = UnixStream::pair().unwrap();
            send(&mut b, &first);
            let r = Session::begin(Box::new(io::BufReader::new(a.try_clone().unwrap())), Box::new(a), None);
            assert!(matches!(r, Err(HelperError::Protocol(_))));
        }
    }

    #[test]
    fn garbage_and_oversized_lines_are_refused() {
        use std::os::unix::net::UnixStream;
        let (a, b) = UnixStream::pair().unwrap();
        let fake = Arc::new(Fake::default());
        let f2 = fake.clone();
        let worker = std::thread::spawn(move || {
            let mut r = io::BufReader::new(b.try_clone().unwrap());
            let mut w = b;
            serve(&mut r, &mut w, &validator, f2.as_ref(), &[], &|_| {}).unwrap();
        });
        let mut w = a.try_clone().unwrap();
        let mut r = io::BufReader::new(a);
        let mut line = String::new();
        r.read_line(&mut line).unwrap(); // hello
        w.write_all(b"not json\n").unwrap();
        line.clear();
        r.read_line(&mut line).unwrap();
        assert!(line.contains(r#""type":"error""#), "{line}");
        w.write_all(&vec![b'a'; MAX_REQUEST_BYTES + 10]).unwrap();
        w.write_all(b"\n").unwrap();
        line.clear();
        r.read_line(&mut line).unwrap();
        assert!(line.contains(r#""type":"error""#), "{line}");
        worker.join().unwrap();
        assert!(fake.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn a_child_that_runs_too_long_or_for_nobody_is_stopped() {
        let mut ex = ChildExecutor::new("/bin/sh".into());
        ex.limit = Duration::from_millis(300);
        let t = std::time::Instant::now();
        let e = ex.run(&["-c".into(), "echo started; sleep 30".into()], &[], &mut |_, _| {}).unwrap_err();
        assert!(e.to_string().contains("longer than") && t.elapsed() < Duration::from_secs(10), "{e}");
        let ex = Arc::new(ChildExecutor::new("/bin/sh".into()));
        let e2 = ex.clone();
        let canceller = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            e2.cancel();
        });
        let t = std::time::Instant::now();
        let e = ex.run(&["-c".into(), "sleep 30".into()], &[], &mut |_, _| {}).unwrap_err();
        canceller.join().unwrap();
        assert!(e.to_string().contains("went away") && t.elapsed() < Duration::from_secs(10), "{e}");
        assert!(ex.run(&["-c".into(), "true".into()], &[], &mut |_, _| {}).is_err(), "a cancelled executor starts nothing more");
    }

    #[test]
    fn a_grandchild_holding_the_output_open_does_not_keep_the_step_waiting() {
        let ex = ChildExecutor::new("/bin/sh".into());
        let t = std::time::Instant::now();
        let code = ex.run(&["-c".into(), "(sleep 20 &) ; echo done; exit 0".into()], &[], &mut |_, _| {}).unwrap();
        assert_eq!(code, 0);
        assert!(t.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn the_child_gets_only_the_environment_it_is_given() {
        std::env::set_var("KORA_MARIADB_HOME_TEST_LEAK", "/tmp/evil");
        let ex = ChildExecutor::new("/bin/sh".into());
        let mut out = Vec::new();
        let code = ex.run(&["-c".into(), "echo \"leak=$KORA_MARIADB_HOME_TEST_LEAK home=$HOME given=$GIVEN\"".into()], &[("GIVEN".into(), "yes".into())], &mut |_, l| out.push(l.to_string())).unwrap();
        assert_eq!(code, 0);
        assert_eq!(out, ["leak= home= given=yes"], "nothing of the worker's own environment reaches the elevated child");
    }

    #[test]
    fn the_child_executor_runs_without_a_shell_and_keeps_result_lines_whole() {
        let ex = ChildExecutor::new("/bin/sh".into());
        let mut out = Vec::new();
        let long = "x".repeat(10_000);
        let script = format!("echo \"$KORA_T\"; echo 'KORA-ADMIN-RESULT {{\"v\":\"{long}\"}}'; echo err >&2; exit 3");
        let code = ex.run(&["-c".into(), script], &[("KORA_T".into(), "a;b $(id)".into())], &mut |s, l| out.push((s.to_string(), l.to_string()))).unwrap();
        assert_eq!(code, 3);
        assert!(out.contains(&("stdout".into(), "a;b $(id)".into())), "{out:?}");
        assert!(out.contains(&("stderr".into(), "err".into())));
        let res = out.iter().find_map(|(_, l)| result_line(l)).unwrap();
        assert_eq!(res["v"].as_str().unwrap().len(), 10_000);
    }
}

#[test]
fn elevated_children_get_system_folders_from_windows_and_the_panel_users_profile() {
    let sys = SystemDirs { system_root: r"C:\Windows".into(), program_files: r"C:\Program Files".into(), program_files_x86: r"C:\Program Files (x86)".into(), program_data: r"C:\ProgramData".into() };
    let u = crate::helper::winpipe::PanelUser { sid: "S-1-5-21-1-2-3-1001".into(), domain: "PC".into(), name: "jo".into(), admin_member: true };
    let e = elevated_child_env(&sys, &u, Some(r"C:\Users\jo"), Some(r"C:\Users\jo\AppData\Local\Kora"));
    let get = |k: &str| e.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    assert_eq!(get("PATH"), Some(r"C:\Windows\System32;C:\Windows;C:\Windows\System32\Wbem;C:\Windows\System32\WindowsPowerShell\v1.0"), "system folders only");
    assert_eq!(get("ProgramFiles"), Some(r"C:\Program Files"));
    assert_eq!(get("ProgramData"), Some(r"C:\ProgramData"));
    assert_eq!(get("TEMP"), Some(r"C:\Windows\Temp"));
    assert_eq!(get("USERNAME"), Some("jo"));
    assert_eq!(get("LOCALAPPDATA"), Some(r"C:\Users\jo\AppData\Local"), "the panel user's profile, not the administrator's who answered");
    assert_eq!(get("APPDATA"), Some(r"C:\Users\jo\AppData\Roaming"));
    assert_eq!(get("KORA_HOME"), Some(r"C:\Users\jo\AppData\Local\Kora"));
    for k in OVERRIDE_VARS {
        assert!(get(k).is_none(), "{k}");
    }
    assert!(e.iter().all(|(k, _)| !crate::helper::SECRET_ENV.contains(&k.as_str())), "secrets come per request only");
    let e = elevated_child_env(&sys, &u, None, None);
    assert!(!e.iter().any(|(k, _)| k == "USERPROFILE" || k == "KORA_HOME"));
}

#[test]
fn the_worker_forgets_override_variables_at_start() {
    let sys = SystemDirs { system_root: r"C:\Windows".into(), program_files: r"C:\Program Files".into(), program_files_x86: String::new(), program_data: r"C:\ProgramData".into() };
    let (remove, set) = worker_env_changes(&sys);
    for k in ["KORA_MARIADB_HOME", "KORA_SELF_UPDATE_API", "KORA_TEST_ARTIFACT_MIRROR", "KORA_CATALOG", "KORA_SERVICE_USER", "KORA_DB_HOST"] {
        assert!(remove.contains(&k), "{k}");
    }
    assert!(set.contains(&("ProgramFiles", r"C:\Program Files".to_string())) && set.contains(&("TEMP", r"C:\Windows\Temp".to_string())));
    let (_, none) = worker_env_changes(&SystemDirs::default());
    assert!(none.is_empty(), "nothing invented when Windows did not answer");
}

#[test]
fn only_one_session_at_a_time_and_the_slot_comes_back() {
    static SLOT: SessionSlot = SessionSlot::new();
    let first = SLOT.try_take().expect("free at first");
    assert!(SLOT.try_take().is_none(), "a second session waits");
    drop(first);
    assert!(SLOT.try_take().is_some(), "free again once the first ended");
    let p = open_problem(&OpenError::InUse, &Context::default());
    assert_eq!(p.code, codes::ADMIN_BUSY);
    assert!(p.message.contains("already doing administrator work"), "{}", p.message);
}
