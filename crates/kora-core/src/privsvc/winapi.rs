//! The few Windows facts the service and the migration need, asked from Windows itself and never from the
//! environment (a user can set `ProgramFiles` or `TEMP` in their own): the system folders, an account's
//! profile folder, and who owns a path. Elsewhere these answer "nothing".

use std::path::Path;

/// Folders as Windows itself reports them (system directory and known-folder APIs).
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

#[cfg(not(windows))]
pub fn system_dirs() -> SystemDirs {
    SystemDirs::default()
}

/// The profile folder of the account `sid` (`ProfileImagePath`), elsewhere `None`.
#[cfg(not(windows))]
pub fn profile_dir(_sid: &str) -> Option<String> {
    None
}

/// The owner SID of `p` (links followed by the OS: callers check for links first), elsewhere `None`.
#[cfg(not(windows))]
pub fn path_owner_sid(_p: &Path) -> Option<String> {
    None
}

#[cfg(windows)]
pub use imp::{path_owner_sid, profile_dir, system_dirs};

#[cfg(windows)]
mod imp {
    use super::*;
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;

    #[repr(C)]
    struct Guid(u32, u16, u16, [u8; 8]);
    const FOLDERID_PROGRAM_FILES: Guid = Guid(0x905e63b6, 0xc1bf, 0x494e, [0xb2, 0xdc, 0x28, 0xd6, 0xe0, 0xda, 0xd5, 0xc8]);
    const FOLDERID_PROGRAM_FILES_X86: Guid = Guid(0x7c5a40ef, 0xa0fb, 0x4bfc, [0x87, 0x4a, 0xc0, 0xf2, 0xe0, 0xb9, 0xfa, 0x8e]);
    const FOLDERID_PROGRAM_DATA: Guid = Guid(0x62ab5d82, 0xfdc1, 0x4dc3, [0xa9, 0xdd, 0x07, 0x0d, 0x1d, 0x49, 0x5d, 0x97]);
    const HKEY_LOCAL_MACHINE: isize = 0x8000_0002u32 as i32 as isize;
    const SE_FILE_OBJECT: u32 = 1;
    const OWNER_SECURITY_INFORMATION: u32 = 1;

    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(p: *mut c_void) -> *mut c_void;
        fn GetSystemWindowsDirectoryW(buf: *mut u16, size: u32) -> u32;
    }
    #[link(name = "advapi32")]
    extern "system" {
        fn ConvertSidToStringSidW(sid: *mut c_void, out: *mut *mut u16) -> i32;
        fn RegGetValueW(key: isize, sub: *const u16, value: *const u16, flags: u32, kind: *mut u32, data: *mut c_void, len: *mut u32) -> i32;
        fn GetNamedSecurityInfoW(
            name: *const u16,
            object_type: u32,
            info: u32,
            owner: *mut *mut c_void,
            group: *mut *mut c_void,
            dacl: *mut *mut c_void,
            sacl: *mut *mut c_void,
            sd: *mut *mut c_void,
        ) -> u32;
    }
    #[link(name = "shell32")]
    extern "system" {
        fn SHGetKnownFolderPath(id: *const Guid, flags: u32, token: *mut c_void, path: *mut *mut u16) -> i32;
    }
    #[link(name = "ole32")]
    extern "system" {
        fn CoTaskMemFree(p: *mut c_void);
    }

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

    pub fn system_dirs() -> SystemDirs {
        let mut buf = vec![0u16; 1024];
        // SAFETY: buffer and size match.
        let n = unsafe { GetSystemWindowsDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
        let root = if n > 0 && n < buf.len() { String::from_utf16_lossy(&buf[..n]) } else { r"C:\Windows".to_string() };
        let drive: String = root.chars().take(2).collect();
        SystemDirs {
            program_files: known_folder(&FOLDERID_PROGRAM_FILES).unwrap_or_else(|| format!(r"{drive}\Program Files")),
            program_files_x86: known_folder(&FOLDERID_PROGRAM_FILES_X86).unwrap_or_default(),
            program_data: known_folder(&FOLDERID_PROGRAM_DATA).unwrap_or_else(|| format!(r"{drive}\ProgramData")),
            system_root: root,
        }
    }

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

    pub fn path_owner_sid(p: &Path) -> Option<String> {
        let w: Vec<u16> = p.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let (mut owner, mut sd): (*mut c_void, *mut c_void) = (std::ptr::null_mut(), std::ptr::null_mut());
        // SAFETY: valid path and out pointers; `sd` is freed below.
        let r = unsafe { GetNamedSecurityInfoW(w.as_ptr(), SE_FILE_OBJECT, OWNER_SECURITY_INFORMATION, &mut owner, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), &mut sd) };
        if r != 0 {
            return None;
        }
        let mut out: *mut u16 = std::ptr::null_mut();
        // SAFETY: a valid SID inside `sd`; the string is freed with LocalFree.
        let s = if unsafe { ConvertSidToStringSidW(owner, &mut out) } != 0 {
            let t = from_wide(out);
            // SAFETY: allocated by ConvertSidToStringSidW.
            unsafe { LocalFree(out.cast()) };
            Some(t)
        } else {
            None
        };
        // SAFETY: allocated by GetNamedSecurityInfoW.
        unsafe { LocalFree(sd) };
        s
    }
}
