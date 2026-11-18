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

/// A line of the log, taken apart (what the panel's Activity log shows).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub at: u64,
    pub account: String,
    pub what: String,
    /// `done`, `failed`, `refused` or `stopped`.
    pub result: &'static str,
    pub detail: Option<String>,
}

/// `YYYY-MM-DDTHH:MM:SSZ` to unix seconds (the inverse of [`rfc3339`]).
pub fn parse_rfc3339(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    if b.len() != 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' || b[19] != b'Z' {
        return None;
    }
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, m, d, h, mi, sec) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    // days-from-civil (Howard Hinnant)
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + h * 3600 + mi * 60 + sec).ok()
}

/// Take a line apart. `None` for anything that is not a line [`line`] wrote.
pub fn parse_line(l: &str) -> Option<Parsed> {
    if !is_audit_line(l) {
        return None;
    }
    let mut it = l.splitn(4, " | ");
    let (when, who, what, result) = (it.next()?, it.next()?, it.next()?, it.next()?);
    let account = match who.split_once(" (") {
        Some((_, name)) => name.trim_end_matches(')').to_string(),
        None => who.to_string(),
    };
    let (result, detail) = if result == "done" {
        ("done", None)
    } else if let Some(rest) = result.strip_prefix("failed") {
        ("failed", Some(rest.trim().trim_matches(['(', ')']).to_string()).filter(|d| !d.is_empty()))
    } else if let Some(why) = result.strip_prefix("refused:") {
        ("refused", Some(why.trim().to_string()).filter(|d| !d.is_empty()))
    } else if result.starts_with("stopped") {
        ("stopped", Some("it took longer than its time limit".to_string()))
    } else {
        return None;
    };
    Some(Parsed { at: parse_rfc3339(when)?, account, what: what.to_string(), result, detail })
}

/// At most this many lines are handed out in one read, and at most this many bytes of the file are looked at.
pub const READ_MAX_LINES: usize = 500;
pub const READ_MAX_BYTES: u64 = 512 * 1024;
const READ_LINE_CAP: usize = 300;

/// Is this a line [`line`] wrote (starts with a UTC time)?
fn is_audit_line(l: &str) -> bool {
    let b = l.as_bytes();
    b.len() > 22 && b[4] == b'-' && b[7] == b'-' && b[10] == b'T' && b[19] == b'Z' && b[20] == b' ' && b[..4].iter().all(u8::is_ascii_digit)
}

/// The account a line is about: its second column (`uid:1000 (jo)` -> `uid:1000`).
fn line_account(l: &str) -> Option<&str> {
    l.split(" | ").nth(1).map(|w| w.split(" (").next().unwrap_or(w).trim())
}

/// The lines `me` may see, oldest first, at most `limit` of the newest: only the lines about the calling account, unless the
/// caller is an administrator of the machine (`admin`: then all of them). Every line is cut to a fixed length, control
/// characters never get through, and a line is also passed through the support bundle's redaction as a second fence (the log is
/// written without secrets by construction, see the module docs). Reading the log is itself not written to it.
pub fn visible_lines(text: &str, me: &str, admin: bool, limit: usize) -> Vec<String> {
    let limit = limit.clamp(1, READ_MAX_LINES);
    let mut out: Vec<String> = text
        .lines()
        .rev()
        .filter(|l| is_audit_line(l) && (admin || line_account(l).is_some_and(|a| a.eq_ignore_ascii_case(me))))
        .take(limit)
        .map(|l| crate::support::redact::scrub(&l.chars().filter(|c| !c.is_control()).take(READ_LINE_CAP).collect::<String>()).text)
        .collect();
    out.reverse();
    out
}

/// The last `max` bytes of the log file (a whole-line start), or nothing when there is no log yet.
pub fn read_tail(path: &std::path::Path, max: u64) -> std::io::Result<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(e) => return Err(e),
    };
    let len = f.metadata()?.len();
    let cut = len > max;
    f.seek(SeekFrom::Start(len.saturating_sub(max)))?;
    let mut buf = Vec::new();
    f.take(max).read_to_end(&mut buf)?;
    let text = String::from_utf8_lossy(&buf).into_owned();
    // Starting in the middle of the file: drop the partial first line.
    Ok(if cut { text.split_once('\n').map(|(_, rest)| rest.to_string()).unwrap_or_default() } else { text })
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

    const LOG: &str = "2026-11-17T09:00:00Z | uid:1000 (jo) | run system.firewall [a] | done\n\
                       2026-11-17T09:01:00Z | uid:1001 (sam) | run system.service [b] | failed (code 1)\n\
                       not a log line\n\
                       2026-11-17T09:02:00Z | uid:1000 (jo) | backup job set [a] | done\n\
                       2026-11-17T09:03:00Z | uid:10000 (sue) | undo system.service | refused: checks failed\n";

    #[test]
    fn a_caller_sees_only_the_lines_about_their_own_account_unless_they_administer_the_machine() {
        let jo = visible_lines(LOG, "uid:1000", false, 100);
        assert_eq!(jo.len(), 2);
        assert!(jo.iter().all(|l| l.contains("uid:1000 (jo)")), "{jo:?}");
        assert!(visible_lines(LOG, "uid:1", false, 100).is_empty(), "a uid that only starts the same is not the same account");
        assert_eq!(visible_lines(LOG, "uid:1000", true, 100).len(), 4, "an administrator sees every line (and never junk)");
        // newest `limit`, oldest first
        let two = visible_lines(LOG, "uid:1000", true, 2);
        assert!(two[0].contains("09:02:00Z") && two[1].contains("09:03:00Z"));
        assert_eq!(visible_lines(LOG, "uid:1000", true, 0).len(), 1, "at least one");
        // SIDs compare without regard to case
        assert_eq!(visible_lines("2026-11-17T09:00:00Z | S-1-5-21-1-2-3-1001 (Jo) | x | done\n", "s-1-5-21-1-2-3-1001", false, 5).len(), 1);
    }

    #[test]
    fn a_long_or_hostile_line_is_cut_cleaned_and_scrubbed() {
        let long = format!("2026-11-17T09:00:00Z | uid:1000 (jo) | {} | done\n", "x".repeat(2000));
        assert!(visible_lines(&long, "uid:1000", false, 5)[0].chars().count() <= READ_LINE_CAP);
        let key = "2026-11-17T09:00:00Z | uid:1000 (jo) | run x | done CFX_LICENSE_KEY=cfxk_abcdefghijklmnopqrstuvwxyz0123456789_ABCDE\n";
        assert!(!visible_lines(key, "uid:1000", false, 5)[0].contains("cfxk_abcdefghijklmnop"));
        let many: String = (0..2000).map(|i| format!("2026-11-17T09:00:{:02}Z | uid:1000 (jo) | run x | done\n", i % 60)).collect();
        assert_eq!(visible_lines(&many, "uid:1000", false, 100_000).len(), READ_MAX_LINES);
    }

    #[test]
    fn only_the_tail_of_a_big_file_is_read_and_a_missing_file_is_empty() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("audit.log");
        assert_eq!(read_tail(&p, 100).unwrap(), "");
        std::fs::write(&p, LOG).unwrap();
        assert_eq!(read_tail(&p, 10_000).unwrap(), LOG);
        let t = read_tail(&p, 120).unwrap();
        assert!(t.starts_with("2026-11-17T09:03") || t.starts_with("2026-11-17T09:02") || t.is_empty() || t.starts_with("not a log"), "{t:?}");
        assert!(t.len() <= 120 && !t.starts_with("26-"), "no partial first line: {t:?}");
    }

    #[test]
    fn a_line_is_taken_apart_again_and_the_time_round_trips() {
        for t in [0u64, 951_782_400, 1_794_909_000, 4_102_444_799] {
            assert_eq!(parse_rfc3339(&rfc3339(t)), Some(t), "{t}");
        }
        assert_eq!(parse_rfc3339("2026-13-01T00:00:00Z"), None);
        assert_eq!(parse_rfc3339("yesterday"), None);
        let id = AccountId::uid(1000);
        let p = parse_line(&line(1_794_909_000, Some(&id), "jo", "run system.firewall [my-server]", &Outcome::Done)).unwrap();
        assert_eq!((p.at, p.account.as_str(), p.what.as_str(), p.result, p.detail), (1_794_909_000, "jo", "run system.firewall [my-server]", "done", None));
        let p = parse_line(&line(5, Some(&id), "", "x", &Outcome::Failed(3))).unwrap();
        assert_eq!((p.account.as_str(), p.result, p.detail.as_deref()), ("uid:1000", "failed", Some("code 3")));
        let p = parse_line(&line(5, None, "", "x", &Outcome::Refused("account not allowed"))).unwrap();
        assert_eq!((p.account.as_str(), p.result, p.detail.as_deref()), ("unknown", "refused", Some("account not allowed")));
        assert_eq!(parse_line(&line(5, Some(&id), "jo", "x", &Outcome::TimedOut)).unwrap().result, "stopped");
        assert_eq!(parse_line("not a log line"), None);
        assert_eq!(parse_line("2026-11-17T09:00:00Z | uid:1000 (jo) | x | exploded"), None);
    }

    #[test]
    fn trimming_keeps_whole_newest_lines() {
        let text = "aaaa\nbbbb\ncccc\ndddd\n";
        assert_eq!(trimmed(text), "cccc\ndddd\n");
        assert_eq!(trimmed("one line only"), "");
        assert!(needs_trim(MAX_LOG_BYTES + 1) && !needs_trim(MAX_LOG_BYTES));
    }
}
