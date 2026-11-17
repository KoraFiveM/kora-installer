//! `kora self-update [--check] [--yes] [--version vX.Y.Z]`.

use std::ffi::OsString;
use std::io::{BufRead, IsTerminal, Write};

use super::release::{select_assets, this_asset_name};
use super::replace::{Flow, RealFs};
use super::*;

#[derive(Debug, Clone, Default)]
pub struct Options {
    pub check: bool,
    pub yes: bool,
    pub version: Option<String>,
}

fn confirm(question: &str) -> bool {
    print!("{question} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut l = String::new();
    std::io::stdin().lock().read_line(&mut l).is_ok() && matches!(l.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// Returns the process exit code.
pub fn run(o: &Options) -> i32 {
    let fail = |m: String| {
        eprintln!("error: {m}");
        1
    };
    let cur = running_version();
    let src = Source::from_env();
    let releases = match fetch_releases(&src) {
        Ok(r) => r,
        Err(e) => return fail(e.to_string()),
    };
    let rel = match decide(&cur, &releases, o.version.as_deref()) {
        Ok(Decision::UpToDate { latest }) => {
            println!("Kora v{cur} is up to date (newest release: v{latest}).");
            return 0;
        }
        Ok(Decision::Update(r)) => r,
        Err(e) => return fail(e.to_string()),
    };
    let new = rel.version().expect("selected releases have versions");
    if o.version.is_some() && new == cur {
        println!("Reinstalling Kora v{new}.");
    } else if new < cur {
        println!("Kora v{new} is OLDER than the running v{cur} (you asked for it with --version).");
    } else {
        println!("Kora v{new} is available (you have v{cur}).");
    }
    let Some(asset) = this_asset_name() else { return fail(SelfUpdateError::UnsupportedPlatform.to_string()) };
    if let Err(e) = select_assets(&rel, asset) {
        return fail(format!("{e}. Nothing was changed."));
    }
    if o.check {
        println!("Run: kora self-update");
        return 0;
    }

    let running = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => return fail(format!("cannot find the kora program: {e}")),
    };
    let target = resolve_target(&running, cfg!(windows), installed_app());
    if !o.yes {
        if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
            return fail("not a terminal: add --yes to update without asking".into());
        }
        if !confirm(&format!("Update {} to v{new}?", target.exe.display())) {
            println!("Not updated.");
            return 0;
        }
    }
    let dir = target.exe.parent().map(Path::to_path_buf).unwrap_or_default();
    if !dir_writable(&dir) {
        if crate::elevate::is_elevated() {
            return fail(format!("{} is not writable even with administrator rights", dir.display()));
        }
        println!("{} needs administrator permission.", dir.display());
        let args: Vec<OsString> = ["self-update", "--yes", "--version", &rel.tag_name].iter().map(OsString::from).collect();
        return match crate::elevate::relaunch_elevated(&args) {
            Ok(()) => {
                println!("The update continues in the administrator window.");
                0
            }
            Err(e) => fail(e.to_string()),
        };
    }
    println!("Downloading {asset} and verifying its checksum...");
    match apply(&src, &rel, asset, &target.exe, &RealFs, Flow::native()) {
        Ok(a) => {
            if let Some(s) = &target.state {
                if let Err(e) = update_app_state(s, &new, &a.sha256) {
                    eprintln!("warning: could not update {}: {e}", s.display());
                }
            }
            println!("Updated {} to v{new}.", a.target.display());
            if let Some(b) = &a.backup {
                if cfg!(windows) {
                    println!("The previous program ({}) is removed the next time kora starts.", b.display());
                } else {
                    println!("The previous program is kept as {}.", b.display());
                }
            }
            if running != target.exe && target.app_copy {
                println!("(This copy, {}, was not changed; the installed app copy was.)", running.display());
            }
            0
        }
        Err(e) => fail(format!("{e}")),
    }
}
