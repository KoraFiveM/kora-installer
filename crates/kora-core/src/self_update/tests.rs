use super::*;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use replace::{Flow, RealFs};

/// Route -> (status, extra headers, body).
type Routes = Arc<Mutex<HashMap<String, (u16, Vec<(String, String)>, Vec<u8>)>>>;

struct Mock {
    base: String,
    routes: Routes,
    hits: Arc<Mutex<Vec<String>>>,
}

impl Mock {
    fn start() -> Mock {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
        let routes: Routes = Default::default();
        let hits: Arc<Mutex<Vec<String>>> = Default::default();
        let (r, h) = (routes.clone(), hits.clone());
        std::thread::spawn(move || {
            for s in l.incoming() {
                let Ok(mut s) = s else { continue };
                let mut buf = [0u8; 4096];
                let n = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                h.lock().unwrap().push(path.clone());
                let (st, hd, body) = r.lock().unwrap().get(&path).cloned().unwrap_or((404, vec![], b"nope".to_vec()));
                let mut out = format!("HTTP/1.1 {st} X\r\nContent-Length: {}\r\nConnection: close\r\n", body.len());
                for (k, v) in hd {
                    out += &format!("{k}: {v}\r\n");
                }
                out += "\r\n";
                let _ = s.write_all(out.as_bytes());
                let _ = s.write_all(&body);
            }
        });
        Mock { base, routes, hits }
    }
    fn put(&self, path: &str, body: &[u8]) {
        self.routes.lock().unwrap().insert(path.into(), (200, vec![], body.to_vec()));
    }
    fn redirect(&self, path: &str, to: &str) {
        self.routes.lock().unwrap().insert(path.into(), (302, vec![("Location".into(), to.into())], vec![]));
    }
    fn src(&self) -> Source {
        Source::from_override(Some(&self.base))
    }
    /// Publish `tag` with a program `content` and a checksum line for `claimed`.
    fn publish(&self, content: &[u8], claimed: &[u8], name: &str) -> Release {
        self.put(&format!("/dl/{name}"), content);
        let sum = hex::encode(Sha256::digest(claimed));
        self.put(&format!("/dl/{name}.sha256"), format!("{sum}  {name}\n").as_bytes());
        release(&self.base, "v0.9.0", &[name.to_string(), format!("{name}.sha256")])
    }
}

fn release(base: &str, tag: &str, assets: &[String]) -> Release {
    Release {
        tag_name: tag.into(),
        draft: false,
        prerelease: false,
        assets: assets.iter().map(|n| release::Asset { name: n.clone(), browser_download_url: format!("{base}/dl/{n}"), size: 0 }).collect(),
    }
}

fn setup(old: &[u8]) -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let t = d.path().join("kora");
    std::fs::write(&t, old).unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&t, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    (d, t)
}

fn leftovers(d: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

#[test]
fn replaces_atomically_with_backup_on_both_flows() {
    for flow in [Flow::Unix, Flow::Windows] {
        let m = Mock::start();
        let rel = m.publish(b"NEW PROGRAM", b"NEW PROGRAM", "kora-linux-x86_64");
        let (d, t) = setup(b"OLD PROGRAM");
        let a = apply(&m.src(), &rel, "kora-linux-x86_64", &t, &RealFs, flow).unwrap();
        assert_eq!(std::fs::read(&t).unwrap(), b"NEW PROGRAM");
        assert_eq!(std::fs::read(a.backup.as_ref().unwrap()).unwrap(), b"OLD PROGRAM");
        assert_eq!(a.sha256, hex::encode(Sha256::digest(b"NEW PROGRAM")));
        assert_eq!(leftovers(d.path()), ["kora", "kora.old"], "no temp files left ({flow:?})");
        #[cfg(unix)]
        assert_eq!(std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&t).unwrap().permissions()) & 0o777, 0o755);
    }
}

#[test]
#[cfg(unix)]
fn setuid_and_setgid_bits_are_not_carried_over() {
    use std::os::unix::fs::PermissionsExt;
    let m = Mock::start();
    let rel = m.publish(b"NEW", b"NEW", "kora-linux-x86_64");
    let (_d, t) = setup(b"OLD");
    std::fs::set_permissions(&t, std::fs::Permissions::from_mode(0o6755)).unwrap();
    assert_eq!(std::fs::metadata(&t).unwrap().permissions().mode() & 0o7777, 0o6755, "test setup");
    apply(&m.src(), &rel, "kora-linux-x86_64", &t, &RealFs, Flow::Unix).unwrap();
    assert_eq!(std::fs::metadata(&t).unwrap().permissions().mode() & 0o7777, 0o755, "plain rwx bits kept, setuid/setgid dropped");
}

#[test]
fn a_second_update_replaces_the_old_backup() {
    let m = Mock::start();
    let rel = m.publish(b"V2", b"V2", "kora-linux-x86_64");
    let (d, t) = setup(b"V1");
    std::fs::write(d.path().join("kora.old"), b"V0").unwrap();
    apply(&m.src(), &rel, "kora-linux-x86_64", &t, &RealFs, Flow::Unix).unwrap();
    assert_eq!(std::fs::read(d.path().join("kora.old")).unwrap(), b"V1");
}

#[test]
fn checksum_mismatch_aborts_and_leaves_the_old_program() {
    for flow in [Flow::Unix, Flow::Windows] {
        let m = Mock::start();
        let rel = m.publish(b"TAMPERED", b"WHAT WAS SIGNED", "kora-linux-x86_64");
        let (d, t) = setup(b"OLD PROGRAM");
        let e = apply(&m.src(), &rel, "kora-linux-x86_64", &t, &RealFs, flow).unwrap_err();
        assert!(matches!(e, SelfUpdateError::ChecksumMismatch { .. }), "{e}");
        assert_eq!(std::fs::read(&t).unwrap(), b"OLD PROGRAM");
        assert_eq!(leftovers(d.path()), ["kora"], "temp file removed, no backup made");
    }
}

#[test]
fn empty_download_is_refused() {
    let m = Mock::start();
    let rel = m.publish(b"", b"", "kora-linux-x86_64");
    let (_d, t) = setup(b"OLD");
    assert!(apply(&m.src(), &rel, "kora-linux-x86_64", &t, &RealFs, Flow::Unix).is_err());
    assert_eq!(std::fs::read(&t).unwrap(), b"OLD");
}

#[test]
fn release_without_asset_or_checksum_is_refused_before_downloading() {
    let m = Mock::start();
    let (d, t) = setup(b"OLD");
    let none = release(&m.base, "v0.9.0", &[]);
    let e = apply(&m.src(), &none, "kora-linux-x86_64", &t, &RealFs, Flow::Unix).unwrap_err();
    assert!(e.to_string().contains("no kora-linux-x86_64"), "{e}");
    let only_bin = release(&m.base, "v0.9.0", &["kora-linux-x86_64".into()]);
    let e = apply(&m.src(), &only_bin, "kora-linux-x86_64", &t, &RealFs, Flow::Unix).unwrap_err();
    assert!(e.to_string().contains("checksum"), "{e}");
    assert!(m.hits.lock().unwrap().is_empty(), "nothing was requested");
    assert_eq!(leftovers(d.path()), ["kora"]);
}

#[test]
fn offline_is_a_clear_network_error() {
    let port = { TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port() }; // closed again
    let src = Source::from_override(Some(&format!("http://127.0.0.1:{port}")));
    let e = fetch_releases(&src).unwrap_err();
    assert!(matches!(e, SelfUpdateError::Network(_)), "{e}");
    assert!(e.to_string().starts_with("could not reach GitHub"));
}

#[test]
fn redirects_off_the_allow_list_are_refused() {
    let m = Mock::start();
    let (d, t) = setup(b"OLD");
    let rel = m.publish(b"NEW", b"NEW", "kora-linux-x86_64");
    m.redirect("/dl/kora-linux-x86_64", "https://evil.example.com/kora");
    let e = apply(&m.src(), &rel, "kora-linux-x86_64", &t, &RealFs, Flow::Unix).unwrap_err();
    assert!(e.to_string().contains("not a GitHub host"), "{e}");
    assert_eq!(leftovers(d.path()), ["kora"]);
    m.redirect("/dl/kora-linux-x86_64", "http://github.com/kora");
    assert!(apply(&m.src(), &rel, "kora-linux-x86_64", &t, &RealFs, Flow::Unix).is_err());
}

#[test]
fn redirects_to_allowed_hosts_are_followed() {
    let m = Mock::start();
    m.put("/final", b"BODY");
    m.redirect("/hop", "/final"); // relative, same host
    assert_eq!(m.src().net.get_text(&format!("{}/hop", m.base), 100).unwrap(), "BODY");
}

#[test]
fn size_cap() {
    let m = Mock::start();
    m.put("/big", &[7u8; 1000]);
    let mut sink = Vec::new();
    let e = m.src().net.download(&format!("{}/big", m.base), &mut sink, 999).unwrap_err();
    assert!(e.contains("too large") || e.contains("larger"), "{e}");
    assert!(sink.is_empty());
    assert_eq!(m.src().net.download(&format!("{}/big", m.base), &mut sink, 1000).unwrap(), 1000);
}

#[test]
fn override_env_only_accepts_loopback_http() {
    assert_eq!(Source::from_override(None).api_base, DEFAULT_API);
    assert_eq!(Source::from_override(Some("https://evil.com")).api_base, DEFAULT_API);
    assert_eq!(Source::from_override(Some("http://evil.com")).api_base, DEFAULT_API);
    let s = Source::from_override(Some("http://127.0.0.1:5/"));
    assert_eq!(s.api_base, "http://127.0.0.1:5");
    assert!(s.net.allow_loopback);
    assert!(!Source::production().net.allow_loopback);
}

fn v(s: &str) -> Version {
    Version::parse(s).unwrap()
}
fn rels(tags: &[(&str, bool)]) -> Vec<Release> {
    tags.iter().map(|(t, draft)| Release { tag_name: t.to_string(), draft: *draft, prerelease: false, assets: vec![] }).collect()
}

#[test]
fn decide_never_downgrades_without_version() {
    let r = rels(&[("v0.2.2-rc1", false), ("v0.2.1", false)]);
    // a stable build is not moved onto a release candidate; a pre-release build is
    assert!(matches!(decide(&v("0.2.1"), &r, None).unwrap(), Decision::UpToDate { latest } if latest == v("0.2.1")));
    assert!(matches!(decide(&v("0.2.0"), &r, None).unwrap(), Decision::Update(x) if x.tag_name == "v0.2.1"));
    assert!(matches!(decide(&v("0.2.1-rc1"), &r, None).unwrap(), Decision::Update(x) if x.tag_name == "v0.2.2-rc1"));
    assert!(matches!(decide(&v("0.2.2-rc1"), &r, None).unwrap(), Decision::UpToDate { .. }));
    assert!(matches!(decide(&v("0.3.0"), &r, None).unwrap(), Decision::UpToDate { .. }));
    // a stable build is newer than its own release candidate
    assert!(matches!(decide(&v("0.2.2"), &r, None).unwrap(), Decision::UpToDate { .. }));
    // drafts are ignored
    let d = rels(&[("v9.9.9", true), ("v0.2.1", false)]);
    assert!(matches!(decide(&v("0.2.1"), &d, None).unwrap(), Decision::UpToDate { .. }));
    assert!(decide(&v("0.2.1"), &[], None).is_err());
}

#[test]
fn explicit_version_may_go_either_way() {
    let r = rels(&[("v0.2.2", false), ("v0.2.0", false)]);
    assert!(matches!(decide(&v("0.2.2"), &r, Some("v0.2.0")).unwrap(), Decision::Update(x) if x.tag_name == "v0.2.0"));
    assert!(matches!(decide(&v("0.2.2"), &r, Some("0.2.2")).unwrap(), Decision::Update(_)));
    assert!(decide(&v("0.2.2"), &r, Some("v7.0.0")).is_err());
    let pre = rels(&[("v0.3.0-rc1", false), ("v0.2.2", false)]);
    assert!(matches!(decide(&v("0.2.2"), &pre, Some("v0.3.0-rc1")).unwrap(), Decision::Update(x) if x.tag_name == "v0.3.0-rc1"));
}

#[test]
fn a_stable_release_behind_many_pre_releases_is_still_found() {
    assert!(releases_url("https://api.github.com").ends_with("/releases?per_page=30"));
    let mut list: Vec<(String, bool)> = (1..=12).map(|i| (format!("v0.4.0-rc{i}"), false)).collect();
    list.push(("v0.3.5".into(), false));
    let r = rels(&list.iter().map(|(t, p)| (t.as_str(), *p)).collect::<Vec<_>>());
    assert!(matches!(decide(&v("0.3.0"), &r, None).unwrap(), Decision::Update(x) if x.tag_name == "v0.3.5"));
}

#[test]
fn fetches_and_picks_from_the_releases_endpoint() {
    let m = Mock::start();
    m.put(
        "/repos/KoraFiveM/kora-installer/releases?per_page=30",
        br#"[{"tag_name":"v0.2.2-rc1","draft":false,"prerelease":true,"assets":[]},{"tag_name":"v0.2.1","assets":[]}]"#,
    );
    let r = fetch_releases(&m.src()).unwrap();
    assert!(matches!(decide(&v("0.2.0"), &r, None).unwrap(), Decision::Update(x) if x.tag_name == "v0.2.1"));
    assert!(matches!(decide(&v("0.2.1-beta"), &r, None).unwrap(), Decision::Update(x) if x.tag_name == "v0.2.2-rc1"));
}

#[test]
fn target_is_the_installed_app_copy_else_the_running_file() {
    let d = tempfile::tempdir().unwrap();
    let running = d.path().join("elsewhere").join("kora");
    std::fs::create_dir_all(running.parent().unwrap()).unwrap();
    std::fs::write(&running, b"x").unwrap();
    let t = resolve_target(&running, false, None);
    assert!(!t.app_copy && t.state.is_none());
    assert_eq!(t.exe, std::fs::canonicalize(&running).unwrap());
    let app = (d.path().join("opt/kora/bin/kora"), d.path().join("opt/kora/bin/app.json"));
    let t = resolve_target(&running, false, Some(app.clone()));
    assert_eq!((t.exe, t.state, t.app_copy), (app.0, Some(app.1), true));
}

#[test]
fn updating_the_app_copy_keeps_app_status_truthful() {
    // fake app state as `kora app install` writes it (AppState) + the real file swap
    let m = Mock::start();
    let rel = m.publish(b"NEW", b"NEW", "kora-linux-x86_64");
    let (d, t) = setup(b"OLD");
    let state = d.path().join("app.json");
    let st = crate::app_install::AppState { version: "0.2.1".into(), sha256: "oldhash".into(), installed_unix: 5 };
    std::fs::write(&state, serde_json::to_vec_pretty(&st).unwrap()).unwrap();
    let target = resolve_target(Path::new("/nowhere"), false, Some((t.clone(), state.clone())));
    let a = apply(&m.src(), &rel, "kora-linux-x86_64", &target.exe, &RealFs, Flow::Unix).unwrap();
    update_app_state(target.state.as_ref().unwrap(), &v("0.9.0"), &a.sha256).unwrap();
    let back: crate::app_install::AppState = serde_json::from_slice(&std::fs::read(&state).unwrap()).unwrap();
    assert_eq!((back.version.as_str(), back.sha256.as_str()), ("0.9.0", a.sha256.as_str()));
    assert!(back.installed_unix > 5);
    assert_eq!(std::fs::read(&t).unwrap(), b"NEW");
}

#[test]
fn app_state_is_updated_in_place() {
    let d = tempfile::tempdir().unwrap();
    let s = d.path().join("app.json");
    std::fs::write(&s, br#"{"version":"0.2.1","sha256":"old","installed_unix":1,"extra":"keep"}"#).unwrap();
    update_app_state(&s, &v("0.2.2"), "newhash").unwrap();
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&s).unwrap()).unwrap();
    assert_eq!((j["version"].as_str(), j["sha256"].as_str(), j["extra"].as_str()), (Some("0.2.2"), Some("newhash"), Some("keep")));
    assert!(j["installed_unix"].as_u64().unwrap() > 1);
    assert_eq!(leftovers(d.path()), ["app.json"]);
}

#[test]
fn write_probe() {
    let d = tempfile::tempdir().unwrap();
    assert!(dir_writable(d.path()));
    assert!(!dir_writable(&d.path().join("missing")));
    assert!(leftovers(d.path()).is_empty());
}

#[test]
fn refresh_writes_the_cache_and_notice_follows() {
    let m = Mock::start();
    // This build is stable, so the release candidate listed first is not offered.
    m.put(
        "/repos/KoraFiveM/kora-installer/releases?per_page=30",
        br#"[{"tag_name":"v7.0.0-rc1","prerelease":true,"assets":[]},{"tag_name":"v6.0.0","assets":[]}]"#,
    );
    let d = tempfile::tempdir().unwrap();
    let cache = d.path().join("c").join("update-check.json");
    let src = m.src();
    let got = update_check::refresh(&src, &src.net, &cache, 123).unwrap();
    assert_eq!(got.to_string(), "6.0.0");
    assert_eq!(update_check::cached_latest_at(&cache), Some(got.clone()));
    let line = update_check::newer(&running_version(), update_check::cached_latest_at(&cache)).map(|v| update_check::notice_text(&v));
    assert_eq!(line.as_deref(), Some("Kora v6.0.0 is available - run: kora self-update"));
}

#[test]
fn refresh_failure_keeps_the_old_cache() {
    let m = Mock::start(); // no route: 404
    let d = tempfile::tempdir().unwrap();
    let cache = d.path().join("update-check.json");
    std::fs::write(&cache, r#"{"checked_unix":1,"latest":"0.5.0"}"#).unwrap();
    assert!(update_check::refresh(&m.src(), &m.src().net, &cache, 99).is_err());
    assert_eq!(update_check::cached_latest_at(&cache).unwrap().to_string(), "0.5.0");
}
