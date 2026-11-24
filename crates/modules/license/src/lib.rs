//! This installation's license (T17.10, T17.12, T17.13).
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET | `/api/v1/admin/license` | `admin.license.read` |
//! | POST | `/api/v1/admin/license/activate`, `/refresh`, `/deactivate`, `/recovery-request` | `admin.license.manage` |
//!
//! [`service::LicenseService`] keeps the license up to date against the license authority;
//! [`restrict`] is the middleware that, when the license is not good, closes the management
//! API (everything that changes something) with `423 license_restricted` while reading,
//! signing in and the license page itself stay open. Websites, databases and everything the
//! nodes do on their own are not touched: they never go through this API.

pub mod service;

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    extract::{FromRef, Request, State},
    http::{Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use builtpanel_audit::{AuditEntry, Outcome};
use builtpanel_database::{license as store, sqlx::PgPool};
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_license_client::ClientError;
use builtpanel_rbac::{
    Require,
    permissions::{LicenseManage, LicenseRead},
};
use builtpanel_session::SessionStore;
use serde::{Deserialize, Serialize};
use service::{LicenseService, ServiceError};
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;

/// The module's manifest.
pub const MANIFEST: &str = include_str!("../../../../modules/license/module.toml");

/// The route the license authority calls to check that this panel answers at its domain (no
/// session: the authority has none). It only signs a nonce for the panel's own domain, with a
/// signature that is no good for anything else.
pub fn public_router() -> Router<LicenseState> {
    Router::new().route("/api/v1/public/license-proof", get(proof))
}

/// The query of the domain check.
#[derive(Deserialize, utoipa::IntoParams)]
pub struct ProofQuery {
    /// The authority's challenge: 16 to 64 letters, digits, `-` or `_`.
    nonce: String,
}

/// This installation's signature of the authority's challenge for its domain.
#[derive(Serialize, ToSchema)]
pub struct ProofView {
    /// This installation.
    installation_id: Uuid,
    /// The domain it is served at.
    domain: String,
    /// Ed25519 signature (base64url) over the challenge, the installation and the domain.
    signature: String,
}

/// The authority's domain check: signs a challenge for the panel's own domain.
#[utoipa::path(
    get,
    path = "/api/v1/public/license-proof",
    tag = "license",
    operation_id = "licenseDomainProof",
    params(ProofQuery),
    responses(
        (status = 200, description = "The signed proof", body = ProofView),
        (status = 404, description = "Licensing is off or no domain is configured", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "A challenge that is not valid", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn proof(
    State(state): State<LicenseState>,
    axum::extract::Query(q): axum::extract::Query<ProofQuery>,
) -> Result<Json<ProofView>, AppError> {
    let ok = (16..=64).contains(&q.nonce.len())
        && q.nonce
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if !ok {
        return Err(AppError::validation(vec![FieldError::new(
            "nonce",
            "invalid",
            "16 to 64 letters, digits, - or _.",
        )]));
    }
    state
        .service
        .domain_proof(&q.nonce)
        .map(|p| {
            Json(ProofView {
                installation_id: p.installation_id,
                domain: p.domain,
                signature: p.signature,
            })
        })
        .ok_or_else(|| AppError::not_found("license proof"))
}

/// What the module's handlers share.
#[derive(Clone)]
pub struct LicenseState {
    /// Database pool.
    pub db: PgPool,
    /// Session store.
    pub sessions: SessionStore,
    /// The license.
    pub service: Arc<LicenseService>,
}

impl FromRef<LicenseState> for PgPool {
    fn from_ref(s: &LicenseState) -> Self {
        s.db.clone()
    }
}

impl FromRef<LicenseState> for SessionStore {
    fn from_ref(s: &LicenseState) -> Self {
        s.sessions.clone()
    }
}

/// Routes for administrators.
pub fn admin_router() -> Router<LicenseState> {
    Router::new()
        .route("/api/v1/admin/license", get(show))
        .route("/api/v1/admin/license/activate", post(activate))
        .route("/api/v1/admin/license/refresh", post(refresh))
        .route("/api/v1/admin/license/deactivate", post(deactivate))
        .route(
            "/api/v1/admin/license/recovery-request",
            post(recovery_request),
        )
}

fn internal(err: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>) -> AppError {
    AppError::internal(err)
}

/// The license, as the license page shows it.
#[derive(Debug, Serialize, ToSchema)]
pub struct LicenseView {
    /// Licensing is on (an authority is configured); when it is not nothing is checked.
    pub enforced: bool,
    /// `unlicensed`, `active`, `grace` (works, but has not been confirmed for a while) or
    /// `restricted`.
    pub mode: String,
    /// Why it is not `active` (a short code such as `offline_grace_exceeded`,
    /// `license_revoked`, `entitlement_invalid`).
    pub reason: Option<String>,
    /// This installation's id (what the authority knows it by).
    pub installation_id: Option<Uuid>,
    /// The license's plan.
    pub plan: Option<String>,
    /// `active`, `suspended` or `revoked`, as the authority last said.
    pub license_status: Option<String>,
    /// What the license switches on.
    pub features: Vec<String>,
    /// Its numeric limits (`-1`: none).
    pub limits: serde_json::Value,
    /// When the license ends (Unix seconds).
    pub license_expires_at: Option<i64>,
    /// When the entitlement in hand ends (Unix seconds).
    pub entitlement_expires_at: Option<i64>,
    /// When it was last confirmed by the authority (Unix seconds).
    pub last_confirmed_at: Option<i64>,
    /// When the offline grace ends (Unix seconds), while it runs.
    pub grace_ends_at: Option<i64>,
    /// What went wrong the last time the authority was asked.
    pub last_error: Option<String>,
}

async fn view(state: &LicenseState) -> Result<LicenseView, AppError> {
    let row = store::get(&state.db).await.map_err(internal)?;
    let stored: builtpanel_license_client::Stored =
        serde_json::from_value(row.stored.clone()).unwrap_or_default();
    Ok(LicenseView {
        enforced: row.enforced,
        mode: row.mode,
        reason: row.reason,
        installation_id: state.service.installation_id(),
        plan: row.plan,
        license_status: row.license_status,
        features: row.features,
        limits: row.limits,
        license_expires_at: row.license_expires_at,
        entitlement_expires_at: row.entitlement_expires_at,
        last_confirmed_at: stored.last_refresh_ok_at,
        grace_ends_at: row.grace_ends_at,
        last_error: row.last_error,
    })
}

/// This installation's license: its state, plan, features, limits and dates.
#[utoipa::path(get, path = "/api/v1/admin/license", tag = "license", operation_id = "adminShowLicense",
    security(("session" = [])),
    responses((status = 200, description = "The license", body = LicenseView)))]
pub async fn show(
    State(state): State<LicenseState>,
    _who: Require<LicenseRead>,
) -> Result<Json<LicenseView>, AppError> {
    Ok(Json(view(&state).await?))
}

fn service_error(e: ServiceError) -> AppError {
    match e {
        ServiceError::Disabled => {
            AppError::conflict("licensing_off", "No license authority is configured.")
        }
        ServiceError::Client(ClientError::Refused { code, .. }) => {
            let detail = match code.as_str() {
                "invalid_license_key" => "This license key is not valid.".to_owned(),
                "activation_limit" => {
                    "This license is in use on as many installations as its plan allows.".to_owned()
                }
                "license_revoked" => "This license was revoked.".to_owned(),
                "license_suspended" => "This license is suspended.".to_owned(),
                "license_expired" => "This license has expired.".to_owned(),
                other => format!("The license authority refused: {other}."),
            };
            AppError::conflict("license_refused", detail)
        }
        ServiceError::Client(ClientError::Unreachable(_)) => AppError::unavailable(
            "authority_unreachable",
            "The license authority cannot be reached.",
        ),
        ServiceError::Client(ClientError::Invalid(why)) => AppError::unavailable(
            "authority_answer_invalid",
            format!("The license authority's answer was not accepted: {why}"),
        ),
        other => internal(other),
    }
}

async fn audited(
    state: &LicenseState,
    who: &builtpanel_rbac::Principal,
    action: &'static str,
) -> Result<(), AppError> {
    builtpanel_audit::insert(
        &state.db,
        &AuditEntry::new(action, Outcome::Success)
            .actor(who.account_id())
            .target("license", Uuid::nil()),
    )
    .await
    .map_err(internal)
}

/// A license key to use.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ActivateRequest {
    /// The key the license came with (`DHP-XXXXX-XXXXX-XXXXX-XXXXX`).
    pub license_key: String,
}

impl std::fmt::Debug for ActivateRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ActivateRequest(..)")
    }
}

/// Enter a license key: this installation takes one place of the license.
#[utoipa::path(post, path = "/api/v1/admin/license/activate", tag = "license", operation_id = "adminActivateLicense",
    security(("session" = [])), params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = ActivateRequest,
    responses((status = 200, description = "Activated", body = LicenseView),
              (status = 409, description = "`license_refused` or `licensing_off`", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 422, description = "Not a license key", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 503, description = "The authority could not be reached", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn activate(
    State(state): State<LicenseState>,
    who: Require<LicenseManage>,
    Json(req): Json<ActivateRequest>,
) -> Result<Json<LicenseView>, AppError> {
    let key = req.license_key.trim().to_owned();
    if key.len() < 8 || key.len() > 64 || key.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(AppError::validation(vec![FieldError::new(
            "license_key",
            "invalid",
            "Not a license key.",
        )]));
    }
    state.service.activate(key).await.map_err(service_error)?;
    audited(&state, &who.0, "license.activated").await?;
    Ok(Json(view(&state).await?))
}

/// Ask the authority again now (when the license was changed there, or to check it).
#[utoipa::path(post, path = "/api/v1/admin/license/refresh", tag = "license", operation_id = "adminRefreshLicense",
    security(("session" = [])), params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses((status = 200, description = "Confirmed", body = LicenseView),
              (status = 409, description = "`license_refused` (the license is no longer good) or `licensing_off`", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 503, description = "The authority could not be reached", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn refresh(
    State(state): State<LicenseState>,
    who: Require<LicenseManage>,
) -> Result<Json<LicenseView>, AppError> {
    state.service.refresh().await.map_err(service_error)?;
    audited(&state, &who.0, "license.refreshed").await?;
    Ok(Json(view(&state).await?))
}

/// Give the place back (to use the license on another installation).
#[utoipa::path(post, path = "/api/v1/admin/license/deactivate", tag = "license", operation_id = "adminDeactivateLicense",
    security(("session" = [])), params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses((status = 200, description = "Let go", body = LicenseView),
              (status = 409, description = "`licensing_off`", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 503, description = "The authority could not be reached", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn deactivate(
    State(state): State<LicenseState>,
    who: Require<LicenseManage>,
) -> Result<Json<LicenseView>, AppError> {
    state.service.deactivate().await.map_err(service_error)?;
    audited(&state, &who.0, "license.deactivated").await?;
    Ok(Json(view(&state).await?))
}

/// What to tell the authority.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RecoveryRequest {
    /// What happened (at most 2000 characters).
    pub note: String,
}

/// Ask the license authority's people for help (a failed update, a lost machine).
#[utoipa::path(post, path = "/api/v1/admin/license/recovery-request", tag = "license", operation_id = "adminLicenseRecoveryRequest",
    security(("session" = [])), params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = RecoveryRequest,
    responses((status = 202, description = "Sent"),
              (status = 409, description = "`licensing_off`", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 503, description = "The authority could not be reached", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn recovery_request(
    State(state): State<LicenseState>,
    who: Require<LicenseManage>,
    Json(req): Json<RecoveryRequest>,
) -> Result<StatusCode, AppError> {
    if req.note.chars().count() > 2000 {
        return Err(AppError::validation(vec![FieldError::new(
            "note",
            "too_long",
            "At most 2000 characters.",
        )]));
    }
    state
        .service
        .recovery_request(req.note)
        .await
        .map_err(service_error)?;
    audited(&state, &who.0, "license.recovery_requested").await?;
    Ok(StatusCode::ACCEPTED)
}

// ---------------------------------------------------------------- restricted mode

/// Paths that stay open in restricted mode: signing in and out, the license page (to enter a
/// key or ask for recovery) and the public health routes.
const OPEN_PREFIXES: [&str; 3] = ["/api/v1/public/", "/api/v1/auth/", "/api/v1/admin/license"];

/// The middleware's state: the database and a short memory of the verdict, so that a request
/// does not cost a query.
#[derive(Clone)]
pub struct Restrictor {
    db: PgPool,
    cache: Arc<Mutex<Option<(Instant, bool)>>>,
}

impl Restrictor {
    /// A restrictor reading the license row.
    #[must_use]
    pub fn new(db: PgPool) -> Self {
        Self {
            db,
            cache: Arc::default(),
        }
    }

    async fn restricted(&self) -> bool {
        const TTL: Duration = Duration::from_secs(5);
        if let Some((at, value)) = *self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            && at.elapsed() < TTL
        {
            return value;
        }
        // If the row cannot be read the panel is not locked out because of it.
        let value = store::get(&self.db).await.is_ok_and(|s| s.restricted());
        *self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((Instant::now(), value));
        value
    }
}

/// While the license is not good, refuses everything that would change something (423
/// `license_restricted`). Reading stays open.
pub async fn restrict(State(r): State<Restrictor>, request: Request, next: Next) -> Response {
    let reading = matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    );
    let path = request.uri().path();
    let open = OPEN_PREFIXES.iter().any(|p| path.starts_with(p));
    if reading || open || !r.restricted().await {
        return next.run(request).await;
    }
    let body = serde_json::json!({
        "type": "https://docs.builtpanel.dev/problems/license_restricted",
        "title": "Locked",
        "status": 423,
        "code": "license_restricted",
        "detail": "This installation's license is not valid, so changes are closed. Your sites, databases and backups keep running. Open the license page to fix it.",
    });
    (
        StatusCode::LOCKED,
        [(header::CONTENT_TYPE, "application/problem+json")],
        body.to_string(),
    )
        .into_response()
}

/// The module's OpenAPI document.
#[derive(OpenApi)]
#[openapi(
    paths(show, activate, refresh, deactivate, recovery_request, proof),
    components(schemas(LicenseView, ActivateRequest, RecoveryRequest, ProofView)),
    tags((name = "license", description = "This installation's license"))
)]
pub struct LicenseApi;
