//! The certificate `tailscale cert` issues for this computer's tailnet name.
//!
//! Needs, on the tailnet side: MagicDNS on, and "HTTPS Certificates" enabled in the Tailscale admin console
//! (DNS page). Kora does not change either; when `tailscale cert` refuses, its own reason is shown. The
//! certificate is a real one (Let's Encrypt through Tailscale), valid for the tailnet name only: opening the
//! panel by the Tailscale IP address gives a name mismatch, so the panel tells people to use the name.
//!
//! The issued certificate is kept in the store (`tls_tailscale`) and used until [`RENEW_DAYS`] before it ends;
//! then it is fetched again. A failed renewal keeps the old certificate until it really expires.
use kora_core::store::{rusqlite, Store};

pub const RENEW_DAYS: i64 = 30;

pub struct Cached {
    pub name: String,
    pub cert_pem: String,
    pub key_pem: String,
    pub not_after: i64,
}

pub fn load(s: &Store) -> Result<Option<Cached>, String> {
    s.read(|c| {
        match c.query_row("SELECT name, cert_pem, key_pem, not_after FROM tls_tailscale WHERE id = 1", [], |r| {
            Ok(Cached { name: r.get(0)?, cert_pem: r.get(1)?, key_pem: r.get(2)?, not_after: r.get(3)? })
        }) {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    })
    .map_err(crate::error::store_text)
}

pub fn save(s: &Store, c: &Cached) -> Result<(), String> {
    s.write(|tx| {
        tx.execute(
            "INSERT INTO tls_tailscale (id, name, cert_pem, key_pem, not_after) VALUES (1, ?1, ?2, ?3, ?4) \
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, cert_pem = excluded.cert_pem, key_pem = excluded.key_pem, not_after = excluded.not_after",
            rusqlite::params![c.name, c.cert_pem, c.key_pem, c.not_after],
        )?;
        Ok(())
    })
    .map_err(crate::error::store_text)
}

/// Is the cached certificate good for `name` for long enough that nothing needs fetching?
pub fn fresh(c: &Cached, name: &str, now: i64) -> bool {
    c.name.eq_ignore_ascii_case(name) && super::certs::days_left(c.not_after, now) > RENEW_DAYS
}

/// Why Tailscale HTTPS cannot be used, in words, when there is no tailnet name.
pub const NO_NAME: &str = "Tailscale HTTPS needs this computer's Tailscale name (MagicDNS). Turn on MagicDNS and \"HTTPS Certificates\" in the Tailscale admin console (DNS page), then try again.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_is_used_until_close_to_the_end_and_only_for_its_name() {
        let c = Cached { name: "box.tail1.ts.net".into(), cert_pem: String::new(), key_pem: String::new(), not_after: 100 * 86_400 };
        assert!(fresh(&c, "BOX.tail1.ts.net", 0));
        assert!(!fresh(&c, "other.tail1.ts.net", 0));
        assert!(fresh(&c, "box.tail1.ts.net", 69 * 86_400), "31 days left");
        assert!(!fresh(&c, "box.tail1.ts.net", 71 * 86_400), "29 days left: renew");
        assert!(!fresh(&c, "box.tail1.ts.net", 200 * 86_400), "expired");
    }

    #[test]
    fn saved_and_read_back() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(d.path()).unwrap();
        assert!(load(&s).unwrap().is_none());
        save(&s, &Cached { name: "a.ts.net".into(), cert_pem: "c".into(), key_pem: "k".into(), not_after: 5 }).unwrap();
        save(&s, &Cached { name: "b.ts.net".into(), cert_pem: "c2".into(), key_pem: "k2".into(), not_after: 6 }).unwrap();
        let c = load(&s).unwrap().unwrap();
        assert_eq!((c.name.as_str(), c.not_after), ("b.ts.net", 6));
    }
}
