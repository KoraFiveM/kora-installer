//! MariaDB detect / install-plan / provision / rollback.
//!
//! Policy (mirrors the old Go installer): always CREATE a new database + user
//! with generated names and passwords, GRANT only on that database, and never
//! reuse, alter or drop anything Kora did not create in this run.
//!
//! All external commands go through [`CommandRunner`] so tests use a fake.
//! Install steps are *described* ([`InstallPlan`]), not blindly executed.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- errors

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("invalid identifier: {0}")]
    InvalidIdentifier(String),
    #[error("invalid value: {0}")]
    InvalidValue(String),
    #[error("command failed: {0}")]
    Command(String),
    #[error("unsupported platform: {0}")]
    Unsupported(String),
    #[error("name collision: {0}")]
    Collision(String),
    #[error("random source failed")]
    Random,
    #[error("checksum error: {0}")]
    Checksum(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("state error: {0}")]
    State(#[from] serde_json::Error),
}

pub type DbResult<T> = Result<T, DbError>;

// ---------------------------------------------------------- command runner

/// A planned external command. `Debug` never reveals env values, stdin, or
/// `--password=` arguments.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub stdin: Option<String>,
}

impl CommandSpec {
    pub fn new(program: &str, args: &[&str]) -> Self {
        Self {
            program: program.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            ..Self::default()
        }
    }
    /// Human readable, redacted command line.
    pub fn display(&self) -> String {
        let mut parts = vec![self.program.clone()];
        for a in &self.args {
            if a.starts_with("--password=") {
                parts.push("--password=<redacted>".into());
            } else {
                parts.push(a.clone());
            }
        }
        parts.join(" ")
    }
}

impl fmt::Debug for CommandSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommandSpec")
            .field("cmdline", &self.display())
            .field("env_keys", &self.env.iter().map(|(k, _)| k).collect::<Vec<_>>())
            .field("stdin", &self.stdin.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

#[derive(Debug, Clone, Default)]
pub struct CommandOutput {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutput {
    pub fn ok(&self) -> bool {
        self.status == 0
    }
}

/// Abstraction over process execution.
pub trait CommandRunner {
    fn run(&self, spec: &CommandSpec) -> DbResult<CommandOutput>;
}

/// Real runner backed by `std::process`.
pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn run(&self, spec: &CommandSpec) -> DbResult<CommandOutput> {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let mut cmd = Command::new(&spec.program);
        cmd.args(&spec.args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(if spec.stdin.is_some() { Stdio::piped() } else { Stdio::null() });
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| DbError::Command(format!("{}: {e}", spec.program)))?;
        if let (Some(input), Some(mut stdin)) = (&spec.stdin, child.stdin.take()) {
            stdin.write_all(input.as_bytes())?;
        }
        let out = child.wait_with_output()?;
        Ok(CommandOutput {
            status: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

// ------------------------------------------------------------------ secret

/// String whose `Debug`/`Display` are redacted. Serialises its real value
/// (needed for the 0600 secrets file only).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}
impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

// --------------------------------------------------------------- platform

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Linux,
    Windows,
    Other,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "linux") {
            Os::Linux
        } else {
            Os::Other
        }
    }
}

// --------------------------------------------------------------- detection

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedService {
    /// systemd unit or Windows service name.
    pub name: String,
    pub running: bool,
    pub version: Option<String>,
    pub port: u16,
    pub socket: Option<String>,
}

const DEFAULT_PORT: u16 = 3306;
const DEFAULT_SOCKET: &str = "/run/mysqld/mysqld.sock";

/// Detect existing MariaDB/MySQL services. Read-only.
pub fn detect(runner: &dyn CommandRunner, os: Os) -> DbResult<Vec<DetectedService>> {
    match os {
        Os::Linux => detect_linux(runner),
        Os::Windows => detect_windows(runner),
        Os::Other => Err(DbError::Unsupported("only Linux and Windows are supported".into())),
    }
}

fn detect_linux(runner: &dyn CommandRunner) -> DbResult<Vec<DetectedService>> {
    let mut found = Vec::new();
    for unit in ["mariadb", "mysql", "mysqld"] {
        let listed = runner.run(&CommandSpec::new(
            "systemctl",
            &["list-unit-files", &format!("{unit}.service"), "--no-legend"],
        ))?;
        if !listed.ok() || listed.stdout.trim().is_empty() {
            continue;
        }
        let active = runner.run(&CommandSpec::new("systemctl", &["is-active", unit]))?;
        let running = active.stdout.trim() == "active";
        let mut svc = DetectedService {
            name: unit.into(),
            running,
            version: None,
            port: DEFAULT_PORT,
            socket: Some(DEFAULT_SOCKET.into()),
        };
        if running {
            // Best effort: needs socket auth (root) to succeed.
            let client = if unit == "mariadb" { "mariadb" } else { "mysql" };
            let q = CommandSpec::new(
                client,
                &["-N", "-B", "-e", "SELECT @@version, @@port, IFNULL(@@socket,'')"],
            );
            if let Ok(out) = runner.run(&q) {
                if out.ok() {
                    apply_query_info(&mut svc, &out.stdout);
                }
            }
        }
        if svc.version.is_none() {
            let bin = if unit == "mariadb" { "mariadbd" } else { "mysqld" };
            if let Ok(out) = runner.run(&CommandSpec::new(bin, &["--version"])) {
                if out.ok() {
                    svc.version = parse_version(&out.stdout);
                }
            }
        }
        found.push(svc);
    }
    Ok(found)
}

fn apply_query_info(svc: &mut DetectedService, out: &str) {
    let cols: Vec<&str> = out.trim().split('\t').collect();
    if cols.len() == 3 {
        svc.version = Some(cols[0].to_string());
        if let Ok(p) = cols[1].parse() {
            svc.port = p;
        }
        svc.socket = (!cols[2].is_empty()).then(|| cols[2].to_string());
    }
}

/// Extract a version like `10.11.6` from `--version` output.
pub fn parse_version(s: &str) -> Option<String> {
    let after = s.split("Ver ").nth(1).unwrap_or(s);
    let tok: String = after
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    (tok.contains('.') && !tok.starts_with('.')).then_some(tok.trim_end_matches('.').to_string())
}

fn detect_windows(runner: &dyn CommandRunner) -> DbResult<Vec<DetectedService>> {
    let script = "Get-CimInstance Win32_Service | Where-Object { $_.Name -match '^(MariaDB|MySQL|KoraMariaDB)' } | ForEach-Object { \"$($_.Name)|$($_.State)|$($_.PathName)\" }";
    let out = runner.run(&CommandSpec::new("powershell", &["-NoProfile", "-NonInteractive", "-Command", script]))?;
    if !out.ok() {
        return Err(DbError::Command("service query failed".into()));
    }
    Ok(parse_windows_services(&out.stdout))
}

pub fn parse_windows_services(text: &str) -> Vec<DetectedService> {
    text.lines()
        .filter_map(|l| {
            let mut it = l.trim().splitn(3, '|');
            let name = it.next()?.trim();
            let state = it.next()?.trim();
            let path = it.next().unwrap_or("");
            if name.is_empty() {
                return None;
            }
            let port = path
                .split_whitespace()
                .find_map(|a| a.strip_prefix("--port="))
                .and_then(|p| p.parse().ok())
                .unwrap_or(DEFAULT_PORT);
            Some(DetectedService {
                name: name.into(),
                running: state.eq_ignore_ascii_case("running"),
                version: None,
                port,
                socket: None,
            })
        })
        .collect()
}

// ------------------------------------------------------------ randomness

fn random_bytes(buf: &mut [u8]) -> DbResult<()> {
    getrandom::fill(buf).map_err(|_| DbError::Random)
}

const PASSWORD_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
pub const PASSWORD_LEN: usize = 32;

/// CSPRNG password from `[A-Za-z0-9]` (safe in URLs, cfg files and SQL),
/// unbiased via rejection sampling.
pub fn generate_password() -> DbResult<Secret> {
    let n = PASSWORD_ALPHABET.len();
    let limit = 256 - (256 % n);
    let mut out = String::with_capacity(PASSWORD_LEN);
    let mut buf = [0u8; 64];
    while out.len() < PASSWORD_LEN {
        random_bytes(&mut buf)?;
        for &b in &buf {
            if (b as usize) < limit && out.len() < PASSWORD_LEN {
                out.push(PASSWORD_ALPHABET[b as usize % n] as char);
            }
        }
    }
    Ok(Secret(out))
}

fn random_hex(bytes: usize) -> DbResult<String> {
    let mut b = vec![0u8; bytes];
    random_bytes(&mut b)?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

pub const NAME_PREFIX: &str = "kora_";
const MAX_IDENT: usize = 64;

/// Generate a `kora_`-prefixed name that is not in `taken`.
pub fn generate_name(taken: &HashSet<String>) -> DbResult<String> {
    for _ in 0..32 {
        let n = format!("{NAME_PREFIX}{}", random_hex(4)?);
        if !taken.contains(&n) {
            return Ok(n);
        }
    }
    Err(DbError::Collision("could not find a free name".into()))
}

// ------------------------------------------------------------ SQL builders

/// Strict identifier check: `[A-Za-z0-9_]{1,64}`, not purely numeric.
pub fn validate_identifier(name: &str) -> DbResult<&str> {
    let ok = !name.is_empty()
        && name.len() <= MAX_IDENT
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && !name.bytes().all(|b| b.is_ascii_digit());
    if ok {
        Ok(name)
    } else {
        Err(DbError::InvalidIdentifier(format!("{name:?}")))
    }
}

/// Backtick-quoted, validated identifier.
pub fn quote_ident(name: &str) -> DbResult<String> {
    Ok(format!("`{}`", validate_identifier(name)?))
}

/// Single-quoted string literal. Rejects NUL/control characters; escapes
/// backslash and quote.
pub fn quote_string(s: &str) -> DbResult<String> {
    if s.chars().any(|c| c.is_control()) {
        return Err(DbError::InvalidValue("control characters not allowed".into()));
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        match c {
            '\'' => out.push_str("''"),
            '\\' => out.push_str("\\\\"),
            _ => out.push(c),
        }
    }
    out.push('\'');
    Ok(out)
}

/// Account host part: `%`, an IPv4/hostname made of `[A-Za-z0-9.%_-]`.
pub fn validate_host(host: &str) -> DbResult<&str> {
    let ok = !host.is_empty()
        && host.len() <= 60
        && host.bytes().all(|b| b.is_ascii_alphanumeric() || b".%_-".contains(&b));
    if ok {
        Ok(host)
    } else {
        Err(DbError::InvalidValue(format!("host {host:?}")))
    }
}

fn account(user: &str, host: &str) -> DbResult<String> {
    validate_identifier(user)?;
    Ok(format!("{}@{}", quote_string(user)?, quote_string(validate_host(host)?)?))
}

pub mod sql {
    use super::*;

    pub fn schema_exists(db: &str) -> DbResult<String> {
        Ok(format!(
            "SELECT COUNT(*) FROM information_schema.SCHEMATA WHERE SCHEMA_NAME = {};",
            quote_string(validate_identifier(db)?)?
        ))
    }
    pub fn user_exists(user: &str) -> DbResult<String> {
        Ok(format!(
            "SELECT COUNT(*) FROM mysql.user WHERE User = {};",
            quote_string(validate_identifier(user)?)?
        ))
    }
    /// Plain CREATE (no IF NOT EXISTS: an existing DB must be an error).
    pub fn create_database(db: &str) -> DbResult<String> {
        Ok(format!(
            "CREATE DATABASE {} CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;",
            quote_ident(db)?
        ))
    }
    pub fn create_user(user: &str, host: &str, password: &Secret) -> DbResult<String> {
        Ok(format!(
            "CREATE USER {} IDENTIFIED BY {};",
            account(user, host)?,
            quote_string(password.expose())?
        ))
    }
    /// Grant on exactly one database; never global or `*.*`.
    pub fn grant_all(db: &str, user: &str, host: &str) -> DbResult<String> {
        Ok(format!(
            "GRANT ALL PRIVILEGES ON {}.* TO {};",
            quote_ident(db)?,
            account(user, host)?
        ))
    }
    pub fn drop_database(db: &str) -> DbResult<String> {
        Ok(format!("DROP DATABASE IF EXISTS {};", quote_ident(db)?))
    }
    pub fn drop_user(user: &str, host: &str) -> DbResult<String> {
        Ok(format!("DROP USER IF EXISTS {};", account(user, host)?))
    }
}

// ------------------------------------------------------------- install plan

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepAction {
    Run(CommandSpec),
    Download { url: String, dest: PathBuf },
    /// Verify `file` against `sums` (a `sha256sums.txt`) entry `name`.
    VerifySha256 { file: PathBuf, sums: PathBuf, name: String },
    ExtractZip { archive: PathBuf, dest: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedStep {
    pub id: String,
    pub description: String,
    pub needs_admin: bool,
    pub action: StepAction,
}

#[derive(Debug, Clone)]
pub struct InstallPlan {
    pub steps: Vec<PlannedStep>,
    /// Windows only: unique service name used.
    pub service_name: Option<String>,
    /// Windows only: generated MariaDB root password (write to a 0600/ACL file).
    pub admin_password: Option<Secret>,
}

impl InstallPlan {
    pub fn needs_admin(&self) -> bool {
        self.steps.iter().any(|s| s.needs_admin)
    }
}

fn step(id: &str, description: &str, needs_admin: bool, action: StepAction) -> PlannedStep {
    PlannedStep { id: id.into(), description: description.into(), needs_admin, action }
}

/// Debian/Ubuntu: apt `mariadb-server`.
pub fn plan_install_linux() -> InstallPlan {
    let run = |p: &str, a: &[&str]| StepAction::Run(CommandSpec::new(p, a));
    InstallPlan {
        steps: vec![
            step("db.apt-update", "Refresh the package list", true, run("apt-get", &["update"])),
            step(
                "db.apt-install",
                "Install the MariaDB database server (mariadb-server)",
                true,
                run("apt-get", &["install", "-y", "mariadb-server"]),
            ),
            step(
                "db.enable",
                "Start MariaDB now and on boot",
                true,
                run("systemctl", &["enable", "--now", "mariadb"]),
            ),
        ],
        service_name: None,
        admin_password: None,
    }
}

/// Windows: official zip from archive.mariadb.org, SHA-256 verified, isolated
/// uniquely named service. `existing_services` guards against name collisions.
pub fn plan_install_windows(
    version: &str,
    root: &Path,
    port: u16,
    existing_services: &[String],
) -> DbResult<InstallPlan> {
    if version.is_empty() || !version.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return Err(DbError::InvalidValue(format!("MariaDB version {version:?}")));
    }
    let taken: HashSet<String> = existing_services.iter().map(|s| s.to_lowercase()).collect();
    let mut service = None;
    for _ in 0..32 {
        let n = format!("KoraMariaDB_{}", random_hex(4)?);
        if !taken.contains(&n.to_lowercase()) {
            service = Some(n);
            break;
        }
    }
    let service = service.ok_or_else(|| DbError::Collision("service name".into()))?;
    let password = generate_password()?;

    let zip_name = format!("mariadb-{version}-winx64.zip");
    let base = format!("https://archive.mariadb.org/mariadb-{version}/winx64-packages/");
    let archive = root.join("mariadb.download.zip");
    let sums = root.join("mariadb.sha256sums.txt");
    let dest = root.join("database");
    let data = root.join("database-data");
    let config = root.join("database-config.ini");
    let install_db = dest.join("bin").join("mariadb-install-db.exe");
    let s = |p: &Path| p.to_string_lossy().into_owned();

    let steps = vec![
        step("db.download", "Download the official MariaDB zip", false,
            StepAction::Download { url: format!("{base}{zip_name}"), dest: archive.clone() }),
        step("db.download-sums", "Download MariaDB's official SHA-256 list", false,
            StepAction::Download { url: format!("{base}sha256sums.txt"), dest: sums.clone() }),
        step("db.verify", "Verify the download's SHA-256 checksum", false,
            StepAction::VerifySha256 { file: archive.clone(), sums, name: zip_name }),
        step("db.extract", "Unpack MariaDB into the Kora folder", false,
            StepAction::ExtractZip { archive, dest }),
        step("db.initialise", &format!("Create the private MariaDB service '{service}' on 127.0.0.1:{port}"), true,
            StepAction::Run(CommandSpec {
                program: s(&install_db),
                args: vec![
                    format!("--datadir={}", s(&data)),
                    format!("--service={service}"),
                    format!("--password={}", password.expose()),
                    format!("--port={port}"),
                    format!("--config={}", s(&config)),
                ],
                ..CommandSpec::default()
            })),
        step("db.start", "Start the MariaDB service", true,
            StepAction::Run(CommandSpec::new("sc", &["start", &service]))),
    ];
    Ok(InstallPlan { steps, service_name: Some(service), admin_password: Some(password) })
}

pub fn plan_install(os: Os, root: &Path, version: &str, port: u16, existing: &[String]) -> DbResult<InstallPlan> {
    match os {
        Os::Linux => Ok(plan_install_linux()),
        Os::Windows => plan_install_windows(version, root, port, existing),
        Os::Other => Err(DbError::Unsupported("only Windows 10/11+ and Debian/Ubuntu are supported".into())),
    }
}

/// Check `file` bytes' SHA-256 against a `sha256sums.txt` entry for `name`.
pub fn verify_sha256(file: &Path, sums_text: &str, name: &str) -> DbResult<()> {
    use sha2::{Digest, Sha256};
    let expected = sums_text
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let h = it.next()?;
            let n = it.next()?.trim_start_matches('*');
            (n == name).then(|| h.to_lowercase())
        })
        .next()
        .ok_or_else(|| DbError::Checksum(format!("no checksum listed for {name}")))?;
    let mut hasher = Sha256::new();
    {
        use std::io::Read;
        let mut f = std::fs::File::open(file)?;
        let mut buf = [0u8; 65536];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
    }
    let actual: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if actual == expected {
        Ok(())
    } else {
        Err(DbError::Checksum("archive checksum mismatch".into()))
    }
}

// --------------------------------------------------------------- provision

#[derive(Clone, Serialize, Deserialize)]
pub struct AdminConn {
    pub host: String,
    pub port: u16,
    /// `None` = unix-socket root auth (run elevated), password otherwise.
    pub user: String,
    pub password: Option<Secret>,
    pub client: String,
}

impl fmt::Debug for AdminConn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AdminConn")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("password", &self.password)
            .finish()
    }
}

impl AdminConn {
    pub fn socket_root(client: &str) -> Self {
        Self { host: "127.0.0.1".into(), port: DEFAULT_PORT, user: "root".into(), password: None, client: client.into() }
    }

    pub(crate) fn command(&self, sql: &str) -> CommandSpec {
        let mut args = vec!["--batch".to_string(), "--skip-column-names".into(), "--connect-timeout=8".into(), "-u".into(), self.user.clone()];
        let mut env = Vec::new();
        match &self.password {
            Some(p) => {
                args.extend(["--protocol=tcp".into(), "-h".into(), self.host.clone(), "-P".into(), self.port.to_string()]);
                env.push(("MYSQL_PWD".to_string(), p.expose().to_string()));
            }
            // Windows has no unix socket: a login without password (e.g. the stock MySQL `root`) goes over TCP.
            None if cfg!(windows) => args.extend(["--protocol=tcp".into(), "-h".into(), self.host.clone(), "-P".into(), self.port.to_string()]),
            None => args.push("--protocol=socket".into()),
        }
        // SQL goes via stdin so it never appears in process listings.
        CommandSpec { program: self.client.clone(), args, env, stdin: Some(sql.to_string()) }
    }

    pub(crate) fn exec(&self, runner: &dyn CommandRunner, sql: &str) -> DbResult<String> {
        let out = runner.run(&self.command(sql))?;
        if out.ok() {
            Ok(out.stdout)
        } else {
            // stderr from the client can echo statements; keep it out of the error.
            Err(DbError::Command(format!("database client exited with status {}", out.status)))
        }
    }
}

/// What this run created; persisted so rollback (also after a crash) only
/// drops Kora's own objects.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProvisionState {
    pub database: String,
    pub user: String,
    pub host: String,
    pub created_database: bool,
    pub created_user: bool,
    pub granted: bool,
}

impl ProvisionState {
    pub fn save(&self, path: &Path) -> DbResult<()> {
        std::fs::write(path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
    pub fn load(path: &Path) -> DbResult<Self> {
        Ok(serde_json::from_slice(&std::fs::read(path)?)?)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    pub password: Secret,
}

fn pct(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

impl Credentials {
    /// oxmysql URI connection string.
    pub fn connection_string(&self) -> String {
        format!(
            "mysql://{}:{}@{}:{}/{}?charset=utf8mb4",
            pct(&self.user), pct(self.password.expose()), self.host, self.port, pct(&self.database)
        )
    }
    /// Line for server.cfg. Contains the password: write only to secrets.cfg.
    pub fn server_cfg_line(&self) -> String {
        format!("set mysql_connection_string \"{}\"\n", self.connection_string())
    }
}

/// Create a brand-new DB + user. Refuses (never reuses) existing names.
/// `state_path` is updated after every successful object creation.
pub fn provision(
    runner: &dyn CommandRunner,
    admin: &AdminConn,
    account_host: &str,
    conn_host: &str,
    state_path: &Path,
) -> DbResult<Credentials> {
    validate_host(account_host)?;
    // Pick names that don't exist yet.
    let mut taken = HashSet::new();
    let (db, user) = loop {
        let db = generate_name(&taken)?;
        let user = db.clone();
        let db_taken = admin.exec(runner, &sql::schema_exists(&db)?)?.trim() != "0";
        let user_taken = admin.exec(runner, &sql::user_exists(&user)?)?.trim() != "0";
        if !db_taken && !user_taken {
            break (db, user);
        }
        taken.insert(db);
        if taken.len() >= 16 {
            return Err(DbError::Collision("no free database/user name".into()));
        }
    };
    let password = generate_password()?;
    let mut state = ProvisionState {
        database: db.clone(),
        user: user.clone(),
        host: account_host.into(),
        ..Default::default()
    };
    state.save(state_path)?;

    admin.exec(runner, &sql::create_database(&db)?)?;
    state.created_database = true;
    state.save(state_path)?;

    admin.exec(runner, &sql::create_user(&user, account_host, &password)?)?;
    state.created_user = true;
    state.save(state_path)?;

    admin.exec(runner, &sql::grant_all(&db, &user, account_host)?)?;
    state.granted = true;
    state.save(state_path)?;

    Ok(Credentials { host: conn_host.into(), port: admin.port, database: db, user, password })
}

/// Drop only what `state` says this run created. Safe to call repeatedly.
pub fn rollback(runner: &dyn CommandRunner, admin: &AdminConn, state: &mut ProvisionState) -> DbResult<()> {
    if state.created_user {
        admin.exec(runner, &sql::drop_user(&state.user, &state.host)?)?;
        state.created_user = false;
        state.granted = false;
    }
    if state.created_database {
        admin.exec(runner, &sql::drop_database(&state.database)?)?;
        state.created_database = false;
    }
    Ok(())
}

// ------------------------------------------------------------ secrets file

/// icacls command restricting `path` to `windows_user` only.
pub fn windows_acl_command(path: &Path, windows_user: &str) -> DbResult<CommandSpec> {
    if windows_user.is_empty()
        || !windows_user.chars().all(|c| c.is_alphanumeric() || " ._-\\$".contains(c))
    {
        return Err(DbError::InvalidValue("windows user name".into()));
    }
    Ok(CommandSpec {
        program: "icacls".into(),
        args: vec![
            path.to_string_lossy().into_owned(),
            "/inheritance:r".into(),
            "/grant:r".into(),
            format!("{windows_user}:(R,W)"),
        ],
        ..CommandSpec::default()
    })
}

/// Write `contents` so only the owner can read it: 0600 on Unix; on Windows an
/// empty file is created, locked down with icacls, then filled.
pub fn write_secret_file(
    runner: &dyn CommandRunner,
    os: Os,
    path: &Path,
    contents: &str,
    windows_user: Option<&str>,
) -> DbResult<()> {
    use std::io::Write;
    if os == Os::Windows {
        let user = windows_user.ok_or_else(|| DbError::InvalidValue("windows user required".into()))?;
        std::fs::File::create(path)?;
        let out = runner.run(&windows_acl_command(path, user)?)?;
        if !out.ok() {
            let _ = std::fs::remove_file(path);
            return Err(DbError::Command("could not restrict file permissions".into()));
        }
        std::fs::write(path, contents)?;
        return Ok(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
        f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        f.write_all(contents.as_bytes())?;
    }
    #[cfg(not(unix))]
    {
        let mut f = std::fs::File::create(path)?;
        f.write_all(contents.as_bytes())?;
    }
    Ok(())
}

/// Write `database.json` (full credentials) and `secrets.cfg` (oxmysql line).
pub fn write_credentials(
    runner: &dyn CommandRunner,
    os: Os,
    dir: &Path,
    creds: &Credentials,
    windows_user: Option<&str>,
) -> DbResult<()> {
    write_secret_file(runner, os, &dir.join("database.json"), &serde_json::to_string_pretty(creds)?, windows_user)?;
    write_secret_file(runner, os, &dir.join("secrets.cfg"), &creds.server_cfg_line(), windows_user)
}

// ------------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// Fake runner: records calls, answers from a closure.
    struct Fake {
        calls: RefCell<Vec<CommandSpec>>,
        respond: Box<dyn Fn(&CommandSpec) -> CommandOutput>,
    }
    impl Fake {
        fn new(f: impl Fn(&CommandSpec) -> CommandOutput + 'static) -> Self {
            Self { calls: RefCell::new(vec![]), respond: Box::new(f) }
        }
        fn sqls(&self) -> Vec<String> {
            self.calls.borrow().iter().filter_map(|c| c.stdin.clone()).collect()
        }
    }
    impl CommandRunner for Fake {
        fn run(&self, spec: &CommandSpec) -> DbResult<CommandOutput> {
            self.calls.borrow_mut().push(spec.clone());
            Ok((self.respond)(spec))
        }
    }
    fn out(s: &str) -> CommandOutput {
        CommandOutput { status: 0, stdout: s.into(), stderr: String::new() }
    }
    fn db_fake() -> Fake {
        Fake::new(|c| match c.stdin.as_deref() {
            Some(s) if s.starts_with("SELECT") => out("0\n"),
            _ => out(""),
        })
    }

    #[test]
    fn names_prefixed_charset_safe_and_collision_checked() {
        let n = generate_name(&HashSet::new()).unwrap();
        assert!(n.starts_with("kora_"));
        assert!(validate_identifier(&n).is_ok());
        assert!(n.len() <= 64);
        let m = generate_name(&HashSet::new()).unwrap();
        assert_ne!(n, m);
    }

    #[test]
    fn password_rules() {
        let p = generate_password().unwrap();
        assert_eq!(p.expose().len(), PASSWORD_LEN);
        assert!(p.expose().bytes().all(|b| b.is_ascii_alphanumeric()));
        assert_ne!(p, generate_password().unwrap());
    }

    #[test]
    fn identifier_rejects_injection() {
        for bad in ["", "a`b", "a b", "x; DROP DATABASE y", "a-b", "123", "é", "a\0b", &"a".repeat(65)] {
            assert!(validate_identifier(bad).is_err(), "{bad:?}");
            assert!(sql::create_database(bad).is_err());
        }
        assert_eq!(quote_ident("kora_ab12").unwrap(), "`kora_ab12`");
    }

    #[test]
    fn string_quoting_escapes() {
        assert_eq!(quote_string("a'b\\c").unwrap(), "'a''b\\\\c'");
        assert!(quote_string("a\nb").is_err());
        assert!(quote_string("a\0b").is_err());
        let sql = sql::create_user("kora_x1", "127.0.0.1", &Secret::new("p'; DROP USER 'root")).unwrap();
        assert!(sql.contains("'p''; DROP USER ''root'"));
        assert!(sql::create_user("kora_x1", "h'ost", &Secret::new("x")).is_err());
    }

    #[test]
    fn grant_is_scoped_to_one_database() {
        let g = sql::grant_all("kora_ab", "kora_ab", "127.0.0.1").unwrap();
        assert_eq!(g, "GRANT ALL PRIVILEGES ON `kora_ab`.* TO 'kora_ab'@'127.0.0.1';");
        assert!(sql::grant_all("*", "u", "h").is_err());
        assert!(sql::create_database("kora_a").unwrap().contains("utf8mb4"));
    }

    #[test]
    fn redaction() {
        let creds = Credentials { host: "127.0.0.1".into(), port: 3306, database: "kora_a".into(), user: "kora_a".into(), password: Secret::new("hunter2hunter2") };
        assert!(!format!("{creds:?}").contains("hunter2"));
        let admin = AdminConn { host: "h".into(), port: 1, user: "root".into(), password: Some(Secret::new("rootpw123")), client: "mariadb".into() };
        assert!(!format!("{admin:?}").contains("rootpw123"));
        let spec = admin.command("CREATE USER 'a' IDENTIFIED BY 'sekret';");
        assert!(!format!("{spec:?}").contains("sekret"));
        assert!(!format!("{spec:?}").contains("rootpw123"));
        let plan = plan_install_windows("10.11.6", Path::new("C:\\k"), 3307, &[]).unwrap();
        let dbg = format!("{plan:?}");
        let pw = plan.admin_password.as_ref().unwrap().expose();
        assert!(!dbg.contains(pw));
        assert!(!plan.steps.iter().any(|s| s.description.contains(pw)));
    }

    #[test]
    fn connection_string_encodes() {
        let c = Credentials { host: "127.0.0.1".into(), port: 3306, database: "kora_a".into(), user: "kora_a".into(), password: Secret::new("p@ss/w:rd") };
        assert_eq!(c.connection_string(), "mysql://kora_a:p%40ss%2Fw%3Ard@127.0.0.1:3306/kora_a?charset=utf8mb4");
        assert!(c.server_cfg_line().starts_with("set mysql_connection_string \"mysql://"));
    }

    #[test]
    fn provision_creates_then_rollback_drops_only_created() {
        let dir = tempfile::tempdir().unwrap();
        let sp = dir.path().join("db-state.json");
        let admin = AdminConn::socket_root("mariadb");
        let fake = db_fake();
        let creds = provision(&fake, &admin, "127.0.0.1", "127.0.0.1", &sp).unwrap();
        let sqls = fake.sqls();
        assert!(sqls.iter().any(|s| s.starts_with("CREATE DATABASE")));
        assert!(sqls.iter().any(|s| s.starts_with("CREATE USER")));
        assert!(sqls.iter().any(|s| s.starts_with("GRANT") && s.contains(&format!("`{}`.*", creds.database))));
        let mut st = ProvisionState::load(&sp).unwrap();
        assert!(st.created_database && st.created_user && st.granted);

        let fake2 = db_fake();
        rollback(&fake2, &admin, &mut st).unwrap();
        let drops = fake2.sqls();
        assert_eq!(drops.len(), 2);
        assert!(drops.iter().all(|s| s.contains(&creds.database) || s.contains(&creds.user)));
        // second rollback is a no-op
        let fake3 = db_fake();
        rollback(&fake3, &admin, &mut st).unwrap();
        assert!(fake3.sqls().is_empty());
    }

    #[test]
    fn rollback_skips_objects_not_created() {
        let admin = AdminConn::socket_root("mariadb");
        // DB created, user creation never happened.
        let mut st = ProvisionState { database: "kora_a1".into(), user: "kora_a1".into(), host: "127.0.0.1".into(), created_database: true, ..Default::default() };
        let f = db_fake();
        rollback(&f, &admin, &mut st).unwrap();
        assert_eq!(f.sqls(), vec!["DROP DATABASE IF EXISTS `kora_a1`;".to_string()]);
        // Nothing created -> nothing dropped.
        let mut none = ProvisionState { database: "preexisting".into(), user: "root".into(), host: "%".into(), ..Default::default() };
        let f = db_fake();
        rollback(&f, &admin, &mut none).unwrap();
        assert!(f.calls.borrow().is_empty());
    }

    #[test]
    fn failed_create_user_rolls_back_only_database() {
        let dir = tempfile::tempdir().unwrap();
        let sp = dir.path().join("s.json");
        let admin = AdminConn::socket_root("mariadb");
        let f = Fake::new(|c| match c.stdin.as_deref() {
            Some(s) if s.starts_with("SELECT") => out("0"),
            Some(s) if s.starts_with("CREATE USER") => CommandOutput { status: 1, ..Default::default() },
            _ => out(""),
        });
        assert!(provision(&f, &admin, "127.0.0.1", "127.0.0.1", &sp).is_err());
        let mut st = ProvisionState::load(&sp).unwrap();
        assert!(st.created_database && !st.created_user);
        let f2 = db_fake();
        rollback(&f2, &admin, &mut st).unwrap();
        assert_eq!(f2.sqls().len(), 1);
        assert!(f2.sqls()[0].starts_with("DROP DATABASE"));
    }

    #[test]
    fn existing_names_are_never_reused() {
        let dir = tempfile::tempdir().unwrap();
        // Every name reports as taken -> collision error, nothing created.
        let f = Fake::new(|_| out("1"));
        let r = provision(&f, &AdminConn::socket_root("mariadb"), "127.0.0.1", "127.0.0.1", &dir.path().join("s.json"));
        assert!(matches!(r, Err(DbError::Collision(_))));
        assert!(!f.sqls().iter().any(|s| s.starts_with("CREATE")));
    }

    #[test]
    fn detect_linux_services() {
        let f = Fake::new(|c| match (c.program.as_str(), c.args.first().map(String::as_str)) {
            ("systemctl", Some("list-unit-files")) if c.args[1] == "mariadb.service" => out("mariadb.service enabled enabled\n"),
            ("systemctl", Some("list-unit-files")) => out(""),
            ("systemctl", Some("is-active")) => out("active\n"),
            ("mariadb", _) => out("10.11.6-MariaDB\t3307\t/run/mysqld/mysqld.sock\n"),
            _ => CommandOutput { status: 1, ..Default::default() },
        });
        let v = detect(&f, Os::Linux).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].name, "mariadb");
        assert!(v[0].running);
        assert_eq!(v[0].port, 3307);
        assert_eq!(v[0].version.as_deref(), Some("10.11.6-MariaDB"));
    }

    #[test]
    fn detect_windows_parse_and_version_parse() {
        let v = parse_windows_services("MariaDB|Running|\"C:\\m\\mysqld.exe\" --defaults-file=x --port=3310 MariaDB\nMySQL80|Stopped|\"C:\\x\"\n");
        assert_eq!(v.len(), 2);
        assert!(v[0].running && v[0].port == 3310);
        assert!(!v[1].running && v[1].port == 3306);
        assert_eq!(parse_version("mariadbd  Ver 10.11.6-MariaDB for debian-linux-gnu"), Some("10.11.6".into()));
    }

    #[test]
    fn install_plans() {
        let l = plan_install(Os::Linux, Path::new("/x"), "", 3306, &[]).unwrap();
        assert!(l.needs_admin());
        assert!(matches!(&l.steps[1].action, StepAction::Run(c) if c.args.contains(&"mariadb-server".to_string())));
        assert!(plan_install(Os::Other, Path::new("/x"), "", 1, &[]).is_err());

        let w = plan_install_windows("10.11.6", Path::new("C:\\k"), 3307, &["KoraMariaDB_x".into()]).unwrap();
        assert!(w.service_name.as_ref().unwrap().starts_with("KoraMariaDB_"));
        assert!(w.steps.iter().any(|s| matches!(s.action, StepAction::VerifySha256 { .. })));
        let verify = w.steps.iter().position(|s| matches!(s.action, StepAction::VerifySha256 { .. })).unwrap();
        let extract = w.steps.iter().position(|s| matches!(s.action, StepAction::ExtractZip { .. })).unwrap();
        assert!(verify < extract);
        assert!(plan_install_windows("1.0/../x", Path::new("C:\\k"), 1, &[]).is_err());
    }

    #[test]
    fn sha256_verification() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.zip");
        std::fs::write(&f, b"abc").unwrap();
        let good = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  a.zip\n";
        assert!(verify_sha256(&f, good, "a.zip").is_ok());
        assert!(verify_sha256(&f, &good.replace("ba78", "0000"), "a.zip").is_err());
        assert!(verify_sha256(&f, good, "b.zip").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn secret_file_is_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let creds = Credentials { host: "127.0.0.1".into(), port: 3306, database: "kora_a".into(), user: "kora_a".into(), password: Secret::new("pw") };
        write_credentials(&db_fake(), Os::Linux, dir.path(), &creds, None).unwrap();
        for n in ["database.json", "secrets.cfg"] {
            let m = std::fs::metadata(dir.path().join(n)).unwrap().permissions().mode();
            assert_eq!(m & 0o777, 0o600);
        }
    }

    #[test]
    fn windows_acl_planned() {
        let c = windows_acl_command(Path::new("C:\\k\\secrets.cfg"), "DOMAIN\\alice").unwrap();
        assert_eq!(c.program, "icacls");
        assert!(c.args.contains(&"/inheritance:r".to_string()));
        assert!(windows_acl_command(Path::new("x"), "a\" /grant Everyone:F").is_err());
        let dir = tempfile::tempdir().unwrap();
        let f = db_fake();
        write_secret_file(&f, Os::Windows, &dir.path().join("s.cfg"), "x", Some("alice")).unwrap();
        assert_eq!(f.calls.borrow()[0].program, "icacls");
    }
}
