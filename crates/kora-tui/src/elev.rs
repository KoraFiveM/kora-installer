//! The one place that talks to the elevation API. Everything else in this crate goes through
//! here (it re-exports `kora_core::elevate`).

use std::ffi::OsString;

pub use kora_core::elevate::{admin_reasons, is_elevated, relaunch_elevated, ElevateError};

// ---- helpers used by the screens ----

/// What the screens call to restart with permission (fake-able in tests).
pub type Elevator = Box<dyn FnMut(&[OsString]) -> Result<(), ElevateError>>;

/// The real thing: restart this program with permission.
pub fn system_elevator() -> Elevator {
    Box::new(relaunch_elevated)
}

/// The arguments this program was started with (what a restart repeats).
pub fn original_args() -> Vec<OsString> {
    std::env::args_os().skip(1).collect()
}

/// Can this computer offer a permission prompt? Windows always; Unix when `sudo` exists.
pub fn can_offer() -> bool {
    if cfg!(windows) {
        return true;
    }
    cfg!(unix)
        && std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join("sudo").is_file()))
}

/// Do-it-yourself steps, shown when the automatic restart is not possible.
pub fn manual_steps(windows: bool) -> String {
    if windows {
        "To do it yourself:\n1. Close this window.\n2. Right-click Windows Terminal (or kora.exe) and choose Run as administrator.\n3. Run kora again.".into()
    } else {
        "To do it yourself:\n1. Close this window.\n2. Run: sudo kora install".into()
    }
}

/// Plain message for a failed restart.
pub fn failure_note(e: &ElevateError, windows: bool) -> String {
    match e {
        ElevateError::Declined => "No problem - nothing was changed. You can try again, or choose the limited setup.".into(),
        ElevateError::Failed(why) => format!("Kora could not ask for permission: {why}.\n{}", manual_steps(windows)),
        ElevateError::Unsupported => format!("This computer can't show a permission prompt from here.\n{}", manual_steps(windows)),
    }
}
