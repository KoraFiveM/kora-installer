//! Optional remote access (plan section 1, phase 6): who is asking, what role they have, and what a
//! remote session may do. Local-only (loopback, launch token) stays the default and is not affected.
//!
//! * [`config`]: `panel.json`, hashed per-user tokens.
//! * [`tailscale`]: the `whois` seam (identity comes from the local tailscaled, never from headers).
//! * [`roles`]: viewer / operator / admin, the exhaustive Op classification and request → Op lookup.
pub mod config;
pub mod roles;
pub mod tailscale;

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use serde::Serialize;

pub use config::{RemoteConfig, TokenUser};
pub use roles::Role;
pub use tailscale::{NoWhois, SystemWhois, Whois};

use crate::security::IpLimiter;

/// How a session proved who it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    /// The launch token, from the computer itself.
    Local,
    Tailscale,
    Token,
}

/// The signed-in user as the UI sees it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Identity {
    pub name: String,
    pub provider: Provider,
    pub role: Role,
}

impl Identity {
    pub fn local() -> Self {
        Self {
            name: "local".into(),
            provider: Provider::Local,
            role: Role::Admin,
        }
    }
}

/// Who is on the other end of one request. The guard puts this in the request extensions.
#[derive(Debug, Clone)]
pub struct Caller {
    /// The peer is not on this computer (not a loopback address). A request with no peer information
    /// (tests that call the router directly) counts as local.
    pub remote: bool,
    pub peer: Option<SocketAddr>,
    /// `None`: not signed in (only possible on the public routes).
    pub identity: Option<Identity>,
}

impl Caller {
    pub fn local() -> Self {
        Self {
            remote: false,
            peer: None,
            identity: Some(Identity::local()),
        }
    }
}

pub fn is_remote_peer(ip: IpAddr) -> bool {
    !ip.to_canonical().is_loopback()
}

struct Store {
    cfg: RemoteConfig,
    path: Option<PathBuf>,
    stamp: Option<(SystemTime, u64)>,
}

fn stamp_of(path: &std::path::Path) -> Option<(SystemTime, u64)> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}

/// Shared remote-access state: the settings (reloaded when `panel.json` changes on disk, so
/// `kora panel user add` works while the panel runs), the whois seam and a failed-attempt limiter.
pub struct Remote {
    store: Mutex<Store>,
    /// `--remote` on the command line: on for this run even if the saved settings say off.
    forced: bool,
    whois: Arc<dyn Whois>,
    /// Failed remote sign-ins, per source address. Separate from the local limiter, so strangers cannot
    /// lock the owner out, nor one remote user lock out another.
    pub limiter: IpLimiter,
    dynamic_hosts: Mutex<Vec<String>>,
}

impl Remote {
    /// Settings kept in memory only (nothing on disk is read or written): the default, so tests and
    /// the local-only panel never touch the machine.
    pub fn memory() -> Self {
        Self::new(None, RemoteConfig::default())
    }

    pub fn new(path: Option<PathBuf>, cfg: RemoteConfig) -> Self {
        let stamp = path.as_deref().and_then(stamp_of);
        Self {
            store: Mutex::new(Store { cfg, path, stamp }),
            forced: false,
            whois: Arc::new(NoWhois),
            limiter: IpLimiter::new(10, Duration::from_secs(60), Duration::from_secs(60), 4096),
            dynamic_hosts: Mutex::new(Vec::new()),
        }
    }

    /// Load `path` (missing file: defaults) and keep it in sync with the file.
    pub fn from_file(path: PathBuf) -> Result<Self, String> {
        let cfg = RemoteConfig::load(&path)?;
        Ok(Self::new(Some(path), cfg))
    }

    pub fn with_whois(mut self, w: Arc<dyn Whois>) -> Self {
        self.whois = w;
        self
    }

    pub fn forced(mut self, on: bool) -> Self {
        self.forced = on;
        self
    }

    pub fn is_forced(&self) -> bool {
        self.forced
    }

    fn lock(&self) -> MutexGuard<'_, Store> {
        let mut g = self
            .store
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(p) = g.path.clone() {
            let now = stamp_of(&p);
            if now != g.stamp {
                match (now, RemoteConfig::load(&p)) {
                    (Some(_), Ok(c)) => g.cfg = c,
                    // File deleted: forget its users (removing the file revokes everything).
                    (None, _) => g.cfg = RemoteConfig::default(),
                    // Half-written or damaged: keep what we had until it reads again.
                    (Some(_), Err(_)) => return g,
                }
                g.stamp = now;
            }
        }
        g
    }

    pub fn config(&self) -> RemoteConfig {
        self.lock().cfg.clone()
    }

    pub fn path(&self) -> Option<PathBuf> {
        self.store
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .path
            .clone()
    }

    pub fn enabled(&self) -> bool {
        self.forced || self.lock().cfg.enabled
    }

    /// Change the settings and save them (when there is a file). The change is not kept if saving fails.
    pub fn update<R>(
        &self,
        f: impl FnOnce(&mut RemoteConfig) -> Result<R, String>,
    ) -> Result<R, String> {
        let mut g = self.lock();
        let mut next = g.cfg.clone();
        let r = f(&mut next)?;
        if let Some(p) = g.path.clone() {
            next.save(&p)?;
            g.stamp = stamp_of(&p);
        }
        g.cfg = next;
        Ok(r)
    }

    pub fn whois(&self, peer: SocketAddr) -> Result<String, String> {
        self.whois.whois(peer)
    }

    /// Ask tailscaled for this machine's MagicDNS names and allow them as `Host` values.
    pub fn learn_self_names(&self) {
        let names = self.whois.self_names();
        *self
            .dynamic_hosts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = names;
    }

    pub fn set_dynamic_hosts(&self, names: Vec<String>) {
        *self
            .dynamic_hosts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = names;
    }

    /// Names allowed in the `Host` header besides the bound address: from the settings and tailscale.
    pub fn extra_hosts(&self) -> Vec<String> {
        let mut v = self.lock().cfg.allowed_hosts.clone();
        v.extend(
            self.dynamic_hosts
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .iter()
                .cloned(),
        );
        v
    }

    /// The identity for a stored session, by the settings as they are now: a removed user or a changed
    /// role takes effect at once.
    pub fn resolve(&self, provider: Provider, name: &str) -> Option<Identity> {
        match provider {
            Provider::Local => Some(Identity::local()),
            Provider::Token => {
                let g = self.lock();
                let u = g.cfg.user(name)?;
                Some(Identity {
                    name: u.name.clone(),
                    provider,
                    role: u.role,
                })
            }
            Provider::Tailscale => {
                let g = self.lock();
                if !g.cfg.tailscale {
                    return None;
                }
                let role = g.cfg.tailscale_role(name)?;
                Some(Identity {
                    name: name.to_string(),
                    provider,
                    role,
                })
            }
        }
    }

    pub fn identity_for_token(&self, token: &str) -> Option<Identity> {
        let g = self.lock();
        let u = g.cfg.user_for_token(token)?;
        Some(Identity {
            name: u.name.clone(),
            provider: Provider::Token,
            role: u.role,
        })
    }

    pub fn identity_for_tailscale(&self, login: &str) -> Option<Identity> {
        self.resolve(Provider::Tailscale, login)
    }
}

/// Split `host`, `host:port` or `[v6]:port` into (host, port).
fn split_host(h: &str) -> (&str, Option<&str>) {
    if let Some(rest) = h.strip_prefix('[') {
        return match rest.split_once(']') {
            Some((ip, tail)) => (ip, tail.strip_prefix(':')),
            None => (h, None),
        };
    }
    match h.matches(':').count() {
        1 => {
            let (n, p) = h.split_once(':').unwrap_or((h, ""));
            (n, Some(p))
        }
        _ => (h, None), // no port, or a bare IPv6 address
    }
}

/// Is the `Host` header `host` one of the configured `names` on the panel's `port`?
/// A name may carry its own port in the settings; otherwise the panel's port is required.
pub fn extra_host_allowed(host: &str, names: &[String], port: u16) -> bool {
    let (hn, hp) = split_host(host);
    let port_ok = match hp {
        Some(p) => p.parse::<u16>().ok() == Some(port),
        None => port == 80,
    };
    names.iter().any(|entry| {
        let (en, ep) = split_host(entry);
        let entry_port_ok = match ep {
            Some(p) => p.parse::<u16>().ok() == Some(port),
            None => true,
        };
        en.eq_ignore_ascii_case(hn) && port_ok && entry_port_ok
    })
}

pub const PLAIN_HTTP_WARNING: &str =
    "Warning: remote access is on. Traffic to this panel is plain HTTP (not encrypted). \
Only use it over a private network such as Tailscale or WireGuard, or through an SSH tunnel.";

/// Decide whether the panel may listen on `bind`, and what to tell the person who started it.
/// Loopback is always fine and says nothing. Anything else needs remote access switched on, and
/// "all interfaces" (`0.0.0.0` / `::`) also needs the names people will use, because the `Host` check
/// cannot guess them.
pub fn bind_plan(
    bind: IpAddr,
    remote_enabled: bool,
    cfg: &RemoteConfig,
    learned_names: &[String],
) -> Result<Vec<String>, String> {
    if bind.to_canonical().is_loopback() {
        return Ok(Vec::new());
    }
    if !remote_enabled {
        return Err(format!(
            "To listen on {bind} you must turn remote access on: run `kora panel --bind {bind} --remote`, \
or turn it on in Settings on this computer first. Without it the panel only listens on 127.0.0.1."
        ));
    }
    let mut warnings = vec![PLAIN_HTTP_WARNING.to_string()];
    if bind.is_unspecified() {
        if cfg.allowed_hosts.is_empty() && learned_names.is_empty() {
            return Err(format!(
                "Listening on {bind} means every network interface, and Kora cannot tell which names people will use. \
Bind to one address instead (for example your Tailscale address), or add the names to allowed_hosts in the remote access settings."
            ));
        }
        warnings.push(format!("Warning: {bind} listens on EVERY network interface of this computer, not just a private one."));
    }
    if cfg.users.is_empty() && !(cfg.tailscale && !cfg.tailscale_users.is_empty()) {
        warnings.push("Nobody can sign in remotely yet. Add a user with: kora panel user add <name> --role admin".into());
    }
    Ok(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with_user() -> RemoteConfig {
        let mut c = RemoteConfig::default();
        c.add_user("a", Role::Admin).unwrap();
        c
    }

    #[test]
    fn loopback_binds_need_nothing() {
        let c = RemoteConfig::default();
        assert!(bind_plan("127.0.0.1".parse().unwrap(), false, &c, &[])
            .unwrap()
            .is_empty());
        assert!(bind_plan("::1".parse().unwrap(), false, &c, &[])
            .unwrap()
            .is_empty());
    }

    #[test]
    fn a_non_loopback_bind_needs_remote_access() {
        let c = cfg_with_user();
        let e = bind_plan("100.64.0.5".parse().unwrap(), false, &c, &[]).unwrap_err();
        assert!(e.contains("--remote"), "{e}");
        let w = bind_plan("100.64.0.5".parse().unwrap(), true, &c, &[]).unwrap();
        assert!(w[0].contains("plain HTTP") && w[0].contains("Tailscale"));
        assert_eq!(
            w.len(),
            1,
            "a user exists, so no 'nobody can sign in' warning"
        );
    }

    #[test]
    fn warns_when_nobody_can_sign_in() {
        let w = bind_plan(
            "100.64.0.5".parse().unwrap(),
            true,
            &RemoteConfig::default(),
            &[],
        )
        .unwrap();
        assert!(w.iter().any(|m| m.contains("kora panel user add")));
    }

    #[test]
    fn all_interfaces_needs_names_and_a_loud_warning() {
        let any: IpAddr = "0.0.0.0".parse().unwrap();
        assert!(bind_plan(any, true, &cfg_with_user(), &[]).is_err());
        assert!(bind_plan(any, false, &cfg_with_user(), &["box.ts.net".into()]).is_err());
        let mut c = cfg_with_user();
        c.allowed_hosts.push("box.example.com".into());
        let w = bind_plan(any, true, &c, &[]).unwrap();
        assert!(w.iter().any(|m| m.contains("EVERY network interface")));
        let w = bind_plan(
            "::".parse().unwrap(),
            true,
            &cfg_with_user(),
            &["box.ts.net".into()],
        )
        .unwrap();
        assert!(w.iter().any(|m| m.contains("EVERY")));
    }

    #[test]
    fn extra_hosts_need_the_panel_port() {
        let names = vec![
            "box.tail.ts.net".to_string(),
            "100.64.0.9:47120".to_string(),
            "fd7a::1".to_string(),
        ];
        assert!(extra_host_allowed("box.tail.ts.net:47120", &names, 47120));
        assert!(extra_host_allowed("BOX.tail.ts.net:47120", &names, 47120));
        assert!(!extra_host_allowed("box.tail.ts.net:9999", &names, 47120));
        assert!(!extra_host_allowed("box.tail.ts.net", &names, 47120));
        assert!(!extra_host_allowed("evil.example.com:47120", &names, 47120));
        assert!(extra_host_allowed("100.64.0.9:47120", &names, 47120));
        assert!(extra_host_allowed("[fd7a::1]:47120", &names, 47120));
        assert!(extra_host_allowed("box.tail.ts.net", &names, 80));
    }

    #[test]
    fn config_file_changes_are_picked_up() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(config::FILE_NAME);
        let r = Remote::from_file(p.clone()).unwrap();
        assert!(r.config().users.is_empty());
        // Another process (the `kora panel user add` command) writes the file.
        let mut c = RemoteConfig::default();
        let tok = c.add_user("zed", Role::Operator).unwrap();
        c.save(&p).unwrap();
        assert_eq!(r.identity_for_token(&tok).unwrap().role, Role::Operator);
        // Removing the file revokes everyone.
        std::fs::remove_file(&p).unwrap();
        assert!(r.identity_for_token(&tok).is_none());
    }
}
