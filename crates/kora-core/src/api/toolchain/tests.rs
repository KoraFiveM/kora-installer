use std::cell::RefCell;
use std::time::Duration;

use super::*;
use crate::api::install::github::GithubUser;
use crate::db::{DbError, DbResult};
use crate::problem::Action;

fn tool(t: Tool, installed: bool, source: Option<Source>, broken: bool) -> ToolStatus {
    ToolStatus { tool: t, name: tools::program_name(t), label: t.label(), installed, version: installed.then(|| "2.0.0".to_string()), source, path: None, broken, can_install: true }
}

fn running_db() -> DbState {
    DbState { installed: true, running: true, name: Some("mariadb".into()), version: Some("11.4".into()), unknown: false }
}

fn item<'a>(items: &'a [CheckItem], id: &str) -> &'a CheckItem {
    items.iter().find(|i| i.id == id).unwrap_or_else(|| panic!("no {id} in {items:?}"))
}

fn kinds(i: &CheckItem) -> Vec<Action> {
    i.problem.as_ref().map(|p| p.actions.clone()).unwrap_or_default()
}

#[test]
fn everything_fine_has_no_fix_buttons() {
    let t = [tool(Tool::Git, true, Some(Source::Private), false), tool(Tool::Gh, true, Some(Source::System), false)];
    let items = build_check(false, &t, &GithubUser::ready("sam"), &running_db(), &AdminState { elevated: false, helper: true, service: true });
    assert!(items.iter().all(|i| i.state == CheckState::Ok), "{items:?}");
    assert_eq!(items.iter().map(|i| i.id).collect::<Vec<_>>(), ["git", "gh", "github", "database", "admin", "service"]);
    assert!(item(&items, "git").summary.contains("Kora's own copy") && item(&items, "gh").summary.contains("on this computer"));
    assert!(item(&items, "github").summary.contains("as sam"));
    assert_eq!(check_json(&items)["ok"], true);
}

#[test]
fn missing_tools_offer_install_it_for_me() {
    let t = [tool(Tool::Git, false, None, false), tool(Tool::Gh, false, None, false)];
    let items = build_check(true, &t, &GithubUser::gh_missing(), &DbState::default(), &AdminState::default());
    assert_eq!(item(&items, "git").state, CheckState::Problem);
    assert_eq!(kinds(item(&items, "git"))[0], Action::InstallTool { tool: Tool::Git });
    assert_eq!(kinds(item(&items, "gh"))[0], Action::InstallTool { tool: Tool::Gh });
    // GitHub waits for gh: no second, confusing fix.
    let g = item(&items, "github");
    assert_eq!((g.state, g.problem.is_none()), (CheckState::Info, true));
    assert_eq!(check_json(&items)["ok"], false);
}

#[test]
fn github_states_each_get_their_own_fix() {
    let t = [tool(Tool::Git, true, None, false), tool(Tool::Gh, true, None, false)];
    let admin = AdminState::default();
    let out = build_check(false, &t, &GithubUser::signed_out(), &running_db(), &admin);
    assert_eq!(kinds(item(&out, "github")), vec![Action::SignInGithub { scopes: vec![] }]);
    let scopes = build_check(false, &t, &GithubUser::missing_scopes("sam", vec!["read:org".into()]), &running_db(), &admin);
    let g = item(&scopes, "github");
    assert_eq!(g.state, CheckState::Problem);
    assert_eq!(kinds(g), vec![Action::SignInGithub { scopes: vec!["read:org".into()] }]);
    assert!(g.summary.contains("permissions"));
}

#[test]
fn a_tool_that_is_there_but_does_not_run_is_a_problem_with_a_reinstall() {
    let t = [tool(Tool::Git, false, Some(Source::System), true), tool(Tool::Gh, true, None, false)];
    let items = build_check(true, &t, &GithubUser::ready("a"), &running_db(), &AdminState::default());
    let g = item(&items, "git");
    assert_eq!((g.state, g.summary.as_str()), (CheckState::Problem, "Found, but it does not run."));
    assert_eq!(kinds(g), vec![Action::InstallTool { tool: Tool::Git }]);
}

#[test]
fn a_stopped_database_offers_start_it_for_me_and_a_missing_one_is_only_information() {
    let t = [tool(Tool::Git, true, None, false), tool(Tool::Gh, true, None, false)];
    let stopped = DbState { running: false, ..running_db() };
    let items = build_check(false, &t, &GithubUser::ready("a"), &stopped, &AdminState::default());
    let d = item(&items, "database");
    assert_eq!(d.state, CheckState::Problem);
    assert!(d.summary.contains("installed but not running"));
    assert_eq!(kinds(d), vec![Action::StartDatabase]);
    assert_eq!(d.problem.as_ref().unwrap().code, codes::INSTALL_DB_STOPPED);
    let none = build_check(false, &t, &GithubUser::ready("a"), &DbState::default(), &AdminState::default());
    assert_eq!(item(&none, "database").state, CheckState::Info);
    assert!(item(&none, "database").problem.is_none());
    let unknown = build_check(false, &t, &GithubUser::ready("a"), &DbState { unknown: true, ..DbState::default() }, &AdminState::default());
    assert_eq!(item(&unknown, "database").state, CheckState::Info);
}

#[test]
fn administrator_and_service_lines_say_it_in_plain_words() {
    let t = [tool(Tool::Git, true, None, false), tool(Tool::Gh, true, None, false)];
    let win = build_check(true, &t, &GithubUser::ready("a"), &running_db(), &AdminState::default());
    assert!(item(&win, "admin").summary.contains("administrator permission"));
    assert_eq!(item(&win, "service").state, CheckState::Info);
    let lin = build_check(false, &t, &GithubUser::ready("a"), &running_db(), &AdminState { elevated: true, ..AdminState::default() });
    assert_eq!(item(&lin, "admin").summary, "Kora is running as root.");
}

#[test]
fn db_state_prefers_the_running_service() {
    let svc = |n: &str, r: bool| DetectedService { name: n.into(), running: r, version: Some("1".into()), port: 3306, socket: None };
    let s = db_state(&[svc("mysql", false), svc("mariadb", true)]);
    assert_eq!((s.installed, s.running, s.name.as_deref()), (true, true, Some("mariadb")));
    let s = db_state(&[svc("mysql", false)]);
    assert_eq!((s.installed, s.running, s.name.as_deref()), (true, false, Some("mysql")));
    assert_eq!(db_state(&[]), DbState::default());
}

// ------------------------------------------------------------ starting the database

/// Pretends to be systemctl / sc: starting makes the service active.
struct Svc {
    calls: RefCell<Vec<String>>,
    started: RefCell<bool>,
    start_fails: bool,
    never_up: bool,
}
impl Svc {
    fn new() -> Self {
        Svc { calls: RefCell::new(vec![]), started: RefCell::new(false), start_fails: false, never_up: false }
    }
}
impl CommandRunner for Svc {
    fn run(&self, spec: &CommandSpec) -> DbResult<crate::db::CommandOutput> {
        let line = format!("{} {}", spec.program, spec.args.join(" "));
        self.calls.borrow_mut().push(line.clone());
        let out = |status: i32, stdout: &str, stderr: &str| Ok(crate::db::CommandOutput { status, stdout: stdout.into(), stderr: stderr.into() });
        if line.starts_with("systemctl start") {
            if self.start_fails {
                return out(1, "", "Failed to start mariadb.service: Unit is masked.");
            }
            *self.started.borrow_mut() = true;
            return out(0, "", "");
        }
        if line.starts_with("systemctl list-unit-files mariadb") {
            return out(0, "mariadb.service enabled\n", "");
        }
        if line.starts_with("systemctl list-unit-files") {
            return out(0, "", "");
        }
        if line.starts_with("systemctl is-active") {
            return out(0, if *self.started.borrow() && !self.never_up { "active\n" } else { "inactive\n" }, "");
        }
        Err(DbError::Command(format!("unexpected {line}")))
    }
}

fn stopped() -> Vec<DetectedService> {
    vec![DetectedService { name: "mariadb".into(), running: false, version: None, port: 3306, socket: None }]
}

#[test]
fn starting_the_database_starts_the_service_and_waits_until_it_is_up() {
    let r = Svc::new();
    let mut said = Vec::new();
    let m = start_database_with(&r, &stopped(), true, false, Duration::from_secs(2), &mut |m| said.push(m.to_string())).unwrap();
    assert!(m.contains("mariadb is running"), "{m}");
    assert!(r.calls.borrow().iter().any(|c| c == "systemctl start mariadb"));
    assert!(said[0].contains("Starting the database server mariadb"));
}

#[test]
fn without_administrator_rights_the_start_is_refused_with_the_elevation_problem_and_nothing_runs() {
    let r = Svc::new();
    let p = start_database_with(&r, &stopped(), false, true, Duration::from_secs(1), &mut |_| {}).unwrap_err();
    assert_eq!(p.code, codes::NEEDS_ELEVATION);
    assert_eq!(p.actions[0], Action::InstallAdminService);
    assert_eq!(p.actions[1], Action::show_command(&["net", "start", "mariadb"], true));
    assert!(r.calls.borrow().is_empty(), "nothing was run: {:?}", r.calls.borrow());
    let l = start_database_with(&r, &stopped(), false, false, Duration::from_secs(1), &mut |_| {}).unwrap_err();
    assert_eq!(l.actions[1], Action::show_command(&["systemctl", "start", "mariadb"], true));
}

#[test]
fn a_start_that_fails_or_never_comes_up_says_so_and_offers_another_go() {
    let bad = Svc { start_fails: true, ..Svc::new() };
    let p = start_database_with(&bad, &stopped(), true, false, Duration::from_secs(1), &mut |_| {}).unwrap_err();
    assert_eq!(p.code, codes::DB_START_FAILED);
    assert!(p.details.as_deref().unwrap().contains("masked"));
    assert!(p.actions.contains(&Action::StartDatabase));
    let slow = Svc { never_up: true, ..Svc::new() };
    let p = start_database_with(&slow, &stopped(), true, false, Duration::from_millis(20), &mut |_| {}).unwrap_err();
    assert_eq!(p.code, codes::DB_START_FAILED);
}

#[test]
fn nothing_to_start_is_not_an_error_when_it_already_runs_and_is_a_plain_problem_when_there_is_none() {
    let up = vec![DetectedService { running: true, ..stopped().remove(0) }];
    assert!(start_database_with(&Svc::new(), &up, true, false, Duration::from_secs(1), &mut |_| {}).unwrap().contains("already running"));
    let p = start_database_with(&Svc::new(), &[], true, false, Duration::from_secs(1), &mut |_| {}).unwrap_err();
    assert_eq!(p.code, codes::INSTALL_DB_MISSING);
}
