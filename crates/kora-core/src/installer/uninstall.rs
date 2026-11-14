//! `kora uninstall`: take back the system-wide changes Kora made, one question at a time.
//!
//! Safe by construction:
//! * only servers in the registry are considered, and for each only what its own records
//!   (`.kora/created.json`) say Kora created is offered: services, update timers, firewall rules,
//!   the service account (only if Kora created it) and the shared program copy (only if no other
//!   server runs it);
//! * the server folder and the database are KEPT unless the operator explicitly asks otherwise;
//! * `dry_run` prints everything and changes nothing.

use std::path::Path;

use super::registry::Registry;
use super::system::{self, Item, Target};
use super::{load_created_from, load_request_from, save_created_to, slug, RollbackOptions};
use crate::service::{apply_plan, CommandRunner, Platform};

/// Answers yes/no questions (the terminal, `--yes`, or a script in tests).
pub trait Asker {
    fn ask(&mut self, question: &str, default_yes: bool) -> bool;
}

/// `--yes`: every question gets its default (system changes: yes; files and databases: no).
pub struct Defaults;

impl Asker for Defaults {
    fn ask(&mut self, _question: &str, default_yes: bool) -> bool {
        default_yes
    }
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Print what would happen, change nothing.
    pub dry_run: bool,
    /// Only this server (registry name, slug or folder).
    pub only: Option<String>,
    /// Also delete the server folders (default: keep). Counts as the answer, no question asked.
    pub delete_files: bool,
    /// Also drop the databases and their users (default: keep).
    pub delete_databases: bool,
}

/// What the operator's machine looks like for this run.
pub struct Env<'a> {
    pub platform: Platform,
    pub runner: &'a dyn CommandRunner,
    pub admin: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Summary {
    pub removed: Vec<String>,
    pub kept: Vec<String>,
    pub failed: Vec<String>,
}

impl Summary {
    pub fn ok(&self) -> bool {
        self.failed.is_empty()
    }
}

/// Deletes a server's files / database through the regular `kora delete` machinery.
pub type Deleter<'a> = &'a mut dyn FnMut(&Path, &RollbackOptions) -> super::Result<Vec<String>>;

fn describe(i: &Item) -> String {
    let cmds: Vec<String> = i.plan.steps.iter().map(|s| s.command_line().unwrap_or_else(|| s.description.clone())).collect();
    if cmds.is_empty() {
        i.note.clone().unwrap_or_else(|| "nothing to do".into())
    } else {
        cmds.join("  |  ")
    }
}

/// Run (or, with `dry_run`, preview) the uninstall for every registered server.
pub fn run(
    reg: &mut Registry,
    opts: &Options,
    env: &Env,
    ask: &mut dyn Asker,
    say: &mut dyn FnMut(&str),
    delete: Deleter,
    save: &mut dyn FnMut(&Registry) -> std::io::Result<()>,
) -> Summary {
    let mut sum = Summary::default();
    let names: Vec<(String, std::path::PathBuf)> = reg
        .servers
        .iter()
        .filter(|s| opts.only.as_deref().is_none_or(|o| s.name == o || slug(&s.name) == o || super::registry::same_path(&s.dir, Path::new(o))))
        .map(|s| (s.name.clone(), s.dir.clone()))
        .collect();
    if names.is_empty() {
        say(&match &opts.only {
            Some(o) => format!("No registered Kora server matches '{o}'. Nothing to do."),
            None => "No Kora servers are registered. Nothing to do.".to_string(),
        });
        return sum;
    }
    let verb = if opts.dry_run { "Would" } else { "Will" };
    for (name, dir) in names {
        say(&format!("\n== {name} ({})", dir.display()));
        let (Some(req), Some(created)) = (load_request_from(&dir), load_created_from(&dir)) else {
            say("  This folder has no Kora records (it is missing or was not installed by Kora), so Kora cannot tell what it created for it and will not guess.");
            if !opts.dry_run && ask.ask(&format!("Forget '{name}' in the Kora server list? (touches nothing else)"), false) {
                reg.remove(&name);
                match save(reg) {
                    Ok(()) => sum.removed.push(format!("{name}: removed from the server list")),
                    Err(e) => sum.failed.push(format!("{name}: cannot write the server list: {e}")),
                }
            } else {
                sum.kept.push(format!("{name}: left in the server list (no records)"));
            }
            continue;
        };
        let target = Target::from_request(&req);
        let has_db = !matches!(req.db, super::DbChoice::Skip) && dir.join(".kora").join("db-provision.json").is_file();

        // Files and databases first: they decide how the system items are undone (kept files are handed back).
        let (mut del_files, mut del_db) = (opts.delete_files, opts.delete_databases && has_db);
        if !opts.dry_run {
            if !del_files {
                del_files = ask.ask(&format!("Also DELETE the server folder {} and everything in it? (default: keep)", dir.display()), false);
            }
            if has_db && !del_db {
                del_db = ask.ask("Also DROP this server's database and its database user? (default: keep)", false);
            }
        }

        let reg_snapshot = reg.clone();
        let used = created.binary.as_deref().is_some_and(|b| system::binary_used_elsewhere(&reg_snapshot, b, &[dir.as_path()]));
        let items = match system::items_for(&target, &created, env.platform, env.runner, !del_files, used) {
            Ok(i) => i,
            Err(e) => {
                sum.failed.push(format!("{name}: {e}"));
                continue;
            }
        };
        if items.is_empty() {
            say("  Kora recorded no system-wide changes for this server (no service, timer, firewall rules or account).");
        }
        let mut created = created;
        for item in &items {
            if opts.dry_run {
                say(&format!("  {verb}: {}\n      {}", item.title, describe(item)));
                continue;
            }
            if let Some(n) = &item.note {
                say(&format!("  note: {n}"));
            }
            if !ask.ask(&format!("{}?", item.title.trim_end_matches('?')), true) {
                sum.kept.push(format!("{name}: {}: kept", item.title));
                continue;
            }
            if item.plan.needs_admin() && !env.admin {
                sum.failed.push(format!("{name}: {}: needs administrator rights (re-run as root / Administrator)", item.title));
                continue;
            }
            match apply_plan(&item.plan, env.runner) {
                Ok(()) => {
                    if system::clears_record(item) {
                        system::clear_record(&mut created, item.kind);
                        if let Err(e) = save_created_to(&dir, &created) {
                            say(&format!("  note: removed, but could not update Kora's record: {e}"));
                        }
                    }
                    if !item.plan.steps.is_empty() {
                        sum.removed.push(format!("{name}: {}", item.title));
                        say(&format!("  done: {}", item.title));
                    }
                }
                Err(e) => {
                    sum.failed.push(format!("{name}: {}: {e}", item.title));
                    say(&format!("  FAILED: {}: {e}", item.title));
                }
            }
        }
        if opts.dry_run {
            say(&format!("  {verb} KEEP the server folder and database{}.", if opts.delete_files || opts.delete_databases { " except what --delete-files / --delete-databases name" } else { " (add --delete-files / --delete-databases to include them)" }));
            say(&format!("  {verb} remove '{name}' from the server list{}.", if opts.delete_files || opts.delete_databases { " when its files or database are deleted" } else { " if you agree" }));
            continue;
        }
        if del_files || del_db {
            let o = RollbackOptions { keep_files: !del_files, keep_db: !del_db };
            match delete(&dir, &o) {
                Ok(notes) => {
                    for n in notes {
                        say(&format!("  note: {n}"));
                    }
                    reg.remove(&name);
                    sum.removed.push(format!("{name}: {}{}", if del_files { "server folder" } else { "" }, if del_db { if del_files { " + database" } else { "database" } } else { "" }));
                }
                Err(e) => {
                    sum.failed.push(format!("{name}: {e}"));
                    say(&format!("  FAILED: {e}"));
                }
            }
        } else if ask.ask(&format!("Forget '{name}' in the Kora server list? (its folder and database stay)"), true) {
            reg.remove(&name);
            sum.removed.push(format!("{name}: removed from the server list"));
        } else {
            sum.kept.push(format!("{name}: left in the server list"));
        }
        if let Err(e) = save(reg) {
            sum.failed.push(format!("{name}: cannot write the server list: {e}"));
        }
    }
    sum
}

/// Machine-wide: the MariaDB Kora installed itself (Windows). Offered only when the operator asked
/// for databases to go (`--delete-databases`) and no remaining registered server still keeps a
/// database in it. `remove` does the real work (service + folder); `record` is Kora's own record.
#[allow(clippy::too_many_arguments)]
pub fn mariadb_cleanup(
    reg: &Registry,
    opts: &Options,
    admin: bool,
    record: Option<(super::windows_db::Record, std::path::PathBuf)>,
    ask: &mut dyn Asker,
    say: &mut dyn FnMut(&str),
    remove: &mut dyn FnMut(&super::windows_db::Record, &Path) -> super::Result<()>,
    sum: &mut Summary,
) {
    let Some((rec, base)) = record else { return };
    let title = format!("the MariaDB server Kora installed (service {}, {})", rec.service, rec.root.display());
    if !opts.delete_databases || opts.only.is_some() {
        if !opts.dry_run {
            say(&format!("\nKept {title}: add --delete-databases (without naming one server) to remove it."));
        }
        sum.kept.push("MariaDB: kept".into());
        return;
    }
    let still_used: Vec<&str> = reg.servers.iter().filter(|s| s.dir.join(".kora").join("db-provision.json").is_file()).map(|s| s.name.as_str()).collect();
    if !still_used.is_empty() {
        say(&format!("\nKept {title}: still used by {}.", still_used.join(", ")));
        sum.kept.push("MariaDB: kept (still in use)".into());
        return;
    }
    if opts.dry_run {
        say(&format!("\nWould remove {title}: stop and delete the Windows service and delete its folder, including every database in it."));
        return;
    }
    say("");
    if !ask.ask(&format!("Remove {title}, including every database in it?"), true) {
        sum.kept.push("MariaDB: kept".into());
        return;
    }
    if !admin {
        sum.failed.push("MariaDB: needs administrator rights (re-run as Administrator)".into());
        return;
    }
    match remove(&rec, &base) {
        Ok(()) => {
            sum.removed.push(format!("MariaDB service {} and its folder", rec.service));
            say("  done: removed the MariaDB server Kora installed");
        }
        Err(e) => {
            sum.failed.push(format!("MariaDB: {e}"));
            say(&format!("  FAILED: {e}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::registry::{PortsRecord, ServerEntry};
    use super::super::{Created, DbChoice, InstallRequest, Mode, Ports, ResourceSelection};
    use super::*;
    use crate::service::testing::FakeRunner;
    use std::path::PathBuf;

    struct Script(Vec<(&'static str, bool)>, Vec<String>);
    impl Asker for Script {
        fn ask(&mut self, q: &str, default_yes: bool) -> bool {
            self.1.push(q.to_string());
            self.0.iter().find(|(needle, _)| q.contains(needle)).map(|(_, a)| *a).unwrap_or(default_yes)
        }
    }

    fn request(name: &str, dir: &Path) -> InstallRequest {
        InstallRequest {
            mode: Mode::Production,
            name: name.into(),
            dir: dir.to_path_buf(),
            hostname: name.into(),
            license_key: None,
            cfx_identifier: None,
            ports: Ports::default(),
            txadmin_public: false,
            artifact_url: None,
            game_build: None,
            resources: ResourceSelection::All,
            db: DbChoice::Skip,
            service: true,
            firewall: false,
            github_owner: None,
            catalog: None,
            dry_run: false,
            db_admin: None,
            start: false,
            app: None,
        }
    }

    /// A server folder with records, plus its registry entry.
    fn server(root: &Path, name: &str, c: &Created) -> ServerEntry {
        let dir = root.join(name);
        std::fs::create_dir_all(dir.join(".kora")).unwrap();
        std::fs::write(dir.join("kora-server.json"), b"{}").unwrap();
        std::fs::write(dir.join("precious.txt"), b"keep me").unwrap();
        std::fs::write(dir.join(".kora/request.json"), serde_json::to_vec(&request(name, &dir)).unwrap()).unwrap();
        std::fs::write(dir.join(".kora/created.json"), serde_json::to_vec(c).unwrap()).unwrap();
        ServerEntry { name: name.into(), dir, mode: Mode::Production, ports: PortsRecord { game: 30120, txadmin: 40120 }, created_unix: 0 }
    }

    fn created(account: &str, bin: &str) -> Created {
        Created {
            service_unit: Some(format!("/etc/systemd/system/kora-{account}.service")),
            service_account: Some(format!("kora-{account}")),
            service_account_created: true,
            dir_owner: Some("0:0".into()),
            binary: Some(bin.into()),
            ..Created::default()
        }
    }

    struct Rig {
        out: Vec<String>,
        saved: usize,
    }

    fn go(reg: &mut Registry, opts: &Options, admin: bool, runner: &FakeRunner, ask: &mut dyn Asker, deleted: &mut Vec<(PathBuf, bool, bool)>) -> (Summary, Rig) {
        let mut rig = Rig { out: vec![], saved: 0 };
        let env = Env { platform: Platform::Linux, runner, admin };
        let mut say = |s: &str| rig.out.push(s.to_string());
        let mut saved = 0;
        let mut del = |d: &Path, o: &RollbackOptions| {
            deleted.push((d.to_path_buf(), o.keep_files, o.keep_db));
            Ok(vec![])
        };
        let mut save = |_: &Registry| {
            saved += 1;
            Ok(())
        };
        let s = run(reg, opts, &env, ask, &mut say, &mut del, &mut save);
        rig.saved = saved;
        (s, rig)
    }

    #[test]
    fn dry_run_lists_everything_and_changes_nothing() {
        let t = tempfile::tempdir().unwrap();
        let mut reg = Registry { servers: vec![server(t.path(), "alpha", &created("alpha", "/opt/kora/v/kora"))] };
        let r = FakeRunner::default();
        r.respond("id -u", 0, "998\n");
        let mut del = vec![];
        let (sum, rig) = go(&mut reg, &Options { dry_run: true, ..Default::default() }, true, &r, &mut Defaults, &mut del);
        let text = rig.out.join("\n");
        for want in ["Stop and remove the background service kora-alpha", "systemctl disable kora-alpha", "userdel kora-alpha", "Delete the root-owned copy", "KEEP the server folder and database"] {
            assert!(text.contains(want), "missing {want:?} in:\n{text}");
        }
        // the only commands run were the read-only `id -u`
        assert!(r.calls().iter().all(|c| c.starts_with("id -u")), "{:?}", r.calls());
        assert!(sum.removed.is_empty() && del.is_empty() && reg.servers.len() == 1 && rig.saved == 0);
    }

    #[test]
    fn defaults_remove_system_items_keep_files_and_databases_and_forget_the_entry() {
        let t = tempfile::tempdir().unwrap();
        let entry = server(t.path(), "alpha", &created("alpha", "/opt/kora/v/kora"));
        let dir = entry.dir.clone();
        let mut reg = Registry { servers: vec![entry] };
        let r = FakeRunner::default();
        r.respond("id -u", 0, "998\n");
        let mut del = vec![];
        let (sum, _) = go(&mut reg, &Options::default(), true, &r, &mut Defaults, &mut del);
        let calls = r.calls();
        assert!(calls.iter().any(|c| c == "systemctl stop kora-alpha"), "{calls:?}");
        assert!(calls.iter().any(|c| c == "remove /etc/systemd/system/kora-alpha.service"));
        assert!(calls.iter().any(|c| *c == "chown -R 0:0 ".to_string() + &dir.to_string_lossy()), "kept folder is handed back first: {calls:?}");
        let chown = calls.iter().position(|c| c.starts_with("chown")).unwrap();
        let userdel = calls.iter().position(|c| c == "userdel kora-alpha").unwrap();
        assert!(chown < userdel);
        assert!(calls.iter().any(|c| c == "remove /opt/kora/v/kora"));
        assert!(del.is_empty(), "files and databases are kept by default");
        assert!(dir.join("precious.txt").is_file());
        assert!(reg.servers.is_empty(), "entry forgotten");
        assert!(sum.ok() && sum.removed.len() >= 4, "{sum:?}");
        // records updated so `kora delete` later does not repeat anything
        let c = load_created_from(&dir).unwrap();
        assert!(c.service_unit.is_none() && !c.service_account_created && c.binary.is_none());
    }

    #[test]
    fn declining_an_item_keeps_it_and_each_item_is_asked_separately() {
        let t = tempfile::tempdir().unwrap();
        let mut reg = Registry { servers: vec![server(t.path(), "alpha", &created("alpha", "/opt/kora/v/kora"))] };
        let r = FakeRunner::default();
        r.respond("id -u", 0, "998\n");
        let mut ask = Script(vec![("system account", false)], vec![]);
        let mut del = vec![];
        let (sum, _) = go(&mut reg, &Options::default(), true, &r, &mut ask, &mut del);
        assert!(!r.calls().iter().any(|c| c.starts_with("userdel")), "declined");
        assert!(r.calls().iter().any(|c| c == "remove /opt/kora/v/kora"));
        assert!(sum.kept.iter().any(|k| k.contains("system account")));
        // service, account, binary, files, forget-entry... each its own question
        assert!(ask.1.len() >= 5, "{:?}", ask.1);
        assert!(ask.1[0].contains("DELETE the server folder"), "files are asked about first, default no");
    }

    #[test]
    fn deleting_files_and_databases_needs_an_explicit_yes_and_goes_through_delete() {
        let t = tempfile::tempdir().unwrap();
        let entry = server(t.path(), "alpha", &Created::default());
        let dir = entry.dir.clone();
        std::fs::write(dir.join(".kora/db-provision.json"), b"{}").unwrap();
        let mut req = request("alpha", &dir);
        req.db = DbChoice::UseExisting;
        std::fs::write(dir.join(".kora/request.json"), serde_json::to_vec(&req).unwrap()).unwrap();
        let mut reg = Registry { servers: vec![entry] };
        let r = FakeRunner::default();
        let mut del = vec![];
        let mut ask = Script(vec![("DELETE the server folder", true), ("DROP", false)], vec![]);
        let (sum, _) = go(&mut reg, &Options::default(), true, &r, &mut ask, &mut del);
        assert_eq!(del, vec![(dir.clone(), false, true)], "files yes, database kept");
        assert!(reg.servers.is_empty() && sum.ok());
        // flags are explicit consent: no question for them
        let entry = server(t.path(), "beta", &Created::default());
        let bdir = entry.dir.clone();
        let mut reg = Registry { servers: vec![entry] };
        let mut del = vec![];
        let mut ask = Script(vec![], vec![]);
        go(&mut reg, &Options { delete_files: true, ..Default::default() }, true, &r, &mut ask, &mut del);
        assert_eq!(del, vec![(bdir, false, true)]);
        assert!(!ask.1.iter().any(|q| q.contains("DELETE")));
    }

    #[test]
    fn without_admin_rights_system_items_are_reported_not_attempted() {
        let t = tempfile::tempdir().unwrap();
        let mut reg = Registry { servers: vec![server(t.path(), "alpha", &created("alpha", "/opt/kora/v/kora"))] };
        let r = FakeRunner::default();
        r.respond("id -u", 0, "998\n");
        let mut del = vec![];
        let (sum, _) = go(&mut reg, &Options::default(), false, &r, &mut Defaults, &mut del);
        assert!(sum.failed.iter().any(|f| f.contains("administrator")), "{sum:?}");
        assert!(!r.calls().iter().any(|c| c.starts_with("systemctl") || c.starts_with("userdel") || c.starts_with("remove")), "{:?}", r.calls());
    }

    #[test]
    fn a_folder_without_kora_records_is_never_touched_beyond_the_list() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("stranger");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x"), b"x").unwrap();
        let entry = ServerEntry { name: "stranger".into(), dir: dir.clone(), mode: Mode::Production, ports: PortsRecord { game: 1, txadmin: 2 }, created_unix: 0 };
        let mut reg = Registry { servers: vec![entry] };
        let r = FakeRunner::default();
        let mut del = vec![];
        let (sum, rig) = go(&mut reg, &Options::default(), true, &r, &mut Defaults, &mut del);
        assert!(r.calls().is_empty() && del.is_empty() && dir.join("x").is_file());
        assert!(rig.out.join(" ").contains("will not guess") && sum.kept.len() == 1 && reg.servers.len() == 1, "default is to leave even the list entry");
    }

    #[test]
    fn a_shared_program_copy_goes_only_with_the_last_server_that_uses_it() {
        let t = tempfile::tempdir().unwrap();
        let bin = "/opt/kora/shared/kora";
        let (a, b) = (server(t.path(), "a", &created("a", bin)), server(t.path(), "b", &created("b", bin)));
        let mut reg = Registry { servers: vec![a, b] };
        let r = FakeRunner::default();
        r.respond("id -u", 0, "998\n");
        let mut del = vec![];
        go(&mut reg, &Options::default(), true, &r, &mut Defaults, &mut del);
        let removes = r.calls().iter().filter(|c| *c == &format!("remove {bin}")).count();
        assert_eq!(removes, 1, "removed once, with the last server: {:?}", r.calls());
        let calls = r.calls();
        let last_userdel = calls.iter().rposition(|c| c.starts_with("userdel")).unwrap();
        let rm = calls.iter().position(|c| *c == format!("remove {bin}")).unwrap();
        assert!(rm > last_userdel, "after both servers' services are gone");
    }

    #[test]
    fn only_filters_to_one_server_and_unknown_names_do_nothing() {
        let t = tempfile::tempdir().unwrap();
        let mut reg = Registry { servers: vec![server(t.path(), "a", &created("a", "/o/k")), server(t.path(), "b", &created("b", "/o/k"))] };
        let r = FakeRunner::default();
        r.respond("id -u", 0, "998\n");
        let mut del = vec![];
        go(&mut reg, &Options { only: Some("b".into()), ..Default::default() }, true, &r, &mut Defaults, &mut del);
        assert_eq!(reg.servers.len(), 1);
        assert_eq!(reg.servers[0].name, "a");
        assert!(!r.calls().iter().any(|c| c.contains("kora-a")));
        let (s, rig) = go(&mut reg, &Options { only: Some("zzz".into()), ..Default::default() }, true, &r, &mut Defaults, &mut del);
        assert!(s == Summary::default() && rig.out[0].contains("No registered Kora server matches"));
    }

    fn rec() -> super::super::windows_db::Record {
        super::super::windows_db::Record { service: "KoraMariaDB_ab12".into(), version: "11.4.14".into(), port: 3307, root: PathBuf::from("C:\\PD\\Kora\\mariadb\\11.4.14"), client: PathBuf::from("c.exe") }
    }

    #[test]
    fn mariadb_goes_only_with_delete_databases_for_everything_and_when_unused() {
        let t = tempfile::tempdir().unwrap();
        let base = t.path().to_path_buf();
        let reg = Registry::default();
        let removed = std::cell::RefCell::new(Vec::new());
        let run = |opts: Options, admin: bool, ans: bool| {
            let mut sum = Summary::default();
            let mut said = Vec::new();
            mariadb_cleanup(&reg, &opts, admin, Some((rec(), base.clone())), &mut Script(vec![("Remove the MariaDB", ans)], vec![]), &mut |s| said.push(s.to_string()), &mut |r, _| { removed.borrow_mut().push(r.service.clone()); Ok(()) }, &mut sum);
            sum
        };
        let s = run(Options::default(), true, true);
        assert!(s.removed.is_empty() && s.kept.len() == 1, "kept without --delete-databases");
        let s = run(Options { delete_databases: true, only: Some("x".into()), ..Options::default() }, true, true);
        assert!(s.removed.is_empty(), "never when one server was named");
        let s = run(Options { delete_databases: true, dry_run: true, ..Options::default() }, true, true);
        assert!(s.removed.is_empty() && s.failed.is_empty());
        let s = run(Options { delete_databases: true, ..Options::default() }, true, false);
        assert!(s.removed.is_empty(), "declined");
        let s = run(Options { delete_databases: true, ..Options::default() }, false, true);
        assert_eq!(s.failed.len(), 1, "needs admin");
        assert!(removed.borrow().is_empty());
        let s = run(Options { delete_databases: true, ..Options::default() }, true, true);
        assert_eq!(s.removed.len(), 1);
        assert_eq!(*removed.borrow(), vec!["KoraMariaDB_ab12".to_string()]);
    }

    #[test]
    fn mariadb_is_kept_while_a_server_still_has_a_database_in_it() {
        let t = tempfile::tempdir().unwrap();
        let e = server(t.path(), "alpha", &created("kora-alpha", "/b"));
        std::fs::write(e.dir.join(".kora/db-provision.json"), b"{}").unwrap();
        let reg = Registry { servers: vec![e] };
        let mut sum = Summary::default();
        mariadb_cleanup(&reg, &Options { delete_databases: true, ..Options::default() }, true, Some((rec(), t.path().to_path_buf())), &mut Defaults, &mut |_| {}, &mut |_, _| panic!("must not remove"), &mut sum);
        assert!(sum.removed.is_empty() && sum.kept.iter().any(|k| k.contains("in use")));
    }
}
