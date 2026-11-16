//! Remote access settings and users (local sessions only; the guard refuses remote callers, and each
//! handler checks again). Token hashes are never returned; a new token is shown once, when it is made.
use std::collections::BTreeMap;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::remote::config::{valid_host, valid_user_name};
use crate::remote::{Caller, Role};
use crate::{blocking, ApiError, AppState};

fn local_only(c: &Caller) -> Result<(), ApiError> {
    if c.remote {
        Err(ApiError::forbidden(
            "This can only be done on the computer that runs Kora, not over remote access.",
        ))
    } else {
        Ok(())
    }
}

fn view(st: &AppState) -> Value {
    let c = st.remote.config();
    json!({
        "enabled": c.enabled,
        // On for this run because of `--remote`, whatever the saved setting says.
        "enabled_by_flag": st.remote.is_forced() && !c.enabled,
        "bind": st.addr.ip().to_string(),
        "port": st.addr.port(),
        // Set while the panel also listens on Tailscale (`kora panel --tailscale`): where to open it there.
        "tailnet_url": st.remote.tailnet_url(),
        "tailscale": c.tailscale,
        "tailscale_users": c.tailscale_users,
        "allowed_hosts": c.allowed_hosts,
        "allow_remote_destructive": c.allow_remote_destructive,
        "users": c.users.iter().map(|u| json!({"name": u.name, "role": u.role, "created": u.created})).collect::<Vec<_>>(),
        "saved_to": st.remote.path().map(|p| p.display().to_string()),
    })
}

/// `GET /remote`: `{enabled, enabled_by_flag, bind, port, tailnet_url, tailscale, tailscale_users: {login: role},
/// allowed_hosts, allow_remote_destructive, users: [{name, role, created}], saved_to}`.
pub async fn remote_get(
    State(st): State<AppState>,
    Extension(c): Extension<Caller>,
) -> Result<Json<Value>, ApiError> {
    local_only(&c)?;
    blocking(move || Ok(Json(view(&st)))).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteSet {
    enabled: Option<bool>,
    tailscale: Option<bool>,
    tailscale_users: Option<BTreeMap<String, Role>>,
    allowed_hosts: Option<Vec<String>>,
    allow_remote_destructive: Option<bool>,
}

/// `PUT /remote`: any of `enabled`, `tailscale`, `tailscale_users`, `allowed_hosts`,
/// `allow_remote_destructive` (what is left out stays as it is). Answers like `GET /remote`.
pub async fn remote_set(
    State(st): State<AppState>,
    Extension(c): Extension<Caller>,
    Json(body): Json<RemoteSet>,
) -> Result<Json<Value>, ApiError> {
    local_only(&c)?;
    if let Some(hosts) = &body.allowed_hosts {
        for h in hosts {
            valid_host(h.trim()).map_err(ApiError::bad_request)?;
        }
    }
    if let Some(users) = &body.tailscale_users {
        if users
            .keys()
            .any(|k| k.trim().is_empty() || k.contains(char::is_whitespace))
        {
            return Err(ApiError::bad_request(
                "A Tailscale login looks like name@example.com.",
            ));
        }
    }
    blocking(move || {
        st.remote
            .update(|cfg| {
                if let Some(v) = body.enabled {
                    cfg.enabled = v;
                }
                if let Some(v) = body.tailscale {
                    cfg.tailscale = v;
                }
                if let Some(v) = body.tailscale_users {
                    cfg.tailscale_users = v
                        .into_iter()
                        .map(|(k, r)| (k.trim().to_ascii_lowercase(), r))
                        .collect();
                }
                if let Some(v) = body.allowed_hosts {
                    let mut hosts: Vec<String> = Vec::new();
                    for h in v.into_iter().map(|h| h.trim().to_string()) {
                        if !hosts.iter().any(|x| x.eq_ignore_ascii_case(&h)) {
                            hosts.push(h);
                        }
                    }
                    cfg.allowed_hosts = hosts;
                }
                if let Some(v) = body.allow_remote_destructive {
                    cfg.allow_remote_destructive = v;
                }
                Ok(())
            })
            .map_err(ApiError::internal)?;
        Ok(Json(view(&st)))
    })
    .await
}

/// `GET /users`: `{users: [{name, role, created}]}`.
pub async fn users_list(
    State(st): State<AppState>,
    Extension(c): Extension<Caller>,
) -> Result<Json<Value>, ApiError> {
    local_only(&c)?;
    blocking(move || {
        let users: Vec<Value> = st
            .remote
            .config()
            .users
            .iter()
            .map(|u| json!({"name": u.name, "role": u.role, "created": u.created}))
            .collect();
        Ok(Json(json!({ "users": users })))
    })
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserAdd {
    name: String,
    role: Role,
}

/// `POST /users` `{name, role}` → 201 `{name, role, token}`. The token is shown this once.
pub async fn user_add(
    State(st): State<AppState>,
    Extension(c): Extension<Caller>,
    Json(body): Json<UserAdd>,
) -> Result<Response, ApiError> {
    local_only(&c)?;
    valid_user_name(body.name.trim()).map_err(ApiError::bad_request)?;
    blocking(move || {
        let name = body.name.trim().to_string();
        let role = body.role;
        let token = st
            .remote
            .update(|cfg| cfg.add_user(&name, role))
            .map_err(|m| {
                if m.starts_with("There is already") {
                    ApiError::conflict(m)
                } else {
                    ApiError::internal(m)
                }
            })?;
        Ok((
            StatusCode::CREATED,
            Json(json!({"name": name, "role": role, "token": token})),
        )
            .into_response())
    })
    .await
}

/// `DELETE /users/{name}` → 204. Their sessions stop working at once.
pub async fn user_remove(
    State(st): State<AppState>,
    Extension(c): Extension<Caller>,
    Path(name): Path<String>,
) -> Result<Response, ApiError> {
    local_only(&c)?;
    blocking(move || {
        let removed = st
            .remote
            .update(|cfg| Ok(cfg.remove_user(&name)))
            .map_err(ApiError::internal)?;
        if removed {
            Ok(StatusCode::NO_CONTENT.into_response())
        } else {
            Err(ApiError::not_found(format!(
                "There is no user named '{name}'."
            )))
        }
    })
    .await
}
