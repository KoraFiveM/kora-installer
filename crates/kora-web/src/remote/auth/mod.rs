//! Remote sign-in that replaces the token link as the everyday credential.
//!
//! # What changes, and what does not
//! A remote user used to prove who they are with a **token**: a bearer secret in a link, valid until revoked, so
//! whoever held the link WAS that user. Now:
//!
//! * **passkeys** (WebAuthn, [`passkey`]): the browser holds a private key in the device's secure hardware; the
//!   panel keeps only the public key. Nothing sent to the panel can be replayed (each sign-in signs a fresh
//!   random challenge, bound to the panel's origin by the browser). Needs a secure origin (HTTPS, or localhost)
//!   and a stable DOMAIN NAME: an IP address cannot be a WebAuthn relying-party ID. See [`passkey::rp_for`].
//! * **one-time codes** ([`totp`]) plus a **password** ([`password`]) where a passkey cannot work (a bare LAN
//!   address, an old browser), with **recovery codes** ([`recovery`]) and lockouts per account and per address.
//! * the only bearer link left is the **enrolment link** ([`db::mint_link`]): made by an administrator, 30
//!   minutes, single use, stored hashed; it lets ONE person register a first credential.
//! * **API tokens** ([`db::create_api_token`]) for scripts: a role, shown once, stored hashed, optional expiry,
//!   accepted only in an `Authorization: Bearer` header (never a URL, never a cookie, never a session), still
//!   subject to the "remote actions" switch for destructive operations.
//! * old per-user token links keep working (nothing breaks on upgrade) until the user enrols a strong
//!   credential, or an administrator requires one for everybody ([`RemoteConfig::require_strong_auth`]).
//!
//! Roles and what a role may do are NOT changed here ([`crate::remote::roles`]); this is only how an identity is
//! proven. Tailscale identity stays as it was.
//!
//! # Threat model
//! * A network attacker (before HTTPS: Part A of the same release) reads or replays traffic: passkey assertions
//!   and one-time codes are single use and bound (challenge, step); passkeys additionally need the origin.
//! * A phisher serves a look-alike page: a passkey is bound to the real origin by the browser and will not
//!   answer for another. A one-time code can be phished; it is the fallback, not the default.
//! * A guesser tries names, passwords, codes: per-address limiter (shared with token sign-in), per-account lock
//!   (passkey sign-in is not blocked by it, so a stranger cannot lock the real owner out of their own passkey),
//!   argon2id with a dummy verification for unknown names, one error for "no such user" and "wrong password".
//! * A stolen database file: credential public keys, password hashes (argon2id), TOTP secrets and recovery
//!   hashes. The TOTP secret has to be reversible, so it protects only as much as the file's permissions
//!   (private, `0600`); a passkey public key is useless on its own.
//! * A stolen session cookie: needs the per-session secret header too (security.rs); the most sensitive actions
//!   ask for a fresh passkey/code check ([`state::needs_fresh`]) even then.
//!
//! No cryptographic primitive is implemented here: WebAuthn is `webauthn-rs`, hashing `argon2`, HMAC/SHA from
//! RustCrypto, comparisons constant time.
pub mod db;
pub mod passkey;
pub mod password;
pub mod recovery;
pub mod state;
pub mod totp;

pub use state::AuthState;

use kora_core::api::wire::{ResetDone, ResetWhat};

use crate::remote::{Remote, Role};

fn audit_note(home: Option<&std::path::Path>, op: &str, by: &str, who: &str, detail: &str) {
    if let Some(home) = home {
        kora_core::store::audit::record_quiet(
            home,
            &kora_core::store::audit::New {
                op: op.into(),
                actor_kind: if by == "local" { kora_core::store::audit::actor::LOCAL.into() } else { kora_core::store::audit::actor::CLI.into() },
                actor_name: Some(who.chars().take(40).collect()),
                result: kora_core::store::audit::result::OK.into(),
                detail: Some(detail.into()),
                ..Default::default()
            },
        );
    }
}

/// Add a user without a token and make their enrolment link secret (shown once). One transaction for the user; the link
/// follows.
pub fn invite(remote: &Remote, name: &str, role: Role, by: &str, audit_home: Option<&std::path::Path>) -> Result<String, String> {
    remote.update(|cfg| cfg.add_invited_user(name, role))?;
    let store = remote.auth_store()?;
    let link = db::mint_link(&store, name, db::Purpose::Invite, by)?;
    audit_note(audit_home, "user_add", by, name, "invited");
    Ok(link)
}

/// Take credentials away from `name` and, unless only the old token ends, give a new enrolment link. Every credential of
/// the kind is gone at once, enrolment links of the user are replaced, and the caller ends their sessions.
pub fn reset(remote: &Remote, name: &str, what: ResetWhat, by: &str, audit_home: Option<&std::path::Path>) -> Result<ResetDone, String> {
    let cfg = remote.config();
    let user = cfg.user(name).ok_or_else(|| format!("There is no user named '{name}'."))?.clone();
    let store = remote.auth_store()?;
    let mut detail = String::new();
    if matches!(what, ResetWhat::Passkeys | ResetWhat::All) {
        let n = db::clear_passkeys(&store, &user.name)?;
        detail.push_str(&format!("{n} passkeys removed; "));
    }
    if matches!(what, ResetWhat::Code | ResetWhat::All) {
        db::clear_totp(&store, &user.name)?;
        detail.push_str("one-time codes removed; ");
    }
    if matches!(what, ResetWhat::LegacyToken | ResetWhat::All) {
        let n = user.name.clone();
        remote.update(move |cfg| {
            if let Some(u) = cfg.users.iter_mut().find(|u| u.name.eq_ignore_ascii_case(&n)) {
                u.salt.clear();
                u.hash.clear();
            }
            Ok(())
        })?;
        detail.push_str("token link ended; ");
    }
    db::drop_links(&store, &user.name)?;
    let link = if what == ResetWhat::LegacyToken { None } else { Some(db::mint_link(&store, &user.name, db::Purpose::Reset, by)?) };
    audit_note(audit_home, "user_reset", by, &user.name, detail.trim_end_matches("; "));
    Ok(ResetDone { name: user.name, link, expires_in: db::LINK_TTL as u64 })
}
