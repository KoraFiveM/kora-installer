//! One shell at a time: a second launch asks the first to show its window, then exits.
//!
//! The first instance holds an exclusive lock on `instance.lock` (the operating system drops it if the
//! process dies, so there are no stale locks) and listens on a loopback port it writes, with a random nonce,
//! into `instance.json` in the same folder (a per-user folder). A later launch that cannot get the lock reads
//! that file and sends one line: `KORA-SHELL1 <nonce> focus [route]`. The nonce stops other local programs
//! that can reach the port but cannot read the user's files from poking it; the only thing the line can do
//! is show the window on a panel route ([`crate::nav::safe_route`]), so the worst a forged line could do is
//! raise a window.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::nav;

const MAGIC: &str = "KORA-SHELL1";

#[derive(Serialize, Deserialize)]
struct Info {
    port: u16,
    nonce: String,
    pid: u32,
}

/// What a second launch can ask for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// Show the window, on this route when given.
    Focus(Option<String>),
}

pub fn encode(nonce: &str, m: &Message) -> String {
    match m {
        Message::Focus(Some(r)) => format!("{MAGIC} {nonce} focus {r}\n"),
        Message::Focus(None) => format!("{MAGIC} {nonce} focus\n"),
    }
}

/// `None` for anything that is not a well-formed, correctly-keyed request.
pub fn decode(line: &str, nonce: &str) -> Option<Message> {
    let mut it = line.trim_end().splitn(4, ' ');
    if it.next()? != MAGIC || it.next()? != nonce || it.next()? != "focus" {
        return None;
    }
    match it.next() {
        None => Some(Message::Focus(None)),
        Some(r) if nav::safe_route(r) => Some(Message::Focus(Some(r.to_string()))),
        Some(_) => None,
    }
}

/// Holds the lock and the listening socket for the life of the process.
pub struct First {
    _lock: File,
    pub listener: TcpListener,
    pub nonce: String,
    dir: PathBuf,
}

impl First {
    /// Check a connection from a later launch; the message to act on, if it was valid.
    pub fn handle(&self, mut stream: TcpStream) -> Option<Message> {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let mut line = String::new();
        // One short line, never more.
        BufReader::new((&mut stream).take(1024)).read_line(&mut line).ok()?;
        let m = decode(&line, &self.nonce);
        let _ = stream.write_all(if m.is_some() { b"ok\n" } else { b"no\n" });
        m
    }
}

impl Drop for First {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.dir.join("instance.json"));
    }
}

pub enum Acquired {
    /// We are the shell: keep this alive.
    First(First),
    /// Another shell runs; it was asked to show its window (`true`), or could not be reached (`false`).
    Existing { told: bool },
}

fn nonce() -> String {
    // The clock and the process id are enough to be unguessable to a program that cannot read this user's files;
    // no random-number dependency is needed for that.
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut x = t as u64 ^ ((std::process::id() as u64) << 32) ^ (t >> 64) as u64;
    let mut out = String::new();
    for _ in 0..4 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        out.push_str(&format!("{x:016x}"));
    }
    out
}

/// Become the first instance, or tell the one that is.
pub fn acquire(dir: &Path, message: &Message) -> std::io::Result<Acquired> {
    std::fs::create_dir_all(dir)?;
    let lock = OpenOptions::new().create(true).truncate(false).read(true).write(true).open(dir.join("instance.lock"))?;
    match lock.try_lock() {
        Ok(()) => {
            let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))?;
            let nonce = nonce();
            let info = Info { port: listener.local_addr()?.port(), nonce: nonce.clone(), pid: std::process::id() };
            let tmp = dir.join("instance.json.tmp");
            std::fs::write(&tmp, serde_json::to_vec(&info).map_err(std::io::Error::other)?)?;
            std::fs::rename(&tmp, dir.join("instance.json"))?;
            Ok(Acquired::First(First { _lock: lock, listener, nonce, dir: dir.to_path_buf() }))
        }
        Err(TryLockError::WouldBlock) => Ok(Acquired::Existing { told: tell(dir, message) }),
        Err(TryLockError::Error(e)) => Err(e),
    }
}

/// Send `message` to the running shell, retrying briefly (it may be a moment from writing its file).
fn tell(dir: &Path, message: &Message) -> bool {
    for _ in 0..10 {
        if let Some(info) = std::fs::read(dir.join("instance.json")).ok().and_then(|b| serde_json::from_slice::<Info>(&b).ok()) {
            let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, info.port));
            if let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_secs(1)) {
                let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
                if s.write_all(encode(&info.nonce, message).as_bytes()).is_ok() {
                    let mut reply = String::new();
                    // Connected: whatever it answers is final (no more waiting).
                    return BufReader::new(s).read_line(&mut reply).is_ok() && reply.trim() == "ok";
                }
            }
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip_and_wrong_keys_or_routes_are_ignored() {
        let n = "abc123";
        for m in [Message::Focus(None), Message::Focus(Some("/servers/a%20b".into()))] {
            assert_eq!(decode(&encode(n, &m), n), Some(m));
        }
        assert_eq!(decode(&encode("other", &Message::Focus(None)), n), None, "wrong nonce");
        assert_eq!(decode("KORA-SHELL1 abc123 quit\n", n), None, "only focus exists");
        assert_eq!(decode("KORA-SHELL1 abc123 focus //evil.example\n", n), None);
        assert_eq!(decode("GET / HTTP/1.1\n", n), None);
        assert_eq!(decode("", n), None);
    }

    #[test]
    fn the_second_launch_reaches_the_first_and_a_dead_one_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let first = match acquire(dir.path(), &Message::Focus(None)).unwrap() {
            Acquired::First(f) => f,
            Acquired::Existing { .. } => panic!("nobody was running"),
        };
        let listener = first.listener.try_clone().unwrap();
        let server = std::thread::spawn(move || {
            let (s, _) = listener.accept().unwrap();
            first.handle(s)
        });
        match acquire(dir.path(), &Message::Focus(Some("/settings".into()))).unwrap() {
            Acquired::Existing { told } => assert!(told, "the first instance answered"),
            Acquired::First(_) => panic!("a second shell must not start"),
        }
        assert_eq!(server.join().unwrap(), Some(Message::Focus(Some("/settings".into()))));
        // `first` was dropped with the thread: the lock is free and the info file gone, so a new shell starts.
        assert!(matches!(acquire(dir.path(), &Message::Focus(None)).unwrap(), Acquired::First(_)));
    }

    #[test]
    fn a_lock_with_nobody_answering_reports_it_could_not_tell() {
        let dir = tempfile::tempdir().unwrap();
        let held = match acquire(dir.path(), &Message::Focus(None)).unwrap() {
            Acquired::First(f) => f,
            _ => panic!(),
        };
        // Nobody calls accept: the connection is queued but never answered.
        match acquire(dir.path(), &Message::Focus(None)).unwrap() {
            Acquired::Existing { told } => assert!(!told),
            Acquired::First(_) => panic!(),
        }
        drop(held);
    }
}
