use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::*;

/// In-memory machine: files, links, machine PATH, shortcuts and the Apps & features key.
#[derive(Default)]
struct Fake {
    files: BTreeMap<PathBuf, Vec<u8>>,
    dirs: Vec<PathBuf>,
    links: BTreeMap<PathBuf, PathBuf>,
    path: Option<String>,
    shortcuts: BTreeMap<PathBuf, ShortcutSpec>,
    key: Option<UninstallEntry>,
    log: RefCell<Vec<String>>,
    broadcasts: usize,
}

fn hash(b: &[u8]) -> String {
    format!("h{}", b.iter().map(|x| *x as u64).sum::<u64>() + b.len() as u64)
}

impl Sys for Fake {
    fn exists(&self, p: &Path) -> bool {
        self.files.contains_key(p) || self.links.contains_key(p) || self.dirs.iter().any(|d| d == p) || self.shortcuts.contains_key(p)
    }
    fn read(&self, p: &Path) -> Option<Vec<u8>> {
        self.files.get(p).cloned()
    }
    fn sha256(&self, p: &Path) -> Option<String> {
        self.files.get(p).map(|b| hash(b))
    }
    fn read_link(&self, p: &Path) -> Option<PathBuf> {
        self.links.get(p).cloned()
    }
    fn write(&mut self, p: &Path, bytes: &[u8], _mode: u32) -> Result<()> {
        self.log.borrow_mut().push(format!("write {}", p.display()));
        self.files.insert(p.to_path_buf(), bytes.to_vec());
        if let Some(d) = p.parent() {
            self.dirs.push(d.to_path_buf());
        }
        Ok(())
    }
    fn copy_exe(&mut self, from: &Path, to: &Path) -> Result<()> {
        self.log.borrow_mut().push(format!("copy {} {}", from.display(), to.display()));
        let b = self.files.get(from).cloned().ok_or_else(|| AppError("no source".into()))?;
        self.files.insert(to.to_path_buf(), b);
        if let Some(d) = to.parent() {
            self.dirs.push(d.to_path_buf());
        }
        Ok(())
    }
    fn symlink(&mut self, target: &Path, link: &Path) -> Result<()> {
        self.links.insert(link.to_path_buf(), target.to_path_buf());
        Ok(())
    }
    fn remove_file(&mut self, p: &Path) -> Result<()> {
        self.log.borrow_mut().push(format!("remove {}", p.display()));
        self.files.remove(p);
        self.links.remove(p);
        self.shortcuts.remove(p);
        Ok(())
    }
    fn remove_dir_if_empty(&mut self, p: &Path) -> Result<()> {
        let busy = self.files.keys().chain(self.links.keys()).any(|f| f.starts_with(p));
        if !busy {
            self.dirs.retain(|d| d != p);
        }
        Ok(())
    }
    fn machine_path(&self) -> Result<Option<String>> {
        Ok(self.path.clone())
    }
    fn set_machine_path(&mut self, v: &str) -> Result<()> {
        self.path = Some(v.to_string());
        Ok(())
    }
    fn broadcast_env_change(&mut self) {
        self.broadcasts += 1;
    }
    fn create_shortcut(&mut self, s: &ShortcutSpec) -> Result<()> {
        self.log.borrow_mut().push(format!("shortcut {}", s.path.display()));
        self.shortcuts.insert(s.path.clone(), s.clone());
        Ok(())
    }
    fn read_shortcut(&self, p: &Path) -> Option<ShortcutSpec> {
        self.shortcuts.get(p).cloned()
    }
    fn uninstall_entry(&self) -> Option<UninstallEntry> {
        self.key.clone()
    }
    fn set_uninstall_entry(&mut self, e: &UninstallEntry) -> Result<()> {
        self.key = Some(e.clone());
        Ok(())
    }
    fn remove_uninstall_entry(&mut self) -> Result<()> {
        self.key = None;
        Ok(())
    }
}

fn win_layout() -> Layout {
    let env = |k: &str| -> Option<OsString> {
        Some(match k {
            "ProgramFiles" => r"C:\Program Files".into(),
            "ProgramData" => r"C:\ProgramData".into(),
            "PUBLIC" => r"C:\Users\Public".into(),
            _ => return None,
        })
    };
    Layout::resolve(true, &env, Path::new(r"C:\x\kora.exe")).unwrap()
}

#[cfg(unix)]
fn unix_layout(session: bool, exe: &str) -> Layout {
    let env = |k: &str| -> Option<OsString> {
        match k {
            "KORA_APP_PREFIX" => Some("/tmp/p".into()),
            "DISPLAY" if session => Some(":0".into()),
            _ => None,
        }
    };
    Layout::resolve(false, &env, Path::new(exe)).unwrap()
}

fn running(version: &str, content: &[u8], exe: &str) -> (Running, Vec<u8>) {
    (Running { exe: PathBuf::from(exe), version: version.into(), sha256: hash(content) }, content.to_vec())
}

fn machine_with(r: &Running, content: Vec<u8>) -> Fake {
    let mut f = Fake { path: Some(r"C:\Windows;%SystemRoot%\System32".into()), ..Fake::default() };
    f.files.insert(r.exe.clone(), content);
    f
}

const OPTS: AppOptions = AppOptions { desktop_shortcut: true, force: false, dry_run: false };

#[test]
fn windows_install_creates_everything_and_is_idempotent() {
    let l = win_layout();
    let (r, c) = running("0.2.0", b"program-v2", r"C:\x\kora.exe");
    let mut f = machine_with(&r, c);
    let out = install_app_with(&mut f, &l, &r, &OPTS).unwrap();
    assert!(out.changed());
    assert!(f.files.contains_key(&l.exe) && f.files.contains_key(&l.state));
    assert_eq!(f.path.as_deref(), Some(r"C:\Windows;%SystemRoot%\System32;C:\Program Files\Kora"), "other entries kept verbatim");
    assert_eq!(f.broadcasts, 1);
    for p in [l.start_menu.as_ref().unwrap(), l.desktop.as_ref().unwrap()] {
        let s = &f.shortcuts[p];
        assert_eq!((s.target.as_path(), s.args.as_str()), (l.exe.as_path(), "manager"));
        assert!(s.run_as_admin && s.workdir == l.dir && s.icon == r"C:\Program Files\Kora\kora.exe,0");
    }
    let k = f.key.clone().unwrap();
    assert_eq!((k.display_name.as_str(), k.display_version.as_str(), k.publisher.as_str(), k.install_location.as_str()), ("Kora", "0.2.0", "Kora", r"C:\Program Files\Kora"));
    assert_eq!(k.uninstall_string, r#""C:\Program Files\Kora\kora.exe" uninstall --app"#, "quoted because of the space");
    // second run: nothing changes
    f.log.borrow_mut().clear();
    let again = install_app_with(&mut f, &l, &r, &OPTS).unwrap();
    assert!(!again.changed(), "{:?}", again.changes);
    assert!(f.log.borrow().is_empty());
    assert_eq!(f.path.as_deref().unwrap().matches("Kora").count(), 1, "PATH entry added once");
    assert_eq!(f.broadcasts, 1);
}

#[test]
fn uninstall_removes_only_what_the_app_made() {
    let l = win_layout();
    let (r, c) = running("0.2.0", b"p", r"C:\x\kora.exe");
    let mut f = machine_with(&r, c);
    install_app_with(&mut f, &l, &r, &OPTS).unwrap();
    // things that must survive
    f.files.insert(PathBuf::from(r"C:\Program Files\Kora\bin\0.2.0-abc\kora.exe"), b"service copy".to_vec());
    f.files.insert(PathBuf::from(r"D:\servers\alpha\server.cfg"), b"cfg".to_vec());
    let other = PathBuf::from(r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\Other.lnk");
    f.shortcuts.insert(
        other.clone(),
        ShortcutSpec {
            path: other.clone(),
            target: r"C:\other.exe".into(),
            args: String::new(),
            workdir: PathBuf::new(),
            icon: String::new(),
            description: String::new(),
            run_as_admin: false,
            minimized: false,
        },
    );
    f.path = Some(format!("{};C:\\Program Files\\Kora Tools", f.path.unwrap()));

    let out = uninstall_app_with(&mut f, &l, false).unwrap();
    assert!(!out.is_empty());
    assert!(!f.files.contains_key(&l.exe) && !f.files.contains_key(&l.state));
    assert!(f.shortcuts.keys().all(|k| *k == other), "our shortcuts gone, the other one kept");
    assert!(f.key.is_none());
    assert_eq!(f.path.as_deref(), Some(r"C:\Windows;%SystemRoot%\System32;C:\Program Files\Kora Tools"));
    assert!(f.files.contains_key(Path::new(r"C:\Program Files\Kora\bin\0.2.0-abc\kora.exe")), "service copies stay");
    assert!(f.files.contains_key(Path::new(r"D:\servers\alpha\server.cfg")), "servers stay");
    // uninstalling again is a harmless no-op
    assert!(uninstall_app_with(&mut f, &l, false).unwrap().iter().all(|s| !s.starts_with("Delete")));
}

#[test]
fn uninstall_leaves_a_foreign_shortcut_named_kora() {
    let l = win_layout();
    let mut f = Fake { path: Some(String::new()), ..Fake::default() };
    let p = l.start_menu.clone().unwrap();
    f.shortcuts.insert(
        p.clone(),
        ShortcutSpec {
            path: p.clone(),
            target: r"C:\games\kora-game.exe".into(),
            args: String::new(),
            workdir: PathBuf::new(),
            icon: String::new(),
            description: String::new(),
            run_as_admin: false,
            minimized: false,
        },
    );
    let out = uninstall_app_with(&mut f, &l, false).unwrap();
    assert!(f.shortcuts.contains_key(&p));
    assert!(out.iter().any(|s| s.contains("left alone")));
}

#[test]
fn uninstall_dry_run_changes_nothing() {
    let l = win_layout();
    let (r, c) = running("0.2.0", b"p", r"C:\x\kora.exe");
    let mut f = machine_with(&r, c);
    install_app_with(&mut f, &l, &r, &OPTS).unwrap();
    f.log.borrow_mut().clear();
    let out = uninstall_app_with(&mut f, &l, true).unwrap();
    assert!(out.len() >= 6 && f.log.borrow().is_empty() && f.key.is_some());
}

#[test]
fn status_reports_the_installed_state() {
    let l = win_layout();
    let (r, c) = running("0.2.0", b"p", r"C:\x\kora.exe");
    let mut f = machine_with(&r, c);
    assert!(!status_with(&f, &l, &r).installed);
    install_app_with(&mut f, &l, &r, &OPTS).unwrap();
    let s = status_with(&f, &l, &r);
    assert!(s.installed && s.on_path && s.uninstall_registered && !s.update_available);
    assert_eq!((s.start_menu, s.desktop, s.version.as_deref()), (Some(true), Some(true), Some("0.2.0")));
    let newer = Running { version: "0.3.0".into(), ..r };
    assert!(status_with(&f, &l, &newer).update_available);
    assert!(s.lines().contains(&"installed: yes".to_string()));
}

#[cfg(unix)]
#[test]
fn unix_install_links_command_and_writes_launcher_only_in_a_desktop_session() {
    for session in [true, false] {
        let l = unix_layout(session, "/home/me/kora");
        let (r, c) = running("0.2.0", b"elf", "/home/me/kora");
        let mut f = Fake::default();
        f.files.insert(r.exe.clone(), c);
        let out = install_app_with(&mut f, &l, &r, &OPTS).unwrap();
        assert_eq!(l.exe, Path::new("/tmp/p/opt/kora/bin/kora"));
        assert!(f.files.contains_key(&l.exe));
        assert_eq!(f.links[Path::new("/tmp/p/usr/local/bin/kora")], l.exe);
        let entry = l.desktop_entry.clone().unwrap();
        assert_eq!(f.files.contains_key(&entry), session, "launcher only with a desktop session");
        if session {
            let text = String::from_utf8(f.files[&entry].clone()).unwrap();
            assert!(text.contains("Exec=/tmp/p/opt/kora/bin/kora manager") && text.contains("Terminal=true") && text.contains(DESKTOP_MARKER));
            assert!(text.contains("Icon=/tmp/p/opt/kora/share/kora-logo-512.png"));
            assert_eq!(f.files[l.icon_png.as_ref().unwrap()], LOGO_PNG);
        }
        assert!(!install_app_with(&mut f, &l, &r, &OPTS).unwrap().changed(), "idempotent");
        assert!(out.changed());
    }
}

#[cfg(unix)]
#[test]
fn unix_does_not_touch_foreign_command_or_launcher() {
    let l = unix_layout(true, "/home/me/kora");
    let (r, c) = running("0.2.0", b"elf", "/home/me/kora");
    let mut f = Fake::default();
    f.files.insert(r.exe.clone(), c);
    f.links.insert(l.link.clone().unwrap(), PathBuf::from("/usr/bin/other-kora"));
    f.files.insert(l.desktop_entry.clone().unwrap(), b"[Desktop Entry]\nName=Mine\n".to_vec());
    let out = install_app_with(&mut f, &l, &r, &OPTS).unwrap();
    assert_eq!(f.links[l.link.as_ref().unwrap()], Path::new("/usr/bin/other-kora"));
    assert_eq!(f.files[l.desktop_entry.as_ref().unwrap()], b"[Desktop Entry]\nName=Mine\n");
    assert!(out.notes.iter().any(|n| n.contains("already points")) && out.notes.iter().any(|n| n.contains("not made by Kora")));
    // and uninstall leaves them too
    uninstall_app_with(&mut f, &l, false).unwrap();
    assert_eq!(f.links[l.link.as_ref().unwrap()], Path::new("/usr/bin/other-kora"));
    assert!(f.files.contains_key(l.desktop_entry.as_ref().unwrap()));
}

#[cfg(unix)]
#[test]
fn unix_user_install_skips_the_system_link() {
    let l = unix_layout(false, "/home/me/.local/bin/kora");
    assert!(l.user_install);
    let (r, c) = running("0.2.0", b"elf", "/home/me/.local/bin/kora");
    let mut f = Fake::default();
    f.files.insert(r.exe.clone(), c);
    let out = install_app_with(&mut f, &l, &r, &OPTS).unwrap();
    assert!(f.links.is_empty());
    assert!(out.notes.iter().any(|n| n.contains("~/.local/bin")));
}

#[cfg(unix)]
#[test]
fn unix_uninstall_removes_the_copy_link_and_launcher() {
    let l = unix_layout(true, "/home/me/kora");
    let (r, c) = running("0.2.0", b"elf", "/home/me/kora");
    let mut f = Fake::default();
    f.files.insert(r.exe.clone(), c);
    f.files.insert(PathBuf::from("/tmp/p/opt/kora/0.2.0-abc/kora"), b"svc".to_vec());
    install_app_with(&mut f, &l, &r, &OPTS).unwrap();
    uninstall_app_with(&mut f, &l, false).unwrap();
    assert!(!f.files.contains_key(&l.exe) && f.links.is_empty());
    assert!(!f.files.contains_key(l.desktop_entry.as_ref().unwrap()) && !f.files.contains_key(l.icon_png.as_ref().unwrap()));
    assert!(f.files.contains_key(Path::new("/tmp/p/opt/kora/0.2.0-abc/kora")), "a server's service copy is not ours to remove");
}

#[test]
fn semver_comparison() {
    use Ordering::*;
    for (a, b, want) in [
        ("0.2.0", "0.2.0", Equal),
        ("0.10.0", "0.9.9", Greater),
        ("1.0.0", "1.0.0-rc.1", Greater),
        ("1.0.0-rc.2", "1.0.0-rc.10", Less),
        ("1.0.0-alpha", "1.0.0-alpha.1", Less),
        ("v1.2", "1.2.0", Equal),
        ("1.0.0+build5", "1.0.0", Equal),
    ] {
        assert_eq!(compare_versions(a, b), Some(want), "{a} vs {b}");
    }
    assert_eq!(compare_versions("garbage", "1.0.0"), None);
}

#[test]
fn quoting_of_launcher_exec_and_uninstall_string() {
    assert_eq!(desktop_exec_arg(Path::new("/opt/kora/bin/kora")), "/opt/kora/bin/kora");
    assert_eq!(desktop_exec_arg(Path::new("/opt/my kora/bin/kora")), "\"/opt/my kora/bin/kora\"");
    assert_eq!(desktop_exec_arg(Path::new("/opt/we\"ird$/kora")), "\"/opt/we\\\"ird\\$/kora\"");
    // A literal `%` would otherwise be read as a field code (`%f`, `%U`) by the desktop.
    assert_eq!(desktop_exec_arg(Path::new("/opt/100%/kora")), "\"/opt/100%%/kora\"");
    assert_eq!(uninstall_string(Path::new(r"C:\Program Files\Kora\kora.exe")), r#""C:\Program Files\Kora\kora.exe" uninstall --app"#);
    assert_eq!(uninstall_string(Path::new(r"C:\Prögramme (x86)\Kora\kora.exe")), "\"C:\\Prögramme (x86)\\Kora\\kora.exe\" uninstall --app");
}

#[cfg(unix)]
#[test]
fn path_hint_only_when_missing() {
    assert!(path_hint(Path::new("/usr/local/bin"), "/usr/bin:/usr/local/bin").is_none());
    let h = path_hint(Path::new("/home/me/.local/bin"), "/usr/bin").unwrap();
    assert!(h.contains("export PATH=\"/home/me/.local/bin:$PATH\""), "{h}");
}

#[test]
fn layout_windows_paths() {
    let l = win_layout();
    assert_eq!(l.exe, Path::new(r"C:\Program Files\Kora\kora.exe"));
    assert_eq!(l.start_menu.unwrap(), Path::new(r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\Kora.lnk"));
    assert_eq!(l.desktop.unwrap(), Path::new(r"C:\Users\Public\Desktop\Kora.lnk"));
}

/// The real Unix implementation, in a temp prefix (never the real /usr/local/bin).
#[cfg(unix)]
#[test]
fn real_unix_install_status_uninstall_in_a_temp_prefix() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("src-kora");
    std::fs::write(&src, b"#!/bin/sh\necho hi\n").unwrap();
    let prefix = t.path().join("root");
    let p = prefix.clone();
    let env = move |k: &str| -> Option<OsString> {
        match k {
            "KORA_APP_PREFIX" => Some(p.clone().into_os_string()),
            "DISPLAY" => Some(":0".into()),
            _ => None,
        }
    };
    let l = Layout::resolve(false, &env, &src).unwrap();
    let r = Running { exe: src.clone(), version: "0.2.0".into(), sha256: crate::service::binary::file_sha256(&src).unwrap() };
    let mut sys = RealSys::new();
    let out = install_app_with(&mut sys, &l, &r, &OPTS).unwrap();
    assert!(out.changed());
    assert_eq!(std::fs::read(&l.exe).unwrap(), std::fs::read(&src).unwrap());
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&l.exe).unwrap().permissions().mode() & 0o777, 0o755);
    }
    assert_eq!(std::fs::read_link(l.link.as_ref().unwrap()).unwrap(), l.exe);
    assert!(std::fs::read_to_string(l.desktop_entry.as_ref().unwrap()).unwrap().contains("Exec="));
    assert!(std::fs::metadata(l.icon_png.as_ref().unwrap()).unwrap().len() > 1000);
    let st = status_with(&sys, &l, &r);
    assert!(st.installed && st.on_path && st.desktop_entry == Some(true));
    assert!(!install_app_with(&mut sys, &l, &r, &OPTS).unwrap().changed());
    // atomic replace with a newer program
    std::fs::write(&src, b"#!/bin/sh\necho newer\n").unwrap();
    let r2 = Running { version: "0.3.0".into(), sha256: crate::service::binary::file_sha256(&src).unwrap(), ..r.clone() };
    assert!(install_app_with(&mut sys, &l, &r2, &OPTS).unwrap().changed());
    assert!(std::fs::read_to_string(&l.exe).unwrap().contains("newer"));
    uninstall_app_with(&mut sys, &l, false).unwrap();
    assert!(!l.exe.exists() && l.link.as_ref().unwrap().symlink_metadata().is_err() && !l.desktop_entry.as_ref().unwrap().exists());
    assert!(!l.dir.exists(), "empty program folder removed");
    assert!(src.exists(), "the source program is untouched");
}

#[test]
#[cfg(unix)]
fn a_leftover_temp_file_gets_the_requested_mode() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("app.json");
    let tmp = p.with_extension("kora-tmp");
    std::fs::write(&tmp, "old").unwrap();
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o666)).unwrap();
    RealSys::new().write(&p, b"{}", 0o600).unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"{}");
    assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
}

// ---------------------------------------------------------------------------------------------
// Per-user shortcuts (0.7.0) and finding the old machine-wide app
// ---------------------------------------------------------------------------------------------

mod per_user {
    use super::*;
    use crate::app_install::user::*;

    fn win_user() -> UserLayout {
        let env = |k: &str| -> Option<OsString> {
            Some(match k {
                "LOCALAPPDATA" => r"C:\Users\Sam\AppData\Local".into(),
                "APPDATA" => r"C:\Users\Sam\AppData\Roaming".into(),
                "USERPROFILE" => r"C:\Users\Sam".into(),
                _ => return None,
            })
        };
        UserLayout::resolve(true, &env, &|_| false).unwrap()
    }

    const EXE: &str = r"C:\Users\Sam\AppData\Local\Kora\bin\kora.exe";

    fn fake_with(exe: &str) -> Fake {
        let mut f = Fake { path: Some(r"C:\Windows".into()), ..Fake::default() };
        f.files.insert(PathBuf::from(exe), b"program".to_vec());
        f
    }

    #[test]
    fn windows_layout_is_per_user_and_follows_a_onedrive_desktop() {
        let ul = win_user();
        assert_eq!(ul.start_menu.as_deref(), Some(Path::new(r"C:\Users\Sam\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Kora.lnk")));
        assert_eq!(ul.desktop.as_deref(), Some(Path::new(r"C:\Users\Sam\Desktop\Kora.lnk")));
        assert_eq!(ul.program, PathBuf::from(EXE));
        let env = |k: &str| -> Option<OsString> {
            Some(match k {
                "USERPROFILE" => r"C:\Users\Sam".into(),
                "OneDrive" => r"C:\Users\Sam\OneDrive".into(),
                _ => return None,
            })
        };
        let od = UserLayout::resolve(true, &env, &|p| p == Path::new(r"C:\Users\Sam\OneDrive\Desktop")).unwrap();
        assert_eq!(od.desktop.as_deref(), Some(Path::new(r"C:\Users\Sam\OneDrive\Desktop\Kora.lnk")));
        assert_eq!(od.start_menu.as_deref(), Some(Path::new(r"C:\Users\Sam\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Kora.lnk")), "falls back to the profile");
        let none = |_: &str| -> Option<OsString> { None };
        assert!(UserLayout::resolve(true, &none, &|_| false).is_err());
    }

    #[test]
    fn the_windows_shortcuts_open_the_panel_without_asking_for_administrator_rights() {
        let ul = win_user();
        let mut f = fake_with(EXE);
        let out = install_user_with(&mut f, &ul, Path::new(EXE), None, &UserOptions::default()).unwrap();
        assert_eq!(out.changes.len(), 2, "{:?}", out.changes);
        for p in [ul.start_menu.as_ref().unwrap(), ul.desktop.as_ref().unwrap()] {
            let s = &f.shortcuts[p];
            assert_eq!(s.target, PathBuf::from(EXE));
            assert_eq!(s.args, "", "a bare `kora`, not `manager`");
            assert!(!s.run_as_admin, "never 'Run as administrator'");
            assert!(s.minimized, "no console window flashing");
        }
        assert!(!f.files.keys().any(|k| k.to_string_lossy().contains("Program Files")), "nothing machine-wide");
        // again: nothing to do
        let again = install_user_with(&mut f, &ul, Path::new(EXE), None, &UserOptions::default()).unwrap();
        assert!(again.changes.is_empty(), "{:?}", again.changes);
    }

    #[test]
    fn no_desktop_shortcut_and_dry_run() {
        let ul = win_user();
        let mut f = fake_with(EXE);
        let dry = install_user_with(&mut f, &ul, Path::new(EXE), None, &UserOptions { desktop_shortcut: true, dry_run: true }).unwrap();
        assert_eq!(dry.changes.len(), 2);
        assert!(f.shortcuts.is_empty(), "a dry run changes nothing");
        install_user_with(&mut f, &ul, Path::new(EXE), None, &UserOptions { desktop_shortcut: false, dry_run: false }).unwrap();
        assert!(f.shortcuts.contains_key(ul.start_menu.as_ref().unwrap()) && !f.shortcuts.contains_key(ul.desktop.as_ref().unwrap()));
    }

    #[test]
    fn a_program_running_from_the_old_folder_gets_a_per_user_copy_first() {
        let (ul, old) = (win_user(), win_layout());
        let mut f = fake_with(r"C:\Program Files\Kora\kora.exe");
        let out = install_user_with(&mut f, &ul, &old.exe, Some(&old.dir), &UserOptions::default()).unwrap();
        assert_eq!(out.exe, PathBuf::from(EXE));
        assert_eq!(f.files[&PathBuf::from(EXE)], b"program".to_vec(), "copied");
        assert!(f.shortcuts.values().all(|s| s.target.as_path() == Path::new(EXE)), "the shortcuts must not point at the folder that is about to go");
        assert!(out.changes.iter().any(|c| c.contains("your PATH")), "{:?}", out.changes);
        assert!(!out.changes.iter().any(|c| c.contains("machine PATH")));
        // a per-user copy that is already there is kept as it is
        let mut g = fake_with(r"C:\Program Files\Kora\kora.exe");
        g.files.insert(PathBuf::from(EXE), b"newer".to_vec());
        install_user_with(&mut g, &ul, &old.exe, Some(&old.dir), &UserOptions::default()).unwrap();
        assert_eq!(g.files[&PathBuf::from(EXE)], b"newer".to_vec());
    }

    #[cfg(unix)]
    fn linux_user(session: bool) -> UserLayout {
        let env = |k: &str| -> Option<OsString> {
            match k {
                "HOME" => Some("/home/sam".into()),
                "DISPLAY" if session => Some(":0".into()),
                _ => None,
            }
        };
        UserLayout::resolve(false, &env, &|_| false).unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn linux_launcher_starts_the_panel_without_a_terminal() {
        let ul = linux_user(true);
        let exe = "/home/sam/.local/bin/kora";
        let mut f = Fake::default();
        f.files.insert(PathBuf::from(exe), b"program".to_vec());
        let out = install_user_with(&mut f, &ul, Path::new(exe), None, &UserOptions::default()).unwrap();
        assert_eq!(out.changes.len(), 2, "logo and launcher: {:?}", out.changes);
        let text = String::from_utf8(f.files[ul.desktop_entry.as_ref().unwrap()].clone()).unwrap();
        assert!(text.contains(&format!("Exec={exe}\n")), "{text}");
        assert!(text.contains("Terminal=false") && !text.contains("manager") && text.contains(DESKTOP_MARKER), "{text}");
        assert!(ul.desktop_entry.as_ref().unwrap().starts_with("/home/sam/.local/share/applications"));
        assert!(install_user_with(&mut f, &ul, Path::new(exe), None, &UserOptions::default()).unwrap().changes.is_empty());
        // someone else's launcher is never overwritten
        let mut g = Fake::default();
        g.files.insert(ul.desktop_entry.clone().unwrap(), b"[Desktop Entry]\nName=Mine\n".to_vec());
        let out = install_user_with(&mut g, &ul, Path::new(exe), None, &UserOptions::default()).unwrap();
        assert!(out.changes.is_empty() && out.notes.iter().any(|n| n.contains("not made by Kora")), "{out:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_computer_with_no_desktop_gets_no_launcher_and_a_plain_note() {
        let ul = linux_user(false);
        let mut f = Fake::default();
        let out = install_user_with(&mut f, &ul, Path::new("/home/sam/.local/bin/kora"), None, &UserOptions::default()).unwrap();
        assert!(out.changes.is_empty() && out.notes.iter().any(|n| n.contains("no desktop")), "{out:?}");
    }

    #[test]
    fn removing_the_windows_shortcuts_takes_only_ours_and_leaves_the_program() {
        let ul = win_user();
        let mut f = fake_with(EXE);
        install_user_with(&mut f, &ul, Path::new(EXE), None, &UserOptions::default()).unwrap();
        assert!(user_status_with(&f, &ul).any());
        // a dry run lists both and changes nothing
        let dry = uninstall_user_with(&mut f, &ul, true).unwrap();
        assert_eq!(dry.len(), 2, "{dry:?}");
        assert!(dry.iter().all(|l| l.contains("shortcut") || l.contains(".lnk")), "{dry:?}");
        assert_eq!(f.shortcuts.len(), 2);
        let out = uninstall_user_with(&mut f, &ul, false).unwrap();
        assert_eq!(out.len(), 2, "{out:?}");
        assert!(f.shortcuts.is_empty());
        assert!(f.files.contains_key(&PathBuf::from(EXE)), "the program is not removed");
        assert!(!user_status_with(&f, &ul).any());
        assert!(uninstall_user_with(&mut f, &ul, false).unwrap().is_empty(), "again: nothing to do");
    }

    #[test]
    fn a_shortcut_with_our_name_that_is_not_ours_is_left_alone() {
        let ul = win_user();
        let mut f = fake_with(EXE);
        let (start, desk) = (ul.start_menu.clone().unwrap(), ul.desktop.clone().unwrap());
        // the old machine-wide app's shortcut ran `kora manager` as administrator
        let old = ShortcutSpec {
            path: start.clone(),
            target: r"C:\Program Files\Kora\kora.exe".into(),
            args: "manager".into(),
            workdir: PathBuf::new(),
            icon: String::new(),
            description: String::new(),
            run_as_admin: true,
            minimized: false,
        };
        // someone else's program
        let other = ShortcutSpec { path: desk.clone(), target: r"C:\games\other.exe".into(), args: String::new(), ..old.clone() };
        f.shortcuts.insert(start.clone(), old);
        f.shortcuts.insert(desk.clone(), other);
        let out = uninstall_user_with(&mut f, &ul, false).unwrap();
        assert_eq!(f.shortcuts.len(), 2, "both stay");
        assert_eq!(out.iter().filter(|l| l.contains("left alone")).count(), 2, "{out:?}");
    }

    #[cfg(unix)]
    #[test]
    fn removing_the_linux_launcher_takes_the_launcher_and_its_picture_only() {
        let ul = linux_user(true);
        let exe = "/home/sam/.local/bin/kora";
        let mut f = Fake::default();
        f.files.insert(PathBuf::from(exe), b"program".to_vec());
        install_user_with(&mut f, &ul, Path::new(exe), None, &UserOptions::default()).unwrap();
        assert!(user_status_with(&f, &ul).any());
        let dry = uninstall_user_with(&mut f, &ul, true).unwrap();
        assert_eq!(dry.len(), 2, "{dry:?}");
        assert!(f.files.contains_key(ul.desktop_entry.as_ref().unwrap()), "a dry run changes nothing");
        let out = uninstall_user_with(&mut f, &ul, false).unwrap();
        assert_eq!(out.len(), 2, "{out:?}");
        assert!(!f.files.contains_key(ul.desktop_entry.as_ref().unwrap()) && !f.files.contains_key(ul.icon_png.as_ref().unwrap()));
        assert!(f.files.contains_key(&PathBuf::from(exe)), "the program stays");
        assert!(uninstall_user_with(&mut f, &ul, false).unwrap().is_empty());
        // someone else's launcher and picture are never removed
        let mut g = Fake::default();
        g.files.insert(ul.desktop_entry.clone().unwrap(), b"[Desktop Entry]\nName=Mine\n".to_vec());
        g.files.insert(ul.icon_png.clone().unwrap(), b"not our picture".to_vec());
        let out = uninstall_user_with(&mut g, &ul, false).unwrap();
        assert_eq!(g.files.len(), 2, "{out:?}");
        assert!(out.iter().any(|l| l.contains("not made by Kora")), "{out:?}");
    }

    #[test]
    fn the_old_app_removal_does_not_touch_the_new_shortcuts() {
        // "Tidy up" makes the new shortcuts, then removes the old app: that must keep the new ones.
        let (ul, old) = (win_user(), win_layout());
        let (r, c) = running("0.6.5", b"old-program", r"C:\x\kora.exe");
        let mut f = machine_with(&r, c);
        install_app_with(&mut f, &old, &r, &OPTS).unwrap();
        install_user_with(&mut f, &ul, Path::new(EXE), Some(&old.dir), &UserOptions { desktop_shortcut: true, dry_run: false }).unwrap();
        uninstall_app_with(&mut f, &old, false).unwrap();
        assert!(user_status_with(&f, &ul).in_place(), "the new way in survives the old app's removal");
    }

    #[test]
    fn status_says_whether_the_way_in_is_there() {
        let ul = win_user();
        let mut f = fake_with(EXE);
        assert!(!user_status_with(&f, &ul).in_place());
        install_user_with(&mut f, &ul, Path::new(EXE), None, &UserOptions::default()).unwrap();
        let st = user_status_with(&f, &ul);
        assert_eq!((st.start_menu, st.desktop, st.in_place()), (Some(true), Some(true), true));
    }

    #[test]
    fn the_old_app_is_found_by_what_it_left_and_a_clean_machine_has_none() {
        let old = win_layout();
        let (r, c) = running("0.6.5", b"old-program", r"C:\x\kora.exe");
        let mut f = machine_with(&r, c);
        assert!(!detect_legacy(&f, &old).present(), "nothing installed");
        install_app_with(&mut f, &old, &r, &OPTS).unwrap();
        let legacy = detect_legacy(&f, &old);
        assert!(legacy.present());
        let text = legacy.items.join("\n");
        for want in ["Delete C:\\Program Files\\Kora\\kora.exe", "shortcut", "machine PATH", "Apps & features"] {
            assert!(text.contains(want), "missing {want:?} in:\n{text}");
        }
        // removing it (what the privileged half of "Tidy up" runs) leaves nothing
        uninstall_app_with(&mut f, &old, false).unwrap();
        assert!(!detect_legacy(&f, &old).present());
    }

    #[test]
    fn a_shortcut_that_is_not_ours_is_not_an_old_app() {
        let old = win_layout();
        let mut f = Fake::default();
        let p = old.start_menu.clone().unwrap();
        f.shortcuts.insert(
            p.clone(),
            ShortcutSpec {
                path: p,
                target: r"C:\games\other.exe".into(),
                args: String::new(),
                workdir: PathBuf::new(),
                icon: String::new(),
                description: String::new(),
                run_as_admin: false,
                minimized: false,
            },
        );
        assert!(!detect_legacy(&f, &old).present());
    }

    #[test]
    fn the_background_panel_is_pointed_at_the_new_program() {
        let vbs = "CreateObject(\"WScript.Shell\").Run \"\"\"C:\\Program Files\\Kora\\kora.exe\"\" panel --no-open --foreground --port 47120\", 0, False\r\n";
        let fixed = repoint_text(vbs, Path::new(r"C:\Program Files\Kora\kora.exe"), Path::new(EXE)).unwrap();
        assert!(fixed.contains(EXE) && !fixed.contains("Program Files"), "{fixed}");
        assert!(fixed.contains("panel --no-open --foreground --port 47120"));
        let loud = vbs.to_uppercase().replace("WSCRIPT.SHELL", "WScript.Shell");
        assert!(repoint_text(&loud, Path::new(r"C:\Program Files\Kora\kora.exe"), Path::new(EXE)).is_some(), "Windows paths ignore case");
        assert_eq!(repoint_text(vbs, Path::new(r"C:\Other\kora.exe"), Path::new(EXE)), None, "not ours: untouched");
        let unit = "ExecStart=\"/opt/kora/bin/kora\" panel --no-open\n";
        let fixed = repoint_text(unit, Path::new("/opt/kora/bin/kora"), Path::new("/home/sam/.local/bin/kora")).unwrap();
        assert_eq!(fixed, "ExecStart=\"/home/sam/.local/bin/kora\" panel --no-open\n");
    }
}
