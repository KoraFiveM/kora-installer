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
use crate::container::dbc;
use crate::container::detect;
use crate::container::lifecycle::{self, Manager};
use crate::container::record::{ContainerRecord, DbRecord};
use crate::container::{self, pins};

/// A progress line from the engine (a pull prints one per layer): short and without control characters.
fn tidy_line(l: &str) -> String {
    l.chars().filter(|c| !c.is_control()).take(160).collect()
}

fn no_engine(check: &container::detect::ContainerCheck, wanted: Option<container::EngineKind>) -> Problem {
    use crate::container::codes;
    let mut p = match (wanted, check.engines.iter().find(|e| wanted.is_none_or(|w| e.kind == w)).and_then(|e| e.problem.clone())) {
        (_, Some(p)) => p,
        (Some(w), None) => Problem::new(codes::NO_ENGINE, format!("{} was chosen to run the server, but it was not found on this computer.", w.label())),
        (None, None) => Problem::new(codes::NO_ENGINE, "Container mode needs Podman or Docker, and neither was found on this computer. Podman (rootless) needs no administrator rights to run servers."),
    };
    p.push_action(Action::open_link("https://podman.io/docs/installation"));
    p.push_action(Action::change_answers("run_mode"));
    p
}

fn dir_owner_ids(dir: &Path) -> Option<(u32, u32)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(dir).ok().map(|m| (m.uid(), m.gid()))
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        None
    }
}

impl InstallStep {
    /// The folder's record, checked against the server list's name for it.
    fn container_record(&self) -> Result<ContainerRecord> {
        let ctx = &*self.ctx;
        let rec = ContainerRecord::load(&ctx.req.dir).ok_or_else(|| InstallError(format!("{} has no container record (kora-container.json)", ctx.req.dir.display())))?;
        rec.check(&slug(&ctx.req.name)).map_err(InstallError)?;
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
        if !cfg!(target_os = "linux") {
            return err("Container mode is only available on Linux.");
        }
        let want = ctx.req.container.clone().unwrap_or_default();
        ctx.emit(self.id, StepStatus::Running, "Looking for a container engine and trying a test container", 0.2);
        let check = detect::check_system();
        let Some(engine) = check.engine_for(want.engine) else { return Err(no_engine(&check, want.engine).into()) };
        let status = check.engines.iter().find(|e| e.kind == engine.kind).cloned();
        let (uid, gid) = dir_owner_ids(&ctx.req.dir).ok_or_else(|| InstallError("cannot read who owns the server folder".into()))?;
        if uid == 0 || gid == 0 {
            return err("The server folder belongs to root. A container server runs as the folder's owner and never as root: choose a folder in your own home, or change its owner.");
        }
        let s = slug(&ctx.req.name);
        let rec = ContainerRecord {
            version: container::record::VERSION,
            engine: engine.kind,
            rootless: engine.rootless,
            image: pins::BASE.reference(),
            container: container::container_name(&s),
            uid,
            gid,
            bind: want.bind.clone(),
            memory_mb: want.memory_mb,
            cpu_tenths: want.cpu_tenths,
            selinux_label: status.as_ref().is_some_and(|s| s.selinux),
            read_only_root: false,
            host_network: false,
            db: (ctx.req.db == DbChoice::Container).then(|| DbRecord {
                container: container::db_container_name(&s),
                volume: container::volume_name(&s),
                network: container::network_name(&s),
                image: pins::MARIADB.reference(),
            }),
        };
        container::spec::check_dir(&ctx.req.dir).map_err(InstallError)?;
        rec.check(&s).map_err(InstallError)?;
        io(rec.save(&ctx.req.dir), "cannot write kora-container.json")?;
        ctx.update_created(|c| {
            if !c.files.iter().any(|f| f == container::record::FILE) {
                c.files.push(container::record::FILE.into());
            }
        })?;
        for n in status.iter().flat_map(|s| s.notes.iter()) {
            ctx.note(n.clone());
        }
        if let Some(p) = status.as_ref().and_then(|s| s.problem.as_ref()) {
            ctx.note(format!("{} {}", p.message, p.actions.iter().find_map(|a| if let Action::ShowCommand { command, .. } = a { Some(format!("Run: {}", command.join(" "))) } else { None }).unwrap_or_default()).trim().to_string());
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
        let db = rec.db.clone().ok_or_else(|| InstallError("this server was not set up with a database container".into()))?;
        let r = lifecycle::system();
        let m = Manager::new(&ctx.req.dir, &rec, &r);
        let s = slug(&ctx.req.name);
        let path = ctx.req.dir.join("database.json");
        let state_path = db_state_path(ctx);
        io(std::fs::create_dir_all(ctx.meta_dir()), "cannot create state folder")?;

        // A retry reuses the login an earlier attempt already chose (the volume may hold it).
        let reused: Option<Credentials> = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok());
        let creds = match reused {
            Some(c) => c,
            None => {
                let vol_exists = crate::container::engine::Engine::new(rec.engine, rec.rootless).run(&r, &container::engine::volume_inspect_args(&db.volume)).is_ok_and(|o| o.success());
                if vol_exists {
                    return Err(Problem::new(
                        container::codes::NAME_TAKEN,
                        format!(
                            "A database volume called {} is already there, probably kept when an earlier server with this name was deleted. Its login is not known any more, so Kora will not use it. Choose another server name, or remove the volume yourself (`{} volume rm {}`) if you do not need its data.",
                            db.volume,
                            rec.engine().program(),
                            db.volume
                        ),
                    )
                    .with_action(Action::change_answers("name"))
                    .into());
                }
                let name = dbc::new_names().map_err(|e| InstallError(e.to_string()))?;
                let pw = db::generate_password().map_err(|e| InstallError(e.to_string()))?;
                let creds = dbc::game_credentials(&s, &name, &name, &pw);
                // Written before the container exists: the volume is initialised with it on the first start.
                ctx.update_created(|c| c.files.push("database.json".into()))?;
                let json = serde_json::to_string_pretty(&creds).map_err(|e| InstallError(e.to_string()))?;
                db::write_secret_file(&db::SystemRunner, db::Os::current(), &path, &json, None).map_err(|e| InstallError(e.to_string()))?;
                ProvisionState { database: creds.database.clone(), user: creds.user.clone(), host: "%".into(), ..Default::default() }.save(&state_path).map_err(|e| InstallError(e.to_string()))?;
                creds
            }
        };

        let mut say = |l: &str| ctx.emit(self.id, StepStatus::Running, tidy_line(l), 0.3);
        if matches!(m.db_state(), Some(container::engine::ContainerState::Missing)) {
            dbc::create_container(&m, &creds, &mut say)?;
        }
        ctx.emit(self.id, StepStatus::Running, "Starting the database container", 0.5);
        m.start_db()?;
        ctx.emit(self.id, StepStatus::Running, "Waiting for the database to be ready (the first start takes a little while)", 0.6);
        let exec = dbc::ExecRunner::new(&rec, &db::SystemRunner).ok_or_else(|| InstallError("no database container".into()))?;
        dbc::wait_ready(&exec, &creds, std::time::Duration::from_secs(180), &std::thread::sleep)?;
        ProvisionState { database: creds.database.clone(), user: creds.user.clone(), host: "%".into(), created_database: false, created_user: false, granted: true }
            .save(&state_path)
            .map_err(|e| InstallError(e.to_string()))?;
        ctx.shared.lock().unwrap().db_name = Some(creds.database.clone());
        Ok(format!("Database {} is ready in the container {} (login {})", creds.database, db.container, creds.user))
    }

    pub(super) fn container_create(&self) -> Result<String> {
        let ctx = &*self.ctx;
        let rec = self.container_record()?;
        let r = lifecycle::system();
        let m = Manager::new(&ctx.req.dir, &rec, &r);
        let mut say = |l: &str| ctx.emit(self.id, StepStatus::Running, tidy_line(l), 0.4);
        m.ensure_game_container(&mut say)?;
        // Start at boot: Docker's restart policy needs nothing more (its daemon is enabled by the system); rootless
        // Podman's restarts come from the stock user unit `podman-restart.service`, which needs lingering to run
        // while nobody is signed in. Both are the person's own account: no administrator rights.
        let mut boot = format!("{} restarts the server by itself (restart policy) and at boot", rec.engine().label());
        if rec.engine == container::EngineKind::Podman && rec.rootless {
            let sys = service::SystemRunner;
            let enable = service::SystemStep::run("Start the server's containers at boot", "systemctl", &["--user", "enable", "podman-restart.service"]).user_level().tolerant();
            let linger = service::SystemStep::run("Keep them running when you are signed out", "loginctl", &["enable-linger"]).user_level().tolerant();
            let mut failed = Vec::new();
            for step in [enable, linger] {
                let line = step.command_line().unwrap_or_default();
                if service::apply_plan(&service::Plan::new(vec![service::SystemStep { ignore_failure: false, ..step }]), &sys).is_err() {
                    failed.push(line);
                }
            }
            if failed.is_empty() {
                boot = "Podman starts the server at boot (podman-restart.service, lingering on)".into();
            } else {
                ctx.note(format!(
                    "The server will run, but may not start again after a restart of this computer until you run: {}. (Rootless Podman needs the user unit `podman-restart.service` enabled and lingering on for your account.)",
                    failed.join(" ; ")
                ));
                boot = "created; start at boot needs the command in the notes".into();
            }
        }
        if rec.engine().bypasses_host_firewall() && ctx.req.firewall {
            ctx.note(format!("{} publishes the game port with its own firewall rules, so it is reachable even where ufw or firewalld say it is closed.", rec.engine.label()));
        }
        Ok(format!("The server's container {} is created ({boot})", rec.container))
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
        rec.check(&slug(&ctx.req.name)).map_err(InstallError)?;
        let r = lifecycle::system();
        let m = Manager::new(&ctx.req.dir, &rec, &r);
        match self.id {
            "container.create" => Ok(m.remove_game()?.join("; ").trim().to_string()).map(|s| if s.is_empty() { "No container to remove".into() } else { s }),
            "build.db" => {
                let done = m.remove_db_side(!ctx.keep_db)?;
                if ctx.keep_db {
                    if let Some(db) = &rec.db {
                        ctx.note(format!("The database's data was kept in the volume {} ({} volume ls shows it). A server with the same name cannot be installed until you remove it.", db.volume, rec.engine().program()));
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
            ["prepare.dir", "prepare.catalog", "prepare.tools", "container.engine", "container.images", "build.resources", "build.db", "build.artifact", "build.config", "system.firewall", "container.create", "finalize", "launch"],
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
