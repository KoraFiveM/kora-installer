//! Central look of every screen: palette, semantic styles, glyphs, border sets and a few small
//! widgets (key-hint chips, status chips, modal, "terminal too small" panel).
//!
//! Three colour levels are supported so the TUI reads everywhere:
//! * [`ColorLevel::TrueColor`]: the brand lime `#95ff00`.
//! * [`ColorLevel::Ansi256`]: lime from the 256-colour cube.
//! * [`ColorLevel::Ansi16`]: bright green (the 16 colour palette has no lime).
//! * [`ColorLevel::None`] (`NO_COLOR` set, or `TERM=dumb`): no colour at all; state is carried by
//!   bold / reverse / dim and by symbols, never by colour alone.
//!
//! Unicode symbols have ASCII fallbacks behind the single [`Glyphs`] switch (non-UTF-8 locale,
//! the legacy Windows console, or `KORA_ASCII=1`).

use std::cell::RefCell;
use std::sync::OnceLock;

use ratatui::{
    layout::{Alignment, Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Clear, Padding, Paragraph, Wrap},
    Frame,
};

/// Smallest terminal the screens are laid out for.
pub const MIN_WIDTH: u16 = 80;
pub const MIN_HEIGHT: u16 = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorLevel {
    None,
    Ansi16,
    Ansi256,
    TrueColor,
}

/// All symbols the screens draw, in a Unicode and an ASCII flavour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyphs {
    pub unicode: bool,
    pub ok: &'static str,
    pub fail: &'static str,
    pub warn: &'static str,
    pub pending: &'static str,
    pub skipped: &'static str,
    pub spinner: &'static [&'static str],
    pub bullet: &'static str,
    pub radio_on: &'static str,
    pub radio_off: &'static str,
    pub pointer: &'static str,
    pub crumb: &'static str,
    pub cursor: &'static str,
    pub arrow: &'static str,
    pub ellipsis: &'static str,
    pub dot_on: &'static str,
    pub dot_off: &'static str,
    pub rule: &'static str,
    pub vbar: &'static str,
    pub gauge_full: &'static str,
    pub gauge_empty: &'static str,
}

impl Glyphs {
    pub const UNICODE: Glyphs = Glyphs {
        unicode: true,
        ok: "✓",
        fail: "✗",
        warn: "!",
        pending: "·",
        skipped: "–",
        spinner: &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"],
        bullet: "•",
        radio_on: "(•)",
        radio_off: "( )",
        pointer: "▶",
        crumb: "›",
        cursor: "▏",
        arrow: "→",
        ellipsis: "…",
        dot_on: "●",
        dot_off: "○",
        rule: "─",
        vbar: "│",
        gauge_full: "━",
        gauge_empty: "─",
    };
    pub const ASCII: Glyphs = Glyphs {
        unicode: false,
        ok: "+",
        fail: "x",
        warn: "!",
        pending: ".",
        skipped: "-",
        spinner: &["|", "/", "-", "\\"],
        bullet: "*",
        radio_on: "(*)",
        radio_off: "( )",
        pointer: ">",
        crumb: ">",
        cursor: "|",
        arrow: "->",
        ellipsis: "...",
        dot_on: "*",
        dot_off: "o",
        rule: "-",
        vbar: "|",
        gauge_full: "#",
        gauge_empty: "-",
    };

    pub fn spin(&self, tick: usize) -> &'static str {
        self.spinner[tick % self.spinner.len()]
    }
}

const ASCII_BORDER: border::Set<'static> = border::Set {
    top_left: "+",
    top_right: "+",
    bottom_left: "+",
    bottom_right: "+",
    vertical_left: "|",
    vertical_right: "|",
    horizontal_top: "-",
    horizontal_bottom: "-",
};

/// Which tone a chip or message has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Ok,
    Warn,
    Fail,
    Info,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub level: ColorLevel,
    pub glyphs: Glyphs,
    /// No animation (spinners still tick, but nothing fades or sweeps).
    pub reduced_motion: bool,
}

impl Theme {
    pub fn new(level: ColorLevel, unicode: bool) -> Self {
        Theme { level, glyphs: if unicode { Glyphs::UNICODE } else { Glyphs::ASCII }, reduced_motion: false }
    }

    /// Detect from the environment (see module docs).
    pub fn detect() -> Self {
        let get = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let term = get("TERM").unwrap_or_default();
        let level = if std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()) || term == "dumb" {
            ColorLevel::None
        } else if get("COLORTERM").is_some_and(|c| c.contains("truecolor") || c.contains("24bit")) || get("WT_SESSION").is_some() {
            ColorLevel::TrueColor
        } else if term.contains("256color") {
            ColorLevel::Ansi256
        } else if cfg!(windows) && term.is_empty() {
            // Windows 10+ consoles understand 24-bit sequences once virtual terminal mode is on.
            ColorLevel::TrueColor
        } else {
            ColorLevel::Ansi16
        };
        let unicode = if get("KORA_ASCII").is_some() {
            false
        } else {
            let loc = ["LC_ALL", "LC_CTYPE", "LANG"].iter().find_map(|k| get(k));
            match loc {
                Some(l) => {
                    let l = l.to_ascii_lowercase();
                    l.contains("utf-8") || l.contains("utf8")
                }
                // No locale at all: unix terminals are UTF-8 today; only the legacy Windows
                // console (no Windows Terminal / ConEmu / modern host) is not.
                None => !cfg!(windows) || get("WT_SESSION").is_some() || get("ConEmuANSI").is_some() || get("TERM_PROGRAM").is_some() || !term.is_empty(),
            }
        };
        let mut t = Theme::new(level, unicode);
        t.reduced_motion = get("KORA_REDUCED_MOTION").is_some() || level == ColorLevel::None;
        t
    }

    // ---- palette --------------------------------------------------------------------------

    /// The Kora lime (`--lime`, #95ff00) or its closest stand-in.
    pub fn brand(&self) -> Color {
        match self.level {
            ColorLevel::TrueColor => Color::Rgb(0x95, 0xff, 0x00),
            ColorLevel::Ansi256 => Color::Indexed(118),
            ColorLevel::Ansi16 => Color::LightGreen,
            ColorLevel::None => Color::Reset,
        }
    }

    fn colored(&self) -> bool {
        self.level != ColorLevel::None
    }

    fn fg(&self, c: Color) -> Style {
        if self.colored() {
            Style::new().fg(c)
        } else {
            Style::new()
        }
    }

    // ---- semantic styles ------------------------------------------------------------------

    pub fn text(&self) -> Style {
        Style::new()
    }
    pub fn title(&self) -> Style {
        self.fg(self.brand()).add_modifier(Modifier::BOLD)
    }
    pub fn brand_style(&self) -> Style {
        self.fg(self.brand())
    }
    /// Focused / selected row or option.
    pub fn selected(&self) -> Style {
        self.fg(self.brand()).add_modifier(Modifier::BOLD)
    }
    pub fn label(&self) -> Style {
        Style::new().add_modifier(Modifier::BOLD)
    }
    pub fn disabled(&self) -> Style {
        if self.colored() {
            Style::new().fg(Color::DarkGray)
        } else {
            Style::new().add_modifier(Modifier::DIM)
        }
    }
    /// Secondary text (descriptions, hints).
    pub fn hint(&self) -> Style {
        if self.colored() {
            Style::new().fg(Color::Gray)
        } else {
            Style::new().add_modifier(Modifier::DIM)
        }
    }
    pub fn success(&self) -> Style {
        self.fg(Color::Green)
    }
    pub fn warn(&self) -> Style {
        self.fg(Color::Yellow)
    }
    pub fn danger(&self) -> Style {
        self.fg(Color::Red).add_modifier(Modifier::BOLD)
    }
    pub fn tone(&self, t: Tone) -> Style {
        match t {
            Tone::Ok => self.success(),
            Tone::Warn => self.warn(),
            Tone::Fail => self.danger(),
            Tone::Info => self.hint(),
        }
    }
    /// Panel border: bright when focused, quiet otherwise.
    pub fn border(&self, focused: bool) -> Style {
        if focused {
            self.fg(self.brand())
        } else if self.colored() {
            Style::new().fg(Color::DarkGray)
        } else {
            Style::new()
        }
    }
    /// A clickable control that is not the main one (`[ Paste ]`, Back, Help).
    pub fn button(&self) -> Style {
        if self.colored() {
            Style::new().fg(Color::Black).bg(Color::Gray)
        } else {
            Style::new().add_modifier(Modifier::REVERSED)
        }
    }
    /// The main action (Next / Install).
    pub fn button_primary(&self) -> Style {
        if self.colored() {
            Style::new().fg(Color::Black).bg(self.brand()).add_modifier(Modifier::BOLD)
        } else {
            Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        }
    }
    /// A control that needs attention (restart with permission, use suggested port).
    pub fn button_warn(&self) -> Style {
        if self.colored() {
            Style::new().fg(Color::Black).bg(Color::Yellow)
        } else {
            Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        }
    }
    /// The `KORA` wordmark chip.
    pub fn wordmark(&self) -> Style {
        self.button_primary()
    }

    // ---- borders & spacing ----------------------------------------------------------------

    pub fn border_set(&self) -> border::Set<'static> {
        if self.glyphs.unicode {
            border::Set {
                top_left: "╭",
                top_right: "╮",
                bottom_left: "╰",
                bottom_right: "╯",
                vertical_left: "│",
                vertical_right: "│",
                horizontal_top: "─",
                horizontal_bottom: "─",
            }
        } else {
            ASCII_BORDER
        }
    }

    /// A bordered panel with a padded title. `title` should not include padding.
    pub fn panel<'a>(&self, title: impl Into<Line<'a>>, focused: bool) -> Block<'a> {
        let mut line: Line<'a> = title.into();
        line.spans.insert(0, Span::raw(" "));
        line.spans.push(Span::raw(" "));
        Block::bordered().border_set(self.border_set()).border_style(self.border(focused)).title(line.style(self.title()))
    }

    /// Same as [`Theme::panel`] with a custom border colour (warnings, failures).
    pub fn panel_tone<'a>(&self, title: impl Into<Line<'a>>, tone: Tone) -> Block<'a> {
        let st = match tone {
            Tone::Ok => self.border(true),
            Tone::Info => self.border(false),
            Tone::Warn => self.warn(),
            Tone::Fail => self.fg(Color::Red),
        };
        let mut line: Line<'a> = title.into();
        line.spans.insert(0, Span::raw(" "));
        line.spans.push(Span::raw(" "));
        Block::bordered().border_set(self.border_set()).border_style(st).title(line.style(st.add_modifier(Modifier::BOLD)))
    }

    /// Plain rounded block (buttons, inputs) without a title.
    pub fn frame<'a>(&self, style: Style) -> Block<'a> {
        Block::bordered().border_set(self.border_set()).border_style(style)
    }

    /// One-cell horizontal breathing room for text inside panels.
    pub fn pad(&self) -> Padding {
        Padding::horizontal(1)
    }

    // ---- small spans ----------------------------------------------------------------------

    /// `[ ✓ OK ]`, `[ ! WARN ]`, `[ ✗ FAIL ]`: colour and symbol; the word alone survives NO_COLOR.
    pub fn chip(&self, tone: Tone, word: &str) -> Span<'static> {
        let g = &self.glyphs;
        let sym = match tone {
            Tone::Ok => g.ok,
            Tone::Warn => g.warn,
            Tone::Fail => g.fail,
            Tone::Info => g.bullet,
        };
        let text = if g.unicode { format!("[ {sym} {word} ]") } else { format!("[ {word} ]") };
        let mut st = self.tone(tone);
        if tone == Tone::Info {
            st = self.hint();
        }
        Span::styled(text, st.add_modifier(Modifier::BOLD))
    }

    /// A small dependency/info badge such as `needs ox-lib`.
    pub fn badge(&self, text: &str) -> Span<'static> {
        Span::styled(format!("<{text}>"), self.warn())
    }

    /// `key  label` chips for a footer, dropping the ones that do not fit in `width`.
    pub fn hints(&self, items: &[(&str, &str)], width: usize) -> Line<'static> {
        let key_style = if self.colored() { Style::new().fg(Color::Black).bg(Color::Gray) } else { Style::new().add_modifier(Modifier::REVERSED) };
        let mut spans: Vec<Span<'static>> = vec![];
        let mut used = 0usize;
        for (k, l) in items {
            let ascii_key;
            let k = if self.glyphs.unicode {
                k
            } else {
                ascii_key = k.replace("↑↓", "Up/Down").replace("←→", "Left/Right");
                ascii_key.as_str()
            };
            let need = k.chars().count() + 2 + 1 + l.chars().count() + 2;
            if used + need > width && !spans.is_empty() {
                break;
            }
            spans.push(Span::styled(format!(" {k} "), key_style));
            spans.push(Span::styled(format!(" {l}  "), self.hint()));
            used += need;
        }
        Line::from(spans)
    }

    /// `[##########----] 40%` style bar as text (ASCII-safe), used where a widget does not fit.
    pub fn bar(&self, ratio: f64, width: usize) -> String {
        let n = ((ratio.clamp(0.0, 1.0)) * width as f64).round() as usize;
        format!("{}{}", self.glyphs.gauge_full.repeat(n), self.glyphs.gauge_empty.repeat(width.saturating_sub(n)))
    }
}

// ---- global access -------------------------------------------------------------------------

thread_local! {
    static OVERRIDE: RefCell<Option<Theme>> = const { RefCell::new(None) };
}

static DETECTED: OnceLock<Theme> = OnceLock::new();

/// The active theme: the environment-detected one, or (tests) the deterministic
/// Unicode + true colour one / a [`with_theme`] override.
pub fn theme() -> Theme {
    if let Some(t) = OVERRIDE.with(|o| *o.borrow()) {
        return t;
    }
    if cfg!(test) {
        return Theme::new(ColorLevel::TrueColor, true);
    }
    *DETECTED.get_or_init(Theme::detect)
}

/// Run `f` with a specific theme on this thread (tests, dumps).
pub fn with_theme<R>(t: Theme, f: impl FnOnce() -> R) -> R {
    let prev = OVERRIDE.with(|o| o.replace(Some(t)));
    let r = f();
    OVERRIDE.with(|o| *o.borrow_mut() = prev);
    r
}

pub fn glyphs() -> Glyphs {
    theme().glyphs
}

// ---- layout helpers --------------------------------------------------------------------------

pub fn too_small(area: Rect) -> bool {
    area.width < MIN_WIDTH || area.height < MIN_HEIGHT
}

/// If the terminal is smaller than `min_w x min_h`, draw the "terminal too small" panel and
/// return `true` (the caller must not draw anything else).
pub fn guard_size(f: &mut Frame, min_w: u16, min_h: u16) -> bool {
    let area = f.area();
    if area.width >= min_w && area.height >= min_h {
        return false;
    }
    let t = theme();
    f.render_widget(Clear, area);
    let lines = vec![
        Line::styled("Terminal too small", t.danger()),
        Line::raw(""),
        Line::raw(format!("Kora needs at least {min_w}x{min_h}; this window is {}x{}.", area.width, area.height)),
        Line::styled("Make the window bigger (or reduce the font size).", t.hint()),
        Line::styled("Ctrl+C quits.", t.hint()),
    ];
    let w = area.width.min(54);
    let h = (lines.len() as u16 + 2).min(area.height);
    let r = centered(area, w, h);
    let p = Paragraph::new(lines).alignment(Alignment::Center).wrap(Wrap { trim: true }).block(t.panel_tone("Kora", Tone::Warn).padding(Padding::horizontal(1)));
    f.render_widget(p, r);
    true
}

/// A `w x h` rectangle centred in `area` (clamped to it).
pub fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let [r] = Layout::horizontal([Constraint::Length(w.min(area.width))]).flex(Flex::Center).areas(area);
    let [r] = Layout::vertical([Constraint::Length(h.min(area.height))]).flex(Flex::Center).areas(r);
    r
}

/// Modal dialog over the current screen: clears its area, then draws a toned panel.
pub fn modal(f: &mut Frame, area: Rect, title: &str, lines: Vec<Line<'static>>, tone: Tone, max_w: u16) {
    let t = theme();
    let w = area.width.saturating_sub(8).clamp(1, max_w);
    let inner_w = w.saturating_sub(4).max(1) as usize;
    let rows: usize = lines.iter().map(|l| (l.width().max(1)).div_ceil(inner_w)).sum();
    let h = (rows as u16 + 2).min(area.height);
    let r = centered(area, w, h).intersection(area);
    f.render_widget(Clear, r);
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(t.panel_tone(title.to_string(), tone).padding(Padding::horizontal(1))), r);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn no_color_theme_carries_state_without_colour() {
        let t = Theme::new(ColorLevel::None, true);
        for s in [t.title(), t.selected(), t.danger(), t.success(), t.button(), t.button_primary(), t.border(true), t.disabled()] {
            assert_eq!(s.fg, None);
            assert_eq!(s.bg, None);
        }
        assert!(t.selected().add_modifier.contains(Modifier::BOLD));
        assert!(t.button_primary().add_modifier.contains(Modifier::REVERSED));
        assert!(t.disabled().add_modifier.contains(Modifier::DIM));
    }

    #[test]
    fn brand_degrades_with_colour_depth() {
        assert_eq!(Theme::new(ColorLevel::TrueColor, true).brand(), Color::Rgb(0x95, 0xff, 0x00));
        assert_eq!(Theme::new(ColorLevel::Ansi256, true).brand(), Color::Indexed(118));
        assert_eq!(Theme::new(ColorLevel::Ansi16, true).brand(), Color::LightGreen);
    }

    #[test]
    fn ascii_glyphs_are_ascii() {
        let g = Glyphs::ASCII;
        for s in [g.ok, g.fail, g.warn, g.pending, g.skipped, g.bullet, g.radio_on, g.radio_off, g.pointer, g.crumb, g.cursor, g.arrow, g.ellipsis, g.dot_on, g.dot_off, g.rule, g.vbar, g.gauge_full, g.gauge_empty] {
            assert!(s.is_ascii(), "{s:?}");
        }
        assert!(g.spinner.iter().all(|s| s.is_ascii()));
        let t = Theme::new(ColorLevel::None, false);
        assert!(t.border_set().top_left.is_ascii());
        assert!(t.chip(Tone::Fail, "FAIL").content.is_ascii());
    }

    #[test]
    fn chips_keep_the_word() {
        let t = Theme::new(ColorLevel::None, true);
        assert!(t.chip(Tone::Ok, "OK").content.contains("OK"));
        assert!(t.chip(Tone::Warn, "WARN").content.contains('!'));
    }

    #[test]
    fn hints_drop_what_does_not_fit() {
        let t = theme();
        let l = t.hints(&[("Enter", "next"), ("Esc", "back"), ("F1", "help")], 20);
        let s: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(s.contains("Enter") && !s.contains("help"), "{s}");
    }

    #[test]
    fn too_small_panel_instead_of_panic() {
        for (w, h) in [(60, 15), (10, 3), (1, 1), (0, 0)] {
            let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
            t.draw(|f| {
                assert!(guard_size(f, MIN_WIDTH, MIN_HEIGHT));
            })
            .unwrap();
        }
        let mut t = Terminal::new(TestBackend::new(60, 15)).unwrap();
        t.draw(|f| {
            guard_size(f, MIN_WIDTH, MIN_HEIGHT);
        })
        .unwrap();
        let s: String = t.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(s.contains("Terminal too small") && s.contains("80x24") && s.contains("60x15"));
    }
}
