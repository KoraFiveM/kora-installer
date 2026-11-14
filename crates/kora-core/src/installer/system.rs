//! System-wide changes Kora made for one server and how to take them back. Shared by
//! `kora delete` (rollback of the service step) and `kora uninstall`.
//!
//! Everything here is derived from the server's own records (`.kora/created.json` and
//! `.kora/request.json`): nothing is removed that the records do not name.

use std::path::{Path, PathBuf};

use super::registry::Registry;
use super::{slug, Created, InstallRequest};
use crate::firewall::{self, PortRequest};
use crate::service::{self, account, binary, CommandRunner, ManagedServer, Plan, Platform, SystemStep};

/// One kind of system change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    UpdateTimer,
    Service,
    Firewall,
    ServiceAccount,
    Binary,
}

/// One removable change, with the plan that removes it.
#[derive(Debug, Clone)]
pub struct Item {
    pub kind: Kind,
    /// Question-style title for prompts and the dry run.
    pub title: String,
    pub plan: Plan,
    /// Extra explanation to show (why nothing will be done, what is kept...).
    pub note: Option<String>,
}

/// The facts needed to build removal plans (read from `request.json`).
#[derive(Debug, Clone)]
pub struct Target {
    pub name: String,
    pub dir: PathBuf,
    pub game_port: u16,
    pub txadmin_port: u16,
    pub txadmin_public: bool,
}

impl Target {
    pub fn from_request(req: &InstallRequest) -> Self {
        Target { name: req.name.clone(), dir: req.dir.clone(), game_port: req.ports.game, txadmin_port: req.ports.txadmin, txadmin_public: req.txadmin_public }
    }

    fn managed(&self, user: &str) -> ManagedServer {
        ManagedServer {
            name: slug(&self.name),
            dir: self.dir.to_string_lossy().into_owned(),
            start_command: vec!["kora".into()],
            user: user.to_string(),
            env_file: None,
            kora_exe: "kora".into(),
            windows_run_as: None,
        }
    }
}

/// Does another registered server (not in `excluding`) still run the kora copy at `path`?
pub fn binary_used_elsewhere(reg: &Registry, path: &str, excluding: &[&Path]) -> bool {
    reg.servers.iter().filter(|s| !excluding.iter().any(|d| super::registry::same_path(d, &s.dir))).any(|s| {
        std::fs::read(super::meta_dir(&s.dir).join("created.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Created>(&b).ok())
            .is_some_and(|c| c.binary.as_deref().is_some_and(|b| b == path || (cfg!(windows) && super::registry::same_path_windows(b, path))))
    })
}

/// Which changes did Kora make for this server, as removal items, in the order they must be undone
/// (timer, service, firewall, then the account the service ran as, then the program copy).
///
/// * `keep_files`: the server folder stays, so its ownership is handed back before the account goes
///   (otherwise the files would belong to a number that a future account could reuse).
/// * `binary_still_used`: another server still runs the shared copy: keep it.
pub fn items_for(
    t: &Target,
    c: &Created,
    platform: Platform,
    runner: &dyn CommandRunner,
    keep_files: bool,
    binary_still_used: bool,
) -> super::Result<Vec<Item>> {
    let err = |e: service::ServiceError| super::InstallError(e.to_string());
    let account_name = c.service_account.clone().unwrap_or_else(|| account::account_name(&slug(&t.name)));
    let server = t.managed(if account_name == "root" { "kora" } else { &account_name });
    let mut v = Vec::new();

    if c.update_timer {
        v.push(Item {
            kind: Kind::UpdateTimer,
            title: "Remove the daily automatic-update schedule".into(),
            plan: service::scheduler_uninstall_plan(&server, platform).map_err(err)?,
            note: None,
        });
    }
    if c.service_unit.is_some() {
        v.push(Item {
            kind: Kind::Service,
            title: format!("Stop and remove the background service {}", server.service_name()),
            plan: service::uninstall_plan(&server, platform).map_err(err)?,
            note: None,
        });
    }
    if c.firewall {
        let preq = PortRequest { server: slug(&t.name), game_port: t.game_port, txadmin_port: t.txadmin_port, expose_txadmin: t.txadmin_public };
        match firewall::detect(platform, runner) {
            Some(kind) => v.push(Item {
                kind: Kind::Firewall,
                title: format!("Close the firewall ports Kora opened ({} / {})", t.game_port, t.txadmin_port),
                plan: firewall::close_plan(&preq, kind).map_err(err)?,
                note: None,
            }),
            None => v.push(Item {
                kind: Kind::Firewall,
                title: "Close the firewall ports Kora opened".into(),
                plan: Plan::default(),
                note: Some("No active firewall was found, so there is nothing to close (rules may already be gone).".into()),
            }),
        }
    }
    if c.service_account_created {
        if let Some(name) = c.service_account.as_deref() {
            if platform == Platform::Linux {
                match account::user_status(runner, name).map_err(err)? {
                    account::UserStatus::Missing => {}
                    account::UserStatus::Exists(0) => {
                        v.push(Item {
                            kind: Kind::ServiceAccount,
                            title: format!("Delete the system account '{name}'"),
                            plan: Plan::default(),
                            note: Some(format!("'{name}' now has uid 0 (root): Kora will not touch it.")),
                        });
                    }
                    account::UserStatus::Exists(_) => {
                        let mut steps: Vec<SystemStep> = Vec::new();
                        if keep_files {
                            if let Some(owner) = &c.dir_owner {
                                steps.push(SystemStep::run(
                                    format!("Give the kept server folder back to its previous owner ({owner}) before the account '{name}' is deleted"),
                                    "chown",
                                    &["-R", owner, &t.dir.to_string_lossy()],
                                ));
                            }
                        }
                        steps.extend(account::delete_plan(name).map_err(err)?.steps);
                        v.push(Item {
                            kind: Kind::ServiceAccount,
                            title: format!("Delete the system account '{name}' that Kora created"),
                            plan: Plan::new(steps),
                            note: None,
                        });
                    }
                }
            }
        }
    }
    if let Some(path) = c.binary.as_deref() {
        let title = format!("Delete the root-owned copy of the kora program ({path})");
        if binary_still_used {
            v.push(Item { kind: Kind::Binary, title, plan: Plan::default(), note: Some("Another Kora server still runs this copy: kept.".into()) });
        } else {
            v.push(Item { kind: Kind::Binary, title, plan: binary::remove_plan(Path::new(path)), note: None });
        }
    }
    Ok(v)
}

/// Should the record be cleared after this item was handled? Yes when commands ran, and for a
/// program copy that is kept because another server still uses it (this server no longer does).
pub fn clears_record(item: &Item) -> bool {
    !item.plan.steps.is_empty() || item.kind == Kind::Binary
}

/// Record in `created.json` that `kind` is gone, so a later delete does not try again.
pub fn clear_record(c: &mut Created, kind: Kind) {
    match kind {
        Kind::UpdateTimer => c.update_timer = false,
        Kind::Service => c.service_unit = None,
        Kind::Firewall => c.firewall = false,
        Kind::ServiceAccount => {
            c.service_account_created = false;
            c.dir_owner = None;
        }
        Kind::Binary => c.binary = None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::testing::FakeRunner;

    fn target() -> Target {
        Target { name: "My Server".into(), dir: "/srv/kora/my-server".into(), game_port: 31120, txadmin_port: 41120, txadmin_public: false }
    }

    fn full_created() -> Created {
        Created {
            service_unit: Some("/etc/systemd/system/kora-my-server.service".into()),
            update_timer: true,
            firewall: true,
            service_account: Some("kora-my-server".into()),
            service_account_created: true,
            dir_owner: Some("0:0".into()),
            binary: Some("/opt/kora/0.1.0-abc/kora".into()),
            ..Created::default()
        }
    }

    fn lines(i: &Item) -> Vec<String> {
        i.plan.steps.iter().map(|s| s.command_line().unwrap_or_else(|| format!("{:?}", s.action))).collect()
    }

    #[test]
    fn everything_kora_made_is_listed_in_undo_order() {
        let r = FakeRunner::default();
        r.respond("id -u kora-my-server", 0, "998\n");
        let items = items_for(&target(), &full_created(), Platform::Linux, &r, false, false).unwrap();
        let kinds: Vec<Kind> = items.iter().map(|i| i.kind).collect();
        // (the fake reports no active firewall, so that item carries a note instead of commands)
        assert_eq!(kinds, [Kind::UpdateTimer, Kind::Service, Kind::Firewall, Kind::ServiceAccount, Kind::Binary]);
        assert!(lines(&items[1]).iter().any(|l| l.contains("systemctl disable kora-my-server")));
        assert!(lines(&items[3]).iter().any(|l| l == "userdel kora-my-server"));
        assert!(!lines(&items[3]).iter().any(|l| l.starts_with("chown")), "folder is deleted with the server: no chown");
    }

    #[test]
    fn firewall_without_an_active_firewall_is_a_note_not_a_command() {
        let r = FakeRunner::default();
        r.respond("ufw status", 1, "");
        r.respond("firewall-cmd", 1, "");
        let c = Created { firewall: true, ..Created::default() };
        let items = items_for(&target(), &c, Platform::Linux, &r, false, false).unwrap();
        assert_eq!(items.len(), 1);
        assert!(items[0].plan.steps.is_empty() && items[0].note.is_some());
    }

    #[test]
    fn firewall_rules_are_removed_with_the_detected_tool() {
        let r = FakeRunner::default();
        r.respond("ufw status", 0, "Status: active\n");
        let c = Created { firewall: true, ..Created::default() };
        let items = items_for(&target(), &c, Platform::Linux, &r, false, false).unwrap();
        let l = lines(&items[0]);
        assert!(!l.is_empty() && l.iter().all(|x| x.starts_with("ufw")), "{l:?}");
    }

    #[test]
    fn an_account_kora_did_not_create_is_never_offered() {
        let r = FakeRunner::default();
        r.respond("id -u", 0, "1001\n");
        let c = Created { service_account: Some("alice".into()), service_account_created: false, ..Created::default() };
        assert!(items_for(&target(), &c, Platform::Linux, &r, false, false).unwrap().is_empty());
        // flag set but the account is already gone: nothing to do either
        let gone = FakeRunner::default();
        gone.respond("id -u", 1, "");
        let c = Created { service_account: Some("kora-x".into()), service_account_created: true, ..Created::default() };
        assert!(items_for(&target(), &c, Platform::Linux, &gone, false, false).unwrap().is_empty());
        // and one that has become uid 0 is refused
        let root = FakeRunner::default();
        root.respond("id -u", 0, "0\n");
        let items = items_for(&target(), &c, Platform::Linux, &root, false, false).unwrap();
        assert!(items[0].plan.steps.is_empty() && items[0].note.as_deref().unwrap().contains("uid 0"));
    }

    #[test]
    fn kept_files_are_handed_back_before_the_account_is_deleted() {
        let r = FakeRunner::default();
        r.respond("id -u", 0, "998\n");
        let c = Created { service_account: Some("kora-x".into()), service_account_created: true, dir_owner: Some("1000:1000".into()), ..Created::default() };
        let items = items_for(&target(), &c, Platform::Linux, &r, true, false).unwrap();
        assert_eq!(lines(&items[0]), ["chown -R 1000:1000 /srv/kora/my-server", "userdel kora-x"]);
    }

    #[test]
    fn a_shared_program_copy_is_kept_while_another_server_uses_it() {
        let r = FakeRunner::default();
        let c = Created { binary: Some("/opt/kora/v/kora".into()), ..Created::default() };
        let kept = items_for(&target(), &c, Platform::Linux, &r, false, true).unwrap();
        assert!(kept[0].plan.steps.is_empty() && kept[0].note.is_some());
        let gone = items_for(&target(), &c, Platform::Linux, &r, false, false).unwrap();
        assert!(gone[0].plan.steps.len() >= 2);
    }

    #[test]
    fn windows_has_no_account_to_delete_and_removes_acl_and_service() {
        let r = FakeRunner::default();
        let items = items_for(&target(), &full_created(), Platform::Windows, &r, false, false).unwrap();
        assert!(!items.iter().any(|i| i.kind == Kind::ServiceAccount));
        let svc = items.iter().find(|i| i.kind == Kind::Service).unwrap();
        let l = lines(svc);
        assert!(l[0].starts_with("icacls.exe") && l[0].contains("NT SERVICE\\kora-my-server"), "{l:?}");
        assert!(l.last().unwrap().contains("sc.exe delete kora-my-server"));
    }

    #[test]
    fn clearing_records_is_per_kind() {
        let mut c = full_created();
        clear_record(&mut c, Kind::ServiceAccount);
        assert!(!c.service_account_created && c.dir_owner.is_none() && c.service_unit.is_some());
        clear_record(&mut c, Kind::Binary);
        assert!(c.binary.is_none());
    }

    #[test]
    fn binary_use_is_read_from_the_other_servers_records() {
        use super::super::registry::{PortsRecord, ServerEntry};
        use super::super::Mode;
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a"), t.path().join("b"));
        for (d, bin) in [(&a, Some("/opt/kora/v/kora")), (&b, None)] {
            std::fs::create_dir_all(d.join(".kora")).unwrap();
            let c = Created { binary: bin.map(String::from), ..Created::default() };
            std::fs::write(d.join(".kora/created.json"), serde_json::to_vec(&c).unwrap()).unwrap();
        }
        let entry = |n: &str, d: &PathBuf| ServerEntry { name: n.into(), dir: d.clone(), mode: Mode::Production, ports: PortsRecord { game: 1, txadmin: 2 }, created_unix: 0 };
        let reg = Registry { servers: vec![entry("a", &a), entry("b", &b)] };
        assert!(binary_used_elsewhere(&reg, "/opt/kora/v/kora", &[b.as_path()]), "a uses it");
        assert!(!binary_used_elsewhere(&reg, "/opt/kora/v/kora", &[a.as_path()]), "only a does, and a is being removed");
        assert!(!binary_used_elsewhere(&reg, "/opt/kora/other/kora", &[]));
    }
}
