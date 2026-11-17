//! Kora's own copies of `gh` (Linux and Windows) and Git (MinGit, Windows), and the ONE place that decides
//! which `git` / `gh` a command runs.
//!
//! * **Resolution** ([`locate`]): a copy that is already on this computer (on the `PATH`, or in the
//!   usual install folders on Windows) is used; otherwise Kora's private copy in `<Kora folder>\tools`.
//!   The shell's `PATH` never has to be refreshed: a freshly installed tool is found by looking in its
//!   folder, not by waiting for a new window. Every `git` / `gh` the program starts goes through
//!   `git::system_command`, which asks this module.
//! * **Installing** ([`install`]): the pinned, checksum-verified download from the official GitHub
//!   release ([`pins`]), unpacked safely ([`unpack`]) and moved into place in one step, so an
//!   interrupted download never leaves a broken tool. No administrator rights, no `PATH` change.
//! * **Uninstalling** ([`remove_private`]): deletes `<Kora folder>\tools`.
//!
//! Linux Git is not downloaded: it comes from the system package manager, as before.

pub mod pins;
pub mod unpack;

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use self::pins::Pin;
use crate::problem::{codes, Action, Problem, Tool};

/// Where a tool comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// Already on this computer.
    System,
    /// Kora's own copy.
    Private,
}

/// A tool that was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub source: Source,
    pub path: PathBuf,
}

/// `"git"` / `"gh"`.
pub fn program_name(tool: Tool) -> &'static str {
    match tool {
        Tool::Git => "git",
        Tool::Gh => "gh",
        _ => "",
    }
}

/// The tool a command name stands for.
pub fn tool_of(program: &str) -> Option<Tool> {
    match program {
        "git" => Some(Tool::Git),
        "gh" => Some(Tool::Gh),
        _ => None,
    }
}

/// `<Kora folder>/tools` (`KORA_HOME` moves it, like everything else Kora keeps).
pub fn private_root() -> PathBuf {
    crate::installer::registry::kora_home().join("tools")
}

fn tool_dir(root: &Path, tool: Tool) -> PathBuf {
    root.join(program_name(tool))
}

const MARKER: &str = ".kora-tool.json";

/// Written last inside a finished copy: a folder without it is not a tool (an interrupted install).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Marker {
    version: String,
    sha256: String,
    /// The program, relative to the tool's folder.
    exe: String,
}

/// A finished private copy: its version and program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Private {
    pub version: String,
    pub exe: PathBuf,
}

/// The private copy of `tool` under `root`, if it is complete.
pub fn private_in(root: &Path, tool: Tool) -> Option<Private> {
    let dir = tool_dir(root, tool);
    let m: Marker = serde_json::from_slice(&std::fs::read(dir.join(MARKER)).ok()?).ok()?;
    // The record is a file on disk: do not follow a path in it out of the folder.
    let rel = unpack::safe_relative(&m.exe, false).ok()??;
    let exe = dir.join(rel);
    exe.is_file().then_some(Private { version: m.version, exe })
}

fn is_runnable(p: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(p) else { return false };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// The first copy of `tool` on `path` (absolute folders only, and not the folder `kora` itself runs from
/// on Windows, where that folder is searched first and may hold a planted `git.exe`), then in `extra`.
pub fn find_system(tool: Tool, path: Option<&OsStr>, extra: &[PathBuf], windows: bool, exe_dir: Option<&Path>) -> Option<PathBuf> {
    let name = if windows { format!("{}.exe", program_name(tool)) } else { program_name(tool).to_string() };
    let on_path: Vec<PathBuf> = path.map(|p| std::env::split_paths(p).collect()).unwrap_or_default();
    let skip = |d: &Path| !d.is_absolute() || (windows && exe_dir.is_some_and(|e| same_dir(e, d)));
    on_path.iter().chain(extra.iter()).filter(|d| !skip(d)).map(|d| d.join(&name)).find(|c| is_runnable(c))
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().trim_end_matches(['\\', '/']).to_ascii_lowercase().replace('/', "\\");
    norm(a) == norm(b)
}

/// Pure core of [`locate`]: a system copy first, else the private one under `root`.
pub fn locate_in(tool: Tool, path: Option<&OsStr>, extra: &[PathBuf], root: &Path, windows: bool, exe_dir: Option<&Path>) -> Option<Found> {
    if let Some(p) = find_system(tool, path, extra, windows, exe_dir) {
        return Some(Found { source: Source::System, path: p });
    }
    private_in(root, tool).map(|p| Found { source: Source::Private, path: p.exe })
}

/// Usual install folders on Windows (Program Files, `%LOCALAPPDATA%\Programs`): a copy installed a moment
/// ago is there even though this process's `PATH` does not list it yet.
fn windows_extra_dirs() -> Vec<PathBuf> {
    if cfg!(windows) {
        crate::git::windows_tool_dirs(&|k| std::env::var(k).ok())
    } else {
        Vec::new()
    }
}

/// Which copy of `tool` Kora uses right now (`None`: there is none).
pub fn locate(tool: Tool) -> Option<Found> {
    if program_name(tool).is_empty() {
        return None;
    }
    let exe_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf));
    locate_in(tool, std::env::var_os("PATH").as_deref(), &windows_extra_dirs(), &private_root(), cfg!(windows), exe_dir.as_deref())
}

/// The program to start for a `git` / `gh` command: the full path of the private copy when that is the one
/// in use, otherwise the bare name (the operating system finds a system copy itself, as it always did).
/// Any other program is returned unchanged.
pub fn program_for(program: &str) -> String {
    let Some(tool) = tool_of(program) else { return program.to_string() };
    match locate(tool) {
        Some(Found { source: Source::Private, path }) => path.to_string_lossy().into_owned(),
        Some(Found { source: Source::System, path }) if cfg!(windows) => path.to_string_lossy().into_owned(),
        _ => program.to_string(),
    }
}

/// `PATH` for a child process when the private Git is the one in use (so `gh`, which starts `git`, finds
/// it): its `cmd` folder in front of the current `PATH`. `None` when nothing has to change.
pub fn child_path() -> Option<OsString> {
    let Found { source: Source::Private, path } = locate(Tool::Git)? else { return None };
    path_with_dir(std::env::var_os("PATH"), path.parent()?)
}

/// `current` with `dir` in front (pure part of [`child_path`]).
pub fn path_with_dir(current: Option<OsString>, dir: &Path) -> Option<OsString> {
    let mut all = vec![dir.to_path_buf()];
    all.extend(current.iter().flat_map(|c| std::env::split_paths(c)));
    std::env::join_paths(all).ok()
}

/// The git credential helper that asks the private `gh` for GitHub sign-in details, when that `gh` is the one
/// in use (a system `gh` has its own `gh auth setup-git` entry). Git runs it through its shell, so the path is
/// quoted for it.
pub fn gh_credential_helper() -> Option<String> {
    let Found { source: Source::Private, path } = locate(Tool::Gh)? else { return None };
    Some(credential_helper_for(&path, cfg!(windows)))
}

/// `!"<gh>" auth git-credential` quoted for the shell git uses.
pub fn credential_helper_for(gh: &Path, windows: bool) -> String {
    let p = gh.to_string_lossy();
    if windows {
        // Git for Windows' sh wants forward slashes; a Windows path cannot contain a double quote.
        format!("!\"{}\" auth git-credential", p.replace('\\', "/"))
    } else {
        format!("!'{}' auth git-credential", p.replace('\'', "'\\''"))
    }
}

/// How one tool stands, for the System check and `kora tools status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolStatus {
    pub tool: Tool,
    pub name: &'static str,
    pub label: &'static str,
    /// Found and it runs.
    pub installed: bool,
    pub version: Option<String>,
    pub source: Option<Source>,
    pub path: Option<String>,
    /// A copy exists but did not run.
    pub broken: bool,
    /// Kora can download its own copy on this computer.
    pub can_install: bool,
}

/// The state of `tool`, by asking it for its version through `r` (so tests need no real tools).
pub fn status(r: &dyn crate::git::CommandRunner, tool: Tool) -> ToolStatus {
    let name = program_name(tool);
    let found = locate(tool);
    let ran = r.run(name, &["--version"], None).ok().filter(|o| o.ok()).and_then(|o| crate::git::parse_version(&o.stdout));
    let broken = found.is_some() && ran.is_none();
    ToolStatus {
        tool,
        name,
        label: tool.label(),
        installed: ran.is_some(),
        version: ran,
        source: found.as_ref().map(|f| f.source),
        path: found.as_ref().map(|f| f.path.to_string_lossy().into_owned()),
        broken,
        can_install: pins::pin_here(tool).is_some(),
    }
}

// ----------------------------------------------------------------------------------------------
// Installing
// ----------------------------------------------------------------------------------------------

/// Why a download failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchFailure {
    /// Offline, a proxy, GitHub busy, a stalled connection.
    Network(String),
    /// The file is not the pinned one.
    Checksum,
}

/// Downloads one file (tests replace it).
pub trait Fetcher {
    /// Write `url` to `dest`; `sha256` is what the file must hash to. `progress(bytes, total)`.
    fn fetch(&self, url: &str, dest: &Path, sha256: &str, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<(), FetchFailure>;
}

/// The real download: HTTPS only, resumable, with the pinned checksum.
pub struct HttpFetcher(pub crate::installer::http::HttpClient);

impl Fetcher for HttpFetcher {
    fn fetch(&self, url: &str, dest: &Path, sha256: &str, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<(), FetchFailure> {
        let cfg = crate::installer::http::FetchConfig::default();
        self.0
            .download_verified(&[url.to_string()], dest, sha256, &cfg, &mut |p| progress(p.bytes, p.total), &mut |_| {})
            .map(|_| ())
            .map_err(|e| if e.contains("SHA-256") { FetchFailure::Checksum } else { FetchFailure::Network(e) })
    }
}

/// What an install produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub tool: Tool,
    pub version: String,
    pub exe: PathBuf,
}

fn manual_url(tool: Tool) -> &'static str {
    match tool {
        Tool::Git => crate::problem::known::GIT_URL,
        _ => crate::problem::known::GH_URL,
    }
}

fn download_failed(tool: Tool, why: &str) -> Problem {
    let slow_down = why.contains("429") || why.to_ascii_lowercase().contains("rate limit");
    let message = if slow_down {
        format!("GitHub asked Kora to slow down, so {} was not downloaded. Wait a minute and try again.", tool.label())
    } else {
        format!("Kora could not download {}. Check your internet connection (or proxy) and try again.", tool.label())
    };
    Problem::new(codes::TOOLS_DOWNLOAD_FAILED, message)
        .with_details(why)
        .with_action(Action::Retry)
        .with_action(Action::open_link(manual_url(tool)))
}

fn failed(tool: Tool, what: &str, why: impl std::fmt::Display) -> Problem {
    Problem::new(codes::TOOLS_DOWNLOAD_FAILED, format!("Kora could not set up {}: {what}.", tool.label()))
        .with_details(why.to_string())
        .with_action(Action::Retry)
        .with_action(Action::open_link(manual_url(tool)))
}

fn unsupported(tool: Tool) -> Problem {
    let mut p = Problem::new(codes::TOOLS_UNSUPPORTED, format!("Kora has no ready-made copy of {} for this computer.", tool.label()));
    if tool == Tool::Git && !cfg!(windows) {
        p.message = "Git comes from your system's package manager on Linux.".into();
        p.push_action(Action::show_command(&["apt-get", "install", "-y", "git"], true));
    }
    p.with_action(Action::open_link(manual_url(tool)))
}

fn random_name() -> String {
    let mut b = [0u8; 6];
    let _ = getrandom::fill(&mut b);
    hex::encode(b)
}

/// Leftovers of an interrupted install (partial downloads, half-unpacked folders, replaced copies).
fn sweep(root: &Path, tool: Tool) {
    let n = program_name(tool);
    let Ok(rd) = std::fs::read_dir(root) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let mine = [format!(".dl-{n}-"), format!(".stage-{n}-"), format!(".old-{n}-")].iter().any(|p| name.starts_with(p.as_str()));
        if mine {
            let p = e.path();
            if p.is_dir() {
                let _ = std::fs::remove_dir_all(&p);
            } else {
                let _ = std::fs::remove_file(&p);
            }
        }
    }
}

/// Removes the listed paths when dropped (an early return leaves nothing behind).
struct Cleanup(Vec<PathBuf>);
impl Drop for Cleanup {
    fn drop(&mut self) {
        for p in &self.0 {
            if p.is_dir() {
                let _ = std::fs::remove_dir_all(p);
            } else {
                let _ = std::fs::remove_file(p);
            }
        }
    }
}

/// One install at a time per tool (the panel and a terminal may both ask): a lock file that goes stale
/// after twenty minutes (a crashed install).
struct Lock(PathBuf);
impl Lock {
    fn take(root: &Path, tool: Tool) -> Result<Lock, Problem> {
        let p = root.join(format!(".installing-{}", program_name(tool)));
        for _ in 0..2 {
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&p) {
                Ok(_) => return Ok(Lock(p)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let age = std::fs::metadata(&p).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok());
                    if age.is_some_and(|a| a < std::time::Duration::from_secs(20 * 60)) {
                        return Err(Problem::new(codes::TOOLS_DOWNLOAD_FAILED, format!("{} is already being installed. Wait a minute and try again.", tool.label())).with_action(Action::Retry));
                    }
                    let _ = std::fs::remove_file(&p);
                }
                Err(e) => return Err(failed(tool, "cannot write to Kora's folder", e)),
            }
        }
        Err(failed(tool, "another install is in the way", ""))
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Download, verify, unpack and move `pin` into place under `root`. `say(message, fraction 0..1)`.
pub fn install_in(root: &Path, pin: &Pin, fetch: &dyn Fetcher, say: &mut dyn FnMut(&str, f32)) -> Result<Installed, Problem> {
    let tool = pin.tool;
    std::fs::create_dir_all(root).map_err(|e| failed(tool, "cannot create Kora's tools folder", e))?;
    let _lock = Lock::take(root, tool)?;
    sweep(root, tool);
    let tag = random_name();
    let part = root.join(format!(".dl-{}-{tag}.part", program_name(tool)));
    let stage = root.join(format!(".stage-{}-{tag}", program_name(tool)));
    let old = root.join(format!(".old-{}-{tag}", program_name(tool)));
    let mut guard = Cleanup(vec![part.clone(), stage.clone()]);

    say(&format!("Downloading {} {}", tool.label(), pin.version), 0.02);
    let total = pin.size;
    let mut last = 0u8;
    fetch
        .fetch(pin.url, &part, pin.sha256, &mut |b, t| {
            let pct = (b.saturating_mul(100) / t.unwrap_or(total).max(1)).min(100) as u8;
            if pct != last {
                last = pct;
                say(&format!("Downloading {} {} ({pct}%)", tool.label(), pin.version), 0.02 + 0.78 * f32::from(pct) / 100.0);
            }
        })
        .map_err(|e| match e {
            FetchFailure::Network(m) => download_failed(tool, &m),
            FetchFailure::Checksum => checksum_problem(tool),
        })?;

    // Verify again here, whatever the downloader did: nothing is unpacked unless it is the pinned file.
    say("Checking the download", 0.82);
    let got = crate::installer::http::sha256_file(&part).map_err(|e| failed(tool, "cannot read the download back", e))?;
    if !got.eq_ignore_ascii_case(pin.sha256) {
        return Err(checksum_problem(tool));
    }

    say(&format!("Unpacking {}", tool.label()), 0.86);
    unpack::unpack(&part, pin.format, &stage, pin.strip_top).map_err(|e| failed(tool, "the download could not be unpacked", e))?;
    let exe_rel = unpack::safe_relative(pin.exe, false).ok().flatten().ok_or_else(|| failed(tool, "bad program path", pin.exe))?;
    if !stage.join(&exe_rel).is_file() {
        return Err(failed(tool, "the download does not contain the program", pin.exe));
    }
    let marker = Marker { version: pin.version.to_string(), sha256: pin.sha256.to_string(), exe: pin.exe.to_string() };
    std::fs::write(stage.join(MARKER), serde_json::to_vec(&marker).unwrap_or_default()).map_err(|e| failed(tool, "cannot write the record", e))?;

    // Into place in one step; the previous copy (if any) is kept until the new one is there.
    say("Finishing", 0.95);
    let dest = tool_dir(root, tool);
    let had_old = dest.exists();
    if had_old {
        std::fs::rename(&dest, &old).map_err(|e| failed(tool, "the old copy is in use. Close anything using it and try again", e))?;
        guard.0.push(old.clone());
    }
    if let Err(e) = std::fs::rename(&stage, &dest) {
        if had_old {
            let _ = std::fs::rename(&old, &dest);
            guard.0.retain(|p| p != &old);
        }
        return Err(failed(tool, "cannot move the new copy into place", e));
    }
    guard.0.retain(|p| p != &stage);
    say(&format!("{} {} is ready", tool.label(), pin.version), 1.0);
    Ok(Installed { tool, version: pin.version.to_string(), exe: dest.join(exe_rel) })
}

fn checksum_problem(tool: Tool) -> Problem {
    Problem::new(codes::TOOLS_CHECKSUM, format!("The download of {} did not match the checksum Kora expects, so it was thrown away and nothing was installed.", tool.label()))
        .with_action(Action::Retry)
        .with_action(Action::open_link(manual_url(tool)))
}

/// Install Kora's private copy of `tool` for this computer.
pub fn install(tool: Tool, fetch: &dyn Fetcher, say: &mut dyn FnMut(&str, f32)) -> Result<Installed, Problem> {
    let Some(pin) = pins::pin_here(tool) else { return Err(unsupported(tool)) };
    install_in(&private_root(), pin, fetch, say)
}

/// [`install`] with the real download.
pub fn install_real(tool: Tool, say: &mut dyn FnMut(&str, f32)) -> Result<Installed, Problem> {
    install(tool, &HttpFetcher(crate::installer::http::HttpClient::default()), say)
}

/// Kora's private copies that exist now, as `"Git 2.56.1"` (what `kora uninstall` would remove).
pub fn private_present() -> Vec<String> {
    private_present_in(&private_root())
}

pub fn private_present_in(root: &Path) -> Vec<String> {
    [Tool::Git, Tool::Gh].iter().filter_map(|t| private_in(root, *t).map(|p| format!("{} {}", t.label(), p.version))).collect()
}

/// Delete Kora's private tools. Returns what was removed (empty: there were none).
pub fn remove_private() -> Result<Vec<String>, String> {
    remove_private_in(&private_root())
}

pub fn remove_private_in(root: &Path) -> Result<Vec<String>, String> {
    if !root.exists() {
        return Ok(vec![]);
    }
    let removed = private_present_in(root);
    std::fs::remove_dir_all(root).map_err(|e| format!("could not remove {}: {e}", root.display()))?;
    Ok(removed)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod message_tests {
    use super::*;

    #[test]
    fn download_failures_read_in_plain_words_and_always_offer_retry() {
        let off = download_failed(Tool::Gh, "github.com: connection refused");
        assert!(off.message.contains("internet connection (or proxy)"), "{}", off.message);
        let busy = download_failed(Tool::Git, "the server answered HTTP 429");
        assert!(busy.message.contains("slow down"), "{}", busy.message);
        for p in [off, busy] {
            assert_eq!(p.actions[0], Action::Retry);
            assert!(p.actions.iter().any(|a| matches!(a, Action::OpenLink { url } if url.starts_with("https://"))));
        }
    }
}
