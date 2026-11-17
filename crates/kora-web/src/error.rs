//! API error type with the JSON shape from the plan (section 2.2), extended with the fields of a
//! [`Problem`]: `{"error": {"code", "message", "details"?, "actions"?, "command"?}}`.
//!
//! Handlers that know what the user can do return [`ApiError::problem`] (or `?` on a `Problem`);
//! core errors (`Res<T>` strings) go through [`ApiError::core`], which unpacks a packed Problem and
//! keeps plain strings working exactly as before.
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use kora_core::problem::{codes, Action, Problem};
use serde_json::json;

#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: String,
    pub message: String,
    /// Only for `needs_elevation`: the `kora ...` command the user must run as admin/root.
    pub command: Option<Vec<String>>,
    /// Technical details for the "Show details" box.
    pub details: Option<String>,
    /// What the user can do (see [`kora_core::problem::Action`]). Empty: the UI's generic choices.
    pub actions: Vec<Action>,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &str, message: impl Into<String>) -> Self {
        Self {
            status,
            code: code.to_string(),
            message: message.into(),
            command: None,
            details: None,
            actions: Vec::new(),
        }
    }
    /// A [`Problem`] with this HTTP status.
    pub fn problem(status: StatusCode, p: Problem) -> Self {
        let mut e = Self::new(status, &p.code, p.message);
        e.details = p.details;
        e.actions = p.actions;
        if e.code == codes::NEEDS_ELEVATION {
            e.command = e.actions.iter().find_map(|a| match a {
                Action::ShowCommand { command, .. } => Some(command.clone()),
                _ => None,
            });
        }
        e
    }
    /// The error as a [`Problem`] (for job errors and tests).
    pub fn to_problem(&self) -> Problem {
        Problem { code: self.code.clone(), message: self.message.clone(), details: self.details.clone(), actions: self.actions.clone() }
    }
    pub fn with_action(mut self, a: Action) -> Self {
        if !self.actions.contains(&a) {
            self.actions.push(a);
        }
        self
    }
    pub fn bad_request(m: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", m)
    }
    pub fn unauthorized(m: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", m)
    }
    pub fn forbidden(m: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", m)
    }
    pub fn not_found(m: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", m)
    }
    pub fn conflict(m: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "conflict", m)
    }
    pub fn internal(m: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", m)
    }
    /// Administrator rights are needed. Offers to ask for them ([`Action::RequestAdmin`], shown once
    /// the panel can) and the command to run as administrator; `command` stays for older pages.
    pub fn needs_elevation(m: impl Into<String>, command: Vec<String>) -> Self {
        Self::problem(StatusCode::FORBIDDEN, elevation_problem(m, command))
    }
    /// 501: the route exists but this build cannot serve it yet.
    pub fn not_implemented(op: kora_core::api::Op) -> Self {
        Self::new(StatusCode::NOT_IMPLEMENTED, "not_implemented", format!("'{}' is not available in this version", op.name()))
    }
    /// Map a core error string (`Res<T>`) to an API error. Core messages are written for people; a
    /// packed [`Problem`] keeps its code, details and actions.
    pub fn core(m: impl Into<String>) -> Self {
        let m = m.into();
        if Problem::is_encoded(&m) {
            let p = Problem::decode(&m);
            let status = if p.code == codes::NEEDS_ELEVATION { StatusCode::FORBIDDEN } else { StatusCode::UNPROCESSABLE_ENTITY };
            return Self::problem(status, p);
        }
        let m = friendly(m);
        let lower = m.to_lowercase();
        if lower.contains("not found") || lower.contains("no server named") || lower.contains("unknown server") {
            Self::not_found(m)
        } else {
            Self::new(StatusCode::UNPROCESSABLE_ENTITY, "bad_request", m)
        }
    }
    /// 429; `rate_limited` is an addition to the plan's code list.
    pub fn rate_limited(m: impl Into<String>) -> Self {
        Self::new(StatusCode::TOO_MANY_REQUESTS, "rate_limited", m)
    }
}

/// The [`Problem`] for "this needs administrator rights": ask for them, or run `command` as administrator.
pub fn elevation_problem(m: impl Into<String>, command: Vec<String>) -> Problem {
    Problem::new(codes::NEEDS_ELEVATION, m)
        .with_action(Action::RequestAdmin)
        .with_action(Action::ShowCommand { command, elevated: true })
}

impl From<Problem> for ApiError {
    /// 422, or 403 for `needs_elevation`.
    fn from(p: Problem) -> Self {
        let status = if p.code == codes::NEEDS_ELEVATION { StatusCode::FORBIDDEN } else { StatusCode::UNPROCESSABLE_ENTITY };
        Self::problem(status, p)
    }
}

/// What a person should read when an action needs Kora's own record of a server (`kora-server.json`)
/// and the server was added from an existing folder instead.
pub const NOT_KORA_INSTALLED: &str = "This server was added from an existing folder, so Kora did not install it and has no record of how it was set up. Updates, resources, ports and starting from Kora only work for servers that Kora installed.";

/// Replace raw file-system errors about Kora's server record with a plain explanation.
pub fn friendly(m: String) -> String {
    let record_missing = m.contains("kora-server.json") && (m.contains("No such file") || m.contains("cannot find") || m.contains("os error 2"));
    if record_missing || m == "No such file or directory (os error 2)" {
        NOT_KORA_INSTALLED.to_string()
    } else {
        m
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for ApiError {}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut err = json!({"code": self.code, "message": self.message});
        if let Some(c) = &self.command {
            err["command"] = json!(c);
        }
        if let Some(d) = &self.details {
            err["details"] = json!(d);
        }
        if !self.actions.is_empty() {
            err["actions"] = json!(self.actions);
        }
        (self.status, Json(json!({ "error": err }))).into_response()
    }
}

/// Run blocking work off the async runtime; a panic becomes an `internal` error.
pub async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    match tokio::task::spawn_blocking(f).await {
        Ok(r) => r,
        Err(_) => Err(ApiError::internal("operation panicked")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_server_record_gets_a_plain_message() {
        let raw = "cannot read kora-server.json: No such file or directory (os error 2)";
        assert_eq!(friendly(raw.into()), NOT_KORA_INSTALLED);
        assert_eq!(friendly("No such file or directory (os error 2)".into()), NOT_KORA_INSTALLED);
        assert_eq!(friendly("the port is busy".into()), "the port is busy");
        assert_eq!(ApiError::core(raw).message, NOT_KORA_INSTALLED);
    }

    async fn body(e: ApiError) -> (StatusCode, serde_json::Value) {
        let r = e.into_response();
        let status = r.status();
        let b = axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap();
        (status, serde_json::from_slice(&b).unwrap())
    }

    #[tokio::test]
    async fn a_packed_problem_reaches_the_browser_whole() {
        let p = Problem::new(codes::GH_MISSING, "The GitHub CLI is not installed.")
            .with_details("gh: not found")
            .with_action(Action::InstallTool { tool: kora_core::problem::Tool::Gh });
        // Through a `Res<T>` string, as a Backend method would return it.
        let (status, v) = body(ApiError::core(String::from(p.clone()))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(v["error"], p.to_json());
        assert_eq!(serde_json::from_value::<Problem>(v["error"].clone()).unwrap(), p);
        // Plain strings answer exactly as before (no details, no actions).
        let (status, v) = body(ApiError::core("the port is busy")).await;
        assert_eq!((status, v), (StatusCode::UNPROCESSABLE_ENTITY, json!({"error": {"code": "bad_request", "message": "the port is busy"}})));
    }

    #[tokio::test]
    async fn needs_elevation_keeps_its_command_and_offers_actions() {
        let (status, v) = body(ApiError::needs_elevation("Needs admin.", vec!["kora".into(), "x".into()])).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(v["error"]["code"], "needs_elevation");
        assert_eq!(v["error"]["command"], json!(["kora", "x"]));
        assert_eq!(v["error"]["actions"], json!([{"kind": "request_admin"}, {"kind": "show_command", "command": ["kora", "x"], "elevated": true}]));
        // A packed elevation problem from the core is a 403 with the same shape.
        let e = ApiError::core(String::from(elevation_problem("Needs admin.", vec!["kora".into(), "x".into()])));
        assert_eq!((e.status, e.command.clone()), (StatusCode::FORBIDDEN, Some(vec!["kora".to_string(), "x".to_string()])));
        assert_eq!(e.to_problem().code, "needs_elevation");
    }
}
