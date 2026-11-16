//! Protocol round trips over a real Unix socket in a temp folder, with a fake executor (no `kora` child).

use super::client::{Helper, HelperError, SocketClient};
use super::server::{Executor, Server};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
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
        on_line("stdout", "step one");
        // a careless child that echoes its secret
        if let Some((_, v)) = env.first() {
            on_line("stderr", &format!("using {v}"));
        }
        Ok(self.code)
    }
}

struct Rig {
    client: SocketClient,
    fake: Arc<Fake>,
    stop: Arc<AtomicBool>,
    _dir: tempfile::TempDir,
}
impl Drop for Rig {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// `peer` = the uid the server believes is connecting; `allowed` = the installing user.
fn rig(peer: u32, allowed: u32, code: i32) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run").join("helper.sock");
    let fake = Arc::new(Fake { code, ..Default::default() });
    // the validator only accepts `app install ...` and `schedule off x`, like a narrow CLI would
    let validator = |verb: &str, args: &[String]| -> Result<(), String> {
        match (verb, args.first().map(String::as_str)) {
            ("app", Some("install")) | ("schedule", Some("off")) | ("install", _) | ("uninstall", _) | ("self-update", _) => Ok(()),
            _ => Err("That command is not one the helper runs.".to_string()),
        }
    };
    let server = Server::bind(&path, allowed, None, Arc::new(validator), fake.clone()).unwrap().with_peer_fn(move |_| Ok(peer));
    let stop = Arc::new(AtomicBool::new(false));
    let (s2, st2) = (Arc::new(server), stop.clone());
    std::thread::spawn(move || s2.serve(&st2));
    Rig { client: SocketClient { path }, fake, stop, _dir: dir }
}

fn run(r: &Rig, verb: &str, args: &[&str], env: &[(&str, &str)]) -> (Result<super::client::Outcome, HelperError>, Vec<String>) {
    let mut lines = Vec::new();
    let a: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let e: Vec<(String, String)> = env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let res = r.client.run(verb, &a, &e, &mut |l| lines.push(l.to_string()));
    (res, lines)
}

#[test]
fn a_request_streams_lines_then_a_result() {
    let r = rig(1000, 1000, 0);
    assert!(r.client.available());
    assert_eq!(r.client.version().as_deref(), Some(env!("CARGO_PKG_VERSION")));
    let (res, lines) = run(&r, "app", &["install", "--yes"], &[]);
    assert_eq!(res.unwrap(), super::client::Outcome { ok: true, code: 0 });
    assert_eq!(lines, ["step one"]);
    let calls = r.fake.calls.lock().unwrap();
    assert_eq!(calls[0].0, ["--no-elevate", "app", "install", "--yes"]);
}

#[test]
fn a_failing_child_is_reported_with_its_exit_code() {
    let r = rig(1000, 1000, 3);
    let (res, _) = run(&r, "uninstall", &["--yes"], &[]);
    assert_eq!(res.unwrap(), super::client::Outcome { ok: false, code: 3 });
}

#[test]
fn root_is_served_too() {
    let r = rig(0, 1000, 0);
    assert!(run(&r, "self-update", &["--yes"], &[]).0.unwrap().ok);
}

#[test]
fn another_user_is_turned_away_and_nothing_runs() {
    let r = rig(1234, 1000, 0);
    assert!(!r.client.available());
    let (res, lines) = run(&r, "app", &["install"], &[]);
    assert!(matches!(res, Err(HelperError::Rejected(m)) if m.contains("installed it")));
    assert!(lines.is_empty() && r.fake.calls.lock().unwrap().is_empty());
}

#[test]
fn the_allow_list_rejects_everything_else() {
    let r = rig(1000, 1000, 0);
    for (verb, args) in [
        ("delete", vec!["srv", "--yes"]),
        ("service-host", vec!["srv"]),
        ("rm", vec!["-rf", "/"]),
        ("sh", vec!["-c", "id"]),
        ("", vec![]),
        ("install", vec!["--code", "KORA-7F3K-92QX"]),
        ("install", vec!["--license-key", "cfxk_x"]),
        ("app", vec!["status"]),
        ("schedule", vec!["set", "x"]),
    ] {
        let (res, _) = run(&r, verb, &args, &[]);
        assert!(matches!(res, Err(HelperError::Rejected(_))), "{verb} {args:?}");
    }
    let (res, _) = run(&r, "install", &["--name", "a"], &[("PATH", "/tmp")]);
    assert!(matches!(res, Err(HelperError::Rejected(_))));
    assert!(r.fake.calls.lock().unwrap().is_empty(), "nothing was executed");
}

#[test]
fn shell_metacharacters_are_plain_arguments() {
    let r = rig(1000, 1000, 0);
    let (res, _) = run(&r, "uninstall", &["--yes", "x; touch /tmp/pwned && $(id) `id` | cat"], &[]);
    assert!(res.unwrap().ok);
    let calls = r.fake.calls.lock().unwrap();
    assert_eq!(calls[0].0.len(), 4, "one argument, not split");
}

#[test]
fn secrets_go_in_the_environment_and_are_masked_in_the_output() {
    let r = rig(1000, 1000, 0);
    let (res, lines) = run(&r, "install", &["--name", "a"], &[("CFX_LICENSE_KEY", "cfxk_TOPSECRET123")]);
    assert!(res.unwrap().ok);
    let calls = r.fake.calls.lock().unwrap();
    assert!(calls[0].0.iter().all(|a| !a.contains("TOPSECRET")), "never on argv");
    assert_eq!(calls[0].1, [("CFX_LICENSE_KEY".to_string(), "cfxk_TOPSECRET123".to_string())]);
    assert_eq!(lines, ["step one", "using ***"]);
}

#[test]
fn garbage_and_oversized_requests_are_refused() {
    use std::io::{Read, Write};
    let r = rig(1000, 1000, 0);
    for payload in [b"not json\n".to_vec(), vec![b'a'; super::MAX_REQUEST_BYTES + 100]] {
        let mut s = std::os::unix::net::UnixStream::connect(&r.client.path).unwrap();
        let _ = s.write_all(&payload);
        let _ = s.write_all(b"\n");
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        assert!(out.contains(r#""type":"error""#), "{out}");
    }
    assert!(r.fake.calls.lock().unwrap().is_empty());
}

#[test]
fn the_socket_is_group_readable_not_world_accessible() {
    use std::os::unix::fs::PermissionsExt;
    let r = rig(1000, 1000, 0);
    assert_eq!(std::fs::metadata(&r.client.path).unwrap().permissions().mode() & 0o777, 0o660);
}

#[test]
fn a_missing_helper_is_unavailable() {
    let c = SocketClient { path: "/nonexistent/kora-helper.sock".into() };
    assert!(!c.available());
    assert!(matches!(c.run("app", &[], &[], &mut |_| {}), Err(HelperError::Unavailable(_))));
}

#[test]
fn only_root_owned_unwritable_binaries_are_trusted() {
    use super::server::writable_by_others;
    assert!(!writable_by_others(0, 0o755));
    assert!(writable_by_others(0, 0o775) && writable_by_others(0, 0o757));
    assert!(writable_by_others(1000, 0o755), "owned by a normal user");
}

#[test]
fn child_output_is_split_into_lines() {
    assert_eq!(super::server::split_lines(b"a\rb\r\nc  \n\n"), ["a", "b", "c"]);
}

#[test]
fn the_real_executor_runs_a_child_without_a_shell() {
    use super::server::{base_env, ProcessExecutor};
    let cfg = super::HelperConfig { uid: 1000, gid: 1000, user: "sam".into(), home: "/home/sam".into(), kora_home: Some("/k".into()), version: "t".into() };
    let ex = ProcessExecutor { exe: "/usr/bin/env".into(), base_env: base_env(&cfg) };
    let mut out = Vec::new();
    let code = ex.run(&[], &[("CFX_LICENSE_KEY".into(), "k".into())], &mut |s, l| out.push(format!("{s}:{l}"))).unwrap();
    assert_eq!(code, 0);
    assert!(out.contains(&"stdout:SUDO_USER=sam".to_string()) && out.contains(&"stdout:KORA_HOME=/k".to_string()) && out.contains(&"stdout:CFX_LICENSE_KEY=k".to_string()));
    assert!(!out.iter().any(|l| l.contains("TERM=") || l.contains("SSH_")), "environment is rebuilt, not inherited");
}
