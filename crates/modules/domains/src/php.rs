//! The PHP version of an account and of its domains (T06.08).
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET | `/api/v1/client/php` | a client |
//! | PUT | `/api/v1/client/php` | a client; the `php_version` feature |
//! | PUT | `/api/v1/client/domains/{id}/php` | the owner of the domain; the `php_version` feature |
//! | GET, PUT | `/api/v1/admin/domains/{id}/php` | `admin.domains.read` / `.manage` (sudo mode) |
//!
//! An account runs one PHP version by default; a domain may ask for another one.
//! The versions on offer are the ones the panel has an image for, narrowed by the
//! `php_versions` setting of this module. Each version a domain asks for besides
//! the account's own runs in a container of its own, and at most three such
//! versions may run at once (the plan's PHP share is divided between them). A
//! change reaches the node in the background: `applied` says whether it has.
//! Aliases follow their domain.

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, put},
};
use builtpanel_database::{php as store, plans, routes};
use builtpanel_domain::DomainKind;
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_node_protocol::MAX_EXTRA_PHP_VERSIONS;
use builtpanel_rbac::{
    Principal, Require, RequireRecent,
    permissions::{DomainsManage, DomainsRead},
};
use builtpanel_settings::Scope;
use builtpanel_tenant_stack::Allowlist;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::{
    DomainsState, Hosting, client_hosting, entry, internal, owned_domain, tenant_of_domain,
};

/// Routes for administrators.
pub fn admin_router() -> Router<DomainsState> {
    Router::new().route(
        "/api/v1/admin/domains/{id}/php",
        get(admin_get).put(admin_set),
    )
}

/// Routes for clients.
pub fn client_router() -> Router<DomainsState> {
    Router::new()
        .route(
            "/api/v1/client/php",
            get(client_get).put(client_set_account),
        )
        .route("/api/v1/client/domains/{id}/php", put(client_set_domain))
}

/// A domain and the PHP version it uses.
#[derive(Debug, Serialize, ToSchema)]
pub struct DomainPhp {
    /// Domain id.
    pub id: Uuid,
    /// The name.
    pub fqdn: String,
    /// The version the domain asks for; `null` when it uses the account's.
    pub php_version: Option<String>,
    /// The version it runs on.
    pub effective_version: String,
}

/// PHP versions of an account.
#[derive(Debug, Serialize, ToSchema)]
pub struct PhpView {
    /// The versions that may be chosen, the one new accounts start with first.
    pub available: Vec<String>,
    /// The account's version.
    #[schema(example = "8.4")]
    pub account_version: String,
    /// Whether the node runs what is set (a change takes a moment to arrive).
    pub applied: bool,
    /// Its domains (aliases follow their domain and are left out).
    pub domains: Vec<DomainPhp>,
}

/// The version to use.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetVersion {
    /// A version from `available`; for a domain, `null` goes back to the account's.
    #[schema(example = "8.3")]
    pub version: Option<String>,
}

async fn offered(state: &DomainsState) -> Result<Vec<String>, AppError> {
    let list = Allowlist::embedded().map_err(internal)?;
    let all: Vec<String> = list.php_versions().into_iter().map(str::to_owned).collect();
    let mut conn = state.db.acquire().await.map_err(internal)?;
    let values = builtpanel_settings::get(&state.settings, &mut conn, "domains", Scope::Global)
        .await
        .map_err(internal)?;
    let wanted: Vec<&str> = values
        .get("php_versions")
        .and_then(serde_json::Value::as_str)
        .map(|text| {
            text.split(|c: char| c.is_whitespace() || c == ',')
                .filter(|e| !e.is_empty())
                .collect()
        })
        .unwrap_or_default();
    if wanted.is_empty() {
        return Ok(all);
    }
    Ok(all
        .into_iter()
        .filter(|v| wanted.contains(&v.as_str()))
        .collect())
}

async fn require_feature(state: &DomainsState, hosting: &Hosting) -> Result<(), AppError> {
    if plans::includes_feature(&state.db, hosting.plan.id, "php_version")
        .await
        .map_err(internal)?
    {
        Ok(())
    } else {
        Err(AppError::forbidden(
            "feature_disabled",
            "Choosing the PHP version is not included in this plan.",
        ))
    }
}

fn bad(field: &'static str, code: &'static str, message: &str) -> AppError {
    AppError::validation(vec![FieldError::new(field, code, message)])
}

async fn show(state: &DomainsState, hosting: &Hosting) -> Result<PhpView, AppError> {
    let php = store::state(&state.db, hosting.tenant.id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("user"))?;
    let rows = routes::domains(&state.db, hosting.tenant.id)
        .await
        .map_err(internal)?;
    let items: Vec<DomainPhp> = rows
        .into_iter()
        .filter(|d| d.kind != "alias")
        .map(|d| DomainPhp {
            id: d.id,
            fqdn: d.fqdn,
            effective_version: d.php_version.clone().unwrap_or_else(|| php.version.clone()),
            php_version: d.php_version,
        })
        .collect();
    let mut available = offered(state).await?;
    // What the account runs is always shown, even if it was taken off the list.
    if !available.contains(&php.version) {
        available.push(php.version.clone());
    }
    Ok(PhpView {
        available,
        account_version: php.version,
        applied: php.state_version == php.applied_version,
        domains: items,
    })
}

/// How many versions would run besides the account's after a change.
async fn extras_after(
    state: &DomainsState,
    tenant_id: Uuid,
    account_version: &str,
    domain: Option<(Uuid, Option<&str>)>,
) -> Result<usize, AppError> {
    let rows = routes::domains(&state.db, tenant_id)
        .await
        .map_err(internal)?;
    let mut versions = std::collections::BTreeSet::new();
    for d in rows {
        let own = match domain {
            Some((id, new)) if id == d.id => new.map(str::to_owned),
            _ => d.php_version,
        };
        if let Some(v) = own
            && v != account_version
        {
            versions.insert(v);
        }
    }
    Ok(versions.len())
}

async fn set_account(
    state: &DomainsState,
    who: &Principal,
    hosting: &Hosting,
    req: SetVersion,
) -> Result<Json<PhpView>, AppError> {
    require_feature(state, hosting).await?;
    let version = req
        .version
        .ok_or_else(|| bad("version", "version_required", "Give a PHP version."))?;
    let current = store::state(&state.db, hosting.tenant.id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("user"))?;
    if version != current.version && !offered(state).await?.contains(&version) {
        return Err(bad(
            "version",
            "unavailable_version",
            "This PHP version is not offered.",
        ));
    }
    if extras_after(state, hosting.tenant.id, &version, None).await? > MAX_EXTRA_PHP_VERSIONS {
        return Err(AppError::conflict(
            "too_many_php_versions",
            "Too many different PHP versions would run: put some domains on the same version first.",
        ));
    }
    let mut tx = state.db.begin().await.map_err(internal)?;
    let changed = store::set_account_version(&mut tx, hosting.tenant.id, &version)
        .await
        .map_err(internal)?;
    if changed {
        builtpanel_audit::insert(
            &mut *tx,
            &builtpanel_audit::AuditEntry::new(
                "php.version_changed",
                builtpanel_audit::Outcome::Success,
            )
            .actor(who.account_id())
            .tenant(hosting.tenant.id)
            .metadata(serde_json::json!({ "from": current.version, "to": version })),
        )
        .await
        .map_err(internal)?;
    }
    tx.commit().await.map_err(internal)?;
    Ok(Json(show(state, hosting).await?))
}

async fn set_domain(
    state: &DomainsState,
    who: &Principal,
    hosting: &Hosting,
    id: Uuid,
    req: SetVersion,
) -> Result<Json<PhpView>, AppError> {
    let domain = owned_domain(state, hosting.tenant.id, id).await?;
    require_feature(state, hosting).await?;
    if domain.domain.kind == DomainKind::Alias {
        return Err(bad(
            "version",
            "alias_follows_domain",
            "An alias uses its domain's PHP version: change it there.",
        ));
    }
    let before = store::domain_version(&state.db, id)
        .await
        .map_err(internal)?;
    let account = store::state(&state.db, hosting.tenant.id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("user"))?;
    if let Some(v) = &req.version
        && before.as_ref() != Some(v)
        && !offered(state).await?.contains(v)
    {
        return Err(bad(
            "version",
            "unavailable_version",
            "This PHP version is not offered.",
        ));
    }
    if extras_after(
        state,
        hosting.tenant.id,
        &account.version,
        Some((id, req.version.as_deref())),
    )
    .await?
        > MAX_EXTRA_PHP_VERSIONS
    {
        return Err(AppError::conflict(
            "too_many_php_versions",
            "Too many different PHP versions would run: put some domains on the same version first.",
        ));
    }
    let mut tx = state.db.begin().await.map_err(internal)?;
    store::set_domain_version(&mut tx, id, req.version.as_deref())
        .await
        .map_err(internal)?;
    if before != req.version {
        builtpanel_audit::insert(
            &mut *tx,
            &entry("domain.php_changed", who, hosting, id).metadata(serde_json::json!({
                "fqdn": domain.domain.fqdn,
                "from": before,
                "to": req.version,
            })),
        )
        .await
        .map_err(internal)?;
    }
    tx.commit().await.map_err(internal)?;
    Ok(Json(show(state, hosting).await?))
}

/// The PHP versions of your account and domains.
#[utoipa::path(
    get,
    path = "/api/v1/client/php",
    tag = "domains",
    operation_id = "clientGetPhp",
    security(("session" = [])),
    responses(
        (status = 200, description = "Versions on offer, the account's version, and each domain's", body = PhpView),
        (status = 403, description = "`client_only`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn client_get(
    State(state): State<DomainsState>,
    who: Principal,
) -> Result<Json<PhpView>, AppError> {
    let h = client_hosting(&state, &who).await?;
    Ok(Json(show(&state, &h).await?))
}

/// Change the PHP version of your account (what its domains use unless they ask for another).
#[utoipa::path(
    put,
    path = "/api/v1/client/php",
    tag = "domains",
    operation_id = "clientSetPhp",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = SetVersion,
    responses(
        (status = 200, description = "Saved; `applied` turns true when the node runs it", body = PhpView),
        (status = 403, description = "`client_only`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`too_many_php_versions`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "`unavailable_version` or `version_required`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn client_set_account(
    State(state): State<DomainsState>,
    who: Principal,
    Json(req): Json<SetVersion>,
) -> Result<Json<PhpView>, AppError> {
    let h = client_hosting(&state, &who).await?;
    set_account(&state, &who, &h, req).await
}

/// Change the PHP version of one of your domains (`null` goes back to the account's).
#[utoipa::path(
    put,
    path = "/api/v1/client/domains/{id}/php",
    tag = "domains",
    operation_id = "clientSetDomainPhp",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = SetVersion,
    responses(
        (status = 200, description = "Saved", body = PhpView),
        (status = 403, description = "`client_only`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain (or it is not yours)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`too_many_php_versions`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "`unavailable_version` or `alias_follows_domain`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn client_set_domain(
    State(state): State<DomainsState>,
    who: Principal,
    Path(id): Path<Uuid>,
    Json(req): Json<SetVersion>,
) -> Result<Json<PhpView>, AppError> {
    let h = client_hosting(&state, &who).await?;
    set_domain(&state, &who, &h, id, req).await
}

/// The PHP versions of the account a domain belongs to.
#[utoipa::path(
    get,
    path = "/api/v1/admin/domains/{id}/php",
    tag = "domains",
    operation_id = "adminGetDomainPhp",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Domain id")),
    responses(
        (status = 200, description = "Versions", body = PhpView),
        (status = 403, description = "`permission_denied`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn admin_get(
    State(state): State<DomainsState>,
    _who: Require<DomainsRead>,
    Path(id): Path<Uuid>,
) -> Result<Json<PhpView>, AppError> {
    let h = tenant_of_domain(&state, id).await?;
    Ok(Json(show(&state, &h).await?))
}

/// Change the PHP version of a domain (`null` goes back to its account's).
#[utoipa::path(
    put,
    path = "/api/v1/admin/domains/{id}/php",
    tag = "domains",
    operation_id = "adminSetDomainPhp",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = SetVersion,
    responses(
        (status = 200, description = "Saved", body = PhpView),
        (status = 403, description = "`permission_denied`, `reauth_required`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`too_many_php_versions`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "`unavailable_version` or `alias_follows_domain`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn admin_set(
    State(state): State<DomainsState>,
    RequireRecent(who, _): RequireRecent<DomainsManage>,
    Path(id): Path<Uuid>,
    Json(req): Json<SetVersion>,
) -> Result<Json<PhpView>, AppError> {
    let h = tenant_of_domain(&state, id).await?;
    set_domain(&state, &who, &h, id, req).await
}
