//! The shell's command line.
//!
//! ```text
//! kora-desktop [--background] [--open <route>] [--kora <path to kora>] [--smoke <report file>]
//! ```
//!
//! * `--background`: start in the tray with no window (what autostart uses).
//! * `--open <route>`: show this panel page (a second launch passes it to the first).
//! * `--kora <path>`: the `kora` program to ask for the panel (default: next to this program, else `PATH`).
//! * `--smoke <file>`: the automated test of the CI (`.github/workflows/desktop.yml`): start, load the panel,
//!   write a JSON report to `<file>`, drive one tray action, quit. It also never starts a background service
//!   or changes autostart.
//! * `--version`, `--help`.

use std::path::PathBuf;

use crate::nav;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Args {
    pub background: bool,
    pub route: Option<String>,
    pub kora: Option<PathBuf>,
    pub smoke: Option<PathBuf>,
    pub version: bool,
    pub help: bool,
}

pub const HELP: &str = "Kora desktop app: a tray icon and a window for Kora Panel.\n\nUsage: kora-desktop [--background] [--open <page>] [--kora <path>]\n\n  --background    start in the tray without opening the window\n  --open <page>   open this page of the panel, e.g. /settings\n  --kora <path>   the kora program to use\n  --version       print the version\n";

pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Args, String> {
    let mut a = Args::default();
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--background" => a.background = true,
            "--version" | "-V" => a.version = true,
            "--help" | "-h" => a.help = true,
            "--open" => {
                let r = it.next().ok_or("--open needs a page, for example --open /settings")?;
                if !nav::safe_route(&r) {
                    return Err(format!("{r:?} is not a page of Kora"));
                }
                a.route = Some(r);
            }
            "--kora" => a.kora = Some(PathBuf::from(it.next().ok_or("--kora needs the path of the kora program")?)),
            "--smoke" => a.smoke = Some(PathBuf::from(it.next().ok_or("--smoke needs a report file")?)),
            other => return Err(format!("unknown option {other:?}. Try --help.")),
        }
    }
    Ok(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(a: &[&str]) -> Result<Args, String> {
        parse(a.iter().map(|s| s.to_string()))
    }

    #[test]
    fn options_parse() {
        assert_eq!(p(&[]).unwrap(), Args::default());
        let a = p(&["--background", "--open", "/settings", "--kora", "C:\\k\\kora.exe", "--smoke", "r.json"]).unwrap();
        assert!(a.background && a.route.as_deref() == Some("/settings") && a.kora == Some(PathBuf::from("C:\\k\\kora.exe")) && a.smoke == Some(PathBuf::from("r.json")));
        assert!(p(&["--version"]).unwrap().version && p(&["-h"]).unwrap().help);
    }

    #[test]
    fn bad_input_gets_a_sentence() {
        assert!(p(&["--open"]).unwrap_err().contains("needs a page"));
        assert!(p(&["--open", "http://evil.example"]).unwrap_err().contains("not a page"));
        assert!(p(&["--kora"]).is_err() && p(&["--smoke"]).is_err());
        assert!(p(&["--frobnicate"]).unwrap_err().contains("unknown option"));
    }
}
