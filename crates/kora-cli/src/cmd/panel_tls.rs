//! `kora panel tls ...` and `kora panel proxy ...`: HTTPS for the panel and the trusted reverse proxy.
//! The settings live in the store; a running panel notices a change within about twenty seconds, so no
//! restart is needed (except to move the panel to another address).

use std::path::Path;
use std::sync::Arc;

use clap::Subcommand;
use kora_web::remote::{self, proxy, SystemWhois};
use kora_web::tls::config::{TlsConfig, TlsMode};
use kora_web::tls::manager::{self, Hints, Manager};
use kora_web::tls::{certs, selfsigned, CertInfo};

#[derive(Subcommand, Debug)]
pub(crate) enum TlsCmd {
    /// Show whether the panel uses HTTPS, which certificate, when it ends, and its fingerprint.
    Status,
    /// Use a certificate and key you already have (PEM files). The panel re-reads them when they change, so renewing with certbot or a script needs no restart.
    Set {
        /// The certificate file (PEM, with any intermediate certificates after it).
        #[arg(long)]
        cert: std::path::PathBuf,
        /// The private key file (PEM). Other accounts must not be able to read it (chmod 600).
        #[arg(long)]
        key: std::path::PathBuf,
    },
    /// Make a certificate on this computer, from a local certificate authority you trust once in your browser. For a panel on your home network, where no public certificate is possible.
    SelfSigned {
        /// A name or address people will open the panel with (repeat for several). This computer's name and addresses are added automatically.
        #[arg(long = "name", value_name = "NAME")]
        names: Vec<String>,
    },
    /// Use the certificate Tailscale issues for this computer's tailnet name (needs MagicDNS and HTTPS certificates enabled in the Tailscale admin console).
    Tailscale,
    /// Use whatever is available: your own certificate if one is set, else Tailscale's when the panel listens on Tailscale, else plain HTTP. The default.
    Auto,
    /// Serve plain HTTP even on addresses other computers can reach (not recommended).
    Off,
    /// Fetch or make the certificate again now (Tailscale and self-signed).
    Renew,
    /// Save the local certificate authority's certificate (public) to a file, to install in a browser or on a phone.
    ExportCa {
        /// Where to write it. Default: kora-local-ca.crt in the Kora folder.
        #[arg(long)]
        to: Option<std::path::PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ProxyCmd {
    /// Trust a reverse proxy (Caddy, nginx) in front of the panel. Prints a secret the proxy must send; forwarded headers are accepted only from it.
    Trust {
        /// The proxy's address as the panel sees it (127.0.0.1 when it runs on this computer). Repeat for several.
        #[arg(required = true)]
        addresses: Vec<std::net::IpAddr>,
        /// The panel address the proxy forwards to, for the configuration examples.
        #[arg(long, default_value = "127.0.0.1:47120")]
        panel: String,
    },
    /// Stop trusting any proxy: forwarded headers are refused again.
    Forget,
    /// Show the trusted proxy setting.
    Status,
}

/// `https` when the panel has a Tailscale (or other) certificate it can serve on the tailnet, else `http` (inside Tailscale either is
/// encrypted). Reads what the running panel saved; fetches nothing.
pub(crate) fn tailnet_scheme(home: &Path) -> &'static str {
    // The cached Tailscale certificate is read without fetching; its name comes from Tailscale's status.
    let m = Manager::new(home.to_path_buf(), Arc::new(SystemWhois));
    m.set_hints(Hints { names: vec![], tailnet: true });
    if m.reload(false).ok().flatten().is_some() {
        "https"
    } else {
        "http"
    }
}

fn home() -> std::path::PathBuf {
    kora_core::monitor::panel_service::home()
}

fn hints_from_settings(home: &Path) -> Hints {
    let names = remote::Remote::from_file(home.join(remote::config::FILE_NAME)).map(|r| r.extra_hosts()).unwrap_or_default();
    Hints { names, tailnet: false }
}

/// A name or address that can go in the allowed hosts (the remote settings' own rule).
fn valid_host_name(n: &str) -> bool {
    remote::config::valid_host(n).is_ok()
}

fn describe(i: &CertInfo, days: i64) {
    println!("  Certificate: {}", i.source.label());
    println!("  Names: {}", i.names.join(", "));
    println!("  Valid until: {} ({} days left){}", certs::format_date(i.not_after), days, if days <= 14 { "  <- renew soon" } else { "" });
    println!("  Fingerprint (SHA-256): {}", i.fingerprint);
    if let Some(ca) = &i.ca_fingerprint {
        println!("  Local CA fingerprint: {ca}");
    }
}

fn mode_text(m: TlsMode) -> &'static str {
    match m {
        TlsMode::Auto => "automatic",
        TlsMode::Provided => "your own certificate",
        TlsMode::Tailscale => "Tailscale certificate",
        TlsMode::SelfSigned => "self-signed (local CA)",
        TlsMode::Off => "off (plain HTTP everywhere)",
    }
}

pub(crate) fn tls(cmd: &TlsCmd) -> i32 {
    match run_tls(cmd, &home()) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn run_tls(cmd: &TlsCmd, home: &Path) -> Result<(), String> {
    let whois = || -> Arc<dyn remote::Whois> { Arc::new(SystemWhois) };
    let save = |c: TlsConfig, fetch: bool| manager::apply(home, whois(), hints_from_settings(home), c, fetch);
    let store = || kora_core::store::Store::open(home).map_err(|p| p.one_line());
    match cmd {
        TlsCmd::Status => {
            let m = Manager::new(home.to_path_buf(), whois());
            m.set_hints(hints_from_settings(home));
            let _ = m.reload(false);
            let r = m.report();
            println!("HTTPS for the panel: {}", mode_text(r.mode));
            match (&r.info, r.days_left) {
                (Some(i), Some(d)) => describe(i, d),
                _ => println!("  No certificate is in use: the panel serves plain HTTP (fine on this computer or inside Tailscale, not on a home network or the internet)."),
            }
            if let Some(e) = &r.error {
                println!("  Problem: {e}");
            }
            println!("Loopback (this computer) always stays plain HTTP. Other addresses use HTTPS once there is a certificate.");
            Ok(())
        }
        TlsCmd::Set { cert, key } => {
            let abs = |p: &Path| std::fs::canonicalize(p).map_err(|e| format!("cannot find {} ({e})", p.display()));
            let (cert, key) = (abs(cert)?, abs(key)?);
            let c = TlsConfig { mode: TlsMode::Provided, cert_path: Some(cert.to_string_lossy().into()), key_path: Some(key.to_string_lossy().into()), ..TlsConfig::load(&store()?)? };
            let (_, info) = save(c, false)?;
            let i = info.ok_or("no certificate was loaded")?;
            println!("The panel will use this certificate:");
            describe(&i, certs::days_left(i.not_after, manager::now_unix()));
            println!("It is re-read when the files change. A running panel switches within about 20 seconds.");
            Ok(())
        }
        TlsCmd::SelfSigned { names } => {
            let c = TlsConfig { mode: TlsMode::SelfSigned, names: names.clone(), ..TlsConfig::load(&store()?)? };
            let (_, info) = save(c, false)?;
            let i = info.ok_or("no certificate was made")?;
            let to = home.join("kora-local-ca.crt");
            let ca_fp = manager::export_ca(home, &to)?;
            println!("Made a certificate for this panel from a local authority that only exists on this computer.");
            describe(&i, certs::days_left(i.not_after, manager::now_unix()));
            println!();
            println!("Trust it ONCE, so browsers stop warning. The authority's certificate is saved here:\n  {}", to.display());
            println!("Before you install it on another computer, compare its fingerprint with this one: {ca_fp}");
            for (what, how) in selfsigned::trust_instructions(&to.to_string_lossy()) {
                println!("  {what}: {how}");
            }
            // The names given here are the names people will type: the panel must also accept them in the `Host` header.
            if !names.is_empty() {
                let path = home.join(remote::config::FILE_NAME);
                let added = remote::RemoteConfig::update_at(&path, |c| {
                    let mut added = Vec::new();
                    for n in selfsigned::clean_names(names) {
                        if valid_host_name(&n) && !c.allowed_hosts.iter().any(|h| h.eq_ignore_ascii_case(&n)) {
                            c.allowed_hosts.push(n.clone());
                            added.push(n);
                        }
                    }
                    Ok(added)
                })?;
                if !added.is_empty() {
                    println!("Added {} to the names the panel answers to (Settings > Remote access).", added.join(", "));
                }
            }
            println!("Open the panel with one of the names above. Names not listed give a warning: add them with `kora panel tls self-signed --name <name>`.");
            Ok(())
        }
        TlsCmd::Tailscale => {
            let c = TlsConfig { mode: TlsMode::Tailscale, ..TlsConfig::load(&store()?)? };
            let (_, info) = save(c, true)?;
            let i = info.ok_or("no certificate was loaded")?;
            describe(&i, certs::days_left(i.not_after, manager::now_unix()));
            println!("Open the panel by its Tailscale name (not the 100.x address): the certificate is for that name only.");
            Ok(())
        }
        TlsCmd::Auto => {
            save(TlsConfig { mode: TlsMode::Auto, ..TlsConfig::load(&store()?)? }, false)?;
            println!("The panel uses whatever certificate is available.");
            Ok(())
        }
        TlsCmd::Off => {
            save(TlsConfig { mode: TlsMode::Off, ..TlsConfig::load(&store()?)? }, false)?;
            println!("The panel serves plain HTTP on every address. Only do this inside a private tunnel (Tailscale, WireGuard, SSH).");
            Ok(())
        }
        TlsCmd::Renew => {
            manager::force_renewal(home)?;
            let m = Manager::new(home.to_path_buf(), whois());
            m.set_hints(hints_from_settings(home));
            match m.reload(true)? {
                Some(i) => {
                    describe(&i, certs::days_left(i.not_after, manager::now_unix()));
                    Ok(())
                }
                None => Err("This setting has nothing to renew (a certificate you provide is renewed by you; replace the files).".into()),
            }
        }
        TlsCmd::ExportCa { to } => {
            let to = to.clone().unwrap_or_else(|| home.join("kora-local-ca.crt"));
            let fp = manager::export_ca(home, &to)?;
            println!("Saved {}\nFingerprint (SHA-256): {fp}", to.display());
            Ok(())
        }
    }
}

pub(crate) fn proxy(cmd: &ProxyCmd) -> i32 {
    let path = remote::config::default_path();
    let run = || -> Result<(), String> {
        match cmd {
            ProxyCmd::Trust { addresses, panel } => {
                let (p, secret) = proxy::ProxyConfig::new(addresses.clone(), vec![]);
                remote::RemoteConfig::update_at(&path, |c| {
                    c.trusted_proxy = Some(p);
                    Ok(())
                })?;
                println!("The panel now trusts a reverse proxy at {}.", addresses.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(", "));
                println!("It honours X-Forwarded-For and X-Forwarded-Proto, and only from a request that carries this secret (shown once):");
                println!("  X-Kora-Proxy-Secret: {secret}");
                for (name, text) in proxy::snippets(&secret, panel) {
                    println!("\n{name}:\n{text}");
                }
                println!("\nAlso add the public name to the allowed hosts and turn remote access on (Settings, Remote access). Anything without the secret is still refused.");
                Ok(())
            }
            ProxyCmd::Forget => {
                remote::RemoteConfig::update_at(&path, |c| {
                    c.trusted_proxy = None;
                    Ok(())
                })?;
                println!("No proxy is trusted any more: requests with forwarded headers are refused.");
                Ok(())
            }
            ProxyCmd::Status => {
                let c = remote::RemoteConfig::load(&path)?;
                match c.trusted_proxy {
                    Some(p) => {
                        println!("Trusted proxy: {} (secret set; rotate it by running `kora panel proxy trust` again).", p.addresses.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(", "))
                    }
                    None => println!("No reverse proxy is trusted: requests with forwarded headers are refused."),
                }
                Ok(())
            }
        }
    };
    match run() {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cli, Command};
    use clap::Parser;

    fn parse(a: &[&str]) -> Result<crate::cmd::panel::PanelCmd, String> {
        match Cli::try_parse_from(a).map(|c| c.command) {
            Ok(Some(Command::Panel(p))) => p.cmd.ok_or("no subcommand".to_string()),
            Ok(_) => Err("not the panel command".into()),
            Err(e) => Err(e.to_string()),
        }
    }

    #[test]
    fn the_commands_parse() {
        use crate::cmd::panel::PanelCmd;
        assert!(matches!(parse(&["kora", "panel", "tls", "status"]), Ok(PanelCmd::Tls { cmd: TlsCmd::Status })));
        assert!(matches!(parse(&["kora", "panel", "tls", "set", "--cert", "c.pem", "--key", "k.pem"]), Ok(PanelCmd::Tls { cmd: TlsCmd::Set { .. } })));
        assert!(parse(&["kora", "panel", "tls", "set", "--cert", "c.pem"]).is_err(), "the key is needed too");
        assert!(matches!(parse(&["kora", "panel", "tls", "self-signed", "--name", "box.lan", "--name", "10.0.0.5"]), Ok(PanelCmd::Tls { cmd: TlsCmd::SelfSigned { names } }) if names.len() == 2));
        assert!(matches!(parse(&["kora", "panel", "tls", "off"]), Ok(PanelCmd::Tls { cmd: TlsCmd::Off })));
        assert!(matches!(parse(&["kora", "panel", "proxy", "trust", "127.0.0.1"]), Ok(PanelCmd::Proxy { cmd: ProxyCmd::Trust { .. } })));
        assert!(parse(&["kora", "panel", "proxy", "trust"]).is_err(), "an address is required");
    }

    #[test]
    fn self_signed_then_status_then_off_in_a_fresh_home() {
        let d = tempfile::tempdir().unwrap();
        run_tls(&TlsCmd::SelfSigned { names: vec!["box.lan".into()] }, d.path()).unwrap();
        assert!(d.path().join("kora-local-ca.crt").exists(), "the CA certificate (public) is saved for installing");
        run_tls(&TlsCmd::Status, d.path()).unwrap();
        let s = kora_core::store::Store::open(d.path()).unwrap();
        assert_eq!(TlsConfig::load(&s).unwrap().mode, TlsMode::SelfSigned);
        // The private key never reaches the exported file.
        let ca = std::fs::read_to_string(d.path().join("kora-local-ca.crt")).unwrap();
        assert!(ca.contains("BEGIN CERTIFICATE") && !ca.contains("PRIVATE KEY"));
        run_tls(&TlsCmd::Renew, d.path()).unwrap();
        run_tls(&TlsCmd::Off, d.path()).unwrap();
        assert_eq!(TlsConfig::load(&s).unwrap().mode, TlsMode::Off);
    }

    #[test]
    fn a_bad_certificate_changes_nothing() {
        let d = tempfile::tempdir().unwrap();
        let (c, k) = (d.path().join("c.pem"), d.path().join("k.pem"));
        std::fs::write(&c, "not a certificate").unwrap();
        std::fs::write(&k, "not a key").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&k, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let e = run_tls(&TlsCmd::Set { cert: c, key: k }, d.path()).unwrap_err();
        assert!(e.contains("certificate"), "{e}");
        let s = kora_core::store::Store::open(d.path()).unwrap();
        assert_eq!(TlsConfig::load(&s).unwrap().mode, TlsMode::Auto, "the old setting is back");
    }
}
