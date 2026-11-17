//! Building a support bundle: collect, scrub, show, save. Never uploaded anywhere.
//!
//! * [`build`] gathers everything through the [`Machine`] trait (so tests use fixtures) and puts each piece
//!   through [`super::redact`]: a file that is known to carry secrets is not read at all, anything else is
//!   scrubbed, and a result that still trips the tripwire is dropped.
//! * The person sees [`Bundle::files`] (name, size, what was hidden, what was left out and why) and can read any
//!   file's text before saving. What is saved is exactly what was shown: the bundle is built once, kept in
//!   memory for a short while ([`cache`]), and written from there.
//! * [`save`] writes a zip into a private folder (owner-only) and returns where. The file never leaves the computer
//!   unless the person sends it.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::redact::{self, Hits, Policy};
use crate::installer::registry::ServerEntry;

/// What the bundle says about itself, in every bundle.
pub const README: &str = "\
This is a Kora support bundle.\n\
\n\
It was made on the owner's computer by Kora, and it was shown to the owner, file by file, before it was saved.\n\
Kora never uploads it: it only leaves the computer if the owner sends it.\n\
\n\
Secrets were removed: licence keys, database logins and connection strings, txAdmin accounts and tokens, webhook\n\
addresses, Git and GitHub tokens, the panel's own token, passwords of any kind, and the names of people in\n\
permission files and home folders. Where a value was hidden you will see <hidden>; where a whole file was left\n\
out because it holds secrets, a line in omitted.txt says so. redactions.txt counts what was hidden in each file.\n\
\n\
Not removed: server names, ports, folder names, public IP addresses, resource names and ordinary log text.\n";

/// Lines kept from the end of a log.
pub const LOG_LINES: usize = 300;
/// Bytes kept from the end of a log.
pub const LOG_BYTES: usize = 64 * 1024;

/// One file of the bundle, for the screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BundleFile {
    pub name: String,
    pub bytes: u64,
    /// How many values were hidden in it.
    pub redactions: u32,
    /// What kinds ("licence key", "address with a login", ...).
    pub hidden: Vec<String>,
}

/// Something deliberately left out, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Omitted {
    pub name: String,
    pub why: String,
}

/// What a bundle is made from (kept in memory until saved).
#[derive(Debug, Clone)]
pub struct Bundle {
    pub id: String,
    pub created_unix: u64,
    pub files: Vec<BundleFile>,
    pub omitted: Vec<Omitted>,
    texts: BTreeMap<String, String>,
}

impl Bundle {
    pub fn text(&self, name: &str) -> Option<&str> {
        self.texts.get(name).map(String::as_str)
    }
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.bytes).sum()
    }
    /// The short text the issue reporter may attach: the summary files only, scrubbed already.
    pub fn summary_text(&self) -> String {
        let mut out = String::new();
        for n in ["about.txt", "system-check.txt", "tools.txt"] {
            if let Some(t) = self.text(n) {
                out.push_str(&format!("--- {n}\n{}\n", t.trim_end()));
            }
        }
        out
    }
    fn add(&mut self, name: &str, text: String, hits: Hits) {
        let redactions = hits.values().sum::<usize>() as u32;
        self.files.push(BundleFile { name: name.to_string(), bytes: text.len() as u64, redactions, hidden: hits.keys().map(|k| (*k).to_string()).collect() });
        self.texts.insert(name.to_string(), text);
    }
}

/// Everything the bundle reads from the machine. The real one is in the panel and CLI crates' glue; tests
/// give fixtures.
pub trait Machine {
    fn now(&self) -> u64;
    fn kora_version(&self) -> String;
    fn os_text(&self) -> String;
    /// The person's Kora folder (shown with their user name hidden).
    fn home(&self) -> PathBuf;
    /// Folders to show as `~`.
    fn user_homes(&self) -> Vec<String>;
    fn servers(&self) -> Vec<ServerEntry>;
    fn system_check(&self) -> Option<String>;
    fn tool_versions(&self) -> Vec<String>;
    fn server_status(&self, e: &ServerEntry) -> String;
    fn service_status(&self, e: &ServerEntry) -> String;
    fn firewall_summary(&self) -> String;
    /// Free bytes where `p` is.
    fn free_bytes(&self, p: &Path) -> Option<u64>;
    fn jobs(&self) -> Vec<JobLine>;
    fn audit(&self) -> Vec<String>;
    fn backups_summary(&self, e: &ServerEntry) -> String;
    fn panel_log(&self) -> Option<String>;
    /// Is the file's text readable (a text file)? Reads at most `max` bytes from the end.
    fn read_tail(&self, p: &Path, max: usize) -> Option<String> {
        let mut f = std::fs::File::open(p).ok()?;
        let len = f.metadata().ok()?.len();
        use std::io::{Read, Seek, SeekFrom};
        let start = len.saturating_sub(max as u64);
        f.seek(SeekFrom::Start(start)).ok()?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).ok()?;
        let mut t = String::from_utf8_lossy(&buf).into_owned();
        if start > 0 {
            // drop the cut first line
            if let Some(i) = t.find('\n') {
                t = t[i + 1..].to_string();
            }
        }
        Some(t)
    }
}

/// A recent job, as a line for the bundle (the problem is its message, never its events).
#[derive(Debug, Clone)]
pub struct JobLine {
    pub op: String,
    pub server: Option<String>,
    pub state: String,
    pub started: u64,
    pub error: Option<String>,
}

fn tail_lines(t: &str, n: usize) -> String {
    let lines: Vec<&str> = t.lines().collect();
    let from = lines.len().saturating_sub(n);
    lines[from..].join("\n")
}

fn dir_listing(dir: &Path, depth: usize, prefix: &str, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries.into_iter().take(200) {
        let name = e.file_name().to_string_lossy().into_owned();
        let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            out.push(format!("{rel}/"));
            if depth > 0 && !matches!(name.as_str(), "artifacts" | "resources" | "cache" | ".git" | "node_modules") {
                dir_listing(&e.path(), depth - 1, &rel, out);
            }
        } else {
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            let note = match redact::policy(&rel) {
                Policy::Omit(_) => "  (exists; contents are never included)",
                Policy::Scrub => "",
            };
            out.push(format!("{rel}  {size} bytes{note}"));
        }
    }
}

/// Build the bundle. `server`: only that server's files (else all).
pub fn build(m: &dyn Machine, server: Option<&str>) -> Bundle {
    let now = m.now();
    let mut b = Bundle { id: crate::backup::clock::stamp(now), created_unix: now, files: vec![], omitted: vec![], texts: BTreeMap::new() };
    let homes = m.user_homes();
    // One door for every text: scrub, check, keep or drop.
    let put = |b: &mut Bundle, name: &str, raw: &str| {
        let s = redact::scrub_with_homes(raw, &homes);
        if let Some(what) = redact::leaks(&s.text) {
            b.omitted.push(Omitted { name: name.to_string(), why: format!("it still looked like it held {what} after cleaning, so it was left out") });
            return;
        }
        b.add(name, s.text, s.hits);
    };

    put(
        &mut b,
        "about.txt",
        &format!(
            "Kora {}\nOperating system: {}\nArchitecture: {}\nMade: {}\nKora folder: {}\nFree space in it: {}\n",
            m.kora_version(),
            m.os_text(),
            std::env::consts::ARCH,
            crate::backup::clock::utc_text(now),
            m.home().display(),
            m.free_bytes(&m.home()).map_or("unknown".to_string(), crate::space::human)
        ),
    );
    put(&mut b, "system-check.txt", &m.system_check().unwrap_or_else(|| "The System check could not be run.".into()));
    put(&mut b, "tools.txt", &m.tool_versions().join("\n"));
    put(&mut b, "firewall.txt", &m.firewall_summary());

    let all = m.servers();
    let chosen: Vec<&ServerEntry> = all.iter().filter(|e| server.is_none_or(|s| e.name == s)).collect();
    let registry: Vec<serde_json::Value> = chosen.iter().map(|e| serde_json::json!({"name": e.name, "dir": e.dir.display().to_string(), "mode": format!("{:?}", e.mode).to_lowercase(), "ports": {"game": e.ports.game, "txadmin": e.ports.txadmin}, "created_unix": e.created_unix})).collect();
    put(&mut b, "servers.json", &serde_json::to_string_pretty(&registry).unwrap_or_default());

    for e in chosen {
        let slug = crate::installer::slug(&e.name);
        let base = format!("servers/{slug}");
        put(
            &mut b,
            &format!("{base}/status.txt"),
            &format!(
                "{}\n{}\nBackups: {}\nFree space in the server folder: {}\n",
                m.server_status(e),
                m.service_status(e),
                m.backups_summary(e),
                m.free_bytes(&e.dir).map_or("unknown".to_string(), crate::space::human)
            ),
        );
        // each resource: where it follows, and whether it is on its normal version (no secrets in any of it)
        if let Ok(sf) = crate::installer::ServerFile::load(&e.dir) {
            let st = crate::resourcestate::StateStore::load(&crate::resourcestate::FileStore, &e.dir);
            let pending = crate::resourcestate::pending(&e.dir, true);
            let mut lines =
                vec![format!("Source: {:?}, versions: {:?}{}", sf.source(), sf.versions(), sf.github_owner.as_deref().map(|o| format!(", forks in the GitHub account {o}")).unwrap_or_default())];
            for r in &sf.resources {
                let key = crate::catalog::normalize_id(&r.id);
                let mark = st.resources.get(&key);
                let pr = mark.and_then(|m| m.pr).map(|n| format!("trying pull request #{n}{}", mark.and_then(|m| m.pr_title.as_deref()).map(|t| format!(" ({t})")).unwrap_or_default()));
                lines.push(format!(
                    "{}  follows {}  {}  {}",
                    r.id,
                    if r.branch.is_empty() { "a pinned version" } else { &r.branch },
                    pr.unwrap_or_else(|| "normal version unless its branch says otherwise".into()),
                    if pending.contains(&r.id) || pending.contains(&key) { "RESTART NEEDED" } else { "" }
                ));
            }
            lines.push(format!("Restart needed (if the server is running): {}", if pending.is_empty() { "none".to_string() } else { pending.join(", ") }));
            put(&mut b, &format!("{base}/resources.txt"), &lines.join("\n"));
        }
        let mut listing = Vec::new();
        dir_listing(&e.dir, 2, "", &mut listing);
        put(&mut b, &format!("{base}/files.txt"), &listing.join("\n"));
        for rel in ["server.cfg", "permissions.cfg", "kora-server.json", "kora-resource-state.json", ".kora/schedule.json"] {
            let p = e.dir.join(rel);
            match redact::policy(rel) {
                Policy::Omit(why) => b.omitted.push(Omitted { name: format!("{base}/{rel}"), why: why.to_string() }),
                Policy::Scrub => {
                    if let Some(t) = m.read_tail(&p, 256 * 1024) {
                        put(&mut b, &format!("{base}/{}", rel.trim_start_matches('.')), &t);
                    }
                }
            }
        }
        // what exists and is never read
        for rel in ["secrets.cfg", "database.json", "txData", ".kora/notify.json", ".kora/alerts.json"] {
            if e.dir.join(rel).exists() {
                if let Policy::Omit(why) = redact::policy(rel) {
                    b.omitted.push(Omitted { name: format!("{base}/{rel}"), why: why.to_string() });
                }
            }
        }
        // install and update logs
        let meta = e.dir.join(".kora");
        let mut logs: Vec<PathBuf> = std::fs::read_dir(&meta).into_iter().flatten().flatten().map(|d| d.path()).filter(|p| p.extension().and_then(|x| x.to_str()) == Some("log")).collect();
        logs.sort();
        for p in logs.into_iter().take(6) {
            if let Some(t) = m.read_tail(&p, LOG_BYTES) {
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                put(&mut b, &format!("{base}/logs/{name}"), &tail_lines(&t, LOG_LINES));
            }
        }
    }

    let jobs: Vec<String> = m
        .jobs()
        .into_iter()
        .take(60)
        .map(|j| {
            let err = j.error.as_deref().map(|e| crate::store::jobs::problem_of_row(e).message).unwrap_or_default();
            format!("{}  {:<22} {:<16} {:<10} {}", crate::backup::clock::utc_text(j.started), j.op, j.server.unwrap_or_default(), j.state, err.replace('\n', " "))
        })
        .collect();
    put(&mut b, "jobs.txt", &jobs.join("\n"));
    put(&mut b, "audit.txt", &m.audit().join("\n"));
    match m.panel_log() {
        Some(t) => put(&mut b, "panel.log", &tail_lines(&t, LOG_LINES)),
        None => put(&mut b, "panel.log", "The panel keeps no log file of its own on this computer; what it did is in jobs.txt and audit.txt."),
    }
    if !b.omitted.is_empty() {
        let text: String = b.omitted.iter().map(|o| format!("{}: left out, it {}\n", o.name, o.why)).collect();
        let hits = Hits::new();
        b.add("omitted.txt", text, hits);
    }
    let counts: String = b
        .files
        .iter()
        .map(|f| {
            format!(
                "{}: {} value{} hidden{}\n",
                f.name,
                f.redactions,
                if f.redactions == 1 { "" } else { "s" },
                if f.hidden.is_empty() { String::new() } else { format!(" ({})", f.hidden.join(", ")) }
            )
        })
        .collect();
    b.add("redactions.txt", counts, Hits::new());
    b.add("README.txt", README.to_string(), Hits::new());
    b
}

/// Write the bundle as a zip into `folder` (created, owner-only). Returns the file.
pub fn save(b: &Bundle, folder: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(folder)?;
    crate::store::privacy::tighten_dir(folder);
    let path = folder.join(format!("kora-support-{}.zip", b.id));
    let file = crate::backup::dbdump::create_private(&path)?;
    let mut z = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, text) in &b.texts {
        z.start_file(name, opts).map_err(std::io::Error::other)?;
        z.write_all(text.as_bytes())?;
    }
    z.finish().map_err(std::io::Error::other)?;
    Ok(path)
}

/// The folder bundles are saved in by default.
pub fn default_folder() -> PathBuf {
    crate::installer::registry::kora_home().join("support")
}

/// A bundle shown on a screen and saved from the same copy.
pub mod cache {
    use super::Bundle;
    use std::sync::Mutex;

    const KEEP_SECS: u64 = 15 * 60;
    static CACHE: Mutex<Vec<(String, Bundle)>> = Mutex::new(Vec::new());

    /// Keep `b` under a fresh id (the id is returned); older ones expire.
    pub fn put(mut b: Bundle) -> String {
        let mut rnd = [0u8; 6];
        let _ = getrandom::fill(&mut rnd);
        let id = format!("{}-{}", b.id, hex::encode(rnd));
        b.id = id.clone();
        let now = crate::monitor::now_unix();
        let mut c = CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        c.retain(|(_, x)| now.saturating_sub(x.created_unix) < KEEP_SECS);
        c.push((id.clone(), b));
        if c.len() > 8 {
            c.remove(0);
        }
        id
    }

    pub fn get(id: &str) -> Option<Bundle> {
        CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner).iter().find(|(k, _)| k == id).map(|(_, b)| b.clone())
    }
}

/// A small bundle for a test double of the backend (the fake in `api::fake`): real shapes, no machine reads.
#[doc(hidden)]
pub fn tests_bundle_for_fake() -> Bundle {
    let mut b = Bundle { id: "x".into(), created_unix: crate::monitor::now_unix(), files: vec![], omitted: vec![], texts: BTreeMap::new() };
    b.add("about.txt", "Kora 0.8.0\nOperating system: Ubuntu 24.04\n".into(), Hits::new());
    b.add("servers/prod/server.cfg", "sv_licenseKey \"<hidden: licence key>\"\nendpoint_add_tcp \"0.0.0.0:30120\"\n".into(), [("licence key", 1usize)].into_iter().collect());
    b.omitted.push(Omitted { name: "servers/prod/secrets.cfg".into(), why: "holds the licence key and the database connection string".into() });
    b.add("README.txt", README.to_string(), Hits::new());
    b
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::installer::registry::PortsRecord;
    use crate::installer::Mode;

    pub struct Fixture {
        #[allow(dead_code)]
        pub dir: tempfile::TempDir,
        pub entry: ServerEntry,
    }

    pub const KEY: &str = "cfxk_AbCdEf0123456789AbCdEf0123456789_xYz123";

    /// A server folder with every kind of file a real one has, secrets included.
    pub fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("srv");
        let w = |rel: &str, text: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        w("server.cfg", &format!("sv_licenseKey \"{KEY}\"\nset mysql_connection_string \"mysql://kora_ab:s3cr3t-db-pass@127.0.0.1/kora_ab\"\nendpoint_add_tcp \"0.0.0.0:30120\"\nensure core\n"));
        w("permissions.cfg", "add_principal identifier.fivem:123456 group.admin\n");
        w("secrets.cfg", &format!("sv_licenseKey \"{KEY}\"\n"));
        w("database.json", r#"{"password":"s3cr3t-db-pass"}"#);
        w(
            "kora-server.json",
            r#"{"name":"Prod","mode":"development","source":"fork","versions":"latest","dir":"/srv/prod","ports":{"game":30120,"txadmin":40120},"game_build":3258,"artifact_build":1,"database":null,"github_owner":"me","created_unix":1,"kora_version":"0.8.0","resources":[{"id":"core","path":"core","repository":"https://github.com/KoraFiveM/core","branch":"main"}],"license_key_name":"Main"}"#,
        );
        w("txData/default/config.json", r#"{"global":{"adminPassword":"hunter2-and-more"}}"#);
        w("txData/default/data/admins.json", r#"[{"name":"root","password_hash":"$2b$10$abcdefghijklmnopqrstuuABCDEFGHIJKLMNOPQRSTUVWXYZ01234"}]"#);
        w(".kora/notify.json", r#"{"discord_webhook":"https://discord.com/api/webhooks/123456789012345678/1234567890AbCdEfGhIjKlMnOpQrStUv"}"#);
        w(".kora/alerts.json", r#"{"webhook_url":"https://hooks.example.com/x?token=abcdef0123456789abcdef"}"#);
        w(".kora/install.log", &format!("starting\ngit clone https://ghp_0123456789abcdefghijklmnopqrstuvwxyz@github.com/a/b.git\nlicence {KEY} accepted\ndone\n"));
        w("kora-resource-state.json", r#"{"version":1,"clock":3,"resources":{"core":{"changed_seq":3,"changed_unix":1,"why":"switched","pr":12,"pr_title":"Fix the HUD"}}}"#);
        w("resources/core/fxmanifest.lua", "fx_version 'cerulean'\n");
        let entry = ServerEntry { name: "Prod".into(), dir: root, mode: Mode::Production, source: None, versions: None, ports: PortsRecord { game: 30120, txadmin: 40120 }, created_unix: 1 };
        Fixture { dir, entry }
    }

    pub struct Fake<'a> {
        pub f: &'a Fixture,
    }

    impl Machine for Fake<'_> {
        fn now(&self) -> u64 {
            1_700_000_000
        }
        fn kora_version(&self) -> String {
            "0.8.0".into()
        }
        fn os_text(&self) -> String {
            "Ubuntu 24.04 (Linux 6.8)".into()
        }
        fn home(&self) -> PathBuf {
            "/home/jamie/.kora".into()
        }
        fn user_homes(&self) -> Vec<String> {
            vec!["/home/jamie".into()]
        }
        fn servers(&self) -> Vec<ServerEntry> {
            vec![self.f.entry.clone()]
        }
        fn system_check(&self) -> Option<String> {
            Some("git: OK 2.43\ngh: not signed in\ndatabase: running".into())
        }
        fn tool_versions(&self) -> Vec<String> {
            vec!["git 2.43.0".into(), "mariadb 11.4".into()]
        }
        fn server_status(&self, _: &ServerEntry) -> String {
            "Status: running".into()
        }
        fn service_status(&self, _: &ServerEntry) -> String {
            "Service: active".into()
        }
        fn firewall_summary(&self) -> String {
            "ufw active; port 30120 allowed".into()
        }
        fn free_bytes(&self, _: &Path) -> Option<u64> {
            Some(50 << 30)
        }
        fn jobs(&self) -> Vec<JobLine> {
            vec![JobLine {
                op: "install_start".into(),
                server: Some("Prod".into()),
                state: "failed".into(),
                started: 1_699_999_000,
                error: Some(format!("git clone failed for https://oauth2:ghp_0123456789abcdefghijklmnopqrstuvwxyz@github.com/a/b.git with key {KEY}")),
            }]
        }
        fn audit(&self) -> Vec<String> {
            vec!["2023-11-14 server_control ok local".into()]
        }
        fn backups_summary(&self, _: &ServerEntry) -> String {
            "Last backup 3 days ago".into()
        }
        fn panel_log(&self) -> Option<String> {
            Some("panel at http://127.0.0.1:47120/?code=0a1b2c3d4e5f60718293a4b5c6d7e8f9\nready".into())
        }
    }

    #[test]
    fn the_bundle_holds_what_helps_and_nothing_secret() {
        let f = fixture();
        let b = build(&Fake { f: &f }, None);
        let names: Vec<&str> = b.files.iter().map(|x| x.name.as_str()).collect();
        for want in [
            "about.txt",
            "system-check.txt",
            "tools.txt",
            "firewall.txt",
            "servers.json",
            "servers/prod/status.txt",
            "servers/prod/server.cfg",
            "servers/prod/permissions.cfg",
            "servers/prod/kora-server.json",
            "servers/prod/logs/install.log",
            "jobs.txt",
            "audit.txt",
            "panel.log",
            "redactions.txt",
            "README.txt",
            "omitted.txt",
        ] {
            assert!(names.contains(&want), "{want} missing in {names:?}");
        }
        // the files that hold secrets were never read
        for gone in ["secrets.cfg", "database.json", "config.json", "admins.json", "notify.json", "alerts.json"] {
            assert!(!names.iter().any(|n| n.ends_with(gone)), "{gone} must not be in the bundle");
        }
        let omitted = b.text("omitted.txt").unwrap();
        for said in ["secrets.cfg", "database.json", "txData", "notify.json", "alerts.json"] {
            assert!(omitted.contains(said), "{said} not named in omitted.txt:\n{omitted}");
        }
        // not one secret anywhere in any file, or in the summary
        let all: String = b.files.iter().map(|x| b.text(&x.name).unwrap()).collect::<Vec<_>>().join("\n====\n");
        for sec in [
            KEY,
            "s3cr3t-db-pass",
            "hunter2-and-more",
            "ghp_0123456789abcdefghijklmnopqrstuvwxyz",
            "1234567890AbCdEfGhIjKlMnOpQrStUv",
            "0a1b2c3d4e5f60718293a4b5c6d7e8f9",
            "abcdef0123456789abcdef",
            "identifier.fivem:123456",
            "jamie",
        ] {
            assert!(!all.contains(sec), "{sec} survived in the bundle:\n{all}");
        }
        assert!(redact::leaks(&all).is_none());
        assert!(!b.summary_text().contains(KEY));
        // what is useful survives
        assert!(b.text("servers/prod/server.cfg").unwrap().contains("endpoint_add_tcp \"0.0.0.0:30120\""));
        assert!(b.text("servers/prod/files.txt").unwrap().contains("secrets.cfg") && b.text("servers/prod/files.txt").unwrap().contains("contents are never included"));
        assert!(b.text("jobs.txt").unwrap().contains("install_start") && b.text("jobs.txt").unwrap().contains("git clone failed"));
        assert!(b.text("about.txt").unwrap().contains("Ubuntu 24.04") && b.text("about.txt").unwrap().contains("~/.kora"));
        assert!(b.text("redactions.txt").unwrap().contains("servers/prod/server.cfg"));
        // each resource's branch / pull request and the restart-needed state are listed
        let res = b.text("servers/prod/resources.txt").unwrap();
        assert!(res.contains("trying pull request #12 (Fix the HUD)") && res.contains("Source:") && res.contains("RESTART NEEDED"), "{res}");
        assert!(b.text("servers/prod/kora-resource-state.json").unwrap().contains("Fix the HUD"));
    }

    #[test]
    fn a_text_that_still_looks_secret_after_cleaning_is_left_out() {
        // simulate a rule gap: a Machine whose status text carries a shape the rules replace but we then re-inject via a file name hint is not
        // possible, so check the guard directly: leaks() on a raw text trips, and `build` records it as omitted when scrub misses it
        let t = "AKIAIOSFODNN7EXAMPLE";
        assert!(redact::scrub(t).text.contains("<hidden"));
        assert_eq!(redact::leaks("cfxk_AbCdEf0123456789"), Some("a licence key"));
    }

    #[test]
    fn what_was_shown_is_what_is_saved_and_the_file_is_private() {
        let f = fixture();
        let b = build(&Fake { f: &f }, Some("Prod"));
        let id = cache::put(b.clone());
        let shown = cache::get(&id).unwrap();
        let out = tempfile::tempdir().unwrap();
        let path = save(&shown, &out.path().join("support")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
        assert_eq!(zip.len(), b.files.len());
        for i in 0..zip.len() {
            let mut e = zip.by_index(i).unwrap();
            let name = e.name().unwrap().to_string();
            let mut text = String::new();
            std::io::Read::read_to_string(&mut e, &mut text).unwrap();
            assert_eq!(Some(text.as_str()), b.text(&name), "{name} differs from what was shown");
        }
        assert!(cache::get("nope").is_none());
    }

    #[test]
    fn only_the_chosen_server_is_included() {
        let f = fixture();
        let b = build(&Fake { f: &f }, Some("Other"));
        assert!(!b.files.iter().any(|x| x.name.starts_with("servers/prod")));
    }
}
