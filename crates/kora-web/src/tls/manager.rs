//! Choosing, loading, watching and renewing the certificate. [`Manager::reload`] applies the saved
//! [`TlsConfig`]; [`Manager::maintain`] runs for as long as the panel does: it notices a changed setting (the
//! CLI wrote it), a changed certificate file (certbot, a script), and a certificate close to its end.
//!
//! Every failure keeps what is already being served: an unreadable renewal never turns HTTPS off.
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kora_core::store::Store;

use super::certs::{self, CertSummary};
use super::config::{TlsConfig, TlsMode};
use super::{selfsigned, tsnet, CertInfo, CertSource, TlsState};
use crate::remote::Whois;

/// What the settings screen, `kora panel tls status` and the System check show.
#[derive(Debug, Clone)]
pub struct Report {
    pub mode: TlsMode,
    pub info: Option<CertInfo>,
    pub days_left: Option<i64>,
    pub error: Option<String>,
    pub cert_path: Option<String>,
    pub key_path: Option<String>,
    /// The local CA's fingerprint, when one was made (to compare with the file a person installs).
    pub ca_fingerprint: Option<String>,
}

/// How often a missing Tailscale certificate is asked for again.
pub const RETRY_EVERY: Duration = Duration::from_secs(300);

pub fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// This computer's name, for the certificate and the CA's label.
pub fn machine_name() -> String {
    for k in ["HOSTNAME", "COMPUTERNAME"] {
        if let Ok(v) = std::env::var(k) {
            if !v.is_empty() {
                return v;
            }
        }
    }
    std::fs::read_to_string("/etc/hostname").map(|s| s.trim().to_string()).ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "this-computer".into())
}

/// Facts the panel supplies when it starts: the names and addresses people will use.
#[derive(Debug, Clone, Default)]
pub struct Hints {
    /// Names or addresses (`allowed_hosts`, the bind address, ...), with or without ports.
    pub names: Vec<String>,
    /// The panel listens on Tailscale addresses: a Tailscale certificate may be wanted.
    pub tailnet: bool,
}

pub struct Manager {
    home: PathBuf,
    pub state: TlsState,
    whois: Arc<dyn Whois>,
    hints: Mutex<Hints>,
    /// The setting and files the loaded certificate came from, to notice a change.
    seen: Mutex<Option<(TlsConfig, Vec<Option<std::time::SystemTime>>)>>,
    last_try: Mutex<Option<std::time::Instant>>,
    /// The day (days since 1970) the expiry warning was last given, so it is given once a day.
    warned_day: Mutex<Option<i64>>,
}

impl Manager {
    pub fn new(home: PathBuf, whois: Arc<dyn Whois>) -> Self {
        Self { home, state: TlsState::new(), whois, hints: Mutex::default(), seen: Mutex::default(), last_try: Mutex::default(), warned_day: Mutex::default() }
    }
    pub fn set_hints(&self, h: Hints) {
        *self.hints.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = h;
    }
    fn hints(&self) -> Hints {
        self.hints.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }
    fn store(&self) -> Result<Store, String> {
        Store::open(&self.home).map_err(kora_core::store::problem_to_string)
    }

    fn info_of(sum: &CertSummary, source: CertSource, ca_fingerprint: Option<String>) -> CertInfo {
        CertInfo { source, names: sum.names.clone(), not_before: sum.not_before, not_after: sum.not_after, fingerprint: sum.fingerprint.clone(), ca_fingerprint }
    }

    fn install_pem(&self, cert_pem: &str, key_pem: &str, source: CertSource, ca_fp: Option<String>) -> Result<CertInfo, String> {
        let chain = certs::parse_chain(cert_pem.as_bytes())?;
        let key = certs::parse_key(key_pem.as_bytes())?;
        let sum = certs::summarise(&chain[0])?;
        if certs::days_left(sum.not_after, now_unix()) < 0 {
            return Err("the certificate has expired".into());
        }
        let ck = certs::certified_key(chain, &key)?;
        let info = Self::info_of(&sum, source, ca_fp);
        self.state.install(ck, info.clone());
        Ok(info)
    }

    fn load_provided(&self, cfg: &TlsConfig) -> Result<CertInfo, String> {
        let (Some(c), Some(k)) = (&cfg.cert_path, &cfg.key_path) else {
            return Err("no certificate files are set (use `kora panel tls set --cert <file> --key <file>`)".into());
        };
        certs::check_key_file(Path::new(k))?;
        let cert = std::fs::read_to_string(c).map_err(|e| format!("cannot read the certificate file {c} ({e})"))?;
        let key = std::fs::read_to_string(k).map_err(|e| format!("cannot read the key file {k} ({e})"))?;
        let info = self.install_pem(&cert, &key, CertSource::Provided, None)?;
        // A certificate for other names than the ones people will type is the usual mistake: say so.
        let wanted = self.hints().names;
        let missing: Vec<&String> = wanted.iter().filter(|w| !certs::covers(&info.names, selfsigned::host_only(w))).collect();
        if !missing.is_empty() && !wanted.is_empty() {
            self.state.set_error(Some(format!(
                "The certificate does not cover {}: browsers will warn when the panel is opened by that name. It covers {}.",
                missing.iter().map(|m| m.as_str()).collect::<Vec<_>>().join(", "),
                info.names.join(", ")
            )));
        }
        Ok(info)
    }

    fn load_tailscale(&self, fetch: bool) -> Result<CertInfo, String> {
        let status = self.whois.status().filter(|s| s.running);
        let name = status.and_then(|s| s.dns_name).ok_or_else(|| tsnet::NO_NAME.to_string())?;
        let store = self.store()?;
        let cached = tsnet::load(&store)?;
        if let Some(c) = &cached {
            if (tsnet::fresh(c, &name, now_unix()) || !fetch) && c.name.eq_ignore_ascii_case(&name) {
                return self.install_pem(&c.cert_pem, &c.key_pem, CertSource::Tailscale, None);
            }
        }
        if !fetch {
            return Err("Tailscale HTTPS has not been set up yet (the panel fetches the certificate when it starts).".into());
        }
        match self.whois.cert(&name) {
            Ok((cert, key)) => {
                let chain = certs::parse_chain(cert.as_bytes())?;
                let sum = certs::summarise(&chain[0])?;
                tsnet::save(&store, &tsnet::Cached { name: name.clone(), cert_pem: cert.clone(), key_pem: key.clone(), not_after: sum.not_after })?;
                self.install_pem(&cert, &key, CertSource::Tailscale, None)
            }
            Err(e) => {
                // A failed renewal keeps the old certificate while it is still valid.
                if let Some(c) = cached.filter(|c| c.name.eq_ignore_ascii_case(&name)) {
                    if let Ok(info) = self.install_pem(&c.cert_pem, &c.key_pem, CertSource::Tailscale, None) {
                        self.state.set_error(Some(format!("Could not renew the Tailscale certificate yet: {e}")));
                        return Ok(info);
                    }
                }
                Err(e)
            }
        }
    }

    fn load_self_signed(&self, cfg: &TlsConfig) -> Result<CertInfo, String> {
        let store = self.store()?;
        let mut names = cfg.names.clone();
        names.extend(self.hints().names);
        names.push(machine_name());
        if let Some(ip) = selfsigned::primary_lan_address() {
            names.push(ip.to_string());
        }
        if let Some(s) = self.whois.status() {
            names.extend(s.host_names());
        }
        let wanted = selfsigned::default_names(&names);
        let label = machine_name();
        let leaf = selfsigned::ensure_leaf(&store, &label, &wanted)?;
        let ca = selfsigned::load_ca(&store)?.ok_or("the local CA is missing")?;
        let ca_fp = certs::parse_chain(ca.cert_pem.as_bytes()).ok().map(|c| certs::fingerprint(&c[0]));
        self.install_pem(&leaf.cert_pem, &leaf.key_pem, CertSource::SelfSigned, ca_fp)
    }

    fn stamp(cfg: &TlsConfig) -> Vec<Option<std::time::SystemTime>> {
        [&cfg.cert_path, &cfg.key_path].iter().map(|p| p.as_ref().and_then(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())).collect()
    }

    /// Apply the saved settings. `fetch_tailscale`: ask `tailscale cert` when the cached one is old or missing
    /// (the CLI's `status` does not). Returns the certificate now in use, `None` for plain HTTP.
    pub fn reload(&self, fetch_tailscale: bool) -> Result<Option<CertInfo>, String> {
        let cfg = TlsConfig::load(&self.store()?)?;
        let result = match cfg.mode {
            TlsMode::Off => {
                self.state.clear();
                Ok(None)
            }
            TlsMode::Provided => self.load_provided(&cfg).map(Some),
            TlsMode::Tailscale => self.load_tailscale(fetch_tailscale).map(Some),
            TlsMode::SelfSigned => self.load_self_signed(&cfg).map(Some),
            TlsMode::Auto => {
                if cfg.cert_path.is_some() {
                    self.load_provided(&cfg).map(Some)
                } else if self.hints().tailnet {
                    // Tailscale HTTPS is a bonus: when it is not enabled, plain HTTP inside the tunnel is as before.
                    match self.load_tailscale(fetch_tailscale) {
                        Ok(i) => Ok(Some(i)),
                        Err(e) => {
                            self.state.set_error(Some(e));
                            Ok(None)
                        }
                    }
                } else {
                    Ok(None)
                }
            }
        };
        *self.seen.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some((cfg.clone(), Self::stamp(&cfg)));
        *self.last_try.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(std::time::Instant::now());
        match result {
            Ok(i) => Ok(i),
            Err(e) => {
                // Keep serving what we had; show why the new one was not taken.
                self.state.set_error(Some(e.clone()));
                Err(e)
            }
        }
    }

    pub fn report(&self) -> Report {
        let cfg = self.store().and_then(|s| TlsConfig::load(&s)).unwrap_or_default();
        let info = self.state.info();
        let ca_fingerprint = self.store().ok().and_then(|s| selfsigned::load_ca(&s).ok().flatten()).and_then(|ca| certs::parse_chain(ca.cert_pem.as_bytes()).ok()).map(|c| certs::fingerprint(&c[0]));
        Report {
            mode: cfg.mode,
            days_left: info.as_ref().map(|i| certs::days_left(i.not_after, now_unix())),
            info,
            error: self.state.last_error(),
            cert_path: cfg.cert_path,
            key_path: cfg.key_path,
            ca_fingerprint,
        }
    }

    /// Is a refresh due: the setting or the files changed, or the certificate is close to its end?
    fn due(&self) -> bool {
        let cfg = match self.store().and_then(|s| TlsConfig::load(&s)) {
            Ok(c) => c,
            Err(_) => return false,
        };
        let seen = self.seen.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        let changed = match &seen {
            None => true,
            Some((c, stamp)) => *c != cfg || *stamp != Self::stamp(&cfg),
        };
        let renew = self.state.info().is_some_and(|i| match i.source {
            CertSource::Tailscale => certs::days_left(i.not_after, now_unix()) <= tsnet::RENEW_DAYS,
            CertSource::SelfSigned => certs::days_left(i.not_after, now_unix()) <= selfsigned::RENEW_DAYS,
            CertSource::Provided => false,
        });
        // Tailscale may come up after the panel (at boot): try again every few minutes until there is a certificate.
        let retry = !self.state.active()
            && matches!(cfg.mode, TlsMode::Auto | TlsMode::Tailscale)
            && (self.hints().tailnet || cfg.mode == TlsMode::Tailscale)
            && self.last_try.lock().unwrap_or_else(std::sync::PoisonError::into_inner).is_none_or(|t| t.elapsed() >= RETRY_EVERY);
        changed || renew || retry
    }

    /// A certificate in its last 14 days: one line on the terminal and one in the activity log, once a day (Settings and the System
    /// check show it all the time).
    fn warn_expiry(&self) {
        let Some(i) = self.state.info() else { return };
        let days = certs::days_left(i.not_after, now_unix());
        let today = now_unix().div_euclid(86_400);
        if days > 14 || *self.warned_day.lock().unwrap_or_else(std::sync::PoisonError::into_inner) == Some(today) {
            return;
        }
        *self.warned_day.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(today);
        let what = if days < 0 { "has EXPIRED".to_string() } else { format!("ends in {days} days") };
        eprintln!("WARNING: the panel's HTTPS certificate ({}) {what}. See `kora panel tls status`.", i.source.label());
        kora_core::store::audit::record_quiet(
            &self.home,
            &kora_core::store::audit::New {
                op: "tls_expiry".into(),
                actor_kind: kora_core::store::audit::actor::LOCAL.into(),
                actor_name: Some("kora panel".into()),
                result: kora_core::store::audit::result::FAILED.into(),
                detail: Some(format!("the HTTPS certificate {what}")),
                ..Default::default()
            },
        );
    }

    /// For as long as the panel runs: check every 20 seconds, reload when something changed.
    pub async fn maintain(self: Arc<Self>) {
        loop {
            let m = self.clone();
            let _ = tokio::task::spawn_blocking(move || {
                if m.due() {
                    let _ = m.reload(true);
                }
                m.warn_expiry();
            })
            .await;
            tokio::time::sleep(Duration::from_secs(20)).await;
        }
    }
}

// ---- operations for the CLI and the settings screen ----------------------------------------------------------------

/// Save a new setting after checking that it loads: nothing is saved when it does not.
pub fn apply(home: &Path, whois: Arc<dyn Whois>, hints: Hints, new: TlsConfig, fetch_tailscale: bool) -> Result<(Manager, Option<CertInfo>), String> {
    let store = Store::open(home).map_err(kora_core::store::problem_to_string)?;
    let old = TlsConfig::load(&store)?;
    new.save(&store)?;
    let m = Manager::new(home.to_path_buf(), whois);
    m.set_hints(hints);
    match m.reload(fetch_tailscale) {
        Ok(i) => Ok((m, i)),
        Err(e) => {
            old.save(&store)?;
            Err(e)
        }
    }
}

/// Throw away what a renewal replaces (the Tailscale cache, the made certificate; never the CA) and note it in the
/// setting so a running panel fetches or makes a new one.
pub fn force_renewal(home: &Path) -> Result<(), String> {
    let store = Store::open(home).map_err(kora_core::store::problem_to_string)?;
    store
        .write(|tx| {
            tx.execute("DELETE FROM tls_tailscale", [])?;
            tx.execute("DELETE FROM tls_local_leaf", [])?;
            Ok(())
        })
        .map_err(kora_core::store::problem_to_string)?;
    let mut c = TlsConfig::load(&store)?;
    c.renewed += 1;
    c.save(&store)
}

/// Write the local CA's certificate (public) where a person can pick it up to install it in a browser.
pub fn export_ca(home: &Path, to: &Path) -> Result<String, String> {
    let store = Store::open(home).map_err(kora_core::store::problem_to_string)?;
    let ca = selfsigned::load_ca(&store)?.ok_or("There is no local certificate authority yet: run `kora panel tls self-signed` first.")?;
    std::fs::write(to, ca.cert_pem.as_bytes()).map_err(|e| format!("could not write {} ({e})", to.display()))?;
    let chain = certs::parse_chain(ca.cert_pem.as_bytes())?;
    Ok(certs::fingerprint(&chain[0]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::{NoWhois, TsSelf};
    use crate::tls::certs::testcerts;

    struct Ts {
        name: Option<String>,
        cert: Mutex<Result<(String, String), String>>,
        calls: Mutex<u32>,
    }
    impl Whois for Ts {
        fn whois(&self, _: std::net::SocketAddr) -> Result<String, String> {
            Err("no".into())
        }
        fn status(&self) -> Option<TsSelf> {
            Some(TsSelf { running: true, dns_name: self.name.clone(), ips: vec!["100.64.0.5".parse().unwrap()], ..TsSelf::default() })
        }
        fn cert(&self, _: &str) -> Result<(String, String), String> {
            *self.calls.lock().unwrap() += 1;
            self.cert.lock().unwrap().clone()
        }
    }

    fn mgr(w: Arc<dyn Whois>) -> (tempfile::TempDir, Manager) {
        let d = tempfile::tempdir().unwrap();
        (Manager::new(d.path().to_path_buf(), w), d).swap()
    }
    trait Swap {
        fn swap(self) -> (tempfile::TempDir, Manager);
    }
    impl Swap for (Manager, tempfile::TempDir) {
        fn swap(self) -> (tempfile::TempDir, Manager) {
            (self.1, self.0)
        }
    }

    fn set(m: &Manager, c: TlsConfig) {
        c.save(&Store::open(&m.home).unwrap()).unwrap();
    }

    #[cfg(unix)]
    fn private(p: &Path) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    #[cfg(not(unix))]
    fn private(_: &Path) {}

    #[test]
    fn auto_without_anything_is_plain_http() {
        let (_d, m) = mgr(Arc::new(NoWhois));
        assert!(m.reload(false).unwrap().is_none());
        assert!(!m.state.active());
    }

    #[test]
    fn a_provided_certificate_is_loaded_and_a_changed_file_is_picked_up() {
        let (d, m) = mgr(Arc::new(NoWhois));
        let (c1, k1) = testcerts::make(&["box.example.com"], 60);
        let (cp, kp) = (d.path().join("c.pem"), d.path().join("k.pem"));
        std::fs::write(&cp, &c1).unwrap();
        std::fs::write(&kp, &k1).unwrap();
        private(&kp);
        set(&m, TlsConfig { mode: TlsMode::Provided, cert_path: Some(cp.to_string_lossy().into()), key_path: Some(kp.to_string_lossy().into()), names: vec![], renewed: 0 });
        let i = m.reload(false).unwrap().unwrap();
        assert_eq!(i.source, CertSource::Provided);
        assert_eq!(i.names, vec!["box.example.com"]);
        assert!(m.state.active() && m.state.hsts());
        assert!(!m.due(), "nothing changed");
        // certbot renews: new files in place
        std::thread::sleep(Duration::from_millis(1100));
        let (c2, k2) = testcerts::make(&["box.example.com"], 90);
        std::fs::write(&cp, &c2).unwrap();
        std::fs::write(&kp, &k2).unwrap();
        private(&kp);
        assert!(m.due(), "the files changed");
        let j = m.reload(false).unwrap().unwrap();
        assert_ne!(i.fingerprint, j.fingerprint);
    }

    #[cfg(unix)]
    #[test]
    fn an_open_key_file_is_refused_and_the_old_certificate_stays() {
        use std::os::unix::fs::PermissionsExt;
        let (d, m) = mgr(Arc::new(NoWhois));
        let (c1, k1) = testcerts::make(&["a.example"], 60);
        let (cp, kp) = (d.path().join("c.pem"), d.path().join("k.pem"));
        std::fs::write(&cp, &c1).unwrap();
        std::fs::write(&kp, &k1).unwrap();
        private(&kp);
        set(&m, TlsConfig { mode: TlsMode::Provided, cert_path: Some(cp.to_string_lossy().into()), key_path: Some(kp.to_string_lossy().into()), names: vec![], renewed: 0 });
        m.reload(false).unwrap();
        let before = m.state.info().unwrap().fingerprint;
        std::fs::set_permissions(&kp, std::fs::Permissions::from_mode(0o644)).unwrap();
        let e = m.reload(false).unwrap_err();
        assert!(e.contains("chmod 600"), "{e}");
        assert_eq!(m.state.info().unwrap().fingerprint, before, "still serving the old one");
        assert!(m.report().error.unwrap().contains("chmod 600"));
    }

    #[test]
    fn a_certificate_for_other_names_is_flagged() {
        let (d, m) = mgr(Arc::new(NoWhois));
        m.set_hints(Hints { names: vec!["panel.lan:47120".into()], tailnet: false });
        let (c, k) = testcerts::make(&["other.example"], 60);
        let (cp, kp) = (d.path().join("c.pem"), d.path().join("k.pem"));
        std::fs::write(&cp, &c).unwrap();
        std::fs::write(&kp, &k).unwrap();
        private(&kp);
        set(&m, TlsConfig { mode: TlsMode::Provided, cert_path: Some(cp.to_string_lossy().into()), key_path: Some(kp.to_string_lossy().into()), names: vec![], renewed: 0 });
        m.reload(false).unwrap();
        assert!(m.state.last_error().unwrap().contains("panel.lan"), "{:?}", m.state.last_error());
    }

    #[test]
    fn an_expired_certificate_is_refused() {
        let (d, m) = mgr(Arc::new(NoWhois));
        let (c, k) = testcerts::make(&["a.example"], -2);
        let (cp, kp) = (d.path().join("c.pem"), d.path().join("k.pem"));
        std::fs::write(&cp, &c).unwrap();
        std::fs::write(&kp, &k).unwrap();
        private(&kp);
        set(&m, TlsConfig { mode: TlsMode::Provided, cert_path: Some(cp.to_string_lossy().into()), key_path: Some(kp.to_string_lossy().into()), names: vec![], renewed: 0 });
        assert!(m.reload(false).unwrap_err().contains("expired"));
        assert!(!m.state.active());
    }

    #[test]
    fn tailscale_certificates_are_fetched_cached_and_renewed() {
        let (c, k) = testcerts::make(&["box.tail1.ts.net"], 80);
        let ts = Arc::new(Ts { name: Some("box.tail1.ts.net".into()), cert: Mutex::new(Ok((c, k))), calls: Mutex::new(0) });
        let (_d, m) = mgr(ts.clone());
        m.set_hints(Hints { names: vec![], tailnet: true });
        // Auto + tailnet: fetched.
        let i = m.reload(true).unwrap().unwrap();
        assert_eq!(i.source, CertSource::Tailscale);
        assert_eq!(*ts.calls.lock().unwrap(), 1);
        // Again: the cache is fresh, Tailscale is not asked.
        m.reload(true).unwrap();
        assert_eq!(*ts.calls.lock().unwrap(), 1);
        // A CLI `status` never fetches but still reads the cache.
        m.state.clear();
        assert!(m.reload(false).unwrap().is_some());
        // Close to the end: renewed; and a failing renewal keeps the old one.
        let near = {
            let s = Store::open(&m.home).unwrap();
            let mut cached = tsnet::load(&s).unwrap().unwrap();
            cached.not_after = now_unix() + 10 * 86_400;
            tsnet::save(&s, &cached).unwrap();
            cached
        };
        *ts.cert.lock().unwrap() = Err("Tailscale could not issue a certificate: HTTPS is not enabled".into());
        let i = m.reload(true).unwrap().unwrap();
        assert_eq!(*ts.calls.lock().unwrap(), 2);
        assert_eq!(i.source, CertSource::Tailscale);
        assert!(m.state.last_error().unwrap().contains("HTTPS is not enabled"));
        let _ = near;
    }

    #[test]
    fn tailscale_https_that_is_not_enabled_is_explained_and_plain_http_stays() {
        let ts = Arc::new(Ts {
            name: Some("box.tail1.ts.net".into()),
            cert: Mutex::new(Err("Tailscale could not issue a certificate: HTTPS is not enabled for this tailnet".into())),
            calls: Mutex::new(0),
        });
        let (_d, m) = mgr(ts);
        m.set_hints(Hints { names: vec![], tailnet: true });
        assert!(m.reload(true).unwrap().is_none(), "auto: a bonus, not an error");
        assert!(m.state.last_error().unwrap().contains("HTTPS is not enabled"));
        // Asked for explicitly it is an error.
        set(&m, TlsConfig { mode: TlsMode::Tailscale, ..Default::default() });
        assert!(m.reload(true).unwrap_err().contains("HTTPS is not enabled"));
        // No MagicDNS name at all.
        let ts = Arc::new(Ts { name: None, cert: Mutex::new(Err("x".into())), calls: Mutex::new(0) });
        let (_d2, m2) = mgr(ts);
        set(&m2, TlsConfig { mode: TlsMode::Tailscale, ..Default::default() });
        assert!(m2.reload(true).unwrap_err().contains("MagicDNS"));
    }

    #[test]
    fn self_signed_has_a_stable_ca_and_off_turns_it_off() {
        let (_d, m) = mgr(Arc::new(NoWhois));
        m.set_hints(Hints { names: vec!["box.lan:47120".into()], tailnet: false });
        set(&m, TlsConfig { mode: TlsMode::SelfSigned, ..Default::default() });
        let a = m.reload(false).unwrap().unwrap();
        assert_eq!(a.source, CertSource::SelfSigned);
        assert!(a.names.contains(&"box.lan".to_string()) && a.names.contains(&"localhost".to_string()));
        assert!(a.ca_fingerprint.is_some() && !m.state.hsts());
        let b = m.reload(false).unwrap().unwrap();
        assert_eq!((a.fingerprint, a.ca_fingerprint), (b.fingerprint, b.ca_fingerprint), "unchanged names: same certificate, same CA");
        set(&m, TlsConfig { mode: TlsMode::Off, ..Default::default() });
        assert!(m.reload(false).unwrap().is_none() && !m.state.active());
        assert!(m.report().ca_fingerprint.is_some(), "the CA is kept for next time");
    }
}
