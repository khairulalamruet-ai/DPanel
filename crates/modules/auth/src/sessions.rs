//! The signed-in account's sessions: list them and end them.
//!
//! | Method | Path | Effect |
//! |---|---|---|
//! | GET    | `/api/v1/auth/sessions` | live sessions, newest first |
//! | DELETE | `/api/v1/auth/sessions/{id}` | end one session |
//! | POST   | `/api/v1/auth/sessions/revoke-others` | end all but the current one |
//! | POST   | `/api/v1/auth/logout-all` | end all, including the current one |
//!
//! An account only ever sees or ends its own sessions; another account's
//! session id answers 404, the same as an id that does not exist. Revocation
//! removes the session from Redis, so the revoked cookie fails on its very
//! next request.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use builtpanel_audit::{AuditEntry, Outcome};
use builtpanel_http_kit::{AppError, ProblemDetails};
use builtpanel_session::{Authenticated, RevokeReason, SessionInfo, clear_cookie, rfc3339};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::AuthState;

pub(crate) fn router() -> Router<AuthState> {
    Router::new()
        .route("/api/v1/auth/sessions", get(list))
        .route("/api/v1/auth/sessions/{id}", delete(revoke))
        .route("/api/v1/auth/sessions/revoke-others", post(revoke_others))
        .route("/api/v1/auth/logout-all", post(logout_all))
}

/// A live session of the signed-in account.
#[derive(Debug, Serialize, ToSchema)]
pub struct SessionView {
    /// Session id.
    pub id: Uuid,
    /// Whether this is the session making the request.
    pub current: bool,
    /// IP address at login.
    pub ip: Option<String>,
    /// Browser user agent at login.
    pub user_agent: Option<String>,
    /// When the session was created (RFC 3339, UTC).
    #[schema(format = DateTime, example = "2026-11-11T09:30:00Z")]
    pub created_at: String,
    /// Last recorded activity (RFC 3339, UTC; up to a minute behind).
    #[schema(format = DateTime)]
    pub last_seen_at: String,
    /// When the session ends regardless of activity (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub expires_at: String,
}

impl SessionView {
    fn new(s: SessionInfo, current_id: Uuid) -> Self {
        Self {
            id: s.id,
            current: s.id == current_id,
            ip: s.ip,
            user_agent: s.user_agent,
            created_at: rfc3339(s.created_at),
            last_seen_at: rfc3339(s.last_seen_at),
            expires_at: rfc3339(s.expires_at),
        }
    }
}

/// Live sessions.
#[derive(Debug, Serialize, ToSchema)]
pub struct SessionList {
    /// Newest first; at most 100.
    pub sessions: Vec<SessionView>,
}

/// Result of a bulk revocation.
#[derive(Debug, Serialize, ToSchema)]
pub struct RevokedCount {
    /// Number of sessions ended.
    pub revoked: u64,
}

fn cleared_cookie_response() -> Result<Response, AppError> {
    let cookie = HeaderValue::from_str(&clear_cookie()).map_err(AppError::internal)?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(header::SET_COOKIE, cookie);
    Ok(response)
}

/// List the signed-in account's live sessions.
#[utoipa::path(
    get,
    path = "/api/v1/auth/sessions",
    tag = "auth",
    operation_id = "listSessions",
    security(("session" = [])),
    responses(
        (status = 200, description = "Live sessions, newest first", body = SessionList),
        (status = 401, description = "Not logged in", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn list(
    State(state): State<AuthState>,
    Authenticated(session): Authenticated,
) -> Result<Json<SessionList>, AppError> {
    let sessions = state
        .sessions
        .list_active(session.account_id)
        .await
        .map_err(AppError::internal)?
        .into_iter()
        .map(|s| SessionView::new(s, session.id))
        .collect();
    Ok(Json(SessionList { sessions }))
}

/// End one of the signed-in account's sessions. Ending the current session
/// is a logout and clears the cookie.
#[utoipa::path(
    delete,
    path = "/api/v1/auth/sessions/{id}",
    tag = "auth",
    operation_id = "revokeSession",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Session id from the list"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 204, description = "Session ended"),
        (status = 403, description = "`csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such live session for this account", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn revoke(
    State(state): State<AuthState>,
    Authenticated(session): Authenticated,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    // Parsed by hand so a malformed id is the same 404 as an unknown one.
    let id: Uuid = id.parse().map_err(|_| AppError::not_found("session"))?;
    let revoked = state
        .sessions
        .revoke_by_id(session.account_id, id, RevokeReason::Logout)
        .await
        .map_err(AppError::internal)?;
    if !revoked {
        return Err(AppError::not_found("session"));
    }
    state
        .audit
        .record_or_log(
            AuditEntry::new("auth.session_revoked", Outcome::Success)
                .actor(session.account_id)
                .target("session", id)
                .metadata(serde_json::json!({ "current": id == session.id })),
        )
        .await;
    if id == session.id {
        return cleared_cookie_response();
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// End every other session of the signed-in account ("log out everywhere else").
#[utoipa::path(
    post,
    path = "/api/v1/auth/sessions/revoke-others",
    tag = "auth",
    operation_id = "revokeOtherSessions",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses(
        (status = 200, description = "Other sessions ended; the current one stays", body = RevokedCount),
        (status = 403, description = "`csrf_failed`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn revoke_others(
    State(state): State<AuthState>,
    Authenticated(session): Authenticated,
) -> Result<Json<RevokedCount>, AppError> {
    let revoked = state
        .sessions
        .revoke_all(
            session.account_id,
            Some(session.id),
            RevokeReason::LogoutAll,
        )
        .await
        .map_err(AppError::internal)?;
    state
        .audit
        .record_or_log(
            AuditEntry::new("auth.sessions_revoked_others", Outcome::Success)
                .actor(session.account_id)
                .metadata(serde_json::json!({ "revoked": revoked })),
        )
        .await;
    Ok(Json(RevokedCount { revoked }))
}

/// End every session of the signed-in account, including the current one.
#[utoipa::path(
    post,
    path = "/api/v1/auth/logout-all",
    tag = "auth",
    operation_id = "logoutAll",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses(
        (status = 204, description = "All sessions ended; cookie cleared"),
        (status = 403, description = "`csrf_failed`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn logout_all(
    State(state): State<AuthState>,
    Authenticated(session): Authenticated,
) -> Result<Response, AppError> {
    let revoked = state
        .sessions
        .revoke_all(session.account_id, None, RevokeReason::LogoutAll)
        .await
        .map_err(AppError::internal)?;
    state
        .audit
        .record_or_log(
            AuditEntry::new("auth.logout_all", Outcome::Success)
                .actor(session.account_id)
                .metadata(serde_json::json!({ "revoked": revoked })),
        )
        .await;
    cleared_cookie_response()
}
