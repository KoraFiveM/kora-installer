//! Progress + outcome screens. State and rendering are pure; `run_*` wrap the terminal.

use kora_core::installer::{ProgressEvent, StepStatus};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Gauge, LineGauge, Padding, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
    Frame,
};

use crate::theme::{self, theme, Tone};

#[derive(Debug, Default, Clone)]
pub struct ProgressState {
    pub steps: Vec<(String, String, StepStatus, String)>, // id, title, status, detail
    pub percent: u8,
    pub detail: String,
    pub tick: usize,
}

impl ProgressState {
    pub fn apply(&mut self, ev: ProgressEvent) {
        self.percent = ev.percent.min(100);
        if !ev.detail.is_empty() {
            self.detail = ev.detail.clone();
        }
        match self.steps.iter_mut().find(|s| s.0 == ev.step) {
            Some(s) => {
                s.1 = ev.title;
                s.2 = ev.status;
                s.3 = ev.detail;
            }
            None => self.steps.push((ev.step, ev.title, ev.status, ev.detail)),
        }
    }

    pub fn failed(&self) -> bool {
        self.steps.iter().any(|s| s.2 == StepStatus::Failed)
    }
}

pub fn render_progress(f: &mut Frame, st: &ProgressState, finished: bool) {
    if theme::guard_size(f, theme::MIN_WIDTH, theme::MIN_HEIGHT) {
        return;
    }
    let t = theme();
    let g = t.glyphs;
    let area = f.area();
    let [head, gauge, list, detail] =
        Layout::vertical([Constraint::Length(2), Constraint::Length(3), Constraint::Min(0), Constraint::Length(4)]).areas(area);
    f.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(" KORA ", t.wordmark()),
                Span::styled(format!("  {}  ", g.crumb), t.disabled()),
                Span::styled("Installing", t.selected()),
            ]),
            Line::styled("Setting up your server. This can take a few minutes; please keep this window open.", t.hint()),
        ]),
        head,
    );
    let failed = st.failed();
    let bar = if failed { t.danger() } else { t.brand_style() };
    let block = t.panel_tone("Overall progress", if failed { Tone::Fail } else { Tone::Ok });
    if g.unicode {
        f.render_widget(
            Gauge::default()
                .block(block)
                .gauge_style(bar)
                .percent(u16::from(st.percent))
                .label(Span::styled(format!("{}%", st.percent), Style::new().add_modifier(ratatui::style::Modifier::BOLD))),
            gauge,
        );
    } else {
        // ASCII terminals: the Gauge block characters are not safe, so draw `####----  40%`.
        let inner = block.inner(gauge);
        f.render_widget(block, gauge);
        f.render_widget(
            LineGauge::default()
                .ratio(f64::from(st.percent) / 100.0)
                .filled_symbol(g.gauge_full)
                .unfilled_symbol(g.gauge_empty)
                .filled_style(bar)
                .unfilled_style(t.disabled())
                .label(format!("{}%", st.percent)),
            inner,
        );
    }
    let lines: Vec<Line> = st
        .steps
        .iter()
        .map(|(_, title, status, _)| {
            let (mark, style, word) = match status {
                StepStatus::Pending => (g.pending, t.disabled(), None),
                StepStatus::Running => (g.spin(st.tick), t.brand_style(), None),
                StepStatus::Done => (g.ok, t.success(), None),
                StepStatus::Skipped => (g.skipped, t.disabled(), Some("skipped")),
                StepStatus::Failed => (g.fail, t.danger(), None),
            };
            let mut spans = vec![Span::styled(format!(" {mark} "), style), Span::styled(title.clone(), if *status == StepStatus::Running { t.selected() } else { style })];
            match status {
                StepStatus::Failed => {
                    spans.push(Span::raw("  "));
                    spans.push(t.chip(Tone::Fail, "FAIL"));
                }
                StepStatus::Skipped => spans.push(Span::styled(format!("  ({})", word.unwrap_or("")), t.disabled())),
                _ => {}
            }
            Line::from(spans)
        })
        .collect();
    // Keep the newest steps visible when the list is taller than the area.
    let h = list.height.saturating_sub(2) as usize;
    let total = lines.len();
    let skip = total.saturating_sub(h);
    f.render_widget(
        Paragraph::new(lines.into_iter().skip(skip).collect::<Vec<_>>()).block(t.panel(format!("Steps ({}/{})", st.steps.iter().filter(|s| matches!(s.2, StepStatus::Done | StepStatus::Skipped)).count(), total), true)),
        list,
    );
    if skip > 0 {
        let mut sb = ScrollbarState::new(total.saturating_sub(h)).position(skip);
        let r = Rect::new(list.x, list.y + 1, list.width, list.height.saturating_sub(2));
        f.render_stateful_widget(Scrollbar::new(ScrollbarOrientation::VerticalRight).begin_symbol(None).end_symbol(None), r, &mut sb);
    }
    let (text, style, tone) = if finished && failed {
        ("Something went wrong. Press any key to see what to do next.".to_string(), t.danger(), Tone::Fail)
    } else {
        (st.detail.clone(), t.text(), Tone::Info)
    };
    f.render_widget(
        Paragraph::new(text).style(style).wrap(Wrap { trim: true }).block(t.panel_tone("What's happening now", tone).padding(Padding::horizontal(1))),
        detail,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    fn ev(step: &str, status: StepStatus, pct: u8) -> ProgressEvent {
        ProgressEvent { step: step.into(), title: format!("Title {step}"), status, detail: format!("detail {step}"), percent: pct }
    }

    fn text(t: &Terminal<TestBackend>) -> String {
        format!("{:?}", t.backend().buffer())
    }

    #[test]
    fn apply_updates_in_place() {
        let mut s = ProgressState::default();
        s.apply(ev("a", StepStatus::Running, 10));
        s.apply(ev("a", StepStatus::Done, 30));
        s.apply(ev("b", StepStatus::Failed, 200));
        assert_eq!(s.steps.len(), 2);
        assert_eq!(s.steps[0].2, StepStatus::Done);
        assert_eq!(s.percent, 100);
        assert!(s.failed());
    }

    #[test]
    fn progress_renders_all_statuses() {
        let mut s = ProgressState::default();
        for (i, st) in
            [StepStatus::Pending, StepStatus::Running, StepStatus::Done, StepStatus::Skipped, StepStatus::Failed].into_iter().enumerate()
        {
            s.apply(ev(&format!("s{i}"), st, 40));
        }
        for (w, h) in [(80, 24), (30, 8), (10, 3)] {
            let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
            t.draw(|f| render_progress(f, &s, true)).unwrap();
        }
        let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
        t.draw(|f| render_progress(f, &s, false)).unwrap();
        let out = text(&t);
        assert!(out.contains("Title s2") && out.contains("40%"));
    }
}
