//! Finding a container engine and saying, in plain words, whether it can run a server.
//!
//! [`check`] is the System check line and the install wizard's gate. For each of Podman and Docker that is
//! installed it asks `info`, then proves the engine can really run a container with a smoke run of the pinned
//! base image (`run --rm --init --cap-drop ALL <alpine> true`: pulls it the first time, and uses the same flags
//! as the real container, so `--init` and dropped capabilities are tested too). Failures are classified from the
//! engine's own error text into guided problems with the command that fixes them:
//!
//! * no subuid/subgid ranges (rootless needs them, an administrator adds them),
//! * cgroup v1 (rootless needs v2),
//! * lingering off (rootless containers die at logout, and do not start at boot: `loginctl enable-linger`),
//! * the Docker socket refusing this account (`docker` group membership, which is root-equivalent: said plainly),
//! * SELinux enforcing (not a failure: the folder is mounted with a private label, `:Z`).
//!
//! Rootless Podman is preferred when present: it needs no administrator rights at all.
//!
//! Facts about the machine that are not engine output (`/etc/subuid`, lingering, the user) come in through
//! [`Facts`], so the logic is tested without a machine.

use serde::{Deserialize, Serialize};

use super::engine::{self, Engine, EngineError, EngineKind, EngineRunner, Info};
use super::pins;
use crate::problem::{Action, Problem};

/// Machine facts the check needs besides the engine's own answers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    /// The account's name and numeric id.
    pub user: String,
    pub uid: u32,
    /// Contents of `/etc/subuid` and `/etc/subgid` (`None`: unreadable).
    pub subuid: Option<String>,
    pub subgid: Option<String>,
    /// `loginctl show-user <uid> -p Linger` said `yes` / `no` (`None`: could not be asked).
    pub linger: Option<bool>,
}

impl Facts {
    #[cfg(unix)]
    pub fn system(runner: &dyn crate::service::CommandRunner) -> Facts {
        // SAFETY: plain getuid(2).
        let uid = unsafe { libc::getuid() };
        let user = crate::userhome::lookup_uid(uid).map(|a| a.name).unwrap_or_default();
        let linger = runner.run("loginctl", &["show-user".into(), uid.to_string(), "-p".into(), "Linger".into()]).ok().and_then(|o| parse_linger(&o.stdout));
        Facts { user, uid, subuid: std::fs::read_to_string("/etc/subuid").ok(), subgid: std::fs::read_to_string("/etc/subgid").ok(), linger }
    }

    #[cfg(not(unix))]
    pub fn system(_runner: &dyn crate::service::CommandRunner) -> Facts {
        Facts::default()
    }
}

/// `Linger=yes` → `Some(true)`.
pub fn parse_linger(out: &str) -> Option<bool> {
    out.lines().find_map(|l| l.trim().strip_prefix("Linger=")).map(|v| v.trim() == "yes")
}

/// Does `/etc/subuid`-style `text` give `user` (by name or numeric id) at least one range of 65536 ids?
pub fn has_subid_range(text: &str, user: &str, uid: u32) -> bool {
    text.lines()
        .filter_map(|l| {
            let mut p = l.trim().split(':');
            let (who, _start, count) = (p.next()?, p.next()?, p.next()?);
            (who == user || who == uid.to_string()).then(|| count.trim().parse::<u64>().unwrap_or(0))
        })
        .any(|c| c >= 65536)
}

/// What the System check says about one engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct EngineStatus {
    pub kind: EngineKind,
    pub version: String,
    pub rootless: bool,
    /// A real container ran (the smoke test passed).
    pub can_run: bool,
    /// One plain line for the System check.
    pub summary: String,
    /// What stops it, with the fix. `None` when it works.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub problem: Option<Problem>,
    /// Things to know that do not stop it (SELinux labelling, lingering, firewall behaviour, root-equivalence).
    #[serde(default)]
    pub notes: Vec<String>,
    /// The folder needs a private SELinux label on its mount.
    #[serde(default)]
    pub selinux: bool,
}

/// The whole answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ContainerCheck {
    /// Some engine can run a container: container mode may be offered.
    pub available: bool,
    /// The one Kora would use (rootless Podman first, then rootless Docker, Podman, Docker).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub chosen: Option<EngineKind>,
    pub engines: Vec<EngineStatus>,
    /// One line for the System check ("Podman 4.9.3, rootless: ready" / "No container engine found").
    pub summary: String,
}

impl ContainerCheck {
    /// The engine to use: the requested one when it works, else the best.
    pub fn engine_for(&self, wanted: Option<EngineKind>) -> Option<Engine> {
        let pick = |s: &EngineStatus| Engine::new(s.kind, s.rootless);
        if let Some(k) = wanted {
            return self.engines.iter().find(|s| s.kind == k && s.can_run).map(pick);
        }
        self.chosen.and_then(|k| self.engines.iter().find(|s| s.kind == k && s.can_run)).map(pick)
    }
}

/// Preference: rootless Podman, rootless Docker, then Podman, then Docker (any rootless one needs no
/// administrator rights; Podman before Docker because it needs no daemon).
fn rank(s: &EngineStatus) -> u8 {
    match (s.kind, s.rootless) {
        (EngineKind::Podman, true) => 0,
        (EngineKind::Docker, true) => 1,
        (EngineKind::Podman, false) => 2,
        (EngineKind::Docker, false) => 3,
    }
}

fn user_action_usermod(user: &str) -> Action {
    Action::show_command(&["usermod", "--add-subuids", "100000-165535", "--add-subgids", "100000-165535", user], true)
}

/// A guided problem from a failed engine command. `what` completes "Kora could not ..." (`create the server's container`).
pub fn problem_for_failure(engine: &Engine, what: &str, e: &EngineError) -> Problem {
    let text = e.stderr().to_ascii_lowercase();
    let details = e.to_string();
    let prog = engine.program();
    if let EngineError::Spawn(p, why) = e {
        return Problem::new(super::codes::NO_ENGINE, format!("Kora could not run {p} to {what}. Is it installed, and on the path of the account Kora runs as?")).with_details(why.clone());
    }
    if text.contains("permission denied") && (text.contains("docker.sock") || text.contains("docker daemon")) {
        return docker_socket_problem(&details);
    }
    if text.contains("newuidmap")
        || text.contains("no subuid")
        || text.contains("no subgid")
        || text.contains("not enough ids available")
        || text.contains("cannot set up namespace") && text.contains("uid")
    {
        return no_subuid_problem(&details, "");
    }
    if text.contains("address already in use")
        || text.contains("port is already allocated")
        || text.contains("bind: address already in use")
        || text.contains("already in use by") && text.contains("port")
    {
        return Problem::new(super::codes::START_FAILED, "A port this server needs is already used by another program. Change the server's ports in the panel's Ports tab, or stop the other program.")
            .with_details(details);
    }
    if text.contains("name is already in use") || text.contains("already in use by container") || text.contains("the container name") && text.contains("is already in use") {
        return Problem::new(
            super::codes::NAME_TAKEN,
            format!("A container with this server's name already exists and is not Kora's. Rename the server, or remove that container yourself with `{prog} rm`."),
        )
        .with_details(details);
    }
    if text.contains("cgroup") && (text.contains("v1") || text.contains("controller") || text.contains("delegat")) {
        return cgroup_problem(&details);
    }
    Problem::new(super::codes::START_FAILED, format!("Kora could not {what}.")).with_details(details).with_action(Action::CopyDetails)
}

fn docker_socket_problem(details: &str) -> Problem {
    Problem::new(
        super::codes::SOCKET_PERMISSION,
        "Docker is installed, but this account is not allowed to use it. Adding the account to the 'docker' group allows it, but be aware that membership of that group is the same as having root on this computer (a member can start a container that sees every file). Rootless Podman needs no such trust.",
    )
    .with_details(details.to_string())
    .with_action(Action::show_command(&["usermod", "-aG", "docker", "<your user name>"], true))
}

fn no_subuid_problem(details: &str, user: &str) -> Problem {
    let who = if user.is_empty() { "<your user name>" } else { user };
    Problem::new(
        super::codes::NO_SUBUID,
        "Rootless containers need a block of extra user ids for this account (called subuid and subgid), and this account has none. An administrator can add them with the command shown, after which you sign out and in again.",
    )
    .with_details(details.to_string())
    .with_action(user_action_usermod(who))
}

fn cgroup_problem(details: &str) -> Problem {
    Problem::new(
        super::codes::CGROUP_V1,
        "This computer uses the older 'cgroup v1' resource controls, which rootless containers cannot use. Most current systems use v2; this one needs to be switched (boot with `systemd.unified_cgroup_hierarchy=1`), or use an engine that runs as root.",
    )
    .with_details(details.to_string())
}

fn linger_problem(user: &str) -> Problem {
    Problem::new(
        super::codes::NOT_LINGERING,
        "Containers run by your account stop when you sign out, and do not start again when the computer restarts, unless 'lingering' is turned on for the account. It takes one command.",
    )
    .with_action(Action::show_command(&["loginctl", "enable-linger", if user.is_empty() { "<your user name>" } else { user }], true))
}

/// A failed image pull in plain words.
pub fn pull_problem(engine: &Engine, image: &str, stderr: &str) -> Problem {
    let t = stderr.to_ascii_lowercase();
    let why = if t.contains("no such host") || t.contains("timeout") || t.contains("connection refused") || t.contains("dial tcp") || t.contains("network is unreachable") {
        "This computer could not reach the image registry (docker.io). Check the internet connection, then try again."
    } else if t.contains("toomanyrequests") || t.contains("rate limit") {
        "Docker Hub refused the download because of its rate limit. Wait a while and try again, or sign the engine in to Docker Hub."
    } else if t.contains("short-name") || t.contains("unqualified") {
        "The engine refused the image's name. Kora uses fully qualified names; check the engine's registries configuration."
    } else {
        "The engine could not download the container image."
    };
    let mut p = Problem::new(super::codes::PULL_FAILED, why).with_details(format!("{} pull {image}: {stderr}", engine.program())).with_action(Action::Retry);
    if t.contains("permission denied") && t.contains("docker.sock") {
        p = docker_socket_problem(stderr);
    }
    p
}

/// Ask one engine about itself and try a real container.
pub fn check_engine(runner: &dyn EngineRunner, kind: EngineKind, facts: &Facts) -> Option<EngineStatus> {
    check_engine_with(runner, kind, facts, true)
}

/// [`check_engine`]; `smoke: false` skips the test container (the answer is then "nothing found that would stop it").
pub fn check_engine_with(runner: &dyn EngineRunner, kind: EngineKind, facts: &Facts, smoke: bool) -> Option<EngineStatus> {
    let prog = kind.program();
    let probe = Engine::new(kind, false);
    let o = match runner.run(prog, &engine::info_args(kind)) {
        Ok(o) => o,
        // Not installed (or not runnable at all): not an engine.
        Err(_) => return None,
    };
    if !o.success() {
        let e = EngineError::Failed { command: format!("{prog} info"), code: o.code, stderr: engine::tidy(&o.stderr) };
        let problem = problem_for_failure(&probe, "ask the engine about itself", &e);
        return Some(EngineStatus {
            kind,
            version: String::new(),
            rootless: false,
            can_run: false,
            summary: format!("{} is installed but did not answer: {}", kind.label(), first_words(&problem.message)),
            problem: Some(problem),
            notes: vec![],
            selinux: false,
        });
    }
    let info: Info = engine::parse_info(kind, &o.stdout).unwrap_or_default();
    let engine = Engine::new(kind, info.rootless);
    let mut notes = Vec::new();
    let mut problem: Option<Problem> = None;

    if info.rootless {
        if info.cgroup_version == 1 {
            problem = Some(cgroup_problem(""));
        }
        let ranges_known = facts.subuid.is_some() && facts.subgid.is_some();
        let ok = |t: &Option<String>| t.as_deref().is_some_and(|t| has_subid_range(t, &facts.user, facts.uid));
        if problem.is_none() && ranges_known && !(ok(&facts.subuid) && ok(&facts.subgid)) {
            problem = Some(no_subuid_problem("/etc/subuid or /etc/subgid has no range of 65536 ids for this account", &facts.user));
        }
    } else if kind == EngineKind::Docker {
        notes.push("Docker runs containers as root and Kora's account may use it: membership of the 'docker' group is the same as root access to this computer.".to_string());
    }
    if engine.bypasses_host_firewall() {
        notes.push(format!(
            "{} publishes ports with its own firewall rules, which skip ufw and firewalld: a port a server publishes is open to the network even if your firewall says it is closed.",
            kind.label()
        ));
    }
    if info.selinux {
        notes.push("SELinux is enforcing: the server folder is shared with the container using a private label (:Z).".to_string());
    }
    if info.rootless && facts.linger == Some(false) {
        notes.push("Lingering is off for this account: containers stop when you sign out and do not start at boot.".to_string());
    }

    // A real container, with the flags the real one uses.
    let mut can_run = false;
    if problem.is_none() && !smoke {
        can_run = true;
    } else if problem.is_none() {
        let smoke = vec!["run".into(), "--rm".into(), "--init".into(), "--cap-drop".into(), "ALL".into(), pins::BASE.reference(), "true".into()];
        match runner.run(prog, &smoke) {
            Ok(r) if r.success() => can_run = true,
            Ok(r) => {
                let e = EngineError::Failed { command: engine::command_text(prog, &smoke), code: r.code, stderr: engine::tidy(&r.stderr) };
                let mut p = problem_for_failure(&engine, "run a test container", &e);
                if p.code == super::codes::START_FAILED {
                    p = Problem::new(super::codes::ENGINE_CANNOT_RUN, format!("{} is installed but could not run a test container.", kind.label())).with_details(e.to_string());
                    if engine::tidy(&r.stderr).to_ascii_lowercase().contains("catatonit") || r.stderr.to_ascii_lowercase().contains("init") {
                        p.message.push_str(" It may be missing its 'init' helper (catatonit); install your distribution's `catatonit` package.");
                    }
                }
                problem = Some(p);
            }
            Err(e) => problem = Some(problem_for_failure(&engine, "run a test container", &EngineError::Spawn(prog.into(), e.to_string()))),
        }
    }
    let version = if info.version.is_empty() { String::new() } else { info.version.clone() };
    let summary = match (&problem, can_run) {
        (None, true) => format!("{} {}{}: ready to run servers", kind.label(), version, if info.rootless { ", rootless" } else { "" }).replace("  ", " "),
        (Some(p), _) => format!("{} {}{}: {}", kind.label(), version, if info.rootless { ", rootless" } else { "" }, first_words(&p.message)).replace("  ", " "),
        (None, false) => format!("{} {}: not tested", kind.label(), version),
    };
    // Lingering is a warning with a fix, not a reason to refuse: shown as the problem only when nothing else is wrong.
    if problem.is_none() && info.rootless && facts.linger == Some(false) {
        problem = Some(linger_problem(&facts.user));
    }
    Some(EngineStatus { kind, version, rootless: info.rootless, can_run, summary, problem, notes, selinux: info.selinux })
}

fn first_words(m: &str) -> String {
    m.split(['.', '\n']).next().unwrap_or(m).trim().to_string()
}

/// Look at both engines. `available` only when one of them ran a real container.
pub fn check(runner: &dyn EngineRunner, facts: &Facts) -> ContainerCheck {
    check_with(runner, facts, true)
}

/// [`check`] without the test container when `smoke` is false (a quick look for a page that only offers a choice).
pub fn check_with(runner: &dyn EngineRunner, facts: &Facts, smoke: bool) -> ContainerCheck {
    let mut engines: Vec<EngineStatus> = [EngineKind::Podman, EngineKind::Docker].into_iter().filter_map(|k| check_engine_with(runner, k, facts, smoke)).collect();
    engines.sort_by_key(|s| (!s.can_run, rank(s)));
    let chosen = engines.iter().find(|s| s.can_run).map(|s| s.kind);
    let summary = match (chosen, engines.first()) {
        (Some(_), Some(s)) => s.summary.clone(),
        (None, Some(s)) => s.summary.clone(),
        _ => "No container engine (Podman or Docker) found".to_string(),
    };
    ContainerCheck { available: chosen.is_some(), chosen, engines, summary }
}

/// A quick look (no test container): the label of the engine container mode would use (`podman, rootless`),
/// or `None`. Unit tests of this crate never look at the machine.
pub fn quick_label() -> Option<String> {
    if cfg!(test) || !cfg!(target_os = "linux") {
        return None;
    }
    let r = crate::service::SystemRunner;
    let c = check_with(&r, &Facts::system(&r), false);
    c.engine_for(None).map(|e| e.label())
}

/// The real check.
pub fn check_system() -> ContainerCheck {
    let r = crate::service::SystemRunner;
    check(&r, &Facts::system(&r))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::testing::FakeRunner;

    const PODMAN_ROOTLESS: &str = r#"{"host":{"cgroupVersion":"v2","security":{"rootless":true}},"version":{"Version":"4.9.3"}}"#;
    const DOCKER_ROOT: &str = r#"{"ServerVersion":"27.1.1","CgroupVersion":"2","SecurityOptions":["name=seccomp,profile=builtin"]}"#;

    /// Docker answers like a daemon that is not running (so only Podman is the engine under test).
    fn no_docker(r: &FakeRunner) {
        r.respond("docker info", 1, "");
        r.responses.borrow_mut().last_mut().unwrap().1.stderr = "Cannot connect to the Docker daemon".into();
    }

    fn facts() -> Facts {
        Facts { user: "alex".into(), uid: 1000, subuid: Some("alex:100000:65536\n".into()), subgid: Some("alex:100000:65536\n".into()), linger: Some(true) }
    }

    #[test]
    fn subid_ranges_need_65536_ids_for_this_account() {
        assert!(has_subid_range("alex:100000:65536\n", "alex", 1000));
        assert!(has_subid_range("1000:100000:65536\n", "alex", 1000), "by numeric id");
        assert!(!has_subid_range("alex:100000:1000\n", "alex", 1000));
        assert!(!has_subid_range("bob:100000:65536\n", "alex", 1000));
        assert!(!has_subid_range("", "alex", 1000));
        assert_eq!(parse_linger("Linger=yes\n"), Some(true));
        assert_eq!(parse_linger("Linger=no\n"), Some(false));
        assert_eq!(parse_linger(""), None);
    }

    #[test]
    fn rootless_podman_that_runs_a_container_is_ready_and_preferred_over_docker() {
        let r = FakeRunner::default();
        r.respond("podman info", 0, PODMAN_ROOTLESS);
        r.respond("docker info", 0, DOCKER_ROOT);
        let c = check(&r, &facts());
        assert!(c.available);
        assert_eq!(c.chosen, Some(EngineKind::Podman));
        assert_eq!(c.engines.len(), 2);
        assert!(c.summary.contains("Podman 4.9.3, rootless") && c.summary.contains("ready"), "{}", c.summary);
        let smoke = format!("podman run --rm --init --cap-drop ALL {} true", pins::BASE.reference());
        assert!(r.calls().iter().any(|l| *l == smoke), "{:#?}", r.calls());
        let docker = c.engines.iter().find(|e| e.kind == EngineKind::Docker).unwrap();
        assert!(docker.notes.iter().any(|n| n.contains("root access")), "{:?}", docker.notes);
        assert!(docker.notes.iter().any(|n| n.contains("ufw")), "Docker's firewall bypass is said: {:?}", docker.notes);
        let p = c.engines.iter().find(|e| e.kind == EngineKind::Podman).unwrap();
        assert!(!p.notes.iter().any(|n| n.contains("ufw")));
        assert_eq!(c.engine_for(None), Some(Engine::new(EngineKind::Podman, true)));
        assert_eq!(c.engine_for(Some(EngineKind::Docker)), Some(Engine::new(EngineKind::Docker, false)));
    }

    #[test]
    fn nothing_installed_says_so() {
        struct NoEngine;
        impl crate::service::CommandRunner for NoEngine {
            fn run(&self, _: &str, _: &[String]) -> std::io::Result<crate::service::CommandOutput> {
                Err(std::io::Error::new(std::io::ErrorKind::NotFound, "no such file"))
            }
            fn write_file(&self, _: &str, _: &str, _: Option<u32>) -> std::io::Result<()> {
                Ok(())
            }
            fn remove_file(&self, _: &str) -> std::io::Result<()> {
                Ok(())
            }
            fn copy_file(&self, _: &str, _: &str, _: Option<u32>) -> std::io::Result<()> {
                Ok(())
            }
            fn remove_empty_dir(&self, _: &str) -> std::io::Result<()> {
                Ok(())
            }
        }
        impl EngineRunner for NoEngine {}
        let c = check(&NoEngine, &facts());
        assert!(!c.available && c.engines.is_empty() && c.chosen.is_none());
        assert_eq!(c.summary, "No container engine (Podman or Docker) found");
    }

    #[test]
    fn missing_subuid_ranges_are_a_guided_problem_with_the_command() {
        let r = FakeRunner::default();
        no_docker(&r);
        r.respond("podman info", 0, PODMAN_ROOTLESS);
        let mut f = facts();
        f.subuid = Some("".into());
        let c = check(&r, &f);
        assert!(!c.available);
        let p = c.engines[0].problem.as_ref().unwrap();
        assert!(p.has_code(super::super::codes::NO_SUBUID));
        assert!(p.actions.contains(&Action::show_command(&["usermod", "--add-subuids", "100000-165535", "--add-subgids", "100000-165535", "alex"], true)), "{:?}", p.actions);
        assert!(!r.calls().iter().any(|l| l.contains(" run ")), "no smoke run when the prerequisite is missing");
    }

    #[test]
    fn cgroup_v1_is_refused_for_rootless_in_words() {
        let r = FakeRunner::default();
        no_docker(&r);
        r.respond("podman info", 0, r#"{"host":{"cgroupVersion":"v1","security":{"rootless":true}},"version":{"Version":"3.4.4"}}"#);
        let c = check(&r, &facts());
        assert!(c.engines[0].problem.as_ref().unwrap().has_code(super::super::codes::CGROUP_V1));
    }

    #[test]
    fn docker_socket_permission_explains_that_the_group_is_root_equivalent() {
        let r = FakeRunner::default();
        r.respond("podman info", 1, "");
        r.responses.borrow_mut().last_mut().unwrap().1.stderr = "podman: command failed".into();
        r.respond("docker info", 1, "");
        r.responses.borrow_mut().last_mut().unwrap().1.stderr = "permission denied while trying to connect to the Docker daemon socket at unix:///var/run/docker.sock".into();
        let c = check(&r, &facts());
        let p = c.engines.iter().find(|e| e.kind == EngineKind::Docker).unwrap().problem.as_ref().unwrap();
        assert!(p.has_code(super::super::codes::SOCKET_PERMISSION));
        assert!(p.message.contains("same as having root"), "{}", p.message);
        assert!(!c.available);
    }

    #[test]
    fn lingering_off_is_a_warning_with_the_command_but_the_engine_still_works() {
        let r = FakeRunner::default();
        r.respond("podman info", 0, PODMAN_ROOTLESS);
        let mut f = facts();
        f.linger = Some(false);
        let c = check(&r, &f);
        assert!(c.available, "it works; only boot and logout are affected");
        let e = &c.engines[0];
        assert!(e.can_run && e.problem.as_ref().unwrap().has_code(super::super::codes::NOT_LINGERING));
        assert!(e.problem.as_ref().unwrap().actions.contains(&Action::show_command(&["loginctl", "enable-linger", "alex"], true)));
    }

    #[test]
    fn a_failed_smoke_run_is_reported_with_the_engines_words() {
        let r = FakeRunner::default();
        no_docker(&r);
        r.respond("podman info", 0, PODMAN_ROOTLESS);
        r.respond("podman run", 126, "");
        r.responses.borrow_mut().last_mut().unwrap().1.stderr = "Error: could not find catatonit".into();
        let c = check(&r, &facts());
        assert!(!c.available);
        let p = c.engines[0].problem.as_ref().unwrap();
        assert!(p.message.contains("catatonit"), "{}", p.message);
    }

    #[test]
    fn selinux_is_a_note_not_a_failure() {
        let r = FakeRunner::default();
        r.respond("podman info", 0, r#"{"host":{"cgroupVersion":"v2","security":{"rootless":true,"selinuxEnabled":true}},"version":{"Version":"5.0.0"}}"#);
        let c = check(&r, &facts());
        assert!(c.available && c.engines[0].selinux && c.engines[0].notes.iter().any(|n| n.contains(":Z")));
    }

    #[test]
    fn engine_failures_are_classified_from_their_own_words() {
        let e = Engine::new(EngineKind::Podman, true);
        let f = |s: &str| EngineError::Failed { command: "podman create".into(), code: 125, stderr: s.into() };
        assert!(problem_for_failure(&e, "x", &f("Error: cannot find newuidmap")).has_code(super::super::codes::NO_SUBUID));
        assert!(problem_for_failure(&e, "x", &f("bind: address already in use")).message.contains("Ports tab"));
        assert!(problem_for_failure(&e, "x", &f("the container name \"kora-main\" is already in use")).has_code(super::super::codes::NAME_TAKEN));
        assert!(problem_for_failure(&e, "x", &f("something odd")).has_code(super::super::codes::START_FAILED));
        assert!(pull_problem(&e, "img", "dial tcp: lookup registry-1.docker.io: no such host").message.contains("internet"));
        assert!(pull_problem(&e, "img", "toomanyrequests: rate limit").message.contains("rate limit"));
    }
}
