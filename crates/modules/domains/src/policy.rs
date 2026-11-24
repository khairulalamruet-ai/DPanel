//! A domain's web server policy (T06.05): a rate limit and extra response headers.
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET, PUT | `/api/v1/admin/domains/{id}/policy` | `admin.domains.read` / `.manage` (sudo mode) |
//! | GET, PUT | `/api/v1/client/domains/{id}/policy` | the owner of the domain |
//!
//! `PUT` replaces the whole policy; an empty one (no rate limit, no headers)
//! removes it. Reading is always allowed; changing needs the `web_policy`
//! feature in the plan. A rate limit is requests per second per visitor, 1 to
//! 1000; visitors over it get 429. Headers are added to every answer; the panel's
//! own headers (HSTS, content type and the like) cannot be set.
//! (HSTS has its own switch, `hsts` on the domain.)

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use builtpanel_database::{
    domain_policies::{self, DomainPolicy, StoredHeader},
    plans,
};
use builtpanel_domain::vhost::{self, Header, MAX_HEADERS, MAX_RATE_LIMIT};
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_rbac::{
    Principal, Require, RequireRecent,
    permissions::{DomainsManage, DomainsRead},
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::{
    DomainsState, Hosting, client_hosting, entry, internal, owned_domain, tenant_of_domain,
};

/// Routes for administrators.
pub fn admin_router() -> Router<DomainsState> {
    Router::new().route(
        "/api/v1/admin/domains/{id}/policy",
        get(admin_get).put(admin_set),
    )
}

/// Routes for clients.
pub fn client_router() -> Router<DomainsState> {
    Router::new().route(
        "/api/v1/client/domains/{id}/policy",
        get(client_get).put(client_set),
    )
}

/// One response header.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct HeaderView {
    /// The name.
    #[schema(example = "X-Frame-Options")]
    pub name: String,
    /// The value: printable ASCII without `"`, `\` and `$`.
    #[schema(example = "SAMEORIGIN")]
    pub value: String,
}

/// A domain's policy.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PolicyView {
    /// Requests per second one visitor may make; `null` for no limit.
    #[schema(minimum = 1, maximum = 1000, example = 20)]
    pub rate_limit_per_second: Option<u16>,
    /// Headers added to every answer (at most 10).
    #[serde(default)]
    pub headers: Vec<HeaderView>,
}

fn view(p: DomainPolicy) -> PolicyView {
    PolicyView {
        rate_limit_per_second: p.rate_limit_per_second,
        headers: p
            .headers
            .into_iter()
            .map(|h| HeaderView {
                name: h.name,
                value: h.value,
            })
            .collect(),
    }
}

async fn get_policy(
    state: &DomainsState,
    hosting: &Hosting,
    id: Uuid,
) -> Result<Json<PolicyView>, AppError> {
    owned_domain(state, hosting.tenant.id, id).await?;
    Ok(Json(view(
        domain_policies::get(&state.db, id)
            .await
            .map_err(internal)?,
    )))
}

async fn set_policy(
    state: &DomainsState,
    who: &Principal,
    hosting: &Hosting,
    id: Uuid,
    req: PolicyView,
) -> Result<Json<PolicyView>, AppError> {
    let domain = owned_domain(state, hosting.tenant.id, id).await?;
    if !plans::includes_feature(&state.db, hosting.plan.id, "web_policy")
        .await
        .map_err(internal)?
    {
        return Err(AppError::forbidden(
            "feature_disabled",
            "Web server settings are not included in this plan.",
        ));
    }
    let checked = vhost::Policy::parse(
        req.rate_limit_per_second,
        req.headers
            .iter()
            .map(|h| Header {
                name: h.name.clone(),
                value: h.value.clone(),
            })
            .collect(),
    )
    .map_err(|e| {
        let field = if e.code.contains("rate") {
            "rate_limit_per_second"
        } else {
            "headers"
        };
        AppError::validation(vec![FieldError::new(field, e.code, e.message)])
    })?;
    let stored = DomainPolicy {
        rate_limit_per_second: checked.rate_limit_per_second,
        headers: checked
            .headers
            .into_iter()
            .map(|h| StoredHeader {
                name: h.name,
                value: h.value,
            })
            .collect(),
    };
    let mut tx = state.db.begin().await.map_err(internal)?;
    domain_policies::set(&mut tx, id, &stored)
        .await
        .map_err(internal)?;
    builtpanel_audit::insert(
        &mut *tx,
        &entry("domain.policy_changed", who, hosting, id).metadata(serde_json::json!({
            "fqdn": domain.domain.fqdn,
            "rate_limit_per_second": stored.rate_limit_per_second,
            "headers": stored.headers.iter().map(|h| h.name.as_str()).collect::<Vec<_>>(),
        })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(view(stored)))
}

/// Read a domain's web server policy.
#[utoipa::path(
    get,
    path = "/api/v1/admin/domains/{id}/policy",
    tag = "domains",
    operation_id = "adminGetDomainPolicy",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Domain id")),
    responses(
        (status = 200, description = "The policy (empty when none is set)", body = PolicyView),
        (status = 403, description = "`permission_denied`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn admin_get(
    State(state): State<DomainsState>,
    _who: Require<DomainsRead>,
    Path(id): Path<Uuid>,
) -> Result<Json<PolicyView>, AppError> {
    let h = tenant_of_domain(&state, id).await?;
    get_policy(&state, &h, id).await
}

/// Set a domain's web server policy (replaces it; an empty one removes it).
#[utoipa::path(
    put,
    path = "/api/v1/admin/domains/{id}/policy",
    tag = "domains",
    operation_id = "adminSetDomainPolicy",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = PolicyView,
    responses(
        (status = 200, description = "Saved", body = PolicyView),
        (status = 403, description = "`permission_denied`, `reauth_required`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "`invalid_rate_limit`, `too_many_headers`, `invalid_header_name`, `invalid_header_value`, `reserved_header` or `duplicate_header`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn admin_set(
    State(state): State<DomainsState>,
    RequireRecent(who, _): RequireRecent<DomainsManage>,
    Path(id): Path<Uuid>,
    Json(req): Json<PolicyView>,
) -> Result<Json<PolicyView>, AppError> {
    let h = tenant_of_domain(&state, id).await?;
    set_policy(&state, &who, &h, id, req).await
}

/// Read the web server policy of one of your domains.
#[utoipa::path(
    get,
    path = "/api/v1/client/domains/{id}/policy",
    tag = "domains",
    operation_id = "clientGetDomainPolicy",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Domain id")),
    responses(
        (status = 200, description = "The policy (empty when none is set)", body = PolicyView),
        (status = 403, description = "`client_only`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain (or it is not yours)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn client_get(
    State(state): State<DomainsState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<Json<PolicyView>, AppError> {
    let h = client_hosting(&state, &who).await?;
    get_policy(&state, &h, id).await
}

/// Set the web server policy of one of your domains (replaces it; an empty one removes it).
#[utoipa::path(
    put,
    path = "/api/v1/client/domains/{id}/policy",
    tag = "domains",
    operation_id = "clientSetDomainPolicy",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = PolicyView,
    responses(
        (status = 200, description = "Saved", body = PolicyView),
        (status = 403, description = "`client_only`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain (or it is not yours)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "`invalid_rate_limit`, `too_many_headers`, `invalid_header_name`, `invalid_header_value`, `reserved_header` or `duplicate_header`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn client_set(
    State(state): State<DomainsState>,
    who: Principal,
    Path(id): Path<Uuid>,
    Json(req): Json<PolicyView>,
) -> Result<Json<PolicyView>, AppError> {
    let h = client_hosting(&state, &who).await?;
    set_policy(&state, &who, &h, id, req).await
}

/// The limits the schema documents (kept here so the spec and the checks agree).
const _: () = {
    assert!(MAX_HEADERS == 10);
    assert!(MAX_RATE_LIMIT == 1000);
};
