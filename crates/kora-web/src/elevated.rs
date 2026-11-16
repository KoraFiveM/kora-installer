//! Privileged actions for the panel: through the helper when it is installed, otherwise the
//! `needs_elevation` answer with the command to run (exactly as before the helper existed).
//!
//! `privileged` takes the `kora ...` command the handler used to hand to the user. The first word
//! (`kora`) is dropped, the next is the helper verb, the rest are its arguments. Secrets (licence
//! key, database password) go in `env`, never in the command.

use std::sync::{Arc, Mutex};

use kora_core::helper::client::{Helper, HelperError, SocketClient};
use serde_json::{json, Value};

use crate::error::ApiError;

type Slot = Mutex<Option<Arc<dyn Helper>>>;
static OVERRIDE: Slot = Mutex::new(None);

/// Test seam: use this client instead of the real socket (`None` = the real one again).
pub fn set_helper_override(h: Option<Arc<dyn Helper>>) {
    *OVERRIDE.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = h;
}

fn client() -> Arc<dyn Helper> {
    OVERRIDE.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone().unwrap_or_else(|| Arc::new(SocketClient::default()))
}

/// Is the helper installed and answering? (`GET /system` `helper_installed`.)
pub fn helper_installed() -> bool {
    client().available()
}

const DEFAULT_MESSAGE: &str = "This needs administrator rights. Run this command as administrator:";

/// Run `command` (`["kora", verb, args...]`) with administrator rights.
/// Helper present: runs it there, `on_line` receives each output line, returns
/// `{"via_helper": true, "output": [last lines]}`. No helper: `needs_elevation`.
pub fn privileged(command: Vec<String>, env: Vec<(String, String)>, on_line: &mut dyn FnMut(&str)) -> Result<Value, ApiError> {
    privileged_msg(DEFAULT_MESSAGE, command, env, on_line)
}

/// [`privileged`] with the handler's own `needs_elevation` message.
pub fn privileged_msg(message: &str, command: Vec<String>, env: Vec<(String, String)>, on_line: &mut dyn FnMut(&str)) -> Result<Value, ApiError> {
    let needs = |c: Vec<String>| ApiError::needs_elevation(message, c);
    let h = client();
    let mut parts = command.iter().map(String::as_str);
    if parts.next() != Some("kora") {
        return Err(needs(command));
    }
    let Some(verb) = parts.next().map(str::to_string) else { return Err(needs(command)) };
    let args: Vec<String> = parts.map(str::to_string).collect();
    if !h.available() {
        return Err(needs(command));
    }
    let mut tail: Vec<String> = Vec::new();
    let res = h.run(&verb, &args, &env, &mut |l| {
        on_line(l);
        tail.push(l.to_string());
        if tail.len() > 20 {
            tail.remove(0);
        }
    });
    match res {
        Ok(o) if o.ok => Ok(json!({"via_helper": true, "output": tail})),
        Ok(o) => {
            let last = tail.iter().rev().find(|l| !l.trim().is_empty()).cloned().unwrap_or_default();
            let detail = if last.is_empty() { String::new() } else { format!(" The last thing it said: {last}") };
            Err(ApiError::core(format!("The change could not be made (it stopped with code {}).{detail}", o.code)))
        }
        // The helper went away between the check and the run: fall back to the command.
        Err(HelperError::Unavailable(_)) => Err(needs(command)),
        Err(HelperError::Rejected(m) | HelperError::Protocol(m)) => Err(ApiError::core(m)),
    }
}

/// Environment for a privileged install: only the names the helper accepts, and only when set.
pub fn secret_env(license_key: Option<&str>, db_admin_password: Option<&str>) -> Vec<(String, String)> {
    let mut v = Vec::new();
    if let Some(k) = license_key.filter(|k| !k.is_empty()) {
        v.push(("CFX_LICENSE_KEY".to_string(), k.to_string()));
    }
    if let Some(p) = db_admin_password.filter(|p| !p.is_empty()) {
        v.push(("KORA_DB_ADMIN_PASSWORD".to_string(), p.to_string()));
    }
    v
}
