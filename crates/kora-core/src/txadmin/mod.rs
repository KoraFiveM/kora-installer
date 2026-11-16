//! txAdmin writer: pure generators for `server.cfg`, `permissions.cfg`, `secrets.cfg`, plus
//! the txAdmin profile/data-dir writer. Reference: old Go `cfg()`, `native.go` (`TXHOST_*`) and
//! the txAdmin recipe (`server.cfg`, `permissions.cfg`).
//!
//! Security model: every value is validated *before* it reaches a cfg line. Free text may not
//! contain control characters (so no newline injection), `"`, `\` or `;`; identifiers and
//! resource names use strict allow-lists; the DB password is percent-encoded inside the
//! connection string. Secrets (license key, DB connection string) are emitted **only** into
//! `secrets.cfg` (mode 0600), which `server.cfg` pulls in with `exec secrets.cfg`.
//!
//! Verified against the real txAdmin **8.1.2** bundled in FXServer artifact 38621
//! (`alpine/opt/cfx-server/citizen/system_resources/monitor/`; the server code is the single
//! bundle `core/index.js`, so line numbers are not meaningful: facts cite the minified symbol and
//! `docs/env-config.md`). Facts used here:
//!
//! * **Data layout.** `TXHOST_DATA_PATH` = txData dir (`txHostConfig.dataPath`). The profile dir is
//!   `<txData>/<profile>` (`profilePath = path.join(dataPath, profileName)`, profile `default`) and
//!   holds `config.json`. **`admins.json` is NOT per profile**: `AdminStore.adminsFile =
//!   txHostConfig.dataSubPath("admins.json")` = `<txData>/admins.json`.
//! * **config.json** (`ConfigStore`, `CONFIG_VERSION = 2`): `{"version":2,"server":{"dataPath",
//!   "cfgPath","startupArgs","onesync","autoStart","quiet","shutdownNoticeDelayMs",
//!   "restartSpawnDelayMs"}}`. `onesync` is one of `on|legacy|off`; `autoStart` defaults to `true`;
//!   `quiet` defaults to `false`; `cfgPath` defaults to `server.cfg`. The server counts as
//!   "configured" (`FxRunner.isConfigured`) iff `server.dataPath` and `server.cfgPath` are
//!   non-empty strings. Unknown keys are kept (we merge).
//! * **Autostart gate** (`FxRunner.signalStartReady`): returns unless `server.autoStart`; warns
//!   "open txAdmin ... to configure" unless configured; warns "The server will not auto start
//!   because there are no admins configured." unless `adminStore.hasAdmins()`. So a fresh install
//!   needs `admins.json` (or `TXHOST_DEFAULT_ACCOUNT`) *before* first boot.
//! * **admins.json** (`AdminStore.loadAdminsFile`): a JSON array that must be non-empty; every item
//!   needs `name` (string, >= 3 chars), `master` (bool), `password_hash` (string starting with
//!   `$2`, i.e. bcrypt - **a master without a hash is rejected**), `providers` (object; allowed
//!   keys `discord`/`citizenfx`; each needs `id` string >= 3 chars, `identifier` string >= 3
//!   chars, `data` object) and `permissions` (array). Exactly one item must have `master:true`.
//!   `$schema` (`ADMIN_SCHEMA_VERSION = 1`) is optional. txAdmin's own `createAdminsFile` writes the
//!   master with `permissions: []` (the master implicitly has everything) and, when no password is
//!   given, a hash of a random throw-away string plus `password_temporary: true`; we do the same.
//!   Name rule for accounts is `^\w[\w.-]{1,18}\w$` (`regexValidFivemUsername`, 3-20 chars).
//! * **TXHOST_\*** (`hostEnvVarSchemas`, `docs/env-config.md`): present in this version:
//!   `DATA_PATH`, `GAME_NAME` (`fivem|redm`), `MAX_SLOTS`, `QUIET_MODE`, `API_TOKEN`, `TXA_URL`,
//!   `TXA_PORT` (default 40120, may not be 30120), `FXS_PORT`, `INTERFACE`, `PROVIDER_NAME`,
//!   `PROVIDER_LOGO`, `DEFAULT_DBHOST|DBPORT|DBUSER|DBPASS|DBNAME`, `DEFAULT_CFXKEY`,
//!   `DEFAULT_ACCOUNT` (`username:fivemId[:bcrypt]`, only consulted when `admins.json` does not
//!   exist yet), `IGNORE_DEPRECATED_CONFIGS`. Values must not be quoted.
//! * **Interface binding caveat.** `TXHOST_INTERFACE` makes txAdmin listen on that address **and**
//!   `server.cfg` validation (`endpoint_add_*` check) rejects any endpoint whose interface differs
//!   ("the `endpoint_add_tcp` interface MUST be ..."), and the deployer rewrites endpoints to it. So
//!   `127.0.0.1` would also make the *game* port local-only: txAdmin 8.1.2 has **no** way to bind
//!   only the panel to loopback. We therefore do not set `TXHOST_INTERFACE`; "private txAdmin"
//!   means its port is never opened in the firewall (see `InstallRequest::txadmin_public`).

use serde_json::{json, Map, Value};
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum TxError {
    #[error("invalid {field}: {reason}")]
    Invalid { field: &'static str, reason: String },
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, TxError>;

fn invalid<T>(field: &'static str, reason: impl Into<String>) -> Result<T> {
    Err(TxError::Invalid { field, reason: reason.into() })
}

/// A secret that never prints in `Debug`.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(s: impl Into<String>) -> Self {
        Secret(s.into())
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

#[derive(Debug, Clone)]
pub struct DbConnection {
    pub user: String,
    pub password: Secret,
    pub host: String,
    pub port: u16,
    pub database: String,
}

/// Linked Cfx.re identity used for the txAdmin master account.
#[derive(Debug, Clone)]
pub struct CfxIdentity {
    /// Numeric Cfx.re account id (the `fivem:<id>` identifier).
    pub fivem_id: u64,
    /// Cfx.re username (letters, digits, `_ . -`).
    pub username: String,
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Server name shown in logs/hostname; free text, validated.
    pub name: String,
    pub game_port: u16,
    pub txadmin_port: u16,
    pub max_clients: u16,
    pub game_build: u32,
    pub license_key: Secret,
    pub db: DbConnection,
    pub master: Option<CfxIdentity>,
    /// Ordered resource names / `[category]` folders, written as `ensure` lines in this order.
    pub ensure: Vec<String>,
}

fn free_text(field: &'static str, v: &str, max: usize) -> Result<()> {
    if v.trim().is_empty() {
        return invalid(field, "must not be empty");
    }
    if v.chars().count() > max {
        return invalid(field, format!("longer than {max} characters"));
    }
    if let Some(c) = v.chars().find(|c| c.is_control() || matches!(c, '"' | '\\' | ';' | '\u{2028}' | '\u{2029}')) {
        return invalid(field, format!("contains disallowed character {c:?}"));
    }
    Ok(())
}

fn ident(field: &'static str, v: &str, max: usize) -> Result<()> {
    if v.is_empty() || v.len() > max || !v.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-')) {
        return invalid(field, "only letters, digits, '_', '.', '-' allowed");
    }
    Ok(())
}

fn resource_name(v: &str) -> Result<()> {
    let core = v.strip_prefix('[').and_then(|s| s.strip_suffix(']')).unwrap_or(v);
    if core.is_empty() || v.len() > 100 || !core.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-')) {
        return invalid("ensure", format!("bad resource name {v:?}"));
    }
    Ok(())
}

impl ServerConfig {
    /// Validate everything; called by every generator.
    pub fn validate(&self) -> Result<()> {
        free_text("name", &self.name, 100)?;
        for (f, p) in [("game_port", self.game_port), ("txadmin_port", self.txadmin_port)] {
            if p < 1024 {
                return invalid(f, "must be 1024-65535");
            }
        }
        if self.game_port == self.txadmin_port {
            return invalid("txadmin_port", "must differ from game_port");
        }
        if !(1..=2048).contains(&self.max_clients) {
            return invalid("max_clients", "must be 1-2048");
        }
        if self.game_build == 0 || self.game_build > 99_999 {
            return invalid("game_build", "out of range");
        }
        let k = self.license_key.expose();
        if k.len() < 8 || k.len() > 64 || !k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
            return invalid("license_key", "unexpected format (letters, digits, '-', '_')");
        }
        self.db.validate()?;
        if let Some(m) = &self.master {
            m.validate()?;
        }
        for r in &self.ensure {
            resource_name(r)?;
        }
        Ok(())
    }
}

impl CfxIdentity {
    fn validate(&self) -> Result<()> {
        if self.fivem_id == 0 || self.fivem_id > 999_999_999_999 {
            return invalid("master.fivem_id", "out of range");
        }
        tx_username("master.username", &self.username)
    }
}

fn pct(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}

impl DbConnection {
    fn validate(&self) -> Result<()> {
        ident("db.user", &self.user, 32)?;
        ident("db.database", &self.database, 64)?;
        let h = &self.host;
        if h.is_empty() || h.len() > 253 || !h.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':')) {
            return invalid("db.host", "only hostname/IP characters allowed");
        }
        if self.port == 0 {
            return invalid("db.port", "must be non-zero");
        }
        let p = self.password.expose();
        if p.is_empty() || p.len() > 256 || p.chars().any(|c| c.is_control()) {
            return invalid("db.password", "empty, too long or contains control characters");
        }
        Ok(())
    }

    /// `mysql://user:pass@host:port/db?charset=utf8mb4` (same shape as the Go `start.py`).
    /// Everything user-controlled is percent-encoded, so the result is safe inside cfg quotes.
    pub fn connection_string(&self) -> Result<String> {
        self.validate()?;
        let host = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        Ok(format!("mysql://{}:{}@{}:{}/{}?charset=utf8mb4", pct(&self.user), pct(self.password.expose()), host, self.port, pct(&self.database)))
    }
}

/// Secret-free `server.cfg`. Pulls secrets via `exec secrets.cfg`.
pub fn server_cfg(c: &ServerConfig) -> Result<String> {
    c.validate()?;
    let mut s = String::new();
    s.push_str("# Kora Framework server configuration. Generated by the Kora installer.\n");
    s.push_str("# Secrets (license key, database credentials) live in secrets.cfg only.\n");
    s.push_str("exec secrets.cfg\n\n");
    s.push_str(&format!("endpoint_add_tcp \"0.0.0.0:{}\"\n", c.game_port));
    s.push_str(&format!("endpoint_add_udp \"0.0.0.0:{}\"\n", c.game_port));
    s.push_str(&format!("sv_maxclients {}\n", c.max_clients));
    s.push_str(&format!("sv_hostname \"{}\"\n", c.name));
    s.push_str(&format!("sets sv_projectName \"{}\"\n", c.name));
    s.push_str(&format!("sv_enforceGameBuild {}\n", c.game_build));
    s.push_str("set steam_webApiKey \"none\"\n");
    s.push_str("set resources_useSystemChat true\nset chat_showJoins false\nset chat_showQuits false\n");
    s.push_str("setr qb_locale \"en\"\nsv_scriptHookAllowed 0\n\n");
    s.push_str("exec permissions.cfg\n\n");
    let mut seen = std::collections::HashSet::new();
    for r in &c.ensure {
        if seen.insert(r.as_str()) {
            s.push_str(&format!("ensure {r}\n"));
        }
    }
    Ok(s)
}

/// `secrets.cfg`: license key and DB connection string. Never log or commit this.
pub fn secrets_cfg(c: &ServerConfig) -> Result<String> {
    c.validate()?;
    let conn = c.db.connection_string()?;
    Ok(format!(
        "# Kora secrets. Restricted permissions, git-ignored, never share.\nsv_licenseKey \"{}\"\nset mysql_connection_string \"{}\"\n",
        c.license_key.expose(),
        conn
    ))
}

/// `permissions.cfg`: the recipe's ACEs plus the master admin principals.
pub fn permissions_cfg(master: Option<&CfxIdentity>) -> Result<String> {
    let mut s = String::from(PERMISSIONS_BASE);
    s.push_str("\n## txAdmin master admin (from the linked Cfx.re account)\n");
    match master {
        Some(m) => {
            m.validate()?;
            s.push_str(&format!("# {}\nadd_principal identifier.fivem:{} kora_core.god\nadd_principal identifier.fivem:{} group.admin\n", m.username, m.fivem_id, m.fivem_id));
        }
        None => s.push_str("# No master identity configured.\n"),
    }
    Ok(s)
}

const PERMISSIONS_BASE: &str = "## Permissions
add_ace group.admin command allow
add_ace group.admin command.zoneeditor allow
add_ace group.admin kora_core.admin allow
add_ace group.admin kora_lib.config.edit allow

## Resource permissions
add_ace resource.kora_core command allow

## Gods
add_ace kora_core.god command allow

## Inheritance
add_principal kora_core.god group.admin
add_principal kora_core.god kora_core.admin
add_principal kora_core.admin kora_core.mod

## QBCore backwards compatibility: principals granted the stock qbcore.* levels keep working
add_principal qbcore.god kora_core.god
add_principal qbcore.admin kora_core.admin
add_principal qbcore.mod kora_core.mod
";

/// Lines to add to the server's `.gitignore`.
pub const GITIGNORE_ENTRIES: &[&str] = &["secrets.cfg", "database.json", "txData/"];

/// Environment for the txAdmin/FXServer process (non-secret).
pub fn txadmin_env(data_dir: &Path, txadmin_port: u16) -> Vec<(String, String)> {
    vec![
        ("TXHOST_DATA_PATH".into(), data_dir.display().to_string()),
        ("TXHOST_TXA_PORT".into(), txadmin_port.to_string()),
        ("TXHOST_GAME_NAME".into(), "fivem".into()),
    ]
}

/// Write `contents` atomically (temp + rename); `secret` files are created 0600 on unix and, on
/// Windows, get their inherited permissions removed (only this user, Administrators and SYSTEM).
pub fn write_file(path: &Path, contents: &str, secret: bool) -> Result<()> {
    // Root writes these into a folder the server's account can change (a re-run install): never
    // through a planted link.
    if cfg!(unix) {
        crate::fsutil::write_no_follow(path, contents.as_bytes(), if secret { 0o600 } else { 0o644 })?;
        return Ok(());
    }
    if let Some(p) = path.parent() {
        fs::create_dir_all(p)?;
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.tmp"));
    let _ = fs::remove_file(&tmp);
    let mut o = fs::OpenOptions::new();
    o.write(true).create_new(true);
    let mut f = o.open(&tmp)?;
    // Windows: lock the (still empty) file down before the secret is written into it.
    if secret && cfg!(windows) {
        if let Err(e) = restrict_secret_file(&tmp) {
            drop(f);
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
    }
    f.write_all(contents.as_bytes())?;
    f.sync_all()?;
    drop(f);
    crate::fsutil::rename_retrying(&tmp, path)?;
    Ok(())
}

/// `icacls` arguments that leave `path` to `user`, Administrators and SYSTEM (well-known SIDs, so any
/// Windows language works) and drop the permissions inherited from the folder (which include `Users`).
pub fn secret_acl_args(path: &Path, user: &str) -> Vec<String> {
    vec![
        path.to_string_lossy().into_owned(),
        "/inheritance:r".into(),
        "/grant:r".into(),
        format!("{user}:(F)"),
        "*S-1-5-18:(F)".into(),
        "*S-1-5-32-544:(F)".into(),
    ]
}

/// The signed-in account as `DOMAIN\name` (what icacls accepts).
fn current_account() -> Option<String> {
    let name = std::env::var("USERNAME").ok().filter(|n| !n.is_empty())?;
    let ok = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || " ._-$".contains(c));
    if !ok(&name) {
        return None;
    }
    match std::env::var("USERDOMAIN").ok().filter(|d| ok(d)) {
        Some(d) => Some(format!("{d}\\{name}")),
        None => Some(name),
    }
}

fn restrict_secret_file(path: &Path) -> Result<()> {
    let user = current_account().ok_or_else(|| TxError::Invalid { field: "account", reason: "cannot tell which Windows account is running (USERNAME)".into() })?;
    let out = crate::sysexe::command("icacls").args(secret_acl_args(path, &user)).output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(TxError::Invalid { field: "permissions", reason: format!("could not restrict permissions on {}: {}", path.display(), String::from_utf8_lossy(&out.stdout).trim()) })
    }
}

/// Write `server.cfg`, `permissions.cfg` and `secrets.cfg` into the FXServer data dir.
/// Everything is generated (and validated) before anything touches disk.
pub fn write_server_files(server_data: &Path, c: &ServerConfig) -> Result<()> {
    let (s, p, k) = (server_cfg(c)?, permissions_cfg(c.master.as_ref())?, secrets_cfg(c)?);
    write_file(&server_data.join("secrets.cfg"), &k, true)?;
    write_file(&server_data.join("server.cfg"), &s, false)?;
    write_file(&server_data.join("permissions.cfg"), &p, false)
}

/// Create/update the txAdmin profile at `<txdata>/<profile>/config.json` (unknown keys preserved)
/// so txAdmin considers the server configured and auto-starts it. Returns the profile dir.
/// See the module docs for the verified key set.
pub fn write_profile(txdata: &Path, profile: &str, server_data: &Path, cfg_path: &str) -> Result<PathBuf> {
    ident("profile", profile, 32)?;
    free_text("cfg_path", cfg_path, 200)?;
    let dir = txdata.join(profile);
    fs::create_dir_all(&dir)?;
    let cfg_file = dir.join("config.json");
    let mut cfg: Map<String, Value> = match fs::read(&cfg_file) {
        Ok(b) => serde_json::from_slice(&b)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Map::new(),
        Err(e) => return Err(e.into()),
    };
    cfg.insert("version".into(), json!(2));
    let mut server = match cfg.remove("server") {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    };
    server.insert("dataPath".into(), json!(server_data.display().to_string()));
    server.insert("cfgPath".into(), json!(cfg_path));
    server.insert("onesync".into(), json!("on"));
    server.insert("autoStart".into(), json!(true));
    cfg.insert("server".into(), Value::Object(server));
    write_file(&cfg_file, &serde_json::to_string_pretty(&cfg)?, false)?;
    Ok(dir)
}

/// The txAdmin master account to pre-seed so the server auto-starts on first boot.
#[derive(Debug, Clone)]
pub enum MasterSeed {
    /// Linked Cfx.re account: logs in with the Cfx.re button. The stored bcrypt hash is of a random
    /// throw-away string (txAdmin does the same), so no password exists.
    Cfx(CfxIdentity),
    /// Local-only account with a generated password (written to `secrets/txadmin-admin.txt`).
    Local { username: String, password: Secret },
}

/// Username of the generated local admin.
pub const LOCAL_ADMIN_USER: &str = "kora-admin";

/// txAdmin account-name rule: `^\w[\w.-]{1,18}\w$` (`regexValidFivemUsername`).
fn tx_username(field: &'static str, v: &str) -> Result<()> {
    let b = v.as_bytes();
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    if !(3..=20).contains(&b.len()) || !word(b[0]) || !word(b[b.len() - 1]) || !b.iter().all(|&c| word(c) || c == b'.' || c == b'-') {
        return invalid(field, "3-20 characters: letters, digits, '_', '.', '-' (no leading/trailing '.' or '-')");
    }
    Ok(())
}

/// Random alphanumeric string of `len` chars from the OS RNG (rejection sampling, no bias).
pub fn random_token(len: usize) -> Result<String> {
    const A: &[u8] = b"abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut out = String::with_capacity(len);
    while out.len() < len {
        let mut buf = [0u8; 32];
        getrandom::fill(&mut buf).map_err(|e| TxError::Io(std::io::Error::other(e.to_string())))?;
        out.extend(buf.iter().filter(|&&b| (b as usize) < A.len() * 4).map(|&b| A[b as usize % A.len()] as char).take(len - out.len()));
    }
    Ok(out)
}

/// bcrypt (cost 10, `$2b$`) hash as txAdmin expects in `password_hash`.
pub fn hash_password(pw: &Secret) -> Result<String> {
    let p = pw.expose();
    if !(6..=128).contains(&p.chars().count()) || p.len() > 72 {
        return invalid("password", "txAdmin needs 6-128 characters (bcrypt uses the first 72 bytes)");
    }
    bcrypt::hash(p, 10).map_err(|e| TxError::Io(std::io::Error::other(e.to_string())))
}

/// Pure generator for `<txData>/admins.json` (format verified against txAdmin 8.1.2, see module
/// docs). `hash` is the bcrypt hash to store; kept separate so output can be golden-tested.
pub fn admins_json(seed: &MasterSeed, hash: &str, temporary: bool) -> Result<String> {
    if !hash.starts_with("$2") {
        return invalid("password_hash", "must be a bcrypt hash");
    }
    let (name, providers) = match seed {
        MasterSeed::Cfx(m) => {
            m.validate()?;
            (m.username.clone(), json!({ "citizenfx": { "id": m.username, "identifier": format!("fivem:{}", m.fivem_id), "data": {} } }))
        }
        MasterSeed::Local { username, .. } => {
            tx_username("master.username", username)?;
            (username.clone(), json!({}))
        }
    };
    let mut admin = Map::new();
    admin.insert("$schema".into(), json!(1));
    admin.insert("name".into(), json!(name));
    admin.insert("master".into(), json!(true));
    admin.insert("password_hash".into(), json!(hash));
    if temporary {
        admin.insert("password_temporary".into(), json!(true));
    }
    admin.insert("providers".into(), providers);
    admin.insert("permissions".into(), json!([]));
    Ok(serde_json::to_string_pretty(&[Value::Object(admin)])? + "\n")
}

/// Pre-seed `<txdata>/admins.json` with the master account. **Never touches an existing file**
/// (txAdmin requires exactly one master and would reject a merge mistake); returns whether it wrote.
pub fn write_admins(txdata: &Path, seed: &MasterSeed) -> Result<bool> {
    let file = txdata.join("admins.json");
    if file.exists() {
        return Ok(false);
    }
    let (hash, temporary) = match seed {
        MasterSeed::Cfx(_) => (hash_password(&Secret::new(random_token(40)?))?, true),
        MasterSeed::Local { password, .. } => (hash_password(password)?, false),
    };
    let json = admins_json(seed, &hash, temporary)?;
    write_file(&file, &json, true)?;
    Ok(true)
}

/// Text for `secrets/txadmin-admin.txt` (generated local admin).
pub fn local_admin_note(username: &str, password: &Secret, txadmin_port: u16) -> String {
    format!(
        "txAdmin master account created by the Kora installer\nURL:      http://127.0.0.1:{txadmin_port}/ (use an SSH tunnel for remote access)\nUsername: {username}\nPassword: {}\n\nChange the password after the first login (txAdmin > Admin Manager). Keep this file private.\n",
        password.expose()
    )
}

#[cfg(test)]
mod tests;
