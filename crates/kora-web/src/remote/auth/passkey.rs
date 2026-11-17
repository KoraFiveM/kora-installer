//! Passkeys (WebAuthn) for remote users, on `webauthn-rs`.
//!
//! # Where the relying-party ID comes from
//! A passkey is bound to a **relying-party ID**: a registrable domain name, and the browser only uses it on that
//! name or a sub-domain. The panel takes it from the name the person typed (the `Host` header, which the
//! panel's host check has already restricted to names the settings list), per access path:
//!
//! | how the panel is opened | relying-party ID | works |
//! |---|---|---|
//! | `https://box.tail1234.ts.net:47120` (Tailscale HTTPS) | `box.tail1234.ts.net` | yes |
//! | `https://panel.example.com` (own domain, reverse proxy) | `panel.example.com` | yes |
//! | `https://box.lan:47120` (a LAN name with a certificate) | `box.lan` | yes |
//! | `https://192.168.1.5:47120` (a bare address) | none: an IP address cannot be an ID | NO: use one-time codes, or give the panel a name |
//! | `http://anything` | none: browsers allow WebAuthn only on a secure origin | NO (except `localhost`) |
//!
//! A passkey registered for one name does not work on another, and the screen says so ([`rp_message`]): pick ONE
//! name for remote access and register there. The ID is saved with each passkey.
//!
//! # Ceremonies
//! Registration and sign-in are two steps each (begin: the panel makes a random challenge and remembers its
//! state in memory under a random handle; finish: the browser's signed answer is checked against that state).
//! The state is removed the first time it is used, valid or not, and expires after [`PENDING_TTL`]: an answer
//! can be tried once, so a captured assertion cannot be replayed. The signature counter is checked and saved.
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use url::Url;
use webauthn_rs::prelude::{CreationChallengeResponse, Passkey, PasskeyAuthentication, PasskeyRegistration, PublicKeyCredential, RegisterPublicKeyCredential, RequestChallengeResponse, Webauthn};
use webauthn_rs::WebauthnBuilder;

use super::db;
use crate::security::random_hex;
use kora_core::store::Store;

pub const PENDING_TTL: Duration = Duration::from_secs(5 * 60);
pub const PENDING_MAX: usize = 256;

/// A relying party: the ID (a domain name) and the origin the browser reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rp {
    pub id: String,
    pub origin: Url,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RpError {
    /// The person opened the panel by an IP address.
    IpAddress(String),
    /// Plain HTTP on a name other than `localhost`.
    NotSecure,
    BadHost,
}

/// What to tell the person.
pub fn rp_message(e: &RpError) -> String {
    match e {
        RpError::IpAddress(ip) => format!("Passkeys need the panel to be opened by a name, not an address like {ip}. Open it by its name (a Tailscale name, or a name from your own domain), or sign in with a one-time code instead."),
        RpError::NotSecure => "Passkeys only work on a secure (https://) address. Ask the person who runs Kora to turn on HTTPS (`kora panel tls`), or sign in with a one-time code instead.".into(),
        RpError::BadHost => "This address cannot be used for passkeys.".into(),
    }
}

/// The relying party for a request that arrived with `host` (the `Host` header, with its port) over a connection
/// that was `secure` (TLS) or not.
pub fn rp_for(host: &str, secure: bool) -> Result<Rp, RpError> {
    let name = crate::tls::selfsigned::host_only(host.trim());
    if name.is_empty() || name.len() > 253 {
        return Err(RpError::BadHost);
    }
    if name.parse::<std::net::IpAddr>().is_ok() {
        return Err(RpError::IpAddress(name.to_string()));
    }
    let name = name.to_ascii_lowercase();
    if !secure && name != "localhost" {
        return Err(RpError::NotSecure);
    }
    // The origin as the browser reports it: scheme, name and the port unless it is the default one.
    let port = host.trim().rsplit_once(':').filter(|(h, _)| !h.contains(']') || host.trim().starts_with('[')).and_then(|(_, p)| p.parse::<u16>().ok());
    let scheme = if secure { "https" } else { "http" };
    let default = if secure { 443 } else { 80 };
    let origin = match port {
        Some(p) if p != default => format!("{scheme}://{name}:{p}"),
        _ => format!("{scheme}://{name}"),
    };
    let origin = Url::parse(&origin).map_err(|_| RpError::BadHost)?;
    Ok(Rp { id: name, origin })
}

fn webauthn(rp: &Rp) -> Result<Webauthn, String> {
    WebauthnBuilder::new(&rp.id, &rp.origin).map_err(|e| format!("this address cannot be used for passkeys ({e})"))?.rp_name("Kora Panel").build().map_err(|e| format!("passkeys are not available here ({e})"))
}

enum Pending {
    Register { user: String, rp: Rp, state: Box<PasskeyRegistration> },
    Login { user: String, rp: Rp, state: Box<PasskeyAuthentication> },
}

/// The ceremonies in flight (in memory: a restart just makes people start the step again).
pub struct Ceremonies {
    pending: Mutex<HashMap<String, (Instant, Pending)>>,
    clock: crate::security::Clock,
}

impl Default for Ceremonies {
    fn default() -> Self {
        Self { pending: Mutex::default(), clock: std::sync::Arc::new(Instant::now) }
    }
}

/// A finished sign-in.
#[derive(Debug)]
pub struct LoginOk {
    pub user: String,
    pub passkey_id: i64,
}

impl Ceremonies {
    pub fn with_clock(clock: crate::security::Clock) -> Self {
        Self { pending: Mutex::default(), clock }
    }
    fn put(&self, p: Pending) -> String {
        let id = random_hex();
        let now = (self.clock)();
        let mut g = self.pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        g.retain(|_, (t, _)| now.saturating_duration_since(*t) < PENDING_TTL);
        while g.len() >= PENDING_MAX {
            let oldest = g.iter().min_by_key(|(_, (t, _))| *t).map(|(k, _)| k.clone());
            match oldest {
                Some(k) => g.remove(&k),
                None => break,
            };
        }
        g.insert(id.clone(), (now, p));
        id
    }
    /// Take the state out (it works once); expired states are gone.
    fn take(&self, id: &str) -> Option<Pending> {
        let now = (self.clock)();
        let mut g = self.pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (t, p) = g.remove(id)?;
        (now.saturating_duration_since(t) < PENDING_TTL).then_some(p)
    }
    pub fn in_flight(&self) -> usize {
        self.pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner).len()
    }

    /// Step one of adding a passkey for `user`. Credentials the user already has are excluded, so the same device
    /// is not registered twice.
    pub fn begin_registration(&self, s: &Store, rp: &Rp, user: &str, display: &str) -> Result<(String, CreationChallengeResponse), String> {
        let w = webauthn(rp)?;
        let id = db::webauthn_id(s, user)?;
        let exclude: Vec<_> = db::passkeys(s, user)?.iter().map(|p| p.passkey.cred_id().clone()).collect();
        let (challenge, state) = w.start_passkey_registration(id, user, display, if exclude.is_empty() { None } else { Some(exclude) }).map_err(|e| format!("could not start the passkey set-up ({e})"))?;
        let handle = self.put(Pending::Register { user: db::fold(user), rp: rp.clone(), state: Box::new(state) });
        Ok((handle, challenge))
    }

    /// Step two: check the browser's answer and give back the new credential (the caller saves it). `user` must be
    /// the one the first step was for.
    pub fn finish_registration(&self, handle: &str, user: &str, answer: &RegisterPublicKeyCredential) -> Result<(Passkey, Rp), String> {
        let Some(Pending::Register { user: u, rp, state }) = self.take(handle) else {
            return Err("That passkey set-up has expired or was already used. Start again.".into());
        };
        if u != db::fold(user) {
            return Err("That passkey set-up belongs to someone else.".into());
        }
        let w = webauthn(&rp)?;
        let pk = w.finish_passkey_registration(answer, &state).map_err(|e| format!("The passkey could not be registered ({e}). Try again, or use another device."))?;
        Ok((pk, rp))
    }

    /// Step one of a sign-in: the user's passkeys for THIS relying party. The error is the same for a name nobody has
    /// and for a user without a passkey here.
    pub fn begin_login(&self, s: &Store, rp: &Rp, user: &str) -> Result<(String, RequestChallengeResponse), String> {
        let w = webauthn(rp)?;
        let keys: Vec<Passkey> = db::passkeys(s, user)?.into_iter().filter(|p| p.info.rp_id == rp.id).map(|p| p.passkey).collect();
        if keys.is_empty() {
            return Err(NO_PASSKEY.into());
        }
        let (challenge, state) = w.start_passkey_authentication(&keys).map_err(|e| format!("could not start the sign-in ({e})"))?;
        let handle = self.put(Pending::Login { user: db::fold(user), rp: rp.clone(), state: Box::new(state) });
        Ok((handle, challenge))
    }

    /// Step two: verify the signed assertion (challenge, origin, relying-party ID, user presence, signature, counter),
    /// save the moved counter and say who signed in.
    pub fn finish_login(&self, s: &Store, handle: &str, answer: &PublicKeyCredential) -> Result<LoginOk, String> {
        let Some(Pending::Login { user, rp, state }) = self.take(handle) else {
            return Err("That sign-in has expired or was already used. Start again.".into());
        };
        let w = webauthn(&rp)?;
        let result = w.finish_passkey_authentication(answer, &state).map_err(|_| "The passkey was not accepted.".to_string())?;
        let mut stored = db::passkeys(s, &user)?;
        let Some(mine) = stored.iter_mut().find(|p| p.passkey.cred_id() == result.cred_id()) else {
            return Err("The passkey was not accepted.".into());
        };
        let updated = if result.needs_update() {
            mine.passkey.update_credential(&result);
            Some(&mine.passkey)
        } else {
            None
        };
        db::touch_passkey(s, mine.info.id, updated)?;
        Ok(LoginOk { user, passkey_id: mine.info.id })
    }
}

pub const NO_PASSKEY: &str = "There is no passkey for that name on this address. Use the name you registered it with, sign in with a one-time code, or ask an administrator for a new enrolment link.";

#[cfg(test)]
mod tests {
    use super::*;
    use webauthn_authenticator_rs::softpasskey::SoftPasskey;
    use webauthn_authenticator_rs::WebauthnAuthenticator;

    fn store() -> (tempfile::TempDir, Store) {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(d.path()).unwrap();
        (d, s)
    }

    fn rp() -> Rp {
        rp_for("box.tail1234.ts.net:47120", true).unwrap()
    }

    #[test]
    fn the_relying_party_follows_the_access_path() {
        let r = rp_for("box.tail1234.ts.net:47120", true).unwrap();
        assert_eq!((r.id.as_str(), r.origin.as_str()), ("box.tail1234.ts.net", "https://box.tail1234.ts.net:47120/"));
        let r = rp_for("Panel.Example.com", true).unwrap();
        assert_eq!((r.id.as_str(), r.origin.as_str()), ("panel.example.com", "https://panel.example.com/"));
        assert_eq!(rp_for("panel.example.com:443", true).unwrap().origin.as_str(), "https://panel.example.com/", "the default port is left out");
        assert_eq!(rp_for("localhost:47120", false).unwrap().origin.as_str(), "http://localhost:47120/", "localhost counts as secure");
        // A bare address cannot be an ID.
        for ip in ["192.168.1.5:47120", "100.64.0.5", "[fd7a::1]:47120", "::1"] {
            assert!(matches!(rp_for(ip, true), Err(RpError::IpAddress(_))), "{ip}");
        }
        assert!(rp_message(&RpError::IpAddress("192.168.1.5".into())).contains("one-time code"));
        // Plain HTTP on a name is not a secure context.
        assert_eq!(rp_for("box.lan:47120", false), Err(RpError::NotSecure));
        assert_eq!(rp_for("", true), Err(RpError::BadHost));
    }

    #[tokio::test]
    async fn register_then_sign_in_with_a_software_passkey() {
        let (_d, s) = store();
        let c = Ceremonies::default();
        let rp = rp();
        let mut auth = SoftPasskey::new(true);
        let (h, challenge) = c.begin_registration(&s, &rp, "sam", "Sam").unwrap();
        let answer = auth.do_registration(rp.origin.clone(), challenge).unwrap();
        let (pk, saved_rp) = c.finish_registration(&h, "sam", &answer).unwrap();
        db::add_passkey(&s, "sam", "Laptop", &saved_rp.id, &pk).unwrap();
        assert_eq!(db::state(&s, "Sam").unwrap().passkeys.len(), 1);

        let (h, challenge) = c.begin_login(&s, &rp, "SAM").unwrap();
        let assertion = auth.do_authentication(rp.origin.clone(), challenge).unwrap();
        let ok = c.finish_login(&s, &h, &assertion).unwrap();
        assert_eq!(ok.user, "sam");
        assert!(db::state(&s, "sam").unwrap().passkeys[0].last_used > 0);

        // The same assertion cannot be used again: the ceremony's state is gone.
        assert!(c.finish_login(&s, &h, &assertion).unwrap_err().contains("expired or was already used"));
    }

    #[tokio::test]
    async fn a_replayed_assertion_for_a_new_challenge_fails() {
        let (_d, s) = store();
        let c = Ceremonies::default();
        let rp = rp();
        let mut auth = SoftPasskey::new(true);
        let (h, ch) = c.begin_registration(&s, &rp, "sam", "Sam").unwrap();
        let answer = auth.do_registration(rp.origin.clone(), ch).unwrap();
        let (pk, r) = c.finish_registration(&h, "sam", &answer).unwrap();
        db::add_passkey(&s, "sam", "L", &r.id, &pk).unwrap();
        let (h1, ch1) = c.begin_login(&s, &rp, "sam").unwrap();
        let first = auth.do_authentication(rp.origin.clone(), ch1).unwrap();
        c.finish_login(&s, &h1, &first).unwrap();
        // An attacker who recorded `first` starts a new sign-in and sends it: the challenge differs.
        let (h2, _) = c.begin_login(&s, &rp, "sam").unwrap();
        assert!(c.finish_login(&s, &h2, &first).unwrap_err().contains("not accepted"));
        assert_eq!(c.in_flight(), 0, "a failed attempt uses the ceremony up too");
    }

    #[tokio::test]
    async fn the_wrong_origin_is_refused() {
        let (_d, s) = store();
        let c = Ceremonies::default();
        let rp = rp();
        let mut auth = SoftPasskey::new(true);
        let (h, ch) = c.begin_registration(&s, &rp, "sam", "Sam").unwrap();
        // The page the authenticator thinks it is on is another site (a look-alike): the signed origin differs.
        let evil = Url::parse("https://evil.example").unwrap();
        let bad = auth.do_registration(evil, ch);
        match bad {
            Err(_) => {}
            Ok(answer) => assert!(c.finish_registration(&h, "sam", &answer).is_err(), "a registration signed for another origin is refused"),
        }
        // A good registration, then a login signed for another origin.
        let (h, ch) = c.begin_registration(&s, &rp, "sam", "Sam").unwrap();
        let answer = auth.do_registration(rp.origin.clone(), ch).unwrap();
        let (pk, r) = c.finish_registration(&h, "sam", &answer).unwrap();
        db::add_passkey(&s, "sam", "L", &r.id, &pk).unwrap();
        let (h, ch) = c.begin_login(&s, &rp, "sam").unwrap();
        let evil = Url::parse("https://evil.example").unwrap();
        if let Ok(a) = auth.do_authentication(evil, ch) {
            assert!(c.finish_login(&s, &h, &a).is_err());
        }
    }

    #[tokio::test]
    async fn a_passkey_for_one_name_does_not_work_on_another() {
        let (_d, s) = store();
        let c = Ceremonies::default();
        let rp1 = rp();
        let mut auth = SoftPasskey::new(true);
        let (h, ch) = c.begin_registration(&s, &rp1, "sam", "Sam").unwrap();
        let a = auth.do_registration(rp1.origin.clone(), ch).unwrap();
        let (pk, r) = c.finish_registration(&h, "sam", &a).unwrap();
        db::add_passkey(&s, "sam", "L", &r.id, &pk).unwrap();
        let other = rp_for("panel.example.com", true).unwrap();
        assert_eq!(c.begin_login(&s, &other, "sam").err().unwrap(), NO_PASSKEY);
        assert_eq!(c.begin_login(&s, &rp1, "nobody").err().unwrap(), NO_PASSKEY, "the same words for a name nobody has");
    }

    #[tokio::test]
    async fn a_registration_cannot_be_finished_by_someone_else_or_twice() {
        let (_d, s) = store();
        let c = Ceremonies::default();
        let rp = rp();
        let mut auth = SoftPasskey::new(true);
        let (h, ch) = c.begin_registration(&s, &rp, "sam", "Sam").unwrap();
        let a = auth.do_registration(rp.origin.clone(), ch).unwrap();
        assert!(c.finish_registration(&h, "eve", &a).unwrap_err().contains("someone else"));
        assert!(c.finish_registration(&h, "sam", &a).unwrap_err().contains("expired or was already used"), "using it up even when refused");
    }

    #[test]
    fn ceremonies_expire_and_are_bounded() {
        let base = Instant::now();
        let off = std::sync::Arc::new(Mutex::new(Duration::ZERO));
        let o = off.clone();
        let c = Ceremonies::with_clock(std::sync::Arc::new(move || base + *o.lock().unwrap()));
        let (_d, s) = store();
        let rp = rp();
        let (h, _) = c.begin_registration(&s, &rp, "sam", "Sam").unwrap();
        *off.lock().unwrap() += PENDING_TTL;
        assert!(c.take(&h).is_none(), "expired");
        for _ in 0..(PENDING_MAX + 20) {
            c.begin_registration(&s, &rp, "sam", "Sam").unwrap();
        }
        assert!(c.in_flight() <= PENDING_MAX);
    }
}
