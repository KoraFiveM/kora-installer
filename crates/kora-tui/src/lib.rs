//! ratatui front end: story-style install wizard, progress and outcome screens.
//!
//! Logic is split into pure state machines/renderers ([`wizard`], [`wizard_view`],
//! [`progress`]) and the thin terminal runners in this file.

pub mod clipboard;
pub mod elev;
pub mod github_screens;
pub mod manager;
pub mod open;
pub mod progress;
pub mod result;
pub mod wizard;
pub mod theme;
pub mod wizard_view;

use std::error::Error;
use std::path::Path;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, KeyCode, KeyModifiers,
};
use kora_core::installer::{InstallOutcome, InstallRequest, ProgressEvent};
use ratatui::{layout::Rect, DefaultTerminal};

/// Terminal guard: always restores the terminal, even on error or panic unwinding.
struct Term {
    t: DefaultTerminal,
}

/// Make panics restore the terminal (incl. mouse/paste modes) before the message prints.
fn install_panic_hook() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore_modes();
            ratatui::restore();
            prev(info);
        }));
    });
}

fn restore_modes() {
    let _ = crossterm::execute!(io::stdout(), DisableBracketedPaste);
    let _ = crossterm::execute!(io::stdout(), DisableMouseCapture);
}

impl Term {
    fn new() -> io::Result<Self> {
        install_panic_hook();
        let t = ratatui::try_init()?;
        // Separate calls: bracketed paste isn't supported by the old Windows console and must not
        // stop the mouse from being enabled (or the reverse).
        let _ = crossterm::execute!(io::stdout(), EnableMouseCapture);
        let _ = crossterm::execute!(io::stdout(), EnableBracketedPaste);
        Ok(Self { t })
    }
}

impl Drop for Term {
    fn drop(&mut self) {
        restore_modes();
        ratatui::restore();
    }
}

/// Run the story-style wizard. `Ok(None)` if the user quits.
pub fn run_install_wizard() -> Result<Option<InstallRequest>, Box<dyn Error>> {
    let mut term = Term::new()?;
    let mut w = wizard::Wizard::new().with_github(github_screens::GithubFlow::new(std::sync::Arc::new(github_screens::SystemGithub)));
    if let Some(saved) = kora_core::installer::saved::load() {
        w.restore_saved(&saved);
    }
    while w.state() == wizard::WizardState::Running {
        term.t.draw(|f| wizard_view::render(f, &w))?;
        if w.wants_tick() && !event::poll(Duration::from_millis(100))? {
            w.tick();
            continue;
        }
        match event::read()? {
            Event::Key(k) if k.kind != KeyEventKind::Release => w.handle_key(k),
            Event::Mouse(m) => {
                let s = term.t.size()?;
                w.handle_mouse(m, Rect::new(0, 0, s.width, s.height));
            }
            Event::Paste(s) => w.handle_paste(&s),
            _ => {}
        }
    }
    match w.state() {
        wizard::WizardState::Elevated => {
            // The elevated copy took over: leave quietly without touching the terminal further.
            drop(term);
            std::process::exit(0);
        }
        wizard::WizardState::Done => Ok(Some(w.build_request())),
        _ => {
            if pause_now() {
                wait_key(&mut term, &mut |f| draw_goodbye(f))?;
            }
            Ok(None)
        }
    }
}

/// Pure rule: on Windows, when the console window is ours (or our elevated copy's), wait for a
/// key press before exiting so the window does not vanish with the message in it.
pub fn should_pause_on_exit(windows: bool, owns_console: bool, relaunched: bool, stdin_tty: bool, stdout_tty: bool) -> bool {
    kora_core::elevate::should_pause(windows, owns_console, relaunched, stdin_tty, stdout_tty)
}

fn pause_now() -> bool {
    use std::io::IsTerminal;
    should_pause_on_exit(
        cfg!(windows),
        kora_core::elevate::owns_console(),
        kora_core::elevate::was_relaunched(),
        io::stdin().is_terminal(),
        io::stdout().is_terminal(),
    )
}

fn draw_goodbye(f: &mut ratatui::Frame) {
    use ratatui::widgets::{Block, Borders, Paragraph};
    f.render_widget(
        Paragraph::new("\nNothing was changed.\n\nPress any key to close this window.")
            .block(Block::default().borders(Borders::ALL).title(" Kora ")),
        f.area(),
    );
}

/// Call right before the program ends: waits for a key press when the window would vanish.
pub fn pause_before_exit() {
    if !pause_now() {
        return;
    }
    if let Ok(mut term) = Term::new() {
        let _ = wait_key(&mut term, &mut |f| draw_goodbye(f));
    }
}

fn wait_key(term: &mut Term, draw: &mut dyn FnMut(&mut ratatui::Frame)) -> Result<(), Box<dyn Error>> {
    loop {
        term.t.draw(|f| draw(f))?;
        if let Event::Key(k) = event::read()? {
            if k.kind != KeyEventKind::Release {
                return Ok(());
            }
        }
    }
}

/// Full-screen progress. Returns when the sender hangs up (installer finished).
/// If a step failed, waits for a key press so the message can be read.
pub fn run_install_progress(rx: Receiver<ProgressEvent>) -> Result<(), Box<dyn Error>> {
    let mut term = Term::new()?;
    let mut st = progress::ProgressState::default();
    let mut finished = false;
    while !finished {
        loop {
            match rx.try_recv() {
                Ok(ev) => st.apply(ev),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    finished = true;
                    break;
                }
            }
        }
        st.tick = st.tick.wrapping_add(1);
        term.t.draw(|f| progress::render_progress(f, &st, finished))?;
        if !finished && event::poll(Duration::from_millis(80))? {
            if let Event::Key(k) = event::read()? {
                if k.kind != KeyEventKind::Release
                    && k.code == KeyCode::Char('c')
                    && k.modifiers.contains(KeyModifiers::CONTROL)
                {
                    return Err("Installation cancelled by user".into());
                }
            }
        }
    }
    if st.failed() {
        wait_key(&mut term, &mut |f| progress::render_progress(f, &st, true))?;
    }
    Ok(())
}

/// Error screen: what went wrong, what to do next (numbered), and clickable [ Open log ],
/// [ Open folder ], [ Copy path ] and [ Copy details ] actions (keys L, F, C, D).
pub fn show_failure(message: &str, next: &str, log: Option<&Path>) -> Result<(), Box<dyn Error>> {
    run_result(result::ResultScreen::failure(message, next, log))
}

/// Final screen with connect info and [ Open folder ] / [ Copy path ].
pub fn show_outcome(o: &InstallOutcome) -> Result<(), Box<dyn Error>> {
    run_result(result::ResultScreen::outcome(o))
}

fn run_result(screen: result::ResultScreen) -> Result<(), Box<dyn Error>> {
    use std::io::Write;
    let mut screen = screen
        .with_hyperlinks(open::system_supports_hyperlinks())
        .with_elevation(!elev::is_elevated() && elev::can_offer());
    let mut elevator = elev::system_elevator();
    let args = elev::original_args();
    let mut term = Term::new()?;
    let mut clipboard = clipboard::SystemClipboard::default();
    let mut opener = open::SystemOpener;
    let mut server = SystemServer::default();
    loop {
        {
            let mut svc = result::Services { clipboard: &mut clipboard, opener: &mut opener, elevator: &mut elevator, args: &args, server: &mut server };
            screen.tick(&mut svc);
        }
        term.t.draw(|f| result::render_result(f, &screen))?;
        let s = term.t.size()?;
        let area = Rect::new(0, 0, s.width, s.height);
        if let Some(seq) = screen.hyperlink_overlay(area) {
            // Re-draws only the path text as an OSC 8 link; terminals without support never get this.
            let mut out = io::stdout();
            let _ = out.write_all(seq.as_bytes());
            let _ = out.flush();
        }
        let mut svc = result::Services { clipboard: &mut clipboard, opener: &mut opener, elevator: &mut elevator, args: &args, server: &mut server };
        // While a start is running, wake up often to show its progress.
        if screen.is_busy() && !event::poll(std::time::Duration::from_millis(250))? {
            continue;
        }
        match event::read()? {
            Event::Key(k) if k.kind != KeyEventKind::Release => {
                if screen.handle_key(k, &mut svc) {
                    return Ok(());
                }
            }
            Event::Mouse(m) => screen.handle_mouse(m, area, &mut svc),
            _ => {}
        }
        if screen.restarted {
            drop(term);
            std::process::exit(0);
        }
    }
}

use std::io;
use std::sync::mpsc;

use crossterm::event::{self, Event, KeyEventKind};
use ratatui::{
    layout::Alignment,
    widgets::{Block, Borders, Paragraph},
    Frame,
};

/// Text shown on the welcome screen.
pub const WELCOME_TEXT: &str =
    "Welcome to Kora.\n\nThis installer is under construction.\n\nPress q or Esc to quit.";

fn draw(frame: &mut Frame) {
    let p = Paragraph::new(WELCOME_TEXT)
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).title(" Kora "));
    frame.render_widget(p, frame.area());
}

/// Run the welcome screen until the user quits.
pub fn run_welcome() -> io::Result<()> {
    let mut terminal = ratatui::init();
    let result = (|| loop {
        terminal.draw(draw)?;
        if let Event::Key(k) = event::read()? {
            if k.kind == KeyEventKind::Press && matches!(k.code, KeyCode::Char('q') | KeyCode::Esc)
            {
                return Ok(());
            }
        }
    })();
    ratatui::restore();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn welcome_renders() {
        let mut t = Terminal::new(TestBackend::new(60, 10)).unwrap();
        t.draw(draw).unwrap();
        let s = format!("{:?}", t.backend().buffer());
        assert!(s.contains("Welcome"));
    }

    #[test]
    fn pause_only_on_windows_when_the_window_is_ours() {
        assert!(should_pause_on_exit(true, true, false, true, true));
        assert!(should_pause_on_exit(true, false, true, true, true), "our elevated copy");
        assert!(!should_pause_on_exit(true, false, false, true, true), "a console that stays open needs no pause");
        assert!(!should_pause_on_exit(false, true, true, true, true));
        assert!(!should_pause_on_exit(true, true, true, false, true), "piped input: never wait");
    }

    #[test]
    fn goodbye_screen_says_what_to_do() {
        let mut t = Terminal::new(TestBackend::new(60, 10)).unwrap();
        t.draw(draw_goodbye).unwrap();
        let s = format!("{:?}", t.backend().buffer());
        assert!(s.contains("Press any key"));
    }
}

/// Starts/stops the real server for the final screen: a start runs on its own thread and reports
/// progress through a channel; a stop goes to the OS service or to the detached process.
#[derive(Default)]
struct SystemServer {
    rx: Option<mpsc::Receiver<result::ServerMsg>>,
}

impl result::ServerCtl for SystemServer {
    fn start(&mut self, name: &str) {
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        let name = name.to_string();
        std::thread::spawn(move || {
            let p = tx.clone();
            let mut progress = move |m: &str, _f: f32| {
                let _ = p.send(result::ServerMsg::Progress(m.to_string()));
            };
            let msg = match kora_core::launch::start_named(&name, &mut progress) {
                Ok(r) => result::ServerMsg::Finished(r),
                Err(e) => result::ServerMsg::Failed(format!("Could not start the server: {e}")),
            };
            let _ = tx.send(msg);
        });
    }

    fn poll(&mut self) -> Vec<result::ServerMsg> {
        let Some(rx) = &self.rx else { return vec![] };
        let v: Vec<_> = rx.try_iter().collect();
        if v.iter().any(|m| !matches!(m, result::ServerMsg::Progress(_))) {
            self.rx = None;
        }
        v
    }

    fn stop(&mut self, name: &str) -> Result<(), String> {
        use kora_core::installer::host;
        use kora_core::service::{ControlAction, ServiceState};
        match host::service_state(name) {
            Ok(ServiceState::NotInstalled) | Err(_) => kora_core::launch::stop_named_detached(name).map(|_| ()),
            Ok(_) => host::service_control(name, ControlAction::Stop).map_err(|e| format!("Could not stop the service: {e}")),
        }
    }
}
