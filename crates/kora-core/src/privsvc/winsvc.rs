//! The Windows service `KoraService` and its named pipe `\\.\pipe\kora-service`. Thin: every decision
//! is in [`super::dispatch`] / [`super::policy`] / [`super::ownership`] (tested on every OS); this
//! file calls Windows. Not run in this repository's unit tests (they run on Linux); the Windows
//! smoke job installs the MSI and drives all of this for real.
//!
//! * **Pipe.** Created by the service as SYSTEM with the descriptor from [`policy::pipe_sddl`], rebuilt
//!   from the settings every time the next instance is made. The first instance carries
//!   `FILE_FLAG_FIRST_PIPE_INSTANCE` (a squatter makes the service refuse to start instead of
//!   sharing the name); remote clients are rejected; the listed accounts get only the specific
//!   rights read data, write data and read attributes: **not** `FILE_CREATE_PIPE_INSTANCE`
//!   (`FILE_APPEND_DATA`, part of `GENERIC_WRITE`), so no allowed account can make a look-alike
//!   instance that panels would connect to. Clients must ask for exactly those rights
//!   ([`CLIENT_ACCESS`]).
//! * **Identity.** After the request line has been read (Windows requires that), the service
//!   impersonates the client on that thread for the one call that reads its token (user SID,
//!   integrity level) and reverts at once. The client opens the pipe at identification level, so the
//!   service could not act as them anyway.
//! * **Program.** Children are the protected `kora.exe`, started with an environment built from
//!   nothing ([`crate::helper::worker::elevated_child_env`]) for the caller's account, whose profile
//!   folder the service looks up itself ([`crate::helper::winpipe::profile_dir`]).
//! * **Client.** The panel checks that the pipe's owner is SYSTEM before sending a byte
//!   ([`crate::helper::winhelper::check_server_owner`]): a program of an allowed account that made a
//!   pipe of its own is owned by that account.

use std::collections::HashMap;
use std::ffi::{c_void, OsString};
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{FromRawHandle, RawHandle};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use windows_service::service::{ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::{define_windows_service, service_dispatcher};

use super::conn::{self, Answer, ClientError, Svc};
use super::dispatch::{Host, InstallRecord, Progress, Ran, RunSpec};
use super::op::MAX_LINE;
use super::paths::{self, WinFiles};
use super::policy::{self, AccountId, Caller, Policy};
use crate::helper::winhelper::Ledger;

type Handle = *mut c_void;
const INVALID: Handle = -1isize as Handle;
const NAME: &str = paths::WINDOWS_SERVICE;
const MAX_CONNECTIONS: usize = 16;
/// `FILE_READ_DATA | FILE_WRITE_DATA | FILE_READ_ATTRIBUTES | READ_CONTROL | SYNCHRONIZE`: what a client
/// is granted and what it must ask for. Deliberately without `FILE_APPEND_DATA` (= create a pipe instance).
pub const CLIENT_ACCESS: u32 = 0x0012_0083;

#[repr(C)]
struct SecurityAttributes {
    n_length: u32,
    descriptor: *mut c_void,
    inherit: i32,
}

#[repr(C)]
struct SidAndAttributes {
    sid: *mut c_void,
    attributes: u32,
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateNamedPipeW(name: *const u16, open_mode: u32, pipe_mode: u32, max: u32, out_size: u32, in_size: u32, timeout: u32, sa: *const SecurityAttributes) -> Handle;
    fn ConnectNamedPipe(pipe: Handle, overlapped: *mut c_void) -> i32;
    fn DisconnectNamedPipe(pipe: Handle) -> i32;
    fn FlushFileBuffers(h: Handle) -> i32;
    fn WaitNamedPipeW(name: *const u16, timeout: u32) -> i32;
    fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *const SecurityAttributes, disposition: u32, flags: u32, template: Handle) -> Handle;
    fn CloseHandle(h: Handle) -> i32;
    fn GetLastError() -> u32;
    fn LocalFree(p: *mut c_void) -> *mut c_void;
    fn GetCurrentThread() -> Handle;
}

#[link(name = "advapi32")]
extern "system" {
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl: *const u16, revision: u32, sd: *mut *mut c_void, size: *mut u32) -> i32;
    fn ConvertSidToStringSidW(sid: *mut c_void, out: *mut *mut u16) -> i32;
    fn ImpersonateNamedPipeClient(pipe: Handle) -> i32;
    fn RevertToSelf() -> i32;
    fn OpenThreadToken(thread: Handle, access: u32, open_as_self: i32, token: *mut Handle) -> i32;
    fn GetTokenInformation(token: Handle, class: u32, info: *mut c_void, len: u32, ret: *mut u32) -> i32;
    fn GetSidSubAuthorityCount(sid: *mut c_void) -> *mut u8;
    fn GetSidSubAuthority(sid: *mut c_void, index: u32) -> *mut u32;
    fn LookupAccountSidW(system: *const u16, sid: *mut c_void, name: *mut u16, name_len: *mut u32, domain: *mut u16, domain_len: *mut u32, sid_use: *mut u32) -> i32;
    fn GetSecurityInfo(h: Handle, object_type: u32, info: u32, owner: *mut *mut c_void, group: *mut *mut c_void, dacl: *mut *mut c_void, sacl: *mut *mut c_void, sd: *mut *mut c_void) -> u32;
}

const PIPE_ACCESS_DUPLEX: u32 = 3;
const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x0008_0000;
const PIPE_WAIT: u32 = 0;
const PIPE_REJECT_REMOTE_CLIENTS: u32 = 8;
const PIPE_UNLIMITED_INSTANCES: u32 = 255;
const ERROR_PIPE_CONNECTED: u32 = 535;
const OPEN_EXISTING: u32 = 3;
const SECURITY_SQOS_PRESENT: u32 = 0x0010_0000;
const SECURITY_IDENTIFICATION: u32 = 0x0001_0000;
const TOKEN_QUERY: u32 = 8;
const TOKEN_USER: u32 = 1;
const TOKEN_INTEGRITY_LEVEL: u32 = 25;
const SE_KERNEL_OBJECT: u32 = 6;
const OWNER_SECURITY_INFORMATION: u32 = 1;

fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

fn from_wide(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: Windows returned a NUL-terminated string.
    let len = (0..).take_while(|&i| unsafe { *p.add(i) } != 0).count();
    // SAFETY: `len` characters were just read.
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, len) })
}

fn sid_string(sid: *mut c_void) -> Option<String> {
    if sid.is_null() {
        return None;
    }
    let mut out: *mut u16 = std::ptr::null_mut();
    // SAFETY: a valid SID and out pointer; the string is freed with LocalFree.
    if unsafe { ConvertSidToStringSidW(sid, &mut out) } == 0 {
        return None;
    }
    let s = from_wide(out);
    // SAFETY: allocated by ConvertSidToStringSidW.
    unsafe { LocalFree(out.cast()) };
    Some(s)
}

struct Owned(Handle);
// SAFETY: a kernel handle may be used and closed from any thread.
unsafe impl Send for Owned {}
impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID {
            // SAFETY: a handle we own.
            unsafe { CloseHandle(self.0) };
        }
    }
}

// ---- the caller's token ----------------------------------------------------------------------------

/// What the token of the client on this pipe says: user SID, its name and domain, integrity RID.
/// Call after the request has been read, on the thread that serves the connection.
fn identify(pipe: Handle) -> (Option<String>, Option<(String, String)>, u32) {
    // SAFETY: a connected pipe handle.
    if unsafe { ImpersonateNamedPipeClient(pipe) } == 0 {
        return (None, None, 0);
    }
    let r = read_thread_token();
    // SAFETY: no preconditions; always revert, whatever happened above.
    unsafe { RevertToSelf() };
    r
}

fn read_thread_token() -> (Option<String>, Option<(String, String)>, u32) {
    let mut tok: Handle = std::ptr::null_mut();
    // SAFETY: the pseudo handle of this thread and a valid out pointer; opened as the impersonated client.
    if unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 0, &mut tok) } == 0 {
        return (None, None, 0);
    }
    let tok = Owned(tok);
    let info = |class: u32| -> Option<Vec<u64>> {
        let mut len = 0u32;
        // SAFETY: the first call asks for the size.
        unsafe { GetTokenInformation(tok.0, class, std::ptr::null_mut(), 0, &mut len) };
        if len == 0 || len > 4096 {
            return None;
        }
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        // SAFETY: the buffer is at least `len` bytes and 8-aligned.
        (unsafe { GetTokenInformation(tok.0, class, buf.as_mut_ptr().cast(), len, &mut len) } != 0).then_some(buf)
    };
    let (mut sid, mut who) = (None, None);
    if let Some(buf) = info(TOKEN_USER) {
        // SAFETY: TOKEN_USER starts with a SID_AND_ATTRIBUTES.
        let sa = unsafe { &*(buf.as_ptr() as *const SidAndAttributes) };
        sid = sid_string(sa.sid);
        let (mut nl, mut dl, mut usage) = (256u32, 256u32, 0u32);
        let (mut name, mut dom) = (vec![0u16; 256], vec![0u16; 256]);
        // SAFETY: valid SID and buffers with their lengths.
        if unsafe { LookupAccountSidW(std::ptr::null(), sa.sid, name.as_mut_ptr(), &mut nl, dom.as_mut_ptr(), &mut dl, &mut usage) } != 0 {
            who = Some((from_wide(dom.as_ptr()), from_wide(name.as_ptr())));
        }
    }
    let mut integrity = 0u32;
    if let Some(buf) = info(TOKEN_INTEGRITY_LEVEL) {
        // SAFETY: TOKEN_MANDATORY_LABEL starts with a SID_AND_ATTRIBUTES; the last sub authority is the level.
        unsafe {
            let sa = &*(buf.as_ptr() as *const SidAndAttributes);
            let n = *GetSidSubAuthorityCount(sa.sid);
            if n > 0 {
                integrity = *GetSidSubAuthority(sa.sid, u32::from(n) - 1);
            }
        }
    }
    (sid, who, integrity)
}

// ---- the service's facts and children -----------------------------------------------------------------

pub struct WinSvc {
    pub files: WinFiles,
    pub sys: crate::helper::worker::SystemDirs,
    names: Mutex<HashMap<AccountId, (String, String)>>,
}

impl WinSvc {
    pub fn new() -> WinSvc {
        let sys = crate::helper::winpipe::system_dirs();
        WinSvc { files: paths::windows_files(&sys.program_files), sys, names: Mutex::new(HashMap::new()) }
    }
    fn remember(&self, id: &AccountId, who: Option<(String, String)>) {
        if let Some(w) = who {
            self.names.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(id.clone(), w);
        }
    }
}

impl Default for WinSvc {
    fn default() -> Self {
        Self::new()
    }
}

fn trusted_owner(path: &str) -> bool {
    // SYSTEM, Administrators, TrustedInstaller
    crate::helper::winpipe::path_owner_sid(Path::new(path)).is_some_and(|o| matches!(o.as_str(), "S-1-5-18" | "S-1-5-32-544") || o.starts_with("S-1-5-80-"))
}

fn read_regular(path: &Path) -> Option<Vec<u8>> {
    let m = std::fs::symlink_metadata(path).ok()?;
    if !m.is_file() {
        return None;
    }
    let mut v = Vec::new();
    File::open(path).ok()?.take(1024 * 1024).read_to_end(&mut v).ok()?;
    Some(v)
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

impl Host for WinSvc {
    fn now(&self) -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
    }
    fn account_name(&self, id: &AccountId) -> String {
        self.names.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(id).map(|(d, n)| format!("{d}\\{n}")).unwrap_or_default()
    }
    fn folder_owner(&self, dir: &str) -> Result<Option<AccountId>, String> {
        crate::fsutil::check_links(Path::new(dir)).map_err(|e| format!("a link is on the way or it cannot be inspected ({e})"))?;
        let m = std::fs::symlink_metadata(dir).map_err(|e| format!("cannot inspect it: {e}"))?;
        if !m.is_dir() {
            return Err("it is not a folder".into());
        }
        let owner = crate::helper::winpipe::path_owner_sid(Path::new(dir));
        // An owner that is not an ordinary account (Administrators, SYSTEM): `uid:0`, never a caller's.
        Ok(owner.map(|o| AccountId::sid(&o).unwrap_or_else(|_| AccountId::uid(0))))
    }
    fn is_server_account(&self, _id: &AccountId) -> bool {
        false
    }
    fn install_record(&self, dir: &str) -> Option<InstallRecord> {
        let b = read_regular(&Path::new(dir).join(".kora").join("request.json"))?;
        let r: RecordFile = serde_json::from_slice(&b).ok()?;
        Some(InstallRecord { name: r.name, game_port: r.ports.game, txadmin_port: r.ports.txadmin })
    }
    fn db_name(&self, dir: &str) -> Option<String> {
        let b = read_regular(&Path::new(dir).join(".kora").join("db-provision.json"))?;
        serde_json::from_slice::<DbFile>(&b).ok().map(|d| d.database)
    }
    fn load_ledger(&self) -> Result<Ledger, String> {
        super::ownership::load(Path::new(&self.files.ledger))
    }
    fn save_ledger(&self, l: &Ledger) -> Result<(), String> {
        l.save(Path::new(&self.files.ledger)).map_err(|e| e.to_string())
    }
    fn audit(&self, line: &str) {
        use std::io::Write;
        let p = Path::new(&self.files.audit);
        let _ = std::fs::create_dir_all(&self.files.data_dir);
        if let Ok(m) = std::fs::metadata(p) {
            if super::audit::needs_trim(m.len()) {
                if let Ok(t) = std::fs::read_to_string(p) {
                    let _ = std::fs::write(p, super::audit::trimmed(&t));
                }
            }
        }
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
            let _ = writeln!(f, "{line}");
        }
    }
    fn own_version(&self) -> String {
        env!("CARGO_PKG_VERSION").into()
    }
}

impl Svc for WinSvc {
    fn host(&self) -> &dyn Host {
        self
    }

    fn policy(&self) -> Result<Policy, String> {
        if !trusted_owner(&self.files.policy) {
            return Err(format!("{} is missing or not owned by SYSTEM / Administrators", self.files.policy));
        }
        let b = read_regular(Path::new(&self.files.policy)).ok_or_else(|| format!("cannot read {}", self.files.policy))?;
        Policy::parse(&String::from_utf8_lossy(&b))
    }

    fn run(&self, caller: &Caller, spec: &RunSpec<'_>, progress: Progress<'_>, cancel: &Arc<AtomicBool>) -> std::io::Result<Ran> {
        if spec.argv.get(1).map(String::as_str) == Some("self-update") {
            let dir = std::path::PathBuf::from(&self.files.data_dir);
            return super::update::run_update(Path::new(&self.files.exe), &dir, progress, &mut |p| self.run_child(caller, spec, p, cancel));
        }
        self.run_child(caller, spec, progress, cancel)
    }
}

impl WinSvc {
    fn run_child(&self, caller: &Caller, spec: &RunSpec<'_>, progress: Progress<'_>, cancel: &Arc<AtomicBool>) -> std::io::Result<Ran> {
        use crate::helper::worker::{elevated_child_env, ChildExecutor, Executor};
        let id = caller.id.clone().ok_or_else(|| std::io::Error::other("no caller"))?;
        let (domain, name) = self.names.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&id).cloned().unwrap_or_default();
        let user = crate::helper::winpipe::PanelUser { sid: id.to_string(), domain, name, admin_member: false };
        let profile = crate::helper::winpipe::profile_dir(&user.sid);
        let mut env = elevated_child_env(&self.sys, &user, profile.as_deref(), None);
        env.push((paths::SERVICE_ENV.into(), "1".into()));
        env.extend(spec.env.iter().map(|(k, v)| (k.clone(), v.clone())));
        let mut ex = ChildExecutor::new(self.files.exe.clone().into());
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
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => Ok(Ran { code: -1, timed_out: true, data: None }),
            Err(e) => Err(e),
        }
    }
}

// ---- the pipe server --------------------------------------------------------------------------------------

fn make_instance(svc: &WinSvc, first: bool) -> std::io::Result<Owned> {
    let sddl = wide(&policy::pipe_sddl(&svc.policy()));
    let mut sd: *mut c_void = std::ptr::null_mut();
    // SAFETY: valid NUL-terminated string and out pointer.
    if unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), 1, &mut sd, std::ptr::null_mut()) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let sa = SecurityAttributes { n_length: std::mem::size_of::<SecurityAttributes>() as u32, descriptor: sd, inherit: 0 };
    let w = wide(paths::WINDOWS_PIPE);
    let mode = PIPE_ACCESS_DUPLEX | if first { FILE_FLAG_FIRST_PIPE_INSTANCE } else { 0 };
    // SAFETY: valid arguments; byte mode, blocking, remote clients refused.
    let h = unsafe { CreateNamedPipeW(w.as_ptr(), mode, PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS, PIPE_UNLIMITED_INSTANCES, 65536, 65536, 0, &sa) };
    let err = std::io::Error::last_os_error();
    // SAFETY: allocated by the conversion above.
    unsafe { LocalFree(sd) };
    if h == INVALID {
        return Err(err);
    }
    Ok(Owned(h))
}

/// Wake the blocking `ConnectNamedPipe` with a client of our own (stop, or restart after an update).
fn wake_listener() {
    let w = wide(paths::WINDOWS_PIPE);
    // SAFETY: valid name; the handle (if any) is closed at once.
    let h = unsafe { CreateFileW(w.as_ptr(), CLIENT_ACCESS, 0, std::ptr::null(), OPEN_EXISTING, 0, std::ptr::null_mut()) };
    if h != INVALID {
        // SAFETY: a handle we just opened.
        unsafe { CloseHandle(h) };
    }
}

fn serve_connection(svc: Arc<WinSvc>, pipe: Owned) {
    let raw = pipe.0;
    let got_request = Arc::new(AtomicBool::new(false));
    {
        // A client that connects and sends nothing must not hold a thread for ever.
        let (flag, h) = (Arc::clone(&got_request), pipe.0 as usize);
        std::thread::spawn(move || {
            for _ in 0..100 {
                std::thread::sleep(Duration::from_millis(100));
                if flag.load(Ordering::SeqCst) {
                    return;
                }
            }
            // SAFETY: only reached while the connection thread still owns the handle (it sets the flag or ends first).
            unsafe { DisconnectNamedPipe(h as Handle) };
        });
    }
    // SAFETY: the handle moves into the File; `pipe` is forgotten below so it is closed once.
    let file = unsafe { File::from_raw_handle(raw as RawHandle) };
    std::mem::forget(pipe);
    let mut reader = BufReader::new((&file).take(MAX_LINE as u64 + 1));
    // Windows lets the server read the client's token only after it has read from the pipe.
    let have = matches!(reader.fill_buf(), Ok(b) if !b.is_empty());
    got_request.store(true, Ordering::SeqCst);
    if have {
        let (sid, who, integrity) = identify(raw);
        let id = sid.as_deref().and_then(|s| AccountId::sid(s).ok());
        if let Some(i) = &id {
            svc.remember(i, who);
        }
        let caller = Caller { id, integrity: Some(integrity), is_root: false, remote: false };
        let mut out = &file;
        conn::handle_stream(&*svc, &mut reader, &mut out, caller);
        // SAFETY: our handle.
        unsafe { FlushFileBuffers(raw) };
    }
    // SAFETY: our handle; the File drop closes it.
    unsafe { DisconnectNamedPipe(raw) };
    if svc.restart_requested() {
        wake_listener();
    }
}

/// Serve until `stop`. Returns an error when the pipe cannot be made first (a squatter).
pub fn serve_pipe(svc: Arc<WinSvc>, stop: &AtomicBool) -> Result<(), String> {
    let active = Arc::new(AtomicUsize::new(0));
    let mut inst = make_instance(&svc, true).map_err(|e| format!("cannot create {} (is another program using the name?): {e}", paths::WINDOWS_PIPE))?;
    while !stop.load(Ordering::SeqCst) {
        // An update was installed and its answer sent: leave (the service manager starts the new program).
        if svc.restart_requested() {
            std::thread::sleep(Duration::from_secs(1)); // the answer in flight is sent
            break;
        }
        // SAFETY: a pipe instance we own; blocks until a client connects.
        let ok = unsafe { ConnectNamedPipe(inst.0, std::ptr::null_mut()) } != 0 || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
        if stop.load(Ordering::SeqCst) {
            break;
        }
        // The next instance first, so there is always one listening (and the name is never free).
        let next = match make_instance(&svc, false) {
            Ok(n) => n,
            Err(e) => {
                svc.audit(&format!("{} | service | could not make the next pipe instance: {e}", super::audit::rfc3339(svc.now())));
                std::thread::sleep(Duration::from_millis(500));
                match make_instance(&svc, false) {
                    Ok(n) => n,
                    Err(_) => return Err(format!("cannot create the next pipe instance: {e}")),
                }
            }
        };
        let conn = std::mem::replace(&mut inst, next);
        if !ok {
            continue;
        }
        if active.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
            active.fetch_sub(1, Ordering::SeqCst);
            continue; // dropped: too many at once
        }
        let (svc, active) = (Arc::clone(&svc), Arc::clone(&active));
        std::thread::spawn(move || {
            serve_connection(svc, conn);
            active.fetch_sub(1, Ordering::SeqCst);
        });
    }
    Ok(())
}

// ---- the Windows service ------------------------------------------------------------------------------------

/// ERROR_FAILED_SERVICE_CONTROLLER_CONNECT: not started by the service manager.
const NOT_A_SERVICE: i32 = 1063;
static EXIT: OnceLock<u32> = OnceLock::new();

define_windows_service!(ffi_service_main, service_main);

fn service_main(_args: Vec<OsString>) {
    let code = match run_as_service() {
        Ok(()) => 0,
        Err(e) => {
            // The service has no console: leave the reason where an administrator looks.
            let svc = WinSvc::new();
            svc.audit(&format!("{} | service | stopped: {e}", super::audit::rfc3339(svc.now())));
            1
        }
    };
    let _ = EXIT.set(code);
}

/// Refuse to run unless this program sits in `%ProgramFiles%\Kora` and administrators own it.
pub fn check_program_trusted(svc: &WinSvc) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = exe.to_string_lossy();
    if !paths::program_location_ok(&exe, true, Some(&svc.sys.program_files)) {
        return Err(format!("{exe} is not in Kora's protected program folder ({}\\Kora), so the service will not run it", svc.sys.program_files));
    }
    if !trusted_owner(&exe) {
        return Err(format!("{exe} is not owned by SYSTEM or Administrators, so the service will not run it"));
    }
    Ok(())
}

fn run_as_service() -> Result<(), String> {
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = Arc::clone(&stop);
    let handler = move |ev: ServiceControl| match ev {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            stop2.store(true, Ordering::SeqCst);
            wake_listener();
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    };
    let h = service_control_handler::register(NAME, handler).map_err(|e| e.to_string())?;
    let report = |state: ServiceState, accept: ServiceControlAccept, wait: Duration, code: u32| {
        let _ = h.set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: accept,
            exit_code: ServiceExitCode::Win32(code),
            checkpoint: 0,
            wait_hint: wait,
            process_id: None,
        });
    };
    report(ServiceState::StartPending, ServiceControlAccept::empty(), Duration::from_secs(15), 0);
    let svc = Arc::new(WinSvc::new());
    let r = check_program_trusted(&svc).and_then(|_| {
        // An update that was just installed: count this start, roll back when the new program keeps failing.
        let paths = super::update::Paths::in_dir(Path::new(&svc.files.data_dir));
        let (boot, said) = super::update::on_start(&paths, Path::new(&svc.files.exe), env!("CARGO_PKG_VERSION"));
        if let Some(s) = said {
            svc.audit(&format!("{} | service | {s}", super::audit::rfc3339(svc.now())));
        }
        match boot {
            super::update::Boot::RolledBack => return Err("rolled back to the previous program; stopping with an error so Windows starts it".to_string()),
            super::update::Boot::Probation(m) => {
                std::thread::spawn(move || {
                    std::thread::sleep(super::update::COMMIT_AFTER);
                    super::update::commit(&paths, &m);
                });
            }
            super::update::Boot::Normal => {}
        }
        report(ServiceState::Running, ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN, Duration::ZERO, 0);
        serve_pipe(Arc::clone(&svc), &stop)
    });
    // After an update the service stops with an error code on purpose: the failure actions restart it on the new program.
    let code = u32::from(r.is_err() || super::update::restart_requested());
    report(ServiceState::Stopped, ServiceControlAccept::empty(), Duration::ZERO, code);
    r
}

/// `kora service serve` on Windows: the service dispatcher; started from a console it says so.
pub fn run_service() -> Result<(), String> {
    match service_dispatcher::start(NAME, ffi_service_main) {
        Ok(()) => match EXIT.get() {
            Some(0) | None => Ok(()),
            Some(_) => Err("the service stopped with an error (see the audit log in the data folder)".into()),
        },
        Err(windows_service::Error::Winapi(e)) if e.raw_os_error() == Some(NOT_A_SERVICE) => {
            Err("this is the Windows service: it is started by Windows (Services), not from a console. Use `kora service status`.".into())
        }
        Err(e) => Err(format!("cannot talk to the Windows service manager: {e}")),
    }
}

// ---- the client (the panel, `kora service status`) -------------------------------------------------------------

fn open_client() -> Result<File, ClientError> {
    let w = wide(paths::WINDOWS_PIPE);
    // SAFETY: valid name.
    unsafe { WaitNamedPipeW(w.as_ptr(), 2000) };
    // Identification only (the service can see who we are, never act as us); exactly the rights the pipe grants.
    // SAFETY: valid name; the handle goes to `File`.
    let h = unsafe { CreateFileW(w.as_ptr(), CLIENT_ACCESS, 0, std::ptr::null(), OPEN_EXISTING, SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION, std::ptr::null_mut()) };
    if h == INVALID {
        // SAFETY: no preconditions.
        let code = unsafe { GetLastError() };
        return Err(match code {
            5 => ClientError::Refused("Kora's administrator service is not turned on for this account.".into()),
            2 | 3 | 231 | 121 => ClientError::Unavailable("Kora's administrator service is not running.".into()),
            c => ClientError::Unavailable(format!("Kora's administrator service did not answer (Windows error {c}).")),
        });
    }
    // The owner of the instance we reached must be SYSTEM (a pipe an allowed account made is theirs).
    let mut owner: *mut c_void = std::ptr::null_mut();
    let mut sd: *mut c_void = std::ptr::null_mut();
    // SAFETY: our handle and valid out pointers; `sd` is freed below.
    let r = unsafe { GetSecurityInfo(h, SE_KERNEL_OBJECT, OWNER_SECURITY_INFORMATION, &mut owner, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), &mut sd) };
    let owner_sid = if r == 0 { sid_string(owner) } else { None };
    if !sd.is_null() {
        // SAFETY: allocated by GetSecurityInfo.
        unsafe { LocalFree(sd) };
    }
    if let Err(why) = crate::helper::winhelper::check_server_owner(owner_sid.as_deref()) {
        // SAFETY: a handle we opened and have not handed on.
        unsafe { CloseHandle(h) };
        return Err(ClientError::Unavailable(format!("Nothing was sent: {why}.")));
    }
    // SAFETY: a handle we own, handed to `File`.
    Ok(unsafe { File::from_raw_handle(h as RawHandle) })
}

pub fn call(verb: &str, args: &[String], env: &[(String, String)], on_line: &mut dyn FnMut(&str)) -> Result<Answer, ClientError> {
    let f = open_client()?;
    let mut w = &f;
    conn::converse(&mut BufReader::new(&f), &mut w, verb, args, env, on_line)
}

pub fn probe() -> Result<String, ClientError> {
    conn::interpret_ping(call(crate::helper::PING, &[], &[], &mut |_| {}))
}

// ---- helpers for the command line (`kora service install | allow | status`) ---------------------------------------

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcess() -> Handle;
    fn WaitForSingleObject(h: Handle, ms: u32) -> u32;
    fn GetExitCodeProcess(h: Handle, code: *mut u32) -> i32;
}

#[link(name = "advapi32")]
extern "system" {
    fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> i32;
    fn LookupAccountNameW(system: *const u16, name: *const u16, sid: *mut c_void, sid_len: *mut u32, domain: *mut u16, domain_len: *mut u32, sid_use: *mut u32) -> i32;
}

/// The SID of the account this process runs as.
pub fn current_user_sid() -> Option<String> {
    let mut tok: Handle = std::ptr::null_mut();
    // SAFETY: the pseudo handle of this process and a valid out pointer.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut tok) } == 0 {
        return None;
    }
    let tok = Owned(tok);
    let mut len = 0u32;
    // SAFETY: the first call asks for the size.
    unsafe { GetTokenInformation(tok.0, TOKEN_USER, std::ptr::null_mut(), 0, &mut len) };
    if len == 0 || len > 4096 {
        return None;
    }
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    // SAFETY: the buffer is at least `len` bytes and 8-aligned.
    if unsafe { GetTokenInformation(tok.0, TOKEN_USER, buf.as_mut_ptr().cast(), len, &mut len) } == 0 {
        return None;
    }
    // SAFETY: TOKEN_USER starts with a SID_AND_ATTRIBUTES.
    let sa = unsafe { &*(buf.as_ptr() as *const SidAndAttributes) };
    sid_string(sa.sid)
}

/// The SID of the account called `name` (`DOMAIN\name`, `name`, or a UPN), as text.
pub fn lookup_account_sid(name: &str) -> Option<String> {
    let w = wide(name);
    let (mut sl, mut dl, mut usage) = (0u32, 0u32, 0u32);
    // SAFETY: the first call asks for the sizes.
    unsafe { LookupAccountNameW(std::ptr::null(), w.as_ptr(), std::ptr::null_mut(), &mut sl, std::ptr::null_mut(), &mut dl, &mut usage) };
    if sl == 0 || sl > 1024 || dl > 1024 {
        return None;
    }
    let mut sid = vec![0u64; (sl as usize).div_ceil(8)];
    let mut dom = vec![0u16; dl as usize + 1];
    // SAFETY: buffers at least as large as the sizes the first call reported.
    if unsafe { LookupAccountNameW(std::ptr::null(), w.as_ptr(), sid.as_mut_ptr().cast(), &mut sl, dom.as_mut_ptr(), &mut dl, &mut usage) } == 0 {
        return None;
    }
    sid_string(sid.as_mut_ptr().cast())
}

/// Where the settings file is (from `%ProgramFiles%` as Windows reports it).
pub fn policy_path() -> std::path::PathBuf {
    paths::windows_files(&crate::helper::winpipe::system_dirs().program_files).policy.into()
}

#[repr(C)]
struct ShellExecuteInfoW {
    cb_size: u32,
    f_mask: u32,
    hwnd: Handle,
    lp_verb: *const u16,
    lp_file: *const u16,
    lp_parameters: *const u16,
    lp_directory: *const u16,
    n_show: i32,
    h_inst_app: Handle,
    lp_id_list: *mut c_void,
    lp_class: *const u16,
    hkey_class: Handle,
    dw_hot_key: u32,
    h_icon_or_monitor: Handle,
    h_process: Handle,
}

#[link(name = "shell32")]
extern "system" {
    fn ShellExecuteExW(info: *mut ShellExecuteInfoW) -> i32;
}

#[link(name = "ole32")]
extern "system" {
    fn CoInitializeEx(reserved: *mut c_void, flags: u32) -> i32;
}

/// Start `file params` with the Windows permission prompt (`runas`), wait for it to end and return
/// its exit code. Used for ONE thing: installing the service itself (`msiexec` on the checked
/// installer), where someone at this computer approves once.
pub fn run_elevated_wait(file: &str, params: &str, limit: Duration) -> Result<u32, String> {
    let verb = wide("runas");
    let f = wide(file);
    let p = wide(params);
    let mut info = ShellExecuteInfoW {
        cb_size: std::mem::size_of::<ShellExecuteInfoW>() as u32,
        f_mask: 0x40 | 0x100, // NOCLOSEPROCESS | NOASYNC
        hwnd: std::ptr::null_mut(),
        lp_verb: verb.as_ptr(),
        lp_file: f.as_ptr(),
        lp_parameters: p.as_ptr(),
        lp_directory: std::ptr::null(),
        n_show: 1,
        h_inst_app: std::ptr::null_mut(),
        lp_id_list: std::ptr::null_mut(),
        lp_class: std::ptr::null(),
        hkey_class: std::ptr::null_mut(),
        dw_hot_key: 0,
        h_icon_or_monitor: std::ptr::null_mut(),
        h_process: std::ptr::null_mut(),
    };
    // SAFETY: COM for the shell (apartment threaded, as ShellExecuteEx asks); the struct is fully initialised.
    unsafe { CoInitializeEx(std::ptr::null_mut(), 2 | 4) };
    // SAFETY: see above.
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        // SAFETY: no preconditions.
        return Err(match unsafe { GetLastError() } {
            1223 => "The Windows permission prompt was answered with No (or closed), so nothing was installed.".to_string(),
            c => format!("Windows could not show the administrator permission prompt (error {c})."),
        });
    }
    if info.h_process.is_null() {
        return Err("Windows started the installer but gave Kora no way to wait for it.".into());
    }
    let proc = Owned(info.h_process);
    // SAFETY: a process handle we own.
    let r = unsafe { WaitForSingleObject(proc.0, limit.as_millis().min(u128::from(u32::MAX - 1)) as u32) };
    if r != 0 {
        return Err("The installer did not finish in time.".into());
    }
    let mut code = 1u32;
    // SAFETY: a process handle we own and a valid out pointer.
    unsafe { GetExitCodeProcess(proc.0, &mut code) };
    Ok(code)
}
