//! Saved licence keys in the operating system's own secret store, with the names and the order staying in the
//! database.
//!
//! # What moves, and what does not
//! `keystore` keeps names, order and timestamps in `license_keys`; the key VALUES go through [`KeyMaterial`]
//! ([`super::material`]). This module is the implementation that picks, per key, where a value lives
//! ([`Place`], recorded in `license_key_location`):
//!
//! | place | what | where it works |
//! |---|---|---|
//! | `store` | the private `license_key_material` table, as before | everywhere (the fallback) |
//! | `dpapi` | the value sealed with the Windows account's key (DPAPI), the sealed text in the same table | Windows, for that user |
//! | `secret-service` | an item in the desktop's keyring (GNOME Keyring, KWallet, ...) over D-Bus | Linux with a session bus and an unlocked default collection |
//!
//! **Why DPAPI on Windows, not Credential Manager.** A DPAPI blob lives in the same transaction as the key's name
//! (nothing external can disagree with the database), works from a background panel running as the user without a
//! vault service or a logon-session quirk (Credential Manager entries are per logon session and a service has its
//! own), has no size or count limits, and a copy of the database file opened by another user or on another
//! computer yields nothing. The cost: it protects against other accounts and other machines, not against another
//! program of the same user (the same is true of the keychain on Linux).
//!
//! # The hard parts
//! * **No keychain** (servers, no session bus, no desktop): keys stay in the private table and Settings says so
//!   ("Kept in Kora's private file (no system keychain on this computer)"). Nothing fails.
//! * **Locked keyring**: Kora never pops an unlock prompt (a background panel has no screen). New keys are kept in
//!   the private table meanwhile; reading a key that lives in the keyring says it is locked and what to do.
//! * **CLI and panel in different sessions**: each key remembers where it lives. A session that cannot reach that
//!   place gets a guided problem naming the key, with "enter it again" ([`Unavailable`]); it never gets an empty
//!   value and never overwrites what it cannot read.
//! * **A keychain entry that disappears** (the user deleted it in their Passwords app): the same guided problem. The
//!   private-file fallback never *silently* replaces a lost value: the key stays marked until it is entered again.
//! * **Rollback**: a keyring write cannot join a database transaction, so a value is written to the keyring FIRST
//!   and the location is recorded in the transaction; deleting from the keyring happens only in [`sweep`], after a
//!   commit, for items no row points to. A crash or rollback leaves at worst an orphan item, never a lost key.
//! * **The installer needs the value** (written to the server's `secrets.cfg`, unchanged): [`crate::keystore::KeyStore::get`]
//!   resolves it in the unprivileged panel or CLI process of the person, which is the one session that can reach their
//!   keyring; the privileged side is handed the value in the operation and never reads a keychain.
//! * **Backup and moving to another computer**: the database file alone does not carry keychain values (and a
//!   DPAPI-sealed value cannot be opened elsewhere). [`crate::keystore::export_keys`] returns every value, whatever
//!   its place, so `kora store export-json` (the way back to v0.7) and a backup feature that includes secrets can
//!   write them out; how they are protected in that file is the backup feature's decision (SEAM: it calls
//!   `export_keys` and, on restore, `KeyStore::add` for each).
//! * **Opt out**: `kora keys keychain off` (or Settings) moves every key back to the private table and stops using
//!   the keychain for new keys; `on` turns it back on and moves the keys in. A downgrade to v0.7 goes through
//!   `kora store export-json`, which carries the values.
//!
//! # Threat model
//! The aim is that a copy of the Kora folder (a backup, a stolen disk image, another account reading a file) does not
//! contain licence keys in clear. It is not protection from malware running as the same user, which can ask the keyring
//! or DPAPI for the value too. Values are never logged or put in an error: [`Problem`]s name the key, not its value.
//! On Linux the Secret Service session is made with the `plain` algorithm: the value crosses the user's own session
//! bus unencrypted, readable only by processes of that user that could ask the service for it anyway.
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rusqlite::Connection;

use super::material::{KeyMaterial, SqliteKeyMaterial};
use super::{meta_get, meta_set, Store};
use crate::problem::{codes, Action, Problem};

#[cfg(windows)]
pub mod dpapi;
pub mod fake;
#[cfg(target_os = "linux")]
pub mod secret_service;

/// Where a key's value is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Store,
    Dpapi,
    SecretService,
}

impl Place {
    pub fn as_str(self) -> &'static str {
        match self {
            Place::Store => "store",
            Place::Dpapi => "dpapi",
            Place::SecretService => "secret-service",
        }
    }
    pub fn parse(s: &str) -> Place {
        match s {
            "dpapi" => Place::Dpapi,
            "secret-service" => Place::SecretService,
            _ => Place::Store,
        }
    }
    /// For Settings, in plain words.
    pub fn label(self) -> &'static str {
        match self {
            Place::Store => "Kora's private file",
            Place::Dpapi => "Windows (protected for your account)",
            Place::SecretService => "your system keychain",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultError {
    /// The keyring exists but is locked; Kora never prompts.
    Locked,
    /// No keyring can be reached from here (and why).
    Unavailable(String),
    Other(String),
}

/// An external secret store (the Linux keyring). Values are addressed by the key's row id.
pub trait Vault: Send + Sync {
    fn place(&self) -> Place;
    /// Can it be used right now?
    fn probe(&self) -> Result<(), VaultError>;
    fn put(&self, id: i64, label: &str, secret: &str) -> Result<(), VaultError>;
    fn get(&self, id: i64) -> Result<Option<String>, VaultError>;
    fn delete(&self, id: i64) -> Result<(), VaultError>;
    /// The ids of every item this Kora folder put there.
    fn list(&self) -> Result<Vec<i64>, VaultError>;
}

/// Turns a value into text that only this account can turn back (Windows DPAPI).
pub trait Sealer: Send + Sync {
    fn place(&self) -> Place;
    fn seal(&self, secret: &str) -> Result<String, String>;
    fn open(&self, sealed: &str) -> Result<String, String>;
}

#[derive(Clone, Default)]
pub struct Backends {
    pub vault: Option<Arc<dyn Vault>>,
    pub sealer: Option<Arc<dyn Sealer>>,
}

thread_local! {
    static OVERRIDE: std::cell::RefCell<Option<Backends>> = const { std::cell::RefCell::new(None) };
}

/// Tests install fakes for THEIR thread (so tests running side by side never see each other's), and nothing else is ever used in a
/// test: the real keyring is only touched by the real program.
pub fn set_backends_for_tests(b: Option<Backends>) {
    OVERRIDE.with(|o| *o.borrow_mut() = b);
}

/// The backends of this computer. `KORA_KEYCHAIN=off` in the environment turns them off (the opt-out for a whole machine).
pub fn backends() -> Backends {
    if let Some(b) = OVERRIDE.with(|o| o.borrow().clone()) {
        return b;
    }
    if cfg!(test) || std::env::var("KORA_KEYCHAIN").is_ok_and(|v| v == "off") {
        return Backends::default();
    }
    real()
}

#[cfg(target_os = "linux")]
fn real() -> Backends {
    Backends { vault: Some(Arc::new(secret_service::SecretServiceVault::new())), sealer: None }
}
#[cfg(windows)]
fn real() -> Backends {
    Backends { vault: None, sealer: Some(Arc::new(dpapi::Dpapi)) }
}
#[cfg(not(any(target_os = "linux", windows)))]
fn real() -> Backends {
    Backends::default()
}

/// The person's choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Use the system keychain when there is one (the default).
    Auto,
    /// Keep everything in Kora's private file.
    Off,
}

const MODE_KEY: &str = "keychain_mode";

pub fn mode(c: &Connection) -> Result<Mode, Problem> {
    Ok(match meta_get(c, MODE_KEY)?.as_deref() {
        Some("off") => Mode::Off,
        _ => Mode::Auto,
    })
}

pub fn set_mode(c: &Connection, m: Mode) -> Result<(), Problem> {
    meta_set(c, MODE_KEY, if m == Mode::Off { "off" } else { "auto" })
}

/// Where the next value goes, decided from the person's choice and what works right now. Pure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Store { why: StoreWhy },
    Sealed,
    Vault,
}

/// Why a value goes to the private file (shown in Settings).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreWhy {
    TurnedOff,
    NoKeychain(String),
    Locked,
}

impl StoreWhy {
    pub fn text(&self) -> String {
        match self {
            StoreWhy::TurnedOff => "Kept in Kora's private file (you turned the system keychain off).".into(),
            StoreWhy::NoKeychain(r) if r.is_empty() => "Kept in Kora's private file (no system keychain on this computer).".into(),
            StoreWhy::NoKeychain(r) => format!("Kept in Kora's private file (no system keychain could be used here: {r})."),
            StoreWhy::Locked => "Kept in Kora's private file for now (your keychain is locked; unlock it from your desktop and move the keys in with `kora keys keychain on`).".into(),
        }
    }
}

pub fn plan(mode: Mode, b: &Backends, probe: Option<&Result<(), VaultError>>) -> Target {
    if mode == Mode::Off {
        return Target::Store { why: StoreWhy::TurnedOff };
    }
    if b.sealer.is_some() {
        return Target::Sealed;
    }
    match (&b.vault, probe) {
        (Some(_), Some(Ok(()))) => Target::Vault,
        (Some(_), Some(Err(VaultError::Locked))) => Target::Store { why: StoreWhy::Locked },
        (Some(_), Some(Err(VaultError::Unavailable(r) | VaultError::Other(r)))) => Target::Store { why: StoreWhy::NoKeychain(r.clone()) },
        _ => Target::Store { why: StoreWhy::NoKeychain(String::new()) },
    }
}

/// The probe, remembered for a few seconds (D-Bus round trips are slow and every key operation asks).
fn probe_cached(v: &Arc<dyn Vault>) -> Result<(), VaultError> {
    static LAST: Mutex<Option<(Instant, Result<(), VaultError>)>> = Mutex::new(None);
    let mut g = LAST.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((t, r)) = g.as_ref() {
        if t.elapsed() < Duration::from_secs(5) {
            return r.clone();
        }
    }
    let r = v.probe();
    *g = Some((Instant::now(), r.clone()));
    r
}

fn current_probe(b: &Backends) -> Option<Result<(), VaultError>> {
    b.vault.as_ref().map(|v| if cfg!(test) { v.probe() } else { probe_cached(v) })
}

// ---- locations ------------------------------------------------------------------------------------------------

pub fn location(c: &Connection, id: i64) -> Result<(Place, String), Problem> {
    match c.query_row("SELECT place, note FROM license_key_location WHERE key_id = ?1", [id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))) {
        Ok((p, n)) => Ok((Place::parse(&p), n)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok((Place::Store, String::new())),
        Err(e) => Err(e.into()),
    }
}

fn set_location(c: &Connection, id: i64, place: Place, note: &str) -> Result<(), Problem> {
    c.execute("INSERT INTO license_key_location (key_id, place, note) VALUES (?1, ?2, ?3) ON CONFLICT(key_id) DO UPDATE SET place = excluded.place, note = excluded.note", rusqlite::params![id, place.as_str(), note])?;
    Ok(())
}

/// A sealed value in the table starts with this, so it can never be taken for a plain key.
const SEALED: &str = "dpapi1:";

/// Why a key's value cannot be read now. Carried in a [`Problem`] whose message has `{key}` where the key's name goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unavailable {
    /// The keyring or the protected storage no longer has it.
    Missing,
    Locked,
    /// This session cannot reach the place (another session, no desktop, another Windows account).
    Unreachable,
}

pub fn unavailable_problem(kind: Unavailable, place: Place) -> Problem {
    let msg = match (kind, place) {
        (Unavailable::Missing, Place::SecretService) => "The licence key {key} is not in your system keychain any more (it was deleted there, or the keychain was reset). Enter it again to keep using it.".to_string(),
        (Unavailable::Missing, _) => "The licence key {key} is missing from where Kora kept it. Enter it again to keep using it.".to_string(),
        (Unavailable::Locked, _) => "The licence key {key} is kept in your system keychain, and the keychain is locked. Unlock it from your desktop (open your Passwords or Wallet app), then try again; or enter the key again to keep it in Kora's private file.".to_string(),
        (Unavailable::Unreachable, Place::Dpapi) => "The licence key {key} was saved by another Windows account or on another computer, so this one cannot open it. Enter it again to keep using it here.".to_string(),
        (Unavailable::Unreachable, _) => "The licence key {key} is kept in the system keychain of your desktop session, which this window cannot reach (a background service, SSH or another account). Open Kora from your desktop session, or enter the key again to keep it in Kora's private file.".to_string(),
    };
    Problem::new(codes::KEYS_VALUE_MISSING, msg).with_action(Action::Retry)
}

/// The kind of a problem made by [`unavailable_problem`] (so the keystore can mark the key).
pub fn is_unavailable(p: &Problem) -> bool {
    p.code == codes::KEYS_VALUE_MISSING
}

/// The implementation `keystore` uses: per key, where the value goes and where it is read from.
pub struct Chosen {
    b: Backends,
}

impl Chosen {
    pub fn new() -> Self {
        Self { b: backends() }
    }
    pub fn with(b: Backends) -> Self {
        Self { b }
    }
    fn vault_for(&self, place: Place) -> Option<&Arc<dyn Vault>> {
        self.b.vault.as_ref().filter(|v| v.place() == place)
    }
}

impl Default for Chosen {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyMaterial for Chosen {
    fn put(&self, c: &Connection, id: i64, secret: &str) -> Result<(), Problem> {
        let m = mode(c)?;
        let probe = current_probe(&self.b);
        match plan(m, &self.b, probe.as_ref()) {
            Target::Sealed => {
                let sealer = self.b.sealer.as_ref().expect("planned a sealer");
                match sealer.seal(secret) {
                    Ok(sealed) => {
                        SqliteKeyMaterial.put(c, id, &format!("{SEALED}{sealed}"))?;
                        set_location(c, id, sealer.place(), "")
                    }
                    // Could not seal: the private file, never a lost key.
                    Err(e) => {
                        SqliteKeyMaterial.put(c, id, secret)?;
                        set_location(c, id, Place::Store, &format!("could not protect it with Windows: {e}"))
                    }
                }
            }
            Target::Vault => {
                let v = self.b.vault.as_ref().expect("planned a vault");
                match v.put(id, "Kora saved licence key", secret) {
                    Ok(()) => {
                        // The value is in the keyring; the table no longer holds it (but the row stays, empty, for a stable shape).
                        SqliteKeyMaterial.delete(c, id)?;
                        set_location(c, id, v.place(), "")
                    }
                    Err(e) => {
                        SqliteKeyMaterial.put(c, id, secret)?;
                        set_location(c, id, Place::Store, &format!("the keychain refused it: {}", describe_error(&e)))
                    }
                }
            }
            Target::Store { why } => {
                SqliteKeyMaterial.put(c, id, secret)?;
                set_location(c, id, Place::Store, &why.text())
            }
        }
    }

    fn get(&self, c: &Connection, id: i64) -> Result<Option<String>, Problem> {
        let (place, _) = location(c, id)?;
        match place {
            Place::Store => SqliteKeyMaterial.get(c, id),
            Place::Dpapi => {
                let Some(stored) = SqliteKeyMaterial.get(c, id)? else { return Err(unavailable_problem(Unavailable::Missing, place)) };
                let Some(sealed) = stored.strip_prefix(SEALED) else { return Ok(Some(stored)) };
                match self.b.sealer.as_ref().filter(|s| s.place() == place) {
                    None => Err(unavailable_problem(Unavailable::Unreachable, place)),
                    Some(s) => s.open(sealed).map(Some).map_err(|_| unavailable_problem(Unavailable::Unreachable, place)),
                }
            }
            Place::SecretService => match self.vault_for(place) {
                None => Err(unavailable_problem(Unavailable::Unreachable, place)),
                Some(v) => match v.get(id) {
                    Ok(Some(s)) => Ok(Some(s)),
                    Ok(None) => Err(unavailable_problem(Unavailable::Missing, place)),
                    Err(VaultError::Locked) => Err(unavailable_problem(Unavailable::Locked, place)),
                    Err(_) => Err(unavailable_problem(Unavailable::Unreachable, place)),
                },
            },
        }
    }

    fn delete(&self, c: &Connection, id: i64) -> Result<(), Problem> {
        // The keyring item is removed by `sweep`, after the transaction has committed (see the module docs).
        SqliteKeyMaterial.delete(c, id)?;
        c.execute("DELETE FROM license_key_location WHERE key_id = ?1", [id])?;
        Ok(())
    }
}

pub fn describe_error(e: &VaultError) -> String {
    match e {
        VaultError::Locked => "it is locked".into(),
        VaultError::Unavailable(r) | VaultError::Other(r) => r.clone(),
    }
}

/// Remove keyring items that no key points to (a deleted key, a rolled-back write). Best effort; a locked keyring leaves them for next time.
pub fn sweep(store: &Store) {
    let b = backends();
    let Some(v) = b.vault else { return };
    let Ok(ids) = v.list() else { return };
    let owned: Vec<i64> = store
        .read(|c| {
            let mut q = c.prepare("SELECT key_id FROM license_key_location WHERE place = ?1")?;
            let rows = q.query_map([v.place().as_str()], |r| r.get::<_, i64>(0))?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
        .unwrap_or_default();
    for id in ids.into_iter().filter(|i| !owned.contains(i)) {
        let _ = v.delete(id);
    }
}

// ---- status and migration -------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPlace {
    pub name: String,
    pub place: Place,
    /// Set when the value cannot be read now (see [`Unavailable`]), in words.
    pub problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub mode: Mode,
    /// Where new keys go now.
    pub active: Place,
    /// A sentence for Settings.
    pub line: String,
    pub keys: Vec<KeyPlace>,
}

pub fn status(store: &Store) -> Result<Status, Problem> {
    let b = backends();
    let probe = current_probe(&b);
    let (m, keys) = store.read(|c| {
        let m = mode(c)?;
        let mut q = c.prepare("SELECT id, name FROM license_keys ORDER BY id")?;
        let rows = q.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?.collect::<Result<Vec<_>, _>>()?;
        let chosen = Chosen::with(b.clone());
        let mut keys = Vec::new();
        for (id, name) in rows {
            let (place, _) = location(c, id)?;
            let problem = match chosen.get(c, id) {
                Ok(_) => None,
                Err(p) if is_unavailable(&p) => Some(p.message.replace("{key}", &format!("\"{name}\""))),
                Err(e) => return Err(e),
            };
            keys.push(KeyPlace { name, place, problem });
        }
        Ok((m, keys))
    })?;
    let target = plan(m, &b, probe.as_ref());
    let (active, line) = match &target {
        Target::Store { why } => (Place::Store, why.text()),
        Target::Sealed => (b.sealer.as_ref().map_or(Place::Store, |s| s.place()), "New keys are kept in Windows' protected storage for your account.".to_string()),
        Target::Vault => (Place::SecretService, "New keys are kept in your system keychain.".to_string()),
    };
    Ok(Status { mode: m, active, line, keys })
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Moved {
    pub moved: usize,
    /// Keys that could not be moved, with why (they stay where they are).
    pub left: Vec<(String, String)>,
}

/// Move every key kept in the private file into the keychain (after turning the mode on).
pub fn migrate_in(store: &Store) -> Result<Moved, Problem> {
    store.write(|c| set_mode(c, Mode::Auto))?;
    let b = backends();
    let probe = current_probe(&b);
    let target = plan(Mode::Auto, &b, probe.as_ref());
    if let Target::Store { why } = &target {
        return Err(Problem::generic(format!("There is no system keychain to move the keys into. {}", why.text())));
    }
    let chosen = Chosen::with(b.clone());
    let rows: Vec<(i64, String)> = store.read(|c| {
        let mut q = c.prepare("SELECT id, name FROM license_keys ORDER BY id")?;
        let rows = q.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })?;
    let mut out = Moved::default();
    for (id, name) in rows {
        let res = store.write(|c| {
            let (place, _) = location(c, id)?;
            if place != Place::Store {
                return Ok(false);
            }
            let Some(value) = SqliteKeyMaterial.get(c, id)?.filter(|v| !v.is_empty()) else { return Ok(false) };
            chosen.put(c, id, &value)?;
            // Verify before the transaction commits: what was written must read back the same.
            let (now_place, _) = location(c, id)?;
            if now_place == Place::Store {
                return Err(Problem::generic("it could not be moved right now"));
            }
            match chosen.get(c, id)? {
                Some(v) if v == value => Ok(true),
                _ => Err(Problem::generic("the keychain did not give the value back")),
            }
        });
        match res {
            Ok(true) => out.moved += 1,
            Ok(false) => {}
            Err(e) => out.left.push((name, e.message)),
        }
    }
    Ok(out)
}

/// Turn the keychain off and move every key back into the private file.
pub fn migrate_out(store: &Store) -> Result<Moved, Problem> {
    store.write(|c| set_mode(c, Mode::Off))?;
    let b = backends();
    let chosen = Chosen::with(b.clone());
    let rows: Vec<(i64, String)> = store.read(|c| {
        let mut q = c.prepare("SELECT id, name FROM license_keys ORDER BY id")?;
        let rows = q.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })?;
    let mut out = Moved::default();
    for (id, name) in rows {
        let res = store.write(|c| {
            let (place, _) = location(c, id)?;
            if place == Place::Store {
                return Ok(false);
            }
            let value = match chosen.get(c, id) {
                Ok(Some(v)) => v,
                Ok(None) => return Err(Problem::generic("there is no value to move")),
                Err(p) => return Err(Problem::generic(p.message.replace("{key}", "this key"))),
            };
            SqliteKeyMaterial.put(c, id, &value)?;
            set_location(c, id, Place::Store, "moved back to Kora's private file")?;
            Ok(true)
        });
        match res {
            Ok(true) => out.moved += 1,
            Ok(false) => {}
            Err(e) => out.left.push((name, e.message)),
        }
    }
    // The keyring items nothing points to now are removed (after the commits above).
    sweep(store);
    Ok(out)
}

#[cfg(test)]
mod tests;
