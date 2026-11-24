//! Application errors rendered as RFC 9457 Problem Details.
//!
//! Handlers return `Result<T, AppError>`. Each variant maps to one HTTP status
//! and a stable machine-readable `code` the frontend can switch on. Internal
//! errors are logged with their cause but answered with a generic message.

use std::borrow::Cow;

use axum::{
    Json,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;

/// Media type for problem details responses.
pub const PROBLEM_JSON: &str = "application/problem+json";
/// Base URI for problem `type` values (documentation anchor, not fetched).
pub const PROBLEM_TYPE_BASE: &str = "https://docs.builtpanel.dev/problems/";

/// One invalid input field (used with 422).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct FieldError {
    /// JSON pointer-ish field path, e.g. `email` or `limits.max_domains`.
    pub field: String,
    /// Stable machine code, e.g. `required`, `too_long`, `invalid_format`.
    pub code: String,
    /// Human-readable message (English; the UI translates by `code`).
    pub message: String,
}

impl FieldError {
    /// Convenience constructor.
    pub fn new(
        field: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            field: field.into(),
            code: code.into(),
            message: message.into(),
        }
    }
}

/// Error type returned by handlers.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// 400 — malformed request (bad JSON, bad query string).
    #[error("bad request: {detail}")]
    BadRequest {
        /// Stable code.
        code: Cow<'static, str>,
        /// Explanation safe to show to the client.
        detail: String,
    },
    /// 401 — not authenticated.
    #[error("unauthorized")]
    Unauthorized,
    /// 401 — an authentication attempt failed (e.g. `invalid_credentials`).
    #[error("authentication failed: {code}")]
    AuthenticationFailed {
        /// Stable code.
        code: Cow<'static, str>,
        /// Explanation safe to show to the client.
        detail: String,
    },
    /// 403 — authenticated but not allowed. `code` distinguishes e.g.
    /// `permission_denied` from `feature_disabled`.
    #[error("forbidden: {code}")]
    Forbidden {
        /// Stable code.
        code: Cow<'static, str>,
        /// Explanation safe to show to the client.
        detail: String,
    },
    /// 404 — resource does not exist (or the caller may not know it exists).
    #[error("not found: {resource}")]
    NotFound {
        /// Resource kind, e.g. `domain`.
        resource: Cow<'static, str>,
    },
    /// 409 — state conflict, e.g. duplicate name or `limit_reached`.
    #[error("conflict: {code}")]
    Conflict {
        /// Stable code.
        code: Cow<'static, str>,
        /// Explanation safe to show to the client.
        detail: String,
    },
    /// 413 — what was sent (or asked for) is bigger than allowed.
    #[error("payload too large: {code}")]
    PayloadTooLarge {
        /// Stable code, e.g. `too_large`.
        code: Cow<'static, str>,
        /// Explanation safe to show to the client.
        detail: String,
    },
    /// 422 — well-formed but invalid input.
    #[error("validation failed ({} field errors)", errors.len())]
    Validation {
        /// Per-field problems.
        errors: Vec<FieldError>,
    },
    /// 429 — too many attempts. Sent with `Retry-After` (whole seconds).
    #[error("too many requests: {code}")]
    TooManyRequests {
        /// Stable code, e.g. `too_many_attempts`.
        code: Cow<'static, str>,
        /// Explanation safe to show to the client.
        detail: String,
        /// Seconds until the client may retry (at least 1).
        retry_after_secs: u64,
    },
    /// 503 — a feature is switched off or not set up (e.g. no node CA yet).
    #[error("unavailable: {code}")]
    Unavailable {
        /// Stable code, e.g. `enrollment_unavailable`.
        code: Cow<'static, str>,
        /// Explanation safe to show to the client.
        detail: String,
    },
    /// 500 — unexpected failure. The source is logged, never sent.
    #[error("internal error: {0}")]
    Internal(#[source] Box<dyn std::error::Error + Send + Sync + 'static>),
}

impl AppError {
    /// 400 with a code and client-safe detail.
    pub fn bad_request(code: &'static str, detail: impl Into<String>) -> Self {
        Self::BadRequest {
            code: code.into(),
            detail: detail.into(),
        }
    }
    /// 401 for a failed authentication attempt.
    pub fn authentication_failed(code: &'static str, detail: impl Into<String>) -> Self {
        Self::AuthenticationFailed {
            code: code.into(),
            detail: detail.into(),
        }
    }
    /// 403 with a code and client-safe detail.
    pub fn forbidden(code: &'static str, detail: impl Into<String>) -> Self {
        Self::Forbidden {
            code: code.into(),
            detail: detail.into(),
        }
    }
    /// 404 for a resource kind.
    pub fn not_found(resource: &'static str) -> Self {
        Self::NotFound {
            resource: resource.into(),
        }
    }
    /// 413 with a code and client-safe detail.
    pub fn payload_too_large(code: &'static str, detail: impl Into<String>) -> Self {
        Self::PayloadTooLarge {
            code: code.into(),
            detail: detail.into(),
        }
    }

    /// 409 with a code and client-safe detail.
    pub fn conflict(code: &'static str, detail: impl Into<String>) -> Self {
        Self::Conflict {
            code: code.into(),
            detail: detail.into(),
        }
    }
    /// 422 from field errors.
    #[must_use]
    pub fn validation(errors: Vec<FieldError>) -> Self {
        Self::Validation { errors }
    }
    /// 429 with a code, client-safe detail and retry delay (rounded up to whole seconds).
    pub fn too_many_requests(
        code: &'static str,
        detail: impl Into<String>,
        retry_after: std::time::Duration,
    ) -> Self {
        let secs = retry_after.as_secs() + u64::from(retry_after.subsec_nanos() > 0);
        Self::TooManyRequests {
            code: code.into(),
            detail: detail.into(),
            retry_after_secs: secs.max(1),
        }
    }
    /// 503 with a code and client-safe detail.
    pub fn unavailable(code: &'static str, detail: impl Into<String>) -> Self {
        Self::Unavailable {
            code: code.into(),
            detail: detail.into(),
        }
    }
    /// 500 wrapping any error.
    pub fn internal(err: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>) -> Self {
        Self::Internal(err.into())
    }

    /// HTTP status for this error.
    #[must_use]
    pub fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest { .. } => StatusCode::BAD_REQUEST,
            Self::Unauthorized | Self::AuthenticationFailed { .. } => StatusCode::UNAUTHORIZED,
            Self::Forbidden { .. } => StatusCode::FORBIDDEN,
            Self::NotFound { .. } => StatusCode::NOT_FOUND,
            Self::Conflict { .. } => StatusCode::CONFLICT,
            Self::PayloadTooLarge { .. } => StatusCode::PAYLOAD_TOO_LARGE,
            Self::Validation { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            Self::TooManyRequests { .. } => StatusCode::TOO_MANY_REQUESTS,
            Self::Unavailable { .. } => StatusCode::SERVICE_UNAVAILABLE,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Stable machine-readable code.
    #[must_use]
    pub fn code(&self) -> &str {
        match self {
            Self::BadRequest { code, .. }
            | Self::AuthenticationFailed { code, .. }
            | Self::Forbidden { code, .. }
            | Self::Conflict { code, .. }
            | Self::PayloadTooLarge { code, .. }
            | Self::TooManyRequests { code, .. }
            | Self::Unavailable { code, .. } => code,
            Self::Unauthorized => "unauthenticated",
            Self::NotFound { .. } => "not_found",
            Self::Validation { .. } => "validation_failed",
            Self::Internal(_) => "internal_error",
        }
    }

    fn detail(&self) -> String {
        match self {
            Self::BadRequest { detail, .. }
            | Self::AuthenticationFailed { detail, .. }
            | Self::Forbidden { detail, .. }
            | Self::Conflict { detail, .. }
            | Self::PayloadTooLarge { detail, .. }
            | Self::TooManyRequests { detail, .. }
            | Self::Unavailable { detail, .. } => detail.clone(),
            Self::Unauthorized => "Authentication is required.".to_owned(),
            Self::NotFound { resource } => format!("The requested {resource} was not found."),
            Self::Validation { .. } => "One or more fields are invalid.".to_owned(),
            Self::Internal(_) => {
                "An unexpected error occurred. Quote the correlation_id when contacting support."
                    .to_owned()
            }
        }
    }
}

/// Wire format (RFC 9457 + Built Panel extensions `code`, `correlation_id`, `errors`).
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ProblemDetails {
    /// Problem type URI.
    #[serde(rename = "type")]
    pub type_uri: String,
    /// Short summary of the status.
    pub title: String,
    /// HTTP status code.
    pub status: u16,
    /// Human-readable explanation.
    pub detail: String,
    /// Stable machine-readable code.
    pub code: String,
    /// Request correlation ID, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    /// Field errors (422 only).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<FieldError>,
}

impl From<&AppError> for ProblemDetails {
    fn from(err: &AppError) -> Self {
        let status = err.status();
        let code = err.code().to_owned();
        Self {
            type_uri: format!("{PROBLEM_TYPE_BASE}{code}"),
            title: status.canonical_reason().unwrap_or("Error").to_owned(),
            status: status.as_u16(),
            detail: err.detail(),
            code,
            correlation_id: crate::request_id::current(),
            errors: match err {
                AppError::Validation { errors } => errors.clone(),
                _ => Vec::new(),
            },
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        if let Self::Internal(source) = &self {
            tracing::error!(error = %source, error.debug = ?source, "internal error");
        } else {
            tracing::debug!(
                status = self.status().as_u16(),
                code = self.code(),
                "request rejected"
            );
        }
        let body = ProblemDetails::from(&self);
        let retry_after = match &self {
            Self::TooManyRequests {
                retry_after_secs, ..
            } => Some(*retry_after_secs),
            _ => None,
        };
        let mut response = (self.status(), Json(body)).into_response();
        response
            .headers_mut()
            .insert(header::CONTENT_TYPE, HeaderValue::from_static(PROBLEM_JSON));
        if let Some(secs) = retry_after {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(secs));
        }
        response
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use axum::{
        Router,
        body::Body,
        extract::{Path, Request},
        routing::get,
    };
    use serde_json::Value;
    use tower::ServiceExt;

    use super::*;

    async fn fail(Path(kind): Path<String>) -> Result<&'static str, AppError> {
        Err(match kind.as_str() {
            "400" => AppError::bad_request("malformed_json", "Body is not valid JSON."),
            "401" => AppError::Unauthorized,
            "401b" => AppError::authentication_failed(
                "invalid_credentials",
                "Invalid username or password.",
            ),
            "403" => AppError::forbidden("feature_disabled", "Your plan does not include DNS."),
            "404" => AppError::not_found("domain"),
            "409" => AppError::conflict("limit_reached", "Domain limit (10) reached."),
            "503" => AppError::unavailable("enrollment_unavailable", "Not set up."),
            "429" => AppError::too_many_requests(
                "too_many_attempts",
                "Too many attempts.",
                std::time::Duration::from_millis(89_001),
            ),
            "422" => AppError::validation(vec![FieldError::new(
                "email",
                "invalid_format",
                "Not an email address.",
            )]),
            _ => AppError::internal(std::io::Error::other("db password=hunter2 refused")),
        })
    }

    async fn call(kind: &str) -> (StatusCode, String, Value) {
        let app = Router::new()
            .route("/fail/{kind}", get(fail))
            .layer(axum::middleware::from_fn(
                crate::request_id::correlation_id_layer,
            ));
        let res = app
            .oneshot(
                Request::builder()
                    .uri(format!("/fail/{kind}"))
                    .header("x-request-id", "test-corr-1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = res.status();
        let ct = res.headers()[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .to_owned();
        let bytes = axum::body::to_bytes(res.into_body(), 64 * 1024)
            .await
            .unwrap();
        (status, ct, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn every_status_renders_problem_json() {
        for (kind, status, code) in [
            ("400", 400, "malformed_json"),
            ("401", 401, "unauthenticated"),
            ("401b", 401, "invalid_credentials"),
            ("403", 403, "feature_disabled"),
            ("404", 404, "not_found"),
            ("409", 409, "limit_reached"),
            ("422", 422, "validation_failed"),
            ("429", 429, "too_many_attempts"),
            ("503", 503, "enrollment_unavailable"),
            ("500", 500, "internal_error"),
        ] {
            let (got, ct, body) = call(kind).await;
            assert_eq!(got.as_u16(), status, "{kind}");
            assert_eq!(ct, PROBLEM_JSON, "{kind}");
            assert_eq!(body["status"], status, "{kind}");
            assert_eq!(body["code"], code, "{kind}");
            assert_eq!(body["type"], format!("{PROBLEM_TYPE_BASE}{code}"), "{kind}");
            assert_eq!(body["correlation_id"], "test-corr-1", "{kind}");
            assert!(
                body["title"].is_string() && body["detail"].is_string(),
                "{kind}"
            );
        }
    }

    #[tokio::test]
    async fn too_many_requests_sets_retry_after_rounded_up() {
        let app = Router::new().route("/fail/{kind}", get(fail));
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/fail/429")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(res.headers()[header::RETRY_AFTER], "90");
        let (_, _, body) = call("409").await;
        assert!(body.get("retry_after_secs").is_none());
    }

    #[test]
    fn retry_after_is_never_zero() {
        let err = AppError::too_many_requests("x", "y", std::time::Duration::ZERO);
        assert!(matches!(
            err,
            AppError::TooManyRequests {
                retry_after_secs: 1,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn validation_lists_field_errors() {
        let (_, _, body) = call("422").await;
        assert_eq!(body["errors"][0]["field"], "email");
        assert_eq!(body["errors"][0]["code"], "invalid_format");
    }

    #[tokio::test]
    async fn non_validation_errors_omit_errors_array() {
        let (_, _, body) = call("409").await;
        assert!(body.get("errors").is_none());
    }

    #[tokio::test]
    async fn internal_error_does_not_leak_its_cause() {
        let (_, _, body) = call("500").await;
        let text = body.to_string();
        assert!(!text.contains("hunter2"), "{text}");
        assert!(!text.contains("db password"), "{text}");
    }
}
