//! The Windows administrator worker: how the (never elevated) panel gets administrator work done
//! with ONE normal Windows permission prompt (UAC) per install or per action.
//!
//! # Shape
//!
//! 1. The panel creates a named pipe with a random name ([`pipe_name`]) and the security
//!    descriptor [`PIPE_SDDL`], as the first and only instance (`FILE_FLAG_FIRST_PIPE_INSTANCE`,
//!    one instance, `PIPE_REJECT_REMOTE_CLIENTS`).
//! 2. It starts ITS OWN program (`std::env::current_exe()`) elevated with `ShellExecuteExW`
//!    (verb `runas`): `kora.exe admin-worker --pipe <name> --parent <panel pid>` ([`worker_args`]).
//!    That is the Windows prompt. No secret is ever on that command line.
//! 3. The worker refuses to run unless it is elevated, connects to the pipe, and checks that the
//!    process at the other end ([`check_server_peer`]) is the panel pid it was given AND runs the
//!    very same program file as itself. The panel checks that the client is the process it just
//!    started ([`check_client_peer`]). Only then does anything else travel.
//! 4. The conversation is the Linux helper's protocol ([`super::Request`] / [`super::Message`],
//!    one JSON object per line), with three additions: the worker first says [`super::Message::Hello`];
//!    one connection carries MANY requests (a whole install, or one action), ended by `bye` or by
//!    the pipe closing; and the verb `session` tells the worker the panel user's Kora folder.
//! 5. Every request goes through the same checks as the Linux helper ([`super::check_request_in`]:
//!    sizes, no secrets on argv, only the two secret environment names) against
//!    [`WORKER_VERBS`], and then the `kora` command-line definition itself (the validator in
//!    `kora-cli`). The worker runs `kora.exe --no-elevate <verb> <args>` as a child (no shell),
//!    streams its lines back (secrets masked) and the result. `admin-step` children may end with one
//!    `KORA-ADMIN-RESULT {json}` line ([`RESULT_PREFIX`]): it becomes the result's `data` and is
//!    never shown or logged.
//! 6. The worker exits when the panel says `bye`, when the pipe breaks (the panel stopped or
//!    restarted), or after [`IDLE_LIMIT`] without a request. A step that is running is finished
//!    first (stopping halfway through a MariaDB or service setup would leave more mess).
//!
//! # Threat model
//!
//! * **Another (unprivileged) local account.** Cannot open the pipe: its DACL only lets elevated
//!   administrators and SYSTEM read and write it, and the owner's implicit rights are limited to
//!   reading the descriptor (`OW` ACE), so the panel's own account cannot loosen it either. Remote
//!   (SMB) clients are rejected. The pipe name is random, created first-instance-only and limited
//!   to one instance, so it cannot be squatted or doubled.
//! * **Another process of the same user, not elevated** (malware running as the person). It cannot
//!   open the pipe (it lacks the enabled Administrators group). It could start its own `runas`
//!   prompt, or tamper with the panel's memory, but both are true of any program the person runs;
//!   Windows itself does not treat UAC as a boundary against the same user. What we guarantee is
//!   that our prompt only ever starts our own `kora.exe`, that the elevated process takes orders
//!   only from the panel process that started it, and only allow-listed ones.
//! * **Another elevated process.** Is already administrator; nothing to win.
//! * **A process racing to be "the worker".** The panel serves only the pid `ShellExecuteExW`
//!   returned; anything else that connects is dropped and the session fails without sending a byte.
//! * **A fake panel.** The worker serves only the pid on its command line, and only when that
//!   process runs the same program file as the worker; a pid that was reused by another program
//!   fails the image check.
//! * **What the person grants.** Approving the prompt lets the panel run the allow-listed actions
//!   once (for one install, or one action), like typing the shown command into an administrator
//!   terminal. On a standard account Windows asks for an administrator's password; the work still
//!   happens for the person signed in (their Kora folder, their server folders, scheduled tasks run
//!   as them), never for the administrator who typed the password. Server folders are only accepted
//!   when they hold a Kora install record and belong to the person (or, when the person is an
//!   administrator, to the Administrators group: a folder an elevated Kora made for them); see
//!   `installer::system_ops::check_dir_for` and `kora-cli`'s validator.
//! * **The worker's token is never lent to the panel.** The worker opens the pipe for
//!   identification only (`SECURITY_IDENTIFICATION`), so the panel cannot impersonate it. The administrator who types the password
//!   lets this person's server run as a Windows service, as an install from an administrator
//!   terminal would.
//!
//! # A persistent Windows helper later
//!
//! A future opt-in service can reuse all of this unchanged: it would listen on a pipe with a DACL
//! for the installing user's SID (instead of a random pipe for one session) and call [`serve`]
//! for each connection, with the same validator and [`ChildExecutor`]; the panel would use
//! [`Session`] over that pipe instead of starting a worker.

use std::collections::BTreeMap;
use std::io::{self, BufRead, Read, Write};
use std::path::Path;
use std::time::Duration;

use super::client::HelperError;
use super::server::{Executor, Validator};
use super::{check_request_in, mask_secrets, Message, Request, MAX_ARG_BYTES, MAX_REQUEST_BYTES, PING};
use crate::problem::{codes, Action, Problem};

/// The top-level `kora` subcommands the worker runs: the Linux helper's, without `install` (on
/// Windows the install runs as the person and only its administrator steps come here), plus
/// `admin-step` (one administrator step of an install or a delete), `admin-db` (start or install
/// Kora's own MariaDB outside an install) and `check --start-database` (the System check's "Start it
/// for me", any installed database service).
pub const WORKER_VERBS: &[&str] = &["self-update", "schedule", "app", "uninstall", "admin-step", "admin-db", "check"];
/// Panel → worker: the person's Kora folder (`args: ["--kora-home", path]`).
pub const SESSION: &str = "session";
/// Panel → worker: that was all, exit.
pub const BYE: &str = "bye";
/// The hidden `kora` subcommand that is the worker.
pub const WORKER_COMMAND: &str = "admin-worker";
/// A child's last line that carries its result (`KORA-ADMIN-RESULT {json}`).
pub const RESULT_PREFIX: &str = "KORA-ADMIN-RESULT ";
/// Pipe names start with this.
pub const PIPE_PREFIX: &str = r"\\.\pipe\kora-admin-";
/// The pipe's security descriptor: Administrators (only when elevated: the group is deny-only in a
/// normal token) and SYSTEM may read and write; the owner (the panel's account) may only read the
/// descriptor, so it cannot grant itself more. Protected: nothing inherited.
pub const PIPE_SDDL: &str = "D:P(A;;GRGW;;;BA)(A;;GRGW;;;SY)(A;;RC;;;OW)";
/// How long the panel waits for the person to answer the prompt.
pub const PROMPT_LIMIT: Duration = Duration::from_secs(5 * 60);
/// After "Yes": how long the worker has to connect.
pub const CONNECT_LIMIT: Duration = Duration::from_secs(60);
/// A worker with nothing to do for this long exits (an install downloads for a while between its
/// administrator steps, so this is generous).
pub const IDLE_LIMIT: Duration = Duration::from_secs(90 * 60);

/// `\\.\pipe\kora-admin-<pid>-<32 hex digits>`.
pub fn pipe_name(panel_pid: u32, random: &[u8; 16]) -> String {
    format!("{PIPE_PREFIX}{panel_pid}-{}", hex::encode(random))
}

/// Is `name` a pipe name the panel makes (and nothing else, such as another machine's pipe)?
pub fn valid_pipe_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix(PIPE_PREFIX) else { return false };
    let Some((pid, rnd)) = rest.split_once('-') else { return false };
    !pid.is_empty() && pid.len() <= 10 && pid.bytes().all(|b| b.is_ascii_digit()) && rnd.len() == 32 && rnd.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// The worker's arguments (after the program name).
pub fn worker_args(pipe: &str, panel_pid: u32) -> Vec<String> {
    vec![WORKER_COMMAND.into(), "--pipe".into(), pipe.into(), "--parent".into(), panel_pid.to_string()]
}

/// Same program file? Windows paths: case-insensitive, `/` = `\`, the `\\?\` prefix ignored.
pub fn same_exe(a: &Path, b: &Path) -> bool {
    fn norm(p: &Path) -> String {
        let s = p.to_string_lossy().replace('/', "\\");
        let s = s.strip_prefix(r"\\?\UNC\").map(|r| format!(r"\\{r}")).unwrap_or(s);
        let s = s.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(s);
        s.trim_end_matches('\\').to_lowercase()
    }
    let (x, y) = (norm(a), norm(b));
    !x.is_empty() && x == y
}

/// Worker side: is the pipe's server the panel we were started for?
pub fn check_server_peer(expected_pid: u32, server_pid: u32, server_image: Option<&Path>, own_image: &Path) -> Result<(), String> {
    if server_pid != expected_pid {
        return Err(format!("the pipe belongs to process {server_pid}, not to Kora Panel ({expected_pid})"));
    }
    match server_image {
        Some(img) if same_exe(img, own_image) => Ok(()),
        Some(img) => Err(format!("process {server_pid} runs {}, not {}", img.display(), own_image.display())),
        None => Err(format!("cannot see which program process {server_pid} runs")),
    }
}

/// Panel side: is the client the worker we started?
pub fn check_client_peer(expected_pid: u32, client_pid: u32) -> Result<(), String> {
    if expected_pid != 0 && client_pid == expected_pid {
        Ok(())
    } else {
        Err(format!("process {client_pid} connected instead of Kora's administrator helper ({expected_pid})"))
    }
}

/// Parse `--pipe <name> --parent <pid>` (the worker's own arguments).
pub fn parse_worker_args(args: &[String]) -> Result<(String, u32), String> {
    let (mut pipe, mut parent) = (None, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--pipe" => pipe = it.next().cloned(),
            "--parent" => parent = it.next().and_then(|p| p.parse::<u32>().ok()),
            _ => return Err(format!("unknown argument {a}")),
        }
    }
    match (pipe, parent) {
        (Some(p), Some(pid)) if valid_pipe_name(&p) && pid != 0 => Ok((p, pid)),
        _ => Err("the administrator helper needs --pipe and --parent from Kora Panel".into()),
    }
}

// ---------------------------------------------------------------------------------------------
// Worker side: serving one session
// ---------------------------------------------------------------------------------------------

/// The `--kora-home` of a `session` line (absolute, sane).
pub fn parse_session(args: &[String]) -> Result<Option<String>, String> {
    match args {
        [] => Ok(None),
        [flag, path] if flag == "--kora-home" => {
            let p = Path::new(path);
            let absolute = p.is_absolute() || (cfg!(not(windows)) && path.starts_with('/'));
            if path.len() > MAX_ARG_BYTES || path.contains('\0') || !absolute || p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
                return Err("The Kora folder given to the administrator helper is not a full folder path.".into());
            }
            Ok(Some(path.clone()))
        }
        _ => Err("The session request was not understood.".into()),
    }
}

/// Split off the result line of an `admin-step` child (`None`: an ordinary line).
pub fn result_line(line: &str) -> Option<serde_json::Value> {
    line.strip_prefix(RESULT_PREFIX).and_then(|j| serde_json::from_str(j.trim()).ok())
}

fn send(w: &mut dyn Write, m: &Message) -> bool {
    let mut line = serde_json::to_string(m).unwrap_or_default();
    line.push('\n');
    w.write_all(line.as_bytes()).and_then(|_| w.flush()).is_ok()
}

/// Read one line of at most [`MAX_REQUEST_BYTES`] (`Ok(None)`: the other end closed the pipe).
fn read_line_capped(r: &mut dyn BufRead) -> io::Result<Option<String>> {
    let mut line = String::new();
    let n = r.take(MAX_REQUEST_BYTES as u64 + 1).read_line(&mut line)?;
    if n == 0 {
        return Ok(None);
    }
    if line.len() > MAX_REQUEST_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "line too long"));
    }
    Ok(Some(line))
}

/// Serve one session on an already checked connection: say hello, then answer requests until
/// `bye` or the end of the pipe. `base_env` = what every child gets (the panel user's name, ...);
/// `touch` is called around every request (`true` = busy) so a watchdog can see idleness.
pub fn serve(reader: &mut dyn BufRead, writer: &mut dyn Write, validator: &dyn Validator, executor: &dyn Executor, base_env: &[(String, String)], touch: &dyn Fn(bool)) -> io::Result<()> {
    if !send(writer, &Message::Hello { version: env!("CARGO_PKG_VERSION").into(), pid: std::process::id() }) {
        return Err(io::Error::new(io::ErrorKind::BrokenPipe, "the panel went away"));
    }
    let mut session_env: BTreeMap<String, String> = BTreeMap::new();
    loop {
        touch(false);
        let line = match read_line_capped(reader) {
            Ok(Some(l)) => l,
            Ok(None) => return Ok(()),
            Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                send(writer, &Message::Error { message: "The request was not understood.".into() });
                return Ok(());
            }
            Err(_) => return Ok(()),
        };
        touch(true);
        let Ok(req) = serde_json::from_str::<Request>(line.trim()) else {
            send(writer, &Message::Error { message: "The request was not understood.".into() });
            continue;
        };
        if req.verb == BYE {
            return Ok(());
        }
        if req.verb == PING {
            send(writer, &Message::Result { ok: true, code: 0, message: None, version: Some(env!("CARGO_PKG_VERSION").into()), data: None });
            continue;
        }
        if req.verb == SESSION {
            match parse_session(&req.args) {
                Ok(home) if req.env.is_empty() => {
                    if let Some(h) = home {
                        session_env.insert("KORA_HOME".into(), h);
                    }
                    send(writer, &Message::Result { ok: true, code: 0, message: None, version: None, data: None });
                }
                Ok(_) => {
                    send(writer, &Message::Error { message: "The session request was not understood.".into() });
                }
                Err(m) => {
                    send(writer, &Message::Error { message: m });
                }
            }
            continue;
        }
        let argv = match check_request_in(&req, WORKER_VERBS, &|v, a| validator.validate(v, a)) {
            Ok(a) => a,
            Err(r) => {
                send(writer, &Message::Error { message: r.0 });
                continue;
            }
        };
        let mut env: Vec<(String, String)> = base_env.to_vec();
        env.extend(session_env.iter().map(|(k, v)| (k.clone(), v.clone())));
        env.extend(req.env.iter().map(|(k, v)| (k.clone(), v.clone())));
        let secrets: Vec<&str> = req.env.values().map(String::as_str).collect();
        let mut alive = true;
        let mut data = None;
        let res = executor.run(&argv, &env, &mut |stream, text| {
            if stream == "stdout" {
                if let Some(v) = result_line(text) {
                    data = Some(v);
                    return;
                }
            }
            if alive {
                alive = send(writer, &Message::Progress { stream: stream.to_string(), line: mask_secrets(text, &secrets) });
            }
        });
        let sent = match res {
            Ok(code) => send(writer, &Message::Result { ok: code == 0, code, message: None, version: None, data }),
            Err(e) => send(writer, &Message::Error { message: format!("The administrator helper could not start Kora: {e}") }),
        };
        if !sent {
            return Ok(());
        }
    }
}

/// Runs `exe argv` as a child with the worker's environment plus `env`, no shell, no window, and
/// streams its output lines. Works on every platform (tests run it on Linux).
pub struct ChildExecutor {
    pub exe: std::path::PathBuf,
}

impl Executor for ChildExecutor {
    fn run(&self, argv: &[String], env: &[(String, String)], on_line: &mut dyn FnMut(&str, &str)) -> io::Result<i32> {
        use std::process::{Command, Stdio};
        let mut cmd = Command::new(&self.exe);
        cmd.args(argv).envs(env.iter().map(|(k, v)| (k, v))).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = cmd.spawn()?;
        let (tx, rx) = std::sync::mpsc::channel::<(&'static str, String)>();
        let mut readers = Vec::new();
        let pipes: [(&'static str, Option<Box<dyn Read + Send>>); 2] =
            [("stdout", child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>)), ("stderr", child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>))];
        for (name, pipe) in pipes {
            if let Some(p) = pipe {
                let tx = tx.clone();
                readers.push(std::thread::spawn(move || {
                    for chunk in io::BufReader::new(p).split(b'\n').map_while(Result::ok) {
                        // A result line is JSON that may be long: never cut or split it.
                        if chunk.starts_with(RESULT_PREFIX.as_bytes()) {
                            if tx.send((name, String::from_utf8_lossy(&chunk).trim_end().to_string())).is_err() {
                                return;
                            }
                            continue;
                        }
                        for l in super::server::split_lines(&chunk) {
                            if tx.send((name, l)).is_err() {
                                return;
                            }
                        }
                    }
                }));
            }
        }
        drop(tx);
        for (name, line) in rx {
            on_line(name, &line);
        }
        for r in readers {
            let _ = r.join();
        }
        Ok(child.wait()?.code().unwrap_or(-1))
    }
}

// ---------------------------------------------------------------------------------------------
// Panel side: one session
// ---------------------------------------------------------------------------------------------

/// How one request ended.
#[derive(Debug, Clone, PartialEq)]
pub struct StepOutcome {
    pub ok: bool,
    pub code: i32,
    /// The `admin-step` result ([`RESULT_PREFIX`]); may hold a secret (a new database login): never log it.
    pub data: Option<serde_json::Value>,
}

/// The panel's end of a session (any byte stream: the real pipe, or a socket pair in tests).
pub struct Session {
    reader: Box<dyn BufRead + Send>,
    writer: Box<dyn Write + Send>,
    /// The worker's process id (from its hello).
    pub worker_pid: u32,
}

fn lost(e: impl std::fmt::Display) -> HelperError {
    HelperError::Protocol(format!("Kora's administrator helper stopped answering ({e})."))
}

impl Session {
    /// Read the worker's hello (it must be this very version) and tell it the Kora folder.
    pub fn begin(reader: Box<dyn BufRead + Send>, writer: Box<dyn Write + Send>, kora_home: Option<&str>) -> Result<Session, HelperError> {
        let mut s = Session { reader, writer, worker_pid: 0 };
        match s.next()? {
            Message::Hello { version, pid } if version == env!("CARGO_PKG_VERSION") => s.worker_pid = pid,
            Message::Hello { version, .. } => return Err(HelperError::Protocol(format!("The administrator helper is Kora {version}, not {}.", env!("CARGO_PKG_VERSION")))),
            _ => return Err(HelperError::Protocol("Kora's administrator helper did not introduce itself.".into())),
        }
        let args = kora_home.map(|h| vec!["--kora-home".to_string(), h.to_string()]).unwrap_or_default();
        s.send(&Request { verb: SESSION.into(), args, env: BTreeMap::new() })?;
        match s.next()? {
            Message::Result { ok: true, .. } => Ok(s),
            Message::Error { message } => Err(HelperError::Rejected(message)),
            _ => Err(HelperError::Protocol("Kora's administrator helper did not accept the session.".into())),
        }
    }

    fn send(&mut self, req: &Request) -> Result<(), HelperError> {
        let mut line = serde_json::to_string(req).map_err(|e| HelperError::Protocol(e.to_string()))?;
        line.push('\n');
        self.writer.write_all(line.as_bytes()).and_then(|_| self.writer.flush()).map_err(lost)
    }

    fn next(&mut self) -> Result<Message, HelperError> {
        // The worker is elevated and checked, but a broken one must still not make the panel buffer forever.
        let mut line = String::new();
        let n = (&mut self.reader).take(4 * MAX_REQUEST_BYTES as u64).read_line(&mut line).map_err(lost)?;
        if n == 0 {
            return Err(lost("the connection closed"));
        }
        super::client::parse_message(&line)
    }

    /// Run one allow-listed command; `on_line` gets every output line.
    pub fn run(&mut self, verb: &str, args: &[String], env: &[(String, String)], on_line: &mut dyn FnMut(&str)) -> Result<StepOutcome, HelperError> {
        self.send(&Request { verb: verb.into(), args: args.to_vec(), env: env.iter().cloned().collect() })?;
        loop {
            match self.next()? {
                Message::Progress { line, .. } => on_line(&line),
                Message::Result { ok, code, data, .. } => return Ok(StepOutcome { ok, code, data }),
                Message::Error { message } => return Err(HelperError::Rejected(message)),
                Message::Hello { .. } => return Err(HelperError::Protocol("Kora's administrator helper sent something unexpected.".into())),
            }
        }
    }

    /// Tell the worker it is done (it exits); errors do not matter any more.
    pub fn finish(mut self) {
        let _ = self.send(&Request { verb: BYE.into(), ..Default::default() });
    }
}

// ---------------------------------------------------------------------------------------------
// Asking: outcomes and what the person reads
// ---------------------------------------------------------------------------------------------

/// Why no session could be opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenError {
    /// "No" at the prompt.
    Declined,
    /// Nobody answered within [`PROMPT_LIMIT`].
    TimedOut,
    /// An earlier prompt is still open.
    Busy,
    /// Not possible here (not Windows, or the panel already runs elevated).
    Unsupported,
    /// The worker started but did not connect, or failed the checks; or Windows refused to start it.
    Failed(String),
}

/// `ShellExecuteExW`'s `GetLastError()` after a failed `runas`.
pub fn shell_execute_error(code: u32) -> OpenError {
    match code {
        1223 => OpenError::Declined, // ERROR_CANCELLED: the person chose No
        2 | 3 => OpenError::Failed("Windows could not find Kora's program file to start it as administrator.".into()),
        5 => OpenError::Failed("Windows did not allow Kora to ask for administrator permission (access denied). A policy on this computer may block it.".into()),
        1155 | 1156 => OpenError::Failed("Windows could not start Kora as administrator.".into()),
        c => OpenError::Failed(format!("Windows could not show the administrator permission prompt (error {c}).")),
    }
}

/// What the failed attempt was for, to word the problem and offer the right choices.
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// "install the server" / "change the automatic updates" (after "Kora needed permission to ").
    pub what: String,
    /// The command to run as administrator instead ("Show me the command").
    pub command: Vec<String>,
    /// "Continue without the administrator parts" makes sense here (installs).
    pub can_skip_admin: bool,
}

fn with_choices(mut p: Problem, ctx: &Context) -> Problem {
    p.push_action(Action::RequestAdmin);
    if ctx.can_skip_admin {
        p.push_action(Action::ContinueWithoutAdmin);
    }
    if !ctx.command.is_empty() {
        p.push_action(Action::ShowCommand { command: ctx.command.clone(), elevated: true });
    }
    p
}

/// The Problem for an [`OpenError`] (Declined, TimedOut, Busy, Failed). Every one offers a way on.
pub fn open_problem(e: &OpenError, ctx: &Context) -> Problem {
    let what = if ctx.what.is_empty() { "do this".to_string() } else { ctx.what.clone() };
    let p = match e {
        OpenError::Declined => Problem::new(
            codes::ADMIN_DECLINED,
            format!("Windows did not get permission to {what}, so nothing that needs administrator rights was changed."),
        )
        .with_details("The Windows permission prompt was answered with No (or closed)."),
        OpenError::TimedOut => Problem::new(
            codes::ADMIN_TIMED_OUT,
            format!("Nobody answered the Windows permission prompt in time, so Kora could not {what}. Nothing that needs administrator rights was changed."),
        )
        .with_details("The prompt appears on the computer that runs Kora. If you did not see it, look for a flashing shield icon in the taskbar there."),
        OpenError::Busy => Problem::new(
            codes::ADMIN_BUSY,
            "Windows is still showing an earlier permission prompt from Kora. Answer it on the computer that runs Kora (No is fine), then try again.",
        ),
        OpenError::Unsupported => Problem::new(codes::NEEDS_ELEVATION, format!("Kora cannot ask for administrator permission here, so it cannot {what} itself.")),
        OpenError::Failed(m) => Problem::new(codes::ADMIN_LOST, format!("Kora could not get administrator permission to {what}.")).with_details(m.clone()),
    };
    with_choices(p, ctx)
}

/// The Problem for a remote session: the prompt would appear on a desktop nobody may be watching,
/// so Kora does not show it unless the person asks for it ("Ask for administrator permission").
pub fn remote_problem(ctx: &Context) -> Problem {
    let what = if ctx.what.is_empty() { "do this".to_string() } else { ctx.what.clone() };
    with_choices(
        Problem::new(
            codes::ADMIN_REMOTE,
            format!(
                "To {what}, Windows has to ask for administrator permission on the computer that runs Kora. You are signed in from another device, so nobody may be there to answer. If someone is at that computer, ask for permission and they can choose Yes."
            ),
        ),
        ctx,
    )
}

/// The worker went away in the middle (crash, closed by someone, the panel restarting).
pub fn lost_problem(e: &HelperError) -> Problem {
    Problem::new(codes::ADMIN_LOST, "Kora's administrator helper stopped before it finished. The step can be tried again (Windows asks for permission again).").with_details(e.to_string())
}

#[cfg(test)]
mod tests;
