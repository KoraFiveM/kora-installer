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
//!   ask for a fresh passkey/code check ([`reauth`]) even then.
//!
//! No cryptographic primitive is implemented here: WebAuthn is `webauthn-rs`, hashing `argon2`, HMAC/SHA from
//! RustCrypto, comparisons constant time.
pub mod db;
pub mod password;
pub mod recovery;
pub mod totp;
