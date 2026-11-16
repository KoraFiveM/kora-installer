//! API error type with the JSON shape from the plan (section 2.2).
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    /// Only for `needs_elevation`: the `kora ...` command the user must run as admin/root.
    pub command: Option<Vec<String>>,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            command: None,
        }
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
    pub fn needs_elevation(m: impl Into<String>, command: Vec<String>) -> Self {
        let mut e = Self::new(StatusCode::FORBIDDEN, "needs_elevation", m);
        e.command = Some(command);
        e
    }
    /// 501: the route exists but this build cannot serve it yet.
    pub fn not_implemented(op: kora_core::api::Op) -> Self {
        Self::new(StatusCode::NOT_IMPLEMENTED, "not_implemented", format!("'{}' is not available in this version", op.name()))
    }
    /// Map a core error string (`Res<T>`) to an API error. Core messages are written for people.
    pub fn core(m: impl Into<String>) -> Self {
        let m = friendly(m.into());
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
}
