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
fn schtasks_args() {
    let p = scheduler_install_plan(&server(), &Schedule::default(), Platform::Windows).unwrap();
    let Action::Run { program, args } = &p.steps[0].action else { panic!() };
    assert_eq!(program, "schtasks.exe");
    let expect: Vec<String> = [
        "/Create", "/TN", "Kora\\Update main", "/TR", "\"/usr/local/bin/kora\" update --auto main",
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
