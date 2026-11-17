//! The audit log in the panel: records state-changing requests, and `GET /audit` (admins only).
//!
//! One row when a state-changing request has been answered (`ok`, `failed`, or `refused` when the role was
//! too low or remote actions were off), plus, for slow operations, a second row when the job ends. Who
//! did it is the person on this computer (`local`, named after the account running Kora), or the remote
//! identity with its provider (token or Tailscale) and role. Nothing from the request body or the
//! response ever goes in: only the operation, the server named in the address, a job id and an error CODE
//! (see `kora_core::store::audit`). Writing never fails a request.
//!
//! Which requests count: every request that is not a read, except the few that only look
//! ([`audited`]). A new operation is audited unless it is added to that list, so the default is a bit
//! too much record, never too little.

use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::http::{Method, StatusCode};
use axum::response::Response;
use axum::{Extension, Json};
use kora_core::api::Op;
use kora_core::store::audit::{self, actor, result, New};
use serde_json::{json, Value};

use crate::error::ApiError;
use crate::remote::{roles, Caller, Identity, Provider, Role};
use crate::AppState;

/// Put in the response of a request that started a job, so the guard can follow the job to its end.
#[derive(Debug, Clone)]
pub struct JobStarted {
    pub id: String,
    pub server: Option<String>,
}

/// Put in the response of a request that acted on GitHub, so the log row says as whom (the one GitHub account `gh` is signed
/// in as, shared by every panel user). A login is a short plain name, never a secret.
#[derive(Debug, Clone)]
pub struct GithubActor(pub String);

/// The state-changing operations that are deliberately NOT written to the log, each with the reason (like the roles table):
/// a new operation is audited unless it is listed here, so the default is a bit too much record, never too little. The test
/// `every_operation_is_audited_or_has_a_reason` walks `Op::ALL` against this table.
pub const NOT_AUDITED: &[(Op, &str)] = &[
    (Op::SessionCreate, "signing in; the sign-in steps write their own rows"),
    (Op::SessionLogout, "signing out changes nothing that is kept"),
    (Op::AuthLogin, "the sign-in steps write their own rows (who, how, the outcome), so the guard does not add a nameless one"),
    (Op::AuthEnrol, "the sign-in steps write their own rows"),
    (Op::AuthReauth, "the sign-in steps write their own rows"),
    (Op::ServersScan, "only looks for folders (a job that changes nothing)"),
    (Op::ServerOpenFolder, "opens a window on this computer; changes nothing"),
    (Op::UpdatePreview, "a preview; changes nothing"),
    (Op::ResourcesPlan, "a plan; changes nothing"),
    (Op::WebhookTest, "sends a test message; changes nothing"),
    (Op::PrPlan, "a plan; changes nothing"),
    (Op::CheckPorts, "a check; changes nothing"),
    (Op::CheckDisk, "a check; changes nothing"),
    (Op::CheckDbLogin, "a check; changes nothing"),
    (Op::CheckCfx, "a check; changes nothing"),
    (Op::InstallPlan, "a plan; changes nothing"),
    (Op::UninstallPlan, "a plan; changes nothing"),
    (Op::AssetInspect, "looks inside a file; changes nothing"),
    (Op::AlertsAck, "marks an alert as seen; a view preference"),
    (Op::BranchBulkPreview, "a preview; changes nothing"),
    (Op::BackupPreview, "a preview of a restore; changes nothing"),
    (Op::SupportBundlePreview, "a preview; changes nothing"),
    (Op::SupportBundleFile, "reads a file the person already made (the bundle is made by an audited operation)"),
    (Op::SupportSummary, "reads a summary; changes nothing"),
    (Op::BackupImportInspect, "looks inside a moving file; changes nothing"),
];

/// Reads that are written down anyway: downloading hands out the secrets inside the file.
const READS_THAT_ARE_AUDITED: &[Op] = &[Op::BackupDownload, Op::SupportBundleDownload];

/// Does a request for `op` with `method` belong in the log?
pub fn audited(op: Op, method: &Method) -> bool {
    if (method == Method::GET || method == Method::HEAD) && !READS_THAT_ARE_AUDITED.contains(&op) {
        return false;
    }
    !NOT_AUDITED.iter().any(|(o, _)| *o == op)
}

/// How an answer is written in the log: `ok`, `refused` (401/403) or `failed`.
pub fn result_for(status: StatusCode) -> &'static str {
    if status.is_success() {
        result::OK
    } else if status == StatusCode::FORBIDDEN || status == StatusCode::UNAUTHORIZED {
        result::REFUSED
    } else {
        result::FAILED
    }
}

#[derive(Debug, Clone)]
struct Actor {
    kind: &'static str,
    name: Option<String>,
    provider: Option<&'static str>,
    role: Option<&'static str>,
}

fn os_user() -> Option<String> {
    ["SUDO_USER", "USER", "USERNAME", "LOGNAME"].iter().find_map(|k| std::env::var(k).ok().filter(|v| !v.trim().is_empty()))
}

fn provider_text(p: Provider) -> &'static str {
    match p {
        Provider::Local => "local",
        Provider::Tailscale => "tailscale",
        Provider::Token => "token",
        Provider::Passkey => "passkey",
        Provider::Code => "code",
        Provider::ApiToken => "api-token",
    }
}

fn actor_of(remote: bool, id: Option<&Identity>) -> Actor {
    match (remote, id) {
        (true, Some(i)) => Actor { kind: actor::REMOTE, name: Some(i.name.clone()), provider: Some(provider_text(i.provider)), role: Some(i.role.as_str()) },
        (true, None) => Actor { kind: actor::REMOTE, name: None, provider: None, role: None },
        (false, _) => Actor { kind: actor::LOCAL, name: os_user(), provider: Some("local"), role: Some("admin") },
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 3 <= b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
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

/// The server a request is about, when its address names one (`/api/v1/servers/<name>/...`).
pub fn server_in(path: &str) -> Option<String> {
    let rest = path.strip_prefix("/api/v1/servers/")?;
    let seg = rest.split('/').next()?;
    (!seg.is_empty() && seg != "scan" && seg != "adopt").then(|| percent_decode(seg))
}

/// Everything the guard needs to write the row once the answer is known.
pub struct Ctx {
    home: std::path::PathBuf,
    op: Op,
    server: Option<String>,
    actor: Actor,
}

enum Msg {
    Row(std::path::PathBuf, Box<New>),
    Flush(std::sync::mpsc::Sender<()>),
}

/// One writer thread, so a request never waits for the disk and rows land in the order they were handed over.
fn queue() -> &'static std::sync::Mutex<std::sync::mpsc::Sender<Msg>> {
    static Q: std::sync::OnceLock<std::sync::Mutex<std::sync::mpsc::Sender<Msg>>> = std::sync::OnceLock::new();
    Q.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<Msg>();
        let _ = std::thread::Builder::new().name("kora-audit".into()).spawn(move || {
            for m in rx {
                match m {
                    Msg::Row(home, n) => audit::record_quiet(&home, &n),
                    Msg::Flush(done) => {
                        let _ = done.send(());
                    }
                }
            }
        });
        std::sync::Mutex::new(tx)
    })
}

/// Wait until every row handed over so far is written (tests; and when the panel stops).
#[doc(hidden)]
pub fn flush() {
    let (tx, rx) = std::sync::mpsc::channel();
    if queue().lock().unwrap_or_else(|p| p.into_inner()).send(Msg::Flush(tx)).is_ok() {
        let _ = rx.recv_timeout(std::time::Duration::from_secs(10));
    }
}

fn write(home: &std::path::Path, op: Op, server: Option<&str>, a: &Actor, res: &str, detail: Option<String>) {
    let row = New {
        op: op.name().to_string(),
        server: server.map(String::from),
        actor_kind: a.kind.to_string(),
        actor_name: a.name.clone(),
        provider: a.provider.map(String::from),
        role: a.role.map(String::from),
        result: res.to_string(),
        detail,
    };
    let _ = queue().lock().unwrap_or_else(|p| p.into_inner()).send(Msg::Row(home.to_path_buf(), Box::new(row)));
}

impl Ctx {
    /// `None` when the request is not one to record.
    pub fn begin(home: &std::path::Path, method: &Method, path: &str, caller: &Caller) -> Option<Ctx> {
        let op = roles::op_for(method.as_str(), path)?;
        audited(op, method).then(|| Ctx { home: home.to_path_buf(), op, server: server_in(path), actor: actor_of(caller.remote, caller.identity.as_ref()) })
    }

    /// The request was answered.
    pub fn answered(self, resp: &Response) {
        let status = resp.status();
        if let Some(job) = resp.extensions().get::<JobStarted>() {
            write(&self.home, self.op, job.server.as_deref().or(self.server.as_deref()), &self.actor, result::STARTED, Some(format!("job {}", job.id)));
            return;
        }
        let github = resp.extensions().get::<GithubActor>().map(|g| format!("as GitHub account {}", g.0));
        let res = result_for(status);
        write(&self.home, self.op, self.server.as_deref(), &self.actor, res, (!status.is_success()).then(|| format!("HTTP {}", status.as_u16())).or(github));
    }

    /// Wait for the job to end (on its own thread) and write how it ended.
    pub fn follow_job(&self, st: &AppState, job: &JobStarted) {
        let Some((_, rx)) = st.registry.subscribe(&job.id) else { return };
        let (home, op, actor, server, id) = (self.home.clone(), self.op, self.actor.clone(), job.server.clone().or_else(|| self.server.clone()), job.id.clone());
        let _ = std::thread::Builder::new().name("kora-audit-job".into()).spawn(move || {
            use kora_core::api::JobMessage;
            loop {
                match rx.recv() {
                    Ok(JobMessage::Event(_)) => {}
                    Ok(JobMessage::Done(v)) => {
                        // Making copies is done on GitHub too: say into which account.
                        let who = (op == Op::ResourcesFork)
                            .then(|| v.get("owner").and_then(Value::as_str).filter(|o| !o.is_empty() && o.len() <= 64).map(|o| format!(", copies in GitHub account {o}")))
                            .flatten();
                        return write(&home, op, server.as_deref(), &actor, result::OK, Some(format!("job {id}{}", who.unwrap_or_default())));
                    }
                    Ok(JobMessage::Failed(e)) => {
                        // Only the error CODE: the text can name paths.
                        let code = crate::jobs::job_error(&e).get("code").and_then(Value::as_str).unwrap_or("failed").to_string();
                        return write(&home, op, server.as_deref(), &actor, result::FAILED, Some(format!("job {id}: {code}")));
                    }
                    Err(_) => return,
                }
            }
        });
    }
}

/// A signed-in remote user was turned away by the role rules.
pub fn refused(home: &std::path::Path, method: &Method, path: &str, id: &Identity) {
    if let Some(op) = roles::op_for(method.as_str(), path) {
        if audited(op, method) {
            write(home, op, server_in(path).as_deref(), &actor_of(true, Some(id)), result::REFUSED, None);
        }
    }
}

/// `GET /audit?limit=&before=&server=&op=&actor=&result=&since=`: the newest first. Admins only.
pub async fn audit_list(State(st): State<AppState>, Extension(caller): Extension<Caller>, Query(q): Query<HashMap<String, String>>) -> Result<Json<Value>, ApiError> {
    if caller.remote && !caller.identity.as_ref().is_some_and(|i| i.role >= Role::Admin) {
        return Err(ApiError::forbidden("Only an administrator can see the audit log."));
    }
    let home = st.audit_home.clone().unwrap_or_else(kora_core::installer::registry::kora_home);
    crate::blocking(move || {
        flush();
        let limit: usize = q.get("limit").and_then(|v| v.parse().ok()).unwrap_or(100);
        let query = audit::Query {
            limit: limit + 1,
            before: q.get("before").and_then(|v| v.parse().ok()),
            server: q.get("server").filter(|v| !v.is_empty()).cloned(),
            op: q.get("op").filter(|v| !v.is_empty()).cloned(),
            actor: q.get("actor").filter(|v| !v.is_empty()).cloned(),
            result: q.get("result").filter(|v| !v.is_empty()).cloned(),
            since: q.get("since").and_then(|v| v.parse().ok()),
        };
        // Looking never creates the store: nothing recorded yet is an empty list.
        let Some(s) = kora_core::store::Store::open_existing(&home).map_err(|p| ApiError::problem(StatusCode::INTERNAL_SERVER_ERROR, p))? else {
            return Ok(Json(json!({"entries": [], "more": false, "retention_days": audit::RETENTION_DAYS})));
        };
        let mut rows = audit::list(&s, &query).map_err(|p| ApiError::problem(StatusCode::INTERNAL_SERVER_ERROR, p))?;
        let more = rows.len() > limit.clamp(1, 500);
        rows.truncate(limit.clamp(1, 500));
        Ok(Json(json!({"entries": rows, "more": more, "retention_days": audit::RETENTION_DAYS})))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_changes_are_audited() {
        assert!(audited(Op::ServerControl, &Method::POST) && audited(Op::KeyRemove, &Method::DELETE) && audited(Op::KeyUpdate, &Method::PUT));
        assert!(!audited(Op::ServersList, &Method::GET) && !audited(Op::CheckPorts, &Method::POST) && !audited(Op::SessionCreate, &Method::POST));
        assert!(!audited(Op::InstallPlan, &Method::POST) && !audited(Op::UpdatePreview, &Method::POST));
        assert!(audited(Op::InstallStart, &Method::POST));
    }

    #[test]
    fn every_operation_is_audited_or_has_a_reason() {
        use kora_core::api::Method as M;
        // the table itself: no duplicates, a reason each, and only operations that would otherwise be written
        for (i, (op, why)) in NOT_AUDITED.iter().enumerate() {
            assert!(!why.trim().is_empty(), "{op:?} needs a reason");
            assert!(NOT_AUDITED[..i].iter().all(|(o, _)| o != op), "{op:?} is listed twice");
            assert!(op.route().0 != M::Get, "{op:?} is a read: reads are not audited anyway, so it does not belong in the table");
        }
        let (mut audited_n, mut skipped_n) = (0, 0);
        for op in Op::ALL {
            let (m, pattern) = op.route();
            let method = Method::from_bytes(m.as_str().as_bytes()).unwrap();
            // the request the router would send to this op reaches this op in the guard (so a row is written for it)
            let path = format!("/api/v1{}", pattern.replace("{s}", "prod").replace("{id}", "x").replace("{file}", "f.log").replace("{job}", "j").replace("{name}", "bob"));
            let reached = roles::op_for(m.as_str(), &path);
            assert_eq!(reached, Some(*op), "{op:?} {path} does not resolve back to itself, so the guard could not record it");
            let expected = (m != M::Get || READS_THAT_ARE_AUDITED.contains(op)) && !NOT_AUDITED.iter().any(|(o, _)| o == op);
            assert_eq!(audited(*op, &method), expected, "{op:?}");
            if expected {
                audited_n += 1;
            } else {
                skipped_n += 1;
            }
        }
        // sanity: most state changes are recorded
        assert!(audited_n > 60 && skipped_n >= NOT_AUDITED.len(), "{audited_n} audited, {skipped_n} not (reads and the table)");
        // every state change that is written down says WHO (the actor of the request) and HOW IT WENT
        let local = actor_of(false, None);
        assert_eq!((local.kind, local.role), ("local", Some("admin")));
        let remote = actor_of(true, Some(&Identity { name: "o".into(), provider: Provider::Token, role: Role::Operator }));
        assert_eq!((remote.kind, remote.role), ("remote", Some("operator")));
        for (status, want) in [(200, "ok"), (201, "ok"), (204, "ok"), (400, "failed"), (401, "refused"), (403, "refused"), (404, "failed"), (409, "failed"), (422, "failed"), (500, "failed")] {
            assert_eq!(result_for(StatusCode::from_u16(status).unwrap()), want, "{status}");
        }
    }

    #[test]
    fn the_server_comes_from_the_address() {
        assert_eq!(server_in("/api/v1/servers/live/control").as_deref(), Some("live"));
        assert_eq!(server_in("/api/v1/servers/Dev%20Den/update/apply").as_deref(), Some("Dev Den"));
        assert_eq!(server_in("/api/v1/servers/scan"), None);
        assert_eq!(server_in("/api/v1/keys"), None);
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("a%2"), "a%2");
    }

    #[test]
    fn who_is_told_plainly() {
        let local = actor_of(false, None);
        assert_eq!((local.kind, local.provider, local.role), ("local", Some("local"), Some("admin")));
        let id = Identity { name: "sam@example.com".into(), provider: Provider::Tailscale, role: Role::Operator };
        let r = actor_of(true, Some(&id));
        assert_eq!((r.kind, r.name.as_deref(), r.provider, r.role), ("remote", Some("sam@example.com"), Some("tailscale"), Some("operator")));
    }
}
