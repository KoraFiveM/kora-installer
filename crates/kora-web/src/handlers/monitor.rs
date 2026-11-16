//! Monitoring: history for the charts, alerts and their per-server settings.
//!
//! The sampler is a [`Monitor`] started by `kora_web::run` (see [`start_system_monitor`]) and kept in a
//! module-level slot, so the router needs no new field. When no monitor is installed (the router used
//! on its own, as in most tests) the answers are simply empty.
#![allow(clippy::unused_async)]

use std::net::{IpAddr, ToSocketAddrs};
use std::sync::Mutex;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use kora_core::monitor::{self, alerts, Monitor, Range};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::remote::Caller;
use crate::{blocking, ApiError, AppState, BackendFactory};

static CURRENT: Mutex<Option<Monitor>> = Mutex::new(None);

/// Make `m` the monitor the handlers read (`None` removes it). Tests install one with fake probes.
pub fn install(m: Option<Monitor>) {
    *CURRENT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = m;
}

fn current() -> Option<Monitor> {
    CURRENT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// Start the real sampler (every 15 s) and install it. Returns the handle so the caller can stop it.
pub fn start_system_monitor(make: BackendFactory) -> Monitor {
    let m = Monitor::system(make);
    m.start();
    install(Some(m.clone()));
    m
}

#[derive(Deserialize)]
pub struct MetricsQuery {
    range: Option<String>,
}

pub async fn metrics_get(
    State(st): State<AppState>,
    Path(s): Path<String>,
    Query(q): Query<MetricsQuery>,
) -> Result<Json<Value>, ApiError> {
    let range = match q.range.as_deref() {
        None => Range::Hour,
        Some(r) => Range::parse(r)
            .ok_or_else(|| ApiError::bad_request("The range must be 1h, 6h or 24h."))?,
    };
    blocking(move || {
        let e = st.entry(&s)?;
        let view = current()
            .map(|m| m.metrics(&e, range, monitor::now_unix()))
            .unwrap_or_default();
        Ok(Json(
            json!({"samples": view.samples, "latest": view.latest}),
        ))
    })
    .await
}

pub async fn alerts_list() -> Json<Value> {
    let list = current().map(|m| m.alerts()).unwrap_or_default();
    Json(json!({"alerts": list}))
}

pub async fn alerts_ack(Path(id): Path<String>) -> Result<Response, ApiError> {
    let id: u64 = id
        .parse()
        .map_err(|_| ApiError::bad_request("That is not an alert number."))?;
    match current() {
        Some(m) if m.ack(id) => Ok(StatusCode::NO_CONTENT.into_response()),
        _ => Err(ApiError::not_found("There is no alert with that number.")),
    }
}

pub async fn alert_settings_get(
    State(st): State<AppState>,
    Path(s): Path<String>,
) -> Result<Json<Value>, ApiError> {
    blocking(move || {
        let e = st.entry(&s)?;
        Ok(Json(alerts::settings_view(&alerts::load_settings(&e.dir))))
    })
    .await
}

pub async fn alert_settings_set(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(s): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    blocking(move || {
        let e = st.entry(&s)?;
        let cur = alerts::load_settings(&e.dir);
        let next = alerts::apply_update(&cur, &body).map_err(ApiError::bad_request)?;
        // The panel sends this address requests from inside the network. Someone on another computer
        // must not be able to aim it at this machine or the private network around it.
        if caller.remote && next.webhook_url.is_some() && next.webhook_url != cur.webhook_url {
            public_webhook(next.webhook_url.as_deref().unwrap_or(""), &|h, p| {
                (h, p).to_socket_addrs().map(Iterator::collect)
            })
            .map_err(ApiError::bad_request)?;
        }
        alerts::save_settings(&e.dir, &next).map_err(ApiError::internal)?;
        Ok(Json(alerts::settings_view(&next)))
    })
    .await
}

/// Not reachable from the internet: this computer, private and shared (CGNAT, Tailscale) ranges,
/// link-local, multicast and "any" addresses.
pub(crate) fn internal_ip(ip: IpAddr) -> bool {
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
                || (o[0] == 100 && (o[1] & 0xc0) == 64)
        }
        IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
        }
    }
}

type Resolve<'a> = &'a dyn Fn(&str, u16) -> std::io::Result<Vec<std::net::SocketAddr>>;

/// Check a webhook address set over remote access: its host must only resolve to public addresses.
pub(crate) fn public_webhook(url: &str, resolve: Resolve<'_>) -> Result<(), String> {
    let (rest, default_port) = match url.split_once("://") {
        Some(("https", r)) => (r, 443),
        Some(("http", r)) => (r, 80),
        _ => return Err("That is not a usable web address.".into()),
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit('@').next().unwrap_or("");
    let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
        let (h, tail) = v6.split_once(']').ok_or("That is not a usable web address.")?;
        (h, tail.strip_prefix(':'))
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (authority, None),
        }
    };
    let port = match port {
        Some(p) => p.parse::<u16>().map_err(|_| "The port in that web address is not a number.".to_string())?,
        None => default_port,
    };
    let refused = || {
        "From another computer, the alert address must be a public internet address, not this computer or \
its private network. Set it on the computer that runs Kora if you need a local address."
            .to_string()
    };
    let addrs = resolve(host, port).map_err(|_| format!("Could not look up '{host}'. Check the address."))?;
    if addrs.is_empty() {
        return Err(format!("Could not look up '{host}'. Check the address."));
    }
    if addrs.iter().any(|a| internal_ip(a.ip())) {
        return Err(refused());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    fn dns(host: &str, port: u16) -> std::io::Result<Vec<SocketAddr>> {
        let ip: IpAddr = match host {
            "hooks.example.com" => "93.184.216.34".parse().unwrap(),
            "sneaky.example.com" => "192.168.1.10".parse().unwrap(),
            "ts.example.com" => "100.101.102.103".parse().unwrap(),
            other => match other.parse() {
                Ok(ip) => ip,
                Err(_) => return Err(std::io::Error::other("no such host")),
            },
        };
        Ok(vec![SocketAddr::new(ip, port)])
    }

    #[test]
    fn internal_ranges() {
        for ip in [
            "127.0.0.1", "10.1.2.3", "172.16.0.1", "172.31.255.255", "192.168.0.1", "169.254.169.254",
            "100.64.0.1", "100.127.255.255", "0.0.0.0", "0.1.2.3", "255.255.255.255", "224.0.0.1",
            "::1", "::", "fd7a:115c:a1e0::1", "fc00::1", "fe80::1", "::ffff:127.0.0.1", "::ffff:10.0.0.1", "ff02::1",
        ] {
            assert!(internal_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["93.184.216.34", "1.1.1.1", "100.128.0.1", "172.32.0.1", "2606:4700::1111"] {
            assert!(!internal_ip(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn webhook_hosts_must_be_public() {
        assert!(public_webhook("https://hooks.example.com/x?y=1", &dns).is_ok());
        assert!(public_webhook("http://user:pw@hooks.example.com:8080/x", &dns).is_ok());
        for bad in [
            "http://127.0.0.1:47120/api/v1/servers",
            "http://localhost/",
            "https://sneaky.example.com/hook",
            "https://ts.example.com/hook",
            "http://[::1]:8080/",
            "http://169.254.169.254/latest/meta-data/",
            "http://x@10.0.0.5/",
        ] {
            let resolve = |h: &str, p: u16| {
                if h == "localhost" {
                    Ok(vec![SocketAddr::new("127.0.0.1".parse().unwrap(), p)])
                } else {
                    dns(h, p)
                }
            };
            let e = public_webhook(bad, &resolve).unwrap_err();
            assert!(e.contains("public internet address"), "{bad}: {e}");
        }
        assert!(public_webhook("https://nowhere.invalid/", &dns).unwrap_err().contains("Could not look up"));
        assert!(public_webhook("https://hooks.example.com:x/", &dns).is_err());
        assert!(public_webhook("ftp://hooks.example.com/", &dns).is_err());
    }
}
