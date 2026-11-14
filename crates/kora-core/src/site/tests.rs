use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

/// One-connection-at-a-time mock HTTP server on 127.0.0.1. Records request bodies.
struct Mock {
    base: String,
    seen: Arc<Mutex<Vec<String>>>,
}

fn mock(status: u16, body: String, delay_ms: u64) -> Mock {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s2 = seen.clone();
    std::thread::spawn(move || {
        for conn in l.incoming() {
            let Ok(mut c) = conn else { break };
            let mut buf = Vec::new();
            let mut tmp = [0u8; 2048];
            loop {
                let n = c.read(&mut tmp).unwrap_or(0);
                buf.extend_from_slice(&tmp[..n]);
                let text = String::from_utf8_lossy(&buf).to_string();
                if let Some(i) = text.find("\r\n\r\n") {
                    let len = text[..i].lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                    if buf.len() >= i + 4 + len || n == 0 {
                        s2.lock().unwrap().push(text);
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(delay_ms));
            let resp = format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            let _ = c.write_all(resp.as_bytes());
        }
    });
    Mock { base, seen }
}

const OK_BODY: &str = r#"{"config":{"mode":"development","name":"Alpha One","resources":["kora_banking","kora_jobs"],
 "artifact":{"kind":"build","build":"38621"},"gameBuild":3258,"ports":{"game":31120,"txadmin":41120},"hostname":"Alpha RP"},
 "cfx":{"identifier":"fivem:123456","username":"Sticky"},"github":{"login":"octo-cat"},"issuedAt":"2026-11-14T10:00:00Z"}"#;

fn fast() -> Options {
    Options { connect_timeout: std::time::Duration::from_millis(500), timeout: std::time::Duration::from_millis(400), max_body: 4096 }
}

#[test]
fn normalises_codes() {
    assert_eq!(normalize_code("kora-7f3k-92qx").unwrap(), "KORA-7F3K-92QX");
    assert_eq!(normalize_code(" 7F3K92QX ").unwrap(), "KORA-7F3K-92QX");
    assert_eq!(normalize_code("KORA7F3K92QX").unwrap(), "KORA-7F3K-92QX");
    for bad in ["", "KORA-7F3K", "KORA-7F3K-92QXX", "KORA-0F3K-92QX", "KORA-7F3K-92QO", "KORA-7F3K-92Q1", "KORA-7F3K-92QL", "KORA-7F3K-92Q!"] {
        assert!(matches!(normalize_code(bad), Err(SiteError::BadCode(_))), "{bad}");
    }
}

#[test]
fn base_url_rules() {
    assert!(check_base_url("https://koraframe.work/").is_ok());
    assert!(check_base_url("http://127.0.0.1:8080").is_ok());
    assert!(check_base_url("http://localhost").is_ok());
    for bad in ["http://koraframe.work", "http://localhost.evil.com", "http://127.0.0.1@evil.com", "ftp://x", "koraframe.work", "https://", "http://127.0.0.1.evil.com:80"] {
        assert!(matches!(check_base_url(bad), Err(SiteError::BadUrl(_))), "{bad}");
    }
}

#[test]
fn redeems_and_parses() {
    let m = mock(200, OK_BODY.into(), 0);
    let r = redeem_with("kora-7f3k-92qx", &m.base, &fast()).unwrap();
    assert_eq!(r.mode, Mode::Development);
    assert_eq!(r.name, "Alpha One");
    assert_eq!(r.resources, ResourceSelection::Only(vec!["kora_banking".into(), "kora_jobs".into()]));
    assert_eq!(r.artifact, ArtifactChoice::Build(38621));
    assert_eq!(r.game_build, Some(3258));
    assert_eq!((r.ports.game, r.ports.txadmin), (31120, 41120));
    assert_eq!(r.hostname.as_deref(), Some("Alpha RP"));
    assert_eq!(r.cfx_identifier.as_deref(), Some("fivem:123456"));
    assert_eq!(r.welcome(), "Welcome, Sticky! Your choices from the website were loaded.");
    assert_eq!(r.github_login.as_deref(), Some("octo-cat"));
    let req = m.seen.lock().unwrap()[0].clone();
    assert!(req.starts_with("POST /api/installer/redeem "), "{req}");
    assert!(req.contains(r#"{"code":"KORA-7F3K-92QX"}"#));
}

#[test]
fn minimal_response_recommended_all() {
    let body = r#"{"config":{"mode":"production","name":"x","resources":"all","artifact":{"kind":"recommended"},"gameBuild":null,
        "ports":{"game":30120,"txadmin":40120},"hostname":null},"cfx":{"identifier":null,"username":null},"github":{"login":null},"issuedAt":"t"}"#;
    let m = mock(200, body.into(), 0);
    let r = redeem_with("KORA-7F3K-92QX", &m.base, &fast()).unwrap();
    assert_eq!(r.resources, ResourceSelection::All);
    assert_eq!(r.artifact, ArtifactChoice::Recommended);
    assert!(r.cfx_username.is_none() && r.hostname.is_none() && r.game_build.is_none());
    assert!(r.welcome().starts_with("Welcome!"));
}

#[test]
fn not_found_is_plain_invalid() {
    let m = mock(404, r#"{"error":"invalid_or_expired"}"#.into(), 0);
    let e = redeem_with("KORA-7F3K-92QX", &m.base, &fast()).unwrap_err();
    assert_eq!(e, SiteError::InvalidOrExpired);
    let t = e.to_string();
    assert!(t.contains("expired") && t.contains("already used") && !t.contains("404"));
}

#[test]
fn rate_limit_and_server_errors() {
    let m = mock(429, "{}".into(), 0);
    assert_eq!(redeem_with("KORA-7F3K-92QX", &m.base, &fast()).unwrap_err(), SiteError::RateLimited);
    let m = mock(503, "oops".into(), 0);
    assert_eq!(redeem_with("KORA-7F3K-92QX", &m.base, &fast()).unwrap_err(), SiteError::Server(503));
}

#[test]
fn timeout_suggests_interactive_path() {
    let m = mock(200, OK_BODY.into(), 3000);
    let e = redeem_with("KORA-7F3K-92QX", &m.base, &fast()).unwrap_err();
    assert!(matches!(e, SiteError::Network(_)), "{e:?}");
    assert!(e.to_string().contains("without a code"));
}

#[test]
fn connection_refused_is_network_error() {
    let port = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let e = redeem_with("KORA-7F3K-92QX", &format!("http://127.0.0.1:{port}"), &fast()).unwrap_err();
    assert!(matches!(e, SiteError::Network(_)));
}

#[test]
fn oversized_body_rejected() {
    let m = mock(200, format!("{{\"pad\":\"{}\"}}", "a".repeat(10_000)), 0);
    let e = redeem_with("KORA-7F3K-92QX", &m.base, &fast()).unwrap_err();
    assert!(matches!(e, SiteError::BadResponse(ref s) if s.contains("too large")), "{e:?}");
}

#[test]
fn https_required_and_no_request_made() {
    let m = mock(200, OK_BODY.into(), 0);
    let http_remote = "http://example.com";
    assert!(matches!(redeem_with("KORA-7F3K-92QX", http_remote, &fast()), Err(SiteError::BadUrl(_))));
    // nothing was sent to the local mock either (we never even connected)
    assert!(m.seen.lock().unwrap().is_empty());
}

#[test]
fn bad_code_never_hits_network() {
    let m = mock(200, OK_BODY.into(), 0);
    assert!(matches!(redeem_with("nope", &m.base, &fast()), Err(SiteError::BadCode(_))));
    assert!(m.seen.lock().unwrap().is_empty());
}

#[test]
fn redirects_are_not_followed() {
    let m = mock(302, "".into(), 0);
    assert!(matches!(redeem_with("KORA-7F3K-92QX", &m.base, &fast()), Err(SiteError::Server(302))));
}

#[test]
fn rejects_malformed_or_hostile_responses() {
    let cases = [
        "not json",
        r#"{"config":{}}"#,
        r#"{"config":{"mode":"weird","name":"x","resources":"all","artifact":{"kind":"recommended"},"ports":{"game":30120,"txadmin":40120}}}"#,
        r#"{"config":{"mode":"production","name":"x","resources":["../evil"],"artifact":{"kind":"recommended"},"ports":{"game":30120,"txadmin":40120}}}"#,
        r#"{"config":{"mode":"production","name":"x","resources":"all","artifact":{"kind":"build","build":"12"},"ports":{"game":30120,"txadmin":40120}}}"#,
        r#"{"config":{"mode":"production","name":"x","resources":"all","artifact":{"kind":"recommended"},"ports":{"game":80,"txadmin":40120}}}"#,
        r#"{"config":{"mode":"production","name":"x","resources":"all","artifact":{"kind":"recommended"},"ports":{"game":30120,"txadmin":30120}}}"#,
    ];
    for c in cases {
        assert!(matches!(parse_response(c.as_bytes()), Err(SiteError::BadResponse(_))), "{c}");
    }
}

#[test]
fn sanitises_text() {
    let body = "{\"config\":{\"mode\":\"production\",\"name\":\"My <b>Server</b> with a very long name that exceeds\",\"resources\":\"all\",\"artifact\":{\"kind\":\"recommended\"},\"ports\":{\"game\":30120,\"txadmin\":40120}},\"cfx\":{\"username\":\"a\\u001b[31mred\"}}";
    let r = parse_response(body.as_bytes()).unwrap();
    assert!(r.name.chars().count() <= 32 && !r.name.contains(['<', '>', '/']));
    assert!(!r.cfx_username.unwrap().contains('\u{1b}'));
}

struct FakeArtifacts;

const HASH: &str = "5412ed0c3a7e0b5b2b1e8d6d1d3c4a5b6c7d8e9f";

impl ArtifactSource for FakeArtifacts {
    fn fetch_text(&self, url: &str) -> artifacts::Result<String> {
        if url == artifacts::JG_URL {
            Ok(format!(
                r#"{{"recommendedArtifact":"38621","windowsDownloadLink":"https://runtime.fivem.net/artifacts/fivem/build_server_windows/master/38621-{HASH}/server.zip","linuxDownloadLink":"https://runtime.fivem.net/artifacts/fivem/build_proot_linux/master/38621-{HASH}/fx.tar.xz"}}"#
            ))
        } else {
            Err(artifacts::ArtifactError::Network("offline".into()))
        }
    }
    fn download(&self, _: &str, _: &mut dyn Write) -> artifacts::Result<()> {
        unreachable!()
    }
}

fn sample(artifact: ArtifactChoice) -> Redeemed {
    Redeemed {
        mode: Mode::Production,
        name: "S".into(),
        resources: ResourceSelection::All,
        artifact,
        artifact_url: None,
        game_build: None,
        ports: Ports::default(),
        hostname: None,
        cfx_identifier: None,
        cfx_username: None,
        github_login: None,
    }
}

#[test]
fn resolves_build_to_official_url() {
    let mut r = sample(ArtifactChoice::Build(38621));
    r.resolve_artifact(&FakeArtifacts, Platform::Linux).unwrap();
    let url = r.artifact_url.clone().unwrap();
    assert!(url.starts_with("https://runtime.fivem.net/artifacts/fivem/build_proot_linux/master/38621-"));
    artifacts::validate_url(&url, Platform::Linux).unwrap();
    let mut r = sample(ArtifactChoice::Build(99999));
    assert_eq!(r.resolve_artifact(&FakeArtifacts, Platform::Linux).unwrap_err(), SiteError::UnknownBuild(99999));
    let mut r = sample(ArtifactChoice::Recommended);
    r.resolve_artifact(&FakeArtifacts, Platform::Linux).unwrap();
    assert!(r.artifact_url.is_none());
}

#[test]
fn apply_to_request_leaves_secrets_alone() {
    let mut req = InstallRequest {
        mode: Mode::Production,
        name: "old".into(),
        dir: "/x".into(),
        hostname: "old".into(),
        license_key: Some("k".into()),
        cfx_identifier: None,
        ports: Ports::default(),
        txadmin_public: false,
        artifact_url: None,
        game_build: None,
        resources: ResourceSelection::All,
        db: crate::installer::DbChoice::Skip,
        service: false,
        firewall: false,
        github_owner: Some("me".into()),
        catalog: None,
        dry_run: false,
        db_admin: None,
    };
    let mut r = sample(ArtifactChoice::Recommended);
    r.mode = Mode::Development;
    r.name = "Alpha".into();
    r.ports = Ports { game: 31000, txadmin: 41000 };
    r.cfx_identifier = Some("fivem:1".into());
    r.github_login = Some("someone".into());
    r.apply_to(&mut req);
    assert_eq!((req.mode, req.name.as_str(), req.hostname.as_str(), req.ports.game), (Mode::Development, "Alpha", "Alpha", 31000));
    assert_eq!(req.cfx_identifier.as_deref(), Some("fivem:1"));
    assert_eq!(req.license_key.as_deref(), Some("k"));
    assert_eq!(req.github_owner.as_deref(), Some("me"), "fork owner is never taken from the website");
    assert_eq!(req.dir, std::path::PathBuf::from("/x"));
}
