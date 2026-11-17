//! `kora support bundle [<server>]`: one file to hand to whoever is helping, with every secret removed.
//! The same bundle as the panel's "Create a support bundle" (`Op::SupportBundle*`): the person sees what is in it
//! before anything is saved, can read any file, and it is never uploaded.

use clap::Subcommand;
use kora_core::api::{Backend, SystemBackend};
use kora_core::problem::plain_text;
use kora_core::space::human;
use kora_core::support::service;

#[derive(Subcommand, Debug)]
pub(crate) enum SupportCmd {
    /// Prepare a support bundle, show what is in it, and save it on this computer after you agree.
    Bundle {
        /// Only this server's files (default: every server).
        server: Option<String>,
        /// Print the cleaned text of one file of the bundle (for example servers/prod/server.cfg) and stop.
        #[arg(long, value_name = "FILE")]
        show: Option<String>,
        /// Save it without asking.
        #[arg(long)]
        yes: bool,
    },
}

pub(crate) fn run(cmd: SupportCmd) -> i32 {
    let SupportCmd::Bundle { server, show, yes } = cmd;
    let mut b = SystemBackend;
    if let Some(s) = &server {
        match kora_core::installer::registry::Registry::load().map(|r| r.find(s).map(|e| e.name.clone())) {
            Ok(Some(_)) => {}
            _ => {
                eprintln!("error: no registered server named '{s}' (see `kora status`)");
                return 1;
            }
        }
    }
    let name = server.as_ref().and_then(|s| kora_core::installer::registry::Registry::load().ok().and_then(|r| r.find(s).map(|e| e.name.clone())));
    let listing = match b.support_preview(name.as_deref()) {
        Ok(l) => l,
        Err(m) => {
            eprintln!("error: {}", plain_text(&m));
            return 1;
        }
    };
    if let Some(f) = show {
        return match service::file_text(&listing.id, &f) {
            Ok(t) => {
                println!("{t}");
                0
            }
            Err(m) => {
                eprintln!("error: {}", plain_text(&m));
                1
            }
        };
    }
    println!("This support bundle would hold ({}):", human(listing.total_bytes));
    for f in &listing.files {
        let hid = if f.redactions == 0 { String::new() } else { format!("  [{} value{} hidden: {}]", f.redactions, if f.redactions == 1 { "" } else { "s" }, f.hidden.join(", ")) };
        println!("  {}  ({}){hid}", f.name, human(f.bytes));
    }
    if !listing.omitted.is_empty() {
        println!("Left out altogether:");
        for o in &listing.omitted {
            println!("  {}: it {}", o.name, o.why);
        }
    }
    println!("{}", listing.never_uploaded);
    println!("To read one file first: kora support bundle {}--show <file>", server.as_ref().map(|s| format!("{s} ")).unwrap_or_default());
    if !yes && !crate::ui::confirm(&format!("Save it in {}?", listing.folder)) {
        println!("Nothing was saved.");
        return 0;
    }
    match service::save(&listing.id) {
        Ok(s) => {
            println!("Saved: {} ({})", s.path, human(s.bytes));
            println!("Send that one file to whoever is helping you.");
            0
        }
        Err(m) => {
            eprintln!("error: {}", plain_text(&m));
            1
        }
    }
}
