//! The real [`Backend`]: registry file, OS services, port probe, txAdmin/FXServer config.
//! Thin on purpose; the rules live in `kora-core`.
//!
//! ## "Ensure a resource"
//! Neither txAdmin nor FXServer exposes a console API Kora can call without credentials (txAdmin's
//! live console needs an admin session; FXServer's RCON is not configured by Kora). The simplest
//! reliable path is therefore: put `ensure <name>` into `<server>/server.cfg` (once; never twice)
//! and, when asked, restart the server's service so it loads. A server that is not running simply
//! loads it on its next start. Names are validated like the installer does (letters, digits,
//! `_ . -`, optionally wrapped in `[ ]` for a category folder).

use std::path::{Path, PathBuf};

use kora_core::firewall::ports::{PortProber, SystemProber};
use kora_core::installer::registry::{self, Registry, ServerEntry, ServerFile};
use kora_core::installer::{self, host, RollbackOptions};
use kora_core::service::{ControlAction, ServiceState};

use kora_core::notify::{self, NotifyConfig};
use kora_core::service::{self as schedule, ScheduleConfig};
use kora_core::update::manual::{self, ApplyOpts, Scope, SnapshotInfo, UpdateEvent, UpdatePreview};
use kora_core::update::UpdateReport;
use super::backend::{Backend, Res, Status, UpdateBadge};

#[derive(Default)]
pub struct SystemBackend;

/// Is `name` something we may write into server.cfg?
pub fn valid_resource_name(v: &str) -> bool {
    let core = v.strip_prefix('[').and_then(|s| s.strip_suffix(']')).unwrap_or(v);
    !core.is_empty() && v.len() <= 100 && core.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

/// `cfg` with `ensure <name>` added at the end, or `None` when it is already ensured.
pub fn with_ensure(cfg: &str, name: &str) -> Option<String> {
    let already = cfg.lines().any(|l| {
        let mut w = l.split_whitespace();
        w.next() == Some("ensure") && w.next() == Some(name) && match w.next() { None => true, Some(x) => x.starts_with('#') }
    });
    if already {
        return None;
    }
    let mut out = cfg.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&format!("ensure {name}\n"));
    Some(out)
}

/// `<dir>/.kora/update-status.json`, written by the updater and by `[ Check for updates ]`:
/// `{"checked_at": 1700000000, "updates": 3, "attention": false, "new_release": false}`.
/// A missing, unreadable or older-shaped file means "not known" (`None`) or whatever fields it has.
pub fn read_badge(dir: &Path) -> Option<UpdateBadge> {
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join(".kora").join("update-status.json")).ok()?).ok()?;
    let flag = |k: &str| match v.get(k) {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::String(s)) => !s.is_empty(),
        _ => false,
    };
    if flag("attention") || flag("needs_attention") {
        return Some(UpdateBadge::NeedsAttention);
    }
    let n = v.get("updates").or_else(|| v.get("behind")).and_then(|x| x.as_u64()).unwrap_or(0) as u32;
    if n > 0 {
        return Some(UpdateBadge::Updates(n));
    }
    if flag("new_release") {
        return Some(UpdateBadge::NewRelease);
    }
    Some(UpdateBadge::UpToDate)
}

/// Write the status file the list badge reads (atomically; a failure only costs the badge).
fn write_status(dir: &Path, p: &UpdatePreview) {
    let meta = dir.join(".kora");
    if std::fs::create_dir_all(&meta).is_err() {
        return;
    }
    let tmp = meta.join("update-status.json.tmp");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    if std::fs::write(&tmp, super::backend::status_json(p, now)).is_ok() && std::fs::rename(&tmp, meta.join("update-status.json")).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// The record the manual update engine wants (it reads `kora-server.json`).
fn record(e: &ServerEntry) -> Res<manual::ServerRecord> {
    manual::load_server(&e.dir.to_string_lossy()).map_err(err)
}

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

struct Quiet;
impl kora_core::installer::ProgressSink for Quiet {
    fn event(&mut self, _ev: kora_core::installer::ProgressEvent) {}
}

impl Backend for SystemBackend {
    fn list(&mut self) -> Res<Vec<ServerEntry>> {
        Registry::load().map(|r| r.servers).map_err(|e| format!("Can't read the server list ({}): {e}", registry::registry_path().display()))
    }

    fn status(&mut self, e: &ServerEntry) -> Status {
        if !e.dir.is_dir() {
            return Status::Missing;
        }
        match host::service_state(&e.name) {
            Ok(ServiceState::Running) => return Status::Running,
            Ok(ServiceState::Stopped) => return Status::Stopped,
            _ => {}
        }
        // No service (started by hand, or an adopted server): is something listening on its game port?
        if SystemProber.udp_free(e.ports.game) && SystemProber.tcp_free(e.ports.game) {
            Status::Stopped
        } else {
            Status::Running
        }
    }

    fn badge(&mut self, e: &ServerEntry) -> Option<UpdateBadge> {
        read_badge(&e.dir)
    }

    fn control(&mut self, e: &ServerEntry, a: ControlAction) -> Res<()> {
        host::service_control(&e.name, a).map_err(err)
    }

    fn resources(&mut self, e: &ServerEntry) -> Vec<String> {
        ServerFile::load(&e.dir).map(|s| s.resources.into_iter().map(|r| r.id).collect()).unwrap_or_default()
    }

    fn ensure(&mut self, e: &ServerEntry, name: &str, restart: bool) -> Res<String> {
        if !valid_resource_name(name) {
            return Err(format!("'{name}' is not a resource name (letters, digits, _ . - only)."));
        }
        let cfg_path = e.dir.join("server.cfg");
        let cfg = std::fs::read_to_string(&cfg_path).map_err(|x| format!("Can't read {}: {x}", cfg_path.display()))?;
        let mut said = match with_ensure(&cfg, name) {
            Some(new) => {
                std::fs::write(&cfg_path, new).map_err(|x| format!("Can't write {}: {x}", cfg_path.display()))?;
                format!("Added 'ensure {name}' to server.cfg.")
            }
            None => format!("'ensure {name}' is already in server.cfg."),
        };
        match self.status(e) {
            Status::Running if restart => {
                self.control(e, ControlAction::Restart)?;
                said.push_str(" The server was restarted.");
            }
            Status::Running => said.push_str(" Restart the server to load it."),
            _ => said.push_str(" It loads the next time the server starts."),
        }
        Ok(said)
    }

    fn delete(&mut self, e: &ServerEntry, keep_files: bool, keep_db: bool) -> Res<Vec<String>> {
        installer::rollback(&e.dir, &RollbackOptions { keep_files, keep_db }, &mut Quiet).map_err(err)
    }

    fn unregister(&mut self, e: &ServerEntry) -> Res<()> {
        let mut r = Registry::load().map_err(err)?;
        r.remove(&e.name);
        r.save_or_clean_to(&registry::registry_path()).map_err(err)
    }

    fn register(&mut self, e: ServerEntry) -> Res<()> {
        let mut r = Registry::load().map_err(err)?;
        r.upsert(e);
        r.save().map_err(err)
    }

    fn scan_roots(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = Vec::new();
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
        if let Some(h) = home {
            v.push(h);
        }
        if cfg!(windows) {
            v.extend(["C:\\", "D:\\"].iter().map(PathBuf::from).filter(|p| p.is_dir()));
        } else {
            v.extend(["/srv", "/opt", "/home"].iter().map(PathBuf::from).filter(|p| p.is_dir()));
        }
        v
    }

    fn preview(&mut self, e: &ServerEntry, scope: Scope) -> Res<UpdatePreview> {
        manual::preview(&record(e)?, scope).map_err(err)
    }

    fn apply(&mut self, e: &ServerEntry, scope: Scope, o: ApplyOpts, on: &mut dyn FnMut(UpdateEvent)) -> Res<UpdateReport> {
        let rec = record(e)?;
        let report = manual::apply(&rec, scope, o, on).map_err(err)?;
        notify::after_update(e, &report);
        Ok(report)
    }

    fn rollback(&mut self, e: &ServerEntry, snapshot: &str) -> Res<()> {
        manual::rollback(&record(e)?, snapshot).map_err(err)
    }

    fn snapshots(&mut self, e: &ServerEntry) -> Vec<SnapshotInfo> {
        record(e).map(|r| manual::snapshots(&r)).unwrap_or_default()
    }

    fn check_updates(&mut self, e: &ServerEntry) -> Res<UpdateBadge> {
        let p = self.preview(e, Scope::All)?;
        write_status(&e.dir, &p);
        Ok(super::backend::badge_of(&p))
    }

    fn notify_load(&mut self, e: &ServerEntry) -> NotifyConfig {
        notify::load(e)
    }

    fn notify_save(&mut self, e: &ServerEntry, c: &NotifyConfig) -> Res<()> {
        notify::save(e, c).map_err(err)
    }

    fn notify_test(&mut self, e: &ServerEntry, c: &NotifyConfig) -> Res<()> {
        notify::send_test(c, &e.name).map_err(err)
    }

    fn schedule_get(&mut self, e: &ServerEntry) -> Option<ScheduleConfig> {
        schedule::get(e)
    }

    fn schedule_set(&mut self, e: &ServerEntry, c: &ScheduleConfig) -> Res<()> {
        schedule::set(e, c).map_err(err)
    }

    fn schedule_remove(&mut self, e: &ServerEntry) -> Res<()> {
        schedule::remove(e).map_err(err)
    }

    fn schedule_elevation(&mut self, e: &ServerEntry, set: Option<&ScheduleConfig>) -> Option<Vec<String>> {
        if kora_core::elevate::is_elevated() {
            return None;
        }
        Some(match set {
            Some(c) => schedule::set_args(e, c),
            None => vec!["schedule".into(), "off".into(), e.dir.to_string_lossy().into_owned()],
        })
    }
}

/// Opens folders and the local txAdmin address with the system's default programs.
pub struct SystemLauncher;

impl super::app::Launcher for SystemLauncher {
    fn open_folder(&mut self, dir: &Path) -> Result<(), String> {
        crate::open::Opener::open(&mut crate::open::SystemOpener, dir, dir)
    }

    fn open_url(&mut self, url: &str) -> Result<(), String> {
        // Only ever the local txAdmin address, built from a port number.
        if !url.starts_with("http://127.0.0.1:") || url.chars().any(|c| c.is_control() || c == ' ' || c == '"') {
            return Err("Only the local txAdmin address can be opened.".into());
        }
        let (program, args): (&str, Vec<&str>) = match crate::open::OsKind::current() {
            crate::open::OsKind::Windows => ("rundll32.exe", vec!["url.dll,FileProtocolHandler", url]),
            crate::open::OsKind::Mac => ("open", vec![url]),
            crate::open::OsKind::Other => ("xdg-open", vec![url]),
        };
        std::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|mut c| {
                std::thread::spawn(move || {
                    let _ = c.wait();
                });
            })
            .map_err(|_| "Couldn't open a browser automatically.".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_names_are_checked() {
        assert!(valid_resource_name("kora_core"));
        assert!(valid_resource_name("[standalone]"));
        assert!(!valid_resource_name(""));
        assert!(!valid_resource_name("a b"));
        assert!(!valid_resource_name("x;quit"));
        assert!(!valid_resource_name("[]"));
    }

    #[test]
    fn ensure_is_appended_once() {
        let cfg = "set a b\nensure foo\n";
        assert_eq!(with_ensure(cfg, "bar").unwrap(), "set a b\nensure foo\nensure bar\n");
        assert!(with_ensure(cfg, "foo").is_none());
        assert_eq!(with_ensure("set a b", "x").unwrap(), "set a b\nensure x\n");
        assert!(with_ensure("ensure foo # note\n", "foo").is_none());
        assert!(with_ensure("# ensure foo\n", "foo").is_some(), "a comment does not count");
        assert!(with_ensure("ensure foobar\n", "foo").is_some(), "a longer name does not count");
    }

    #[test]
    fn badge_file_is_read() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(read_badge(d.path()), None);
        std::fs::create_dir(d.path().join(".kora")).unwrap();
        let w = |s: &str| std::fs::write(d.path().join(".kora/update-status.json"), s).unwrap();
        w(r#"{"checked_at":5,"updates":3,"attention":false,"new_release":false}"#);
        assert_eq!(read_badge(d.path()), Some(UpdateBadge::Updates(3)));
        w(r#"{"updates":0,"attention":true}"#);
        assert_eq!(read_badge(d.path()), Some(UpdateBadge::NeedsAttention));
        w(r#"{"behind":2}"#);
        assert_eq!(read_badge(d.path()), Some(UpdateBadge::Updates(2)), "the old shape still reads");
        w("not json");
        assert_eq!(read_badge(d.path()), None);
        w(r#"{"new_release":true}"#);
        assert_eq!(read_badge(d.path()), Some(UpdateBadge::NewRelease));
        w(r#"{}"#);
        assert_eq!(read_badge(d.path()), Some(UpdateBadge::UpToDate));
    }
}
