//! The tables behind remote sign-in (migration 5): who has which credentials, enrolment links, API tokens.
//! Every function takes the store; every change that must be atomic (single-use links, one-time-code steps,
//! recovery codes, lockout counters) is ONE transaction with the check inside it, so two requests at the same
//! moment cannot both win.
//!
//! PRIVATE data: password hashes, TOTP secrets, passkey public keys and recovery-code hashes live here and
//! never leave in an API answer, log line, audit row or support report. [`UserState`] is what may be shown:
//! counts and dates, never a value.
use kora_core::store::{rusqlite, Store};
use rusqlite::OptionalExtension;
use uuid::Uuid;

use crate::security::ct_eq;

fn err(p: kora_core::problem::Problem) -> String {
    crate::error::store_text(p)
}

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// The key every table uses for a user name.
pub fn fold(name: &str) -> String {
    name.trim().to_lowercase()
}

/// The columns `state` reads: password hash, TOTP secret, confirmed, recovery hashes, locked until.
type AuthRow = (Option<String>, Option<Vec<u8>>, i64, String, i64);

/// What may be shown about one user's credentials.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserState {
    pub passkeys: Vec<PasskeyInfo>,
    pub totp: bool,
    pub password: bool,
    pub recovery_left: usize,
    pub locked_until: i64,
}

impl UserState {
    /// Can this user sign in with something stronger than the old token link?
    pub fn strong(&self) -> bool {
        !self.passkeys.is_empty() || (self.totp && self.password)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasskeyInfo {
    pub id: i64,
    pub label: String,
    pub rp_id: String,
    pub created: i64,
    pub last_used: i64,
}

/// The stored WebAuthn credential (public key and counter), as the webauthn crate saved it.
pub struct StoredPasskey {
    pub info: PasskeyInfo,
    pub passkey: webauthn_rs::prelude::Passkey,
}

/// The WebAuthn user handle of `user` (random, made once; never the name), making the row if needed.
pub fn webauthn_id(s: &Store, user: &str) -> Result<Uuid, String> {
    let key = fold(user);
    s.write(|tx| {
        let have: Option<Vec<u8>> = tx.query_row("SELECT webauthn_id FROM remote_user_auth WHERE user = ?1", [&key], |r| r.get(0)).optional()?;
        if let Some(b) = have {
            if let Ok(u) = Uuid::from_slice(&b) {
                return Ok(u);
            }
        }
        let id = Uuid::new_v4();
        tx.execute(
            "INSERT INTO remote_user_auth (user, webauthn_id) VALUES (?1, ?2) ON CONFLICT(user) DO UPDATE SET webauthn_id = excluded.webauthn_id",
            rusqlite::params![key, id.as_bytes().as_slice()],
        )?;
        Ok(id)
    })
    .map_err(err)
}

pub fn state(s: &Store, user: &str) -> Result<UserState, String> {
    let key = fold(user);
    s.read(|c| {
        let mut st = UserState::default();
        let row: Option<AuthRow> = c
            .query_row("SELECT password_hash, totp_secret, totp_confirmed, recovery_hashes, locked_until FROM remote_user_auth WHERE user = ?1", [&key], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .optional()?;
        if let Some((pw, secret, confirmed, rec, locked)) = row {
            st.password = pw.is_some_and(|p| !p.is_empty());
            st.totp = secret.is_some() && confirmed == 1;
            st.recovery_left = serde_json::from_str::<Vec<String>>(&rec).map(|v| v.len()).unwrap_or(0);
            st.locked_until = locked;
        }
        let mut q = c.prepare("SELECT id, label, rp_id, created, last_used FROM remote_passkeys WHERE user = ?1 ORDER BY id")?;
        for row in q.query_map([&key], |r| Ok(PasskeyInfo { id: r.get(0)?, label: r.get(1)?, rp_id: r.get(2)?, created: r.get(3)?, last_used: r.get(4)? }))? {
            st.passkeys.push(row?);
        }
        Ok(st)
    })
    .map_err(err)
}

// ---- passkeys -----------------------------------------------------------------------------------------------

pub fn passkeys(s: &Store, user: &str) -> Result<Vec<StoredPasskey>, String> {
    let key = fold(user);
    s.read(|c| {
        let mut q = c.prepare("SELECT id, label, rp_id, created, last_used, passkey_json FROM remote_passkeys WHERE user = ?1 ORDER BY id")?;
        let rows = q.query_map([&key], |r| Ok((PasskeyInfo { id: r.get(0)?, label: r.get(1)?, rp_id: r.get(2)?, created: r.get(3)?, last_used: r.get(4)? }, r.get::<_, String>(5)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (info, json) = row?;
            // A credential that cannot be read is skipped (never a reason to refuse every other one).
            if let Ok(passkey) = serde_json::from_str(&json) {
                out.push(StoredPasskey { info, passkey });
            }
        }
        Ok(out)
    })
    .map_err(err)
}

pub fn add_passkey(s: &Store, user: &str, label: &str, rp_id: &str, passkey: &webauthn_rs::prelude::Passkey) -> Result<i64, String> {
    let key = fold(user);
    let json = serde_json::to_string(passkey).map_err(|e| e.to_string())?;
    let cred: Vec<u8> = passkey.cred_id().as_slice().to_vec();
    s.write(|tx| {
        tx.execute("INSERT INTO remote_passkeys (user, label, credential_id, passkey_json, rp_id, created) VALUES (?1, ?2, ?3, ?4, ?5, ?6)", rusqlite::params![key, label, cred, json, rp_id, now()])?;
        Ok(tx.last_insert_rowid())
    })
    .map_err(|p| {
        let m = err(p);
        if m.contains("UNIQUE") {
            "That passkey is already registered.".to_string()
        } else {
            m
        }
    })
}

pub fn remove_passkey(s: &Store, user: &str, id: i64) -> Result<bool, String> {
    let key = fold(user);
    s.write(|tx| Ok(tx.execute("DELETE FROM remote_passkeys WHERE id = ?1 AND user = ?2", rusqlite::params![id, key])? > 0)).map_err(err)
}

/// Save the credential after a sign-in (its signature counter moved) and note when it was used.
pub fn touch_passkey(s: &Store, id: i64, updated: Option<&webauthn_rs::prelude::Passkey>) -> Result<(), String> {
    s.write(|tx| {
        match updated {
            Some(p) => tx.execute(
                "UPDATE remote_passkeys SET passkey_json = ?1, last_used = ?2 WHERE id = ?3",
                rusqlite::params![serde_json::to_string(p).map_err(|e| kora_core::problem::Problem::generic(e.to_string()))?, now(), id],
            )?,
            None => tx.execute("UPDATE remote_passkeys SET last_used = ?1 WHERE id = ?2", rusqlite::params![now(), id])?,
        };
        Ok(())
    })
    .map_err(err)
}

// ---- password, one-time codes, recovery codes -------------------------------------------------------------

/// Set the password and a confirmed TOTP secret together with fresh recovery codes (the end of enrolment).
pub fn set_password_and_totp(s: &Store, user: &str, password_hash: &str, secret: &[u8], recovery: &[String]) -> Result<(), String> {
    let key = fold(user);
    let id = Uuid::new_v4();
    s.write(|tx| {
        tx.execute(
            "INSERT INTO remote_user_auth (user, webauthn_id, password_hash, totp_secret, totp_confirmed, totp_last_step, recovery_hashes, failures, locked_until) VALUES (?1, ?2, ?3, ?4, 1, 0, ?5, 0, 0) \
             ON CONFLICT(user) DO UPDATE SET password_hash = excluded.password_hash, totp_secret = excluded.totp_secret, totp_confirmed = 1, totp_last_step = 0, recovery_hashes = excluded.recovery_hashes, failures = 0, locked_until = 0",
            rusqlite::params![key, id.as_bytes().as_slice(), password_hash, secret, serde_json::to_string(recovery).map_err(|e| kora_core::problem::Problem::generic(e.to_string()))?],
        )?;
        Ok(())
    })
    .map_err(err)
}

pub fn set_password(s: &Store, user: &str, password_hash: &str) -> Result<(), String> {
    let key = fold(user);
    s.write(|tx| Ok(tx.execute("UPDATE remote_user_auth SET password_hash = ?1, failures = 0, locked_until = 0 WHERE user = ?2", rusqlite::params![password_hash, key])? > 0))
        .map_err(err)
        .and_then(|ok| if ok { Ok(()) } else { Err("That user has no one-time-code sign-in to change the password of.".into()) })
}

pub fn replace_recovery(s: &Store, user: &str, recovery: &[String]) -> Result<(), String> {
    let key = fold(user);
    s.write(|tx| {
        tx.execute(
            "UPDATE remote_user_auth SET recovery_hashes = ?1 WHERE user = ?2",
            rusqlite::params![serde_json::to_string(recovery).map_err(|e| kora_core::problem::Problem::generic(e.to_string()))?, key],
        )?;
        Ok(())
    })
    .map_err(err)
}

/// Forget the password, the secret and the recovery codes (an administrator's reset, or removing the method).
pub fn clear_totp(s: &Store, user: &str) -> Result<(), String> {
    let key = fold(user);
    s.write(|tx| {
        tx.execute(
            "UPDATE remote_user_auth SET password_hash = NULL, totp_secret = NULL, totp_confirmed = 0, totp_last_step = 0, recovery_hashes = '[]', failures = 0, locked_until = 0 WHERE user = ?1",
            [&key],
        )?;
        Ok(())
    })
    .map_err(err)
}

pub fn clear_passkeys(s: &Store, user: &str) -> Result<usize, String> {
    let key = fold(user);
    s.write(|tx| Ok(tx.execute("DELETE FROM remote_passkeys WHERE user = ?1", [&key])?)).map_err(err)
}

/// The password hash and the TOTP secret with its last accepted step, for a sign-in attempt.
pub struct CodeSecrets {
    pub password_hash: String,
    pub totp_secret: Vec<u8>,
    pub last_step: u64,
    pub locked_until: i64,
}

pub fn code_secrets(s: &Store, user: &str) -> Result<Option<CodeSecrets>, String> {
    let key = fold(user);
    s.read(|c| {
        Ok(c.query_row(
            "SELECT password_hash, totp_secret, totp_last_step, locked_until FROM remote_user_auth WHERE user = ?1 AND totp_confirmed = 1 AND password_hash IS NOT NULL AND totp_secret IS NOT NULL",
            [&key],
            |r| Ok(CodeSecrets { password_hash: r.get(0)?, totp_secret: r.get(1)?, last_step: r.get::<_, i64>(2)? as u64, locked_until: r.get(3)? }),
        )
        .optional()?)
    })
    .map_err(err)
}

/// Record an accepted TOTP step, only when it is newer than the last one (replay protection, atomic).
pub fn take_totp_step(s: &Store, user: &str, step: u64) -> Result<bool, String> {
    let key = fold(user);
    s.write(|tx| Ok(tx.execute("UPDATE remote_user_auth SET totp_last_step = ?1 WHERE user = ?2 AND totp_last_step < ?1", rusqlite::params![step as i64, key])? > 0)).map_err(err)
}

/// Use up the recovery code `given`, if it is one of the user's. Atomic: two uses of one code cannot both pass.
pub fn use_recovery_code(s: &Store, user: &str, given: &str) -> Result<bool, String> {
    let key = fold(user);
    s.write(|tx| {
        let json: Option<String> = tx.query_row("SELECT recovery_hashes FROM remote_user_auth WHERE user = ?1", [&key], |r| r.get(0)).optional()?;
        let Some(json) = json else { return Ok(false) };
        let mut list: Vec<String> = serde_json::from_str(&json).unwrap_or_default();
        match super::recovery::find(&list, given) {
            Some(i) => {
                list.remove(i);
                tx.execute(
                    "UPDATE remote_user_auth SET recovery_hashes = ?1 WHERE user = ?2",
                    rusqlite::params![serde_json::to_string(&list).map_err(|e| kora_core::problem::Problem::generic(e.to_string()))?, key],
                )?;
                Ok(true)
            }
            None => Ok(false),
        }
    })
    .map_err(err)
}

/// Failed password or code attempts in a row before the ACCOUNT is locked for [`LOCK_FOR`] seconds.
pub const LOCK_AFTER: i64 = 5;
pub const LOCK_FOR: i64 = 15 * 60;

/// Count one failure; at [`LOCK_AFTER`] the account is locked. Returns the unix time it is locked until (0: not).
pub fn note_failure(s: &Store, user: &str) -> Result<i64, String> {
    let key = fold(user);
    s.write(|tx| {
        tx.execute("UPDATE remote_user_auth SET failures = failures + 1 WHERE user = ?1", [&key])?;
        let f: Option<i64> = tx.query_row("SELECT failures FROM remote_user_auth WHERE user = ?1", [&key], |r| r.get(0)).optional()?;
        if f.is_some_and(|f| f >= LOCK_AFTER) {
            let until = now() + LOCK_FOR;
            tx.execute("UPDATE remote_user_auth SET locked_until = ?1, failures = 0 WHERE user = ?2", rusqlite::params![until, key])?;
            return Ok(until);
        }
        Ok(0)
    })
    .map_err(err)
}

pub fn clear_failures(s: &Store, user: &str) -> Result<(), String> {
    let key = fold(user);
    s.write(|tx| {
        tx.execute("UPDATE remote_user_auth SET failures = 0, locked_until = 0 WHERE user = ?1", [&key])?;
        Ok(())
    })
    .map_err(err)
}

// ---- enrolment links --------------------------------------------------------------------------------------

/// Why a link was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// A new user's first credential.
    Invite,
    /// An administrator reset: the user adds credentials again.
    Reset,
}

impl Purpose {
    fn text(self) -> &'static str {
        match self {
            Purpose::Invite => "invite",
            Purpose::Reset => "reset",
        }
    }
}

/// How long an enrolment link works.
pub const LINK_TTL: i64 = 30 * 60;

fn link_hash(secret: &str) -> String {
    crate::remote::config::hash_token("kora-enrol-link", secret)
}

/// A new link secret for `user` (shown once; only its hash is stored). Older unused links of the user stay valid
/// until they expire, but at most [`LINKS_PER_USER`] at a time.
pub fn mint_link(s: &Store, user: &str, purpose: Purpose, created_by: &str) -> Result<String, String> {
    let secret = crate::security::random_hex();
    let key = fold(user);
    let t = now();
    s.write(|tx| {
        tx.execute("DELETE FROM remote_enrolments WHERE expires < ?1", [t])?;
        let mut have: Vec<String> = {
            let mut q = tx.prepare("SELECT hash FROM remote_enrolments WHERE user = ?1 ORDER BY expires")?;
            let rows = q.query_map([&key], |r| r.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        while have.len() as i64 >= LINKS_PER_USER {
            let oldest = have.remove(0);
            tx.execute("DELETE FROM remote_enrolments WHERE hash = ?1", [&oldest])?;
        }
        tx.execute(
            "INSERT INTO remote_enrolments (hash, user, purpose, created_by, expires) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![link_hash(&secret), key, purpose.text(), created_by, t + LINK_TTL],
        )?;
        Ok(())
    })
    .map_err(err)?;
    Ok(secret)
}

pub const LINKS_PER_USER: i64 = 3;

/// Who a link is for, without using it up.
pub fn peek_link(s: &Store, secret: &str) -> Result<Option<String>, String> {
    let h = link_hash(secret);
    s.read(|c| Ok(c.query_row("SELECT user FROM remote_enrolments WHERE hash = ?1 AND expires >= ?2", rusqlite::params![h, now()], |r| r.get::<_, String>(0)).optional()?)).map_err(err)
}

/// Use the link up: true exactly once, for a link that exists and has not expired.
pub fn consume_link(s: &Store, secret: &str) -> Result<Option<String>, String> {
    let h = link_hash(secret);
    s.write(|tx| {
        let user: Option<String> = tx.query_row("SELECT user FROM remote_enrolments WHERE hash = ?1 AND expires >= ?2", rusqlite::params![h, now()], |r| r.get(0)).optional()?;
        if user.is_some() {
            tx.execute("DELETE FROM remote_enrolments WHERE hash = ?1", [&h])?;
        }
        Ok(user)
    })
    .map_err(err)
}

pub fn drop_links(s: &Store, user: &str) -> Result<(), String> {
    let key = fold(user);
    s.write(|tx| {
        tx.execute("DELETE FROM remote_enrolments WHERE user = ?1", [&key])?;
        Ok(())
    })
    .map_err(err)
}

// ---- API tokens -------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiToken {
    pub id: i64,
    pub name: String,
    pub role: crate::remote::Role,
    pub created: i64,
    /// 0: never.
    pub expires: i64,
    pub last_used: i64,
    pub created_by: String,
}

pub fn valid_token_name(name: &str) -> Result<(), String> {
    crate::remote::config::valid_user_name(name).map_err(|_| "A token name uses letters, numbers, dots, dashes and underscores, up to 40 characters (for example backup-script).".to_string())
}

/// Make a token for a script: the secret is returned once and only its salted hash is kept. `days`: lifetime.
pub fn create_api_token(s: &Store, name: &str, role: crate::remote::Role, days: Option<u32>, created_by: &str) -> Result<String, String> {
    valid_token_name(name.trim())?;
    let token = format!("kora_{}", crate::security::random_hex());
    let salt = crate::security::random_hex();
    let hash = crate::remote::config::hash_token(&salt, &token);
    let expires = days.map_or(0, |d| now() + i64::from(d) * 86_400);
    s.write(|tx| {
        tx.execute(
            "INSERT INTO remote_api_tokens (name, role, salt, hash, created, expires, created_by) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![name.trim(), role.as_str(), salt, hash, now(), expires, created_by],
        )?;
        Ok(())
    })
    .map_err(|p| {
        let m = err(p);
        if m.contains("UNIQUE") {
            format!("There is already an API token named '{}'.", name.trim())
        } else {
            m
        }
    })?;
    Ok(token)
}

fn token_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ApiToken> {
    let role: String = r.get(2)?;
    Ok(ApiToken { id: r.get(0)?, name: r.get(1)?, role: role.parse().unwrap_or(crate::remote::Role::Viewer), created: r.get(3)?, expires: r.get(4)?, last_used: r.get(5)?, created_by: r.get(6)? })
}

pub fn list_api_tokens(s: &Store) -> Result<Vec<ApiToken>, String> {
    s.read(|c| {
        let mut q = c.prepare("SELECT id, name, role, created, expires, last_used, created_by FROM remote_api_tokens ORDER BY id")?;
        let rows = q.query_map([], token_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    })
    .map_err(err)
}

pub fn revoke_api_token(s: &Store, id: i64) -> Result<Option<String>, String> {
    s.write(|tx| {
        let name: Option<String> = tx.query_row("SELECT name FROM remote_api_tokens WHERE id = ?1", [id], |r| r.get(0)).optional()?;
        if name.is_some() {
            tx.execute("DELETE FROM remote_api_tokens WHERE id = ?1", [id])?;
        }
        Ok(name)
    })
    .map_err(err)
}

/// The API token `presented` is, if any and not expired. Every token is hashed and compared in constant time.
/// `last_used` is updated at most once a minute (a script that calls often does not write on every request).
pub fn api_token_for(s: &Store, presented: &str) -> Result<Option<ApiToken>, String> {
    if !presented.starts_with("kora_") {
        return Ok(None);
    }
    let t = now();
    let rows: Vec<(ApiToken, String, String)> = s
        .read(|c| {
            let mut q = c.prepare("SELECT id, name, role, created, expires, last_used, created_by, salt, hash FROM remote_api_tokens")?;
            let rows = q.query_map([], |r| Ok((token_row(r)?, r.get::<_, String>(7)?, r.get::<_, String>(8)?)))?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
        .map_err(err)?;
    let mut found = None;
    for (tok, salt, hash) in rows {
        if ct_eq(&crate::remote::config::hash_token(&salt, presented), &hash) {
            found = Some(tok);
        }
    }
    let Some(tok) = found else { return Ok(None) };
    if tok.expires != 0 && tok.expires <= t {
        return Ok(None);
    }
    if t - tok.last_used >= 60 {
        let _ = s.write(|tx| {
            tx.execute("UPDATE remote_api_tokens SET last_used = ?1 WHERE id = ?2", rusqlite::params![t, tok.id])?;
            Ok(())
        });
    }
    Ok(Some(tok))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::Role;

    fn store() -> (tempfile::TempDir, Store) {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(d.path()).unwrap();
        (d, s)
    }

    #[test]
    fn enrolment_links_work_once_and_expire() {
        let (_d, s) = store();
        let a = mint_link(&s, "Sam", Purpose::Invite, "cli").unwrap();
        assert_eq!(peek_link(&s, &a).unwrap().as_deref(), Some("sam"));
        assert_eq!(peek_link(&s, &a).unwrap().as_deref(), Some("sam"), "looking does not use it up");
        assert_eq!(consume_link(&s, &a).unwrap().as_deref(), Some("sam"));
        assert_eq!(consume_link(&s, &a).unwrap(), None, "single use");
        assert_eq!(peek_link(&s, "nope").unwrap(), None);
        // expired
        let b = mint_link(&s, "sam", Purpose::Reset, "cli").unwrap();
        s.write(|tx| {
            tx.execute("UPDATE remote_enrolments SET expires = ?1", [now() - 1])?;
            Ok(())
        })
        .unwrap();
        assert_eq!(consume_link(&s, &b).unwrap(), None);
        // only the hash is stored
        let c = mint_link(&s, "sam", Purpose::Invite, "cli").unwrap();
        let all: String = s.read(|c2| Ok(c2.query_row("SELECT group_concat(hash) FROM remote_enrolments", [], |r| r.get::<_, String>(0))?)).unwrap();
        assert!(!all.contains(&c));
    }

    #[test]
    fn a_user_has_only_a_few_live_links() {
        let (_d, s) = store();
        let links: Vec<String> = (0..5).map(|_| mint_link(&s, "sam", Purpose::Invite, "cli").unwrap()).collect();
        let live = links.iter().filter(|l| peek_link(&s, l).unwrap().is_some()).count();
        assert_eq!(live as i64, LINKS_PER_USER);
        assert!(peek_link(&s, &links[4]).unwrap().is_some(), "the newest is kept");
    }

    #[test]
    fn api_tokens_are_hashed_scoped_and_expire() {
        let (_d, s) = store();
        let t = create_api_token(&s, "backup-script", Role::Operator, Some(30), "sam").unwrap();
        assert!(t.starts_with("kora_") && t.len() > 40);
        let found = api_token_for(&s, &t).unwrap().unwrap();
        assert_eq!((found.name.as_str(), found.role), ("backup-script", Role::Operator));
        assert!(api_token_for(&s, "kora_nope").unwrap().is_none() && api_token_for(&s, "nope").unwrap().is_none());
        let stored: String = s.read(|c| Ok(c.query_row("SELECT salt || hash FROM remote_api_tokens", [], |r| r.get::<_, String>(0))?)).unwrap();
        assert!(!stored.contains(&t[5..]), "never stored");
        assert!(create_api_token(&s, "backup-script", Role::Viewer, None, "sam").unwrap_err().contains("already"));
        assert!(create_api_token(&s, "bad name", Role::Viewer, None, "sam").is_err());
        // expiry
        s.write(|tx| {
            tx.execute("UPDATE remote_api_tokens SET expires = ?1", [now() - 5])?;
            Ok(())
        })
        .unwrap();
        assert!(api_token_for(&s, &t).unwrap().is_none(), "expired");
        s.write(|tx| {
            tx.execute("UPDATE remote_api_tokens SET expires = 0", [])?;
            Ok(())
        })
        .unwrap();
        assert!(api_token_for(&s, &t).unwrap().is_some(), "never expires");
        assert_eq!(list_api_tokens(&s).unwrap().len(), 1);
        assert_eq!(revoke_api_token(&s, found.id).unwrap().as_deref(), Some("backup-script"));
        assert!(api_token_for(&s, &t).unwrap().is_none(), "revoked");
        assert_eq!(revoke_api_token(&s, found.id).unwrap(), None);
    }

    #[test]
    fn totp_steps_only_move_forward_and_recovery_codes_work_once() {
        let (_d, s) = store();
        let codes = super::super::recovery::generate().unwrap();
        let stored: Vec<String> = codes.iter().map(|c| super::super::recovery::store_form(c).unwrap()).collect();
        set_password_and_totp(&s, "Sam", "$hash", &[1u8; 20], &stored).unwrap();
        assert!(take_totp_step(&s, "sam", 100).unwrap());
        assert!(!take_totp_step(&s, "sam", 100).unwrap(), "the same step again");
        assert!(!take_totp_step(&s, "sam", 99).unwrap(), "an older step");
        assert!(take_totp_step(&s, "sam", 101).unwrap());
        let st = state(&s, "SAM").unwrap();
        assert!(st.totp && st.password && st.strong() && st.recovery_left == 10);
        assert!(use_recovery_code(&s, "sam", &codes[2]).unwrap());
        assert!(!use_recovery_code(&s, "sam", &codes[2]).unwrap(), "once");
        assert_eq!(state(&s, "sam").unwrap().recovery_left, 9);
        assert!(!use_recovery_code(&s, "nobody", &codes[0]).unwrap());
        clear_totp(&s, "sam").unwrap();
        let st = state(&s, "sam").unwrap();
        assert!(!st.totp && !st.password && !st.strong() && st.recovery_left == 0);
        assert!(code_secrets(&s, "sam").unwrap().is_none());
    }

    #[test]
    fn the_account_locks_after_repeated_failures_and_recovers_on_success() {
        let (_d, s) = store();
        set_password_and_totp(&s, "sam", "$hash", &[1u8; 20], &[]).unwrap();
        for _ in 0..(LOCK_AFTER - 1) {
            assert_eq!(note_failure(&s, "sam").unwrap(), 0);
        }
        let until = note_failure(&s, "sam").unwrap();
        assert!(until > now() + LOCK_FOR - 5);
        assert_eq!(code_secrets(&s, "sam").unwrap().unwrap().locked_until, until);
        clear_failures(&s, "sam").unwrap();
        assert_eq!(code_secrets(&s, "sam").unwrap().unwrap().locked_until, 0);
    }
}
