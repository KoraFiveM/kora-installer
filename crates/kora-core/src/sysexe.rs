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
const SYSTEM32: &[&str] = &["icacls", "sc", "netsh", "netstat", "tasklist", "taskkill", "schtasks", "msiexec", "whoami", "attrib", "takeown", "route"];

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

/// Run `cmd` to the end with a time limit, for programs an installer, a service or a custom action starts: nobody can
/// answer them, so they must end by themselves.
///
/// * the child's input is nothing (a tool that asks a question gets "end of input" instead of waiting for a person);
/// * its output is read on threads, and when the child has gone the readers get [`PIPE_GRACE`] for the last bytes and are
///   then left behind: a grandchild that inherited the output pipe and lives on (a service a tool started, an updater)
///   must not keep us waiting, which `Command::output` does for as long as the grandchild lives;
/// * longer than `limit` the child is killed and the answer is [`std::io::ErrorKind::TimedOut`].
pub fn output_within(mut cmd: std::process::Command, limit: std::time::Duration) -> std::io::Result<std::process::Output> {
    use std::io::Read;
    use std::process::Stdio;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let reader = |pipe: Option<Box<dyn Read + Send>>| {
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        if let Some(mut p) = pipe {
            std::thread::spawn(move || {
                let mut buf = [0u8; 8192];
                // sent in pieces, so what was read before the grace ran out is not lost
                while let Ok(n) = p.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    if tx.send(buf[..n].to_vec()).is_err() {
                        return;
                    }
                }
            });
        }
        rx
    };
    let out_rx = reader(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let err_rx = reader(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let deadline = Instant::now() + limit;
    let status = loop {
        if let Some(st) = child.try_wait()? {
            break st;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, format!("{} did not finish within {} seconds and was stopped", cmd.get_program().to_string_lossy(), limit.as_secs())));
        }
        std::thread::sleep(Duration::from_millis(15));
    };
    let grace = Instant::now() + PIPE_GRACE;
    let collect = |rx: mpsc::Receiver<Vec<u8>>| {
        let mut v = Vec::new();
        while let Ok(piece) = rx.recv_timeout(grace.saturating_duration_since(Instant::now())) {
            v.extend_from_slice(&piece);
        }
        v
    };
    Ok(std::process::Output { status, stdout: collect(out_rx), stderr: collect(err_rx) })
}

/// How long the last output of a finished child is waited for ([`output_within`]).
pub const PIPE_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

/// `sudo` from root-owned folders only (not from whatever the user's `PATH` lists first).
pub fn unix_sudo() -> Option<PathBuf> {
    ["/usr/bin/sudo", "/bin/sudo", "/usr/local/bin/sudo", "/usr/sbin/sudo"].iter().map(PathBuf::from).find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// `sh -c script` / `cmd /C script`.
    fn shell(script: &str) -> std::process::Command {
        let mut c = std::process::Command::new(if cfg!(windows) { "cmd" } else { "sh" });
        c.args([if cfg!(windows) { "/C" } else { "-c" }, script]);
        c
    }

    #[test]
    fn a_tool_that_never_ends_is_stopped_at_the_limit() {
        let t = Instant::now();
        let r = output_within(shell(if cfg!(windows) { "ping -n 60 127.0.0.1 > nul" } else { "sleep 60" }), Duration::from_millis(400));
        assert_eq!(r.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
        assert!(t.elapsed() < Duration::from_secs(20), "stopped in {:?}", t.elapsed());
    }

    #[test]
    fn a_tool_that_asks_a_question_gets_the_end_of_its_input() {
        // `more` / `cat` wait for input for ever when inherited from a console nobody sees
        let r = output_within(shell(if cfg!(windows) { "more" } else { "cat" }), Duration::from_secs(60)).unwrap();
        assert!(r.status.success() || cfg!(windows));
    }

    #[test]
    fn a_grandchild_holding_the_output_pipe_does_not_keep_us_waiting() {
        let t = Instant::now();
        let script = if cfg!(windows) { "start /B ping -n 40 127.0.0.1 & echo hi" } else { "sleep 40 & echo hi" };
        let r = output_within(shell(script), Duration::from_secs(60)).unwrap();
        assert!(String::from_utf8_lossy(&r.stdout).contains("hi"));
        assert!(t.elapsed() < Duration::from_secs(15), "waited {:?} for the grandchild", t.elapsed());
    }

    #[test]
    fn output_and_exit_code_come_back() {
        let r = output_within(shell(if cfg!(windows) { "echo out& echo err 1>&2& exit 3" } else { "echo out; echo err >&2; exit 3" }), Duration::from_secs(60)).unwrap();
        assert_eq!(r.status.code(), Some(3));
        assert!(String::from_utf8_lossy(&r.stdout).contains("out") && String::from_utf8_lossy(&r.stderr).contains("err"));
    }

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
