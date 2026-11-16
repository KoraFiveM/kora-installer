//! Every `kora` command and flag is accounted for: it is served by an operation of the shared list
//! (and so by the panel), or it is deliberately CLI-only, or it has been removed. Adding a flag without
//! deciding which fails `every_cli_item_is_classified`; deleting one fails `no_stale_entries`.
//!
//! Keys: a command is its path (`webhook set`), a flag or argument is the command path plus the flag
//! (`install --game-port`, `webhook set <url>`). The top-level flags have no command path.
use std::collections::BTreeSet;

use clap::CommandFactory;
use kora_core::api::Op;

#[derive(Debug, Clone, Copy)]
enum Entry {
    /// The panel does this through this operation.
    Route(Op),
    /// Only makes sense in a terminal or on the machine itself.
    CliOnly(&'static str),
    /// Still accepted for old scripts, but only to say it is gone.
    Removed(&'static str),
}
use Entry::{CliOnly, Removed, Route};

const TABLE: &[(&str, Entry)] = &[
    (
        "--no-elevate",
        CliOnly("the panel runs as the signed-in user and never relaunches itself"),
    ),
    ("install", Route(Op::InstallStart)),
    (
        "install --code",
        Removed("setup codes no longer exist (0.4.0)"),
    ),
    ("install --name", Route(Op::InstallStart)),
    ("install --dir", Route(Op::InstallStart)),
    ("install --mode", Route(Op::InstallStart)),
    ("install --license-key", Route(Op::InstallStart)),
    ("install --game-port", Route(Op::InstallStart)),
    ("install --txadmin-port", Route(Op::InstallStart)),
    ("install --txadmin-public", Route(Op::InstallStart)),
    ("install --cfx-identifier", Route(Op::InstallStart)),
    ("install --artifact-url", Route(Op::InstallStart)),
    ("install --resources", Route(Op::InstallStart)),
    ("install --db", Route(Op::InstallStart)),
    ("install --service", Route(Op::InstallStart)),
    ("install --firewall", Route(Op::InstallStart)),
    ("install --github-owner", Route(Op::InstallStart)),
    (
        "install --catalog",
        CliOnly(
            "hidden developer override of the catalog source; the panel reads the catalog itself",
        ),
    ),
    ("install --db-admin-user", Route(Op::InstallStart)),
    ("install --db-admin-host", Route(Op::InstallStart)),
    ("install --db-admin-port", Route(Op::InstallStart)),
    ("install --start", Route(Op::InstallStart)),
    ("install --no-start", Route(Op::InstallStart)),
    ("install --dry-run", Route(Op::InstallPlan)),
    ("install --no-app", Route(Op::InstallStart)),
    ("install --no-desktop-shortcut", Route(Op::InstallStart)),
    ("install --auto-ports", Route(Op::CheckPorts)),
    ("install --hostname", Route(Op::InstallStart)),
    ("install --game-build", Route(Op::InstallStart)),
    ("install --yes", CliOnly("the panel confirms in the UI")),
    (
        "manager",
        CliOnly("opens the terminal manager; the panel is the web equivalent"),
    ),
    ("panel", CliOnly("starts the panel itself")),
    ("panel --port", CliOnly("starts the panel itself")),
    ("panel --no-open", CliOnly("starts the panel itself")),
    ("panel --install-service", CliOnly("sets up how the panel itself is started, so it cannot be done from inside it")),
    ("panel --remove-service", CliOnly("sets up how the panel itself is started, so it cannot be done from inside it")),
    ("panel --print-link", CliOnly("prints the link used to open the panel")),
    ("update", Route(Op::UpdateApply)),
    ("update --auto", Route(Op::UpdateAuto)),
    ("update <server>", Route(Op::UpdateApply)),
    ("update --resource", Route(Op::UpdateApply)),
    ("update --all", Route(Op::UpdateApply)),
    ("update --preview", Route(Op::UpdatePreview)),
    ("update --yes", CliOnly("the panel confirms in the UI")),
    ("update --restart", Route(Op::UpdateApply)),
    ("update --rollback", Route(Op::SnapshotRollback)),
    ("update --list-snapshots", Route(Op::SnapshotsList)),
    ("add-resources", Route(Op::ResourcesAdd)),
    ("add-resources <server>", Route(Op::ResourcesAdd)),
    (
        "add-resources --code",
        Removed("setup codes no longer exist (0.4.0)"),
    ),
    ("add-resources --resource", Route(Op::ResourcesAdd)),
    ("add-resources --dry-run", Route(Op::ResourcesPlan)),
    (
        "add-resources --yes",
        CliOnly("the panel confirms in the UI"),
    ),
    ("status", Route(Op::ServersList)),
    ("self-update", Route(Op::SelfUpdateApply)),
    ("self-update --check", Route(Op::SelfUpdateCheck)),
    ("self-update --yes", CliOnly("the panel confirms in the UI")),
    ("self-update --version", Route(Op::SelfUpdateApply)),
    ("webhook", Route(Op::WebhookGet)),
    ("webhook set", Route(Op::WebhookSet)),
    ("webhook set <server>", Route(Op::WebhookSet)),
    ("webhook set <url>", Route(Op::WebhookSet)),
    (
        "webhook set --stdin",
        CliOnly("the panel takes the URL in the request body, not from a terminal"),
    ),
    ("webhook set --test", Route(Op::WebhookTest)),
    ("webhook test", Route(Op::WebhookTest)),
    ("webhook test <server>", Route(Op::WebhookTest)),
    ("webhook show", Route(Op::WebhookGet)),
    ("webhook show <server>", Route(Op::WebhookGet)),
    ("webhook remove", Route(Op::WebhookRemove)),
    ("webhook remove <server>", Route(Op::WebhookRemove)),
    ("delete", Route(Op::ServerDelete)),
    ("delete <server>", Route(Op::ServerDelete)),
    ("delete --keep-files", Route(Op::ServerDelete)),
    ("delete --keep-db", Route(Op::ServerDelete)),
    ("delete --yes", CliOnly("the panel confirms in the UI")),
    ("schedule", Route(Op::ScheduleGet)),
    ("schedule show", Route(Op::ScheduleGet)),
    ("schedule show <server>", Route(Op::ScheduleGet)),
    ("schedule set", Route(Op::ScheduleSet)),
    ("schedule set <server>", Route(Op::ScheduleSet)),
    ("schedule set --daily", Route(Op::ScheduleSet)),
    ("schedule set --weekly", Route(Op::ScheduleSet)),
    ("schedule set --as-user", Route(Op::ScheduleSet)),
    ("schedule set --as-system", Route(Op::ScheduleSet)),
    ("schedule off", Route(Op::ScheduleRemove)),
    ("schedule off <server>", Route(Op::ScheduleRemove)),
    ("app", Route(Op::AppStatus)),
    ("app install", Route(Op::AppInstall)),
    ("app install --yes", CliOnly("the panel confirms in the UI")),
    ("app install --no-desktop-shortcut", Route(Op::AppInstall)),
    ("app install --force", Route(Op::AppInstall)),
    ("app install --dry-run", Route(Op::AppInstall)),
    ("app status", Route(Op::AppStatus)),
    ("app uninstall", Route(Op::AppUninstall)),
    (
        "app uninstall --yes",
        CliOnly("the panel confirms in the UI"),
    ),
    ("app uninstall --dry-run", Route(Op::AppUninstall)),
    ("github", Route(Op::GithubOwners)),
    ("github owners", Route(Op::GithubOwners)),
    (
        "service-host",
        CliOnly("run by the operating system service, never by a person"),
    ),
    (
        "service-host <name>",
        CliOnly("run by the operating system service, never by a person"),
    ),
    ("start", Route(Op::ServerControl)),
    ("start <server>", Route(Op::ServerControl)),
    ("stop", Route(Op::ServerControl)),
    ("stop <server>", Route(Op::ServerControl)),
    ("restart", Route(Op::ServerControl)),
    ("restart <server>", Route(Op::ServerControl)),
    ("uninstall", Route(Op::UninstallRun)),
    ("uninstall <server>", Route(Op::UninstallRun)),
    ("uninstall --dry-run", Route(Op::UninstallPlan)),
    ("uninstall --yes", CliOnly("the panel confirms in the UI")),
    ("uninstall --delete-files", Route(Op::UninstallRun)),
    ("uninstall --delete-databases", Route(Op::UninstallRun)),
    ("uninstall --app", Route(Op::AppUninstall)),
];

fn walk(c: &clap::Command, prefix: &str, out: &mut BTreeSet<String>) {
    for a in c.get_arguments() {
        if matches!(
            a.get_action(),
            clap::ArgAction::Help | clap::ArgAction::Version
        ) {
            continue;
        }
        let name = if a.is_positional() {
            format!("<{}>", a.get_id())
        } else {
            format!("--{}", a.get_long().unwrap_or(a.get_id().as_str()))
        };
        out.insert(format!("{prefix} {name}").trim().to_string());
    }
    for s in c.get_subcommands().filter(|s| s.get_name() != "help") {
        let path = format!("{prefix} {}", s.get_name()).trim().to_string();
        out.insert(path.clone());
        walk(s, &path, out);
    }
}

fn cli_items() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    walk(&crate::Cli::command(), "", &mut out);
    out
}

#[test]
fn every_cli_item_is_classified() {
    let listed: BTreeSet<&str> = TABLE.iter().map(|(k, _)| *k).collect();
    let missing: Vec<String> = cli_items()
        .into_iter()
        .filter(|i| !listed.contains(i.as_str()))
        .collect();
    assert!(missing.is_empty(), "add these to TABLE in crates/kora-cli/src/parity.rs (Route(Op::..), CliOnly or Removed):\n{}", missing.join("\n"));
}

#[test]
fn no_stale_entries() {
    let items = cli_items();
    let stale: Vec<&str> = TABLE
        .iter()
        .map(|(k, _)| *k)
        .filter(|k| !items.contains(*k))
        .collect();
    assert!(
        stale.is_empty(),
        "the CLI no longer has these; remove them from TABLE:\n{}",
        stale.join("\n")
    );
    let mut seen = BTreeSet::new();
    for (k, _) in TABLE {
        assert!(seen.insert(*k), "duplicate entry {k}");
    }
}

#[test]
fn routes_exist_and_reasons_are_given() {
    for (k, e) in TABLE {
        match e {
            Route(op) => assert!(Op::ALL.contains(op), "{k}: {op:?} is not in Op::ALL"),
            CliOnly(why) | Removed(why) => assert!(why.len() > 8, "{k}: give a real reason"),
        }
    }
}
