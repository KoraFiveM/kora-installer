//! ratatui front end: story-style install wizard, progress and outcome screens.
//!
//! Logic is split into pure state machines/renderers ([`wizard`], [`wizard_view`],
//! [`progress`]) and the thin terminal runners in this file.

pub mod progress;
pub mod wizard;
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
            let _ = crossterm::execute!(io::stdout(), DisableMouseCapture, DisableBracketedPaste);
            ratatui::restore();
            prev(info);
        }));
    });
}

impl Term {
    fn new() -> io::Result<Self> {
        install_panic_hook();
        let t = ratatui::try_init()?;
        let _ = crossterm::execute!(io::stdout(), EnableMouseCapture, EnableBracketedPaste);
        Ok(Self { t })
    }
}

impl Drop for Term {
    fn drop(&mut self) {
        let _ = crossterm::execute!(io::stdout(), DisableMouseCapture, DisableBracketedPaste);
        ratatui::restore();
    }
}

/// Run the story-style wizard. `Ok(None)` if the user quits.
pub fn run_install_wizard() -> Result<Option<InstallRequest>, Box<dyn Error>> {
    run_install_wizard_with(None, None)
}

/// Like [`run_install_wizard`], with a setup-code redeemer for the Welcome stage and/or answers
/// the CLI already loaded from `--code`.
pub fn run_install_wizard_with(
    redeemer: Option<wizard::Redeemer>,
    prefill: Option<kora_core::site::Redeemed>,
) -> Result<Option<InstallRequest>, Box<dyn Error>> {
    let mut term = Term::new()?;
    let mut w = wizard::Wizard::new();
    if let Some(r) = redeemer {
        w = w.with_redeemer(r);
    }
    if let Some(p) = prefill {
        w.apply_redeemed(p, true);
    }
    while w.state() == wizard::WizardState::Running {
        term.t.draw(|f| wizard_view::render(f, &w))?;
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
    Ok((w.state() == wizard::WizardState::Done).then(|| w.build_request()))
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

/// Error screen (plain-language message, what to do next, log location); waits for a key.
pub fn show_failure(message: &str, next: &str, log: Option<&Path>) -> Result<(), Box<dyn Error>> {
    let mut term = Term::new()?;
    wait_key(&mut term, &mut |f| progress::render_failure(f, message, next, log))
}

/// Final screen with connect info; waits for a key.
pub fn show_outcome(o: &InstallOutcome) -> Result<(), Box<dyn Error>> {
    let mut term = Term::new()?;
    wait_key(&mut term, &mut |f| progress::render_outcome(f, o))
}

use std::io;

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
}
