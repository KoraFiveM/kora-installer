//! Resources in a backup: which can be fetched again (by reference), which must be kept in full, and
//! putting a referenced one back at its recorded commit.
//!
//! A resource is kept BY REFERENCE only when everything about it can be rebuilt from its repository:
//! a git copy, with a remote, nothing changed on this computer, nothing kept aside (stash), and no commit
//! that exists only here. Anything else is kept in full, with its git history, so restoring brings back
//! exactly what was there. Paid (Cfx.re Portal) assets and folders that are not git copies are always kept
//! in full: they cannot be downloaded again.
//!
//! Everything goes through the git module's [`CommandRunner`] so tests use a fake.

use std::path::Path;

use super::types::{ResourceBackup, StoredAs};
use crate::git::CommandRunner;
use crate::installer::registry::ResourceRecord;

/// Take the credentials out of a repository address (`https://user:token@github.com/x/y` → without them).
/// A token must never end up in an archive's manifest.
pub fn strip_userinfo(url: &str) -> String {
    for scheme in ["https://", "http://", "ssh://", "git://"] {
        if let Some(rest) = url.strip_prefix(scheme) {
            let (authority, path) = rest.split_once('/').map_or((rest, ""), |(a, p)| (a, p));
            let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
            return if path.is_empty() { format!("{scheme}{host}") } else { format!("{scheme}{host}/{path}") };
        }
    }
    url.to_string()
}

/// A repository address we are willing to clone: no leading dash (an option), no control characters or
/// spaces, a known kind of address.
pub fn valid_repository(url: &str) -> bool {
    !url.is_empty()
        && url.len() < 500
        && !url.starts_with('-')
        && !url.chars().any(|c| c.is_control() || c.is_whitespace() || c == '"' || c == '\'' || c == '`')
        && (url.starts_with("https://") || url.starts_with("http://") || url.starts_with("ssh://") || url.starts_with("git://") || url.starts_with("git@"))
}

pub fn valid_commit(c: &str) -> bool {
    (7..=64).contains(&c.len()) && c.bytes().all(|b| b.is_ascii_hexdigit())
}

fn run(git: &dyn CommandRunner, cwd: &Path, args: &[&str]) -> Option<String> {
    let o = git.run("git", args, Some(cwd)).ok()?;
    o.ok().then(|| o.stdout.trim().to_string())
}

/// How to keep one recorded resource. `None`: its folder is not on disk (nothing to keep).
pub fn classify(git: &dyn CommandRunner, resources_dir: &Path, rec: &ResourceRecord, everything: bool) -> Option<ResourceBackup> {
    let folder = resources_dir.join(&rec.path);
    if !folder.is_dir() {
        return None;
    }
    let full = |why: &str| ResourceBackup {
        id: rec.id.clone(),
        path: rec.path.clone(),
        stored: StoredAs::Full,
        repository: strip_userinfo(&rec.repository),
        branch: rec.branch.clone(),
        commit: rec.commit.clone(),
        why_full: Some(why.to_string()),
        bytes: 0,
    };
    if rec.is_portal() {
        return Some(full("a paid asset: it cannot be downloaded again"));
    }
    if !folder.join(".git").exists() {
        return Some(full("not a git copy, so it cannot be fetched again"));
    }
    if everything {
        return Some(full("you chose to keep everything in full"));
    }
    let Some(head) = run(git, &folder, &["rev-parse", "HEAD"]) else {
        return Some(full("its git history could not be read"));
    };
    match run(git, &folder, &["status", "--porcelain"]) {
        None => return Some(full("its git state could not be read")),
        Some(s) if !s.is_empty() => return Some(full("it has changes made on this computer")),
        Some(_) => {}
    }
    if run(git, &folder, &["stash", "list"]).is_none_or(|s| !s.is_empty()) {
        return Some(full("it has changes kept aside (stashed)"));
    }
    let Some(url) = run(git, &folder, &["remote", "get-url", "origin"]).filter(|u| !u.is_empty()) else {
        return Some(full("it has no remote repository to fetch from"));
    };
    let repo = strip_userinfo(&url);
    if !valid_repository(&repo) {
        return Some(full("its repository address is not one Kora can fetch from"));
    }
    match run(git, &folder, &["rev-list", "--count", "HEAD", "--not", "--remotes"]).and_then(|n| n.parse::<u64>().ok()) {
        Some(0) => {}
        Some(_) => return Some(full("it has commits that exist only on this computer")),
        None => return Some(full("its commits could not be compared with the remote")),
    }
    let branch = run(git, &folder, &["rev-parse", "--abbrev-ref", "HEAD"]).filter(|b| b != "HEAD").unwrap_or_default();
    Some(ResourceBackup { id: rec.id.clone(), path: rec.path.clone(), stored: StoredAs::Reference, repository: repo, branch, commit: Some(head), why_full: None, bytes: 0 })
}

/// Put a referenced resource back at its recorded commit: clone it when the folder is missing, fetch when
/// the commit is not there yet, then check it out (on its branch when it was on one). Returns a plain
/// sentence about what was done.
pub fn ensure_at_commit(git: &dyn CommandRunner, resources_dir: &Path, rb: &ResourceBackup) -> Result<String, String> {
    let Some(commit) = rb.commit.as_deref().filter(|c| valid_commit(c)) else {
        return Err(format!("{}: the backup does not name a commit", rb.id));
    };
    if !valid_repository(&rb.repository) {
        return Err(format!("{}: the repository address in the backup is not one Kora will use", rb.id));
    }
    if !rb.branch.is_empty() && !crate::git::branches::valid_branch(&rb.branch) {
        return Err(format!("{}: the branch name in the backup is not valid", rb.id));
    }
    let folder = crate::fsutil::join_inside(resources_dir, &rb.path).ok_or_else(|| format!("{}: the folder name in the backup is not valid", rb.id))?;
    let short = &commit[..commit.len().min(8)];
    if !folder.exists() {
        if let Some(parent) = folder.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: cannot create its folder ({e})", rb.id))?;
        }
        let dest = folder.to_string_lossy().into_owned();
        let o = git.run("git", &["clone", "--", &rb.repository, &dest], None).map_err(|e| format!("{}: git could not run ({e})", rb.id))?;
        if !o.ok() {
            let _ = std::fs::remove_dir_all(&folder);
            return Err(format!("{}: could not download {} ({})", rb.id, rb.repository, first_line(&o.stderr)));
        }
    } else if !folder.join(".git").exists() {
        return Err(format!("{}: a folder with that name is already there and is not a git copy, so it was left alone", rb.id));
    }
    let have = |c: &str| run(git, &folder, &["cat-file", "-e", &format!("{c}^{{commit}}")]).is_some();
    if !have(commit) {
        let o = git.run("git", &["fetch", "origin"], Some(&folder)).map_err(|e| format!("{}: git could not run ({e})", rb.id))?;
        if !o.ok() && !have(commit) {
            return Err(format!("{}: could not fetch from {} ({})", rb.id, rb.repository, first_line(&o.stderr)));
        }
        if !have(commit) {
            return Err(format!("{}: the version {short} is no longer on {}, so it could not be brought back", rb.id, rb.repository));
        }
    }
    let ok = if rb.branch.is_empty() { git.run("git", &["checkout", "-f", "--detach", commit], Some(&folder)) } else { git.run("git", &["checkout", "-f", "-B", &rb.branch, commit], Some(&folder)) }
        .map_err(|e| format!("{}: git could not run ({e})", rb.id))?;
    if !ok.ok() {
        return Err(format!("{}: could not switch to version {short} ({})", rb.id, first_line(&ok.stderr)));
    }
    Ok(format!("{} is back at version {short}", rb.id))
}

fn first_line(s: &str) -> String {
    s.lines().find(|l| !l.trim().is_empty()).unwrap_or("no details").trim().chars().take(200).collect()
}

#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use crate::git::Output;
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// A git that answers from a table keyed by the argument list, and records what ran.
    #[derive(Default)]
    pub struct FakeGit {
        pub answers: RefCell<HashMap<String, Output>>,
        pub log: RefCell<Vec<String>>,
        pub clone_makes_git_dir: bool,
    }

    impl FakeGit {
        pub fn say(&self, args: &str, out: &str) {
            self.answers.borrow_mut().insert(args.to_string(), Output { code: 0, stdout: out.to_string(), stderr: String::new() });
        }
        pub fn fail(&self, args: &str, err: &str) {
            self.answers.borrow_mut().insert(args.to_string(), Output { code: 1, stdout: String::new(), stderr: err.to_string() });
        }
    }

    impl CommandRunner for FakeGit {
        fn run(&self, _p: &str, args: &[&str], cwd: Option<&Path>) -> std::io::Result<Output> {
            let key = args.join(" ");
            self.log.borrow_mut().push(key.clone());
            if args[0] == "clone" && self.clone_makes_git_dir {
                let dest = Path::new(args[args.len() - 1]);
                std::fs::create_dir_all(dest.join(".git"))?;
                return Ok(Output::default());
            }
            // An answer for one folder (`core:status --porcelain`) wins over one for every folder.
            let own = cwd.and_then(|c| c.file_name()).map(|n| format!("{}:{key}", n.to_string_lossy()));
            if let Some(o) = own.and_then(|k| self.answers.borrow().get(&k).cloned()) {
                return Ok(o);
            }
            Ok(self.answers.borrow().get(&key).cloned().unwrap_or(Output { code: 1, stdout: String::new(), stderr: format!("fake git: no answer for `{key}`") }))
        }
        fn sleep(&self, _d: std::time::Duration) {}
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeGit;
    use super::*;

    fn rec(path: &str) -> ResourceRecord {
        ResourceRecord { id: path.into(), path: path.into(), repository: "https://github.com/KoraFiveM/x".into(), branch: "main".into(), commit: None, ..Default::default() }
    }

    fn clean_git() -> FakeGit {
        let g = FakeGit::default();
        g.say("rev-parse HEAD", "abcdef1234567890abcdef1234567890abcdef12");
        g.say("status --porcelain", "");
        g.say("stash list", "");
        g.say("remote get-url origin", "https://ghp_SECRET@github.com/KoraFiveM/x.git");
        g.say("rev-list --count HEAD --not --remotes", "0");
        g.say("rev-parse --abbrev-ref HEAD", "main");
        g
    }

    fn with_folder(name: &str, git_dir: bool) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join(name)).unwrap();
        if git_dir {
            std::fs::create_dir_all(d.path().join(name).join(".git")).unwrap();
        }
        d
    }

    #[test]
    fn a_clean_pushed_clone_is_kept_by_reference_without_its_token() {
        let d = with_folder("core", true);
        let rb = classify(&clean_git(), d.path(), &rec("core"), false).unwrap();
        assert_eq!(rb.stored, StoredAs::Reference);
        assert_eq!(rb.repository, "https://github.com/KoraFiveM/x.git");
        assert_eq!(rb.branch, "main");
        assert_eq!(rb.commit.as_deref(), Some("abcdef1234567890abcdef1234567890abcdef12"));
        assert!(!serde_json::to_string(&rb).unwrap().contains("SECRET"));
    }

    #[test]
    fn anything_that_cannot_be_fetched_again_is_kept_in_full() {
        let d = with_folder("core", true);
        let why = |g: FakeGit| classify(&g, d.path(), &rec("core"), false).unwrap().why_full.unwrap();
        let g = clean_git();
        g.say("status --porcelain", " M fxmanifest.lua");
        assert!(why(g).contains("changes made on this computer"));
        let g = clean_git();
        g.say("stash list", "stash@{0}: WIP");
        assert!(why(g).contains("kept aside"));
        let g = clean_git();
        g.say("rev-list --count HEAD --not --remotes", "2");
        assert!(why(g).contains("only on this computer"));
        let g = clean_git();
        g.fail("remote get-url origin", "no remote");
        assert!(why(g).contains("no remote"));
        // git itself unusable (e.g. "dubious ownership"): full, never "by reference" on a guess
        assert!(why(FakeGit::default()).contains("could not be read"));
        // chosen
        assert!(classify(&clean_git(), d.path(), &rec("core"), true).unwrap().why_full.unwrap().contains("chose"));
    }

    #[test]
    fn paid_assets_and_plain_folders_are_always_kept_in_full() {
        let d = with_folder("asset", true);
        let mut r = rec("asset");
        r.source = Some(crate::paidassets::SOURCE.into());
        assert!(classify(&clean_git(), d.path(), &r, false).unwrap().why_full.unwrap().contains("paid asset"));
        let d = with_folder("plain", false);
        assert!(classify(&clean_git(), d.path(), &rec("plain"), false).unwrap().why_full.unwrap().contains("not a git copy"));
        assert!(classify(&clean_git(), d.path(), &rec("gone"), false).is_none(), "a folder that is not there is nothing to keep");
    }

    fn reference() -> ResourceBackup {
        ResourceBackup {
            id: "core".into(),
            path: "core".into(),
            stored: StoredAs::Reference,
            repository: "https://github.com/KoraFiveM/x.git".into(),
            branch: "main".into(),
            commit: Some("abcdef1234567890".into()),
            why_full: None,
            bytes: 0,
        }
    }

    #[test]
    fn a_missing_resource_is_cloned_and_put_on_its_commit() {
        let d = tempfile::tempdir().unwrap();
        let g = FakeGit { clone_makes_git_dir: true, ..Default::default() };
        g.say("cat-file -e abcdef1234567890^{commit}", "");
        g.say("checkout -f -B main abcdef1234567890", "");
        let msg = ensure_at_commit(&g, d.path(), &reference()).unwrap();
        assert!(msg.contains("back at version abcdef12"));
        assert_eq!(g.log.borrow()[0].split(' ').next(), Some("clone"));
        assert!(g.log.borrow().iter().any(|l| l.starts_with("checkout -f -B main")));
    }

    #[test]
    fn an_existing_resource_is_fetched_only_when_the_commit_is_missing() {
        let d = with_folder("core", true);
        let g = FakeGit::default();
        g.fail("cat-file -e abcdef1234567890^{commit}", "missing");
        g.say("fetch origin", "");
        // after the fetch the commit is still not there: a clear message, nothing checked out
        let e = ensure_at_commit(&g, d.path(), &reference()).unwrap_err();
        assert!(e.contains("no longer on"), "{e}");
        assert!(!g.log.borrow().iter().any(|l| l.starts_with("checkout")));
    }

    #[test]
    fn hostile_values_in_a_backup_are_not_passed_to_git() {
        let d = tempfile::tempdir().unwrap();
        let g = FakeGit::default();
        for bad in [
            ResourceBackup { repository: "--upload-pack=evil".into(), ..reference() },
            ResourceBackup { repository: "file:///etc".into(), ..reference() },
            ResourceBackup { branch: "-x".into(), ..reference() },
            ResourceBackup { commit: Some("--force".into()), ..reference() },
            ResourceBackup { path: "../outside".into(), ..reference() },
        ] {
            assert!(ensure_at_commit(&g, d.path(), &bad).is_err());
        }
        assert!(g.log.borrow().is_empty(), "git was never run: {:?}", g.log.borrow());
    }

    #[test]
    fn credentials_are_stripped_from_addresses() {
        assert_eq!(strip_userinfo("https://user:tok@github.com/a/b.git"), "https://github.com/a/b.git");
        assert_eq!(strip_userinfo("https://github.com/a/b"), "https://github.com/a/b");
        assert_eq!(strip_userinfo("git@github.com:a/b.git"), "git@github.com:a/b.git");
    }
}
