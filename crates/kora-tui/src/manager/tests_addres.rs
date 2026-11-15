//! Add resources, report an issue and open a pull request (manager screens).

use crossterm::event::KeyCode;
use kora_core::addresources::AddOutcome;
use kora_core::catalog::Catalog;
use kora_core::feedback::PrPlan;
use kora_core::installer::Mode;

use super::addres::{AddResStage, FbStage};
use super::app::*;
use super::testing::*;

const SHA: &str = "0123456789012345678901234567890123456789";

fn catalog() -> Catalog {
    let rel = format!(
        r#""latest":"1.0.0","versions":[{{"version":"1.0.0","tag":"v1.0.0","commit":"{SHA}"}}]"#
    );
    let r = |id: &str, extra: &str, order: u32| {
        format!(
            r#"{{"id":"{id}","title":"{id} title","repository":"https://github.com/o/{id}.git","path":"{id}","group":"other","kind":"optional","branch":"main","order":{order},{rel}{extra}}}"#
        )
    };
    let json = format!(
        r#"{{"schemaVersion":3,"creators":[{{"id":"acme","name":"Acme Scripts","url":"https://acme.example","description":"Nice scripts"}}],"resources":[{},{},{},{}]}}"#,
        r("compat-a", r#","type":"compat""#, 1),
        r(
            "bridge-x",
            r#","type":"bridge","dependsOn":["compat-a"]"#,
            2
        ),
        r(
            "acme-one",
            r#","type":"creator","creator":"acme","dependsOn":["bridge-x"]"#,
            3
        ),
        r("acme-two", r#","type":"creator","creator":"acme""#, 4),
    );
    Catalog::parse_validated(&json).unwrap()
}

fn setup(mode: Mode) -> (App, Shared, tempfile::TempDir) {
    let d = tempfile::tempdir().unwrap();
    let mut e = entry("alpha", &managed_dir(d.path(), "alpha"), 30120);
    e.mode = mode;
    let (a, s) = app_with(vec![e]);
    s.borrow_mut().add_catalog = Some((catalog(), vec![]));
    s.borrow_mut().resources = vec!["kora-lib".into(), "kora-core".into()];
    (a, s, d)
}

fn notice(a: &App) -> String {
    a.notice
        .as_ref()
        .map(|n| n.text.clone())
        .unwrap_or_default()
}

fn open_add(a: &mut App) {
    chr(a, 'g');
    assert!(matches!(a.screen, Screen::AddRes(_)), "{}", screen_text(a));
}

fn browse_to_creator(a: &mut App) {
    chr(a, 'b'); // browse
    key(a, KeyCode::Down);
    key(a, KeyCode::Down); // Creators
    key(a, KeyCode::Enter); // the creators
    key(a, KeyCode::Enter); // Acme
}

#[test]
fn main_screen_offers_the_three_actions_and_pr_only_for_dev() {
    let (a, _s, _d) = setup(Mode::Production);
    let t = screen_text(&a);
    assert!(
        t.contains("Add resources (g)")
            && t.contains("Report an issue (b)")
            && t.contains("Open pull request (p)"),
        "{t}"
    );
    assert!(
        !clickable(&a, AREA, Hit::Act(Action::OpenPr)),
        "production has no forks to propose from"
    );
    let (a, _s, _d) = setup(Mode::Development);
    assert!(clickable(&a, AREA, Hit::Act(Action::OpenPr)));
}

#[test]
fn browsing_shows_nothing_selected_and_sections_with_counts() {
    let (mut a, _s, _d) = setup(Mode::Production);
    open_add(&mut a);
    chr(&mut a, 'b');
    let t = screen_text(&a);
    assert!(
        t.contains("Compatibility")
            && t.contains("1 available")
            && t.contains("Bridges")
            && t.contains("Creators")
            && t.contains("1 creators"),
        "{t}"
    );
    assert!(t.contains("0 chosen"), "{t}");
    let Screen::AddRes(s) = &a.screen else {
        panic!()
    };
    assert!(s.picked.is_empty());
}

#[test]
fn creator_drill_in_select_all_and_dependencies_are_shown() {
    let (mut a, _s, _d) = setup(Mode::Production);
    open_add(&mut a);
    browse_to_creator(&mut a);
    let t = screen_text(&a);
    assert!(
        t.contains("Acme Scripts")
            && t.contains("Nice scripts")
            && t.contains("acme-one title")
            && t.contains("acme-two title"),
        "{t}"
    );
    chr(&mut a, 'a'); // select all
    let Screen::AddRes(s) = &a.screen else {
        panic!()
    };
    assert_eq!(s.picked, ["acme-one", "acme-two"]);
    // go back to the bridges: bridge-x and compat-a are shown ticked as needed
    key(&mut a, KeyCode::Esc); // creators
    key(&mut a, KeyCode::Esc); // sections
    let Screen::AddRes(s) = &a.screen else {
        panic!()
    };
    assert_eq!(s.stage, AddResStage::Sections);
    key(&mut a, KeyCode::Down);
    key(&mut a, KeyCode::Enter);
    let t = screen_text(&a);
    assert!(t.contains("[x]") && t.contains("needed by acme-one"), "{t}");
}

#[test]
fn a_needed_dependency_cannot_be_deselected_but_all_none_clears_the_rest() {
    let (mut a, _s, _d) = setup(Mode::Production);
    open_add(&mut a);
    browse_to_creator(&mut a);
    chr(&mut a, ' '); // acme-one
    key(&mut a, KeyCode::Esc); // creators
    key(&mut a, KeyCode::Esc); // sections
    key(&mut a, KeyCode::Down);
    key(&mut a, KeyCode::Enter); // bridges
    chr(&mut a, ' '); // try to deselect bridge-x
    assert!(notice(&a).contains("needed by acme-one"), "{}", notice(&a));
    let Screen::AddRes(s) = &a.screen else {
        panic!()
    };
    assert_eq!(s.picked, ["acme-one"], "the blocked toggle changed nothing");
}

#[test]
fn review_lists_dependencies_folders_and_ensure_lines_then_installs_the_picked_ids() {
    let (mut a, s, _d) = setup(Mode::Production);
    s.borrow_mut().add_result = Some(Ok(AddOutcome {
        installed: vec!["compat-a".into(), "bridge-x".into(), "acme-one".into()],
        ensured: vec!["[compat]".into()],
        ..Default::default()
    }));
    open_add(&mut a);
    browse_to_creator(&mut a);
    chr(&mut a, ' ');
    chr(&mut a, 'i');
    let t = screen_text(&a);
    assert!(
        t.contains("[compat]/compat-a")
            && t.contains("[bridges]/bridge-x")
            && t.contains("[creators]/[acme]/acme-one"),
        "{t}"
    );
    assert!(
        t.contains("ensure [compat], ensure [bridges], ensure [creators]"),
        "{t}"
    );
    assert!(t.contains("(needed)"));
    key(&mut a, KeyCode::Enter);
    assert!(
        s.borrow()
            .calls
            .iter()
            .any(|c| c == "add_install alpha acme-one"),
        "{:?}",
        s.borrow().calls
    );
    let t = screen_text(&a);
    assert!(
        t.contains("added  acme-one")
            && t.contains("ensure [compat]")
            && t.contains("Restart the server"),
        "{t}"
    );
    assert!(notice(&a).contains("Added 3"));
}

#[test]
fn review_without_a_choice_says_so() {
    let (mut a, _s, _d) = setup(Mode::Production);
    open_add(&mut a);
    chr(&mut a, 'b');
    chr(&mut a, 'i');
    assert!(notice(&a).contains("Nothing is chosen"), "{}", notice(&a));
    let Screen::AddRes(s) = &a.screen else {
        panic!()
    };
    assert_eq!(s.stage, AddResStage::Sections);
}

#[test]
fn a_website_code_goes_straight_to_the_review() {
    let (mut a, s, _d) = setup(Mode::Production);
    s.borrow_mut().add_redeem = Some(Ok(vec!["bridge-x".into()]));
    open_add(&mut a);
    chr(&mut a, 'c');
    typed(&mut a, "KORA-7F3K-92QX");
    key(&mut a, KeyCode::Enter);
    assert!(s
        .borrow()
        .calls
        .iter()
        .any(|c| c == "add_redeem KORA-7F3K-92QX"));
    let t = screen_text(&a);
    assert!(
        t.contains("[bridges]/bridge-x") && t.contains("[compat]/compat-a"),
        "{t}"
    );
    // an unknown id in the code is refused as a whole
    let (mut a, s, _d) = setup(Mode::Production);
    s.borrow_mut().add_redeem = Some(Ok(vec!["nope".into()]));
    open_add(&mut a);
    chr(&mut a, 'c');
    typed(&mut a, "KORA-7F3K-92QX");
    key(&mut a, KeyCode::Enter);
    assert!(notice(&a).contains("does not offer"), "{}", notice(&a));
    let Screen::AddRes(st) = &a.screen else {
        panic!()
    };
    assert_eq!(st.stage, AddResStage::Code);
}

#[test]
fn a_bad_code_and_an_empty_code_stay_on_the_code_screen() {
    let (mut a, s, _d) = setup(Mode::Production);
    s.borrow_mut().add_redeem = Some(Err("That setup code is not valid".into()));
    open_add(&mut a);
    chr(&mut a, 'c');
    key(&mut a, KeyCode::Enter);
    assert!(notice(&a).contains("Type or paste the code"));
    typed(&mut a, "KORA-7F3K-92QX");
    key(&mut a, KeyCode::Enter);
    assert!(notice(&a).contains("not valid"));
    assert!(matches!(&a.screen, Screen::AddRes(st) if st.stage == AddResStage::Code));
}

#[test]
fn catalog_load_failure_returns_to_the_list() {
    let (mut a, s, _d) = setup(Mode::Production);
    s.borrow_mut().add_catalog_error = Some("offline".into());
    chr(&mut a, 'g');
    assert!(matches!(a.screen, Screen::Main));
    assert!(notice(&a).contains("offline"));
}

#[test]
fn install_failure_needing_github_hands_over_to_the_sign_in() {
    let (mut a, s, _d) = setup(Mode::Development);
    s.borrow_mut().add_result = Some(Err(kora_core::feedback::SIGN_IN_HINT.into()));
    open_add(&mut a);
    chr(&mut a, 'b');
    key(&mut a, KeyCode::Enter);
    chr(&mut a, ' ');
    chr(&mut a, 'i');
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.exit, Some(Exit::GithubLogin));
}

// ---------------------------------------------------------------- issue and pr

#[test]
fn report_issue_picks_a_resource_then_sends_title_and_details() {
    for mode in [Mode::Production, Mode::Development] {
        let (mut a, s, _d) = setup(mode);
        s.borrow_mut().signed_in = true;
        chr(&mut a, 'b');
        assert!(screen_text(&a).contains("kora-lib"));
        key(&mut a, KeyCode::Down);
        key(&mut a, KeyCode::Enter);
        key(&mut a, KeyCode::Enter); // no title yet
        assert!(notice(&a).contains("short title"), "{}", notice(&a));
        typed(&mut a, "Crash");
        key(&mut a, KeyCode::Tab);
        typed(&mut a, "it broke");
        key(&mut a, KeyCode::Enter);
        assert!(
            s.borrow()
                .calls
                .iter()
                .any(|c| c == "issue alpha kora-core [Crash] [it broke]"),
            "{:?}",
            s.borrow().calls
        );
        let t = screen_text(&a);
        assert!(
            t.contains("Your report was sent") && t.contains("/issues/1"),
            "{t}"
        );
        chr(&mut a, 'c');
        assert_eq!(
            s.borrow().clip.as_deref(),
            Some("https://github.com/KoraFiveM/x/issues/1")
        );
    }
}

#[test]
fn not_signed_in_hands_over_to_the_github_sign_in_instead_of_failing() {
    let (mut a, _s, _d) = setup(Mode::Production);
    chr(&mut a, 'b');
    assert_eq!(a.exit, Some(Exit::GithubLogin));
    assert!(matches!(a.screen, Screen::Main));
}

#[test]
fn pull_requests_are_refused_on_production_servers() {
    let (mut a, s, _d) = setup(Mode::Production);
    s.borrow_mut().signed_in = true;
    chr(&mut a, 'p');
    assert!(matches!(a.screen, Screen::Main));
    assert!(notice(&a).contains("development servers"), "{}", notice(&a));
}

fn plan() -> PrPlan {
    PrPlan {
        resource: "kora-lib".into(),
        upstream: "KoraFiveM/kora_lib".into(),
        base: "dev".into(),
        head_owner: "me".into(),
        head_branch: "fix/x".into(),
        ahead: 2,
        dir: "/nowhere".into(),
    }
}

#[test]
fn pr_flow_checks_the_fork_shows_what_will_be_pushed_then_opens() {
    let (mut a, s, _d) = setup(Mode::Development);
    s.borrow_mut().signed_in = true;
    s.borrow_mut().pr_plan = Some(Ok(plan()));
    chr(&mut a, 'p');
    key(&mut a, KeyCode::Enter); // kora-lib
    let t = screen_text(&a);
    assert!(
        t.contains("push branch 'fix/x'")
            && t.contains("KoraFiveM/kora_lib")
            && t.contains("2 new commit"),
        "{t}"
    );
    typed(&mut a, "Fix");
    key(&mut a, KeyCode::Enter);
    assert!(
        s.borrow()
            .calls
            .iter()
            .any(|c| c == "pr_open kora-lib [Fix] []"),
        "{:?}",
        s.borrow().calls
    );
    assert!(screen_text(&a).contains("pull request is open"));
}

#[test]
fn pr_problems_are_shown_and_nothing_is_opened() {
    let (mut a, s, _d) = setup(Mode::Development);
    s.borrow_mut().signed_in = true;
    s.borrow_mut().pr_plan = Some(Err(
        "kora-lib has no commits that KoraFiveM/kora_lib lacks".into()
    ));
    chr(&mut a, 'p');
    key(&mut a, KeyCode::Enter);
    assert!(notice(&a).contains("no commits"));
    assert!(matches!(&a.screen, Screen::Feedback(f) if f.stage == FbStage::Pick));
    assert!(!s.borrow().calls.iter().any(|c| c.starts_with("pr_open")));
}
