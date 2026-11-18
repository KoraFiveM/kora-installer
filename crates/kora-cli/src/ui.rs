//! Prompts and output helpers shared by several commands.

use kora_core::installer::{self, registry};

/// THE place a [`Problem`] is printed for a terminal: `error: <message>`, then the details and one
/// hint line per action that has one (`Problem`'s `Display`). Everything that fails in front of a
/// person on the command line comes through here.
pub(crate) fn print_error(p: &kora_core::problem::Problem) {
    eprintln!("error: {p}");
}

/// [`print_error`], and the exit code 1.
pub(crate) fn fail(p: impl Into<kora_core::problem::Problem>) -> i32 {
    print_error(&p.into());
    1
}

pub(crate) fn print_lines(prefix: &str, lines: &[String]) {
    for l in lines {
        println!("{prefix}{l}");
    }
}

pub(crate) fn confirm(question: &str) -> bool {
    use std::io::Write;
    print!("{question} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).is_ok() && matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// The folder of `server` (a registered name or a folder).
pub(crate) fn server_dir(server: &str) -> Result<std::path::PathBuf, String> {
    let reg = registry::Registry::load().unwrap_or_default();
    if let Some(e) = reg.find(server) {
        return Ok(e.dir.clone());
    }
    let p = std::path::Path::new(server);
    if p.join("server.cfg").is_file() || p.join("kora-server.json").is_file() {
        return Ok(p.to_path_buf());
    }
    Err(format!("there is no server called '{server}' (see `kora status`), and it is not a server folder"))
}

/// Asks on the terminal; an empty answer takes the default.
pub(crate) struct Terminal;

impl installer::uninstall::Asker for Terminal {
    fn ask(&mut self, question: &str, default_yes: bool) -> bool {
        use std::io::Write;
        print!("{question} {} ", if default_yes { "[Y/n]" } else { "[y/N]" });
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() {
            return false;
        }
        match line.trim().to_ascii_lowercase().as_str() {
            "" => default_yes,
            "y" | "yes" => true,
            _ => false,
        }
    }
}

/// Ask a yes/no question on the terminal; an empty answer means yes.
pub(crate) fn ask_yes_no(question: &str) -> bool {
    use std::io::Write;
    print!("{question}");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return false;
    }
    !matches!(line.trim().to_lowercase().as_str(), "n" | "no")
}
