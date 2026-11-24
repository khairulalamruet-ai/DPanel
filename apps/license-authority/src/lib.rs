//! The license authority (T17.01): the central service that issues and checks licenses.
//!
//! It runs on its own server with its own database and its own signing key, apart from any
//! Built Panel control plane. Administrators manage customers, plans and licenses
//! ([`admin`]); installations activate and refresh over a public API ([`public`]) and prove
//! they hold their own private key before anything is signed for them.

pub mod admin;
pub mod billing;
pub mod binding;
pub mod config;
pub mod domain_proof;
pub mod integrity;
pub mod public;

use std::{sync::Arc, time::Duration};

use axum::{
    Json, Router,
    extract::{FromRef, FromRequestParts},
    http::{header, request::Parts},
    routing::get,
};
use builtpanel_auth_core::password::PasswordHasher;
use builtpanel_http_kit::AppError;
use builtpanel_license::SigningKey;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

/// How the authority behaves.
#[derive(Debug, Clone)]
pub struct Settings {
    /// The `iss` claim.
    pub issuer: String,
    /// The `aud` claim.
    pub audience: String,
    /// How long an issued entitlement is valid.
    pub entitlement_ttl: Duration,
    /// How long an admin sign-in lasts.
    pub session_ttl: Duration,
    /// How long a challenge may be answered.
    pub nonce_ttl: Duration,
    /// Where new signing keys are written when the key is rotated.
    pub key_dir: std::path::PathBuf,
    /// The reverse proxies in front of the authority: only from these is `X-Forwarded-For`
    /// believed, to learn the address a request really comes from.
    pub trusted_proxies: Vec<std::net::IpAddr>,
    /// Whether the first activation of an installation also has to prove, from outside, that
    /// the panel answers at the domain it claims ([`domain_proof`]).
    pub domain_proof: bool,
    /// What payments webhooks are signed with (none: the webhook is off).
    pub payment_webhook_secret: Option<secrecy::SecretString>,
    /// The clocks of a subscription that is not paid ([`billing`]).
    pub billing: billing::Timing,
}

impl Settings {
    /// The settings a configuration asks for.
    #[must_use]
    pub fn from_config(c: &config::LaConfig) -> Self {
        Self {
            issuer: c.issuer.clone(),
            audience: c.audience.clone(),
            entitlement_ttl: Duration::from_secs(u64::from(c.entitlement_hours) * 3600),
            session_ttl: Duration::from_secs(u64::from(c.session_hours) * 3600),
            nonce_ttl: Duration::from_secs(300),
            key_dir: c.signing_key_file.parent().map_or_else(
                || std::path::PathBuf::from("."),
                std::path::Path::to_path_buf,
            ),
            trusted_proxies: c.trusted_proxies.clone(),
            domain_proof: c.require_domain_proof,
            payment_webhook_secret: c.payment_webhook_secret.clone(),
            billing: billing::Timing {
                past_due_days: i64::from(c.billing_past_due_days),
                grace_days: i64::from(c.billing_grace_days),
                suspended_days: i64::from(c.billing_suspended_days),
                renewal_notice_days: i64::from(c.billing_renewal_notice_days),
                period_days: i64::from(c.billing_period_days),
            },
        }
    }
}

/// What the handlers share.
#[derive(Clone)]
pub struct LaState {
    /// The authority's database.
    pub db: PgPool,
    /// The key licenses are signed with (replaced when it is rotated).
    pub signer: Arc<std::sync::RwLock<Arc<dyn SigningKey>>>,
    /// Behaviour.
    pub settings: Settings,
    /// Hashes and checks admin passwords.
    pub hasher: Arc<PasswordHasher>,
    /// Fetches a panel's proof that it answers at its domain.
    pub prover: Arc<dyn domain_proof::DomainProver>,
}

impl LaState {
    /// The key to sign with now.
    ///
    /// # Panics
    /// Never in practice: the lock is only poisoned if a thread panicked while swapping keys.
    #[must_use]
    pub fn signing_key(&self) -> Arc<dyn SigningKey> {
        self.signer
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl FromRef<LaState> for PgPool {
    fn from_ref(s: &LaState) -> Self {
        s.db.clone()
    }
}

/// A name as it is compared everywhere in the binding: lower case, no trailing dot.
#[must_use]
pub fn binding_name(name: &str) -> String {
    builtpanel_license::normalize_name(name)
}

/// The SHA-256 of a text (license keys and bearer tokens are only ever kept as this).
#[must_use]
pub fn sha256(text: &str) -> Vec<u8> {
    Sha256::digest(text.as_bytes()).to_vec()
}

/// A signed-in administrator.
#[derive(Debug, Clone)]
pub struct Admin {
    /// Who.
    pub id: Uuid,
    /// Their address.
    pub email: String,
    /// `admin` (everything) or `viewer` (reads only).
    pub role: String,
}

impl Admin {
    /// Refuses a viewer.
    ///
    /// # Errors
    /// 403 `viewer_cannot_change`.
    pub fn may_change(&self) -> Result<(), AppError> {
        if self.role == "admin" {
            Ok(())
        } else {
            Err(AppError::forbidden(
                "viewer_cannot_change",
                "This account can only look.",
            ))
        }
    }
}

impl<S> FromRequestParts<S> for Admin
where
    PgPool: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let unauthenticated = || AppError::authentication_failed("not_signed_in", "Sign in first.");
        let token = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or_else(unauthenticated)?;
        let db = PgPool::from_ref(state);
        let row: Option<(Uuid, String, String)> = sqlx::query_as(
            "SELECT a.id, a.email, a.role FROM admin_sessions s JOIN admin_users a ON a.id = s.admin_id
             WHERE s.token_hash = $1 AND s.expires_at > now()",
        )
        .bind(sha256(token))
        .fetch_optional(&db)
        .await
        .map_err(AppError::internal)?;
        let (id, email, role) = row.ok_or_else(unauthenticated)?;
        Ok(Self { id, email, role })
    }
}

/// Writes an entry in the authority's audit log.
///
/// # Errors
/// A database error.
pub async fn audit(
    db: impl sqlx::PgExecutor<'_>,
    actor: &str,
    action: &str,
    target: Option<&str>,
    detail: serde_json::Value,
) -> Result<(), AppError> {
    sqlx::query("INSERT INTO audit_logs (actor, action, target, detail) VALUES ($1, $2, $3, $4)")
        .bind(actor)
        .bind(action)
        .bind(target)
        .bind(detail)
        .execute(db)
        .await
        .map_err(AppError::internal)?;
    Ok(())
}

/// Brings the database schema up to date.
///
/// # Errors
/// A migration error.
pub async fn migrate(db: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations").run(db).await
}

/// Makes sure the signing key's public half is in `signing_keys` (and is the active one for
/// licenses). The private half is never written.
///
/// # Errors
/// A database error, or a different key under the same id.
pub async fn register_key(db: &PgPool, key: &dyn SigningKey) -> Result<(), AppError> {
    let existing: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT public_key FROM signing_keys WHERE kid = $1")
            .bind(key.kid())
            .fetch_optional(db)
            .await
            .map_err(AppError::internal)?;
    match existing {
        Some(public) if public == key.public_key() => Ok(()),
        Some(_) => Err(AppError::conflict(
            "key_id_taken",
            "A different key is registered under this key id.",
        )),
        None => {
            let mut tx = db.begin().await.map_err(AppError::internal)?;
            sqlx::query("UPDATE signing_keys SET status = 'retired', retired_at = now() WHERE purpose = 'license' AND status = 'active'")
                .execute(&mut *tx)
                .await
                .map_err(AppError::internal)?;
            sqlx::query(
                "INSERT INTO signing_keys (kid, purpose, public_key) VALUES ($1, 'license', $2)",
            )
            .bind(key.kid())
            .bind(key.public_key().to_vec())
            .execute(&mut *tx)
            .await
            .map_err(AppError::internal)?;
            tx.commit().await.map_err(AppError::internal)?;
            Ok(())
        }
    }
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
}

async fn health() -> Json<Health> {
    Json(Health { status: "ok" })
}

/// The whole application.
pub fn router(state: LaState) -> Router {
    Router::new()
        .route("/health", get(health))
        .merge(public::router())
        .merge(admin::router())
        .merge(integrity::router())
        .merge(billing::router())
        .with_state(state)
}
