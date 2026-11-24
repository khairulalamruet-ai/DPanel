//! A domain's page cache (T12.11): the web server keeps the pages of a PHP site
//! and gives them again to visitors who are not signed in.
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET, PUT | `/api/v1/admin/domains/{id}/page-cache` | `admin.domains.read` / `.manage` (sudo mode) |
//! | POST | `/api/v1/admin/domains/{id}/page-cache/purge` | `admin.domains.manage` (sudo mode) |
//! | GET, PUT | `/api/v1/client/domains/{id}/page-cache` | the owner of the domain |
//! | POST | `/api/v1/client/domains/{id}/page-cache/purge` | the owner of the domain |
//!
//! Switching it on or off and purging change the site's web server file (the
//! purge is a number in every cache key going up), which is sent to the node the
//! way every other change is. Reading is always allowed; changing needs the
//! `web_policy` feature in the plan. Visitors with a login or session cookie, an
//! `Authorization` header, the WordPress sign-in paths and every answer that sets
//! a cookie are never served from the cache. Not for aliases (they use their
//! domain's cache).

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use builtpanel_database::{page_cache, plans};
use builtpanel_domain::{DomainKind, vhost::PageCache};
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

/// Minutes a page stays fresh when nothing else is said.
const DEFAULT_MINUTES: i16 = 10;

/// Routes for administrators.
pub fn admin_router() -> Router<DomainsState> {
    Router::new()
        .route(
            "/api/v1/admin/domains/{id}/page-cache",
            get(admin_get).put(admin_set),
        )
        .route(
            "/api/v1/admin/domains/{id}/page-cache/purge",
            post(admin_purge),
        )
}

/// Routes for clients.
pub fn client_router() -> Router<DomainsState> {
    Router::new()
        .route(
            "/api/v1/client/domains/{id}/page-cache",
            get(client_get).put(client_set),
        )
        .route(
            "/api/v1/client/domains/{id}/page-cache/purge",
            post(client_purge),
        )
}

/// A domain's page cache.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PageCacheView {
    /// Whether it is on.
    pub enabled: bool,
    /// Minutes a page stays fresh, 1 to 1440.
    #[schema(minimum = 1, maximum = 1440, example = 10)]
    pub ttl_minutes: u16,
    /// How many times it was purged (a purge is counted when the web server
    /// file with it is made).
    #[serde(default)]
    pub purges: u32,
    /// When it was last purged (seconds since 1970); `null` if never.
    #[serde(default)]
    pub purged_at: Option<i64>,
}

/// What a person sends to change the cache.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetPageCache {
    /// Whether it is on.
    pub enabled: bool,
    /// Minutes a page stays fresh, 1 to 1440 (10 when left out).
    #[schema(minimum = 1, maximum = 1440, example = 10)]
    pub ttl_minutes: Option<u16>,
}

async fn show(state: &DomainsState, id: Uuid) -> Result<PageCacheView, AppError> {
    let cache = page_cache::get(&state.db, id).await.map_err(internal)?;
    let purged_at = page_cache::purged_at(&state.db, id)
        .await
        .map_err(internal)?;
    Ok(cache.map_or(
        PageCacheView {
            enabled: false,
            ttl_minutes: u16::try_from(DEFAULT_MINUTES).unwrap_or(10),
            purges: 0,
            purged_at: None,
        },
        |c| PageCacheView {
            enabled: c.enabled,
            ttl_minutes: u16::try_from(c.ttl_minutes).unwrap_or(10),
            purges: u32::try_from(c.generation.saturating_sub(1)).unwrap_or(0),
            purged_at,
        },
    ))
}

async fn writable(
    state: &DomainsState,
    hosting: &Hosting,
    id: Uuid,
) -> Result<super::DomainListing, AppError> {
    let domain = owned_domain(state, hosting.tenant.id, id).await?;
    if domain.domain.kind == DomainKind::Alias {
        return Err(AppError::validation(vec![FieldError::new(
            "domain",
            "alias_has_no_cache",
            "An alias uses its domain's cache: change it there.",
        )]));
    }
    if !plans::includes_feature(&state.db, hosting.plan.id, "web_policy")
        .await
        .map_err(internal)?
    {
        return Err(AppError::forbidden(
            "feature_disabled",
            "Web server settings are not included in this plan.",
        ));
    }
    Ok(domain)
}

async fn set_cache(
    state: &DomainsState,
    who: &Principal,
    hosting: &Hosting,
    id: Uuid,
    req: SetPageCache,
) -> Result<Json<PageCacheView>, AppError> {
    let domain = writable(state, hosting, id).await?;
    let ttl = req
        .ttl_minutes
        .unwrap_or(u16::try_from(DEFAULT_MINUTES).unwrap_or(10));
    PageCache::parse(ttl, 1).map_err(|e| {
        AppError::validation(vec![FieldError::new("ttl_minutes", e.code, e.message)])
    })?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    page_cache::set(&mut tx, id, req.enabled, i16::try_from(ttl).unwrap_or(10))
        .await
        .map_err(internal)?;
    builtpanel_audit::insert(
        &mut *tx,
        &entry("domain.page_cache_changed", who, hosting, id).metadata(serde_json::json!({
            "fqdn": domain.domain.fqdn,
            "enabled": req.enabled,
            "ttl_minutes": ttl,
        })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(show(state, id).await?))
}

async fn purge(
    state: &DomainsState,
    who: &Principal,
    hosting: &Hosting,
    id: Uuid,
) -> Result<Json<PageCacheView>, AppError> {
    let domain = writable(state, hosting, id).await?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    if !page_cache::purge(&mut tx, id).await.map_err(internal)? {
        return Err(AppError::validation(vec![FieldError::new(
            "domain",
            "cache_never_on",
            "The page cache of this domain has never been switched on.",
        )]));
    }
    builtpanel_audit::insert(
        &mut *tx,
        &entry("domain.page_cache_purged", who, hosting, id)
            .metadata(serde_json::json!({ "fqdn": domain.domain.fqdn })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(show(state, id).await?))
}

/// Read a domain's page cache.
#[utoipa::path(
    get,
    path = "/api/v1/admin/domains/{id}/page-cache",
    tag = "domains",
    operation_id = "adminGetDomainPageCache",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Domain id")),
    responses(
        (status = 200, description = "The cache (off when never set)", body = PageCacheView),
        (status = 403, description = "`permission_denied`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn admin_get(
    State(state): State<DomainsState>,
    _who: Require<DomainsRead>,
    Path(id): Path<Uuid>,
) -> Result<Json<PageCacheView>, AppError> {
    let h = tenant_of_domain(&state, id).await?;
    owned_domain(&state, h.tenant.id, id).await?;
    Ok(Json(show(&state, id).await?))
}

/// Switch a domain's page cache on or off, and set how long a page stays fresh.
#[utoipa::path(
    put,
    path = "/api/v1/admin/domains/{id}/page-cache",
    tag = "domains",
    operation_id = "adminSetDomainPageCache",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = SetPageCache,
    responses(
        (status = 200, description = "Saved", body = PageCacheView),
        (status = 403, description = "`permission_denied`, `reauth_required`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "`invalid_cache_ttl` or `alias_has_no_cache`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn admin_set(
    State(state): State<DomainsState>,
    RequireRecent(who, _): RequireRecent<DomainsManage>,
    Path(id): Path<Uuid>,
    Json(req): Json<SetPageCache>,
) -> Result<Json<PageCacheView>, AppError> {
    let h = tenant_of_domain(&state, id).await?;
    set_cache(&state, &who, &h, id, req).await
}

/// Purge a domain's page cache.
#[utoipa::path(
    post,
    path = "/api/v1/admin/domains/{id}/page-cache/purge",
    tag = "domains",
    operation_id = "adminPurgeDomainPageCache",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 200, description = "Purged (the web server file is sent to the node)", body = PageCacheView),
        (status = 403, description = "`permission_denied`, `reauth_required`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "`cache_never_on` or `alias_has_no_cache`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn admin_purge(
    State(state): State<DomainsState>,
    RequireRecent(who, _): RequireRecent<DomainsManage>,
    Path(id): Path<Uuid>,
) -> Result<Json<PageCacheView>, AppError> {
    let h = tenant_of_domain(&state, id).await?;
    purge(&state, &who, &h, id).await
}

/// Read the page cache of one of your domains.
#[utoipa::path(
    get,
    path = "/api/v1/client/domains/{id}/page-cache",
    tag = "domains",
    operation_id = "clientGetDomainPageCache",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Domain id")),
    responses(
        (status = 200, description = "The cache (off when never set)", body = PageCacheView),
        (status = 403, description = "`client_only`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain (or it is not yours)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn client_get(
    State(state): State<DomainsState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<Json<PageCacheView>, AppError> {
    let h = client_hosting(&state, &who).await?;
    owned_domain(&state, h.tenant.id, id).await?;
    Ok(Json(show(&state, id).await?))
}

/// Switch the page cache of one of your domains on or off.
#[utoipa::path(
    put,
    path = "/api/v1/client/domains/{id}/page-cache",
    tag = "domains",
    operation_id = "clientSetDomainPageCache",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = SetPageCache,
    responses(
        (status = 200, description = "Saved", body = PageCacheView),
        (status = 403, description = "`client_only`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain (or it is not yours)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "`invalid_cache_ttl` or `alias_has_no_cache`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn client_set(
    State(state): State<DomainsState>,
    who: Principal,
    Path(id): Path<Uuid>,
    Json(req): Json<SetPageCache>,
) -> Result<Json<PageCacheView>, AppError> {
    let h = client_hosting(&state, &who).await?;
    set_cache(&state, &who, &h, id, req).await
}

/// Purge the page cache of one of your domains.
#[utoipa::path(
    post,
    path = "/api/v1/client/domains/{id}/page-cache/purge",
    tag = "domains",
    operation_id = "clientPurgeDomainPageCache",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 200, description = "Purged (the web server file is sent to the node)", body = PageCacheView),
        (status = 403, description = "`client_only`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain (or it is not yours)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "`cache_never_on` or `alias_has_no_cache`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn client_purge(
    State(state): State<DomainsState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<Json<PageCacheView>, AppError> {
    let h = client_hosting(&state, &who).await?;
    purge(&state, &who, &h, id).await
}
