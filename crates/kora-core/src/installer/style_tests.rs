//! Clone / fork and released / latest: the mapping from the old `mode`, and both spellings on disk.

use super::registry::{PortsRecord, ServerEntry, ServerFile};
use super::*;

fn file(mode: Mode) -> ServerFile {
    ServerFile {
        name: "s".into(),
        mode,
        source: None,
        versions: None,
        dir: "/x".into(),
        ports: PortsRecord { game: 1, txadmin: 2 },
        game_build: 1,
        artifact_build: 1,
        database: None,
        github_owner: None,
        created_unix: 0,
        kora_version: "0".into(),
        resources: vec![],
    }
}

#[test]
fn the_old_mode_maps_to_the_two_settings() {
    assert_eq!(style(Mode::Production, None, None), (Source::Clone, Versions::Released));
    assert_eq!(style(Mode::Development, None, None), (Source::Fork, Versions::Latest));
    // A new field wins over the old one, each on its own.
    assert_eq!(style(Mode::Production, Some(Source::Fork), None), (Source::Fork, Versions::Released));
    assert_eq!(style(Mode::Development, None, Some(Versions::Released)), (Source::Fork, Versions::Released));
}

#[test]
fn a_request_with_only_new_fields_gets_sensible_defaults() {
    assert_eq!(resolve(None, None, None), (Source::Clone, Versions::Released));
    assert_eq!(resolve(None, Some(Source::Fork), None), (Source::Fork, Versions::Latest));
    assert_eq!(resolve(None, Some(Source::Clone), Some(Versions::Latest)), (Source::Clone, Versions::Latest));
    assert_eq!(resolve(Some(Mode::Development), None, None), (Source::Fork, Versions::Latest));
}

#[test]
fn the_mode_older_versions_read_follows_the_versions() {
    assert_eq!(compat_mode(Versions::Released), Mode::Production);
    assert_eq!(compat_mode(Versions::Latest), Mode::Development);
}

#[test]
fn a_server_file_from_before_0_8_still_loads_and_is_migrated_on_read() {
    let d = tempfile::tempdir().unwrap();
    for (word, want) in [("production", (Source::Clone, Versions::Released)), ("development", (Source::Fork, Versions::Latest))] {
        let old = format!(
            r#"{{"name":"s","mode":"{word}","dir":{:?},"ports":{{"game":1,"txadmin":2}},"game_build":1,"artifact_build":1,"database":null,"github_owner":null,"created_unix":0,"kora_version":"0.7.0","resources":[]}}"#,
            d.path()
        );
        std::fs::write(registry::server_file_path(d.path()), old).unwrap();
        let sf = ServerFile::load(d.path()).unwrap();
        assert_eq!((sf.source(), sf.versions()), want);
        assert_eq!((sf.source, sf.versions), (None, None), "nothing is rewritten by reading");
    }
}

#[test]
fn saving_writes_both_spellings_and_an_old_reader_still_understands_the_file() {
    let d = tempfile::tempdir().unwrap();
    let mut sf = file(Mode::Production);
    sf.dir = d.path().to_path_buf();
    // Own copies that follow released versions: not expressible as one old mode; the old word follows the versions.
    sf.source = Some(Source::Fork);
    sf.versions = Some(Versions::Released);
    sf.mode = Mode::Development;
    sf.save().unwrap();
    let raw: serde_json::Value = serde_json::from_slice(&std::fs::read(registry::server_file_path(d.path())).unwrap()).unwrap();
    assert_eq!((raw["mode"].as_str(), raw["source"].as_str(), raw["versions"].as_str()), (Some("production"), Some("fork"), Some("released")));
    // An older Kora ignores the two new keys (it has `mode` and no deny_unknown_fields).
    let back = ServerFile::load(d.path()).unwrap();
    assert_eq!((back.source(), back.versions()), (Source::Fork, Versions::Released));
}

#[test]
fn registry_entries_read_both_shapes() {
    let old: ServerEntry = serde_json::from_str(r#"{"name":"a","dir":"/x","mode":"development","ports":{"game":1,"txadmin":2},"created_unix":1}"#).unwrap();
    assert_eq!((old.source(), old.versions()), (Source::Fork, Versions::Latest));
    let new: ServerEntry = serde_json::from_str(r#"{"name":"a","dir":"/x","mode":"production","source":"clone","versions":"latest","ports":{"game":1,"txadmin":2},"created_unix":1}"#).unwrap();
    assert_eq!((new.source(), new.versions()), (Source::Clone, Versions::Latest));
    let text = serde_json::to_string(&old).unwrap();
    assert!(!text.contains("source"), "an untouched old record stays as it was: {text}");
}
