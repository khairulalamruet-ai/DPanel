//! The pages visitors see when a site cannot answer (T06.13).
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET | `/api/v1/admin/error-pages` | `admin.domains.read` |
//! | PUT, DELETE | `/api/v1/admin/error-pages/{kind}` | `admin.domains.manage`, sudo mode |
//!
//! Four pages: `suspended_domain`, `suspended_account`, `not_found` and
//! `bad_gateway` (a site whose web server does not answer). Each starts as the
//! installation's own plain page; `PUT` replaces it with HTML of at most 16 KiB,
//! `DELETE` goes back to the plain page. A change is sent to every node a moment
//! later.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use builtpanel_database::error_pages as store;
use builtpanel_domain::error_pages::{self, KINDS, Kind, MAX_PAGE_BYTES};
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_rbac::{
    Require, RequireRecent,
    permissions::{DomainsManage, DomainsRead},
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::{DomainsState, internal};

/// Routes for administrators.
pub fn admin_router() -> Router<DomainsState> {
    Router::new()
        .route("/api/v1/admin/error-pages", get(list))
        .route(
            "/api/v1/admin/error-pages/{kind}",
            axum::routing::put(set).delete(reset),
        )
}

/// One page.
#[derive(Debug, Serialize, ToSchema)]
pub struct PageView {
    /// `suspended_domain`, `suspended_account`, `not_found` or `bad_gateway`.
    #[schema(example = "not_found")]
    pub kind: String,
    /// Whether an administrator replaced the installation's page.
    pub is_custom: bool,
    /// The HTML visitors get.
    pub html: String,
}

/// The pages.
#[derive(Debug, Serialize, ToSchema)]
pub struct PageList {
    /// One of each kind.
    pub items: Vec<PageView>,
    /// Longest page, in bytes.
    #[schema(example = 16384)]
    pub max_bytes: usize,
}

/// A page to keep.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PageInput {
    /// The HTML.
    pub html: String,
}

async fn all(state: &DomainsState) -> Result<PageList, AppError> {
    let custom = store::custom(&state.db).await.map_err(internal)?;
    Ok(PageList {
        items: KINDS
            .iter()
            .map(|k| {
                let own = custom.iter().find(|(name, _)| name == k.as_str());
                PageView {
                    kind: k.as_str().to_owned(),
                    is_custom: own.is_some(),
                    html: own.map_or_else(|| k.default_page(), |(_, html)| html.clone()),
                }
            })
            .collect(),
        max_bytes: MAX_PAGE_BYTES,
    })
}

fn kind_of(text: &str) -> Result<Kind, AppError> {
    Kind::parse(text).ok_or_else(|| AppError::not_found("page"))
}

fn audit(
    action: &'static str,
    who: &builtpanel_rbac::Principal,
    kind: &str,
) -> builtpanel_audit::AuditEntry {
    builtpanel_audit::AuditEntry::new(action, builtpanel_audit::Outcome::Success)
        .actor(who.account_id())
        .metadata(serde_json::json!({ "kind": kind }))
}

/// The pages visitors see, with the installation's own where none was set.
#[utoipa::path(
    get,
    path = "/api/v1/admin/error-pages",
    tag = "domains",
    operation_id = "listErrorPages",
    security(("session" = [])),
    responses(
        (status = 200, description = "The four pages", body = PageList),
        (status = 403, description = "`permission_denied`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn list(
    State(state): State<DomainsState>,
    _who: Require<DomainsRead>,
) -> Result<Json<PageList>, AppError> {
    Ok(Json(all(&state).await?))
}

/// Replace a page.
#[utoipa::path(
    put,
    path = "/api/v1/admin/error-pages/{kind}",
    tag = "domains",
    operation_id = "setErrorPage",
    security(("session" = [])),
    params(
        ("kind" = String, Path, description = "`suspended_domain`, `suspended_account`, `not_found` or `bad_gateway`"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = PageInput,
    responses(
        (status = 200, description = "Saved", body = PageList),
        (status = 403, description = "`permission_denied`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such page", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "`page_empty`, `page_too_large` or `page_characters`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn set(
    State(state): State<DomainsState>,
    RequireRecent(who, _): RequireRecent<DomainsManage>,
    Path(kind): Path<String>,
    Json(input): Json<PageInput>,
) -> Result<Json<PageList>, AppError> {
    let kind = kind_of(&kind)?;
    error_pages::check(&input.html).map_err(|(code, message)| {
        AppError::validation(vec![FieldError::new("html", code, message)])
    })?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    store::set(&mut tx, kind.as_str(), &input.html, Some(who.account_id()))
        .await
        .map_err(internal)?;
    builtpanel_audit::insert(
        &mut *tx,
        &audit("admin.error_page_changed", &who, kind.as_str()),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(all(&state).await?))
}

/// Go back to the installation's own page.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/error-pages/{kind}",
    tag = "domains",
    operation_id = "resetErrorPage",
    security(("session" = [])),
    params(
        ("kind" = String, Path, description = "The kind of page"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 204, description = "Done (also when it already was the plain page)"),
        (status = 403, description = "`permission_denied`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such page", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn reset(
    State(state): State<DomainsState>,
    RequireRecent(who, _): RequireRecent<DomainsManage>,
    Path(kind): Path<String>,
) -> Result<StatusCode, AppError> {
    let kind = kind_of(&kind)?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    if store::reset(&mut tx, kind.as_str())
        .await
        .map_err(internal)?
    {
        builtpanel_audit::insert(
            &mut *tx,
            &audit("admin.error_page_reset", &who, kind.as_str()),
        )
        .await
        .map_err(internal)?;
    }
    tx.commit().await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}
