//! The Windows plumbing of the administrator worker: the pipe, the `runas` prompt and the peer
//! checks. Kept as thin as possible: every decision is a pure function in [`super::worker`]
//! (tested on every OS); this file only calls Windows. On other systems the entry points say it
//! is not available.

use std::sync::Arc;
use std::time::Duration;

use super::server::Validator;
use super::worker::{OpenError, Session};

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

static PANEL_USER: std::sync::OnceLock<PanelUser> = std::sync::OnceLock::new();

/// In the worker: the panel's account (set once the pipe's server was checked). `None` elsewhere.
pub fn panel_user() -> Option<&'static PanelUser> {
    PANEL_USER.get()
}

/// The environment every child of the worker gets on top of the worker's own: who "you" is
/// (`USERNAME`/`USERDOMAIN`, read by `service::installing_user`), even when an administrator of
/// another account answered the prompt.
pub fn child_env(u: &PanelUser) -> Vec<(String, String)> {
    vec![
        ("USERNAME".into(), u.name.clone()),
        ("USERDOMAIN".into(), u.domain.clone()),
        ("KORA_WORKER_USER_SID".into(), u.sid.clone()),
        ("KORA_HELPER".into(), "1".into()),
    ]
}

#[cfg(windows)]
pub use imp::{open_session, path_owner_sid, profile_dir, run_worker, selftest, system_dirs};

/// The system folders as Windows reports them (elsewhere: empty).
#[cfg(not(windows))]
pub fn system_dirs() -> super::worker::SystemDirs {
    super::worker::SystemDirs::default()
}

/// The profile folder of the account `sid` (`ProfileImagePath`), elsewhere `None`.
#[cfg(not(windows))]
pub fn profile_dir(_sid: &str) -> Option<String> {
    None
}

#[cfg(not(windows))]
pub fn open_session(_kora_home: Option<&str>, _limit: Duration) -> Result<Session, OpenError> {
    Err(OpenError::Unsupported)
}

#[cfg(not(windows))]
pub fn run_worker(_args: &[String], _validator: Arc<dyn Validator>) -> Result<(), String> {
    Err("the administrator helper only exists on Windows".into())
}

/// The owner SID (`S-1-...`) of a file or folder.
#[cfg(not(windows))]
pub fn path_owner_sid(_p: &std::path::Path) -> Option<String> {
    None
}

/// CI only (see [`imp::selftest`] on Windows).
#[cfg(not(windows))]
pub fn selftest(_say: &mut dyn FnMut(&str)) -> Result<(), String> {
    Err("the administrator helper only exists on Windows".into())
}

#[cfg(windows)]
mod imp {
    use super::*;
    use crate::helper::worker::{self, check_client_peer, check_server_peer, parse_worker_args, pipe_name, shell_execute_error, worker_args, CONNECT_LIMIT, IDLE_LIMIT, PIPE_SDDL};
    use std::ffi::c_void;
    use std::fs::File;
    use std::io::BufReader;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::os::windows::io::{FromRawHandle, RawHandle};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    type Handle = *mut c_void;
    const INVALID: Handle = -1isize as Handle;

    #[repr(C)]
    struct SecurityAttributes {
        n_length: u32,
        descriptor: *mut c_void,
        inherit: i32,
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
        fn SetNamedPipeHandleState(pipe: Handle, mode: *const u32, max_collection: *const u32, timeout: *const u32) -> i32;
        fn GetNamedPipeClientProcessId(pipe: Handle, pid: *mut u32) -> i32;
        fn GetNamedPipeServerProcessId(pipe: Handle, pid: *mut u32) -> i32;
        fn WaitNamedPipeW(name: *const u16, timeout: u32) -> i32;
        fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *const SecurityAttributes, disposition: u32, flags: u32, template: Handle) -> Handle;
        fn CloseHandle(h: Handle) -> i32;
        fn GetLastError() -> u32;
        fn GetProcessId(h: Handle) -> u32;
        fn WaitForSingleObject(h: Handle, ms: u32) -> u32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        fn QueryFullProcessImageNameW(h: Handle, flags: u32, buf: *mut u16, len: *mut u32) -> i32;
        fn LocalFree(p: *mut c_void) -> *mut c_void;
        fn GetSystemWindowsDirectoryW(buf: *mut u16, size: u32) -> u32;
    }

    #[link(name = "advapi32")]
    extern "system" {
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl: *const u16, revision: u32, sd: *mut *mut c_void, size: *mut u32) -> i32;
        fn ConvertSidToStringSidW(sid: *mut c_void, out: *mut *mut u16) -> i32;
        fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> i32;
        fn GetTokenInformation(token: Handle, class: u32, info: *mut c_void, len: u32, ret: *mut u32) -> i32;
        fn LookupAccountSidW(system: *const u16, sid: *mut c_void, name: *mut u16, name_len: *mut u32, domain: *mut u16, domain_len: *mut u32, sid_use: *mut u32) -> i32;
        fn RegGetValueW(key: isize, sub: *const u16, value: *const u16, flags: u32, kind: *mut u32, data: *mut c_void, len: *mut u32) -> i32;
        fn GetNamedSecurityInfoW(name: *const u16, object_type: u32, info: u32, owner: *mut *mut c_void, group: *mut *mut c_void, dacl: *mut *mut c_void, sacl: *mut *mut c_void, sd: *mut *mut c_void) -> u32;
    }

    #[link(name = "shell32")]
    extern "system" {
        fn ShellExecuteExW(info: *mut ShellExecuteInfoW) -> i32;
        fn SHGetKnownFolderPath(id: *const Guid, flags: u32, token: Handle, path: *mut *mut u16) -> i32;
    }

    #[link(name = "ole32")]
    extern "system" {
        fn CoInitializeEx(reserved: *mut c_void, flags: u32) -> i32;
        fn CoTaskMemFree(p: *mut c_void);
    }

    #[repr(C)]
    struct Guid(u32, u16, u16, [u8; 8]);
    const FOLDERID_PROGRAM_FILES: Guid = Guid(0x905e63b6, 0xc1bf, 0x494e, [0xb2, 0xdc, 0x28, 0xd6, 0xe0, 0xda, 0xd5, 0xc8]);
    const FOLDERID_PROGRAM_FILES_X86: Guid = Guid(0x7c5a40ef, 0xa0fb, 0x4bfc, [0x87, 0x4a, 0xc0, 0xf2, 0xe0, 0xb9, 0xfa, 0x8e]);
    const FOLDERID_PROGRAM_DATA: Guid = Guid(0x62ab5d82, 0xfdc1, 0x4dc3, [0xa9, 0xdd, 0x07, 0x0d, 0x1d, 0x49, 0x5d, 0x97]);
    const HKEY_LOCAL_MACHINE: isize = 0x8000_0002u32 as i32 as isize;

    fn known_folder(id: &Guid) -> Option<String> {
        let mut p: *mut u16 = std::ptr::null_mut();
        // SAFETY: valid GUID and out pointer; the string is freed with CoTaskMemFree.
        let hr = unsafe { SHGetKnownFolderPath(id, 0, std::ptr::null_mut(), &mut p) };
        let s = if hr == 0 { Some(from_wide(p)) } else { None };
        if !p.is_null() {
            // SAFETY: allocated by SHGetKnownFolderPath.
            unsafe { CoTaskMemFree(p.cast()) };
        }
        s.filter(|s| !s.is_empty())
    }

    /// The system folders from Windows itself (not from this process's variables).
    pub fn system_dirs() -> worker::SystemDirs {
        let mut buf = vec![0u16; 1024];
        // SAFETY: buffer and size match.
        let n = unsafe { GetSystemWindowsDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
        let root = if n > 0 && n < buf.len() { String::from_utf16_lossy(&buf[..n]) } else { r"C:\Windows".to_string() };
        let drive: String = root.chars().take(2).collect();
        worker::SystemDirs {
            program_files: known_folder(&FOLDERID_PROGRAM_FILES).unwrap_or_else(|| format!(r"{drive}\Program Files")),
            program_files_x86: known_folder(&FOLDERID_PROGRAM_FILES_X86).unwrap_or_default(),
            program_data: known_folder(&FOLDERID_PROGRAM_DATA).unwrap_or_else(|| format!(r"{drive}\ProgramData")),
            system_root: root,
        }
    }

    /// `HKLM\...\ProfileList\<sid>\ProfileImagePath` (expanded).
    pub fn profile_dir(sid: &str) -> Option<String> {
        if sid.is_empty() || !sid.bytes().all(|b| b == b'S' || b == b'-' || b.is_ascii_digit()) {
            return None;
        }
        let key = wide(&format!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList\{sid}"));
        let value = wide("ProfileImagePath");
        let mut buf = vec![0u16; 1024];
        let mut len = (buf.len() * 2) as u32;
        // SAFETY: valid strings, buffer and length in bytes; REG_SZ | REG_EXPAND_SZ (expanded).
        let r = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), value.as_ptr(), 0x2 | 0x4, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut len) };
        (r == 0).then(|| from_wide(buf.as_ptr())).filter(|s| !s.is_empty())
    }

    const PIPE_ACCESS_DUPLEX: u32 = 3;
    const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x0008_0000;
    const PIPE_NOWAIT: u32 = 1;
    const PIPE_WAIT: u32 = 0;
    const PIPE_REJECT_REMOTE_CLIENTS: u32 = 8;
    const ERROR_PIPE_CONNECTED: u32 = 535;
    const ERROR_NO_DATA: u32 = 232;
    const GENERIC_RW: u32 = 0x8000_0000 | 0x4000_0000;
    const OPEN_EXISTING: u32 = 3;
    const SECURITY_SQOS_PRESENT: u32 = 0x0010_0000;
    const SECURITY_IDENTIFICATION: u32 = 0x0001_0000;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const TOKEN_QUERY: u32 = 8;
    const SYNCHRONIZE: u32 = 0x0010_0000;
    const TOKEN_USER: u32 = 1;
    const SE_FILE_OBJECT: u32 = 1;
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

    /// An earlier prompt is still on screen (its thread has not returned yet).
    static PROMPT_OPEN: AtomicBool = AtomicBool::new(false);

    fn random16() -> [u8; 16] {
        let mut b = [0u8; 16];
        if getrandom::fill(&mut b).is_err() {
            // Never expected; the name is not the protection (the DACL and the peer checks are).
            let t = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
            b = t.to_le_bytes();
        }
        b
    }

    fn create_pipe(name: &str) -> std::io::Result<Owned> {
        let sddl = wide(PIPE_SDDL);
        let mut sd: *mut c_void = std::ptr::null_mut();
        // SAFETY: valid NUL-terminated string and out pointer.
        if unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), 1, &mut sd, std::ptr::null_mut()) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let sa = SecurityAttributes { n_length: std::mem::size_of::<SecurityAttributes>() as u32, descriptor: sd, inherit: 0 };
        let w = wide(name);
        // SAFETY: valid arguments; one instance, byte mode, not waiting (polled), remote clients refused.
        let h = unsafe { CreateNamedPipeW(w.as_ptr(), PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS, 1, 65536, 65536, 0, &sa) };
        let err = std::io::Error::last_os_error();
        // SAFETY: allocated by the conversion above.
        unsafe { LocalFree(sd) };
        if h == INVALID {
            return Err(err);
        }
        Ok(Owned(h))
    }

    /// Start this program elevated with the prompt; blocks until the person answered.
    fn shell_runas(exe: &Path, params: &str) -> Result<Owned, u32> {
        let verb = wide("runas");
        let file: Vec<u16> = exe.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let params = wide(params);
        let mut info = ShellExecuteInfoW {
            cb_size: std::mem::size_of::<ShellExecuteInfoW>() as u32,
            // NOCLOSEPROCESS | NOASYNC | FLAG_NO_UI
            f_mask: 0x40 | 0x100 | 0x400,
            hwnd: std::ptr::null_mut(),
            lp_verb: verb.as_ptr(),
            lp_file: file.as_ptr(),
            lp_parameters: params.as_ptr(),
            lp_directory: std::ptr::null(),
            n_show: 0, // SW_HIDE: the worker has no window
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
            return Err(unsafe { GetLastError() });
        }
        if info.h_process.is_null() {
            return Err(1156);
        }
        Ok(Owned(info.h_process))
    }

    /// How the worker was started: through the prompt, or (the CI self-test, already elevated) directly.
    enum Launched {
        Shell(Owned),
        Child(std::process::Child),
    }

    impl Launched {
        fn pid(&self) -> u32 {
            match self {
                // SAFETY: a process handle we own.
                Launched::Shell(h) => unsafe { GetProcessId(h.0) },
                Launched::Child(c) => c.id(),
            }
        }
        fn exited(&mut self) -> bool {
            match self {
                // SAFETY: a process handle we own.
                Launched::Shell(h) => (unsafe { WaitForSingleObject(h.0, 0) }) == 0,
                Launched::Child(c) => c.try_wait().map(|s| s.is_some()).unwrap_or(true),
            }
        }
    }

    type Launch = fn(&Path, &str, &[String]) -> Result<Launched, u32>;

    fn launch_runas(exe: &Path, params: &str, _args: &[String]) -> Result<Launched, u32> {
        shell_runas(exe, params).map(Launched::Shell)
    }

    fn launch_direct(exe: &Path, _params: &str, args: &[String]) -> Result<Launched, u32> {
        use std::os::windows::process::CommandExt;
        std::process::Command::new(exe)
            .args(args)
            .stdin(std::process::Stdio::null())
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .spawn()
            .map(Launched::Child)
            .map_err(|e| e.raw_os_error().unwrap_or(1) as u32)
    }

    /// Panel side: ask Windows (the prompt), wait for the worker to connect, check it, and return
    /// the session. `limit`: how long the person has to answer.
    pub fn open_session(kora_home: Option<&str>, limit: Duration) -> Result<Session, OpenError> {
        if crate::elevate::is_elevated() {
            return Err(OpenError::Unsupported);
        }
        open_with(kora_home, limit, launch_runas).map(|(s, _)| s)
    }

    fn open_with(kora_home: Option<&str>, limit: Duration, launch: Launch) -> Result<(Session, Launched), OpenError> {
        if PROMPT_OPEN.swap(true, Ordering::SeqCst) {
            return Err(OpenError::Busy);
        }
        let exe = match std::env::current_exe() {
            Ok(e) => e,
            Err(e) => {
                PROMPT_OPEN.store(false, Ordering::SeqCst);
                return Err(OpenError::Failed(format!("cannot find the kora program: {e}")));
            }
        };
        let me = std::process::id();
        let name = pipe_name(me, &random16());
        let pipe = match create_pipe(&name) {
            Ok(p) => p,
            Err(e) => {
                PROMPT_OPEN.store(false, Ordering::SeqCst);
                return Err(OpenError::Failed(format!("cannot create the connection for the administrator helper: {e}")));
            }
        };
        let args = worker_args(&name, me);
        let params = args.iter().map(|a| crate::elevate::quote_windows_arg(a)).collect::<Vec<_>>().join(" ");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let r = launch(&exe, &params, &args);
            PROMPT_OPEN.store(false, Ordering::SeqCst);
            // Nobody waits any more (timed out): dropping it is fine, the worker finds no pipe and exits by itself.
            let _ = tx.send(r);
        });
        let mut process = match rx.recv_timeout(limit) {
            Ok(Ok(p)) => p,
            Ok(Err(code)) => return Err(shell_execute_error(code)),
            Err(_) => return Err(OpenError::TimedOut),
        };
        let worker_pid = process.pid();
        let deadline = Instant::now() + CONNECT_LIMIT;
        loop {
            // SAFETY: our pipe handle (non-blocking mode: answers at once).
            let ok = unsafe { ConnectNamedPipe(pipe.0, std::ptr::null_mut()) } != 0;
            // SAFETY: no preconditions.
            let err = unsafe { GetLastError() };
            if ok || err == ERROR_PIPE_CONNECTED || err == ERROR_NO_DATA {
                break;
            }
            if process.exited() {
                return Err(OpenError::Failed("Kora's administrator helper stopped before it connected.".into()));
            }
            if Instant::now() > deadline {
                return Err(OpenError::Failed("Kora's administrator helper did not connect.".into()));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let mut client = 0u32;
        // SAFETY: our pipe handle and a valid out pointer.
        let got = unsafe { GetNamedPipeClientProcessId(pipe.0, &mut client) } != 0;
        if let Err(m) = if got { check_client_peer(worker_pid, client) } else { Err("cannot see who connected".into()) } {
            // SAFETY: our pipe handle.
            unsafe { DisconnectNamedPipe(pipe.0) };
            return Err(OpenError::Failed(m));
        }
        let wait = PIPE_WAIT;
        // SAFETY: our pipe handle; back to blocking reads and writes.
        unsafe { SetNamedPipeHandleState(pipe.0, &wait, std::ptr::null(), std::ptr::null()) };
        // SAFETY: the handle is ours and handed over to `File`, which closes it.
        let file = unsafe { File::from_raw_handle(pipe.0 as RawHandle) };
        std::mem::forget(pipe);
        let reader = file.try_clone().map_err(|e| OpenError::Failed(e.to_string()))?;
        let s = Session::begin(Box::new(BufReader::new(reader)), Box::new(file), kora_home).map_err(|e| OpenError::Failed(e.to_string()))?;
        Ok((s, process))
    }

    /// `kora admin-worker --selftest`, for CI (whose account is already elevated, so there is no
    /// prompt to answer): everything the panel and the worker do except the prompt itself. The
    /// worker is started directly instead of through `runas`; the pipe, its security, both peer
    /// checks, the protocol, the allow-list and an `admin-step` child are the real ones.
    pub fn selftest(say: &mut dyn FnMut(&str)) -> Result<(), String> {
        use crate::helper::client::HelperError;
        if !crate::elevate::is_elevated() {
            return Err("the self-test is for an account that is already elevated (CI); people use Kora Panel".into());
        }
        let home = crate::installer::registry::kora_home().to_string_lossy().into_owned();
        let (mut s, mut child) = open_with(Some(&home), Duration::from_secs(30), launch_direct).map_err(|e| format!("the worker did not start or connect: {e:?}"))?;
        say(&format!("ok: the worker started, connected through the pipe and passed both peer checks (pid {})", s.worker_pid));
        let ping = s.run(crate::helper::PING, &[], &[], &mut |_| {}).map_err(|e| format!("ping: {e}"))?;
        if !ping.ok {
            return Err("ping was not answered".into());
        }
        say("ok: ping answered");
        for (verb, args) in [("install", vec!["--name", "x"]), ("delete", vec!["x", "--yes"]), ("cmd.exe", vec!["/c", "whoami"]), ("admin-step", vec!["run", "--step", "system.firewall", "--dir", r"C:\Windows"])] {
            let a: Vec<String> = args.iter().map(|x| x.to_string()).collect();
            match s.run(verb, &a, &[], &mut |_| {}) {
                Err(HelperError::Rejected(m)) => say(&format!("ok: refused {verb} {}: {m}", args.join(" "))),
                other => return Err(format!("{verb} {args:?} was not refused: {other:?}")),
            }
        }
        // A real step child: undoing nothing in a folder with an install record.
        let dir = std::env::temp_dir().join(format!("kora-worker-selftest-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".kora")).map_err(|e| e.to_string())?;
        let req = crate::installer::InstallRequest {
            mode: crate::installer::Mode::Production,
            name: "selftest".into(),
            dir: dir.clone(),
            hostname: String::new(),
            license_key: None,
            license_key_name: None,
            cfx_identifier: None,
            ports: crate::installer::Ports::default(),
            txadmin_public: false,
            artifact_url: None,
            game_build: None,
            resources: crate::installer::ResourceSelection::All,
            db: crate::installer::DbChoice::Skip,
            service: false,
            firewall: false,
            github_owner: None,
            catalog: None,
            dry_run: false,
            db_admin: None,
            start: false,
            app: None,
        };
        std::fs::write(dir.join(".kora").join("request.json"), serde_json::to_vec(&req).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let args = crate::installer::system_ops::step_args(true, "system.schedule", &dir, true, true, &None);
        let mut lines = Vec::new();
        let out = s.run("admin-step", &args, &[], &mut |l| lines.push(l.to_string())).map_err(|e| format!("admin-step: {e}"))?;
        let _ = std::fs::remove_dir_all(&dir);
        let r: crate::installer::system_ops::StepResult = out.data.and_then(|v| serde_json::from_value(v).ok()).ok_or("the step child sent no result line")?;
        if !out.ok || r.error.is_some() {
            return Err(format!("the step child failed: {:?} {lines:?}", r.error));
        }
        say(&format!("ok: an admin-step child ran and reported back ({})", r.detail));
        s.finish();
        let t = std::time::Instant::now();
        while !child.exited() {
            if t.elapsed() > Duration::from_secs(15) {
                return Err("the worker did not exit after bye".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        say("ok: the worker exited after bye");
        Ok(())
    }

    fn process_image(pid: u32) -> Option<PathBuf> {
        // SAFETY: plain call; the handle is closed by `Owned`.
        let h = Owned(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) });
        if h.0.is_null() {
            return None;
        }
        let mut buf = vec![0u16; 32768];
        let mut len = buf.len() as u32;
        // SAFETY: buffer and length match.
        if unsafe { QueryFullProcessImageNameW(h.0, 0, buf.as_mut_ptr(), &mut len) } == 0 {
            return None;
        }
        Some(PathBuf::from(std::ffi::OsString::from_wide(&buf[..len as usize])))
    }

    fn sid_string(sid: *mut c_void) -> Option<String> {
        let mut out: *mut u16 = std::ptr::null_mut();
        // SAFETY: `sid` is a valid SID from Windows.
        if unsafe { ConvertSidToStringSidW(sid, &mut out) } == 0 {
            return None;
        }
        let s = from_wide(out);
        // SAFETY: allocated by ConvertSidToStringSidW.
        unsafe { LocalFree(out.cast()) };
        Some(s)
    }

    fn process_user(pid: u32) -> Option<PanelUser> {
        // SAFETY: plain calls; handles closed by `Owned`, buffers sized as Windows says.
        unsafe {
            let p = Owned(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid));
            if p.0.is_null() {
                return None;
            }
            let mut t: Handle = std::ptr::null_mut();
            if OpenProcessToken(p.0, TOKEN_QUERY, &mut t) == 0 {
                return None;
            }
            let t = Owned(t);
            let mut len = 0u32;
            GetTokenInformation(t.0, TOKEN_USER, std::ptr::null_mut(), 0, &mut len);
            let mut buf = vec![0u64; (len as usize).div_ceil(8).max(1)];
            if GetTokenInformation(t.0, TOKEN_USER, buf.as_mut_ptr().cast(), len, &mut len) == 0 {
                return None;
            }
            let sa = &*(buf.as_ptr() as *const SidAndAttributes);
            let sid = sid_string(sa.sid)?;
            // TOKEN_GROUPS: any Administrators entry, enabled or deny-only (a filtered admin token).
            let mut glen = 0u32;
            GetTokenInformation(t.0, 2, std::ptr::null_mut(), 0, &mut glen);
            let mut gbuf = vec![0u64; (glen as usize).div_ceil(8).max(1)];
            let mut admin_member = false;
            if GetTokenInformation(t.0, 2, gbuf.as_mut_ptr().cast(), glen, &mut glen) != 0 {
                let count = *(gbuf.as_ptr() as *const u32) as usize;
                let first = (gbuf.as_ptr() as *const u8).add(std::mem::size_of::<usize>()) as *const SidAndAttributes;
                for i in 0..count {
                    if sid_string((*first.add(i)).sid).as_deref() == Some("S-1-5-32-544") {
                        admin_member = true;
                    }
                }
            }
            let (mut nl, mut dl, mut usage) = (256u32, 256u32, 0u32);
            let (mut name, mut dom) = (vec![0u16; 256], vec![0u16; 256]);
            if LookupAccountSidW(std::ptr::null(), sa.sid, name.as_mut_ptr(), &mut nl, dom.as_mut_ptr(), &mut dl, &mut usage) == 0 {
                return None;
            }
            Some(PanelUser { sid, name: from_wide(name.as_ptr()), domain: from_wide(dom.as_ptr()), admin_member })
        }
    }

    pub fn path_owner_sid(p: &Path) -> Option<String> {
        let w: Vec<u16> = p.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let (mut owner, mut sd): (*mut c_void, *mut c_void) = (std::ptr::null_mut(), std::ptr::null_mut());
        // SAFETY: valid path and out pointers; `sd` is freed below.
        let r = unsafe { GetNamedSecurityInfoW(w.as_ptr(), SE_FILE_OBJECT, OWNER_SECURITY_INFORMATION, &mut owner, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), &mut sd) };
        if r != 0 {
            return None;
        }
        let s = sid_string(owner);
        // SAFETY: allocated by GetNamedSecurityInfoW.
        unsafe { LocalFree(sd) };
        s
    }

    /// Worker side (`kora admin-worker --pipe .. --parent ..`): connect, check, serve, exit.
    pub fn run_worker(args: &[String], validator: Arc<dyn Validator>) -> Result<(), String> {
        let (name, parent) = parse_worker_args(args)?;
        if !crate::elevate::is_elevated() {
            return Err("the administrator helper must run as administrator (Kora Panel starts it)".into());
        }
        let w = wide(&name);
        // SAFETY: valid name.
        unsafe { WaitNamedPipeW(w.as_ptr(), 30_000) };
        // Identification only: the panel (not elevated) may see who we are but never act as us.
        // SAFETY: valid name; the handle goes to `File`.
        let h = unsafe { CreateFileW(w.as_ptr(), GENERIC_RW, 0, std::ptr::null(), OPEN_EXISTING, SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION, std::ptr::null_mut()) };
        if h == INVALID {
            return Err(format!("cannot reach Kora Panel: {}", std::io::Error::last_os_error()));
        }
        // SAFETY: a handle we own, handed to `File`.
        let file = unsafe { File::from_raw_handle(h as RawHandle) };
        let mut server = 0u32;
        // SAFETY: our handle and a valid out pointer.
        if unsafe { GetNamedPipeServerProcessId(h, &mut server) } == 0 {
            return Err("cannot see who made the connection".into());
        }
        let own = std::env::current_exe().map_err(|e| e.to_string())?;
        check_server_peer(parent, server, process_image(server).as_deref(), &own)?;
        let user = process_user(server).ok_or("cannot see which account runs Kora Panel")?;
        let _ = PANEL_USER.set(user.clone());
        // Idle watchdog: a worker nobody talks to goes away.
        let last = Arc::new(AtomicU64::new(0));
        let busy = Arc::new(AtomicBool::new(false));
        let start = Instant::now();
        {
            let (last, busy) = (last.clone(), busy.clone());
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_secs(30));
                let idle = start.elapsed().as_secs().saturating_sub(last.load(Ordering::SeqCst));
                if !busy.load(Ordering::SeqCst) && idle > IDLE_LIMIT.as_secs() {
                    std::process::exit(0);
                }
            });
        }
        let touch = |b: bool| {
            busy.store(b, Ordering::SeqCst);
            last.store(start.elapsed().as_secs(), Ordering::SeqCst);
        };
        let mut reader = BufReader::new(file.try_clone().map_err(|e| e.to_string())?);
        let mut writer = file;
        let exec = Arc::new(worker::ChildExecutor::new(own));
        // The panel's process ends (crash, closed): stop a running child at once instead of
        // letting it finish for nobody.
        {
            let exec = exec.clone();
            // SAFETY: plain call; the handle is closed by `Owned` in the thread.
            let h = Owned(unsafe { OpenProcess(SYNCHRONIZE, 0, parent) });
            if !h.0.is_null() {
                std::thread::spawn(move || {
                    let h = h;
                    // SAFETY: a process handle we own.
                    unsafe { WaitForSingleObject(h.0, u32::MAX) };
                    worker::Executor::cancel(exec.as_ref());
                });
            }
        }
        // The children's whole environment, built from nothing (never this process's inherited one).
        let base = worker::elevated_child_env(&system_dirs(), &user, profile_dir(&user.sid).as_deref(), None);
        worker::serve(&mut reader, &mut writer, validator.as_ref(), exec.as_ref(), &base, &touch).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn children_act_for_the_panel_user_not_the_administrator_who_answered() {
        let u = PanelUser { sid: "S-1-5-21-1-2-3-1001".into(), domain: "PC".into(), name: "jo".into(), admin_member: false };
        let e = child_env(&u);
        assert!(e.contains(&("USERNAME".into(), "jo".into())) && e.contains(&("USERDOMAIN".into(), "PC".into())));
        assert!(e.contains(&("KORA_WORKER_USER_SID".into(), u.sid.clone())));
        assert!(e.iter().all(|(k, _)| !super::super::SECRET_ENV.contains(&k.as_str())));
    }

    #[cfg(not(windows))]
    #[test]
    fn elsewhere_it_is_not_available() {
        assert!(matches!(open_session(None, Duration::from_secs(1)), Err(OpenError::Unsupported)));
        assert!(run_worker(&[], Arc::new(|_: &str, _: &[String]| Ok(()))).is_err());
        assert!(path_owner_sid(std::path::Path::new("/")).is_none());
    }
}
