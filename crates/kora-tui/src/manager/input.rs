//! A one-line text field: typing, cursor keys, paste-safe (newlines and control characters are
//! dropped, over-long text is cut and reported).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    style::{Modifier, Style},
    text::Span,
};

use crate::clipboard::sanitize_paste;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextInput {
    chars: Vec<char>,
    cur: usize,
    max: usize,
}

impl TextInput {
    pub fn new(max: usize) -> Self {
        Self { chars: Vec::new(), cur: 0, max }
    }

    pub fn with_text(max: usize, text: &str) -> Self {
        let mut t = Self::new(max);
        t.set(text);
        t
    }

    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    pub fn set(&mut self, text: &str) {
        self.chars = text.chars().filter(|c| !c.is_control()).take(self.max).collect();
        self.cur = self.chars.len();
    }

    /// Insert pasted text. Returns true when it had to be shortened.
    pub fn paste(&mut self, text: &str) -> bool {
        let room = self.max.saturating_sub(self.chars.len());
        let (clean, cut) = sanitize_paste(text, room);
        for c in clean.chars() {
            self.chars.insert(self.cur, c);
            self.cur += 1;
        }
        cut
    }

    /// Handle an editing key; false when the key is not an editing key (the caller may use it).
    pub fn key(&mut self, k: KeyEvent) -> bool {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL) || k.modifiers.contains(KeyModifiers::ALT);
        match k.code {
            KeyCode::Char(c) if !ctrl && !c.is_control() => {
                if self.chars.len() < self.max {
                    self.chars.insert(self.cur, c);
                    self.cur += 1;
                }
                true
            }
            KeyCode::Backspace => {
                if self.cur > 0 {
                    self.cur -= 1;
                    self.chars.remove(self.cur);
                }
                true
            }
            KeyCode::Delete => {
                if self.cur < self.chars.len() {
                    self.chars.remove(self.cur);
                }
                true
            }
            KeyCode::Left => {
                self.cur = self.cur.saturating_sub(1);
                true
            }
            KeyCode::Right => {
                self.cur = (self.cur + 1).min(self.chars.len());
                true
            }
            KeyCode::Home => {
                self.cur = 0;
                true
            }
            KeyCode::End => {
                self.cur = self.chars.len();
                true
            }
            _ => false,
        }
    }

    /// The text as spans with the cursor shown as a reversed cell, shortened to `width` cells
    /// around the cursor.
    pub fn spans(&self, width: usize, focused: bool) -> Vec<Span<'static>> {
        let width = width.max(2);
        let start = (self.cur + 1).saturating_sub(width);
        let visible: Vec<char> = self.chars.iter().skip(start).take(width).copied().collect();
        let at = self.cur - start;
        let before: String = visible.iter().take(at).collect();
        let under = visible.get(at).copied();
        let after: String = visible.iter().skip(at + 1).collect();
        if !focused {
            return vec![Span::raw(visible.iter().collect::<String>())];
        }
        vec![
            Span::raw(before),
            Span::styled(under.unwrap_or(' ').to_string(), Style::default().add_modifier(Modifier::REVERSED)),
            Span::raw(after),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    #[test]
    fn typing_editing_and_cursor() {
        let mut t = TextInput::new(10);
        for c in "abc".chars() {
            t.key(key(KeyCode::Char(c)));
        }
        t.key(key(KeyCode::Left));
        t.key(key(KeyCode::Char('X')));
        assert_eq!(t.text(), "abXc");
        t.key(key(KeyCode::Backspace));
        t.key(key(KeyCode::Delete));
        assert_eq!(t.text(), "ab");
        assert!(!t.key(key(KeyCode::Enter)), "Enter is left to the screen");
        assert!(!t.key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL)));
    }

    #[test]
    fn paste_drops_newlines_and_is_capped() {
        let mut t = TextInput::new(6);
        assert!(!t.paste("ab\r\ncd\u{1b}"));
        assert_eq!(t.text(), "abcd");
        assert!(t.paste("123456789"), "reported as shortened");
        assert_eq!(t.text(), "abcd12");
    }

    #[test]
    fn long_text_scrolls_with_the_cursor() {
        let t = TextInput::with_text(100, "0123456789abcdefghij");
        let s: String = t.spans(8, true).iter().map(|s| s.content.to_string()).collect();
        assert_eq!(s.chars().count(), 8);
        assert!(s.contains('j'), "the end (where the cursor is) stays visible");
    }
}
