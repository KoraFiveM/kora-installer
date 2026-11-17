//! A tiny Server-Sent-Events reader for the panel's `GET /api/v1/events` (`event: servers` + `data: {json}`).
//! Fed one line at a time so it can be tested without a network.

/// One complete event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub name: String,
    pub data: String,
}

#[derive(Debug, Default)]
pub struct Parser {
    name: String,
    data: Vec<String>,
}

/// Largest event the shell accepts (a list of servers is a few kilobytes).
pub const MAX_EVENT: usize = 512 * 1024;

impl Parser {
    /// Feed one line (without its line ending). A blank line ends an event.
    pub fn line(&mut self, line: &str) -> Option<Event> {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            if self.data.is_empty() {
                self.name.clear();
                return None;
            }
            let ev = Event { name: if self.name.is_empty() { "message".into() } else { std::mem::take(&mut self.name) }, data: self.data.join("\n") };
            self.name.clear();
            self.data.clear();
            return Some(ev);
        }
        if line.starts_with(':') {
            return None; // a comment: the keep-alive
        }
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line, ""),
        };
        match field {
            "event" => self.name = value.to_string(),
            "data" if self.data.iter().map(String::len).sum::<usize>() + value.len() <= MAX_EVENT => self.data.push(value.to_string()),
            _ => {}
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(lines: &[&str]) -> Vec<Event> {
        let mut p = Parser::default();
        lines.iter().filter_map(|l| p.line(l)).collect()
    }

    #[test]
    fn events_come_out_whole() {
        let ev = feed(&["event: servers", "data: {\"servers\":[]}", "", ": keep-alive", "", "data: a", "data: b", ""]);
        assert_eq!(ev, vec![Event { name: "servers".into(), data: "{\"servers\":[]}".into() }, Event { name: "message".into(), data: "a\nb".into() }]);
    }

    #[test]
    fn crlf_comments_and_stray_blank_lines_are_fine() {
        assert_eq!(feed(&["", "", "event: x\r", "data: y\r", "\r"]), vec![Event { name: "x".into(), data: "y".into() }]);
        assert!(feed(&["event: lonely", ""]).is_empty(), "no data, no event");
        assert_eq!(feed(&["id: 4", "retry: 10", "data:z", ""])[0].data, "z");
    }

    #[test]
    fn an_absurdly_large_event_is_cut_not_buffered() {
        let big = format!("data: {}", "x".repeat(MAX_EVENT));
        let ev = feed(&[&big, "data: more", ""]);
        assert_eq!(ev.len(), 1);
        assert!(ev[0].data.len() <= MAX_EVENT + 10);
    }
}
