//! Remote sign-in: passkeys, password with one-time codes, enrolment links, your own credentials and devices, and the
//! fresh check for sensitive actions. The design and threat model are in [`crate::remote::auth`].
//!
//! Rules every handler here follows:
//! * the sign-in and enrolment routes need no session (the guard lets them through), so each one counts failures per
//!   source address on the same limiter as token sign-in, and answers the same words for "no such name" and "wrong
//!   password" so names cannot be found out;
//! * a local caller (the computer itself) is told to use the sign-in link: these are for remote users;
//! * sign-ins, failures, enrolments, resets and revocations go to the audit log with a short note and never a secret;
//! * a secret appears only in the answer that creates it.
use std::net::{IpAddr, Ipv4Addr};

use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use kora_core::api::wire;
use kora_core::problem::{codes, Action, Problem};
use kora_core::store::audit::{self, actor, result, New};
use kora_core::store::Store;
use serde::Serialize;

use crate::remote::auth::passkey::{self, Rp};
use crate::remote::auth::{db, password, recovery, state, totp};
use crate::remote::{self, Caller, Identity, Provider, Role};
use crate::tls::serve::Transport;
use crate::{blocking, security, ApiError, AppState};

fn remote_only(c: &Caller) -> Result<(), ApiError> {
    if c.remote {
        Ok(())
    } else {
        Err(ApiError::bad_request("These sign-in methods are for people who open the panel from another computer. On this computer use the link from `kora panel`."))
    }
}

fn ip_of(c: &Caller) -> IpAddr {
    c.peer.map_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED), |p| p.ip())
}

fn store_of(st: &AppState) -> Result<Store, ApiError> {
    st.remote.auth_store().map_err(ApiError::internal)
}

/// The name the person typed to open the panel (with its port), as the guard accepted it.
fn host_of(h: &HeaderMap) -> String {
    h.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("").to_string()
}

fn rp_of(h: &HeaderMap, t: &Transport) -> Result<Rp, String> {
    passkey::rp_for(&host_of(h), t.secure).map_err(|e| passkey::rp_message(&e))
}

fn bad_login() -> ApiError {
    ApiError::unauthorized("That name, password or code is not right (or the account is locked for a few minutes). Try again, or use a passkey.")
}

fn note(st: &AppState, op: &str, who: Option<&str>, provider: &str, role: Option<Role>, res: &str, detail: &str) {
    if let Some(home) = &st.audit_home {
        audit::record_quiet(
            home,
            &New {
                op: op.into(),
                actor_kind: actor::REMOTE.into(),
                actor_name: who.map(|w| w.chars().take(40).collect()),
                provider: Some(provider.into()),
                role: role.map(|r| r.as_str().into()),
                result: res.into(),
                detail: Some(detail.into()),
                ..Default::default()
            },
        );
    }
}

fn device_of(h: &HeaderMap) -> String {
    security::describe_device(h.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or(""))
}

fn throttled(st: &AppState, c: &Caller) -> Result<(), ApiError> {
    let ip = ip_of(c);
    if st.remote.is_self_ip(ip) {
        return Err(ApiError::forbidden(security::SELF_ADDRESS_REFUSED));
    }
    if st.remote.limiter.locked(ip) {
        return Err(ApiError::rate_limited("Too many failed attempts from this address. Try again in a minute."));
    }
    Ok(())
}

/// A signed-in response: the cookie (with `Secure` over TLS), and the session the browser had is ended.
fn signed_in<T: Serialize>(st: &AppState, c: &Caller, t: &Transport, h: &HeaderMap, session: security::NewSession, body: &T) -> Response {
    if let Some(old) = security::current_session(st, h, c.remote) {
        st.sessions.remove(&old);
    }
    let cookie = format!("{}={}; {}", security::cookie_name(st.addr.port()), session.id, crate::tls::rules::cookie_attrs(t.secure));
    let mut r = Json(body).into_response();
    r.headers_mut().insert(header::SET_COOKIE, HeaderValue::from_str(&cookie).expect("ascii cookie"));
    r
}

fn start_session(st: &AppState, c: &Caller, h: &HeaderMap, provider: Provider, user: &str) -> Result<(security::NewSession, Identity), ApiError> {
    let ident = st.remote.resolve(provider, user).ok_or_else(bad_login)?;
    let s = st.sessions.create_full(provider, &ident.name, true, ident.role, &device_of(h), &ip_of(c).to_string());
    Ok((s, ident))
}

fn parse<T: serde::de::DeserializeOwned>(v: serde_json::Value) -> Result<T, ApiError> {
    serde_json::from_value(v).map_err(|_| ApiError::bad_request("The browser's answer was not what a passkey gives. Try again."))
}

fn options_json<T: Serialize>(o: &T) -> Result<serde_json::Value, ApiError> {
    serde_json::to_value(o).map_err(|e| ApiError::internal(e.to_string()))
}

fn label_of(l: &str) -> String {
    let l: String = l.trim().chars().filter(|c| !c.is_control()).take(40).collect();
    if l.is_empty() {
        "Passkey".into()
    } else {
        l
    }
}

/// `GET /auth/options`.
pub async fn auth_options(State(st): State<AppState>, Extension(c): Extension<Caller>, Extension(t): Extension<Transport>, headers: HeaderMap) -> Json<wire::AuthOptions> {
    let cfg = st.remote.config();
    let (passkey, problem) = if !c.remote {
        (false, Some("On this computer use the link from `kora panel`.".to_string()))
    } else {
        match rp_of(&headers, &t) {
            Ok(_) => (true, None),
            Err(m) => (false, Some(m)),
        }
    };
    Json(wire::AuthOptions {
        passkey,
        passkey_problem: problem,
        code: c.remote,
        token: c.remote && !cfg.require_strong_auth,
        tailscale: c.remote && cfg.tailscale,
        strong_required: cfg.require_strong_auth,
    })
}

/// `POST /auth/login`.
pub async fn auth_login(
    State(st): State<AppState>,
    Extension(c): Extension<Caller>,
    Extension(t): Extension<Transport>,
    headers: HeaderMap,
    Json(req): Json<wire::LoginRequest>,
) -> Result<Response, ApiError> {
    remote_only(&c)?;
    throttled(&st, &c)?;
    let ip = ip_of(&c);
    match req {
        wire::LoginRequest::PasskeyBegin { user } => {
            let rp = rp_of(&headers, &t).map_err(ApiError::bad_request)?;
            let known = crate::remote::config::valid_user_name(user.trim()).is_ok() && st.remote.config().user(user.trim()).is_some();
            let begun = if known { st.auth.ceremonies.begin_login(&store_of(&st)?, &rp, user.trim()) } else { Err(passkey::NO_PASSKEY.to_string()) };
            match begun {
                Ok((handle, options)) => Ok(Json(wire::LoginResponse::Challenge { handle, options: options_json(&options)? }).into_response()),
                Err(m) => {
                    // A name nobody has and a user without a passkey here look the same, and both count against the address.
                    st.remote.limiter.failure(ip);
                    Err(ApiError::unauthorized(m))
                }
            }
        }
        wire::LoginRequest::PasskeyFinish { handle, credential } => {
            let cred = parse(credential)?;
            let st2 = st.clone();
            let outcome = blocking(move || Ok(st2.auth.ceremonies.finish_login(&store_of(&st2)?, &handle, &cred))).await?;
            match outcome {
                Ok(ok) => {
                    let (session, ident) = start_session(&st, &c, &headers, Provider::Passkey, &ok.user)?;
                    st.remote.limiter.success(ip);
                    note(&st, "auth_login", Some(&ident.name), "passkey", Some(ident.role), result::OK, "passkey");
                    Ok(signed_in(&st, &c, &t, &headers, session.clone(), &wire::LoginResponse::SignedIn { identity: (&ident).into(), secret: session.secret.clone(), notice: None }))
                }
                Err(m) => {
                    st.remote.limiter.failure(ip);
                    note(&st, "auth_login", None, "passkey", None, result::FAILED, "passkey refused");
                    Err(ApiError::unauthorized(m))
                }
            }
        }
        wire::LoginRequest::Code { user, password: pw, code } => {
            let name = user.trim().to_string();
            let st2 = st.clone();
            let outcome = blocking(move || code_login(&st2, &name, &pw, &code)).await?;
            match outcome {
                Ok((user, used_recovery)) => {
                    let (session, ident) = start_session(&st, &c, &headers, Provider::Code, &user)?;
                    st.remote.limiter.success(ip);
                    let notice = used_recovery.map(|left| format!("You signed in with a recovery code ({left} left). Make new ones in your account settings when you can."));
                    note(&st, "auth_login", Some(&ident.name), "code", Some(ident.role), result::OK, if notice.is_some() { "recovery code" } else { "one-time code" });
                    Ok(signed_in(&st, &c, &t, &headers, session.clone(), &wire::LoginResponse::SignedIn { identity: (&ident).into(), secret: session.secret.clone(), notice }))
                }
                Err(()) => {
                    st.remote.limiter.failure(ip);
                    note(&st, "auth_login", None, "code", None, result::FAILED, "password or code refused");
                    Err(bad_login())
                }
            }
        }
    }
}

/// Check a password and a one-time (or recovery) code. `Ok((user, Some(left)))` when a recovery code was used. Every
/// failure is the same `Err(())`; the account counts it.
fn code_login(st: &AppState, name: &str, pw: &str, code: &str) -> Result<Result<(String, Option<usize>), ()>, ApiError> {
    let store = store_of(st)?;
    let cfg_user = if crate::remote::config::valid_user_name(name).is_ok() { st.remote.config().user(name).map(|u| u.name.clone()) } else { None };
    let secrets = match &cfg_user {
        Some(u) => db::code_secrets(&store, u).map_err(ApiError::internal)?,
        None => None,
    };
    let (Some(user), Some(sec)) = (cfg_user, secrets) else {
        // Spend the time of a real check, so the answer does not say which names exist.
        password::verify_dummy(pw);
        return Ok(Err(()));
    };
    let now = db::now();
    if sec.locked_until > now {
        password::verify_dummy(pw);
        return Ok(Err(()));
    }
    let fail = |store: &Store| {
        let _ = db::note_failure(store, &user);
        Ok(Err(()))
    };
    if !password::verify(&sec.password_hash, pw) {
        return fail(&store);
    }
    // A recovery code looks like `abcde-fghij`; a one-time code is six digits.
    let digits: String = code.chars().filter(|c| !c.is_whitespace() && *c != '-').collect();
    if digits.len() == totp::DIGITS as usize && digits.bytes().all(|b| b.is_ascii_digit()) {
        match totp::accept(&sec.totp_secret, sec.last_step, &digits, now as u64) {
            Some(step) if db::take_totp_step(&store, &user, step).map_err(ApiError::internal)? => {
                let _ = db::clear_failures(&store, &user);
                Ok(Ok((user, None)))
            }
            _ => fail(&store),
        }
    } else if db::use_recovery_code(&store, &user, code).map_err(ApiError::internal)? {
        let _ = db::clear_failures(&store, &user);
        let left = db::state(&store, &user).map(|s| s.recovery_left).unwrap_or(0);
        Ok(Ok((user, Some(left))))
    } else {
        fail(&store)
    }
}

// ---- enrolment ------------------------------------------------------------------------------------------------------

fn link_key(link: &str) -> String {
    format!("link:{}", crate::remote::config::hash_token("kora-enrol-pending", link))
}

fn totp_setup(user: &str, secret: &[u8; 20]) -> Result<(String, String, String), ApiError> {
    let uri = totp::otpauth_uri("Kora Panel", user, secret);
    let svg = totp::qr_svg(&uri).map_err(ApiError::internal)?;
    Ok((totp::base32(secret), uri, svg))
}

/// `POST /auth/enrol`.
pub async fn auth_enrol(
    State(st): State<AppState>,
    Extension(c): Extension<Caller>,
    Extension(t): Extension<Transport>,
    headers: HeaderMap,
    Json(req): Json<wire::EnrolRequest>,
) -> Result<Response, ApiError> {
    remote_only(&c)?;
    throttled(&st, &c)?;
    let ip = ip_of(&c);
    // The store is opened where it is used and never kept across an `.await`.
    let link_of = |l: &str| -> Result<String, ApiError> {
        let who = db::peek_link(&store_of(&st)?, l.trim()).map_err(ApiError::internal)?;
        let user = who.and_then(|w| st.remote.config().user(&w).map(|u| u.name.clone()));
        user.ok_or_else(|| {
            st.remote.limiter.failure(ip);
            ApiError::unauthorized("This link has expired or was already used. Ask the person who runs Kora for a new one.")
        })
    };
    match req {
        wire::EnrolRequest::Check { link } => {
            let user = link_of(&link)?;
            let role = st.remote.config().user(&user).map(|u| u.role).unwrap_or(Role::Viewer);
            let (passkey, problem) = match rp_of(&headers, &t) {
                Ok(_) => (true, None),
                Err(m) => (false, Some(m)),
            };
            Ok(Json(wire::EnrolResponse::Who { user, role: role.into(), passkey, passkey_problem: problem }).into_response())
        }
        wire::EnrolRequest::PasskeyBegin { link } => {
            let user = link_of(&link)?;
            let rp = rp_of(&headers, &t).map_err(ApiError::bad_request)?;
            let (handle, options) = st.auth.ceremonies.begin_registration(&store_of(&st)?, &rp, &user, &user).map_err(ApiError::bad_request)?;
            Ok(Json(wire::EnrolResponse::Challenge { handle, options: options_json(&options)? }).into_response())
        }
        wire::EnrolRequest::PasskeyFinish { link, handle, label, credential } => {
            let user = link_of(&link)?;
            let cred = parse(credential)?;
            let (pk, rp) = st.auth.ceremonies.finish_registration(&handle, &user, &cred).map_err(ApiError::bad_request)?;
            // The link is used up here, once: two people racing with one link cannot both finish.
            let st2 = st.clone();
            let secret = link.trim().to_string();
            let (user2, label) = (user.clone(), label_of(&label));
            let used = blocking(move || {
                let store2 = store_of(&st2)?;
                if db::consume_link(&store2, &secret).map_err(ApiError::internal)?.is_none() {
                    return Ok(false);
                }
                db::add_passkey(&store2, &user2, &label, &rp.id, &pk).map_err(ApiError::bad_request)?;
                Ok(true)
            })
            .await?;
            if !used {
                return Err(ApiError::unauthorized("This link was already used."));
            }
            let (session, ident) = start_session(&st, &c, &headers, Provider::Passkey, &user)?;
            note(&st, "auth_enrol", Some(&ident.name), "passkey", Some(ident.role), result::OK, "passkey added");
            Ok(signed_in(&st, &c, &t, &headers, session.clone(), &wire::EnrolResponse::Done { identity: (&ident).into(), secret: session.secret.clone(), recovery_codes: vec![] }))
        }
        wire::EnrolRequest::CodeBegin { link } => {
            let user = link_of(&link)?;
            let secret = totp::new_secret().map_err(ApiError::internal)?;
            st.auth.put_totp(&link_key(link.trim()), secret);
            let (secret, otpauth, qr_svg) = totp_setup(&user, &secret)?;
            Ok(Json(wire::EnrolResponse::CodeSetup { secret, otpauth, qr_svg }).into_response())
        }
        wire::EnrolRequest::CodeFinish { link, password: pw, code } => {
            let user = link_of(&link)?;
            let key = link_key(link.trim());
            let Some(secret) = st.auth.get_totp(&key) else {
                return Err(ApiError::bad_request("Start the one-time-code set-up again: the secret expired."));
            };
            password::strength(&pw, &user).map_err(ApiError::bad_request)?;
            if totp::accept(&secret, 0, &code, db::now() as u64).is_none() {
                st.remote.limiter.failure(ip);
                return Err(ApiError::bad_request("That code is not right. Check the time on your phone, wait for the next code, and try again."));
            }
            let st2 = st.clone();
            let secret_link = link.trim().to_string();
            let user2 = user.clone();
            let step = totp::step_of(db::now() as u64);
            let codes = blocking(move || {
                let store2 = store_of(&st2)?;
                let hash = password::hash(&pw).map_err(ApiError::internal)?;
                let codes = recovery::generate().map_err(ApiError::internal)?;
                let stored: Vec<String> = codes.iter().map(|c| recovery::store_form(c)).collect::<Result<_, _>>().map_err(ApiError::internal)?;
                if db::consume_link(&store2, &secret_link).map_err(ApiError::internal)?.is_none() {
                    return Err(ApiError::unauthorized("This link was already used."));
                }
                db::set_password_and_totp(&store2, &user2, &hash, &secret, &stored).map_err(ApiError::internal)?;
                // The code that proved the set-up is spent: it cannot be replayed to sign in.
                let _ = db::take_totp_step(&store2, &user2, step.max(totp::accept(&secret, 0, &code, db::now() as u64).unwrap_or(step)));
                Ok(codes)
            })
            .await?;
            st.auth.drop_totp(&key);
            let (session, ident) = start_session(&st, &c, &headers, Provider::Code, &user)?;
            note(&st, "auth_enrol", Some(&ident.name), "code", Some(ident.role), result::OK, "one-time codes set up");
            Ok(signed_in(&st, &c, &t, &headers, session.clone(), &wire::EnrolResponse::Done { identity: (&ident).into(), secret: session.secret.clone(), recovery_codes: codes }))
        }
    }
}

// ---- your own account -----------------------------------------------------------------------------------------------

fn me(c: &Caller) -> Result<Identity, ApiError> {
    remote_only(c)?;
    let id = c.identity.clone().ok_or_else(|| ApiError::unauthorized("missing or invalid session"))?;
    match id.provider {
        Provider::Token | Provider::Passkey | Provider::Code => Ok(id),
        _ => Err(ApiError::bad_request("Your sign-in does not come from a password, passkey or token, so there is nothing to set up here.")),
    }
}

fn passkey_views(s: &db::UserState) -> Vec<wire::PasskeyView> {
    s.passkeys.iter().map(|p| wire::PasskeyView { id: p.id, label: p.label.clone(), rp_id: p.rp_id.clone(), created: p.created, last_used: p.last_used }).collect()
}

/// `GET /auth/account`.
pub async fn auth_account(State(st): State<AppState>, Extension(c): Extension<Caller>, Extension(t): Extension<Transport>, headers: HeaderMap) -> Result<Json<wire::AccountView>, ApiError> {
    let id = me(&c)?;
    let current = security::current_session(&st, &headers, true);
    blocking(move || {
        let store = store_of(&st)?;
        let s = db::state(&store, &id.name).map_err(ApiError::internal)?;
        let legacy = st.remote.config().user(&id.name).is_some_and(|u| u.has_token());
        let devices = st
            .sessions
            .list_for(&id.name, current.as_deref())
            .into_iter()
            .map(|d| wire::DeviceView { handle: d.handle, device: d.device, address: d.address, signed_in: d.signed_in, last_seen: d.last_seen, current: d.current })
            .collect();
        Ok(Json(wire::AccountView {
            user: id.name.clone(),
            provider: id.provider.into(),
            role: id.role.into(),
            passkeys: passkey_views(&s),
            code: s.totp && s.password,
            recovery_left: s.recovery_left as u32,
            legacy_token: legacy,
            strong: s.strong(),
            passkey_problem: rp_of(&headers, &t).err(),
            upgrade_suggested: id.provider == Provider::Token && !s.strong(),
            devices,
        }))
    })
    .await
}

fn reauth_needed() -> ApiError {
    ApiError::problem(StatusCode::FORBIDDEN, Problem::new(codes::REMOTE_REAUTH, "Confirm it is you first: use your passkey or a one-time code, then try again.").with_action(Action::Retry))
}

/// `POST /auth/account`.
pub async fn auth_account_change(
    State(st): State<AppState>,
    Extension(c): Extension<Caller>,
    Extension(t): Extension<Transport>,
    headers: HeaderMap,
    Json(req): Json<wire::AccountChange>,
) -> Result<Json<wire::AccountChanged>, ApiError> {
    let id = me(&c)?;
    let current = security::current_session(&st, &headers, true).ok_or_else(|| ApiError::unauthorized("missing or invalid session"))?;
    let user = id.name.clone();
    let before = db::state(&store_of(&st)?, &user).map_err(ApiError::internal)?;
    // Changing HOW you sign in needs a recent check once you have something strong to check with. Adding the first
    // credential (from a token session, say) cannot ask for one. Signing a device out never needs it.
    let protective = matches!(req, wire::AccountChange::DeviceRevoke { .. } | wire::AccountChange::DevicesRevokeOthers);
    if before.strong() && !protective && !st.sessions.is_fresh(&current) {
        return Err(reauth_needed());
    }
    let ip = ip_of(&c);
    match req {
        wire::AccountChange::PasskeyBegin => {
            let rp = rp_of(&headers, &t).map_err(ApiError::bad_request)?;
            let (handle, options) = st.auth.ceremonies.begin_registration(&store_of(&st)?, &rp, &user, &user).map_err(ApiError::bad_request)?;
            Ok(Json(wire::AccountChanged::Challenge { handle, options: options_json(&options)? }))
        }
        wire::AccountChange::PasskeyFinish { handle, label, credential } => {
            let cred = parse(credential)?;
            let (pk, rp) = st.auth.ceremonies.finish_registration(&handle, &user, &cred).map_err(ApiError::bad_request)?;
            db::add_passkey(&store_of(&st)?, &user, &label_of(&label), &rp.id, &pk).map_err(ApiError::bad_request)?;
            note(&st, "auth_account", Some(&user), "passkey", Some(id.role), result::OK, "passkey added");
            Ok(Json(wire::AccountChanged::Done))
        }
        wire::AccountChange::PasskeyRemove { id: pid } => {
            let others = before.passkeys.iter().filter(|p| p.id != pid).count();
            if others == 0 && !(before.totp && before.password) && before.passkeys.iter().any(|p| p.id == pid) {
                return Err(ApiError::conflict("This is your only way to sign in. Add another passkey or set up one-time codes first, then remove this one."));
            }
            if !db::remove_passkey(&store_of(&st)?, &user, pid).map_err(ApiError::internal)? {
                return Err(ApiError::not_found("There is no such passkey."));
            }
            // A removed passkey (a lost laptop) ends the other signed-in devices at once.
            st.sessions.revoke_user(&user, Some(&current));
            note(&st, "auth_account", Some(&user), "passkey", Some(id.role), result::OK, "passkey removed");
            Ok(Json(wire::AccountChanged::Done))
        }
        wire::AccountChange::CodeBegin => {
            let secret = totp::new_secret().map_err(ApiError::internal)?;
            st.auth.put_totp(&format!("account:{}", db::fold(&user)), secret);
            let (secret, otpauth, qr_svg) = totp_setup(&user, &secret)?;
            Ok(Json(wire::AccountChanged::CodeSetup { secret, otpauth, qr_svg }))
        }
        wire::AccountChange::CodeFinish { password: pw, code } => {
            let key = format!("account:{}", db::fold(&user));
            let Some(secret) = st.auth.get_totp(&key) else { return Err(ApiError::bad_request("Start the one-time-code set-up again: the secret expired.")) };
            password::strength(&pw, &user).map_err(ApiError::bad_request)?;
            let Some(step) = totp::accept(&secret, 0, &code, db::now() as u64) else {
                st.remote.limiter.failure(ip);
                return Err(ApiError::bad_request("That code is not right. Check the time on your phone and try again."));
            };
            let (user2, st2) = (user.clone(), st.clone());
            let codes = blocking(move || {
                let store = store_of(&st2)?;
                let hash = password::hash(&pw).map_err(ApiError::internal)?;
                let codes = recovery::generate().map_err(ApiError::internal)?;
                let stored: Vec<String> = codes.iter().map(|c| recovery::store_form(c)).collect::<Result<_, _>>().map_err(ApiError::internal)?;
                db::set_password_and_totp(&store, &user2, &hash, &secret, &stored).map_err(ApiError::internal)?;
                let _ = db::take_totp_step(&store, &user2, step);
                Ok(codes)
            })
            .await?;
            st.auth.drop_totp(&key);
            note(&st, "auth_account", Some(&user), "code", Some(id.role), result::OK, "one-time codes set up");
            Ok(Json(wire::AccountChanged::Recovery { codes }))
        }
        wire::AccountChange::PasswordChange { current: cur, password: pw } => {
            let store = store_of(&st)?;
            let Some(sec) = db::code_secrets(&store, &user).map_err(ApiError::internal)? else {
                return Err(ApiError::bad_request("You have not set up one-time codes, so there is no password to change."));
            };
            if !password::verify(&sec.password_hash, &cur) {
                st.remote.limiter.failure(ip);
                let _ = db::note_failure(&store, &user);
                return Err(ApiError::unauthorized("Your current password is not right."));
            }
            password::strength(&pw, &user).map_err(ApiError::bad_request)?;
            drop(store);
            let hash = blocking(move || password::hash(&pw).map_err(ApiError::internal)).await?;
            db::set_password(&store_of(&st)?, &user, &hash).map_err(ApiError::internal)?;
            st.sessions.revoke_user(&user, Some(&current));
            note(&st, "auth_account", Some(&user), "code", Some(id.role), result::OK, "password changed");
            Ok(Json(wire::AccountChanged::Done))
        }
        wire::AccountChange::RecoveryNew => {
            if !(before.totp && before.password) {
                return Err(ApiError::bad_request("Recovery codes belong to one-time codes, which you have not set up."));
            }
            let codes = recovery::generate().map_err(ApiError::internal)?;
            let stored: Vec<String> = codes.iter().map(|c| recovery::store_form(c)).collect::<Result<_, _>>().map_err(ApiError::internal)?;
            db::replace_recovery(&store_of(&st)?, &user, &stored).map_err(ApiError::internal)?;
            note(&st, "auth_account", Some(&user), "code", Some(id.role), result::OK, "new recovery codes");
            Ok(Json(wire::AccountChanged::Recovery { codes }))
        }
        wire::AccountChange::DeviceRevoke { handle } => {
            if !st.sessions.revoke_handle(&user, &handle) {
                return Err(ApiError::not_found("That device is already signed out."));
            }
            note(&st, "auth_account", Some(&user), id.provider_text(), Some(id.role), result::OK, "device signed out");
            Ok(Json(wire::AccountChanged::Done))
        }
        wire::AccountChange::DevicesRevokeOthers => {
            let n = st.sessions.revoke_user(&user, Some(&current));
            note(&st, "auth_account", Some(&user), id.provider_text(), Some(id.role), result::OK, &format!("{n} other devices signed out"));
            Ok(Json(wire::AccountChanged::Done))
        }
        wire::AccountChange::TokenRevoke => {
            if !before.strong() {
                return Err(ApiError::conflict("Set up a passkey or one-time codes first, so you do not lock yourself out."));
            }
            let user2 = user.clone();
            st.remote
                .update(move |cfg| {
                    if let Some(u) = cfg.users.iter_mut().find(|u| u.name.eq_ignore_ascii_case(&user2)) {
                        u.salt.clear();
                        u.hash.clear();
                    }
                    Ok(())
                })
                .map_err(ApiError::internal)?;
            note(&st, "auth_account", Some(&user), id.provider_text(), Some(id.role), result::OK, "old token link ended");
            Ok(Json(wire::AccountChanged::Done))
        }
    }
}

/// `POST /auth/reauth`.
pub async fn auth_reauth(
    State(st): State<AppState>,
    Extension(c): Extension<Caller>,
    Extension(t): Extension<Transport>,
    headers: HeaderMap,
    Json(req): Json<wire::ReauthRequest>,
) -> Result<Json<wire::ReauthResponse>, ApiError> {
    remote_only(&c)?;
    let id = c.identity.clone().ok_or_else(|| ApiError::unauthorized("missing or invalid session"))?;
    let current = security::current_session(&st, &headers, true).ok_or_else(|| ApiError::unauthorized("missing or invalid session"))?;
    throttled(&st, &c)?;
    let ip = ip_of(&c);
    let fresh = |st: &AppState| {
        st.sessions.mark_fresh(&current);
        Json(wire::ReauthResponse::Fresh { valid_for: security::FRESH_FOR.as_secs() })
    };
    match req {
        wire::ReauthRequest::PasskeyBegin => {
            let rp = rp_of(&headers, &t).map_err(ApiError::bad_request)?;
            let (handle, options) = st.auth.ceremonies.begin_login(&store_of(&st)?, &rp, &id.name).map_err(ApiError::bad_request)?;
            Ok(Json(wire::ReauthResponse::Challenge { handle, options: options_json(&options)? }))
        }
        wire::ReauthRequest::PasskeyFinish { handle, credential } => {
            let cred = parse(credential)?;
            let st2 = st.clone();
            let outcome = blocking(move || Ok(st2.auth.ceremonies.finish_login(&store_of(&st2)?, &handle, &cred))).await?;
            match outcome {
                // It must be THIS user's passkey: a different user's does not make this session fresh.
                Ok(ok) if ok.user.eq_ignore_ascii_case(&id.name) => {
                    note(&st, "auth_reauth", Some(&id.name), "passkey", Some(id.role), result::OK, "passkey");
                    Ok(fresh(&st))
                }
                _ => {
                    st.remote.limiter.failure(ip);
                    note(&st, "auth_reauth", Some(&id.name), "passkey", Some(id.role), result::FAILED, "refused");
                    Err(ApiError::unauthorized("The passkey was not accepted."))
                }
            }
        }
        wire::ReauthRequest::Code { password: pw, code } => {
            let (st2, name) = (st.clone(), id.name.clone());
            let outcome = blocking(move || code_login(&st2, &name, &pw, &code)).await?;
            match outcome {
                Ok((user, _)) if user.eq_ignore_ascii_case(&id.name) => {
                    note(&st, "auth_reauth", Some(&id.name), "code", Some(id.role), result::OK, "one-time code");
                    Ok(fresh(&st))
                }
                _ => {
                    st.remote.limiter.failure(ip);
                    note(&st, "auth_reauth", Some(&id.name), "code", Some(id.role), result::FAILED, "refused");
                    Err(bad_login())
                }
            }
        }
    }
}

impl Identity {
    fn provider_text(&self) -> &'static str {
        match self.provider {
            Provider::Passkey => "passkey",
            Provider::Code => "code",
            Provider::Token => "token",
            _ => "other",
        }
    }
}
