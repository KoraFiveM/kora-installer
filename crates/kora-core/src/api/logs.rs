//! Log files of one server: which exist, their last lines (cleaned of terminal escape codes) and
//! (the viewer's scrolling state is front-end code).

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Most lines kept per file and most bytes read from its end.
pub const MAX_LINES: usize = 2000;
const MAX_BYTES: u64 = 512 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LogSource {
    pub name: &'static str,
    pub path: PathBuf,
}

/// The logs of a server folder that exist: `server.log`, `service-host.log`, `update.log`.
pub fn discover(dir: &Path) -> Vec<LogSource> {
    let candidates: [(&'static str, [PathBuf; 2]); 3] = [
        ("server.log", [dir.join(".kora").join("server.log"), dir.join("server.log")]),
        ("service-host.log", [dir.join(".kora").join("service-host.log"), dir.join("service-host.log")]),
        ("update.log", [dir.join("update.log"), dir.join(".kora").join("update.log")]),
    ];
    candidates
        .into_iter()
        .filter_map(|(name, paths)| paths.into_iter().find(|p| p.is_file()).map(|path| LogSource { name, path }))
        .collect()
}

/// Remove ANSI escape sequences and other control characters (tabs become spaces).
pub fn clean_line(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\u{1b}' => match it.peek() {
                Some('[') => {
                    it.next();
                    for n in it.by_ref() {
                        if ('@'..='~').contains(&n) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    it.next();
                    while let Some(n) = it.next() {
                        if n == '\u{7}' || (n == '\u{1b}' && it.peek() == Some(&'\\')) {
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\t' => out.push_str("    "),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// The last `max` lines of a file (reads only its end).
pub fn tail(path: &Path, max: usize) -> Result<Vec<String>, String> {
    let mut f = std::fs::File::open(path).map_err(|e| format!("Can't read {}: {e}", path.display()))?;
    let len = f.metadata().map_err(|e| e.to_string())?.len();
    let start = len.saturating_sub(MAX_BYTES);
    f.seek(SeekFrom::Start(start)).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    f.take(MAX_BYTES).read_to_end(&mut buf).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<&str> = text.lines().collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0); // probably cut in the middle
    }
    let skip = lines.len().saturating_sub(max);
    Ok(lines.into_iter().skip(skip).map(clean_line).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_colour_codes_and_controls() {
        assert_eq!(clean_line("\u{1b}[31mred\u{1b}[0m ok\r"), "red ok");
        assert_eq!(clean_line("a\tb\u{7}"), "a    b");
        assert_eq!(clean_line("\u{1b}]0;title\u{7}x"), "x");
    }

    #[test]
    fn tail_returns_last_lines_only() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x.log");
        let body: String = (0..50).map(|i| format!("line {i}\n")).collect();
        std::fs::write(&p, body).unwrap();
        let t = tail(&p, 3).unwrap();
        assert_eq!(t, ["line 47", "line 48", "line 49"]);
        assert!(tail(&d.path().join("none.log"), 3).is_err());
    }

    #[test]
    fn discovers_known_logs() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join(".kora")).unwrap();
        std::fs::write(d.path().join(".kora/service-host.log"), "a\n").unwrap();
        std::fs::write(d.path().join("update.log"), "u1\n").unwrap();
        let v = discover(d.path());
        assert_eq!(v.iter().map(|s| s.name).collect::<Vec<_>>(), ["service-host.log", "update.log"]);
        assert!(discover(&d.path().join("none")).is_empty());
    }
}
