//! Remote access settings and users (local sessions only; the guard refuses remote callers, and each
//! handler checks again). Token hashes are never returned; a new token is shown once, when it is made.
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use kora_core::api::wire;

use crate::remote::auth::db;
use crate::remote::config::{valid_host, valid_user_name};
use crate::remote::{Caller, Role};
use crate::{blocking, ApiError, AppState};

fn local_only(c: &Caller) -> Result<(), ApiError> {
    if c.remote {
        Err(ApiError::forbidden("This can only be done on the computer that runs Kora, not over remote access."))
    } else {
        Ok(())
    }
}

fn panel_users(st: &AppState, c: &crate::remote::config::RemoteConfig) -> Vec<wire::PanelUser> {
    let store = st.remote.auth_store().ok();
    c.users
        .iter()
        .map(|u| {
            let s = store.as_ref().and_then(|s| db::state(s, &u.name).ok()).unwrap_or_default();
            wire::PanelUser { name: u.name.clone(), role: u.role.into(), created: u.created, passkeys: s.passkeys.len() as u32, code: s.totp && s.password, legacy_token: u.has_token() }
        })
        .collect()
}

fn view(st: &AppState) -> wire::RemoteSettings {
    let c = st.remote.config();
    wire::RemoteSettings {
        enabled: c.enabled,
        // On for this run because of `--remote`, whatever the saved setting says.
        enabled_by_flag: st.remote.is_forced() && !c.enabled,
        bind: st.addr.ip().to_string(),
        port: st.addr.port(),
        // Set while the panel also listens on Tailscale (`kora panel --tailscale`): where to open it there.
        tailnet_url: st.remote.tailnet_url(),
        tailscale: c.tailscale,
        tailscale_users: c.tailscale_users.iter().map(|(k, r)| (k.clone(), (*r).into())).collect(),
        allowed_hosts: c.allowed_hosts.clone(),
        allow_remote_destructive: c.allow_remote_destructive,
        users: panel_users(st, &c),
        saved_to: st.remote.path().map(|p| p.display().to_string()),
        require_strong_auth: c.require_strong_auth,
    }
}

/// `GET /remote`: `{enabled, enabled_by_flag, bind, port, tailnet_url, tailscale, tailscale_users: {login: role},
/// allowed_hosts, allow_remote_destructive, users: [{name, role, created}], saved_to}`.
pub async fn remote_get(State(st): State<AppState>, Extension(c): Extension<Caller>) -> Result<Json<wire::RemoteSettings>, ApiError> {
    local_only(&c)?;
    blocking(move || Ok(Json(view(&st)))).await
}

use wire::RemoteSettingsUpdate as RemoteSet;

/// `PUT /remote`: any of `enabled`, `tailscale`, `tailscale_users`, `allowed_hosts`,
/// `allow_remote_destructive` (what is left out stays as it is). Answers like `GET /remote`.
pub async fn remote_set(State(st): State<AppState>, Extension(c): Extension<Caller>, Json(body): Json<RemoteSet>) -> Result<Json<wire::RemoteSettings>, ApiError> {
    local_only(&c)?;
    if let Some(hosts) = &body.allowed_hosts {
        for h in hosts {
            valid_host(h.trim()).map_err(ApiError::bad_request)?;
        }
    }
    if let Some(users) = &body.tailscale_users {
        if users.keys().any(|k| k.trim().is_empty() || k.contains(char::is_whitespace)) {
            return Err(ApiError::bad_request("A Tailscale login looks like name@example.com."));
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
                    cfg.tailscale_users = v.into_iter().map(|(k, r)| (k.trim().to_ascii_lowercase(), Role::from(r))).collect();
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
                if let Some(v) = body.require_strong_auth {
                    cfg.require_strong_auth = v;
                }
                Ok(())
            })
            .map_err(ApiError::internal)?;
        Ok(Json(view(&st)))
    })
    .await
}

/// `GET /users`: `{users: [{name, role, created}]}`.
pub async fn users_list(State(st): State<AppState>, Extension(c): Extension<Caller>) -> Result<Json<wire::UsersList>, ApiError> {
    local_only(&c)?;
    blocking(move || Ok(Json(wire::UsersList { users: panel_users(&st, &st.remote.config()) }))).await
}

use wire::UserAddRequest as UserAdd;

/// `POST /users` `{name, role}` → 201 `{name, role, link, expires_in}`. The new user has no token: the one-time enrolment
/// link (shown this once, 30 minutes, single use) lets them register a passkey or set up one-time codes.
pub async fn user_add(State(st): State<AppState>, Extension(c): Extension<Caller>, Json(body): Json<UserAdd>) -> Result<Response, ApiError> {
    local_only(&c)?;
    valid_user_name(body.name.trim()).map_err(ApiError::bad_request)?;
    blocking(move || {
        let name = body.name.trim().to_string();
        let role = Role::from(body.role);
        let link = crate::remote::auth::invite(&st.remote, &name, role, "local", st.audit_home.as_deref()).map_err(|m| {
            if m.starts_with("There is already") {
                ApiError::conflict(m)
            } else {
                ApiError::internal(m)
            }
        })?;
        Ok((StatusCode::CREATED, Json(wire::NewUser { name, role: role.into(), link, expires_in: db::LINK_TTL as u64 })).into_response())
    })
    .await
}

fn credentials_of(st: &AppState, name: &str) -> Result<wire::UserCredentials, ApiError> {
    let cfg = st.remote.config();
    let u = cfg.user(name).ok_or_else(|| ApiError::not_found(format!("There is no user named '{name}'.")))?;
    let store = st.remote.auth_store().map_err(ApiError::internal)?;
    let s = db::state(&store, &u.name).map_err(ApiError::internal)?;
    Ok(wire::UserCredentials {
        name: u.name.clone(),
        role: u.role.into(),
        passkeys: s.passkeys.iter().map(|p| wire::PasskeyView { id: p.id, label: p.label.clone(), rp_id: p.rp_id.clone(), created: p.created, last_used: p.last_used }).collect(),
        code: s.totp && s.password,
        recovery_left: s.recovery_left as u32,
        legacy_token: u.has_token(),
        signed_in_devices: st.sessions.list_for(&u.name, None).len() as u32,
    })
}

/// `GET /users/{name}/credentials`: what the person can sign in with (counts and dates, never a value).
pub async fn user_credentials(State(st): State<AppState>, Extension(c): Extension<Caller>, Path(name): Path<String>) -> Result<Json<wire::UserCredentials>, ApiError> {
    local_only(&c)?;
    blocking(move || credentials_of(&st, &name).map(Json)).await
}

/// `POST /users/{name}/reset` `{what}`: take credentials away ("lost my device"), end their sessions and give them a new
/// enrolment link (not for `legacy_token`, which only ends the old token link).
pub async fn user_reset(State(st): State<AppState>, Extension(c): Extension<Caller>, Path(name): Path<String>, Json(body): Json<wire::ResetRequest>) -> Result<Json<wire::ResetDone>, ApiError> {
    local_only(&c)?;
    blocking(move || {
        let done = crate::remote::auth::reset(&st.remote, &name, body.what, "local", st.audit_home.as_deref()).map_err(|m| {
            if m.contains("no user named") {
                ApiError::not_found(m)
            } else {
                ApiError::internal(m)
            }
        })?;
        st.sessions.revoke_user(&done.name, None);
        Ok(Json(done))
    })
    .await
}

fn token_views(store: &kora_core::store::Store) -> Result<Vec<wire::ApiTokenView>, ApiError> {
    Ok(db::list_api_tokens(store)
        .map_err(ApiError::internal)?
        .into_iter()
        .map(|t| wire::ApiTokenView { id: t.id, name: t.name, role: t.role.into(), created: t.created, expires: t.expires, last_used: t.last_used })
        .collect())
}

/// `GET /api-tokens`.
pub async fn api_tokens_list(State(st): State<AppState>, Extension(c): Extension<Caller>) -> Result<Json<wire::ApiTokensList>, ApiError> {
    local_only(&c)?;
    blocking(move || Ok(Json(wire::ApiTokensList { tokens: token_views(&st.remote.auth_store().map_err(ApiError::internal)?)? }))).await
}

/// `POST /api-tokens` `{name, role, days?}` → 201 `{id, name, role, token}`: the token is shown this once.
pub async fn api_token_create(State(st): State<AppState>, Extension(c): Extension<Caller>, Json(body): Json<wire::ApiTokenRequest>) -> Result<Response, ApiError> {
    local_only(&c)?;
    if body.days == Some(0) {
        return Err(ApiError::bad_request("A token that lasts 0 days is already over. Leave the days out for a token that never expires."));
    }
    blocking(move || {
        let store = st.remote.auth_store().map_err(ApiError::internal)?;
        let role = Role::from(body.role);
        let token = db::create_api_token(&store, &body.name, role, body.days, "local").map_err(|m| if m.contains("already") { ApiError::conflict(m) } else { ApiError::bad_request(m) })?;
        let id = db::list_api_tokens(&store).map_err(ApiError::internal)?.into_iter().find(|t| t.name == body.name.trim()).map_or(0, |t| t.id);
        Ok((StatusCode::CREATED, Json(wire::NewApiToken { id, name: body.name.trim().to_string(), role: role.into(), token })).into_response())
    })
    .await
}

/// `DELETE /api-tokens/{id}` → 204.
pub async fn api_token_revoke(State(st): State<AppState>, Extension(c): Extension<Caller>, Path(id): Path<String>) -> Result<Response, ApiError> {
    local_only(&c)?;
    let id: i64 = id.parse().map_err(|_| ApiError::bad_request("That is not a token number."))?;
    blocking(move || {
        let store = st.remote.auth_store().map_err(ApiError::internal)?;
        match db::revoke_api_token(&store, id).map_err(ApiError::internal)? {
            Some(_) => Ok(StatusCode::NO_CONTENT.into_response()),
            None => Err(ApiError::not_found("There is no such API token.")),
        }
    })
    .await
}

/// `DELETE /users/{name}` → 204. Their sessions stop working at once.
pub async fn user_remove(State(st): State<AppState>, Extension(c): Extension<Caller>, Path(name): Path<String>) -> Result<Response, ApiError> {
    local_only(&c)?;
    blocking(move || {
        let removed = st.remote.update(|cfg| Ok(cfg.remove_user(&name))).map_err(ApiError::internal)?;
        if removed {
            Ok(StatusCode::NO_CONTENT.into_response())
        } else {
            Err(ApiError::not_found(format!("There is no user named '{name}'.")))
        }
    })
    .await
}
