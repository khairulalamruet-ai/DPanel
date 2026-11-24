//! Login as user (T04.13), the client side: spending the one-time token an
//! administrator got from `POST /api/v1/admin/users/{id}/impersonate`.
//!
//! The token works once and for 60 seconds; spending it opens a normal session
//! for the client account that is marked with the administrator, so the client
//! app can show "you are viewing as ..." and the audit log names both. Such a
//! session never counts as recently authenticated: nothing that asks for the
//! account's own password (changing it, two-factor settings) works in it.

use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::{HeaderMap, HeaderValue, header},
    response::Response,
};
use builtpanel_audit::{AuditEntry, Outcome};
use builtpanel_database::accounts::{self, DbAccountKind, DbAccountStatus};
use builtpanel_http_kit::{
    AppError, ProblemDetails,
    client_ip::{client_ip, rate_limit_key},
};
use builtpanel_session::{NewSession, RevokeReason, cookie_value, set_cookie};
use serde::Deserialize;
use std::net::SocketAddr;
use utoipa::ToSchema;

use crate::{AuthState, Me, count_failure, no_store_json, too_many_attempts, user_agent};

/// The token from the administrator.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RedeemRequest {
    /// The one-time token.
    pub token: String,
}

impl std::fmt::Debug for RedeemRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedeemRequest").finish_non_exhaustive()
    }
}

fn invalid_token() -> AppError {
    AppError::authentication_failed(
        "invalid_token",
        "This sign-in link is invalid, has expired or was already used.",
    )
}

/// Open a session as a client with an administrator's one-time token.
#[utoipa::path(
    post,
    path = "/api/v1/public/auth/impersonate",
    tag = "auth",
    operation_id = "redeemImpersonation",
    request_body = RedeemRequest,
    responses(
        (status = 200, description = "Session cookie set; `impersonated_by` names the administrator", body = Me,
         headers(("set-cookie" = String, description = "__Host-bp_session=…; Path=/; Secure; HttpOnly; SameSite=Strict"))),
        (status = 401, description = "`invalid_token`: unknown, expired or already used", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "`account_unavailable`: the account cannot be signed in to now", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Malformed body", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 429, description = "Too many invalid tokens from this address", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn redeem(
    State(state): State<AuthState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<RedeemRequest>,
) -> Result<Response, AppError> {
    let ip = client_ip(peer.ip(), &headers, &state.trusted_proxies);
    let ip_subject = rate_limit_key(ip);
    if let Some(left) = state
        .limits
        .ip
        .locked_for(&ip_subject)
        .await
        .map_err(AppError::internal)?
    {
        return Err(too_many_attempts(left));
    }
    let Some(grant) = state
        .sessions
        .redeem_impersonation(req.token.trim())
        .await
        .map_err(AppError::internal)?
    else {
        count_failure(&state.limits.ip, &ip_subject).await?;
        return Err(invalid_token());
    };

    // Both accounts are checked now, not when the token was made: a minute is
    // long enough for either to be suspended.
    let admin = accounts::find_profile(&state.db, grant.admin_id)
        .await
        .map_err(AppError::internal)?
        .filter(|a| a.kind == DbAccountKind::Admin && a.status == DbAccountStatus::Active)
        .ok_or_else(invalid_token)?;
    let user = accounts::find_profile(&state.db, grant.user_id)
        .await
        .map_err(AppError::internal)?
        .filter(|u| u.kind == DbAccountKind::Client && u.status == DbAccountStatus::Active)
        .ok_or_else(|| {
            AppError::forbidden(
                "account_unavailable",
                "This account cannot be signed in to right now.",
            )
        })?;

    // Never keep a session that existed before this one.
    if let Some(previous) = cookie_value(&headers)
        && let Err(err) = state.sessions.revoke(previous, RevokeReason::Logout).await
    {
        tracing::warn!(error = %err, "previous session not revoked at impersonation");
    }
    let (token, session) = state
        .sessions
        .create_impersonated(
            NewSession {
                account_id: user.id,
                ip: Some(ip),
                user_agent: user_agent(&headers).map(str::to_owned),
            },
            admin.id,
        )
        .await
        .map_err(AppError::internal)?;
    // The entry names both people: who did it (actor) and as whom (target).
    state
        .audit
        .record_or_log(
            AuditEntry::new("auth.impersonation_started", Outcome::Success)
                .actor(admin.id)
                .actor_ip(ip)
                .target("user", user.id)
                .metadata(serde_json::json!({
                    "administrator": admin.username,
                    "user": user.username,
                    "session_id": session.id,
                })),
        )
        .await;

    let me = Me::load(
        &state.db,
        user,
        &session.csrf_token,
        Some((admin.id, admin.username)),
    )
    .await?;
    let cookie = HeaderValue::from_str(&set_cookie(&token)).map_err(AppError::internal)?;
    let mut response = no_store_json(me);
    response.headers_mut().insert(header::SET_COOKIE, cookie);
    Ok(response)
}
