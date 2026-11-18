//! In-memory stand-ins for the keyring and for DPAPI, with switches for every way the real ones fail. Used by the tests
//! of this crate and (with the `fake` feature) of the others; never in the real program.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use super::{Place, Sealer, Vault, VaultError};

#[derive(Default)]
pub struct FakeVault {
    pub items: Mutex<BTreeMap<i64, String>>,
    pub state: Mutex<FakeState>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum FakeState {
    #[default]
    Working,
    Locked,
    NoBus,
}

impl FakeVault {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
    pub fn set(&self, s: FakeState) {
        *self.state.lock().unwrap() = s;
    }
    fn check(&self) -> Result<(), VaultError> {
        match &*self.state.lock().unwrap() {
            FakeState::Working => Ok(()),
            FakeState::Locked => Err(VaultError::Locked),
            FakeState::NoBus => Err(VaultError::Unavailable("no D-Bus session".into())),
        }
    }
}

impl Vault for FakeVault {
    fn place(&self) -> Place {
        Place::SecretService
    }
    fn probe(&self) -> Result<(), VaultError> {
        self.check()
    }
    fn put(&self, id: i64, _label: &str, secret: &str) -> Result<(), VaultError> {
        self.check()?;
        self.items.lock().unwrap().insert(id, secret.to_string());
        Ok(())
    }
    fn get(&self, id: i64) -> Result<Option<String>, VaultError> {
        self.check()?;
        Ok(self.items.lock().unwrap().get(&id).cloned())
    }
    fn delete(&self, id: i64) -> Result<(), VaultError> {
        self.check()?;
        self.items.lock().unwrap().remove(&id);
        Ok(())
    }
    fn list(&self) -> Result<Vec<i64>, VaultError> {
        self.check()?;
        Ok(self.items.lock().unwrap().keys().copied().collect())
    }
}

/// "Seals" by reversing and tagging with an account name: opening under another account fails, like DPAPI.
pub struct FakeSealer {
    pub account: Mutex<String>,
    pub fail_seal: Mutex<bool>,
}

impl FakeSealer {
    pub fn new(account: &str) -> Arc<Self> {
        Arc::new(Self { account: Mutex::new(account.into()), fail_seal: Mutex::new(false) })
    }
}

impl Sealer for FakeSealer {
    fn place(&self) -> Place {
        Place::Dpapi
    }
    fn seal(&self, secret: &str) -> Result<String, String> {
        if *self.fail_seal.lock().unwrap() {
            return Err("protected storage is not available".into());
        }
        Ok(format!("{}|{}", self.account.lock().unwrap(), secret.chars().rev().collect::<String>()))
    }
    fn open(&self, sealed: &str) -> Result<String, String> {
        let (who, rest) = sealed.split_once('|').ok_or("not sealed by this program")?;
        if who != *self.account.lock().unwrap() {
            return Err("sealed for another account".into());
        }
        Ok(rest.chars().rev().collect())
    }
}
