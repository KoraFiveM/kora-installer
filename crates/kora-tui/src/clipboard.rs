//! Clipboard access and paste sanitising. The trait lets tests inject a fake clipboard; the real
//! one never panics or hangs when there is no display (ssh, containers): it returns a plain message.

/// What the UI needs from a clipboard.
pub trait Clipboard {
    fn get_text(&mut self) -> Result<String, String>;
    fn set_text(&mut self, text: &str) -> Result<(), String>;
}

/// Shown whenever the system clipboard can't be used.
pub const NO_CLIPBOARD: &str =
    "The clipboard isn't available here. Paste with your terminal's own shortcut (for example right-click or Ctrl+Shift+V) instead.";

/// The operating system clipboard (created on first use and kept, because on Linux the copied
/// text is only served while the program that copied it is alive).
#[derive(Default)]
pub struct SystemClipboard {
    inner: Option<arboard::Clipboard>,
    failed: bool,
}

impl SystemClipboard {
    fn open(&mut self) -> Result<&mut arboard::Clipboard, String> {
        if self.inner.is_none() {
            if self.failed {
                return Err(NO_CLIPBOARD.into());
            }
            match arboard::Clipboard::new() {
                Ok(c) => self.inner = Some(c),
                Err(_) => {
                    self.failed = true;
                    return Err(NO_CLIPBOARD.into());
                }
            }
        }
        self.inner.as_mut().ok_or_else(|| NO_CLIPBOARD.to_string())
    }
}

impl Clipboard for SystemClipboard {
    fn get_text(&mut self) -> Result<String, String> {
        self.open()?.get_text().map_err(|_| "The clipboard has no text to paste.".to_string())
    }

    fn set_text(&mut self, text: &str) -> Result<(), String> {
        self.open()?.set_text(text.to_string()).map_err(|_| NO_CLIPBOARD.to_string())
    }
}

/// Clean pasted text for a single-line field: drop newlines and other control characters, trim
/// the ends and cap it at `max` characters. Returns the text and whether it had to be shortened.
pub fn sanitize_paste(text: &str, max: usize) -> (String, bool) {
    let cleaned: String = text.chars().filter(|c| !c.is_control()).collect();
    let trimmed = cleaned.trim();
    let n = trimmed.chars().count();
    if n > max {
        (trimmed.chars().take(max).collect(), true)
    } else {
        (trimmed.to_string(), false)
    }
}

/// Last `n` lines of a text (for "Copy details").
pub fn last_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// Test double.
#[cfg(test)]
#[derive(Default)]
pub struct FakeClipboard {
    pub text: Option<String>,
    pub fail: bool,
}

#[cfg(test)]
impl Clipboard for FakeClipboard {
    fn get_text(&mut self) -> Result<String, String> {
        if self.fail {
            return Err(NO_CLIPBOARD.into());
        }
        self.text.clone().ok_or_else(|| "The clipboard has no text to paste.".into())
    }

    fn set_text(&mut self, text: &str) -> Result<(), String> {
        if self.fail {
            return Err(NO_CLIPBOARD.into());
        }
        self.text = Some(text.to_string());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_controls_trims_and_caps() {
        assert_eq!(sanitize_paste("  hello\r\nworld\t!\x1b[31m ", 100), ("helloworld![31m".to_string(), false));
        assert_eq!(sanitize_paste("abcdef", 3), ("abc".to_string(), true));
        assert_eq!(sanitize_paste("\n\n", 5), (String::new(), false));
        assert_eq!(sanitize_paste("ééé", 2).0, "éé");
    }

    #[test]
    fn system_clipboard_never_panics_without_a_display() {
        // CI and ssh sessions have no clipboard: this must be a plain Err, not a panic or a hang.
        let mut c = SystemClipboard::default();
        let _ = c.get_text();
        let _ = c.set_text("x");
        let _ = c.set_text("y");
    }

    #[test]
    fn tail_lines() {
        assert_eq!(last_lines("a\nb\nc\nd", 2), "c\nd");
        assert_eq!(last_lines("a", 40), "a");
    }
}
