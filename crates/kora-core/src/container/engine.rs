//! The container engine behind one small interface: Podman or Docker, through their command line.
//!
//! Everything is an argument list run through [`EngineRunner`] (the existing
//! [`CommandRunner`](crate::service::CommandRunner) plus a line-streaming variant for image pulls). No
//! shell is ever involved and no value is interpolated into a string that something parses again: a
//! server named `a b; rm -rf ~` is one argument. Tests use a fake runner and assert the exact lines.
//!
//! Output formats used (both engines accept Go templates in `--format`):
//! * `inspect --format '{{.State.Status}}'` (`running`, `exited`, `created`, `paused`, `restarting`, `dead`)
//! * `info --format json` (Podman: `host.security.rootless`, `version.Version`; Docker: `ServerVersion`,
//!   `SecurityOptions` containing `name=rootless`, `CgroupVersion`).

use std::io;

use serde::{Deserialize, Serialize};

use crate::service::{CommandOutput, CommandRunner, SystemRunner};

/// Which engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum EngineKind {
    Podman,
    Docker,
}

impl EngineKind {
    /// The program name looked up on `PATH`.
    pub fn program(self) -> &'static str {
        match self {
            EngineKind::Podman => "podman",
            EngineKind::Docker => "docker",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            EngineKind::Podman => "Podman",
            EngineKind::Docker => "Docker",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "podman" => Some(EngineKind::Podman),
            "docker" => Some(EngineKind::Docker),
            _ => None,
        }
    }
}

/// A command runner that can also hand output over line by line while a long command runs.
pub trait EngineRunner: CommandRunner {
    /// Like [`CommandRunner::run`], calling `on_line` for each line of output as it arrives (the default
    /// runs the command and then replays the output).
    fn run_lines(&self, program: &str, args: &[String], on_line: &mut dyn FnMut(&str)) -> io::Result<CommandOutput> {
        let o = self.run(program, args)?;
        for l in o.stdout.lines().chain(o.stderr.lines()) {
            on_line(l);
        }
        Ok(o)
    }
}

impl EngineRunner for SystemRunner {
    fn run_lines(&self, program: &str, args: &[String], on_line: &mut dyn FnMut(&str)) -> io::Result<CommandOutput> {
        use std::io::Read;
        use std::process::Stdio;
        use std::sync::mpsc;
        let mut child = crate::sysexe::command(program).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
        let (tx, rx) = mpsc::channel::<(bool, String)>();
        let pump = |mut r: Box<dyn Read + Send>, is_err: bool, tx: mpsc::Sender<(bool, String)>| {
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                let mut line = Vec::new();
                loop {
                    let n = match r.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    for &b in &buf[..n] {
                        // Progress bars redraw with a carriage return: each redraw is a line.
                        if b == b'\n' || b == b'\r' {
                            if !line.is_empty() {
                                let _ = tx.send((is_err, String::from_utf8_lossy(&line).into_owned()));
                                line.clear();
                            }
                        } else if line.len() < 4000 {
                            line.push(b);
                        }
                    }
                }
                if !line.is_empty() {
                    let _ = tx.send((is_err, String::from_utf8_lossy(&line).into_owned()));
                }
            })
        };
        let out = child.stdout.take().map(|o| pump(Box::new(o), false, tx.clone()));
        let err = child.stderr.take().map(|e| pump(Box::new(e), true, tx.clone()));
        drop(tx);
        let (mut so, mut se) = (String::new(), String::new());
        for (is_err, l) in rx {
            on_line(&l);
            let dst = if is_err { &mut se } else { &mut so };
            if dst.len() < 64 * 1024 {
                dst.push_str(&l);
                dst.push('\n');
            }
        }
        for h in out.into_iter().chain(err) {
            let _ = h.join();
        }
        let st = child.wait()?;
        Ok(CommandOutput { code: st.code().unwrap_or(-1), stdout: so, stderr: se })
    }
}

/// An engine on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Engine {
    pub kind: EngineKind,
    /// Rootless: the engine runs as the person, not as root (Podman default, Docker only in rootless mode).
    pub rootless: bool,
}

impl Engine {
    pub fn new(kind: EngineKind, rootless: bool) -> Self {
        Engine { kind, rootless }
    }

    pub fn program(&self) -> &'static str {
        self.kind.program()
    }

    /// `podman, rootless` / `docker` / `docker, rootless`: what `kora status` and the panel show.
    pub fn label(&self) -> String {
        format!("{}{}", self.kind.program(), if self.rootless { ", rootless" } else { "" })
    }

    /// Does this engine give a published port its own firewall rules that `ufw` / `firewalld` do not see?
    /// (Docker writes iptables/nftables rules itself; rootless Podman's port forwarder is an ordinary program
    /// listening on the port, which the host firewall does see; root Podman behaves like Docker.)
    pub fn bypasses_host_firewall(&self) -> bool {
        // Rootless engines forward ports from an ordinary program (rootlessport / rootlesskit) that listens
        // on the host like any other; rooted ones (Docker, Podman as root) write netfilter rules.
        !self.rootless
    }

    /// Run `args` (without the program name).
    pub fn run(&self, runner: &dyn CommandRunner, args: &[String]) -> io::Result<CommandOutput> {
        runner.run(self.program(), args)
    }

    /// Run and require success; the error carries the engine's own words.
    pub fn run_ok(&self, runner: &dyn CommandRunner, args: &[String]) -> Result<CommandOutput, EngineError> {
        let o = self.run(runner, args).map_err(|e| EngineError::Spawn(self.program().into(), e.to_string()))?;
        if o.success() {
            Ok(o)
        } else {
            Err(EngineError::Failed { command: command_text(self.program(), args), code: o.code, stderr: tidy(&o.stderr) })
        }
    }
}

/// What went wrong running the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineError {
    /// The program could not be started (not installed, not allowed).
    Spawn(String, String),
    Failed {
        command: String,
        code: i32,
        stderr: String,
    },
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineError::Spawn(p, e) => write!(f, "could not run {p}: {e}"),
            EngineError::Failed { command, code, stderr } => write!(f, "`{command}` failed (exit {code}): {stderr}"),
        }
    }
}

impl std::error::Error for EngineError {}

impl EngineError {
    /// The engine's own error text (empty for a spawn failure).
    pub fn stderr(&self) -> &str {
        match self {
            EngineError::Failed { stderr, .. } => stderr,
            EngineError::Spawn(_, e) => e,
        }
    }
}

/// A command as text for messages: arguments with spaces quoted. Never fed to a shell.
pub fn command_text(program: &str, args: &[String]) -> String {
    std::iter::once(program)
        .chain(args.iter().map(String::as_str))
        .map(|a| if a.is_empty() || a.contains(char::is_whitespace) { format!("'{a}'") } else { a.to_string() })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The first useful lines of an engine's error output, short.
pub fn tidy(s: &str) -> String {
    s.lines().map(str::trim).filter(|l| !l.is_empty()).take(6).collect::<Vec<_>>().join(" / ").chars().take(600).collect()
}

fn s(v: &str) -> String {
    v.to_string()
}

// ---------------------------------------------------------------------------------------------
// Pure command builders (arguments without the program name)
// ---------------------------------------------------------------------------------------------

pub fn info_args(kind: EngineKind) -> Vec<String> {
    match kind {
        EngineKind::Podman => vec![s("info"), s("--format"), s("json")],
        EngineKind::Docker => vec![s("info"), s("--format"), s("{{json .}}")],
    }
}

pub fn pull_args(image: &str) -> Vec<String> {
    vec![s("pull"), s(image)]
}

pub fn image_exists_args(image: &str) -> Vec<String> {
    vec![s("image"), s("exists"), s(image)]
}

/// `docker` has no `image exists`; `image inspect` works on both.
pub fn image_inspect_args(image: &str) -> Vec<String> {
    vec![s("image"), s("inspect"), s("--format"), s("{{.Id}}"), s(image)]
}

pub fn inspect_state_args(name: &str) -> Vec<String> {
    vec![s("inspect"), s("--type"), s("container"), s("--format"), s("{{.State.Status}}"), s(name)]
}

pub fn inspect_args(name: &str) -> Vec<String> {
    vec![s("inspect"), s(name)]
}

pub fn start_args(name: &str) -> Vec<String> {
    vec![s("start"), s(name)]
}

/// `stop -t <seconds>`: SIGTERM, then SIGKILL after the timeout (the same 30 seconds as the systemd unit's
/// `TimeoutStopSec`).
pub fn stop_args(name: &str, timeout_secs: u32) -> Vec<String> {
    vec![s("stop"), s("-t"), timeout_secs.to_string(), s(name)]
}

pub fn restart_args(name: &str, timeout_secs: u32) -> Vec<String> {
    vec![s("restart"), s("-t"), timeout_secs.to_string(), s(name)]
}

/// Remove a container whether or not it runs; its anonymous volumes with it.
pub fn rm_args(name: &str) -> Vec<String> {
    vec![s("rm"), s("-f"), s("-v"), s(name)]
}

pub fn network_create_args(name: &str, slug: &str) -> Vec<String> {
    vec![s("network"), s("create"), s("--label"), format!("{}={slug}", super::LABEL_SERVER), s(name)]
}

pub fn network_rm_args(name: &str) -> Vec<String> {
    vec![s("network"), s("rm"), s(name)]
}

pub fn network_inspect_args(name: &str) -> Vec<String> {
    vec![s("network"), s("inspect"), s("--format"), s("{{.Name}}"), s(name)]
}

pub fn volume_create_args(name: &str, slug: &str) -> Vec<String> {
    vec![s("volume"), s("create"), s("--label"), format!("{}={slug}", super::LABEL_SERVER), s(name)]
}

pub fn volume_rm_args(name: &str) -> Vec<String> {
    vec![s("volume"), s("rm"), s(name)]
}

pub fn volume_inspect_args(name: &str) -> Vec<String> {
    vec![s("volume"), s("inspect"), s("--format"), s("{{.Name}}"), s(name)]
}

/// `logs -f --since <rfc3339|seconds> <name>`: what the console follower reads.
pub fn logs_follow_args(name: &str, since: &str) -> Vec<String> {
    vec![s("logs"), s("--follow"), s("--since"), s(since), s(name)]
}

pub fn logs_tail_args(name: &str, lines: u32) -> Vec<String> {
    vec![s("logs"), s("--tail"), lines.to_string(), s(name)]
}

/// `exec -i` into a running container, forwarding the named environment variables from the engine client's own
/// environment (`-e NAME`, no value on the command line).
pub fn exec_args(container: &str, env_names: &[&str], program: &str, args: &[String]) -> Vec<String> {
    let mut v = vec![s("exec"), s("-i")];
    for n in env_names {
        v.push(s("-e"));
        v.push(s(n));
    }
    v.push(s(container));
    v.push(s(program));
    v.extend(args.iter().cloned());
    v
}

/// All containers carrying Kora's label for this server, one name per line.
pub fn list_server_objects_args(kind: &str, slug: &str) -> Vec<String> {
    // `ps` prints `.Names`; `network ls` / `volume ls` print `.Name`.
    let (head, field): (Vec<String>, &str) = if kind == "container" { (vec![s("ps"), s("-a")], "{{.Names}}") } else { (vec![s(kind), s("ls")], "{{.Name}}") };
    let mut v = head;
    v.extend([s("--filter"), format!("label={}={slug}", super::LABEL_SERVER), s("--format"), s(field)]);
    v
}

// ---------------------------------------------------------------------------------------------
// Parsing what the engine prints
// ---------------------------------------------------------------------------------------------

/// The state of a container as Kora uses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerState {
    Running,
    /// Exists but is not running (exited, created, stopped, dead).
    Stopped,
    /// No such container.
    Missing,
    Other(String),
}

/// `inspect --format '{{.State.Status}}'` output (and its failure) as a state.
pub fn parse_state(o: &CommandOutput) -> ContainerState {
    if !o.success() {
        let e = format!("{}{}", o.stderr, o.stdout).to_ascii_lowercase();
        if e.contains("no such") || e.contains("not found") || e.contains("no container") || e.contains("does not exist") || e.contains("error: no such") {
            return ContainerState::Missing;
        }
        return ContainerState::Other(tidy(&o.stderr));
    }
    match o.stdout.trim() {
        // A container the restart policy is bringing back is on its way: like systemd's "activating".
        "running" | "restarting" => ContainerState::Running,
        "exited" | "created" | "stopped" | "dead" | "configured" | "initialized" => ContainerState::Stopped,
        "" => ContainerState::Other("the engine gave no state".into()),
        other => ContainerState::Other(other.to_string()),
    }
}

/// What `info` told us.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Info {
    pub version: String,
    pub rootless: bool,
    /// `1` or `2` (0 = not told).
    pub cgroup_version: u8,
    /// SELinux is enforcing on the host (labels on bind mounts matter).
    pub selinux: bool,
}

/// Read `info` JSON from either engine. Unknown fields are ignored; missing ones read as "not told".
pub fn parse_info(kind: EngineKind, json: &str) -> Option<Info> {
    let v: serde_json::Value = serde_json::from_str(json.trim()).ok()?;
    match kind {
        EngineKind::Podman => {
            let host = v.get("host")?;
            let cg = host.get("cgroupVersion").and_then(|c| c.as_str()).map(|c| c.trim_start_matches('v')).and_then(|c| c.parse::<u8>().ok()).unwrap_or(0);
            Some(Info {
                version: v.get("version").and_then(|x| x.get("Version")).and_then(|x| x.as_str()).unwrap_or("").to_string(),
                rootless: host.get("security").and_then(|x| x.get("rootless")).and_then(|x| x.as_bool()).unwrap_or(false),
                cgroup_version: cg,
                selinux: host.get("security").and_then(|x| x.get("selinuxEnabled")).and_then(|x| x.as_bool()).unwrap_or(false),
            })
        }
        EngineKind::Docker => {
            let sec = v.get("SecurityOptions").and_then(|x| x.as_array()).cloned().unwrap_or_default();
            let has = |needle: &str| sec.iter().filter_map(|x| x.as_str()).any(|x| x.contains(needle));
            Some(Info {
                version: v.get("ServerVersion").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                rootless: has("name=rootless"),
                cgroup_version: v.get("CgroupVersion").and_then(|x| x.as_str()).and_then(|c| c.parse::<u8>().ok()).unwrap_or(0),
                selinux: has("name=selinux"),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_are_argument_lists_with_nothing_interpolated() {
        // A hostile name stays one argument.
        let evil = "a b; rm -rf ~ $(id)";
        assert_eq!(start_args(evil), vec!["start", evil]);
        assert_eq!(stop_args("kora-main", 30), vec!["stop", "-t", "30", "kora-main"]);
        assert_eq!(rm_args("kora-main"), vec!["rm", "-f", "-v", "kora-main"]);
        assert_eq!(network_create_args("kora-main-net", "main"), vec!["network", "create", "--label", "kora.server=main", "kora-main-net"]);
        assert_eq!(volume_create_args("kora-main-dbdata", "main"), vec!["volume", "create", "--label", "kora.server=main", "kora-main-dbdata"]);
        assert_eq!(inspect_state_args("kora-main"), vec!["inspect", "--type", "container", "--format", "{{.State.Status}}", "kora-main"]);
        assert_eq!(logs_follow_args("kora-main", "10s"), vec!["logs", "--follow", "--since", "10s", "kora-main"]);
        assert_eq!(list_server_objects_args("container", "main"), vec!["ps", "-a", "--filter", "label=kora.server=main", "--format", "{{.Names}}"]);
        assert_eq!(list_server_objects_args("network", "main"), vec!["network", "ls", "--filter", "label=kora.server=main", "--format", "{{.Name}}"]);
    }

    #[test]
    fn exec_forwards_the_environment_by_name_only() {
        let a = exec_args("kora-main-db", &["MYSQL_PWD"], "mariadb-dump", &["-u".into(), "kora_x".into(), "db".into()]);
        assert_eq!(a, vec!["exec", "-i", "-e", "MYSQL_PWD", "kora-main-db", "mariadb-dump", "-u", "kora_x", "db"]);
        assert!(!a.iter().any(|x| x.contains('=')), "no NAME=value on the command line");
    }

    #[test]
    fn state_words_map_to_states() {
        let ok = |s: &str| CommandOutput { code: 0, stdout: format!("{s}\n"), stderr: String::new() };
        assert_eq!(parse_state(&ok("running")), ContainerState::Running);
        assert_eq!(parse_state(&ok("restarting")), ContainerState::Running);
        assert_eq!(parse_state(&ok("exited")), ContainerState::Stopped);
        assert_eq!(parse_state(&ok("created")), ContainerState::Stopped);
        assert_eq!(parse_state(&ok("stopped")), ContainerState::Stopped);
        assert_eq!(parse_state(&ok("paused")), ContainerState::Other("paused".into()));
        let missing = CommandOutput { code: 125, stdout: String::new(), stderr: "Error: no such container kora-main".into() };
        assert_eq!(parse_state(&missing), ContainerState::Missing);
        let docker_missing = CommandOutput { code: 1, stdout: String::new(), stderr: "Error response from daemon: No such container: x".into() };
        assert_eq!(parse_state(&docker_missing), ContainerState::Missing);
        let denied = CommandOutput { code: 1, stdout: String::new(), stderr: "permission denied while trying to connect to the Docker daemon socket".into() };
        assert!(matches!(parse_state(&denied), ContainerState::Other(_)));
    }

    #[test]
    fn info_is_read_from_both_engines() {
        let p = r#"{"host":{"cgroupVersion":"v2","security":{"rootless":true,"selinuxEnabled":false}},"version":{"Version":"4.9.3"}}"#;
        assert_eq!(parse_info(EngineKind::Podman, p), Some(Info { version: "4.9.3".into(), rootless: true, cgroup_version: 2, selinux: false }));
        let d = r#"{"ServerVersion":"27.1.1","CgroupVersion":"1","SecurityOptions":["name=apparmor","name=seccomp,profile=builtin"]}"#;
        assert_eq!(parse_info(EngineKind::Docker, d), Some(Info { version: "27.1.1".into(), rootless: false, cgroup_version: 1, selinux: false }));
        let dr = r#"{"ServerVersion":"27.1.1","CgroupVersion":"2","SecurityOptions":["name=seccomp,profile=builtin","name=rootless","name=selinux"]}"#;
        let i = parse_info(EngineKind::Docker, dr).unwrap();
        assert!(i.rootless && i.selinux && i.cgroup_version == 2);
        assert_eq!(parse_info(EngineKind::Podman, "not json"), None);
    }

    #[test]
    fn labels_and_firewall_facts() {
        assert_eq!(Engine::new(EngineKind::Podman, true).label(), "podman, rootless");
        assert_eq!(Engine::new(EngineKind::Docker, false).label(), "docker");
        assert!(Engine::new(EngineKind::Docker, false).bypasses_host_firewall());
        assert!(!Engine::new(EngineKind::Podman, true).bypasses_host_firewall());
        assert!(Engine::new(EngineKind::Podman, false).bypasses_host_firewall());
    }
}
