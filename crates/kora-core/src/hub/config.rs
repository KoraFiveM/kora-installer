//! The nodes: in the Kora home's private database (`hub_nodes`, see `store::privacy` for who can read it;
//! `<kora home>/hub.json` before v0.8). The token lives only here and is never returned by the API.
use crate::problem::Problem;
use crate::store::{problem_to_string, Store};
use std::net::{IpAddr, Ipv6Addr};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeConfig {
    pub id: String,
    pub name: String,
    pub url: String,
    pub token: String,
}

// Never print the token, even in a debug dump or a failed assertion.
impl std::fmt::Debug for NodeConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeConfig").field("id", &self.id).field("name", &self.name).field("url", &self.url).field("token", &"<hidden>").finish()
    }
}

/// A checked node address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UrlCheck {
    /// Normalised: no trailing slash.
    pub url: String,
    /// Plain `http://` to an address that is neither private nor Tailscale: the token and the data would
    /// cross the internet unencrypted.
    pub insecure: bool,
}

fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            v.is_loopback() || v.is_private() || v.is_link_local() || (o[0] == 100 && (64..128).contains(&o[1]))
        }
        IpAddr::V6(v) => {
            let s = v.segments();
            v.is_loopback() || (s[0] & 0xfe00) == 0xfc00 || (s[0] & 0xffc0) == 0xfe80 || is_tailscale_v6(v)
        }
    }
}

fn is_tailscale_v6(v: Ipv6Addr) -> bool {
    // fd7a:115c:a1e0::/48
    let s = v.segments();
    s[0] == 0xfd7a && s[1] == 0x115c && s[2] == 0xa1e0
}

fn host_is_private(host: &str) -> bool {
    let h = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = h.parse::<IpAddr>() {
        return is_private(ip);
    }
    let l = h.to_ascii_lowercase();
    // Names that stay on the machine, the LAN or the tailnet; a name without dots is a LAN name.
    l == "localhost" || l.ends_with(".localhost") || l.ends_with(".local") || l.ends_with(".lan") || l.ends_with(".ts.net") || !l.contains('.')
}

/// Accept only `http://` or `https://` addresses with a host, no login part, query or fragment.
pub fn check_url(raw: &str) -> Result<UrlCheck, String> {
    let raw = raw.trim();
    let (https, rest) = if let Some(r) = raw.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = raw.strip_prefix("http://") {
        (false, r)
    } else {
        return Err("The address must start with http:// or https://.".into());
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    if authority.is_empty() {
        return Err("The address has no host name.".into());
    }
    if authority.contains('@') {
        return Err("Leave user names and passwords out of the address; the token is entered separately.".into());
    }
    if path.contains('?') || path.contains('#') || authority.contains(' ') || raw.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("The address must be just the panel's web address, for example http://192.168.1.20:47120.".into());
    }
    // Host and optional port.
    let (host, port) = if let Some(r) = authority.strip_prefix('[') {
        let end = r.find(']').ok_or("The address is not valid.")?;
        (&authority[..end + 2], r[end + 1..].strip_prefix(':'))
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (authority, None),
        }
    };
    if host.is_empty() || host == "[]" {
        return Err("The address has no host name.".into());
    }
    if let Some(p) = port {
        if p.parse::<u16>().map_or(true, |n| n == 0) {
            return Err("The port in the address is not valid.".into());
        }
    }
    let path = path.trim_end_matches('/');
    let url = format!("{}://{authority}{path}", if https { "https" } else { "http" });
    Ok(UrlCheck { url, insecure: !https && !host_is_private(host) })
}

/// Where the nodes live and how they are read and written. Cheap to construct. They are in the Kora home's
/// private database (`hub_nodes`); `path` (`<kora home>/hub.json` before v0.8) only says which Kora home.
#[derive(Debug, Clone)]
pub struct NodeStore {
    /// `<kora home>/hub.json`: only names the Kora home.
    path: PathBuf,
    db: PathBuf,
}

fn read_nodes(c: &rusqlite::Connection) -> Result<Vec<NodeConfig>, Problem> {
    let mut st = c.prepare("SELECT id, name, url, token FROM hub_nodes ORDER BY seq")?;
    let v = st.query_map([], |r| Ok(NodeConfig { id: r.get(0)?, name: r.get(1)?, url: r.get(2)?, token: r.get(3)? }))?.collect::<Result<Vec<_>, _>>()?;
    Ok(v)
}

fn write_nodes(c: &rusqlite::Connection, nodes: &[NodeConfig]) -> Result<(), Problem> {
    c.execute("DELETE FROM hub_nodes", [])?;
    for n in nodes {
        c.execute(
            "INSERT INTO hub_nodes (id, name, url, token) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(id) DO UPDATE SET name = excluded.name, url = excluded.url, token = excluded.token",
            rusqlite::params![n.id, n.name, n.url, n.token],
        )?;
    }
    Ok(())
}

fn blocked(s: &Store) -> Result<(), String> {
    match s.blocked("hub") {
        Some(b) => Err(format!("The nodes file ({}) could not be read ({}); it was left exactly as it was. Fix or move it and try again.", b.file.display(), b.why)),
        None => Ok(()),
    }
}

impl NodeStore {
    pub fn at(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let db = crate::store::db_path(path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new(".")));
        Self { path, db }
    }
    /// The nodes of this process's Kora home.
    pub fn default_store() -> Self {
        Self::at(crate::installer::registry::kora_home().join("hub.json"))
    }
    /// Where the nodes are saved: the store's database file.
    pub fn path(&self) -> &Path {
        &self.db
    }

    pub fn load(&self) -> Result<Vec<NodeConfig>, String> {
        let Some(s) = Store::open_existing_beside(&self.path).map_err(problem_to_string)? else { return Ok(vec![]) };
        blocked(&s)?;
        s.read(|c| read_nodes(c)).map_err(problem_to_string)
    }

    /// Replace the whole list (prefer [`add`](Self::add) and [`remove`](Self::remove): each is one transaction).
    pub fn save(&self, nodes: &[NodeConfig]) -> Result<(), String> {
        let s = Store::open_beside(&self.path).map_err(problem_to_string)?;
        blocked(&s)?;
        s.write(|tx| write_nodes(tx, nodes)).map_err(problem_to_string)
    }

    /// Add a node (checked by the caller); refuses a repeated name or address.
    pub fn add(&self, name: &str, url: &str, token: &str) -> Result<NodeConfig, String> {
        let s = Store::open_beside(&self.path).map_err(problem_to_string)?;
        blocked(&s)?;
        let mut refused: Option<String> = None;
        let r = s.write(|tx| {
            let mut nodes = read_nodes(tx)?;
            if nodes.iter().any(|n| n.name.eq_ignore_ascii_case(name)) {
                refused = Some(format!("There is already a node called '{name}'."));
                return Err(Problem::generic("refused"));
            }
            if nodes.iter().any(|n| n.url == url) {
                refused = Some("That node is already in the list.".into());
                return Err(Problem::generic("refused"));
            }
            let node = NodeConfig { id: new_id(&nodes), name: name.into(), url: url.into(), token: token.into() };
            nodes.push(node.clone());
            write_nodes(tx, &nodes)?;
            Ok(node)
        });
        r.map_err(|p| refused.unwrap_or_else(|| problem_to_string(p)))
    }

    /// `Ok(false)` when there was no such node.
    pub fn remove(&self, id: &str) -> Result<bool, String> {
        let Some(s) = Store::open_existing_beside(&self.path).map_err(problem_to_string)? else { return Ok(false) };
        blocked(&s)?;
        s.write(|tx| {
            let n = tx.execute("DELETE FROM hub_nodes WHERE id = ?1", [id])?;
            Ok(n > 0)
        })
        .map_err(problem_to_string)
    }

    pub fn get(&self, id: &str) -> Result<Option<NodeConfig>, String> {
        Ok(self.load()?.into_iter().find(|n| n.id == id))
    }
}

fn new_id(existing: &[NodeConfig]) -> String {
    loop {
        let mut b = [0u8; 4];
        let _ = getrandom::fill(&mut b);
        let id = format!("n{}", hex::encode(b));
        if !existing.iter().any(|n| n.id == id) {
            return id;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(check_url("http://192.168.1.5:47120/").unwrap(), UrlCheck { url: "http://192.168.1.5:47120".into(), insecure: false });
        assert!(!check_url("http://100.101.1.2:1").unwrap().insecure);
        assert!(!check_url("http://box.tail1234.ts.net:47120").unwrap().insecure);
        assert!(!check_url("http://localhost:1").unwrap().insecure);
        assert!(!check_url("http://[::1]:1").unwrap().insecure);
        assert!(check_url("http://8.8.8.8:47120").unwrap().insecure);
        assert!(check_url("http://100.128.0.1").unwrap().insecure);
        assert!(check_url("http://panel.example.com").unwrap().insecure);
        assert!(!check_url("https://panel.example.com").unwrap().insecure);
        for bad in ["ftp://x", "panel", "http://", "http://u:p@host", "http://h/?a=1", "http://h:99999", "http://h:0", "https://", "javascript:alert(1)", "http://a b"] {
            assert!(check_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn store_round_trip_and_mode() {
        let d = tempfile::tempdir().unwrap();
        let s = NodeStore::at(d.path().join("hub.json"));
        assert!(s.load().unwrap().is_empty());
        let n = s.add("Home", "http://10.0.0.2:47120", "sekret").unwrap();
        assert!(s.add("home", "http://10.0.0.3:1", "x").is_err());
        assert!(s.add("Other", "http://10.0.0.2:47120", "x").is_err());
        assert_eq!(s.get(&n.id).unwrap().unwrap().token, "sekret");
        assert!(!format!("{n:?}").contains("sekret"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(crate::store::db_path(d.path())).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert!(s.remove(&n.id).unwrap());
        assert!(!s.remove(&n.id).unwrap());
    }
}
