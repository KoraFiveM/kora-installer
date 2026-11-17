//! `kora reach <server>`: "Can players reach my server?" as a checklist in words, plus the actions
//! that go with it (automatic port forwarding, opening the firewall, what outside lookups are allowed).
//! The same checks as the panel's Overview card (`Op::ReachCheck`).

use clap::Args;
use kora_core::api::{Backend, SystemBackend};
use kora_core::installer::registry;
use kora_core::reach::settings::Consent;
use kora_core::reach::{ReachReport, State};

#[derive(Args, Debug, Clone)]
pub(crate) struct ReachArgs {
    /// Server name or folder.
    pub server: String,
    /// Also look the server up on Cfx.re's public server list (for this check; --remember keeps the choice).
    #[arg(long)]
    pub cfx_list: bool,
    /// Also ask api.ipify.org for this connection's public address (for this check; --remember keeps the choice).
    #[arg(long)]
    pub ip_echo: bool,
    /// Keep the outside lookups chosen above for next time.
    #[arg(long)]
    pub remember: bool,
    /// Turn automatic port forwarding (UPnP / NAT-PMP) of the game port on or off for this server.
    #[arg(long, value_parser = ["on", "off"])]
    pub forward: Option<String>,
    /// Open the game port (TCP and UDP) in this computer's firewall. Needs administrator rights.
    #[arg(long)]
    pub open_firewall: bool,
    /// Print the result as JSON.
    #[arg(long)]
    pub json: bool,
}

fn mark(s: State) -> &'static str {
    match s {
        State::Ok => "[ OK ]    ",
        State::Likely => "[ LIKELY ]",
        State::Problem => "[ PROBLEM ]",
        State::Unknown => "[ UNKNOWN ]",
    }
}

/// The report as lines for a terminal.
pub(crate) fn render(r: &ReachReport) -> Vec<String> {
    let mut out = vec![format!("Can players reach '{}'?", r.server), String::new()];
    for l in &r.layers {
        out.push(format!("{} {}", mark(l.state), l.title));
        if !l.detail.is_empty() {
            out.push(format!("             {}", l.detail));
        }
        for f in &l.fixes {
            out.push(format!("             Fix: {}", f.label));
        }
        for (i, s) in l.steps.iter().enumerate() {
            out.push(format!("             {}. {s}", i + 1));
        }
    }
    out.push(String::new());
    out.push(r.headline.clone());
    if r.connect.lan.is_some() || r.connect.public.is_some() || r.connect.cfx.is_some() {
        out.push(String::new());
        out.push("Addresses to give people:".into());
        if let Some(a) = &r.connect.lan {
            out.push(format!("  Same network:  {a}"));
        }
        if let Some(a) = &r.connect.public {
            out.push(format!("  Internet:      {a}"));
        }
        if let Some(a) = &r.connect.cfx {
            out.push(format!("  Cfx.re link:   {a}"));
        }
        if let Some(c) = &r.connect.command {
            out.push(format!("  In FiveM (F8): {c}"));
        }
    }
    for c in &r.contacted {
        out.push(format!("Asked {}: it learned {}.", c.what, c.learned));
    }
    for n in &r.needs_consent {
        out.push(match *n {
            "cfx_list" => "Tip: add --cfx-list to look your server up on Cfx.re's public server list (add --remember to keep this).".into(),
            _ => "Tip: add --ip-echo to find your public address through api.ipify.org (add --remember to keep this).".into(),
        });
    }
    out
}

pub(crate) fn run(a: ReachArgs) -> i32 {
    let fail = |m: String| {
        eprintln!("error: {m}");
        1
    };
    let reg = match registry::Registry::load() {
        Ok(r) => r,
        Err(e) => return fail(format!("cannot read the server registry: {e}")),
    };
    let Some(entry) = reg.find(&a.server).cloned() else {
        return fail(format!("no registered server named '{}' (see `kora status`)", a.server));
    };
    let mut b = SystemBackend;
    if let Some(f) = &a.forward {
        match b.forwarding_set(&entry, f == "on") {
            Ok(v) if v.enabled => println!("Automatic port forwarding is ON for '{}'. Kora asks your router to forward port {} (TCP and UDP) while the server runs, and takes it away when it stops. (A router that programs can change is a risk worth knowing about: turn it off again with --forward off.)", entry.name, entry.ports.game),
            Ok(_) => println!("Automatic port forwarding is OFF for '{}'. Anything Kora forwarded was removed.", entry.name),
            Err(e) => return fail(kora_core::problem::plain_text(&e)),
        }
    }
    if a.open_firewall {
        return match b.firewall_open(&entry) {
            Ok(notes) => {
                notes.iter().for_each(|n| println!("{n}"));
                0
            }
            Err(e) => fail(kora_core::problem::plain_text(&e)),
        };
    }
    let saved = b.reach_consent();
    let wanted = Consent { cfx_list: saved.cfx_list || a.cfx_list, ip_echo: saved.ip_echo || a.ip_echo };
    if a.remember && wanted != saved {
        if let Err(e) = b.reach_consent_set(wanted) {
            return fail(kora_core::problem::plain_text(&e));
        }
    }
    let once = (wanted != saved).then_some(wanted);
    match b.reach_check(&entry, once) {
        Ok(r) => {
            if a.json {
                println!("{}", serde_json::to_string_pretty(&r).unwrap_or_default());
            } else {
                render(&r).iter().for_each(|l| println!("{l}"));
            }
            0
        }
        Err(e) => fail(kora_core::problem::plain_text(&e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kora_core::reach::{assess, sample_facts};

    #[test]
    fn the_checklist_reads_in_words_with_the_exact_values() {
        let r = assess(&sample_facts("main", 30120));
        let text = render(&r).join("\n");
        assert!(text.contains("Can players reach 'main'?"));
        assert!(text.contains("[ OK ]") && text.contains("Your router must forward port 30120"), "{text}");
        assert!(text.contains("outside/external port 30120") && text.contains("192.168.1.50"), "{text}");
        assert!(text.contains("Same network:  192.168.1.50:30120"), "{text}");
        assert!(text.contains("Internet:      84.12.5.6:30120"), "{text}");
        assert!(text.contains("--cfx-list") || !r.needs_consent.contains(&"cfx_list"));
    }
}
