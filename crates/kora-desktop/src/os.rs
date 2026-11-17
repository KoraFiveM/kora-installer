//! The operating-system calls: everything that is not a decision (those are in `kora-shell`).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use kora_shell::autostart;
use kora_shell::platform;

/// No console window flashing when a child is started from the windowed program (Windows).
pub fn hidden(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

/// The program is built for the "windows" subsystem (no console). When it is started from a terminal, print there.
pub fn attach_console() {
    #[cfg(windows)]
    unsafe {
        // ATTACH_PARENT_PROCESS = u32::MAX
        windows_sys::Win32::System::Console::AttachConsole(u32::MAX);
    }
}

pub fn env(k: &str) -> Option<String> {
    std::env::var(k).ok()
}

/// `reg.exe` by full path (never searched for on PATH).
fn reg_exe() -> PathBuf {
    let root = env("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    PathBuf::from(format!(r"{root}\System32\reg.exe"))
}

/// Run `reg.exe`; its output (both streams) as text, or `None` when it could not be started.
fn reg(args: &[String]) -> Option<String> {
    let mut c = Command::new(reg_exe());
    c.args(args).stdin(Stdio::null());
    hidden(&mut c);
    let o = c.output().ok()?;
    Some(format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
}

pub fn autostart_enabled(exe: &Path) -> bool {
    if cfg!(windows) {
        let run = reg(&autostart::reg_query_args()).unwrap_or_default();
        let approved = reg(&autostart::reg_query_approved_args()).unwrap_or_default();
        autostart::windows_enabled(&run, &approved, exe)
    } else {
        autostart::xdg_path(&env).and_then(|p| std::fs::read_to_string(p).ok()).is_some_and(|t| autostart::xdg_is_ours(&t))
    }
}

pub fn set_autostart(enable: bool, exe: &Path) -> Result<(), String> {
    if cfg!(windows) {
        let args = if enable { autostart::reg_add_args(exe) } else { autostart::reg_delete_args() };
        let out = reg(&args).ok_or("could not run reg.exe")?;
        // Deleting a value that is not there is fine.
        if enable && !out.contains("successfully") {
            return Err(out.trim().to_string());
        }
        Ok(())
    } else {
        let path = autostart::xdg_path(&env).ok_or("no home folder")?;
        if enable {
            std::fs::create_dir_all(path.parent().ok_or("no folder")?).map_err(|e| e.to_string())?;
            std::fs::write(&path, autostart::xdg_entry(exe)).map_err(|e| e.to_string())
        } else {
            match std::fs::read_to_string(&path) {
                Ok(t) if autostart::xdg_is_ours(&t) => std::fs::remove_file(&path).map_err(|e| e.to_string()),
                _ => Ok(()),
            }
        }
    }
}

/// Windows: make notifications show as "Kora" (see `platform::AUMID`). Best effort.
pub fn register_toast_identity(icon_png: Option<&Path>) {
    if cfg!(windows) {
        for args in platform::aumid_register_args(icon_png) {
            let _ = reg(&args);
        }
    }
}

pub fn unregister_toast_identity() {
    if cfg!(windows) {
        let _ = reg(&platform::aumid_unregister_args());
    }
}

/// Open an `http(s)` address in the default browser. The caller has already checked it with `kora_shell::nav`.
pub fn open_in_browser(url: &str) {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return;
    }
    let mut c = if cfg!(windows) {
        // `rundll32 url.dll,FileProtocolHandler <url>`: no shell parsing of the address.
        let mut c = Command::new(format!(r"{}\System32\rundll32.exe", env("SystemRoot").unwrap_or_else(|| r"C:\Windows".into())));
        c.arg("url.dll,FileProtocolHandler").arg(url);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    hidden(&mut c);
    let _ = c.spawn();
}

/// Is the web view this program needs installed? The version, or why not.
pub fn webview_version() -> Result<String, String> {
    wry::webview_version().map_err(|e| e.to_string())
}

/// Minutes after local midnight (for quiet hours).
pub fn local_minute_of_day() -> u16 {
    #[cfg(windows)]
    {
        let mut t: windows_sys::Win32::Foundation::SYSTEMTIME = unsafe { std::mem::zeroed() };
        unsafe { windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut t) };
        t.wHour * 60 + t.wMinute
    }
    #[cfg(not(windows))]
    {
        let out = Command::new("date").arg("+%H:%M").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
        match out.split_once(':') {
            Some((h, m)) => h.parse::<u16>().unwrap_or(0) * 60 + m.parse::<u16>().unwrap_or(0),
            None => 0,
        }
    }
}

/// Run a fixed administrator command with Windows' prompt IN FRONT of the app.
///
/// What decides where the prompt appears: Windows lets a process bring a window forward only when it is the
/// foreground process (or was allowed to). A background process (the panel, started at sign-in) asks for
/// elevation with no foreground right, so the consent prompt opens behind the other windows as a flashing
/// taskbar shield. `ShellExecuteExW` with the verb `runas`, an OWNER window (`hwnd`) that is the foreground
/// window of this foreground app, makes the prompt belong to that window and appear above it.
#[cfg(windows)]
pub fn elevate(hwnd: isize, command: &platform::Command) -> Result<(), String> {
    use std::os::raw::c_void;
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_CANCELLED};
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW};
    use windows_sys::Win32::UI::WindowsAndMessaging::{SetForegroundWindow, SW_SHOWNORMAL};

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }
    let verb = wide("runas");
    let file = wide(&command.program.to_string_lossy());
    let params = wide(&platform::quote_args(&command.args));
    // SAFETY: the structure is zeroed then filled; the three strings outlive the call.
    unsafe {
        let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOASYNC;
        info.hwnd = hwnd as *mut c_void;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpParameters = params.as_ptr();
        info.nShow = SW_SHOWNORMAL;
        SetForegroundWindow(info.hwnd);
        if ShellExecuteExW(&mut info) == 0 {
            let e = GetLastError();
            return Err(if e == ERROR_CANCELLED { "Windows asked for permission and it was not given.".to_string() } else { format!("Windows could not start it (error {e}).") });
        }
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn elevate(_hwnd: isize, _command: &platform::Command) -> Result<(), String> {
    Err("This is only available on Windows.".into())
}

/// Run a command to the end without a window, ignoring its output. `true` when it succeeded.
pub fn run_quiet(c: &platform::Command) -> bool {
    let mut cmd = Command::new(&c.program);
    cmd.args(&c.args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    hidden(&mut cmd);
    cmd.status().map(|s| s.success()).unwrap_or(false)
}
