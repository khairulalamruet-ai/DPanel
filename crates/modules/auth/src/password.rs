//! Changing the signed-in account's password.
//!
//! | Method | Path | Effect |
//! |---|---|---|
//! | PUT | `/api/v1/auth/password` | check the current password, set a new one, end the other sessions |
//!
//! The current password has to be given (a stolen session cannot change it), a wrong one counts toward
//! the account lockout, the new one has to pass the password policy, and every other session of the
//! account ends. The administrator `root` has the server's own password: it is changed on the server
//! (`passwd`), not here.

use axum::{Json, Router, extract::State, routing::put};
use builtpanel_audit::{AuditEntry, Outcome};
use builtpanel_auth_core::policy::PasswordPolicy;
use builtpanel_database::accounts;
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_session::{Authenticated, RevokeReason};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{AuthState, MAX_PASSWORD_BYTES, mfa::require_password};

pub(crate) fn router() -> Router<AuthState> {
    Router::new().route("/api/v1/auth/password", put(change))
}

/// The old and the new password.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangePassword {
    /// The password in use now.
    #[schema(format = Password)]
    pub current_password: String,
    /// The password to use from now on.
    #[schema(format = Password)]
    pub new_password: String,
}

impl std::fmt::Debug for ChangePassword {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChangePassword").finish_non_exhaustive()
    }
}

/// What the change did.
#[derive(Debug, Serialize, ToSchema)]
pub struct PasswordChanged {
    /// Other sessions of the account that were ended.
    pub other_sessions_ended: u64,
}

/// Change the signed-in account's password.
#[utoipa::path(
    put,
    path = "/api/v1/auth/password",
    tag = "auth",
    operation_id = "changePassword",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = ChangePassword,
    responses(
        (status = 200, description = "Changed; the other sessions ended", body = PasswordChanged),
        (status = 403, description = "`password_incorrect` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`system_account`: the server's root has the server's own password", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "The new password breaks the rules (`password_too_short`, `password_too_weak`, `password_unchanged`, ...)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 429, description = "Too many failed attempts", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn change(
    State(state): State<AuthState>,
    Authenticated(session): Authenticated,
    Json(req): Json<ChangePassword>,
) -> Result<Json<PasswordChanged>, AppError> {
    let account_id = session.account_id;
    let account = accounts::find_for_reauth(&state.db, account_id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::Unauthorized)?;
    if account.password_hash == accounts::SYSTEM_ROOT_PASSWORD {
        return Err(AppError::conflict(
            "system_account",
            "This account uses the server's own password; change it on the server with `passwd`.",
        ));
    }
    if req.new_password.len() > MAX_PASSWORD_BYTES {
        return Err(AppError::validation(vec![FieldError::new(
            "new_password",
            "password_too_long",
            "The password is too long.",
        )]));
    }
    // Prove the current password first: a wrong one counts toward the lockout.
    let same = req.current_password == req.new_password;
    require_password(&state, account_id, req.current_password).await?;

    let profile = accounts::find_profile(&state.db, account_id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::Unauthorized)?;
    let password = SecretString::from(req.new_password);
    let mut errors = Vec::new();
    if same {
        errors.push(FieldError::new(
            "new_password",
            "password_unchanged",
            "The new password must differ from the current one.",
        ));
    }
    if let Err(violations) =
        PasswordPolicy::default().check(&password, &profile.username, &profile.email)
    {
        errors.extend(
            violations
                .iter()
                .map(|v| FieldError::new("new_password", v.code(), v.message())),
        );
    }
    if !errors.is_empty() {
        return Err(AppError::validation(errors));
    }

    let hash = state
        .hasher
        .hash_async(password)
        .await
        .map_err(AppError::internal)?;
    accounts::update_password_hash(&state.db, account_id, &hash)
        .await
        .map_err(AppError::internal)?;
    // Whoever knew the old password (or held a session made with it) is signed out; this session stays.
    let ended = state
        .sessions
        .revoke_all(account_id, Some(session.id), RevokeReason::PasswordChanged)
        .await
        .map_err(AppError::internal)?;
    state
        .audit
        .record_or_log(
            AuditEntry::new("auth.password_changed", Outcome::Success)
                .actor(account_id)
                .metadata(serde_json::json!({ "other_sessions_ended": ended })),
        )
        .await;
    Ok(Json(PasswordChanged {
        other_sessions_ended: ended,
    }))
}
