//! The generated TypeScript (`web/src/api/generated/`) is up to date with the Rust API description.
//! Regenerate with `KORA_UPDATE_TS=1 cargo test -p kora-web --test api_ts`.
use std::path::PathBuf;

use kora_core::api::tsgen;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../web/src/api/generated")
}

/// Compare without caring about line ends (a Windows checkout may have CRLF).
fn same(a: &str, b: &str) -> bool {
    a.replace("\r\n", "\n") == b.replace("\r\n", "\n")
}

#[test]
fn generated_typescript_is_up_to_date() {
    let g = tsgen::generate();
    let update = std::env::var("KORA_UPDATE_TS").is_ok_and(|v| v == "1");
    let mut stale = Vec::new();
    for (file, want) in [("types.ts", &g.types_ts), ("routes.ts", &g.routes_ts)] {
        let path = dir().join(file);
        let have = std::fs::read_to_string(&path).unwrap_or_default();
        if !same(&have, want) {
            if update {
                std::fs::create_dir_all(dir()).unwrap();
                std::fs::write(&path, want).unwrap();
            } else {
                stale.push(file);
            }
        }
    }
    assert!(stale.is_empty(), "web/src/api/generated/{stale:?} is out of date. Run: KORA_UPDATE_TS=1 cargo test -p kora-web --test api_ts");
}
