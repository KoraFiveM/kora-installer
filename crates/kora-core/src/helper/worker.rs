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
//! 3. Before asking, the panel opens its own program file so that nobody can write, rename or
//!    delete it until the session ends (no write or delete sharing): the prompt and every
//!    elevated child start exactly the file the panel runs. The worker refuses to run unless it is
//!    elevated, locks its own file the same way, connects to the pipe, and checks that the process
//!    at the other end ([`check_server_peer`]) is the panel pid it was given AND runs the very same
//!    FILE as itself (volume serial and file index, not path text). The panel checks that the
//!    client is the process it just started ([`check_client_peer`]). Only then does anything else
//!    travel.
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
//!   Windows itself does not treat UAC as a boundary against the same user. What we guarantee:
//!   from the moment the panel asks until the session ends, the program file it runs cannot be
//!   replaced, so the prompt and every elevated child start that file; the elevated process takes
//!   orders only from the panel process that started it, and only allow-listed ones; its children
//!   get an environment built from nothing ([`elevated_child_env`]), so the person's own
//!   environment variables cannot steer them. What we cannot guarantee: a program running as the
//!   person BEFORE the panel starts can replace `kora.exe` in their own folder (that is true of
//!   every program a person installs for themselves), and one that injects code into the running
//!   panel can use an open session until it ends.
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
pub use super::server::Executor;
use super::server::Validator;
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

/// A file's identity on Windows: (volume serial number, file index). Two paths name the same
/// file exactly when these are equal (8.3 names, `subst` drives, junctions and case all resolve).
pub type FileId = (u32, u64);

/// Worker side: is the pipe's server the panel we were started for? The panel's program file and
/// the worker's own (both taken from the running processes, then opened) must be the very same
/// file; `server_image` is only for the message.
pub fn check_server_peer(expected_pid: u32, server_pid: u32, server: Option<FileId>, own: Option<FileId>, server_image: &str) -> Result<(), String> {
    if server_pid != expected_pid {
        return Err(format!("the pipe belongs to process {server_pid}, not to Kora Panel ({expected_pid})"));
    }
    match (server, own) {
        (Some(a), Some(b)) if a == b => Ok(()),
        (Some(_), Some(_)) => Err(format!("process {server_pid} runs {server_image}, which is not this program")),
        _ => Err(format!("cannot see which program process {server_pid} runs")),
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
    serve_with(reader, writer, validator, executor, base_env, touch, &ServeOptions::default())
}

/// How one served connection differs between the worker and the persistent helper.
pub struct ServeOptions<'a> {
    /// The verbs at all (the worker's [`WORKER_VERBS`], the helper's `winhelper::SERVICE_VERBS`).
    pub verbs: &'a [&'a str],
    /// The helper: the only Kora folder a `session` may name (recorded at install time).
    pub fixed_home: Option<&'a str>,
    /// Called after every request that ran: (verb, step or sub-command, ok). For the log (never
    /// arguments or environment values) and for the helper's restart after updating itself.
    pub on_done: &'a dyn Fn(&str, &str, bool),
}

impl Default for ServeOptions<'_> {
    fn default() -> Self {
        ServeOptions { verbs: WORKER_VERBS, fixed_home: None, on_done: &|_, _, _| {} }
    }
}

/// What a request is about, for the log: the `--step` of an `admin-step`, else the first argument
/// when it is a plain word (`start`, `refresh`), else nothing. Never a path or a value.
pub fn log_label(verb: &str, args: &[String]) -> String {
    if verb == "admin-step" {
        let action = args.first().map(String::as_str).unwrap_or("");
        let step = args.iter().position(|a| a == "--step").and_then(|i| args.get(i + 1)).map(String::as_str).unwrap_or("");
        return format!("{action} {step}").trim().to_string();
    }
    args.first().filter(|a| !a.is_empty() && a.len() <= 24 && a.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')).cloned().unwrap_or_default()
}

/// [`serve`] with [`ServeOptions`].
pub fn serve_with(reader: &mut dyn BufRead, writer: &mut dyn Write, validator: &dyn Validator, executor: &dyn Executor, base_env: &[(String, String)], touch: &dyn Fn(bool), opts: &ServeOptions) -> io::Result<()> {
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
                Ok(home) if req.env.is_empty() && opts.fixed_home.is_some_and(|f| !super::winhelper::session_home_ok(home.as_deref(), f)) => {
                    send(writer, &Message::Error { message: "Kora's helper works only with the Kora folder it was turned on for.".into() });
                }
                Ok(home) if req.env.is_empty() => {
                    // The helper's children get the recorded folder from their base environment.
                    if let (Some(h), None) = (home, opts.fixed_home) {
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
        let argv = match check_request_in(&req, opts.verbs, &|v, a| validator.validate(v, a)) {
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
                if !alive {
                    // Nobody is listening any more: do not leave an elevated child running for nobody.
                    executor.cancel();
                }
            }
        });
        (opts.on_done)(&req.verb, &log_label(&req.verb, &req.args), matches!(res, Ok(0)));
        let sent = match res {
            Ok(code) => send(writer, &Message::Result { ok: code == 0, code, message: None, version: None, data }),
            Err(e) => send(writer, &Message::Error { message: format!("The administrator helper could not start Kora: {e}") }),
        };
        if !sent {
            return Ok(());
        }
    }
}

/// Folders as Windows itself reports them (system directory and known-folder APIs), never from
/// inherited variables: a user can set `ProgramFiles` or `TEMP` in their own environment, and an
/// elevated child must not take its trust roots from there.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SystemDirs {
    /// `C:\Windows`
    pub system_root: String,
    /// `C:\Program Files`
    pub program_files: String,
    /// `C:\Program Files (x86)` (may be empty)
    pub program_files_x86: String,
    /// `C:\ProgramData`
    pub program_data: String,
}

/// Variables that steer Kora (test seams and overrides). Never honoured by an elevated child of the
/// worker or the helper: [`elevated_child_env`] does not pass them, and the child removes them
/// again at start ([`scrub_overrides`]) as a second fence.
pub const OVERRIDE_VARS: &[&str] = &[
    "KORA_SELF_UPDATE_API",
    "KORA_TEST_ARTIFACT_MIRROR",
    "KORA_TEST_FAIL_HEALTH",
    "KORA_MARIADB_HOME",
    "KORA_CATALOG",
    "KORA_SERVICE_USER",
    "KORA_DB_HOST",
    "KORA_DB_PORT",
    "KORA_DB_ADMIN_USER",
    "KORA_APP_PREFIX",
    "KORA_SKIP_SPACE_CHECK",
    "KORA_ALLOW_ROOT_PANEL",
    "KORA_UPDATE_AS",
];

/// The complete environment of an elevated child, built from nothing (the child is started with
/// an empty environment plus this): system folders from Windows, a PATH of system folders only,
/// who "you" is (the panel's account, also when another administrator answered the prompt) with
/// that account's own profile folders (`profile` = its `ProfileImagePath`), the Kora folder, and
/// nothing else. The request's two secret variables are added per request.
pub fn elevated_child_env(sys: &SystemDirs, user: &super::winpipe::PanelUser, profile: Option<&str>, kora_home: Option<&str>) -> Vec<(String, String)> {
    let root = sys.system_root.trim_end_matches(['\\', '/']);
    let s32 = format!("{root}\\System32");
    let drive: String = root.chars().take(2).collect();
    let mut v: Vec<(String, String)> = vec![
        ("SystemRoot".into(), root.into()),
        ("windir".into(), root.into()),
        ("SystemDrive".into(), drive),
        ("ComSpec".into(), format!("{s32}\\cmd.exe")),
        ("PATHEXT".into(), ".COM;.EXE;.BAT;.CMD".into()),
        ("PATH".into(), format!("{s32};{root};{s32}\\Wbem;{s32}\\WindowsPowerShell\\v1.0")),
        ("TEMP".into(), format!("{root}\\Temp")),
        ("TMP".into(), format!("{root}\\Temp")),
        ("ProgramFiles".into(), sys.program_files.clone()),
        ("ProgramW6432".into(), sys.program_files.clone()),
        ("CommonProgramFiles".into(), format!("{}\\Common Files", sys.program_files)),
        ("ProgramData".into(), sys.program_data.clone()),
        ("ALLUSERSPROFILE".into(), sys.program_data.clone()),
    ];
    if !sys.program_files_x86.is_empty() {
        v.push(("ProgramFiles(x86)".into(), sys.program_files_x86.clone()));
    }
    v.extend(super::winpipe::child_env(user));
    if let Some(p) = profile.map(|p| p.trim_end_matches(['\\', '/'])).filter(|p| !p.is_empty()) {
        v.push(("USERPROFILE".into(), p.into()));
        v.push(("APPDATA".into(), format!("{p}\\AppData\\Roaming")));
        v.push(("LOCALAPPDATA".into(), format!("{p}\\AppData\\Local")));
    }
    if let Some(h) = kora_home {
        v.push(("KORA_HOME".into(), h.into()));
    }
    v
}

/// The worker itself (elevated by the prompt, started with the person's environment): forget
/// [`OVERRIDE_VARS`] and take the system folders from Windows before anything else runs. Its
/// children get their own environment built from nothing anyway ([`elevated_child_env`]).
pub fn sanitize_worker_env(sys: &SystemDirs) {
    let (remove, set) = worker_env_changes(sys);
    for k in remove {
        std::env::remove_var(k);
    }
    for (k, v) in set {
        std::env::set_var(k, v);
    }
}

/// What [`sanitize_worker_env`] removes and sets.
pub fn worker_env_changes(sys: &SystemDirs) -> (Vec<&'static str>, Vec<(&'static str, String)>) {
    let mut set = Vec::new();
    if !sys.system_root.is_empty() {
        let root = sys.system_root.trim_end_matches(['\\', '/']);
        for (k, v) in [("SystemRoot", root.to_string()), ("windir", root.to_string()), ("SystemDrive", root.chars().take(2).collect()), ("ProgramFiles", sys.program_files.clone()), ("ProgramData", sys.program_data.clone()), ("TEMP", format!("{root}\\Temp")), ("TMP", format!("{root}\\Temp"))] {
            if !v.is_empty() {
                set.push((k, v));
            }
        }
    }
    (OVERRIDE_VARS.to_vec(), set)
}

/// In an elevated child of the worker or the helper (`KORA_HELPER=1`): forget [`OVERRIDE_VARS`]
/// before anything reads them. Call first thing in `main`, before any thread starts.
pub fn scrub_overrides() {
    if std::env::var("KORA_HELPER").is_ok_and(|v| v == "1") && crate::elevate::is_elevated() {
        for k in OVERRIDE_VARS {
            std::env::remove_var(k);
        }
    }
}

/// How long one elevated child may run (an install of Kora's MariaDB downloads for a while).
pub const STEP_LIMIT: Duration = Duration::from_secs(45 * 60);

/// One elevated child at a time in this process (the worker's sessions, the helper's connections).
static CHILD_RUNS: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Runs `exe argv` as a child with ONLY `env` (the environment is cleared first), no shell, no
/// window, and streams its output lines. One child at a time per process; a child is killed when
/// it runs longer than `limit` or when [`Executor::cancel`] was called (the panel went away).
/// Works on every platform (tests run it on Linux).
pub struct ChildExecutor {
    pub exe: std::path::PathBuf,
    pub limit: Duration,
    pub cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl ChildExecutor {
    pub fn new(exe: std::path::PathBuf) -> Self {
        ChildExecutor { exe, limit: STEP_LIMIT, cancelled: Default::default() }
    }
}

impl Executor for ChildExecutor {
    fn cancel(&self) {
        self.cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    fn run(&self, argv: &[String], env: &[(String, String)], on_line: &mut dyn FnMut(&str, &str)) -> io::Result<i32> {
        use std::process::{Command, Stdio};
        use std::sync::atomic::Ordering;
        let _one_at_a_time = CHILD_RUNS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.cancelled.load(Ordering::SeqCst) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "Kora Panel went away"));
        }
        let mut cmd = Command::new(&self.exe);
        cmd.args(argv).env_clear().envs(env.iter().map(|(k, v)| (k, v))).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = cmd.spawn()?;
        let (tx, rx) = std::sync::mpsc::channel::<(&'static str, String)>();
        let pipes: [(&'static str, Option<Box<dyn Read + Send>>); 2] =
            [("stdout", child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>)), ("stderr", child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>))];
        for (name, pipe) in pipes {
            if let Some(p) = pipe {
                let tx = tx.clone();
                // Not joined: a grandchild that keeps the pipe open must not keep us waiting.
                std::thread::spawn(move || {
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
                });
            }
        }
        drop(tx);
        let deadline = std::time::Instant::now() + self.limit;
        loop {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok((name, line)) => on_line(name, &line),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    let why = if self.cancelled.load(Ordering::SeqCst) {
                        Some("Kora Panel went away, so the step was stopped".to_string())
                    } else if std::time::Instant::now() > deadline {
                        Some(format!("the step took longer than {} minutes and was stopped", self.limit.as_secs() / 60))
                    } else {
                        None
                    };
                    if let Some(why) = why {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(io::Error::new(io::ErrorKind::TimedOut, why));
                    }
                    if let Ok(Some(st)) = child.try_wait() {
                        // Exited; give the readers a moment for the last lines, then stop waiting.
                        let grace = std::time::Instant::now() + Duration::from_secs(2);
                        while let Ok((name, line)) = rx.recv_timeout(grace.saturating_duration_since(std::time::Instant::now())) {
                            on_line(name, &line);
                        }
                        return Ok(st.code().unwrap_or(-1));
                    }
                }
            }
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

static PANEL_RUNS: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The panel's end of a session (any byte stream: the real pipe, or a socket pair in tests).
pub struct Session {
    reader: Box<dyn BufRead + Send>,
    writer: Box<dyn Write + Send>,
    /// The worker's process id (from its hello).
    pub worker_pid: u32,
    /// Kept for as long as the session lives (the lock on the program file).
    guard: Option<Box<dyn Send>>,
}

fn lost(e: impl std::fmt::Display) -> HelperError {
    HelperError::Protocol(format!("Kora's administrator helper stopped answering ({e})."))
}

impl Session {
    /// Read the worker's hello (it must be this very version) and tell it the Kora folder.
    pub fn begin(reader: Box<dyn BufRead + Send>, writer: Box<dyn Write + Send>, kora_home: Option<&str>) -> Result<Session, HelperError> {
        let mut s = Session { reader, writer, worker_pid: 0, guard: None };
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

    /// Keep `g` alive until the session ends.
    pub fn hold(&mut self, g: Box<dyn Send>) {
        self.guard = Some(g);
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

    /// Run one allow-listed command; `on_line` gets every output line. One elevated run at a time
    /// in the panel, however many sessions are open (an install's and a separate action's).
    pub fn run(&mut self, verb: &str, args: &[String], env: &[(String, String)], on_line: &mut dyn FnMut(&str)) -> Result<StepOutcome, HelperError> {
        let _one_at_a_time = PANEL_RUNS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
