//! Server-side sessions.
//!
//! * **Token**: 32 bytes from the OS CSPRNG, base64url-encoded, sent only in the
//!   `__Host-bp_session` cookie (`HttpOnly; Secure; SameSite=Strict; Path=/`).
//! * **Hot store**: Redis key `builtpanel:session:<sha256(token)>` holding
//!   [`SessionData`] as JSON. The raw token is never stored anywhere.
//! * **Expiry**: an idle timeout (sliding) and an absolute lifetime (fixed);
//!   the Redis TTL is always the smaller of the two remaining windows. Activity
//!   refreshes the TTL at most once per [`SessionConfig::touch_interval`].
//! * **History**: PostgreSQL `sessions` row on create (token hash, IP, UA,
//!   absolute expiry) and `revoked_at`/`revoke_reason` on revoke.
//! * **Extractor**: [`Authenticated`] yields the session or a 401 Problem.
//! * **Management**: [`SessionStore::list_active`], [`SessionStore::revoke_by_id`]
//!   and [`SessionStore::revoke_all`] let an account see and end its sessions.
//!   Revocation deletes the Redis entry, so it takes effect on the very next
//!   request.
//! * **CSRF**: every session carries a random [`CsrfToken`]. The client gets it
//!   in a response body (login, `/me`) and keeps it in memory only. For any
//!   method other than GET/HEAD/OPTIONS/TRACE, [`Authenticated`] requires the
//!   `X-CSRF-Token` header to equal it (constant-time) and otherwise rejects
//!   with 403 `csrf_failed`. Together with `SameSite=Strict` this protects all
//!   session-authenticated state changes.
//! * **Sudo mode**: a session records when its owner last proved their
//!   identity (`auth_time`: login, or [`SessionStore::mark_reauthenticated`]).
//!   The [`RecentAuth`] extractor guards sensitive routes and answers 403
//!   `reauth_required` once that is older than [`SessionConfig::sudo_window`].

use std::{
    net::IpAddr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    extract::{FromRef, FromRequestParts},
    http::{HeaderMap, Method, header, request::Parts},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use builtpanel_cache::{RedisHandle, redis::AsyncCommands as _};
use builtpanel_http_kit::AppError;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

/// Cookie name. The `__Host-` prefix makes browsers require `Secure`,
/// `Path=/` and no `Domain`, so subdomains cannot set or read it.
pub const COOKIE_NAME: &str = "__Host-bp_session";
/// Raw token length in bytes (256 bits).
pub const TOKEN_BYTES: usize = 32;
const KEY_PREFIX: &str = "builtpanel:session:";
/// Maximum stored user-agent length in characters (matches the DB CHECK).
pub const MAX_USER_AGENT: usize = 512;
/// Request header carrying the CSRF token.
pub const CSRF_HEADER: &str = "x-csrf-token";

/// Per-session anti-CSRF token (32 random bytes, base64url). Useless without
/// the session cookie; `Debug` is redacted so it never reaches logs.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct CsrfToken(String);

impl CsrfToken {
    fn generate() -> Result<Self, SessionError> {
        let mut raw = [0u8; TOKEN_BYTES];
        getrandom::fill(&mut raw).map_err(|_| SessionError::Rng)?;
        Ok(Self(URL_SAFE_NO_PAD.encode(raw)))
    }

    /// The value to hand to the client.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Constant-time comparison with a presented value. An empty stored
    /// token (sessions created before CSRF existed) never matches.
    #[must_use]
    pub fn matches(&self, presented: &[u8]) -> bool {
        let expected = self.0.as_bytes();
        if expected.is_empty() || expected.len() != presented.len() {
            return false;
        }
        expected
            .iter()
            .zip(presented)
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
    }
}

impl std::fmt::Debug for CsrfToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CsrfToken(<redacted>)")
    }
}

/// Methods that must not change state and therefore skip the CSRF check.
fn is_safe_method(method: &Method) -> bool {
    matches!(
        *method,
        Method::GET | Method::HEAD | Method::OPTIONS | Method::TRACE
    )
}

/// Session timeouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionConfig {
    /// Session ends after this long without activity.
    pub idle_timeout: Duration,
    /// Session ends this long after creation regardless of activity.
    pub absolute_timeout: Duration,
    /// Minimum time between TTL refreshes (limits Redis writes per request).
    pub touch_interval: Duration,
    /// How long after authenticating sensitive actions stay allowed (sudo mode).
    pub sudo_window: Duration,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            idle_timeout: Duration::from_secs(30 * 60),
            absolute_timeout: Duration::from_secs(12 * 60 * 60),
            touch_interval: Duration::from_secs(60),
            sudo_window: Duration::from_secs(10 * 60),
        }
    }
}

/// What is stored per session. The only secret is the CSRF token, which is
/// worthless without the session cookie (whose raw value is never stored).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionData {
    /// Session id (primary key of the `sessions` row).
    pub id: Uuid,
    /// Owner.
    pub account_id: Uuid,
    /// Unix seconds.
    pub created_at: i64,
    /// Unix seconds of the last TTL refresh.
    pub last_seen_at: i64,
    /// Unix seconds after which the session is invalid no matter what.
    pub absolute_expires_at: i64,
    /// Anti-CSRF token for state-changing requests.
    #[serde(default)]
    pub csrf_token: CsrfToken,
    /// Unix seconds when the owner last proved their identity (login or a
    /// later re-authentication). Sessions stored before sudo mode existed
    /// have 0 and therefore always need re-authentication.
    #[serde(default)]
    pub auth_time: i64,
    /// The administrator who opened this session "as" the account (login as
    /// user). Such a session never counts as recently authenticated, so it
    /// cannot do anything that asks for the account's own password.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub impersonator: Option<Uuid>,
}

impl SessionData {
    /// Seconds of sudo mode left at `now`, or `None` when re-authentication
    /// is required. A clock that moved backwards never extends the window.
    #[must_use]
    pub fn sudo_remaining(&self, now: i64, window: Duration) -> Option<u64> {
        let age = now.checked_sub(self.auth_time).filter(|age| *age >= 0)?;
        u64::try_from(secs(window) - age)
            .ok()
            .filter(|left| *left > 0)
    }
}

/// Input for [`SessionStore::create`].
#[derive(Debug, Clone)]
pub struct NewSession {
    /// Owner.
    pub account_id: Uuid,
    /// Client IP (recorded for the session list and audit).
    pub ip: Option<IpAddr>,
    /// Client user agent (truncated to 512 bytes).
    pub user_agent: Option<String>,
}

/// Why a session was ended (matches the `sessions_revoke_reason` CHECK).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevokeReason {
    /// User logged out.
    Logout,
    /// User logged out everywhere.
    LogoutAll,
    /// An administrator ended it.
    Admin,
    /// Password changed.
    PasswordChanged,
    /// Security response (e.g. suspected compromise).
    Security,
}

impl RevokeReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::Logout => "logout",
            Self::LogoutAll => "logout_all",
            Self::Admin => "admin",
            Self::PasswordChanged => "password_changed",
            Self::Security => "security",
        }
    }
}

/// Session errors. None of them contain token material.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// OS randomness unavailable.
    #[error("random number generator unavailable")]
    Rng,
    /// Redis failure.
    #[error("session store: {0}")]
    Cache(#[from] builtpanel_cache::CacheError),
    /// Redis command failure.
    #[error("session store: {0}")]
    Redis(#[from] builtpanel_cache::redis::RedisError),
    /// PostgreSQL failure.
    #[error("session history: {0}")]
    Database(#[from] sqlx::Error),
    /// Corrupt session payload in Redis.
    #[error("session payload is corrupt")]
    Corrupt,
}

/// One live session, as shown to its owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    /// Session id.
    pub id: Uuid,
    /// IP address at login.
    pub ip: Option<String>,
    /// User agent at login.
    pub user_agent: Option<String>,
    /// Unix seconds of login.
    pub created_at: i64,
    /// Unix seconds of the last recorded activity (at most
    /// [`SessionConfig::touch_interval`] behind).
    pub last_seen_at: i64,
    /// Unix seconds of the absolute expiry.
    pub expires_at: i64,
}

/// Most sessions returned by [`SessionStore::list_active`].
pub const MAX_LISTED_SESSIONS: i64 = 100;

/// Formats Unix seconds as an RFC 3339 UTC timestamp (`2026-11-11T09:30:00Z`).
#[must_use]
pub fn rfc3339(unix_seconds: i64) -> String {
    time::OffsetDateTime::from_unix_timestamp(unix_seconds)
        .ok()
        .and_then(|t| {
            t.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .unwrap_or_default()
}

fn key_from_hash_bytes(hash: &[u8]) -> Option<String> {
    <[u8; 32]>::try_from(hash).ok().map(|h| redis_key(&h))
}

/// The cookie value handed to the browser. Never logged.
pub struct SessionToken(SecretString);

impl SessionToken {
    /// The value to put in the cookie.
    #[must_use]
    pub fn expose(&self) -> &str {
        self.0.expose_secret()
    }
}

impl std::fmt::Debug for SessionToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionToken(<redacted>)")
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

fn secs(d: Duration) -> i64 {
    i64::try_from(d.as_secs()).unwrap_or(i64::MAX)
}

/// How long an impersonation token can be used after it is issued.
pub const IMPERSONATION_TTL_SECS: u64 = 60;
const IMPERSONATION_PREFIX: &str = "builtpanel:impersonation:";

/// Who may open a session as whom, held until the token is spent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImpersonationGrant {
    /// The administrator.
    pub admin_id: Uuid,
    /// The account they open a session as.
    pub user_id: Uuid,
}

fn impersonation_key(hash: &[u8; 32]) -> String {
    let mut key = String::with_capacity(IMPERSONATION_PREFIX.len() + 64);
    key.push_str(IMPERSONATION_PREFIX);
    for b in hash {
        key.push_str(&format!("{b:02x}"));
    }
    key
}

/// Decodes a cookie value; anything that is not exactly 32 base64url bytes is rejected.
fn decode_token(value: &str) -> Option<[u8; TOKEN_BYTES]> {
    if value.len() != 43 {
        return None;
    }
    let bytes = URL_SAFE_NO_PAD.decode(value).ok()?;
    bytes.try_into().ok()
}

fn token_hash(raw: &[u8; TOKEN_BYTES]) -> [u8; 32] {
    Sha256::digest(raw).into()
}

fn redis_key(hash: &[u8; 32]) -> String {
    let mut key = String::with_capacity(KEY_PREFIX.len() + 64);
    key.push_str(KEY_PREFIX);
    for b in hash {
        key.push_str(&format!("{b:02x}"));
    }
    key
}

/// `Set-Cookie` value establishing the session (browser-session lifetime;
/// expiry is enforced server-side).
#[must_use]
pub fn set_cookie(token: &SessionToken) -> String {
    format!(
        "{COOKIE_NAME}={}; Path=/; Secure; HttpOnly; SameSite=Strict",
        token.expose()
    )
}

/// `Set-Cookie` value that deletes the session cookie.
#[must_use]
pub fn clear_cookie() -> String {
    format!("{COOKIE_NAME}=; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=0")
}

/// Extracts the session cookie value from request headers.
#[must_use]
pub fn cookie_value(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == COOKIE_NAME)
        .map(|(_, value)| value)
}

/// Creates, loads and revokes sessions. Cheap to clone.
#[derive(Clone, Debug)]
pub struct SessionStore {
    redis: RedisHandle,
    db: PgPool,
    config: SessionConfig,
}

impl SessionStore {
    /// New store.
    #[must_use]
    pub fn new(redis: RedisHandle, db: PgPool, config: SessionConfig) -> Self {
        Self { redis, db, config }
    }

    /// The configured timeouts.
    #[must_use]
    pub fn config(&self) -> SessionConfig {
        self.config
    }

    /// Creates a session and returns the cookie token. The caller must send
    /// [`set_cookie`] and should discard any previous session (rotation).
    ///
    /// # Errors
    /// RNG, Redis or PostgreSQL failures.
    pub async fn create(
        &self,
        new: NewSession,
    ) -> Result<(SessionToken, SessionData), SessionError> {
        self.create_for(new, None).await
    }

    /// Creates a session for `new.account_id` on behalf of the administrator
    /// `impersonator` (login as user, T04.13).
    ///
    /// # Errors
    /// As [`SessionStore::create`].
    pub async fn create_impersonated(
        &self,
        new: NewSession,
        impersonator: Uuid,
    ) -> Result<(SessionToken, SessionData), SessionError> {
        self.create_for(new, Some(impersonator)).await
    }

    async fn create_for(
        &self,
        new: NewSession,
        impersonator: Option<Uuid>,
    ) -> Result<(SessionToken, SessionData), SessionError> {
        let mut raw = [0u8; TOKEN_BYTES];
        getrandom::fill(&mut raw).map_err(|_| SessionError::Rng)?;
        let hash = token_hash(&raw);
        let now = now_unix();
        let absolute_expires_at = now.saturating_add(secs(self.config.absolute_timeout));

        // The DB limit counts characters (PostgreSQL length()), so truncate by chars.
        let user_agent = new
            .user_agent
            .map(|ua| ua.chars().take(MAX_USER_AGENT).collect::<String>());
        let id = sqlx::query_scalar!(
            r#"INSERT INTO sessions (account_id, token_sha256, ip, user_agent, expires_at, impersonator_id)
               VALUES ($1, $2, $3::text::inet, $4, to_timestamp($5::bigint), $6)
               RETURNING id"#,
            new.account_id,
            &hash[..],
            new.ip.map(|ip| ip.to_string()),
            user_agent,
            absolute_expires_at,
            impersonator,
        )
        .fetch_one(&self.db)
        .await?;

        let data = SessionData {
            id,
            account_id: new.account_id,
            created_at: now,
            last_seen_at: now,
            absolute_expires_at,
            csrf_token: CsrfToken::generate()?,
            // An impersonated session has not proven anything to the account.
            auth_time: if impersonator.is_some() { 0 } else { now },
            impersonator,
        };
        let payload = serde_json::to_string(&data).map_err(|_| SessionError::Corrupt)?;
        let ttl = self.ttl_from(now, absolute_expires_at);
        let mut conn = self.redis.connection().await?;
        let () = conn.set_ex(redis_key(&hash), payload, ttl).await?;

        Ok((
            SessionToken(SecretString::from(URL_SAFE_NO_PAD.encode(raw))),
            data,
        ))
    }

    /// Makes a one-time token with which the administrator `admin_id` can open a
    /// session as `user_id` within [`IMPERSONATION_TTL_SECS`]. Only a hash of
    /// the token is kept; it works once.
    ///
    /// # Errors
    /// Redis failures.
    pub async fn issue_impersonation(
        &self,
        admin_id: Uuid,
        user_id: Uuid,
    ) -> Result<SecretString, SessionError> {
        let mut raw = [0u8; TOKEN_BYTES];
        getrandom::fill(&mut raw).map_err(|_| SessionError::Rng)?;
        let payload = serde_json::to_string(&ImpersonationGrant { admin_id, user_id })
            .map_err(|_| SessionError::Corrupt)?;
        let mut conn = self.redis.connection().await?;
        let () = conn
            .set_ex(
                impersonation_key(&token_hash(&raw)),
                payload,
                IMPERSONATION_TTL_SECS,
            )
            .await?;
        Ok(SecretString::from(URL_SAFE_NO_PAD.encode(raw)))
    }

    /// Spends an impersonation token: the grant it was issued for, or `None`
    /// when it is unknown, malformed, expired or already used. Spending is one
    /// atomic step, so two requests with the same token cannot both succeed.
    ///
    /// # Errors
    /// Redis failures or a corrupt grant.
    pub async fn redeem_impersonation(
        &self,
        token: &str,
    ) -> Result<Option<ImpersonationGrant>, SessionError> {
        let Some(raw) = decode_token(token) else {
            return Ok(None);
        };
        let mut conn = self.redis.connection().await?;
        let payload: Option<String> = builtpanel_cache::redis::cmd("GETDEL")
            .arg(impersonation_key(&token_hash(&raw)))
            .query_async(&mut conn)
            .await?;
        payload
            .map(|p| serde_json::from_str(&p).map_err(|_| SessionError::Corrupt))
            .transpose()
    }

    /// Loads a session by cookie value, enforcing both timeouts and refreshing
    /// the idle window. `Ok(None)` for unknown, malformed or expired tokens.
    ///
    /// # Errors
    /// Redis failures or a corrupt payload.
    pub async fn load(&self, cookie_value: &str) -> Result<Option<SessionData>, SessionError> {
        let Some(raw) = decode_token(cookie_value) else {
            return Ok(None);
        };
        let key = redis_key(&token_hash(&raw));
        let mut conn = self.redis.connection().await?;
        let payload: Option<String> = conn.get(&key).await?;
        let Some(payload) = payload else {
            return Ok(None);
        };
        let mut data: SessionData =
            serde_json::from_str(&payload).map_err(|_| SessionError::Corrupt)?;

        let now = now_unix();
        if now >= data.absolute_expires_at {
            let _: u64 = conn.del(&key).await?;
            return Ok(None);
        }
        if now - data.last_seen_at >= secs(self.config.touch_interval) {
            data.last_seen_at = now;
            let payload = serde_json::to_string(&data).map_err(|_| SessionError::Corrupt)?;
            let ttl = self.ttl_from(now, data.absolute_expires_at);
            // XX: only refresh if it still exists (a concurrent revoke wins).
            let _: Option<String> = builtpanel_cache::redis::cmd("SET")
                .arg(&key)
                .arg(payload)
                .arg("EX")
                .arg(ttl)
                .arg("XX")
                .query_async(&mut conn)
                .await?;
        }
        Ok(Some(data))
    }

    /// Records that the session's owner just re-authenticated, restarting the
    /// sudo window. The session's expiry is unchanged. Returns `false` when
    /// the session no longer exists.
    ///
    /// # Errors
    /// Redis failures or a corrupt payload.
    pub async fn mark_reauthenticated(&self, cookie_value: &str) -> Result<bool, SessionError> {
        let Some(raw) = decode_token(cookie_value) else {
            return Ok(false);
        };
        let key = redis_key(&token_hash(&raw));
        let mut conn = self.redis.connection().await?;
        let payload: Option<String> = conn.get(&key).await?;
        let Some(payload) = payload else {
            return Ok(false);
        };
        let mut data: SessionData =
            serde_json::from_str(&payload).map_err(|_| SessionError::Corrupt)?;
        let now = now_unix();
        if now >= data.absolute_expires_at {
            return Ok(false);
        }
        data.auth_time = now;
        let payload = serde_json::to_string(&data).map_err(|_| SessionError::Corrupt)?;
        // XX + KEEPTTL: never resurrect a revoked session, never extend its life.
        let updated: Option<String> = builtpanel_cache::redis::cmd("SET")
            .arg(&key)
            .arg(payload)
            .arg("XX")
            .arg("KEEPTTL")
            .query_async(&mut conn)
            .await?;
        Ok(updated.is_some())
    }

    /// Current Unix time as used for session timestamps.
    #[must_use]
    pub fn now(&self) -> i64 {
        now_unix()
    }

    /// Ends a session. Returns whether it existed in the hot store.
    ///
    /// # Errors
    /// Redis or PostgreSQL failures.
    pub async fn revoke(
        &self,
        cookie_value: &str,
        reason: RevokeReason,
    ) -> Result<bool, SessionError> {
        let Some(raw) = decode_token(cookie_value) else {
            return Ok(false);
        };
        let hash = token_hash(&raw);
        let mut conn = self.redis.connection().await?;
        let removed: u64 = conn.del(redis_key(&hash)).await?;
        sqlx::query!(
            r#"UPDATE sessions SET revoked_at = now(), revoke_reason = $2
               WHERE token_sha256 = $1 AND revoked_at IS NULL"#,
            &hash[..],
            reason.as_str(),
        )
        .execute(&self.db)
        .await?;
        Ok(removed > 0)
    }

    /// The account's live sessions, newest first (at most
    /// [`MAX_LISTED_SESSIONS`]). Sessions whose Redis entry has gone (idle
    /// timeout) are marked `expired` in the history and left out.
    ///
    /// # Errors
    /// Redis or PostgreSQL failures.
    pub async fn list_active(&self, account_id: Uuid) -> Result<Vec<SessionInfo>, SessionError> {
        let rows = sqlx::query!(
            r#"SELECT id, token_sha256, host(ip) AS ip, user_agent
               FROM sessions
               WHERE account_id = $1 AND revoked_at IS NULL AND expires_at > now()
               ORDER BY created_at DESC, id DESC
               LIMIT $2"#,
            account_id,
            MAX_LISTED_SESSIONS,
        )
        .fetch_all(&self.db)
        .await?;
        if rows.is_empty() {
            return Ok(Vec::new());
        }

        let keys: Vec<String> = rows
            .iter()
            .map(|r| key_from_hash_bytes(&r.token_sha256).unwrap_or_default())
            .collect();
        let mut conn = self.redis.connection().await?;
        let payloads: Vec<Option<String>> = builtpanel_cache::redis::cmd("MGET")
            .arg(&keys)
            .query_async(&mut conn)
            .await?;

        let mut live = Vec::with_capacity(rows.len());
        let mut gone = Vec::new();
        for (row, payload) in rows.into_iter().zip(payloads) {
            let data = payload.and_then(|p| serde_json::from_str::<SessionData>(&p).ok());
            match data {
                // All three times come from the session itself, so they share
                // one clock and are consistent with each other.
                Some(data) => live.push(SessionInfo {
                    id: row.id,
                    ip: row.ip,
                    user_agent: row.user_agent,
                    created_at: data.created_at,
                    last_seen_at: data.last_seen_at,
                    expires_at: data.absolute_expires_at,
                }),
                None => gone.push(row.id),
            }
        }
        if !gone.is_empty() {
            sqlx::query!(
                r#"UPDATE sessions SET revoked_at = now(), revoke_reason = 'expired'
                   WHERE id = ANY($1) AND revoked_at IS NULL"#,
                &gone,
            )
            .execute(&self.db)
            .await?;
        }
        Ok(live)
    }

    /// Ends one session of `account_id` by id. Returns `false` when the
    /// account has no such live session (also for other accounts' ids).
    ///
    /// # Errors
    /// Redis or PostgreSQL failures.
    pub async fn revoke_by_id(
        &self,
        account_id: Uuid,
        session_id: Uuid,
        reason: RevokeReason,
    ) -> Result<bool, SessionError> {
        let hash = sqlx::query_scalar!(
            r#"UPDATE sessions SET revoked_at = now(), revoke_reason = $3
               WHERE id = $2 AND account_id = $1 AND revoked_at IS NULL
               RETURNING token_sha256"#,
            account_id,
            session_id,
            reason.as_str(),
        )
        .fetch_optional(&self.db)
        .await?;
        let Some(hash) = hash else {
            return Ok(false);
        };
        if let Some(key) = key_from_hash_bytes(&hash) {
            let mut conn = self.redis.connection().await?;
            let _: u64 = conn.del(key).await?;
        }
        Ok(true)
    }

    /// Ends every live session of the account, optionally keeping one
    /// (typically the caller's). Returns how many were ended.
    ///
    /// # Errors
    /// Redis or PostgreSQL failures.
    pub async fn revoke_all(
        &self,
        account_id: Uuid,
        keep: Option<Uuid>,
        reason: RevokeReason,
    ) -> Result<u64, SessionError> {
        let hashes = sqlx::query_scalar!(
            r#"UPDATE sessions SET revoked_at = now(), revoke_reason = $3
               WHERE account_id = $1 AND revoked_at IS NULL
                 AND ($2::uuid IS NULL OR id <> $2)
               RETURNING token_sha256"#,
            account_id,
            keep,
            reason.as_str(),
        )
        .fetch_all(&self.db)
        .await?;
        let keys: Vec<String> = hashes
            .iter()
            .filter_map(|h| key_from_hash_bytes(h))
            .collect();
        if !keys.is_empty() {
            let mut conn = self.redis.connection().await?;
            let _: u64 = builtpanel_cache::redis::cmd("DEL")
                .arg(&keys)
                .query_async(&mut conn)
                .await?;
        }
        Ok(u64::try_from(hashes.len()).unwrap_or(u64::MAX))
    }

    /// Seconds until the earlier of idle and absolute expiry (at least 1).
    fn ttl_from(&self, now: i64, absolute_expires_at: i64) -> u64 {
        let idle = secs(self.config.idle_timeout);
        let remaining = absolute_expires_at.saturating_sub(now);
        u64::try_from(idle.min(remaining).max(1)).unwrap_or(1)
    }
}

/// Extractor for routes that require a logged-in user. Rejects with a
/// Problem Details 401 when the cookie is missing, malformed or expired, and
/// with 403 `csrf_failed` when a state-changing request lacks the session's
/// CSRF token in `X-CSRF-Token`.
#[derive(Debug, Clone)]
pub struct Authenticated(pub SessionData);

impl<S> FromRequestParts<S> for Authenticated
where
    SessionStore: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        authenticate(parts, &SessionStore::from_ref(state))
            .await
            .map(Self)
    }
}

/// Session + CSRF checks shared by the extractors.
async fn authenticate(parts: &Parts, store: &SessionStore) -> Result<SessionData, AppError> {
    let Some(value) = cookie_value(&parts.headers) else {
        return Err(AppError::Unauthorized);
    };
    let data = match store.load(value).await {
        Ok(Some(data)) => data,
        Ok(None) => return Err(AppError::Unauthorized),
        Err(err) => return Err(AppError::internal(err)),
    };
    if !is_safe_method(&parts.method) {
        let presented = parts
            .headers
            .get(CSRF_HEADER)
            .map_or(&b""[..], |v| v.as_bytes());
        if !data.csrf_token.matches(presented) {
            return Err(AppError::forbidden(
                "csrf_failed",
                "Missing or invalid CSRF token.",
            ));
        }
    }
    Ok(data)
}

/// Extractor for sensitive routes (sudo mode): everything [`Authenticated`]
/// checks, plus the owner must have authenticated within
/// [`SessionConfig::sudo_window`]. Otherwise 403 `reauth_required`; the client
/// re-authenticates (password, and second factor if enabled) and retries.
#[derive(Debug, Clone)]
pub struct RecentAuth(pub SessionData);

impl<S> FromRequestParts<S> for RecentAuth
where
    SessionStore: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let store = SessionStore::from_ref(state);
        let data = authenticate(parts, &store).await?;
        if data
            .sudo_remaining(now_unix(), store.config().sudo_window)
            .is_none()
        {
            return Err(AppError::forbidden(
                "reauth_required",
                "Confirm your identity to continue.",
            ));
        }
        Ok(Self(data))
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    #[test]
    fn cookie_attributes_are_hardened() {
        let token = SessionToken(SecretString::from("abc".to_owned()));
        let c = set_cookie(&token);
        for attr in [
            "__Host-bp_session=abc",
            "Path=/",
            "Secure",
            "HttpOnly",
            "SameSite=Strict",
        ] {
            assert!(c.contains(attr), "{c}");
        }
        assert!(!c.to_ascii_lowercase().contains("domain="), "{c}");
        assert!(clear_cookie().contains("Max-Age=0"));
    }

    #[test]
    fn csrf_tokens_compare_exactly_and_are_redacted() {
        let t = CsrfToken::generate().unwrap();
        assert_eq!(t.expose().len(), 43);
        assert_ne!(t, CsrfToken::generate().unwrap());
        assert!(t.matches(t.expose().as_bytes()));
        let mut wrong = t.expose().as_bytes().to_vec();
        wrong[0] ^= 1;
        assert!(!t.matches(&wrong));
        assert!(!t.matches(&t.expose().as_bytes()[..42]));
        assert!(!t.matches(b""));
        assert!(!CsrfToken::default().matches(b""));
        assert_eq!(format!("{t:?}"), "CsrfToken(<redacted>)");
    }

    #[test]
    fn only_unsafe_methods_need_csrf() {
        for m in [Method::GET, Method::HEAD, Method::OPTIONS, Method::TRACE] {
            assert!(is_safe_method(&m), "{m}");
        }
        for m in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
            assert!(!is_safe_method(&m), "{m}");
        }
    }

    #[test]
    fn sessions_stored_before_csrf_still_parse_but_never_pass() {
        let old = r#"{"id":"0190b6a2-0000-7000-8000-000000000000","account_id":"0190b6a2-0000-7000-8000-000000000001","created_at":1,"last_seen_at":1,"absolute_expires_at":2}"#;
        let data: SessionData = serde_json::from_str(old).unwrap();
        assert!(!data.csrf_token.matches(b""));
    }

    #[test]
    fn sudo_window_counts_from_auth_time_and_fails_closed() {
        let window = Duration::from_secs(600);
        let mut data: SessionData = serde_json::from_str(
            r#"{"id":"0190b6a2-0000-7000-8000-000000000000","account_id":"0190b6a2-0000-7000-8000-000000000001","created_at":1,"last_seen_at":1,"absolute_expires_at":2}"#,
        )
        .unwrap();
        assert_eq!(data.auth_time, 0, "old sessions have no auth time");
        assert_eq!(data.sudo_remaining(1_000_000, window), None);

        data.auth_time = 1_000_000;
        assert_eq!(data.sudo_remaining(1_000_000, window), Some(600));
        assert_eq!(data.sudo_remaining(1_000_599, window), Some(1));
        assert_eq!(data.sudo_remaining(1_000_600, window), None);
        assert_eq!(data.sudo_remaining(2_000_000, window), None);
        // Clock went backwards: do not grant.
        assert_eq!(data.sudo_remaining(999_999, window), None);
    }

    #[test]
    fn timestamps_format_as_rfc3339_utc() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1_794_400_000), "2026-11-11T12:26:40Z");
        assert_eq!(rfc3339(i64::MAX), "");
    }

    #[test]
    fn token_debug_is_redacted() {
        let token = SessionToken(SecretString::from("supersecret".to_owned()));
        assert_eq!(format!("{token:?}"), "SessionToken(<redacted>)");
    }

    #[test]
    fn only_well_formed_tokens_decode() {
        let good = URL_SAFE_NO_PAD.encode([7u8; TOKEN_BYTES]);
        assert_eq!(good.len(), 43);
        assert!(decode_token(&good).is_some());
        for bad in [
            "",
            "short",
            &good[..42],
            &format!("{good}A"),
            "!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!",
        ] {
            assert!(decode_token(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn redis_key_uses_hash_not_token() {
        let raw = [1u8; TOKEN_BYTES];
        let key = redis_key(&token_hash(&raw));
        assert!(key.starts_with("builtpanel:session:"));
        assert_eq!(key.len(), "builtpanel:session:".len() + 64);
        assert!(!key.contains(&URL_SAFE_NO_PAD.encode(raw)));
    }

    #[test]
    fn cookie_value_is_found_among_others() {
        let mut h = HeaderMap::new();
        h.append(
            header::COOKIE,
            HeaderValue::from_static("a=1; __Host-bp_session=tok; b=2"),
        );
        assert_eq!(cookie_value(&h), Some("tok"));
        let mut h = HeaderMap::new();
        h.append(header::COOKIE, HeaderValue::from_static("bp_session=spoof"));
        assert_eq!(cookie_value(&h), None);
    }
}
