//! Windows: what the system says about a process (for [`super::installer_user`]). Thin: only reads; every decision is in the pure module.

use std::ffi::c_void;

use super::installer_user::Proc;

type Handle = *mut c_void;

#[repr(C)]
struct ProcessEntry32W {
    size: u32,
    usage: u32,
    pid: u32,
    heap_id: usize,
    module_id: u32,
    threads: u32,
    parent_pid: u32,
    priority: i32,
    flags: u32,
    exe_file: [u16; 260],
}

#[link(name = "kernel32")]
extern "system" {
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
    fn CloseHandle(h: Handle) -> i32;
    fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> isize;
    fn Process32FirstW(snapshot: isize, entry: *mut ProcessEntry32W) -> i32;
    fn Process32NextW(snapshot: isize, entry: *mut ProcessEntry32W) -> i32;
    fn LocalFree(p: *mut c_void) -> *mut c_void;
}
#[link(name = "advapi32")]
extern "system" {
    fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> i32;
    fn GetTokenInformation(token: Handle, class: u32, info: *mut c_void, len: u32, ret: *mut u32) -> i32;
    fn ConvertSidToStringSidW(sid: *mut c_void, out: *mut *mut u16) -> i32;
}

const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const TOKEN_QUERY: u32 = 0x8;
const TOKEN_USER: u32 = 1;
const TOKEN_SESSION_ID: u32 = 12;
const TOKEN_ELEVATION: u32 = 20;
const TH32CS_SNAPPROCESS: u32 = 0x2;

struct Owned(Handle);
impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: a handle this function opened.
        unsafe { CloseHandle(self.0) };
    }
}

fn token_info(tok: Handle, class: u32) -> Option<Vec<u64>> {
    let mut len = 0u32;
    // SAFETY: the first call asks for the size.
    unsafe { GetTokenInformation(tok, class, std::ptr::null_mut(), 0, &mut len) };
    if len == 0 || len > 4096 {
        return None;
    }
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    // SAFETY: the buffer is at least `len` bytes and 8-aligned.
    (unsafe { GetTokenInformation(tok, class, buf.as_mut_ptr().cast(), len, &mut len) } != 0).then_some(buf)
}

fn parent_of(pid: u32) -> Option<u32> {
    // SAFETY: the snapshot is closed below; `entry` has its size set.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == -1 || snap == 0 {
            return None;
        }
        let mut entry: ProcessEntry32W = std::mem::zeroed();
        entry.size = std::mem::size_of::<ProcessEntry32W>() as u32;
        let mut found = None;
        let mut more = Process32FirstW(snap, &mut entry) != 0;
        while more {
            if entry.pid == pid {
                found = Some(entry.parent_pid);
                break;
            }
            more = Process32NextW(snap, &mut entry) != 0;
        }
        CloseHandle(snap as Handle);
        found
    }
}

fn one(pid: u32) -> Option<Proc> {
    // SAFETY: handles are closed by `Owned`; buffers are as long as the sizes passed say.
    unsafe {
        let h = Owned(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid));
        if h.0.is_null() {
            return None;
        }
        let mut tok: Handle = std::ptr::null_mut();
        if OpenProcessToken(h.0, TOKEN_QUERY, &mut tok) == 0 {
            return None;
        }
        let tok = Owned(tok);
        let user = token_info(tok.0, TOKEN_USER)?;
        // TOKEN_USER starts with a SID_AND_ATTRIBUTES whose first field is the SID pointer.
        let sid_ptr = *(user.as_ptr() as *const *mut c_void);
        let mut out: *mut u16 = std::ptr::null_mut();
        if ConvertSidToStringSidW(sid_ptr, &mut out) == 0 || out.is_null() {
            return None;
        }
        let mut n = 0usize;
        while *out.add(n) != 0 && n < 256 {
            n += 1;
        }
        let sid = String::from_utf16_lossy(std::slice::from_raw_parts(out, n));
        LocalFree(out.cast());
        let session = *(token_info(tok.0, TOKEN_SESSION_ID)?.as_ptr() as *const u32);
        let elevated = *(token_info(tok.0, TOKEN_ELEVATION)?.as_ptr() as *const u32) != 0;
        Some(Proc { sid, elevated, session, parent: None })
    }
}

/// The facts about `pid`, and (one level up) the program that started it.
pub fn process_facts(pid: u32) -> Option<Proc> {
    let mut p = one(pid)?;
    p.parent = parent_of(pid).filter(|pp| *pp != 0 && *pp != pid).and_then(one).map(Box::new);
    Some(p)
}
