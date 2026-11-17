//! The audit log: one line per privileged operation, allowed or refused. Pure formatting and the
//! size rule; the service appends the lines to a root-owned file (Windows: `audit.log` in the
//! protected service folder; Linux: `/var/log/kora/audit.log`, also visible in the journal).
//!
//! A line holds: when (UTC), which account (the OS identity, plus its name when known), the
//! operation's short label ([`super::op::Op::label`]: operation and the server's short name, never
//! a path), and the outcome. It never holds an argument value, an environment value, a licence
//! key or a password: the caller does not influence anything in the line except the server name,
//! which the parser has restricted to letters, digits, space, dot, dash and underscore.

use super::policy::AccountId;

/// The log is cut to its newest half when it grows past this.
pub const MAX_LOG_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Ran and succeeded.
    Done,
    /// Ran and failed (the child's exit code).
    Failed(i32),
    /// Stopped after its time limit.
    TimedOut,
    /// Not run: the reason's short code (never free text from the caller).
    Refused(&'static str),
}

fn clean(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).map(|c| if c == '|' { '/' } else { c }).take(max).collect()
}

/// `2026-11-17T10:00:00Z | uid:1000 (jo) | run system.firewall [my-server] | done`
pub fn line(unix_secs: u64, who: Option<&AccountId>, name: &str, what: &str, outcome: &Outcome) -> String {
    let who = match who {
        Some(id) if !name.is_empty() => format!("{id} ({})", clean(name, 48)),
        Some(id) => id.to_string(),
        None => "unknown".into(),
    };
    let result = match outcome {
        Outcome::Done => "done".to_string(),
        Outcome::Failed(c) => format!("failed (code {c})"),
        Outcome::TimedOut => "stopped (time limit)".into(),
        Outcome::Refused(why) => format!("refused: {why}"),
    };
    format!("{} | {who} | {} | {result}", rfc3339(unix_secs), clean(what, 96))
}

/// UTC time as `YYYY-MM-DDTHH:MM:SSZ` (no time-zone database needed).
pub fn rfc3339(secs: u64) -> String {
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // civil-from-days (Howard Hinnant)
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Should the log be cut now?
pub fn needs_trim(size: u64) -> bool {
    size > MAX_LOG_BYTES
}

/// The newest half of `text`, starting at a line boundary.
pub fn trimmed(text: &str) -> &str {
    let from = text.len() / 2;
    let start = if text[..from].ends_with('\n') { from } else { text[from..].find('\n').map(|i| from + i + 1).unwrap_or(text.len()) };
    &text[start..]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(1_794_909_000), "2026-11-17T09:50:00Z");
    }

    #[test]
    fn a_line_has_who_what_and_the_result_and_no_free_text() {
        let id = AccountId::uid(1000);
        let l = line(0, Some(&id), "jo", "run system.firewall [my-server]", &Outcome::Done);
        assert_eq!(l, "1970-01-01T00:00:00Z | uid:1000 (jo) | run system.firewall [my-server] | done");
        assert!(line(0, Some(&id), "", "x", &Outcome::Failed(3)).ends_with("failed (code 3)"));
        assert!(line(0, None, "", "x", &Outcome::Refused("not allowed")).contains("unknown | x | refused: not allowed"));
        // a name cannot forge a second line or a column
        let l = line(0, Some(&id), "evil\n| done", "a\r\nb | c", &Outcome::TimedOut);
        assert!(!l.contains('\n') && !l.contains('\r'));
        assert_eq!(l.matches(" | ").count(), 3);
    }

    #[test]
    fn trimming_keeps_whole_newest_lines() {
        let text = "aaaa\nbbbb\ncccc\ndddd\n";
        assert_eq!(trimmed(text), "cccc\ndddd\n");
        assert_eq!(trimmed("one line only"), "");
        assert!(needs_trim(MAX_LOG_BYTES + 1) && !needs_trim(MAX_LOG_BYTES));
    }
}
