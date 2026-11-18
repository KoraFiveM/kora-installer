//! Discord webhook notifications for update reports.
//!
//! The webhook URL is a secret: it is stored per server in `<server>/.kora/notify.json`
//! (0600 on Unix, icacls-restricted on Windows), is never printed or logged in full (only as
//! `discord.com/api/webhooks/<id>/***`), and only Discord hosts are accepted. Sending never
//! panics and never fails an update: [`after_update`] swallows every error into `update.log`.
//! The payload carries only the server name, counts and resource ids (no paths, IPs or secrets).

use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::db::{self, Os};
use crate::installer::registry::ServerEntry;
use crate::update::UpdateReport;

/// The registry entry of a server (only `name` and `dir` are used here).
pub type ServerRecord = ServerEntry;

/// Environment variable `kora webhook set` reads when no URL is given.
pub const ENV_VAR: &str = "KORA_DISCORD_WEBHOOK";

const MAX_BODY: usize = 100_000;
const MAX_RESPONSE: u64 = 64 * 1024;
const MAX_LINES: usize = 10;
const MAX_RETRIES: u32 = 2;
const MAX_RETRY_WAIT: Duration = Duration::from_secs(30);
const TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NotifyError {
    #[error("no Discord webhook is configured")]
    NotConfigured,
    #[error("not a valid Discord webhook URL: {0}")]
    InvalidUrl(&'static str),
    #[error("the notification is too large to send")]
    TooLarge,
    #[error("Discord is rate limiting this webhook; try again later")]
    RateLimited,
    #[error("Discord answered with HTTP {0}")]
    Http(u16),
    #[error("Discord redirected the request; refusing to follow it")]
    Redirected,
    #[error("timed out talking to Discord")]
    Timeout,
    #[error("could not reach Discord")]
    Network,
    #[error("could not save the webhook settings: {0}")]
    Io(String),
}

// ------------------------------------------------------------------ config

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotifyConfig {
    #[serde(default)]
    pub discord_webhook: Option<String>,
}

impl std::fmt::Debug for NotifyConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotifyConfig").field("discord_webhook", &self.discord_webhook.as_deref().map(redact)).finish()
    }
}

pub fn config_path(server: &ServerRecord) -> PathBuf {
    server.dir.join(".kora").join("notify.json")
}

/// The saved settings; a missing or unreadable file means "no webhook".
pub fn load(server: &ServerRecord) -> NotifyConfig {
    std::fs::read(config_path(server)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Save the settings (owner-only file). An empty config removes the file.
pub fn save(server: &ServerRecord, cfg: &NotifyConfig) -> Result<(), NotifyError> {
    let path = config_path(server);
    if cfg.discord_webhook.is_none() {
        return match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(NotifyError::Io(e.to_string())),
        };
    }
    let io = |e: std::io::Error| NotifyError::Io(e.to_string());
    std::fs::create_dir_all(path.parent().unwrap_or(Path::new("."))).map_err(io)?;
    let text = serde_json::to_string_pretty(cfg).map_err(|e| NotifyError::Io(e.to_string()))?;
    let tmp = path.with_extension("json.tmp");
    let user = std::env::var("USERNAME").ok().filter(|u| !u.is_empty());
    db::write_secret_file(&db::SystemRunner, Os::current(), &tmp, &text, user.as_deref()).map_err(|e| NotifyError::Io(e.to_string()))?;
    crate::fsutil::rename_retrying(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        io(e)
    })
}

/// The icacls command that locks `notify.json` to `windows_user` (testable on every OS).
pub fn windows_acl_plan(server: &ServerRecord, windows_user: &str) -> Result<db::CommandSpec, NotifyError> {
    db::windows_acl_command(&config_path(server), windows_user).map_err(|e| NotifyError::Io(e.to_string()))
}

// ------------------------------------------------------------- validation

const HOSTS: [&str; 6] = ["discord.com", "ptb.discord.com", "canary.discord.com", "discordapp.com", "ptb.discordapp.com", "canary.discordapp.com"];

/// `(host, id, token)` of a well-formed Discord webhook URL.
fn parse(url: &str) -> Result<(String, &str, &str), NotifyError> {
    use NotifyError::InvalidUrl as E;
    if url.len() > 400 {
        return Err(E("too long"));
    }
    if !url.is_ascii() || url.bytes().any(|b| b.is_ascii_control() || b == b' ' || b == b'\\') {
        return Err(E("unexpected characters"));
    }
    let rest = url.strip_prefix("https://").ok_or(E("must start with https://"))?;
    let (host, path) = rest.split_once('/').ok_or(E("missing path"))?;
    if host.contains('@') || host.contains(':') {
        return Err(E("user info and ports are not allowed"));
    }
    let host = host.to_ascii_lowercase();
    if !HOSTS.contains(&host.as_str()) {
        return Err(E("host must be discord.com or discordapp.com"));
    }
    let tail = path.strip_prefix("api/webhooks/").ok_or(E("path must be /api/webhooks/<id>/<token>"))?;
    let (id, token) = tail.split_once('/').ok_or(E("path must be /api/webhooks/<id>/<token>"))?;
    if id.is_empty() || id.len() > 25 || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err(E("webhook id must be a number"));
    }
    if token.is_empty() || token.len() > 200 || !token.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
        return Err(E("webhook token has unexpected characters"));
    }
    Ok((host, id, token))
}

/// Only `https://discord.com/api/webhooks/<id>/<token>` (and discordapp.com, ptb./canary.).
pub fn validate_webhook(url: &str) -> Result<(), NotifyError> {
    parse(url.trim_end_matches('\n')).map(|_| ())
}

/// `discord.com/api/webhooks/<id>/***` - safe to print and log.
pub fn redact(url: &str) -> String {
    match parse(url.trim()) {
        Ok((host, id, _)) => format!("{host}/api/webhooks/{id}/***"),
        Err(_) => "<webhook url hidden>".to_string(),
    }
}

/// Take the URL from stdin, the argument or `$KORA_DISCORD_WEBHOOK` (in that order) and validate it.
pub fn resolve_url_input(arg: Option<&str>, use_stdin: bool, env: Option<&str>, stdin: &mut dyn BufRead) -> Result<String, NotifyError> {
    let raw = if use_stdin {
        let mut line = String::new();
        stdin.take(1024).read_line(&mut line).map_err(|e| NotifyError::Io(e.to_string()))?;
        line
    } else {
        arg.or(env).ok_or(NotifyError::NotConfigured)?.to_string()
    };
    let url = raw.trim().to_string();
    validate_webhook(&url)?;
    Ok(url)
}

// ---------------------------------------------------------------- payload

fn limit_lines(value: &str) -> String {
    let lines: Vec<&str> = value.lines().collect();
    if lines.len() <= MAX_LINES {
        return value.to_string();
    }
    format!("{}\n…and {} more", lines[..MAX_LINES].join("\n"), lines.len() - MAX_LINES)
}

/// The existing report embed, plus a counts line, the first lines of each list, ids only for
/// problems (messages may carry paths) and no mentions.
pub fn build_payload(report: &UpdateReport) -> serde_json::Value {
    let mut p = report.discord_payload();
    let counts = format!("{} updated · {} skipped · {} rolled back · {} need attention", report.updated.len(), report.skipped.len(), report.rolled_back.len(), report.needs_attention.len());
    let ids = |v: Vec<&String>| v.into_iter().cloned().collect::<Vec<_>>().join("\n");
    let rolled = ids(report.rolled_back.iter().map(|(i, _)| i).collect());
    let attention = ids(report.needs_attention.iter().map(|(i, _)| i).collect());
    if let Some(embed) = p["embeds"].get_mut(0) {
        embed["description"] = counts.into();
        let mut fields = Vec::new();
        for f in embed["fields"].as_array().cloned().unwrap_or_default() {
            let name = f["name"].as_str().unwrap_or("").to_string();
            let value = match name.as_str() {
                "Rolled back" => rolled.clone(),
                "Needs attention" => attention.clone(),
                _ => f["value"].as_str().unwrap_or("").to_string(),
            };
            if value.is_empty() || value == "-" {
                continue;
            }
            fields.push(serde_json::json!({"name": name, "value": limit_lines(&value).chars().take(1000).collect::<String>()}));
        }
        embed["fields"] = fields.into();
    }
    p["allowed_mentions"] = serde_json::json!({"parse": []});
    p
}

fn test_payload(server_name: &str) -> serde_json::Value {
    serde_json::json!({
        "username": "Kora Updater",
        "allowed_mentions": {"parse": []},
        "embeds": [{
            "title": format!("Kora webhook test: {server_name}"),
            "description": "Update reports for this server will be posted here.",
            "color": 0x3498db,
        }]
    })
}

// ---------------------------------------------------------------- sending

struct Net {
    timeout: Duration,
    sleep: fn(Duration),
}

fn real_net() -> Net {
    Net { timeout: TIMEOUT, sleep: std::thread::sleep }
}

fn is_timeout(t: &ureq::Transport) -> bool {
    use std::error::Error;
    let mut src: Option<&(dyn Error + 'static)> = t.source();
    while let Some(e) = src {
        if let Some(io) = e.downcast_ref::<std::io::Error>() {
            return matches!(io.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock);
        }
        src = e.source();
    }
    false
}

fn retry_after(resp: ureq::Response) -> Duration {
    let header = resp.header("retry-after").and_then(|h| h.trim().parse::<f64>().ok());
    let mut buf = Vec::new();
    let _ = resp.into_reader().take(MAX_RESPONSE).read_to_end(&mut buf);
    let body = serde_json::from_slice::<serde_json::Value>(&buf).ok().and_then(|v| v["retry_after"].as_f64());
    let secs = body.or(header).unwrap_or(1.0);
    Duration::from_secs_f64(secs.clamp(0.0, 3600.0))
}

/// POST `body` to `url` (already validated by the caller); no redirects, bounded time and size.
fn post(url: &str, body: &serde_json::Value, net: &Net) -> Result<(), NotifyError> {
    let text = body.to_string();
    if text.len() > MAX_BODY {
        return Err(NotifyError::TooLarge);
    }
    let agent = ureq::AgentBuilder::new().redirects(0).timeout(net.timeout).timeout_connect(net.timeout).build();
    let mut attempt = 0;
    loop {
        let res = agent.post(url).set("Content-Type", "application/json").set("User-Agent", "kora-installer").send_string(&text);
        match res {
            Ok(r) if r.status() >= 300 => return Err(if r.status() < 400 { NotifyError::Redirected } else { NotifyError::Http(r.status()) }),
            Ok(r) => {
                // drain a bounded amount so the connection ends cleanly
                let _ = r.into_reader().take(MAX_RESPONSE).read_to_end(&mut Vec::new());
                return Ok(());
            }
            Err(ureq::Error::Status(429, r)) => {
                let wait = retry_after(r);
                if attempt >= MAX_RETRIES || wait > MAX_RETRY_WAIT {
                    return Err(NotifyError::RateLimited);
                }
                attempt += 1;
                (net.sleep)(wait);
            }
            Err(ureq::Error::Status(c, _)) => return Err(if (300..400).contains(&c) { NotifyError::Redirected } else { NotifyError::Http(c) }),
            Err(ureq::Error::Transport(t)) => return Err(if is_timeout(&t) { NotifyError::Timeout } else { NotifyError::Network }),
        }
    }
}

fn configured(cfg: &NotifyConfig) -> Result<&str, NotifyError> {
    let url = cfg.discord_webhook.as_deref().ok_or(NotifyError::NotConfigured)?;
    validate_webhook(url)?;
    Ok(url)
}

/// Post the update report to the configured webhook.
pub fn send_update_report(cfg: &NotifyConfig, report: &UpdateReport) -> Result<(), NotifyError> {
    let url = configured(cfg)?;
    std::panic::catch_unwind(|| post(url, &build_payload(report), &real_net())).unwrap_or(Err(NotifyError::Network))
}

/// Post a "this works" message.
pub fn send_test(cfg: &NotifyConfig, server_name: &str) -> Result<(), NotifyError> {
    let url = configured(cfg)?;
    std::panic::catch_unwind(|| post(url, &test_payload(server_name), &real_net())).unwrap_or(Err(NotifyError::Network))
}

/// Post a short monitoring alert (title and one line of text; no paths, addresses or secrets).
pub fn send_alert(cfg: &NotifyConfig, title: &str, text: &str, colour: u32) -> Result<(), NotifyError> {
    let url = configured(cfg)?;
    let body = serde_json::json!({
        "username": "Kora Monitor",
        "allowed_mentions": {"parse": []},
        "embeds": [{"title": title, "description": text, "color": colour}]
    });
    std::panic::catch_unwind(|| post(url, &body, &real_net())).unwrap_or(Err(NotifyError::Network))
}

// ------------------------------------------------------------------- hook

/// Is this report worth a message? Runs that changed or broke nothing stay quiet.
pub fn is_noteworthy(r: &UpdateReport) -> bool {
    !r.updated.is_empty() || !r.rolled_back.is_empty() || !r.needs_attention.is_empty() || !r.sql_flagged.is_empty()
}

fn log_warning(server: &ServerRecord, text: &str) {
    use std::io::Write;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(server.dir.join("update.log")) {
        let _ = writeln!(f, "=== unix {now} ===\n{text}");
    }
}

/// Call after any update (auto or manual). Sends the report when a webhook is configured; a
/// failure only becomes a warning line in `update.log`, never an error or panic.
pub fn after_update(server: &ServerRecord, report: &UpdateReport) {
    after_update_with(server, report, &send_update_report);
}

pub fn after_update_with(server: &ServerRecord, report: &UpdateReport, send: &dyn Fn(&NotifyConfig, &UpdateReport) -> Result<(), NotifyError>) {
    let run = std::panic::AssertUnwindSafe(|| {
        let cfg = load(server);
        if cfg.discord_webhook.is_none() || !is_noteworthy(report) {
            return;
        }
        if let Err(e) = send(&cfg, report) {
            log_warning(server, &format!("warning: could not send the Discord notification: {e}"));
        }
    });
    if std::panic::catch_unwind(run).is_err() {
        log_warning(server, "warning: could not send the Discord notification");
    }
}

// ------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::installer::registry::PortsRecord;
    use crate::installer::types::Mode;
    use crate::update::{SkipReason, Updated};
    use std::io::Write;
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    const GOOD: &str = "https://discord.com/api/webhooks/123456789/abc_DEF-ghi";

    fn server(dir: &Path) -> ServerRecord {
        ServerEntry { source: None, versions: None, name: "Alpha".into(), dir: dir.to_path_buf(), mode: Mode::Development, ports: PortsRecord { game: 1, txadmin: 2 }, created_unix: 0 }
    }

    fn report() -> UpdateReport {
        UpdateReport {
            server: "Alpha".into(),
            updated: vec![Updated { id: "qb-core".into(), from: "aaaaaaaa".into(), to: "bbbbbbbb".into() }],
            needs_attention: vec![("ox_lib".into(), "failed in /home/secret/path".into())],
            ..Default::default()
        }
    }

    #[test]
    fn validation_accepts_discord_hosts() {
        for h in ["discord.com", "discordapp.com", "ptb.discord.com", "canary.discord.com", "ptb.discordapp.com", "canary.discordapp.com", "DISCORD.com"] {
            assert!(validate_webhook(&format!("https://{h}/api/webhooks/1/tok_en-1")).is_ok(), "{h}");
        }
    }

    #[test]
    fn validation_rejects_lookalikes_and_tricks() {
        for u in [
            "http://discord.com/api/webhooks/1/t",
            "https://discord.com.evil.net/api/webhooks/1/t",
            "https://evil.net/discord.com/api/webhooks/1/t",
            "https://notdiscord.com/api/webhooks/1/t",
            "https://discord.com@evil.net/api/webhooks/1/t",
            "https://evil.net@discord.com/api/webhooks/1/t",
            "https://discord.com:8443/api/webhooks/1/t",
            "https://discord.com\\@evil.net/api/webhooks/1/t",
            "https://evil.discord.com/api/webhooks/1/t",
            "https://discord.com/api/webhooks/x/t",
            "https://discord.com/api/webhooks/1/",
            "https://discord.com/api/webhooks/1",
            "https://discord.com/api/webhooks/1/t?x=1",
            "https://discord.com/api/webhooks/1/t/extra",
            "https://discord.com/other/1/t",
            "https://discord.com",
            "https://discord.com/api/webhooks/1/t t",
            "ftp://discord.com/api/webhooks/1/t",
            "",
        ] {
            assert!(validate_webhook(u).is_err(), "{u}");
        }
    }

    #[test]
    fn redaction_hides_the_token_everywhere() {
        assert_eq!(redact(GOOD), "discord.com/api/webhooks/123456789/***");
        assert!(!redact("https://evil.net/secret-token").contains("secret"));
        let cfg = NotifyConfig { discord_webhook: Some(GOOD.into()) };
        assert!(!format!("{cfg:?}").contains("abc_DEF"));
        for e in [NotifyError::InvalidUrl("x"), NotifyError::Http(500), NotifyError::RateLimited, NotifyError::Timeout, NotifyError::Network, NotifyError::TooLarge, NotifyError::Redirected] {
            assert!(!e.to_string().contains("abc_DEF"));
        }
    }

    #[test]
    fn errors_from_a_failed_send_never_carry_the_url() {
        // refused connection on a closed port; the error text must not include the URL/token
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/api/webhooks/1/SECRETTOKEN", l.local_addr().unwrap());
        drop(l);
        let e = post(&url, &serde_json::json!({}), &Net { timeout: Duration::from_millis(500), sleep: |_| {} }).unwrap_err();
        assert!(!format!("{e} {e:?}").contains("SECRETTOKEN"));
    }

    #[test]
    fn config_round_trip_and_permissions() {
        let d = tempfile::tempdir().unwrap();
        let s = server(d.path());
        assert_eq!(load(&s), NotifyConfig::default());
        let cfg = NotifyConfig { discord_webhook: Some(GOOD.into()) };
        save(&s, &cfg).unwrap();
        assert_eq!(load(&s), cfg);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(config_path(&s)).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        assert!(!config_path(&s).with_extension("json.tmp").exists());
        save(&s, &NotifyConfig::default()).unwrap();
        assert!(!config_path(&s).exists());
        save(&s, &NotifyConfig::default()).unwrap();
    }

    #[test]
    fn windows_acl_plan_restricts_to_the_user() {
        let s = server(Path::new("C:\\Servers\\alpha"));
        let c = windows_acl_plan(&s, "Sticky").unwrap();
        assert_eq!(c.program, "icacls");
        assert!(c.args.iter().any(|a| a == "/inheritance:r"));
        assert!(c.args.iter().any(|a| a == "Sticky:(R,W,D)"));
        assert!(c.args[0].ends_with("notify.json"));
        assert!(windows_acl_plan(&s, "bad;user").is_err());
    }

    #[test]
    fn payload_has_counts_ids_and_no_paths() {
        let p = build_payload(&report());
        let text = p.to_string();
        assert!(text.contains("1 updated · 0 skipped · 0 rolled back · 1 need attention"));
        assert!(text.contains("qb-core") && text.contains("ox_lib"));
        assert!(!text.contains("/home/secret"));
        assert_eq!(p["embeds"][0]["color"], 0xf39c12);
        assert_eq!(p["allowed_mentions"]["parse"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn payload_limits_long_lists() {
        let mut r = UpdateReport { server: "A".into(), ..Default::default() };
        for i in 0..30 {
            r.skipped.push((format!("res{i}"), SkipReason::Dirty));
        }
        let t = build_payload(&r).to_string();
        assert!(t.contains("res9") && !t.contains("res10") && t.contains("and 20 more"));
    }

    // ---- mock HTTP server (std::net only; never talks to Discord)

    type Handler = Box<dyn Fn(usize, &str) -> String + Send>;

    /// Serves one canned raw response per request; returns (base url, bodies seen, request count).
    fn mock(handler: Handler) -> (String, Arc<Mutex<Vec<String>>>, Arc<AtomicUsize>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/api/webhooks/1/tok", l.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let count = Arc::new(AtomicUsize::new(0));
        let (s2, c2) = (seen.clone(), count.clone());
        std::thread::spawn(move || {
            for conn in l.incoming() {
                let Ok(mut c) = conn else { break };
                let _ = c.set_read_timeout(Some(Duration::from_secs(3)));
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let n = c.read(&mut chunk).unwrap_or(0);
                    buf.extend_from_slice(&chunk[..n]);
                    let t = String::from_utf8_lossy(&buf).to_string();
                    if let Some((head, body)) = t.split_once("\r\n\r\n") {
                        let len = head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok())).unwrap_or(0);
                        if body.len() >= len {
                            break;
                        }
                    }
                    if n == 0 {
                        break;
                    }
                }
                let body = String::from_utf8_lossy(&buf).split_once("\r\n\r\n").map(|x| x.1.to_string()).unwrap_or_default();
                let i = c2.fetch_add(1, Ordering::SeqCst);
                s2.lock().unwrap().push(body.clone());
                let resp = handler(i, &body);
                if resp == "HANG" {
                    std::thread::sleep(Duration::from_secs(2));
                    continue;
                }
                let _ = c.write_all(resp.as_bytes());
            }
        });
        (url, seen, count)
    }

    fn http(status: &str, extra: &str, body: &str) -> String {
        format!("HTTP/1.1 {status}\r\n{extra}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
    }

    fn fast() -> Net {
        Net { timeout: Duration::from_millis(400), sleep: |_| {} }
    }

    #[test]
    fn send_success_posts_the_report_json() {
        let (url, seen, _) = mock(Box::new(|_, _| http("204 No Content", "", "")));
        post(&url, &build_payload(&report()), &fast()).unwrap();
        let body = seen.lock().unwrap()[0].clone();
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["embeds"][0]["title"], "Kora update: Alpha");
    }

    #[test]
    fn rate_limit_is_retried_then_succeeds() {
        let (url, _, count) = mock(Box::new(|i, _| if i < 2 { http("429 Too Many Requests", "", "{\"retry_after\": 0.01}") } else { http("204 No Content", "", "") }));
        post(&url, &serde_json::json!({"a": 1}), &fast()).unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn rate_limit_gives_up_after_two_retries() {
        let (url, _, count) = mock(Box::new(|_, _| http("429 Too Many Requests", "Retry-After: 0\r\n", "{}")));
        assert_eq!(post(&url, &serde_json::json!({}), &fast()), Err(NotifyError::RateLimited));
        assert_eq!(count.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn huge_retry_after_is_not_waited_for() {
        let (url, _, count) = mock(Box::new(|_, _| http("429 Too Many Requests", "", "{\"retry_after\": 3000}")));
        assert_eq!(post(&url, &serde_json::json!({}), &fast()), Err(NotifyError::RateLimited));
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn server_errors_are_reported_without_retry() {
        let (url, _, count) = mock(Box::new(|_, _| http("503 Service Unavailable", "", "oops")));
        assert_eq!(post(&url, &serde_json::json!({}), &fast()), Err(NotifyError::Http(503)));
        let (url, _, _) = mock(Box::new(|_, _| http("404 Not Found", "", "{}")));
        assert_eq!(post(&url, &serde_json::json!({}), &fast()), Err(NotifyError::Http(404)));
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn hung_server_times_out() {
        let (url, _, _) = mock(Box::new(|_, _| "HANG".into()));
        let t = std::time::Instant::now();
        assert_eq!(post(&url, &serde_json::json!({}), &fast()), Err(NotifyError::Timeout));
        assert!(t.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn oversized_bodies_are_refused_before_sending() {
        let (url, _, count) = mock(Box::new(|_, _| http("204 No Content", "", "")));
        let big = serde_json::json!({"x": "a".repeat(MAX_BODY + 1)});
        assert_eq!(post(&url, &big, &fast()), Err(NotifyError::TooLarge));
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn huge_responses_are_bounded() {
        let (url, _, _) = mock(Box::new(|_, _| http("200 OK", "", &"x".repeat(5_000_000))));
        let _ = post(&url, &serde_json::json!({}), &fast());
    }

    #[test]
    fn redirects_are_never_followed() {
        let (target, _, target_hits) = mock(Box::new(|_, _| http("204 No Content", "", "")));
        let (url, _, _) = mock(Box::new(move |_, _| http("307 Temporary Redirect", &format!("Location: {target}\r\n"), "")));
        assert_eq!(post(&url, &serde_json::json!({}), &fast()), Err(NotifyError::Redirected));
        assert_eq!(target_hits.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn public_senders_validate_first() {
        let r = report();
        assert_eq!(send_update_report(&NotifyConfig::default(), &r), Err(NotifyError::NotConfigured));
        let bad = NotifyConfig { discord_webhook: Some("http://127.0.0.1:1/api/webhooks/1/t".into()) };
        assert!(matches!(send_update_report(&bad, &r), Err(NotifyError::InvalidUrl(_))));
        assert!(matches!(send_test(&bad, "x"), Err(NotifyError::InvalidUrl(_))));
    }

    #[test]
    fn failure_to_send_never_fails_the_update_and_is_logged_without_the_url() {
        let d = tempfile::tempdir().unwrap();
        let s = server(d.path());
        save(&s, &NotifyConfig { discord_webhook: Some(GOOD.into()) }).unwrap();
        after_update_with(&s, &report(), &|_, _| Err(NotifyError::Http(500)));
        after_update_with(&s, &report(), &|_, _| panic!("boom"));
        let log = std::fs::read_to_string(d.path().join("update.log")).unwrap();
        assert!(log.contains("warning") && log.contains("HTTP 500"));
        assert_eq!(log.matches("warning").count(), 2);
        assert!(!log.contains("abc_DEF"));
    }

    #[test]
    fn hook_is_silent_without_webhook_or_news() {
        let d = tempfile::tempdir().unwrap();
        let s = server(d.path());
        let calls = AtomicUsize::new(0);
        let counting = |_: &NotifyConfig, _: &UpdateReport| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        };
        after_update_with(&s, &report(), &counting);
        save(&s, &NotifyConfig { discord_webhook: Some(GOOD.into()) }).unwrap();
        after_update_with(&s, &UpdateReport { server: "A".into(), ..Default::default() }, &counting);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        after_update_with(&s, &report(), &counting);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(!d.path().join("update.log").exists());
    }

    #[test]
    fn url_input_prefers_stdin_then_arg_then_env() {
        let mut empty: &[u8] = b"";
        assert_eq!(resolve_url_input(Some(GOOD), false, Some("ignored"), &mut empty).unwrap(), GOOD);
        assert_eq!(resolve_url_input(None, false, Some(GOOD), &mut empty).unwrap(), GOOD);
        let mut input: &[u8] = format!("{GOOD}\r\n").leak().as_bytes();
        assert_eq!(resolve_url_input(Some("ignored"), true, None, &mut input).unwrap(), GOOD);
        assert_eq!(resolve_url_input(None, false, None, &mut empty), Err(NotifyError::NotConfigured));
        assert!(resolve_url_input(Some("https://evil.net/x"), false, None, &mut empty).is_err());
    }
}
