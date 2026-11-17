//! The Linux service: a Unix socket, the caller's uid from `SO_PEERCRED`, and
//! [`super::dispatch::handle`] for every request. Started by `kora.service` (`kora service serve`) as
//! root from the packaged program (`/usr/lib/kora`, `/opt/kora`).
//!
//! * One request per connection (the framing of [`crate::helper`]: one JSON line in; `progress` lines
//!   and exactly one `result` (or `error`) out). The step's `KORA-ADMIN-RESULT` line becomes the
//!   result's `data`.
//! * The settings file is re-read on every connection, so `kora service allow|deny` takes effect at
//!   once; it must be root-owned and not writable by others, else nobody but root is served.
//! * `ping` answers whether THIS caller may use the service (`ok`), without running anything.
//! * The socket is `0666`: the uid check is the control, and a refused user gets a clear reason
//!   instead of "permission denied".

use std::io::{self, BufReader, Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::dispatch::{Host, InstallRecord, Progress, Ran, RunSpec};
use super::op::MAX_LINE;
use super::paths;
use super::policy::{AccountId, Caller, Policy};
use crate::helper::winhelper::Ledger;
use crate::helper::PING;

const MAX_CONNECTIONS: usize = 16;

pub use super::conn::Svc;

/// The caller from the connection (never from the request).
pub fn caller_of(s: &UnixStream) -> Caller {
    match crate::helper::server::peer_uid_of(s) {
        Ok(0) => Caller { id: Some(AccountId::uid(0)), integrity: None, is_root: true, remote: false },
        Ok(u) => Caller { id: Some(AccountId::uid(u)), integrity: None, is_root: false, remote: false },
        Err(_) => Caller { id: None, integrity: None, is_root: false, remote: false },
    }
}

/// One connection: caller already identified (see [`caller_of`]).
pub fn handle_connection(svc: &dyn Svc, stream: UnixStream, caller: Caller) {
    let Ok(mut out) = stream.try_clone() else { return };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut reader = BufReader::new((&stream).take(MAX_LINE as u64 + 1));
    super::conn::handle_stream(svc, &mut reader, &mut out, caller);
}

/// Accept until `stop` is set.
pub fn serve(svc: Arc<dyn Svc>, listener: UnixListener, stop: &AtomicBool) {
    let active = Arc::new(AtomicUsize::new(0));
    let _ = listener.set_nonblocking(true);
    while !stop.load(Ordering::Relaxed) {
        // An update was installed: leave once the answers in flight are sent (systemd starts the new program).
        if svc.restart_requested() && active.load(Ordering::SeqCst) == 0 {
            break;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                if active.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                    active.fetch_sub(1, Ordering::SeqCst);
                    continue;
                }
                let (svc, active) = (Arc::clone(&svc), Arc::clone(&active));
                std::thread::spawn(move || {
                    let _ = stream.set_nonblocking(false);
                    let caller = caller_of(&stream);
                    handle_connection(&*svc, stream, caller);
                    active.fetch_sub(1, Ordering::SeqCst);
                });
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(25)),
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    }
}

/// Bind the socket (replacing a stale file), mode 0666.
pub fn bind(path: &Path) -> io::Result<UnixListener> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let _ = std::fs::remove_file(path);
    let l = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666))?;
    Ok(l)
}

// ---- the real service ----------------------------------------------------------------------

/// Where the real service keeps its files (tests use a temporary folder).
#[derive(Debug, Clone)]
pub struct Files {
    pub policy: PathBuf,
    pub ledger: PathBuf,
    pub audit: PathBuf,
}

impl Files {
    pub fn system() -> Files {
        Files { policy: paths::LINUX_POLICY.into(), ledger: paths::LINUX_LEDGER.into(), audit: paths::LINUX_AUDIT.into() }
    }
}

pub struct LinuxSvc {
    pub exe: PathBuf,
    pub files: Files,
    /// Require root-owned files (false only in tests).
    pub strict_owner: bool,
}

/// Open `path` for reading without following a link at the last component.
fn open_nofollow(path: &Path) -> io::Result<std::fs::File> {
    std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(path)
}

fn read_capped(path: &Path) -> Option<Vec<u8>> {
    let mut f = open_nofollow(path).ok()?;
    if !f.metadata().ok()?.is_file() {
        return None;
    }
    let mut v = Vec::new();
    (&mut f).take(1024 * 1024).read_to_end(&mut v).ok()?;
    Some(v)
}

/// Is every part of `dir` below the first part a user owns free of links? Links made by root before
/// that point (`/home` -> `/var/home`) are the administrator's own and fine.
pub fn path_has_user_link(dir: &Path) -> io::Result<bool> {
    let mut cur = PathBuf::new();
    let mut user_territory = false;
    for c in dir.components() {
        cur.push(c);
        let m = std::fs::symlink_metadata(&cur)?;
        if m.file_type().is_symlink() {
            if m.uid() != 0 || user_territory {
                return Ok(true);
            }
            // a root-made link on the way: follow it to keep checking what is below it
            cur = std::fs::canonicalize(&cur)?;
            continue;
        }
        if m.uid() != 0 {
            user_territory = true;
        }
    }
    Ok(false)
}

#[derive(serde::Deserialize)]
struct RecordPorts {
    game: u16,
    txadmin: u16,
}
#[derive(serde::Deserialize)]
struct RecordFile {
    name: String,
    ports: RecordPorts,
}
#[derive(serde::Deserialize)]
struct DbFile {
    database: String,
}

impl Host for LinuxSvc {
    fn now(&self) -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
    }
    fn account_name(&self, id: &AccountId) -> String {
        id.as_str().strip_prefix("uid:").and_then(|n| n.parse().ok()).and_then(crate::helper::install::lookup_account).map(|a| a.name).unwrap_or_default()
    }
    fn folder_owner(&self, dir: &str) -> Result<Option<AccountId>, String> {
        let p = Path::new(dir);
        match path_has_user_link(p) {
            Ok(false) => {}
            Ok(true) => return Err("a link is on the way".into()),
            Err(e) => return Err(format!("cannot inspect it: {e}")),
        }
        let m = std::fs::symlink_metadata(p).map_err(|e| format!("cannot inspect it: {e}"))?;
        if !m.is_dir() {
            return Err("it is not a folder".into());
        }
        Ok(Some(AccountId::uid(m.uid())))
    }
    fn is_server_account(&self, id: &AccountId) -> bool {
        let Some(a) = id.as_str().strip_prefix("uid:").and_then(|n| n.parse().ok()).and_then(crate::helper::install::lookup_account) else { return false };
        (a.name == "kora" || a.name.starts_with("kora-")) && a.uid != 0 && a.uid < crate::service::account::uid_min()
    }
    fn install_record(&self, dir: &str) -> Option<InstallRecord> {
        let b = read_capped(&Path::new(dir).join(".kora").join("request.json"))?;
        let r: RecordFile = serde_json::from_slice(&b).ok()?;
        Some(InstallRecord { name: r.name, game_port: r.ports.game, txadmin_port: r.ports.txadmin })
    }
    fn db_name(&self, dir: &str) -> Option<String> {
        let b = read_capped(&Path::new(dir).join(".kora").join("db-provision.json"))?;
        serde_json::from_slice::<DbFile>(&b).ok().map(|d| d.database)
    }
    fn load_ledger(&self) -> Result<Ledger, String> {
        super::ownership::load(&self.files.ledger)
    }
    fn save_ledger(&self, l: &Ledger) -> Result<(), String> {
        l.save(&self.files.ledger).map_err(|e| e.to_string())
    }
    fn audit(&self, line: &str) {
        eprintln!("{line}"); // the journal
        if let Some(d) = self.files.audit.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(m) = std::fs::metadata(&self.files.audit) {
            if super::audit::needs_trim(m.len()) {
                if let Ok(t) = std::fs::read_to_string(&self.files.audit) {
                    let _ = std::fs::write(&self.files.audit, super::audit::trimmed(&t));
                }
            }
        }
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).mode(0o640).open(&self.files.audit) {
            let _ = writeln!(f, "{line}");
        }
    }
    fn own_version(&self) -> String {
        env!("CARGO_PKG_VERSION").into()
    }
}

impl Svc for LinuxSvc {
    fn host(&self) -> &dyn Host {
        self
    }

    fn policy(&self) -> Result<Policy, String> {
        let p = &self.files.policy;
        let m = std::fs::symlink_metadata(p).map_err(|e| format!("cannot read {}: {e}", p.display()))?;
        if self.strict_owner && (m.file_type().is_symlink() || paths::writable_by_others(m.uid(), m.mode())) {
            return Err(format!("{} can be changed by someone other than root", p.display()));
        }
        let b = read_capped(p).ok_or_else(|| format!("cannot read {}", p.display()))?;
        Policy::parse(&String::from_utf8_lossy(&b))
    }

    fn run(&self, caller: &Caller, spec: &RunSpec<'_>, progress: Progress<'_>, cancel: &Arc<AtomicBool>) -> io::Result<Ran> {
        if spec.argv.get(1).map(String::as_str) == Some("self-update") {
            return self.run_update(caller, spec, progress, cancel);
        }
        self.run_child(caller, spec, progress, cancel)
    }
}

impl LinuxSvc {
    /// `UpdateApply` (see [`super::update::run_update`]).
    fn run_update(&self, caller: &Caller, spec: &RunSpec<'_>, progress: Progress<'_>, cancel: &Arc<AtomicBool>) -> io::Result<Ran> {
        let dir = self.files.ledger.parent().ok_or_else(|| io::Error::other("no data folder"))?;
        super::update::run_update(&self.exe, dir, progress, &mut |p| self.run_child(caller, spec, p, cancel))
    }

    fn run_child(&self, caller: &Caller, spec: &RunSpec<'_>, progress: Progress<'_>, cancel: &Arc<AtomicBool>) -> io::Result<Ran> {
        use crate::helper::worker::{ChildExecutor, Executor};
        let uid = caller.id.as_ref().and_then(|i| i.as_str().strip_prefix("uid:")).and_then(|n| n.parse::<u32>().ok()).ok_or_else(|| io::Error::other("no caller"))?;
        let acct = crate::helper::install::lookup_account(uid).ok_or_else(|| io::Error::other("the caller's account has no passwd entry"))?;
        let mut env = paths::linux_child_env(&acct.name, acct.uid, acct.gid, &acct.home, None);
        env.extend(spec.env.iter().map(|(k, v)| (k.clone(), v.clone())));
        let mut ex = ChildExecutor::new(self.exe.clone());
        ex.limit = spec.limit;
        ex.cancelled = Arc::clone(cancel);
        let mut data = None;
        let r = ex.run(spec.argv, &env, &mut |stream, line| {
            if stream == "stdout" {
                if let Some(v) = crate::helper::worker::result_line(line) {
                    data = Some(v);
                    return;
                }
            }
            progress(stream, line);
        });
        match r {
            Ok(code) => Ok(Ran { code, timed_out: false, data }),
            Err(e) if e.kind() == io::ErrorKind::TimedOut => Ok(Ran { code: -1, timed_out: true, data: None }),
            Err(e) => Err(e),
        }
    }
}

/// `kora service serve`: refuses to start unless it is root and runs from the protected folder.
pub fn run_service() -> Result<(), String> {
    if !crate::elevate::is_elevated() {
        return Err("the Kora service must run as root (it is started by kora.service)".into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    check_program_trusted(&exe)?;
    // An update that was just installed: count this start, roll back when the new program keeps failing.
    let paths = super::update::Paths::in_dir(Path::new(paths::LINUX_LEDGER).parent().unwrap_or(Path::new("/var/lib/kora")));
    let (boot, said) = super::update::on_start(&paths, &exe, env!("CARGO_PKG_VERSION"));
    if let Some(s) = said {
        eprintln!("kora service: {s}");
    }
    match boot {
        super::update::Boot::RolledBack => return Err("rolled back to the previous program; exiting so it is started".into()),
        super::update::Boot::Probation(m) => {
            let p = paths.clone();
            std::thread::spawn(move || {
                std::thread::sleep(super::update::COMMIT_AFTER);
                super::update::commit(&p, &m);
            });
        }
        super::update::Boot::Normal => {}
    }
    let svc = Arc::new(LinuxSvc { exe, files: Files::system(), strict_owner: true });
    let l = bind(Path::new(paths::LINUX_SOCKET)).map_err(|e| format!("cannot open {}: {e}", paths::LINUX_SOCKET))?;
    let stop = AtomicBool::new(false);
    serve(svc, l, &stop);
    Ok(())
}

/// The program (and every folder above it) must be root-owned and not writable by others, and be
/// in the packaged location.
pub fn check_program_trusted(exe: &Path) -> Result<(), String> {
    if !paths::program_location_ok(&exe.to_string_lossy(), false, None) {
        return Err(format!("{} is not in Kora's protected program folder ({}), so the service will not run it", exe.display(), paths::LINUX_PROGRAM_DIRS.join(" or ")));
    }
    let mut p = Some(exe);
    while let Some(path) = p {
        let m = std::fs::metadata(path).map_err(|e| format!("cannot inspect {}: {e}", path.display()))?;
        if paths::writable_by_others(m.uid(), m.mode()) {
            return Err(format!("{} can be changed by someone other than root, so the service will not run from it", path.display()));
        }
        p = path.parent().filter(|q| !q.as_os_str().is_empty());
    }
    Ok(())
}

// ---- client (the panel and `kora service status`) ------------------------------------------------

pub use super::conn::{Answer, ClientError};

/// Talk to the service on `path`. Before sending anything the socket must be owned by root (the
/// panel never hands a licence key to a socket another user made).
pub fn call(path: &Path, verb: &str, args: &[String], env: &[(String, String)], on_line: &mut dyn FnMut(&str)) -> Result<Answer, ClientError> {
    call_with(path, verb, args, env, on_line, true)
}

pub fn call_with(path: &Path, verb: &str, args: &[String], env: &[(String, String)], on_line: &mut dyn FnMut(&str), require_root_owner: bool) -> Result<Answer, ClientError> {
    let m = std::fs::symlink_metadata(path).map_err(|e| ClientError::Unavailable(format!("Kora's administrator service is not running ({e}).")))?;
    if require_root_owner && !paths::socket_owner_ok(m.uid()) {
        return Err(ClientError::Unavailable("That socket does not belong to Kora's administrator service (it is not owned by root), so nothing was sent to it.".into()));
    }
    let s = UnixStream::connect(path).map_err(|e| ClientError::Unavailable(format!("Kora's administrator service is not running ({e}).")))?;
    let mut w = s.try_clone().map_err(|e| ClientError::Broken(e.to_string()))?;
    super::conn::converse(&mut BufReader::new(s), &mut w, verb, args, env, on_line)
}

/// Is the service there, and does it serve this account? `Ok(version)` or the reason.
pub fn probe(path: &Path) -> Result<String, ClientError> {
    super::conn::interpret_ping(call(path, PING, &[], &[], &mut |_| {}))
}

#[cfg(test)]
mod tests;
