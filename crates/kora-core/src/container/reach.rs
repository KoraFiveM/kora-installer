//! What the reachability checklist says differently for a container server.
//!
//! The checklist's questions (is it listening, is the firewall in the way, is the router forwarding) are asked
//! of the host's published port, so they work unchanged: a published port is an ordinary listener on the host
//! (rootless Podman and rootless Docker forward it from a normal program; Docker's own `docker-proxy` holds
//! the UDP/TCP socket). What changes is the firewall line. A **root** engine (Docker, or Podman as root)
//! publishes a port by writing its own firewall rules, ahead of `ufw` and `firewalld`: the host firewall's
//! "closed" does not close it, and its "open" is not what opened it. The line says so plainly instead of
//! telling the person to open a port that is already open.

use super::engine::Engine;
use crate::reach::{FixId, LayerId, ReachReport, ReachState};

/// Adjust the report for a server run by `engine`.
pub fn adjust(report: &mut ReachReport, engine: &Engine) {
    let rooted = engine.bypasses_host_firewall();
    let label = engine.kind.label();
    for l in report.layers.iter_mut().filter(|l| l.id == LayerId::Firewall) {
        if rooted {
            let blocked = matches!(l.state, ReachState::Problem | ReachState::Unknown);
            if blocked {
                l.state = ReachState::Likely;
            }
            l.title = format!("{label} opens this server's port itself; your firewall does not control it");
            l.detail = format!(
                "{label} publishes ports with its own firewall rules, which are applied before ufw or firewalld, so the port is open to the network even where your firewall says it is closed. If you want to limit who can reach it, publish it on one address (the server's \"publish on\" setting) or add rules to {label}'s DOCKER-USER chain."
            );
            l.steps.clear();
            l.fixes.retain(|f| f.id != FixId::OpenFirewall);
        } else if !l.detail.is_empty() {
            l.detail.push_str(&format!(" ({label} runs without administrator rights here and forwards the port from an ordinary program, so your firewall applies to it like to any other.)"));
        } else {
            l.detail = format!("{label} runs without administrator rights here and forwards the port from an ordinary program, so your firewall applies to it like to any other.");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::engine::EngineKind;
    use super::*;
    use crate::reach::{assess, sample_facts, FirewallFacts};

    fn report(kind: Option<&'static str>, opened: bool) -> ReachReport {
        let mut f = sample_facts("main", 30120);
        f.firewall = FirewallFacts { kind, opened_by_kora: opened, readable: true };
        assess(&f)
    }

    fn fw(r: &ReachReport) -> &crate::reach::Layer {
        r.layers.iter().find(|l| l.id == LayerId::Firewall).unwrap()
    }

    #[test]
    fn docker_bypassing_ufw_is_said_and_no_open_button_is_offered() {
        let mut r = report(Some("ufw"), false);
        assert_eq!(fw(&r).state, ReachState::Problem, "as a host service this would be a problem");
        adjust(&mut r, &Engine::new(EngineKind::Docker, false));
        let l = fw(&r);
        assert_eq!(l.state, ReachState::Likely);
        assert!(l.title.contains("Docker opens this server's port itself") && l.detail.contains("ufw") && l.detail.contains("DOCKER-USER"), "{l:?}");
        assert!(l.fixes.iter().all(|f| f.id != FixId::OpenFirewall) && l.steps.is_empty());
    }

    #[test]
    fn rootless_podman_keeps_the_hosts_firewall_in_charge() {
        let mut r = report(Some("ufw"), false);
        let before = fw(&r).clone();
        adjust(&mut r, &Engine::new(EngineKind::Podman, true));
        let l = fw(&r);
        assert_eq!(l.state, before.state);
        assert_eq!(l.fixes, before.fixes, "opening the port in ufw is still the fix");
        assert!(l.detail.contains("without administrator rights") && l.detail.starts_with(&before.detail));
    }
}
