//! Updating Kora through the service, with rollback. The program lives in a folder users cannot write, so
//! the service does it: the `UpdateApply` operation runs the ordinary `kora self-update --yes` as the
//! child (same release feed, same checksum check, same atomic swap), and around it the service:
//!
//! 1. keeps a copy of the running program (`kora.previous` in its data folder; the `.old` the swap leaves
//!    is cleaned up by Windows at the next start, so it is not relied on);
//! 2. after the swap, starts the NEW program with `--version` (clean environment, time limit): if it does
//!    not run, the copy is put back at once and the person is told, nothing restarts;
//! 3. otherwise records a pending update ([`Marker`]), answers, and exits so the service manager starts
//!    the new program (`Restart=always`; Windows: a failure exit code with restart actions on non-crash
//!    failures);
//! 4. the new program counts its own starts: if it does not stay up ([`COMMIT_AFTER`]) for
//!    [`MAX_STARTS`] starts in a row it puts the previous program back and restarts ([`startup`]).
//!
//! Provenance: the child verifies the release asset against the release's `.sha256` as today; a signed
//! provenance attestation (`gh attestation verify`) is checked by the release job, not here (the
//! service has no `gh`): recorded as a follow-up in `docs/privileged-service.md`.

use serde::{Deserialize, Serialize};

/// The new program must stay up this long before the update counts as good.
pub const COMMIT_AFTER: std::time::Duration = std::time::Duration::from_secs(60);
/// Starts of the new program allowed before it is rolled back.
pub const MAX_STARTS: u32 = 3;

/// `update.json` in the service's data folder, written before the service restarts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marker {
    /// The version the update installed.
    pub to: String,
    /// The version it replaced.
    pub from: String,
    /// How many times the new program has started since (it counts itself).
    pub starts: u32,
}

/// What the service does at start, given a marker and its own version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Startup {
    /// No pending update: carry on.
    Nothing,
    /// The new program started: remember the start, commit after [`COMMIT_AFTER`].
    Pending(Marker),
    /// The marker is about another version (the previous program is running after a rollback, or the
    /// marker is stale): drop it.
    Clear,
    /// The new program keeps failing: put the previous one back and restart.
    Rollback,
}

pub fn startup(marker: Option<&Marker>, own_version: &str) -> Startup {
    let Some(m) = marker else { return Startup::Nothing };
    if !same_version(&m.to, own_version) {
        return Startup::Clear;
    }
    if m.starts >= MAX_STARTS {
        return Startup::Rollback;
    }
    Startup::Pending(Marker { starts: m.starts + 1, ..m.clone() })
}

fn same_version(a: &str, b: &str) -> bool {
    a.trim_start_matches('v') == b.trim_start_matches('v')
}

/// The version out of `kora --version` ("kora 0.8.0").
pub fn parse_version_output(out: &str) -> Option<String> {
    let rest = out.trim().strip_prefix("kora ")?;
    let v = rest.split_whitespace().next()?;
    crate::privsvc::op::version_ok(v).then(|| v.trim_start_matches('v').to_string())
}

/// Did the new program start and say a version newer than (or equal to, for a reinstall) the old one?
pub fn new_program_ok(version_output: &str, from: &str) -> Result<String, String> {
    let v = parse_version_output(version_output).ok_or("the new program did not report its version")?;
    match crate::app_install::compare_versions(&v, from) {
        Some(std::cmp::Ordering::Less) => Err(format!("the new program reports {v}, older than {from}")),
        _ => Ok(v),
    }
}

// ---- the files ---------------------------------------------------------------------------------------------------

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Where the service keeps the pending-update record and the previous program (its data folder).
#[derive(Debug, Clone)]
pub struct Paths {
    pub marker: PathBuf,
    pub backup: PathBuf,
}

impl Paths {
    pub fn in_dir(dir: &Path) -> Paths {
        Paths { marker: dir.join("update.json"), backup: dir.join(if cfg!(windows) { "kora.previous.exe" } else { "kora.previous" }) }
    }
}

static RESTART: AtomicBool = AtomicBool::new(false);
/// Ask the service to exit once the current answer is sent, so the service manager starts the new program.
pub fn request_restart() {
    RESTART.store(true, Ordering::SeqCst);
}
pub fn restart_requested() -> bool {
    RESTART.load(Ordering::SeqCst)
}

pub fn read_marker(p: &Path) -> Option<Marker> {
    serde_json::from_slice(&std::fs::read(p).ok()?).ok()
}

pub fn write_marker(p: &Path, m: &Marker) -> std::io::Result<()> {
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(m).map_err(std::io::Error::other)?)?;
    let _ = std::fs::remove_file(p);
    std::fs::rename(tmp, p)
}

/// Put the previous program back at `exe`: written next to it, then swapped in (a running program can be
/// renamed on Windows, not overwritten).
pub fn restore(backup: &Path, exe: &Path) -> std::io::Result<()> {
    let mut new = exe.as_os_str().to_os_string();
    new.push(".restore");
    let new = PathBuf::from(new);
    std::fs::copy(backup, &new)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755))?;
    }
    #[cfg(windows)]
    {
        let mut bad = exe.as_os_str().to_os_string();
        bad.push(".bad");
        let bad = PathBuf::from(bad);
        let _ = std::fs::remove_file(&bad);
        std::fs::rename(exe, &bad)?;
    }
    std::fs::rename(&new, exe)
}

/// How long the new program may take to print its version before it counts as broken.
const START_PROBE_LIMIT: std::time::Duration = std::time::Duration::from_secs(60);

/// The update operation around the child (`run_child` does `kora self-update --yes`): keep the running program,
/// let the child swap it, check the new one starts (else put the old one back at once), record the pending
/// update and ask for a restart.
pub fn run_update(
    exe: &Path,
    data_dir: &Path,
    progress: super::dispatch::Progress<'_>,
    run_child: &mut dyn FnMut(super::dispatch::Progress<'_>) -> std::io::Result<super::dispatch::Ran>,
) -> std::io::Result<super::dispatch::Ran> {
    let paths = Paths::in_dir(data_dir);
    let from = env!("CARGO_PKG_VERSION");
    std::fs::copy(exe, &paths.backup)?;
    let ran = run_child(&mut *progress)?;
    if ran.code != 0 || ran.timed_out {
        return Ok(ran); // the swap is atomic: nothing changed
    }
    // The new program might hang at start-up: it gets a time limit (and no input), else the service would wait for ever.
    let mut probe = std::process::Command::new(exe);
    probe.arg("--version").env_clear();
    let out = crate::sysexe::output_within(probe, START_PROBE_LIMIT);
    let verdict = match out {
        Ok(o) if o.status.success() => new_program_ok(&String::from_utf8_lossy(&o.stdout), from),
        Ok(o) => Err(format!("the new program stopped with {}", o.status)),
        Err(e) => Err(format!("the new program did not start: {e}")),
    };
    match verdict {
        Ok(v) => {
            write_marker(&paths.marker, &Marker { to: v.clone(), from: from.into(), starts: 0 })?;
            progress("stdout", &format!("Kora {v} is installed. The administrator service restarts on it now."));
            request_restart();
            Ok(ran)
        }
        Err(why) => {
            restore(&paths.backup, exe)?;
            progress("stderr", &format!("The new program did not work ({why}), so the previous one was put back. Nothing changed."));
            Ok(super::dispatch::Ran { code: 1, timed_out: false, data: None })
        }
    }
}

/// What starting the service found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Boot {
    Normal,
    /// The new program started for the first times: clear the marker after [`COMMIT_AFTER`] (this record).
    Probation(Marker),
    /// The previous program was put back: the service must exit now so it is restarted as that program.
    RolledBack,
}

/// Run first thing in the service. Never fails the start: a problem here is reported in the returned text.
pub fn on_start(paths: &Paths, exe: &Path, own_version: &str) -> (Boot, Option<String>) {
    let marker = read_marker(&paths.marker);
    match startup(marker.as_ref(), own_version) {
        Startup::Nothing => (Boot::Normal, None),
        Startup::Clear => {
            let _ = std::fs::remove_file(&paths.marker);
            (Boot::Normal, None)
        }
        Startup::Pending(m) => {
            let _ = write_marker(&paths.marker, &m);
            (Boot::Probation(m), None)
        }
        Startup::Rollback => match restore(&paths.backup, exe) {
            Ok(()) => {
                let _ = std::fs::remove_file(&paths.marker);
                (Boot::RolledBack, Some(format!("the update to {} did not stay up; the previous program was put back", marker.map(|m| m.to).unwrap_or_default())))
            }
            Err(e) => (Boot::Normal, Some(format!("the update did not stay up and the previous program could not be put back: {e}"))),
        },
    }
}

/// After [`COMMIT_AFTER`] of staying up: the update is good.
pub fn commit(paths: &Paths, m: &Marker) {
    if read_marker(&paths.marker).as_ref().is_some_and(|c| c.to == m.to) {
        let _ = std::fs::remove_file(&paths.marker);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(to: &str, starts: u32) -> Marker {
        Marker { to: to.into(), from: "0.7.0".into(), starts }
    }

    #[test]
    fn the_new_program_counts_its_starts_and_is_rolled_back_after_three() {
        assert_eq!(startup(None, "0.8.0"), Startup::Nothing);
        assert_eq!(startup(Some(&m("0.8.0", 0)), "0.8.0"), Startup::Pending(m("0.8.0", 1)));
        assert_eq!(startup(Some(&m("v0.8.0", 2)), "0.8.0"), Startup::Pending(Marker { to: "v0.8.0".into(), from: "0.7.0".into(), starts: 3 }));
        assert_eq!(startup(Some(&m("0.8.0", 3)), "0.8.0"), Startup::Rollback, "it did not stay up three times");
    }

    #[test]
    fn a_marker_for_another_version_is_dropped() {
        assert_eq!(startup(Some(&m("0.8.0", 3)), "0.7.0"), Startup::Clear, "the previous program is running again");
        assert_eq!(startup(Some(&m("0.8.0", 0)), "0.9.0"), Startup::Clear, "stale");
    }

    #[test]
    fn the_new_program_must_say_a_version_that_is_not_older() {
        assert_eq!(parse_version_output("kora 0.8.0\n"), Some("0.8.0".into()));
        assert_eq!(parse_version_output("kora v0.8.1-rc1"), Some("0.8.1-rc1".into()));
        for bad in ["", "error: segfault", "kora", "kora latest", "something 1.2.3"] {
            assert_eq!(parse_version_output(bad), None, "{bad:?}");
        }
        assert_eq!(new_program_ok("kora 0.8.0", "0.7.0"), Ok("0.8.0".into()));
        assert_eq!(new_program_ok("kora 0.7.0", "0.7.0"), Ok("0.7.0".into()), "a reinstall");
        assert!(new_program_ok("kora 0.6.0", "0.7.0").unwrap_err().contains("older"));
        assert!(new_program_ok("", "0.7.0").is_err());
    }

    #[test]
    fn the_marker_round_trips() {
        let j = serde_json::to_string(&m("0.8.0", 1)).unwrap();
        assert_eq!(serde_json::from_str::<Marker>(&j).unwrap(), m("0.8.0", 1));
    }

    #[test]
    fn a_failing_new_program_is_rolled_back_on_its_third_start_and_a_good_one_commits() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::in_dir(d.path());
        let exe = d.path().join("kora-exe");
        std::fs::write(&exe, "NEW").unwrap();
        std::fs::write(&paths.backup, "OLD").unwrap();
        write_marker(&paths.marker, &m("0.8.0", 0)).unwrap();
        for n in 1..=3 {
            let (b, said) = on_start(&paths, &exe, "0.8.0");
            assert_eq!(b, Boot::Probation(m("0.8.0", n)), "start {n}");
            assert!(said.is_none());
        }
        let (b, said) = on_start(&paths, &exe, "0.8.0");
        assert_eq!(b, Boot::RolledBack);
        assert!(said.unwrap().contains("put back"));
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "OLD");
        assert!(!paths.marker.exists());
        // a program that stays up: the marker goes after the probation
        write_marker(&paths.marker, &m("0.8.0", 0)).unwrap();
        let (Boot::Probation(rec), _) = on_start(&paths, &exe, "0.8.0") else { panic!("probation") };
        commit(&paths, &rec);
        assert!(!paths.marker.exists());
        // the old program starting after a rollback drops a marker that is not its own
        write_marker(&paths.marker, &m("0.8.0", 3)).unwrap();
        assert_eq!(on_start(&paths, &exe, "0.7.0").0, Boot::Normal);
        assert!(!paths.marker.exists());
    }
}
