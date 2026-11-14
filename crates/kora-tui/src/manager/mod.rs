//! `kora manager`: the multi-server manager screen (list, per-server actions, add server).
//!
//! * [`app`]: state machine (keys, mouse, paste, jobs); no terminal access.
//! * [`view`]: drawing + hit-testing from one layout.
//! * [`backend`] / [`adapter`]: everything that touches the system, behind a trait (fakes in tests).
//! * [`adopt`]: finding existing server folders; [`logs`]: log tailing; [`input`]: text field.
//! * [`api_stubs`]: TEMPORARY stand-ins for the update/notify/schedule APIs of other branches.

pub mod adapter;
pub mod adopt;
pub mod api_stubs;
pub mod app;
pub mod backend;
pub mod input;
pub mod logs;
pub mod view;

#[cfg(test)]
mod testing;
#[cfg(test)]
mod tests;

/// The single switch point for the stubbed APIs (see `api_stubs.rs`).
pub use api_stubs as api;

use std::error::Error;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyEventKind};
use ratatui::layout::Rect;

pub use app::Exit;

use app::{App, Screen, Services};

/// The real services: registry, OS services, browser/explorer, system clipboard.
pub fn system_services() -> Services {
    Services {
        backend: Box::new(adapter::SystemBackend),
        launcher: Box::new(adapter::SystemLauncher),
        clipboard: Box::new(crate::clipboard::SystemClipboard::default()),
    }
}

/// Run the manager until the user quits or asks for the install wizard.
pub fn run_manager() -> Result<Exit, Box<dyn Error>> {
    let mut term = super::Term::new()?;
    let mut app = App::new(system_services());
    let mut last_refresh = Instant::now();
    loop {
        term.t.draw(|f| view::render(f, &app))?;
        // The "working" text is on screen now; do the slow job, then draw its result.
        if app.run_pending() {
            continue;
        }
        if let Some(e) = app.exit {
            return Ok(e);
        }
        if event::poll(Duration::from_millis(500))? {
            match event::read()? {
                Event::Key(k) if k.kind != KeyEventKind::Release => app.handle_key(k),
                Event::Mouse(m) => {
                    let s = term.t.size()?;
                    app.handle_mouse(m, Rect::new(0, 0, s.width, s.height));
                }
                Event::Paste(s) => app.handle_paste(&s),
                _ => {}
            }
        } else if last_refresh.elapsed() >= Duration::from_secs(3) && matches!(app.screen, Screen::Main) {
            app.refresh_status();
            last_refresh = Instant::now();
        }
    }
}
