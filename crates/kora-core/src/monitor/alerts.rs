//! Alert rules, their per-server settings and the generic webhook.
//!
//! One alert per condition until it clears: [`evaluate`] is given the conditions that are active now and
//! only answers with *changes*. Settings live in `<server>/.kora/alerts.json` (owner-only: the generic
//! webhook address is a secret and is never returned, only [`redact_url`]).

use crate::problem::Problem;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::Sample;
use crate::api::UpdateBadge;
use crate::space::{human, GIB};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum AlertKind {
    /// Was running and now is not, and nobody asked it to stop.
    Stopped,
    /// A stopped alert cleared: the server is running again.
    Recovered,
    DiskLow,
    RamHigh,
    UpdateAttention,
    /// A backup (usually a scheduled one) did not work.
    BackupFailed,
    /// Machine-level (belongs to no server): the panel's HTTPS certificate ends soon.
    CertExpiring,
    /// Machine-level: the panel's HTTPS certificate has expired.
    CertExpired,
    /// Machine-level: Kora could not renew the panel's HTTPS certificate by itself.
    CertRenewalFailed,
    /// Machine-level: Kora's administrator service was answering and stopped.
    ServiceStopped,
}

impl AlertKind {
    /// The tone the UI shows: `danger`, `warning` or `success`.
    pub fn tone(self) -> &'static str {
        match self {
            AlertKind::Stopped | AlertKind::BackupFailed | AlertKind::CertExpired | AlertKind::ServiceStopped => "danger",
            AlertKind::Recovered => "success",
            AlertKind::DiskLow | AlertKind::RamHigh | AlertKind::UpdateAttention | AlertKind::CertExpiring | AlertKind::CertRenewalFailed => "warning",
        }
    }
    /// Machine-level kinds belong to no server (their alert has an empty `server`): they are about Kora's own panel and service.
    pub fn machine(self) -> bool {
        matches!(self, AlertKind::CertExpiring | AlertKind::CertExpired | AlertKind::CertRenewalFailed | AlertKind::ServiceStopped)
    }
    pub fn title(self) -> &'static str {
        match self {
            AlertKind::Stopped => "Server stopped",
            AlertKind::Recovered => "Server is back up",
            AlertKind::DiskLow => "Disk space is low",
            AlertKind::RamHigh => "Memory is nearly full",
            AlertKind::UpdateAttention => "Updates need attention",
            AlertKind::BackupFailed => "Backup failed",
            AlertKind::CertExpiring => "HTTPS certificate ending soon",
            AlertKind::CertExpired => "HTTPS certificate expired",
            AlertKind::CertRenewalFailed => "HTTPS certificate not renewed",
            AlertKind::ServiceStopped => "Administrator service stopped answering",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Alert {
    pub id: u64,
    /// The server it is about; empty for a machine-level alert ([`AlertKind::machine`]).
    pub server: String,
    pub kind: AlertKind,
    pub tone: &'static str,
    pub title: &'static str,
    pub message: String,
    pub raised: u64,
    /// When the condition cleared; `None` while it is still true (and for one-off events).
    pub resolved: Option<u64>,
    pub acked: bool,
}

// ------------------------------------------------------------------ settings

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rules {
    pub stopped: bool,
    pub recovered: bool,
    pub disk: bool,
    pub ram: bool,
    pub updates: bool,
    /// Machine-level alerts (the panel's certificate, the administrator service, a backup pass that could not run) are sent to this
    /// server's Discord / webhook too. They belong to no server: they go to every server's notification settings that have this on,
    /// each address once.
    pub machine: bool,
}

impl Default for Rules {
    fn default() -> Self {
        Self { stopped: true, recovered: true, disk: true, ram: true, updates: true, machine: true }
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AlertSettings {
    pub enabled: bool,
    pub rules: Rules,
    pub disk_min_gb: f64,
    pub disk_min_percent: f64,
    pub ram_max_percent: f64,
    /// Also post to the server's Discord webhook (the one under Notifications), when there is one.
    pub discord: bool,
    /// A secret: POST JSON here. Never returned by the API.
    pub webhook_url: Option<String>,
    /// The address was set over remote access. It was checked to be public then, and every send checks
    /// again what the name points to now, so a name that later moves to an inside address is not used.
    pub webhook_set_remotely: bool,
}

impl Default for AlertSettings {
    fn default() -> Self {
        Self { enabled: true, rules: Rules::default(), disk_min_gb: 5.0, disk_min_percent: 5.0, ram_max_percent: 90.0, discord: true, webhook_url: None, webhook_set_remotely: false }
    }
}

impl std::fmt::Debug for AlertSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AlertSettings").field("enabled", &self.enabled).field("rules", &self.rules).field("webhook_url", &self.webhook_url.as_deref().map(redact_url)).finish_non_exhaustive()
    }
}

pub fn settings_path(dir: &Path) -> PathBuf {
    dir.join(".kora").join("alerts.json")
}

/// Saved settings; a missing or unreadable file means the defaults.
pub fn load_settings(dir: &Path) -> AlertSettings {
    std::fs::read(settings_path(dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save_settings(dir: &Path, s: &AlertSettings) -> Result<(), Problem> {
    let path = settings_path(dir);
    let text = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    super::write_private(&path, &text).map_err(|e| e.lead("Could not save the alert settings"))
}

/// Only `http(s)://host...`, no spaces, control characters or backslashes.
pub fn validate_url(url: &str) -> Result<(), Problem> {
    let bad = |m: &str| Err(Problem::generic(format!("That is not a usable web address ({m}).")));
    if url.len() > 2048 || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return bad("it has spaces or is too long");
    }
    // Web address rules read `\` as `/`, so `http://a\@b/` goes to `a`, while a simple reading sees `b`.
    if url.contains('\\') {
        return bad("it has a backslash");
    }
    let Some(rest) = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")) else {
        return bad("it must start with http:// or https://");
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or("");
    if host.is_empty() || host.starts_with(':') {
        return bad("the host is missing");
    }
    match url::Url::parse(url) {
        Ok(u) if u.host().is_some() => Ok(()),
        _ => bad("it could not be read"),
    }
}

/// Not reachable from the internet: this computer, private and shared (CGNAT, Tailscale) ranges,
/// link-local, multicast, reserved and "any" addresses, and IPv6 forms that carry such an IPv4 address
/// (mapped, compatible, NAT64, 6to4) or hide one (Teredo).
pub fn internal_ip(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || o[0] == 0
                || o[0] >= 240
                || (o[0] == 100 && (o[1] & 0xc0) == 64)
                || (o[0] == 198 && (o[1] & 0xfe) == 18)
                || (o[0] == 192 && o[1] == 0 && o[2] == 0)
        }
        IpAddr::V6(v6) => {
            let s = v6.segments();
            let v4_at = |i: usize| IpAddr::V4(Ipv4Addr::new((s[i] >> 8) as u8, s[i] as u8, (s[i + 1] >> 8) as u8, s[i + 1] as u8));
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00
                || (s[0] & 0xffc0) == 0xfe80
                || (s[0] & 0xffc0) == 0xfec0
                // ::a.b.c.d (IPv4-compatible)
                || (s[..6].iter().all(|x| *x == 0) && internal_ip(v4_at(6)))
                // 64:ff9b::/96 (NAT64) carries the IPv4 address at the end; 64:ff9b:1::/48 is local NAT64.
                || (s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] && internal_ip(v4_at(6)))
                || (s[0] == 0x64 && s[1] == 0xff9b && s[2] == 1)
                // 2002::/16 (6to4) carries it right after the prefix.
                || (s[0] == 0x2002 && internal_ip(v4_at(1)))
                // 2001::/32 (Teredo) hides it, so none of it is trusted.
                || (s[0] == 0x2001 && s[1] == 0)
        }
    }
}

/// Looks a host name up: `(host, port)` to addresses. Tests pass their own.
pub type Lookup<'a> = &'a dyn Fn(&str, u16) -> std::io::Result<Vec<SocketAddr>>;

const NOT_PUBLIC: &str = "From another computer, the alert address must be a public internet address, not this computer or \
its private network. Set it on the computer that runs Kora if you need a local address.";

/// Check a webhook address set over remote access: its host must only resolve to public addresses. The
/// address is read by the same rules the sender uses (`url`), so the host checked is the host contacted.
pub fn public_webhook(url: &str, lookup: Lookup<'_>) -> Result<(), Problem> {
    validate_url(url)?;
    let u = url::Url::parse(url).map_err(|_| "That is not a usable web address.".to_string())?;
    if !matches!(u.scheme(), "http" | "https") {
        return Err("That is not a usable web address.".into());
    }
    let port = u.port_or_known_default().ok_or("That is not a usable web address.")?;
    let addrs = match u.host() {
        Some(url::Host::Ipv4(v4)) => vec![SocketAddr::new(IpAddr::V4(v4), port)],
        Some(url::Host::Ipv6(v6)) => vec![SocketAddr::new(IpAddr::V6(v6), port)],
        Some(url::Host::Domain(d)) => {
            let looked = lookup(d, port).map_err(|_| format!("Could not look up '{d}'. Check the address."))?;
            if looked.is_empty() {
                return Err(format!("Could not look up '{d}'. Check the address.").into());
            }
            looked
        }
        None => return Err("That is not a usable web address.".into()),
    };
    if addrs.iter().any(|a| internal_ip(a.ip())) {
        return Err(NOT_PUBLIC.into());
    }
    Ok(())
}

/// Resolve `netloc` (`host:port`) for a send to a remotely set address: every address must be public,
/// and the connection is made to exactly these addresses (no second lookup an attacker could change).
pub fn public_addrs(netloc: &str, lookup: &dyn Fn(&str) -> std::io::Result<Vec<SocketAddr>>) -> std::io::Result<Vec<SocketAddr>> {
    let addrs = lookup(netloc)?;
    if addrs.is_empty() || addrs.iter().any(|a| internal_ip(a.ip())) {
        return Err(std::io::Error::other("the alert address now points inside the network"));
    }
    Ok(addrs)
}

/// `https://host/***`: safe to show and log (no credentials, path or query).
pub fn redact_url(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("https", url));
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or("");
    if host.is_empty() || !matches!(scheme, "http" | "https") {
        return "<address hidden>".into();
    }
    format!("{scheme}://{host}/***")
}

/// What the API returns: everything except the secret address.
pub fn settings_view(s: &AlertSettings) -> Value {
    json!({
        "enabled": s.enabled,
        "rules": s.rules,
        "disk_min_gb": s.disk_min_gb,
        "disk_min_percent": s.disk_min_percent,
        "ram_max_percent": s.ram_max_percent,
        "discord": s.discord,
        "webhook": {"configured": s.webhook_url.is_some(), "redacted": s.webhook_url.as_deref().map(redact_url)},
        "email": {"supported": false},
    })
}

/// Apply a partial update. Absent keys keep their value; `webhook_url` of `""` or `null` removes the address.
pub fn apply_update(cur: &AlertSettings, body: &Value) -> Result<AlertSettings, Problem> {
    let obj = body.as_object().ok_or("Send the alert settings as a JSON object.")?;
    let mut s = cur.clone();
    let flag = |v: &Value, name: &str| v.as_bool().ok_or_else(|| format!("'{name}' must be true or false."));
    let num = |v: &Value, name: &str| v.as_f64().filter(|n| n.is_finite()).ok_or_else(|| format!("'{name}' must be a number."));
    for (k, v) in obj {
        match k.as_str() {
            "enabled" => s.enabled = flag(v, k)?,
            "discord" => s.discord = flag(v, k)?,
            "rules" => {
                let r = v.as_object().ok_or("'rules' must be an object.")?;
                for (rk, rv) in r {
                    let b = flag(rv, rk)?;
                    match rk.as_str() {
                        "stopped" => s.rules.stopped = b,
                        "recovered" => s.rules.recovered = b,
                        "disk" => s.rules.disk = b,
                        "ram" => s.rules.ram = b,
                        "updates" => s.rules.updates = b,
                        "machine" => s.rules.machine = b,
                        other => return Err(format!("There is no alert rule called '{other}'.").into()),
                    }
                }
            }
            "disk_min_gb" => s.disk_min_gb = num(v, k)?,
            "disk_min_percent" => s.disk_min_percent = num(v, k)?,
            "ram_max_percent" => s.ram_max_percent = num(v, k)?,
            "webhook_url" => match v {
                Value::Null => s.webhook_url = None,
                Value::String(u) if u.trim().is_empty() => s.webhook_url = None,
                Value::String(u) => {
                    validate_url(u.trim())?;
                    s.webhook_url = Some(u.trim().to_string());
                }
                _ => return Err("'webhook_url' must be text.".into()),
            },
            // Read-only echoes from the view; ignored.
            "webhook" | "email" => {}
            other => return Err(format!("Unknown alert setting '{other}'.").into()),
        }
    }
    if !(0.0..=100_000.0).contains(&s.disk_min_gb) {
        return Err("The minimum free disk space must be between 0 and 100000 GB.".into());
    }
    if !(0.0..=100.0).contains(&s.disk_min_percent) {
        return Err("The minimum free disk space must be between 0 and 100 percent.".into());
    }
    if !(1.0..=100.0).contains(&s.ram_max_percent) {
        return Err("The memory limit must be between 1 and 100 percent.".into());
    }
    Ok(s)
}

// ------------------------------------------------------------------ rules

/// What changed for one server in one sample.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// A condition became true: raise and deliver one alert.
    Raise(AlertKind, String),
    /// The condition is false again.
    Resolve(AlertKind),
    /// A one-off event (the server came back up).
    Event(AlertKind, String),
}

pub struct Facts {
    /// Was the previous sample running (`None`: first look since the panel started).
    pub prev_up: Option<bool>,
    /// A stop or restart was asked for through Kora just before this sample.
    pub intentional_stop: bool,
    pub disk_total: Option<u64>,
}

/// The changes this sample causes, given the alerts that are `active` (debounce: a condition that is
/// already active never raises again until it has cleared).
pub fn evaluate(s: &Sample, f: &Facts, set: &AlertSettings, active: &HashMap<AlertKind, u64>) -> Vec<Change> {
    let mut out = Vec::new();
    let is_active = |k: AlertKind| active.contains_key(&k);
    if !set.enabled {
        out.extend(active.keys().map(|k| Change::Resolve(*k)));
        return out;
    }

    // Stopped / back up.
    if is_active(AlertKind::Stopped) {
        if s.up {
            out.push(Change::Resolve(AlertKind::Stopped));
            if set.rules.recovered {
                out.push(Change::Event(AlertKind::Recovered, "The server is running again.".into()));
            }
        } else if !set.rules.stopped {
            out.push(Change::Resolve(AlertKind::Stopped));
        }
    } else if set.rules.stopped && f.prev_up == Some(true) && !s.up && !f.intentional_stop {
        out.push(Change::Raise(AlertKind::Stopped, "The server stopped without being asked to. It may have crashed.".into()));
    }

    // The rest are plain conditions: raise on the edge, resolve when false (or switched off).
    let mut cond = |kind: AlertKind, on: bool, state: Option<String>| {
        let want = on && state.is_some();
        match (is_active(kind), want) {
            (false, true) => out.push(Change::Raise(kind, state.unwrap_or_default())),
            (true, false) => out.push(Change::Resolve(kind)),
            _ => {}
        }
    };

    let disk = s.disk_free_mb.map(|mb| mb * 1024 * 1024).and_then(|free| {
        let pct = f.disk_total.filter(|t| *t > 0).map(|t| free as f64 / t as f64 * 100.0);
        let low = (free as f64) < set.disk_min_gb * GIB as f64 || pct.is_some_and(|p| p < set.disk_min_percent);
        low.then(|| match pct {
            Some(p) => format!("Only {} ({p:.0}%) is free on the server's disk.", human(free)),
            None => format!("Only {} is free on the server's disk.", human(free)),
        })
    });
    cond(AlertKind::DiskLow, set.rules.disk, disk);

    let ram = s.sys_ram_pct.filter(|p| f64::from(*p) > set.ram_max_percent).map(|p| format!("{p:.0}% of this machine's memory is in use."));
    cond(AlertKind::RamHigh, set.rules.ram, ram);

    let upd = (s.badge == Some(UpdateBadge::NeedsAttention)).then(|| "Some resources have changes Kora cannot update on its own. Open the server's Resources tab.".to_string());
    cond(AlertKind::UpdateAttention, set.rules.updates, upd);
    out
}

// ------------------------------------------------------------------ generic webhook

/// POST `body` as JSON to a user-chosen address. Bounded time, no redirects; the address is never
/// part of an error message. With `public_only` (the address was set over remote access) the name is
/// looked up again now and the send is refused if any address it gives is internal.
pub fn post_json(url: &str, body: &Value, public_only: bool) -> Result<(), Problem> {
    if public_only {
        post_via(url, body, |n: &str| public_addrs(n, &|n| n.to_socket_addrs().map(Iterator::collect)))
    } else {
        post_via(url, body, |n: &str| n.to_socket_addrs().map(Iterator::collect))
    }
}

/// [`post_json`] with the lookup the connection uses (tests point a name at a local listener).
pub(crate) fn post_via(url: &str, body: &Value, resolve: impl Fn(&str) -> std::io::Result<Vec<SocketAddr>> + Send + Sync + 'static) -> Result<(), Problem> {
    validate_url(url)?;
    let t = Duration::from_secs(5);
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout(t)
        .timeout_connect(t)
        // A proxy would be looked up instead of the address, skipping the check above.
        .try_proxy_from_env(false)
        .resolver(resolve)
        .build();
    match agent.post(url).set("Content-Type", "application/json").set("User-Agent", "kora-monitor").send_string(&body.to_string()) {
        Ok(r) if r.status() < 300 => Ok(()),
        Ok(r) | Err(ureq::Error::Status(_, r)) => Err(format!("The webhook answered with HTTP {}.", r.status()).into()),
        Err(_) => Err("Could not reach the webhook address.".into()),
    }
}

/// What a machine-level alert is sent to: the Discord addresses and generic webhooks of the servers whose notification settings allow it,
/// each address once (`servers`: each server's settings and its Discord webhook). Pure; [`super::RealSink`] does the sending.
pub fn machine_targets(servers: &[(AlertSettings, Option<String>)]) -> (Vec<String>, Vec<(String, bool)>) {
    let (mut discord, mut hooks): (Vec<String>, Vec<(String, bool)>) = (vec![], vec![]);
    for (set, discord_url) in servers {
        if !set.enabled || !set.rules.machine {
            continue;
        }
        if let Some(u) = discord_url.as_ref().filter(|_| set.discord) {
            if !discord.contains(u) {
                discord.push(u.clone());
            }
        }
        if let Some(u) = &set.webhook_url {
            if !hooks.iter().any(|(h, _)| h == u) {
                hooks.push((u.clone(), set.webhook_set_remotely));
            }
        }
    }
    (discord, hooks)
}

/// The JSON a generic webhook receives.
pub fn webhook_payload(a: &Alert) -> Value {
    json!({
        "source": "kora",
        "server": a.server,
        "kind": a.kind,
        "severity": a.tone,
        "title": a.title,
        "message": a.message,
        "time": a.raised,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(up: bool) -> Sample {
        Sample { t: 100, up, ..Sample::default() }
    }
    fn facts(prev: Option<bool>) -> Facts {
        Facts { prev_up: prev, intentional_stop: false, disk_total: None }
    }
    fn ev(s: &Sample, f: &Facts, set: &AlertSettings, active: &[AlertKind]) -> Vec<Change> {
        evaluate(s, f, set, &active.iter().map(|k| (*k, 1)).collect())
    }

    #[test]
    fn a_stop_nobody_asked_for_raises_once_and_a_return_clears_it() {
        let set = AlertSettings::default();
        let c = ev(&sample(false), &facts(Some(true)), &set, &[]);
        assert!(matches!(c.as_slice(), [Change::Raise(AlertKind::Stopped, _)]));
        // Still down: nothing new (debounced).
        assert!(ev(&sample(false), &facts(Some(false)), &set, &[AlertKind::Stopped]).is_empty());
        let c = ev(&sample(true), &facts(Some(false)), &set, &[AlertKind::Stopped]);
        assert!(matches!(c.as_slice(), [Change::Resolve(AlertKind::Stopped), Change::Event(AlertKind::Recovered, _)]));
    }

    #[test]
    fn asked_for_stops_and_first_looks_do_not_alert() {
        let set = AlertSettings::default();
        let mut f = facts(Some(true));
        f.intentional_stop = true;
        assert!(ev(&sample(false), &f, &set, &[]).is_empty());
        assert!(ev(&sample(false), &facts(None), &set, &[]).is_empty());
        assert!(ev(&sample(false), &facts(Some(false)), &set, &[]).is_empty());
    }

    #[test]
    fn disk_uses_gigabytes_or_percent() {
        let set = AlertSettings::default();
        let mut s = sample(true);
        s.disk_free_mb = Some(3 * 1024); // 3 GB
        let c = ev(&s, &Facts { disk_total: Some(2000 * GIB), ..facts(None) }, &set, &[]);
        assert!(matches!(c.as_slice(), [Change::Raise(AlertKind::DiskLow, m)] if m.contains('3')), "{c:?}");
        // 100 GB free of 1000 GB is 10 %: fine.
        s.disk_free_mb = Some(100 * 1024);
        assert!(ev(&s, &Facts { disk_total: Some(1000 * GIB), ..facts(None) }, &set, &[]).is_empty());
        // 40 GB of 1000 GB is 4 %: under the percentage even though over 5 GB.
        s.disk_free_mb = Some(40 * 1024);
        assert_eq!(ev(&s, &Facts { disk_total: Some(1000 * GIB), ..facts(None) }, &set, &[]).len(), 1);
        // Cleared again.
        s.disk_free_mb = Some(500 * 1024);
        assert_eq!(ev(&s, &Facts { disk_total: Some(1000 * GIB), ..facts(None) }, &set, &[AlertKind::DiskLow]), vec![Change::Resolve(AlertKind::DiskLow)]);
    }

    #[test]
    fn ram_updates_and_switches() {
        let mut set = AlertSettings::default();
        let mut s = sample(true);
        s.sys_ram_pct = Some(95.0);
        s.badge = Some(UpdateBadge::NeedsAttention);
        assert_eq!(ev(&s, &facts(None), &set, &[]).len(), 2);
        set.rules.ram = false;
        set.rules.updates = false;
        assert!(ev(&s, &facts(None), &set, &[]).is_empty());
        // Everything off: active alerts resolve.
        set = AlertSettings { enabled: false, ..AlertSettings::default() };
        assert_eq!(ev(&s, &facts(None), &set, &[AlertKind::RamHigh]), vec![Change::Resolve(AlertKind::RamHigh)]);
    }

    #[test]
    fn machine_alerts_go_to_each_address_once_and_only_where_the_server_allows_them() {
        let on = AlertSettings { webhook_url: Some("https://hooks.test/a".into()), ..AlertSettings::default() };
        let off = AlertSettings { rules: Rules { machine: false, ..Rules::default() }, webhook_url: Some("https://hooks.test/b".into()), ..AlertSettings::default() };
        let disabled = AlertSettings { enabled: false, webhook_url: Some("https://hooks.test/c".into()), ..AlertSettings::default() };
        let no_discord = AlertSettings { discord: false, ..AlertSettings::default() };
        let (discord, hooks) = machine_targets(&[
            (on.clone(), Some("https://discord.test/1".into())),
            (on, Some("https://discord.test/1".into())),
            (off, Some("https://discord.test/2".into())),
            (disabled, Some("https://discord.test/3".into())),
            (no_discord, Some("https://discord.test/4".into())),
        ]);
        assert_eq!(discord, ["https://discord.test/1"]);
        assert_eq!(hooks, [("https://hooks.test/a".to_string(), false)]);
        // the new rule is switched with the others and old files without it mean "on"
        let s = apply_update(&AlertSettings::default(), &json!({"rules": {"machine": false}})).unwrap();
        assert!(!s.rules.machine);
        let old: AlertSettings = serde_json::from_str(r#"{"rules": {"stopped": true}}"#).unwrap();
        assert!(old.rules.machine, "a settings file from before has no such rule: it is on");
    }

    #[test]
    fn machine_kinds_have_words_a_tone_and_belong_to_no_server() {
        for k in [AlertKind::CertExpiring, AlertKind::CertExpired, AlertKind::CertRenewalFailed, AlertKind::ServiceStopped] {
            assert!(k.machine() && !k.title().is_empty() && matches!(k.tone(), "danger" | "warning"), "{k:?}");
        }
        assert!(!AlertKind::Stopped.machine() && !AlertKind::BackupFailed.machine());
        assert_eq!((AlertKind::CertExpired.tone(), AlertKind::CertExpiring.tone()), ("danger", "warning"));
    }

    #[test]
    fn settings_update_is_partial_and_validated() {
        let cur = AlertSettings::default();
        let s = apply_update(&cur, &json!({"disk_min_gb": 10, "rules": {"ram": false}, "webhook_url": "https://example.test/hook?k=secret"})).unwrap();
        assert_eq!((s.disk_min_gb, s.rules.ram, s.rules.disk), (10.0, false, true));
        assert_eq!(s.webhook_url.as_deref(), Some("https://example.test/hook?k=secret"));
        // Absent keeps it, empty removes it.
        assert!(apply_update(&s, &json!({})).unwrap().webhook_url.is_some());
        assert!(apply_update(&s, &json!({"webhook_url": ""})).unwrap().webhook_url.is_none());
        for bad in [
            json!({"ram_max_percent": 0}),
            json!({"disk_min_percent": 101}),
            json!({"webhook_url": "ftp://x"}),
            json!({"nope": 1}),
            json!({"rules": {"x": true}}),
            json!({"enabled": "yes"}),
            json!([]),
        ] {
            assert!(apply_update(&cur, &bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_secret_address_never_appears_in_views_or_debug() {
        let s = AlertSettings { webhook_url: Some("https://user:pw@example.test/hook/abc123?key=zzz".into()), ..AlertSettings::default() };
        let v = settings_view(&s).to_string();
        let d = format!("{s:?}");
        for text in [&v, &d] {
            assert!(!text.contains("abc123") && !text.contains("zzz") && !text.contains("pw@"), "{text}");
        }
        assert!(v.contains("https://example.test/***"));
    }

    #[test]
    fn settings_round_trip_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_settings(dir.path()), AlertSettings::default());
        let s = AlertSettings { disk_min_gb: 9.0, webhook_url: Some("http://127.0.0.1:1/x".into()), ..AlertSettings::default() };
        save_settings(dir.path(), &s).unwrap();
        assert_eq!(load_settings(dir.path()), s);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(settings_path(dir.path())).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "alerts.json must be owner-only");
        }
    }

    fn dns(host: &str, port: u16) -> std::io::Result<Vec<SocketAddr>> {
        let ip: IpAddr = match host {
            "hooks.example.com" => "93.184.216.34".parse().unwrap(),
            "sneaky.example.com" => "192.168.1.10".parse().unwrap(),
            "ts.example.com" => "100.101.102.103".parse().unwrap(),
            "localhost" => "127.0.0.1".parse().unwrap(),
            _ => return Err(std::io::Error::other("no such host")),
        };
        Ok(vec![SocketAddr::new(ip, port)])
    }

    #[test]
    fn internal_ranges() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "100.127.255.255",
            "0.0.0.0",
            "0.1.2.3",
            "255.255.255.255",
            "224.0.0.1",
            "198.18.0.1",
            "198.19.255.255",
            "240.0.0.1",
            "192.0.0.8",
            "::1",
            "::",
            "fd7a:115c:a1e0::1",
            "fc00::1",
            "fe80::1",
            "fec0::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "ff02::1",
            "::127.0.0.1",
            "::10.0.0.1",
            "64:ff9b::7f00:1",
            "64:ff9b::a9fe:a9fe",
            "64:ff9b:1::1",
            "2002:7f00:1::",
            "2002:c0a8:101::1",
            "2001:0:4136:e378::1",
        ] {
            assert!(internal_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["93.184.216.34", "1.1.1.1", "100.128.0.1", "172.32.0.1", "198.20.0.1", "2606:4700::1111", "64:ff9b::101:101", "2002:101:101::1", "::1.1.1.1"] {
            assert!(!internal_ip(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn webhook_hosts_must_be_public() {
        assert!(public_webhook("https://hooks.example.com/x?y=1", &dns).is_ok());
        assert!(public_webhook("http://user:pw@hooks.example.com:8080/x", &dns).is_ok());
        assert!(public_webhook("https://93.184.216.34/hook", &dns).is_ok());
        for bad in [
            "http://127.0.0.1:47120/api/v1/servers",
            "http://localhost/",
            "https://sneaky.example.com/hook",
            "https://ts.example.com/hook",
            "http://[::1]:8080/",
            "http://[64:ff9b::7f00:1]/",
            "http://169.254.169.254/latest/meta-data/",
            "http://x@10.0.0.5/",
            // Read the way the sender reads them: these are 127.0.0.1.
            "http://0x7f000001/",
            "http://127.1/",
            "http://hooks.example.com@127.0.0.1/",
        ] {
            let e = public_webhook(bad, &dns).unwrap_err();
            assert!(e.to_string().contains("public internet address"), "{bad}: {e}");
        }
        // `\` counts as `/` for the sender, so this would go to 127.0.0.1.
        let e = public_webhook("http://127.0.0.1:8080\\@hooks.example.com/", &dns).unwrap_err();
        assert!(e.to_string().contains("backslash"), "{e}");
        assert!(public_webhook("https://nowhere.invalid/", &dns).unwrap_err().to_string().contains("Could not look up"));
        assert!(public_webhook("https://hooks.example.com:x/", &dns).is_err());
        assert!(public_webhook("ftp://hooks.example.com/", &dns).is_err());
    }

    #[test]
    fn backslashes_are_refused_everywhere() {
        assert!(validate_url("http://127.0.0.1:8080\\@hooks.example.com/").is_err());
        assert!(apply_update(&AlertSettings::default(), &json!({"webhook_url": "http://a\\@b/"})).is_err());
        assert!(validate_url("http://192.168.1.50/hook").is_ok(), "a local address is fine here");
    }

    #[test]
    fn sends_to_remote_set_addresses_check_the_lookup_each_time() {
        let to = |ips: &[&str]| -> Vec<SocketAddr> { ips.iter().map(|i| SocketAddr::new(i.parse().unwrap(), 80)).collect() };
        let public = to(&["93.184.216.34"]);
        assert_eq!(public_addrs("h:80", &|_| Ok(public.clone())).unwrap(), public);
        // The name moved to an inside address since it was saved (DNS rebinding), or one of several is inside.
        assert!(public_addrs("h:80", &|_| Ok(to(&["127.0.0.1"]))).is_err());
        assert!(public_addrs("h:80", &|_| Ok(to(&["93.184.216.34", "10.0.0.1"]))).is_err());
        assert!(public_addrs("h:80", &|_| Ok(vec![])).is_err());
        // The real send: refused before any connection.
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.set_nonblocking(true).unwrap();
        let port = l.local_addr().unwrap().port();
        let e = post_json(&format!("http://127.0.0.1:{port}/x"), &json!({}), true).unwrap_err();
        assert!(e.to_string().contains("Could not reach"), "{e}");
        assert!(l.accept().is_err(), "nothing connected");
        // Set on this computer: the same address is used as before.
        let t = std::thread::spawn(move || {
            use std::io::Write;
            l.set_nonblocking(false).unwrap();
            let (mut c, _) = l.accept().unwrap();
            let _ = read_request(&mut c);
            let _ = c.write_all(b"HTTP/1.0 204 No Content\r\n\r\n");
        });
        post_json(&format!("http://127.0.0.1:{port}/x"), &json!({}), false).unwrap();
        t.join().unwrap();
    }

    /// Read one whole HTTP request (headers and body) before the test server answers: closing with
    /// unread data resets the connection, and on Windows the body often arrives in a second packet.
    fn read_request(c: &mut std::net::TcpStream) -> String {
        use std::io::Read;
        let _ = c.set_nonblocking(false);
        let _ = c.set_read_timeout(Some(Duration::from_secs(5)));
        let mut data = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let text = String::from_utf8_lossy(&data).into_owned();
            if let Some((head, body)) = text.split_once("\r\n\r\n") {
                let len = head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                if body.len() >= len {
                    return text;
                }
            }
            match c.read(&mut buf) {
                Ok(0) | Err(_) => return text,
                Ok(n) => data.extend_from_slice(&buf[..n]),
            }
        }
    }

    #[test]
    fn the_connection_goes_to_the_checked_addresses() {
        use std::io::Write;
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let t = std::thread::spawn(move || {
            let (mut c, _) = l.accept().unwrap();
            let req = read_request(&mut c);
            let _ = c.write_all(b"HTTP/1.0 204 No Content\r\n\r\n");
            req
        });
        // A name nobody can look up still reaches the listener: only the given lookup was used.
        post_via("http://hook.invalid/p", &json!({}), move |n: &str| {
            assert_eq!(n, "hook.invalid:80");
            Ok(vec![addr])
        })
        .unwrap();
        assert!(t.join().unwrap().starts_with("POST /p "));
    }

    #[test]
    fn the_remote_flag_defaults_to_off_for_old_files() {
        let s: AlertSettings = serde_json::from_str(r#"{"webhook_url": "http://10.0.0.1/"}"#).unwrap();
        assert!(!s.webhook_set_remotely);
    }
}
