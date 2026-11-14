//! TEMPORARY local mirror of the agreed `kora_core::firewall::ports` API (same names and
//! signatures). Once that module lands in kora-core, delete this file and change the imports in
//! `wizard.rs` to `kora_core::firewall::...`.

use std::net::{TcpListener, UdpSocket};

pub trait PortProber {
    fn tcp_free(&self, port: u16) -> bool;
    fn udp_free(&self, port: u16) -> bool;
}

pub struct SystemProber;

impl PortProber for SystemProber {
    fn tcp_free(&self, port: u16) -> bool {
        TcpListener::bind(("0.0.0.0", port)).is_ok()
    }
    fn udp_free(&self, port: u16) -> bool {
        UdpSocket::bind(("0.0.0.0", port)).is_ok()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortStatus {
    Free,
    InUse { suggestion: Option<u16> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortReport {
    pub game: PortStatus,
    pub txadmin: PortStatus,
}

pub fn registered_ports() -> Vec<u16> {
    Vec::new()
}

pub fn suggest_free_port(start: u16, avoid: &[u16], need_udp: bool, prober: &dyn PortProber) -> Option<u16> {
    let first = (u32::from(start) + 1).max(1024);
    (first..=65535u32)
        .take(500)
        .map(|p| p as u16)
        .find(|p| !avoid.contains(p) && prober.tcp_free(*p) && (!need_udp || prober.udp_free(*p)))
}

pub fn check_ports(game: u16, txadmin: u16, avoid: &[u16], prober: &dyn PortProber) -> PortReport {
    let one = |p: u16, udp: bool| {
        if prober.tcp_free(p) && (!udp || prober.udp_free(p)) {
            PortStatus::Free
        } else {
            PortStatus::InUse { suggestion: suggest_free_port(p, avoid, udp, prober) }
        }
    };
    PortReport { game: one(game, true), txadmin: one(txadmin, false) }
}
