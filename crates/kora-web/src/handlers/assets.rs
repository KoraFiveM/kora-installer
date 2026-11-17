//! Paid (Cfx.re Portal) assets: list, inspect an uploaded or downloaded zip, import, remove, and the
//! recent zips in the Downloads folder.
//!
//! Each `pub async fn` is mounted by `crate::routes` for the [`Op`] of the same name; the HTTP
//! method and path come from `Op::route`, so only the handler's extractors live here.
//!
//! **Uploads.** `POST /servers/{s}/assets/inspect` takes the zip itself as the request body
//! (`Content-Type: application/zip` or `application/octet-stream`, the file name in `X-File-Name`,
//! URL-encoded). It is streamed to `<server>/.kora/asset-uploads/<id>/<name>` (never held in memory)
//! up to [`UPLOAD_LIMIT`], inspected, and answered with the plan and the upload `id`, which
//! `POST /servers/{s}/assets` then imports. The same inspect route with a JSON body
//! `{"download": "<file>.zip"}` reads a recent zip from the Downloads folder instead (only from this
//! computer, never over remote access).
#![allow(clippy::unused_async)]

use std::io::Write;
use std::path::{Path as FsPath, PathBuf};
use std::sync::RwLock;

use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::Response;
use axum::{Extension, Json};
use kora_core::api::Op;
use kora_core::installer::registry::{now_unix, ServerEntry};
use kora_core::paidassets::{self, ImportOpts};
use serde_json::{json, Value};

use crate::remote::Caller;
use crate::{blocking, ApiError, AppState};

/// Largest zip accepted as an upload.
pub const UPLOAD_LIMIT: u64 = 2 << 30;
/// How recent a zip in the Downloads folder must be to be offered.
pub const RECENT_SECS: u64 = 24 * 60 * 60;
/// Uploads older than this are cleared away.
const UPLOAD_KEEP_SECS: u64 = 24 * 60 * 60;
const JSON_LIMIT: usize = 16 * 1024;

static DOWNLOADS: RwLock<Option<PathBuf>> = RwLock::new(None);
static UPLOADS: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Tests: use `dir` as the Downloads folder (`None` = the real one).
pub fn set_downloads_dir(dir: Option<PathBuf>) {
    *DOWNLOADS.write().unwrap_or_else(|e| e.into_inner()) = dir;
}

/// Tests: keep uploads in `dir` instead of the server folder (`None` = the server folder).
pub fn set_upload_dir(dir: Option<PathBuf>) {
    *UPLOADS.write().unwrap_or_else(|e| e.into_inner()) = dir;
}

fn downloads_dir() -> Option<PathBuf> {
    let o = DOWNLOADS.read().unwrap_or_else(|e| e.into_inner()).clone();
    o.or_else(paidassets::default_downloads_dir)
}

fn uploads_dir(e: &ServerEntry) -> PathBuf {
    let o = UPLOADS.read().unwrap_or_else(|e| e.into_inner()).clone();
    o.unwrap_or_else(|| e.dir.join(".kora").join("asset-uploads"))
}

fn local_only(caller: &Caller) -> Result<(), ApiError> {
    if caller.remote {
        return Err(ApiError::forbidden("Your Downloads folder can only be used on the computer that runs Kora. Upload the zip instead."));
    }
    Ok(())
}

fn to_json<T: serde::Serialize>(v: &T) -> Result<Value, ApiError> {
    serde_json::to_value(v).map_err(|x| ApiError::internal(x.to_string()))
}

fn valid_upload_id(id: &str) -> bool {
    (16..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A safe file name for the upload, from the `X-File-Name` header (URL-encoded). Keeps the name the
/// zip had, because a zip without a folder is named after it.
fn upload_name(h: &HeaderMap) -> String {
    let raw = h.get("x-file-name").and_then(|v| v.to_str().ok()).unwrap_or("");
    let decoded = percent_decode(raw);
    let base = decoded.rsplit(['/', '\\']).next().unwrap_or("").trim();
    let clean: String = base.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ' ' | '(' | ')')).take(120).collect();
    let clean = clean.trim().trim_start_matches('.').to_string();
    if clean.is_empty() || clean.contains("..") {
        "upload.zip".into()
    } else if clean.to_ascii_lowercase().ends_with(".zip") {
        clean
    } else {
        format!("{clean}.zip")
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok();
            if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Remove uploads older than a day.
fn prune_uploads(dir: &FsPath) {
    let now = std::time::SystemTime::now();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| now.duration_since(t).ok()).is_some_and(|d| d.as_secs() > UPLOAD_KEEP_SECS);
        if old {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// Stream `body` into `path` (a new file), at most `limit` bytes. The file is removed on failure.
pub(crate) async fn save_body(body: Body, path: PathBuf, limit: u64) -> Result<u64, ApiError> {
    use tokio_stream::StreamExt;
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Bytes>(8);
    let p = path.clone();
    let writer = tokio::task::spawn_blocking(move || -> Result<u64, String> {
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&p).map_err(|e| format!("Can't save the upload: {e}"))?;
        let mut n = 0u64;
        while let Some(chunk) = rx.blocking_recv() {
            f.write_all(&chunk).map_err(|e| format!("Can't save the upload: {e}"))?;
            n += chunk.len() as u64;
        }
        f.flush().map_err(|e| format!("Can't save the upload: {e}"))?;
        Ok(n)
    });
    let mut stream = body.into_data_stream();
    let mut total = 0u64;
    let mut failure: Option<ApiError> = None;
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(_) => {
                failure = Some(ApiError::bad_request("The upload was interrupted. Try again."));
                break;
            }
        };
        total += chunk.len() as u64;
        if total > limit {
            failure = Some(ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "too_large", format!("That file is larger than {}, too big for a resource.", paidassets::human(limit))));
            break;
        }
        if tx.send(chunk).await.is_err() {
            break; // the writer failed; its error is reported below
        }
    }
    drop(tx);
    let written = writer.await.map_err(|_| ApiError::internal("saving the upload failed"));
    let result = match (failure, written) {
        (Some(f), _) => Err(f),
        (None, Err(e)) => Err(e),
        (None, Ok(Err(m))) => Err(ApiError::internal(m)),
        (None, Ok(Ok(0))) => Err(ApiError::bad_request("The file is empty.")),
        (None, Ok(Ok(n))) => Ok(n),
    };
    if result.is_err() {
        let _ = std::fs::remove_file(&path);
    }
    result
}

fn is_json(h: &HeaderMap) -> bool {
    h.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|v| v.trim_start().to_ascii_lowercase().starts_with("application/json"))
}

/// Where a zip named by a JSON body is: `{"upload": id}` or `{"download": name}`.
fn zip_from(body: &Value, e: &ServerEntry, caller: &Caller) -> Result<PathBuf, ApiError> {
    if let Some(id) = body.get("upload").and_then(Value::as_str) {
        if !valid_upload_id(id) {
            return Err(ApiError::bad_request("That upload is not known. Upload the zip again."));
        }
        let dir = uploads_dir(e).join(id);
        let file = std::fs::read_dir(&dir)
            .ok()
            .and_then(|mut d| d.find_map(|x| x.ok().map(|x| x.path()).filter(|p| p.is_file())))
            .ok_or_else(|| ApiError::not_found("That upload is gone (uploads are kept for a day). Upload the zip again."))?;
        return Ok(file);
    }
    if let Some(name) = body.get("download").and_then(Value::as_str) {
        local_only(caller)?;
        let dir = downloads_dir().ok_or_else(|| ApiError::not_found("No Downloads folder was found on this computer."))?;
        return paidassets::resolve_download(&dir, name, now_unix(), RECENT_SECS).map_err(ApiError::not_found);
    }
    Err(ApiError::bad_request("Send a zip file, or pick one from your Downloads folder."))
}

pub async fn assets_list(State(st): State<AppState>, Path(s): Path<String>) -> Result<Json<Value>, ApiError> {
    blocking(move || {
        let e = st.entry(&s)?;
        let v = st.backend().assets_list(&e).map_err(ApiError::core)?;
        let mut j = to_json(&v)?;
        j["portal_url"] = json!(paidassets::PORTAL_URL);
        Ok(Json(j))
    })
    .await
}

pub async fn asset_inspect(State(st): State<AppState>, Extension(caller): Extension<Caller>, Path(s): Path<String>, headers: HeaderMap, body: Body) -> Result<Json<Value>, ApiError> {
    let st2 = st.clone();
    let e = blocking(move || st2.entry(&s)).await?;
    if is_json(&headers) {
        let bytes = axum::body::to_bytes(body, JSON_LIMIT).await.map_err(|_| ApiError::bad_request("That request is too large."))?;
        let v: Value = serde_json::from_slice(&bytes).map_err(|_| ApiError::bad_request("That is not valid JSON."))?;
        if v.get("upload").is_some() {
            return Err(ApiError::bad_request("Send a zip file, or pick one from your Downloads folder."));
        }
        let zip = zip_from(&v, &e, &caller)?;
        let name = zip.file_name().map(|n| n.to_string_lossy().into_owned());
        return blocking(move || {
            let plan = st.backend().asset_inspect(&e, &zip).map_err(ApiError::core)?;
            Ok(Json(json!({"plan": to_json(&plan)?, "download": name})))
        })
        .await;
    }
    let declared = headers.get(header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
    if declared.is_some_and(|n| n > UPLOAD_LIMIT) {
        return Err(ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "too_large", format!("That file is larger than {}, too big for a resource.", paidassets::human(UPLOAD_LIMIT))));
    }
    let id = crate::security::random_hex();
    let root = uploads_dir(&e);
    let dir = root.join(&id);
    let r2 = root.clone();
    let d2 = dir.clone();
    blocking(move || {
        prune_uploads(&r2);
        std::fs::create_dir_all(&d2).map_err(|x| ApiError::internal(format!("Can't save the upload: {x}")))
    })
    .await?;
    let path = dir.join(upload_name(&headers));
    if let Err(x) = save_body(body, path.clone(), UPLOAD_LIMIT).await {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(x);
    }
    blocking(move || {
        let plan = st.backend().asset_inspect(&e, &path);
        let keep = plan.as_ref().is_ok_and(|p| p.can_import);
        if !keep {
            let _ = std::fs::remove_dir_all(&dir);
        }
        let plan = plan.map_err(ApiError::core)?;
        Ok(Json(json!({"plan": to_json(&plan)?, "upload": if keep { Some(id) } else { None }})))
    })
    .await
}

pub async fn asset_import(State(st): State<AppState>, Extension(caller): Extension<Caller>, Path(s): Path<String>, Json(body): Json<Value>) -> Result<Response, ApiError> {
    let replace = body.get("replace").and_then(Value::as_bool).unwrap_or(true);
    let st2 = st.clone();
    let (e, zip) = blocking(move || {
        let e = st2.entry(&s)?;
        let zip = zip_from(&body, &e, &caller)?;
        Ok((e, zip))
    })
    .await?;
    let upload_dir = zip.parent().filter(|p| p.parent() == Some(uploads_dir(&e).as_path())).map(FsPath::to_path_buf);
    let make = st.backend.clone();
    let name = e.name.clone();
    st.spawn_job(Op::AssetImport, Some(name), move |h| {
        let out = make().asset_import(&e, &zip, &ImportOpts { replace }, &mut |m| {
            h.event(json!({"type": "status", "message": m}));
        })?;
        for a in &out.imported {
            h.event(json!({"type": "asset", "name": a.name, "state": "imported"}));
        }
        if let Some(d) = upload_dir {
            let _ = std::fs::remove_dir_all(d);
        }
        Ok(serde_json::to_value(&out)?)
    })
}

pub async fn asset_remove(State(st): State<AppState>, Path((s, name)): Path<(String, String)>) -> Result<Json<Value>, ApiError> {
    if !paidassets::valid_asset_name(&name) {
        return Err(ApiError::bad_request("That is not a resource name."));
    }
    blocking(move || {
        let e = st.entry(&s)?;
        let notes = st.backend().asset_remove(&e, &name).map_err(ApiError::core)?;
        Ok(Json(json!({"notes": notes})))
    })
    .await
}

pub async fn asset_downloads(Extension(caller): Extension<Caller>) -> Result<Json<Value>, ApiError> {
    local_only(&caller)?;
    blocking(|| {
        let Some(dir) = downloads_dir() else {
            return Ok(Json(json!({"found": false, "zips": []})));
        };
        let zips = paidassets::recent_zips(&dir, now_unix(), RECENT_SECS);
        Ok(Json(json!({"found": true, "folder": dir, "zips": to_json(&zips)?})))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_names_are_made_safe() {
        let h = |v: &str| {
            let mut m = HeaderMap::new();
            m.insert("x-file-name", v.parse().unwrap());
            upload_name(&m)
        };
        assert_eq!(h("qs-housing.zip"), "qs-housing.zip");
        assert_eq!(h("my%20asset%20(1).zip"), "my asset (1).zip");
        assert_eq!(h("..%2F..%2Fetc%2Fpasswd"), "passwd.zip");
        assert_eq!(h("C:%5Cx%5Cy.zip"), "y.zip");
        assert_eq!(h("..."), "upload.zip");
        assert_eq!(h("a..b.zip"), "upload.zip");
        assert_eq!(upload_name(&HeaderMap::new()), "upload.zip");
    }

    #[test]
    fn upload_ids_are_hex() {
        assert!(valid_upload_id(&"ab".repeat(16)));
        assert!(!valid_upload_id("../../etc"));
        assert!(!valid_upload_id("abc"));
    }
}
