//! Opening the game port in this computer's firewall for a server that already exists (the "fix"
//! button of the reachability checklist; `kora reach <server> --open-firewall`). Uses the install's
//! own plan ([`crate::firewall::open_plan`]) and record, so the server's delete removes the same rules.
//! Only the game port (TCP and UDP): txAdmin's port is never opened here.

use crate::firewall::{self, PortRequest};
use crate::installer::registry::ServerEntry;
use crate::problem::{codes, Action, Problem};
use crate::service::{self, Platform, SystemRunner};

/// Needs administrator rights (the caller checks and asks for them first).
pub fn open_game_port(e: &ServerEntry) -> Result<Vec<String>, String> {
    if !crate::helper::winhelper::port_allowed(e.ports.game) {
        return Err(format!("Kora does not open port {} in the firewall: Windows, remote administration or databases use it. Choose another game port first.", e.ports.game));
    }
    let rn = SystemRunner;
    let Some(kind) = firewall::detect(Platform::current(), &rn) else {
        return Ok(vec!["No active firewall was found, so there was nothing to open.".into()]);
    };
    let req = PortRequest { server: crate::installer::slug(&e.name), game_port: e.ports.game, txadmin_port: e.ports.txadmin, expose_txadmin: false };
    let plan = firewall::open_plan(&req, kind).map_err(|x| x.to_string())?;
    if !crate::elevate::is_elevated() {
        // The command to run as administrator. (Seam: the privileged-service work can offer the
        // permission prompt / helper for this verb; until then the person runs it.)
        let command = vec!["kora".to_string(), "reach".into(), e.name.clone(), "--open-firewall".into()];
        return Err(Problem::new(codes::NEEDS_ELEVATION, "Changing the firewall needs administrator rights. Run this command as administrator, then check again:")
            .with_action(Action::ShowCommand { command, elevated: true })
            .into());
    }
    // Recorded first (like the install does), so deleting the server removes these rules again.
    let mut created = crate::installer::load_created_from(&e.dir).unwrap_or_default();
    if !created.firewall {
        created.firewall = true;
        std::fs::create_dir_all(e.dir.join(".kora")).map_err(|x| x.to_string())?;
        crate::installer::save_created_to(&e.dir, &created).map_err(|x| x.0)?;
    }
    service::apply_plan(&plan, &rn).map_err(|x| x.to_string())?;
    Ok(vec![format!("Opened port {} (TCP and UDP) in the firewall.", e.ports.game)])
}
