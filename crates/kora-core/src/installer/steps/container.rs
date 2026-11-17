//! The install steps of container mode (Linux), each with its undo. See `crate::container` for the design.
//!
//! Plan (container mode), in order: `container.engine` (find and prove an engine, write the folder's
//! `kora-container.json`), `container.images` (pull the pinned images, with progress), then the usual build
//! steps; `build.db` creates the MariaDB container (network, volume, container, wait until its login works,
//! `database.json`); `container.create` creates the game container and sets up start at boot. The last steps
//! (`finalize`, `launch`) are the ones every mode has: `launch` starts the container through the same seam as the
//! Start button.
//!
//! Undo, in the reverse order: the game container is removed, then the database container and the private
//! network (and the data volume unless the person keeps the database), then the record. Nothing is removed that
//! the record does not name, and the record is checked first (`ContainerRecord::check`): the folder can be
//! changed by the server's own processes, the server list cannot. A folder that is not in the server list is
//! left alone, like the service steps do.

use super::*;
use crate::container;
use crate::container::lifecycle::{self, Manager};
use crate::container::record::ContainerRecord;

/// A progress line from the engine (a pull prints one per layer): short and without control characters.
fn tidy_line(l: &str) -> String {
    l.chars().filter(|c| !c.is_control()).take(160).collect()
}

impl InstallStep {
    /// The folder's record, checked against the server list's name for it.
    fn container_record(&self) -> Result<ContainerRecord> {
        let ctx = &*self.ctx;
        let rec = ContainerRecord::load(&ctx.req.dir).ok_or_else(|| Problem::generic(format!("{} has no container record (kora-container.json)", ctx.req.dir.display())))?;
        rec.check(&slug(&ctx.req.name)).map_err(Problem::from)?;
        Ok(rec)
    }

    pub(super) fn container_is_done(&self) -> Option<bool> {
        let ctx = &self.ctx;
        match self.id {
            "container.engine" => Some(ContainerRecord::load(&ctx.req.dir).is_some()),
            "build.db" if ctx.req.db == DbChoice::Container => Some(ProvisionState::load(&db_state_path(ctx)).map(|s| s.granted).unwrap_or(false) && ctx.req.dir.join("database.json").is_file()),
            "container.images" | "container.create" => Some(false),
            _ => None,
        }
    }

    pub(super) fn container_engine(&self) -> Result<String> {
        let ctx = &*self.ctx;
        let want = ctx.req.container.clone().unwrap_or_default();
        ctx.emit(self.id, StepStatus::Running, "Looking for a container engine and trying a test container", 0.2);
        let (engine, status, check) = container::setup::choose_engine(want.engine)?;
        let s = slug(&ctx.req.name);
        let rec = container::setup::new_record(&ctx.req.dir, &s, &engine, status.as_ref(), &want, ctx.req.db == DbChoice::Container).map_err(Problem::from)?;
        io(rec.save(&ctx.req.dir), "cannot write kora-container.json")?;
        // The record is NOT listed in `created.files`: the config step's roll-back removes those files, and it runs before the
        // database and engine steps' roll-back, which read this record to know what to remove (the database container, the
        // network and the volume stayed behind). The engine step's own undo removes the record, last.
        for n in status.iter().flat_map(|s| s.notes.iter()) {
            ctx.note(n.clone());
        }
        if let Some(p) = status.as_ref().and_then(|s| s.problem.as_ref()) {
            ctx.note(
                format!(
                    "{} {}",
                    p.message,
                    p.actions.iter().find_map(|a| if let Action::ShowCommand { command, .. } = a { Some(format!("Run: {}", command.join(" "))) } else { None }).unwrap_or_default()
                )
                .trim(),
            );
        }
        Ok(format!("Using {} ({})", engine.label(), check.summary))
    }

    pub(super) fn container_images(&self) -> Result<String> {
        let ctx = &*self.ctx;
        let rec = self.container_record()?;
        let r = lifecycle::system();
        let m = Manager::new(&ctx.req.dir, &rec, &r);
        let mut say = |l: &str| ctx.emit(self.id, StepStatus::Running, tidy_line(l), 0.5);
        m.ensure_image(&rec.image, &mut say)?;
        if let Some(db) = &rec.db {
            m.ensure_image(&db.image, &mut say)?;
        }
        Ok("The container images are ready".into())
    }

    pub(super) fn container_database(&self) -> Result<String> {
        let ctx = &*self.ctx;
        let rec = self.container_record()?;
        let db = rec.db.clone().ok_or_else(|| Problem::generic("this server was not set up with a database container"))?;
        let r = lifecycle::system();
        let m = Manager::new(&ctx.req.dir, &rec, &r);
        let state_path = db_state_path(ctx);
        io(std::fs::create_dir_all(ctx.meta_dir()), "cannot create state folder")?;
        let mut say = |l: &str| ctx.emit(self.id, StepStatus::Running, tidy_line(l), 0.5);
        let mut fresh = |c: &Credentials| {
            let _ = ctx.update_created(|x| x.files.push("database.json".into()));
            let _ = ProvisionState { database: c.database.clone(), user: c.user.clone(), host: "%".into(), ..Default::default() }.save(&state_path);
        };
        let creds = container::setup::ensure_database(&m, &slug(&ctx.req.name), &mut say, &mut fresh)?;
        ProvisionState { database: creds.database.clone(), user: creds.user.clone(), host: "%".into(), created_database: false, created_user: false, granted: true }.save(&state_path)?;
        ctx.shared.lock().unwrap().db_name = Some(creds.database.clone());
        Ok(format!("Database {} is ready in the container {} (login {})", creds.database, db.container, creds.user))
    }

    pub(super) fn container_create(&self) -> Result<String> {
        let ctx = &*self.ctx;
        let rec = self.container_record()?;
        let r = lifecycle::system();
        let m = Manager::new(&ctx.req.dir, &rec, &r);
        let mut say = |l: &str| ctx.emit(self.id, StepStatus::Running, tidy_line(l), 0.4);
        let boot = container::setup::create_game(&m, &mut say)?;
        for n in &boot.notes {
            ctx.note(n.clone());
        }
        if rec.engine().bypasses_host_firewall() && ctx.req.firewall {
            ctx.note(format!("{} publishes the game port with its own firewall rules, so it is reachable even where ufw or firewalld say it is closed.", rec.engine.label()));
        }
        Ok(format!("The server's container {} is created ({})", rec.container, boot.detail))
    }

    /// Undo of the container steps; `None` when `self.id` is not one of them (and `build.db` of another mode).
    pub(super) fn undo_container(&self) -> Option<Result<String>> {
        let ctx = &*self.ctx;
        let relevant = matches!(self.id, "container.engine" | "container.images" | "container.create") || (self.id == "build.db" && ctx.req.db == DbChoice::Container);
        if !relevant {
            return None;
        }
        Some(self.undo_container_inner())
    }

    fn undo_container_inner(&self) -> Result<String> {
        let ctx = &*self.ctx;
        if self.id == "container.images" {
            return Ok("The downloaded images were left in the engine's cache (`podman image prune` / `docker image prune` frees them)".into());
        }
        let Some(rec) = ContainerRecord::load(&ctx.req.dir) else { return Ok("Nothing to undo".into()) };
        if !ctx.system_trusted {
            ctx.note(format!(
                "Kora left this server's containers alone: {} is not in Kora's server list, so Kora cannot be sure which containers belong to it. Remove them by hand if needed (`{} ps -a --filter label={}=<name>`).",
                ctx.req.dir.display(),
                rec.engine().program(),
                container::LABEL_SERVER
            ));
            return Ok("Left alone (not in the server list; see notes)".into());
        }
        rec.check(&slug(&ctx.req.name)).map_err(Problem::from)?;
        let r = lifecycle::system();
        let m = Manager::new(&ctx.req.dir, &rec, &r);
        match self.id {
            "container.create" => Ok(m.remove_game()?.join("; ").trim().to_string()).map(|s| if s.is_empty() { "No container to remove".into() } else { s }),
            "build.db" => {
                let done = m.remove_db_side(!ctx.keep_db)?;
                if ctx.keep_db {
                    if let Some(db) = &rec.db {
                        ctx.note(format!(
                            "The database's data was kept in the volume {} ({} volume ls shows it). A server with the same name cannot be installed until you remove it.",
                            db.volume,
                            rec.engine().program()
                        ));
                    }
                } else {
                    let _ = std::fs::remove_file(ctx.req.dir.join("database.json"));
                }
                Ok(if done.is_empty() { "No database container to remove".into() } else { done.join("; ") })
            }
            // `container.engine`: the record itself, last.
            _ => {
                io(ContainerRecord::remove(&ctx.req.dir), "cannot remove kora-container.json")?;
                let leftover = m.leftovers();
                if !leftover.is_empty() {
                    ctx.note(format!("Still on the engine with this server's label: {}", leftover.join(", ")));
                }
                Ok("Removed the container record".into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(container: bool, db: &str, service: bool) -> InstallRequest {
        serde_json::from_value(serde_json::json!({
            "mode": "production", "name": "Main", "dir": "/home/alex/kora/main", "hostname": "Main",
            "license_key": null, "cfx_identifier": null, "ports": {"game": 30120, "txadmin": 40120},
            "artifact_url": null, "game_build": null, "resources": "All", "db": db,
            "service": service, "firewall": true, "github_owner": null, "catalog": null, "dry_run": false,
            "start": true,
            "container": if container { serde_json::json!({}) } else { serde_json::Value::Null },
        }))
        .unwrap()
    }

    fn ids(r: &InstallRequest, kora_user: bool) -> Vec<&'static str> {
        crate::installer::plan_steps_for(r, kora_user).iter().map(|p| p.id).collect()
    }

    #[test]
    fn the_container_plan_has_its_own_steps_in_order_and_no_service_or_account() {
        let r = request(true, "Container", false);
        assert!(r.is_container());
        assert_eq!(
            ids(&r, true),
            [
                "prepare.dir",
                "prepare.catalog",
                "prepare.tools",
                "container.engine",
                "container.images",
                "build.resources",
                "build.db",
                "build.artifact",
                "build.config",
                "system.firewall",
                "finalize",
                "container.create",
                "launch"
            ],
            "no `kora` account even when one would be planned, no service, no schedule"
        );
        let plan = crate::installer::plan_steps_for(&r, false);
        assert!(plan.iter().all(|p| !p.needs_admin || p.id == "system.firewall"), "only the optional firewall step needs administrator rights");
        assert_eq!(plan.iter().find(|p| p.id == "build.db").unwrap().title, "Create the database container");
        assert!(plan.iter().all(|p| p.skippable == crate::installer::SKIPPABLE.contains(&p.id)), "no container step may be skipped");
    }

    #[test]
    fn a_service_install_is_exactly_what_it_was() {
        let r = request(false, "UseExisting", true);
        let v = ids(&r, false);
        assert!(v.contains(&"system.service") && !v.iter().any(|i| i.starts_with("container.")), "{v:?}");
        // and a request saved before container mode (no `container` key at all) still reads
        let mut j = serde_json::to_value(&r).unwrap();
        j.as_object_mut().unwrap().remove("container");
        assert!(serde_json::from_value::<InstallRequest>(j).unwrap().container.is_none());
    }

    #[test]
    fn the_request_a_paused_install_keeps_carries_the_container_choice() {
        let mut r = request(true, "Container", false);
        r.container.as_mut().unwrap().engine = Some(container::EngineKind::Podman);
        r.license_key = Some("cfxk_secretvalue".into());
        let text = serde_json::to_string(&r).unwrap();
        assert!(text.contains("\"container\":{\"engine\":\"podman\"}"), "{text}");
        let back: InstallRequest = serde_json::from_str(&text).unwrap();
        assert_eq!(back.container, r.container);
    }
}
