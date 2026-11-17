//! The guard that keeps tests out of the real Kora folder (`kora_home()` panics in a test binary that has not chosen one).

#[test]
fn a_test_that_asks_for_the_real_kora_folder_is_stopped_and_told_what_to_use() {
    std::env::remove_var("KORA_HOME");
    std::env::remove_var("KORA_ALLOW_REAL_HOME");
    let r = std::panic::catch_unwind(kora_core::installer::registry::kora_home);
    let msg = *r.expect_err("must panic").downcast::<String>().unwrap();
    assert!(msg.contains("use_test_home") && msg.contains("KORA_HOME"), "{msg}");
    // the seam gives an empty temporary folder, then the same one every time
    let a = kora_core::installer::registry::use_test_home();
    assert!(a.starts_with(std::env::temp_dir()) && a.is_dir());
    assert_eq!(kora_core::installer::registry::kora_home(), a);
    assert_eq!(kora_core::installer::registry::use_test_home(), a);
}
