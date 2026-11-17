//! Per-server backup settings and the "last backup" state, kept in Kora's own store (documents
//! `backup-settings` and `backup-status`, one object keyed by server name), never inside the server
//! folder: deleting a server must not delete the record of where its backups are.
//!
//! The "last backup" line is derived from the backups that really exist in the destination (so a
//! backup copied in by hand counts, and one deleted by hand stops counting), plus the status document for
//! failures.

use std::collections::BTreeMap;

use serde_json::Value;

use super::clock;
use super::types::{BackupInfo, BackupSchedule, BackupSettings, BackupStatus, BackupSummary, Cadence};
use crate::problem::Problem;
use crate::store::{documents, Store};

const SETTINGS_DOC: &str = "backup-settings";
const STATUS_DOC: &str = "backup-status";

fn read_map<T: serde::de::DeserializeOwned>(s: &Store, doc: &str) -> Result<BTreeMap<String, T>, Problem> {
    let Some(v) = documents::get(s, doc)? else { return Ok(BTreeMap::new()) };
    Ok(match v {
        Value::Object(m) => m.into_iter().filter_map(|(k, v)| serde_json::from_value(v).ok().map(|t| (k, t))).collect(),
        _ => BTreeMap::new(),
    })
}

pub fn load_settings(server: &str) -> BackupSettings {
    Store::open_home().ok().and_then(|s| read_map::<BackupSettings>(&s, SETTINGS_DOC).ok()).and_then(|mut m| m.remove(server)).unwrap_or_default()
}

pub fn save_settings(server: &str, new: &BackupSettings) -> Result<(), Problem> {
    let s = Store::open_home()?;
    documents::update(&s, SETTINGS_DOC, |d| {
        if !d.is_object() {
            *d = Value::Object(Default::default());
        }
        d[server] = serde_json::to_value(new).map_err(|e| Problem::generic(e.to_string()))?;
        Ok(())
    })
}

/// Every server's settings (for the scheduler).
pub fn all_settings() -> BTreeMap<String, BackupSettings> {
    Store::open_home().ok().and_then(|s| read_map(&s, SETTINGS_DOC).ok()).unwrap_or_default()
}

pub fn load_status(server: &str) -> BackupStatus {
    Store::open_home().ok().and_then(|s| read_map::<BackupStatus>(&s, STATUS_DOC).ok()).and_then(|mut m| m.remove(server)).unwrap_or_default()
}

/// Change the status in one transaction.
pub fn update_status(server: &str, f: impl FnOnce(&mut BackupStatus)) {
    let Ok(s) = Store::open_home() else { return };
    let _ = documents::update(&s, STATUS_DOC, |d| {
        if !d.is_object() {
            *d = Value::Object(Default::default());
        }
        let mut st: BackupStatus = serde_json::from_value(d[server].clone()).unwrap_or_default();
        f(&mut st);
        d[server] = serde_json::to_value(&st).map_err(|e| Problem::generic(e.to_string()))?;
        Ok(())
    });
}

/// Forget a server's settings and status (not its backups).
pub fn forget(server: &str) {
    let Ok(s) = Store::open_home() else { return };
    for doc in [SETTINGS_DOC, STATUS_DOC] {
        let _ = documents::update(&s, doc, |d| {
            if let Some(o) = d.as_object_mut() {
                o.remove(server);
            }
            Ok(())
        });
    }
}

/// A schedule that is acceptable: a real time, a weekday for weekly.
pub fn validate_schedule(s: &BackupSchedule) -> Result<(), String> {
    crate::service::parse_time(&s.time).map_err(|e| e.to_string())?;
    match (s.cadence, s.weekday) {
        (Cadence::Weekly, None) => Err("Pick a day of the week for a weekly backup.".into()),
        (_, Some(d)) if d > 6 => Err("The day of the week must be Monday to Sunday.".into()),
        _ => Ok(()),
    }
}

fn hm(time: &str) -> (u32, u32) {
    let mut it = time.split(':').filter_map(|p| p.parse::<u32>().ok());
    (it.next().unwrap_or(3), it.next().unwrap_or(30))
}

/// The most recent moment the schedule says a backup should have started, as of `now`.
pub fn previous_slot(s: &BackupSchedule, now: u64) -> u64 {
    let (h, m) = hm(&s.time);
    let wd = if s.cadence == Cadence::Weekly { Some(s.weekday.unwrap_or(0)) } else { None };
    clock::previous_slot(now, clock::local_offset_secs(now), h, m, wd)
}

/// Is a scheduled backup due? Once per slot: a missed slot is made up when the panel next runs, a
/// failed one is not retried before the next slot.
pub fn is_due(s: &BackupSchedule, status: &BackupStatus, now: u64) -> bool {
    s.enabled && status.last_scheduled_unix.is_none_or(|last| last < previous_slot(s, now))
}

/// How old the newest backup may get before it counts as late: one period of the schedule plus a grace.
pub fn late_after_secs(s: &BackupSchedule) -> u64 {
    let period = match s.cadence {
        Cadence::Daily => 86_400,
        Cadence::Weekly => 7 * 86_400,
    };
    period + 6 * 3600
}

/// The line the Overview and the servers list show.
pub fn summary(list: &[BackupInfo], settings: &BackupSettings, status: &BackupStatus, now: u64) -> BackupSummary {
    let newest = list.iter().map(|b| b.created_unix).max().into_iter().chain(status.last_ok_unix).max();
    let scheduled = settings.schedule.enabled;
    let text = match newest {
        None => "No backup yet".to_string(),
        Some(t) => format!("Last backup {}", clock::ago(now.saturating_sub(t))),
    };
    let failed_after = status.last_failed_unix.zip(newest.or(Some(0))).is_some_and(|(f, ok)| f > ok);
    let warning = if failed_after {
        Some(format!("The last backup failed{}.", status.last_error.as_deref().map(|e| format!(": {e}")).unwrap_or_default()))
    } else if scheduled && newest.is_some_and(|t| now.saturating_sub(t) > late_after_secs(&settings.schedule)) {
        Some(match settings.schedule.cadence {
            Cadence::Daily => "The last backup is older than a day, but backups are meant to run every day.".to_string(),
            Cadence::Weekly => "The last backup is older than a week, but backups are meant to run every week.".to_string(),
        })
    } else if scheduled && newest.is_none() && now.saturating_sub(status.last_scheduled_unix.unwrap_or(now)) > late_after_secs(&settings.schedule) {
        Some("Backups are scheduled, but none has been made yet.".to_string())
    } else {
        None
    };
    BackupSummary { last_ok_unix: newest, text, warning, scheduled }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::types::{DbKind, Label};

    fn b(at: u64) -> BackupInfo {
        BackupInfo {
            id: format!("{at}"),
            file: format!("x-{at}.tar.gz"),
            bytes: 1,
            sha256: String::new(),
            encrypted: false,
            created_unix: at,
            label: Label::Manual,
            kora_version: "0.8.0".into(),
            server: "x".into(),
            database: DbKind::None,
            by_reference: 0,
            in_full: 0,
            stopped: false,
            too_new: false,
        }
    }

    #[test]
    fn settings_and_status_live_in_the_store() {
        let _ = crate::installer::registry::use_test_home();
        let mut s = BackupSettings::default();
        s.folder = "/mnt/backups".into();
        s.schedule.enabled = true;
        save_settings("alpha", &s).unwrap();
        save_settings("beta", &BackupSettings::default()).unwrap();
        assert_eq!(load_settings("alpha").folder, "/mnt/backups");
        assert_eq!(load_settings("nobody"), BackupSettings::default());
        update_status("alpha", |st| st.last_error = Some("x".into()));
        update_status("alpha", |st| st.last_failed_unix = Some(5));
        let st = load_status("alpha");
        assert_eq!((st.last_error.as_deref(), st.last_failed_unix), (Some("x"), Some(5)));
        forget("alpha");
        assert_eq!(load_settings("alpha"), BackupSettings::default());
        assert_eq!(load_status("alpha"), BackupStatus::default());
        assert!(all_settings().contains_key("beta"));
    }

    #[test]
    fn the_last_backup_line() {
        let st = BackupStatus::default();
        let off = BackupSettings::default();
        assert_eq!(summary(&[], &off, &st, 1000).text, "No backup yet");
        assert_eq!(summary(&[], &off, &st, 1000).warning, None);
        let three_days = 3 * 86_400;
        let s = summary(&[b(1000)], &off, &st, 1000 + three_days);
        assert_eq!(s.text, "Last backup 3 days ago");
        assert_eq!(s.warning, None, "no schedule, so nothing is late");
        let mut on = BackupSettings::default();
        on.schedule.enabled = true;
        let s = summary(&[b(1000)], &on, &st, 1000 + three_days);
        assert!(s.warning.unwrap().contains("older than a day"));
        assert!(summary(&[b(1000)], &on, &st, 1000 + 3600).warning.is_none());
        let failed = BackupStatus { last_failed_unix: Some(5000), last_error: Some("the disk is full".into()), ..Default::default() };
        assert_eq!(summary(&[b(1000)], &off, &failed, 6000).warning.as_deref(), Some("The last backup failed: the disk is full."));
        // a later success clears the warning
        assert!(summary(&[b(9000)], &off, &failed, 9500).warning.is_none());
    }

    #[test]
    fn due_once_per_slot_and_not_for_a_brand_new_schedule() {
        let sch = BackupSchedule { enabled: true, cadence: Cadence::Daily, time: "03:30".into(), weekday: None };
        let now = 1_700_000_000;
        let slot = previous_slot(&sch, now);
        assert!(slot <= now && now - slot <= 86_400 + 3600);
        assert!(is_due(&sch, &BackupStatus::default(), now), "never ran: due");
        let ran = BackupStatus { last_scheduled_unix: Some(slot + 60), ..Default::default() };
        assert!(!is_due(&sch, &ran, now));
        assert!(is_due(&sch, &ran, now + 86_400));
        assert!(!is_due(&BackupSchedule { enabled: false, ..sch.clone() }, &BackupStatus::default(), now));
    }

    #[test]
    fn schedules_are_validated() {
        let ok = BackupSchedule { enabled: true, cadence: Cadence::Weekly, time: "04:00".into(), weekday: Some(6) };
        assert!(validate_schedule(&ok).is_ok());
        assert!(validate_schedule(&BackupSchedule { weekday: None, ..ok.clone() }).is_err());
        assert!(validate_schedule(&BackupSchedule { time: "4am".into(), ..ok.clone() }).is_err());
        assert!(validate_schedule(&BackupSchedule { weekday: Some(9), ..ok }).is_err());
    }
}
