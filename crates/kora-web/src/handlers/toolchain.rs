//! Tools and the System check: which git / gh is in use, installing Kora's own copy, the guided
//! System check page's data, and "start the database for me".
//!
//! Each `pub async fn` is mounted by `crate::routes` for the [`Op`] of the same name. What touches the
//! computer goes through [`ToolchainEnv`]; tests replace it with [`set_env`].
use std::sync::{Arc, RwLock};

use axum::extract::{Path, State};
use axum::Json;
use kora_core::api::toolchain::{check_json, system_check, SystemToolchain, ToolchainEnv};
use kora_core::api::Op;
use kora_core::problem::{codes, Problem, Tool};
use serde_json::{json, Value};

use crate::{blocking, ApiError, AppState};

static ENV: RwLock<Option<Arc<dyn ToolchainEnv>>> = RwLock::new(None);

/// Replace the toolchain seam (tests). `None` goes back to the real computer.
pub fn set_env(e: Option<Arc<dyn ToolchainEnv>>) {
    *ENV.write().unwrap_or_else(|p| p.into_inner()) = e;
}

/// The toolchain seam in use.
pub fn env() -> Arc<dyn ToolchainEnv> {
    ENV.read().unwrap_or_else(|p| p.into_inner()).clone().unwrap_or_else(|| Arc::new(SystemToolchain))
}

fn cmd(parts: &[&str]) -> Vec<String> {
    std::iter::once("kora").chain(parts.iter().copied()).map(str::to_string).collect()
}

/// `GET /tools` → `{tools: [{tool, name, label, installed, version, source, path, broken, can_install}]}`.
pub async fn tools_status() -> Result<Json<Value>, ApiError> {
    let e = env();
    let tools = blocking(move || Ok(e.tools())).await?;
    Ok(Json(json!({"tools": tools})))
}

/// `GET /system/check` → `{items: [{id, label, state, summary, problem}], ok}`. `state` is `ok`, `problem`
/// (the `problem` has the fix buttons) or `info`.
pub async fn system_check_get(State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    let e = env();
    let g = super::github::env();
    let items = blocking(move || {
        let mut items = system_check(e.as_ref(), &g.auth());
        // HTTPS: only a listener other computers can reach (not loopback, not the tailnet) makes plain HTTP a problem.
        let exposed = crate::tls::rules::plain_is_risky(st.addr.ip());
        items.push(crate::tls::check_item(st.tls.info().as_ref(), st.tls.last_error().as_deref(), exposed, crate::tls::manager::now_unix()));
        Ok(items)
    })
    .await?;
    Ok(Json(check_json(&items)))
}

fn parse_tool(s: &str) -> Result<Tool, ApiError> {
    match s {
        "git" => Ok(Tool::Git),
        "gh" => Ok(Tool::Gh),
        _ => Err(ApiError::not_found("Kora can install `git` and `gh` only.")),
    }
}

/// `POST /tools/{tool}/install` → `202 {job}`. Kora downloads its own copy (checked against a pinned
/// checksum), with no administrator rights. Git on Linux is the system package: that needs root, through the
/// helper when it is installed, else the `needs_elevation` answer with the command. The job result is
/// `{tool, version, path, source}`; failures carry a Problem (Retry, a manual link).
pub async fn tool_install(State(st): State<AppState>, Path(tool): Path<String>) -> Result<axum::response::Response, ApiError> {
    let tool = parse_tool(&tool)?;
    let e = env();
    let system_git = tool == Tool::Git && !cfg!(windows);
    let elevated = if system_git {
        let e2 = e.clone();
        blocking(move || Ok(e2.admin().elevated)).await?
    } else {
        true
    };
    if system_git && !elevated {
        let msg = "Installing Git needs administrator rights. Run this command as administrator:";
        let command = cmd(&["tools", "install", "git"]);
        let what = "install Git";
        crate::elevated::precheck_verb(&kora_core::privsvc::problems::Context { what: what.into(), command: command.clone(), can_skip_admin: false }, msg)?;
        return st.spawn_job(Op::ToolInstall, None, move |h| {
            let out = crate::elevated::privileged_as(what, msg, command, vec![], &mut |m| h.event(json!({"type": "status", "message": m}))).map_err(|e| e.message)?;
            Ok(json!({"tool": "git", "output": out["output"]}))
        });
    }
    st.spawn_job(Op::ToolInstall, None, move |h| {
        let s = e.install(tool, &mut |m, f| h.event(json!({"type": "progress", "message": m, "fraction": f})))?;
        Ok(json!({"tool": s.name, "version": s.version, "path": s.path, "source": s.source}))
    })
}

/// `POST /system/database/start` → `202 {job}`: start the installed (stopped) database server. It needs
/// administrator rights: through the administrator service when it is there for this account (no prompt),
/// else `needs_elevation` with the choices (install the service, the command).
pub async fn db_start(State(st): State<AppState>) -> Result<axum::response::Response, ApiError> {
    let e = env();
    let (db, admin) = {
        let e = e.clone();
        blocking(move || Ok((e.database(), e.admin()))).await?
    };
    if !db.installed {
        return Err(ApiError::problem(
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            Problem::new(codes::INSTALL_DB_MISSING, "No database server is installed on this computer. Kora can install one when you set up a server."),
        ));
    }
    if db.running {
        return Err(ApiError::conflict("The database server is already running."));
    }
    if !admin.elevated {
        // The administrator service starts it (no prompt); Windows, until its service lands, after
        // one permission prompt (a remote session only when the person asked). Without either:
        // the choices (install the service, the command).
        let name = db.name.clone().unwrap_or_default();
        let msg = format!("Starting the database server {name} needs administrator rights. Run this command as administrator:");
        let command = cmd(&["check", "--start-database"]);
        let what = "start the database server";
        crate::elevated::precheck_verb(&kora_core::privsvc::problems::Context { what: what.into(), command: command.clone(), can_skip_admin: false }, &msg)?;
        return st.spawn_job(Op::DbStart, None, move |h| {
            let out =
                crate::elevated::privileged_as(what, &msg, command, vec![], &mut |m| h.event(json!({"type": "status", "message": m}))).map_err(|e| kora_core::api::JobFailure::new(e.to_problem()))?;
            Ok(json!({"output": out["output"]}))
        });
    }
    st.spawn_job(Op::DbStart, None, move |h| {
        let m = e.start_database(&mut |m| h.event(json!({"type": "status", "message": m})))?;
        Ok(json!({"message": m}))
    })
}
