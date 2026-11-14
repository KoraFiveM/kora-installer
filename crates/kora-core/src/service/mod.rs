//! OS service layer: systemd units / Windows services and the auto-update scheduler.
//!
//! Nothing here touches the OS directly. Every function either *generates* a
//! [`Plan`] (a list of described [`SystemStep`]s the TUI can show on the
//! elevation screen before anything happens) or runs commands through a
//! [`CommandRunner`], which tests replace with a fake.
//! Windows plans are plain command data (`sc.exe`, `schtasks`) and are
//! generated on any OS.

mod schedule;

pub use schedule::{
    scheduler_install_plan, scheduler_uninstall_plan, systemd_timer_unit, systemd_update_unit,
    Schedule, TimeOfDay, Weekday,
};

use std::io;

/// Errors from the service/firewall layers.
#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("command `{command}` failed ({detail})")]
    Command { command: String, detail: String },
    #[error("io error: {0}")]
    Io(#[from] io::Error),
}

pub type ServiceResult<T> = Result<T, ServiceError>;

/// Target operating system family (kept as data so plans are testable anywhere).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Linux,
    Windows,
}

impl Platform {
    /// The platform this binary is running on.
    pub fn current() -> Self {
        if cfg!(windows) {
            Platform::Windows
        } else {
            Platform::Linux
        }
    }
}

/// Result of running an external command.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutput {
    pub fn success(&self) -> bool {
        self.code == 0
    }
}

/// Everything that touches the OS goes through this trait.
pub trait CommandRunner {
    fn run(&self, program: &str, args: &[String]) -> io::Result<CommandOutput>;
    /// Write `contents` to `path` (creating parent dirs); `mode` is a Unix mode if set.
    fn write_file(&self, path: &str, contents: &str, mode: Option<u32>) -> io::Result<()>;
    fn remove_file(&self, path: &str) -> io::Result<()>;
}

/// Real runner using `std::process` and `std::fs`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn run(&self, program: &str, args: &[String]) -> io::Result<CommandOutput> {
        let out = std::process::Command::new(program).args(args).output()?;
        Ok(CommandOutput {
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }

    fn write_file(&self, path: &str, contents: &str, mode: Option<u32>) -> io::Result<()> {
        let p = std::path::Path::new(path);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(p, contents)?;
        #[cfg(unix)]
        if let Some(m) = mode {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(m))?;
        }
        #[cfg(not(unix))]
        let _ = mode;
        Ok(())
    }

    fn remove_file(&self, path: &str) -> io::Result<()> {
        match std::fs::remove_file(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }
}

/// What a step does when applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Run { program: String, args: Vec<String> },
    WriteFile { path: String, contents: String, mode: Option<u32> },
    RemoveFile { path: String },
}

/// One system change: plain-language description plus what it needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemStep {
    /// Plain-language text for the elevation screen.
    pub description: String,
    pub needs_admin: bool,
    pub action: Action,
    /// Tolerate a failing command (used by removal plans: "already gone" is fine).
    pub ignore_failure: bool,
}

impl SystemStep {
    pub fn run(description: impl Into<String>, program: &str, args: &[&str]) -> Self {
        SystemStep {
            description: description.into(),
            needs_admin: true,
            action: Action::Run {
                program: program.to_string(),
                args: args.iter().map(|s| s.to_string()).collect(),
            },
            ignore_failure: false,
        }
    }

    pub fn run_owned(description: impl Into<String>, program: &str, args: Vec<String>) -> Self {
        SystemStep {
            description: description.into(),
            needs_admin: true,
            action: Action::Run { program: program.to_string(), args },
            ignore_failure: false,
        }
    }

    pub fn write(description: impl Into<String>, path: &str, contents: String, mode: u32) -> Self {
        SystemStep {
            description: description.into(),
            needs_admin: true,
            action: Action::WriteFile { path: path.to_string(), contents, mode: Some(mode) },
            ignore_failure: false,
        }
    }

    pub fn remove(description: impl Into<String>, path: &str) -> Self {
        SystemStep {
            description: description.into(),
            needs_admin: true,
            action: Action::RemoveFile { path: path.to_string() },
            ignore_failure: true,
        }
    }

    pub fn tolerant(mut self) -> Self {
        self.ignore_failure = true;
        self
    }

    pub fn user_level(mut self) -> Self {
        self.needs_admin = false;
        self
    }

    /// Command line as text (for logs/preview); `None` for file actions.
    pub fn command_line(&self) -> Option<String> {
        match &self.action {
            Action::Run { program, args } => Some(format!("{program} {}", args.join(" "))),
            _ => None,
        }
    }
}

/// An ordered list of system changes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub steps: Vec<SystemStep>,
}

impl Plan {
    pub fn new(steps: Vec<SystemStep>) -> Self {
        Plan { steps }
    }
    pub fn needs_admin(&self) -> bool {
        self.steps.iter().any(|s| s.needs_admin)
    }
    pub fn descriptions(&self) -> Vec<&str> {
        self.steps.iter().map(|s| s.description.as_str()).collect()
    }
    pub fn extend(&mut self, other: Plan) {
        self.steps.extend(other.steps);
    }
}

/// Apply every step in order through `runner`; stops at the first hard failure.
pub fn apply_plan(plan: &Plan, runner: &dyn CommandRunner) -> ServiceResult<()> {
    for step in &plan.steps {
        let res: ServiceResult<()> = match &step.action {
            Action::Run { program, args } => {
                let line = step.command_line().unwrap_or_default();
                match runner.run(program, args) {
                    Ok(o) if o.success() => Ok(()),
                    Ok(o) => Err(ServiceError::Command {
                        command: line,
                        detail: format!("exit {}: {}", o.code, o.stderr.trim()),
                    }),
                    Err(e) => Err(ServiceError::Command { command: line, detail: e.to_string() }),
                }
            }
            Action::WriteFile { path, contents, mode } => {
                runner.write_file(path, contents, *mode).map_err(Into::into)
            }
            Action::RemoveFile { path } => runner.remove_file(path).map_err(Into::into),
        };
        if let Err(e) = res {
            if !step.ignore_failure {
                return Err(e);
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Managed server model
// ---------------------------------------------------------------------------

/// A server Kora manages as an OS service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedServer {
    /// Short id: lowercase letters, digits, `-`, `_` (1-48 chars).
    pub name: String,
    /// Server directory (working directory of the service).
    pub dir: String,
    /// Executable and arguments that start the server.
    pub start_command: Vec<String>,
    /// Unprivileged account that runs the server (Linux). Must not be root.
    pub user: String,
    /// Optional env file with secrets (`KEY=value` lines). Defaults to `<dir>/secrets.env`.
    pub env_file: Option<String>,
    /// Path of the `kora` binary (Windows service host and scheduler use it).
    pub kora_exe: String,
    /// Windows scheduled-task account; `None` means SYSTEM.
    pub windows_run_as: Option<String>,
}

fn bad_chars(s: &str) -> bool {
    s.chars().any(|c| c.is_control())
}

impl ManagedServer {
    pub fn validate(&self) -> ServiceResult<()> {
        let n = &self.name;
        if n.is_empty()
            || n.len() > 48
            || !n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            return Err(ServiceError::Invalid(
                "server name must be 1-48 characters: lowercase letters, digits, '-' or '_'".into(),
            ));
        }
        if self.dir.is_empty() || bad_chars(&self.dir) {
            return Err(ServiceError::Invalid("server folder is empty or has control characters".into()));
        }
        if self.start_command.is_empty() || self.start_command.iter().any(|a| bad_chars(a)) {
            return Err(ServiceError::Invalid("start command is empty or has control characters".into()));
        }
        let u = &self.user;
        let mut chars = u.chars();
        let first_ok = chars.next().is_some_and(|c| c.is_ascii_lowercase() || c == '_');
        if !first_ok || !u.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-') {
            return Err(ServiceError::Invalid("service user name is not a valid account name".into()));
        }
        if u == "root" {
            return Err(ServiceError::Invalid("the server must not run as root".into()));
        }
        if self.kora_exe.is_empty() || bad_chars(&self.kora_exe) || self.kora_exe.contains('"') {
            return Err(ServiceError::Invalid("kora executable path is invalid".into()));
        }
        if self.env_file.as_deref().is_some_and(bad_chars) {
            return Err(ServiceError::Invalid("env file path has control characters".into()));
        }
        if self.windows_run_as.as_deref().is_some_and(|s| bad_chars(s) || s.contains('"')) {
            return Err(ServiceError::Invalid("Windows account name is invalid".into()));
        }
        Ok(())
    }

    /// Service name on both OSes: `kora-<name>`.
    pub fn service_name(&self) -> String {
        format!("kora-{}", self.name)
    }

    pub fn unit_path(&self) -> String {
        format!("/etc/systemd/system/{}.service", self.service_name())
    }

    pub fn env_file_path(&self) -> String {
        self.env_file
            .clone()
            .unwrap_or_else(|| format!("{}/secrets.env", self.dir.trim_end_matches('/')))
    }
}

/// Quote one argument for a systemd `ExecStart=` line.
fn systemd_quote(arg: &str) -> String {
    let escaped = arg.replace('\\', "\\\\").replace('"', "\\\"").replace('%', "%%").replace('$', "$$");
    if escaped.is_empty() || escaped.contains(|c: char| c.is_whitespace() || c == '\'' || c == ';') || escaped != arg {
        format!("\"{escaped}\"")
    } else {
        escaped
    }
}

/// Hardened systemd unit for a managed server.
///
/// Clean stop: on SIGTERM txAdmin logs "Got SIGTERM, shutting down..." and the FXServer process
/// then dies with SIGSEGV during its own teardown (observed with artifact 38621: `code=dumped,
/// status=11/SEGV`). systemd counts `dumped` as a failure whatever `SuccessExitStatus=` or
/// `LimitCORE=0` say (both tried), so the unit's `ExecStart` is `kora service-host <dir>`, which
/// supervises FXServer and exits 0 for that segfault when a stop was requested (see
/// `installer::host`). `KillMode=mixed` sends SIGTERM only to that supervisor (it forwards it) and
/// SIGKILLs leftovers after `TimeoutStopSec`.
pub fn systemd_unit(server: &ManagedServer) -> ServiceResult<String> {
    server.validate()?;
    let exec = server.start_command.iter().map(|a| systemd_quote(a)).collect::<Vec<_>>().join(" ");
    // PrivateTmp gives the service its own empty /tmp: a server folder under /tmp or /var/tmp would vanish.
    let dir = server.dir.as_str();
    let under_tmp = ["/tmp", "/var/tmp"].iter().any(|t| dir == *t || dir.starts_with(&format!("{t}/")));
    let private_tmp = if under_tmp { "false" } else { "true" };
    Ok(format!(
        "[Unit]\n\
         Description=Kora server {name}\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         User={user}\n\
         WorkingDirectory={dir}\n\
         EnvironmentFile=-{env}\n\
         ExecStart={exec}\n\
         Restart=on-failure\n\
         RestartSec=10\n\
         KillMode=mixed\n\
         TimeoutStopSec=30\n\
         LimitNOFILE=65535\n\
         NoNewPrivileges=true\n\
         PrivateTmp={private_tmp}\n\
         PrivateDevices=true\n\
         ProtectSystem=full\n\
         ProtectKernelTunables=true\n\
         ProtectKernelModules=true\n\
         ProtectControlGroups=true\n\
         RestrictSUIDSGID=true\n\
         LockPersonality=true\n\
         ReadWritePaths={dir}\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        name = server.name,
        user = server.user,
        dir = server.dir,
        env = server.env_file_path(),
        private_tmp = private_tmp,
    ))
}

/// Plan to register (but not start) the server as an OS service.
pub fn install_plan(server: &ManagedServer, platform: Platform) -> ServiceResult<Plan> {
    server.validate()?;
    let svc = server.service_name();
    Ok(match platform {
        Platform::Linux => Plan::new(vec![
            SystemStep::write(
                format!("Create the background service definition for '{}' ({}), running as the unprivileged user '{}'", server.name, server.unit_path(), server.user),
                &server.unit_path(),
                systemd_unit(server)?,
                0o644,
            ),
            SystemStep::run("Tell systemd to read the new service definition", "systemctl", &["daemon-reload"]),
            SystemStep::run(format!("Start '{}' automatically at boot and restart it if it crashes", server.name), "systemctl", &["enable", &svc]),
        ]),
        Platform::Windows => {
            let bin = format!("\"{}\" service-host {}", server.kora_exe, server.name);
            Plan::new(vec![
                SystemStep::run_owned(
                    format!("Register the Windows service '{svc}' so '{}' starts automatically when Windows boots", server.name),
                    "sc.exe",
                    vec!["create".into(), svc.clone(), "binPath=".into(), bin, "start=".into(), "auto".into(), "DisplayName=".into(), format!("Kora server {}", server.name)],
                ),
                SystemStep::run_owned(
                    "Add a description to the service",
                    "sc.exe",
                    vec!["description".into(), svc.clone(), format!("Kora-managed server {}", server.name)],
                ),
                SystemStep::run_owned(
                    "Make the service restart itself if it crashes (after 60 seconds, up to three times, counter reset daily)",
                    "sc.exe",
                    vec!["failure".into(), svc.clone(), "reset=".into(), "86400".into(), "actions=".into(), "restart/60000/restart/60000/restart/60000".into()],
                ),
                SystemStep::run_owned(
                    "Treat a non-zero exit as a crash so the restart rule applies",
                    "sc.exe",
                    vec!["failureflag".into(), svc, "1".into()],
                ),
            ])
        }
    })
}

/// Plan to remove the service Kora created (and only that).
pub fn uninstall_plan(server: &ManagedServer, platform: Platform) -> ServiceResult<Plan> {
    server.validate()?;
    let svc = server.service_name();
    Ok(match platform {
        Platform::Linux => Plan::new(vec![
            SystemStep::run(format!("Stop '{}' if it is running", server.name), "systemctl", &["stop", &svc]).tolerant(),
            SystemStep::run(format!("Stop '{}' starting at boot", server.name), "systemctl", &["disable", &svc]).tolerant(),
            SystemStep::remove(format!("Delete Kora's service definition {}", server.unit_path()), &server.unit_path()),
            SystemStep::run("Tell systemd the service definition is gone", "systemctl", &["daemon-reload"]),
            SystemStep::run(format!("Clear any leftover 'failed' mark for '{}'", server.name), "systemctl", &["reset-failed", &svc]).tolerant(),
        ]),
        Platform::Windows => Plan::new(vec![
            SystemStep::run(format!("Stop '{}' if it is running", server.name), "sc.exe", &["stop", &svc]).tolerant(),
            SystemStep::run(format!("Remove the Windows service '{svc}' that Kora created"), "sc.exe", &["delete", &svc]),
        ]),
    })
}

/// start/stop/restart verbs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlAction {
    Start,
    Stop,
    Restart,
}

impl ControlAction {
    fn verb(self) -> &'static str {
        match self {
            ControlAction::Start => "start",
            ControlAction::Stop => "stop",
            ControlAction::Restart => "restart",
        }
    }
}

/// Describes start/stop/restart as steps (so the UI can explain them).
pub fn control_plan(server: &ManagedServer, platform: Platform, action: ControlAction) -> ServiceResult<Plan> {
    server.validate()?;
    let svc = server.service_name();
    let text = format!("{} the server '{}'", capitalise(action.verb()), server.name);
    Ok(Plan::new(match (platform, action) {
        (Platform::Linux, a) => vec![SystemStep::run(text, "systemctl", &[a.verb(), &svc])],
        (Platform::Windows, ControlAction::Restart) => vec![
            SystemStep::run(format!("Stop the server '{}'", server.name), "sc.exe", &["stop", &svc]).tolerant(),
            SystemStep::run(format!("Start the server '{}' again", server.name), "sc.exe", &["start", &svc]),
        ],
        (Platform::Windows, a) => vec![SystemStep::run(text, "sc.exe", &[a.verb(), &svc])],
    }))
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// Observed service state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceState {
    Running,
    Stopped,
    /// Not registered with the OS.
    NotInstalled,
    Other(String),
}

/// Talks to the OS about one platform's services through a runner.
pub struct ServiceManager<'a> {
    pub platform: Platform,
    pub runner: &'a dyn CommandRunner,
}

impl<'a> ServiceManager<'a> {
    pub fn new(platform: Platform, runner: &'a dyn CommandRunner) -> Self {
        ServiceManager { platform, runner }
    }

    fn control(&self, server: &ManagedServer, action: ControlAction) -> ServiceResult<()> {
        apply_plan(&control_plan(server, self.platform, action)?, self.runner)
    }
    pub fn start(&self, server: &ManagedServer) -> ServiceResult<()> {
        self.control(server, ControlAction::Start)
    }
    pub fn stop(&self, server: &ManagedServer) -> ServiceResult<()> {
        self.control(server, ControlAction::Stop)
    }
    pub fn restart(&self, server: &ManagedServer) -> ServiceResult<()> {
        self.control(server, ControlAction::Restart)
    }

    pub fn status(&self, server: &ManagedServer) -> ServiceResult<ServiceState> {
        server.validate()?;
        let svc = server.service_name();
        let io = |e: io::Error| ServiceError::Command { command: "status".into(), detail: e.to_string() };
        Ok(match self.platform {
            Platform::Linux => {
                let o = self.runner.run("systemctl", &["is-active".into(), svc]).map_err(io)?;
                match o.stdout.trim() {
                    "active" | "activating" => ServiceState::Running,
                    "inactive" | "failed" | "deactivating" => ServiceState::Stopped,
                    "" | "unknown" => ServiceState::NotInstalled,
                    other => ServiceState::Other(other.to_string()),
                }
            }
            Platform::Windows => {
                let o = self.runner.run("sc.exe", &["query".into(), svc]).map_err(io)?;
                let text = o.stdout.to_uppercase();
                if o.code == 1060 || text.contains("1060") {
                    ServiceState::NotInstalled
                } else if text.contains("RUNNING") || text.contains("START_PENDING") {
                    ServiceState::Running
                } else if text.contains("STOPPED") || text.contains("STOP_PENDING") {
                    ServiceState::Stopped
                } else {
                    ServiceState::Other(o.stdout.trim().to_string())
                }
            }
        })
    }

    pub fn is_running(&self, server: &ManagedServer) -> ServiceResult<bool> {
        Ok(self.status(server)? == ServiceState::Running)
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use std::cell::RefCell;

    /// Fake runner: records every call and answers from scripted responses.
    #[derive(Default)]
    pub struct FakeRunner {
        pub log: RefCell<Vec<String>>,
        /// (substring of "program args", output) – first match wins; default is exit 0, empty.
        pub responses: RefCell<Vec<(String, CommandOutput)>>,
        pub files: RefCell<Vec<(String, String, Option<u32>)>>,
    }

    impl FakeRunner {
        pub fn respond(&self, needle: &str, code: i32, stdout: &str) {
            self.responses.borrow_mut().push((
                needle.to_string(),
                CommandOutput { code, stdout: stdout.to_string(), stderr: String::new() },
            ));
        }
        pub fn calls(&self) -> Vec<String> {
            self.log.borrow().clone()
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(&self, program: &str, args: &[String]) -> io::Result<CommandOutput> {
            let line = format!("{program} {}", args.join(" "));
            self.log.borrow_mut().push(line.clone());
            for (needle, out) in self.responses.borrow().iter() {
                if line.contains(needle) {
                    return Ok(out.clone());
                }
            }
            Ok(CommandOutput::default())
        }
        fn write_file(&self, path: &str, contents: &str, mode: Option<u32>) -> io::Result<()> {
            self.log.borrow_mut().push(format!("write {path}"));
            self.files.borrow_mut().push((path.into(), contents.into(), mode));
            Ok(())
        }
        fn remove_file(&self, path: &str) -> io::Result<()> {
            self.log.borrow_mut().push(format!("remove {path}"));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests;
