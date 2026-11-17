use super::*;
use std::io::Write;

const H: &str = "5412ed86f9eff1b81d2afb10cff85153b043d3b8";
fn lin(n: u32) -> String {
    format!("https://runtime.fivem.net/artifacts/fivem/build_proot_linux/master/{n}-{H}/fx.tar.xz")
}
fn win(n: u32) -> String {
    format!("https://runtime.fivem.net/artifacts/fivem/build_server_windows/master/{n}-{H}/server.zip")
}

#[test]
fn url_validation() {
    assert!(validate_url(&lin(1), Platform::Linux).is_ok());
    assert!(validate_url(&win(1), Platform::Windows).is_ok());
    assert!(validate_url(&win(1).replace("server.zip", "server.7z"), Platform::Windows).is_ok());
    assert!(validate_url(&lin(1), Platform::Windows).is_err());
    for u in [
        lin(1).replace("https", "http"),
        lin(1).replace("runtime.fivem.net", "runtime.fivem.net.evil.com"),
        lin(1).replace("runtime.fivem.net", "x@runtime.fivem.net"),
        format!("{}?x=1", lin(1)),
        lin(1).replace("fx.tar.xz", "../fx.tar.xz"),
        lin(1).replace("/1-", "/abc-"),
        lin(1).replace(H, "short"),
        lin(1).replace("/fx.tar.xz", "/x/fx.tar.xz"),
    ] {
        assert!(validate_url(&u, Platform::Linux).is_err(), "{u}");
    }
}

#[test]
fn jg_parse_golden() {
    let json = format!(r#"{{"recommendedArtifact":"38621","windowsDownloadLink":"{}","linuxDownloadLink":"{}","brokenArtifacts":{{"38000":"bad"}}}}"#, win(38621), lin(38621));
    let l = parse_jg(&json, Platform::Linux).unwrap();
    assert_eq!(l.recommended.as_ref().unwrap().build, 38621);
    assert_eq!(l.pick(Channel::Recommended).unwrap().url, lin(38621));
    assert_eq!(l.broken[&38000], "bad");
    assert!(parse_jg(&json.replace("38621\",\"win", "1\",\"win"), Platform::Linux).is_err());
    assert!(parse_jg(&json.replace("runtime.fivem.net", "evil.net"), Platform::Linux).is_err());
}

struct Fake {
    jg: Option<String>,
    cfx: Option<String>,
    body: Vec<u8>,
}
impl ArtifactSource for Fake {
    fn fetch_text(&self, url: &str) -> Result<String> {
        let v = if url == JG_URL { &self.jg } else { &self.cfx };
        v.clone().ok_or_else(|| ArtifactError::Network("offline".into()))
    }
    fn download(&self, _: &str, out: &mut dyn Write) -> Result<()> {
        out.write_all(&self.body)?;
        Ok(())
    }
}

fn cfx_json() -> String {
    format!(
        r#"{{"recommended":"38435","latest":"38700","recommended_download":"{}","latest_download":"{}","critical_download":"{}","optional_download":"http://evil/x"}}"#,
        lin(38435),
        lin(38700),
        lin(7290)
    )
}

#[test]
fn merge_sources() {
    let jg = format!(r#"{{"recommendedArtifact":"38621","linuxDownloadLink":"{}","windowsDownloadLink":"{}","brokenArtifacts":{{"7290":"old"}}}}"#, lin(38621), win(38621));
    let f = Fake { jg: Some(jg), cfx: Some(cfx_json()), body: vec![] };
    let l = list_artifacts(&f, Platform::Linux).unwrap();
    assert_eq!(l.recommended.as_ref().unwrap().build, 38621);
    assert_eq!(l.latest.as_ref().unwrap().build, 38700);
    assert_eq!(l.all.iter().map(|b| b.build).collect::<Vec<_>>(), vec![38700, 38621, 38435, 7290]);
    assert_eq!(l.find(7290).unwrap().broken.as_deref(), Some("old"));
    let f = Fake { jg: None, cfx: Some(cfx_json()), body: vec![] };
    assert_eq!(list_artifacts(&f, Platform::Linux).unwrap().recommended.unwrap().build, 38435);
    let f = Fake { jg: None, cfx: None, body: vec![] };
    assert!(list_artifacts(&f, Platform::Linux).is_err());
}

#[test]
fn html_listing() {
    let html = format!(r#"<a href="./38621-{H}/">x</a><a href="./38000-{H}/">y</a><a href="../">up</a><a href="./9-bad/">z</a>"#);
    let l = parse_cfx_listing(&html, Platform::Windows).unwrap();
    assert_eq!(l.all.len(), 2);
    assert_eq!(l.latest.unwrap().build, 38621);
}

#[test]
fn download_verification() {
    let d = tempfile::tempdir().unwrap();
    let f = Fake { jg: None, cfx: None, body: b"hello".to_vec() };
    let sha = hex::encode(Sha256::digest(b"hello"));
    let p = d.path().join("a");
    let v = Verify { sha256: Some(sha.to_uppercase()), size: Some(5), max_size: None };
    assert_eq!(download_verified(&f, &lin(1), Platform::Linux, &p, &v).unwrap(), sha);
    let p2 = d.path().join("b");
    let bad = Verify { sha256: Some("00".into()), ..Default::default() };
    assert!(matches!(download_verified(&f, &lin(1), Platform::Linux, &p2, &bad), Err(ArtifactError::Verify(_))));
    assert!(!p2.exists());
    let p3 = d.path().join("c");
    let big = Verify { max_size: Some(3), ..Default::default() };
    assert!(download_verified(&f, &lin(1), Platform::Linux, &p3, &big).is_err());
    assert!(!p3.exists());
    let size = Verify { size: Some(9), ..Default::default() };
    assert!(download_verified(&f, &lin(1), Platform::Linux, &d.path().join("d"), &size).is_err());
    assert!(download_verified(&f, "http://x", Platform::Linux, &d.path().join("e"), &Verify::default()).is_err());
}

#[test]
fn game_build() {
    assert_eq!(check_game_build(3751, 3258), GameBuildCheck::Ok);
    assert!(game_build_warning(2802, 3258).unwrap().contains("3258"));
    assert!(game_build_warning(3258, 3258).is_none());
}

fn tar_xz(entries: &[(&str, &[u8])], raw_name: Option<&str>) -> Vec<u8> {
    let mut tb = tar::Builder::new(Vec::new());
    for (n, data) in entries {
        let mut h = tar::Header::new_gnu();
        h.set_size(data.len() as u64);
        h.set_mode(0o755);
        h.set_entry_type(tar::EntryType::Regular);
        if raw_name == Some(n) {
            h.as_old_mut().name[..n.len()].copy_from_slice(n.as_bytes());
            h.set_cksum();
            tb.append(&h, *data).unwrap();
        } else {
            tb.append_data(&mut h, n, *data).unwrap();
        }
    }
    let mut h = tar::Header::new_gnu();
    h.set_entry_type(tar::EntryType::Symlink);
    h.set_size(0);
    tb.append_link(&mut h, "alpine/link", "run.sh").unwrap();
    let raw = tb.into_inner().unwrap();
    let mut e = xz2::write::XzEncoder::new(Vec::new(), 1);
    e.write_all(&raw).unwrap();
    e.finish().unwrap()
}

#[cfg(unix)]
fn store_with(build: u32, dir: &std::path::Path, store: &ArtifactStore, tag: &str) {
    let a = dir.join(format!("a{build}.tar.xz"));
    std::fs::write(&a, tar_xz(&[("run.sh", tag.as_bytes()), ("alpine/x", b"x")], None)).unwrap();
    store.swap(build, &a, Platform::Linux).unwrap();
}

#[test]
#[cfg(unix)] // Linux archives contain symlinks, which Windows cannot extract (by design)
fn extract_swap_rollback() {
    let d = tempfile::tempdir().unwrap();
    let store = ArtifactStore::new(d.path().join("artifacts"));
    store_with(1, d.path(), &store, "one");
    assert_eq!(std::fs::read_to_string(store.build_dir(1).join("run.sh")).unwrap(), "one");
    #[cfg(unix)]
    assert_eq!(std::fs::read_link(store.build_dir(1).join("alpine/link")).unwrap().to_str(), Some("run.sh"));
    store_with(2, d.path(), &store, "two");
    assert_eq!(store.state().unwrap(), StoreState { current: Some(2), previous: Some(1) });
    store_with(3, d.path(), &store, "three");
    assert!(!store.build_dir(1).exists(), "older than previous is pruned");
    assert_eq!(store.rollback().unwrap(), 2);
    assert_eq!(store.current_dir().unwrap().unwrap(), store.build_dir(2));
    assert_eq!(store.rollback().unwrap(), 3);
    // failed swap leaves state intact
    let junk = d.path().join("junk");
    std::fs::write(&junk, b"garbage!").unwrap();
    assert!(store.swap(9, &junk, Platform::Linux).is_err());
    assert_eq!(store.state().unwrap().current, Some(3));
    assert!(!store.build_dir(9).exists());
    assert!(store.swap(3, &junk, Platform::Linux).is_err());
}

#[test]
fn rejects_traversal_and_missing_exe() {
    let d = tempfile::tempdir().unwrap();
    let a = d.path().join("t.tar.xz");
    std::fs::write(&a, tar_xz(&[("run.sh", b"x"), ("../evil", b"x")], Some("../evil"))).unwrap();
    let out = d.path().join("o");
    std::fs::create_dir(&out).unwrap();
    assert!(extract_archive(&a, &out, Platform::Linux, ExtractLimits::default()).is_err());
    assert!(!d.path().join("evil").exists());
    let a2 = d.path().join("n.tar.xz");
    std::fs::write(&a2, tar_xz(&[("other", b"x")], None)).unwrap();
    let out2 = d.path().join("o2");
    std::fs::create_dir(&out2).unwrap();
    assert!(extract_archive(&a2, &out2, Platform::Linux, ExtractLimits::default()).is_err());
    let out3 = d.path().join("o3");
    std::fs::create_dir(&out3).unwrap();
    let lim = ExtractLimits { max_total: 1, max_entries: 10 };
    assert!(extract_archive(&a, &out3, Platform::Linux, lim).is_err());
}

#[test]
fn zip_extract_and_traversal() {
    let d = tempfile::tempdir().unwrap();
    let mk = |names: &[&str]| {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for n in names {
            w.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(b"x").unwrap();
        }
        w.finish().unwrap().into_inner()
    };
    let a = d.path().join("s.zip");
    std::fs::write(&a, mk(&["FXServer.exe", "citizen/a.txt"])).unwrap();
    let o = d.path().join("o");
    std::fs::create_dir(&o).unwrap();
    extract_archive(&a, &o, Platform::Windows, ExtractLimits::default()).unwrap();
    assert!(o.join("citizen/a.txt").is_file());
    for (i, evil) in ["../x", "/abs", "a\\b", "C:/x"].iter().enumerate() {
        std::fs::write(&a, mk(&["FXServer.exe", evil])).unwrap();
        let o = d.path().join(format!("e{i}"));
        std::fs::create_dir(&o).unwrap();
        assert!(extract_archive(&a, &o, Platform::Windows, ExtractLimits::default()).is_err(), "{evil}");
    }
    std::fs::write(&a, mk(&["FXServer.exe", "fxserver.EXE"])).unwrap();
    let o = d.path().join("dup");
    std::fs::create_dir(&o).unwrap();
    assert!(extract_archive(&a, &o, Platform::Windows, ExtractLimits::default()).is_err());
}

#[cfg(unix)]
fn tar_with_link(link: &str, target: &str) -> Vec<u8> {
    let mut tb = tar::Builder::new(Vec::new());
    let mut h = tar::Header::new_gnu();
    h.set_size(1);
    h.set_mode(0o755);
    h.set_entry_type(tar::EntryType::Regular);
    tb.append_data(&mut h, "run.sh", &b"x"[..]).unwrap();
    let mut h = tar::Header::new_gnu();
    h.set_entry_type(tar::EntryType::Symlink);
    h.set_size(0);
    tb.append_link(&mut h, link, target).unwrap();
    let raw = tb.into_inner().unwrap();
    let mut e = xz2::write::XzEncoder::new(Vec::new(), 1);
    e.write_all(&raw).unwrap();
    e.finish().unwrap()
}

#[test]
#[cfg(unix)]
fn absolute_links_only_allowed_inside_alpine_rootfs() {
    let d = tempfile::tempdir().unwrap();
    // The real FXServer archive has `alpine/bin/ls -> /bin/busybox` (resolved by proot).
    let ok = d.path().join("ok.tar.xz");
    std::fs::write(&ok, tar_with_link("alpine/bin/ls", "/bin/busybox")).unwrap();
    let dest = d.path().join("ok");
    std::fs::create_dir(&dest).unwrap();
    extract_archive(&ok, &dest, Platform::Linux, ExtractLimits::default()).unwrap();
    for (link, target) in [("evil", "/etc/passwd"), ("alpine/up", "/bin/../../etc"), ("alpine/rel", "../../x")] {
        let f = d.path().join("bad.tar.xz");
        std::fs::write(&f, tar_with_link(link, target)).unwrap();
        let dest = d.path().join(format!("bad-{}", link.replace('/', "_")));
        std::fs::create_dir(&dest).unwrap();
        assert!(extract_archive(&f, &dest, Platform::Linux, ExtractLimits::default()).is_err(), "{link} -> {target}");
    }
}

/// Entries for a crafted tar.xz: `Ok(data)` is a file, `Err(target)` a symlink.
#[cfg(unix)]
fn tar_entries(entries: &[(&str, std::result::Result<&str, &str>)]) -> Vec<u8> {
    let mut tb = tar::Builder::new(Vec::new());
    for (name, kind) in entries {
        let mut h = tar::Header::new_gnu();
        match kind {
            Ok(data) => {
                h.set_size(data.len() as u64);
                h.set_mode(0o755);
                h.set_entry_type(tar::EntryType::Regular);
                tb.append_data(&mut h, name, data.as_bytes()).unwrap();
            }
            Err(target) => {
                h.set_entry_type(tar::EntryType::Symlink);
                h.set_size(0);
                tb.append_link(&mut h, name, target).unwrap();
            }
        }
    }
    let raw = tb.into_inner().unwrap();
    let mut e = xz2::write::XzEncoder::new(Vec::new(), 1);
    e.write_all(&raw).unwrap();
    e.finish().unwrap()
}

#[test]
#[cfg(unix)]
fn nothing_is_written_through_a_link() {
    let d = tempfile::tempdir().unwrap();
    let outside = d.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    let out = outside.to_str().unwrap();
    let cases: Vec<Vec<(&str, std::result::Result<&str, &str>)>> = vec![
        // An absolute rootfs link, then a link created beneath it.
        vec![("run.sh", Ok("x")), ("alpine/a", Err(out)), ("alpine/a/pwn", Err("/x"))],
        // The same with a file: the file would land wherever the link points.
        vec![("run.sh", Ok("x")), ("alpine/a", Err(out)), ("alpine/a/pwn", Ok("x"))],
        vec![("run.sh", Ok("x")), ("alpine/a/pwn", Ok("x")), ("alpine/a", Err(out))],
        // A relative chain that each looks fine on its own: d/l -> root, d/l/z -> root/..
        vec![("run.sh", Ok("x")), ("d/l", Err("..")), ("d/l/z", Err("../.."))],
        // `..` after passing through a link: a/b/l -> a, so a/m -> b/l/../.. is root/..
        vec![("run.sh", Ok("x")), ("a/b/l", Err("..")), ("a/m", Err("b/l/../.."))],
    ];
    for (i, entries) in cases.iter().enumerate() {
        let f = d.path().join(format!("c{i}.tar.xz"));
        std::fs::write(&f, tar_entries(entries)).unwrap();
        let dest = d.path().join(format!("c{i}"));
        std::fs::create_dir(&dest).unwrap();
        assert!(extract_archive(&f, &dest, Platform::Linux, ExtractLimits::default()).is_err(), "case {i}");
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0, "case {i} wrote outside");
    }
}

#[test]
#[cfg(unix)]
fn ordinary_rootfs_links_still_extract() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("ok.tar.xz");
    let entries: [(&str, std::result::Result<&str, &str>); 8] = [
        ("run.sh", Ok("x")),
        ("alpine/bin/busybox", Ok("x")),
        ("alpine/bin/ls", Err("/bin/busybox")),
        ("alpine/lib/ld-musl-x86_64.so.1", Err("/lib/libc.musl-x86_64.so.1")),
        ("alpine/usr/bin/env", Err("../../bin/busybox")),
        ("alpine/sbin", Err("bin")),
        ("alpine/usr/lib/libz.so", Err("libz.so.1.3")),
        ("alpine/usr/sbin/sh", Err("../../sbin/ls")),
    ];
    std::fs::write(&f, tar_entries(&entries)).unwrap();
    let dest = d.path().join("ok");
    std::fs::create_dir(&dest).unwrap();
    extract_archive(&f, &dest, Platform::Linux, ExtractLimits::default()).unwrap();
    assert_eq!(std::fs::read_link(dest.join("alpine/usr/bin/env")).unwrap().to_str(), Some("../../bin/busybox"));
    assert!(dest.join("alpine/usr/bin/env").is_file());
    assert_eq!(std::fs::read_link(dest.join("alpine/bin/ls")).unwrap().to_str(), Some("/bin/busybox"));
}
