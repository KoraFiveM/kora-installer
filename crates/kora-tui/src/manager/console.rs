//! The live console screen: the server's output, following as it is written, and a box to type
//! commands into. Output comes from the logs (see `kora_core::console`); commands are queued in a
//! file that the server's `kora_console` resource runs.

use std::path::PathBuf;

use super::input::TextInput;
use super::logs::tail;

/// Most lines kept in memory.
pub const MAX_LINES: usize = 1000;
const MAX_HISTORY: usize = 50;

#[derive(Debug, Clone)]
pub struct ConsoleState {
    pub dir: PathBuf,
    pub lines: Vec<String>,
    /// Lines hidden below the window (0 = following the end).
    pub scroll: usize,
    pub input: TextInput,
    /// The server has the `kora_console` helper (commands can be sent).
    pub enabled: bool,
    pub note: Option<String>,
    source: Option<PathBuf>,
    seen_len: u64,
    history: Vec<String>,
    hist_pos: Option<usize>,
}

impl ConsoleState {
    pub fn open(dir: PathBuf) -> Self {
        let mut s = Self {
            enabled: kora_core::console::is_installed(&dir),
            dir,
            lines: vec![],
            scroll: 0,
            input: TextInput::new(kora_core::console::MAX_COMMAND),
            note: None,
            source: None,
            seen_len: u64::MAX,
            history: vec![],
            hist_pos: None,
        };
        s.refresh();
        s
    }

    pub fn source_name(&self) -> Option<String> {
        self.source.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned())
    }

    /// Re-read the log when it grew, changed or switched file. True when the lines changed.
    pub fn refresh(&mut self) -> bool {
        let src = kora_core::console::source(&self.dir);
        let len = src.as_ref().and_then(|p| std::fs::metadata(p).ok()).map_or(0, |m| m.len());
        if src == self.source && len == self.seen_len {
            return false;
        }
        let grew = src == self.source && len > self.seen_len && self.seen_len != u64::MAX;
        self.source = src;
        self.seen_len = len;
        let new = match &self.source {
            Some(p) => tail(p, MAX_LINES).unwrap_or_default(),
            None => vec![],
        };
        // While scrolled up, keep the same lines in view as new ones arrive.
        if grew && self.scroll > 0 {
            self.scroll += new.len().saturating_sub(self.lines.len());
        }
        self.lines = new;
        self.scroll = self.scroll.min(self.lines.len().saturating_sub(1));
        true
    }

    pub fn window(&self, h: usize) -> &[String] {
        let end = self.lines.len().saturating_sub(self.scroll);
        &self.lines[end.saturating_sub(h)..end]
    }

    pub fn scroll_up(&mut self, n: usize) {
        self.scroll = (self.scroll + n).min(self.lines.len().saturating_sub(1));
    }

    pub fn scroll_down(&mut self, n: usize) {
        self.scroll = self.scroll.saturating_sub(n);
    }

    pub fn to_end(&mut self) {
        self.scroll = 0;
    }

    /// Queue what is typed. The box is emptied only when it was accepted.
    pub fn send(&mut self) {
        let text = self.input.text();
        match kora_core::console::send(&self.dir, &text) {
            Ok(()) => {
                let t = text.trim().to_string();
                if self.history.last() != Some(&t) {
                    self.history.push(t);
                    if self.history.len() > MAX_HISTORY {
                        self.history.remove(0);
                    }
                }
                self.hist_pos = None;
                self.input.set("");
                self.scroll = 0;
                self.note = Some("Sent. The server prints the result below (it checks for commands twice a second).".into());
            }
            Err(e) => self.note = Some(e),
        }
    }

    /// Up / Down in the box: walk through earlier commands.
    pub fn history(&mut self, back: bool) {
        if self.history.is_empty() {
            return;
        }
        let pos = match (self.hist_pos, back) {
            (None, true) => self.history.len() - 1,
            (None, false) => return,
            (Some(p), true) => p.saturating_sub(1),
            (Some(p), false) if p + 1 < self.history.len() => p + 1,
            (Some(_), false) => {
                self.hist_pos = None;
                self.input.set("");
                return;
            }
        };
        self.hist_pos = Some(pos);
        self.input.set(&self.history[pos].clone());
    }

    /// Add the helper to the server (needs a restart to take effect).
    pub fn enable(&mut self) {
        match kora_core::console::install(&self.dir) {
            Ok(changed) => {
                self.enabled = kora_core::console::is_installed(&self.dir);
                self.note = Some(if changed {
                    "Live console added to this server. Restart the server once so it loads, then you can type commands here.".into()
                } else {
                    "Live console was already added.".into()
                });
            }
            Err(e) => self.note = Some(e),
        }
    }
}

/// Colour class of one console line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Typed,
    Error,
    Warn,
}

pub fn tone(line: &str) -> Tone {
    let l = line.to_ascii_lowercase();
    if line.starts_with("> ") {
        Tone::Typed
    } else if l.contains("error") || l.contains("failed") || l.contains("exception") {
        Tone::Error
    } else if l.contains("warn") {
        Tone::Warn
    } else {
        Tone::Plain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("server.cfg"), "ensure kora_core\n").unwrap();
        std::fs::create_dir_all(d.path().join(".kora")).unwrap();
        d
    }

    #[test]
    fn follows_the_log_as_it_grows_and_keeps_the_view_when_scrolled_up() {
        let d = server();
        let log = d.path().join(".kora").join("console.log");
        std::fs::write(&log, "a\nb\nc\n").unwrap();
        let mut c = ConsoleState::open(d.path().to_path_buf());
        assert_eq!(c.window(10), ["a", "b", "c"]);
        assert!(!c.refresh(), "nothing new");
        std::fs::write(&log, "a\nb\nc\nd\n").unwrap();
        assert!(c.refresh());
        assert_eq!(c.window(1), ["d"], "following the end");
        c.scroll_up(1);
        assert_eq!(c.window(1), ["c"]);
        std::fs::write(&log, "a\nb\nc\nd\ne\nf\n").unwrap();
        c.refresh();
        assert_eq!(c.window(1), ["c"], "scrolled up: the same line stays in view");
        c.to_end();
        assert_eq!(c.window(1), ["f"]);
    }

    #[test]
    fn typed_commands_are_queued_remembered_and_recalled() {
        let d = server();
        let mut c = ConsoleState::open(d.path().to_path_buf());
        assert!(!c.enabled);
        c.input.set("status");
        c.send();
        assert!(c.note.as_deref().unwrap().contains("live console"), "refused until the helper is added");
        assert_eq!(c.input.text(), "status", "kept so nothing typed is lost");
        c.enable();
        assert!(c.enabled);
        c.send();
        c.input.set("say hi");
        c.send();
        assert_eq!(std::fs::read_to_string(kora_core::console::input_path(d.path())).unwrap(), "status\nsay hi\n");
        assert_eq!(c.input.text(), "");
        c.history(true);
        assert_eq!(c.input.text(), "say hi");
        c.history(true);
        assert_eq!(c.input.text(), "status");
        c.history(false);
        c.history(false);
        assert_eq!(c.input.text(), "", "past the newest entry the box is empty again");
    }

    #[test]
    fn lines_are_toned() {
        assert_eq!(tone("> restart kora_core"), Tone::Typed);
        assert_eq!(tone("SCRIPT ERROR: boom"), Tone::Error);
        assert_eq!(tone("[warn] slow"), Tone::Warn);
        assert_eq!(tone("Started resource x"), Tone::Plain);
    }
}
