//! `kora add-resources` and `kora add-asset`: put more resources on an installed server.

use crate::ui::{confirm, server_dir};
use crate::CODE_REMOVED;
use std::path::PathBuf;

use clap::{Args, Parser};
use kora_core::installer::Mode;

#[derive(Args, Debug, Default)]
pub(crate) struct AddResourcesArgs {
    /// Server name or folder.
    pub(crate) server: String,
    /// REMOVED in 0.4.0 (setup codes no longer exist): use --resource or `kora panel`.
    /// Hidden from help; it still parses so an old script gets the explanation instead of a usage error.
    #[arg(long, value_name = "CODE", hide = true)]
    pub(crate) code: Option<String>,
    /// Catalog resource id to add (repeatable). Dependencies are added automatically.
    #[arg(long = "resource", value_name = "ID")]
    pub(crate) resource: Vec<String>,
    /// Show what would be added, then stop.
    #[arg(long)]
    pub(crate) dry_run: bool,
    /// Do not ask for confirmation.
    #[arg(long, short = 'y')]
    pub(crate) yes: bool,
}

#[derive(Args, Debug)]
pub(crate) struct AddAssetArgs {
    /// Server name or folder.
    pub(crate) server: String,
    /// The .zip file(s) you downloaded from the Cfx.re Portal (as they are; do not unpack them).
    #[arg(required = true, value_name = "ZIP")]
    pub(crate) zip: Vec<std::path::PathBuf>,
    /// Show what would be imported, then stop.
    #[arg(long)]
    pub(crate) dry_run: bool,
    /// Do not ask for confirmation.
    #[arg(long, short = 'y')]
    pub(crate) yes: bool,
}

fn print_asset_plan(p: &kora_core::paidassets::ImportPlan) {
    use kora_core::paidassets::{human, CATEGORY};
    println!("{}:", p.zip_name);
    for r in &p.resources {
        if !r.has_fxmanifest {
            continue;
        }
        let mut tags = vec![human(r.size)];
        if let Some(v) = &r.version {
            tags.push(format!("version {v}"));
        }
        tags.push(if r.escrowed { "escrowed".into() } else { "not escrowed".into() });
        if r.escrowed {
            tags.push(if r.has_fxap { ".fxap present".into() } else { ".fxap MISSING".into() });
        }
        let what = if r.replaces { "replaces the imported copy (kept as a backup)" } else { "new" };
        println!("  {}  ->  resources/{CATEGORY}/{}  ({}; {what})", r.name, r.name, tags.join(", "));
    }
    for pr in &p.problems {
        println!("  {} {}", if pr.blocking { "[!!]" } else { "note:" }, pr.message);
    }
}

/// `kora add-asset <server> <zip>...`: import paid assets downloaded from the Cfx.re Portal.
pub(crate) fn add_asset(a: &AddAssetArgs) -> i32 {
    use kora_core::paidassets as pa;
    let dir = match server_dir(&a.server) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let plans: Vec<(std::path::PathBuf, pa::ImportPlan)> = a.zip.iter().map(|z| (z.clone(), pa::inspect(&dir, z))).collect();
    for (_, p) in &plans {
        print_asset_plan(p);
    }
    let ok: Vec<&(std::path::PathBuf, pa::ImportPlan)> = plans.iter().filter(|(_, p)| p.can_import).collect();
    if ok.len() < plans.len() {
        eprintln!("{} of {} zip(s) can't be imported; see [!!] above.", plans.len() - ok.len(), plans.len());
    }
    if ok.is_empty() {
        println!("Get your assets from {}", pa::PORTAL_URL);
        return 1;
    }
    println!("The server's licence key must belong to the Cfx.re account that bought these assets.");
    if a.dry_run {
        println!("Dry run: nothing was changed.");
        return 0;
    }
    if !a.yes && !confirm("Import these now?") {
        eprintln!("Cancelled.");
        return 1;
    }
    let mut failed = plans.len() - ok.len();
    for (zip, p) in ok {
        match pa::import(&dir, zip, &pa::ImportOpts::default(), &mut |m| println!("{m}...")) {
            Ok(r) => {
                for i in &r.imported {
                    let v = i.version.as_deref().map(|v| format!(" {v}")).unwrap_or_default();
                    println!("[ok] {}{v}{}", i.name, if i.replaced { " (replaced; the old copy is kept as a backup)" } else { "" });
                }
                for n in &r.ensured {
                    println!("Added `ensure {n}` to server.cfg.");
                }
                for n in &r.notes {
                    println!("note: {n}");
                }
            }
            Err(e) => {
                eprintln!("[!!] {}: {e}", p.zip_name);
                failed += 1;
            }
        }
    }
    if failed > 0 {
        1
    } else {
        0
    }
}

/// `kora add-resources <server> --resource ID...`. Browsing the catalog sections
/// (with per-creator drill-in) is in the panel: a server's Resources tab.
pub(crate) fn add_resources(a: AddResourcesArgs) -> i32 {
    if a.code.is_some() {
        eprintln!("{CODE_REMOVED}");
        return 2;
    }
    use kora_core::addresources as add;
    use kora_core::git::SystemRunner;
    let mut server = match kora_core::update::manual::load_server(&a.server) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let ids: Vec<String> = if a.resource.is_empty() {
        eprintln!("error: say what to add: --resource <ID> (repeatable). To browse the catalog, run `kora` and use the server's Resources tab.");
        return 2;
    } else {
        a.resource.clone()
    };
    let (cat, note) = match add::load_catalog() {
        Ok(x) => x,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    if let Some(n) = note {
        println!("note: {n}");
    }
    let plan = match add::plan(&cat, &server, &ids) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    for id in &plan.already {
        println!("{id} is already installed.");
    }
    if plan.is_empty() {
        println!("Nothing to add.");
        return 0;
    }
    println!("Will add ({} mode):", if server.mode == Mode::Development { "development" } else { "production" });
    for i in &plan.items {
        let how = if i.third_party {
            "pinned, never forked"
        } else if i.fork {
            "your fork"
        } else if i.commit.is_some() {
            "pinned"
        } else {
            "branch head"
        };
        println!("  {}{}  ->  resources/{}  ({how})", i.id, if i.dependency { " (needed by another)" } else { "" }, i.rel);
    }
    println!("server.cfg gets `ensure` lines for: {}", plan.ensure.join(", "));
    if a.dry_run {
        println!("Dry run: nothing was changed.");
        return 0;
    }
    if !a.yes && !confirm("Add these now?") {
        eprintln!("Cancelled.");
        return 1;
    }
    if plan.items.iter().any(|i| i.fork) && !kora_core::git::is_signed_in(&SystemRunner) {
        println!("Forking needs you to be signed in to GitHub.");
        if let Err(e) = kora_core::git::auth_login_interactive() {
            eprintln!("error: could not sign in to GitHub ({e}).");
            return 1;
        }
    }
    let cloner = add::GitCloner(&SystemRunner);
    match add::execute(&cloner, &mut server, &plan, &mut |m| println!("{m}...")) {
        Ok(out) => {
            for id in &out.installed {
                println!("[ok] {id}");
            }
            for (id, e) in &out.failed {
                eprintln!("[!!] {id}: {e}");
            }
            for n in &out.notes {
                println!("note: {n}");
            }
            if !out.ensured.is_empty() {
                println!("Added to server.cfg: {}", out.ensured.iter().map(|f| format!("ensure {f}")).collect::<Vec<_>>().join(", "));
            }
            if !out.installed.is_empty() {
                println!("Restart the server to load the new resources.");
            }
            i32::from(!out.ok())
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Cli;
    use crate::Command;
    use clap::Parser;

    fn parse_add(args: &[&str]) -> Result<AddResourcesArgs, clap::Error> {
        let mut v = vec!["kora", "add-resources"];
        v.extend_from_slice(args);
        match Cli::try_parse_from(v)?.command {
            Some(Command::AddResources(a)) => Ok(a),
            _ => unreachable!(),
        }
    }

    #[test]
    fn add_asset_arguments_and_help() {
        let parse = |args: &[&str]| {
            let mut v = vec!["kora", "add-asset"];
            v.extend_from_slice(args);
            Cli::try_parse_from(v).map(|c| match c.command {
                Some(Command::AddAsset(a)) => a,
                _ => unreachable!(),
            })
        };
        let a = parse(&["srv", "a.zip", "b.zip", "--dry-run"]).unwrap();
        assert_eq!((a.server.as_str(), a.zip.len(), a.dry_run, a.yes), ("srv", 2, true, false));
        assert!(parse(&["srv", "a.zip", "-y"]).unwrap().yes);
        assert!(parse(&["srv"]).is_err(), "at least one zip");
        let help = <Cli as clap::CommandFactory>::command().find_subcommand_mut("add-asset").unwrap().render_long_help().to_string();
        assert!(help.contains("https://portal.cfx.re/assets/granted-assets"), "{help}");
    }

    #[test]
    fn add_resources_arguments() {
        let a = parse_add(&["srv", "--resource", "a", "--resource", "b", "--dry-run"]).unwrap();
        assert_eq!((a.server.as_str(), a.resource.len(), a.dry_run, a.code.is_none()), ("srv", 2, true, true));
        let a = parse_add(&["srv", "--code", "KORA-7F3K-92QX", "-y"]).unwrap();
        assert!(a.code.is_some() && a.yes, "--code still parses (it fails at run time with exit 2)");
        assert_eq!(add_resources(a), 2, "setup codes were removed");
        assert!(parse_add(&[]).is_err(), "the server is required");
    }
}
