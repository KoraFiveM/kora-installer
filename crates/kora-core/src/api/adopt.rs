//! Finding existing servers to add to the manager: folders Kora made (`kora-server.json`), old
//! Kora layouts (a `.kora` folder) and plain FXServer folders. Scanning only READS; adopting a
//! folder writes nothing into it (it only adds a registry entry).

use std::path::{Path, PathBuf};

use crate::installer::registry::{self, now_unix, PortsRecord, ServerEntry, ServerFile};
use crate::installer::{slug, Mode};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Made by Kora: has `kora-server.json`.
    Kora,
    /// An older Kora layout: has a `.kora` folder but no `kora-server.json`.
    OldKora,
    /// A plain FXServer folder (server.cfg + txData, an FXServer binary, or an artifacts folder).
    FxServer,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Kora => "Kora server",
            Kind::OldKora => "older Kora layout",
            Kind::FxServer => "FXServer folder",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Candidate {
    pub dir: PathBuf,
    pub name: String,
    pub kind: Kind,
    pub mode: Mode,
    pub ports: PortsRecord,
}

pub const DEFAULT_PORTS: PortsRecord = PortsRecord { game: 30120, txadmin: 40120 };
const MAX_VISITED: usize = 20_000;

/// Game port from `endpoint_add_tcp "0.0.0.0:30120"` in a server.cfg.
pub fn parse_game_port(cfg: &str) -> Option<u16> {
    cfg.lines().find_map(|l| {
        let l = l.trim();
        let rest = l.strip_prefix("endpoint_add_tcp")?;
        let addr = rest.trim().trim_matches('"');
        addr.rsplit(':').next()?.trim_matches('"').parse().ok()
    })
}

fn has_fxserver(dir: &Path) -> bool {
    let exe = if cfg!(windows) { "FXServer.exe" } else { "FXServer" };
    if dir.join(exe).is_file() || dir.join("alpine").is_dir() {
        return true;
    }
    std::fs::read_dir(dir.join("artifacts")).map(|rd| rd.flatten().any(|e| e.path().join(exe).is_file())).unwrap_or(false)
}

fn folder_name(dir: &Path) -> String {
    dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "server".into())
}

/// What `dir` is, if it is a server folder at all.
pub fn classify(dir: &Path) -> Option<Candidate> {
    if let Ok(sf) = ServerFile::load(dir) {
        return Some(Candidate { dir: dir.to_path_buf(), name: sf.name, kind: Kind::Kora, mode: sf.mode, ports: sf.ports });
    }
    let cfg_port = std::fs::read_to_string(dir.join("server.cfg")).ok().and_then(|c| parse_game_port(&c));
    let ports = PortsRecord { game: cfg_port.unwrap_or(DEFAULT_PORTS.game), txadmin: DEFAULT_PORTS.txadmin };
    let kind = if dir.join(".kora").is_dir() {
        Kind::OldKora
    } else if has_fxserver(dir) || (dir.join("server.cfg").is_file() && dir.join("txData").is_dir()) {
        Kind::FxServer
    } else {
        return None;
    };
    Some(Candidate { dir: dir.to_path_buf(), name: folder_name(dir), kind, mode: Mode::Production, ports })
}

fn skip_dir(name: &str) -> bool {
    name.starts_with('.') || matches!(name, "node_modules" | "target" | "proc" | "sys" | "dev" | "windows" | "system32" | "$recycle.bin")
}

/// Look for server folders under `roots`, `depth` levels down (0 = the roots themselves).
/// Folders already registered (`known`) are left out; a found folder is not searched further.
pub fn scan(roots: &[PathBuf], depth: usize, known: &[PathBuf]) -> Vec<Candidate> {
    let mut found: Vec<Candidate> = Vec::new();
    let mut visited = 0usize;
    let mut level: Vec<PathBuf> = roots.to_vec();
    for d in 0..=depth {
        let mut next = Vec::new();
        for dir in level {
            visited += 1;
            if visited > MAX_VISITED {
                return found;
            }
            if let Some(c) = classify(&dir) {
                if !known.iter().any(|k| registry::same_path(k, &c.dir)) && !found.iter().any(|f| registry::same_path(&f.dir, &c.dir)) {
                    found.push(c);
                }
                continue;
            }
            if d == depth {
                continue;
            }
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            let mut subs: Vec<PathBuf> =
                rd.flatten().filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false)).filter(|e| !skip_dir(&e.file_name().to_string_lossy().to_lowercase())).map(|e| e.path()).collect();
            subs.sort();
            next.extend(subs);
        }
        level = next;
    }
    found
}

/// Registry entry for a candidate. `taken` = names already used; a number is added when needed.
pub fn to_entry(c: &Candidate, taken: &[String]) -> ServerEntry {
    let base = {
        let s = slug(&c.name);
        if s.is_empty() {
            "server".to_string()
        } else {
            s
        }
    };
    let mut name = base.clone();
    let mut n = 2;
    while taken.iter().any(|t| slug(t) == slug(&name)) {
        name = format!("{base}-{n}");
        n += 1;
    }
    ServerEntry { source: None, versions: None, name, dir: c.dir.clone(), mode: c.mode, ports: c.ports, created_unix: now_unix() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(p: &Path) {
        std::fs::create_dir_all(p).unwrap();
    }

    #[test]
    fn parses_the_game_port() {
        assert_eq!(parse_game_port("# x\nendpoint_add_tcp \"0.0.0.0:30121\"\nendpoint_add_udp \"0.0.0.0:30121\""), Some(30121));
        assert_eq!(parse_game_port("endpoint_add_tcp \"[::]:40000\""), Some(40000));
        assert_eq!(parse_game_port("nothing"), None);
    }

    #[test]
    fn scan_finds_each_kind_and_skips_known_and_unrelated() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        // old Kora layout
        mk(&r.join("games/old/.kora"));
        // plain FXServer folder with a custom port
        mk(&r.join("games/plain/txData"));
        std::fs::write(r.join("games/plain/server.cfg"), "endpoint_add_tcp \"0.0.0.0:30200\"\n").unwrap();
        // artifacts layout
        let exe = if cfg!(windows) { "FXServer.exe" } else { "FXServer" };
        mk(&r.join("fx/artifacts/123"));
        std::fs::write(r.join("fx/artifacts/123").join(exe), "").unwrap();
        // already registered + unrelated + hidden
        mk(&r.join("games/known/.kora"));
        mk(&r.join("docs/notes"));
        mk(&r.join(".hidden/x/.kora"));
        // too deep
        mk(&r.join("a/b/c/d/deep/.kora"));
        let known = vec![r.join("games/known")];
        let found = scan(&[r.to_path_buf()], 3, &known);
        let names: Vec<(String, Kind)> = found.iter().map(|c| (c.name.clone(), c.kind)).collect();
        assert!(names.contains(&("old".into(), Kind::OldKora)), "{names:?}");
        assert!(names.contains(&("plain".into(), Kind::FxServer)), "{names:?}");
        assert!(names.contains(&("fx".into(), Kind::FxServer)), "{names:?}");
        assert_eq!(found.len(), 3, "{names:?}");
        assert_eq!(found.iter().find(|c| c.name == "plain").unwrap().ports.game, 30200);
        assert_eq!(found.iter().find(|c| c.name == "old").unwrap().ports, DEFAULT_PORTS);
    }

    #[test]
    fn kora_server_file_gives_name_mode_and_ports() {
        let d = tempfile::tempdir().unwrap();
        let sf = ServerFile {
            source: None,
            versions: None,
            name: "Main".into(),
            mode: Mode::Development,
            dir: d.path().to_path_buf(),
            ports: PortsRecord { game: 30130, txadmin: 40130 },
            game_build: 1,
            artifact_build: 1,
            database: None,
            github_owner: None,
            created_unix: 1,
            kora_version: "x".into(),
            resources: vec![],
        };
        sf.save().unwrap();
        let c = classify(d.path()).unwrap();
        assert_eq!((c.kind, c.name.as_str(), c.mode, c.ports.game), (Kind::Kora, "Main", Mode::Development, 30130));
        assert!(classify(&d.path().join("missing")).is_none());
    }

    #[test]
    fn entry_names_are_unique_slugs() {
        let c = Candidate { dir: "/x/My Server".into(), name: "My Server".into(), kind: Kind::FxServer, mode: Mode::Production, ports: DEFAULT_PORTS };
        let e = to_entry(&c, &[]);
        assert_eq!(e.name, slug("My Server"));
        let e2 = to_entry(&c, std::slice::from_ref(&e.name));
        assert_eq!(e2.name, format!("{}-2", e.name));
    }
}
