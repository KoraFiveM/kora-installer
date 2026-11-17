//! The ONE place that removes secrets from anything that goes into a support bundle or a bug report.
//!
//! # Threat model
//! A support bundle is handed to another person (a helper, a maintainer on GitHub). It must never carry a
//! secret of the owner: a licence key, a database login, a txAdmin password or token, a Discord or generic
//! webhook address, a Git or GitHub token, the panel's own token, a saved password of any kind. The risk is
//! not malice but forgetting: a new file, a log line that echoes a connection string, a URL with a token.
//!
//! # Approach: deny by default, then scrub the rest
//! 1. [`policy`]: files that are *known to carry secrets* (`secrets.cfg`, `database.json`, txAdmin's data,
//!    Kora's webhook settings, key files, `.env`, the store) are NEVER read into a bundle: the bundle lists
//!    that they exist and says why they are left out. A file with an unknown name is scrubbed.
//! 2. [`scrub`]: every other text goes through pattern rules: secret-shaped values (licence keys, webhook
//!    addresses, addresses with a login, tokens of known kinds, private keys, password hashes), values of
//!    settings whose NAME says they are secret (in `.cfg`, `.env`, ini, YAML, JSON and prose forms), secrets
//!    in a web address's query, and in HTTP headers. People's identifiers in permission files and the
//!    user name in a home folder path are hidden too.
//! 3. [`leaks`]: a final tripwire runs the strongest patterns over the result. A text that still trips it
//!    (a bug in the rules above) is dropped altogether, never shipped.
//!
//! Redaction removes values and keeps the shape (`sv_licenseKey "<hidden>"`), so a helper can still see that
//! a setting exists. What is NOT removed, and the README of every bundle says so: server names, ports,
//! folder names, public IP addresses, the names of resources, log text that is not secret-shaped.
//! Everything here is pure text in, text out, so it is tested with fixtures of real-looking files.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use regex::{Captures, Regex};

/// What to do with a file by its path (relative to a server folder or the Kora home, `/` separators).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Policy {
    /// Never read: it holds secrets. The text is the reason shown to the person.
    Omit(&'static str),
    /// Read and scrub.
    Scrub,
}

/// Decide by name. Deny-by-default for everything known to carry a secret.
pub fn policy(rel: &str) -> Policy {
    let p = format!("/{}", rel.replace('\\', "/").trim_start_matches('/').to_ascii_lowercase());
    let name = p.rsplit('/').next().unwrap_or(&p);
    let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    if name == "secrets.cfg" || name.ends_with("secrets.cfg") {
        return Policy::Omit("holds the licence key and the database connection string");
    }
    if name == "database.json" {
        return Policy::Omit("holds the database login");
    }
    if p.split('/').any(|seg| seg == "txdata") {
        return Policy::Omit("txAdmin's data holds its admin accounts, password hashes and tokens");
    }
    if matches!(name, "notify.json" | "alerts.json") || name.starts_with("notify.") && ext == "json" {
        return Policy::Omit("holds webhook addresses");
    }
    if matches!(
        name,
        "owners.json"
            | "remote.json"
            | "users.json"
            | "secrets.json"
            | "keys.json"
            | "panel-token"
            | "panel.token"
            | "token"
            | ".env"
            | "credentials"
            | "id_rsa"
            | "id_ed25519"
            | "known_hosts"
            | ".netrc"
            | ".git-credentials"
            | "hosts.yml"
    ) || name.starts_with(".env.")
    {
        return Policy::Omit("holds saved keys, tokens or sign-in details");
    }
    if matches!(ext, "pem" | "key" | "p12" | "pfx" | "crt" | "kdbx" | "db" | "sqlite" | "sqlite3" | "db-wal" | "db-shm") || p.ends_with("/store/kora.db") {
        return Policy::Omit("is a key, certificate or database file");
    }
    if p.contains("/.kora/asset-uploads/") || p.contains("/.kora/asset-backups/") || p.contains("/.kora/restore-") {
        return Policy::Omit("is a copy of other people's files");
    }
    Policy::Scrub
}

/// Words that make a setting's NAME a secret. Matched anywhere in the name, ignoring case.
const SECRET_WORDS: &str = "key|token|secret|passw|pwd|passphrase|credential|auth|webhook|licen[cs]e|connection_?string|dsn|cookie|session|private|salt|bearer|signature|rcon";

/// A name that ends like this names something that is not the secret itself ("license_key_name", "token_set").
fn harmless_name(name: &str) -> bool {
    let n = name.trim_matches(['"', '\'', ' ']).to_ascii_lowercase();
    ["_name", "name", "_set", "_present", "_len", "_count", "_path", "_file", "_url_set", "_enabled", "_required"].iter().any(|s| n.ends_with(s)) && !n.ends_with("username") || n.ends_with("_name")
}

fn harmless_value(v: &str) -> bool {
    let t = v.trim().trim_matches(['"', '\'', ',', ';']);
    t.is_empty()
        || matches!(t.to_ascii_lowercase().as_str(), "true" | "false" | "null" | "none" | "yes" | "no" | "on" | "off" | "<hidden>" | "<hidden>\"" | "[]" | "{}")
        || t.parse::<f64>().is_ok()
        || t.starts_with("<hidden")
}

/// What was hidden, by kind (counts only: never the values).
pub type Hits = BTreeMap<&'static str, usize>;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Scrubbed {
    pub text: String,
    pub hits: Hits,
}

impl Scrubbed {
    pub fn total(&self) -> usize {
        self.hits.values().sum()
    }
}

struct Rule {
    kind: &'static str,
    cell: OnceLock<Regex>,
    pattern: String,
    /// Replacement; `$1` etc. keep the part before the secret.
    with: &'static str,
}

fn rules() -> &'static Vec<Rule> {
    static R: OnceLock<Vec<Rule>> = OnceLock::new();
    R.get_or_init(|| {
        let rule = |kind, pattern: &str, with| Rule { kind, cell: OnceLock::new(), pattern: pattern.to_string(), with };
        vec![
            rule("private key", r"(?s)-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----.*?(?:-----END [A-Z0-9 ]*PRIVATE KEY-----|\z)", "<hidden: private key>"),
            rule("licence key", r"(?i)\bcfxk_[A-Za-z0-9_\-]{6,}", "<hidden: licence key>"),
            rule("webhook address", r#"(?i)https?://(?:[a-z0-9\-]+\.)*(?:discord(?:app)?\.com/api(?:/v\d+)?/webhooks|hooks\.slack\.com/(?:services|workflows)|outlook\.office\.com/webhook|webhook\.site|hooks\.zapier\.com/hooks)/[^\s"'<>)\]]+"#, "<hidden: webhook address>"),
            rule("address with a login", r#"[A-Za-z][A-Za-z0-9+.\-]*://[^/\s:@"'<>]+:[^/\s@"'<>]*@[^\s"'<>)\]]*"#, "<hidden: address with a login>"),
            rule("address with a login", r#"(?i)(?:mysql|mariadb|postgres(?:ql)?|mongodb(?:\+srv)?|redis|amqps?)://[^\s"'<>)\]]*@[^\s"'<>)\]]*"#, "<hidden: address with a login>"),
            rule("token", r"\bgh[pousr]_[A-Za-z0-9]{16,}\b", "<hidden: token>"),
            rule("token", r"\bgithub_pat_[A-Za-z0-9_]{20,}\b", "<hidden: token>"),
            rule("token", r"\bxox[baprs]-[A-Za-z0-9\-]{10,}", "<hidden: token>"),
            rule("token", r"\bglpat-[A-Za-z0-9_\-]{16,}", "<hidden: token>"),
            rule("token", r"\bAKIA[0-9A-Z]{16}\b", "<hidden: token>"),
            rule("token", r"\beyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]*", "<hidden: token>"),
            rule("token", r"(?i)\bBearer\s+[A-Za-z0-9._~+/=\-]{8,}", "Bearer <hidden: token>"),
            rule("password hash", r"\$2[aby]\$\d{2}\$[./A-Za-z0-9]{40,}", "<hidden: password hash>"),
            rule("password hash", r#"\$argon2(?:id|i|d)\$[^\s"'<>]+"#, "<hidden: password hash>"),
            rule("secret in an address", r#"(?i)([?&;#](?:token|code|ks|key|apikey|api_key|secret|password|pass|auth|access_token|sig|signature|session|ticket)=)[^&\s"'<>#]+"#, "${1}<hidden>"),
            rule("header", r"(?im)^(\s*(?:proxy-)?(?:authorization|x-kora-session|x-kora-token|x-api-key|cookie|set-cookie)\s*:\s*).*$", "${1}<hidden>"),
            rule("person's identifier", r"(?i)\b(identifier\.(?:fivem|discord|steam|license2?|xbl|live|ip))\s*[:=]\s*[A-Za-z0-9.:]+", "${1}:<hidden>"),
            rule("command line secret", r"(?i)(--[a-z\-]*(?:key|token|password|secret|passphrase|webhook)[a-z\-]*(?:=|\s+))(<hidden[^>]*>|\S+)", "${1}<hidden>"),
        ]
    })
}

fn name_rules() -> &'static [Regex; 4] {
    static CELL: OnceLock<[Regex; 4]> = OnceLock::new();
    CELL.get_or_init(|| {
        let w = SECRET_WORDS;
        [
            // cfg: `sv_licenseKey "x"`, `set mysql_connection_string "x"`, `rcon_password x`
            Regex::new(&format!(r#"(?im)^(\s*(?:set[rsa]?\s+)?)([A-Za-z0-9_.\-]*(?:{w})[A-Za-z0-9_.\-]*)(\s+)(\S.*)$"#)).expect("cfg rule"),
            // env / ini / toml / yaml: `NAME=value`, `name: value`
            Regex::new(&format!(r#"(?im)^(\s*[-\s]*)([A-Za-z0-9_.\-]*(?:{w})[A-Za-z0-9_.\-]*)(\s*[=:]\s*)(\S.*)$"#)).expect("assign rule"),
            // JSON: `"name": "value"`
            Regex::new(&format!(r#"(?i)("[^"\n]*(?:{w})[^"\n]*")(\s*:\s*)("(?:[^"\\\n]|\\.)*"|[^,}}\]\s][^,}}\]\n]*)"#)).expect("json rule"),
            // prose and logs: `password: hunter2`, `token = abc`, `licence key is cfxk_..`
            Regex::new(r#"(?i)\b(password|passwd|pwd|passphrase|token|secret|api[_-]?key|licen[cs]e[_ -]?key|auth[_-]?key)(\s*[=:]\s*|\s+is\s+)([^\s"',;]+)"#).expect("prose rule"),
        ]
    })
}

fn paths_rules() -> &'static [Regex; 3] {
    static CELL: OnceLock<[Regex; 3]> = OnceLock::new();
    CELL.get_or_init(|| {
        [Regex::new(r"(?i)([A-Z]:[\\/]+Users[\\/]+)[^\\/\s:*?<>|]+").expect("win home"), Regex::new(r"(/home/)[^/\s:]+").expect("linux home"), Regex::new(r"(/Users/)[^/\s:]+").expect("mac home")]
    })
}

fn bump(h: &mut Hits, kind: &'static str, n: usize) {
    if n > 0 {
        *h.entry(kind).or_insert(0) += n;
    }
}

/// Hide secrets in `text`. Pure.
pub fn scrub(text: &str) -> Scrubbed {
    let mut hits = Hits::new();
    let mut cur = text.to_string();
    // Each rule compiles once, on first use.
    for r in rules() {
        let rx = r.cell.get_or_init(|| Regex::new(&r.pattern).expect("redaction pattern"));
        let n = rx.find_iter(&cur).count();
        if n > 0 {
            cur = rx.replace_all(&cur, r.with).into_owned();
            bump(&mut hits, r.kind, n);
        }
    }
    let [cfg, assign, json, prose] = name_rules();
    // cfg and assignment forms: hide the value unless the name or value is harmless
    for (rx, kind) in [(cfg, "setting"), (assign, "setting")] {
        let mut n = 0;
        cur = rx
            .replace_all(&cur, |c: &Captures| {
                if harmless_name(&c[2]) || harmless_value(&c[4]) {
                    return c[0].to_string();
                }
                n += 1;
                let quote = if c[4].trim_start().starts_with('"') { "\"<hidden>\"" } else { "<hidden>" };
                format!("{}{}{}{}", &c[1], &c[2], &c[3], quote)
            })
            .into_owned();
        bump(&mut hits, kind, n);
    }
    let mut n = 0;
    cur = json
        .replace_all(&cur, |c: &Captures| {
            if harmless_name(&c[1]) || harmless_value(&c[3]) {
                return c[0].to_string();
            }
            n += 1;
            format!("{}{}\"<hidden>\"", &c[1], &c[2])
        })
        .into_owned();
    bump(&mut hits, "setting", n);
    let mut n = 0;
    cur = prose
        .replace_all(&cur, |c: &Captures| {
            if harmless_value(&c[3]) {
                return c[0].to_string();
            }
            n += 1;
            format!("{}{}<hidden>", &c[1], &c[2])
        })
        .into_owned();
    bump(&mut hits, "secret in text", n);
    // user names inside home folders
    let mut n = 0;
    for rx in paths_rules() {
        n += rx.find_iter(&cur).filter(|m| !m.as_str().ends_with("<user>")).count();
        cur = rx.replace_all(&cur, "${1}<user>").into_owned();
    }
    bump(&mut hits, "user name in a path", n);
    Scrubbed { text: cur, hits }
}

/// [`scrub`] after also masking the given folder prefixes (the person's home folder) with `~`.
pub fn scrub_with_homes(text: &str, homes: &[String]) -> Scrubbed {
    let mut t = text.to_string();
    let mut extra = 0;
    for h in homes.iter().filter(|h| h.len() > 3) {
        let n = t.matches(h.as_str()).count();
        if n > 0 {
            t = t.replace(h.as_str(), "~");
            extra += n;
        }
    }
    let mut s = scrub(&t);
    bump(&mut s.hits, "user name in a path", extra);
    s
}

/// The tripwire: does this (already scrubbed) text still hold something that must never leave? A text that
/// does is dropped by the caller.
pub fn leaks(text: &str) -> Option<&'static str> {
    static CELL: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();
    let list = CELL.get_or_init(|| {
        [
            ("a licence key", r"(?i)\bcfxk_[A-Za-z0-9_\-]{6,}"),
            ("a private key", r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----"),
            ("a webhook address", r#"(?i)(?:discord(?:app)?\.com/api(?:/v\d+)?/webhooks|hooks\.slack\.com/services)/[^\s"'<>)]{8,}"#),
            ("an address with a login", r#"[A-Za-z][A-Za-z0-9+.\-]*://[^/\s:@"'<>]+:[^/\s@"'<>]+@"#),
            ("a token", r"\bgh[pousr]_[A-Za-z0-9]{16,}\b|\bgithub_pat_[A-Za-z0-9_]{20,}|\beyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]*|\bAKIA[0-9A-Z]{16}\b"),
            ("a password hash", r"\$2[aby]\$\d{2}\$[./A-Za-z0-9]{40,}"),
        ]
        .into_iter()
        .map(|(k, p)| (k, Regex::new(p).expect("tripwire pattern")))
        .collect()
    });
    list.iter().find(|(_, rx)| rx.is_match(text)).map(|(k, _)| *k)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(t: &str) -> String {
        scrub(t).text
    }

    /// Strings that must never survive in any output below.
    const SECRETS: &[&str] = &[
        "cfxk_AbCdEf0123456789AbCdEf0123456789_xYz123",
        "hunter2-and-more",
        "s3cr3t-db-pass",
        "ghp_0123456789abcdefghijklmnopqrstuvwxyz",
        "github_pat_11ABCDEFG0abcdefghijkl_mnopqrstuvwxyz0123456789",
        "1234567890/AbCdEfGhIjKlMnOpQrStUvWxYz-_abcdefghijklmnopqrstuv",
        "AKIAIOSFODNN7EXAMPLE",
        "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U",
        "$2b$10$abcdefghijklmnopqrstuuABCDEFGHIJKLMNOPQRSTUVWXYZ0123",
        "0a1b2c3d4e5f60718293a4b5c6d7e8f9",
    ];

    fn assert_no_secret(text: &str) {
        for sec in SECRETS {
            assert!(!text.contains(sec), "secret {sec:?} survived in:\n{text}");
        }
        assert!(leaks(text).is_none(), "the tripwire fired ({:?}) on:\n{text}", leaks(text));
    }

    const SERVER_CFG: &str = r#"
# Kora server.cfg
endpoint_add_tcp "0.0.0.0:30120"
endpoint_add_udp "0.0.0.0:30120"
sv_hostname "My Kora Server"
sv_maxclients 48
exec secrets.cfg
sv_licenseKey "cfxk_AbCdEf0123456789AbCdEf0123456789_xYz123"
set mysql_connection_string "mysql://kora_ab12:s3cr3t-db-pass@127.0.0.1:3306/kora_ab12?charset=utf8mb4"
rcon_password hunter2-and-more
set steam_webApiKey "0a1b2c3d4e5f60718293a4b5c6d7e8f9"
set sv_tebexSecret "0a1b2c3d4e5f60718293a4b5c6d7e8f9"
setr game_language "en"
ensure oxmysql
ensure kora_core
add_ace group.admin command allow
add_principal identifier.fivem:123456 group.admin
add_principal identifier.discord:98765432109876 group.admin
"#;

    #[test]
    fn a_server_cfg_loses_its_secrets_and_keeps_its_shape() {
        let r = scrub(SERVER_CFG);
        assert_no_secret(&r.text);
        for keep in [
            "endpoint_add_tcp \"0.0.0.0:30120\"",
            "sv_hostname \"My Kora Server\"",
            "sv_maxclients 48",
            "ensure oxmysql",
            "ensure kora_core",
            "setr game_language \"en\"",
            "add_ace group.admin command allow",
            "exec secrets.cfg",
        ] {
            assert!(r.text.contains(keep), "{keep} lost:\n{}", r.text);
        }
        assert!(r.text.contains("sv_licenseKey \"<hidden"), "{}", r.text);
        assert!(r.text.contains("rcon_password <hidden>"));
        assert!(r.text.contains("identifier.fivem:<hidden>") && r.text.contains("identifier.discord:<hidden>"));
        assert!(r.total() >= 6);
        // the same text again changes nothing more (idempotent)
        assert_eq!(scrub(&r.text).text, r.text);
    }

    #[test]
    fn secret_files_are_never_read() {
        for p in [
            "secrets.cfg",
            "srv/secrets.cfg",
            "database.json",
            "C:\\Servers\\kora\\database.json",
            "txData/default/config.json",
            "txData/default/data/admins.json",
            "srv/txData/default/cfg.log",
            ".kora/notify.json",
            ".kora/alerts.json",
            "store/kora.db",
            ".env",
            ".env.production",
            "id_rsa",
            "server.pem",
            "x.key",
            "panel-token",
            ".git-credentials",
            ".kora/asset-uploads/abc/qs.zip",
        ] {
            assert!(matches!(policy(p), Policy::Omit(_)), "{p} must be omitted");
        }
        for p in ["server.cfg", "permissions.cfg", "kora-server.json", ".kora/install.log", ".kora/schedule.json", "panel.log", "resources/[kora]/core/fxmanifest.lua"] {
            assert_eq!(policy(p), Policy::Scrub, "{p}");
        }
    }

    #[test]
    fn json_with_secret_names_keeps_booleans_and_names() {
        let j = r#"{
  "license_key": "cfxk_AbCdEf0123456789AbCdEf0123456789_xYz123",
  "license_key_name": "Main",
  "token_set": true,
  "webhook_url": "https://discord.com/api/webhooks/123456789012345678/1234567890/AbCdEfGhIjKlMnOpQrStUvWxYz-_abcdefghijklmnopqrstuv",
  "password": "hunter2-and-more",
  "nested": {"db": {"password": "s3cr3t-db-pass", "host": "127.0.0.1", "port": 3306}},
  "github_token": "ghp_0123456789abcdefghijklmnopqrstuvwxyz",
  "name": "Prod",
  "ports": [30120, 40120]
}"#;
        let t = s(j);
        assert_no_secret(&t);
        let v: serde_json::Value = serde_json::from_str(&t).expect("still valid JSON");
        assert_eq!(v["license_key_name"], "Main");
        assert_eq!(v["token_set"], true);
        assert_eq!(v["nested"]["db"]["host"], "127.0.0.1");
        assert_eq!(v["nested"]["db"]["port"], 3306);
        assert_eq!(v["name"], "Prod");
        assert_eq!(v["password"], "<hidden>");
        assert!(v["license_key"].as_str().unwrap().starts_with("<hidden"));
    }

    #[test]
    fn env_ini_and_yaml_forms() {
        let t = s("DB_PASSWORD=s3cr3t-db-pass\nAPI_TOKEN: ghp_0123456789abcdefghijklmnopqrstuvwxyz\n  secret_key = hunter2-and-more\n- password: hunter2-and-more\nPORT=30120\nHOSTNAME=prod\nLICENSE_KEY_NAME=Main\n");
        assert_no_secret(&t);
        for keep in ["PORT=30120", "HOSTNAME=prod", "LICENSE_KEY_NAME=Main"] {
            assert!(t.contains(keep), "{keep} lost: {t}");
        }
    }

    #[test]
    fn addresses_with_logins_webhooks_and_tokens_in_logs() {
        let log = "\
git remote origin https://ghp_0123456789abcdefghijklmnopqrstuvwxyz@github.com/KoraFiveM/core.git\n\
clone https://oauth2:ghp_0123456789abcdefghijklmnopqrstuvwxyz@github.com/x/y.git failed\n\
connecting mysql://kora_ab12:s3cr3t-db-pass@127.0.0.1:3306/kora_ab12\n\
webhook https://discord.com/api/webhooks/123456789012345678/1234567890/AbCdEfGhIjKlMnOpQrStUvWxYz-_abcdefghijklmnopqrstuv returned 204\n\
Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U\n\
panel at http://127.0.0.1:47120/?code=0a1b2c3d4e5f60718293a4b5c6d7e8f9 opened\n\
open http://127.0.0.1:47120/#token=0a1b2c3d4e5f60718293a4b5c6d7e8f9\n\
X-Kora-Session: 0a1b2c3d4e5f60718293a4b5c6d7e8f9\n\
Cookie: kora_session=0a1b2c3d4e5f60718293a4b5c6d7e8f9; theme=dark\n\
aws key AKIAIOSFODNN7EXAMPLE seen\n\
txAdmin: admin password: hunter2-and-more\n\
kora install --license-key cfxk_AbCdEf0123456789AbCdEf0123456789_xYz123 --name Prod\n\
kora keys add Main --token=hunter2-and-more\n\
hash $2b$10$abcdefghijklmnopqrstuuABCDEFGHIJKLMNOPQRSTUVWXYZ0123 stored\n";
        let r = scrub(log);
        assert_no_secret(&r.text);
        // what is not secret stays, so the log is still readable
        for keep in ["git remote origin", "returned 204", "--name Prod", "Cookie: <hidden>", "aws key", "stored"] {
            assert!(r.text.contains(keep), "{keep} lost:\n{}", r.text);
        }
        assert!(r.text.contains("--license-key <hidden>"), "{}", r.text);
        assert!(r.hits.contains_key("webhook address") && r.hits.contains_key("licence key"));
    }

    #[test]
    fn private_keys_are_cut_out_even_when_unterminated() {
        let k = "before\n-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmU\nAAAAB3NzaC1lZDI1NTE5AAAAIA==\n-----END OPENSSH PRIVATE KEY-----\nafter\n-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBgkqhkiG9w0BAQEFAASC";
        let t = s(k);
        assert!(!t.contains("b3BlbnNz") && !t.contains("MIIEvQ") && t.contains("before") && t.contains("after"));
        assert!(leaks(&t).is_none());
    }

    #[test]
    fn home_folders_lose_the_user_name() {
        let t = s("C:\\Users\\Jamie Smith\\AppData\\Local\\Kora\\store\nC:/Users/jamie/x\n/home/jamie/.kora/server-backups\n/Users/jamie/Library\n");
        assert!(!t.contains("jamie") && !t.contains("Jamie"), "{t}");
        assert!(t.contains("AppData\\Local\\Kora") && t.contains("/.kora/server-backups"));
        let r = scrub_with_homes("kora home /srv/people/alex/.kora and /srv/people/alex", &["/srv/people/alex".to_string()]);
        assert_eq!(r.text, "kora home ~/.kora and ~");
    }

    #[test]
    fn things_that_look_odd_but_are_fine_stay() {
        let ok = "update 4f2a9c1e0b7d5a63c8e1f20493b7d6a15c8e0f29 applied\nkeyboard layout uk\nlicense_key_name Main\nkora_version 0.8.0\nsha256 ab12cd34ef56ab12cd34ef56ab12cd34ef56ab12cd34ef56ab12cd34ef56ab12\nport 30120 token_set true\n";
        let r = scrub(ok);
        // a commit hash, a checksum and plain words are not secrets
        assert!(r.text.contains("4f2a9c1e0b7d5a63c8e1f20493b7d6a15c8e0f29") && r.text.contains("ab12cd34ef56"), "{}", r.text);
        assert!(r.text.contains("license_key_name Main") && r.text.contains("token_set true"));
    }

    #[test]
    fn the_tripwire_drops_what_the_rules_missed() {
        // a text with a secret in a shape the rules do not know about is not what the tripwire is for; but a bug that leaves a known
        // shape in must trip it
        assert_eq!(leaks("here cfxk_AbCdEf0123456789"), Some("a licence key"));
        assert_eq!(leaks("https://discord.com/api/webhooks/1/abcdefghijkl"), Some("a webhook address"));
        assert_eq!(leaks("mysql://u:pw@host/db"), Some("an address with a login"));
        assert_eq!(leaks("nothing to see here"), None);
    }

    #[test]
    fn a_big_text_with_many_secrets_is_clean_and_fast() {
        let mut big = String::new();
        for i in 0..2000 {
            big.push_str(&format!("line {i}: set mysql_connection_string \"mysql://u{i}:pw{i}@h/db\"\nplain line {i} with nothing\n"));
        }
        let t0 = std::time::Instant::now();
        let r = scrub(&big);
        assert!(r.text.contains("plain line 1999 with nothing") && !r.text.contains("pw1999"));
        assert!(t0.elapsed() < std::time::Duration::from_secs(10), "{:?}", t0.elapsed());
    }

    #[test]
    fn counts_never_hold_values() {
        let r = scrub(SERVER_CFG);
        let shown = format!("{:?}", r.hits);
        assert_no_secret(&shown);
    }
}
