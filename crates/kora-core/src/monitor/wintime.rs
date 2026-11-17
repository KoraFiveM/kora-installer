//! When did the game server of a folder start, on Windows? The pure part: turning what Windows says into a unix time and choosing
//! the process (tested on any system; the calls into Windows are in `probe::os`).
//!
//! The documented API is used, not a command's text: the process list comes from `CreateToolhelp32Snapshot`, a process's program
//! from `QueryFullProcessImageNameW`, and its creation time from `GetProcessTimes` (a FILETIME: 100-nanosecond steps since 1601).
//! Only processes whose program is called `FXServer.exe` and lives inside the server's own folder count, so another server's
//! FXServer (or one somewhere else) never decides whether THIS server restarted.

use std::path::Path;

/// 100-nanosecond FILETIME steps between 1601-01-01 and 1970-01-01.
const EPOCH_DIFF_SECS: u64 = 11_644_473_600;

/// A FILETIME (`dwHighDateTime << 32 | dwLowDateTime`) as seconds since 1970. `None` for zero (a process that has no time) and for
/// anything before 1970.
pub fn filetime_to_unix(ft: u64) -> Option<u64> {
    (ft != 0).then(|| ft / 10_000_000).and_then(|s| s.checked_sub(EPOCH_DIFF_SECS))
}

/// Windows folders compare without regard to case and treat `/` and `\` alike; a `\\?\` prefix means the same folder.
fn norm(s: &str) -> String {
    let s = s.strip_prefix(r"\\?\").unwrap_or(s);
    s.replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

/// Is `image` (a process's full program path) the `FXServer.exe` of the server in `dir`?
pub fn is_fxserver_of(image: &str, dir: &Path) -> bool {
    let (img, base) = (norm(image), norm(&dir.to_string_lossy()));
    !base.is_empty() && img.rsplit('\\').next() == Some("fxserver.exe") && img.starts_with(&format!("{base}\\"))
}

/// The server's start: the oldest creation time of its processes (a restart makes new ones; a child never starts before the parent).
pub fn oldest(times: impl IntoIterator<Item = Option<u64>>) -> Option<u64> {
    times.into_iter().flatten().min()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_filetime_becomes_a_unix_time() {
        // 2026-11-17 09:50:00 UTC
        assert_eq!(filetime_to_unix((1_794_909_000 + EPOCH_DIFF_SECS) * 10_000_000 + 1234), Some(1_794_909_000));
        assert_eq!(filetime_to_unix(EPOCH_DIFF_SECS * 10_000_000), Some(0));
        assert_eq!(filetime_to_unix(0), None, "no time");
        assert_eq!(filetime_to_unix(10_000_000), None, "1601: not a start time");
    }

    #[test]
    fn only_the_fxserver_inside_this_servers_folder_counts() {
        let dir = Path::new(r"C:\Users\Jo\kora\Main Server");
        assert!(is_fxserver_of(r"C:\Users\Jo\kora\Main Server\artifacts\FXServer.exe", dir));
        assert!(is_fxserver_of(r"c:/users/jo/kora/main server/artifacts/fxserver.EXE", dir), "case and slashes do not matter");
        assert!(is_fxserver_of(r"\\?\C:\Users\Jo\kora\Main Server\FXServer.exe", dir));
        assert!(is_fxserver_of(r"C:\Users\Jo\kora\Main Server\FXServer.exe", Path::new(r"C:\Users\Jo\kora\Main Server\")));
        // another server whose folder starts the same, another program, another place
        assert!(!is_fxserver_of(r"C:\Users\Jo\kora\Main Server 2\FXServer.exe", dir));
        assert!(!is_fxserver_of(r"C:\Users\Jo\kora\Main Server\artifacts\node.exe", dir));
        assert!(!is_fxserver_of(r"C:\Program Files\FXServer\FXServer.exe", dir));
        assert!(!is_fxserver_of(r"C:\Users\Jo\kora\Main Server\notFXServer.exe", dir));
        assert!(!is_fxserver_of(r"C:\x\FXServer.exe", Path::new("")));
    }

    #[test]
    fn the_server_started_when_its_oldest_process_did() {
        assert_eq!(oldest([Some(300), None, Some(100), Some(200)]), Some(100));
        assert_eq!(oldest([None, None]), None);
        assert_eq!(oldest(std::iter::empty()), None);
    }
}
