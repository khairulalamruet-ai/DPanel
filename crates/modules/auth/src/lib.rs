//! Auth module: login, logout and the current account.
//!
//! | Method | Path | Auth |
//! |---|---|---|
//! | POST | `/api/v1/public/auth/login` | none |
//! | POST | `/api/v1/auth/logout` | session |
//! | GET  | `/api/v1/auth/me` | session |
//! | GET, PUT | `/api/v1/auth/preferences` | session |
//!
//! Two-factor authentication and sudo mode are in [`mfa`]; session listing
//! and revocation in [`sessions`].
//!
//! Security properties:
//! * Unknown accounts and wrong passwords give the same 401 `invalid_credentials`
//!   and take the same time (dummy argon2 verification).
//! * Account status (suspended/disabled) is only revealed after a correct password.
//! * Any session presented at login is revoked first (session fixation).
//! * Every attempt is recorded in `login_events`.
//! * Oversized input is rejected before hashing (bounded CPU per request).
//! * Lockout (see [`LoginLimits`]): failed logins are counted per account
//!   (per hashed login name when no account matches, so unknown and existing
//!   names behave the same) and per client address. Reaching a threshold
//!   answers 429 `too_many_attempts` with `Retry-After` until the lock expires,
//!   even for the correct password. A successful login clears the account count.
//! * `root` logs in with the server's own root password (like WHM), checked by
//!   the `builtpanel-root-auth` helper; see [`root_login`]. The same lockout applies.
//!
//! The server must be run with `into_make_service_with_connect_info::<SocketAddr>()`
//! so the client IP is available.
//!
//! CSRF: login and `/me` return the session's `csrf_token`; the client keeps it
//! in memory and sends it as `X-CSRF-Token` on every state-changing request
//! (enforced by [`builtpanel_session::Authenticated`]). Login itself is exempt:
//! it has no session yet, accepts only `application/json` (which a cross-site
//! form cannot send without a CORS preflight) and rotates any prior session.

pub mod impersonate;
pub mod mfa;
mod password;
pub mod preferences;
pub mod root_login;
pub mod sessions;

use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use axum::{
    Json, Router,
    extract::{ConnectInfo, FromRef, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use builtpanel_audit::{AuditEntry, AuditWriter, Outcome};
use builtpanel_auth_core::{
    password::{PasswordHasher, Verification},
    secretbox::MasterKey,
};
use builtpanel_cache::{
    RedisHandle,
    lockout::{FailureLimiter, LockoutPolicy},
};
use builtpanel_database::{
    accounts::{self, AccountProfile, DbAccountKind, DbAccountStatus, LoginEvent, LoginOutcome},
    sqlx::PgPool,
};
use builtpanel_domain::{Email, Username};
use builtpanel_http_kit::{
    AppError, ProblemDetails,
    client_ip::{client_ip, rate_limit_key},
};
use builtpanel_session::{
    Authenticated, CsrfToken, NewSession, RevokeReason, SessionStore, clear_cookie, cookie_value,
    set_cookie,
};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use utoipa::{
    Modify, OpenApi, ToSchema,
    openapi::security::{ApiKey, ApiKeyValue, SecurityScheme},
};
use uuid::Uuid;

/// Longest accepted login identifier (characters).
pub const MAX_LOGIN_CHARS: usize = 254;
/// Longest accepted password at login (bytes); longer input is rejected
/// without hashing so a request cannot buy unbounded CPU.
pub const MAX_PASSWORD_BYTES: usize = 1024;

/// Dependencies of the auth handlers.
#[derive(Clone)]
pub struct AuthState {
    /// Database pool.
    pub db: PgPool,
    /// Session store.
    pub sessions: SessionStore,
    /// Password hasher with the configured cost.
    pub hasher: PasswordHasher,
    /// Failed-login lockout.
    pub limits: LoginLimits,
    /// Reverse proxies whose `X-Forwarded-For` is trusted.
    pub trusted_proxies: Arc<[IpAddr]>,
    /// Redis, for short-lived pending 2FA logins.
    pub redis: RedisHandle,
    /// Key that encrypts TOTP secrets at rest.
    pub master_key: MasterKey,
    /// Issuer name shown in authenticator apps.
    pub totp_issuer: Arc<str>,
    /// Audit log.
    pub audit: AuditWriter,
    /// The helper that checks the server's root password for the administrator `root`;
    /// `None` turns that login off.
    pub root_login: Option<root_login::RootLogin>,
}

impl FromRef<AuthState> for PgPool {
    fn from_ref(s: &AuthState) -> Self {
        s.db.clone()
    }
}

/// Failed-login counters: one per account (or login name), one per client
/// address (IPv6 per /64).
#[derive(Debug, Clone)]
pub struct LoginLimits {
    /// Per account / login name.
    pub account: FailureLimiter,
    /// Per client address.
    pub ip: FailureLimiter,
}

impl LoginLimits {
    /// Builds both limiters on one Redis handle.
    #[must_use]
    pub fn new(redis: RedisHandle, account: LockoutPolicy, ip: LockoutPolicy) -> Self {
        Self {
            account: FailureLimiter::new(redis.clone(), "login:account", account),
            ip: FailureLimiter::new(redis, "login:ip", ip),
        }
    }
}

/// Lockout subject for a login attempt: the account when one matched,
/// otherwise a hash of the normalised login name (never raw input in keys).
pub(crate) fn lockout_subject(account_id: Option<Uuid>, normalised_login: &str) -> String {
    match account_id {
        Some(id) => format!("a:{}", id.simple()),
        None => format!(
            "n:{}",
            URL_SAFE_NO_PAD.encode(Sha256::digest(normalised_login.as_bytes()))
        ),
    }
}

pub(crate) fn too_many_attempts(retry_after: Duration) -> AppError {
    AppError::too_many_requests(
        "too_many_attempts",
        "Too many failed login attempts. Try again later.",
        retry_after,
    )
}

impl FromRef<AuthState> for SessionStore {
    fn from_ref(s: &AuthState) -> Self {
        s.sessions.clone()
    }
}

/// Routes with full paths; merge into the application router.
pub fn router() -> Router<AuthState> {
    Router::new()
        .route("/api/v1/public/auth/login", post(login))
        .route("/api/v1/public/auth/impersonate", post(impersonate::redeem))
        .route("/api/v1/auth/logout", post(logout))
        .route("/api/v1/auth/me", get(me))
        .merge(mfa::router())
        .merge(sessions::router())
        .merge(preferences::router())
        .merge(password::router())
}

/// Login request body.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LoginRequest {
    /// Username or email address.
    #[schema(example = "khairul")]
    pub login: String,
    /// Password.
    #[schema(format = Password)]
    pub password: String,
}

impl std::fmt::Debug for LoginRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginRequest")
            .field("login", &self.login)
            .finish_non_exhaustive()
    }
}

/// The signed-in account and its session's CSRF token.
#[derive(Serialize, ToSchema)]
pub struct Me {
    /// Account id.
    pub id: Uuid,
    /// `admin`, `reseller` or `client`.
    #[schema(value_type = String, example = "admin")]
    pub kind: DbAccountKind,
    /// Username.
    pub username: String,
    /// Email.
    pub email: String,
    /// UI locale (`en` or `bn`).
    pub locale: String,
    /// Send as `X-CSRF-Token` on POST/PUT/PATCH/DELETE. Keep in memory only.
    pub csrf_token: String,
    /// Whether two-factor authentication (TOTP) is enabled.
    pub mfa_enabled: bool,
    /// An administrator made two-factor authentication mandatory for this account.
    pub mfa_required: bool,
    /// Two-factor authentication is mandatory but not set up yet: until it is,
    /// permission-protected endpoints answer 403 `mfa_setup_required`. The UI
    /// should send the user to the security page to enrol.
    pub mfa_setup_required: bool,
    /// Role names, e.g. `["super_admin"]`.
    pub roles: Vec<String>,
    /// Permission grants from all roles: exact keys (`admin.roles.read`),
    /// subtrees (`admin.*`) or `*` for everything. Use them to show or hide
    /// UI; the server enforces them on every request.
    pub permissions: Vec<String>,
    /// Set when an administrator opened this session as the account (login as
    /// user): the client app shows a banner naming them.
    pub impersonated_by: Option<Impersonator>,
}

/// The administrator behind an impersonated session.
#[derive(Serialize, ToSchema)]
pub struct Impersonator {
    /// Administrator's account id.
    pub id: Uuid,
    /// Administrator's username.
    pub username: String,
}

impl std::fmt::Debug for Me {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Me")
            .field("id", &self.id)
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

impl Me {
    /// Builds the response from the account's profile, session and access.
    async fn load(
        db: &PgPool,
        p: AccountProfile,
        csrf: &CsrfToken,
        impersonator: Option<(Uuid, String)>,
    ) -> Result<Self, AppError> {
        let access = builtpanel_database::rbac::load_access(db, p.id)
            .await
            .map_err(AppError::internal)?;
        let (roles, permissions, mfa_enabled, mfa_required) = access
            .map_or_else(Default::default, |a| {
                (a.roles, a.permissions, a.mfa_enabled, a.mfa_required)
            });
        Ok(Self {
            id: p.id,
            kind: p.kind,
            username: p.username,
            email: p.email,
            locale: p.locale,
            csrf_token: csrf.expose().to_owned(),
            mfa_enabled,
            mfa_required,
            mfa_setup_required: mfa_required && !mfa_enabled,
            roles,
            permissions,
            impersonated_by: impersonator.map(|(id, username)| Impersonator { id, username }),
        })
    }
}

/// JSON response that must never be cached (it carries tokens or secrets).
pub(crate) fn no_store_json<T: Serialize>(body: T) -> Response {
    let mut response = Json(body).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn invalid_credentials() -> AppError {
    AppError::authentication_failed("invalid_credentials", "Invalid username or password.")
}

pub(crate) fn user_agent(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
}

pub(crate) async fn count_failure(limiter: &FailureLimiter, subject: &str) -> Result<(), AppError> {
    let recorded = limiter
        .record_failure(subject)
        .await
        .map_err(AppError::internal)?;
    if recorded.locked {
        let policy = limiter.policy();
        tracing::warn!(
            subject,
            failures = recorded.failures,
            lockout_secs = policy.lockout.as_secs(),
            "login lockout started"
        );
    }
    Ok(())
}

/// Records a login attempt in `login_events` and, except for the
/// intermediate "password ok, second factor needed" step, in the audit log.
pub(crate) async fn record(state: &AuthState, ev: LoginEvent<'_>) -> Result<(), AppError> {
    accounts::insert_login_event(&state.db, &ev)
        .await
        .map_err(AppError::internal)?;
    if let Some(entry) = login_audit_entry(&ev) {
        state.audit.record_or_log(entry).await;
    }
    Ok(())
}

/// The audit entry for a login outcome. The name that was typed is not
/// copied: people sometimes type their password into the username field.
fn login_audit_entry(ev: &LoginEvent<'_>) -> Option<AuditEntry> {
    let (action, outcome) = match ev.outcome {
        LoginOutcome::Ok => ("auth.login", Outcome::Success),
        LoginOutcome::MfaRequired => return None,
        LoginOutcome::BadCredentials | LoginOutcome::MfaFailed => {
            ("auth.login_failed", Outcome::Failure)
        }
        LoginOutcome::AccountSuspended
        | LoginOutcome::AccountDisabled
        | LoginOutcome::RateLimited
        | LoginOutcome::LockedOut => ("auth.login_failed", Outcome::Denied),
    };
    let mut entry = AuditEntry::new(action, outcome).actor_ip(ev.ip);
    if let Some(id) = ev.account_id {
        entry = entry.actor(id);
    }
    if outcome != Outcome::Success {
        entry = entry.metadata(serde_json::json!({ "reason": ev.outcome.as_str() }));
    }
    Some(entry)
}

/// Log in with username or email and password.
#[utoipa::path(
    post,
    path = "/api/v1/public/auth/login",
    tag = "auth",
    operation_id = "login",
    request_body = LoginRequest,
    responses(
        (status = 202, description = "Password accepted but a second factor is required: a short-lived `__Host-bp_mfa` cookie is set; continue with POST /api/v1/public/auth/login/mfa", body = mfa::MfaChallenge),
        (status = 200, description = "Logged in; session cookie set", body = Me,
         headers(("set-cookie" = String, description = "__Host-bp_session=…; Path=/; Secure; HttpOnly; SameSite=Strict"))),
        (status = 401, description = "Invalid username or password", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "Correct password but account suspended or disabled", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Malformed body", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 429, description = "Too many failed attempts for this account or address", body = ProblemDetails, content_type = "application/problem+json",
         headers(("retry-after" = u64, description = "Seconds until the lock expires"))),
        (status = 503, description = "`root` with the server's root password: the root login helper is not reachable", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn login(
    State(state): State<AuthState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<LoginRequest>,
) -> Result<Response, AppError> {
    let ip = client_ip(peer.ip(), &headers, &state.trusted_proxies);
    let ip_subject = rate_limit_key(ip);
    let ua = user_agent(&headers);
    let attempted = req.login.trim();
    let password = SecretString::from(req.password);

    let event = |account_id: Option<Uuid>, outcome| LoginEvent {
        account_id,
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
        record(&state, event(None, LoginOutcome::RateLimited)).await?;
        return Err(too_many_attempts(left));
    }

    if attempted.chars().count() > MAX_LOGIN_CHARS
        || secrecy::ExposeSecret::expose_secret(&password).len() > MAX_PASSWORD_BYTES
    {
        count_failure(&state.limits.ip, &ip_subject).await?;
        record(&state, event(None, LoginOutcome::BadCredentials)).await?;
        return Err(invalid_credentials());
    }

    // `root` is a reserved name: it only ever means the server's root (see [`root_login`]).
    let as_root = attempted.eq_ignore_ascii_case("root");
    // Identifiers that cannot exist (bad format, reserved) simply match nothing.
    let (username, email) = if attempted.contains('@') {
        (None, Email::parse(attempted).ok())
    } else {
        (Username::parse(attempted).ok(), None)
    };
    let account = if as_root {
        if state.root_login.is_some() {
            accounts::find_system_root(&state.db)
                .await
                .map_err(AppError::internal)?
        } else {
            None
        }
    } else if username.is_some() || email.is_some() {
        accounts::find_for_login(&state.db, username.as_ref(), email.as_ref())
            .await
            .map_err(AppError::internal)?
    } else {
        None
    };

    let normalised = match (&username, &email) {
        _ if as_root => "root".to_owned(),
        (Some(u), _) => u.as_str().to_owned(),
        (_, Some(e)) => e.as_str().to_owned(),
        _ => attempted.to_lowercase(),
    };
    let subject = lockout_subject(account.as_ref().map(|a| a.id), &normalised);
    let account_id = account.as_ref().map(|a| a.id);
    if let Some(left) = state
        .limits
        .account
        .locked_for(&subject)
        .await
        .map_err(AppError::internal)?
    {
        record(&state, event(account_id, LoginOutcome::LockedOut)).await?;
        return Err(too_many_attempts(left));
    }

    let Some(account) = account else {
        let _ = state.hasher.verify_dummy_async(password).await;
        count_failure(&state.limits.account, &subject).await?;
        count_failure(&state.limits.ip, &ip_subject).await?;
        record(&state, event(None, LoginOutcome::BadCredentials)).await?;
        return Err(invalid_credentials());
    };

    let verification = check_password(&state, &password, &account.password_hash).await?;
    if !verification.matches {
        count_failure(&state.limits.account, &subject).await?;
        count_failure(&state.limits.ip, &ip_subject).await?;
        record(
            &state,
            event(Some(account.id), LoginOutcome::BadCredentials),
        )
        .await?;
        return Err(invalid_credentials());
    }

    match account.status {
        DbAccountStatus::Active => {}
        DbAccountStatus::Suspended => {
            record(
                &state,
                event(Some(account.id), LoginOutcome::AccountSuspended),
            )
            .await?;
            return Err(AppError::forbidden(
                "account_suspended",
                "This account is suspended.",
            ));
        }
        DbAccountStatus::Disabled => {
            record(
                &state,
                event(Some(account.id), LoginOutcome::AccountDisabled),
            )
            .await?;
            return Err(AppError::forbidden(
                "account_disabled",
                "This account is disabled.",
            ));
        }
    }

    if verification.needs_rehash {
        match state.hasher.hash_async(password).await {
            Ok(hash) => {
                if let Err(err) = accounts::update_password_hash(&state.db, account.id, &hash).await
                {
                    tracing::warn!(error = %err, "password rehash not stored");
                }
            }
            Err(err) => tracing::warn!(error = %err, "password rehash failed"),
        }
    }

    if builtpanel_database::mfa::totp_enabled(&state.db, account.id)
        .await
        .map_err(AppError::internal)?
    {
        // The failure counter is deliberately not reset here: knowing the
        // password must not buy unlimited guesses at the second factor.
        record(&state, event(Some(account.id), LoginOutcome::MfaRequired)).await?;
        return mfa::challenge(&state, account.id, attempted).await;
    }

    if let Err(err) = state.limits.account.reset(&subject).await {
        tracing::warn!(error = %err, "failed-login counter not reset");
    }
    finish_login(&state, account.id, attempted, ip, &headers).await
}

/// Checks a password against an account's stored hash, or, for the administrator `root`,
/// against the server's root password through the root helper.
///
/// # Errors
/// Internal errors; 503 `root_login_unavailable` when the root helper cannot be asked.
pub(crate) async fn check_password(
    state: &AuthState,
    password: &SecretString,
    stored: &str,
) -> Result<Verification, AppError> {
    if stored != accounts::SYSTEM_ROOT_PASSWORD {
        return state
            .hasher
            .verify_async(password.clone(), stored.to_owned())
            .await
            .map_err(AppError::internal);
    }
    let Some(helper) = &state.root_login else {
        // Turned off in the configuration: as slow as a wrong password, and as silent.
        let _ = state.hasher.verify_dummy_async(password.clone()).await;
        return Ok(Verification {
            matches: false,
            needs_rehash: false,
        });
    };
    let matches = match helper.check(password).await {
        Ok(root_login::RootCheck::Match) => true,
        Ok(root_login::RootCheck::Mismatch) => false,
        Ok(root_login::RootCheck::NoPassword) => {
            tracing::warn!(
                "root login refused: root has no password on this server (set one with `passwd`)"
            );
            false
        }
        Ok(root_login::RootCheck::UnsupportedScheme) => {
            tracing::warn!(
                "root login refused: root's password hash uses a scheme the helper does not know \
                 (set it again with `passwd` to store it as yescrypt or SHA-512)"
            );
            false
        }
        Err(err) => {
            tracing::error!(error = %err, "root login: the root helper could not be asked");
            return Err(AppError::unavailable(
                "root_login_unavailable",
                "Logging in as root is not available right now. Try again later or use another administrator.",
            ));
        }
    };
    Ok(Verification {
        matches,
        needs_rehash: false,
    })
}

/// Completes a login after every required factor has been verified: rotates
/// any previous session, creates the new one, records the event and returns
/// the account with the session cookie.
pub(crate) async fn finish_login(
    state: &AuthState,
    account_id: Uuid,
    attempted: &str,
    ip: IpAddr,
    headers: &HeaderMap,
) -> Result<Response, AppError> {
    let ua = user_agent(headers);
    // Session fixation: never keep a session that existed before authentication.
    if let Some(previous) = cookie_value(headers)
        && let Err(err) = state.sessions.revoke(previous, RevokeReason::Logout).await
    {
        tracing::warn!(error = %err, "previous session not revoked at login");
    }

    let (token, session) = state
        .sessions
        .create(NewSession {
            account_id,
            ip: Some(ip),
            user_agent: ua.map(str::to_owned),
        })
        .await
        .map_err(AppError::internal)?;
    record(
        state,
        LoginEvent {
            account_id: Some(account_id),
            attempted,
            ip,
            user_agent: ua,
            outcome: LoginOutcome::Ok,
        },
    )
    .await?;

    let profile = accounts::find_profile(&state.db, account_id)
        .await
        .map_err(AppError::internal)?
        .ok_or_else(|| {
            AppError::internal(std::io::Error::other("account vanished during login"))
        })?;
    let me = Me::load(&state.db, profile, &session.csrf_token, None).await?;

    let cookie = HeaderValue::from_str(&set_cookie(&token)).map_err(AppError::internal)?;
    let mut response = no_store_json(me);
    response.headers_mut().insert(header::SET_COOKIE, cookie);
    Ok(response)
}

/// End the current session.
#[utoipa::path(
    post,
    path = "/api/v1/auth/logout",
    tag = "auth",
    operation_id = "logout",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses(
        (status = 204, description = "Logged out; cookie cleared"),
        (status = 401, description = "Not logged in", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "Missing or wrong X-CSRF-Token (code csrf_failed)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn logout(
    State(state): State<AuthState>,
    Authenticated(session): Authenticated,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    if let Some(value) = cookie_value(&headers) {
        state
            .sessions
            .revoke(value, RevokeReason::Logout)
            .await
            .map_err(AppError::internal)?;
    }
    state
        .audit
        .record_or_log(
            AuditEntry::new("auth.logout", Outcome::Success)
                .actor(session.account_id)
                .target("session", session.id),
        )
        .await;
    let cookie = HeaderValue::from_str(&clear_cookie()).map_err(AppError::internal)?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(header::SET_COOKIE, cookie);
    Ok(response)
}

/// The signed-in account.
#[utoipa::path(
    get,
    path = "/api/v1/auth/me",
    tag = "auth",
    operation_id = "getMe",
    security(("session" = [])),
    responses(
        (status = 200, description = "Current account and the session's CSRF token", body = Me),
        (status = 401, description = "Not logged in", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn me(
    State(state): State<AuthState>,
    Authenticated(session): Authenticated,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let profile = accounts::find_profile(&state.db, session.account_id)
        .await
        .map_err(AppError::internal)?;
    match profile {
        Some(p) if p.status == DbAccountStatus::Active => {
            let impersonator = match session.impersonator {
                Some(id) => accounts::find_profile(&state.db, id)
                    .await
                    .map_err(AppError::internal)?
                    .map(|a| (a.id, a.username)),
                None => None,
            };
            Ok(no_store_json(
                Me::load(&state.db, p, &session.csrf_token, impersonator).await?,
            ))
        }
        _ => {
            // Account deleted or blocked after login: end the session.
            if let Some(value) = cookie_value(&headers) {
                let _ = state.sessions.revoke(value, RevokeReason::Security).await;
            }
            Err(AppError::Unauthorized)
        }
    }
}

struct SessionCookieScheme;

impl Modify for SessionCookieScheme {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "session",
            SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::with_description(
                builtpanel_session::COOKIE_NAME,
                "HttpOnly session cookie set by POST /api/v1/public/auth/login",
            ))),
        );
    }
}

/// OpenAPI fragment for this module (full paths; merge into the root document).
#[derive(OpenApi)]
#[openapi(
    paths(
        login, logout, me, impersonate::redeem,
        mfa::login_mfa, mfa::sudo, mfa::status, mfa::enroll_totp, mfa::confirm_totp,
        mfa::disable_totp, sessions::list, sessions::revoke, sessions::revoke_others,
        sessions::logout_all, preferences::read, preferences::replace, password::change
    ),
    components(schemas(
        LoginRequest, Me, Impersonator, impersonate::RedeemRequest, mfa::MfaChallenge, mfa::MfaLoginRequest, mfa::MfaStatus,
        mfa::SudoRequest, mfa::SudoStatus, mfa::TotpEnrollment, mfa::CodeRequest, mfa::BackupCodes, sessions::SessionView, sessions::SessionList,
        sessions::RevokedCount, preferences::Preferences, password::ChangePassword, password::PasswordChanged
    )),
    modifiers(&SessionCookieScheme),
    tags((name = "auth", description = "Login, logout, the current account and two-factor authentication"))
)]
pub struct AuthApi;

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn login_request_debug_hides_password() {
        let req = LoginRequest {
            login: "alice".into(),
            password: "hunter2-secret".into(),
        };
        let dbg = format!("{req:?}");
        assert!(dbg.contains("alice") && !dbg.contains("hunter2"), "{dbg}");
    }

    #[test]
    fn lockout_subjects_never_contain_raw_input() {
        let id = Uuid::now_v7();
        assert_eq!(
            lockout_subject(Some(id), "ignored"),
            format!("a:{}", id.simple())
        );
        let s = lockout_subject(None, "victim@example.com");
        assert!(s.starts_with("n:") && !s.contains("victim"), "{s}");
        assert_eq!(s.len(), 2 + 43);
        assert_eq!(s, lockout_subject(None, "victim@example.com"));
        assert_ne!(s, lockout_subject(None, "other@example.com"));
    }

    #[test]
    fn openapi_fragment_has_paths_and_cookie_scheme() {
        let doc = AuthApi::openapi();
        let json = doc.to_json().expect("json");
        for p in [
            "/api/v1/public/auth/login",
            "/api/v1/auth/logout",
            "/api/v1/auth/me",
        ] {
            assert!(json.contains(p), "{p}");
        }
        assert!(json.contains("__Host-bp_session"));
    }
}
