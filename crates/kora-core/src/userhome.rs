//! Whose account is Kora working for, and where is that account's home folder?
//!
//! Kora often runs with `sudo` (it relaunches itself that way for services, the firewall and the
//! database). `sudo` changes `HOME` to `/root` on most distributions, so anything worked out from
//! `HOME` would land in root's private folder. Everything that needs "the user's home" asks here:
//!
//! * under `sudo` from a normal account: that account (`SUDO_USER`, checked against the passwd
//!   database; `SUDO_USER=root` is ignored),
//! * otherwise `$HOME`, else the passwd entry of the current user,
//! * Windows: `USERPROFILE` (then `HOME`), as before.
//!
//! [`default_server_dir`] is the one place that decides the suggested server folder:
//! `<home>/kora/<slug>` for a normal account (or `sudo` from one), and `/home/kora/<slug>` when
//! someone is logged in as root directly (common on rented servers). For the latter the install
//! creates a normal `kora` account that owns the folder (see [`kora_user_plan`]).

use std::path::{Path, PathBuf};

use crate::service::{account::UserStatus, CommandRunner, Plan, SystemStep};

/// The normal account Kora creates when root installs without `sudo`.
pub const KORA_USER: &str = "kora";
/// Its home folder, which holds the server folders.
pub const KORA_USER_HOME: &str = "/home/kora";

/// A passwd entry (just what Kora needs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub home: PathBuf,
}

/// The facts the decisions depend on (a seam: tests fill them in by hand).
pub trait UserEnv {
    fn var(&self, key: &str) -> Option<String>;
    /// Effective user id (0 = root). Always 1 off Unix (never root).
    fn euid(&self) -> u32;
    fn by_name(&self, name: &str) -> Option<Account>;
    fn by_uid(&self, uid: u32) -> Option<Account>;
}

/// The real process environment and passwd database.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemUserEnv;

impl UserEnv for SystemUserEnv {
    fn var(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }
    fn euid(&self) -> u32 {
        #[cfg(unix)]
        {
            // SAFETY: geteuid has no preconditions and cannot fail.
            unsafe { libc::geteuid() }
        }
        #[cfg(not(unix))]
        {
            1
        }
    }
    fn by_name(&self, name: &str) -> Option<Account> {
        lookup_name(name)
    }
    fn by_uid(&self, uid: u32) -> Option<Account> {
        lookup_uid(uid)
    }
}

/// Who Kora is working for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RealUser {
    /// Running as root through `sudo` from this normal account.
    Sudo(Account),
    /// Logged in as root itself (no `sudo` from a normal account). `home` is root's own.
    Root { home: Option<PathBuf> },
    /// A normal account running Kora directly (and everyone on Windows).
    Normal { home: Option<PathBuf> },
}

impl RealUser {
    /// The home folder of the person Kora works for (root's own when logged in as root).
    pub fn home(&self) -> Option<&Path> {
        match self {
            RealUser::Sudo(a) => Some(&a.home),
            RealUser::Root { home } | RealUser::Normal { home } => home.as_deref(),
        }
    }
}

/// A user name as `useradd` accepts it (and nothing a command line could misread).
pub fn valid_user_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 32
        && !s.starts_with('-')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        && !s.ends_with('.')
        && s != "."
        && s != ".."
}

/// A home folder Kora is willing to put things in: absolute and not the filesystem root.
fn usable_home(p: &Path) -> bool {
    p.is_absolute() && p.parent().is_some()
}

fn env_home(env: &dyn UserEnv) -> Option<PathBuf> {
    env.var("HOME")
        .filter(|h| !h.trim().is_empty())
        .map(PathBuf::from)
        .filter(|p| usable_home(p))
}

/// The normal account behind `sudo`: `SUDO_USER` when it is a valid name, not root, known to the
/// passwd database (and matches `SUDO_UID` when that is set), with a usable home folder.
pub fn sudo_account_with(env: &dyn UserEnv) -> Option<Account> {
    if env.euid() != 0 {
        return None;
    }
    let name = env.var("SUDO_USER")?;
    let name = name.as_str();
    if name == "root" || !valid_user_name(name) {
        return None;
    }
    let a = env.by_name(name)?;
    if a.uid == 0 || !usable_home(&a.home) {
        return None;
    }
    if let Some(uid) = env
        .var("SUDO_UID")
        .and_then(|u| u.trim().parse::<u32>().ok())
    {
        if uid != a.uid {
            return None;
        }
    }
    Some(a)
}

/// The account behind `sudo` in this process, if any.
pub fn sudo_account() -> Option<Account> {
    sudo_account_with(&SystemUserEnv)
}

pub fn real_user_with(env: &dyn UserEnv, windows: bool) -> RealUser {
    if windows {
        let home = ["USERPROFILE", "HOME"]
            .iter()
            .find_map(|k| env.var(k).filter(|v| !v.trim().is_empty()))
            .map(PathBuf::from);
        return RealUser::Normal { home };
    }
    let euid = env.euid();
    if euid == 0 {
        if let Some(a) = sudo_account_with(env) {
            return RealUser::Sudo(a);
        }
        return RealUser::Root {
            home: env_home(env)
                .or_else(|| env.by_uid(0).map(|a| a.home).filter(|p| usable_home(p))),
        };
    }
    RealUser::Normal {
        home: env_home(env).or_else(|| env.by_uid(euid).map(|a| a.home).filter(|p| usable_home(p))),
    }
}

/// Who Kora is working for in this process.
pub fn real_user() -> RealUser {
    real_user_with(&SystemUserEnv, cfg!(windows))
}

/// The home folder of the person Kora is working for (see the module docs).
pub fn real_user_home() -> Option<PathBuf> {
    real_user().home().map(Path::to_path_buf)
}

/// The suggested server folder for a server whose name gives `slug` (see the module docs).
/// Never relative: when no home folder can be found it is an error that says what to do.
pub fn default_server_dir_with(
    env: &dyn UserEnv,
    windows: bool,
    slug: &str,
) -> Result<PathBuf, String> {
    let slug = if slug.trim().is_empty() {
        "my-server"
    } else {
        slug.trim()
    };
    if windows {
        return Ok(PathBuf::from(format!("C:\\Kora\\{slug}")));
    }
    match real_user_with(env, false) {
        RealUser::Root { .. } => Ok(Path::new(KORA_USER_HOME).join(slug)),
        RealUser::Sudo(a) => Ok(a.home.join("kora").join(slug)),
        RealUser::Normal { home: Some(h) } => Ok(h.join("kora").join(slug)),
        RealUser::Normal { home: None } => Err(format!(
            "Kora could not find your home folder (HOME is not set and your account has none), so it cannot suggest a server folder. Choose one yourself, for example /srv/kora/{slug} (on the command line: --dir /srv/kora/{slug})."
        )),
    }
}

/// [`default_server_dir_with`] for this process.
pub fn default_server_dir(slug: &str) -> Result<PathBuf, String> {
    default_server_dir_with(&SystemUserEnv, cfg!(windows), slug)
}

/// Does installing into `dir` need the `kora` account? Only when Kora runs as root (Linux) and the
/// folder is inside `/home/kora` (the default for root, or chosen to avoid a private home folder).
pub fn kora_user_planned_with(env: &dyn UserEnv, windows: bool, dir: &Path) -> bool {
    !windows
        && env.euid() == 0
        && dir.starts_with(KORA_USER_HOME)
        && dir != Path::new(KORA_USER_HOME)
}

pub fn kora_user_planned(dir: &Path) -> bool {
    kora_user_planned_with(&SystemUserEnv, cfg!(windows), dir)
}

/// Steps that make sure the `kora` account exists and that other accounts (the server's service
/// account) can pass through its home folder. Idempotent: an existing account is kept as it is.
pub fn kora_user_plan(status: UserStatus) -> Result<Plan, String> {
    let mut steps = Vec::new();
    match status {
        UserStatus::Exists(0) => {
            return Err(format!("The account '{KORA_USER}' is the root account. Kora will not put a server in its home folder; choose another server folder."));
        }
        UserStatus::Exists(_) => {}
        UserStatus::Missing => {
            steps.push(SystemStep::run(
                format!("Create the user account '{KORA_USER}' (home folder {KORA_USER_HOME}) that owns the server folder"),
                "useradd",
                &["--create-home", "--home-dir", KORA_USER_HOME, "--shell", "/bin/bash", "--user-group", KORA_USER],
            ));
            // When the folder was already there useradd leaves its owner alone.
            steps.push(SystemStep::run(
                format!("Make '{KORA_USER}' the owner of {KORA_USER_HOME}"),
                "chown",
                &[&format!("{KORA_USER}:{KORA_USER}"), KORA_USER_HOME],
            ));
        }
    }
    steps.push(SystemStep::run(
        format!("Let the server's own account pass through {KORA_USER_HOME} (it still cannot list what is in it)"),
        "chmod",
        &["o+x", KORA_USER_HOME],
    ));
    Ok(Plan::new(steps))
}

/// Create the `kora` account if needed (through `runner`). Returns a plain sentence for the progress screen.
pub fn ensure_kora_user(runner: &dyn CommandRunner) -> Result<String, String> {
    let status =
        crate::service::account::user_status(runner, KORA_USER).map_err(|e| e.to_string())?;
    let plan = kora_user_plan(status)?;
    crate::service::apply_plan(&plan, runner).map_err(|e| e.to_string())?;
    Ok(match status {
        UserStatus::Missing => format!("Created the user account '{KORA_USER}' ({KORA_USER_HOME})"),
        UserStatus::Exists(_) => format!("Using the existing user account '{KORA_USER}'"),
    })
}

/// The folders from `dir` up to (not including) `/home/kora` that should belong to `kora`.
pub fn kora_owned_paths(dir: &Path) -> Vec<PathBuf> {
    let home = Path::new(KORA_USER_HOME);
    let mut v: Vec<PathBuf> = dir
        .ancestors()
        .take_while(|p| *p != home && p.starts_with(home))
        .map(Path::to_path_buf)
        .collect();
    v.reverse();
    v
}

/// Hand `paths` to the `kora` account (`chown`, not recursive unless `recursive`).
pub fn chown_to_kora_plan(paths: &[PathBuf], recursive: bool) -> Plan {
    let owner = format!("{KORA_USER}:{KORA_USER}");
    Plan::new(
        paths
            .iter()
            .map(|p| {
                let p = p.to_string_lossy();
                let mut args: Vec<&str> = Vec::new();
                if recursive {
                    args.push("-R");
                }
                args.extend([owner.as_str(), "--", &p]);
                SystemStep::run(
                    format!("Make '{KORA_USER}' the owner of {p}"),
                    "chown",
                    &args,
                )
            })
            .collect(),
    )
}

/// The `KORA_HOME` the elevated copy should use, decided BEFORE `sudo` (which changes `HOME`):
/// `None` when it is already set (sudo keeps it), else this user's own Kora folder.
pub fn kora_home_for_relaunch(kora_home_set: bool, user_kora_home: PathBuf) -> Option<PathBuf> {
    (!kora_home_set && user_kora_home.is_absolute()).then_some(user_kora_home)
}

// ---------------------------------------------------------------------------------------------
// passwd lookups
// ---------------------------------------------------------------------------------------------

#[cfg(unix)]
fn from_passwd(pw: &libc::passwd) -> Account {
    // SAFETY: the pointers come from a successful getpw*_r call and point into its buffer.
    let s = |p: *const libc::c_char| {
        unsafe { std::ffi::CStr::from_ptr(p) }
            .to_string_lossy()
            .into_owned()
    };
    Account {
        name: s(pw.pw_name),
        uid: pw.pw_uid,
        gid: pw.pw_gid,
        home: PathBuf::from(s(pw.pw_dir)),
    }
}

/// Look a user name up in the passwd database (files, LDAP, ... whatever the system uses).
#[cfg(unix)]
pub fn lookup_name(name: &str) -> Option<Account> {
    let c = std::ffi::CString::new(name).ok()?;
    let mut buf = vec![0u8; 16384];
    // SAFETY: zeroed passwd is a valid out-parameter; buffers are valid for the stated sizes.
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut res: *mut libc::passwd = std::ptr::null_mut();
    let rc = unsafe {
        libc::getpwnam_r(
            c.as_ptr(),
            &mut pw,
            buf.as_mut_ptr().cast(),
            buf.len(),
            &mut res,
        )
    };
    if rc != 0 || res.is_null() {
        return None;
    }
    Some(from_passwd(&pw))
}

#[cfg(unix)]
pub fn lookup_uid(uid: u32) -> Option<Account> {
    let mut buf = vec![0u8; 16384];
    // SAFETY: as above.
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut res: *mut libc::passwd = std::ptr::null_mut();
    let rc =
        unsafe { libc::getpwuid_r(uid, &mut pw, buf.as_mut_ptr().cast(), buf.len(), &mut res) };
    if rc != 0 || res.is_null() {
        return None;
    }
    Some(from_passwd(&pw))
}

#[cfg(not(unix))]
pub fn lookup_name(_name: &str) -> Option<Account> {
    None
}

#[cfg(not(unix))]
pub fn lookup_uid(_uid: u32) -> Option<Account> {
    None
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use std::collections::HashMap;

    /// Hand-made environment + passwd database.
    #[derive(Default, Clone)]
    pub struct FakeEnv {
        pub vars: HashMap<String, String>,
        pub euid: u32,
        pub accounts: Vec<Account>,
    }

    impl FakeEnv {
        pub fn new(euid: u32) -> Self {
            let mut f = FakeEnv {
                euid,
                ..Default::default()
            };
            f.accounts.push(Account {
                name: "root".into(),
                uid: 0,
                gid: 0,
                home: "/root".into(),
            });
            f.accounts.push(Account {
                name: "sam".into(),
                uid: 1000,
                gid: 1000,
                home: "/home/sam".into(),
            });
            f
        }
        pub fn var(mut self, k: &str, v: &str) -> Self {
            self.vars.insert(k.into(), v.into());
            self
        }
    }

    impl UserEnv for FakeEnv {
        fn var(&self, key: &str) -> Option<String> {
            self.vars.get(key).cloned()
        }
        fn euid(&self) -> u32 {
            self.euid
        }
        fn by_name(&self, name: &str) -> Option<Account> {
            self.accounts.iter().find(|a| a.name == name).cloned()
        }
        fn by_uid(&self, uid: u32) -> Option<Account> {
            self.accounts.iter().find(|a| a.uid == uid).cloned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::FakeEnv;
    use super::*;
    use crate::service::testing::FakeRunner;

    fn dir(env: &FakeEnv) -> Result<PathBuf, String> {
        default_server_dir_with(env, false, "my-server")
    }

    // Linux paths and sudo: only meaningful on Unix (on Windows `/home/..` is not absolute).
    #[cfg(unix)]
    #[test]
    fn a_normal_user_gets_a_folder_in_their_home() {
        let env = FakeEnv::new(1000).var("HOME", "/home/sam");
        assert_eq!(dir(&env).unwrap(), Path::new("/home/sam/kora/my-server"));
        assert_eq!(
            real_user_with(&env, false),
            RealUser::Normal {
                home: Some("/home/sam".into())
            }
        );
    }

    // Linux paths and sudo: only meaningful on Unix (on Windows `/home/..` is not absolute).
    #[cfg(unix)]
    #[test]
    fn sudo_from_a_normal_user_uses_that_users_home_not_roots() {
        let env = FakeEnv::new(0)
            .var("HOME", "/root")
            .var("SUDO_USER", "sam")
            .var("SUDO_UID", "1000");
        assert_eq!(dir(&env).unwrap(), Path::new("/home/sam/kora/my-server"));
        assert!(matches!(real_user_with(&env, false), RealUser::Sudo(a) if a.name == "sam"));
    }

    #[test]
    fn root_without_sudo_gets_the_kora_users_home() {
        let env = FakeEnv::new(0).var("HOME", "/root");
        assert_eq!(dir(&env).unwrap(), Path::new("/home/kora/my-server"));
        // sudo from root itself is still root
        let env = env.var("SUDO_USER", "root").var("SUDO_UID", "0");
        assert_eq!(dir(&env).unwrap(), Path::new("/home/kora/my-server"));
    }

    // Linux paths and sudo: only meaningful on Unix (on Windows `/home/..` is not absolute).
    #[cfg(unix)]
    #[test]
    fn without_home_the_passwd_entry_is_used_and_otherwise_a_clear_error() {
        let env = FakeEnv::new(1000);
        assert_eq!(
            dir(&env).unwrap(),
            Path::new("/home/sam/kora/my-server"),
            "HOME unset: passwd entry"
        );
        let env = FakeEnv::new(1000).var("HOME", "");
        assert_eq!(
            dir(&env).unwrap(),
            Path::new("/home/sam/kora/my-server"),
            "HOME empty"
        );
        let env = FakeEnv::new(4242).var("HOME", "relative/path");
        let e = dir(&env).unwrap_err();
        assert!(
            e.contains("home folder") && e.contains("--dir /srv/kora/my-server"),
            "{e}"
        );
        let env = FakeEnv::new(4242).var("HOME", "/");
        assert!(
            dir(&env).is_err(),
            "the filesystem root is not a home folder"
        );
    }

    // Linux paths and sudo: only meaningful on Unix (on Windows `/home/..` is not absolute).
    #[cfg(unix)]
    #[test]
    fn never_a_relative_path() {
        let cases = [
            FakeEnv::new(1000),
            FakeEnv::new(1000).var("HOME", "/home/sam"),
            FakeEnv::new(0),
            FakeEnv::new(0).var("SUDO_USER", "sam"),
            FakeEnv::new(0).var("SUDO_USER", "nobody-here"),
            FakeEnv::new(4242),
        ];
        for env in cases {
            if let Ok(p) = dir(&env) {
                assert!(p.is_absolute(), "{p:?}");
            }
        }
        assert_eq!(
            default_server_dir_with(&FakeEnv::new(1000), false, "  ").unwrap(),
            Path::new("/home/sam/kora/my-server"),
            "empty slug"
        );
        assert_eq!(
            default_server_dir_with(&FakeEnv::new(1), true, "x").unwrap(),
            Path::new("C:\\Kora\\x")
        );
    }

    // Linux paths and sudo: only meaningful on Unix (on Windows `/home/..` is not absolute).
    #[cfg(unix)]
    #[test]
    fn sudo_user_is_validated() {
        for bad in [
            "root",
            "",
            "-rf",
            "sam;id",
            "../etc",
            "sam ",
            "a/b",
            "x".repeat(33).as_str(),
            ".",
        ] {
            let env = FakeEnv::new(0).var("SUDO_USER", bad).var("HOME", "/root");
            assert_eq!(sudo_account_with(&env), None, "{bad:?}");
        }
        // unknown account, wrong uid, not root ourselves
        assert_eq!(
            sudo_account_with(&FakeEnv::new(0).var("SUDO_USER", "ghost")),
            None
        );
        assert_eq!(
            sudo_account_with(
                &FakeEnv::new(0)
                    .var("SUDO_USER", "sam")
                    .var("SUDO_UID", "1001")
            ),
            None
        );
        assert_eq!(
            sudo_account_with(&FakeEnv::new(1000).var("SUDO_USER", "sam")),
            None
        );
        // an account with a relative home is ignored
        let mut env = FakeEnv::new(0).var("SUDO_USER", "odd");
        env.accounts.push(Account {
            name: "odd".into(),
            uid: 1002,
            gid: 1002,
            home: "rel".into(),
        });
        assert_eq!(sudo_account_with(&env), None);
        assert!(sudo_account_with(&FakeEnv::new(0).var("SUDO_USER", "sam")).is_some());
        assert!(valid_user_name("first.last") && valid_user_name("svc_kora-1"));
    }

    #[test]
    fn the_kora_user_is_planned_only_for_root_and_folders_in_its_home() {
        let root = FakeEnv::new(0);
        let sam = FakeEnv::new(1000);
        assert!(kora_user_planned_with(
            &root,
            false,
            Path::new("/home/kora/my-server")
        ));
        assert!(kora_user_planned_with(
            &root,
            false,
            Path::new("/home/kora/a/b")
        ));
        assert!(!kora_user_planned_with(
            &root,
            false,
            Path::new("/home/kora")
        ));
        assert!(!kora_user_planned_with(
            &root,
            false,
            Path::new("/home/korax/s")
        ));
        assert!(!kora_user_planned_with(
            &root,
            false,
            Path::new("/srv/kora/s")
        ));
        assert!(
            !kora_user_planned_with(&sam, false, Path::new("/home/kora/my-server")),
            "not root: nothing to create"
        );
        assert!(
            !kora_user_planned_with(&root, true, Path::new("/home/kora/my-server")),
            "never on Windows"
        );
    }

    #[test]
    fn kora_user_plan_creates_once_and_never_for_uid_0() {
        let p = kora_user_plan(UserStatus::Missing).unwrap();
        let lines: Vec<String> = p.steps.iter().filter_map(|s| s.command_line()).collect();
        assert_eq!(
            lines,
            [
                "useradd --create-home --home-dir /home/kora --shell /bin/bash --user-group kora",
                "chown kora:kora /home/kora",
                "chmod o+x /home/kora"
            ]
        );
        let p = kora_user_plan(UserStatus::Exists(1001)).unwrap();
        let lines: Vec<String> = p.steps.iter().filter_map(|s| s.command_line()).collect();
        assert_eq!(
            lines,
            ["chmod o+x /home/kora"],
            "an existing account is left alone"
        );
        assert!(kora_user_plan(UserStatus::Exists(0)).is_err());
    }

    #[test]
    fn ensure_kora_user_goes_through_the_runner() {
        let r = FakeRunner::default();
        r.respond("id -u kora", 1, "");
        let said = ensure_kora_user(&r).unwrap();
        assert!(said.contains("Created"), "{said}");
        let calls = r.calls();
        assert_eq!(calls[0], "id -u kora");
        assert!(calls[1].starts_with("useradd --create-home"), "{calls:?}");
        let r = FakeRunner::default();
        r.respond("id -u kora", 0, "0\n");
        assert!(ensure_kora_user(&r).is_err(), "uid 0 is refused");
        assert_eq!(r.calls().len(), 1, "nothing is changed");
    }

    #[test]
    fn folders_inside_the_kora_home_are_handed_over() {
        assert_eq!(
            kora_owned_paths(Path::new("/home/kora/a/b")),
            [
                PathBuf::from("/home/kora/a"),
                PathBuf::from("/home/kora/a/b")
            ]
        );
        assert!(kora_owned_paths(Path::new("/srv/x")).is_empty());
        let p = chown_to_kora_plan(&[PathBuf::from("/home/kora/s")], true);
        assert_eq!(
            p.steps[0].command_line().unwrap(),
            "chown -R kora:kora -- /home/kora/s"
        );
    }

    // Linux paths and sudo: only meaningful on Unix (on Windows `/home/..` is not absolute).
    #[cfg(unix)]
    #[test]
    fn kora_home_is_pinned_before_sudo_unless_already_set() {
        assert_eq!(
            kora_home_for_relaunch(false, "/home/sam/.kora".into()),
            Some(PathBuf::from("/home/sam/.kora"))
        );
        assert_eq!(kora_home_for_relaunch(true, "/home/sam/.kora".into()), None);
        assert_eq!(
            kora_home_for_relaunch(false, ".kora".into()),
            None,
            "never a relative folder"
        );
    }
}
