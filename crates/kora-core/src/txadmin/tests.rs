use super::*;

fn cfg() -> ServerConfig {
    ServerConfig {
        name: "Kora Test Server".into(),
        game_port: 30120,
        txadmin_port: 40120,
        max_clients: 48,
        game_build: 3751,
        license_key: Secret::new("cfxk_ABCDEF123456_xyz"),
        db: DbConnection { user: "kora_ab12".into(), password: Secret::new("p@ss:w/rd\"?#"), host: "127.0.0.1".into(), port: 3306, database: "kora_ab12".into() },
        master: Some(CfxIdentity { fivem_id: 1234567, username: "stick_alex".into() }),
        ensure: ["mapmanager", "kora_lib", "kora_sql", "kora_inventory", "kora_core", "[compat]", "[kora]", "kora_lib"].iter().map(|s| s.to_string()).collect(),
    }
}

fn golden(name: &str, actual: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/txadmin/golden").join(name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        fs::write(&path, actual).unwrap();
    }
    assert_eq!(fs::read_to_string(&path).unwrap(), actual, "golden {name} differs (UPDATE_GOLDEN=1 to refresh)");
}

#[test]
fn golden_files() {
    let c = cfg();
    golden("server.cfg", &server_cfg(&c).unwrap());
    golden("secrets.cfg", &secrets_cfg(&c).unwrap());
    golden("permissions.cfg", &permissions_cfg(c.master.as_ref()).unwrap());
}

#[test]
fn server_cfg_has_no_secrets() {
    let c = cfg();
    let s = server_cfg(&c).unwrap();
    assert!(!s.contains("cfxk_") && !s.contains("mysql_connection") && !s.contains("p%40ss"));
    let k = secrets_cfg(&c).unwrap();
    assert!(k.contains("sv_licenseKey \"cfxk_") && k.contains("p%40ss%3Aw%2Frd%22%3F%23"));
    assert_eq!(k.lines().filter(|l| l.contains('"')).count(), 2);
    assert!(!format!("{c:?}").contains("cfxk_") && !format!("{c:?}").contains("p@ss"));
}

#[test]
fn injection_rejected() {
    for n in ["a\nensure evil", "a\r", "a\"b", "a\\", "a;quit", "", "x\u{2028}y", &"z".repeat(101)] {
        let mut c = cfg();
        c.name = n.into();
        assert!(server_cfg(&c).is_err(), "{n:?}");
    }
    for r in ["a b", "a\nb", "a;b", "[a", "", "../x", "x\"", "[]"] {
        let mut c = cfg();
        c.ensure = vec![r.into()];
        assert!(server_cfg(&c).is_err(), "{r:?}");
    }
    let mut c = cfg();
    c.license_key = Secret::new("abc\"\nquit");
    assert!(secrets_cfg(&c).is_err());
    let mut c = cfg();
    c.db.password = Secret::new("a\nb");
    assert!(secrets_cfg(&c).is_err());
    let mut c = cfg();
    c.db.host = "h\"x".into();
    assert!(secrets_cfg(&c).is_err());
    c = cfg();
    c.db.user = "u;x".into();
    assert!(secrets_cfg(&c).is_err());
    assert!(permissions_cfg(Some(&CfxIdentity { fivem_id: 1, username: "a\nadd_ace x y allow".into() })).is_err());
    c = cfg();
    c.txadmin_port = c.game_port;
    assert!(server_cfg(&c).is_err());
}

#[test]
fn ensure_order_and_dedup() {
    let s = server_cfg(&cfg()).unwrap();
    let ens: Vec<&str> = s.lines().filter(|l| l.starts_with("ensure ")).collect();
    assert_eq!(ens, ["ensure mapmanager", "ensure kora_lib", "ensure kora_sql", "ensure kora_inventory", "ensure kora_core", "ensure [compat]", "ensure [kora]"]);
}

#[test]
fn writes_files_with_secret_perms() {
    let d = tempfile::tempdir().unwrap();
    write_server_files(d.path(), &cfg()).unwrap();
    for f in ["server.cfg", "permissions.cfg", "secrets.cfg"] {
        assert!(d.path().join(f).is_file());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(d.path().join("secrets.cfg")).unwrap().permissions().mode() & 0o777, 0o600);
    }
    // invalid config writes nothing
    let d2 = tempfile::tempdir().unwrap();
    let mut c = cfg();
    c.name = "bad\n".into();
    assert!(write_server_files(d2.path(), &c).is_err());
    assert_eq!(fs::read_dir(d2.path()).unwrap().count(), 0);
}

const HASH: &str = "$2b$10$abcdefghijklmnopqrstuuABCDEFGHIJKLMNOPQRSTUVWXYZ01234";

#[test]
fn golden_admins_and_profile() {
    let cfx = MasterSeed::Cfx(cfg().master.unwrap());
    golden("admins-cfx.json", &admins_json(&cfx, HASH, true).unwrap());
    let local = MasterSeed::Local { username: LOCAL_ADMIN_USER.into(), password: Secret::new("pw-for-golden") };
    golden("admins-local.json", &admins_json(&local, HASH, false).unwrap());
    let d = tempfile::tempdir().unwrap();
    let dir = write_profile(&d.path().join("txData"), "default", Path::new("/srv/server-data"), "server.cfg").unwrap();
    golden("config.json", &fs::read_to_string(dir.join("config.json")).unwrap());
    golden("txadmin-admin.txt", &local_admin_note(LOCAL_ADMIN_USER, &Secret::new("pw-for-golden"), 40120));
}

#[test]
fn admins_shape_matches_txadmin_validation() {
    // Mirrors AdminStore.loadAdminsFile's structure test (txAdmin 8.1.2).
    for (seed, tmp) in [(MasterSeed::Cfx(cfg().master.unwrap()), true), (MasterSeed::Local { username: "kora-admin".into(), password: Secret::new("secret-pw") }, false)] {
        let v: Value = serde_json::from_str(&admins_json(&seed, HASH, tmp).unwrap()).unwrap();
        let a = v.as_array().unwrap();
        assert_eq!(a.len(), 1);
        assert!(a[0]["name"].as_str().unwrap().len() >= 3);
        assert_eq!(a[0]["master"], true);
        assert!(a[0]["password_hash"].as_str().unwrap().starts_with("$2"));
        assert!(a[0]["permissions"].is_array());
        for (_, p) in a[0]["providers"].as_object().unwrap() {
            assert!(p["id"].as_str().unwrap().len() >= 3 && p["identifier"].as_str().unwrap().len() >= 3 && p["data"].is_object());
        }
        assert_eq!(a.iter().filter(|x| x["master"] == true).count(), 1);
    }
    assert!(admins_json(&MasterSeed::Cfx(cfg().master.unwrap()), "plaintext", false).is_err(), "hash required");
    assert!(admins_json(&MasterSeed::Local { username: "ab".into(), password: Secret::new("x") }, HASH, false).is_err());
    assert!(admins_json(&MasterSeed::Local { username: "bad\nname".into(), password: Secret::new("x") }, HASH, false).is_err());
}

#[test]
fn write_admins_is_real_bcrypt_and_never_overwrites() {
    let d = tempfile::tempdir().unwrap();
    let pw = Secret::new(random_token(20).unwrap());
    assert_eq!(pw.expose().len(), 20);
    let seed = MasterSeed::Local { username: LOCAL_ADMIN_USER.into(), password: pw.clone() };
    assert!(write_admins(d.path(), &seed).unwrap());
    let v: Value = serde_json::from_slice(&fs::read(d.path().join("admins.json")).unwrap()).unwrap();
    let h = v[0]["password_hash"].as_str().unwrap();
    assert!(bcrypt::verify(pw.expose(), h).unwrap());
    assert!(v[0].get("password_temporary").is_none());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(d.path().join("admins.json")).unwrap().permissions().mode() & 0o777, 0o600);
    }
    let before = fs::read(d.path().join("admins.json")).unwrap();
    assert!(!write_admins(d.path(), &MasterSeed::Cfx(cfg().master.unwrap())).unwrap());
    assert_eq!(before, fs::read(d.path().join("admins.json")).unwrap(), "existing admins.json untouched");
    // identity seed: temporary random hash, provider recorded
    let d2 = tempfile::tempdir().unwrap();
    assert!(write_admins(d2.path(), &MasterSeed::Cfx(cfg().master.unwrap())).unwrap());
    let v: Value = serde_json::from_slice(&fs::read(d2.path().join("admins.json")).unwrap()).unwrap();
    assert_eq!(v[0]["providers"]["citizenfx"]["identifier"], "fivem:1234567");
    assert_eq!(v[0]["password_temporary"], true);
}

#[test]
fn random_tokens_differ_and_use_alphabet() {
    let (a, b) = (random_token(24).unwrap(), random_token(24).unwrap());
    assert_ne!(a, b);
    assert!(a.bytes().all(|c| c.is_ascii_alphanumeric()));
}

#[test]
fn profile_merge() {
    let d = tempfile::tempdir().unwrap();
    let tx = d.path().join("txData");
    fs::create_dir_all(tx.join("default")).unwrap();
    fs::write(tx.join("default/config.json"), r#"{"version":1,"discordBot":{"x":1},"server":{"keep":true,"autoStart":false}}"#).unwrap();
    let dir = write_profile(&tx, "default", Path::new("/srv/server-data"), "server.cfg").unwrap();
    let v: Value = serde_json::from_slice(&fs::read(dir.join("config.json")).unwrap()).unwrap();
    assert_eq!(v["version"], 2);
    assert_eq!(v["discordBot"]["x"], 1);
    assert_eq!(v["server"]["keep"], true);
    assert_eq!(v["server"]["autoStart"], true);
    assert_eq!(v["server"]["dataPath"], "/srv/server-data");
    assert!(!tx.join("default/data").exists() && !tx.join("admins.json").exists());
    assert!(write_profile(&tx, "../x", Path::new("/s"), "server.cfg").is_err());
}

#[test]
fn env() {
    let e = txadmin_env(Path::new("/d"), 40120);
    assert!(e.contains(&("TXHOST_TXA_PORT".into(), "40120".into())));
    assert!(e.contains(&("TXHOST_GAME_NAME".into(), "fivem".into())));
}

#[test]
fn cfg_comments_have_no_semicolon() {
    // FXServer splits a cfg line at ';' even inside a '#' comment ("No such command never.").
    let c = cfg();
    for (n, s) in [("server", server_cfg(&c).unwrap()), ("secrets", secrets_cfg(&c).unwrap()), ("permissions", permissions_cfg(c.master.as_ref()).unwrap())] {
        for l in s.lines().filter(|l| l.trim_start().starts_with('#')) {
            assert!(!l.contains(';'), "{n}.cfg comment contains ';': {l}");
        }
    }
}
