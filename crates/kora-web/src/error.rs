//! API error type with the JSON shape from the plan (section 2.2), extended with the fields of a
//! [`Problem`]: `{"error": {"code", "message", "details"?, "actions"?, "command"?}}`.
//!
//! Core errors are [`Problem`]s ([`kora_core::problem::Res`]): handlers return them with `?` (an
//! `impl From<Problem> for ApiError`) or through [`ApiError::core`]. A Problem keeps its code,
//! details and actions all the way to the JSON envelope.
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use kora_core::problem::{codes, Action, MessageRef, Problem};
use serde_json::json;

#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: String,
    pub message: String,
    /// Only for `needs_elevation`: the `kora ...` command the user must run as admin/root.
    pub command: Option<Vec<String>>,
    /// A [`Problem`]'s details and actions (boxed: most errors have neither, and `ApiError` is
    /// returned everywhere). Read them with [`ApiError::details`] and [`ApiError::actions`].
    extra: Option<Box<Extra>>,
}

#[derive(Debug, Clone, Default)]
struct Extra {
    details: Option<String>,
    actions: Vec<Action>,
    i18n: Option<MessageRef>,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &str, message: impl Into<String>) -> Self {
        Self { status, code: code.to_string(), message: message.into(), command: None, extra: None }
    }
    /// Technical details for the "Show details" box.
    pub fn details(&self) -> Option<&str> {
        self.extra.as_ref().and_then(|x| x.details.as_deref())
    }
    /// What the user can do (see [`kora_core::problem::Action`]). Empty: the UI's generic choices.
    pub fn actions(&self) -> &[Action] {
        self.extra.as_ref().map_or(&[], |x| x.actions.as_slice())
    }
    /// A [`Problem`] with this HTTP status.
    pub fn problem(status: StatusCode, p: Problem) -> Self {
        let mut e = Self::new(status, &p.code, p.message);
        if p.details.is_some() || !p.actions.is_empty() || p.i18n.is_some() {
            e.extra = Some(Box::new(Extra { details: p.details, actions: p.actions, i18n: p.i18n }));
        }
        if e.code == codes::NEEDS_ELEVATION {
            e.command = e.actions().iter().find_map(|a| match a {
                Action::ShowCommand { command, .. } => Some(command.clone()),
                _ => None,
            });
        }
        e
    }
    /// The error as a [`Problem`] (for job errors and tests).
    pub fn to_problem(&self) -> Problem {
        Problem {
            code: self.code.clone(),
            message: self.message.clone(),
            details: self.details().map(String::from),
            actions: self.actions().to_vec(),
            i18n: self.extra.as_ref().and_then(|x| x.i18n.clone()),
        }
    }
    pub fn with_action(mut self, a: Action) -> Self {
        let x = self.extra.get_or_insert_with(Box::default);
        if !x.actions.contains(&a) {
            x.actions.push(a);
        }
        self
    }
    pub fn bad_request(m: impl Into<Problem>) -> Self {
        Self::plain_or_problem(StatusCode::BAD_REQUEST, "bad_request", m.into())
    }
    pub fn unauthorized(m: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", m)
    }
    pub fn forbidden(m: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", m)
    }
    pub fn not_found(m: impl Into<Problem>) -> Self {
        Self::plain_or_problem(StatusCode::NOT_FOUND, "not_found", m.into())
    }
    pub fn conflict(m: impl Into<Problem>) -> Self {
        Self::plain_or_problem(StatusCode::CONFLICT, "conflict", m.into())
    }
    /// Status 500. A plain message (or a generic [`Problem`]) gets the code `internal`; a Problem that has a
    /// code of its own keeps it, with its details and actions.
    pub fn internal(m: impl Into<Problem>) -> Self {
        Self::plain_or_problem(StatusCode::INTERNAL_SERVER_ERROR, "internal", m.into())
    }
    /// A plain sentence (a generic Problem with no details and no actions) gets this status and code;
    /// a Problem that says more keeps its own code, details and actions under the same status.
    fn plain_or_problem(status: StatusCode, code: &str, p: Problem) -> Self {
        if p.code == codes::FAILED && p.details.is_none() && p.actions.is_empty() {
            Self::new(status, code, p.message)
        } else {
            Self::problem(status, p)
        }
    }
    /// Administrator rights are needed. Offers to install the administrator service ([`Action::InstallAdminService`])
    /// and the command to run as administrator; `command` stays for older pages.
    pub fn needs_elevation(m: impl Into<String>, command: Vec<String>) -> Self {
        Self::problem(StatusCode::FORBIDDEN, elevation_problem(m, command))
    }
    /// 501: the route exists but this build cannot serve it yet.
    pub fn not_implemented(op: kora_core::api::Op) -> Self {
        Self::new(StatusCode::NOT_IMPLEMENTED, "not_implemented", format!("'{}' is not available in this version", op.name()))
    }
    /// Map a core error to an API error. A Problem with a code of its own, details or actions keeps
    /// them (422, 403 for `needs_elevation`). A plain sentence (code `failed`, nothing else) answers
    /// as `bad_request`, or `not_found` (404) when it says a server or thing is not found: the one
    /// place that still reads such a sentence, kept so the older answers do not change.
    pub fn core(e: impl Into<Problem>) -> Self {
        let p: Problem = e.into();
        if p.code != codes::FAILED || p.details.is_some() || !p.actions.is_empty() {
            return p.into();
        }
        let m = p.message;
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
    Problem::new(codes::NEEDS_ELEVATION, m).with_action(Action::InstallAdminService).with_action(Action::ShowCommand { command, elevated: true })
}

impl From<Problem> for ApiError {
    /// 422, or 403 for `needs_elevation`.
    fn from(p: Problem) -> Self {
        let status = if p.code == codes::NEEDS_ELEVATION { StatusCode::FORBIDDEN } else { StatusCode::UNPROCESSABLE_ENTITY };
        Self::problem(status, p)
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
        if let Some(d) = self.details() {
            err["details"] = json!(d);
        }
        if !self.actions().is_empty() {
            err["actions"] = json!(self.actions());
        }
        if let Some(m) = self.extra.as_ref().and_then(|x| x.i18n.as_ref()) {
            err["i18n"] = json!(m);
        }
        (self.status, Json(json!({ "error": err }))).into_response()
    }
}

/// A store [`Problem`] as one plain line, for the sign-in and certificate settings code, which still
/// reports its failures as plain sentences (`Result<_, String>`) that the handlers show as they are.
pub fn store_text(p: Problem) -> String {
    p.one_line()
}

/// Run blocking work off the async runtime; a panic becomes an `internal` error.
pub async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, ApiError> + Send + 'static) -> Result<T, ApiError> {
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
        let tmp = tempfile::tempdir().unwrap();
        let p = kora_core::installer::ServerFile::load(tmp.path()).unwrap_err();
        let e = ApiError::core(p);
        assert_eq!(e.code, codes::SERVER_NOT_KORA);
        assert!(e.message.contains("Kora did not install it"));
    }

    async fn body(e: ApiError) -> (StatusCode, serde_json::Value) {
        let r = e.into_response();
        let status = r.status();
        let b = axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap();
        (status, serde_json::from_slice(&b).unwrap())
    }

    #[tokio::test]
    async fn a_problem_reaches_the_browser_whole() {
        let p = Problem::new(codes::GH_MISSING, "The GitHub CLI is not installed.").with_details("gh: not found").with_action(Action::InstallTool { tool: kora_core::problem::Tool::Gh });
        // As a Backend method returns it.
        let (status, v) = body(ApiError::core(p.clone())).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(v["error"], p.to_json());
        assert_eq!(serde_json::from_value::<Problem>(v["error"].clone()).unwrap(), p);
        // A plain sentence answers exactly as before (no details, no actions).
        let (status, v) = body(ApiError::core("the port is busy")).await;
        assert_eq!((status, v), (StatusCode::UNPROCESSABLE_ENTITY, json!({"error": {"code": "bad_request", "message": "the port is busy"}})));
    }

    #[tokio::test]
    async fn needs_elevation_keeps_its_command_and_offers_actions() {
        let (status, v) = body(ApiError::needs_elevation("Needs admin.", vec!["kora".into(), "x".into()])).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(v["error"]["code"], "needs_elevation");
        assert_eq!(v["error"]["command"], json!(["kora", "x"]));
        assert_eq!(v["error"]["actions"], json!([{"kind": "install_admin_service"}, {"kind": "show_command", "command": ["kora", "x"], "elevated": true}]));
        // An elevation problem from the core is a 403 with the same shape.
        let e = ApiError::core(elevation_problem("Needs admin.", vec!["kora".into(), "x".into()]));
        assert_eq!((e.status, e.command.clone()), (StatusCode::FORBIDDEN, Some(vec!["kora".to_string(), "x".to_string()])));
        assert_eq!(e.to_problem().code, "needs_elevation");
    }

    #[tokio::test]
    async fn a_message_id_reaches_the_browser_and_is_left_out_when_there_is_none() {
        let with = Problem::new(codes::INSTALL_DB_STOPPED, "The database server MariaDB is installed but not running.").with_message_id("errors.problems.dbStopped", &[("name", "MariaDB")]);
        let (_, v) = body(ApiError::from(with.clone())).await;
        assert_eq!(v["error"]["i18n"], json!({"id": "errors.problems.dbStopped", "params": {"name": "MariaDB"}}));
        assert_eq!(ApiError::from(with.clone()).to_problem(), with);
        let (_, v) = body(ApiError::from(Problem::new(codes::GH_MISSING, "x"))).await;
        assert!(v["error"].get("i18n").is_none());
    }
}
