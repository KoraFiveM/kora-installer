//! The hub: other Kora panels ("nodes") and their servers, plus one combined fleet list.
//!
//! Node management (`nodes_list`, `node_add`, `node_remove`) and every `node_*` op are admin-only and
//! local-only: a hub is run by the person at this panel, and a node's token is never shown again. A node
//! is reached with a bearer token; its answers are read through [`RemoteBackend`], which only offers what
//! the node's `/meta` says it can do.
#![allow(clippy::unused_async)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use kora_core::api::Op;
use kora_core::hub::{
    check_url, Meta, NodeConfig, NodeStore, RemoteBackend, RemoteError, Transport, UreqTransport,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{blocking, ApiError, AppState};

// ---- seams (tests and the parity run install fakes; nothing here touches the real network then) ------

static STORE_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);
static TRANSPORT: Mutex<Option<Arc<dyn Transport>>> = Mutex::new(None);

/// Use this nodes file instead of `<kora home>/hub.json`.
pub fn set_store_path(p: Option<PathBuf>) {
    *STORE_PATH.lock().unwrap() = p;
}
/// Use this HTTP transport instead of the real one.
pub fn set_transport(t: Option<Arc<dyn Transport>>) {
    *TRANSPORT.lock().unwrap() = t;
}

fn store() -> NodeStore {
    match STORE_PATH.lock().unwrap().clone() {
        Some(p) => NodeStore::at(p),
        None => NodeStore::default_store(),
    }
}
fn transport() -> Arc<dyn Transport> {
    TRANSPORT
        .lock()
        .unwrap()
        .clone()
        .unwrap_or_else(|| Arc::new(UreqTransport))
}
fn remote(n: &NodeConfig) -> RemoteBackend {
    RemoteBackend::new(&n.url, &n.token, transport())
}

// ---- errors ------------------------------------------------------------------------------------------

/// Map a node's failure to our error shape. A node's 401/403 must never become our 401 (the UI would sign
/// the user out of *this* panel), so those are `502 node_unauthorized`.
fn remote_err(e: RemoteError) -> ApiError {
    match e {
        RemoteError::Unreachable(m) => {
            ApiError::new(StatusCode::BAD_GATEWAY, "node_unreachable", m)
        }
        RemoteError::Unauthorized(m) => {
            ApiError::new(StatusCode::BAD_GATEWAY, "node_unauthorized", m)
        }
        RemoteError::Unsupported(m) => {
            ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "node_unsupported", m)
        }
        RemoteError::Invalid(m) => ApiError::new(StatusCode::BAD_GATEWAY, "node_error", m),
        RemoteError::JobFailed(m) => ApiError::new(StatusCode::BAD_GATEWAY, "node_error", m),
        RemoteError::Status {
            status, message, ..
        } => match status {
            404 => ApiError::not_found(message),
            409 => ApiError::conflict(message),
            400 | 422 => ApiError::bad_request(message),
            _ => ApiError::new(StatusCode::BAD_GATEWAY, "node_error", message),
        },
    }
}

fn load_nodes() -> Result<Vec<NodeConfig>, ApiError> {
    store().load().map_err(ApiError::internal)
}
fn find_node(id: &str) -> Result<NodeConfig, ApiError> {
    load_nodes()?
        .into_iter()
        .find(|n| n.id == id)
        .ok_or_else(|| ApiError::not_found(format!("There is no node '{id}'.")))
}

// ---- probing -----------------------------------------------------------------------------------------

struct Probe {
    meta: Result<Meta, String>,
    /// Servers of the node when it could be listed.
    servers: Result<Vec<Value>, String>,
}

fn probe(n: &NodeConfig) -> Probe {
    let rb = remote(n);
    match rb.meta() {
        Err(e) => {
            let m = e.to_string();
            Probe {
                meta: Err(m.clone()),
                servers: Err(m),
            }
        }
        Ok(meta) => {
            let servers = rb.servers_list().map_err(|e| e.to_string());
            Probe {
                meta: Ok(meta),
                servers,
            }
        }
    }
}

/// Probe every node on its own thread (each bounded by the transport timeouts).
fn probe_all(nodes: &[NodeConfig]) -> Vec<Probe> {
    std::thread::scope(|s| {
        let hs: Vec<_> = nodes.iter().map(|n| s.spawn(move || probe(n))).collect();
        hs.into_iter()
            .map(|h| {
                h.join().unwrap_or(Probe {
                    meta: Err("The check failed.".into()),
                    servers: Err("The check failed.".into()),
                })
            })
            .collect()
    })
}

fn can_control(m: &Meta) -> bool {
    m.supports("server_control") && m.supports("jobs_get")
}

fn node_view(n: &NodeConfig, p: &Probe) -> Value {
    let insecure = check_url(&n.url).map(|c| c.insecure).unwrap_or(false);
    let mut v = json!({
        "id": n.id,
        "name": n.name,
        "url": n.url,
        "token_set": !n.token.is_empty(),
        "insecure": insecure,
        "warning": insecure.then_some("This address uses plain http:// over the internet, so the token and your server details can be read on the way. Use https:// or Tailscale."),
        "reachable": p.meta.is_ok(),
        "version": null,
        "api": null,
        "platform": null,
        "ops": [],
        "server_count": null,
        "can_control": false,
        "last_error": null,
    });
    match &p.meta {
        Ok(m) => {
            v["version"] = json!(m.version);
            v["api"] = json!(m.api);
            v["platform"] = json!(m.platform);
            v["ops"] = json!(m.ops);
            v["can_control"] = json!(can_control(m));
        }
        Err(e) => v["last_error"] = json!(e),
    }
    match &p.servers {
        Ok(s) => v["server_count"] = json!(s.len()),
        Err(e) if p.meta.is_ok() => v["last_error"] = json!(e),
        Err(_) => {}
    }
    v
}

/// A node's server summary tagged with where it lives and what can be done to it from here.
fn tag(mut s: Value, n: &NodeConfig, meta: &Meta) -> Value {
    s["node_id"] = json!(n.id);
    s["node_name"] = json!(n.name);
    s["actions"] = if can_control(meta) {
        json!(["start", "stop", "restart"])
    } else {
        json!([])
    };
    s
}

// ---- handlers ----------------------------------------------------------------------------------------

pub async fn nodes_list() -> Result<Json<Value>, ApiError> {
    blocking(|| {
        let nodes = load_nodes()?;
        let probes = probe_all(&nodes);
        Ok(Json(json!({ "nodes": nodes.iter().zip(&probes).map(|(n, p)| node_view(n, p)).collect::<Vec<_>>() })))
    })
    .await
}

#[derive(Deserialize)]
pub struct NodeBody {
    #[serde(default)]
    name: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    token: String,
}

pub async fn node_add(Json(b): Json<NodeBody>) -> Result<Response, ApiError> {
    blocking(move || {
        let name = b.name.trim().to_string();
        if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
            return Err(ApiError::bad_request(
                "Give the node a name of up to 64 characters.",
            ));
        }
        let token = b.token.trim().to_string();
        if token.is_empty()
            || token.len() > 512
            || token.chars().any(|c| c.is_control() || c.is_whitespace())
        {
            return Err(ApiError::bad_request(
                "Enter the token from that panel (see 'kora panel user add' there).",
            ));
        }
        let chk = check_url(&b.url).map_err(ApiError::bad_request)?;
        // Check it is a Kora panel and that the token works before anything is saved. `/meta` needs no
        // login, so the server list is what proves the token.
        let probe_node = NodeConfig {
            id: String::new(),
            name: name.clone(),
            url: chk.url.clone(),
            token: token.clone(),
        };
        let p = probe(&probe_node);
        if let Err(e) = &p.meta {
            return Err(ApiError::new(
                StatusCode::BAD_GATEWAY,
                "node_unreachable",
                e.clone(),
            ));
        }
        let rb = remote(&probe_node);
        if let Err(e) = rb.servers_list() {
            return Err(remote_err(e));
        }
        let node = store().add(&name, &chk.url, &token).map_err(|m| {
            if m.contains("already") {
                ApiError::conflict(m)
            } else {
                ApiError::internal(m)
            }
        })?;
        Ok((
            StatusCode::CREATED,
            Json(json!({ "node": node_view(&node, &p) })),
        )
            .into_response())
    })
    .await
}

pub async fn node_remove(Path(id): Path<String>) -> Result<Response, ApiError> {
    blocking(move || {
        if store().remove(&id).map_err(ApiError::internal)? {
            Ok(StatusCode::NO_CONTENT.into_response())
        } else {
            Err(ApiError::not_found(format!("There is no node '{id}'.")))
        }
    })
    .await
}

pub async fn node_servers(Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    blocking(move || {
        let n = find_node(&id)?;
        let rb = remote(&n);
        let meta = rb.meta().map_err(remote_err)?;
        let list = rb.servers_list().map_err(remote_err)?;
        Ok(Json(
            json!({ "servers": list.into_iter().map(|s| tag(s, &n, &meta)).collect::<Vec<_>>() }),
        ))
    })
    .await
}

#[derive(Deserialize)]
pub struct ControlBody {
    #[serde(default)]
    action: String,
}

pub async fn node_server_control(
    State(st): State<AppState>,
    Path((id, s)): Path<(String, String)>,
    Json(b): Json<ControlBody>,
) -> Result<Response, ApiError> {
    if !matches!(b.action.as_str(), "start" | "stop" | "restart") {
        return Err(ApiError::bad_request(
            "The action must be start, stop or restart.",
        ));
    }
    let n = blocking({
        let id = id.clone();
        move || find_node(&id)
    })
    .await?;
    // Refuse at once, before any job, when the node cannot do this (older version or restricted).
    blocking({
        let n = n.clone();
        move || {
            let rb = remote(&n);
            rb.require("server_control").map_err(remote_err)?;
            rb.require("jobs_get").map_err(remote_err)?;
            Ok(())
        }
    })
    .await?;
    let action = b.action;
    st.spawn_job(
        Op::NodeServerControl,
        Some(format!("node:{id}:{s}")),
        move |h| {
            h.event(
                json!({"type": "status", "message": format!("Asking {} to {}", n.name, action)}),
            );
            let rb = remote(&n);
            rb.server_control(&s, &action, &mut |e| h.event(e.clone()))
                .map_err(|e| e.to_string())
        },
    )
}

pub async fn fleet(State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    blocking(move || {
        let nodes = load_nodes()?;
        // Remote nodes are probed in parallel threads while the local list is read.
        let (local, probes) = std::thread::scope(|sc| {
            let h = sc.spawn(|| probe_all(&nodes));
            let local = super::servers::list_json(&mut *st.backend());
            (local, h.join().unwrap_or_default_probes(nodes.len()))
        });
        let mut servers: Vec<Value> = local?["servers"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|mut s| {
                s["node_id"] = Value::Null;
                s["node_name"] = json!("This panel");
                s["actions"] = json!(["start", "stop", "restart"]);
                s
            })
            .collect();
        let mut views = Vec::new();
        for (n, p) in nodes.iter().zip(&probes) {
            views.push(node_view(n, p));
            if let (Ok(meta), Ok(list)) = (&p.meta, &p.servers) {
                servers.extend(list.iter().cloned().map(|s| tag(s, n, meta)));
            }
        }
        let partial = views.iter().any(|v| v["last_error"].is_string());
        Ok(Json(
            json!({ "servers": servers, "nodes": views, "partial": partial }),
        ))
    })
    .await
}

trait JoinProbes {
    fn unwrap_or_default_probes(self, n: usize) -> Vec<Probe>;
}
impl JoinProbes for std::thread::Result<Vec<Probe>> {
    fn unwrap_or_default_probes(self, n: usize) -> Vec<Probe> {
        self.unwrap_or_else(|_| {
            (0..n)
                .map(|_| Probe {
                    meta: Err("The check failed.".into()),
                    servers: Err("The check failed.".into()),
                })
                .collect()
        })
    }
}
