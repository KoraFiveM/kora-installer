//! `kora resource show <server> <resource>`: the same facts as the panel's resource page: what the resource is, where
//! its files are, and whether the running server still has an old version of it.

use clap::Subcommand;
use kora_core::branches::{self as br, Reply, Request, State};
use kora_core::problem::plain_text;

#[derive(Subcommand)]
pub enum ResourceCmd {
    /// Show one resource: its repository, which version it is on, and whether a restart is needed.
    Show {
        /// Registry name, or the server folder.
        server: String,
        resource: String,
    },
}

pub fn resource(cmd: ResourceCmd) -> i32 {
    match cmd {
        ResourceCmd::Show { server, resource } => show(&server, &resource),
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
