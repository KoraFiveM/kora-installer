//! `kora resource show <server> <resource>`: the same facts as the panel's resource page: what the resource is, where
//! its files are, and whether the running server still has an old version of it.

use clap::{Args, Subcommand};
use kora_core::branches::{self as br, Reply, Request, State};
use kora_core::collab::{self, Checks, Reply as CReply, Request as CRequest};
use kora_core::installer::Versions;
use kora_core::problem::plain_text;

#[derive(Args, Clone)]
pub struct Target {
    /// Registry name, or the server folder.
    server: String,
    resource: String,
}

#[derive(Args, Clone)]
pub struct Text {
    /// The text to send (a message or a comment).
    #[arg(short = 'm', long)]
    message: String,
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum VersionsArg {
    Released,
    Latest,
}

#[derive(Subcommand)]
pub enum ResourceCmd {
    /// Show one resource: its repository, which version it is on, and whether a restart is needed.
    Show {
        /// Registry name, or the server folder.
        server: String,
        resource: String,
    },
    /// Show what changed in a resource's folder (files, and a readable diff with --diff).
    Changes {
        #[command(flatten)]
        t: Target,
        /// Also print the diff.
        #[arg(long)]
        diff: bool,
    },
    /// The latest commits of the branch the resource is on.
    Log {
        #[command(flatten)]
        t: Target,
    },
    /// Commit everything that changed in the resource.
    Commit {
        #[command(flatten)]
        t: Target,
        #[command(flatten)]
        m: Text,
    },
    /// Push the branch the resource is on to your own copy on GitHub (never forced). Acts as the GitHub account `gh` is signed in as.
    Push {
        #[command(flatten)]
        t: Target,
    },
    /// Make a new branch from where the resource is now (your changes come along).
    NewBranch {
        #[command(flatten)]
        t: Target,
        name: String,
    },
    /// Open issues of the resource's repository.
    Issues {
        #[command(flatten)]
        t: Target,
    },
    /// Comment on an issue, as the GitHub account `gh` is signed in as.
    IssueComment {
        #[command(flatten)]
        t: Target,
        number: u32,
        #[command(flatten)]
        m: Text,
    },
    /// Releases of the resource's repository, and which one this server is pinned to.
    Releases {
        #[command(flatten)]
        t: Target,
    },
    /// Make your own copy (a fork) of resources that are plain clones: all of them, or the ones named.
    Fork {
        /// Registry name, or the server folder.
        server: String,
        /// Only these resources.
        resources: Vec<String>,
        /// The GitHub user or organisation to fork into (default: the account `gh` is signed in as).
        #[arg(long, value_name = "LOGIN_OR_ORG")]
        owner: Option<String>,
    },
    /// Follow released versions or the latest changes. Nothing moves until you check for updates.
    Versions {
        /// Registry name, or the server folder.
        server: String,
        #[arg(value_enum)]
        versions: VersionsArg,
    },
}

pub fn resource(cmd: ResourceCmd) -> i32 {
    match cmd {
        ResourceCmd::Show { server, resource } => show(&server, &resource),
        ResourceCmd::Changes { t, diff } => collab_run(
            &t,
            |r| CRequest::Changes { resource: r },
            |reply| match reply {
                CReply::Changes(c) => {
                    println!("{} changed file(s) on {}", c.count, c.branch.as_deref().unwrap_or("a single commit"));
                    for f in &c.files {
                        println!("  {:<9} {}", f.status, f.path);
                    }
                    match (c.ahead, c.behind) {
                        (Some(a), Some(b)) => println!("{a} commit(s) not pushed yet, {b} to get from GitHub"),
                        _ => println!("This branch is not on GitHub yet."),
                    }
                    if diff {
                        println!("\n{}{}", c.diff, if c.diff_truncated { "\n(the diff is cut here)" } else { "" });
                    }
                }
                _ => unreachable_reply(),
            },
        ),
        ResourceCmd::Log { t } => collab_run(
            &t,
            |r| CRequest::Log { resource: r },
            |reply| {
                if let CReply::Log(l) = reply {
                    for c in l.commits {
                        println!("{} {}  ({})", &c.sha[..7], c.subject, c.author);
                    }
                }
            },
        ),
        ResourceCmd::Commit { t, m } => collab_run(
            &t,
            |r| CRequest::Commit { resource: r, message: m.message.clone() },
            |reply| {
                if let CReply::Committed(c) = reply {
                    println!("Saved {} file(s) as commit {}.", c.files, &c.commit[..7]);
                }
            },
        ),
        ResourceCmd::Push { t } => collab_run(
            &t,
            |r| CRequest::Push { resource: r },
            |reply| {
                if let CReply::Pushed(p) = reply {
                    println!("Pushed {} to {}{} as {}.", p.branch, p.to, if p.new_branch { " (a new branch there)" } else { "" }, p.acting_as.as_deref().unwrap_or("the signed-in GitHub account"));
                }
            },
        ),
        ResourceCmd::NewBranch { t, name } => collab_run(&t, |r| CRequest::NewBranch { resource: r, name: name.clone() }, |_| println!("Now on branch {name}. Your changes came with you.")),
        ResourceCmd::Issues { t } => collab_run(
            &t,
            |r| CRequest::Issues { resource: r },
            |reply| {
                if let CReply::Issues(i) = reply {
                    println!("Open issues of {}:", i.repo);
                    for x in i.issues {
                        println!("  #{:<5} {}  ({}, {} comment(s))", x.number, x.title, x.author, x.comments);
                    }
                    println!("Report a new one: {}", i.new_issue_url);
                }
            },
        ),
        ResourceCmd::IssueComment { t, number, m } => collab_run(&t, |r| CRequest::Comment { resource: r, kind: "issue".into(), number, body: m.message.clone(), source: None }, print_acted),
        ResourceCmd::Releases { t } => collab_run(
            &t,
            |r| CRequest::Releases { resource: r },
            |reply| {
                if let CReply::Releases(r) = reply {
                    println!("Releases of {}:", r.repo);
                    for x in r.releases {
                        let tag = if x.pinned {
                            " (this server)"
                        } else if x.newer {
                            " (newer)"
                        } else {
                            ""
                        };
                        println!("  {}{tag}  {}", x.tag, x.name);
                    }
                }
            },
        ),
        ResourceCmd::Fork { server, resources, owner } => {
            let only = (!resources.is_empty()).then_some(resources);
            match collab::run_for(&server, &CRequest::Fork { owner, only }, &mut |done, total, id| {
                if !id.is_empty() {
                    println!("[{done}/{total}] {id}");
                }
            }) {
                Ok(CReply::Forked(o)) => {
                    for f in &o.forked {
                        println!("{}", f.message);
                    }
                    for s in &o.skipped {
                        println!("skipped: {s}");
                    }
                    for f in &o.failed {
                        eprintln!("{}: {}", f.resource, plain_text(&f.problem.encode()));
                    }
                    if o.server_is_fork {
                        println!("This server now uses your own copies (in {}).", o.owner);
                    }
                    i32::from(!o.failed.is_empty())
                }
                Ok(_) => 1,
                Err(p) => fail(&p),
            }
        }
        ResourceCmd::Versions { server, versions } => {
            let v = match versions {
                VersionsArg::Released => Versions::Released,
                VersionsArg::Latest => Versions::Latest,
            };
            match collab::run_for(&server, &CRequest::SetVersions { versions: v }, &mut |_, _, _| {}) {
                Ok(CReply::Versions(v)) => {
                    println!("{}", v.message);
                    0
                }
                Ok(_) => 1,
                Err(p) => fail(&p),
            }
        }
    }
}

fn fail(p: &kora_core::problem::Problem) -> i32 {
    eprintln!("error: {}", plain_text(&p.encode()));
    1
}

fn unreachable_reply() {
    eprintln!("error: unexpected answer");
}

fn print_acted(reply: CReply) {
    if let CReply::Acted(a) = reply {
        println!("{}{}", a.message, a.acting_as.map(|l| format!(" (as {l})")).unwrap_or_default());
    }
}

/// Run one collab request on `t.server`'s resource and let `show` print the answer; exit 1 with the problem otherwise.
fn collab_run(t: &Target, build: impl Fn(String) -> CRequest, show: impl FnOnce(CReply)) -> i32 {
    match collab::run_for(&t.server, &build(t.resource.clone()), &mut |_, _, _| {}) {
        Ok(r) => {
            show(r);
            0
        }
        Err(p) => fail(&p),
    }
}

/// How a pull request's checks read in one word.
pub(crate) fn checks_word(c: Checks) -> &'static str {
    match c {
        Checks::Passing => "passing",
        Checks::Failing => "failing",
        Checks::Pending => "pending",
        Checks::None => "none",
    }
}

fn show(server: &str, resource: &str) -> i32 {
    let page = match br::run_on(server, &Request::Page { resource: resource.to_string() }, &mut |_, _, _| {}) {
        Ok(Reply::Page(p)) => p,
        Ok(_) => return 1,
        Err(p) => {
            eprintln!("error: {}", plain_text(&p.encode()));
            return 1;
        }
    };
    println!("{}", page.id);
    println!("  repository:  {}", page.repository);
    println!("  folder:      resources/{}", page.folder);
    match (&page.pinned_commit, page.branch.state) {
        (Some(c), _) => println!("  release:     pinned to {}", &c[..c.len().min(7)]),
        (None, State::Pinned | State::Unavailable) => {}
        (None, _) => println!("  follows:     {}", page.default_branch),
    }
    println!("  now on:      {}", page.branch.label);
    if let Some(note) = page.branch.note.as_deref().filter(|_| matches!(page.branch.state, State::Pinned | State::Unavailable)) {
        println!("               {note}");
    }
    if !matches!(page.branch.state, State::Default | State::Pinned | State::Unavailable) {
        println!("  NOT ON ITS NORMAL VERSION: {}", page.branch.label);
    }
    let Ok(sf) = kora_core::update::manual::load_server(server) else {
        return 0;
    };
    let running = kora_core::installer::registry::Registry::load()
        .ok()
        .and_then(|r| r.find(server).cloned())
        .is_some_and(|e| kora_core::api::Backend::status(&mut kora_core::api::system::SystemBackend, &e) == kora_core::api::Status::Running);
    let info = kora_core::resourcestate::info(&sf.dir, &page.id, running);
    if info.needed {
        println!("  {}", kora_core::resourcestate::NOTICE);
        if let Some(why) = info.why {
            println!("               ({why})");
        }
    }
    0
}

/// One line for a resource that is not on its normal version (the same words in `kora status`, `kora branch list` and
/// `kora resource show`): "! kora_hud: pull request #12 (not its normal version)". `None` for a normal one.
pub(crate) fn off_normal_line(r: &br::ResourceBranch) -> Option<String> {
    matches!(r.state, State::Branch | State::Pr | State::Detached).then(|| format!("! {}: {} (not on its normal version, {})", r.id, r.label, r.default_branch))
}

/// What needs a person's eye on one server: resources not on their normal version and resources waiting for a restart.
pub(crate) fn attention_lines(server: &str, running: bool) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(Reply::Overview(rows)) = br::run_on(server, &Request::Overview, &mut |_, _, _| {}) {
        out.extend(rows.iter().filter_map(off_normal_line));
    }
    if let Ok(sf) = kora_core::update::manual::load_server(server) {
        for id in kora_core::resourcestate::pending(&sf.dir, running) {
            out.push(format!("! {id}: {}", kora_core::resourcestate::NOTICE));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(state: State, label: &str) -> br::ResourceBranch {
        br::ResourceBranch {
            id: "kora_hud".into(),
            folder: "kora_hud".into(),
            state,
            branch: None,
            pr: None,
            default_branch: "dev".into(),
            label: label.into(),
            changed_count: 0,
            stashes: 0,
            note: None,
            problem: None,
        }
    }

    #[test]
    fn only_a_resource_off_its_normal_version_is_marked() {
        assert_eq!(off_normal_line(&row(State::Pr, "pull request #12")).as_deref(), Some("! kora_hud: pull request #12 (not on its normal version, dev)"));
        assert!(off_normal_line(&row(State::Branch, "branch fix")).is_some());
        for s in [State::Default, State::Pinned, State::Unavailable] {
            assert!(off_normal_line(&row(s, "x")).is_none());
        }
    }
}
