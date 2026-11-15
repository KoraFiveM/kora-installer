//! Which program a bare command name really starts.
//!
//! On Windows `CreateProcess` (and Rust's `Command::new("icacls")`) looks in the folder the running
//! program lives in BEFORE `System32` and `PATH`. Kora is typically run from the user's Downloads
//! folder and then re-run as administrator, so a planted `icacls.exe`/`powershell.exe`/`git.exe` next to
//! `kora.exe` would run with administrator rights. [`resolve`] therefore turns bare names into absolute
//! paths: Windows system tools come from `%SystemRoot%\System32`, everything else is searched on `PATH`
//! while skipping relative entries and the folder of the running program. Unix names are returned as is
//! (`sudo` resets `PATH` to a root-owned `secure_path`; see [`unix_sudo`] for the one lookup we do ourselves).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Tools that live in `System32` on every Windows.
const SYSTEM32: &[&str] = &["icacls", "sc", "netsh", "netstat", "tasklist", "taskkill", "schtasks", "msiexec", "whoami", "attrib", "takeown"];

fn stem(program: &str) -> String {
    let l = program.to_ascii_lowercase();
    l.strip_suffix(".exe").unwrap_or(&l).to_string()
}

/// Pure core, testable on any OS: `windows`, `system_root` (`%SystemRoot%`), `path` (`%PATH%`),
/// `exe_dir` (folder of the running program), `exists` (file check).
pub fn resolve_with(program: &str, windows: bool, system_root: Option<&str>, path: Option<&OsString>, exe_dir: Option<&Path>, exists: &dyn Fn(&Path) -> bool) -> OsString {
    if !windows || program.contains(['/', '\\', ':']) {
        return program.into();
    }
    let s = stem(program);
    if let Some(root) = system_root.filter(|r| !r.is_empty()) {
        let sys = Path::new(root).join("System32");
        let candidate = if s == "powershell" { sys.join("WindowsPowerShell").join("v1.0").join("powershell.exe") } else { sys.join(format!("{s}.exe")) };
        if (s == "powershell" || SYSTEM32.contains(&s.as_str())) && exists(&candidate) {
            return candidate.into_os_string();
        }
    }
    let file = if program.to_ascii_lowercase().ends_with(".exe") { program.to_string() } else { format!("{program}.exe") };
    if let Some(p) = path {
        for dir in std::env::split_paths(p) {
            if !dir.is_absolute() || exe_dir.is_some_and(|e| same_dir(e, &dir)) {
                continue;
            }
            let c = dir.join(&file);
            if exists(&c) {
                return c.into_os_string();
            }
        }
    }
    program.into()
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().trim_end_matches(['\\', '/']).to_ascii_lowercase().replace('/', "\\");
    norm(a) == norm(b)
}

/// Absolute program for `program` (see the module docs). Never fails: when nothing better is found the
/// name is returned unchanged.
pub fn resolve(program: &str) -> OsString {
    if !cfg!(windows) {
        return program.into();
    }
    let exe_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf));
    resolve_with(program, true, std::env::var("SystemRoot").ok().as_deref(), std::env::var_os("PATH").as_ref(), exe_dir.as_deref(), &|p| p.is_file())
}

/// `std::process::Command` for `program` with the lookup above applied.
pub fn command(program: &str) -> std::process::Command {
    std::process::Command::new(resolve(program))
}

/// `sudo` from root-owned folders only (not from whatever the user's `PATH` lists first).
pub fn unix_sudo() -> Option<PathBuf> {
    ["/usr/bin/sudo", "/bin/sudo", "/usr/local/bin/sudo", "/usr/sbin/sudo"].iter().map(PathBuf::from).find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    fn exists_in(files: &'static [&'static str]) -> impl Fn(&Path) -> bool {
        move |p: &Path| files.iter().any(|f| p.to_string_lossy().eq_ignore_ascii_case(f))
    }

    #[cfg(windows)]
    #[test]
    fn system_tools_come_from_system32_even_if_planted_next_to_kora() {
        let ex = exists_in(&[r"C:\Windows\System32\icacls.exe", r"C:\Users\u\Downloads\icacls.exe"]);
        let path: OsString = r"C:\Users\u\Downloads;C:\Windows".into();
        let r = resolve_with("icacls", true, Some(r"C:\Windows"), Some(&path), Some(Path::new(r"C:\Users\u\Downloads")), &ex);
        assert_eq!(r, OsString::from(Path::new(r"C:\Windows").join("System32").join("icacls.exe")));
    }

    #[test]
    fn powershell_is_the_windows_powershell_exe() {
        let want = Path::new(r"C:\Windows").join("System32").join("WindowsPowerShell").join("v1.0").join("powershell.exe");
        let w = want.to_string_lossy().into_owned();
        let ex = move |p: &Path| p.to_string_lossy() == w;
        assert_eq!(resolve_with("powershell", true, Some(r"C:\Windows"), None, None, &ex), want.into_os_string());
    }

    #[cfg(windows)]
    #[test]
    fn other_tools_skip_the_exe_folder_and_relative_path_entries() {
        let ex = exists_in(&[r"C:\Users\u\Downloads\git.exe", r"C:\Program Files\Git\cmd\git.exe", r".\git.exe"]);
        let path: OsString = r".;C:\Users\u\Downloads;C:\Program Files\Git\cmd".into();
        let r = resolve_with("git", true, Some(r"C:\Windows"), Some(&path), Some(Path::new(r"C:\Users\u\Downloads")), &ex);
        assert_eq!(r, OsString::from(Path::new(r"C:\Program Files\Git\cmd").join("git.exe")));
    }

    #[test]
    fn explicit_paths_and_unix_names_are_untouched() {
        let never = |_: &Path| true;
        assert_eq!(resolve_with(r"C:\x\mariadb.exe", true, None, None, None, &never), OsString::from(r"C:\x\mariadb.exe"));
        assert_eq!(resolve_with("systemctl", false, None, None, None, &never), OsString::from("systemctl"));
    }
}
