//! The real [`Backend`] is `kora_core::api::SystemBackend`; this file keeps the part that needs the
//! TUI's `open` module: opening folders and the local txAdmin address.

use std::path::Path;

pub use kora_core::api::system::*;

/// Opens folders and the local txAdmin address with the system's default programs.
pub struct SystemLauncher;

impl super::app::Launcher for SystemLauncher {
    fn open_folder(&mut self, dir: &Path) -> Result<(), String> {
        crate::open::Opener::open(&mut crate::open::SystemOpener, dir, dir)
    }

    fn open_url(&mut self, url: &str) -> Result<(), String> {
        // Only ever the local txAdmin address, built from a port number.
        if !url.starts_with("http://127.0.0.1:") || url.chars().any(|c| c.is_control() || c == ' ' || c == '"') {
            return Err("Only the local txAdmin address can be opened.".into());
        }
        let (program, args): (&str, Vec<&str>) = match crate::open::OsKind::current() {
            crate::open::OsKind::Windows => ("rundll32.exe", vec!["url.dll,FileProtocolHandler", url]),
            crate::open::OsKind::Other => ("xdg-open", vec![url]),
        };
        std::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|mut c| {
                std::thread::spawn(move || {
                    let _ = c.wait();
                });
            })
            .map_err(|_| "Couldn't open a browser automatically.".to_string())
    }
}
