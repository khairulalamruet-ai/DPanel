//! The certificate a domain is served with (T05.16).
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET | `/api/v1/admin/domains/{id}/ssl` | `admin.domains.read` |
//! | GET | `/api/v1/client/domains/{id}/ssl` | the owner of the domain |
//! | PUT, DELETE | `/api/v1/admin/domains/{id}/ssl/custom` | `admin.domains.manage`, sudo mode |
//! | PUT, DELETE | `/api/v1/client/domains/{id}/ssl/custom` | the owner of the domain |
//!
//! The nodes report the certificates their web server holds; the control plane
//! keeps the latest per domain: who issued it and when it is valid. A domain
//! that has none yet (the first visit to it makes the web server fetch one) says
//! `none`. A certificate that ends within 14 days is `expiring`, one that has
//! ended `expired`. Nothing secret is shown.
//!
//! **Uploading a certificate** (needs the `custom_ssl` feature): the site's
//! certificate and the ones that vouch for it, and its private key. Everything is
//! checked before anything is kept ([`builtpanel_pki::custom`]): the certificate must
//! cover the domain and be valid now, each must be signed by the next, the key must
//! be an unprotected PKCS#8 key and the one of the certificate; a failure is a 422
//! naming the field and what is wrong (`key_mismatch` for a key of another
//! certificate). The key is sealed with the panel's master key before it is
//! stored, with the domain as associated data, and is never returned. Removing the
//! upload gives the domain back to the web server's own certificates.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use builtpanel_audit::{AuditEntry, Outcome};
use builtpanel_database::{certificates, plans};
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_rbac::{
    Principal, Require, RequireRecent,
    permissions::{DomainsManage, DomainsRead},
};
use builtpanel_session::rfc3339;
use secrecy::ExposeSecret as _;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::{DomainsState, Hosting, client_hosting, internal, owned_domain, tenant_of_domain};

/// How many days before its end a certificate counts as expiring.
pub const EXPIRING_DAYS: i64 = 14;

/// Routes for administrators.
pub fn admin_router() -> Router<DomainsState> {
    Router::new()
        .route("/api/v1/admin/domains/{id}/ssl", get(admin_ssl))
        .route(
            "/api/v1/admin/domains/{id}/ssl/custom",
            axum::routing::put(admin_upload).delete(admin_remove),
        )
}

/// Routes for clients.
pub fn client_router() -> Router<DomainsState> {
    Router::new()
        .route("/api/v1/client/domains/{id}/ssl", get(client_ssl))
        .route(
            "/api/v1/client/domains/{id}/ssl/custom",
            axum::routing::put(client_upload).delete(client_remove),
        )
}

/// A domain's certificate.
#[derive(Debug, Serialize, ToSchema)]
pub struct SslView {
    /// `none` (no certificate yet), `valid`, `expiring` (ends within 14 days) or `expired`.
    #[schema(example = "valid")]
    pub state: String,
    /// `acme` (obtained by the web server) or `custom` (uploaded).
    pub kind: Option<String>,
    /// Who issued it.
    #[schema(example = "R11")]
    pub issuer: Option<String>,
    /// Valid from (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub not_before: Option<String>,
    /// Valid until (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub not_after: Option<String>,
    /// Whole days until it ends (negative once it has).
    pub days_left: Option<i64>,
    /// When a node last reported it (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub last_seen_at: Option<String>,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// How a certificate that ends at `not_after` stands at `at`.
#[must_use]
pub fn state_of(not_after: i64, at: i64) -> (&'static str, i64) {
    // Floor division: a certificate with 20 hours left has 0 days left, one that
    // ended an hour ago has -1.
    let days_left = (not_after - at).div_euclid(86_400);
    let state = if not_after <= at {
        "expired"
    } else if days_left < EXPIRING_DAYS {
        "expiring"
    } else {
        "valid"
    };
    (state, days_left)
}

async fn view(
    state: &DomainsState,
    hosting: &Hosting,
    domain_id: Uuid,
) -> Result<Json<SslView>, AppError> {
    owned_domain(state, hosting.tenant.id, domain_id).await?;
    let Some(cert) = certificates::of_domain(&state.db, domain_id)
        .await
        .map_err(internal)?
    else {
        return Ok(Json(SslView {
            state: "none".to_owned(),
            kind: None,
            issuer: None,
            not_before: None,
            not_after: None,
            days_left: None,
            last_seen_at: None,
        }));
    };
    let (label, days_left) = state_of(cert.not_after, now());
    Ok(Json(SslView {
        state: label.to_owned(),
        kind: Some(cert.kind),
        issuer: Some(cert.issuer),
        not_before: Some(rfc3339(cert.not_before)),
        not_after: Some(rfc3339(cert.not_after)),
        days_left: Some(days_left),
        last_seen_at: Some(rfc3339(cert.last_seen_at)),
    }))
}

/// The certificate of a domain.
#[utoipa::path(
    get,
    path = "/api/v1/admin/domains/{id}/ssl",
    tag = "domains",
    operation_id = "adminDomainSsl",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Domain id")),
    responses(
        (status = 200, description = "The certificate, or `none`", body = SslView),
        (status = 403, description = "`permission_denied`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn admin_ssl(
    State(state): State<DomainsState>,
    _who: Require<DomainsRead>,
    Path(id): Path<Uuid>,
) -> Result<Json<SslView>, AppError> {
    let h = tenant_of_domain(&state, id).await?;
    view(&state, &h, id).await
}

/// The certificate of one of your domains.
#[utoipa::path(
    get,
    path = "/api/v1/client/domains/{id}/ssl",
    tag = "domains",
    operation_id = "clientDomainSsl",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Domain id")),
    responses(
        (status = 200, description = "The certificate, or `none`", body = SslView),
        (status = 403, description = "`client_only`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain (or it is not yours)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn client_ssl(
    State(state): State<DomainsState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<Json<SslView>, AppError> {
    let h = client_hosting(&state, &who).await?;
    view(&state, &h, id).await
}

/// A certificate to use for a domain.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UploadCertificate {
    /// The site's certificate, then the certificates that vouch for it, as PEM
    /// (`-----BEGIN CERTIFICATE-----` blocks). At most 5.
    pub certificate: String,
    /// Its private key, as an unprotected PKCS#8 PEM (`-----BEGIN PRIVATE KEY-----`).
    #[schema(format = Password)]
    pub private_key: String,
}

impl std::fmt::Debug for UploadCertificate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UploadCertificate").finish_non_exhaustive()
    }
}

/// What the key is sealed to: this domain's certificate and nothing else.
fn key_context(domain_id: Uuid) -> Vec<u8> {
    builtpanel_database::certificates::key_context(domain_id)
}

async fn upload(
    state: &DomainsState,
    who: &Principal,
    hosting: &Hosting,
    domain_id: Uuid,
    req: UploadCertificate,
) -> Result<Json<SslView>, AppError> {
    let domain = owned_domain(state, hosting.tenant.id, domain_id).await?;
    if !plans::includes_feature(&state.db, hosting.plan.id, "custom_ssl")
        .await
        .map_err(internal)?
    {
        return Err(AppError::forbidden(
            "feature_disabled",
            "Uploading a certificate is not included in this plan.",
        ));
    }
    let checked = builtpanel_pki::custom::check(
        &req.certificate,
        &req.private_key,
        &domain.domain.fqdn,
        now(),
    )
    .map_err(|e| AppError::validation(vec![FieldError::new(e.field(), e.code(), e.to_string())]))?;
    let sealed = state
        .master_key
        .seal(
            &key_context(domain_id),
            checked.private_key_pem.expose_secret().as_bytes(),
        )
        .map_err(internal)?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    certificates::set_custom(
        &mut tx,
        &certificates::NewCustom {
            domain_id,
            issuer: &checked.issuer,
            not_before: checked.not_before,
            not_after: checked.not_after,
            chain_pem: &checked.certificate_pem,
            private_key_enc: &sealed,
            uploaded_by: who.account_id(),
        },
    )
    .await
    .map_err(internal)?;
    builtpanel_audit::insert(
        &mut *tx,
        &AuditEntry::new("ssl.custom_uploaded", Outcome::Success)
            .actor(who.account_id())
            .tenant(hosting.tenant.id)
            .target("domain", domain_id)
            .metadata(serde_json::json!({
                "fqdn": domain.domain.fqdn,
                "issuer": checked.issuer,
                "not_after": checked.not_after,
            })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    view(state, hosting, domain_id).await
}

async fn remove(
    state: &DomainsState,
    who: &Principal,
    hosting: &Hosting,
    domain_id: Uuid,
) -> Result<StatusCode, AppError> {
    let domain = owned_domain(state, hosting.tenant.id, domain_id).await?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    if !certificates::remove_custom(&mut tx, domain_id)
        .await
        .map_err(internal)?
    {
        return Err(AppError::not_found("uploaded certificate"));
    }
    builtpanel_audit::insert(
        &mut *tx,
        &AuditEntry::new("ssl.custom_removed", Outcome::Success)
            .actor(who.account_id())
            .tenant(hosting.tenant.id)
            .target("domain", domain_id)
            .metadata(serde_json::json!({ "fqdn": domain.domain.fqdn })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Use an uploaded certificate for a domain.
#[utoipa::path(
    put,
    path = "/api/v1/admin/domains/{id}/ssl/custom",
    tag = "domains",
    operation_id = "adminUploadCertificate",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = UploadCertificate,
    responses(
        (status = 200, description = "Saved; the certificate as it will be shown", body = SslView),
        (status = 403, description = "`permission_denied`, `reauth_required`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "The upload is not usable: the field and a code such as `key_mismatch`, `name_mismatch`, `certificate_expired`, `broken_chain`, `key_encrypted`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn admin_upload(
    State(state): State<DomainsState>,
    RequireRecent(who, _): RequireRecent<DomainsManage>,
    Path(id): Path<Uuid>,
    Json(req): Json<UploadCertificate>,
) -> Result<Json<SslView>, AppError> {
    let h = tenant_of_domain(&state, id).await?;
    upload(&state, &who, &h, id, req).await
}

/// Go back to the web server's own certificate.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/domains/{id}/ssl/custom",
    tag = "domains",
    operation_id = "adminRemoveCertificate",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 204, description = "Removed"),
        (status = 403, description = "`permission_denied`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain, or no uploaded certificate", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn admin_remove(
    State(state): State<DomainsState>,
    RequireRecent(who, _): RequireRecent<DomainsManage>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let h = tenant_of_domain(&state, id).await?;
    remove(&state, &who, &h, id).await
}

/// Use an uploaded certificate for one of your domains.
#[utoipa::path(
    put,
    path = "/api/v1/client/domains/{id}/ssl/custom",
    tag = "domains",
    operation_id = "clientUploadCertificate",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = UploadCertificate,
    responses(
        (status = 200, description = "Saved; the certificate as it will be shown", body = SslView),
        (status = 403, description = "`client_only`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain (or it is not yours)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "The upload is not usable: the field and a code such as `key_mismatch`, `name_mismatch`, `certificate_expired`, `broken_chain`, `key_encrypted`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn client_upload(
    State(state): State<DomainsState>,
    who: Principal,
    Path(id): Path<Uuid>,
    Json(req): Json<UploadCertificate>,
) -> Result<Json<SslView>, AppError> {
    let h = client_hosting(&state, &who).await?;
    upload(&state, &who, &h, id, req).await
}

/// Go back to the web server's own certificate for one of your domains.
#[utoipa::path(
    delete,
    path = "/api/v1/client/domains/{id}/ssl/custom",
    tag = "domains",
    operation_id = "clientRemoveCertificate",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 204, description = "Removed"),
        (status = 403, description = "`client_only` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain (or it is not yours), or no uploaded certificate", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn client_remove(
    State(state): State<DomainsState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let h = client_hosting(&state, &who).await?;
    remove(&state, &who, &h, id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_certificate_is_valid_expiring_or_expired() {
        let at = 1_000_000_000;
        let day = 86_400;
        assert_eq!(state_of(at + 60 * day, at), ("valid", 60));
        assert_eq!(
            state_of(at + 14 * day, at),
            ("valid", 14),
            "exactly 14 days is not yet expiring"
        );
        assert_eq!(state_of(at + 14 * day - 1, at), ("expiring", 13));
        assert_eq!(state_of(at + 20 * 3600, at), ("expiring", 0));
        assert_eq!(state_of(at, at), ("expired", 0));
        assert_eq!(state_of(at - 3600, at), ("expired", -1));
        assert_eq!(state_of(at - 3 * day, at), ("expired", -3));
    }
}
