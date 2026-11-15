//! Render tests for the main screens (80x24, 120x40, 60x15 too small, ASCII / NO_COLOR themes) and
//! plain-text dumps for design review (written only when `KORA_TUI_DUMP_DIR` is set).

use std::fmt::Write as _;

use crossterm::event::KeyCode;
use kora_core::catalog::Catalog;
use kora_core::installer::{InstallOutcome, ProgressEvent, StepStatus};
use ratatui::{backend::TestBackend, Frame, Terminal};

use super::testing::*;
use crate::progress::{render_progress, ProgressState};
use crate::result::{render_result, ResultScreen};
use crate::wizard::{Screen as WS, Wizard};

/// A wizard that does not depend on who runs the tests: `Wizard::new()` offers a restart with
/// permission to non-root users, which adds a first step ("step 2 of 16" instead of "step 1 of 15").
fn wizard() -> Wizard {
    Wizard::new().with_elevation(false, vec![], crate::elev::system_elevator())
}

const SHA: &str = "0123456789012345678901234567890123456789";

fn catalog() -> Catalog {
    let rel = format!(r#""latest":"1.0.0","versions":[{{"version":"1.0.0","tag":"v1.0.0","commit":"{SHA}"}}]"#);
    let r = |id: &str, extra: &str, order: u32| {
        format!(r#"{{"id":"{id}","title":"{id} title","repository":"https://github.com/o/{id}.git","path":"{id}","group":"other","kind":"optional","branch":"main","order":{order},{rel}{extra}}}"#)
    };
    let json = format!(
        r#"{{"schemaVersion":3,"creators":[{{"id":"acme","name":"Acme Scripts","url":"https://acme.example","description":"Nice scripts"}}],"resources":[{},{},{},{}]}}"#,
        r("compat-a", r#","type":"compat""#, 1),
        r("bridge-x", r#","type":"bridge","dependsOn":["compat-a"]"#, 2),
        r("acme-one", r#","type":"creator","creator":"acme","dependsOn":["bridge-x"]"#, 3),
        r("acme-two", r#","type":"creator","creator":"acme""#, 4),
    );
    Catalog::parse_validated(&json).unwrap()
}

pub fn dump(w: u16, h: u16, draw: impl FnOnce(&mut Frame)) -> String {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(draw).unwrap();
    let b = t.backend().buffer().clone();
    let mut s = String::new();
    for y in 0..h {
        let line: String = (0..w).map(|x| b[(x, y)].symbol().to_string()).collect();
        let _ = writeln!(s, "{}", line.trim_end());
    }
    s
}

#[test]
fn write_dumps() {
    let Ok(dir) = std::env::var("KORA_TUI_DUMP_DIR") else { return };
    std::fs::create_dir_all(&dir).unwrap();
    let mut out: Vec<(String, String)> = vec![];
    for (w, h) in [(80u16, 24u16), (120, 40)] {
        let mut add = |name: &str, s: String| out.push((format!("{name}-{w}x{h}"), s));
        for (name, screen) in [("wizard-welcome", WS::Welcome), ("wizard-mode", WS::Mode), ("wizard-ports", WS::Ports), ("wizard-dblogin", WS::DbAdminLogin), ("wizard-review", WS::Review)] {
            let mut wz = wizard();
            wz.screen = screen;
            add(name, dump(w, h, |f| crate::wizard_view::render(f, &wz)));
        }
        let mut wz = wizard();
        wz.screen = WS::Welcome;
        wz.help_open = true;
        add("wizard-help-modal", dump(w, h, |f| crate::wizard_view::render(f, &wz)));

        let mut p = ProgressState::default();
        for (i, st) in [StepStatus::Done, StepStatus::Done, StepStatus::Running, StepStatus::Pending, StepStatus::Skipped].into_iter().enumerate() {
            p.apply(ProgressEvent { step: format!("s{i}"), title: format!("Step number {i}"), status: st, detail: "Downloading the game server".into(), percent: 40 });
        }
        add("progress", dump(w, h, |f| render_progress(f, &p, false)));
        let fail = ResultScreen::failure("Port 30120 is already in use.", "What was done so far is kept.\nTo try again: run `kora install` again.", None);
        add("result-failure", dump(w, h, |f| render_result(f, &fail)));
        let ok = ResultScreen::outcome(&InstallOutcome {
            dir: "/srv/kora/alpha".into(), connect: "192.168.1.20:30120".into(), txadmin_url: "http://localhost:40120".into(),
            name: "alpha".into(), start_attempted: true, running: true, game_listening: true, ..Default::default()
        });
        add("result-success", dump(w, h, |f| render_result(f, &ok)));

        let d = tempfile::tempdir().unwrap();
        let mut es = vec![];
        for (n, port) in [("alpha", 30120u16), ("bravo", 30121), ("charlie", 30122)] {
            es.push(entry(n, &managed_dir(d.path(), n), port));
        }
        let (mut a, s) = app_with(es);
        s.borrow_mut().status.insert("alpha".into(), kora_core_status_running());
        s.borrow_mut().add_catalog = Some((catalog(), vec![]));
        s.borrow_mut().resources = vec!["kora-lib".into()];
        a.reload();
        let mut a2 = a;
        add("manager-main", text(&a2, w, h));
        chr(&mut a2, 'g');
        chr(&mut a2, 'b');
        add("addres-sections", text(&a2, w, h));
        key(&mut a2, KeyCode::Down);
        key(&mut a2, KeyCode::Down);
        key(&mut a2, KeyCode::Enter);
        key(&mut a2, KeyCode::Enter);
        add("addres-creator", text(&a2, w, h));
        chr(&mut a2, 'a');
        chr(&mut a2, 'i');
        add("addres-review", text(&a2, w, h));
        let (mut a3, _s3) = app_with(vec![]);
        add("manager-empty", text(&a3, w, h));
        let _ = &mut a3;
    }
    let (small_w, small_h) = (60, 15);
    let wz = wizard();
    out.push(("wizard-too-small-60x15".into(), dump(small_w, small_h, |f| crate::wizard_view::render(f, &wz))));
    with_theme(Theme::new(ColorLevel::None, false), || {
        for (n, s) in all_screens(80, 24) {
            out.push((format!("ascii-nocolor-{n}-80x24"), s));
        }
    });
    for (n, s) in out {
        std::fs::write(format!("{dir}/{n}.txt"), s).unwrap();
    }
}

fn kora_core_status_running() -> super::backend::Status {
    super::backend::Status::Running
}


// ------------------------------------------------------------------------------------------
// Render tests
// ------------------------------------------------------------------------------------------

use crate::theme::{with_theme, ColorLevel, Theme};

/// Every main screen as (name, text) for one terminal size.
fn all_screens(w: u16, h: u16) -> Vec<(&'static str, String)> {
    let mut out = vec![];
    for (name, screen) in [("welcome", WS::Welcome), ("mode", WS::Mode), ("ports", WS::Ports), ("dblogin", WS::DbAdminLogin), ("review", WS::Review), ("license", WS::License)] {
        let mut wz = wizard();
        wz.screen = screen;
        out.push((name, dump(w, h, |f| crate::wizard_view::render(f, &wz))));
    }
    let mut p = ProgressState::default();
    for (i, st) in [StepStatus::Done, StepStatus::Running, StepStatus::Failed, StepStatus::Pending, StepStatus::Skipped].into_iter().enumerate() {
        p.apply(ProgressEvent { step: format!("s{i}"), title: format!("Step {i}"), status: st, detail: "detail".into(), percent: 40 });
    }
    out.push(("progress", dump(w, h, |f| render_progress(f, &p, false))));
    let fail = ResultScreen::failure("Port 30120 is already in use.", "To try again: run `kora install` again.", None);
    out.push(("failure", dump(w, h, |f| render_result(f, &fail))));
    let ok = ResultScreen::outcome(&InstallOutcome { dir: "/srv/kora/alpha".into(), connect: "1.2.3.4:30120".into(), txadmin_url: "http://localhost:40120".into(), name: "alpha".into(), start_attempted: true, running: true, game_listening: true, ..Default::default() });
    out.push(("success", dump(w, h, |f| render_result(f, &ok))));
    let d = tempfile::tempdir().unwrap();
    let (mut a, s) = app_with(vec![entry("alpha", &managed_dir(d.path(), "alpha"), 30120)]);
    s.borrow_mut().add_catalog = Some((catalog(), vec![]));
    a.reload();
    out.push(("manager", text(&a, w, h)));
    chr(&mut a, 'g');
    chr(&mut a, 'b');
    out.push(("sections", text(&a, w, h)));
    key(&mut a, KeyCode::Down);
    key(&mut a, KeyCode::Down);
    key(&mut a, KeyCode::Enter);
    key(&mut a, KeyCode::Enter);
    out.push(("creator", text(&a, w, h)));
    out
}

#[test]
fn main_screens_render_at_80x24_and_120x40_with_their_key_text() {
    for (w, h) in [(80, 24), (120, 40)] {
        let screens = all_screens(w, h);
        let get = |n: &str| screens.iter().find(|(x, _)| *x == n).unwrap().1.clone();
        for (name, want) in [
            ("welcome", &["KORA", "Welcome to Kora", "step 1 of", "(•) Take me through it", "Enter", "Next", "Back", "Help"][..]),
            ("mode", &["What kind of server is this?", "Production", "Development"]),
            ("ports", &["Network ports", "Game port", "[ Paste ]", "[ Clear ]"]),
            ("dblogin", &["Database administrator login", "Admin user", "Test login"]),
            ("review", &["Ready to install", "Name:", "Ports:", "This will change your computer as follows:", "Install"]),
            ("progress", &["KORA", "Overall progress", "40%", "Steps", "Step 0", "[ ✗ FAIL ]", "What's happening now"]),
            ("failure", &["[ ✗ FAIL ]", "The install could not finish", "Port 30120", "What to do next:", "Press Enter to exit"]),
            ("success", &["[ ✓ OK ]", "Your server is running", "[ Open txAdmin ]", "1.2.3.4:30120"]),
            ("manager", &["KORA", "manager", "Servers (1)", "alpha", "stopped", "Start (s)", "select"]),
            ("sections", &["Compatibility", "Bridges", "Creators", "chosen", "Browse"][..3]),
            ("creator", &["Acme Scripts", "Nice scripts", "acme-one title", "<needs bridge-x>", "a  select all / none", "Compatibility"]),
        ] {
            let t = get(name);
            for s in want {
                assert!(t.contains(s), "{name} at {w}x{h} is missing {s:?}:\n{t}");
            }
        }
    }
}

#[test]
fn below_80x24_every_top_level_screen_says_the_terminal_is_too_small() {
    for (w, h) in [(60, 15), (79, 40), (120, 23), (10, 3), (1, 1)] {
        for (name, t) in all_screens(w, h) {
            assert!(t.contains("Terminal too small") || w < 30 || h < 6, "{name} at {w}x{h}:\n{t}");
        }
    }
    let t = dump(60, 15, |f| crate::wizard_view::render(f, &wizard()));
    assert!(t.contains("Terminal too small") && t.contains("80x24") && t.contains("60x15"), "{t}");
}

#[test]
fn ascii_theme_without_colour_draws_only_ascii_and_keeps_the_words() {
    let t = Theme::new(ColorLevel::None, false);
    with_theme(t, || {
        for (name, text) in all_screens(100, 30) {
            assert!(text.is_ascii(), "{name} has non-ASCII output:\n{}", text.chars().filter(|c| !c.is_ascii()).collect::<String>());
        }
        let wz = wizard();
        let mut term = Terminal::new(TestBackend::new(100, 30)).unwrap();
        term.draw(|f| crate::wizard_view::render(f, &wz)).unwrap();
        // NO_COLOR: no cell carries a foreground or background colour.
        assert!(term.backend().buffer().content().iter().all(|c| c.fg == ratatui::style::Color::Reset && c.bg == ratatui::style::Color::Reset));
        let s: String = all_screens(100, 30).into_iter().find(|(n, _)| *n == "failure").unwrap().1;
        assert!(s.contains("[ FAIL ]") && s.contains("could not finish"), "{s}");
    });
}

#[test]
fn sixteen_colour_theme_uses_named_colours_only() {
    with_theme(Theme::new(ColorLevel::Ansi16, true), || {
        let wz = wizard();
        let mut term = Terminal::new(TestBackend::new(100, 30)).unwrap();
        term.draw(|f| crate::wizard_view::render(f, &wz)).unwrap();
        use ratatui::style::Color;
        assert!(term.backend().buffer().content().iter().all(|c| !matches!(c.fg, Color::Rgb(..) | Color::Indexed(_)) && !matches!(c.bg, Color::Rgb(..) | Color::Indexed(_))));
    });
}
