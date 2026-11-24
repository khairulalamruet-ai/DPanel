//! The panel's name servers and the delegation check (T05.12).
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET  | `/api/v1/admin/dns/nameservers` | `admin.domains.read` |
//! | POST | `/api/v1/admin/domains/{id}/dns/check` | `admin.domains.read` |
//! | GET  | `/api/v1/client/dns/nameservers` | a client account |
//! | POST | `/api/v1/client/domains/{id}/dns/check` | the owner of the domain |
//!
//! The administrator sets up to four name servers in the module's settings
//! (`nameserver_1` .. `nameserver_4`). New zones are created with them, existing
//! zones get them as their apex name servers on the next sync, clients are told
//! to point their domains at them, and the check asks the public DNS which name
//! servers a domain really has and compares: `ok`, `partial`, `mismatch`,
//! `not_delegated` or `not_configured`. A check is remembered for 15 seconds, so
//! pressing the button repeatedly asks the DNS once.

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use builtpanel_database::{dns as store, sqlx::PgPool};
use builtpanel_dns::resolver::{Assessment, assess};
use builtpanel_http_kit::{AppError, ProblemDetails};
use builtpanel_rbac::{Principal, Require, permissions::DomainsRead};
use builtpanel_session::rfc3339;
use builtpanel_settings::{Scope, SettingsRegistry};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{
    DomainsState, Hosting, client_hosting, internal, owned_domain, records::zone_of,
    tenant_of_domain,
};

/// How long a check is reused.
const REUSE_SECS: i64 = 15;

/// Routes for administrators.
pub fn admin_router() -> Router<DomainsState> {
    Router::new()
        .route("/api/v1/admin/dns/nameservers", get(admin_nameservers))
        .route("/api/v1/admin/domains/{id}/dns/check", post(admin_check))
}

/// Routes for clients.
pub fn client_router() -> Router<DomainsState> {
    Router::new()
        .route("/api/v1/client/dns/nameservers", get(client_nameservers))
        .route("/api/v1/client/domains/{id}/dns/check", post(client_check))
}

/// The name servers the administrator set, absolute (`ns1.example.net.`), in
/// order, without duplicates or blanks.
///
/// # Errors
/// Settings or database errors.
pub async fn configured(
    registry: &SettingsRegistry,
    pool: &PgPool,
) -> Result<Vec<String>, AppError> {
    let mut conn = pool.acquire().await.map_err(internal)?;
    let values = builtpanel_settings::get(registry, &mut conn, "domains", Scope::Global)
        .await
        .map_err(internal)?;
    let mut out: Vec<String> = Vec::new();
    for i in 1..=4 {
        let Some(text) = values
            .get(&format!("nameserver_{i}"))
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        let name = text.trim().trim_end_matches('.').to_ascii_lowercase();
        if name.is_empty() {
            continue;
        }
        let absolute = format!("{name}.");
        if !out.contains(&absolute) {
            out.push(absolute);
        }
    }
    Ok(out)
}

/// The panel's name servers.
#[derive(Debug, Serialize, ToSchema)]
pub struct Nameservers {
    /// Where a domain's name servers must point at the registrar, in order.
    #[schema(example = json!(["ns1.example.net", "ns2.example.net"]))]
    pub nameservers: Vec<String>,
}

fn shown(list: Vec<String>) -> Nameservers {
    Nameservers {
        nameservers: list
            .into_iter()
            .map(|n| n.trim_end_matches('.').to_owned())
            .collect(),
    }
}

/// What a delegation check found.
#[derive(Debug, Serialize, ToSchema)]
pub struct DelegationView {
    /// `ok`, `partial`, `mismatch`, `not_delegated` or `not_configured`.
    #[schema(example = "ok")]
    pub status: String,
    /// The panel's name servers.
    pub expected: Vec<String>,
    /// What the public DNS says the domain uses.
    pub found: Vec<String>,
    /// Panel name servers the domain does not use yet.
    pub missing: Vec<String>,
    /// Name servers the domain uses that are not the panel's.
    pub unexpected: Vec<String>,
    /// When the DNS was asked (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub checked_at: String,
    /// The answer of a check made a moment ago, not a new one.
    pub cached: bool,
}

fn delegation_view(
    a: &Assessment,
    checked_at: i64,
    cached: bool,
) -> Result<DelegationView, AppError> {
    let status = serde_json::to_value(a.status)
        .map_err(internal)?
        .as_str()
        .unwrap_or_default()
        .to_owned();
    Ok(DelegationView {
        status,
        expected: a.expected.clone(),
        found: a.found.clone(),
        missing: a.missing.clone(),
        unexpected: a.unexpected.clone(),
        checked_at: rfc3339(checked_at),
        cached,
    })
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

async fn check(
    state: &DomainsState,
    hosting: &Hosting,
    domain_id: Uuid,
) -> Result<Json<DelegationView>, AppError> {
    let domain = owned_domain(state, hosting.tenant.id, domain_id).await?;
    let zone = zone_of(state, domain_id, domain.domain.kind).await?;
    if let Some((saved, at)) = store::delegation(&state.db, zone.id)
        .await
        .map_err(internal)?
        && now() - at < REUSE_SECS
        && let Ok(assessment) = serde_json::from_value::<StoredAssessment>(saved)
    {
        return Ok(Json(delegation_view(&assessment.into(), at, true)?));
    }
    let expected = configured(&state.settings, &state.db).await?;
    let assessment = if expected.is_empty() {
        assess(&expected, &[])
    } else {
        let found = state.resolver.lookup_ns(&zone.name).await.map_err(|e| {
            AppError::unavailable(
                "dns_lookup_failed",
                format!("The public DNS could not be asked just now: {e}"),
            )
        })?;
        assess(&expected, &found)
    };
    store::save_delegation(
        &state.db,
        zone.id,
        &serde_json::to_value(StoredAssessment::from(&assessment)).map_err(internal)?,
    )
    .await
    .map_err(internal)?;
    Ok(Json(delegation_view(&assessment, now(), false)?))
}

/// The form a check is kept in.
#[derive(serde::Serialize, serde::Deserialize)]
struct StoredAssessment {
    status: String,
    expected: Vec<String>,
    found: Vec<String>,
    missing: Vec<String>,
    unexpected: Vec<String>,
}

impl From<&Assessment> for StoredAssessment {
    fn from(a: &Assessment) -> Self {
        Self {
            status: serde_json::to_value(a.status)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default(),
            expected: a.expected.clone(),
            found: a.found.clone(),
            missing: a.missing.clone(),
            unexpected: a.unexpected.clone(),
        }
    }
}

impl From<StoredAssessment> for Assessment {
    fn from(s: StoredAssessment) -> Self {
        use builtpanel_dns::resolver::Delegation;
        let status = match s.status.as_str() {
            "ok" => Delegation::Ok,
            "partial" => Delegation::Partial,
            "mismatch" => Delegation::Mismatch,
            "not_delegated" => Delegation::NotDelegated,
            _ => Delegation::NotConfigured,
        };
        Self {
            status,
            expected: s.expected,
            found: s.found,
            missing: s.missing,
            unexpected: s.unexpected,
        }
    }
}

/// The name servers a domain must point at.
#[utoipa::path(
    get,
    path = "/api/v1/admin/dns/nameservers",
    tag = "domains",
    operation_id = "adminNameservers",
    security(("session" = [])),
    responses(
        (status = 200, description = "The panel's name servers", body = Nameservers),
        (status = 403, description = "`permission_denied`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn admin_nameservers(
    State(state): State<DomainsState>,
    _who: Require<DomainsRead>,
) -> Result<Json<Nameservers>, AppError> {
    Ok(Json(shown(configured(&state.settings, &state.db).await?)))
}

/// The name servers to give your registrar.
#[utoipa::path(
    get,
    path = "/api/v1/client/dns/nameservers",
    tag = "domains",
    operation_id = "clientNameservers",
    security(("session" = [])),
    responses(
        (status = 200, description = "Tell the registrar these name servers; empty until the administrator sets them", body = Nameservers),
        (status = 403, description = "`client_only`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn client_nameservers(
    State(state): State<DomainsState>,
    who: Principal,
) -> Result<Json<Nameservers>, AppError> {
    // Any client may know them; the signed-in account decides who is asking.
    client_hosting(&state, &who).await?;
    Ok(Json(shown(configured(&state.settings, &state.db).await?)))
}

/// Ask the public DNS which name servers a domain uses and compare them with the panel's.
#[utoipa::path(
    post,
    path = "/api/v1/admin/domains/{id}/dns/check",
    tag = "domains",
    operation_id = "adminCheckDelegation",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 200, description = "What was found", body = DelegationView),
        (status = 403, description = "`permission_denied` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`no_zone`: a subdomain uses its parent's name servers", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 503, description = "`dns_lookup_failed`: the public DNS could not be asked", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn admin_check(
    State(state): State<DomainsState>,
    _who: Require<DomainsRead>,
    Path(id): Path<Uuid>,
) -> Result<Json<DelegationView>, AppError> {
    let h = tenant_of_domain(&state, id).await?;
    check(&state, &h, id).await
}

/// Check whether one of your domains points at the panel's name servers.
#[utoipa::path(
    post,
    path = "/api/v1/client/domains/{id}/dns/check",
    tag = "domains",
    operation_id = "clientCheckDelegation",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 200, description = "What was found", body = DelegationView),
        (status = 403, description = "`client_only` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain (or it is not yours)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`no_zone`: a subdomain uses its parent's name servers", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 503, description = "`dns_lookup_failed`: the public DNS could not be asked", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn client_check(
    State(state): State<DomainsState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<Json<DelegationView>, AppError> {
    let h = client_hosting(&state, &who).await?;
    check(&state, &h, id).await
}
