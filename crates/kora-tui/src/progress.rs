//! Progress + outcome screens. State and rendering are pure; `run_*` wrap the terminal.

use kora_core::installer::{InstallOutcome, ProgressEvent, StepStatus};
use ratatui::{
    layout::{Alignment, Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph},
    Frame,
};

use crate::wizard_view::{wrap, BRAND};

const SPINNER: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];

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
    let area = f.area();
    let [head, gauge, list, detail] =
        Layout::vertical([Constraint::Length(2), Constraint::Length(3), Constraint::Min(0), Constraint::Length(3)]).areas(area);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" KORA ", Style::default().fg(Color::Black).bg(BRAND).add_modifier(Modifier::BOLD)),
            Span::raw("  Setting up your server. This can take a few minutes; please keep this window open."),
        ])),
        head,
    );
    f.render_widget(
        Gauge::default()
            .block(Block::default().borders(Borders::ALL).title(" Overall progress "))
            .gauge_style(Style::default().fg(if st.failed() { Color::Red } else { BRAND }))
            .percent(u16::from(st.percent))
            .label(format!("{}%", st.percent)),
        gauge,
    );
    let lines: Vec<Line> = st
        .steps
        .iter()
        .map(|(_, title, status, _)| {
            let (mark, style) = match status {
                StepStatus::Pending => ("·".to_string(), Style::default().fg(Color::DarkGray)),
                StepStatus::Running => (SPINNER[st.tick % SPINNER.len()].to_string(), Style::default().fg(BRAND)),
                StepStatus::Done => ("✓".to_string(), Style::default().fg(Color::Green)),
                StepStatus::Skipped => ("-".to_string(), Style::default().fg(Color::DarkGray)),
                StepStatus::Failed => ("✗".to_string(), Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            };
            Line::from(vec![Span::styled(format!(" {mark} "), style), Span::styled(title.clone(), style)])
        })
        .collect();
    // Keep the newest steps visible when the list is taller than the area.
    let h = list.height.saturating_sub(2) as usize;
    let skip = lines.len().saturating_sub(h);
    f.render_widget(
        Paragraph::new(lines.into_iter().skip(skip).collect::<Vec<_>>())
            .block(Block::default().borders(Borders::ALL).title(" Steps ")),
        list,
    );
    let text = if finished && st.failed() {
        "Something went wrong. Press any key to see what to do next.".to_string()
    } else {
        st.detail.clone()
    };
    f.render_widget(
        Paragraph::new(text).block(Block::default().borders(Borders::ALL).title(" What's happening now ")),
        detail,
    );
}

pub fn render_outcome(f: &mut Frame, o: &InstallOutcome) {
    let area = f.area();
    let mut lines = vec![
        Line::styled("Your server is ready!", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Line::raw(""),
        Line::raw(format!("Folder:            {}", o.dir.display())),
        Line::raw(format!("Players connect:   {}", o.connect)),
        Line::raw(format!("Control panel:     {}", o.txadmin_url)),
        Line::raw(""),
    ];
    let w = area.width.saturating_sub(4) as usize;
    for n in &o.notes {
        for (i, l) in wrap(n, w.saturating_sub(2)).into_iter().enumerate() {
            lines.push(Line::raw(format!("{}{l}", if i == 0 { "• " } else { "  " })));
        }
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled("Press any key to finish.", Style::default().fg(Color::DarkGray)));
    f.render_widget(
        Paragraph::new(lines).alignment(Alignment::Left).block(
            Block::default().borders(Borders::ALL).border_style(Style::default().fg(BRAND)).title(" Kora - all done "),
        ),
        area,
    );
}

/// Plain-language error screen shown after a failed install.
pub fn render_failure(f: &mut Frame, message: &str, next: &str, log: Option<&std::path::Path>) {
    let area = f.area();
    let w = area.width.saturating_sub(4) as usize;
    let mut lines = vec![
        Line::styled("The install could not finish", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
        Line::raw(""),
    ];
    for l in wrap(message, w) {
        lines.push(Line::raw(l));
    }
    lines.push(Line::raw(""));
    for l in wrap(next, w) {
        lines.push(Line::raw(l));
    }
    if let Some(p) = log {
        lines.push(Line::raw(""));
        for l in wrap(&format!("Details were saved in: {}", p.display()), w) {
            lines.push(Line::raw(l));
        }
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled("Press any key to exit.", Style::default().fg(Color::DarkGray)));
    f.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Red)).title(" Kora - something went wrong ")),
        area,
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

    #[test]
    fn failure_renders_message_and_log() {
        let mut t = Terminal::new(TestBackend::new(80, 20)).unwrap();
        let log = std::path::PathBuf::from("/srv/kora/.kora/install.log");
        t.draw(|f| render_failure(f, "Port 30120 is already in use.", "Fix it and run again.", Some(&log))).unwrap();
        let out = text(&t);
        assert!(out.contains("could not finish") && out.contains("Port 30120") && out.contains("install.log"));
        let mut t = Terminal::new(TestBackend::new(8, 3)).unwrap();
        t.draw(|f| render_failure(f, "x", "y", None)).unwrap();
    }

    #[test]
    fn outcome_renders() {
        let o = InstallOutcome {
            dir: "/srv/kora".into(),
            connect: "connect 1.2.3.4:30120".into(),
            txadmin_url: "http://localhost:40120".into(),
            notes: vec!["Forward port 30120 on your router.".into()],
        };
        let mut t = Terminal::new(TestBackend::new(80, 20)).unwrap();
        t.draw(|f| render_outcome(f, &o)).unwrap();
        assert!(text(&t).contains("ready"));
        let mut t = Terminal::new(TestBackend::new(8, 3)).unwrap();
        t.draw(|f| render_outcome(f, &o)).unwrap();
    }
}
