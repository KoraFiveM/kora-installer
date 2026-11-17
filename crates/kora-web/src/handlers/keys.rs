//! Saved licence keys, the key of an installed server, and the remembered profile.
//!
//! Every answer here shows a key as its name and its last four characters only (`KeyInfo`): the full key
//! never leaves `kora_core::keystore` except into the server's own configuration. Adding, renaming,
//! replacing and deleting keys, and pasting a new key anywhere, is for admins and local sessions; other
//! roles can pick from the saved names.
#![allow(clippy::unused_async)]

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::{Extension, Json};
use kora_core::api::Status;
use kora_core::installer::paused::{self, PausedStore};
use kora_core::keystore::{KeyInfo, KeyStore};
use kora_core::problem::{codes, Action, Problem};
use kora_core::profile::{Field, Profile, ProfileStore};
use serde_json::{json, Value};

use crate::remote::{Caller, Role};
use crate::{blocking, ApiError, AppState};

/// A key-store [`Problem`] as an API error with a fitting status.
pub fn api(p: Problem) -> ApiError {
    let status = match p.code.as_str() {
        codes::KEYS_NOT_FOUND => StatusCode::NOT_FOUND,
        codes::KEYS_NAME_TAKEN | codes::KEYS_DUPLICATE | codes::KEYS_SERVER_CONFIG => StatusCode::CONFLICT,
        codes::KEYS_INVALID => StatusCode::BAD_REQUEST,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    ApiError::problem(status, p)
}

/// Local sessions and remote admins may manage keys.
pub fn may_manage(c: &Caller) -> bool {
    !c.remote || c.identity.as_ref().is_some_and(|i| i.role >= Role::Admin)
}

/// Refuse a remote caller below admin who tries to add or paste a key.
pub fn require_manage(c: &Caller) -> Result<(), ApiError> {
    if may_manage(c) {
        Ok(())
    } else {
        Err(ApiError::forbidden("Only an admin can add a new licence key. Pick one of the saved keys, or ask an admin to add it."))
    }
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn key_json(k: &KeyInfo) -> Value {
    json!({"name": k.name, "last4": k.last4, "added": k.added})
}

/// `GET /keys` → `{keys: [{name, last4, added, servers: [name], stopped_installs}], can_manage, path}`.
/// Never a full key. `path` (where the file is) is only for local callers.
pub async fn keys_list(State(st): State<AppState>, Extension(c): Extension<Caller>) -> Result<Json<Value>, ApiError> {
    let can_manage = may_manage(&c);
    let remote = c.remote;
    blocking(move || {
        let servers = st.backend().list().map_err(ApiError::internal)?;
        let store = KeyStore::default();
        let pending = PausedStore::default();
        let saved = kora_core::installer::saved::path();
        let keys: Vec<Value> = store
            .list_with_servers(&servers)
            .map_err(api)?
            .iter()
            .map(|(k, used)| {
                let mut v = key_json(k);
                v["servers"] = json!(used);
                v["stopped_installs"] = json!(pending.count_key_with_saved(&saved, &k.name));
                v
            })
            .collect();
        Ok(Json(json!({"keys": keys, "can_manage": can_manage, "path": (!remote).then(|| store.path().display().to_string())})))
    })
    .await
}

/// `POST /keys` `{name, license_key}` → `201 {key: {name, last4, added}}`.
pub async fn key_add(Extension(c): Extension<Caller>, Json(body): Json<Value>) -> Result<(StatusCode, Json<Value>), ApiError> {
    require_manage(&c)?;
    let (name, key) = (text(&body, "name").unwrap_or_default(), text(&body, "license_key").unwrap_or_default());
    blocking(move || {
        let k = KeyStore::default().add(&name, &key).map_err(api)?;
        Ok((StatusCode::CREATED, Json(json!({"key": key_json(&k)}))))
    })
    .await
}

/// `PUT /keys/{name}` `{new_name?, license_key?}` → `{key, notes}`. Rename, replace the key, or both.
/// Stopped installs and saved answers that used the old name follow a rename. Servers already
/// installed keep the key they have.
pub async fn key_update(Extension(c): Extension<Caller>, Path(name): Path<String>, Json(body): Json<Value>) -> Result<Json<Value>, ApiError> {
    require_manage(&c)?;
    let (new_name, key) = (text(&body, "new_name"), text(&body, "license_key"));
    if new_name.is_none() && key.is_none() {
        return Err(ApiError::bad_request("Give a new name or a new key."));
    }
    blocking(move || {
        let k = KeyStore::default().update(&name, new_name.as_deref(), key.as_deref()).map_err(api)?;
        let mut notes = vec![];
        if name != k.name {
            let saved = kora_core::installer::saved::path();
            let n = PausedStore::default().count_key_with_saved(&saved, &name);
            PausedStore::default().rename_key(&name, &k.name);
            paused::rename_key_in_saved_answers(&saved, &name, &k.name);
            if n > 0 {
                notes.push(format!("{n} unfinished install{} now use{} the new name.", if n == 1 { "" } else { "s" }, if n == 1 { "s" } else { "" }));
            }
        }
        if key.is_some() {
            notes.push("Servers that were already installed with the old key keep using it. Use \"Change licence key\" on a server to switch it.".to_string());
        }
        Ok(Json(json!({"key": key_json(&k), "notes": notes})))
    })
    .await
}

/// `DELETE /keys/{name}` → `{removed, notes}`. Servers installed with the key are not touched; an
/// unfinished install that used it is told it needs another key.
pub async fn key_remove(Extension(c): Extension<Caller>, Path(name): Path<String>) -> Result<Json<Value>, ApiError> {
    require_manage(&c)?;
    blocking(move || {
        let waiting = PausedStore::default().count_key_with_saved(&kora_core::installer::saved::path(), &name);
        KeyStore::default().remove(&name).map_err(api)?;
        let mut notes = vec!["Servers that were installed with this key keep working: they have their own copy.".to_string()];
        if waiting > 0 {
            notes.push(format!(
                "{waiting} unfinished install{} used this key and will need another one picked before {} can carry on.",
                if waiting == 1 { "" } else { "s" },
                if waiting == 1 { "it" } else { "they" }
            ));
        }
        Ok(Json(json!({"removed": true, "notes": notes})))
    })
    .await
}

/// `PUT /servers/{s}/licence-key` `{name}` (a saved key) or `{license_key, save_as?}` (a new one) →
/// `{server, last4, key_name, running, restart_needed, message}`. Writes the key where the server's
/// configuration keeps it. A running server only reads it when it starts: `restart_needed` says so and
/// the panel offers the restart (`POST /servers/{s}/control`).
pub async fn server_key_set(
    State(st): State<AppState>,
    Extension(c): Extension<Caller>,
    Path(s): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let (name, pasted, save_as) = (text(&body, "name"), text(&body, "license_key"), text(&body, "save_as").filter(|n| !n.trim().is_empty()));
    if pasted.is_some() || save_as.is_some() {
        require_manage(&c)?;
    }
    if name.is_some() == pasted.is_some() {
        return Err(ApiError::bad_request("Pick one of the saved keys, or paste a new key (not both).").with_action(Action::change_answers("licence")));
    }
    blocking(move || {
        let e = st.entry(&s)?;
        let store = KeyStore::default();
        let (key, key_name) = match (&name, &pasted) {
            (Some(n), _) => (store.get(n).map_err(api)?, Some(n.clone())),
            (_, Some(k)) => {
                let k = kora_core::keystore::check_key(k).map_err(api)?;
                let saved_as = match (&save_as, store.name_of(&k).map_err(api)?) {
                    (_, Some(existing)) => Some(existing),
                    (Some(n), None) => Some(store.add(n, &k).map_err(api)?.name),
                    (None, None) => None,
                };
                (k, saved_as)
            }
            _ => unreachable!("checked above"),
        };
        let status = st.backend().status(&e);
        if status == Status::Missing {
            return Err(ApiError::conflict("The server's folder is missing, so its licence key cannot be changed."));
        }
        kora_core::keystore::set_server_key(&e.dir, &key).map_err(api)?;
        let running = status == Status::Running;
        let message = if running {
            "The new licence key is saved in the server's configuration. The server only reads it when it starts, so restart it to switch over."
        } else {
            "The new licence key is saved in the server's configuration. No restart is needed: the server is stopped and will use it the next time it starts."
        };
        Ok(Json(json!({
            "server": e.name,
            "last4": kora_core::keystore::last4(&key),
            "key_name": key_name,
            "running": running,
            "restart_needed": running,
            "message": message,
        })))
    })
    .await
}

fn profile_json(p: &Profile) -> Value {
    json!({"cfx_username": p.cfx_username, "cfx_identifier": p.cfx_identifier, "github_owner": p.github_owner})
}

/// `GET /profile` → `{cfx_username, cfx_identifier, github_owner}` (each `null` until first used).
pub async fn profile_get() -> Result<Json<Value>, ApiError> {
    blocking(|| Ok(Json(profile_json(&ProfileStore::default().load())))).await
}

/// One field of `PUT /profile`: left out = keep, `null` = forget, text = set.
fn field(body: &Value, key: &str, check: impl Fn(&str) -> Result<(), String>) -> Result<Field, ApiError> {
    match body.get(key) {
        None => Ok(Field::Keep),
        Some(Value::Null) => Ok(Field::Forget),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(Field::Forget),
        Some(Value::String(s)) => {
            check(s.trim()).map_err(ApiError::bad_request)?;
            Ok(Field::Set(s.trim().to_string()))
        }
        Some(_) => Err(ApiError::bad_request("That request was not understood. Reload the page and try again.")),
    }
}

/// `PUT /profile` `{cfx_username?, cfx_identifier?, github_owner?}` → the profile. `null` forgets a field.
pub async fn profile_set(Json(body): Json<Value>) -> Result<Json<Value>, ApiError> {
    let name = field(&body, "cfx_username", |n| {
        if kora_core::cfx::valid_username(n.trim_start_matches('@')) {
            Ok(())
        } else {
            Err("A Cfx.re username has 3 to 20 letters, numbers, dots, dashes or underscores.".into())
        }
    })?;
    let ident = field(&body, "cfx_identifier", |i| {
        if i.strip_prefix("fivem:").is_some_and(|d| !d.is_empty() && d.len() <= 12 && d.bytes().all(|b| b.is_ascii_digit())) {
            Ok(())
        } else {
            Err("A Cfx.re identifier looks like fivem:123456.".into())
        }
    })?;
    let owner = field(&body, "github_owner", kora_core::git::owners::check_login_syntax)?;
    blocking(move || {
        let p = ProfileStore::default()
            .update(name, ident, owner)
            .map_err(|e| ApiError::internal(format!("Kora could not save this ({e}).")))?;
        Ok(Json(profile_json(&p)))
    })
    .await
}
