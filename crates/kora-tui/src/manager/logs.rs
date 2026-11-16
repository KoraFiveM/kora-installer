//! Scrolling state of the log viewer. Finding and reading the files is `kora_core::api::logs`.

use std::path::Path;

pub use kora_core::api::logs::*;

#[derive(Debug, Clone, Default)]
pub struct LogView {
    pub sources: Vec<LogSource>,
    pub idx: usize,
    pub lines: Vec<String>,
    /// Lines hidden below the window (0 = showing the end of the file).
    pub scroll: usize,
    pub error: Option<String>,
}

impl LogView {
    pub fn open(dir: &Path) -> Self {
        let mut v = Self { sources: discover(dir), ..Self::default() };
        v.reload();
        v
    }

    pub fn current(&self) -> Option<&LogSource> {
        self.sources.get(self.idx)
    }

    pub fn reload(&mut self) {
        self.scroll = 0;
        self.error = None;
        self.lines.clear();
        if let Some(s) = self.current() {
            match tail(&s.path.clone(), MAX_LINES) {
                Ok(l) => self.lines = l,
                Err(e) => self.error = Some(e),
            }
        }
    }

    pub fn select(&mut self, delta: isize) {
        if self.sources.is_empty() {
            return;
        }
        let n = self.sources.len() as isize;
        self.idx = (self.idx as isize + delta).rem_euclid(n) as usize;
        self.reload();
    }

    pub fn scroll_up(&mut self, n: usize) {
        self.scroll = (self.scroll + n).min(self.lines.len().saturating_sub(1));
    }

    pub fn scroll_down(&mut self, n: usize) {
        self.scroll = self.scroll.saturating_sub(n);
    }

    pub fn to_top(&mut self) {
        self.scroll = self.lines.len().saturating_sub(1);
    }

    pub fn to_end(&mut self) {
        self.scroll = 0;
    }

    /// The `h` lines shown for the current scroll position.
    pub fn window(&self, h: usize) -> &[String] {
        let end = self.lines.len().saturating_sub(self.scroll);
        &self.lines[end.saturating_sub(h)..end]
    }

    /// Text copied by the Copy button: everything loaded.
    pub fn copy_text(&self) -> String {
        self.lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_known_logs_and_scrolls() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join(".kora")).unwrap();
        std::fs::write(d.path().join(".kora/service-host.log"), "a\nb\nc\nd\ne\n").unwrap();
        std::fs::write(d.path().join("update.log"), "u1\n").unwrap();
        let mut v = LogView::open(d.path());
        assert_eq!(v.sources.iter().map(|s| s.name).collect::<Vec<_>>(), ["service-host.log", "update.log"]);
        assert_eq!(v.window(2), ["d", "e"]);
        v.scroll_up(2);
        assert_eq!(v.window(2), ["b", "c"]);
        v.scroll_up(100);
        assert_eq!(v.scroll, 4, "clamped to the first line");
        v.scroll_down(1);
        v.to_end();
        assert_eq!(v.window(1), ["e"]);
        v.select(1);
        assert_eq!(v.lines, ["u1"]);
        v.select(1);
        assert_eq!(v.current().unwrap().name, "service-host.log", "wraps around");
        assert_eq!(v.copy_text(), "a\nb\nc\nd\ne");
    }

    #[test]
    fn no_logs_is_an_empty_view() {
        let d = tempfile::tempdir().unwrap();
        let v = LogView::open(d.path());
        assert!(v.sources.is_empty() && v.lines.is_empty());
        assert!(v.window(5).is_empty());
    }
}
