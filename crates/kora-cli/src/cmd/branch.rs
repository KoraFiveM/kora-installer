//! `kora branch ...` and `kora pr ...`: change the branch of a Development server's resources, try a pull request, go back
//! to the normal version, for one resource or all of them. The same operations as the panel's Branches tab.
//!
//! Exit codes: 0 done, 1 failed, 3 something needs a choice (a resource has changes of your own: add `--stash` or `--discard`).

use clap::{Args, Subcommand};
use kora_core::branches::{self as br, BulkStatus, Changes, PrSource, Reply, Request, ResourceBranch, State, Target};
use kora_core::git::branches::Status;
use kora_core::problem::{plain_text, Problem};

#[derive(Args, Clone, Default)]
pub struct ChangeArgs {
    /// If a resource has changes of your own: keep them aside (they can be brought back with `kora branch restore`).
    #[arg(long, conflicts_with = "discard")]
    stash: bool,
    /// If a resource has changes of your own: throw them away (asks first, unless --yes).
    #[arg(long)]
    discard: bool,
    /// Do not ask before discarding.
    #[arg(long, short = 'y')]
    yes: bool,
}

#[derive(Subcommand)]
pub enum BranchCmd {
    /// Show which branch each resource of a Development server is on.
    List {
        /// Registry name, or the server folder.
        server: String,
    },
    /// Show the branches of one resource, and changes you kept aside.
    Show {
        server: String,
        resource: String,
        /// Download the newest branch lists from GitHub first.
        #[arg(long)]
        refresh: bool,
    },
    /// Put one resource on another branch.
    Switch {
        server: String,
        resource: String,
        branch: String,
        /// Where to take the branch from if the resource does not have it yet: origin (your fork) or upstream (the original).
        #[arg(long, value_name = "origin|upstream")]
        from: Option<String>,
        #[command(flatten)]
        changes: ChangeArgs,
    },
    /// Back to the normal version: one resource, or all of them when none is named.
    Default {
        server: String,
        resource: Option<String>,
        #[command(flatten)]
        changes: ChangeArgs,
    },
    /// Put every resource that has a branch on it (`--preview` only shows which ones have it).
    All {
        server: String,
        branch: String,
        /// Only show which resources have the branch.
        #[arg(long)]
        preview: bool,
        /// Do not download the newest branch lists first.
        #[arg(long)]
        no_fetch: bool,
        #[command(flatten)]
        changes: ChangeArgs,
    },
    /// Bring changes you kept aside back (see `kora branch show`).
    Restore {
        server: String,
        resource: String,
        /// The number shown by `kora branch show`.
        index: u32,
    },
}

#[derive(Subcommand)]
pub enum PrCmd {
    /// List the open pull requests of the original repository and your fork, for one resource.
    List { server: String, resource: String },
    /// Try a pull request on one resource: its number or its GitHub address.
    Checkout {
        server: String,
        resource: String,
        /// The pull request's number (12) or address (https://github.com/owner/name/pull/12).
        pr: String,
        /// Which repository a bare number is on: upstream (the original, the default) or fork (yours).
        #[arg(long, value_name = "upstream|fork")]
        source: Option<String>,
        #[command(flatten)]
        changes: ChangeArgs,
    },
}

fn changes_of(a: &ChangeArgs) -> Option<Changes> {
    if a.discard {
        if !a.yes && !crate::ui::confirm("This throws away the changes you made in the resource folder, for good. Continue?") {
            println!("Cancelled. Nothing was changed.");
            return None;
        }
        Some(Changes::Discard)
    } else if a.stash {
        Some(Changes::Stash)
    } else {
        Some(Changes::Ask)
    }
}

fn fail(p: &Problem) -> i32 {
    eprintln!("error: {}", plain_text(&p.encode()));
    1
}

fn ask(server: &str, req: &Request) -> Result<Reply, Problem> {
    br::run_on(server, req, &mut |_, _, _| {})
}

fn show_row(r: &ResourceBranch) {
    let extra = match (r.changed_count, r.stashes) {
        (0, 0) => String::new(),
        (c, 0) => format!("  ({c} changed file(s))"),
        (0, s) => format!("  ({s} set(s) of changes kept aside)"),
        (c, s) => format!("  ({c} changed file(s), {s} set(s) kept aside)"),
    };
    let note = r.note.as_deref().filter(|_| matches!(r.state, State::Pinned | State::Unavailable)).map(|n| format!(": {n}")).unwrap_or_default();
    // Not on the normal version: marked, in the same words as `kora status` and the panel.
    let mark = if crate::cmd::resource::off_normal_line(r).is_some() { "!" } else { " " };
    println!("{mark} {:<28} {}{note}{extra}", r.id, r.label);
}

fn print_switched(r: &br::SwitchResult) -> i32 {
    let o = &r.outcome;
    println!("{}: {}", r.resource, o.message);
    if o.status == Status::NeedsChoice {
        for f in &o.now.changed_files {
            println!("    changed: {f}");
        }
        println!("Run it again with --stash to keep your changes aside, or --discard to throw them away.");
        return 3;
    }
    if let Some(s) = &o.stashed {
        println!("Your changes are kept aside as number {} ({}). Bring them back with `kora branch restore`.", s.index, s.message);
    }
    if let Some(note) = &r.restart_note {
        if r.needs_restart {
            println!("{note}");
        }
    }
    0
}

pub fn branch(cmd: BranchCmd) -> i32 {
    match cmd {
        BranchCmd::List { server } => match ask(&server, &Request::Overview) {
            Ok(Reply::Overview(rows)) => {
                rows.iter().filter(|r| r.state != State::Pinned).for_each(show_row);
                0
            }
            Ok(_) => 1,
            Err(p) => fail(&p),
        },
        BranchCmd::Show { server, resource, refresh } => match ask(&server, &Request::Choices { resource, refresh }) {
            Ok(Reply::Choices(c)) => {
                show_row(&c.resource);
                for n in &c.notes {
                    println!("note: {n}");
                }
                let list = |title: &str, v: &[String]| {
                    if !v.is_empty() {
                        println!("{title}: {}", v.join(", "));
                    }
                };
                list("In this folder", &c.branches.local);
                list("On your fork", &c.branches.fork);
                list("On the original", &c.branches.upstream);
                for s in &c.stashes {
                    println!("Kept aside #{}: {} (made on {})", s.index, s.message, s.branch);
                }
                0
            }
            Ok(_) => 1,
            Err(p) => fail(&p),
        },
        BranchCmd::Switch { server, resource, branch, from, changes } => {
            let Some(ch) = changes_of(&changes) else { return 0 };
            let target = Target::Branch { name: branch, remote: from };
            switch(&server, resource, target, ch)
        }
        BranchCmd::Default { server, resource: Some(resource), changes } => {
            let Some(ch) = changes_of(&changes) else { return 0 };
            switch(&server, resource, Target::Default, ch)
        }
        BranchCmd::Default { server, resource: None, changes } => {
            let Some(ch) = changes_of(&changes) else { return 0 };
            bulk_apply(&server, Target::Default, ch)
        }
        BranchCmd::All { server, branch, preview, no_fetch, changes } => {
            if preview {
                return match br::run_on(&server, &Request::BulkPreview { branch: branch.clone(), fetch: !no_fetch }, &mut |_, _, _| {}) {
                    Ok(Reply::BulkPreview(p)) => {
                        for r in &p.rows {
                            let what = match (&r.note, r.has, r.already) {
                                (Some(n), _, _) => format!("cannot check: {n}"),
                                (_, true, true) => "already on it".to_string(),
                                (_, true, false) => "has it".to_string(),
                                _ => "does not have it".to_string(),
                            };
                            println!("{:<28} {what}  (now: {})", r.id, r.now);
                        }
                        println!("{} have {}, {} do not.", p.have, branch, p.missing);
                        0
                    }
                    Ok(_) => 1,
                    Err(e) => fail(&e),
                };
            }
            let Some(ch) = changes_of(&changes) else { return 0 };
            bulk_apply(&server, Target::Branch { name: branch, remote: None }, ch)
        }
        BranchCmd::Restore { server, resource, index } => match ask(&server, &Request::RestoreStash { resource, index }) {
            Ok(Reply::Switched(r)) => print_switched(&r),
            Ok(_) => 1,
            Err(p) => fail(&p),
        },
    }
}

fn switch(server: &str, resource: String, target: Target, changes: Changes) -> i32 {
    match ask(server, &Request::Switch { resource, target, changes }) {
        Ok(Reply::Switched(r)) => print_switched(&r),
        Ok(_) => 1,
        Err(p) => fail(&p),
    }
}

fn bulk_apply(server: &str, target: Target, changes: Changes) -> i32 {
    let mut say = |done: usize, total: usize, id: &str| {
        if !id.is_empty() {
            eprintln!("[{done}/{total}] {id}");
        }
    };
    match br::run_on(server, &Request::BulkApply { target, only: None, changes }, &mut say) {
        Ok(Reply::BulkApply(r)) => {
            for i in &r.items {
                let mark = match i.status {
                    BulkStatus::Switched => "done",
                    BulkStatus::Already => "already there",
                    BulkStatus::NeedsChoice => "has changes of your own",
                    BulkStatus::Skipped => "left as it is",
                    BulkStatus::Failed => "failed",
                };
                println!("{:<28} {mark}: {}", i.id, i.message);
            }
            println!("{} changed, {} left as they are, {} need a choice, {} failed.", r.switched, r.skipped, r.needs_choice, r.failed);
            if let Some(n) = &r.restart_note {
                println!("{n}");
            }
            if r.failed > 0 {
                1
            } else if r.needs_choice > 0 {
                println!("Run it again with --stash or --discard to deal with the ones that have changes of your own.");
                3
            } else {
                0
            }
        }
        Ok(_) => 1,
        Err(p) => fail(&p),
    }
}

pub fn pr(cmd: PrCmd) -> i32 {
    match cmd {
        PrCmd::List { server, resource } => match ask(&server, &Request::Prs { resource }) {
            Ok(Reply::Prs(p)) => {
                for n in &p.notes {
                    println!("note: {n}");
                }
                for r in &p.prs {
                    let place = match r.source {
                        PrSource::Upstream => "original",
                        PrSource::Fork => "your fork",
                    };
                    println!("#{:<5} {}{}  by {}  [{place}: {}]", r.number, r.title, if r.draft { " (draft)" } else { "" }, r.author, r.repo);
                }
                if p.prs.is_empty() {
                    println!("No open pull requests.");
                }
                0
            }
            Ok(_) => 1,
            Err(e) => fail(&e),
        },
        PrCmd::Checkout { server, resource, pr, source, changes } => {
            let source = match source.as_deref() {
                None => None,
                Some("upstream") => Some(PrSource::Upstream),
                Some("fork") => Some(PrSource::Fork),
                Some(other) => {
                    eprintln!("error: --source must be upstream or fork, not '{other}'");
                    return 2;
                }
            };
            let Some(ch) = changes_of(&changes) else { return 0 };
            switch(&server, resource, Target::Pr { input: pr, source }, ch)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> Result<crate::Cli, clap::Error> {
        let mut v = vec!["kora"];
        v.extend_from_slice(args);
        crate::Cli::try_parse_from(v)
    }

    #[test]
    fn keeping_changes_aside_and_throwing_them_away_cannot_be_asked_together() {
        assert!(parse(&["branch", "switch", "s", "r", "feature", "--stash", "--discard"]).is_err());
        assert!(parse(&["branch", "switch", "s", "r", "feature", "--stash"]).is_ok());
        assert!(parse(&["branch", "default", "s", "--discard", "--yes"]).is_ok());
        assert!(parse(&["pr", "checkout", "s", "r", "https://github.com/o/n/pull/1", "--source", "fork", "--stash"]).is_ok());
    }

    #[test]
    fn the_choice_about_changes_defaults_to_asking() {
        assert_eq!(changes_of(&ChangeArgs::default()), Some(Changes::Ask));
        assert_eq!(changes_of(&ChangeArgs { stash: true, ..Default::default() }), Some(Changes::Stash));
        // `--yes` skips the question (without it the person is asked on the terminal, which a test cannot answer).
        assert_eq!(changes_of(&ChangeArgs { discard: true, yes: true, ..Default::default() }), Some(Changes::Discard));
    }

    #[test]
    fn a_server_that_is_not_registered_is_a_plain_error_and_a_non_zero_exit() {
        assert_eq!(branch(BranchCmd::List { server: "no-such-server".into() }), 1);
        assert_eq!(pr(PrCmd::List { server: "no-such-server".into(), resource: "kora_hud".into() }), 1);
    }
}
