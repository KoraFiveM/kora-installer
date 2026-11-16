//! Starting Kora Panel for a person: is there a screen to open a browser on, how to open it, which
//! port to use, and what to print. Everything that touches the machine (environment, ports, the
//! opener program) goes through a small seam so the decisions are tested without a desktop.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::DEFAULT_PORT;

/// Ports tried, in order, when the default one is taken by something else.
pub const FALLBACK_PORTS: std::ops::RangeInclusive<u16> = 47121..=47140;

/// How long to wait for the browser opener before assuming it is the browser itself (still running).
pub const OPENER_WAIT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Linux,
    MacOs,
    Windows,
}

impl Os {
    pub fn current() -> Self {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::MacOs
        } else {
            Os::Linux
        }
    }
}

/// Reads an environment variable; empty counts as unset.
pub type EnvFn<'a> = &'a dyn Fn(&str) -> Option<String>;

pub fn real_env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

/// No screen to open a browser on: an SSH session, or Linux without a graphical display.
pub fn headless_with(os: Os, env: EnvFn<'_>) -> bool {
    if env("SSH_CONNECTION").is_some() {
        return true;
    }
    os == Os::Linux && env("DISPLAY").is_none() && env("WAYLAND_DISPLAY").is_none()
}

/// [`headless_with`] for this machine.
pub fn is_headless() -> bool {
    headless_with(Os::current(), &real_env)
}

/// A program and its arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenerCommand {
    pub program: String,
    pub args: Vec<String>,
}

/// Which program opens `url` in a browser. On Linux under `sudo` (we are root and `SUDO_USER` is a
/// real person) the browser belongs to that person's desktop, so the opener runs as them with their
/// display settings.
pub fn opener_command(os: Os, url: &str, env: EnvFn<'_>, as_root: bool) -> OpenerCommand {
    let cmd = |p: &str, a: Vec<String>| OpenerCommand {
        program: p.into(),
        args: a,
    };
    match os {
        Os::Windows => cmd(
            "rundll32",
            vec!["url.dll,FileProtocolHandler".into(), url.into()],
        ),
        Os::MacOs => cmd("open", vec![url.into()]),
        Os::Linux => match env("SUDO_USER").filter(|u| as_root && u != "root") {
            Some(user) => {
                let mut args = vec!["-u".into(), user, "--".into(), "env".into()];
                let uid = env("SUDO_UID").filter(|u| u.bytes().all(|b| b.is_ascii_digit()));
                for key in [
                    "DISPLAY",
                    "WAYLAND_DISPLAY",
                    "XDG_RUNTIME_DIR",
                    "DBUS_SESSION_BUS_ADDRESS",
                ] {
                    let value = env(key).or_else(|| match (key, &uid) {
                        ("XDG_RUNTIME_DIR", Some(u)) => Some(format!("/run/user/{u}")),
                        ("DBUS_SESSION_BUS_ADDRESS", Some(u)) => {
                            Some(format!("unix:path=/run/user/{u}/bus"))
                        }
                        _ => None,
                    });
                    if let Some(v) = value {
                        args.push(format!("{key}={v}"));
                    }
                }
                args.push("xdg-open".into());
                args.push(url.into());
                cmd("runuser", args)
            }
            None => cmd("xdg-open", vec![url.into()]),
        },
    }
}

/// What became of the opener after waiting for it.
#[derive(Debug, PartialEq, Eq)]
pub enum OpenerExit {
    /// Exited with this code (`None`: killed by a signal).
    Exited(Option<i32>),
    /// Still running after the wait: it is the browser itself, which is fine.
    StillRunning,
}

/// Did opening the browser work? A non-zero exit is a real failure (e.g. `xdg-open` found no browser).
pub fn opener_result(r: std::io::Result<OpenerExit>, program: &str) -> Result<(), String> {
    match r {
        Err(e) => Err(format!("could not run {program}: {e}")),
        Ok(OpenerExit::StillRunning) | Ok(OpenerExit::Exited(Some(0))) => Ok(()),
        Ok(OpenerExit::Exited(Some(c))) => Err(format!("{program} failed (exit code {c})")),
        Ok(OpenerExit::Exited(None)) => Err(format!("{program} was stopped")),
    }
}

/// Run `cmd` with no input or output and wait up to `wait` for it to finish.
pub fn run_and_wait(cmd: &OpenerCommand, wait: Duration) -> std::io::Result<OpenerExit> {
    let mut child = Command::new(&cmd.program)
        .args(&cmd.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(OpenerExit::Exited(status.code()));
        }
        if start.elapsed() >= wait {
            // Leave it running: it is most likely the browser window.
            return Ok(OpenerExit::StillRunning);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Open `url` in a browser on this machine and say whether it really worked.
pub fn open_browser(url: &str) -> Result<(), String> {
    let os = Os::current();
    let as_root = os == Os::Linux && current_uid() == Some(0);
    let cmd = opener_command(os, url, &real_env, as_root);
    opener_result(run_and_wait(&cmd, OPENER_WAIT), &cmd.program)
}

/// The effective user id on Linux (`None` elsewhere or when unknown).
pub fn current_uid() -> Option<u32> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("Uid:"))?;
    // Uid: real effective saved fs
    line.split_whitespace().nth(2)?.parse().ok()
}

/// The command to run on your own computer to reach a panel on a server with no screen. The server
/// address comes from `SSH_CONNECTION` when we are in an SSH session.
pub fn ssh_hint(port: u16, env: EnvFn<'_>) -> String {
    let user = env("SUDO_USER")
        .filter(|u| u != "root")
        .or_else(|| env("USER"))
        .or_else(|| env("LOGNAME"))
        .unwrap_or_else(|| "you".into());
    let host = env("SSH_CONNECTION")
        .and_then(|c| c.split_whitespace().nth(2).map(str::to_string))
        .unwrap_or_else(|| "your-server".into());
    format!("ssh -L {port}:127.0.0.1:{port} {user}@{host}")
}

/// The sign-in link of a panel on this computer.
pub fn sign_in_link(port: u16, token: &str) -> String {
    format!("http://127.0.0.1:{port}/#token={token}")
}

/// What to print once the panel is up. `opened` is the browser result (`None`: not tried).
pub fn ready_message(
    link: &str,
    port: u16,
    headless: bool,
    opened: Option<&Result<(), String>>,
    env: EnvFn<'_>,
) -> Vec<String> {
    let mut out = Vec::new();
    match opened {
        Some(Ok(())) => out.push("Kora Panel is open in your browser.".to_string()),
        Some(Err(e)) => out.push(format!(
            "Could not open a browser ({e}). Open the link below yourself."
        )),
        None => {}
    }
    out.push(String::new());
    out.push("  Sign in to Kora Panel with this link:".into());
    out.push(format!("  {link}"));
    out.push(String::new());
    if headless {
        out.push(
            "This computer has no screen. On your own computer, run this first and keep it open:"
                .into(),
        );
        out.push(format!("  {}", ssh_hint(port, env)));
        out.push("then open the link above in your browser there.".into());
    }
    out.push("The link contains your sign-in token: keep it private. Show it again with `kora panel --print-link`.".into());
    out
}

/// Port checks, so the choice of port is tested without real sockets.
pub trait PortCheck {
    /// Nothing listens on 127.0.0.1:`port` and we may listen there.
    fn is_free(&self, port: u16) -> bool;
    /// A Kora Panel answers on 127.0.0.1:`port`.
    fn is_kora_panel(&self, port: u16) -> bool;
}

pub struct SystemPorts;

impl PortCheck for SystemPorts {
    fn is_free(&self, port: u16) -> bool {
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        // Windows lets two programs share a port in some cases, so "someone answers" also means taken.
        TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_err()
            && TcpListener::bind(addr).is_ok()
    }
    fn is_kora_panel(&self, port: u16) -> bool {
        probe_meta(port).is_some_and(|r| looks_like_kora_meta(&r))
    }
}

fn probe_meta(port: u16) -> Option<String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(500)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    s.set_write_timeout(Some(Duration::from_secs(2))).ok()?;
    let req = format!(
        "GET /api/v1/meta HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
    );
    s.write_all(req.as_bytes()).ok()?;
    let mut buf = Vec::new();
    let _ = s.take(64 * 1024).read_to_end(&mut buf);
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Is `response` (a raw HTTP response to `GET /api/v1/meta`) from a Kora Panel?
pub fn looks_like_kora_meta(response: &str) -> bool {
    let Some((head, body)) = response.split_once("\r\n\r\n") else {
        return false;
    };
    if !head.starts_with("HTTP/1.1 200") && !head.starts_with("HTTP/1.0 200") {
        return false;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body.trim()) else {
        return false;
    };
    v["api"].as_u64() == Some(1) && v["ops"].is_array() && v["version"].is_string()
}

/// The ports to try: the wanted one, then (unless it was asked for explicitly) the default and the
/// fallback range.
pub fn candidate_ports(preferred: u16, explicit: bool) -> Vec<u16> {
    let mut v = vec![preferred];
    if !explicit {
        for p in std::iter::once(DEFAULT_PORT).chain(FALLBACK_PORTS) {
            if !v.contains(&p) {
                v.push(p);
            }
        }
    }
    v
}

/// The first free port of [`candidate_ports`], or `None` when all are taken.
pub fn pick_port(preferred: u16, explicit: bool, ports: &dyn PortCheck) -> Option<u16> {
    candidate_ports(preferred, explicit)
        .into_iter()
        .find(|p| ports.is_free(*p))
}

/// Wait until a Kora Panel answers on `port`, checking every `every`, for at most `timeout`.
pub fn wait_for_panel(
    port: u16,
    timeout: Duration,
    every: Duration,
    ports: &dyn PortCheck,
) -> bool {
    let start = Instant::now();
    loop {
        if ports.is_kora_panel(port) {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(every);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k: &str| m.get(k).cloned().filter(|v| !v.is_empty())
    }

    #[test]
    fn headless_means_ssh_or_linux_without_a_display() {
        assert!(headless_with(Os::Linux, &env_of(&[])));
        assert!(!headless_with(Os::Linux, &env_of(&[("DISPLAY", ":0")])));
        assert!(!headless_with(
            Os::Linux,
            &env_of(&[("WAYLAND_DISPLAY", "wayland-0")])
        ));
        assert!(headless_with(
            Os::Linux,
            &env_of(&[("DISPLAY", "localhost:10"), ("SSH_CONNECTION", "1 2 3 4")])
        ));
        assert!(!headless_with(Os::Windows, &env_of(&[])));
        assert!(!headless_with(Os::MacOs, &env_of(&[])));
        assert!(headless_with(
            Os::Windows,
            &env_of(&[("SSH_CONNECTION", "1 2 3 4")])
        ));
        assert!(
            headless_with(Os::Linux, &env_of(&[("DISPLAY", "")])),
            "empty is unset"
        );
    }

    #[test]
    fn the_opener_fits_each_system() {
        let url = "http://127.0.0.1:47120/#token=abc";
        let none = env_of(&[]);
        let c = opener_command(Os::Windows, url, &none, false);
        assert_eq!(c.program, "rundll32");
        assert_eq!(c.args, vec!["url.dll,FileProtocolHandler", url]);
        let c = opener_command(Os::MacOs, url, &none, false);
        assert_eq!(
            (c.program.as_str(), c.args),
            ("open", vec![url.to_string()])
        );
        let c = opener_command(Os::Linux, url, &none, false);
        assert_eq!(
            (c.program.as_str(), c.args),
            ("xdg-open", vec![url.to_string()])
        );
        // Root without sudo (signed in as root on a desktop): plain xdg-open.
        let c = opener_command(Os::Linux, url, &none, true);
        assert_eq!(c.program, "xdg-open");
        // SUDO_USER left over but we are not root: plain xdg-open.
        let c = opener_command(Os::Linux, url, &env_of(&[("SUDO_USER", "sam")]), false);
        assert_eq!(c.program, "xdg-open");
        // `sudo` from root itself: plain xdg-open.
        let c = opener_command(Os::Linux, url, &env_of(&[("SUDO_USER", "root")]), true);
        assert_eq!(c.program, "xdg-open");
    }

    #[test]
    fn under_sudo_the_browser_opens_as_the_person_with_their_desktop() {
        let url = "http://127.0.0.1:47120/";
        let env = env_of(&[
            ("SUDO_USER", "sam"),
            ("SUDO_UID", "1000"),
            ("DISPLAY", ":0"),
        ]);
        let c = opener_command(Os::Linux, url, &env, true);
        assert_eq!(c.program, "runuser");
        assert_eq!(
            c.args,
            vec![
                "-u",
                "sam",
                "--",
                "env",
                "DISPLAY=:0",
                "XDG_RUNTIME_DIR=/run/user/1000",
                "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus",
                "xdg-open",
                url
            ]
        );
        // Values that sudo kept are passed on as they are; a strange SUDO_UID is ignored.
        let env = env_of(&[
            ("SUDO_USER", "sam"),
            ("SUDO_UID", "1000; rm"),
            ("WAYLAND_DISPLAY", "wayland-0"),
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
        ]);
        let c = opener_command(Os::Linux, url, &env, true);
        assert_eq!(
            c.args[4..],
            [
                "WAYLAND_DISPLAY=wayland-0",
                "XDG_RUNTIME_DIR=/run/user/1000",
                "xdg-open",
                url
            ]
        );
    }

    #[test]
    fn a_failing_opener_is_reported() {
        assert!(opener_result(Ok(OpenerExit::Exited(Some(0))), "xdg-open").is_ok());
        assert!(
            opener_result(Ok(OpenerExit::StillRunning), "xdg-open").is_ok(),
            "the browser itself"
        );
        let e = opener_result(Ok(OpenerExit::Exited(Some(3))), "xdg-open").unwrap_err();
        assert!(e.contains("xdg-open") && e.contains('3'), "{e}");
        assert!(opener_result(Ok(OpenerExit::Exited(None)), "open").is_err());
        let e = opener_result(
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no such file",
            )),
            "xdg-open",
        )
        .unwrap_err();
        assert!(e.contains("could not run xdg-open"), "{e}");
    }

    #[cfg(unix)]
    #[test]
    fn the_exit_status_is_waited_for() {
        let sh = |script: &str| OpenerCommand {
            program: "sh".into(),
            args: vec!["-c".into(), script.into()],
        };
        let wait = Duration::from_secs(5);
        assert_eq!(
            run_and_wait(&sh("exit 0"), wait).unwrap(),
            OpenerExit::Exited(Some(0))
        );
        assert_eq!(
            run_and_wait(&sh("exit 4"), wait).unwrap(),
            OpenerExit::Exited(Some(4))
        );
        assert_eq!(
            run_and_wait(&sh("sleep 2"), Duration::from_millis(100)).unwrap(),
            OpenerExit::StillRunning
        );
        let missing = OpenerCommand {
            program: "/nonexistent/kora-opener".into(),
            args: vec![],
        };
        assert!(run_and_wait(&missing, wait).is_err());
    }

    #[test]
    fn the_tunnel_hint_names_this_server_when_it_can() {
        let env = env_of(&[
            ("SSH_CONNECTION", "203.0.113.9 51000 198.51.100.7 22"),
            ("USER", "sam"),
        ]);
        assert_eq!(
            ssh_hint(47121, &env),
            "ssh -L 47121:127.0.0.1:47121 sam@198.51.100.7"
        );
        // Under sudo the person who signed in is SUDO_USER, not root.
        let env = env_of(&[("SUDO_USER", "sam"), ("USER", "root")]);
        assert_eq!(
            ssh_hint(47120, &env),
            "ssh -L 47120:127.0.0.1:47120 sam@your-server"
        );
        assert_eq!(
            ssh_hint(1, &env_of(&[("LOGNAME", "kim")])),
            "ssh -L 1:127.0.0.1:1 kim@your-server"
        );
        assert_eq!(
            ssh_hint(1, &env_of(&[])),
            "ssh -L 1:127.0.0.1:1 you@your-server"
        );
    }

    #[test]
    fn the_ready_message_always_shows_the_link() {
        let link = sign_in_link(47120, "ab12");
        assert_eq!(link, "http://127.0.0.1:47120/#token=ab12");
        let env = env_of(&[("SSH_CONNECTION", "a b 10.0.0.5 22"), ("USER", "sam")]);
        let text = ready_message(&link, 47120, true, None, &env).join("\n");
        assert!(text.contains(&link), "{text}");
        assert!(
            text.contains("ssh -L 47120:127.0.0.1:47120 sam@10.0.0.5"),
            "{text}"
        );
        let text = ready_message(&link, 47120, false, Some(&Ok(())), &env).join("\n");
        assert!(
            text.contains("open in your browser")
                && text.contains(&link)
                && !text.contains("ssh -L"),
            "{text}"
        );
        let failed = Err("xdg-open failed (exit code 3)".to_string());
        let text = ready_message(&link, 47120, false, Some(&failed), &env).join("\n");
        assert!(
            text.contains("Could not open a browser") && text.contains(&link),
            "{text}"
        );
    }

    struct Fake {
        taken: Vec<u16>,
        ours: Vec<u16>,
    }
    impl PortCheck for Fake {
        fn is_free(&self, p: u16) -> bool {
            !self.taken.contains(&p) && !self.ours.contains(&p)
        }
        fn is_kora_panel(&self, p: u16) -> bool {
            self.ours.contains(&p)
        }
    }

    #[test]
    fn the_next_free_port_is_used_unless_one_was_asked_for() {
        let f = Fake {
            taken: vec![47120, 47121],
            ours: vec![],
        };
        assert_eq!(pick_port(47120, false, &f), Some(47122));
        assert_eq!(
            pick_port(47120, true, &f),
            None,
            "an explicit port is never changed"
        );
        assert_eq!(pick_port(47200, true, &f), Some(47200));
        // A saved port that is now taken falls back to the default, then the range.
        let f = Fake {
            taken: vec![47300],
            ours: vec![],
        };
        assert_eq!(pick_port(47300, false, &f), Some(47120));
        let all = Fake {
            taken: candidate_ports(47120, false),
            ours: vec![],
        };
        assert_eq!(pick_port(47120, false, &all), None);
        assert_eq!(candidate_ports(47120, false).len(), 21);
        assert_eq!(candidate_ports(47125, false)[..3], [47125, 47120, 47121]);
    }

    #[test]
    fn waiting_for_the_panel_stops_when_it_answers_or_times_out() {
        let f = Fake {
            taken: vec![],
            ours: vec![47120],
        };
        let t = Duration::from_millis(30);
        assert!(wait_for_panel(47120, t, Duration::from_millis(5), &f));
        assert!(!wait_for_panel(47121, t, Duration::from_millis(5), &f));
    }

    #[test]
    fn only_a_kora_meta_answer_counts_as_our_panel() {
        let ok = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\r\n{\"version\":\"0.4.3\",\"api\":1,\"platform\":\"linux\",\"ops\":[]}";
        assert!(looks_like_kora_meta(ok));
        assert!(!looks_like_kora_meta(
            "HTTP/1.1 404 Not Found\r\n\r\n{\"api\":1,\"ops\":[],\"version\":\"x\"}"
        ));
        assert!(!looks_like_kora_meta(
            "HTTP/1.1 200 OK\r\n\r\n<html>someone else</html>"
        ));
        assert!(!looks_like_kora_meta("HTTP/1.1 200 OK\r\n\r\n{\"api\":2}"));
        assert!(!looks_like_kora_meta(""));
    }
}
