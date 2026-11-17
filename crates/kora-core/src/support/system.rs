//! The real [`Machine`](super::bundle::Machine): reads this computer. Every read is best effort: a thing that
//! cannot be read becomes a line saying so, never a failure of the whole bundle.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use super::bundle::{JobLine, Machine};
use crate::api::install::github::{GithubEnv, SystemGithub};
use crate::api::toolchain::{system_check, SystemToolchain, ToolchainEnv};
use crate::api::{Backend, Status};
use crate::installer::registry::ServerEntry;
use crate::store::{audit, jobs, Store};

pub struct SystemMachine<'a, B: Backend + ?Sized> {
    backend: RefCell<&'a mut B>,
}

impl<'a, B: Backend + ?Sized> SystemMachine<'a, B> {
    pub fn new(backend: &'a mut B) -> Self {
        SystemMachine { backend: RefCell::new(backend) }
    }
}

fn run(program: &str, args: &[&str]) -> Option<String> {
    let o = crate::sysexe::command(program).args(args).stdin(std::process::Stdio::null()).output().ok()?;
    let mut t = String::from_utf8_lossy(&o.stdout).into_owned();
    if t.trim().is_empty() {
        t = String::from_utf8_lossy(&o.stderr).into_owned();
    }
    Some(t)
}

fn first_line(t: &str) -> String {
    t.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string()
}

/// A program's `--version` first line, or why not.
fn version_of(program: &str) -> String {
    match run(program, &["--version"]) {
        Some(t) if !first_line(&t).is_empty() => format!("{program}: {}", first_line(&t)),
        _ => format!("{program}: not found"),
    }
}

pub fn os_version_text() -> String {
    if cfg!(windows) {
        return run("cmd", &["/C", "ver"]).map(|t| first_line(&t)).filter(|t| !t.is_empty()).unwrap_or_else(|| "Windows".into());
    }
    let pretty = std::fs::read_to_string("/etc/os-release").ok().and_then(|t| t.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=").map(|v| v.trim_matches('"').to_string())));
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease").ok().map(|k| k.trim().to_string());
    match (pretty, kernel) {
        (Some(p), Some(k)) => format!("{p} (Linux {k})"),
        (Some(p), None) => p,
        (None, Some(k)) => format!("Linux {k}"),
        _ => std::env::consts::OS.to_string(),
    }
}

impl<B: Backend + ?Sized> Machine for SystemMachine<'_, B> {
    fn now(&self) -> u64 {
        crate::monitor::now_unix()
    }
    fn kora_version(&self) -> String {
        format!("{} ({} {})", env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH)
    }
    fn os_text(&self) -> String {
        os_version_text()
    }
    fn home(&self) -> PathBuf {
        crate::installer::registry::kora_home()
    }
    fn user_homes(&self) -> Vec<String> {
        let mut v: Vec<String> = ["HOME", "USERPROFILE"].iter().filter_map(|k| std::env::var(k).ok()).filter(|h| !h.is_empty()).collect();
        if let Some(h) = crate::userhome::real_user_home() {
            v.push(h.display().to_string());
        }
        v.sort();
        v.dedup();
        v
    }
    fn servers(&self) -> Vec<ServerEntry> {
        self.backend.borrow_mut().list().unwrap_or_default()
    }
    fn system_check(&self) -> Option<String> {
        let items = system_check(&SystemToolchain, &SystemGithub.auth());
        Some(items.iter().map(|i| format!("{}: {:?} - {}", i.label, i.state, i.summary)).collect::<Vec<_>>().join("\n"))
    }
    fn tool_versions(&self) -> Vec<String> {
        let client = crate::installer::db_client();
        let mut v = vec![version_of("git"), version_of("gh"), version_of(&client)];
        if let Some(d) = crate::backup::dbdump::dump_program(&client, &crate::backup::dbdump::program_runs) {
            v.push(version_of(&d));
        } else {
            v.push("database dump tool (mariadb-dump / mysqldump): not found".into());
        }
        let t = SystemToolchain.tools();
        for s in t {
            v.push(format!("{} as Kora sees it: {}", s.tool.label(), if s.installed { s.version.clone().unwrap_or_else(|| "installed".into()) } else { "not installed".into() }));
        }
        v
    }
    fn server_status(&self, e: &ServerEntry) -> String {
        let mut b = self.backend.borrow_mut();
        let st = match b.status(e) {
            Status::Running => "running",
            Status::Stopped => "stopped",
            Status::Missing => "folder missing",
        };
        let d = b.status_detail(e);
        format!(
            "Server {}: {st} (game port {}: {}, txAdmin port {}: {})\nMode: {:?}",
            e.name,
            e.ports.game,
            if d.game_listening { "listening" } else { "not listening" },
            e.ports.txadmin,
            if d.txadmin_up { "answering" } else { "not answering" },
            e.mode
        )
    }
    fn service_status(&self, e: &ServerEntry) -> String {
        let svc = match crate::installer::host::service_state(&e.name) {
            Ok(s) => format!("{s:?}"),
            Err(x) => format!("unknown ({})", x.one_line()),
        };
        let timer = match crate::service::get(e) {
            Some(c) => crate::service::describe(&c),
            None => "no automatic-update timer".into(),
        };
        format!("Service: {svc}\nUpdate schedule: {timer}")
    }
    fn firewall_summary(&self) -> String {
        let kind = crate::firewall::detect(crate::service::Platform::current(), &crate::service::SystemRunner);
        let Some(kind) = kind else { return "No active firewall (ufw, firewalld, Windows Firewall) was found.".into() };
        let text = if cfg!(windows) {
            let all = run("netsh", &["advfirewall", "firewall", "show", "rule", "name=all"]).unwrap_or_default();
            // only Kora's rules
            all.split("\r\n\r\n").flat_map(|b| b.split("\n\n")).filter(|b| b.to_ascii_lowercase().contains("kora")).collect::<Vec<_>>().join("\n\n")
        } else {
            run("ufw", &["status"]).or_else(|| run("firewall-cmd", &["--list-all"])).unwrap_or_default()
        };
        format!("Firewall: {kind:?}\n{}", text.lines().take(60).collect::<Vec<_>>().join("\n"))
    }
    fn free_bytes(&self, p: &Path) -> Option<u64> {
        crate::space::DiskInfo::free_bytes(&crate::space::SystemDisk, p).ok()
    }
    fn jobs(&self) -> Vec<JobLine> {
        let Some(s) = Store::open_existing(&crate::installer::registry::kora_home()).ok().flatten() else { return vec![] };
        jobs::list(&s, 60).unwrap_or_default().into_iter().map(|j| JobLine { op: j.op, server: j.server, state: j.state, started: j.started, error: j.error }).collect()
    }
    fn audit(&self) -> Vec<String> {
        let Some(s) = Store::open_existing(&crate::installer::registry::kora_home()).ok().flatten() else { return vec![] };
        audit::list(&s, &audit::Query { limit: 100, ..Default::default() })
            .unwrap_or_default()
            .into_iter()
            .map(|r| format!("{}  {:<22} {:<16} {:<8} {} {}", crate::backup::clock::utc_text(r.at), r.op, r.server.unwrap_or_default(), r.result, r.actor_kind, r.detail.unwrap_or_default()))
            .collect()
    }
    fn backups_summary(&self, e: &ServerEntry) -> String {
        let cfg = crate::backup::settings::load_settings(&e.name);
        let st = crate::backup::settings::load_status(&e.name);
        let list = {
            use crate::backup::dest::Destination;
            crate::backup::restore::destination_for(&e.name).list()
        };
        let s = crate::backup::settings::summary(&list, &cfg, &st, crate::monitor::now_unix());
        format!("{}{}", s.text, s.warning.map(|w| format!(" ({w})")).unwrap_or_default())
    }
    fn panel_log(&self) -> Option<String> {
        let home = crate::installer::registry::kora_home();
        ["panel.log", "logs/panel.log"].iter().find_map(|n| self.read_tail(&home.join(n), super::bundle::LOG_BYTES))
    }
}
