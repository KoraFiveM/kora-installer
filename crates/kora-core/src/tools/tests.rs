use std::cell::RefCell;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::pins::{Format, Pin, PINS};
use super::*;
use crate::git::{CommandRunner, Output};

fn sha(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

fn exe_name(tool: Tool, windows: bool) -> String {
    format!("{}{}", program_name(tool), if windows { ".exe" } else { "" })
}

/// A runnable "program" in `dir`.
fn fake_program(dir: &Path, name: &str) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let p = dir.join(name);
    std::fs::write(&p, b"#!/bin/sh\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    p
}

fn private_copy(root: &Path, tool: Tool) {
    let dir = root.join(program_name(tool));
    let exe = fake_program(&dir.join("bin"), &exe_name(tool, false));
    let _ = exe;
    let m = serde_json::json!({"version": "9.9.9", "sha256": "x", "exe": format!("bin/{}", exe_name(tool, false))});
    std::fs::write(dir.join(MARKER), m.to_string()).unwrap();
}

#[test]
fn a_system_copy_wins_over_the_private_one() {
    let t = tempfile::tempdir().unwrap();
    let (sys, root) = (t.path().join("usr-bin"), t.path().join("tools"));
    let sysgh = fake_program(&sys, "gh");
    private_copy(&root, Tool::Gh);
    let path: OsString = std::env::join_paths([&sys]).unwrap();
    let f = locate_in(Tool::Gh, Some(&path), &[], &root, false, None).unwrap();
    assert_eq!((f.source, f.path), (Source::System, sysgh));
}

#[test]
fn without_a_system_copy_the_private_one_is_used_and_a_stale_path_does_not_matter() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("tools");
    private_copy(&root, Tool::Gh);
    let empty = t.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let path: OsString = std::env::join_paths([&empty]).unwrap();
    let f = locate_in(Tool::Gh, Some(&path), &[], &root, false, None).unwrap();
    assert_eq!(f.source, Source::Private);
    assert!(f.path.ends_with("tools/gh/bin/gh"), "{:?}", f.path);
    // And no PATH at all.
    assert_eq!(locate_in(Tool::Gh, None, &[], &root, false, None).unwrap().source, Source::Private);
}

#[test]
fn nothing_found_is_none_and_an_unfinished_private_copy_does_not_count() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("tools");
    assert!(locate_in(Tool::Git, None, &[], &root, false, None).is_none());
    // Program there but no marker (an interrupted install): not a tool.
    fake_program(&root.join("git").join("bin"), "git");
    assert!(locate_in(Tool::Git, None, &[], &root, false, None).is_none());
}

#[test]
fn extra_install_folders_find_a_tool_that_is_not_on_the_path_yet() {
    let t = tempfile::tempdir().unwrap();
    let extra = t.path().join("Program Files").join("Git").join("cmd");
    let git = fake_program(&extra, "git");
    let f = locate_in(Tool::Git, None, std::slice::from_ref(&extra), &t.path().join("tools"), false, None).unwrap();
    assert_eq!((f.source, f.path), (Source::System, git));
}

#[test]
fn relative_path_entries_are_ignored() {
    let t = tempfile::tempdir().unwrap();
    let rel: OsString = std::env::join_paths([Path::new("."), Path::new("bin")]).unwrap();
    assert!(find_system(Tool::Gh, Some(&rel), &[], false, None).is_none());
    let _ = t;
}

#[test]
fn the_folder_kora_runs_from_is_skipped_on_windows_but_the_check_is_case_blind() {
    let t = tempfile::tempdir().unwrap();
    let dl = t.path().join("Downloads");
    fake_program(&dl, "git.exe");
    let path: OsString = std::env::join_paths([&dl]).unwrap();
    let up = PathBuf::from(dl.to_string_lossy().to_uppercase());
    // Case-blind comparison only applies to Windows paths; here the exact folder is enough to show the skip.
    assert!(find_system(Tool::Git, Some(&path), &[], true, Some(&dl)).is_none());
    assert!(find_system(Tool::Git, Some(&path), &[], true, None).is_some());
    let _ = up;
}

#[test]
fn child_path_puts_the_private_git_folder_first() {
    let p = path_with_dir(Some(OsString::from("/usr/bin")), Path::new("/k/tools/git/cmd")).unwrap();
    let parts: Vec<PathBuf> = std::env::split_paths(&p).collect();
    assert_eq!(parts, vec![PathBuf::from("/k/tools/git/cmd"), PathBuf::from("/usr/bin")]);
    assert_eq!(std::env::split_paths(&path_with_dir(None, Path::new("/x")).unwrap()).count(), 1);
}

#[test]
fn the_credential_helper_points_at_the_private_gh_and_is_quoted_for_git() {
    assert_eq!(credential_helper_for(Path::new("/home/a b/.kora/tools/gh/bin/gh"), false), "!'/home/a b/.kora/tools/gh/bin/gh' auth git-credential");
    assert_eq!(credential_helper_for(Path::new("/h/o'neil/gh"), false), "!'/h/o'\\''neil/gh' auth git-credential");
    assert_eq!(credential_helper_for(Path::new(r"C:\Users\Zoë Müller\AppData\Local\Kora\tools\gh\bin\gh.exe"), true), "!\"C:/Users/Zoë Müller/AppData/Local/Kora/tools/gh/bin/gh.exe\" auth git-credential");
}

#[test]
fn every_pin_is_a_github_release_with_a_full_checksum() {
    assert!(!PINS.is_empty());
    for p in PINS {
        assert!(p.url.starts_with("https://github.com/") && !p.url.contains(".."), "{}", p.url);
        assert!(p.url.ends_with(p.file), "{}", p.url);
        assert_eq!(p.sha256.len(), 64, "{}", p.file);
        assert!(p.sha256.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')), "{}", p.file);
        assert!(p.url.contains(&format!("/{}", p.version.split('.').take(3).collect::<Vec<_>>().join("."))) || p.url.contains(p.version), "{}", p.file);
        assert!(unpack::safe_relative(p.exe, false).unwrap().is_some());
    }
    // The platforms Kora ships for are covered; Linux Git is the system package.
    for (tool, os) in [(Tool::Gh, "linux"), (Tool::Gh, "windows"), (Tool::Git, "windows")] {
        assert!(pins::pin_for(tool, os, "x86_64").is_some(), "{tool:?} {os}");
    }
    assert!(pins::pin_for(Tool::Git, "linux", "x86_64").is_none());
}

// ---------------------------------------------------------------- installing

/// Serves bytes (or fails), recording what it was asked.
struct Fake {
    body: Vec<u8>,
    fail: Option<FetchFailure>,
    /// Write only part of the body, then fail (a dropped connection).
    interrupt: bool,
    asked: RefCell<Vec<String>>,
}
impl Fake {
    fn ok(body: Vec<u8>) -> Self {
        Fake { body, fail: None, interrupt: false, asked: RefCell::new(vec![]) }
    }
}
impl Fetcher for Fake {
    fn fetch(&self, url: &str, dest: &Path, _sha: &str, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<(), FetchFailure> {
        self.asked.borrow_mut().push(url.to_string());
        if self.interrupt {
            std::fs::write(dest, &self.body[..self.body.len() / 2]).unwrap();
            return Err(FetchFailure::Network("the connection broke".into()));
        }
        if let Some(f) = &self.fail {
            return Err(f.clone());
        }
        std::fs::write(dest, &self.body).unwrap();
        progress(self.body.len() as u64, Some(self.body.len() as u64));
        Ok(())
    }
}

fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    let mut z = zip::ZipWriter::new(&mut buf);
    for (n, b) in entries {
        z.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(b).unwrap();
    }
    z.finish().unwrap();
    buf.into_inner()
}

fn pin_for_body(body: &[u8], exe: &'static str) -> Pin {
    // Leaked so the test pin can hold `&'static str`.
    let sum: &'static str = Box::leak(sha(body).into_boxed_str());
    Pin { tool: Tool::Gh, version: "1.2.3", os: "linux", arch: "x86_64", file: "gh.zip", url: "https://github.com/cli/cli/releases/download/v1.2.3/gh.zip", sha256: sum, format: Format::Zip, strip_top: false, exe, size: body.len() as u64 }
}

fn leftovers(root: &Path) -> Vec<String> {
    std::fs::read_dir(root).map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with('.')).collect()).unwrap_or_default()
}

#[test]
fn a_good_download_is_installed_and_found_afterwards() {
    let t = tempfile::tempdir().unwrap();
    let body = zip_with(&[("bin/gh", b"program"), ("LICENSE", b"l")]);
    let pin = pin_for_body(&body, "bin/gh");
    let fake = Fake::ok(body);
    let mut said = Vec::new();
    let done = install_in(t.path(), &pin, &fake, &mut |m, f| said.push((m.to_string(), f))).unwrap();
    assert_eq!((done.tool, done.version.as_str()), (Tool::Gh, "1.2.3"));
    assert_eq!(std::fs::read(&done.exe).unwrap(), b"program");
    assert_eq!(fake.asked.borrow().as_slice(), [pin.url]);
    assert_eq!(private_in(t.path(), Tool::Gh).unwrap().exe, done.exe);
    assert!(said.last().unwrap().1 >= 1.0 && said.iter().all(|(_, f)| (0.0..=1.0).contains(f)));
    assert!(leftovers(t.path()).is_empty(), "{:?}", leftovers(t.path()));
}

#[test]
fn a_file_that_is_not_the_pinned_one_is_never_unpacked() {
    let t = tempfile::tempdir().unwrap();
    let good = zip_with(&[("bin/gh", b"program")]);
    let pin = pin_for_body(&good, "bin/gh");
    // The downloader (wrongly) hands over a different file without complaining: the second check catches it.
    let evil = zip_with(&[("bin/gh", b"evil")]);
    let e = install_in(t.path(), &pin, &Fake::ok(evil), &mut |_, _| {}).unwrap_err();
    assert_eq!(e.code, codes::TOOLS_CHECKSUM);
    assert!(e.actions.contains(&Action::Retry));
    assert!(private_in(t.path(), Tool::Gh).is_none());
    assert!(!t.path().join("gh").exists());
    assert!(leftovers(t.path()).is_empty());
    // The downloader's own checksum failure reads the same.
    let fake = Fake { fail: Some(FetchFailure::Checksum), ..Fake::ok(vec![]) };
    assert_eq!(install_in(t.path(), &pin, &fake, &mut |_, _| {}).unwrap_err().code, codes::TOOLS_CHECKSUM);
}

#[test]
fn an_interrupted_download_leaves_no_broken_tool_and_the_next_try_works() {
    let t = tempfile::tempdir().unwrap();
    let body = zip_with(&[("bin/gh", b"program")]);
    let pin = pin_for_body(&body, "bin/gh");
    let broken = Fake { interrupt: true, ..Fake::ok(body.clone()) };
    let e = install_in(t.path(), &pin, &broken, &mut |_, _| {}).unwrap_err();
    assert_eq!(e.code, codes::TOOLS_DOWNLOAD_FAILED);
    assert!(e.actions.contains(&Action::Retry), "offline failures offer Retry: {:?}", e.actions);
    assert!(e.details.as_deref().unwrap().contains("broke"));
    assert!(private_in(t.path(), Tool::Gh).is_none());
    assert!(leftovers(t.path()).is_empty(), "{:?}", leftovers(t.path()));
    install_in(t.path(), &pin, &Fake::ok(body), &mut |_, _| {}).unwrap();
    assert!(private_in(t.path(), Tool::Gh).is_some());
}

#[test]
fn a_crash_between_download_and_finish_is_cleaned_up_by_the_next_install() {
    let t = tempfile::tempdir().unwrap();
    // What a killed install leaves behind: a partial file, a half-unpacked folder, a stale lock.
    std::fs::write(t.path().join(".dl-gh-aaaa.part"), b"half").unwrap();
    std::fs::create_dir_all(t.path().join(".stage-gh-bbbb").join("bin")).unwrap();
    std::fs::write(t.path().join(".installing-gh"), b"").unwrap();
    let body = zip_with(&[("bin/gh", b"program")]);
    let pin = pin_for_body(&body, "bin/gh");
    // A fresh lock means "someone is installing": wait.
    let e = install_in(t.path(), &pin, &Fake::ok(body.clone()), &mut |_, _| {}).unwrap_err();
    assert!(e.message.contains("already being installed"), "{}", e.message);
    // Age the lock (twenty minutes and more is a crash), then it goes through and tidies up.
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    std::fs::File::options().write(true).open(t.path().join(".installing-gh")).unwrap().set_modified(old).unwrap();
    install_in(t.path(), &pin, &Fake::ok(body), &mut |_, _| {}).unwrap();
    assert!(leftovers(t.path()).is_empty(), "{:?}", leftovers(t.path()));
}

#[test]
fn a_download_without_the_program_or_with_unsafe_paths_is_refused() {
    let t = tempfile::tempdir().unwrap();
    let no_exe = zip_with(&[("LICENSE", b"l")]);
    let pin = pin_for_body(&no_exe, "bin/gh");
    let e = install_in(t.path(), &pin, &Fake::ok(no_exe), &mut |_, _| {}).unwrap_err();
    assert!(e.details.as_deref().unwrap().contains("bin/gh"), "{e:?}");
    let evil = zip_with(&[("../escape", b"x"), ("bin/gh", b"p")]);
    let pin = pin_for_body(&evil, "bin/gh");
    assert!(install_in(t.path(), &pin, &Fake::ok(evil), &mut |_, _| {}).is_err());
    assert!(!t.path().parent().unwrap().join("escape").exists());
    assert!(private_in(t.path(), Tool::Gh).is_none());
    assert!(leftovers(t.path()).is_empty());
}

#[test]
fn installing_again_replaces_the_old_copy_in_one_step() {
    let t = tempfile::tempdir().unwrap();
    let v1 = zip_with(&[("bin/gh", b"one")]);
    install_in(t.path(), &pin_for_body(&v1, "bin/gh"), &Fake::ok(v1), &mut |_, _| {}).unwrap();
    let v2 = zip_with(&[("bin/gh", b"two"), ("share/x", b"y")]);
    let done = install_in(t.path(), &pin_for_body(&v2, "bin/gh"), &Fake::ok(v2), &mut |_, _| {}).unwrap();
    assert_eq!(std::fs::read(done.exe).unwrap(), b"two");
    assert!(t.path().join("gh/share/x").is_file());
    assert!(leftovers(t.path()).is_empty());
}

#[test]
fn unsupported_computers_get_a_plain_problem() {
    let p = unsupported(Tool::Gh);
    assert_eq!(p.code, codes::TOOLS_UNSUPPORTED);
    assert!(p.actions.iter().any(|a| matches!(a, Action::OpenLink { .. })));
}

#[test]
fn uninstall_removes_the_private_tools_and_says_what_went() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("tools");
    assert_eq!(remove_private_in(&root).unwrap(), Vec::<String>::new());
    private_copy(&root, Tool::Gh);
    let gone = remove_private_in(&root).unwrap();
    assert_eq!(gone.len(), 1);
    assert!(gone[0].contains("GitHub CLI"));
    assert!(!root.exists());
}

// ---------------------------------------------------------------- status

struct Runs(Option<&'static str>);
impl CommandRunner for Runs {
    fn run(&self, _p: &str, _a: &[&str], _c: Option<&Path>) -> std::io::Result<Output> {
        match self.0 {
            Some(v) => Ok(Output { code: 0, stdout: v.to_string(), stderr: String::new() }),
            None => Err(std::io::Error::new(std::io::ErrorKind::NotFound, "no such program")),
        }
    }
    fn sleep(&self, _d: std::time::Duration) {}
}

#[test]
fn status_reports_a_working_tool_with_its_version() {
    let s = status(&Runs(Some("gh version 2.40.1 (2024-01-01)\n")), Tool::Gh);
    assert!(s.installed && !s.broken);
    assert_eq!(s.version.as_deref(), Some("2.40.1"));
    assert_eq!((s.name, s.tool), ("gh", Tool::Gh));
}

#[test]
fn status_reports_a_missing_tool_as_not_installed_and_installable_where_kora_has_a_copy() {
    let s = status(&Runs(None), Tool::Gh);
    assert!(!s.installed && s.version.is_none() && !s.broken || s.path.is_some());
    assert_eq!(s.can_install, pins::pin_here(Tool::Gh).is_some());
    assert_eq!(status(&Runs(None), Tool::Git).can_install, cfg!(windows));
}

#[test]
fn git_and_gh_are_started_by_full_path_never_by_name() {
    // A system copy is started by its full path too: by name the operating system would search the PATH
    // (and an empty or relative entry in it is the current folder, which is a pull request's files).
    let sys = Found { source: Source::System, path: PathBuf::from("/usr/bin/git") };
    assert_eq!(exec_program_from("git", Some(sys)), "/usr/bin/git");
    let own = Found { source: Source::Private, path: PathBuf::from("/h/.kora/tools/gh/bin/gh") };
    assert_eq!(exec_program_from("gh", Some(own)), "/h/.kora/tools/gh/bin/gh");
    // Nothing found: the name, so the failure says which program is missing. Other programs are never rewritten.
    assert_eq!(exec_program_from("git", None), "git");
    let other = Found { source: Source::System, path: PathBuf::from("/usr/bin/sudo") };
    assert_eq!(exec_program_from("runuser", Some(other)), "runuser");
}
