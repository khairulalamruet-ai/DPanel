//! A domain's web server file, and the directives its owner adds to it (T06.06).
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET, PUT | `/api/v1/admin/domains/{id}/vhost` | `admin.domains.read` / `.manage` (sudo mode) |
//! | GET, PUT | `/api/v1/client/domains/{id}/vhost` | the owner of the domain; the `edit_vhost` feature |
//!
//! `GET` shows the file the site runs on (an alias shows its domain's) and the
//! owner's own block. `PUT` replaces the block; an empty one removes it. The block
//! may only use a short list of directives that shape the site's own answers
//! (`location`, `rewrite`, `return`, `expires`, `add_header` and a few more); a
//! directive outside the list, or one that cannot be told apart from text, is
//! refused with 422 `forbidden_directive` naming it. The file is made again and
//! sent to the node with the next sync; if the web server there refuses it, the
//! file that was running keeps running.

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use builtpanel_database::{domain_policies, plans};
use builtpanel_domain::vhost::{MAX_CUSTOM_BYTES, check_custom_block};
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
        "/api/v1/admin/domains/{id}/vhost",
        get(admin_get).put(admin_set),
    )
}

/// Routes for clients.
pub fn client_router() -> Router<DomainsState> {
    Router::new().route(
        "/api/v1/client/domains/{id}/vhost",
        get(client_get).put(client_set),
    )
}

/// A domain's web server file.
#[derive(Debug, Serialize, ToSchema)]
pub struct VhostView {
    /// The file the site runs on, as the panel makes it from the template, the
    /// domain's settings and the block below. `null` while the account has no
    /// web server yet (it is being set up or removed).
    pub config: Option<String>,
    /// The owner's own directives (empty for none).
    pub custom_block: String,
    /// Longest block, in bytes.
    #[schema(example = 8192)]
    pub max_custom_bytes: usize,
}

/// The owner's own directives.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetVhost {
    /// The directives; an empty text removes the block.
    pub custom_block: String,
}

async fn require_feature(state: &DomainsState, hosting: &Hosting) -> Result<(), AppError> {
    if plans::includes_feature(&state.db, hosting.plan.id, "edit_vhost")
        .await
        .map_err(internal)?
    {
        Ok(())
    } else {
        Err(AppError::forbidden(
            "feature_disabled",
            "Editing the web server settings is not included in this plan.",
        ))
    }
}

async fn show(
    state: &DomainsState,
    hosting: &Hosting,
    id: Uuid,
) -> Result<Json<VhostView>, AppError> {
    let domain = owned_domain(state, hosting.tenant.id, id).await?;
    // An alias runs on its domain's file.
    let file_name = match (domain.domain.kind, domain.domain.parent_domain_id) {
        (builtpanel_domain::DomainKind::Alias, Some(parent)) => {
            owned_domain(state, hosting.tenant.id, parent)
                .await?
                .domain
                .fqdn
        }
        _ => domain.domain.fqdn.clone(),
    };
    let files = builtpanel_web_config::site_files(
        &state.db,
        hosting.tenant.id,
        &hosting.tenant.username,
        hosting.tenant.status.as_str(),
    )
    .await
    .map_err(|e| internal(std::io::Error::other(e)))?;
    let custom_block = domain_policies::custom_block(&state.db, id)
        .await
        .map_err(internal)?
        .unwrap_or_default();
    Ok(Json(VhostView {
        config: files
            .into_iter()
            .find(|(name, _)| *name == file_name)
            .map(|(_, text)| text),
        custom_block,
        max_custom_bytes: MAX_CUSTOM_BYTES,
    }))
}

async fn set(
    state: &DomainsState,
    who: &Principal,
    hosting: &Hosting,
    id: Uuid,
    req: SetVhost,
) -> Result<Json<VhostView>, AppError> {
    let domain = owned_domain(state, hosting.tenant.id, id).await?;
    require_feature(state, hosting).await?;
    if domain.domain.kind == builtpanel_domain::DomainKind::Alias {
        return Err(AppError::validation(vec![FieldError::new(
            "custom_block",
            "alias_has_no_file",
            "An alias uses its domain's settings: change them there.",
        )]));
    }
    check_custom_block(&req.custom_block).map_err(|e| {
        AppError::validation(vec![FieldError::new("custom_block", e.code, e.message)])
    })?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    domain_policies::set_custom_block(&mut tx, id, &req.custom_block)
        .await
        .map_err(internal)?;
    builtpanel_audit::insert(
        &mut *tx,
        &entry("domain.vhost_changed", who, hosting, id).metadata(serde_json::json!({
            "fqdn": domain.domain.fqdn,
            "bytes": req.custom_block.len(),
        })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    show(state, hosting, id).await
}

/// See a domain's web server file and its owner's own directives.
#[utoipa::path(
    get,
    path = "/api/v1/admin/domains/{id}/vhost",
    tag = "domains",
    operation_id = "adminGetDomainVhost",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Domain id")),
    responses(
        (status = 200, description = "The file and the block", body = VhostView),
        (status = 403, description = "`permission_denied`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn admin_get(
    State(state): State<DomainsState>,
    _who: Require<DomainsRead>,
    Path(id): Path<Uuid>,
) -> Result<Json<VhostView>, AppError> {
    let h = tenant_of_domain(&state, id).await?;
    show(&state, &h, id).await
}

/// Replace a domain's own directives (an empty text removes them).
#[utoipa::path(
    put,
    path = "/api/v1/admin/domains/{id}/vhost",
    tag = "domains",
    operation_id = "adminSetDomainVhost",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = SetVhost,
    responses(
        (status = 200, description = "Saved", body = VhostView),
        (status = 403, description = "`permission_denied`, `reauth_required`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "`forbidden_directive` (naming it), `reserved_header`, `custom_too_large`, `custom_characters`, `template_syntax`, `template_braces` or `alias_has_no_file`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn admin_set(
    State(state): State<DomainsState>,
    RequireRecent(who, _): RequireRecent<DomainsManage>,
    Path(id): Path<Uuid>,
    Json(req): Json<SetVhost>,
) -> Result<Json<VhostView>, AppError> {
    let h = tenant_of_domain(&state, id).await?;
    set(&state, &who, &h, id, req).await
}

/// See the web server file of one of your domains and your own directives.
#[utoipa::path(
    get,
    path = "/api/v1/client/domains/{id}/vhost",
    tag = "domains",
    operation_id = "clientGetDomainVhost",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Domain id")),
    responses(
        (status = 200, description = "The file and the block", body = VhostView),
        (status = 403, description = "`client_only` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain (or it is not yours)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn client_get(
    State(state): State<DomainsState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<Json<VhostView>, AppError> {
    let h = client_hosting(&state, &who).await?;
    require_feature(&state, &h).await?;
    show(&state, &h, id).await
}

/// Replace your own directives for one of your domains (an empty text removes them).
#[utoipa::path(
    put,
    path = "/api/v1/client/domains/{id}/vhost",
    tag = "domains",
    operation_id = "clientSetDomainVhost",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = SetVhost,
    responses(
        (status = 200, description = "Saved", body = VhostView),
        (status = 403, description = "`client_only`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain (or it is not yours)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "`forbidden_directive` (naming it), `reserved_header`, `custom_too_large`, `custom_characters`, `template_syntax`, `template_braces` or `alias_has_no_file`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn client_set(
    State(state): State<DomainsState>,
    who: Principal,
    Path(id): Path<Uuid>,
    Json(req): Json<SetVhost>,
) -> Result<Json<VhostView>, AppError> {
    let h = client_hosting(&state, &who).await?;
    set(&state, &who, &h, id, req).await
}
