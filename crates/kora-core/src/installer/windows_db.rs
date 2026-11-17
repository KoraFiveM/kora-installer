//! Windows: install a private MariaDB server for Kora, end to end.
//!
//! The *plan* lives in [`crate::db::plan_install_windows`] (official zip via MariaDB's mirror network with archive.mariadb.org
//! as fallback, SHA-256 verified against a hash pinned in this program, one uniquely named service). This module runs it:
//! download, verify, unpack (without the zip's top-level folder), `mariadb-install-db.exe` (data
//! folder + service + generated root password), loopback-only config, start, wait until it accepts
//! logins. Everything lives in one Kora-owned folder, `%ProgramData%\Kora\mariadb\<version>\`,
//! restricted to Administrators and SYSTEM. The admin login is saved in `database-admin.json`
//! (same folder rules) so later installs can create their NEW databases and `kora delete` can drop
//! them again. `mariadb.json` records what Kora created, which is all `kora uninstall
//! --delete-databases` ever removes.
//!
//! Pure helpers (paths, config text, zip unpacking, records) are unit-tested on every OS; only the
//! orchestration at the bottom needs a real Windows machine (the `windows-smoke` workflow runs it).

use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::{err, Problem, Result};
use crate::artifacts::ArtifactSource as _;
use crate::db::{self, AdminConn, CommandSpec, Secret, StepAction};

/// MariaDB LTS release Kora installs on Windows. 11.4.13 because MariaDB's download mirrors carry it (they lag the
/// archive by a release) and mirrors are ~300x faster than archive.mariadb.org.
pub const MARIADB_VERSION: &str = "11.4.13";

/// SHA-256 of `mariadb-11.4.13-winx64.zip`: the anchor of trust. Identical in archive.mariadb.org's `sha256sums.txt`
/// and in MariaDB's REST API listing. Whatever mirror delivers the file, it must hash to this.
pub const MARIADB_ZIP_SHA256: &str = "d62986d433eeebfde218560b276103831604a61e929e87f1a17f5aebd80257e2";

/// The pinned hash for `version`, if this build knows one.
pub fn pinned_sha256(version: &str) -> Option<&'static str> {
    (version == MARIADB_VERSION).then_some(MARIADB_ZIP_SHA256)
}

/// Machine-wide folder holding Kora's MariaDB (`%ProgramData%\Kora\mariadb`).
pub fn base_dir() -> Option<PathBuf> {
    base_dir_from(std::env::var_os("KORA_MARIADB_HOME"), std::env::var_os("ProgramData"))
}

/// `KORA_MARIADB_HOME` overrides (tests); otherwise `%ProgramData%\Kora\mariadb`.
pub fn base_dir_from(home_override: Option<std::ffi::OsString>, program_data: Option<std::ffi::OsString>) -> Option<PathBuf> {
    if let Some(h) = home_override.filter(|h| !h.is_empty()) {
        return Some(PathBuf::from(h));
    }
    program_data.filter(|p| !p.is_empty()).map(|p| PathBuf::from(p).join("Kora").join("mariadb"))
}

pub fn record_path(base: &Path) -> PathBuf {
    base.join("mariadb.json")
}

pub fn admin_path(base: &Path) -> PathBuf {
    base.join("database-admin.json")
}

/// What Kora created (no secrets).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub service: String,
    pub version: String,
    pub port: u16,
    /// `<base>\<version>`: program files + data; removed (only) by `kora uninstall --delete-databases`.
    pub root: PathBuf,
    /// The `mariadb.exe` client.
    pub client: PathBuf,
}

/// Admin login of the Kora-installed server.
#[derive(Clone, Serialize, Deserialize)]
pub struct SavedAdmin {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: Secret,
    pub client: PathBuf,
}

impl std::fmt::Debug for SavedAdmin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SavedAdmin").field("host", &self.host).field("port", &self.port).field("user", &self.user).finish_non_exhaustive()
    }
}

impl SavedAdmin {
    pub fn to_conn(&self) -> AdminConn {
        AdminConn { host: self.host.clone(), port: self.port, user: self.user.clone(), password: Some(self.password.clone()), client: self.client.to_string_lossy().into_owned() }
    }
}

pub fn load_record(base: &Path) -> Option<Record> {
    serde_json::from_slice(&std::fs::read(record_path(base)).ok()?).ok()
}

pub fn load_admin(base: &Path) -> Option<SavedAdmin> {
    serde_json::from_slice(&std::fs::read(admin_path(base)).ok()?).ok()
}

/// The admin login of Kora's own MariaDB, when there is one on this machine.
pub fn saved_admin() -> Option<SavedAdmin> {
    load_admin(&base_dir()?)
}

pub fn saved_record() -> Option<Record> {
    load_record(&base_dir()?)
}

/// `my.ini` text: Kora's server only listens on this machine.
pub fn config_text(datadir: &Path, port: u16) -> String {
    let fwd = |p: &Path| p.to_string_lossy().replace('\\', "/");
    format!("[mysqld]\ndatadir=\"{}\"\nport={port}\nbind-address=127.0.0.1\nskip-name-resolve\n", fwd(datadir))
}

/// Insert/replace the Kora lines in an existing `my.ini` produced by `mariadb-install-db.exe`.
pub fn harden_ini(existing: &str, port: u16) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut in_mysqld = false;
    let mut inserted = false;
    for line in existing.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_mysqld = t.eq_ignore_ascii_case("[mysqld]");
            out.push(line.to_string());
            if in_mysqld && !inserted {
                out.push("bind-address=127.0.0.1".into());
                out.push("skip-name-resolve".into());
                inserted = true;
            }
            continue;
        }
        let key = t.split('=').next().unwrap_or("").trim().to_ascii_lowercase();
        if in_mysqld && matches!(key.as_str(), "bind-address" | "skip-name-resolve") {
            continue;
        }
        out.push(line.to_string());
    }
    if !inserted {
        out.push("[mysqld]".into());
        out.push("bind-address=127.0.0.1".into());
        out.push("skip-name-resolve".into());
    }
    let _ = port;
    let mut s = out.join("\r\n");
    s.push_str("\r\n");
    s
}

/// Appended to errors where a virus scanner is the likely culprit (it holds or quarantines new .exe files).
pub const AV_HINT: &str =
    "Windows Security (or another antivirus) may have blocked or quarantined a file: open Windows Security > Virus & threat protection > Protection history, allow it, and run the install again.";

/// `my.ini` is read as ANSI by some MariaDB builds, so a folder with non-ASCII letters is refused up front.
pub fn check_ascii_path(p: &Path) -> Result<()> {
    if p.to_string_lossy().is_ascii() {
        Ok(())
    } else {
        err(format!("{} contains characters outside plain English letters, which MariaDB's configuration file cannot hold reliably. Set the KORA_MARIADB_HOME environment variable to a folder such as C:\\ProgramData\\Kora\\mariadb and run again.", p.display()))
    }
}

/// A free TCP port on loopback, starting at 3306.
pub fn pick_port() -> u16 {
    pick_port_with(&crate::firewall::ports::SystemProber, &crate::firewall::ports::registered_ports())
}

/// 3306 when free, else the next free port (never one another Kora server is registered on).
pub fn pick_port_with(prober: &dyn crate::firewall::ports::PortProber, avoid: &[u16]) -> u16 {
    crate::firewall::ports::suggest_free_port(3305, avoid, false, prober).unwrap_or(3306)
}

/// Unpack a MariaDB zip into `dest`, dropping the single top-level folder (`mariadb-x.y.z-winx64/`).
/// Rejects absolute paths, `..`, drive letters and backslashes; refuses to overwrite.
pub fn extract_zip_stripped(archive: &Path, dest: &Path) -> Result<()> {
    let io_err = |e: std::io::Error| Problem::generic(format!("cannot unpack MariaDB: {e}"));
    let mut z = zip::ZipArchive::new(std::io::BufReader::new(std::fs::File::open(archive).map_err(io_err)?)).map_err(|e| Problem::generic(format!("the MariaDB download is not a valid zip: {e}")))?;
    std::fs::create_dir_all(dest).map_err(io_err)?;
    let mut total: u64 = 0;
    for i in 0..z.len() {
        let mut f = z.by_index(i).map_err(|e| Problem::generic(format!("bad MariaDB zip entry: {e}")))?;
        let name = f.name().map_err(|e| Problem::generic(format!("bad MariaDB zip entry name: {e}")))?.into_owned();
        let Some(rel) = strip_first_component(&name)? else { continue };
        let out = dest.join(&rel);
        if f.is_dir() {
            std::fs::create_dir_all(&out).map_err(io_err)?;
            continue;
        }
        if let Some(p) = out.parent() {
            std::fs::create_dir_all(p).map_err(io_err)?;
        }
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&out).map_err(io_err)?;
        // Bounded while copying, so a zip bomb stops at the limit instead of filling the disk first.
        const LIMIT: u64 = 6 << 30;
        let left = LIMIT - total;
        let n = std::io::copy(&mut std::io::Read::take(&mut f, left + 1), &mut file).map_err(io_err)?;
        total += n;
        file.flush().map_err(io_err)?;
        if n > left {
            return err("the MariaDB zip unpacks to an unreasonable size");
        }
    }
    Ok(())
}

/// `top/a/b` -> `a/b`; `None` for the top folder itself. Errors on unsafe names.
pub fn strip_first_component(name: &str) -> Result<Option<PathBuf>> {
    if name.contains('\\') || name.contains(':') || name.starts_with('/') {
        return err(format!("unsafe path in the MariaDB zip: {name}"));
    }
    let mut comps = name.split('/').filter(|c| !c.is_empty());
    let _top = comps.next();
    let rest: Vec<&str> = comps.collect();
    if rest.is_empty() {
        return Ok(None);
    }
    let p: PathBuf = rest.iter().collect();
    if p.components().any(|c| !matches!(c, Component::Normal(_))) {
        return err(format!("unsafe path in the MariaDB zip: {name}"));
    }
    Ok(Some(p))
}

/// `icacls` arguments that leave `path` (a folder or file) to Administrators and SYSTEM only.
pub fn admin_only_acl_args(path: &Path, is_dir: bool) -> Vec<String> {
    let flags = if is_dir { "(OI)(CI)F" } else { "F" };
    vec![
        path.to_string_lossy().into_owned(),
        "/inheritance:r".into(),
        "/grant:r".into(),
        format!("*S-1-5-32-544:{flags}"), // BUILTIN\Administrators (SID: works on every Windows language)
        format!("*S-1-5-18:{flags}"),     // NT AUTHORITY\SYSTEM
    ]
}

fn run_checked(spec: &CommandSpec, what: &str) -> Result<db::CommandOutput> {
    let out = db::CommandRunner::run(&db::SystemRunner, spec).map_err(|e| Problem::generic(format!("{what}: {e}")))?;
    if !out.ok() {
        let detail = format!("{}{}", out.stderr, out.stdout);
        let first: Vec<&str> = detail.lines().filter(|l| !l.trim().is_empty()).take(4).collect();
        return err(format!("{what} failed (exit {}): {}", out.status, first.join(" | ")));
    }
    Ok(out)
}

/// Well-known SIDs that may own Kora's machine-wide folders: Administrators, SYSTEM, TrustedInstaller.
const TRUSTED_OWNERS: &[&str] = &["S-1-5-32-544", "S-1-5-18", "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464"];

/// PowerShell that prints the owner SID of `path` (a SID, so no language differences).
pub fn owner_sid_command(path: &Path) -> CommandSpec {
    let quoted = path.to_string_lossy().replace('\'', "''");
    CommandSpec::new(
        "powershell",
        &[
            "-NoProfile",
            "-NonInteractive",
            "-Command", // .NET directly, not Get-Acl: started from PowerShell 7, Windows PowerShell inherits a module
            // path it cannot load Microsoft.PowerShell.Security from.
            &format!("(New-Object System.Security.AccessControl.DirectorySecurity -ArgumentList '{quoted}', 'Owner').GetOwner([System.Security.Principal.SecurityIdentifier]).Value"),
        ],
    )
}

/// Make the folders that will hold Kora's MariaDB administrator-only, or refuse. `dirs` = the
/// folders from the outside in (`%ProgramData%\Kora`, then `...\mariadb`), already created.
/// Anyone may create folders in `%ProgramData%`, and whoever creates one owns it and can change its
/// permissions back after `icacls` restricted them. So each folder must be a real folder (not a
/// link) owned by Administrators / SYSTEM, checked AFTER it was created (a folder planted first is
/// caught, not overlooked); its owner is then set to Administrators explicitly. The innermost one
/// is also restricted to Administrators and SYSTEM.
pub fn secure_dirs(runner: &dyn db::CommandRunner, dirs: &[PathBuf]) -> Result<()> {
    for d in dirs {
        if crate::fsutil::is_symlink(d) {
            return err(format!("{} is a link, so Kora will not put the database server there. Delete it (as administrator) and try again.", d.display()));
        }
        let out = runner.run(&owner_sid_command(d)).map_err(|e| Problem::generic(format!("cannot read the owner of {}: {e}", d.display())))?;
        let sid = out.stdout.trim();
        if !out.ok() || sid.is_empty() {
            return err(format!("cannot read the owner of {} ({}).", d.display(), out.stderr.lines().next().unwrap_or("").trim()));
        }
        if !TRUSTED_OWNERS.contains(&sid) {
            return err(format!(
                "{} already exists and belongs to another account ({sid}), which could change it later. Kora will not put the database server there: delete that folder (as administrator) and try again.",
                d.display()
            ));
        }
        let own = CommandSpec::new("icacls", &[&d.to_string_lossy(), "/setowner", "*S-1-5-32-544"]);
        let o = runner.run(&own).map_err(|e| Problem::generic(format!("setting the owner of {}: {e}", d.display())))?;
        if !o.ok() {
            return err(format!("setting the owner of {} to Administrators failed: {}", d.display(), o.stdout.lines().next().unwrap_or("").trim()));
        }
    }
    if let Some(inner) = dirs.last() {
        let spec = CommandSpec { program: "icacls".into(), args: admin_only_acl_args(inner, true), ..CommandSpec::default() };
        let o = runner.run(&spec).map_err(|e| Problem::generic(format!("restricting the MariaDB folder to administrators: {e}")))?;
        if !o.ok() {
            return err(format!("restricting the MariaDB folder to administrators failed: {}", o.stdout.lines().next().unwrap_or("").trim()));
        }
    }
    Ok(())
}

/// `%ProgramData%\Kora` (when `base` is the default `...\Kora\mariadb`) and `base`, outside in.
fn base_and_parent(base: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(p) = base.parent().filter(|p| p.file_name().is_some_and(|n| n == "Kora")) {
        v.push(p.to_path_buf());
    }
    v.push(base.to_path_buf());
    v
}

pub(super) fn restrict(path: &Path, is_dir: bool, what: &str) -> Result<()> {
    let spec = CommandSpec { program: "icacls".into(), args: admin_only_acl_args(path, is_dir), ..CommandSpec::default() };
    run_checked(&spec, what).map(|_| ())
}

fn write_json_restricted<T: Serialize>(path: &Path, value: &T, secret: bool) -> Result<()> {
    let text = serde_json::to_string_pretty(value).map_err(|e| Problem::generic(e.to_string()))?;
    std::fs::File::create(path).map_err(|e| Problem::generic(format!("cannot write {}: {e}", path.display())))?;
    if secret {
        restrict(path, false, "restricting database-admin.json to administrators")?;
    }
    std::fs::write(path, text).map_err(|e| Problem::generic(format!("cannot write {}: {e}", path.display())))
}

/// Wait until `client` can log in as `admin` (a freshly started service needs a few seconds).
pub fn wait_for_login(admin: &AdminConn, timeout: Duration) -> Result<()> {
    let start = Instant::now();
    let mut last = String::new();
    while start.elapsed() < timeout {
        match db::CommandRunner::run(&db::SystemRunner, &admin.command("SELECT 1")) {
            Ok(o) if o.ok() => return Ok(()),
            Ok(o) => last = o.stderr.lines().next().unwrap_or("").trim().to_string(),
            Err(e) => last = e.to_string(),
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    err(format!("the MariaDB service did not accept logins within {} seconds ({last}). {AV_HINT}", timeout.as_secs()))
}

fn service_exists(name: &str) -> bool {
    crate::sysexe::command("sc.exe").args(["query", name]).output().map(|o| o.status.success()).unwrap_or(false)
}

pub fn list_service_names() -> Vec<String> {
    db::detect(&db::SystemRunner, db::Os::Windows).map(|v| v.into_iter().map(|s| s.name).collect()).unwrap_or_default()
}

/// Download + verify + unpack + initialise + start. Returns the saved record. Needs administrator rights.
/// `say` receives plain-language progress lines.
pub fn install(http: &super::HttpClient, say: &mut dyn FnMut(&str)) -> Result<Record> {
    let base = base_dir().ok_or_else(|| Problem::generic("cannot find %ProgramData% for the MariaDB folder"))?;
    check_ascii_path(&base)?;
    let root = base.join(MARIADB_VERSION);
    let port = pick_port();
    let plan = db::plan_install_windows(MARIADB_VERSION, &root, port, &list_service_names()).map_err(|e| Problem::generic(e.to_string()))?;
    let service = plan.service_name.clone().ok_or_else(|| Problem::generic("internal error: no service name planned"))?;
    let password = plan.admin_password.clone().ok_or_else(|| Problem::generic("internal error: no admin password planned"))?;
    if root.join("database").exists() || root.join("database-data").exists() {
        return err(format!("{} already holds a MariaDB folder that is not registered with Kora. Move or delete it (or install MariaDB yourself and use --db existing).", root.display()));
    }
    std::fs::create_dir_all(&root).map_err(|e| Problem::generic(format!("cannot create {}: {e}", root.display())))?;
    secure_dirs(&db::SystemRunner, &base_and_parent(&base))?;
    let client = root.join("database").join("bin").join("mariadb.exe");
    let datadir = root.join("database-data");

    // Everything below can be undone by deleting `root` and the service; remember the service first
    // so a failed attempt can be cleaned up by `cleanup_failed`.
    for s in &plan.steps {
        say(&s.description);
        match &s.action {
            StepAction::DownloadVerified { sources, dest, name, sums_url } => {
                // Pinned hash first; for a version without one, MariaDB's own list (never "no verification").
                let want = match pinned_sha256(MARIADB_VERSION) {
                    Some(h) => h.to_string(),
                    None => {
                        let list = http.fetch_text(sums_url).map_err(|e| Problem::generic(format!("cannot fetch MariaDB's checksum list: {e}")))?;
                        db::sha256_from_sums(&list, name).map_err(|e| Problem::generic(format!("MariaDB download rejected: {e}. Nothing was installed.")))?
                    }
                };
                let cfg = super::http::FetchConfig::default();
                let mut last = String::new();
                let used = {
                    let say = std::cell::RefCell::new(&mut *say);
                    let mut progress = |p: &super::http::FetchProgress| {
                        let l = p.line(&format!("MariaDB {MARIADB_VERSION}"));
                        if l != last {
                            (say.borrow_mut())(&l);
                            last = l;
                        }
                    };
                    let mut notes = |m: &str| (say.borrow_mut())(m);
                    http.download_verified(sources, dest, &want, &cfg, &mut progress, &mut notes)
                }
                .map_err(|e| Problem::generic(format!("MariaDB download failed: {e}")))?;
                say(&format!("Downloaded MariaDB {MARIADB_VERSION} from {} (SHA-256 verified)", used.split_once("://").map_or(used.as_str(), |(_, r)| r).split('/').next().unwrap_or("")));
            }
            StepAction::ExtractZip { archive, dest } => {
                extract_zip_stripped(archive, dest).map_err(|e| e.and_say(AV_HINT))?;
                let _ = std::fs::remove_file(archive);
                if !dest.join("bin").join("mariadb-install-db.exe").is_file() {
                    return err(format!("the MariaDB zip does not contain bin\\mariadb-install-db.exe. {AV_HINT}"));
                }
            }
            StepAction::Run(spec) if spec.program.ends_with("mariadb-install-db.exe") => {
                run_checked(spec, "creating the MariaDB data folder and service").map_err(|e| e.and_say(AV_HINT))?;
                // Loopback only, whatever install-db wrote.
                let p = datadir.join("my.ini");
                let text = std::fs::read_to_string(&p).unwrap_or_default();
                let new = if text.trim().is_empty() { config_text(&datadir, port) } else { harden_ini(&text, port) };
                std::fs::write(&p, new).map_err(|e| Problem::generic(format!("cannot write {}: {e}", p.display())))?;
                // Save the login right away: if a later step fails the service is still reachable.
                let saved = SavedAdmin { host: "127.0.0.1".into(), port, user: "root".into(), password: password.clone(), client: client.clone() };
                write_json_restricted(&admin_path(&base), &saved, true)?;
                let rec = Record { service: service.clone(), version: MARIADB_VERSION.into(), port, root: root.clone(), client: client.clone() };
                write_json_restricted(&record_path(&base), &rec, false)?;
            }
            StepAction::Run(spec) => {
                // `sc start <service>`
                run_checked(spec, "starting the MariaDB service")?;
            }
        }
    }
    let rec = load_record(&base).ok_or_else(|| Problem::generic("the MariaDB record was not written"))?;
    let saved = load_admin(&base).ok_or_else(|| Problem::generic("the MariaDB admin file was not written"))?;
    say("Waiting for MariaDB to accept connections");
    wait_for_login(&saved.to_conn(), Duration::from_secs(90))?;
    Ok(rec)
}

/// Make sure Kora's MariaDB (if one was installed earlier) is running; used before provisioning.
pub fn ensure_running(rec: &Record) -> Result<()> {
    let q = crate::sysexe::command("sc.exe").args(["query", &rec.service]).output().map_err(|e| Problem::generic(format!("sc.exe: {e}")))?;
    let text = String::from_utf8_lossy(&q.stdout).to_uppercase();
    if text.contains("RUNNING") {
        return Ok(());
    }
    if !q.status.success() {
        return err(format!("Kora's MariaDB service '{}' no longer exists. Remove {} and run the install again.", rec.service, rec.root.display()));
    }
    run_checked(&CommandSpec::new("sc.exe", &["start", &rec.service]), "starting the MariaDB service")?;
    Ok(())
}

/// Stop + delete the service and delete the folder. Idempotent; the caller has checked admin rights.
pub fn remove(rec: &Record, base: &Path, say: &mut dyn FnMut(&str)) -> Result<()> {
    if service_exists(&rec.service) {
        say(&format!("Stopping and removing the MariaDB service {}", rec.service));
        let _ = db::CommandRunner::run(&db::SystemRunner, &CommandSpec::new("sc.exe", &["stop", &rec.service]));
        wait_service_stopped(&rec.service, Duration::from_secs(60));
        run_checked(&CommandSpec::new("sc.exe", &["delete", &rec.service]), "removing the MariaDB service")?;
    }
    say(&format!("Deleting {}", rec.root.display()));
    remove_dir_retry(&rec.root)?;
    let _ = std::fs::remove_file(admin_path(base));
    let _ = std::fs::remove_file(record_path(base));
    let _ = std::fs::remove_dir(base);
    Ok(())
}

fn wait_service_stopped(name: &str, timeout: Duration) {
    let start = Instant::now();
    while start.elapsed() < timeout {
        let out = crate::sysexe::command("sc.exe").args(["query", name]).output();
        match out {
            Ok(o) if String::from_utf8_lossy(&o.stdout).to_uppercase().contains("STOPPED") => return,
            Ok(o) if !o.status.success() => return,
            _ => std::thread::sleep(Duration::from_secs(1)),
        }
    }
}

/// `remove_dir_all` that retries: Windows keeps files locked for a moment after a process exits.
pub fn remove_dir_retry(p: &Path) -> Result<()> {
    let mut last = String::new();
    for _ in 0..20 {
        if !p.exists() {
            return Ok(());
        }
        match std::fs::remove_dir_all(p) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last = e.to_string();
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    }
    err(format!("cannot delete {}: {last}", p.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_dir_prefers_override_then_programdata() {
        assert_eq!(base_dir_from(Some("X:\\m".into()), Some("C:\\ProgramData".into())), Some(PathBuf::from("X:\\m")));
        assert_eq!(base_dir_from(None, Some("C:\\ProgramData".into())), Some(PathBuf::from("C:\\ProgramData").join("Kora").join("mariadb")));
        assert_eq!(base_dir_from(Some("".into()), None), None);
    }

    #[test]
    fn ini_is_loopback_only_and_idempotent() {
        let t = config_text(Path::new("C:\\ProgramData\\Kora\\data"), 3307);
        assert!(t.contains("bind-address=127.0.0.1") && t.contains("port=3307") && t.contains("datadir=\"C:/ProgramData/Kora/data\""));
        let existing = "[mysqld]\r\ndatadir=C:/x\r\nport=3307\r\nbind-address=0.0.0.0\r\n[client]\r\nport=3307\r\n";
        let h = harden_ini(existing, 3307);
        assert_eq!(h.matches("bind-address").count(), 1);
        assert!(h.contains("bind-address=127.0.0.1") && !h.contains("0.0.0.0"));
        assert!(h.contains("[client]") && h.contains("datadir=C:/x"));
        assert_eq!(harden_ini(&h, 3307), h, "running it twice changes nothing");
        let none = harden_ini("[client]\r\nport=1\r\n", 1);
        assert!(none.contains("[mysqld]") && none.contains("bind-address=127.0.0.1"));
    }

    #[test]
    fn zip_strips_the_top_folder_and_refuses_unsafe_names() {
        assert_eq!(strip_first_component("mariadb-1/bin/a.exe").unwrap(), Some(PathBuf::from("bin").join("a.exe")));
        assert_eq!(strip_first_component("mariadb-1/").unwrap(), None);
        assert_eq!(strip_first_component("mariadb-1").unwrap(), None);
        for bad in ["/etc/x", "a/../../x", "a\\b", "C:/x", "a/b:c"] {
            assert!(strip_first_component(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn extracting_a_zip_drops_the_top_folder() {
        use std::io::Write as _;
        let dir = tempfile::tempdir().unwrap();
        let zp = dir.path().join("m.zip");
        {
            let mut w = zip::ZipWriter::new(std::fs::File::create(&zp).unwrap());
            let o = zip::write::SimpleFileOptions::default();
            w.add_directory("mariadb-1/", o).unwrap();
            w.start_file("mariadb-1/bin/mariadb-install-db.exe", o).unwrap();
            w.write_all(b"x").unwrap();
            w.start_file("mariadb-1/share/a.txt", o).unwrap();
            w.write_all(b"y").unwrap();
            w.finish().unwrap();
        }
        let dest = dir.path().join("database");
        extract_zip_stripped(&zp, &dest).unwrap();
        assert!(dest.join("bin").join("mariadb-install-db.exe").is_file());
        assert!(dest.join("share").join("a.txt").is_file());
        assert!(!dest.join("mariadb-1").exists());
    }

    struct Owners(std::cell::RefCell<Vec<String>>, &'static str);
    impl db::CommandRunner for Owners {
        fn run(&self, spec: &CommandSpec) -> db::DbResult<db::CommandOutput> {
            self.0.borrow_mut().push(format!("{} {}", spec.program, spec.args.join(" ")));
            let stdout = if spec.program == "powershell" { format!("{}\r\n", self.1) } else { String::new() };
            Ok(db::CommandOutput { status: 0, stdout, stderr: String::new() })
        }
    }

    #[test]
    fn a_folder_planted_by_another_account_is_refused_and_ours_is_taken_over() {
        let dirs = base_and_parent(&PathBuf::from("C:\\ProgramData").join("Kora").join("mariadb"));
        assert_eq!(dirs.len(), 2, "%ProgramData%\\Kora is checked too: {dirs:?}");
        // Owned by an ordinary user (they created it before Kora did): nothing is changed.
        let r = Owners(Default::default(), "S-1-5-21-1-2-3-1001");
        let e = secure_dirs(&r, &dirs).unwrap_err().to_string();
        assert!(e.contains("belongs to another account") && e.contains("S-1-5-21-1-2-3-1001"), "{e}");
        assert!(!r.0.borrow().iter().any(|c| c.starts_with("icacls")), "{:?}", r.0.borrow());
        // Owned by Administrators: owner set explicitly, the inner folder restricted.
        let r = Owners(Default::default(), "S-1-5-32-544");
        secure_dirs(&r, &dirs).unwrap();
        let calls = r.0.borrow();
        assert_eq!(calls.iter().filter(|c| c.contains("/setowner *S-1-5-32-544")).count(), 2, "{calls:?}");
        assert!(calls.last().unwrap().contains("/inheritance:r") && calls.last().unwrap().contains("mariadb"));
        // A path with a quote cannot break out of the PowerShell string.
        let c = owner_sid_command(Path::new("C:\\a'b"));
        assert!(c.args.last().unwrap().contains("'C:\\a''b'"));
        // An override folder (KORA_MARIADB_HOME) is checked on its own.
        assert_eq!(base_and_parent(Path::new("D:\\db")), vec![PathBuf::from("D:\\db")]);
    }

    #[test]
    fn acl_uses_well_known_sids_and_drops_inheritance() {
        let a = admin_only_acl_args(Path::new("C:\\ProgramData\\Kora\\mariadb"), true);
        assert_eq!(a[1], "/inheritance:r");
        assert!(a.contains(&"*S-1-5-32-544:(OI)(CI)F".to_string()) && a.contains(&"*S-1-5-18:(OI)(CI)F".to_string()));
        assert!(!a.iter().any(|x| x.contains("Users") || x.contains("Everyone")));
        let f = admin_only_acl_args(Path::new("x.json"), false);
        assert!(f.contains(&"*S-1-5-18:F".to_string()));
    }

    #[test]
    fn records_round_trip_and_the_password_never_prints() {
        let dir = tempfile::tempdir().unwrap();
        let a = SavedAdmin { host: "127.0.0.1".into(), port: 3307, user: "root".into(), password: Secret::new("hunter2hunter2"), client: PathBuf::from("C:\\m\\mariadb.exe") };
        std::fs::write(admin_path(dir.path()), serde_json::to_vec(&a).unwrap()).unwrap();
        let back = load_admin(dir.path()).unwrap();
        assert_eq!(back.port, 3307);
        assert!(!format!("{back:?}").contains("hunter2"));
        let c = back.to_conn();
        assert_eq!(c.client, "C:\\m\\mariadb.exe");
        assert!(c.password.is_some());
        assert!(load_record(dir.path()).is_none());
    }

    #[test]
    fn non_ascii_mariadb_folders_are_refused_and_ascii_ones_pass() {
        assert!(check_ascii_path(Path::new("C:\\ProgramData\\Kora\\mariadb")).is_ok());
        let e = check_ascii_path(Path::new("C:\\Users\\Zo\u{eb} M\u{fc}ller\\mariadb")).unwrap_err().to_string();
        assert!(e.contains("KORA_MARIADB_HOME"), "{e}");
    }

    struct Busy(Vec<u16>);
    impl crate::firewall::ports::PortProber for Busy {
        fn tcp_free(&self, p: u16) -> bool {
            !self.0.contains(&p)
        }
        fn udp_free(&self, _: u16) -> bool {
            true
        }
    }

    #[test]
    fn busy_3306_moves_kora_mariadb_to_the_next_free_port() {
        assert_eq!(pick_port_with(&Busy(vec![]), &[]), 3306);
        assert_eq!(pick_port_with(&Busy(vec![3306]), &[]), 3307);
        assert_eq!(pick_port_with(&Busy(vec![3306, 3307]), &[3308]), 3309);
    }

    #[test]
    fn pick_port_returns_a_bindable_port() {
        let p = pick_port();
        assert!((3306..3400).contains(&p));
        let held = std::net::TcpListener::bind(("127.0.0.1", p)).unwrap();
        assert_ne!(pick_port(), p, "a port in use is skipped");
        drop(held);
    }

    #[test]
    fn plan_runs_in_a_kora_owned_folder_with_unique_service() {
        let base = Path::new("C:\\ProgramData\\Kora\\mariadb");
        let root = base.join(MARIADB_VERSION);
        let p = db::plan_install_windows(MARIADB_VERSION, &root, 3307, &["KoraMariaDB_aaaa".into()]).unwrap();
        let svc = p.service_name.unwrap();
        assert!(svc.starts_with("KoraMariaDB_") && svc != "KoraMariaDB_aaaa");
        assert!(p.steps.iter().all(|s| match &s.action {
            StepAction::DownloadVerified { dest, sources, .. } =>
                dest.starts_with(&root) && sources.iter().all(|u| u.starts_with("https://")) && sources[0].contains("downloads.mariadb.org") && sources[1].contains("archive.mariadb.org"),
            StepAction::ExtractZip { dest, .. } => dest.starts_with(&root),
            _ => true,
        }));
        // `--config` must name an EXISTING file for mariadb-install-db (real run: FATAL ERROR); never pass it.
        assert!(p.steps.iter().all(|s| match &s.action {
            StepAction::Run(c) => !c.args.iter().any(|a| a.starts_with("--config")),
            _ => true,
        }));
    }
}
