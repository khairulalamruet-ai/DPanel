//! Temporary preview addresses (T06.14).
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET, POST | `/api/v1/client/domains/{id}/previews` | the owner of the domain |
//! | DELETE | `/api/v1/client/domains/{id}/previews/{preview_id}` | the owner of the domain |
//!
//! A preview link is `<random>.<preview domain>`: it shows the site of one domain
//! before the domain's own name points to this panel. It works for the number of
//! hours the administrator set (`preview_hours`, 24 by default) and then stops
//! answering. An account may have at most five at a time. Without a
//! `preview_domain` setting there are no previews (409 `previews_off`). Aliases and
//! suspended domains have none.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get},
};
use builtpanel_database::previews;
use builtpanel_domain::{DomainKind, DomainStatus, Fqdn};
use builtpanel_http_kit::{AppError, ProblemDetails};
use builtpanel_session::rfc3339;
use builtpanel_settings::Scope;
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{DomainsState, client_hosting, entry, internal, owned_domain};

/// Most links an account may have at once.
pub const MAX_ACTIVE: i64 = 5;

/// Routes for clients.
pub fn client_router() -> Router<DomainsState> {
    Router::new()
        .route(
            "/api/v1/client/domains/{id}/previews",
            get(list).post(create),
        )
        .route(
            "/api/v1/client/domains/{id}/previews/{preview_id}",
            delete(remove),
        )
}

/// A preview link.
#[derive(Debug, Serialize, ToSchema)]
pub struct PreviewView {
    /// Id.
    pub id: Uuid,
    /// The address to open (over http or https).
    #[schema(example = "k3j9x0a1b2c3d4e5.preview.example.net")]
    pub host: String,
    /// It stops working at this time (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub expires_at: String,
}

/// The links of a domain.
#[derive(Debug, Serialize, ToSchema)]
pub struct PreviewList {
    /// Newest first; expired ones are gone.
    pub items: Vec<PreviewView>,
    /// Whether the panel offers previews at all.
    pub enabled: bool,
}

fn view(p: previews::Preview) -> PreviewView {
    PreviewView {
        id: p.id,
        host: p.host,
        expires_at: rfc3339(p.expires_at),
    }
}

/// The preview domain and how long a link lives.
async fn settings(state: &DomainsState) -> Result<(Option<String>, i32), AppError> {
    let mut conn = state.db.acquire().await.map_err(internal)?;
    let values = builtpanel_settings::get(&state.settings, &mut conn, "domains", Scope::Global)
        .await
        .map_err(internal)?;
    let domain = values
        .get("preview_domain")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(str::to_owned);
    let hours = values
        .get("preview_hours")
        .and_then(serde_json::Value::as_i64)
        .and_then(|h| i32::try_from(h).ok())
        .unwrap_or(24)
        .clamp(1, 168);
    Ok((domain, hours))
}

/// See the preview links of one of your domains.
#[utoipa::path(
    get,
    path = "/api/v1/client/domains/{id}/previews",
    tag = "domains",
    operation_id = "clientListPreviews",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Domain id")),
    responses(
        (status = 200, description = "The links that still work", body = PreviewList),
        (status = 403, description = "`client_only`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain (or it is not yours)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn list(
    State(state): State<DomainsState>,
    who: builtpanel_rbac::Principal,
    Path(id): Path<Uuid>,
) -> Result<Json<PreviewList>, AppError> {
    let hosting = client_hosting(&state, &who).await?;
    owned_domain(&state, hosting.tenant.id, id).await?;
    let enabled = settings(&state).await?.0.is_some();
    Ok(Json(PreviewList {
        items: previews::active(&state.db, id)
            .await
            .map_err(internal)?
            .into_iter()
            .map(view)
            .collect(),
        enabled,
    }))
}

/// Make a preview link for one of your domains.
#[utoipa::path(
    post,
    path = "/api/v1/client/domains/{id}/previews",
    tag = "domains",
    operation_id = "clientCreatePreview",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 201, description = "Made; it works until `expires_at`", body = PreviewView),
        (status = 403, description = "`client_only` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain (or it is not yours)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`previews_off`, `preview_domain_invalid`, `limit_reached`, `no_preview_for_alias` or `domain_suspended`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn create(
    State(state): State<DomainsState>,
    who: builtpanel_rbac::Principal,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<PreviewView>), AppError> {
    let hosting = client_hosting(&state, &who).await?;
    let domain = owned_domain(&state, hosting.tenant.id, id).await?;
    let (Some(preview_domain), hours) = settings(&state).await? else {
        return Err(AppError::conflict(
            "previews_off",
            "This panel does not offer preview links.",
        ));
    };
    if domain.domain.kind == DomainKind::Alias {
        return Err(AppError::conflict(
            "no_preview_for_alias",
            "An alias shows its domain's site: make the link for the domain.",
        ));
    }
    if domain.domain.status == DomainStatus::Suspended {
        return Err(AppError::conflict(
            "domain_suspended",
            "A suspended domain has no preview.",
        ));
    }
    let mut tx = state.db.begin().await.map_err(internal)?;
    // One at a time per account, so two requests cannot together pass the limit.
    builtpanel_database::domains::lock_tenant(&mut tx, hosting.tenant.id)
        .await
        .map_err(internal)?;
    if previews::count_active(&mut tx, hosting.tenant.id)
        .await
        .map_err(internal)?
        >= MAX_ACTIVE
    {
        return Err(AppError::conflict(
            "limit_reached",
            format!("An account may have at most {MAX_ACTIVE} preview links at a time."),
        ));
    }
    let mut made = None;
    for _ in 0..5 {
        let random = Uuid::new_v4().simple().to_string();
        let host = format!("{}.{preview_domain}", &random[..16]);
        // What the panel makes is a name like any other: checked the same way.
        if Fqdn::parse(&host).map_or(true, |f| f.as_str() != host) {
            return Err(internal(std::io::Error::other(
                "the preview domain is not a name",
            )));
        }
        made = previews::create(
            &mut tx,
            hosting.tenant.id,
            id,
            &host,
            hours,
            Some(who.account_id()),
        )
        .await
        .map_err(internal)?;
        if made.is_some() {
            break;
        }
    }
    let made = made.ok_or_else(|| internal(std::io::Error::other("no free preview name")))?;
    builtpanel_audit::insert(
        &mut *tx,
        &entry("domain.preview_created", &who, &hosting, id)
            .metadata(serde_json::json!({ "fqdn": domain.domain.fqdn, "hours": hours })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok((StatusCode::CREATED, Json(view(made))))
}

/// Remove a preview link before it expires.
#[utoipa::path(
    delete,
    path = "/api/v1/client/domains/{id}/previews/{preview_id}",
    tag = "domains",
    operation_id = "clientDeletePreview",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("preview_id" = Uuid, Path, description = "Link id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 204, description = "Removed"),
        (status = 403, description = "`client_only` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain or link", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn remove(
    State(state): State<DomainsState>,
    who: builtpanel_rbac::Principal,
    Path((id, preview_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    let hosting = client_hosting(&state, &who).await?;
    let domain = owned_domain(&state, hosting.tenant.id, id).await?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    if !previews::delete(&mut tx, id, preview_id)
        .await
        .map_err(internal)?
    {
        return Err(AppError::not_found("preview"));
    }
    builtpanel_audit::insert(
        &mut *tx,
        &entry("domain.preview_deleted", &who, &hosting, id)
            .metadata(serde_json::json!({ "fqdn": domain.domain.fqdn })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}
