use super::testing::FakeRunner;
use super::*;

fn server() -> ManagedServer {
    ManagedServer {
        name: "main".into(),
        dir: "/srv/kora/main".into(),
        start_command: vec!["/srv/kora/main/run.sh".into(), "+exec".into(), "server.cfg".into()],
        user: "kora".into(),
        env_file: None,
        kora_exe: "/usr/local/bin/kora".into(),
        windows_run_as: None,
    }
}

#[test]
fn unit_contents() {
    let u = systemd_unit(&server()).unwrap();
    for want in [
        "User=kora",
        "WorkingDirectory=/srv/kora/main",
        "EnvironmentFile=-/srv/kora/main/secrets.env",
        "ExecStart=/srv/kora/main/run.sh +exec server.cfg",
        "Restart=on-failure",
        "KillMode=mixed",
        "NoNewPrivileges=true",
        "ProtectSystem=full",
        "WantedBy=multi-user.target",
    ] {
        assert!(u.contains(want), "missing {want}\n{u}");
    }
    assert!(!u.contains("User=root"));
}

#[test]
fn exec_quoting_and_injection() {
    let mut s = server();
    s.start_command = vec!["/bin/x".into(), "a b".into(), "50%".into()];
    let u = systemd_unit(&s).unwrap();
    assert!(u.contains("ExecStart=/bin/x \"a b\" \"50%%\""), "{u}");
    s.start_command = vec!["/bin/x\nUser=root".into()];
    assert!(systemd_unit(&s).is_err());
}

#[test]
fn validation() {
    let mut s = server();
    s.user = "root".into();
    assert!(s.validate().is_err());
    s = server();
    s.name = "Bad Name".into();
    assert!(s.validate().is_err());
    s = server();
    s.start_command.clear();
    assert!(s.validate().is_err());
}

#[test]
fn linux_install_plan_described() {
    let p = install_plan(&server(), Platform::Linux).unwrap();
    assert!(p.needs_admin());
    assert_eq!(p.steps.len(), 3);
    assert!(p.descriptions()[0].contains("unprivileged user 'kora'"));
    assert!(p.steps[2].command_line().unwrap().contains("enable kora-main"));
}

#[test]
fn windows_install_plan() {
    let p = install_plan(&server(), Platform::Windows).unwrap();
    let lines: Vec<_> = p.steps.iter().filter_map(|s| s.command_line()).collect();
    assert!(lines[0].starts_with("sc.exe create kora-main binPath= "));
    assert!(lines[0].contains("start= auto"));
    assert!(lines.iter().any(|l| l.contains("failure kora-main reset= 86400 actions= restart/60000/restart/60000/restart/60000")));
    assert!(p.descriptions().iter().all(|d| !d.is_empty()));
}

#[test]
fn uninstall_only_kora_things() {
    let p = uninstall_plan(&server(), Platform::Linux).unwrap();
    let removed: Vec<_> = p
        .steps
        .iter()
        .filter_map(|s| match &s.action {
            Action::RemoveFile { path } => Some(path.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(removed, vec!["/etc/systemd/system/kora-main.service"]);
    let w = uninstall_plan(&server(), Platform::Windows).unwrap();
    assert!(w.steps.last().unwrap().command_line().unwrap().contains("sc.exe delete kora-main"));
}

#[test]
fn apply_plan_writes_and_runs() {
    let r = FakeRunner::default();
    let p = install_plan(&server(), Platform::Linux).unwrap();
    apply_plan(&p, &r).unwrap();
    assert_eq!(r.calls()[0], "write /etc/systemd/system/kora-main.service");
    assert_eq!(r.files.borrow()[0].2, Some(0o644));
    assert_eq!(r.calls().len(), 3);
}

#[test]
fn apply_plan_failure_handling() {
    let r = FakeRunner::default();
    r.respond("stop", 5, "");
    // tolerant stop fails, then the rest runs
    apply_plan(&uninstall_plan(&server(), Platform::Linux).unwrap(), &r).unwrap();
    assert_eq!(r.calls().len(), 5);
    let r2 = FakeRunner::default();
    r2.respond("enable", 1, "");
    assert!(apply_plan(&install_plan(&server(), Platform::Linux).unwrap(), &r2).is_err());
}

#[test]
fn status_linux_and_windows() {
    let r = FakeRunner::default();
    r.respond("is-active", 0, "active\n");
    let m = ServiceManager::new(Platform::Linux, &r);
    assert!(m.is_running(&server()).unwrap());
    let r = FakeRunner::default();
    r.respond("is-active", 3, "inactive\n");
    assert_eq!(ServiceManager::new(Platform::Linux, &r).status(&server()).unwrap(), ServiceState::Stopped);
    // `systemctl is-active` says "inactive" for a unit that does not exist at all: that is "not installed".
    let r = FakeRunner::default();
    r.respond("is-active", 3, "inactive\n");
    r.respond("LoadState", 0, "not-found\n");
    assert_eq!(ServiceManager::new(Platform::Linux, &r).status(&server()).unwrap(), ServiceState::NotInstalled);
    let r = FakeRunner::default();
    r.respond("sc.exe query", 0, "STATE : 4  RUNNING");
    assert!(ServiceManager::new(Platform::Windows, &r).is_running(&server()).unwrap());
    let r = FakeRunner::default();
    r.respond("sc.exe query", 1060, "");
    assert_eq!(ServiceManager::new(Platform::Windows, &r).status(&server()).unwrap(), ServiceState::NotInstalled);
}

#[test]
fn start_stop_restart_commands() {
    let r = FakeRunner::default();
    let m = ServiceManager::new(Platform::Linux, &r);
    m.start(&server()).unwrap();
    m.stop(&server()).unwrap();
    m.restart(&server()).unwrap();
    assert_eq!(r.calls(), vec!["systemctl start kora-main", "systemctl stop kora-main", "systemctl restart kora-main"]);
    let r = FakeRunner::default();
    r.respond("sc.exe query", 0, "STATE : 1  STOPPED");
    ServiceManager::new(Platform::Windows, &r).restart(&server()).unwrap();
    assert_eq!(r.calls(), vec!["sc.exe stop kora-main", "sc.exe query kora-main", "sc.exe start kora-main"]);
}

#[test]
fn schedule_default_is_daily_4am() {
    assert_eq!(Schedule::default().on_calendar(), "*-*-* 04:00:00");
    assert!(TimeOfDay::new(24, 0).is_err());
    assert!(TimeOfDay::new(3, 60).is_err());
}

#[test]
fn systemd_scheduler_units() {
    let t = systemd_timer_unit(&server(), &Schedule::Weekly(Weekday::Sun, TimeOfDay::new(3, 30).unwrap())).unwrap();
    assert!(t.contains("OnCalendar=Sun *-*-* 03:30:00"));
    assert!(t.contains("Persistent=true"));
    assert!(t.contains("Unit=kora-update-main.service"));
    let s = systemd_update_unit(&server()).unwrap();
    assert!(s.contains("ExecStart=/usr/local/bin/kora update --auto /srv/kora/main"));
    assert!(s.contains("Type=oneshot"));
    let c = Schedule::Custom(vec![Weekday::Mon, Weekday::Thu], TimeOfDay::new(5, 5).unwrap());
    assert_eq!(c.on_calendar(), "Mon,Thu *-*-* 05:05:00");
    assert!(scheduler_install_plan(&server(), &Schedule::Custom(vec![], TimeOfDay::new(1, 0).unwrap()), Platform::Linux).is_err());
}

#[test]
fn windows_update_task_uses_the_folder_because_system_has_no_registry() {
    let mut s = server();
    s.dir = "C:\\Users\\Zoë Müller\\kora\\main\\".into();
    s.kora_exe = "C:\\Program Files\\Kora\\bin\\0.1.0-abc\\kora.exe".into();
    let x = task_xml(&s, &Schedule::default(), RunAs::System, "").unwrap();
    assert!(x.contains("<Command>C:\\Program Files\\Kora\\bin\\0.1.0-abc\\kora.exe</Command>"), "{x}");
    assert!(x.contains("<Arguments>update --auto &quot;C:\\Users\\Zoë Müller\\kora\\main&quot;</Arguments>"), "{x}");
}

#[test]
fn schtasks_plan_for_system_and_installing_user() {
    let sys = scheduler_install_plan(&server(), &Schedule::default(), Platform::Windows).unwrap();
    assert_eq!(sys.steps.len(), 3);
    let Action::WriteFile { path, contents, .. } = &sys.steps[0].action else { panic!() };
    // Never in the server folder (other accounts could swap it before schtasks reads it).
    assert!(path.ends_with("\\Kora\\tasks\\update-main.xml") && !path.contains("/srv/kora/main"), "{path}");
    assert!(contents.contains("<UserId>S-1-5-18</UserId>") && !contents.contains("InteractiveToken"));
    assert!(contents.contains("<StartWhenAvailable>true</StartWhenAvailable>"));
    assert!(contents.contains("<StartBoundary>2024-01-01T04:00:00</StartBoundary>") && contents.contains("<DaysInterval>1</DaysInterval>"));
    let Action::Run { program, args } = &sys.steps[1].action else { panic!() };
    assert_eq!(program, "schtasks.exe");
    assert_eq!(args, &["/Create".to_string(), "/TN".into(), "Kora\\Update main".into(), "/XML".into(), path.clone(), "/F".into()]);
    assert!(matches!(sys.steps[2].action, Action::RemoveFile { .. }));

    let cfg = ScheduleConfig { preset: Preset::Weekly, time: "23:15".into(), weekday: Some(4), run_as: RunAs::InstallingUser, enabled: true };
    let p = schedule_install_plan(&server(), &cfg, "PC-ÖZ\\Zoë O'Neil & Co", Platform::Windows).unwrap();
    let Action::WriteFile { contents, .. } = &p.steps[0].action else { panic!() };
    assert!(contents.contains("<UserId>PC-ÖZ\\Zoë O&apos;Neil &amp; Co</UserId><LogonType>InteractiveToken</LogonType>"), "{contents}");
    assert!(contents.contains("<DaysOfWeek><Friday /></DaysOfWeek>") && contents.contains("T23:15:00"));
    assert!(!contents.contains("<Password>"), "no password is ever stored");
    assert!(p.descriptions()[1].contains("as PC-ÖZ"));
}

#[test]
fn systemd_units_for_installing_user_and_system() {
    let mut s = server();
    s.dir = "/srv/kora/my server é".into();
    let u = systemd_update_unit_as(&s, RunAs::InstallingUser, "zoë").unwrap();
    assert!(u.contains("Environment=KORA_UPDATE_AS=zoë\n") && u.contains("update --auto \"/srv/kora/my server é\""), "{u}");
    assert!(!systemd_update_unit_as(&s, RunAs::System, "").unwrap().contains("KORA_UPDATE_AS"));
    assert!(systemd_update_unit_as(&s, RunAs::InstallingUser, "bad user").is_err());
    assert!(systemd_update_unit_as(&s, RunAs::InstallingUser, "").is_err());
    let t = systemd_timer_unit(&s, &Schedule::default()).unwrap();
    assert!(t.contains("RandomizedDelaySec=600") && t.contains("Persistent=true"));
}

#[test]
fn config_validation_table() {
    let c = |preset, time: &str, weekday| ScheduleConfig { preset, time: time.into(), weekday, run_as: RunAs::InstallingUser, enabled: true };
    let ok = [(Preset::Daily, "04:00", None), (Preset::Daily, "00:00", None), (Preset::Daily, "23:59", Some(3)), (Preset::Weekly, "04:00", Some(0)), (Preset::Weekly, "12:30", Some(6)), (Preset::Custom, "05:05", Some(2)), (Preset::Custom, "05:05", None)];
    for (p, t, d) in ok {
        assert!(c(p, t, d).validate().is_ok(), "{p:?} {t} {d:?}");
    }
    let bad = [
        (Preset::Daily, "24:00", None),
        (Preset::Daily, "4:00", None),
        (Preset::Daily, "04:60", None),
        (Preset::Daily, "04:00:00", None),
        (Preset::Daily, "0400", None),
        (Preset::Daily, "ab:cd", None),
        (Preset::Daily, "", None),
        (Preset::Daily, "04-00", None),
        (Preset::Daily, "+4:00", None),
        (Preset::Weekly, "04:00", None),
        (Preset::Weekly, "04:00", Some(7)),
        (Preset::Custom, "04:00", Some(9)),
    ];
    for (p, t, d) in bad {
        assert!(c(p, t, d).validate().is_err(), "{p:?} {t} {d:?} must be rejected");
    }
}

#[test]
fn describe_is_plain_language() {
    let mut c = ScheduleConfig::default_for_install();
    assert_eq!(describe_for(&c, "jamie"), "Every day at 04:00, as you (jamie)");
    c.run_as = RunAs::System;
    assert_eq!(describe_for(&c, "jamie"), "Every day at 04:00, as the system account");
    c = ScheduleConfig { preset: Preset::Weekly, time: "03:30".into(), weekday: Some(6), run_as: RunAs::InstallingUser, enabled: true };
    assert_eq!(describe_for(&c, "zoë"), "Every Sunday at 03:30, as you (zoë)");
    c.enabled = false;
    assert!(describe_for(&c, "x").starts_with("Off"));
}

#[test]
fn an_older_system_schedule_reads_back_as_system() {
    // Linux: the unit files the previous Kora wrote.
    let s = server();
    let old_timer = systemd_timer_unit(&s, &Schedule::Weekly(Weekday::Sun, TimeOfDay::new(3, 30).unwrap())).unwrap();
    let old_service = systemd_update_unit(&s).unwrap();
    let c = parse_timer(&old_timer, &old_service).unwrap();
    assert_eq!((c.preset, c.time.as_str(), c.weekday, c.run_as), (Preset::Weekly, "03:30", Some(6), RunAs::System));
    let new_service = systemd_update_unit_as(&s, RunAs::InstallingUser, "jamie").unwrap();
    assert_eq!(parse_timer(&systemd_timer_unit(&s, &Schedule::default()).unwrap(), &new_service).unwrap().run_as, RunAs::InstallingUser);
    assert_eq!(parse_timer(&systemd_timer_unit(&s, &Schedule::default()).unwrap(), &new_service).unwrap().preset, Preset::Daily);

    // through get_with, no schedule.json present
    let r = FakeRunner::default();
    r.respond("cat /etc/systemd/system/kora-update-main.timer", 0, &old_timer);
    r.respond("cat /etc/systemd/system/kora-update-main.service", 0, &old_service);
    let dir = std::env::temp_dir().join("kora-sched-test-none");
    assert_eq!(get_with(&s, &dir, &r, Platform::Linux).unwrap().run_as, RunAs::System);
    // nothing there: no schedule
    assert!(get_with(&s, &dir, &FakeRunner::default(), Platform::Linux).is_none());

    // Windows: what `schtasks /Query /XML` printed for a task made with /RU SYSTEM
    let xml = "<Task><Triggers><CalendarTrigger><StartBoundary>2024-05-01T04:00:00</StartBoundary><ScheduleByDay><DaysInterval>1</DaysInterval></ScheduleByDay></CalendarTrigger></Triggers><Principals><Principal id=\"Author\"><UserId>S-1-5-18</UserId><RunLevel>HighestAvailable</RunLevel></Principal></Principals></Task>";
    let c = parse_task_xml(xml).unwrap();
    assert_eq!((c.preset, c.time.as_str(), c.run_as), (Preset::Daily, "04:00", RunAs::System));
    // and our own XML round-trips
    let ours = task_xml(&s, &Schedule::Weekly(Weekday::Wed, TimeOfDay::new(1, 5).unwrap()), RunAs::InstallingUser, "PC\\jamie").unwrap();
    let c = parse_task_xml(&ours).unwrap();
    assert_eq!((c.preset, c.time.as_str(), c.weekday, c.run_as), (Preset::Weekly, "01:05", Some(2), RunAs::InstallingUser));
}

#[test]
fn stored_config_is_the_source_of_truth_and_a_missing_job_reads_as_off() {
    let dir = std::env::temp_dir().join(format!("kora-sched-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let cfg = ScheduleConfig { preset: Preset::Daily, time: "02:15".into(), weekday: None, run_as: RunAs::System, enabled: true };
    save_schedule(&dir, &cfg, "jamie").unwrap();
    let s = server();
    // OS job missing -> reported disabled, config kept
    let c = get_with(&s, &dir, &FakeRunner::default(), Platform::Linux).unwrap();
    assert!(!c.enabled && c.time == "02:15");
    // OS job present -> enabled as stored
    let r = FakeRunner::default();
    r.respond("cat /etc/systemd/system/kora-update-main.timer", 0, &systemd_timer_unit(&s, &Schedule::default()).unwrap());
    assert_eq!(get_with(&s, &dir, &r, Platform::Linux).unwrap(), cfg);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn set_args_rebuild_the_cli_call_for_elevation() {
    let rec = ServerRecord { name: "main".into(), dir: "/srv/kora/main".into(), mode: crate::installer::Mode::Development, ports: crate::installer::registry::PortsRecord { game: 30120, txadmin: 40120 }, created_unix: 0 };
    let w = ScheduleConfig { preset: Preset::Weekly, time: "03:30".into(), weekday: Some(6), run_as: RunAs::System, enabled: true };
    assert_eq!(set_args(&rec, &w), ["schedule", "set", "/srv/kora/main", "--weekly", "Sun", "03:30", "--as-system"].map(String::from));
    assert_eq!(set_args(&rec, &ScheduleConfig::default_for_install()), ["schedule", "set", "/srv/kora/main", "--daily", "04:00", "--as-user"].map(String::from));
}

#[test]
fn removal_touches_only_kora_named_jobs() {
    let mut s = server();
    s.name = "my-srv".into();
    for p in [Platform::Linux, Platform::Windows] {
        for step in scheduler_uninstall_plan(&s, p).unwrap().steps {
            match &step.action {
                Action::RemoveFile { path } => assert!(path.starts_with("/etc/systemd/system/kora-update-my-srv."), "{path}"),
                Action::Run { program, args } if program == "schtasks.exe" => assert_eq!(args, &["/Delete", "/TN", "Kora\\Update my-srv", "/F"].map(String::from)),
                Action::Run { args, .. } => assert!(args.iter().all(|a| a == "daemon-reload" || a == "disable" || a == "--now" || a == "kora-update-my-srv.timer")),
                other => panic!("unexpected {other:?}"),
            }
        }
    }
}

#[test]
fn scheduler_uninstall() {
    let l = scheduler_uninstall_plan(&server(), Platform::Linux).unwrap();
    assert_eq!(l.steps.len(), 4);
    let w = scheduler_uninstall_plan(&server(), Platform::Windows).unwrap();
    assert_eq!(w.steps[0].command_line().unwrap(), "schtasks.exe /Delete /TN Kora\\Update main /F");
}

#[test]
fn private_tmp_only_outside_tmp() {
    assert!(systemd_unit(&server()).unwrap().contains("PrivateTmp=true"));
    let mut s = server();
    s.dir = "/tmp/kora-full/srv".into();
    assert!(systemd_unit(&s).unwrap().contains("PrivateTmp=false"));
    s.dir = "/tmpfoo/srv".into();
    assert!(systemd_unit(&s).unwrap().contains("PrivateTmp=true"));
}

#[test]
fn windows_service_runs_as_its_virtual_account_with_access_to_its_folder_only() {
    let mut s = server();
    s.dir = "C:\\Kora\\main".into();
    s.kora_exe = "C:\\Program Files\\Kora\\bin\\0.1.0-abc\\kora.exe".into();
    let p = install_plan(&s, Platform::Windows).unwrap();
    let lines: Vec<_> = p.steps.iter().filter_map(|s| s.command_line()).collect();
    // the virtual account is set at creation, and the service gets the folder (not the registry name: it cannot read the installer's per-user registry)
    assert!(lines[0].contains("obj= NT SERVICE\\kora-main"), "{}", lines[0]);
    assert!(lines[0].contains("service-host \"C:\\Kora\\main\""), "{}", lines[0]);
    let acl = lines.iter().find(|l| l.starts_with("icacls.exe")).unwrap();
    assert_eq!(acl, "icacls.exe C:\\Kora\\main /grant NT SERVICE\\kora-main:(OI)(CI)M");
    // and the grant is taken away again before the service goes
    let u = uninstall_plan(&s, Platform::Windows).unwrap();
    assert_eq!(u.steps[0].command_line().unwrap(), "icacls.exe C:\\Kora\\main /remove NT SERVICE\\kora-main");
    assert!(u.steps[0].ignore_failure);
}

#[test]
fn windows_service_keeps_only_ordinary_user_privileges_and_grants_are_a_separate_plan() {
    let mut s = server();
    s.dir = "C:\\Kora\\main".into();
    let reg: Vec<_> = windows_register_plan(&s).steps.iter().filter_map(|s| s.command_line()).collect();
    let privs = reg.iter().find(|l| l.contains(" privs ")).expect("the privilege list is set");
    assert_eq!(*privs, "sc.exe privs kora-main SeChangeNotifyPrivilege/SeIncreaseWorkingSetPrivilege");
    assert!(!privs.contains("Impersonate"));
    assert!(reg.iter().all(|l| !l.starts_with("icacls")), "registering needs no folder rights: {reg:?}");
    let grants: Vec<_> = windows_grant_plan(&s).steps.iter().filter_map(|s| s.command_line()).collect();
    assert!(grants.iter().all(|l| l.starts_with("icacls.exe")) && grants.len() == 4);
    let all = install_plan(&s, Platform::Windows).unwrap().steps.len();
    assert_eq!(all, reg.len() + grants.len());
}

#[test]
fn copy_and_rmdir_actions_go_through_the_runner() {
    let r = FakeRunner::default();
    let p = Plan::new(vec![
        SystemStep::copy("copy", "/home/me/kora", "/opt/kora/v/kora", 0o755),
        SystemStep::remove_dir_if_empty("rmdir", "/opt/kora/v"),
    ]);
    apply_plan(&p, &r).unwrap();
    assert_eq!(r.calls(), ["copy /home/me/kora /opt/kora/v/kora", "rmdir /opt/kora/v"]);
}

#[cfg(unix)]
#[test]
fn system_runner_copies_atomically_with_mode_and_only_removes_empty_dirs() {
    use std::os::unix::fs::PermissionsExt;
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("src");
    std::fs::write(&src, b"program").unwrap();
    let dst = t.path().join("opt/kora/v1/kora");
    let rn = SystemRunner;
    rn.copy_file(src.to_str().unwrap(), dst.to_str().unwrap(), Some(0o755)).unwrap();
    assert_eq!(std::fs::read(&dst).unwrap(), b"program");
    assert_eq!(std::fs::metadata(&dst).unwrap().permissions().mode() & 0o777, 0o755);
    assert!(!dst.with_extension("kora-new").exists(), "no temp file left");
    // not empty: left alone, no error
    rn.remove_empty_dir(dst.parent().unwrap().to_str().unwrap()).unwrap();
    assert!(dst.exists());
    rn.remove_file(dst.to_str().unwrap()).unwrap();
    rn.remove_empty_dir(dst.parent().unwrap().to_str().unwrap()).unwrap();
    assert!(!dst.parent().unwrap().exists());
    rn.remove_empty_dir(dst.parent().unwrap().to_str().unwrap()).unwrap(); // already gone: fine
}

#[test]
fn windows_service_account_can_read_the_secrets_it_needs_despite_removed_inheritance() {
    let mut s = server();
    s.dir = "C:\\Kora\\main\\".into();
    let p = install_plan(&s, Platform::Windows).unwrap();
    let lines: Vec<_> = p.steps.iter().filter_map(|s| s.command_line()).collect();
    for f in ["secrets.cfg", "database.json"] {
        let l = lines.iter().find(|l| l.contains(f)).unwrap_or_else(|| panic!("no grant for {f}: {lines:?}"));
        assert_eq!(*l, format!("icacls.exe C:\\Kora\\main\\{f} /grant NT SERVICE\\kora-main:(R)"));
    }
    let l = lines.iter().find(|l| l.contains("admins.json")).expect("txAdmin must be able to read its admin list");
    assert_eq!(*l, "icacls.exe C:\\Kora\\main\\txData\\admins.json /grant NT SERVICE\\kora-main:(M)");
    // a server without a database has no database.json: that grant must not abort the install
    assert!(p.steps.iter().filter(|st| st.command_line().is_some_and(|l| l.contains("database.json") || l.contains("secrets.cfg") || l.contains("admins.json"))).all(|st| st.ignore_failure));
}

/// Answers `sc query` with a scripted sequence of states; records every call.
struct Sequence {
    states: std::cell::RefCell<Vec<&'static str>>,
    log: std::cell::RefCell<Vec<String>>,
}

impl CommandRunner for Sequence {
    fn run(&self, program: &str, args: &[String]) -> std::io::Result<CommandOutput> {
        self.log.borrow_mut().push(format!("{program} {}", args.join(" ")));
        let out = if args.first().map(String::as_str) == Some("query") {
            let mut s = self.states.borrow_mut();
            let st = if s.len() > 1 { s.remove(0) } else { s[0] };
            format!("SERVICE_NAME: kora-main\n        STATE              : 3  {st}\n")
        } else {
            String::new()
        };
        Ok(CommandOutput { code: 0, stdout: out, stderr: String::new() })
    }
    fn write_file(&self, _: &str, _: &str, _: Option<u32>) -> std::io::Result<()> { Ok(()) }
    fn remove_file(&self, _: &str) -> std::io::Result<()> { Ok(()) }
    fn copy_file(&self, _: &str, _: &str, _: Option<u32>) -> std::io::Result<()> { Ok(()) }
    fn remove_empty_dir(&self, _: &str) -> std::io::Result<()> { Ok(()) }
}

#[test]
fn windows_restart_waits_for_stopped_before_starting_again() {
    let r = Sequence { states: std::cell::RefCell::new(vec!["STOP_PENDING", "STOP_PENDING", "STOPPED"]), log: Default::default() };
    ServiceManager::new(Platform::Windows, &r).restart(&server()).unwrap();
    let log = r.log.borrow();
    let start = log.iter().position(|l| l.starts_with("sc.exe start")).expect("start was called");
    let queries_before = log[..start].iter().filter(|l| l.contains("query")).count();
    assert!(log[0].starts_with("sc.exe stop"));
    assert_eq!(queries_before, 3, "polled until STOPPED (STOP_PENDING is not stopped): {log:?}");
}

#[test]
fn windows_stop_returns_only_when_stopped_and_gives_up_with_a_clear_error() {
    let r = Sequence { states: std::cell::RefCell::new(vec!["STOP_PENDING", "STOPPED"]), log: Default::default() };
    ServiceManager::new(Platform::Windows, &r).stop(&server()).unwrap();
    let stuck = Sequence { states: std::cell::RefCell::new(vec!["STOP_PENDING"]), log: Default::default() };
    let e = ServiceManager::new(Platform::Windows, &stuck).wait_until_stopped(&server(), std::time::Duration::from_millis(5), std::time::Duration::from_millis(1)).unwrap_err().to_string();
    assert!(e.contains("did not stop"), "{e}");
    // Linux stops synchronously: no polling
    let l = Sequence { states: std::cell::RefCell::new(vec!["x"]), log: Default::default() };
    ServiceManager::new(Platform::Linux, &l).wait_until_stopped(&server(), std::time::Duration::ZERO, std::time::Duration::ZERO).unwrap();
    assert!(l.log.borrow().is_empty());
}

/// Runs the real `schtasks` (needs administrator rights and changes the machine), so it is ignored by
/// default; the windows-smoke workflow runs it on an ephemeral Windows VM with `-- --ignored`.
#[cfg(windows)]
#[test]
#[ignore = "changes the real machine: run by the windows-smoke workflow"]
fn real_windows_scheduled_task_is_created_and_removed() {
    let mut s = server();
    s.name = "smoketask".into();
    s.dir = "C:\\kora-smoke\\task".into();
    s.kora_exe = "C:\\Windows\\System32\\cmd.exe".into();
    let rn = SystemRunner;
    let query = || rn.run("schtasks.exe", &["/Query".into(), "/TN".into(), "Kora\\Update smoketask".into(), "/XML".into()]).unwrap();
    for run_as in [RunAs::InstallingUser, RunAs::System] {
        let cfg = ScheduleConfig { preset: Preset::Daily, time: "04:00".into(), weekday: None, run_as, enabled: true };
        let user = installing_user();
        apply_plan(&schedule_install_plan(&s, &cfg, &user, Platform::Windows).unwrap(), &rn).unwrap();
        let q = query();
        assert!(q.success(), "task not found after creating it ({run_as:?}): {q:?}");
        assert!(q.stdout.contains("update --auto") && q.stdout.contains("C:\\kora-smoke\\task"), "task command line: {}", q.stdout);
        assert!(q.stdout.contains("04:00:00") && q.stdout.contains("StartWhenAvailable"), "daily 04:00 trigger: {}", q.stdout);
        let back = get_with(&s, std::path::Path::new("C:\\kora-smoke\\none"), &rn, Platform::Windows).expect("task read back");
        assert_eq!((back.run_as, back.time.as_str()), (run_as, "04:00"), "{}", q.stdout);
        apply_plan(&scheduler_uninstall_plan(&s, Platform::Windows).unwrap(), &rn).unwrap();
        assert!(!query().success(), "task is still there after removing it ({run_as:?})");
    }
}

#[test]
fn task_xml_is_written_as_utf16_le_with_a_bom_as_schtasks_requires() {
    let xml = task_xml(&server(), &Schedule::default(), RunAs::System, "").unwrap();
    assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-16\"?>"));
    let b = super::file_bytes(&xml);
    assert_eq!(&b[..2], &[0xFF, 0xFE], "byte-order mark");
    let units: Vec<u16> = b[2..].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    assert_eq!(String::from_utf16(&units).unwrap().replace("\r\n", "\n"), xml, "round-trips, non-ASCII included");
    let u = task_xml(&server(), &Schedule::default(), RunAs::InstallingUser, "PC-ÖZ\\Zoë").unwrap();
    let b = super::file_bytes(&u);
    let units: Vec<u16> = b[2..].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    assert!(String::from_utf16(&units).unwrap().contains("PC-ÖZ\\Zoë"));
}

#[test]
fn other_files_stay_utf8() {
    assert_eq!(super::file_bytes("[Unit]\nDescription=x\n"), b"[Unit]\nDescription=x\n");
    assert_eq!(super::file_bytes("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<a/>"), "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<a/>".as_bytes());
}

#[test]
fn a_folder_with_spaces_cannot_widen_the_units_write_access() {
    // ReadWritePaths= is space separated: "/srv/a /etc" would make /etc writable for the service.
    for bad in ["/srv/a /etc", "/srv/a\t/etc", "/srv/%h", "/srv/a\"b", "/srv/$x"] {
        let mut s = server();
        s.dir = bad.into();
        assert!(systemd_unit(&s).is_err(), "{bad}");
    }
    let mut s = server();
    s.env_file = Some("/srv/x/env /etc/shadow".into());
    assert!(systemd_unit(&s).is_err());
}

#[test]
fn a_quote_in_the_folder_cannot_add_arguments_to_the_windows_service_command() {
    let mut s = server();
    s.dir = "C:\\srv\" --evil \"x".into();
    assert!(install_plan(&s, Platform::Windows).is_err());
}

#[test]
#[cfg(unix)]
fn an_existing_file_is_locked_down_before_the_contents_go_in() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("task.xml");
    std::fs::write(&p, "old").unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o666)).unwrap();
    SystemRunner.write_file(p.to_str().unwrap(), "secret", Some(0o600)).unwrap();
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "secret");
    assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
    // Without a mode the file is just written (as before).
    let q = d.path().join("plain.txt");
    SystemRunner.write_file(q.to_str().unwrap(), "x", None).unwrap();
    assert_eq!(std::fs::read_to_string(&q).unwrap(), "x");
}

#[cfg(unix)]
#[test]
fn a_forged_binary_in_created_json_never_becomes_the_root_update_job() {
    // `created.json` is in the server folder, which the service account and the panel's user can
    // write: a program named there must not end up in the ExecStart= of the root update unit.
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("srv");
    std::fs::create_dir_all(dir.join(".kora")).unwrap();
    std::fs::write(dir.join(".kora/created.json"), r#"{"created_dir":false,"resources":[],"artifacts":false,"files":[],"service_unit":null,"firewall":false,"registered":true,"binary":"/tmp/evil"}"#).unwrap();
    let rec = crate::installer::registry::ServerEntry {
        name: "main".into(),
        dir: dir.clone(),
        mode: crate::installer::Mode::Production,
        ports: crate::installer::registry::PortsRecord { game: 30120, txadmin: 40120 },
        created_unix: 0,
    };
    let s = managed_for(&rec);
    assert_ne!(s.kora_exe, "/tmp/evil");
    // This test program itself when it is root-owned all the way down, else a pinned root-owned copy.
    let me = std::env::current_exe().unwrap();
    if super::binary::exe_is_service_safe(&me) {
        assert_eq!(s.kora_exe, me.to_string_lossy());
    } else {
        assert!(s.kora_exe.starts_with("/opt/kora/"), "{}", s.kora_exe);
    }
    let unit = systemd_update_unit(&s).unwrap();
    assert!(!unit.contains("/tmp/evil"), "{unit}");
}

#[test]
fn the_installing_users_update_task_runs_without_elevation_and_may_restart_only_its_service() {
    let mut s = server();
    s.dir = "C:\\Kora\\main".into();
    let xml = task_xml(&s, &Schedule::Daily(TimeOfDay::new(4, 0).unwrap()), RunAs::InstallingUser, "PC\\jo").unwrap();
    assert!(xml.contains("<RunLevel>LeastPrivilege</RunLevel>") && !xml.contains("HighestAvailable"), "{xml}");
    let me = "S-1-5-21-1-2-3-1001";
    let d = windows_user_control_sddl(me).unwrap();
    assert!(d.ends_with(&format!("(A;;CCLCSWRPWPDTLOCRRC;;;{me})")), "query, start, stop: {d}");
    assert!(!d.contains(&format!("WDWO;;;{me}")) && !d.contains(&format!("SD;;;{me}")), "never reconfigure or delete");
    assert!(windows_user_control_sddl("S-1-5-32-544").is_none() && windows_user_control_sddl("S-1-5-21-1)(A;;GA;;;WD").is_none());
    let lines: Vec<_> = windows_register_plan_for(&s, Some(me)).steps.iter().filter_map(|s| s.command_line()).collect();
    assert!(lines.last().unwrap().starts_with("sc.exe sdset kora-main D:"), "{lines:?}");
    assert_eq!(windows_register_plan_for(&s, None).steps.len(), windows_register_plan(&s).steps.len());
    assert_eq!(parse_whoami_sid("\"pc\\jo\",\"S-1-5-21-1-2-3-1001\"\r\n").as_deref(), Some(me));
    assert_eq!(parse_whoami_sid("garbage"), None);
}

#[test]
fn a_retried_service_step_adopts_only_kora_s_own_service_for_the_same_folder() {
    let qc = "[SC] QueryServiceConfig SUCCESS\r\n\r\nSERVICE_NAME: kora-main\r\n        TYPE               : 10  WIN32_OWN_PROCESS\r\n        BINARY_PATH_NAME   : \"C:\\Program Files\\Kora\\bin\\0.7.0-abc\\kora.exe\" service-host \"C:\\Users\\jo\\kora\\main\"\r\n";
    let b = parse_qc_binpath(qc).unwrap();
    assert!(binpath_is_for(&b, "C:\\Users\\jo\\kora\\main"));
    assert!(binpath_is_for(&b, "c:\\users\\jo\\kora\\main\\"), "case and trailing slash");
    assert!(!binpath_is_for(&b, "C:\\Users\\other\\kora\\main"));
    assert!(!binpath_is_for("\"C:\\evil\\x.exe\" service-host \"C:\\Users\\jo\\kora\\main\"", "C:\\Users\\jo\\kora\\main"));
    assert!(!binpath_is_for("C:\\Windows\\system32\\svchost.exe -k netsvcs", "C:\\Users\\jo\\kora\\main"));
    assert_eq!(parse_qc_binpath("nothing"), None);
}
