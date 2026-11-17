use super::*;

const ME: &str = "S-1-5-21-111-222-333-1001";

fn cfg() -> ServiceConfig {
    ServiceConfig { sid: ME.into(), user: "jo".into(), domain: "PC".into(), kora_home: r"C:\Users\jo\AppData\Local\Kora".into(), version: "0.7.0".into() }
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

#[test]
fn only_ordinary_account_sids_are_accepted() {
    assert!(valid_user_sid(ME));
    assert!(valid_user_sid("S-1-12-1-1111111111-2222222222-3333333333-4444444444"), "Entra ID account");
    for bad in ["S-1-5-18", "S-1-5-32-544", "S-1-1-0", "S-1-5-21-1-2-3", "S-1-5-21-1-2-3-4-5", "S-1-5-21-a-2-3-4", "", "S-1-5-21-1-2-3-4)(A;;GA;;;WD"] {
        assert!(!valid_user_sid(bad), "{bad}");
    }
}

#[test]
fn the_pipe_rule_names_only_the_user_and_system_and_denies_the_network() {
    let d = pipe_sddl(ME).unwrap();
    assert_eq!(d, format!("O:SYG:SYD:P(D;;GA;;;NU)(A;;GRGW;;;{ME})(A;;GA;;;SY)"));
    assert!(!d.contains(";WD)") && !d.contains(";BU)") && !d.contains(";AU)"), "no Everyone, Users or Authenticated Users");
    assert!(pipe_sddl("S-1-5-21-1-2-3-4)(A;;GA;;;WD").is_err(), "no SDDL injection through the SID");
}

#[test]
fn only_the_installing_account_at_medium_integrity_or_above_is_served() {
    let c = cfg();
    assert!(check_client(Some(ME), Some(MEDIUM_INTEGRITY), &c).is_ok());
    assert!(check_client(Some(&ME.to_lowercase()), Some(0x3000), &c).is_ok(), "elevated, same account");
    assert!(check_client(Some("S-1-5-21-111-222-333-1002"), Some(0x3000), &c).is_err(), "another account, even elevated");
    assert!(check_client(Some(SYSTEM_SID), Some(0x4000), &c).is_err());
    assert!(check_client(Some(ME), Some(0x1000), &c).unwrap_err().contains("low integrity"));
    assert!(check_client(Some(ME), None, &c).is_err());
    assert!(check_client(None, Some(0x2000), &c).is_err());
}

#[test]
fn the_panel_only_talks_to_a_pipe_owned_by_system() {
    assert!(check_server_owner(Some("S-1-5-18")).is_ok());
    assert!(check_server_owner(Some(ME)).unwrap_err().contains("pretending"), "a squatter running as the person");
    assert!(check_server_owner(Some("S-1-5-32-544")).is_err());
    assert!(check_server_owner(None).is_err());
}

#[test]
fn the_helper_lives_in_its_own_admin_only_folder_and_its_path_is_quoted() {
    let pf = Path::new(r"C:\Program Files");
    let dir = helper_dir(pf);
    assert!(dir.ends_with(Path::new("Kora").join("helper")));
    assert_eq!(bin_path(Path::new(r"C:\Program Files\Kora\helper\kora.exe")).unwrap(), r#""C:\Program Files\Kora\helper\kora.exe" helper serve"#);
    assert!(bin_path(Path::new("C:\\a\"b\\kora.exe")).is_err());
    let acl = folder_acl_args(&dir);
    assert!(acl.contains(&"/inheritance:r".to_string()) && acl.contains(&"*S-1-5-32-545:(OI)(CI)RX".to_string()));
    assert!(!acl.iter().any(|a| a.contains("S-1-5-32-545:(OI)(CI)F") || a.contains("S-1-5-32-545:(OI)(CI)M")), "users only read and run");
}

#[test]
fn the_install_plan_copies_before_anything_runs_and_starts_last() {
    let (copy, rest) = install_plan(Path::new(r"C:\Program Files"), r"C:\Users\jo\AppData\Local\Kora\bin\kora.exe", &cfg()).unwrap();
    assert!(matches!(copy.steps.last().map(|s| &s.action), Some(crate::service::Action::CopyFile { .. })));
    let lines: Vec<String> = rest.steps.iter().filter_map(|s| s.command_line()).collect();
    let create = lines.iter().find(|l| l.contains(" create ")).unwrap();
    assert!(create.contains("start= auto") && create.contains("obj= LocalSystem") && create.contains("helper serve"), "{create}");
    assert!(lines.iter().any(|l| l.contains("failure KoraHelper") && l.contains("restart/")));
    assert_eq!(lines.last().unwrap(), "sc.exe start KoraHelper");
    assert!(rest.steps.iter().any(|s| matches!(&s.action, crate::service::Action::WriteFile { contents, .. } if contents.contains(ME))));
    let un: Vec<String> = uninstall_plan(Path::new(r"C:\Program Files")).steps.iter().filter_map(|s| s.command_line()).collect();
    assert_eq!(un, ["sc.exe stop KoraHelper", "sc.exe delete KoraHelper"]);
}

#[test]
fn the_service_name_cannot_be_a_server_service() {
    assert!(!SERVICE_NAME.to_ascii_lowercase().starts_with("kora-"));
}

#[test]
fn routing_keeps_the_unsafe_operations_on_the_prompt() {
    let d = r"C:\Users\jo\kora\a";
    for (verb, args) in [
        ("admin-step", s(&["run", "--step", "system.service", "--dir", d])),
        ("admin-step", s(&["run", "--step", "system.firewall", "--dir", d])),
        ("admin-step", s(&["run", "--step", "prepare.db", "--dir", d])),
        ("admin-step", s(&["run", "--step", "build.db", "--dir", d])),
        ("admin-step", s(&["undo", "--step", "system.service", "--dir", d])),
        ("admin-step", s(&["undo", "--step", "build.db", "--dir", d])),
        ("admin-db", s(&["start"])),
        ("check", s(&["--start-database"])),
        ("helper", s(&["refresh", "--version", "0.8.0"])),
    ] {
        assert_eq!(route(verb, &args), Route::Helper, "{verb} {args:?}");
    }
    for (verb, args) in [
        ("admin-step", s(&["run", "--step", "system.schedule", "--dir", d])),
        ("admin-step", s(&["undo", "--step", "system.schedule", "--dir", d])),
        ("admin-step", s(&["undo", "--step", "prepare.db", "--dir", d])),
        ("schedule", s(&["set", "a", "--daily", "04:00"])),
        ("schedule", s(&["off", "a"])),
        ("self-update", s(&["--yes"])),
        ("app", s(&["uninstall", "--yes"])),
        ("uninstall", s(&["--yes"])),
        ("helper", s(&["install"])),
        ("helper", s(&["uninstall"])),
        ("check", s(&[])),
        ("install", s(&["--name", "a"])),
    ] {
        assert!(matches!(route(verb, &args), Route::Prompt(_)), "{verb} {args:?}");
    }
    assert!(SERVICE_VERBS.iter().all(|v| !["schedule", "self-update", "app", "uninstall", "install", "delete"].contains(v)));
}

#[test]
fn firewall_ports_exclude_system_and_remote_administration_ports() {
    for p in [30120, 40120, 30121, 1024, 65535] {
        assert!(port_allowed(p), "{p}");
    }
    for p in [0, 22, 80, 135, 139, 445, 1023, 3389, 5985, 5986, 3306, 1433, 5900] {
        assert!(!port_allowed(p), "{p}");
    }
}

#[test]
fn the_kora_folder_is_fixed_at_install_time() {
    let rec = r"C:\Users\jo\AppData\Local\Kora";
    assert!(session_home_ok(None, rec));
    assert!(session_home_ok(Some(r"c:\users\JO\appdata\local\kora\"), rec));
    assert!(!session_home_ok(Some(r"C:\Users\other\AppData\Local\Kora"), rec));
    assert!(!session_home_ok(Some(r"C:\Windows\System32"), rec));
}

#[test]
fn the_helper_only_updates_itself_forward_and_the_panel_uses_only_an_equal_version() {
    assert!(refresh_allowed("0.7.1", "0.7.0"));
    assert!(!refresh_allowed("0.7.0", "0.7.0") && !refresh_allowed("0.6.5", "0.7.0") && !refresh_allowed("garbage", "0.7.0"));
    assert_eq!(version_state("0.7.0", "0.7.0"), VersionState::Current);
    assert_eq!(version_state("0.6.9", "0.7.0"), VersionState::Older);
    assert_eq!(version_state("0.7.1", "0.7.0"), VersionState::Newer);
}

#[test]
fn the_ledger_lets_each_account_remove_only_its_own_things() {
    let other = "S-1-5-21-111-222-333-1002";
    let mut l = Ledger::default();
    l.record("service:kora-a".into(), ME);
    l.record("db:kora_x1".into(), other);
    let k = |step: &str, db: Option<&str>| ledger_keys(step, "a", db);
    assert!(ledger_allows(&l, &k("system.service", None), ME, true).is_ok());
    assert!(ledger_allows(&l, &k("system.service", None), other, true).unwrap_err().contains("another Windows account"));
    assert!(ledger_allows(&l, &k("build.db", Some("kora_x1")), ME, true).is_err(), "another account's database");
    assert!(ledger_allows(&l, &k("system.firewall", None), ME, true).unwrap_err().contains("older Kora"), "not recorded: not removed by the helper");
    assert!(ledger_allows(&l, &k("system.firewall", None), ME, false).is_ok(), "making something new is fine");
    assert!(ledger_allows(&l, &k("system.service", None), other, false).is_err(), "cannot take over another account's service name");
    assert_eq!(k("build.db", None), Vec::<String>::new());
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("Kora").join("owners.json");
    l.save(&p).unwrap();
    assert_eq!(Ledger::load(&p), l);
    l.forget("service:kora-a");
    assert_eq!(l.owner("service:kora-a"), None);
    assert_eq!(Ledger::load(&t.path().join("missing.json")), Ledger::default());
}

#[test]
fn children_get_the_fixed_folder_and_service_mode() {
    let sys = crate::helper::worker::SystemDirs { system_root: r"C:\Windows".into(), program_files: r"C:\Program Files".into(), program_files_x86: String::new(), program_data: r"C:\ProgramData".into() };
    let e = cfg().child_env(&sys, None);
    assert!(e.contains(&(SERVICE_ENV.to_string(), "1".to_string())));
    assert!(e.contains(&("KORA_HOME".to_string(), cfg().kora_home)));
    assert!(e.contains(&("KORA_WORKER_USER_SID".to_string(), ME.to_string())));
}

#[test]
fn the_risk_text_is_short_and_says_what_matters() {
    let sentences = RISK_TEXT.matches(". ").count() + 1;
    assert!(sentences <= 3, "{sentences}");
    assert!(RISK_TEXT.contains("any program running as you") && RISK_TEXT.contains("only you"));
}
