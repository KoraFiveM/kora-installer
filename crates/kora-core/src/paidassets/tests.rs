use super::*;
use crate::installer::registry::PortsRecord;
use crate::installer::Mode;
use std::io::Write;
use zip::write::SimpleFileOptions;

enum E<'a> {
    F(&'a str, &'a [u8]),
    D(&'a str),
    Link(&'a str, &'a str),
}

fn make_zip(dir: &Path, name: &str, entries: &[E]) -> PathBuf {
    let p = dir.join(name);
    let mut w = zip::ZipWriter::new(fs::File::create(&p).unwrap());
    let o = SimpleFileOptions::default();
    for e in entries {
        match e {
            E::F(n, b) => {
                w.start_file(*n, o).unwrap();
                w.write_all(b).unwrap();
            }
            E::D(n) => w.add_directory(*n, o).unwrap(),
            E::Link(n, t) => w.add_symlink(*n, *t, o).unwrap(),
        }
    }
    w.finish().unwrap();
    p
}

const MANIFEST: &[u8] = b"fx_version 'cerulean'\ngame 'gta5'\nversion '1.4.2'\nescrow_ignore { 'config.lua' }\n";

fn asset(prefix: &str) -> Vec<(String, &[u8])> {
    vec![
        (format!("{prefix}fxmanifest.lua"), MANIFEST),
        (format!("{prefix}.fxap"), b"\x01\x02secret"),
        (format!("{prefix}client/main.lua"), b"FXAP\x00\x01encrypted"),
        (format!("{prefix}config.lua"), b"Config = {}\n"),
    ]
}

fn zip_of(dir: &Path, name: &str, files: &[(String, &[u8])]) -> PathBuf {
    let es: Vec<E> = files.iter().map(|(n, b)| E::F(n.as_str(), b)).collect();
    make_zip(dir, name, &es)
}

/// A Kora-installed server folder with a `server.cfg` and `kora-server.json`.
fn server() -> (tempfile::TempDir, PathBuf) {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("srv");
    fs::create_dir_all(dir.join("resources/[kora]/kora_core")).unwrap();
    fs::write(dir.join("server.cfg"), "endpoint_add_tcp \"0.0.0.0:30120\"\nensure [kora]\n").unwrap();
    ServerFile {
        source: None,
        versions: None,
        name: "srv".into(),
        mode: Mode::Production,
        dir: dir.clone(),
        ports: PortsRecord { game: 30120, txadmin: 40120 },
        game_build: 1,
        artifact_build: 1,
        database: None,
        github_owner: None,
        created_unix: 0,
        kora_version: "0".into(),
        resources: vec![ResourceRecord {
            id: "kora-core".into(),
            path: "[kora]/kora_core".into(),
            repository: "https://github.com/x/kora_core.git".into(),
            branch: "main".into(),
            ..Default::default()
        }],
    }
    .save()
    .unwrap();
    (t, dir)
}

fn quiet() -> impl FnMut(&str) {
    |_| {}
}

fn kinds(p: &ImportPlan) -> Vec<ProblemKind> {
    p.problems.iter().map(|x| x.kind).collect()
}

#[test]
fn single_resource_is_found_and_imported() {
    let (t, dir) = server();
    let z = zip_of(t.path(), "qs-housing.zip", &asset("qs-housing/"));
    let p = inspect(&dir, &z);
    assert!(p.can_import, "{:?}", p.problems);
    assert_eq!(p.resources.len(), 1);
    let r = &p.resources[0];
    assert_eq!((r.name.as_str(), r.has_fxmanifest, r.has_fxap, r.escrowed, r.files), ("qs-housing", true, true, true, 4));
    assert_eq!(r.version.as_deref(), Some("1.4.2"));
    assert!(!r.replaces);
    assert!(p.problems.is_empty());

    let rep = import(&dir, &z, &ImportOpts::default(), &mut quiet()).unwrap();
    assert_eq!(rep.imported.len(), 1);
    assert!(!rep.imported[0].replaced);
    let dest = dir.join("resources/[paid]/qs-housing");
    assert!(dest.join(".fxap").is_file(), "the hidden .fxap is kept");
    assert_eq!(fs::read(dest.join("client/main.lua")).unwrap(), b"FXAP\x00\x01encrypted");
    assert_eq!(rep.ensured, vec!["qs-housing"]);
    let sf = ServerFile::load(&dir).unwrap();
    let rec = sf.resources.iter().find(|r| r.id == "qs-housing").unwrap();
    assert!(rec.is_portal());
    assert_eq!(rec.path, "[paid]/qs-housing");
    assert_eq!(rec.version.as_deref(), Some("1.4.2"));
    assert!(rec.imported_unix.is_some());
    assert!(!dir.join(".kora").read_dir().unwrap().flatten().any(|e| e.file_name().to_string_lossy().starts_with("asset-import-")), "staging removed");
    let l = list(&dir).unwrap();
    assert_eq!((l.len(), l[0].present, l[0].has_fxap, l[0].backups), (1, true, true, 0));
}

#[test]
fn double_nested_zip_is_flattened_keeping_the_name() {
    let (t, dir) = server();
    let z = zip_of(t.path(), "download (1).zip", &asset("jim-mechanic/jim-mechanic/"));
    let p = inspect(&dir, &z);
    assert!(p.can_import, "{:?}", p.problems);
    assert_eq!(p.resources.len(), 1);
    assert_eq!(p.resources[0].name, "jim-mechanic");
    assert_eq!(p.resources[0].zip_path, "jim-mechanic/jim-mechanic");
    import(&dir, &z, &ImportOpts::default(), &mut quiet()).unwrap();
    let dest = dir.join("resources/[paid]/jim-mechanic");
    assert!(dest.join("fxmanifest.lua").is_file());
    assert!(dest.join(".fxap").is_file());
    assert!(!dest.join("jim-mechanic").exists(), "no double nesting left");
}

#[test]
fn several_resources_in_one_zip() {
    let (t, dir) = server();
    let mut files = asset("bundle/[assets]/a-one/");
    files.extend(asset("bundle/b_two/"));
    files.push(("bundle/README.txt".into(), b"read me"));
    let z = zip_of(t.path(), "bundle.zip", &files);
    let p = inspect(&dir, &z);
    assert!(p.can_import, "{:?}", p.problems);
    let names: Vec<_> = p.importable().map(|r| r.name.as_str()).collect();
    assert_eq!(names.len(), 2);
    assert!(names.contains(&"a-one") && names.contains(&"b_two"));
    let rep = import(&dir, &z, &ImportOpts::default(), &mut quiet()).unwrap();
    assert_eq!(rep.imported.len(), 2);
    assert!(dir.join("resources/[paid]/a-one/.fxap").is_file());
    assert!(dir.join("resources/[paid]/b_two/.fxap").is_file());
    let cfg = fs::read_to_string(dir.join("server.cfg")).unwrap();
    assert!(cfg.contains("ensure a-one\n") && cfg.contains("ensure b_two\n"));
}

#[test]
fn missing_fxap_is_a_warning() {
    let (t, dir) = server();
    let files: Vec<_> = asset("qs-x/").into_iter().filter(|(n, _)| !n.ends_with(".fxap")).collect();
    let z = zip_of(t.path(), "qs-x.zip", &files);
    let p = inspect(&dir, &z);
    assert!(p.resources[0].escrowed && !p.resources[0].has_fxap);
    assert_eq!(kinds(&p), vec![ProblemKind::MissingFxap]);
    assert!(!p.problems[0].blocking);
    assert!(p.can_import, "a warning does not stop the import");
    let rep = import(&dir, &z, &ImportOpts::default(), &mut quiet()).unwrap();
    assert!(rep.notes.iter().any(|n| n.contains(".fxap")));
}

#[test]
fn an_open_resource_is_not_called_escrowed() {
    let (t, dir) = server();
    let z = zip_of(t.path(), "free.zip", &[("free/fxmanifest.lua".into(), b"fx_version 'cerulean'\n"), ("free/server.lua".into(), b"print('hi')")]);
    let p = inspect(&dir, &z);
    assert!(!p.resources[0].escrowed && p.problems.is_empty());
}

#[test]
fn no_fxmanifest_is_refused_and_extra_folders_are_skipped() {
    let (t, dir) = server();
    let z = zip_of(t.path(), "x.zip", &[("thing/client.lua".into(), b"x")]);
    let p = inspect(&dir, &z);
    assert!(!p.can_import);
    assert_eq!(kinds(&p), vec![ProblemKind::NoFxmanifest]);
    assert!(p.problems[0].blocking);
    assert!(import(&dir, &z, &ImportOpts::default(), &mut quiet()).unwrap_err().to_string().contains("fxmanifest"));

    let mut files = asset("good/");
    files.push(("docs/manual.pdf".into(), b"pdf"));
    let z = zip_of(t.path(), "y.zip", &files);
    let p = inspect(&dir, &z);
    assert!(p.can_import);
    assert!(p.problems.iter().any(|x| x.kind == ProblemKind::NoFxmanifest && !x.blocking));
    assert_eq!(p.importable().count(), 1);
}

#[test]
fn a_manifest_at_the_zip_root_uses_the_zip_name() {
    let (t, dir) = server();
    let z = zip_of(t.path(), "cool_car.zip", &asset(""));
    let p = inspect(&dir, &z);
    assert!(p.can_import, "{:?}", p.problems);
    assert_eq!(p.resources[0].name, "cool_car");
    assert!(kinds(&p).contains(&ProblemKind::NoFolder));
}

#[test]
fn zip_slip_and_absolute_paths_are_refused() {
    let (t, dir) = server();
    for (i, bad) in ["../evil.lua", "res/../../evil.lua", "/etc/evil", "C:\\evil", "res\\..\\..\\evil"].iter().enumerate() {
        let mut files = asset("res/");
        files.push((bad.to_string(), b"x"));
        let z = zip_of(t.path(), &format!("bad{i}.zip"), &files);
        let p = inspect(&dir, &z);
        assert!(!p.can_import, "{bad}");
        assert!(kinds(&p).contains(&ProblemKind::UnsafePath), "{bad}: {:?}", p.problems);
        assert!(import(&dir, &z, &ImportOpts::default(), &mut quiet()).is_err());
        assert!(!t.path().join("evil.lua").exists() && !dir.join("evil.lua").exists());
    }
    assert!(!dir.join("resources/[paid]/res").exists());
}

#[test]
fn symlink_entries_are_skipped_never_created() {
    let (t, dir) = server();
    let mut es = vec![];
    let files = asset("linky/");
    for (n, b) in &files {
        es.push(E::F(n.as_str(), b));
    }
    es.push(E::D("linky/d/"));
    es.push(E::Link("linky/d/l1", ".."));
    es.push(E::Link("linky/l2", "d/l1/.."));
    es.push(E::Link("linky/passwd", "/etc/passwd"));
    let z = make_zip(t.path(), "linky.zip", &es);
    let p = inspect(&dir, &z);
    assert!(p.can_import, "{:?}", p.problems);
    let skip = p.problems.iter().find(|x| x.kind == ProblemKind::SkippedEntry).unwrap();
    assert!(!skip.blocking && skip.message.starts_with("3 link"), "{}", skip.message);
    import(&dir, &z, &ImportOpts::default(), &mut quiet()).unwrap();
    let dest = dir.join("resources/[paid]/linky");
    for l in ["d/l1", "l2", "passwd"] {
        assert!(fs::symlink_metadata(dest.join(l)).is_err(), "{l} was not created");
    }
    assert!(dest.join("d").is_dir());
}

#[test]
fn zip_bomb_ratio_is_refused() {
    let (t, dir) = server();
    let zeros = vec![0u8; 40 << 20];
    let mut files = asset("bomb/");
    files.push(("bomb/stream/big.ytd".into(), &zeros));
    let z = zip_of(t.path(), "bomb.zip", &files);
    assert!(fs::metadata(&z).unwrap().len() < 1 << 20, "it compresses very well");
    let p = inspect(&dir, &z);
    assert!(!p.can_import);
    assert!(kinds(&p).contains(&ProblemKind::ZipBomb), "{:?}", p.problems);
    assert!(import(&dir, &z, &ImportOpts::default(), &mut quiet()).is_err());
    assert!(!dir.join("resources/[paid]/bomb").exists());
}

#[test]
fn a_damaged_or_foreign_file_is_not_a_zip() {
    let (t, dir) = server();
    let p = t.path().join("nope.zip");
    fs::write(&p, b"not a zip at all").unwrap();
    let plan = inspect(&dir, &p);
    assert!(!plan.can_import);
    assert_eq!(kinds(&plan), vec![ProblemKind::NotAZip]);
}

#[test]
fn name_taken_by_another_resource_is_refused() {
    let (t, dir) = server();
    fs::create_dir_all(dir.join("resources/[standalone]/qs-housing")).unwrap();
    let z = zip_of(t.path(), "q.zip", &asset("qs-housing/"));
    let p = inspect(&dir, &z);
    assert!(!p.can_import);
    let pr = p.problems.iter().find(|x| x.kind == ProblemKind::NameTaken).unwrap();
    assert!(pr.message.contains("[standalone]"), "{}", pr.message);
}

#[test]
fn bad_names_and_duplicates_are_refused() {
    let (t, dir) = server();
    let z = zip_of(t.path(), "q.zip", &asset("bad name!/"));
    assert!(kinds(&inspect(&dir, &z)).contains(&ProblemKind::BadName));
    let mut files = asset("a/dup/");
    files.extend(asset("b/dup/"));
    let z = zip_of(t.path(), "d.zip", &files);
    assert!(kinds(&inspect(&dir, &z)).contains(&ProblemKind::Duplicate));
}

fn replace_setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let (t, dir) = server();
    let z1 = zip_of(t.path(), "v1.zip", &asset("qs-housing/"));
    import(&dir, &z1, &ImportOpts::default(), &mut quiet()).unwrap();
    fs::write(dir.join("resources/[paid]/qs-housing/config.lua"), "Config = { mine = true }\n").unwrap();
    let mut v2 = asset("qs-housing/");
    v2[0].1 = b"fx_version 'cerulean'\nversion '2.0.0'\n";
    let z2 = zip_of(t.path(), "v2.zip", &v2);
    (t, dir, z2)
}

#[test]
fn replacing_keeps_a_backup_and_records_the_new_version() {
    let (_t, dir, z2) = replace_setup();
    let p = inspect(&dir, &z2);
    assert!(p.can_import && p.resources[0].replaces, "{:?}", p.problems);
    assert!(import(&dir, &z2, &ImportOpts { replace: false }, &mut quiet()).unwrap_err().to_string().contains("already imported"));
    let rep = import(&dir, &z2, &ImportOpts::default(), &mut quiet()).unwrap();
    assert!(rep.imported[0].replaced);
    let backup = dir.join(rep.imported[0].backup.as_ref().unwrap());
    assert_eq!(fs::read_to_string(backup.join("config.lua")).unwrap(), "Config = { mine = true }\n");
    assert_eq!(fs::read_to_string(dir.join("resources/[paid]/qs-housing/config.lua")).unwrap(), "Config = {}\n");
    let sf = ServerFile::load(&dir).unwrap();
    let recs: Vec<_> = sf.resources.iter().filter(|r| r.id == "qs-housing").collect();
    assert_eq!(recs.len(), 1, "one record, updated");
    assert_eq!(recs[0].version.as_deref(), Some("2.0.0"));
    let cfg = fs::read_to_string(dir.join("server.cfg")).unwrap();
    assert_eq!(cfg.matches("ensure qs-housing").count(), 1, "ensure line added once");
    assert!(rep.ensured.is_empty());
}

#[test]
fn only_the_newest_backups_are_kept() {
    let (_t, dir, z2) = replace_setup();
    for _ in 0..5 {
        import(&dir, &z2, &ImportOpts::default(), &mut quiet()).unwrap();
    }
    assert_eq!(backups_of(&dir, "qs-housing").len(), KEEP_BACKUPS);
}

#[test]
fn a_failed_replace_restores_the_old_copy() {
    let (_t, dir, z2) = replace_setup();
    let before = fs::read(server_file_path(&dir)).unwrap();
    FAIL_AFTER.with(|f| f.set(Some(1)));
    let e = import(&dir, &z2, &ImportOpts::default(), &mut quiet()).unwrap_err();
    FAIL_AFTER.with(|f| f.set(None));
    assert!(e.to_string().contains("put back"), "{e}");
    assert_eq!(fs::read_to_string(dir.join("resources/[paid]/qs-housing/config.lua")).unwrap(), "Config = { mine = true }\n", "the old copy is back");
    assert!(backups_of(&dir, "qs-housing").is_empty(), "the backup was moved back, not left behind");
    assert_eq!(fs::read(server_file_path(&dir)).unwrap(), before, "record unchanged");
}

#[test]
fn a_failure_halfway_through_several_resources_undoes_the_first() {
    let (t, dir) = server();
    let mut files = asset("a1/");
    files.extend(asset("b2/"));
    let z = zip_of(t.path(), "two.zip", &files);
    FAIL_AFTER.with(|f| f.set(Some(1)));
    assert!(import(&dir, &z, &ImportOpts::default(), &mut quiet()).is_err());
    FAIL_AFTER.with(|f| f.set(None));
    assert!(!dir.join("resources/[paid]/a1").exists() && !dir.join("resources/[paid]/b2").exists());
    assert!(!fs::read_to_string(dir.join("server.cfg")).unwrap().contains("ensure a1"));
}

#[test]
fn servers_without_a_kora_record_use_a_side_file() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("adopted");
    fs::create_dir_all(dir.join("resources")).unwrap();
    fs::write(dir.join("server.cfg"), "ensure chat").unwrap();
    let z = zip_of(t.path(), "a.zip", &asset("asset_a/"));
    import(&dir, &z, &ImportOpts::default(), &mut quiet()).unwrap();
    assert!(side_path(&dir).is_file());
    assert_eq!(list(&dir).unwrap()[0].name, "asset_a");
    assert_eq!(fs::read_to_string(dir.join("server.cfg")).unwrap(), "ensure chat\nensure asset_a\n");
}

#[test]
fn remove_moves_the_folder_away_and_drops_ensure_and_record() {
    let (t, dir) = server();
    let z = zip_of(t.path(), "q.zip", &asset("qs-housing/"));
    import(&dir, &z, &ImportOpts::default(), &mut quiet()).unwrap();
    let notes = remove(&dir, "qs-housing").unwrap();
    assert!(notes.iter().any(|n| n.contains("backup")));
    assert!(!dir.join("resources/[paid]/qs-housing").exists());
    assert_eq!(backups_of(&dir, "qs-housing").len(), 1);
    assert!(!fs::read_to_string(dir.join("server.cfg")).unwrap().contains("qs-housing"));
    assert!(list(&dir).unwrap().is_empty());
    assert!(ServerFile::load(&dir).unwrap().resources.iter().any(|r| r.id == "kora-core"), "other records kept");
    assert!(remove(&dir, "qs-housing").unwrap_err().to_string().contains("not found"));
    assert!(remove(&dir, "kora-core").is_err(), "only paid assets can be removed here");
}

#[test]
fn ensure_lines_are_removed_exactly() {
    assert_eq!(without_ensure("ensure a\nensure ab\nensure a # x\n", "a").as_deref(), Some("ensure ab\n"));
    assert_eq!(without_ensure("ensure ab\n", "a"), None);
}

#[test]
fn manifest_versions() {
    assert_eq!(manifest_version("fx_version 'cerulean'\nversion '1.0.3'\n").as_deref(), Some("1.0.3"));
    assert_eq!(manifest_version("version \"2\"").as_deref(), Some("2"));
    assert_eq!(manifest_version("version('3.1')").as_deref(), Some("3.1"));
    assert_eq!(manifest_version("fx_version 'cerulean'"), None);
    assert_eq!(manifest_version("versionx 'no'"), None);
}

#[test]
fn recent_downloads_are_listed_and_resolved_safely() {
    let t = tempfile::tempdir().unwrap();
    let d = t.path();
    fs::write(d.join("a.zip"), b"x").unwrap();
    fs::write(d.join("B.ZIP"), b"xy").unwrap();
    fs::write(d.join("notes.txt"), b"x").unwrap();
    fs::create_dir(d.join("dir.zip")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("/etc/hostname", d.join("link.zip")).unwrap();
    let now = now_unix();
    let v = recent_zips(d, now, 86_400);
    let names: Vec<_> = v.iter().map(|z| z.name.as_str()).collect();
    assert_eq!(names.len(), 2, "{names:?}");
    assert!(names.contains(&"a.zip") && names.contains(&"B.ZIP"));
    assert!(recent_zips(d, now + 2 * 86_400, 86_400).is_empty(), "older than a day");
    assert_eq!(resolve_download(d, "a.zip", now, 86_400).unwrap(), d.join("a.zip"));
    for bad in ["../a.zip", "notes.txt", "link.zip", "/etc/passwd", "sub/a.zip", ""] {
        assert!(resolve_download(d, bad, now, 86_400).is_err(), "{bad}");
    }
}

// ------------------------------------------------------------------------------------------- hints

#[test]
fn entitlement_errors_get_plain_fixes() {
    let lines = [
        "[    c-scripting-core] Creating script environments for qs-housing",
        "\u{1b}[31m[ citizen-server-impl] You lack the required entitlement to use qs-housing\u{1b}[0m",
        "[ citizen-server-impl] Failed to verify protected resource jim-mechanic",
        "[  resources] Resource could not be downloaded: 'cool_car'",
        "[ citizen-server-impl] You lack the required entitlement to use qs-housing",
        "[ script:chat] nothing here",
    ];
    let h = entitlement_hints(&lines);
    assert_eq!(h.len(), 3, "{h:?}");
    let e = h.iter().find(|x| x.kind == HintKind::Entitlement).unwrap();
    assert_eq!(e.resource.as_deref(), Some("qs-housing"));
    assert_eq!(
        e.fix,
        "Your licence key belongs to a different Cfx.re account than the one that bought qs-housing. Open portal.cfx.re → Assets → Granted Assets and transfer it, or use a key from that account."
    );
    assert!(!e.line.contains('\u{1b}'));
    let v = h.iter().find(|x| x.kind == HintKind::VerifyFailed).unwrap();
    assert_eq!(v.resource.as_deref(), Some("jim-mechanic"));
    assert!(v.fix.contains(".fxap"));
    let d = h.iter().find(|x| x.kind == HintKind::DownloadFailed).unwrap();
    assert_eq!(d.resource.as_deref(), Some("cool_car"));
    assert!(entitlement_hints(&["all good"]).is_empty());
    let unnamed = entitlement_hints(&["You lack the required entitlement to use this resource"]);
    assert!(unnamed[0].fix.contains("bought this asset"), "{}", unnamed[0].fix);
}

#[test]
fn hints_are_read_from_the_server_console_log() {
    let (_t, dir) = server();
    assert!(hints_for_server(&dir).is_empty());
    fs::create_dir_all(dir.join(".kora")).unwrap();
    fs::write(dir.join(".kora/console.log"), "boot\nFailed to verify protected resource qs-housing\n").unwrap();
    let h = hints_for_server(&dir);
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].resource.as_deref(), Some("qs-housing"));
}

#[test]
fn asset_names_that_are_paths_or_options_are_refused() {
    for evil in [".", "..", "-x", "[x]", "a/b", "a\\b"] {
        assert!(!valid_asset_name(evil), "{evil}");
    }
    assert!(valid_asset_name("cool_car-v2.1"));
}
