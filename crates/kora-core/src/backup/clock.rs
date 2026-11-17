//! Dates without a date library: the calendar maths for backup ids and schedules, and the machine's
//! offset from UTC (the schedule says "03:30 local time").

/// (year, month 1-12, day 1-31) of a day number counted from 1970-01-01 (proleptic Gregorian).
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 0 = Monday .. 6 = Sunday, of a day number counted from 1970-01-01 (a Thursday).
pub fn weekday_of_days(days: i64) -> u8 {
    ((days + 3).rem_euclid(7)) as u8
}

/// `20260317-041507`: the id of a backup made at `unix` (UTC, so ids sort the same everywhere).
pub fn stamp(unix: u64) -> String {
    let days = (unix / 86_400) as i64;
    let secs = unix % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", secs / 3600, (secs / 60) % 60, secs % 60)
}

/// "2026-03-17 04:15 UTC", for the support bundle and logs.
pub fn utc_text(unix: u64) -> String {
    let days = (unix / 86_400) as i64;
    let secs = unix % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02} UTC", secs / 3600, (secs / 60) % 60)
}

/// Seconds east of UTC at `unix` on this machine (0 when it cannot be told).
pub fn local_offset_secs(unix: u64) -> i64 {
    offset_os(unix)
}

#[cfg(unix)]
fn offset_os(unix: u64) -> i64 {
    let t = unix as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: `t` is a valid time_t and `tm` a properly sized out-parameter; localtime_r is thread-safe.
    let r = unsafe { libc::localtime_r(&t, &mut tm) };
    if r.is_null() {
        0
    } else {
        tm.tm_gmtoff as i64
    }
}

#[cfg(windows)]
fn offset_os(_unix: u64) -> i64 {
    #[repr(C)]
    struct SystemTime {
        w: [u16; 8],
    }
    #[repr(C)]
    struct Tzi {
        bias: i32,
        standard_name: [u16; 32],
        standard_date: SystemTime,
        standard_bias: i32,
        daylight_name: [u16; 32],
        daylight_date: SystemTime,
        daylight_bias: i32,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetTimeZoneInformation(info: *mut Tzi) -> u32;
    }
    // SAFETY: a zeroed plain-data struct of the layout the API documents (TIME_ZONE_INFORMATION).
    let mut tz: Tzi = unsafe { std::mem::zeroed() };
    let id = unsafe { GetTimeZoneInformation(&mut tz) };
    let minutes = match id {
        2 => tz.bias + tz.daylight_bias,
        _ => tz.bias + tz.standard_bias,
    };
    -(minutes as i64) * 60
}

/// The most recent moment at or before `now` that matches the schedule (unix seconds), given the
/// machine's offset: `time` = "HH:MM" local; `weekday` = Some(0..=6) for weekly.
pub fn previous_slot(now: u64, offset: i64, hour: u32, minute: u32, weekday: Option<u8>) -> u64 {
    let local_now = now as i64 + offset;
    let mut day = local_now.div_euclid(86_400);
    let tod = local_now.rem_euclid(86_400);
    let at = i64::from(hour) * 3600 + i64::from(minute) * 60;
    if tod < at {
        day -= 1;
    }
    if let Some(w) = weekday {
        let back = (i64::from(weekday_of_days(day)) - i64::from(w)).rem_euclid(7);
        day -= back;
    }
    (day * 86_400 + at - offset).max(0) as u64
}

/// "just now", "5 minutes ago", "3 hours ago", "yesterday", "3 days ago", "5 weeks ago".
pub fn ago(secs: u64) -> String {
    let plural = |n: u64, unit: &str| if n == 1 { format!("1 {unit} ago") } else { format!("{n} {unit}s ago") };
    match secs {
        0..=59 => "just now".into(),
        60..=3599 => plural(secs / 60, "minute"),
        3600..=86_399 => plural(secs / 3600, "hour"),
        86_400..=172_799 => "yesterday".into(),
        172_800..=1_209_599 => plural(secs / 86_400, "day"),
        _ => plural(secs / 604_800, "week"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_maths() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(weekday_of_days(0), 3, "1970-01-01 was a Thursday");
        assert_eq!(weekday_of_days(19_723), 0, "2024-01-01 was a Monday");
        assert_eq!(stamp(1_700_000_000), "20231114-221320");
        assert_eq!(utc_text(1_700_000_000), "2023-11-14 22:13 UTC");
    }

    #[test]
    fn the_previous_slot_is_today_or_yesterday() {
        // 2023-11-14 22:13:20 UTC, machine at UTC+1 (23:13 local), daily 03:30 → today 03:30 local = 02:30 UTC
        let now = 1_700_000_000;
        let slot = previous_slot(now, 3600, 3, 30, None);
        assert_eq!(utc_text(slot), "2023-11-14 02:30 UTC");
        // before the time of day: yesterday's slot
        let early = 1_699_932_000; // 2023-11-14 03:20 UTC = 04:20 local → after 03:30? 04:20 > 03:30, so today
        assert_eq!(utc_text(previous_slot(early, 3600, 3, 30, None)), "2023-11-14 02:30 UTC");
        let before = 1_699_932_000 - 3 * 3600; // 00:20 UTC = 01:20 local: before 03:30 → yesterday
        assert_eq!(utc_text(previous_slot(before, 3600, 3, 30, None)), "2023-11-13 02:30 UTC");
        // weekly on Sunday (6): the 2023-11-14 is a Tuesday → 2023-11-12
        assert_eq!(utc_text(previous_slot(now, 0, 4, 0, Some(6))), "2023-11-12 04:00 UTC");
        // weekly on the same weekday but not yet the time → a week ago
        assert_eq!(utc_text(previous_slot(now, 0, 23, 0, Some(1))), "2023-11-07 23:00 UTC");
    }

    #[test]
    fn ages_in_words() {
        assert_eq!(ago(30), "just now");
        assert_eq!(ago(60), "1 minute ago");
        assert_eq!(ago(7200), "2 hours ago");
        assert_eq!(ago(90_000), "yesterday");
        assert_eq!(ago(3 * 86_400 + 5), "3 days ago");
        assert_eq!(ago(21 * 86_400), "3 weeks ago");
    }
}
