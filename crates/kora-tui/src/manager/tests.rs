use std::path::PathBuf;

use crossterm::event::KeyCode;
use kora_core::installer::Mode;

use kora_core::notify::NotifyConfig;
use kora_core::service::{Preset, RunAs, ScheduleConfig};
use kora_core::update::manual::*;
use kora_core::update::{DeferReason, UpdateReport, Updated};
use super::app::*;
use super::backend::{Status, UpdateBadge};
use super::testing::*;

fn two(d: &std::path::Path) -> (App, Shared) {
    let (a, s) = app_with(vec![entry("alpha", &managed_dir(d, "alpha"), 30120), entry("beta", &managed_dir(d, "beta"), 30121)]);
    (a, s)
}

fn notice(a: &App) -> String {
    a.notice.as_ref().map(|n| n.text.clone()).unwrap_or_default()
}

// ------------------------------------------------------------ list

#[test]
fn empty_state_offers_install_and_add() {
    let (mut a, _s) = app_with(vec![]);
    let t = screen_text(&a);
    assert!(t.contains("No servers yet - install one"), "{t}");
    assert!(t.contains("Install a new server") && t.contains("Add an existing server"));
    click(&mut a, AREA, Hit::Act(Action::InstallNew));
    assert_eq!(a.exit, Some(Exit::InstallNew));
}

#[test]
fn empty_state_add_existing_opens_the_add_screen() {
    let (mut a, _s) = app_with(vec![]);
    click(&mut a, AREA, Hit::Act(Action::Add));
    assert!(matches!(a.screen, Screen::Add(_)));
    assert!(screen_text(&a).contains("Install a new server"));
}

#[test]
fn list_shows_name_status_badge_and_detail() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = two(d.path());
    {
        let mut f = s.borrow_mut();
        f.status.insert("alpha".into(), Status::Running);
        f.badges.insert("alpha".into(), UpdateBadge::Updates(3));
        f.badges.insert("beta".into(), UpdateBadge::NeedsAttention);
    }
    a.reload();
    let t = screen_text(&a);
    assert!(t.contains("alpha") && t.contains("beta"), "{t}");
    assert!(t.contains("● running") && t.contains("○ stopped"), "{t}");
    assert!(t.contains("3 updates") && t.contains("needs attention"), "{t}");
    assert!(t.contains("port 30120") && t.contains("127.0.0.1:40120") && t.contains("production"), "{t}");
    assert!(t.contains(d.path().join("alpha").to_str().unwrap()), "{t}");
    chr(&mut a, 'j');
    assert_eq!(a.sel, 1);
    assert!(screen_text(&a).contains("port 30121"));
    chr(&mut a, 'k');
    key(&mut a, KeyCode::End);
    assert_eq!(a.sel, 1);
    key(&mut a, KeyCode::Home);
    assert_eq!(a.sel, 0);
    key(&mut a, KeyCode::Up);
    assert_eq!(a.sel, 0, "stays on the first row");
}

#[test]
fn every_badge_has_its_text() {
    assert_eq!(UpdateBadge::UpToDate.text(), "up to date");
    assert_eq!(UpdateBadge::Updates(1).text(), "1 update");
    assert_eq!(UpdateBadge::Updates(3).text(), "3 updates");
    assert_eq!(UpdateBadge::NeedsAttention.text(), "needs attention");
    assert_eq!(UpdateBadge::NewRelease.text(), "new release");
}

#[test]
fn registry_error_is_shown_with_a_retry() {
    let (mut a, s) = app_with(vec![]);
    s.borrow_mut().list_error = Some("Can't read the server list: boom".into());
    a.reload();
    let t = screen_text(&a);
    assert!(t.contains("Can't read the server list: boom") && !t.contains("No servers yet"), "{t}");
    s.borrow_mut().list_error = None;
    click(&mut a, AREA, Hit::Act(Action::Refresh));
    assert!(screen_text(&a).contains("No servers yet"));
}

#[test]
fn missing_folder_is_flagged_and_cannot_be_started() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = app_with(vec![entry("gone", &d.path().join("nope"), 30120)]);
    s.borrow_mut().status.insert("gone".into(), Status::Missing);
    a.reload();
    assert!(screen_text(&a).contains("! missing"));
    assert!(!clickable(&a, AREA, Hit::Act(Action::Start)));
    chr(&mut a, 's');
    assert!(s.borrow().calls.is_empty());
    assert!(notice(&a).contains("missing"));
}

#[test]
fn long_lists_scroll_to_keep_the_selection_visible() {
    let d = tempfile::tempdir().unwrap();
    let servers: Vec<_> = (0..40).map(|i| entry(&format!("srv{i:02}"), &managed_dir(d.path(), &format!("s{i}")), 30000 + i)).collect();
    let (mut a, _s) = app_with(servers);
    key(&mut a, KeyCode::End);
    assert_eq!(a.sel, 39);
    assert!(screen_text(&a).contains("srv39"));
    for r in super::view::hits(&a, AREA).iter().filter_map(|(h, r)| matches!(h, Hit::Row(_)).then_some(r)) {
        assert!(r.bottom() <= AREA.height);
    }
}

// ------------------------------------------------------------ service actions

#[test]
fn start_stop_restart_follow_the_live_state() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = two(d.path());
    assert!(clickable(&a, AREA, Hit::Act(Action::Start)));
    assert!(!clickable(&a, AREA, Hit::Act(Action::Stop)) && !clickable(&a, AREA, Hit::Act(Action::Restart)));
    chr(&mut a, 's');
    assert_eq!(s.borrow().calls, ["Start alpha"]);
    assert!(notice(&a).contains("'alpha' started"));
    assert!(screen_text(&a).contains("● running"));
    assert!(clickable(&a, AREA, Hit::Act(Action::Stop)) && !clickable(&a, AREA, Hit::Act(Action::Start)));
    click(&mut a, AREA, Hit::Act(Action::Restart));
    click(&mut a, AREA, Hit::Act(Action::Stop));
    assert_eq!(s.borrow().calls, ["Start alpha", "Restart alpha", "Stop alpha"]);
    assert!(screen_text(&a).contains("○ stopped"));
}

#[test]
fn busy_text_is_drawn_before_the_slow_job_runs() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = two(d.path());
    a.handle_key(crossterm::event::KeyEvent::new(KeyCode::Char('s'), crossterm::event::KeyModifiers::NONE));
    assert!(screen_text(&a).contains("Starting the server"), "{}", screen_text(&a));
    assert!(s.borrow().calls.is_empty(), "nothing ran yet");
    a.handle_key(crossterm::event::KeyEvent::new(KeyCode::Char('x'), crossterm::event::KeyModifiers::NONE));
    assert_eq!(a.busy, Some(Job::Control(ControlActionKey::Start)), "input is ignored while working");
    assert!(a.run_pending());
    assert_eq!(s.borrow().calls, ["Start alpha"]);
    assert!(!a.run_pending());
}

#[test]
fn service_errors_are_shown_not_swallowed() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = two(d.path());
    s.borrow_mut().control_error = Some("'alpha' has no background service.".into());
    chr(&mut a, 's');
    assert_eq!(a.notice.as_ref().unwrap().level, Level::Bad);
    assert!(screen_text(&a).contains("has no background service"));
}

#[test]
fn open_txadmin_and_folder() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = two(d.path());
    chr(&mut a, 't');
    click(&mut a, AREA, Hit::Act(Action::OpenFolder));
    let calls = s.borrow().calls.clone();
    assert_eq!(calls[0], "url http://127.0.0.1:40120");
    assert_eq!(calls[1], format!("folder {}", d.path().join("alpha").display()));
    s.borrow_mut().launch_error = Some("no browser".into());
    chr(&mut a, 't');
    assert!(notice(&a).contains("no browser") && notice(&a).contains("http://127.0.0.1:40120"), "the address is given when it can't be opened");
}

// ------------------------------------------------------------ logs

fn with_logs(d: &std::path::Path) -> (App, Shared) {
    let dir = managed_dir(d, "alpha");
    let body: String = (1..=100).map(|i| format!("line {i}\n")).collect();
    std::fs::write(dir.join(".kora/service-host.log"), body).unwrap();
    std::fs::write(dir.join("update.log"), "updated x\n").unwrap();
    app_with(vec![entry("alpha", &dir, 30120)])
}

#[test]
fn logs_show_tail_scroll_switch_and_copy() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = with_logs(d.path());
    chr(&mut a, 'l');
    let t = screen_text(&a);
    assert!(t.contains("service-host.log") && t.contains("update.log") && t.contains("line 100") && t.contains("(end)"), "{t}");
    assert!(!t.contains("line 1\n"));
    key(&mut a, KeyCode::PageUp);
    let t = screen_text(&a);
    assert!(!t.contains("line 100") && t.contains("line 90"), "{t}");
    wheel(&mut a, AREA, true);
    wheel(&mut a, AREA, false);
    key(&mut a, KeyCode::Home);
    assert!(screen_text(&a).contains("line 1 of 100"), "{}", screen_text(&a));
    key(&mut a, KeyCode::End);
    click(&mut a, AREA, Hit::Act(Action::Copy));
    let clip = s.borrow().clip.clone().unwrap();
    assert!(clip.starts_with("line 1\n") && clip.ends_with("line 100"));
    assert!(notice(&a).contains("Copied 100 lines"));
    key(&mut a, KeyCode::Tab);
    assert!(screen_text(&a).contains("updated x"));
    click(&mut a, AREA, Hit::Act(Action::Back));
    assert!(matches!(a.screen, Screen::Main));
}

#[test]
fn logs_without_files_explain_themselves_and_cannot_copy() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, _s) = app_with(vec![entry("alpha", &managed_dir(d.path(), "alpha"), 30120)]);
    chr(&mut a, 'l');
    assert!(screen_text(&a).contains("No log files yet"));
    assert!(!clickable(&a, AREA, Hit::Act(Action::Copy)));
    key(&mut a, KeyCode::Esc);
    assert!(matches!(a.screen, Screen::Main));
}

#[test]
fn clipboard_failure_is_reported() {
    struct NoClip;
    impl crate::clipboard::Clipboard for NoClip {
        fn get_text(&mut self) -> Result<String, String> {
            Err("no".into())
        }
        fn set_text(&mut self, _: &str) -> Result<(), String> {
            Err("The clipboard isn't available here.".into())
        }
    }
    let d = tempfile::tempdir().unwrap();
    let (mut a, _s) = with_logs(d.path());
    a.svc.clipboard = Box::new(NoClip);
    chr(&mut a, 'l');
    chr(&mut a, 'c');
    assert_eq!(a.notice.as_ref().unwrap().level, Level::Bad);
}

// ------------------------------------------------------------ ensure

#[test]
fn ensure_by_typing_with_and_without_restart() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = two(d.path());
    chr(&mut a, 'e');
    key(&mut a, KeyCode::Enter);
    assert!(notice(&a).contains("Type or pick"), "empty name is refused");
    typed(&mut a, "kora_core");
    key(&mut a, KeyCode::Enter);
    assert_eq!(s.borrow().calls, ["ensure alpha kora_core restart=true"]);
    assert!(matches!(a.screen, Screen::Main));
    assert!(notice(&a).contains("ensured kora_core"));
    chr(&mut a, 'e');
    key(&mut a, KeyCode::Tab);
    typed(&mut a, "mapx");
    key(&mut a, KeyCode::Enter);
    assert_eq!(s.borrow().calls[1], "ensure alpha mapx restart=false");
}

#[test]
fn ensure_can_pick_a_known_resource_and_filters_as_you_type() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = two(d.path());
    s.borrow_mut().resources = vec!["kora_core".into(), "kora_hud".into(), "ox_lib".into()];
    chr(&mut a, 'e');
    let t = screen_text(&a);
    assert!(t.contains("kora_core") && t.contains("ox_lib"), "{t}");
    typed(&mut a, "hud");
    let t = screen_text(&a);
    assert!(t.contains("kora_hud") && !t.contains("ox_lib"), "{t}");
    key(&mut a, KeyCode::Down);
    key(&mut a, KeyCode::Enter);
    assert_eq!(s.borrow().calls, ["ensure alpha kora_hud restart=true"]);
    // click a row to fill the box
    chr(&mut a, 'e');
    click(&mut a, AREA, Hit::Row(2));
    assert!(screen_text(&a).contains("Resource name: ox_lib"), "{}", screen_text(&a));
}

#[test]
fn ensure_error_keeps_the_screen_open() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = two(d.path());
    s.borrow_mut().ensure_error = Some("'a b' is not a resource name".into());
    chr(&mut a, 'e');
    typed(&mut a, "a b");
    key(&mut a, KeyCode::Enter);
    assert!(matches!(a.screen, Screen::Ensure(_)));
    assert!(screen_text(&a).contains("is not a resource name"));
}

#[test]
fn paste_into_a_field_drops_newlines() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = two(d.path());
    chr(&mut a, 'e');
    a.handle_paste("my_res\r\nsecond line");
    key(&mut a, KeyCode::Enter);
    assert_eq!(s.borrow().calls, ["ensure alpha my_ressecond line restart=true"].map(String::from), "one line, no control characters");
    // paste on the list screen is ignored
    a.handle_paste("whatever");
    assert!(matches!(a.screen, Screen::Main));
}

// ------------------------------------------------------------ delete

#[test]
fn delete_asks_per_item_then_the_name() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = two(d.path());
    chr(&mut a, 'd');
    assert!(screen_text(&a).contains("Delete the server folder"));
    chr(&mut a, 'n'); // keep files
    assert!(screen_text(&a).contains("Files: kept") && screen_text(&a).contains("database"));
    chr(&mut a, 'y'); // delete database
    let t = screen_text(&a);
    assert!(t.contains("Type the server name to confirm: alpha") && t.contains("Database: will be deleted"), "{t}");
    typed(&mut a, "alph");
    key(&mut a, KeyCode::Enter);
    assert!(notice(&a).contains("Type exactly: alpha"));
    assert!(s.borrow().calls.is_empty(), "nothing deleted with a wrong name");
    chr(&mut a, 'a');
    key(&mut a, KeyCode::Enter);
    assert_eq!(s.borrow().calls, ["delete alpha keep_files=true keep_db=false"]);
    assert!(matches!(a.screen, Screen::Main));
    assert_eq!(a.rows.len(), 1);
    assert!(notice(&a).contains("Deleted: 'alpha'") && notice(&a).contains("note one"));
}

#[test]
fn delete_can_be_cancelled_at_every_step_and_by_mouse() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = two(d.path());
    chr(&mut a, 'd');
    key(&mut a, KeyCode::Esc);
    assert!(matches!(a.screen, Screen::Main));
    chr(&mut a, 'd');
    click(&mut a, AREA, Hit::Act(Action::Yes));
    click(&mut a, AREA, Hit::Act(Action::No));
    click(&mut a, AREA, Hit::Act(Action::Back));
    assert!(matches!(a.screen, Screen::Main));
    assert!(s.borrow().calls.is_empty() && a.rows.len() == 2);
}

#[test]
fn a_running_server_is_not_deleted() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = two(d.path());
    s.borrow_mut().status.insert("alpha".into(), Status::Running);
    a.reload();
    chr(&mut a, 'd');
    assert!(matches!(a.screen, Screen::Main));
    assert!(notice(&a).contains("Stop the server first"));
}

#[test]
fn delete_failure_is_reported_and_the_list_reloaded() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = two(d.path());
    s.borrow_mut().delete_error = Some("database is locked".into());
    chr(&mut a, 'd');
    chr(&mut a, 'y');
    chr(&mut a, 'y');
    typed(&mut a, "alpha");
    key(&mut a, KeyCode::Enter);
    assert!(notice(&a).contains("Delete did not finish: database is locked"));
    assert!(matches!(a.screen, Screen::Main));
}

#[test]
fn an_adopted_server_is_only_taken_off_the_list() {
    let d = tempfile::tempdir().unwrap();
    let plain = d.path().join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let (mut a, s) = app_with(vec![entry("plain", &plain, 30120)]);
    assert!(screen_text(&a).contains("Remove from list"));
    chr(&mut a, 'd');
    let t = screen_text(&a);
    assert!(t.contains("nothing is deleted"), "{t}");
    chr(&mut a, 'y');
    assert_eq!(s.borrow().calls, ["unregister plain"]);
    assert!(plain.is_dir());
    assert!(screen_text(&a).contains("No servers yet"));
}

// ------------------------------------------------------------ add

#[test]
fn add_menu_launches_the_install_wizard() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, _s) = two(d.path());
    chr(&mut a, 'a');
    chr(&mut a, 'i');
    assert_eq!(a.exit, Some(Exit::InstallNew));
}

#[test]
fn add_existing_scans_and_registers_read_only() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("srv");
    let found = root.join("my fx");
    std::fs::create_dir_all(found.join("txData")).unwrap();
    std::fs::write(found.join("server.cfg"), "endpoint_add_tcp \"0.0.0.0:30555\"\n").unwrap();
    std::fs::create_dir_all(root.join("already/.kora")).unwrap();
    let before: Vec<_> = std::fs::read_dir(&found).unwrap().flatten().map(|e| e.file_name()).collect();
    let (mut a, s) = app_with(vec![entry("already", &root.join("already"), 30120)]);
    s.borrow_mut().roots = vec![root.clone()];
    chr(&mut a, 'a');
    chr(&mut a, 'e');
    let t = screen_text(&a);
    assert!(t.contains("Found 1 folder") && t.contains("my fx") && t.contains("FXServer folder"), "{t}");
    assert!(!t.contains("already  "), "registered folders are not offered");
    key(&mut a, KeyCode::Enter);
    assert!(matches!(a.screen, Screen::Main));
    assert_eq!(s.borrow().calls, ["register my-fx"]);
    let e = &a.current().unwrap().entry;
    assert_eq!((e.name.as_str(), e.ports.game, e.mode), ("my-fx", 30555, Mode::Production));
    assert!(notice(&a).contains("Nothing in its folder was changed"));
    let after: Vec<_> = std::fs::read_dir(&found).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(before.len(), after.len(), "the folder is untouched");
}

#[test]
fn add_existing_with_nothing_found_falls_back_to_typing() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = app_with(vec![]);
    s.borrow_mut().roots = vec![d.path().to_path_buf()];
    chr(&mut a, 'a');
    click(&mut a, AREA, Hit::Act(Action::AddExisting));
    assert!(screen_text(&a).contains("No server folders found"));
    assert!(!clickable(&a, AREA, Hit::Act(Action::Confirm)));
    key(&mut a, KeyCode::Enter);
    assert!(screen_text(&a).contains("Folder:"), "Enter with nothing found goes to typing a path");
}

#[test]
fn add_by_typing_a_folder_checks_it() {
    let d = tempfile::tempdir().unwrap();
    let good = d.path().join("good");
    std::fs::create_dir_all(good.join(".kora")).unwrap();
    let junk = d.path().join("junk");
    std::fs::create_dir_all(&junk).unwrap();
    let (mut a, s) = app_with(vec![]);
    chr(&mut a, 'a');
    chr(&mut a, 'p');
    key(&mut a, KeyCode::Enter);
    assert!(notice(&a).contains("Type or paste"));
    a.handle_paste("relative/dir");
    key(&mut a, KeyCode::Enter);
    assert!(notice(&a).contains("full folder path"));
    for _ in 0..12 {
        key(&mut a, KeyCode::Backspace);
    }
    a.handle_paste(&format!("{}\n", d.path().join("nope").display()));
    key(&mut a, KeyCode::Enter);
    assert!(notice(&a).contains("is not a folder"));
    a.handle_paste("x");
    while let Screen::Add(s) = &mut a.screen {
        if s.input.is_empty() {
            break;
        }
        key(&mut a, KeyCode::Backspace);
    }
    a.handle_paste(&junk.display().to_string());
    key(&mut a, KeyCode::Enter);
    assert!(notice(&a).contains("doesn't look like"));
    assert!(s.borrow().calls.is_empty());
    while let Screen::Add(s) = &mut a.screen {
        if s.input.is_empty() {
            break;
        }
        key(&mut a, KeyCode::Backspace);
    }
    a.handle_paste(&good.display().to_string());
    key(&mut a, KeyCode::Enter);
    assert_eq!(s.borrow().calls, ["register good"]);
    // adding it again says so
    chr(&mut a, 'a');
    chr(&mut a, 'p');
    a.handle_paste(&good.display().to_string());
    key(&mut a, KeyCode::Enter);
    assert!(notice(&a).contains("already in the list"));
    key(&mut a, KeyCode::Esc);
    assert!(matches!(a.screen, Screen::Add(_)), "Esc goes back one step first");
    key(&mut a, KeyCode::Esc);
    assert!(matches!(a.screen, Screen::Main));
}

// ------------------------------------------------------------ mouse

#[test]
fn mouse_clicks_select_rows_and_press_buttons() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = two(d.path());
    click(&mut a, AREA, Hit::Row(1));
    assert_eq!(a.sel, 1);
    click(&mut a, AREA, Hit::Act(Action::Start));
    assert_eq!(s.borrow().calls, ["Start beta"]);
    click_at(&mut a, AREA, 118, 28);
    click_at(&mut a, AREA, 0, 0);
    assert_eq!(a.sel, 1, "clicks on nothing do nothing");
    wheel(&mut a, AREA, true);
    assert_eq!(a.sel, 0);
    wheel(&mut a, AREA, false);
    assert_eq!(a.sel, 1);
}

#[test]
fn hit_rectangles_stay_inside_the_screen_for_any_size() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, _s) = two(d.path());
    for (w, h) in [(120u16, 32u16), (80, 24), (60, 20), (40, 12), (20, 6)] {
        let area = ratatui::layout::Rect::new(0, 0, w, h);
        for (_, r) in super::view::hits(&a, area) {
            assert!(r.right() <= w && r.bottom() <= h && r.width > 0, "{r:?} outside {w}x{h}");
        }
        let _ = text(&a, w, h);
    }
    // narrow: list above details, the Start button still works
    let narrow = ratatui::layout::Rect::new(0, 0, 70, 26);
    click(&mut a, narrow, Hit::Act(Action::Start));
    assert_eq!(a.current().unwrap().status, Status::Running);
}

#[test]
fn every_screen_draws_on_a_tiny_terminal_without_panicking() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = with_logs(d.path());
    s.borrow_mut().preview = Some(Ok(UpdatePreview { resources: vec![change("r1", ChangeStatus::Clean)], new_release: None }));
    for k in ['l', 'e', 'd', 'u', 'n', 'c', 'a'] {
        chr(&mut a, k);
        for (w, h) in [(10u16, 4u16), (30, 8), (100, 40)] {
            let _ = text(&a, w, h);
            let _ = super::view::hits(&a, ratatui::layout::Rect::new(0, 0, w, h));
        }
        key(&mut a, KeyCode::Esc);
        while !matches!(a.screen, Screen::Main) {
            key(&mut a, KeyCode::Esc);
        }
    }
}

// ------------------------------------------------------------ update

fn change(id: &str, st: ChangeStatus) -> ResourceChange {
    ResourceChange { id: id.into(), from: "a1b2c3d".into(), to: "e4f5a6b".into(), behind: 3, changelog: vec!["fix: crash on join".into(), "feat: new hud".into()], needs_restart: id.starts_with("kora_core"), status: st }
}

fn preview(resources: Vec<ResourceChange>) -> UpdatePreview {
    UpdatePreview { resources, new_release: None }
}

fn updates(d: &std::path::Path) -> (App, Shared) {
    let (a, s) = two(d);
    s.borrow_mut().preview = Some(Ok(preview(vec![
        change("kora_core", ChangeStatus::Clean),
        change("kora_hud", ChangeStatus::Clean),
        change("my_edit", ChangeStatus::Dirty),
        change("old", ChangeStatus::Diverged),
    ])));
    (a, s)
}

#[test]
fn update_preview_lists_changes_and_only_clean_ones_are_ticked() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = updates(d.path());
    s.borrow_mut().preview.as_mut().unwrap().as_mut().unwrap().resources.push(change("fresh", ChangeStatus::UpToDate));
    chr(&mut a, 'u');
    let t = screen_text(&a);
    assert!(t.contains("2 of 2 updatable"), "{t}");
    assert!(t.contains("Resource") && t.contains("From → to") && t.contains("Status") && t.contains("Restart"), "table header: {t}");
    assert!(t.contains("[x] kora_core") && t.contains("a1b2c3d → e4f5a6b") && t.contains("3 behind") && t.contains("needs restart"), "{t}");
    assert!(t.contains("[-] my_edit") && t.contains("Dirty") && t.contains("[-] old") && t.contains("Diverged") && t.contains("Up to date"), "{t}");
    assert!(t.contains("Changes in kora_core") && t.contains("fix: crash on join"), "changelog of the highlighted one: {t}");
    key(&mut a, KeyCode::Down);
    key(&mut a, KeyCode::Down);
    assert!(screen_text(&a).contains("Changes in my_edit"), "the changelog follows the highlight");
    key(&mut a, KeyCode::Up);
    key(&mut a, KeyCode::Up);
    chr(&mut a, ' ');
    assert!(screen_text(&a).contains("[ ] kora_core"));
    chr(&mut a, ' ');
    key(&mut a, KeyCode::Down);
    key(&mut a, KeyCode::Down);
    chr(&mut a, ' ');
    assert!(screen_text(&a).contains("[-] my_edit"), "a dirty resource can't be ticked");
    chr(&mut a, 'a');
    assert!(screen_text(&a).contains("0 of 2"), "toggle all: everything ticked -> none");
    chr(&mut a, 'a');
    assert!(screen_text(&a).contains("2 of 2"), "then all the clean ones");
}

#[test]
fn update_shows_a_new_release_and_the_restart_policies_cycle() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = updates(d.path());
    s.borrow_mut().preview.as_mut().unwrap().as_mut().unwrap().new_release = Some("1.4.0".into());
    chr(&mut a, 'u');
    assert!(screen_text(&a).contains("new Kora release is available: 1.4.0"));
    let mut seen = vec![];
    for _ in 0..5 {
        let t = screen_text(&a);
        seen.push(["nobody is playing", "no full restart", "whole server", "Don't restart"].iter().position(|x| t.contains(x)));
        chr(&mut a, 'r');
    }
    assert_eq!(seen, [Some(0), Some(1), Some(2), Some(3), Some(0)], "when quiet -> ensure -> full -> none -> when quiet");
}

#[test]
fn update_all_ticks_everything_and_applies_with_the_chosen_policy() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = updates(d.path());
    chr(&mut a, 'u');
    chr(&mut a, 'r'); // ensure
    chr(&mut a, 'r'); // full restart
    chr(&mut a, 'a'); // none
    click(&mut a, AREA, Hit::Act(Action::ToggleAll)); // all again
    click(&mut a, AREA, Hit::Act(Action::ApplySelected));
    assert_eq!(s.borrow().calls, ["apply alpha Resources([\"kora_core\", \"kora_hud\"]) restart=FullRestart"]);
}

fn report_ok() -> UpdateReport {
    UpdateReport {
        server: "alpha".into(),
        updated: vec![Updated { id: "kora_core".into(), from: "a1".into(), to: "b2".into() }, Updated { id: "kora_hud".into(), from: "a1".into(), to: "b2".into() }],
        deferred: vec![("kora_chat".into(), DeferReason::PlayersOnline(3))],
        ..UpdateReport::default()
    }
}

#[test]
fn update_streams_fine_events_live_and_ends_on_a_report() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = updates(d.path());
    s.borrow_mut().apply_events = vec![
        UpdateEvent::Snapshot("1700000000-0".into()),
        UpdateEvent::Resource { id: "kora_core".into(), state: "updating".into() },
        UpdateEvent::Resource { id: "kora_hud".into(), state: "updating".into() },
        UpdateEvent::Resource { id: "kora_core".into(), state: "updated".into() },
        UpdateEvent::Resource { id: "kora_hud".into(), state: "updated".into() },
        UpdateEvent::Health("server is healthy".into()),
        UpdateEvent::Done,
    ];
    s.borrow_mut().apply = Some(Ok(report_ok()));
    chr(&mut a, 'u');
    key(&mut a, KeyCode::Enter);
    let frames = s.borrow().frames.clone();
    assert_eq!(frames[0].rows, [("kora_core".to_string(), "waiting".to_string()), ("kora_hud".to_string(), "waiting".to_string())], "first frame: everything waiting");
    let states: Vec<_> = frames.iter().map(|f| f.rows.iter().map(|r| r.1.as_str()).collect::<Vec<_>>().join("/")).collect();
    assert!(states.contains(&"updating/waiting".to_string()) && states.contains(&"updated/updating".to_string()) && states.last().unwrap() == "updated/updated", "{states:?}");
    assert_eq!(frames.last().unwrap().snapshot.as_deref(), Some("1700000000-0"));
    assert!(frames.last().unwrap().done);
    // the live page itself draws the rows and the snapshot id
    let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(90, 14)).unwrap();
    t.draw(|f| super::view::render_live(f, &frames[4])).unwrap();
    let shown: String = t.backend().buffer().content().iter().map(|c| c.symbol()).collect();
    assert!(shown.contains("Updating") && shown.contains("kora_core") && shown.contains("updated") && shown.contains("1700000000-0"), "{shown}");
    // the final report
    let t = screen_text(&a);
    assert!(t.contains("2 updated, 0 skipped, 1 waiting, 0 rolled back, 0 need you"), "{t}");
    assert!(t.contains("Snapshot: 1700000000-0") && t.contains("kora_core") && t.contains("server is healthy"), "{t}");
}

#[test]
fn update_handles_coarse_events_too() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = updates(d.path());
    s.borrow_mut().apply_events = vec![UpdateEvent::Resource { id: "kora_core".into(), state: "updated".into() }, UpdateEvent::Done];
    s.borrow_mut().apply = Some(Ok(report_ok()));
    chr(&mut a, 'u');
    key(&mut a, KeyCode::Enter);
    let t = screen_text(&a);
    assert!(t.contains("kora_core") && t.contains("updated") && t.contains("kora_hud") && t.contains("waiting"), "{t}");
    assert!(!clickable(&a, AREA, Hit::Act(Action::Rollback)), "no snapshot id was announced: nothing to roll back to");
}

#[test]
fn a_failed_update_offers_roll_back_with_a_confirmation() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = updates(d.path());
    s.borrow_mut().apply_events = vec![
        UpdateEvent::Snapshot("snap-42".into()),
        UpdateEvent::Resource { id: "kora_core".into(), state: "rolled back: did not start".into() },
        UpdateEvent::Health("server did not come back healthy; went back to the snapshot".into()),
    ];
    let mut r = UpdateReport::default();
    r.rolled_back.push(("kora_core".into(), "did not start".into()));
    r.needs_attention.push(("kora_hud".into(), "conflict".into()));
    s.borrow_mut().apply = Some(Ok(r));
    chr(&mut a, 'u');
    key(&mut a, KeyCode::Enter);
    let t = screen_text(&a);
    assert!(t.contains("Rolled back: kora_core: did not start") && t.contains("Needs you: kora_hud: conflict") && t.contains("snap-42"), "{t}");
    click(&mut a, AREA, Hit::Act(Action::Rollback));
    assert!(screen_text(&a).contains("Roll back to snapshot snap-42?"));
    chr(&mut a, 'n');
    assert!(!screen_text(&a).contains("Roll back to snapshot") && s.borrow().calls.len() == 1, "declined: nothing happened");
    chr(&mut a, 'b');
    key(&mut a, KeyCode::Esc);
    assert!(matches!(a.screen, Screen::Update(_)), "Esc closes the question first");
    chr(&mut a, 'b');
    click(&mut a, AREA, Hit::Act(Action::Yes));
    assert_eq!(s.borrow().calls[1], "rollback alpha snap-42");
    assert!(notice(&a).contains("Rolled back to snapshot snap-42"));
    key(&mut a, KeyCode::Esc);
    assert!(matches!(a.screen, Screen::Main));
}

#[test]
fn snapshots_list_and_roll_back_one() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = updates(d.path());
    chr(&mut a, 'u');
    chr(&mut a, 's');
    assert!(screen_text(&a).contains("No snapshots yet"));
    key(&mut a, KeyCode::Esc);
    assert!(screen_text(&a).contains("Resource"), "Esc goes back to the preview");
    s.borrow_mut().snapshots = vec![
        SnapshotInfo { id: "1700000200-0".into(), created_unix: 1700000200, resources: vec!["kora_core".into(), "a".into(), "b".into(), "c".into()] },
        SnapshotInfo { id: "1700000100-0".into(), created_unix: 1700000100, resources: vec!["kora_hud".into()] },
    ];
    chr(&mut a, 's');
    let t = screen_text(&a);
    assert!(t.contains("1700000200-0") && t.contains("4 resource(s): kora_core, a, b and 1 more") && t.contains("1700000100-0"), "{t}");
    click(&mut a, AREA, Hit::Row(1));
    key(&mut a, KeyCode::Enter);
    assert!(screen_text(&a).contains("Roll back to snapshot 1700000100-0?"));
    chr(&mut a, 'y');
    assert_eq!(s.borrow().calls, ["rollback alpha 1700000100-0"]);
}

#[test]
fn update_with_nothing_ticked_does_not_run() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = updates(d.path());
    chr(&mut a, 'u');
    chr(&mut a, 'a'); // none
    assert!(!clickable(&a, AREA, Hit::Act(Action::ApplySelected)));
    key(&mut a, KeyCode::Enter);
    assert!(notice(&a).contains("Tick at least one"));
    assert!(s.borrow().calls.is_empty());
}

#[test]
fn update_errors_offer_a_retry_and_up_to_date_says_so() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = two(d.path());
    s.borrow_mut().preview = Some(Err("GitHub is not reachable".into()));
    chr(&mut a, 'u');
    let t = screen_text(&a);
    assert!(t.contains("Couldn't check for updates: GitHub is not reachable"), "{t}");
    s.borrow_mut().preview = Some(Ok(preview(vec![])));
    click(&mut a, AREA, Hit::Act(Action::Retry));
    assert!(screen_text(&a).contains("Everything is up to date"));
    s.borrow_mut().preview = Some(Ok(preview(vec![change("r", ChangeStatus::Clean)])));
    s.borrow_mut().apply = Some(Err("disk full".into()));
    key(&mut a, KeyCode::Esc);
    chr(&mut a, 'u');
    key(&mut a, KeyCode::Enter);
    assert!(screen_text(&a).contains("The update did not finish: disk full"));
}

#[test]
fn update_rows_toggle_by_click() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, _s) = updates(d.path());
    chr(&mut a, 'u');
    click(&mut a, AREA, Hit::Row(1));
    assert!(screen_text(&a).contains("[ ] kora_hud"));
    click(&mut a, AREA, Hit::Act(Action::CycleRestart));
    assert!(screen_text(&a).contains("no full restart"));
}

// ------------------------------------------------------------ update badges

#[test]
fn check_for_updates_refreshes_the_badge() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = updates(d.path());
    assert!(a.rows[0].badge.is_none());
    click(&mut a, AREA, Hit::Act(Action::CheckUpdates));
    assert_eq!(s.borrow().calls, ["check alpha"]);
    assert_eq!(a.rows[0].badge, Some(UpdateBadge::NeedsAttention), "a dirty resource needs a person");
    assert!(notice(&a).contains("needs attention"));
    s.borrow_mut().preview = Some(Ok(preview(vec![change("a", ChangeStatus::Clean), change("b", ChangeStatus::Clean)])));
    chr(&mut a, 'v');
    assert_eq!(a.rows[0].badge, Some(UpdateBadge::Updates(2)));
    assert!(screen_text(&a).contains("2 updates"));
    s.borrow_mut().checked = Some(Err("offline".into()));
    chr(&mut a, 'v');
    assert!(notice(&a).contains("Couldn't check 'alpha': offline"));
}

#[test]
fn badges_come_from_the_preview() {
    use super::backend::{badge_of, status_json};
    assert_eq!(badge_of(&preview(vec![])), UpdateBadge::UpToDate);
    assert_eq!(badge_of(&preview(vec![change("a", ChangeStatus::Clean)])), UpdateBadge::Updates(1));
    assert_eq!(badge_of(&preview(vec![change("a", ChangeStatus::Clean), change("b", ChangeStatus::Diverged)])), UpdateBadge::NeedsAttention);
    assert_eq!(badge_of(&UpdatePreview { resources: vec![], new_release: Some("2".into()) }), UpdateBadge::NewRelease);
    let v: serde_json::Value = serde_json::from_str(&status_json(&preview(vec![change("a", ChangeStatus::Clean)]), 77)).unwrap();
    assert_eq!(v, serde_json::json!({"checked_at": 77, "updates": 1, "attention": false, "new_release": false}));
}

// ------------------------------------------------------------ notifications

const HOOK: &str = "https://discord.com/api/webhooks/123456789/SECRETTOKEN_abc-1";

#[test]
fn notifications_validate_mask_save_test_and_remove() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = two(d.path());
    chr(&mut a, 'n');
    assert!(screen_text(&a).contains("Current: not set"));
    assert!(!clickable(&a, AREA, Hit::Act(Action::SendTest)) && !clickable(&a, AREA, Hit::Act(Action::Remove)));
    key(&mut a, KeyCode::Enter);
    assert!(notice(&a).contains("Nothing to save"));
    a.handle_paste("https://example.com/hook");
    let t = screen_text(&a);
    assert!(t.contains("24 characters entered") && !t.contains("example.com"), "a bad address is not echoed: {t}");
    key(&mut a, KeyCode::Enter);
    assert!(notice(&a).contains("not a valid Discord webhook URL") && s.borrow().calls.is_empty(), "{}", notice(&a));
    for _ in 0..30 {
        key(&mut a, KeyCode::Backspace);
    }
    a.handle_paste(&format!("{HOOK}\n"));
    let t = screen_text(&a);
    assert!(t.contains("discord.com/api/webhooks/123456789/***") && t.contains("looks right") && !t.contains("SECRETTOKEN"), "even before saving the token is hidden: {t}");
    key(&mut a, KeyCode::Enter);
    assert_eq!(s.borrow().notify.discord_webhook.as_deref(), Some(HOOK));
    let t = screen_text(&a);
    assert!(t.contains("Current: discord.com/api/webhooks/123456789/***") && !t.contains("SECRETTOKEN"), "the token is never shown after saving: {t}");
    click(&mut a, AREA, Hit::Act(Action::SendTest));
    assert!(notice(&a).contains("Test message sent"));
    s.borrow_mut().notify_test_error = Some("Discord answered with HTTP 404".into());
    key(&mut a, KeyCode::F(2));
    assert!(notice(&a).contains("The test message failed: Discord answered with HTTP 404"));
    key(&mut a, KeyCode::F(3));
    assert_eq!(s.borrow().notify, NotifyConfig::default());
    assert!(screen_text(&a).contains("Current: not set"));
}

#[test]
fn the_paste_button_reads_the_clipboard() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = two(d.path());
    chr(&mut a, 'n');
    click(&mut a, AREA, Hit::Act(Action::Paste));
    assert!(notice(&a).contains("clipboard"), "an empty clipboard is explained: {}", notice(&a));
    s.borrow_mut().clip = Some(format!("  {HOOK}\n"));
    click(&mut a, AREA, Hit::Act(Action::Paste));
    assert!(screen_text(&a).contains("looks right"));
    key(&mut a, KeyCode::Enter);
    assert_eq!(s.borrow().notify.discord_webhook.as_deref(), Some(HOOK));
}

#[test]
fn a_save_failure_is_reported_and_keeps_the_old_state() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = two(d.path());
    s.borrow_mut().notify_save_error = Some("disk is read-only".into());
    chr(&mut a, 'n');
    a.handle_paste(HOOK);
    key(&mut a, KeyCode::Enter);
    assert!(notice(&a).contains("Couldn't save: disk is read-only"));
    assert!(screen_text(&a).contains("Current: not set"));
}

#[test]
fn webhook_helpers() {
    assert!(check_webhook(HOOK).is_ok());
    assert!(check_webhook("https://discord.com/api/webhooks/").is_err());
    assert!(check_webhook("http://discord.com/api/webhooks/1/x").is_err());
    assert!(check_webhook("https://evil.example/https://discord.com/api/webhooks/1/x").is_err());
    assert_eq!(mask_webhook(HOOK), "discord.com/api/webhooks/123456789/***");
    assert_eq!(mask_webhook("weird"), "<webhook url hidden>");
}

// ------------------------------------------------------------ schedule

fn cfg(preset: Preset, time: &str, weekday: Option<u8>, run_as: RunAs) -> ScheduleConfig {
    ScheduleConfig { preset, time: time.into(), weekday, run_as, enabled: true }
}

#[test]
fn schedule_defaults_validates_and_saves() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = two(d.path());
    chr(&mut a, 'c');
    let t = screen_text(&a);
    assert!(t.contains("Now: off") && t.contains("Every day") && t.contains("04:00") && t.contains("As you (recommended)"), "{t}");
    assert!(t.contains("YOUR GitHub") && !t.contains("PRIVATE repositories cannot"), "the tradeoff of the chosen option is explained: {t}");
    assert!(!t.contains("Day:"));
    key(&mut a, KeyCode::Down); // time field
    for _ in 0..5 {
        key(&mut a, KeyCode::Backspace);
    }
    typed(&mut a, "25:99");
    key(&mut a, KeyCode::Enter);
    assert!(notice(&a).contains("HH:MM") && s.borrow().calls.is_empty(), "{}", notice(&a));
    for _ in 0..5 {
        key(&mut a, KeyCode::Backspace);
    }
    typed(&mut a, "03:30");
    key(&mut a, KeyCode::Up); // preset
    key(&mut a, KeyCode::Right); // weekly
    assert!(screen_text(&a).contains("Day:"));
    key(&mut a, KeyCode::Down);
    key(&mut a, KeyCode::Down); // weekday
    key(&mut a, KeyCode::Right);
    key(&mut a, KeyCode::Down); // run as
    key(&mut a, KeyCode::Right);
    let t = screen_text(&a);
    assert!(t.contains("System (the machine account)") && t.contains("PRIVATE repositories cannot be fetched"), "the other tradeoff now: {t}");
    click(&mut a, AREA, Hit::Act(Action::Save));
    assert_eq!(s.borrow().schedule.clone().unwrap(), cfg(Preset::Weekly, "03:30", Some(1), RunAs::System));
    let t = screen_text(&a);
    assert!(t.contains("Now: Every Tuesday at 03:30, as the system account"), "{t}");
    key(&mut a, KeyCode::F(3));
    assert!(s.borrow().schedule.is_none());
    assert!(screen_text(&a).contains("Now: off"));
    key(&mut a, KeyCode::F(3));
    assert!(notice(&a).contains("already off"));
}

#[test]
fn schedule_loads_the_existing_one_and_custom_may_mean_any_day() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = two(d.path());
    s.borrow_mut().schedule = Some(cfg(Preset::Custom, "06:15", None, RunAs::InstallingUser));
    chr(&mut a, 'c');
    let t = screen_text(&a);
    assert!(t.contains("Custom day") && t.contains("06:15") && t.contains("Every day") && t.contains("Now: Every day at 06:15, as you"), "{t}");
    click(&mut a, AREA, Hit::Row(3));
    assert!(screen_text(&a).contains("System"), "clicking a field changes it");
    click(&mut a, AREA, Hit::Act(Action::Save));
    assert_eq!(s.borrow().schedule.clone().unwrap(), cfg(Preset::Custom, "06:15", None, RunAs::System));
    click(&mut a, AREA, Hit::Row(2)); // day: Monday
    click(&mut a, AREA, Hit::Act(Action::Save));
    assert_eq!(s.borrow().schedule.clone().unwrap().weekday, Some(0));
}

#[test]
fn changing_the_schedule_without_permission_hands_over_to_an_elevated_run() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = two(d.path());
    s.borrow_mut().elevation = Some(vec!["schedule".into(), "set".into(), "/srv/alpha".into(), "--daily".into(), "04:00".into(), "--as-user".into()]);
    chr(&mut a, 'c');
    click(&mut a, AREA, Hit::Act(Action::Save));
    assert_eq!(a.exit, Some(Exit::Elevate(vec!["schedule".into(), "set".into(), "/srv/alpha".into(), "--daily".into(), "04:00".into(), "--as-user".into()])));
    assert!(s.borrow().schedule.is_none(), "nothing was changed in-process");
    // removing: only when there is something to remove
    let (mut a, s) = two(d.path());
    s.borrow_mut().schedule = Some(cfg(Preset::Daily, "04:00", None, RunAs::InstallingUser));
    s.borrow_mut().elevation = Some(vec!["schedule".into(), "off".into(), "/srv/alpha".into()]);
    chr(&mut a, 'c');
    key(&mut a, KeyCode::F(3));
    assert_eq!(a.exit, Some(Exit::Elevate(vec!["schedule".into(), "off".into(), "/srv/alpha".into()])));
}

#[test]
fn schedule_time_validation_is_the_cores() {
    use kora_core::service::parse_time;
    for ok in ["00:00", "04:30", "23:59"] {
        assert!(parse_time(ok).is_ok(), "{ok}");
    }
    for bad in ["", "24:00", "4:05", "12:60", "1230", "ab:cd"] {
        assert!(parse_time(bad).is_err(), "{bad}");
    }
}

// ------------------------------------------------------------ misc

#[test]
fn quit_keys_and_control_c() {
    let d = tempfile::tempdir().unwrap();
    for k in [KeyCode::Char('q'), KeyCode::Esc] {
        let (mut a, _s) = two(d.path());
        key(&mut a, k);
        assert_eq!(a.exit, Some(Exit::Quit));
    }
    let (mut a, _s) = two(d.path());
    chr(&mut a, 'l');
    a.handle_key(crossterm::event::KeyEvent::new(KeyCode::Char('c'), crossterm::event::KeyModifiers::CONTROL));
    assert_eq!(a.exit, Some(Exit::Quit), "Ctrl+C leaves from anywhere");
}

#[test]
fn reload_keeps_the_selected_server() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = two(d.path());
    chr(&mut a, 'j');
    s.borrow_mut().servers.insert(0, entry("aaa", &managed_dir(d.path(), "aaa"), 30200));
    key(&mut a, KeyCode::F(5));
    assert_eq!(a.current().unwrap().entry.name, "beta");
    s.borrow_mut().servers.retain(|x| x.name != "beta");
    a.reload();
    assert!(a.sel < a.rows.len());
    let _: PathBuf = a.current().unwrap().entry.dir.clone();
}
