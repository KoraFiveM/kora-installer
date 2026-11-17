//! `kora webhook`: Discord notifications for update reports.

use clap::Subcommand;
use kora_core::installer::registry;

#[derive(Subcommand)]
pub(crate) enum WebhookCmd {
    /// Save a Discord webhook for a server. Prefer --stdin or $KORA_DISCORD_WEBHOOK so the URL stays out of shell history.
    Set {
        /// Server name or folder.
        server: String,
        /// The webhook URL (https://discord.com/api/webhooks/<id>/<token>).
        url: Option<String>,
        /// Read the URL from standard input.
        #[arg(long)]
        stdin: bool,
        /// Send a test message after saving.
        #[arg(long)]
        test: bool,
    },
    /// Send a test message.
    Test { server: String },
    /// Show whether a webhook is set (the token is hidden).
    Show { server: String },
    /// Delete the saved webhook.
    Remove { server: String },
}

pub(crate) fn webhook(cmd: WebhookCmd) -> i32 {
    use kora_core::notify;
    let find = |s: &str| -> Result<registry::ServerEntry, String> {
        let reg = registry::Registry::load().map_err(|e| format!("cannot read the server registry: {e}"))?;
        reg.find(s).cloned().ok_or_else(|| format!("no registered server named '{s}' (see `kora status`)"))
    };
    let fail = |m: String| {
        eprintln!("error: {m}");
        1
    };
    let test = |srv: &registry::ServerEntry, cfg: &notify::NotifyConfig| match notify::send_test(cfg, &srv.name) {
        Ok(()) => {
            println!("Test message sent.");
            0
        }
        Err(e) => fail(format!("test message failed: {e}")),
    };
    match cmd {
        WebhookCmd::Set { server, url, stdin, test: do_test } => {
            let srv = match find(&server) {
                Ok(s) => s,
                Err(e) => return fail(e),
            };
            let env = std::env::var(notify::ENV_VAR).ok();
            let url = match notify::resolve_url_input(url.as_deref(), stdin, env.as_deref(), &mut std::io::stdin().lock()) {
                Ok(u) => u,
                Err(notify::NotifyError::NotConfigured) => return fail(format!("give a URL, use --stdin, or set ${}", notify::ENV_VAR)),
                Err(e) => return fail(e.to_string()),
            };
            let cfg = notify::NotifyConfig { discord_webhook: Some(url) };
            if let Err(e) = notify::save(&srv, &cfg) {
                return fail(e.to_string());
            }
            println!("Saved Discord webhook for '{}': {}", srv.name, notify::redact(cfg.discord_webhook.as_deref().unwrap_or("")));
            if do_test {
                return test(&srv, &cfg);
            }
            println!("Run `kora webhook test {}` to check it.", srv.name);
            0
        }
        WebhookCmd::Test { server } => match find(&server) {
            Ok(srv) => {
                let cfg = notify::load(&srv);
                if cfg.discord_webhook.is_none() {
                    return fail(format!("no webhook set for '{}'; use `kora webhook set`", srv.name));
                }
                test(&srv, &cfg)
            }
            Err(e) => fail(e),
        },
        WebhookCmd::Show { server } => match find(&server) {
            Ok(srv) => {
                match notify::load(&srv).discord_webhook {
                    Some(u) => println!("{}: {}", srv.name, notify::redact(&u)),
                    None => println!("{}: no webhook set", srv.name),
                }
                0
            }
            Err(e) => fail(e),
        },
        WebhookCmd::Remove { server } => match find(&server) {
            Ok(srv) => match notify::save(&srv, &notify::NotifyConfig::default()) {
                Ok(()) => {
                    println!("Removed the webhook for '{}'.", srv.name);
                    0
                }
                Err(e) => fail(e.to_string()),
            },
            Err(e) => fail(e),
        },
    }
}

#[cfg(test)]
mod tests {

    use crate::Cli;
    use clap::Parser;

    #[test]
    fn webhook_subcommands_parse() {
        let p = |a: &[&str]| Cli::try_parse_from(std::iter::once("kora").chain(a.iter().copied()));
        assert!(p(&["webhook", "set", "alpha", "--stdin", "--test"]).is_ok());
        assert!(p(&["webhook", "set", "alpha", "https://discord.com/api/webhooks/1/t"]).is_ok());
        assert!(p(&["webhook", "test", "alpha"]).is_ok());
        assert!(p(&["webhook", "show", "alpha"]).is_ok());
        assert!(p(&["webhook", "remove", "alpha"]).is_ok());
        assert!(p(&["webhook", "set"]).is_err());
        assert!(p(&["webhook"]).is_err());
    }
}
