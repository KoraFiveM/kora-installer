//! Selection, fallbacks, migration both ways, locked and lost entries, orphans: all with fakes (nothing here touches a real
//! keyring or DPAPI). The fakes are installed per test thread.
use std::sync::Arc;

use super::fake::{FakeSealer, FakeState, FakeVault};
use super::*;
use crate::keystore::KeyStore;

const K1: &str = "cfxk_aaaaaaaaaaaaaaaaaaaa_1111111";
const K2: &str = "cfxk_bbbbbbbbbbbbbbbbbbbb_2222222";

struct Rig {
    _d: tempfile::TempDir,
    keys: KeyStore,
    pub vault: Arc<FakeVault>,
    pub sealer: Arc<FakeSealer>,
}

impl Drop for Rig {
    fn drop(&mut self) {
        set_backends_for_tests(None);
    }
}

fn rig() -> Rig {
    let d = tempfile::tempdir().unwrap();
    let keys = KeyStore::at(d.path().join("secrets.json"));
    set_backends_for_tests(Some(Backends::default()));
    Rig { _d: d, keys, vault: FakeVault::new(), sealer: FakeSealer::new("sam") }
}

impl Rig {
    fn with_vault(&self) {
        set_backends_for_tests(Some(Backends { vault: Some(self.vault.clone()), sealer: None }));
    }
    fn with_sealer(&self) {
        set_backends_for_tests(Some(Backends { vault: None, sealer: Some(self.sealer.clone()) }));
    }
    fn none(&self) {
        set_backends_for_tests(Some(Backends::default()));
    }
    fn store(&self) -> Store {
        Store::open(self._d.path()).unwrap()
    }
    fn table_value(&self, name: &str) -> Option<String> {
        self.store()
            .read(|c| Ok(c.query_row("SELECT m.material FROM license_key_material m JOIN license_keys k ON k.id = m.key_id WHERE k.name = ?1", [name], |r| r.get::<_, String>(0)).ok()))
            .unwrap()
    }
    fn place(&self, name: &str) -> Place {
        self.keys.keychain_status().unwrap().keys.iter().find(|k| k.name == name).unwrap().place
    }
}

#[test]
fn the_choice_is_made_from_the_setting_and_what_works() {
    let vault: Arc<dyn Vault> = FakeVault::new();
    let with_vault = Backends { vault: Some(vault), sealer: None };
    let none = Backends::default();
    let sealed = Backends { vault: None, sealer: Some(FakeSealer::new("a")) };
    assert_eq!(plan(Mode::Auto, &with_vault, Some(&Ok(()))), Target::Vault);
    assert_eq!(plan(Mode::Auto, &sealed, None), Target::Sealed);
    assert_eq!(plan(Mode::Off, &with_vault, Some(&Ok(()))), Target::Store { why: StoreWhy::TurnedOff });
    assert_eq!(plan(Mode::Off, &sealed, None), Target::Store { why: StoreWhy::TurnedOff });
    assert_eq!(plan(Mode::Auto, &none, None), Target::Store { why: StoreWhy::NoKeychain(String::new()) });
    assert_eq!(plan(Mode::Auto, &with_vault, Some(&Err(VaultError::Locked))), Target::Store { why: StoreWhy::Locked });
    assert!(matches!(plan(Mode::Auto, &with_vault, Some(&Err(VaultError::Unavailable("no bus".into())))), Target::Store { why: StoreWhy::NoKeychain(r) } if r == "no bus"));
    // The words the person reads on a server.
    assert_eq!(StoreWhy::NoKeychain(String::new()).text(), "Kept in Kora's private file (no system keychain on this computer).");
    assert!(StoreWhy::Locked.text().contains("locked") && StoreWhy::TurnedOff.text().contains("turned the system keychain off"));
}

#[test]
fn without_a_keychain_everything_stays_in_the_private_file_and_says_so() {
    let r = rig();
    r.keys.add("Main", K1).unwrap();
    assert_eq!(r.keys.get("Main").unwrap(), K1);
    assert_eq!(r.table_value("Main").as_deref(), Some(K1));
    let s = r.keys.keychain_status().unwrap();
    assert_eq!((s.active, s.mode), (Place::Store, Mode::Auto));
    assert!(s.line.contains("no system keychain on this computer"), "{}", s.line);
    assert_eq!(r.keys.list().unwrap()[0].kept, "Kora's private file");
}

#[test]
fn a_new_key_goes_to_the_keychain_and_the_table_keeps_no_value() {
    let r = rig();
    r.with_vault();
    r.keys.add("Main", K1).unwrap();
    assert_eq!(r.keys.get("Main").unwrap(), K1);
    assert_eq!(r.place("Main"), Place::SecretService);
    assert_eq!(r.table_value("Main"), None, "the value is not in the database file");
    assert_eq!(r.vault.items.lock().unwrap().len(), 1);
    // The name and the last four are still in the database (never the key).
    let info = &r.keys.list().unwrap()[0];
    assert_eq!((info.last4.as_str(), info.kept.as_str(), info.problem.clone()), ("1111", "your system keychain", None));
    // Replacing and renaming follow the key; removing it cleans the keychain up afterwards.
    r.keys.replace("Main", K2).unwrap();
    assert_eq!(r.keys.get("Main").unwrap(), K2);
    r.keys.rename("Main", "Live").unwrap();
    assert_eq!(r.keys.get("Live").unwrap(), K2);
    r.keys.remove("Live").unwrap();
    assert!(r.vault.items.lock().unwrap().is_empty(), "no orphan left in the keychain");
}

#[test]
fn a_locked_keychain_never_prompts_and_never_loses_a_key() {
    let r = rig();
    r.with_vault();
    r.keys.add("Main", K1).unwrap();
    r.vault.set(FakeState::Locked);
    // New keys wait in the private file.
    r.keys.add("Second", K2).unwrap();
    assert_eq!(r.place("Second"), Place::Store);
    assert!(r.keys.keychain_status().unwrap().line.contains("locked"));
    // The key in the keychain cannot be read: a guided problem naming it, not an empty value.
    let e = r.keys.get("Main").unwrap_err();
    assert!(e.message.contains("\"Main\"") && e.message.contains("locked"), "{}", e.message);
    assert_eq!(e.code, codes::KEYS_VALUE_MISSING);
    // The list still works, shows where it is and why it cannot be used now.
    let list = r.keys.list().unwrap();
    assert!(list[0].problem.as_deref().unwrap().contains("locked") && list[1].problem.is_none());
    // Unlocked again: all is as it was.
    r.vault.set(FakeState::Working);
    assert_eq!(r.keys.get("Main").unwrap(), K1);
}

#[test]
fn a_key_whose_keychain_entry_disappeared_is_a_guided_problem_until_it_is_entered_again() {
    let r = rig();
    r.with_vault();
    r.keys.add("Main", K1).unwrap();
    r.vault.items.lock().unwrap().clear(); // the person deleted it in their Passwords app
    let e = r.keys.get("Main").unwrap_err();
    assert!(e.message.contains("\"Main\"") && e.message.contains("Enter it again"), "{}", e.message);
    // Nothing silently turns it into an empty key: it stays marked, and other operations keep the mark.
    r.keys.rename("Main", "Primary").unwrap();
    assert!(r.keys.get("Primary").is_err());
    assert!(r.keys.list().unwrap()[0].problem.is_some());
    // Entering it again fixes it.
    r.keys.replace("Primary", K2).unwrap();
    assert_eq!(r.keys.get("Primary").unwrap(), K2);
    assert!(r.keys.list().unwrap()[0].problem.is_none());
}

#[test]
fn another_session_that_cannot_reach_the_keychain_is_told_and_changes_nothing() {
    let r = rig();
    r.with_vault();
    r.keys.add("Main", K1).unwrap();
    // The panel service: no desktop session, no keychain.
    r.none();
    let e = r.keys.get("Main").unwrap_err();
    assert!(e.message.contains("desktop session") && e.message.contains("enter the key again"), "{}", e.message);
    assert!(r.keys.list().unwrap()[0].problem.is_some());
    // Adding another key from there works (private file) and does not disturb the first.
    r.keys.add("Other", K2).unwrap();
    assert_eq!(r.place("Other"), Place::Store);
    r.with_vault();
    assert_eq!(r.keys.get("Main").unwrap(), K1);
}

#[test]
fn keys_move_into_the_keychain_and_back_out_without_losing_any() {
    let r = rig();
    r.keys.add("Main", K1).unwrap();
    r.keys.add("Second", K2).unwrap();
    // No keychain: refused with the reason, nothing changes.
    let e = r.keys.keychain_on().unwrap_err();
    assert!(e.message.contains("no system keychain"), "{}", e.message);
    r.with_vault();
    let moved = r.keys.keychain_on().unwrap();
    assert_eq!((moved.moved, moved.left.len()), (2, 0));
    assert_eq!((r.place("Main"), r.place("Second")), (Place::SecretService, Place::SecretService));
    assert_eq!(r.table_value("Main"), None);
    assert_eq!((r.keys.get("Main").unwrap().as_str(), r.keys.get("Second").unwrap().as_str()), (K1, K2));
    // The way back (opt out): every value returns to the private file, the keychain is emptied.
    let back = r.keys.keychain_off().unwrap();
    assert_eq!(back.moved, 2);
    assert_eq!(r.table_value("Main").as_deref(), Some(K1));
    assert!(r.vault.items.lock().unwrap().is_empty());
    let s = r.keys.keychain_status().unwrap();
    assert_eq!((s.mode, s.active), (Mode::Off, Place::Store));
    assert!(s.line.contains("turned the system keychain off"));
    // And while it is off, new keys stay in the file even though a keychain exists.
    r.keys.add("Third", "cfxk_cccccccccccccccccccc_3333333").unwrap();
    assert_eq!(r.place("Third"), Place::Store);
}

#[test]
fn a_move_that_fails_leaves_the_key_where_it_was() {
    let r = rig();
    r.keys.add("Main", K1).unwrap();
    r.with_vault();
    // The keychain accepts the probe but loses the value (verify before commit catches it).
    struct Lossy(Arc<FakeVault>);
    impl Vault for Lossy {
        fn place(&self) -> Place {
            Place::SecretService
        }
        fn probe(&self) -> Result<(), VaultError> {
            Ok(())
        }
        fn put(&self, _: i64, _: &str, _: &str) -> Result<(), VaultError> {
            Ok(()) // says yes, keeps nothing
        }
        fn get(&self, id: i64) -> Result<Option<String>, VaultError> {
            self.0.get(id)
        }
        fn delete(&self, id: i64) -> Result<(), VaultError> {
            self.0.delete(id)
        }
        fn list(&self) -> Result<Vec<i64>, VaultError> {
            self.0.list()
        }
    }
    set_backends_for_tests(Some(Backends { vault: Some(Arc::new(Lossy(r.vault.clone()))), sealer: None }));
    let moved = r.keys.keychain_on().unwrap();
    assert_eq!(moved.moved, 0);
    assert_eq!(moved.left.len(), 1);
    assert_eq!(r.table_value("Main").as_deref(), Some(K1), "still in the private file");
    assert_eq!(r.place("Main"), Place::Store);
    assert_eq!(r.keys.get("Main").unwrap(), K1);
}

#[test]
fn a_refused_write_falls_back_to_the_private_file() {
    let r = rig();
    // A keychain that was fine at the probe but refuses the write.
    struct Refuses;
    impl Vault for Refuses {
        fn place(&self) -> Place {
            Place::SecretService
        }
        fn probe(&self) -> Result<(), VaultError> {
            Ok(())
        }
        fn put(&self, _: i64, _: &str, _: &str) -> Result<(), VaultError> {
            Err(VaultError::Other("quota".into()))
        }
        fn get(&self, _: i64) -> Result<Option<String>, VaultError> {
            Ok(None)
        }
        fn delete(&self, _: i64) -> Result<(), VaultError> {
            Ok(())
        }
        fn list(&self) -> Result<Vec<i64>, VaultError> {
            Ok(vec![])
        }
    }
    set_backends_for_tests(Some(Backends { vault: Some(Arc::new(Refuses)), sealer: None }));
    r.keys.add("Main", K1).unwrap();
    assert_eq!(r.place("Main"), Place::Store);
    assert_eq!(r.keys.get("Main").unwrap(), K1, "never a lost key");
}

#[test]
fn orphans_in_the_keychain_are_swept_but_other_keys_are_not_touched() {
    let r = rig();
    r.with_vault();
    r.keys.add("Main", K1).unwrap();
    // A write that was rolled back left an item no key points to.
    r.vault.items.lock().unwrap().insert(999, "stale".into());
    r.keys.add("Second", K2).unwrap(); // any change sweeps
    let ids: Vec<i64> = r.vault.items.lock().unwrap().keys().copied().collect();
    assert!(!ids.contains(&999) && ids.len() == 2, "{ids:?}");
    assert_eq!(r.keys.get("Main").unwrap(), K1);
}

#[test]
fn sealed_values_open_for_this_account_only() {
    let r = rig();
    r.with_sealer();
    r.keys.add("Main", K1).unwrap();
    assert_eq!(r.place("Main"), Place::Dpapi);
    let stored = r.table_value("Main").unwrap();
    assert!(stored.starts_with("dpapi1:") && !stored.contains(K1), "sealed text only");
    assert_eq!(r.keys.get("Main").unwrap(), K1);
    // The database file copied to another Windows account: a guided problem naming the key.
    *r.sealer.account.lock().unwrap() = "someone-else".into();
    let e = r.keys.get("Main").unwrap_err();
    assert!(e.message.contains("\"Main\"") && e.message.contains("another Windows account"), "{}", e.message);
    // Sealing that fails keeps the key in the file instead of losing it.
    *r.sealer.account.lock().unwrap() = "sam".into();
    *r.sealer.fail_seal.lock().unwrap() = true;
    r.keys.add("Second", K2).unwrap();
    assert_eq!(r.place("Second"), Place::Store);
    assert_eq!(r.keys.get("Second").unwrap(), K2);
}

#[test]
fn moving_to_another_machine_carries_every_value_through_export() {
    let r = rig();
    r.with_vault();
    r.keys.add("Main", K1).unwrap();
    r.vault.set(FakeState::Locked);
    r.keys.add("Second", K2).unwrap(); // in the file
    // `kora store export-json` (the way back to v0.7) writes every key it can read, and names the ones it cannot.
    let out = tempfile::tempdir().unwrap();
    let files = crate::store::export::export_json(&r.store(), &out.path().join("e")).unwrap();
    let names: Vec<String> = files.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
    assert!(names.contains(&"secrets.json".to_string()) && names.contains(&"keys-not-exported.txt".to_string()), "{names:?}");
    let secrets = std::fs::read_to_string(out.path().join("e/secrets.json")).unwrap();
    assert!(secrets.contains(K2) && !secrets.contains(K1));
    let note = std::fs::read_to_string(out.path().join("e/keys-not-exported.txt")).unwrap();
    assert!(note.contains("\"Main\"") && !note.contains(K1));
    // With the keychain reachable, both are exported.
    r.vault.set(FakeState::Working);
    let again = tempfile::tempdir().unwrap();
    crate::store::export::export_json(&r.store(), &again.path().join("e")).unwrap();
    let secrets = std::fs::read_to_string(again.path().join("e/secrets.json")).unwrap();
    assert!(secrets.contains(K1) && secrets.contains(K2));
    assert!(!again.path().join("e/keys-not-exported.txt").exists());
}

#[test]
fn the_legacy_import_and_existing_keys_keep_working_with_no_location_row() {
    // Keys saved before this version have no row in `license_key_location`: they are in the private file.
    let r = rig();
    let s = r.store();
    s.write(|c| {
        c.execute("INSERT INTO license_keys (name, name_folded, added) VALUES ('Old', 'old', 1)", [])?;
        c.execute("INSERT INTO license_key_material (key_id, material) VALUES (1, ?1)", [K1])?;
        Ok(())
    })
    .unwrap();
    assert_eq!(r.keys.get("Old").unwrap(), K1);
    assert_eq!(r.place("Old"), Place::Store);
    r.with_vault();
    assert_eq!(r.keys.keychain_on().unwrap().moved, 1);
    assert_eq!(r.keys.get("Old").unwrap(), K1);
}
