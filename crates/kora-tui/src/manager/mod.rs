//! `kora manager`: the multi-server manager screen (list, per-server actions, add server).
//!
//! * [`app`]: state machine (keys, mouse, paste, jobs); no terminal access.
//! * [`view`]: drawing + hit-testing from one layout.
//! * [`backend`] / [`adapter`]: everything that touches the system, behind a trait (fakes in tests).
//! * [`adopt`]: finding existing server folders; [`logs`]: log tailing; [`input`]: text field.

pub mod adapter;
pub mod addres;
pub mod adopt;
pub mod app;
pub mod backend;
pub mod console;
pub mod input;
pub mod logs;
pub mod parity;
pub mod view;

#[cfg(test)]
mod testing;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_addres;
#[cfg(test)]
mod tests_render;

use std::cell::RefCell;
use std::error::Error;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyEventKind};
use ratatui::layout::Rect;

pub use app::Exit;

use app::{App, Screen, Services};

/// The real services: registry, OS services, browser/explorer, system clipboard.
/// `live` is how a running update is drawn; with no terminal handle it draws nothing.
pub fn system_services_with(live: Box<dyn FnMut(&app::Live)>) -> Services {
    Services {
        backend: Box::new(adapter::SystemBackend),
        launcher: Box::new(adapter::SystemLauncher),
        clipboard: Box::new(crate::clipboard::SystemClipboard::default()),
        live,
    }
}

/// Run the manager until the user quits or asks for the install wizard.
pub fn run_manager() -> Result<Exit, Box<dyn Error>> {
    let term = Rc::new(RefCell::new(super::Term::new()?));
    let t2 = term.clone();
    let live: Box<dyn FnMut(&app::Live)> = Box::new(move |l| {
        let _ = t2.borrow_mut().t.draw(|f| view::render_live(f, l));
    });
    let mut app = App::new(system_services_with(live));
    let mut last_refresh = Instant::now();
    loop {
        app.tick_console();
        term.borrow_mut().t.draw(|f| view::render(f, &app))?;
        // The "working" text is on screen now; do the slow job, then draw its result.
        if app.run_pending() {
            continue;
        }
        if let Some(e) = app.exit.clone() {
            return Ok(e);
        }
        if event::poll(Duration::from_millis(500))? {
            match event::read()? {
                Event::Key(k) if k.kind != KeyEventKind::Release => app.handle_key(k),
                Event::Mouse(m) => {
                    let s = term.borrow().t.size()?;
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
