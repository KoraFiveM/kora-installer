//! What the sampler asks the machine. Everything that touches the OS or the network sits behind
//! [`Probe`], so the sampler and the alert rules are tested with a fake that never leaves the process.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::installer::registry::ServerEntry;

/// Players as FXServer reports them on its local `players.json` / `info.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameInfo {
    pub players: u32,
    pub max_players: Option<u32>,
}

/// CPU and memory of the server's process tree.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProcUsage {
    /// Percent of the whole machine's CPU capacity since the previous sample (`None` for the first one).
    pub cpu_pct: Option<f32>,
    pub rss_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskUsage {
    pub free: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemUsage {
    pub total: u64,
    pub available: u64,
}

pub trait Probe: Send + Sync {
    /// Is the server folder still there?
    fn dir_exists(&self, dir: &Path) -> bool;
    /// Players now; `None` when the server does not answer.
    fn game_info(&self, game_port: u16) -> Option<GameInfo>;
    /// CPU and RAM of the server and everything it started; `None` when not measurable here.
    fn process(&self, e: &ServerEntry) -> Option<ProcUsage>;
    /// Free and total bytes of the volume holding `dir`.
    fn disk(&self, dir: &Path) -> Option<DiskUsage>;
    /// RAM of the whole machine.
    fn memory(&self) -> Option<MemUsage>;
}

// ------------------------------------------------------------------------- local HTTP

const HTTP_TIMEOUT: Duration = Duration::from_secs(2);
const HTTP_MAX: u64 = 2 * 1024 * 1024;

/// A plain `GET` on `127.0.0.1:<port><path>` over std sockets (FXServer serves these two files locally).
/// Returns the body of a 200 answer.
pub(crate) fn http_get_local(port: u16, path: &str) -> Option<String> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut s = TcpStream::connect_timeout(&addr, HTTP_TIMEOUT).ok()?;
    s.set_read_timeout(Some(HTTP_TIMEOUT)).ok()?;
    s.set_write_timeout(Some(HTTP_TIMEOUT)).ok()?;
    // One write, so the whole request arrives together.
    let request = format!("GET {path} HTTP/1.0\r\nHost: 127.0.0.1\r\nUser-Agent: kora-monitor\r\nConnection: close\r\n\r\n");
    s.write_all(request.as_bytes()).ok()?;
    let mut raw = Vec::new();
    // A server that closes with part of the request unread resets the connection after it has sent
    // its answer: keep what arrived, and only give up when nothing did.
    if s.take(HTTP_MAX).read_to_end(&mut raw).is_err() && raw.is_empty() {
        return None;
    }
    let text = String::from_utf8_lossy(&raw);
    let (head, body) = text.split_once("\r\n\r\n")?;
    let status = head.lines().next()?.split_whitespace().nth(1)?;
    (status == "200").then(|| body.to_string())
}

/// `players.json` is an array of players; `info.json` has `vars.sv_maxClients` (a string).
pub(crate) fn parse_game_info(players_json: &str, info_json: Option<&str>) -> Option<GameInfo> {
    let players = serde_json::from_str::<serde_json::Value>(players_json).ok()?;
    let players = u32::try_from(players.as_array()?.len()).ok()?;
    let max_players = info_json
        .and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok())
        .and_then(|v| {
            let m = &v["vars"]["sv_maxClients"];
            m.as_u64()
                .or_else(|| m.as_str().and_then(|s| s.trim().parse().ok()))
                .and_then(|n| u32::try_from(n).ok())
        });
    Some(GameInfo {
        players,
        max_players,
    })
}

// ------------------------------------------------------------------------- the real thing

/// Reads the real machine. CPU needs the previous reading, so this keeps one per server.
#[derive(Default)]
pub struct SystemProbe {
    last_cpu: Mutex<HashMap<String, (f64, Instant)>>,
}

impl SystemProbe {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Probe for SystemProbe {
    fn dir_exists(&self, dir: &Path) -> bool {
        dir.is_dir()
    }

    fn game_info(&self, port: u16) -> Option<GameInfo> {
        let players = http_get_local(port, "/players.json")?;
        let info = http_get_local(port, "/info.json");
        parse_game_info(&players, info.as_deref())
    }

    fn process(&self, e: &ServerEntry) -> Option<ProcUsage> {
        let (cpu_secs, rss) = os::process_tree(&e.dir)?;
        let now = Instant::now();
        let mut last = self
            .last_cpu
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let cpu_pct = match last.insert(e.name.clone(), (cpu_secs, now)) {
            Some((prev, at)) if cpu_secs >= prev => {
                let dt = now.duration_since(at).as_secs_f64();
                let cores = std::thread::available_parallelism().map_or(1, usize::from) as f64;
                (dt > 0.5)
                    .then(|| (((cpu_secs - prev) / dt / cores) * 100.0).clamp(0.0, 100.0) as f32)
            }
            _ => None,
        };
        Some(ProcUsage {
            cpu_pct,
            rss_bytes: rss,
        })
    }

    fn disk(&self, dir: &Path) -> Option<DiskUsage> {
        use crate::space::DiskInfo;
        let free = crate::space::SystemDisk.free_bytes(dir).ok()?;
        let total = os::disk_total(dir)?;
        Some(DiskUsage { free, total })
    }

    fn memory(&self) -> Option<MemUsage> {
        os::memory()
    }
}

#[cfg(unix)]
mod os {
    use super::*;
    use std::collections::HashSet;
    use std::path::PathBuf;

    pub fn disk_total(p: &Path) -> Option<u64> {
        use std::os::unix::ffi::OsStrExt;
        let mut p = p.to_path_buf();
        while !p.exists() {
            p = p.parent()?.to_path_buf();
        }
        let c = std::ffi::CString::new(p.as_os_str().as_bytes()).ok()?;
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: `c` is a valid NUL-terminated path and `st` a properly sized out-parameter.
        if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
            return None;
        }
        Some((st.f_blocks as u64).saturating_mul(st.f_frsize as u64))
    }

    pub fn memory() -> Option<MemUsage> {
        parse_meminfo(&std::fs::read_to_string("/proc/meminfo").ok()?)
    }

    pub fn parse_meminfo(text: &str) -> Option<MemUsage> {
        let kb = |key: &str| -> Option<u64> {
            text.lines()
                .find_map(|l| {
                    l.strip_prefix(key)?
                        .trim()
                        .trim_end_matches("kB")
                        .trim()
                        .parse::<u64>()
                        .ok()
                })
                .map(|v| v * 1024)
        };
        Some(MemUsage {
            total: kb("MemTotal:")?,
            available: kb("MemAvailable:")?,
        })
    }

    /// `(utime + stime in ticks, ppid, rss pages)` from the text of `/proc/<pid>/stat`.
    pub fn parse_stat(text: &str) -> Option<(u64, u32, u64)> {
        // The command name is in brackets and may contain spaces and brackets: split after the last one.
        let rest = &text[text.rfind(')')? + 1..];
        let f: Vec<&str> = rest.split_whitespace().collect();
        let ppid = f.get(1)?.parse().ok()?;
        let ticks = f.get(11)?.parse::<u64>().ok()? + f.get(12)?.parse::<u64>().ok()?;
        let rss = f.get(21)?.parse().ok()?;
        Some((ticks, ppid, rss))
    }

    fn pids() -> Vec<u32> {
        std::fs::read_dir("/proc")
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().to_str()?.parse().ok())
            .collect()
    }

    /// Does this process belong to the server folder? Its program or first argument lives inside it.
    fn belongs(pid: u32, dir: &Path) -> bool {
        if std::fs::read_link(format!("/proc/{pid}/exe")).is_ok_and(|p| p.starts_with(dir)) {
            return true;
        }
        let Ok(raw) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
            return false;
        };
        let first = raw.split(|b| *b == 0).next().unwrap_or(&[]);
        PathBuf::from(String::from_utf8_lossy(first).into_owned()).starts_with(dir)
    }

    pub fn process_tree(dir: &Path) -> Option<(f64, u64)> {
        let me = std::process::id();
        let mut table: HashMap<u32, (u64, u32, u64)> = HashMap::new();
        for pid in pids() {
            if let Some(st) = std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .ok()
                .and_then(|t| parse_stat(&t))
            {
                table.insert(pid, st);
            }
        }
        let mut roots: Vec<u32> = Vec::new();
        // A server Kora started itself keeps its pid in `.kora/server.pid`.
        if let Some(p) = std::fs::read_to_string(dir.join(".kora").join("server.pid"))
            .ok()
            .and_then(|t| t.trim().parse::<u32>().ok())
        {
            if table.contains_key(&p) {
                roots.push(p);
            }
        }
        if roots.is_empty() {
            roots = table
                .keys()
                .copied()
                .filter(|p| *p != me && belongs(*p, dir))
                .collect();
        }
        if roots.is_empty() {
            return None;
        }
        let mut set: HashSet<u32> = roots.iter().copied().collect();
        // Add children until nothing new turns up (the table is small).
        loop {
            let before = set.len();
            for (pid, (_, ppid, _)) in &table {
                if set.contains(ppid) && *pid != me {
                    set.insert(*pid);
                }
            }
            if set.len() == before {
                break;
            }
        }
        let tick = unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as f64;
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(1) as u64;
        let (ticks, rss) = set
            .iter()
            .filter_map(|p| table.get(p))
            .fold((0u64, 0u64), |(t, r), (ti, _, rs)| (t + ti, r + rs));
        Some((ticks as f64 / tick, rss * page))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn stat_with_spaces_and_brackets_in_the_name() {
            let t = "1234 (FX (server) x) S 77 1 1 0 -1 4194560 100 0 0 0 150 50 0 0 20 0 5 0 99 1000000 2500 18446744073709551615";
            assert_eq!(parse_stat(t), Some((200, 77, 2500)));
        }

        #[test]
        fn meminfo_is_read_in_bytes() {
            let m = parse_meminfo(
                "MemTotal:       16000000 kB\nMemFree: 1 kB\nMemAvailable:    4000000 kB\n",
            )
            .unwrap();
            assert_eq!(
                (m.total, m.available),
                (16_000_000 * 1024, 4_000_000 * 1024)
            );
        }

        #[test]
        fn a_process_started_from_the_folder_is_found_with_its_children() {
            let dir = tempfile::tempdir().unwrap();
            let sleep = dir.path().join("fxserver-stand-in");
            std::fs::copy("/bin/sleep", &sleep)
                .or_else(|_| std::fs::copy("/usr/bin/sleep", &sleep))
                .unwrap();
            let mut child = std::process::Command::new(&sleep).arg("5").spawn().unwrap();
            let found = process_tree(dir.path());
            let _ = child.kill();
            let _ = child.wait();
            let (cpu, rss) = found.expect("the stand-in belongs to the folder");
            assert!(cpu >= 0.0 && rss > 0);
            assert!(
                process_tree(dir.path()).is_none(),
                "nothing left once it is gone"
            );
        }
    }
}

/// Windows: process trees are not measured (no extra dependency for it); memory and disk are.
#[cfg(windows)]
mod os {
    use super::*;

    pub fn process_tree(_dir: &Path) -> Option<(f64, u64)> {
        None
    }

    pub fn disk_total(p: &Path) -> Option<u64> {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        extern "system" {
            fn GetDiskFreeSpaceExW(
                dir: *const u16,
                available: *mut u64,
                total: *mut u64,
                free: *mut u64,
            ) -> i32;
        }
        let mut p = p.to_path_buf();
        while !p.exists() {
            p = p.parent()?.to_path_buf();
        }
        let wide: Vec<u16> = p
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut total = 0u64;
        // SAFETY: `wide` is NUL-terminated; the two optional out-parameters may be null.
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                wide.as_ptr(),
                std::ptr::null_mut(),
                &mut total,
                std::ptr::null_mut(),
            )
        };
        (ok != 0).then_some(total)
    }

    #[repr(C)]
    struct MemoryStatusEx {
        length: u32,
        memory_load: u32,
        total_phys: u64,
        avail_phys: u64,
        total_page_file: u64,
        avail_page_file: u64,
        total_virtual: u64,
        avail_virtual: u64,
        avail_extended_virtual: u64,
    }

    pub fn memory() -> Option<MemUsage> {
        #[link(name = "kernel32")]
        extern "system" {
            fn GlobalMemoryStatusEx(buf: *mut MemoryStatusEx) -> i32;
        }
        let mut m = MemoryStatusEx {
            length: std::mem::size_of::<MemoryStatusEx>() as u32,
            memory_load: 0,
            total_phys: 0,
            avail_phys: 0,
            total_page_file: 0,
            avail_page_file: 0,
            total_virtual: 0,
            avail_virtual: 0,
            avail_extended_virtual: 0,
        };
        // SAFETY: `m` is a correctly laid out MEMORYSTATUSEX with `length` set.
        (unsafe { GlobalMemoryStatusEx(&mut m) } != 0).then_some(MemUsage {
            total: m.total_phys,
            available: m.avail_phys,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn serve_once(body: &'static str, status: &'static str) -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for _ in 0..2 {
                if let Ok((mut c, _)) = l.accept() {
                    let mut buf = [0u8; 512];
                    let _ = c.read(&mut buf);
                    let _ = write!(
                        c,
                        "HTTP/1.0 {status}\r\nContent-Type: application/json\r\n\r\n{body}"
                    );
                }
            }
        });
        port
    }

    #[test]
    fn local_http_reads_a_200_body() {
        let port = serve_once(r#"[{"id":1},{"id":2}]"#, "200 OK");
        let body = http_get_local(port, "/players.json").unwrap();
        assert_eq!(
            parse_game_info(&body, Some(r#"{"vars":{"sv_maxClients":"48"}}"#)),
            Some(GameInfo {
                players: 2,
                max_players: Some(48)
            })
        );
    }

    #[test]
    fn local_http_ignores_other_statuses_and_closed_ports() {
        let port = serve_once("nope", "404 Not Found");
        assert_eq!(http_get_local(port, "/players.json"), None);
        let closed = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        assert_eq!(http_get_local(closed, "/players.json"), None);
    }

    #[test]
    fn game_info_survives_missing_or_odd_info() {
        assert_eq!(
            parse_game_info("[]", None),
            Some(GameInfo {
                players: 0,
                max_players: None
            })
        );
        assert_eq!(
            parse_game_info("[]", Some("{")),
            Some(GameInfo {
                players: 0,
                max_players: None
            })
        );
        assert_eq!(parse_game_info(r#"{"not":"a list"}"#, None), None);
        assert_eq!(
            parse_game_info("[1]", Some(r#"{"vars":{"sv_maxClients":64}}"#))
                .unwrap()
                .max_players,
            Some(64)
        );
    }
}
