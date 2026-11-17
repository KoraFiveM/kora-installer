//! "Can players reach my server?": the checklist, what the person allowed Kora to look up outside,
//! automatic port forwarding, and opening the firewall.
//!
//! Each `pub async fn` is mounted by `crate::routes` for the [`kora_core::api::Op`] of the same name.
//! The checklist reads the machine, talks to the local router, and only if the person allowed it asks
//! the outside services named in `kora_core::reach::probe`; the answer lists what was contacted.
#![allow(clippy::unused_async)]

use axum::extract::{Path, State};
use axum::Json;
use kora_core::api::wire::{ForwardingRequest, ReachConsentPatch};
use kora_core::reach::settings::Consent;
use serde_json::{json, Value};

use crate::{blocking, ApiError, AppState};

pub async fn reach_check(State(st): State<AppState>, Path(s): Path<String>, Json(body): Json<ReachConsentPatch>) -> Result<Json<Value>, ApiError> {
    blocking(move || {
        let e = st.entry(&s)?;
        let mut b = st.backend();
        let saved = b.reach_consent();
        // A one-off "yes" only ever widens the check for this call; it never saves anything.
        let once = (body.cfx_list.is_some() || body.ip_echo.is_some()).then(|| Consent { cfx_list: body.cfx_list.unwrap_or(saved.cfx_list), ip_echo: body.ip_echo.unwrap_or(saved.ip_echo) });
        let mut report = b.reach_check(&e, once).map_err(ApiError::core)?;
        report.consent = saved;
        let v = serde_json::to_value(&report).map_err(|x| ApiError::internal(x.to_string()))?;
        Ok(Json(v))
    })
    .await
}

pub async fn reach_settings_get(State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    blocking(move || Ok(Json(json!(st.backend().reach_consent())))).await
}

pub async fn reach_settings_set(State(st): State<AppState>, Json(body): Json<ReachConsentPatch>) -> Result<Json<Value>, ApiError> {
    blocking(move || {
        let mut b = st.backend();
        let old = b.reach_consent();
        let new = Consent { cfx_list: body.cfx_list.unwrap_or(old.cfx_list), ip_echo: body.ip_echo.unwrap_or(old.ip_echo) };
        b.reach_consent_set(new).map_err(ApiError::core)?;
        Ok(Json(json!(new)))
    })
    .await
}

pub async fn forwarding_set(State(st): State<AppState>, Path(s): Path<String>, Json(body): Json<ForwardingRequest>) -> Result<Json<Value>, ApiError> {
    blocking(move || {
        let e = st.entry(&s)?;
        let view = st.backend().forwarding_set(&e, body.enabled).map_err(ApiError::core)?;
        Ok(Json(json!(view)))
    })
    .await
}

pub async fn reach_firewall_open(State(st): State<AppState>, Path(s): Path<String>) -> Result<Json<Value>, ApiError> {
    blocking(move || {
        let e = st.entry(&s)?;
        // Not elevated: the administrator service does it (the game port only, as the person's own server, recorded in
        // its ownership ledger). Without the service the answer carries the choices and `kora reach <name> --open-firewall`.
        if !kora_core::elevate::is_elevated() && crate::elevated::service_ready() {
            crate::elevated::reach_open_firewall(&e.name, &e.dir, &mut |_| {})?;
            return Ok(Json(json!({"notes": [format!("Opened port {} (TCP and UDP) in the firewall.", e.ports.game)]})));
        }
        let notes = match st.backend().firewall_open(&e) {
            Ok(n) => n,
            // The computer's own answer is "run this as administrator": the panel adds the way through the service
            // (install it, or ask an administrator to turn it on for this account) and names the command that exists.
            Err(m) if m.code == kora_core::problem::codes::NEEDS_ELEVATION => {
                let shown = vec!["kora".to_string(), "reach".into(), e.name.clone(), "--open-firewall".into()];
                let ctx = kora_core::privsvc::problems::Context { what: "open the firewall for this server's game port".into(), command: shown, can_skip_admin: false };
                crate::elevated::precheck(&ctx, "Changing the firewall needs administrator rights. Run this command as administrator, then check again:")?;
                return Err(ApiError::core(m));
            }
            Err(m) => return Err(ApiError::core(m)),
        };
        Ok(Json(json!({"notes": notes})))
    })
    .await
}
