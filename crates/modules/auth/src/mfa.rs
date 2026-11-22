//! Two-factor authentication (TOTP + backup codes).
//!
//! | Method | Path | Auth |
//! |---|---|---|
//! | POST | `/api/v1/public/auth/login/mfa` | pending-login cookie |
//! | GET  | `/api/v1/auth/mfa` | session |
//! | POST | `/api/v1/auth/sudo` | session + password (+ second factor) |
//! | POST | `/api/v1/auth/mfa/totp/enroll` | session in sudo mode |
//! | POST | `/api/v1/auth/mfa/totp/confirm` | session |
//! | POST | `/api/v1/auth/mfa/totp/disable` | session in sudo mode |
//!
//! Login with 2FA enabled is two requests. The password step answers 202 and
//! sets `__Host-bp_mfa`, an HttpOnly cookie holding a random token whose
//! SHA-256 keys a 5-minute Redis entry naming the account. No session exists
//! until the second step succeeds, so a password alone grants nothing.
//!
//! Security properties:
//! * TOTP secrets are stored encrypted (XChaCha20-Poly1305) and bound to the
//!   account id; backup codes are stored as SHA-256 hashes and are single-use.
//! * A TOTP code is accepted once: the matched time step is recorded and
//!   older-or-equal steps are refused, atomically.
//! * Wrong second-factor codes count toward the same lockout as wrong
//!   passwords, and a correct password does not reset that count.
//! * Enabling and disabling require sudo mode: the user authenticated within
//!   the last few minutes (login, or `POST /api/v1/auth/sudo` with the password
//!   and, when 2FA is on, a second factor). So a stolen session cannot switch
//!   2FA off, and neither can a stolen session plus the password.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::{
    Json, Router,
    extract::{ConnectInfo, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use builtpanel_audit::{AuditEntry, Outcome};
use builtpanel_auth_core::totp::{self, TotpSecret};
use builtpanel_database::{
    accounts::{self, DbAccountStatus, LoginEvent, LoginOutcome},
    mfa as db,
};
use builtpanel_http_kit::{
    AppError, ProblemDetails,
    client_ip::{client_ip, rate_limit_key},
};
use builtpanel_session::{Authenticated, RecentAuth, cookie_value};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    AuthState, MAX_PASSWORD_BYTES, count_failure, finish_login, lockout_subject, no_store_json,
    record, too_many_attempts, user_agent,
};

/// Cookie carrying the pending-login token between the two login steps.
pub const MFA_COOKIE_NAME: &str = "__Host-bp_mfa";
/// How long the second step may take.
pub const MFA_PENDING_TTL: Duration = Duration::from_secs(300);
const PENDING_PREFIX: &str = "builtpanel:mfa_pending:";
const TOKEN_BYTES: usize = 32;
/// Longest accepted second-factor input (bytes).
const MAX_CODE_BYTES: usize = 32;

pub(crate) fn router() -> Router<AuthState> {
    Router::new()
        .route("/api/v1/public/auth/login/mfa", post(login_mfa))
        .route("/api/v1/auth/sudo", post(sudo))
        .route("/api/v1/auth/mfa", get(status))
        .route("/api/v1/auth/mfa/totp/enroll", post(enroll_totp))
        .route("/api/v1/auth/mfa/totp/confirm", post(confirm_totp))
        .route("/api/v1/auth/mfa/totp/disable", post(disable_totp))
}

/// Body of the 202 answer to a password login when 2FA is enabled.
#[derive(Debug, Serialize, ToSchema)]
pub struct MfaChallenge {
    /// Always `true`.
    pub mfa_required: bool,
    /// Accepted second factors: `totp`, `backup_code`.
    pub methods: Vec<String>,
    /// Seconds until the pending login expires.
    pub expires_in: u64,
}

/// Second login step.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MfaLoginRequest {
    /// A 6-digit authenticator code or a backup code.
    #[schema(example = "123456")]
    pub code: String,
}

/// 2FA state of the signed-in account.
#[derive(Debug, Serialize, ToSchema)]
pub struct MfaStatus {
    /// TOTP is enabled (enrolment confirmed).
    pub totp_enabled: bool,
    /// Backup codes not yet used.
    pub backup_codes_remaining: i64,
}

/// Re-authentication to enter sudo mode.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SudoRequest {
    /// Current password.
    #[schema(format = Password)]
    pub password: String,
    /// Authenticator or backup code; required when 2FA is enabled.
    #[serde(default)]
    #[schema(example = "123456")]
    pub code: Option<String>,
}

/// Sudo mode state.
#[derive(Debug, Serialize, ToSchema)]
pub struct SudoStatus {
    /// Seconds until sensitive actions need re-authentication again.
    pub sudo_expires_in: u64,
}

/// A new, not yet confirmed TOTP secret.
#[derive(Serialize, ToSchema)]
pub struct TotpEnrollment {
    /// Base32 secret for manual entry.
    pub secret: String,
    /// `otpauth://` URI; render it as a QR code for the authenticator app.
    pub otpauth_uri: String,
}

/// A 6-digit authenticator code.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CodeRequest {
    /// Code currently shown by the authenticator app.
    #[schema(example = "123456")]
    pub code: String,
}

/// Backup codes, shown exactly once.
#[derive(Serialize, ToSchema)]
pub struct BackupCodes {
    /// Ten single-use codes; store them somewhere safe.
    pub backup_codes: Vec<String>,
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Associated data binding an encrypted TOTP secret to its account.
fn secret_context(account_id: Uuid) -> String {
    format!("account_mfa:totp:{}", account_id.simple())
}

fn pending_key(token: &str) -> Option<String> {
    let raw = URL_SAFE_NO_PAD.decode(token).ok()?;
    if raw.len() != TOKEN_BYTES {
        return None;
    }
    let mut key = String::from(PENDING_PREFIX);
    for b in Sha256::digest(&raw) {
        key.push_str(&format!("{b:02x}"));
    }
    Some(key)
}

fn mfa_cookie(value: &str, max_age: u64) -> Result<HeaderValue, AppError> {
    HeaderValue::from_str(&format!(
        "{MFA_COOKIE_NAME}={value}; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age={max_age}"
    ))
    .map_err(AppError::internal)
}

fn mfa_cookie_value(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == MFA_COOKIE_NAME)
        .map(|(_, value)| value)
}

/// Starts the second login step: stores the pending login and answers 202
/// with the pending cookie. Called by the password step.
pub(crate) async fn challenge(
    state: &AuthState,
    account_id: Uuid,
    attempted: &str,
) -> Result<Response, AppError> {
    let mut raw = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut raw).map_err(AppError::internal)?;
    let token = URL_SAFE_NO_PAD.encode(raw);
    let key = pending_key(&token)
        .ok_or_else(|| AppError::internal(std::io::Error::other("pending token encoding")))?;
    // "<account id>\n<what the user typed>": the second line is only for the audit trail.
    let value = format!("{account_id}\n{attempted}");
    state
        .redis
        .set_ex(&key, &value, MFA_PENDING_TTL)
        .await
        .map_err(AppError::internal)?;

    let mut response = (
        StatusCode::ACCEPTED,
        Json(MfaChallenge {
            mfa_required: true,
            methods: vec!["totp".to_owned(), "backup_code".to_owned()],
            expires_in: MFA_PENDING_TTL.as_secs(),
        }),
    )
        .into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::SET_COOKIE,
        mfa_cookie(&token, MFA_PENDING_TTL.as_secs())?,
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

fn mfa_session_expired() -> AppError {
    AppError::authentication_failed(
        "mfa_session_expired",
        "The sign-in attempt expired. Enter your password again.",
    )
}

fn invalid_mfa_code() -> AppError {
    AppError::authentication_failed("invalid_mfa_code", "Invalid or already used code.")
}

/// Decrypts the account's confirmed TOTP secret.
fn open_secret(state: &AuthState, account_id: Uuid, blob: &[u8]) -> Result<TotpSecret, AppError> {
    let bytes = state
        .master_key
        .open(secret_context(account_id).as_bytes(), blob)
        .map_err(AppError::internal)?;
    TotpSecret::from_bytes(&bytes).map_err(AppError::internal)
}

/// Checks a second factor for a confirmed TOTP account: a 6-digit code
/// (accepted once per time step) or an unused backup code.
async fn second_factor_ok(
    state: &AuthState,
    account_id: Uuid,
    input: &str,
) -> Result<bool, AppError> {
    if input.len() > MAX_CODE_BYTES {
        return Ok(false);
    }
    let Some(factor) = db::find_totp(&state.db, account_id)
        .await
        .map_err(AppError::internal)?
        .filter(|f| f.confirmed)
    else {
        return Ok(false);
    };
    let digits: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    if digits.len() == totp::CODE_DIGITS && digits.bytes().all(|b| b.is_ascii_digit()) {
        let secret = open_secret(state, account_id, &factor.secret_enc)?;
        let Some(step) = secret.verify(&digits, unix_now()) else {
            return Ok(false);
        };
        let step = i64::try_from(step).map_err(AppError::internal)?;
        // False when this step was already used: a replayed code.
        return db::accept_totp_step(&state.db, account_id, step)
            .await
            .map_err(AppError::internal);
    }
    match totp::hash_backup_code(input) {
        Some(hash) => db::consume_backup_code(&state.db, account_id, &hash)
            .await
            .map_err(AppError::internal),
        None => Ok(false),
    }
}

/// Second login step: verify a TOTP or backup code and start the session.
#[utoipa::path(
    post,
    path = "/api/v1/public/auth/login/mfa",
    tag = "auth",
    operation_id = "loginMfa",
    request_body = MfaLoginRequest,
    responses(
        (status = 200, description = "Logged in; session cookie set, pending cookie cleared", body = crate::Me),
        (status = 401, description = "`invalid_mfa_code`, or `mfa_session_expired` when the pending login is missing or older than 5 minutes", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 429, description = "Too many failed attempts", body = ProblemDetails, content_type = "application/problem+json",
         headers(("retry-after" = u64, description = "Seconds until the lock expires")))
    )
)]
pub async fn login_mfa(
    State(state): State<AuthState>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<MfaLoginRequest>,
) -> Result<Response, AppError> {
    let ip = client_ip(peer.ip(), &headers, &state.trusted_proxies);
    let ip_subject = rate_limit_key(ip);
    let ua = user_agent(&headers);

    let key = mfa_cookie_value(&headers)
        .and_then(pending_key)
        .ok_or_else(mfa_session_expired)?;
    let pending = state
        .redis
        .get(&key)
        .await
        .map_err(AppError::internal)?
        .ok_or_else(mfa_session_expired)?;
    let (id_text, attempted) = pending.split_once('\n').unwrap_or((pending.as_str(), ""));
    let account_id: Uuid = id_text.parse().map_err(AppError::internal)?;
    let event = |outcome| LoginEvent {
        account_id: Some(account_id),
        attempted,
        ip,
        user_agent: ua,
        outcome,
    };

    if let Some(left) = state
        .limits
        .ip
        .locked_for(&ip_subject)
        .await
        .map_err(AppError::internal)?
    {
        record(&state, event(LoginOutcome::RateLimited)).await?;
        return Err(too_many_attempts(left));
    }
    let subject = lockout_subject(Some(account_id), "");
    if let Some(left) = state
        .limits
        .account
        .locked_for(&subject)
        .await
        .map_err(AppError::internal)?
    {
        record(&state, event(LoginOutcome::LockedOut)).await?;
        return Err(too_many_attempts(left));
    }

    // The account may have been suspended or deleted since the password step.
    let active = accounts::find_for_reauth(&state.db, account_id)
        .await
        .map_err(AppError::internal)?
        .is_some_and(|a| a.status == DbAccountStatus::Active);
    if !active {
        let _ = state.redis.del(&key).await;
        return Err(mfa_session_expired());
    }

    if !second_factor_ok(&state, account_id, &req.code).await? {
        count_failure(&state.limits.account, &subject).await?;
        count_failure(&state.limits.ip, &ip_subject).await?;
        record(&state, event(LoginOutcome::MfaFailed)).await?;
        return Err(invalid_mfa_code());
    }

    // Single use: only the request that removes the pending login may finish it.
    if !state.redis.del(&key).await.map_err(AppError::internal)? {
        return Err(mfa_session_expired());
    }
    if let Err(err) = state.limits.account.reset(&subject).await {
        tracing::warn!(error = %err, "failed-login counter not reset");
    }
    let mut response = finish_login(&state, account_id, attempted, ip, &headers).await?;
    response
        .headers_mut()
        .append(header::SET_COOKIE, mfa_cookie("", 0)?);
    Ok(response)
}

/// Re-checks the signed-in user's password before a sensitive change.
/// Wrong passwords count toward the account lockout.
pub(crate) async fn require_password(
    state: &AuthState,
    account_id: Uuid,
    password: String,
) -> Result<(), AppError> {
    let subject = lockout_subject(Some(account_id), "");
    if let Some(left) = state
        .limits
        .account
        .locked_for(&subject)
        .await
        .map_err(AppError::internal)?
    {
        return Err(too_many_attempts(left));
    }
    let account = accounts::find_for_reauth(&state.db, account_id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::Unauthorized)?;
    let matches = password.len() <= MAX_PASSWORD_BYTES
        && crate::check_password(state, &SecretString::from(password), &account.password_hash)
            .await?
            .matches;
    if !matches {
        count_failure(&state.limits.account, &subject).await?;
        return Err(AppError::forbidden(
            "password_incorrect",
            "The password is incorrect.",
        ));
    }
    Ok(())
}

/// Enter sudo mode: prove identity again so sensitive actions are allowed
/// for the next few minutes (`auth.sudo_minutes`).
#[utoipa::path(
    post,
    path = "/api/v1/auth/sudo",
    tag = "auth",
    operation_id = "enterSudoMode",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = SudoRequest,
    responses(
        (status = 200, description = "Sudo mode active", body = SudoStatus),
        (status = 403, description = "`password_incorrect`, `mfa_code_required` (2FA is on and no code was sent), `invalid_mfa_code` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 429, description = "Too many failed attempts", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn sudo(
    State(state): State<AuthState>,
    Authenticated(session): Authenticated,
    headers: HeaderMap,
    Json(req): Json<SudoRequest>,
) -> Result<Json<SudoStatus>, AppError> {
    let account_id = session.account_id;
    if let Err(err) = verify_sudo(&state, account_id, req).await {
        state
            .audit
            .record_or_log(
                AuditEntry::new("auth.sudo_failed", Outcome::Denied)
                    .actor(account_id)
                    .metadata(serde_json::json!({ "reason": err.code() })),
            )
            .await;
        return Err(err);
    }
    let marked = match cookie_value(&headers) {
        Some(value) => state
            .sessions
            .mark_reauthenticated(value)
            .await
            .map_err(AppError::internal)?,
        None => false,
    };
    if !marked {
        return Err(AppError::Unauthorized);
    }
    state
        .audit
        .record_or_log(AuditEntry::new("auth.sudo_entered", Outcome::Success).actor(account_id))
        .await;
    Ok(Json(SudoStatus {
        sudo_expires_in: state.sessions.config().sudo_window.as_secs(),
    }))
}

/// Checks the password and, when 2FA is on, a second factor.
async fn verify_sudo(
    state: &AuthState,
    account_id: Uuid,
    req: SudoRequest,
) -> Result<(), AppError> {
    require_password(state, account_id, req.password).await?;
    if db::totp_enabled(&state.db, account_id)
        .await
        .map_err(AppError::internal)?
    {
        let code = req.code.ok_or_else(|| {
            AppError::forbidden(
                "mfa_code_required",
                "Enter a code from your authenticator app.",
            )
        })?;
        if !second_factor_ok(state, account_id, &code).await? {
            count_failure(
                &state.limits.account,
                &lockout_subject(Some(account_id), ""),
            )
            .await?;
            return Err(AppError::forbidden(
                "invalid_mfa_code",
                "Invalid or already used code.",
            ));
        }
    }
    Ok(())
}

/// 2FA state of the signed-in account.
#[utoipa::path(
    get,
    path = "/api/v1/auth/mfa",
    tag = "auth",
    operation_id = "getMfaStatus",
    security(("session" = [])),
    responses(
        (status = 200, description = "2FA state", body = MfaStatus),
        (status = 401, description = "Not logged in", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn status(
    State(state): State<AuthState>,
    Authenticated(session): Authenticated,
) -> Result<Json<MfaStatus>, AppError> {
    let totp_enabled = db::totp_enabled(&state.db, session.account_id)
        .await
        .map_err(AppError::internal)?;
    let backup_codes_remaining = db::unused_backup_codes(&state.db, session.account_id)
        .await
        .map_err(AppError::internal)?;
    Ok(Json(MfaStatus {
        totp_enabled,
        backup_codes_remaining,
    }))
}

/// Start TOTP enrolment: returns a new secret to add to an authenticator app.
/// 2FA is not active until the secret is confirmed with a code.
#[utoipa::path(
    post,
    path = "/api/v1/auth/mfa/totp/enroll",
    tag = "auth",
    operation_id = "enrollTotp",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses(
        (status = 200, description = "New secret; calling again before confirming replaces it", body = TotpEnrollment),
        (status = 403, description = "`reauth_required` (call POST /api/v1/auth/sudo, then retry) or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`mfa_already_enabled`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn enroll_totp(
    State(state): State<AuthState>,
    RecentAuth(session): RecentAuth,
) -> Result<Response, AppError> {
    let account_id = session.account_id;
    let profile = accounts::find_profile(&state.db, account_id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::Unauthorized)?;

    let secret = TotpSecret::generate().map_err(AppError::internal)?;
    let sealed = state
        .master_key
        .seal(secret_context(account_id).as_bytes(), secret.as_bytes())
        .map_err(AppError::internal)?;
    if !db::begin_totp_enrollment(&state.db, account_id, &sealed)
        .await
        .map_err(AppError::internal)?
    {
        return Err(AppError::conflict(
            "mfa_already_enabled",
            "Two-factor authentication is already enabled.",
        ));
    }
    let otpauth_uri = secret
        .otpauth_uri(&state.totp_issuer, &profile.username)
        .map_err(AppError::internal)?;
    state
        .audit
        .record_or_log(
            AuditEntry::new("auth.mfa_enrollment_started", Outcome::Success).actor(account_id),
        )
        .await;
    Ok(no_store_json(TotpEnrollment {
        secret: secret.to_base32(),
        otpauth_uri,
    }))
}

/// Confirm TOTP enrolment with a code from the authenticator app. Enables
/// 2FA and returns the backup codes (shown only this once).
#[utoipa::path(
    post,
    path = "/api/v1/auth/mfa/totp/confirm",
    tag = "auth",
    operation_id = "confirmTotp",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = CodeRequest,
    responses(
        (status = 200, description = "2FA enabled", body = BackupCodes),
        (status = 400, description = "`invalid_mfa_code`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`mfa_not_pending`: no enrolment in progress", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn confirm_totp(
    State(state): State<AuthState>,
    Authenticated(session): Authenticated,
    Json(req): Json<CodeRequest>,
) -> Result<Response, AppError> {
    let account_id = session.account_id;
    let not_pending =
        || AppError::conflict("mfa_not_pending", "There is no 2FA enrolment to confirm.");
    let factor = db::find_totp(&state.db, account_id)
        .await
        .map_err(AppError::internal)?
        .filter(|f| !f.confirmed)
        .ok_or_else(not_pending)?;
    let secret = open_secret(&state, account_id, &factor.secret_enc)?;
    let step = (req.code.len() <= MAX_CODE_BYTES)
        .then(|| secret.verify(&req.code, unix_now()))
        .flatten()
        .ok_or_else(|| {
            AppError::bad_request(
                "invalid_mfa_code",
                "The code is not valid. Try the next one.",
            )
        })?;
    let step = i64::try_from(step).map_err(AppError::internal)?;

    let codes = totp::generate_backup_codes().map_err(AppError::internal)?;
    let hashes: Vec<[u8; 32]> = codes.iter().map(totp::BackupCode::hash).collect();
    if !db::confirm_totp(&state.db, account_id, step, &hashes)
        .await
        .map_err(AppError::internal)?
    {
        return Err(not_pending());
    }
    state
        .audit
        .record_or_log(AuditEntry::new("auth.mfa_enabled", Outcome::Success).actor(account_id))
        .await;
    Ok(no_store_json(BackupCodes {
        backup_codes: codes.iter().map(|c| c.expose().to_owned()).collect(),
    }))
}

/// Disable TOTP and delete all backup codes. Requires sudo mode.
#[utoipa::path(
    post,
    path = "/api/v1/auth/mfa/totp/disable",
    tag = "auth",
    operation_id = "disableTotp",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses(
        (status = 204, description = "2FA disabled (also when it was not enabled)"),
        (status = 403, description = "`reauth_required` (call POST /api/v1/auth/sudo, then retry) or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn disable_totp(
    State(state): State<AuthState>,
    RecentAuth(session): RecentAuth,
) -> Result<StatusCode, AppError> {
    if db::remove_totp(&state.db, session.account_id)
        .await
        .map_err(AppError::internal)?
    {
        state
            .audit
            .record_or_log(
                AuditEntry::new("auth.mfa_disabled", Outcome::Success).actor(session.account_id),
            )
            .await;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn pending_keys_hash_the_token_and_reject_malformed_ones() {
        let token = URL_SAFE_NO_PAD.encode([5u8; TOKEN_BYTES]);
        let key = pending_key(&token).unwrap();
        assert!(key.starts_with(PENDING_PREFIX));
        assert_eq!(key.len(), PENDING_PREFIX.len() + 64);
        assert!(!key.contains(&token));
        for bad in ["", "short", "!!!", &URL_SAFE_NO_PAD.encode([5u8; 31])] {
            assert!(pending_key(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn pending_cookie_is_hardened_and_found_among_others() {
        let c = mfa_cookie("tok", 300).unwrap();
        let c = c.to_str().unwrap();
        for attr in [
            "__Host-bp_mfa=tok",
            "Path=/",
            "Secure",
            "HttpOnly",
            "SameSite=Strict",
            "Max-Age=300",
        ] {
            assert!(c.contains(attr), "{c}");
        }
        let mut h = HeaderMap::new();
        h.append(
            header::COOKIE,
            HeaderValue::from_static("a=1; __Host-bp_mfa=tok; __Host-bp_session=s"),
        );
        assert_eq!(mfa_cookie_value(&h), Some("tok"));
        assert_eq!(mfa_cookie_value(&HeaderMap::new()), None);
    }

    #[test]
    fn secret_context_is_per_account() {
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        assert_ne!(secret_context(a), secret_context(b));
        assert!(secret_context(a).starts_with("account_mfa:totp:"));
    }
}
