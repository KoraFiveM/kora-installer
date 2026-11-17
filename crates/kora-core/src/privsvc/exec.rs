//! Running the service's child: the `kora` program from the protected folder, started with an environment
//! built from nothing (never the caller's, never the service's own), no shell, one at a time, a time limit,
//! cancelled when the caller goes away. Works on every platform (the tests run it on Linux).

use std::io::{self, BufRead, Read};
use std::time::Duration;

/// A child's last line that carries its result (`KORA-ADMIN-RESULT {json}`): it becomes the answer's `data`
/// and is never shown or logged.
pub const RESULT_PREFIX: &str = "KORA-ADMIN-RESULT ";

/// Split off the result line of an `admin-step` child (`None`: an ordinary line).
pub fn result_line(line: &str) -> Option<serde_json::Value> {
    line.strip_prefix(RESULT_PREFIX).and_then(|j| serde_json::from_str(j.trim()).ok())
}

/// Runs the child. `on_line(stream, line)` is called for each output line; returns the exit code.
pub trait Executor: Send + Sync {
    fn run(&self, argv: &[String], env: &[(String, String)], on_line: &mut dyn FnMut(&str, &str)) -> io::Result<i32>;
    /// The other end went away: stop the running child (if the executor can).
    fn cancel(&self) {}
}

const LINE_CAP: usize = 8 * 1024;

/// Split a chunk of child output into display lines (handles `\r` progress redraws).
pub fn split_lines(chunk: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(chunk).split(['\n', '\r']).map(|l| l.trim_end()).filter(|l| !l.is_empty()).map(|l| l.chars().take(LINE_CAP).collect()).collect()
}

/// The account the panel runs as, seen by the worker (from the panel process's token).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PanelUser {
    /// `S-1-5-21-...`
    pub sid: String,
    pub domain: String,
    pub name: String,
    /// The account is in the Administrators group (even when the panel's token does not use it):
    /// a person who could have elevated themselves, so folders an elevated Kora made for them
    /// (owned by Administrators) are theirs too.
    pub admin_member: bool,
}

/// The environment every child of the worker gets on top of the worker's own: who "you" is
/// (`USERNAME`/`USERDOMAIN`, read by `service::installing_user`), even when an administrator of
/// another account answered the prompt.
pub fn user_env(u: &PanelUser) -> Vec<(String, String)> {
    vec![
        ("USERNAME".into(), u.name.clone()),
        ("USERDOMAIN".into(), u.domain.clone()),
        ("KORA_WORKER_USER_SID".into(), u.sid.clone()),
        ("KORA_WORKER_ADMIN_MEMBER".into(), if u.admin_member { "1" } else { "0" }.into()),
    ]
}

pub use super::winapi::SystemDirs;

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
pub fn elevated_child_env(sys: &SystemDirs, user: &PanelUser, profile: Option<&str>, kora_home: Option<&str>) -> Vec<(String, String)> {
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
    v.extend(user_env(user));
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

/// In an elevated child of the service (`KORA_SERVICE=1`): forget [`OVERRIDE_VARS`]
/// before anything reads them. Call first thing in `main`, before any thread starts.
pub fn scrub_overrides() {
    if super::paths::in_service() && crate::elevate::is_elevated() {
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
                        for l in split_lines(&chunk) {
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
