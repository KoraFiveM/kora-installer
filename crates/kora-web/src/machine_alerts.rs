//! Machine-level alerts: things that belong to no server but to Kora's own panel and service. They use the alert model of the
//! servers' alerts (`kora_core::monitor`: one open alert per kind, shown at the top of every page and in the desktop app's
//! notifications, sent to Discord / the generic webhook through the servers' notification settings), with an empty `server`.
//!
//! * the panel's HTTPS certificate: ends in N days (within 14), has expired, or Kora could not renew it by itself;
//! * Kora's administrator service stopped answering after it had been answering;
//! * a scheduled-backup pass that could not run at all (`kora_core::backup::scheduler::Announce::machine_failed`).
//!
//! The decisions are pure ([`tls_wants`], [`ServiceWatch::observe`]); [`start`] is the thread that looks every
//! [`LOOK_EVERY`] seconds and applies them. A failed look never raises anything.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use kora_core::monitor::{AlertKind, Monitor};
use kora_core::privsvc::problems::Availability;

use crate::tls::{certs, CertInfo, CertSource, TlsState};

/// How often the thread looks.
pub const LOOK_EVERY: Duration = Duration::from_secs(30);
/// A certificate this close to its end raises the alert (the System check and Settings warn at the same point).
pub const WARN_DAYS: i64 = 14;

/// What to do about one kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Want {
    Raise(AlertKind, String),
    Resolve(AlertKind),
}

/// The certificate alerts for what is being served: `days` left (negative once expired), where it came from, and the last renewal /
/// load error. No certificate at all (plain HTTP) clears all three.
pub fn tls_wants(info: Option<(i64, CertSource)>, error: Option<&str>) -> Vec<Want> {
    use AlertKind::{CertExpired, CertExpiring, CertRenewalFailed};
    let Some((days, source)) = info else { return vec![Want::Resolve(CertExpiring), Want::Resolve(CertExpired), Want::Resolve(CertRenewalFailed)] };
    let fix = match source {
        CertSource::Provided => "Replace the certificate files; the panel reads them again by itself.",
        _ => "Kora renews it by itself; if it does not, run `kora panel tls renew`.",
    };
    let mut v = Vec::new();
    if days < 0 {
        v.push(Want::Raise(CertExpired, format!("The panel's HTTPS certificate has expired, so browsers will warn people who open the panel. {fix}")));
        v.push(Want::Resolve(CertExpiring));
    } else if days <= WARN_DAYS {
        let when = match days {
            0 => "today".to_string(),
            1 => "in 1 day".to_string(),
            n => format!("in {n} days"),
        };
        v.push(Want::Raise(CertExpiring, format!("The panel's HTTPS certificate expires {when}. {fix}")));
        v.push(Want::Resolve(CertExpired));
    } else {
        v.push(Want::Resolve(CertExpiring));
        v.push(Want::Resolve(CertExpired));
    }
    match error {
        // Serving a certificate while the last attempt to fetch or load one failed: the renewal did not work.
        Some(e) => v.push(Want::Raise(CertRenewalFailed, format!("Kora could not renew the panel's HTTPS certificate: {e}. The old one is still in use until it ends."))),
        None => v.push(Want::Resolve(CertRenewalFailed)),
    }
    v
}

/// Watches the administrator service: an alert only when it WAS answering and stopped (a computer that never had it is not told off).
#[derive(Debug, Default)]
pub struct ServiceWatch {
    was_ready: bool,
}

impl ServiceWatch {
    pub fn observe(&mut self, a: &Availability) -> Option<Want> {
        match a {
            Availability::Ready => {
                self.was_ready = true;
                Some(Want::Resolve(AlertKind::ServiceStopped))
            }
            // Answering, but not for this account: not stopped.
            Availability::NotAllowed(_) => None,
            Availability::Missing if self.was_ready => Some(Want::Raise(
                AlertKind::ServiceStopped,
                "Kora's administrator service stopped answering, so anything that needs administrator rights (installing, services, firewall rules, updates, scheduled backups) will not work until it runs again. Linux: sudo systemctl restart kora. Windows: start the \"Kora\" service in Services.".into(),
            )),
            Availability::Missing => None,
        }
    }
}

/// Apply what the decisions say to the monitor.
pub fn apply(m: &Monitor, wants: Vec<Want>, now: u64) {
    for w in wants {
        match w {
            Want::Raise(k, msg) => m.raise_machine(k, msg, now),
            Want::Resolve(k) => m.resolve_machine(k, now),
        }
    }
}

/// The running thread (stop it when the panel stops).
pub struct Handle(Arc<AtomicBool>);

impl Handle {
    pub fn stop(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

fn tls_facts(s: &TlsState, now: i64) -> Option<(i64, CertSource)> {
    s.info().map(|i: CertInfo| (certs::days_left(i.not_after, now), i.source))
}

/// Start looking (certificate and service) every [`LOOK_EVERY`].
pub fn start(monitor: Monitor, tls: TlsState) -> Handle {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let _ = std::thread::Builder::new().name("kora-machine-alerts".into()).spawn(move || {
        let mut watch = ServiceWatch::default();
        while !flag.load(Ordering::Relaxed) {
            let now = kora_core::monitor::now_unix();
            // A bug in one pass must not end the watch.
            let (m, t) = (monitor.clone(), tls.clone());
            let w = &mut watch;
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                apply(&m, tls_wants(tls_facts(&t, i64::try_from(now).unwrap_or(0)), t.last_error().as_deref()), now);
                if let Some(x) = w.observe(&crate::elevated::availability()) {
                    apply(&m, vec![x], now);
                }
            }));
            let mut waited = Duration::ZERO;
            while waited < LOOK_EVERY && !flag.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(500));
                waited += Duration::from_millis(500);
            }
        }
    });
    Handle(stop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use AlertKind::*;

    fn raised(w: &[Want]) -> Vec<AlertKind> {
        w.iter().filter_map(|x| if let Want::Raise(k, _) = x { Some(*k) } else { None }).collect()
    }

    #[test]
    fn a_certificate_far_from_its_end_raises_nothing_and_clears_everything() {
        let w = tls_wants(Some((60, CertSource::Tailscale)), None);
        assert!(raised(&w).is_empty());
        assert!(w.contains(&Want::Resolve(CertExpiring)) && w.contains(&Want::Resolve(CertExpired)) && w.contains(&Want::Resolve(CertRenewalFailed)));
        assert_eq!(raised(&tls_wants(None, Some("no certificate yet"))), [], "plain HTTP is not an expiry");
    }

    #[test]
    fn the_last_fourteen_days_say_how_many_and_an_expired_one_says_so() {
        for (days, text) in [(14, "expires in 14 days"), (9, "expires in 9 days"), (1, "expires in 1 day."), (0, "expires today")] {
            let w = tls_wants(Some((days, CertSource::SelfSigned)), None);
            assert_eq!(raised(&w), [CertExpiring], "{days}");
            let Want::Raise(_, m) = &w[0] else { panic!() };
            assert!(m.contains(text) && m.contains("kora panel tls renew"), "{m}");
            assert!(w.contains(&Want::Resolve(CertExpired)));
        }
        assert_eq!(raised(&tls_wants(Some((15, CertSource::SelfSigned)), None)), []);
        let w = tls_wants(Some((-2, CertSource::Provided)), None);
        assert_eq!(raised(&w), [CertExpired]);
        let Want::Raise(_, m) = &w[0] else { panic!() };
        assert!(m.contains("has expired") && m.contains("Replace the certificate files"), "{m}");
        assert!(w.contains(&Want::Resolve(CertExpiring)));
    }

    #[test]
    fn a_failed_renewal_is_its_own_alert_and_clears_when_the_error_does() {
        let w = tls_wants(Some((20, CertSource::Tailscale)), Some("Could not renew the Tailscale certificate yet: tailscaled is not running"));
        assert_eq!(raised(&w), [CertRenewalFailed]);
        let Want::Raise(_, m) = w.last().unwrap() else { panic!() };
        assert!(m.contains("tailscaled is not running") && m.contains("old one is still in use"));
        assert!(tls_wants(Some((20, CertSource::Tailscale)), None).contains(&Want::Resolve(CertRenewalFailed)));
    }

    #[test]
    fn the_service_is_only_missed_after_it_was_there() {
        let mut w = ServiceWatch::default();
        assert_eq!(w.observe(&Availability::Missing), None, "never had it: nothing to say");
        assert_eq!(w.observe(&Availability::NotAllowed("x".into())), None);
        assert_eq!(w.observe(&Availability::Ready), Some(Want::Resolve(ServiceStopped)));
        let Some(Want::Raise(ServiceStopped, m)) = w.observe(&Availability::Missing) else { panic!() };
        assert!(m.contains("stopped answering") && m.contains("systemctl restart kora"));
        assert_eq!(w.observe(&Availability::Ready), Some(Want::Resolve(ServiceStopped)));
    }

    #[test]
    fn applying_raises_through_the_monitor_once_and_clears() {
        use kora_core::api::fake::FakeBackend;
        let mon = Monitor::system(Arc::new(|| Box::new(FakeBackend::sample())));
        apply(&mon, tls_wants(Some((5, CertSource::SelfSigned)), None), 100);
        apply(&mon, tls_wants(Some((4, CertSource::SelfSigned)), None), 200);
        let list = mon.alerts();
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].server.as_str(), list[0].kind), ("", CertExpiring));
        apply(&mon, tls_wants(Some((90, CertSource::SelfSigned)), None), 300);
        assert_eq!(mon.alerts()[0].resolved, Some(300));
    }
}
