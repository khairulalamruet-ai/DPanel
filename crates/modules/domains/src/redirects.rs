//! Redirects of a domain (T05.04).
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET, POST | `/api/v1/admin/domains/{id}/redirects` | `admin.domains.read` / `.manage`, sudo mode |
//! | PATCH, DELETE | `/api/v1/admin/domains/{id}/redirects/{redirect_id}` | `admin.domains.manage`, sudo mode |
//! | GET, POST | `/api/v1/client/domains/{id}/redirects` | the owner of the domain |
//! | PATCH, DELETE | `/api/v1/client/domains/{id}/redirects/{redirect_id}` | the owner of the domain |
//!
//! A redirect applies to one exact path of a domain and sends the visitor to a
//! path of the same site or to an `http(s)` address, with 301, 302, 307 or 308.
//! Changing or adding one needs the `redirects` feature in the plan; reading
//! them does not.
//!
//! **Loops.** Before a redirect is saved its chain is followed: where it leads,
//! what redirect applies there on any domain of the panel, and so on. A chain
//! that comes back to where it started (`/a` to `/b` to `/a`, or `example.com`
//! to `www.example.com` and back) is refused with 422 `redirect_loop` naming
//! the circle; so is a chain of more than 10 steps. Only exact paths are
//! matched, as the web server will.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, patch},
};
use builtpanel_database::{
    plans,
    redirects::{self, AddError, Redirect},
    sqlx::PgConnection,
};
use builtpanel_domain::{Place, RedirectCode, SourcePath, Target};
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_rbac::{
    Principal, Require, RequireRecent,
    permissions::{DomainsManage, DomainsRead},
};
use builtpanel_session::rfc3339;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{
    DomainsState, Hosting, client_hosting, entry, internal, owned_domain, tenant_of_domain,
};

/// Most redirects one domain may have.
pub const MAX_PER_DOMAIN: i64 = 500;
/// Longest chain of redirects followed before giving up.
const MAX_HOPS: usize = 10;

/// Routes for administrators.
pub fn admin_router() -> Router<DomainsState> {
    Router::new()
        .route(
            "/api/v1/admin/domains/{id}/redirects",
            get(admin_list).post(admin_add),
        )
        .route(
            "/api/v1/admin/domains/{id}/redirects/{redirect_id}",
            patch(admin_update).delete(admin_delete),
        )
}

/// Routes for clients.
pub fn client_router() -> Router<DomainsState> {
    Router::new()
        .route(
            "/api/v1/client/domains/{id}/redirects",
            get(client_list).post(client_add),
        )
        .route(
            "/api/v1/client/domains/{id}/redirects/{redirect_id}",
            patch(client_update).delete(client_delete),
        )
}

/// A redirect.
#[derive(Debug, Serialize, ToSchema)]
pub struct RedirectView {
    /// Id.
    pub id: Uuid,
    /// The domain it belongs to.
    pub domain_id: Uuid,
    /// The exact path it applies to.
    #[schema(example = "/old-page")]
    pub source_path: String,
    /// A path of the same site or an http(s) address.
    #[schema(example = "https://example.org/new-page")]
    pub target: String,
    /// 301, 302, 307 or 308.
    #[schema(example = 301)]
    pub code: i16,
    /// Creation time (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub created_at: String,
}

fn view(r: Redirect) -> RedirectView {
    RedirectView {
        id: r.id,
        domain_id: r.domain_id,
        source_path: r.source_path,
        target: r.target,
        code: r.code,
        created_at: rfc3339(r.created_at),
    }
}

/// The redirects of a domain.
#[derive(Debug, Serialize, ToSchema)]
pub struct RedirectList {
    /// By path.
    pub items: Vec<RedirectView>,
}

/// A redirect to add.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AddRedirect {
    /// The exact path it applies to, starting with `/` (`/` is the front page).
    #[schema(example = "/old-page")]
    pub source_path: String,
    /// A path of the same site or an http(s) address.
    #[schema(example = "/new-page")]
    pub target: String,
    /// 301 (default), 302, 307 or 308.
    pub code: Option<i64>,
}

/// Changes to a redirect (the path it applies to cannot change: remove it and add another).
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateRedirect {
    /// New target.
    pub target: Option<String>,
    /// New code.
    pub code: Option<i64>,
}

/// Follows where a redirect from `host` + `source` to `target` would lead.
///
/// # Errors
/// 422 `redirect_loop` or `redirect_chain_too_long`.
async fn check_chain(
    conn: &mut PgConnection,
    host: &str,
    source: &SourcePath,
    target: &Target,
) -> Result<(), AppError> {
    let start = Place {
        host: host.to_owned(),
        path: source.as_str().to_owned(),
    };
    let mut trail = vec![start.clone()];
    let mut seen: HashSet<Place> = HashSet::from([start]);
    let mut current = target.place(host);
    for _ in 0..MAX_HOPS {
        trail.push(current.clone());
        if !seen.insert(current.clone()) {
            let circle: Vec<String> = trail.iter().map(ToString::to_string).collect();
            return Err(AppError::validation(vec![FieldError::new(
                "target",
                "redirect_loop",
                format!(
                    "This redirect would send visitors in a circle: {}.",
                    circle.join(" -> ")
                ),
            )]));
        }
        let next = redirects::target_for(conn, &current.host, &current.path)
            .await
            .map_err(internal)?;
        let Some(next) = next else {
            return Ok(());
        };
        // Stored targets were checked when they were saved.
        let Ok(next) = Target::parse(&next) else {
            return Ok(());
        };
        current = next.place(&current.host);
    }
    Err(AppError::validation(vec![FieldError::new(
        "target",
        "redirect_chain_too_long",
        format!("Visitors would be sent on through more than {MAX_HOPS} redirects."),
    )]))
}

async fn require_feature(state: &DomainsState, hosting: &Hosting) -> Result<(), AppError> {
    if plans::includes_feature(&state.db, hosting.plan.id, "redirects")
        .await
        .map_err(internal)?
    {
        Ok(())
    } else {
        Err(AppError::forbidden(
            "feature_disabled",
            "Redirects are not included in this plan.",
        ))
    }
}

fn code_of(n: Option<i64>) -> Result<i16, AppError> {
    let code = RedirectCode::from_number(n.unwrap_or(301)).map_err(|e| {
        AppError::validation(vec![FieldError::new("code", e.code(), e.to_string())])
    })?;
    Ok(code.number())
}

fn target_of(text: &str) -> Result<Target, AppError> {
    Target::parse(text)
        .map_err(|e| AppError::validation(vec![FieldError::new("target", e.code(), e.to_string())]))
}

async fn add_redirect(
    state: &DomainsState,
    who: &Principal,
    hosting: &Hosting,
    domain_id: Uuid,
    req: AddRedirect,
) -> Result<RedirectView, AppError> {
    let domain = owned_domain(state, hosting.tenant.id, domain_id).await?;
    require_feature(state, hosting).await?;
    let mut errors = Vec::new();
    let source = SourcePath::parse(&req.source_path)
        .map_err(|e| errors.push(FieldError::new("source_path", e.code(), e.to_string())))
        .ok();
    let target = Target::parse(&req.target)
        .map_err(|e| errors.push(FieldError::new("target", e.code(), e.to_string())))
        .ok();
    let code = RedirectCode::from_number(req.code.unwrap_or(301))
        .map_err(|e| errors.push(FieldError::new("code", e.code(), e.to_string())))
        .ok();
    let (Some(source), Some(target), Some(code), true) = (source, target, code, errors.is_empty())
    else {
        return Err(AppError::validation(errors));
    };

    let mut tx = state.db.begin().await.map_err(internal)?;
    // One redirect at a time per account, so two requests cannot together close a circle.
    builtpanel_database::domains::lock_tenant(&mut tx, hosting.tenant.id)
        .await
        .map_err(internal)?;
    if redirects::count(&mut tx, domain_id)
        .await
        .map_err(internal)?
        >= MAX_PER_DOMAIN
    {
        return Err(AppError::conflict(
            "limit_reached",
            format!("A domain may have at most {MAX_PER_DOMAIN} redirects."),
        ));
    }
    check_chain(&mut tx, &domain.domain.fqdn, &source, &target).await?;
    let added = redirects::add(
        &mut tx,
        domain_id,
        source.as_str(),
        target.as_str(),
        code.number(),
    )
    .await
    .map_err(|e| match e {
        AddError::Duplicate => AppError::conflict(
            "redirect_exists",
            "This path already has a redirect; change it instead.",
        ),
        AddError::UnknownDomain => AppError::not_found("domain"),
        AddError::Database(db) => internal(db),
    })?;
    builtpanel_audit::insert(
        &mut *tx,
        &entry("redirect.added", who, hosting, domain_id).metadata(serde_json::json!({
            "domain": domain.domain.fqdn,
            "source_path": added.source_path,
            "target": added.target,
            "code": added.code,
        })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(view(added))
}

async fn update_redirect(
    state: &DomainsState,
    who: &Principal,
    hosting: &Hosting,
    domain_id: Uuid,
    redirect_id: Uuid,
    req: UpdateRedirect,
) -> Result<RedirectView, AppError> {
    let domain = owned_domain(state, hosting.tenant.id, domain_id).await?;
    require_feature(state, hosting).await?;
    if req.target.is_none() && req.code.is_none() {
        return Err(AppError::validation(vec![FieldError::new(
            "target",
            "nothing_to_change",
            "Give a target or a code to change.",
        )]));
    }
    let existing = redirects::get(&state.db, domain_id, redirect_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("redirect"))?;
    let target = req.target.as_deref().map(target_of).transpose()?;
    let code = req.code.map(|c| code_of(Some(c))).transpose()?;

    let mut tx = state.db.begin().await.map_err(internal)?;
    builtpanel_database::domains::lock_tenant(&mut tx, hosting.tenant.id)
        .await
        .map_err(internal)?;
    if let Some(target) = &target {
        let source = SourcePath::parse(&existing.source_path)
            .map_err(|e| internal(std::io::Error::other(e.to_string())))?;
        check_chain(&mut tx, &domain.domain.fqdn, &source, target).await?;
    }
    if !redirects::update(
        &mut tx,
        domain_id,
        redirect_id,
        target.as_ref().map(Target::as_str),
        code,
    )
    .await
    .map_err(internal)?
    {
        return Err(AppError::not_found("redirect"));
    }
    builtpanel_audit::insert(
        &mut *tx,
        &entry("redirect.changed", who, hosting, domain_id).metadata(serde_json::json!({
            "domain": domain.domain.fqdn,
            "source_path": existing.source_path,
            "target": target.as_ref().map(Target::as_str),
            "code": code,
        })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(view(
        redirects::get(&state.db, domain_id, redirect_id)
            .await
            .map_err(internal)?
            .ok_or_else(|| AppError::not_found("redirect"))?,
    ))
}

async fn delete_redirect(
    state: &DomainsState,
    who: &Principal,
    hosting: &Hosting,
    domain_id: Uuid,
    redirect_id: Uuid,
) -> Result<(), AppError> {
    let domain = owned_domain(state, hosting.tenant.id, domain_id).await?;
    let existing = redirects::get(&state.db, domain_id, redirect_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("redirect"))?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    if !redirects::delete(&mut tx, domain_id, redirect_id)
        .await
        .map_err(internal)?
    {
        return Err(AppError::not_found("redirect"));
    }
    builtpanel_audit::insert(
        &mut *tx,
        &entry("redirect.deleted", who, hosting, domain_id).metadata(serde_json::json!({
            "domain": domain.domain.fqdn,
            "source_path": existing.source_path,
        })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(())
}

async fn list_redirects(
    state: &DomainsState,
    hosting: &Hosting,
    domain_id: Uuid,
) -> Result<Json<RedirectList>, AppError> {
    owned_domain(state, hosting.tenant.id, domain_id).await?;
    Ok(Json(RedirectList {
        items: redirects::list(&state.db, domain_id)
            .await
            .map_err(internal)?
            .into_iter()
            .map(view)
            .collect(),
    }))
}

/// List a domain's redirects.
#[utoipa::path(
    get,
    path = "/api/v1/admin/domains/{id}/redirects",
    tag = "domains",
    operation_id = "adminListRedirects",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Domain id")),
    responses(
        (status = 200, description = "The redirects", body = RedirectList),
        (status = 403, description = "`permission_denied`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn admin_list(
    State(state): State<DomainsState>,
    _who: Require<DomainsRead>,
    Path(id): Path<Uuid>,
) -> Result<Json<RedirectList>, AppError> {
    let h = tenant_of_domain(&state, id).await?;
    list_redirects(&state, &h, id).await
}

/// Add a redirect to a domain.
#[utoipa::path(
    post,
    path = "/api/v1/admin/domains/{id}/redirects",
    tag = "domains",
    operation_id = "adminAddRedirect",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = AddRedirect,
    responses(
        (status = 201, description = "Added", body = RedirectView),
        (status = 403, description = "`permission_denied`, `reauth_required`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`redirect_exists` or `limit_reached`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid path, target or code; `redirect_loop`; `redirect_chain_too_long`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn admin_add(
    State(state): State<DomainsState>,
    RequireRecent(who, _): RequireRecent<DomainsManage>,
    Path(id): Path<Uuid>,
    Json(req): Json<AddRedirect>,
) -> Result<(StatusCode, Json<RedirectView>), AppError> {
    let h = tenant_of_domain(&state, id).await?;
    Ok((
        StatusCode::CREATED,
        Json(add_redirect(&state, &who, &h, id, req).await?),
    ))
}

/// Change a redirect's target or code.
#[utoipa::path(
    patch,
    path = "/api/v1/admin/domains/{id}/redirects/{redirect_id}",
    tag = "domains",
    operation_id = "adminUpdateRedirect",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("redirect_id" = Uuid, Path, description = "Redirect id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = UpdateRedirect,
    responses(
        (status = 200, description = "Updated", body = RedirectView),
        (status = 403, description = "`permission_denied`, `reauth_required`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain or redirect", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Nothing to change, invalid target or code, `redirect_loop` or `redirect_chain_too_long`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn admin_update(
    State(state): State<DomainsState>,
    RequireRecent(who, _): RequireRecent<DomainsManage>,
    Path((id, redirect_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<UpdateRedirect>,
) -> Result<Json<RedirectView>, AppError> {
    let h = tenant_of_domain(&state, id).await?;
    Ok(Json(
        update_redirect(&state, &who, &h, id, redirect_id, req).await?,
    ))
}

/// Remove a redirect.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/domains/{id}/redirects/{redirect_id}",
    tag = "domains",
    operation_id = "adminDeleteRedirect",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("redirect_id" = Uuid, Path, description = "Redirect id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 204, description = "Removed"),
        (status = 403, description = "`permission_denied`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain or redirect", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn admin_delete(
    State(state): State<DomainsState>,
    RequireRecent(who, _): RequireRecent<DomainsManage>,
    Path((id, redirect_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    let h = tenant_of_domain(&state, id).await?;
    delete_redirect(&state, &who, &h, id, redirect_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// List the redirects of one of your domains.
#[utoipa::path(
    get,
    path = "/api/v1/client/domains/{id}/redirects",
    tag = "domains",
    operation_id = "clientListRedirects",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Domain id")),
    responses(
        (status = 200, description = "The redirects", body = RedirectList),
        (status = 403, description = "`client_only`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain (or it is not yours)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn client_list(
    State(state): State<DomainsState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<Json<RedirectList>, AppError> {
    let h = client_hosting(&state, &who).await?;
    list_redirects(&state, &h, id).await
}

/// Add a redirect to one of your domains.
#[utoipa::path(
    post,
    path = "/api/v1/client/domains/{id}/redirects",
    tag = "domains",
    operation_id = "clientAddRedirect",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = AddRedirect,
    responses(
        (status = 201, description = "Added", body = RedirectView),
        (status = 403, description = "`client_only`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain (or it is not yours)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`redirect_exists` or `limit_reached`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid path, target or code; `redirect_loop`; `redirect_chain_too_long`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn client_add(
    State(state): State<DomainsState>,
    who: Principal,
    Path(id): Path<Uuid>,
    Json(req): Json<AddRedirect>,
) -> Result<(StatusCode, Json<RedirectView>), AppError> {
    let h = client_hosting(&state, &who).await?;
    Ok((
        StatusCode::CREATED,
        Json(add_redirect(&state, &who, &h, id, req).await?),
    ))
}

/// Change a redirect of one of your domains.
#[utoipa::path(
    patch,
    path = "/api/v1/client/domains/{id}/redirects/{redirect_id}",
    tag = "domains",
    operation_id = "clientUpdateRedirect",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("redirect_id" = Uuid, Path, description = "Redirect id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = UpdateRedirect,
    responses(
        (status = 200, description = "Updated", body = RedirectView),
        (status = 403, description = "`client_only`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain or redirect (or it is not yours)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Nothing to change, invalid target or code, `redirect_loop` or `redirect_chain_too_long`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn client_update(
    State(state): State<DomainsState>,
    who: Principal,
    Path((id, redirect_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<UpdateRedirect>,
) -> Result<Json<RedirectView>, AppError> {
    let h = client_hosting(&state, &who).await?;
    Ok(Json(
        update_redirect(&state, &who, &h, id, redirect_id, req).await?,
    ))
}

/// Remove a redirect of one of your domains.
#[utoipa::path(
    delete,
    path = "/api/v1/client/domains/{id}/redirects/{redirect_id}",
    tag = "domains",
    operation_id = "clientDeleteRedirect",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("redirect_id" = Uuid, Path, description = "Redirect id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 204, description = "Removed"),
        (status = 403, description = "`client_only` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain or redirect (or it is not yours)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn client_delete(
    State(state): State<DomainsState>,
    who: Principal,
    Path((id, redirect_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    let h = client_hosting(&state, &who).await?;
    delete_redirect(&state, &who, &h, id, redirect_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
