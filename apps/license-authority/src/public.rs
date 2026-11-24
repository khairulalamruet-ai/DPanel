//! The installations' API (T17.08): challenge, activate, refresh, heartbeat, deactivate,
//! status, and the public key bundle.
//!
//! Nothing is signed for an installation that has not just proved it holds its private key:
//! it asks for a challenge (a nonce), signs the nonce with the key whose public half it
//! registered, and sends the signature with the request. A nonce works once and for five
//! minutes. A license is checked inside the same transaction that counts its activations, so
//! two installations racing for the last place cannot both get it.

use std::collections::BTreeMap;

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use builtpanel_http_kit::{AppError, FieldError};
use builtpanel_license::{Claims, Document, SCHEMA_VERSION, verify_with};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;

use crate::{LaState, audit, sha256};

/// The routes.
pub fn router() -> Router<LaState> {
    Router::new()
        .route("/api/v1/licenses/challenge", post(challenge))
        .route("/api/v1/licenses/activate", post(activate))
        .route("/api/v1/licenses/refresh", post(refresh))
        .route("/api/v1/licenses/heartbeat", post(heartbeat))
        .route("/api/v1/licenses/deactivate", post(deactivate))
        .route("/api/v1/licenses/status", post(status))
        .route("/api/v1/licenses/recovery-request", post(recovery_request))
        .route("/api/v1/public-keys", get(public_keys))
}

fn bad(field: &'static str, message: &str) -> AppError {
    AppError::validation(vec![FieldError::new(field, "invalid", message)])
}

fn db(e: sqlx::Error) -> AppError {
    AppError::internal(e)
}

fn db_error(e: sqlx::Error) -> AppError {
    db(e)
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// What an installation asks a challenge for.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChallengeRequest {
    /// Its identity.
    pub installation_id: Uuid,
}

/// A nonce to sign.
#[derive(Serialize)]
pub struct Challenge {
    /// The nonce (base64url, 32 bytes).
    pub nonce: String,
    /// Until when it works (Unix seconds).
    pub expires_at: i64,
}

async fn challenge(
    State(state): State<LaState>,
    Json(req): Json<ChallengeRequest>,
) -> Result<Json<Challenge>, AppError> {
    let mut nonce = vec![0u8; 32];
    {
        use std::io::Read as _;
        std::fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut nonce))
            .map_err(AppError::internal)?;
    }
    let ttl = i64::try_from(state.settings.nonce_ttl.as_secs()).unwrap_or(300);
    sqlx::query("DELETE FROM activation_nonces WHERE expires_at < now()")
        .execute(&state.db)
        .await
        .map_err(db)?;
    let outstanding: i64 =
        sqlx::query_scalar("SELECT count(*) FROM activation_nonces WHERE installation_id = $1")
            .bind(req.installation_id)
            .fetch_one(&state.db)
            .await
            .map_err(db)?;
    if outstanding >= 20 {
        return Err(AppError::too_many_requests(
            "too_many_challenges",
            "Too many challenges are waiting for this installation.",
            std::time::Duration::from_secs(60),
        ));
    }
    let expires_at: i64 = sqlx::query_scalar(
        "INSERT INTO activation_nonces (nonce, installation_id, expires_at)
         VALUES ($1, $2, now() + make_interval(secs => $3::double precision))
         RETURNING extract(epoch FROM expires_at)::bigint",
    )
    .bind(&nonce)
    .bind(req.installation_id)
    .bind(ttl)
    .fetch_one(&state.db)
    .await
    .map_err(db)?;
    Ok(Json(Challenge {
        nonce: URL_SAFE_NO_PAD.encode(&nonce),
        expires_at,
    }))
}

/// The proof an installation sends with a request.
#[derive(Deserialize)]
pub struct Proof {
    /// The installation.
    pub installation_id: Uuid,
    /// The nonce it was given.
    pub nonce: String,
    /// Its signature of [`proof_message`] (base64url).
    pub signature: String,
    /// Its host name (signed together with the nonce; the license is bound to it).
    #[serde(default)]
    pub hostname: Option<String>,
    /// The domain the panel is reached at (signed too).
    #[serde(default)]
    pub domain: Option<String>,
}

/// The host facts an installation signs with a request: its host name and domain, normalised.
fn host_proof(hostname: Option<&str>, domain: Option<&str>) -> Option<String> {
    match (hostname, domain) {
        (Some(h), Some(d)) => Some(format!(
            "{}\n{}",
            builtpanel_license::normalize_name(h),
            builtpanel_license::normalize_name(d)
        )),
        _ => None,
    }
}

/// What an installation signs: the purpose, the nonce, itself (and, to activate, the license
/// key's hash). A signature made for one purpose is no use for another.
#[must_use]
pub fn proof_message(
    purpose: &str,
    nonce: &str,
    installation_id: Uuid,
    extra: Option<&str>,
) -> Vec<u8> {
    let mut m = format!("builtpanel-{purpose}-v1\n{nonce}\n{installation_id}");
    if let Some(extra) = extra {
        m.push('\n');
        m.push_str(extra);
    }
    m.into_bytes()
}

/// Spends a nonce. It is spent for good, even if what follows fails (it is not part of the
/// transaction that is rolled back), so a proof can be tried once.
async fn take_nonce(db: &sqlx::PgPool, installation_id: Uuid, nonce: &str) -> Result<(), AppError> {
    let raw = URL_SAFE_NO_PAD
        .decode(nonce)
        .map_err(|_| AppError::authentication_failed("bad_nonce", "The challenge is not valid."))?;
    let taken = sqlx::query("DELETE FROM activation_nonces WHERE nonce = $1 AND installation_id = $2 AND expires_at > now()")
        .bind(&raw)
        .bind(installation_id)
        .execute(db)
        .await
        .map_err(db_error)?
        .rows_affected();
    if taken == 1 {
        Ok(())
    } else {
        Err(AppError::authentication_failed(
            "bad_nonce",
            "The challenge is not valid or was used already.",
        ))
    }
}

pub(crate) fn check_signature(
    public_key: &[u8],
    message: &[u8],
    signature: &str,
) -> Result<(), AppError> {
    let sig = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| AppError::authentication_failed("bad_signature", "The proof is not valid."))?;
    if verify_with(public_key, message, &sig) {
        Ok(())
    } else {
        Err(AppError::authentication_failed(
            "bad_signature",
            "The proof is not valid.",
        ))
    }
}

/// [`prove`], for the other parts of the authority.
///
/// # Errors
/// The authentication errors of [`prove`].
pub(crate) async fn prove_request(
    state: &LaState,
    tx: &mut Transaction<'_, Postgres>,
    purpose: &str,
    proof: &Proof,
) -> Result<(), AppError> {
    prove(state, tx, purpose, proof).await
}

/// Proves that the request comes from the installation it names (a known one).
async fn prove(
    state: &LaState,
    tx: &mut Transaction<'_, Postgres>,
    purpose: &str,
    proof: &Proof,
) -> Result<(), AppError> {
    let public_key: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT public_key FROM installations WHERE id = $1")
            .bind(proof.installation_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(db)?;
    let public_key = public_key.ok_or_else(|| {
        AppError::authentication_failed("unknown_installation", "This installation is not known.")
    })?;
    take_nonce(&state.db, proof.installation_id, &proof.nonce).await?;
    check_signature(
        &public_key,
        &proof_message(
            purpose,
            &proof.nonce,
            proof.installation_id,
            host_proof(proof.hostname.as_deref(), proof.domain.as_deref()).as_deref(),
        ),
        &proof.signature,
    )
}

/// A license row as the checks need it.
#[derive(FromRow)]
struct Held {
    id: Uuid,
    status: String,
    expires_at: Option<i64>,
    plan_name: String,
    features: Vec<String>,
    limits: serde_json::Value,
    max_activations: i32,
    bound_domains: Vec<String>,
    bound_hostnames: Vec<String>,
    bound_ips: Vec<String>,
}

impl Held {
    fn bound(&self) -> crate::binding::Bound {
        crate::binding::Bound {
            domains: self.bound_domains.clone(),
            hostnames: self.bound_hostnames.clone(),
            ips: self.bound_ips.clone(),
        }
    }
}

macro_rules! held {
    ($($tail:literal),*) => {
        concat!("SELECT l.id, l.status, extract(epoch FROM l.expires_at)::bigint AS expires_at,
    p.name AS plan_name, p.features, p.limits, p.max_activations,
    l.bound_domains, l.bound_hostnames, l.bound_ips
    FROM licenses l JOIN plans p ON p.id = l.plan_id"$(, $tail)*)
    };
}

fn usable(held: &Held, now: i64) -> Result<(), AppError> {
    match held.status.as_str() {
        "revoked" => {
            return Err(AppError::forbidden(
                "license_revoked",
                "This license was revoked.",
            ));
        }
        "suspended" => {
            return Err(AppError::forbidden(
                "license_suspended",
                "This license is suspended.",
            ));
        }
        _ => {}
    }
    if held.expires_at.is_some_and(|e| e <= now) {
        return Err(AppError::forbidden(
            "license_expired",
            "This license has expired.",
        ));
    }
    Ok(())
}

/// What an installation gets.
#[derive(Serialize)]
pub struct Granted {
    /// The signed entitlement.
    pub entitlement: Document,
    /// The license in short.
    pub license: LicenseSummary,
    /// The authority's clock (Unix seconds).
    pub server_time: i64,
}

/// A license in short.
#[derive(Serialize)]
pub struct LicenseSummary {
    /// Id.
    pub id: Uuid,
    /// The plan's name.
    pub plan: String,
    /// `active`, `suspended` or `revoked`.
    pub status: String,
    /// When it ends (Unix seconds).
    pub expires_at: Option<i64>,
}

async fn issue(
    state: &LaState,
    tx: &mut Transaction<'_, Postgres>,
    held: &Held,
    installation_id: Uuid,
    now: i64,
) -> Result<Granted, AppError> {
    let ttl = i64::try_from(state.settings.entitlement_ttl.as_secs()).unwrap_or(i64::MAX / 4);
    let mut exp = now.saturating_add(ttl);
    if let Some(end) = held.expires_at {
        exp = exp.min(end);
    }
    let limits: BTreeMap<String, i64> =
        serde_json::from_value(held.limits.clone()).map_err(AppError::internal)?;
    let key = state.signing_key();
    let jti = Uuid::now_v7();
    let claims = Claims {
        v: SCHEMA_VERSION,
        iss: state.settings.issuer.clone(),
        aud: state.settings.audience.clone(),
        license_id: held.id,
        installation_id,
        features: held.features.clone(),
        limits: limits.clone(),
        bind: held.bound().binding(),
        iat: now,
        exp,
        jti,
        kid: String::new(),
    };
    let entitlement = Document::sign(&claims, key.as_ref()).map_err(AppError::internal)?;
    sqlx::query(
        "INSERT INTO entitlements (jti, license_id, installation_id, kid, issued_at, expires_at, features, limits)
         VALUES ($1, $2, $3, $4, to_timestamp($5::bigint), to_timestamp($6::bigint), $7, $8)",
    )
    .bind(jti)
    .bind(held.id)
    .bind(installation_id)
    .bind(key.kid())
    .bind(now)
    .bind(exp)
    .bind(&held.features)
    .bind(&held.limits)
    .execute(&mut **tx)
    .await
    .map_err(db)?;
    sqlx::query("UPDATE activations SET last_refresh_at = now() WHERE license_id = $1 AND installation_id = $2 AND deactivated_at IS NULL")
        .bind(held.id)
        .bind(installation_id)
        .execute(&mut **tx)
        .await
        .map_err(db)?;
    Ok(Granted {
        entitlement,
        license: LicenseSummary {
            id: held.id,
            plan: held.plan_name.clone(),
            status: held.status.clone(),
            expires_at: held.expires_at,
        },
        server_time: now,
    })
}

/// A request to activate.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivateRequest {
    /// The key the customer typed in.
    pub license_key: String,
    /// The installation's identity.
    pub installation_id: Uuid,
    /// Its public key (base64, 32 bytes).
    pub public_key: String,
    /// The nonce it was given.
    pub nonce: String,
    /// Its signature of the proof message (base64url).
    pub signature: String,
    /// Its host name.
    #[serde(default)]
    pub hostname: Option<String>,
    /// The domain the panel is reached at.
    #[serde(default)]
    pub domain: Option<String>,
}

async fn activate(
    State(state): State<LaState>,
    extensions: axum::http::Extensions,
    headers: HeaderMap,
    Json(req): Json<ActivateRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let idempotency = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    if let Some(key) = &idempotency
        && !(8..=128).contains(&key.len())
    {
        return Err(bad("idempotency-key", "8 to 128 characters."));
    }
    let request_hash = sha256(&format!(
        "{}|{}|{}",
        req.license_key, req.installation_id, req.public_key
    ));
    if let Some(key) = &idempotency {
        let seen: Option<(Vec<u8>, serde_json::Value)> =
            sqlx::query_as("SELECT request_hash, response FROM idempotency_keys WHERE scope = 'activate' AND key = $1")
                .bind(key)
                .fetch_optional(&state.db)
                .await
                .map_err(db)?;
        if let Some((hash, response)) = seen {
            return if hash == request_hash {
                Ok(Json(response))
            } else {
                Err(AppError::conflict(
                    "idempotency_key_reused",
                    "This key was used for another request.",
                ))
            };
        }
    }
    let public_key = STANDARD
        .decode(&req.public_key)
        .ok()
        .filter(|k| k.len() == 32)
        .ok_or_else(|| bad("public_key", "A base64 Ed25519 public key (32 bytes)."))?;
    if req
        .hostname
        .as_deref()
        .is_some_and(|h| h.len() > 253 || h.chars().any(char::is_control))
    {
        return Err(bad("hostname", "At most 253 characters."));
    }
    let key_hash = sha256(req.license_key.trim());
    take_nonce(&state.db, req.installation_id, &req.nonce).await?;
    let mut tx = state.db.begin().await.map_err(db)?;
    // Proof of possession: the signature covers the nonce, the installation and the license
    // key's hash, made with the key being registered.
    let hash_hex: String = key_hash.iter().map(|b| format!("{b:02x}")).collect();
    check_signature(
        &public_key,
        &proof_message(
            "activate",
            &req.nonce,
            req.installation_id,
            Some(
                &match host_proof(req.hostname.as_deref(), req.domain.as_deref()) {
                    Some(host) => format!("{hash_hex}\n{host}"),
                    None => hash_hex.clone(),
                },
            ),
        ),
        &req.signature,
    )?;
    // The license, locked for the length of the check and the insert.
    let held: Option<Held> = sqlx::query_as(held!(" WHERE l.key_hash = $1 FOR UPDATE OF l"))
        .bind(&key_hash)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?;
    let held = held.ok_or_else(|| {
        AppError::forbidden("invalid_license_key", "This license key is not valid.")
    })?;
    let now = unix_now();
    usable(&held, now)?;
    // Where the request comes from, and what it says it is, must be what the license is bound to.
    crate::binding::enforce(
        &state,
        held.id,
        req.installation_id,
        &held.bound(),
        crate::binding::source_ip(&state.settings, &extensions, &headers),
        req.hostname.as_deref(),
        req.domain.as_deref(),
    )
    .await?;
    // A new installation also proves, from outside, that its panel answers at its domain.
    if state.settings.domain_proof
        && !sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM installations WHERE id = $1)",
        )
        .bind(req.installation_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(db)?
    {
        crate::domain_proof::verify(
            &state,
            req.installation_id,
            &public_key,
            req.domain.as_deref().unwrap_or_default(),
            &held.bound().ips,
        )
        .await?;
    }
    // The installation: new, or the same key as before (a different key under the same id
    // would be somebody else claiming it).
    let known: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT public_key FROM installations WHERE id = $1")
            .bind(req.installation_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db)?;
    match known {
        Some(existing) if existing != public_key => {
            return Err(AppError::conflict(
                "installation_key_mismatch",
                "This installation id belongs to another key.",
            ));
        }
        Some(_) => {
            sqlx::query("UPDATE installations SET last_seen_at = now(), hostname = COALESCE($2, hostname) WHERE id = $1")
                .bind(req.installation_id)
                .bind(req.hostname.as_deref())
                .execute(&mut *tx)
                .await
                .map_err(db)?;
        }
        None => {
            sqlx::query("INSERT INTO installations (id, public_key, hostname) VALUES ($1, $2, $3)")
                .bind(req.installation_id)
                .bind(&public_key)
                .bind(req.hostname.as_deref())
                .execute(&mut *tx)
                .await
                .map_err(|e| {
                    if e.as_database_error()
                        .and_then(|d| d.code().map(|c| c.to_string()))
                        .as_deref()
                        == Some("23505")
                    {
                        AppError::conflict(
                            "installation_key_mismatch",
                            "This key is registered for another installation.",
                        )
                    } else {
                        db(e)
                    }
                })?;
        }
    }
    let already: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM activations WHERE license_id = $1 AND installation_id = $2 AND deactivated_at IS NULL)",
    )
    .bind(held.id)
    .bind(req.installation_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(db)?;
    if !already {
        let holders: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM activations WHERE license_id = $1 AND deactivated_at IS NULL",
        )
        .bind(held.id)
        .fetch_one(&mut *tx)
        .await
        .map_err(db)?;
        if holders >= i64::from(held.max_activations) {
            return Err(AppError::conflict(
                "activation_limit",
                format!(
                    "This license is in use on {} installation(s) already, which is all its plan allows.",
                    held.max_activations
                ),
            ));
        }
        sqlx::query("INSERT INTO activations (license_id, installation_id) VALUES ($1, $2)")
            .bind(held.id)
            .bind(req.installation_id)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        audit(
            &mut *tx,
            "installation",
            "license.activated",
            Some(&held.id.to_string()),
            serde_json::json!({ "installation_id": req.installation_id }),
        )
        .await?;
    }
    let granted = issue(&state, &mut tx, &held, req.installation_id, now).await?;
    let response = serde_json::to_value(&granted).map_err(AppError::internal)?;
    if let Some(key) = &idempotency {
        sqlx::query("INSERT INTO idempotency_keys (key, scope, request_hash, response) VALUES ($1, 'activate', $2, $3) ON CONFLICT DO NOTHING")
            .bind(key)
            .bind(&request_hash)
            .bind(&response)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
    }
    tx.commit().await.map_err(db)?;
    Ok(Json(response))
}

/// The license an installation holds now (its newest active activation).
async fn held_by(
    tx: &mut Transaction<'_, Postgres>,
    installation_id: Uuid,
) -> Result<Held, AppError> {
    sqlx::query_as(held!(
        " WHERE l.id = (SELECT license_id FROM activations WHERE installation_id = $1 AND deactivated_at IS NULL
                              ORDER BY activated_at DESC LIMIT 1)"
    ))
    .bind(installation_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(db)?
    .ok_or_else(|| AppError::forbidden("not_activated", "This installation holds no license."))
}

/// An installation whose scans failed (and has not been cleared) gets no entitlement and may
/// not let go of its license either: starting over under a new identity must not be a way out.
async fn check_integrity(
    tx: &mut Transaction<'_, Postgres>,
    installation_id: Uuid,
) -> Result<(), AppError> {
    let state: String =
        sqlx::query_scalar("SELECT integrity_state FROM installations WHERE id = $1")
            .bind(installation_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(db)?;
    if state == "ok" {
        Ok(())
    } else {
        Err(AppError::forbidden(
            "integrity_failed",
            "A program of this installation is not the release's. Ask for a recovery.",
        ))
    }
}

/// The binding check of a request that carries a proof.
async fn enforce_for(
    state: &LaState,
    held: &Held,
    proof: &Proof,
    extensions: &axum::http::Extensions,
    headers: &HeaderMap,
) -> Result<(), AppError> {
    crate::binding::enforce(
        state,
        held.id,
        proof.installation_id,
        &held.bound(),
        crate::binding::source_ip(&state.settings, extensions, headers),
        proof.hostname.as_deref(),
        proof.domain.as_deref(),
    )
    .await
}

async fn refresh(
    State(state): State<LaState>,
    extensions: axum::http::Extensions,
    headers: HeaderMap,
    Json(proof): Json<Proof>,
) -> Result<Json<Granted>, AppError> {
    let mut tx = state.db.begin().await.map_err(db)?;
    prove(&state, &mut tx, "refresh", &proof).await?;
    let held = held_by(&mut tx, proof.installation_id).await?;
    let now = unix_now();
    usable(&held, now)?;
    check_integrity(&mut tx, proof.installation_id).await?;
    enforce_for(&state, &held, &proof, &extensions, &headers).await?;
    let granted = issue(&state, &mut tx, &held, proof.installation_id, now).await?;
    sqlx::query("UPDATE installations SET last_seen_at = now() WHERE id = $1")
        .bind(proof.installation_id)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    tx.commit().await.map_err(db)?;
    Ok(Json(granted))
}

/// A heartbeat.
#[derive(Deserialize)]
pub struct HeartbeatRequest {
    /// The proof.
    #[serde(flatten)]
    pub proof: Proof,
    /// What the installation reports about itself (version, counts).
    #[serde(default)]
    pub info: serde_json::Value,
}

/// What a heartbeat answers: the license's state, so a revocation is noticed at once.
#[derive(Serialize)]
pub struct Beat {
    /// `active`, `suspended`, `revoked` or `expired`.
    pub license_status: String,
    /// The installation's integrity state: `ok`, `failed` or `recovering`.
    pub integrity: String,
    /// The authority's clock.
    pub server_time: i64,
}

async fn heartbeat(
    State(state): State<LaState>,
    extensions: axum::http::Extensions,
    headers: HeaderMap,
    Json(req): Json<HeartbeatRequest>,
) -> Result<Json<Beat>, AppError> {
    let info = if req.info.is_object() {
        req.info
    } else {
        serde_json::json!({})
    };
    if info.to_string().len() > 4096 {
        return Err(bad("info", "At most 4 KiB."));
    }
    let mut tx = state.db.begin().await.map_err(db)?;
    prove(&state, &mut tx, "heartbeat", &req.proof).await?;
    let held = held_by(&mut tx, req.proof.installation_id).await?;
    enforce_for(&state, &held, &req.proof, &extensions, &headers).await?;
    sqlx::query("UPDATE activations SET last_heartbeat_at = now(), heartbeat_info = $3 WHERE license_id = $1 AND installation_id = $2 AND deactivated_at IS NULL")
        .bind(held.id)
        .bind(req.proof.installation_id)
        .bind(info)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    sqlx::query("UPDATE installations SET last_seen_at = now() WHERE id = $1")
        .bind(req.proof.installation_id)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    tx.commit().await.map_err(db)?;
    let integrity: String =
        sqlx::query_scalar("SELECT integrity_state FROM installations WHERE id = $1")
            .bind(req.proof.installation_id)
            .fetch_one(&state.db)
            .await
            .map_err(db)?;
    let now = unix_now();
    let license_status = match usable(&held, now) {
        Ok(()) => "active".to_owned(),
        Err(_) if held.status == "active" => "expired".to_owned(),
        Err(_) => held.status.clone(),
    };
    Ok(Json(Beat {
        license_status,
        integrity,
        server_time: now,
    }))
}

async fn deactivate(
    State(state): State<LaState>,
    Json(proof): Json<Proof>,
) -> Result<StatusCode, AppError> {
    let mut tx = state.db.begin().await.map_err(db)?;
    prove(&state, &mut tx, "deactivate", &proof).await?;
    check_integrity(&mut tx, proof.installation_id).await?;
    let changed = sqlx::query("UPDATE activations SET deactivated_at = now() WHERE installation_id = $1 AND deactivated_at IS NULL")
        .bind(proof.installation_id)
        .execute(&mut *tx)
        .await
        .map_err(db)?
        .rows_affected();
    if changed > 0 {
        audit(
            &mut *tx,
            "installation",
            "license.deactivated",
            Some(&proof.installation_id.to_string()),
            serde_json::json!({}),
        )
        .await?;
    }
    tx.commit().await.map_err(db)?;
    Ok(StatusCode::NO_CONTENT)
}

/// What an installation may ask about its license.
#[derive(Serialize)]
pub struct StatusAnswer {
    /// It holds a license.
    pub activated: bool,
    /// The license, if it does.
    pub license: Option<LicenseSummary>,
    /// The authority's clock.
    pub server_time: i64,
}

async fn status(
    State(state): State<LaState>,
    Json(proof): Json<Proof>,
) -> Result<Json<StatusAnswer>, AppError> {
    let mut tx = state.db.begin().await.map_err(db)?;
    prove(&state, &mut tx, "status", &proof).await?;
    let held = held_by(&mut tx, proof.installation_id).await;
    tx.commit().await.map_err(db)?;
    let license = held.ok().map(|h| LicenseSummary {
        id: h.id,
        plan: h.plan_name,
        status: h.status,
        expires_at: h.expires_at,
    });
    Ok(Json(StatusAnswer {
        activated: license.is_some(),
        license,
        server_time: unix_now(),
    }))
}

/// The public keys that licenses were signed with, as the bundle Built Panel builds ship
/// (`kid base64-key` per line); retired keys stay so old documents can still be checked.
async fn public_keys(State(state): State<LaState>) -> Result<String, AppError> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT kid, encode(public_key, 'base64') FROM signing_keys WHERE purpose = 'license' ORDER BY created_at",
    )
    .fetch_all(&state.db)
    .await
    .map_err(db)?;
    Ok(rows
        .into_iter()
        .map(|(k, v)| format!("{k} {v}\n"))
        .collect())
}

/// A request for help.
#[derive(Deserialize)]
pub struct RecoveryRequest {
    /// The proof.
    #[serde(flatten)]
    pub proof: Proof,
    /// What happened, in the customer's words.
    #[serde(default)]
    pub note: String,
}

async fn recovery_request(
    State(state): State<LaState>,
    Json(req): Json<RecoveryRequest>,
) -> Result<StatusCode, AppError> {
    if req.note.chars().count() > 2000 || req.note.chars().any(|c| c.is_control() && c != '\n') {
        return Err(bad("note", "At most 2000 characters."));
    }
    let mut tx = state.db.begin().await.map_err(db)?;
    prove(&state, &mut tx, "recovery", &req.proof).await?;
    // One open request per installation: asking again adds to it, not to the queue.
    let open: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM recovery_requests WHERE installation_id = $1 AND state = 'requested' LIMIT 1",
    )
    .bind(req.proof.installation_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db)?;
    match open {
        Some(id) => {
            sqlx::query("UPDATE recovery_requests SET note = $2, updated_at = now() WHERE id = $1")
                .bind(id)
                .bind(&req.note)
                .execute(&mut *tx)
                .await
                .map_err(db)?;
        }
        None => {
            sqlx::query("INSERT INTO recovery_requests (installation_id, note) VALUES ($1, $2)")
                .bind(req.proof.installation_id)
                .bind(&req.note)
                .execute(&mut *tx)
                .await
                .map_err(db)?;
        }
    }
    audit(
        &mut *tx,
        "installation",
        "recovery.requested",
        Some(&req.proof.installation_id.to_string()),
        serde_json::json!({}),
    )
    .await?;
    tx.commit().await.map_err(db)?;
    Ok(StatusCode::ACCEPTED)
}
