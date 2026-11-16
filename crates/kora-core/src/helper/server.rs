//! The helper's server side: socket, peer check, request handling, and the real executor that
//! spawns the `kora` child. Linux only; on other systems [`run`] says it is not available.

use std::io;

/// Checks the verb's arguments with the real command-line definition (injected by `kora-cli`, which
/// owns the clap `Cli`). `Err` text is shown to the user.
pub trait Validator: Send + Sync {
    fn validate(&self, verb: &str, args: &[String]) -> Result<(), String>;
}

impl<F: Fn(&str, &[String]) -> Result<(), String> + Send + Sync> Validator for F {
    fn validate(&self, verb: &str, args: &[String]) -> Result<(), String> {
        self(verb, args)
    }
}

/// Runs the child. `on_line(stream, line)` is called for each output line; returns the exit code.
pub trait Executor: Send + Sync {
    fn run(&self, argv: &[String], env: &[(String, String)], on_line: &mut dyn FnMut(&str, &str)) -> io::Result<i32>;
}

/// Could a file with this owner and mode be changed by someone other than root?
pub fn writable_by_others(owner_uid: u32, mode: u32) -> bool {
    owner_uid != 0 || mode & 0o022 != 0
}

/// Read the installing user's record.
pub fn load_config(path: &std::path::Path) -> Result<crate::helper::HelperConfig, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{} is not valid: {e}", path.display()))
}

#[cfg(unix)]
pub use unix::*;

#[cfg(unix)]
mod unix {
    use super::*;
    use crate::helper::{check_request, mask_secrets, peer_allowed, HelperConfig, Message, Request, MAX_REQUEST_BYTES, PING};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    const MAX_CONNECTIONS: usize = 16;
    const LINE_CAP: usize = 8 * 1024;

    /// The uid of the process at the other end of `s` (`SO_PEERCRED`).
    pub fn peer_uid_of(s: &UnixStream) -> io::Result<u32> {
        use std::os::fd::AsRawFd;
        let mut cred: libc::ucred = libc::ucred { pid: 0, uid: u32::MAX, gid: u32::MAX };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        // SAFETY: `cred` and `len` are valid for the call and `len` is its real size.
        let r = unsafe { libc::getsockopt(s.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, (&mut cred as *mut libc::ucred).cast(), &mut len) };
        if r != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(cred.uid)
    }

    type PeerFn = dyn Fn(&UnixStream) -> io::Result<u32> + Send + Sync;

    pub struct Server {
        listener: UnixListener,
        allowed_uid: u32,
        validator: Arc<dyn Validator>,
        executor: Arc<dyn Executor>,
        peer: Box<PeerFn>,
        run_lock: Mutex<()>,
        active: AtomicUsize,
    }

    impl Server {
        /// Bind `path` (replacing a stale socket file). `owner` = (uid, gid) to chown the socket to;
        /// `None` leaves it as the creating user (tests). The mode is set to `0660` straight after binding.
        pub fn bind(path: &Path, allowed_uid: u32, owner: Option<(u32, u32)>, validator: Arc<dyn Validator>, executor: Arc<dyn Executor>) -> io::Result<Server> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let _ = std::fs::remove_file(path);
            // No umask change: it is process-wide and would race with other threads creating files.
            // The socket is tightened straight after binding; the peer-uid check on every connection
            // is the real control, so the brief window before the chmod grants nothing.
            let listener = UnixListener::bind(path)?;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o660))?;
            if let Some((uid, gid)) = owner {
                let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).map_err(|_| io::Error::other("bad socket path"))?;
                // SAFETY: valid NUL-terminated path.
                if unsafe { libc::chown(c.as_ptr(), uid, gid) } != 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            listener.set_nonblocking(true)?;
            Ok(Server { listener, allowed_uid, validator, executor, peer: Box::new(peer_uid_of), run_lock: Mutex::new(()), active: AtomicUsize::new(0) })
        }

        /// Replace the peer-credential lookup (tests: pretend to be another user).
        pub fn with_peer_fn(mut self, f: impl Fn(&UnixStream) -> io::Result<u32> + Send + Sync + 'static) -> Self {
            self.peer = Box::new(f);
            self
        }

        /// Accept connections until `stop` is set; each connection is handled on its own thread.
        pub fn serve(self: &Arc<Self>, stop: &AtomicBool) {
            while !stop.load(Ordering::Relaxed) {
                match self.listener.accept() {
                    Ok((stream, _)) => {
                        if self.active.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                            self.active.fetch_sub(1, Ordering::SeqCst);
                            continue; // dropped: too many at once
                        }
                        let me = Arc::clone(self);
                        std::thread::spawn(move || {
                            let _ = stream.set_nonblocking(false);
                            me.handle(stream);
                            me.active.fetch_sub(1, Ordering::SeqCst);
                        });
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(25)),
                    Err(_) => std::thread::sleep(Duration::from_millis(200)),
                }
            }
        }

        /// One connection: peer check, read one request line, answer.
        pub fn handle(&self, stream: UnixStream) {
            let mut out = match stream.try_clone() {
                Ok(o) => o,
                Err(_) => return,
            };
            let send = |out: &mut UnixStream, m: &Message| -> bool {
                let mut line = serde_json::to_string(m).unwrap_or_default();
                line.push('\n');
                out.write_all(line.as_bytes()).and_then(|_| out.flush()).is_ok()
            };
            match (self.peer)(&stream) {
                Ok(uid) if peer_allowed(uid, self.allowed_uid) => {}
                _ => {
                    send(&mut out, &Message::Error { message: "The Kora helper only works for the user who installed it.".into() });
                    return;
                }
            }
            let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
            let mut line = String::new();
            let n = BufReader::new((&stream).take(MAX_REQUEST_BYTES as u64 + 1)).read_line(&mut line);
            let _ = stream.set_read_timeout(None);
            if n.is_err() || line.len() > MAX_REQUEST_BYTES || !line.ends_with('\n') && line.len() >= MAX_REQUEST_BYTES {
                send(&mut out, &Message::Error { message: "The request was not understood.".into() });
                return;
            }
            let Ok(req) = serde_json::from_str::<Request>(line.trim()) else {
                send(&mut out, &Message::Error { message: "The request was not understood.".into() });
                return;
            };
            if req.verb == PING {
                send(&mut out, &Message::Result { ok: true, code: 0, message: None, version: Some(env!("CARGO_PKG_VERSION").into()) });
                return;
            }
            let validator = Arc::clone(&self.validator);
            let argv = match check_request(&req, &|v, a| validator.validate(v, a)) {
                Ok(a) => a,
                Err(r) => {
                    send(&mut out, &Message::Error { message: r.0 });
                    return;
                }
            };
            let env: Vec<(String, String)> = req.env.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            let secrets: Vec<&str> = req.env.values().map(String::as_str).collect();
            let _one_at_a_time = self.run_lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut alive = true;
            let res = self.executor.run(&argv, &env, &mut |stream_name, text| {
                if alive {
                    let line = mask_secrets(text, &secrets);
                    alive = send(&mut out, &Message::Progress { stream: stream_name.to_string(), line });
                }
            });
            match res {
                Ok(code) => send(&mut out, &Message::Result { ok: code == 0, code, message: None, version: None }),
                Err(e) => send(&mut out, &Message::Error { message: format!("The helper could not start Kora: {e}") }),
            };
        }
    }

    /// The real executor: spawns `exe` with a rebuilt environment and streams its output.
    pub struct ProcessExecutor {
        pub exe: PathBuf,
        /// Base environment (`PATH`, `HOME`, ...), see [`base_env`].
        pub base_env: Vec<(String, String)>,
    }

    /// What `sudo kora ...` would give the child, for the installing user.
    pub fn base_env(cfg: &HelperConfig) -> Vec<(String, String)> {
        let mut v = vec![
            ("PATH".to_string(), "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string()),
            ("HOME".to_string(), cfg.home.clone()),
            ("USER".to_string(), cfg.user.clone()),
            ("LOGNAME".to_string(), cfg.user.clone()),
            ("SUDO_USER".to_string(), cfg.user.clone()),
            ("SUDO_UID".to_string(), cfg.uid.to_string()),
            ("SUDO_GID".to_string(), cfg.gid.to_string()),
            ("LANG".to_string(), "C.UTF-8".to_string()),
            ("KORA_HELPER".to_string(), "1".to_string()),
        ];
        if let Some(k) = &cfg.kora_home {
            v.push(("KORA_HOME".to_string(), k.clone()));
        }
        v
    }

    /// Split a chunk of child output into display lines (handles `\r` progress redraws).
    pub fn split_lines(chunk: &[u8]) -> Vec<String> {
        String::from_utf8_lossy(chunk)
            .split(['\n', '\r'])
            .map(|l| l.trim_end())
            .filter(|l| !l.is_empty())
            .map(|l| l.chars().take(LINE_CAP).collect())
            .collect()
    }

    impl Executor for ProcessExecutor {
        fn run(&self, argv: &[String], env: &[(String, String)], on_line: &mut dyn FnMut(&str, &str)) -> io::Result<i32> {
            use std::process::{Command, Stdio};
            let mut child = Command::new(&self.exe)
                .args(argv)
                .env_clear()
                .envs(self.base_env.iter().map(|(k, v)| (k, v)))
                .envs(env.iter().map(|(k, v)| (k, v)))
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;
            let (tx, rx) = std::sync::mpsc::channel::<(&'static str, String)>();
            let mut readers = Vec::new();
            for (name, pipe) in [("stdout", child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>)), ("stderr", child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>))] {
                if let Some(p) = pipe {
                    let tx = tx.clone();
                    readers.push(std::thread::spawn(move || {
                        for chunk in BufReader::new(p).split(b'\n').map_while(Result::ok) {
                            for l in split_lines(&chunk) {
                                if tx.send((name, l)).is_err() {
                                    return;
                                }
                            }
                        }
                    }));
                }
            }
            drop(tx);
            for (name, line) in rx {
                on_line(name, &line);
            }
            for r in readers {
                let _ = r.join();
            }
            Ok(child.wait()?.code().unwrap_or(-1))
        }
    }

    /// Refuse to run from a binary (or folder) somebody other than root can replace.
    pub fn check_exe_trusted(exe: &Path) -> Result<(), String> {
        let mut p = Some(exe);
        while let Some(path) = p {
            let m = std::fs::metadata(path).map_err(|e| format!("cannot inspect {}: {e}", path.display()))?;
            if writable_by_others(m.uid(), m.mode()) {
                return Err(format!(
                    "{} can be changed by someone other than root, so the helper will not run from it. Run `sudo kora helper install` again.",
                    path.display()
                ));
            }
            p = path.parent().filter(|q| !q.as_os_str().is_empty());
        }
        Ok(())
    }

    /// `kora helper serve`: runs until the process is stopped.
    pub fn run(validator: Arc<dyn Validator>) -> Result<(), String> {
        if !crate::elevate::is_elevated() {
            return Err("the helper must run as root (it is started by its system service)".into());
        }
        let cfg = load_config(Path::new(crate::helper::CONFIG_PATH))?;
        if cfg.uid == 0 {
            return Err("the helper was installed for root; reinstall it from your own account with `sudo kora helper install`".into());
        }
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        check_exe_trusted(&exe)?;
        let exec = Arc::new(ProcessExecutor { exe, base_env: base_env(&cfg) });
        let server = Server::bind(Path::new(crate::helper::SOCKET_PATH), cfg.uid, Some((0, cfg.gid)), validator, exec).map_err(|e| format!("cannot open {}: {e}", crate::helper::SOCKET_PATH))?;
        let stop = AtomicBool::new(false);
        Arc::new(server).serve(&stop);
        Ok(())
    }
}

#[cfg(not(unix))]
pub fn run(_validator: std::sync::Arc<dyn Validator>) -> Result<(), String> {
    Err("the Kora helper is not available on Windows yet".into())
}
