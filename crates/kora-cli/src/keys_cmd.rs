//! `kora keys ...` (saved licence keys) and `kora profile ...` (what Kora remembers: Cfx.re name and
//! GitHub owner). The same operations as the panel's Settings → Licence keys.
//!
//! A key is never taken from the command line (it would show in the process list and the shell
//! history): it is read from the `CFX_LICENSE_KEY` environment variable, or from standard input
//! (one line; piped, or typed when run in a terminal). Listings show a key as its name and last four
//! characters only.

use std::io::{BufRead, IsTerminal};

use clap::Subcommand;
use kora_core::installer::paused::{self, PausedStore};
use kora_core::installer::registry;
use kora_core::keystore::KeyStore;
use kora_core::problem::{plain_text, Problem};
use kora_core::profile::{Field, ProfileStore};

#[derive(Subcommand)]
pub enum KeysCmd {
    /// List the saved keys (name and the last 4 characters only) and which servers use them.
    List,
    /// Save a licence key under a name. The key is read from CFX_LICENSE_KEY or standard input, never from the command line.
    Add {
        /// A name you will recognise, e.g. "Main" or "Test server".
        name: String,
    },
    /// Put a different key under an existing name (read from CFX_LICENSE_KEY or standard input). Installed servers keep their own key.
    Replace {
        name: String,
    },
    /// Give a saved key another name.
    Rename {
        name: String,
        new_name: String,
    },
    /// Forget a saved key. Servers installed with it keep working: they have their own copy.
    Remove {
        name: String,
        /// Do not ask for confirmation.
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Change the licence key of an installed server to a saved key (restart the server afterwards if it is running).
    Use {
        /// Registry name of the server.
        server: String,
        /// The saved key to use.
        name: String,
    },
}

#[derive(Subcommand)]
pub enum ProfileCmd {
    /// Show what Kora remembers between installs (Cfx.re name, its identifier, GitHub owner). Nothing secret.
    Show,
    /// Forget it all: the next install asks again.
    Forget,
}

fn say(p: Problem) -> i32 {
    let s: String = p.into();
    eprintln!("error: {}", plain_text(&s));
    1
}

/// The key to save: from the environment, else one line of standard input.
fn read_key() -> Result<String, String> {
    if let Some(k) = std::env::var("CFX_LICENSE_KEY").ok().filter(|k| !k.trim().is_empty()) {
        return Ok(k);
    }
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        eprintln!("Paste the licence key and press Enter (it is shown as you type; do not do this where others can see the screen):");
    }
    let mut line = String::new();
    stdin.lock().read_line(&mut line).map_err(|e| format!("could not read the key: {e}"))?;
    if line.trim().is_empty() {
        return Err("no key was given: set CFX_LICENSE_KEY or pipe the key in on standard input.".into());
    }
    Ok(line)
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

pub fn keys(cmd: KeysCmd) -> i32 {
    let store = KeyStore::default();
    match cmd {
        KeysCmd::List => {
            let servers = registry::Registry::load().map(|r| r.servers).unwrap_or_default();
            match store.list_with_servers(&servers) {
                Ok(list) if list.is_empty() => {
                    println!("No saved licence keys. Add one with `kora keys add <name>` (the key is read from CFX_LICENSE_KEY or standard input).");
                    0
                }
                Ok(list) => {
                    for (k, used) in list {
                        let used = if used.is_empty() { String::new() } else { format!("  used by: {}", used.join(", ")) };
                        println!("{}  (key ending {}){used}", k.name, k.last4);
                    }
                    0
                }
                Err(p) => say(p),
            }
        }
        KeysCmd::Add { name } => match read_key().map_err(Problem::generic).and_then(|k| store.add(&name, &k)) {
            Ok(k) => {
                println!("Saved the key ending {} as \"{}\".", k.last4, k.name);
                0
            }
            Err(p) => say(p),
        },
        KeysCmd::Replace { name } => match read_key().map_err(Problem::generic).and_then(|k| store.replace(&name, &k)) {
            Ok(k) => {
                println!("\"{}\" now holds the key ending {}. Servers that were already installed keep the key they have: use `kora keys use <server> \"{}\"` to switch one.", k.name, k.last4, k.name);
                0
            }
            Err(p) => say(p),
        },
        KeysCmd::Rename { name, new_name } => match store.rename(&name, &new_name) {
            Ok(k) => {
                let n = PausedStore::default().rename_key(&name, &k.name) + usize::from(paused::rename_key_in_saved_answers(&kora_core::installer::saved::path(), &name, &k.name));
                println!("Renamed \"{name}\" to \"{}\".", k.name);
                if n > 0 {
                    println!("{} now use the new name.", plural(n, "unfinished install", "unfinished installs"));
                }
                0
            }
            Err(p) => say(p),
        },
        KeysCmd::Remove { name, yes } => {
            if let Err(p) = store.get(&name).map(drop) {
                return say(p);
            }
            let waiting = PausedStore::default().count_key(&name) + usize::from(paused::saved_answers_use_key(&kora_core::installer::saved::path(), &name));
            if !yes {
                if !std::io::stdin().is_terminal() {
                    eprintln!("error: pass --yes to remove a saved key without a terminal to ask in.");
                    return 2;
                }
                if waiting > 0 {
                    println!("{} use this key and will need another one picked before they can carry on.", plural(waiting, "unfinished install", "unfinished installs"));
                }
                print!("Forget the saved key \"{name}\"? Servers installed with it keep working. [y/N] ");
                let _ = std::io::Write::flush(&mut std::io::stdout());
                let mut a = String::new();
                if std::io::stdin().read_line(&mut a).is_err() || !matches!(a.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
                    println!("Cancelled. Nothing was changed.");
                    return 0;
                }
            }
            match store.remove(&name) {
                Ok(()) => {
                    println!("Forgot \"{name}\". Servers installed with it keep working: they have their own copy.");
                    if waiting > 0 {
                        println!("{} used it and will need another key.", plural(waiting, "unfinished install", "unfinished installs"));
                    }
                    0
                }
                Err(p) => say(p),
            }
        }
        KeysCmd::Use { server, name } => {
            let reg = match registry::Registry::load() {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: cannot read the server registry: {e}");
                    return 1;
                }
            };
            let Some(entry) = reg.find(&server).cloned() else {
                eprintln!("error: no registered server named '{server}' (see `kora status`)");
                return 1;
            };
            let key = match store.get(&name) {
                Ok(k) => k,
                Err(p) => return say(p),
            };
            if let Err(p) = kora_core::keystore::set_server_key(&entry.dir, &key) {
                return say(p);
            }
            let running = kora_core::api::Backend::status(&mut kora_core::api::system::SystemBackend, &entry) == kora_core::api::Status::Running;
            println!("\"{}\" now uses the saved key \"{name}\" (ending {}).", entry.name, kora_core::keystore::last4(&key));
            if running {
                println!("The server is running and only reads its key when it starts: restart it with `kora restart {}`.", entry.name);
            } else {
                println!("No restart is needed: the server is stopped and will use the new key next time it starts.");
            }
            0
        }
    }
}

pub fn profile(cmd: ProfileCmd) -> i32 {
    let store = ProfileStore::default();
    match cmd {
        ProfileCmd::Show => {
            let p = store.load();
            let show = |v: &Option<String>| v.clone().unwrap_or_else(|| "(not remembered yet)".into());
            println!("Cfx.re name:        {}", show(&p.cfx_username));
            println!("Cfx.re identifier:  {}", show(&p.cfx_identifier));
            println!("GitHub owner:       {}", show(&p.github_owner));
            println!("Kept in {}", store.path().display());
            0
        }
        ProfileCmd::Forget => match store.update(Field::Forget, Field::Forget, Field::Forget).and_then(|_| store.forget_all()) {
            Ok(()) => {
                println!("Forgot the remembered Cfx.re name and GitHub owner. The next install asks again.");
                0
            }
            Err(e) => {
                eprintln!("error: could not update {}: {e}", store.path().display());
                1
            }
        },
    }
}

/// The files this feature keeps in the Kora home that exist: saved keys, remembered names.
fn data_files() -> Vec<std::path::PathBuf> {
    [KeyStore::default().path().to_path_buf(), ProfileStore::default().path().to_path_buf()].into_iter().filter(|p| p.is_file()).collect()
}

/// `kora uninstall` (all servers): your saved keys are YOUR data, like the server folders and databases, so they
/// are kept unless you say yes here (the default is no, and `--yes` keeps them). A dry run only says so.
pub fn uninstall_note(dry_run: bool, ask: Option<&mut dyn kora_core::installer::uninstall::Asker>) {
    let files = data_files();
    if files.is_empty() {
        return;
    }
    let list = files.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ");
    match ask {
        _ if dry_run => println!("Your saved licence keys and remembered names ({list}) would be kept unless you say yes when asked."),
        None => println!("Your saved licence keys and remembered names were kept ({list}). Delete those files yourself if you want them gone."),
        Some(a) => {
            if a.ask(&format!("Also DELETE your saved licence keys and remembered names ({list})? (default: keep)"), false) {
                for p in files {
                    match std::fs::remove_file(&p) {
                        Ok(()) => println!("Deleted {}.", p.display()),
                        Err(e) => eprintln!("  failed: could not delete {}: {e}", p.display()),
                    }
                }
            } else {
                println!("Kept your saved licence keys and remembered names.");
            }
        }
    }
}

/// What `kora install --license-key-name` and `--save-license-key` do before the install starts:
/// returns the key to install with and the saved name it belongs to, if any.
pub fn resolve_install_key(key: Option<String>, name: Option<&str>, save_as: Option<&str>) -> Result<(Option<String>, Option<String>), String> {
    let store = KeyStore::default();
    if let Some(n) = name {
        let k = store.get(n).map_err(|p| plain_text(&String::from(p)))?;
        let canonical = store.list().map_err(|p| plain_text(&String::from(p)))?.into_iter().find(|i| i.name.eq_ignore_ascii_case(n)).map(|i| i.name);
        return Ok((Some(k), canonical.or_else(|| Some(n.to_string()))));
    }
    let Some(save) = save_as else { return Ok((key, None)) };
    let Some(k) = key else { return Err("--save-license-key needs a key: set CFX_LICENSE_KEY (or use --license-key).".into()) };
    let k = k.trim().to_string();
    let plain = |p: Problem| plain_text(&String::from(p));
    let saved = match store.name_of(&k).map_err(plain)? {
        Some(existing) => existing,
        None => {
            kora_core::keystore::check_key(&k).map_err(plain)?;
            store.add(save, &k).map_err(plain)?.name
        }
    };
    Ok((Some(k), Some(saved)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subcommands_parse_and_a_key_is_never_an_argument() {
        use clap::CommandFactory;
        let cli = crate::Cli::command();
        let keys = cli.find_subcommand("keys").unwrap();
        for sub in keys.get_subcommands() {
            let flags: Vec<_> = sub.get_arguments().filter_map(|a| a.get_long()).collect();
            assert!(!flags.iter().any(|f| f.contains("key")), "{}: {flags:?}", sub.get_name());
            assert!(sub.get_arguments().all(|a| !a.get_id().as_str().contains("key") || a.get_id() == "name"), "{}", sub.get_name());
        }
        let names: Vec<_> = keys.get_subcommands().map(|s| s.get_name().to_string()).collect();
        assert_eq!(names, ["list", "add", "replace", "rename", "remove", "use"]);
    }

    #[test]
    fn plural_helper() {
        assert_eq!(plural(1, "a", "b"), "1 a");
        assert_eq!(plural(2, "a", "b"), "2 b");
    }
}
