//! The Linux Secret Service (GNOME Keyring, KWallet's Secret Service, KeePassXC, ...) over the user's D-Bus session bus.
//!
//! Items live in the DEFAULT collection with the attributes `application=kora`, `kind=licence-key`, `home=<hash of this
//! Kora folder>` (two Kora folders on one computer never see each other's keys) and `key-id=<row id>`. The label says
//! "Kora saved licence key" and never the key's name or value.
//!
//! Kora never prompts: when the collection is locked every call answers [`VaultError::Locked`] (a background panel has no
//! screen; the person unlocks from their desktop). No session bus, no service, no default collection: [`VaultError::Unavailable`].
//! The session uses the `plain` algorithm (see the module docs of [`super`]).
//!
//! UNVERIFIED on a real keyring in CI: the logic around this (selection, fallbacks, migration, lost entries) is tested
//! with [`super::fake::FakeVault`]; this file is the thin layer that speaks D-Bus.
use std::collections::HashMap;

use secret_service::blocking::{Collection, SecretService};
use secret_service::{EncryptionType, Error};
use sha2::{Digest, Sha256};

use super::{Place, Vault, VaultError};

pub struct SecretServiceVault {
    home: String,
}

impl SecretServiceVault {
    pub fn new() -> Self {
        let home = crate::installer::registry::kora_home();
        let h = Sha256::digest(home.to_string_lossy().as_bytes());
        Self { home: hex::encode(&h[..8]) }
    }

    fn attrs<'a>(&'a self, id: &'a str) -> HashMap<&'a str, &'a str> {
        HashMap::from([("application", "kora"), ("kind", "licence-key"), ("home", self.home.as_str()), ("key-id", id)])
    }

    /// Run `f` on the unlocked default collection.
    fn with<T>(&self, f: impl FnOnce(&Collection<'_>) -> Result<T, Error>) -> Result<T, VaultError> {
        // No session bus: say so plainly instead of waiting for a connection attempt to time out.
        if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() && !runtime_bus_exists() {
            return Err(VaultError::Unavailable("there is no desktop session bus (a server, SSH or a background service)".into()));
        }
        let ss = SecretService::connect(EncryptionType::Plain).map_err(map)?;
        let col = ss.get_default_collection().or_else(|_| ss.get_collection_by_alias("default")).map_err(map)?;
        if col.is_locked().map_err(map)? {
            return Err(VaultError::Locked);
        }
        f(&col).map_err(map)
    }
}

impl Default for SecretServiceVault {
    fn default() -> Self {
        Self::new()
    }
}

/// `/run/user/<uid>/bus` exists: the usual session bus of a user whose environment did not export its address.
fn runtime_bus_exists() -> bool {
    let uid = unsafe { libc::geteuid() };
    std::path::Path::new(&format!("/run/user/{uid}/bus")).exists()
}

fn map(e: Error) -> VaultError {
    match e {
        Error::Locked | Error::Prompt | Error::PromptDisconnected => VaultError::Locked,
        Error::Unavailable => VaultError::Unavailable("no Secret Service (GNOME Keyring, KWallet, ...) is running".into()),
        Error::NoResult => VaultError::Unavailable("there is no default keyring to use".into()),
        other => VaultError::Other(other.to_string()),
    }
}

impl Vault for SecretServiceVault {
    fn place(&self) -> Place {
        Place::SecretService
    }
    fn probe(&self) -> Result<(), VaultError> {
        self.with(|_| Ok(()))
    }
    fn put(&self, id: i64, label: &str, secret: &str) -> Result<(), VaultError> {
        let id = id.to_string();
        self.with(|c| c.create_item(label, self.attrs(&id), secret.as_bytes(), true, "text/plain").map(|_| ()))
    }
    fn get(&self, id: i64) -> Result<Option<String>, VaultError> {
        let id = id.to_string();
        self.with(|c| {
            let items = c.search_items(self.attrs(&id))?;
            match items.first() {
                None => Ok(None),
                Some(i) => Ok(Some(String::from_utf8_lossy(&i.get_secret()?).into_owned())),
            }
        })
    }
    fn delete(&self, id: i64) -> Result<(), VaultError> {
        let id = id.to_string();
        self.with(|c| {
            for i in c.search_items(self.attrs(&id))? {
                i.delete()?;
            }
            Ok(())
        })
    }
    fn list(&self) -> Result<Vec<i64>, VaultError> {
        self.with(|c| {
            let items = c.search_items(HashMap::from([("application", "kora"), ("kind", "licence-key"), ("home", self.home.as_str())]))?;
            let mut ids = Vec::new();
            for i in items {
                if let Some(id) = i.get_attributes()?.get("key-id").and_then(|v| v.parse::<i64>().ok()) {
                    ids.push(id);
                }
            }
            Ok(ids)
        })
    }
}
