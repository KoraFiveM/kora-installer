//! Which account this program runs as, for the permission lists (`icacls`) of files and folders only that person may open.
//!
//! The `USERNAME` environment variable says who a program was STARTED with, not who it is: `runas /env`, a scheduled
//! task, `Start-Process -Credential` in PowerShell 7 and some service wrappers hand a program another account's
//! environment. An access list made from it grants the wrong person and locks the real user out of their own file: the
//! panel then could never publish its sign-in record (found on a real machine by the standard-account panel test).
//! Windows asks the process token instead; elsewhere the variable is all there is (and nothing uses it).

/// `DOMAIN\name` of the account this process runs as.
pub fn name() -> Option<String> {
    choose(token_name(), std::env::var("USERNAME").ok())
}

/// The token's answer wins; the environment is only the fallback when Windows cannot say.
pub fn choose(token: Option<String>, env: Option<String>) -> Option<String> {
    token.filter(|t| !t.is_empty()).or_else(|| env.filter(|e| !e.is_empty()))
}

#[cfg(windows)]
fn token_name() -> Option<String> {
    #[link(name = "secur32")]
    extern "system" {
        fn GetUserNameExW(format: i32, buffer: *mut u16, size: *mut u32) -> u8;
    }
    const NAME_SAM_COMPATIBLE: i32 = 2;
    let mut buf = vec![0u16; 512];
    let mut len = buf.len() as u32;
    // SAFETY: a buffer of `len` UTF-16 units and a valid size pointer; on success `len` is the length without the NUL.
    let ok = unsafe { GetUserNameExW(NAME_SAM_COMPATIBLE, buf.as_mut_ptr(), &mut len) };
    (ok != 0 && len > 0).then(|| String::from_utf16_lossy(&buf[..len as usize]))
}

#[cfg(not(windows))]
fn token_name() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_account_the_system_reports_wins_over_a_stale_environment() {
        assert_eq!(choose(Some(r"PC\korastd".into()), Some("Jamie".into())).as_deref(), Some(r"PC\korastd"));
        assert_eq!(choose(None, Some("Jamie".into())).as_deref(), Some("Jamie"), "no answer from Windows: the variable");
        assert_eq!(choose(Some(String::new()), Some("Jamie".into())).as_deref(), Some("Jamie"));
        assert_eq!(choose(None, Some(String::new())), None);
        assert_eq!(choose(None, None), None);
    }
}
