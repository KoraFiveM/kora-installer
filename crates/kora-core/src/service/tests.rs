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
    ServiceManager::new(Platform::Windows, &r).restart(&server()).unwrap();
    assert_eq!(r.calls(), vec!["sc.exe stop kora-main", "sc.exe start kora-main"]);
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
    let p = scheduler_install_plan(&s, &Schedule::default(), Platform::Windows).unwrap();
    let Action::Run { args, .. } = &p.steps[0].action else { panic!() };
    assert_eq!(args[4], "\"C:\\Program Files\\Kora\\bin\\0.1.0-abc\\kora.exe\" update --auto \"C:\\Users\\Zoë Müller\\kora\\main\"");
}

#[test]
fn schtasks_args() {
    let p = scheduler_install_plan(&server(), &Schedule::default(), Platform::Windows).unwrap();
    let Action::Run { program, args } = &p.steps[0].action else { panic!() };
    assert_eq!(program, "schtasks.exe");
    let expect: Vec<String> = [
        "/Create", "/TN", "Kora\\Update main", "/TR", "\"/usr/local/bin/kora\" update --auto \"/srv/kora/main\"",
        "/SC", "DAILY", "/ST", "04:00", "/RU", "SYSTEM", "/RL", "HIGHEST", "/F",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(args, &expect);
    let w = scheduler_install_plan(&server(), &Schedule::Custom(vec![Weekday::Mon, Weekday::Fri], TimeOfDay::new(23, 15).unwrap()), Platform::Windows).unwrap();
    let line = w.steps[0].command_line().unwrap();
    assert!(line.contains("/SC WEEKLY /D MON,FRI /ST 23:15"));
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
    // a server without a database has no database.json: that grant must not abort the install
    assert!(p.steps.iter().filter(|st| st.command_line().is_some_and(|l| l.contains("database.json") || l.contains("secrets.cfg"))).all(|st| st.ignore_failure));
}
