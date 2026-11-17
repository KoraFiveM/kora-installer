//! Everything the Kora desktop app DECIDES, with no window, tray or operating-system call in it.
//!
//! The desktop app (`crates/kora-desktop`, a separate binary built only by `.github/workflows/desktop.yml`) is a
//! thin shell around the web panel: a tray icon, a native window that shows the panel, native notifications. All
//! of its logic lives here so it is unit-tested on every machine and in the normal CI, and so the main `kora`
//! binary never needs a GUI toolkit. The GUI layer only turns these values into widgets and OS calls.
//!
//! * [`nav`]: which addresses the window may show, which go to the browser, which are refused (the threat model
//!   is in that module).
//! * [`bridge`]: the tiny allow-list of requests the page may make to the shell.
//! * [`model`], [`tray`]: the panel's data as the shell reads it, the tray's colour/tooltip and its menu.
//! * [`toast`]: which events become notifications (coalescing, pause, quiet hours, deep links).
//! * [`session`]: what `kora desktop session` prints and how the shell reconnects.
//! * [`autostart`]: the exact file/registry contents for "Start with Windows/your desktop".
//! * [`instance`]: single instance (a second launch asks the first to show its window).
//! * [`settings`], [`cli`]: the shell's own small preferences and its command line.

pub mod autostart;
pub mod bridge;
pub mod cli;
pub mod icon;
pub mod instance;
pub mod model;
pub mod nav;
pub mod platform;
pub mod session;
pub mod settings;
pub mod toast;
pub mod tray;

/// The name people see (window title, tray tooltip, notifications).
pub const APP_NAME: &str = "Kora";
