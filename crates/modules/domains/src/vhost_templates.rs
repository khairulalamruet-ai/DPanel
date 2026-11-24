//! The web server templates every site's file is made from (T06.07).
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET, POST | `/api/v1/admin/vhost-templates` | `admin.domains.read` / `.manage`, sudo mode |
//! | GET, PUT, DELETE | `/api/v1/admin/vhost-templates/{id}` | `admin.domains.read` / `.manage`, sudo mode |
//! | GET | `/api/v1/admin/vhost-templates/{id}/versions` | `admin.domains.read` |
//! | POST | `/api/v1/admin/vhost-templates/{id}/default` | `admin.domains.manage`, sudo mode |
//! | POST | `/api/v1/admin/vhost-templates/preview` | `admin.domains.read` |
//! | POST | `/api/v1/admin/vhost-templates/apply` | `admin.domains.manage`, sudo mode |
//!
//! Every save is a new version and the older ones stay readable. Saving a
//! template does not touch the sites that already exist: `apply` makes every
//! account with a web server send its files again (the same pass that follows any
//! change of a domain), so the new text reaches the nodes a few accounts at a
//! time. A template that does not pass the checks of
//! [`builtpanel_domain::vhost::check_template`] is refused with 422 and the reason.
//! `preview` fills a template in for a made-up site that uses every part
//! (aliases, policy, the owner's own block) and shows the result.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use builtpanel_audit::{AuditEntry, Outcome};
use builtpanel_database::vhost_templates::{self as store, VhostTemplate, WriteError};
use builtpanel_domain::{
    Docroot, Fqdn,
    vhost::{self, Header, MAX_TEMPLATE_BYTES, Policy, Site},
};
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_rbac::{
    Principal, Require, RequireRecent,
    permissions::{DomainsManage, DomainsRead},
};
use builtpanel_session::rfc3339;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::{DomainsState, internal};

/// Routes for administrators.
pub fn admin_router() -> Router<DomainsState> {
    Router::new()
        .route("/api/v1/admin/vhost-templates", get(list).post(create))
        .route("/api/v1/admin/vhost-templates/preview", post(preview))
        .route("/api/v1/admin/vhost-templates/apply", post(apply))
        .route(
            "/api/v1/admin/vhost-templates/{id}",
            get(show).put(update).delete(remove),
        )
        .route("/api/v1/admin/vhost-templates/{id}/versions", get(versions))
        .route(
            "/api/v1/admin/vhost-templates/{id}/default",
            post(set_default),
        )
}

/// A template, without its text (for lists).
#[derive(Debug, Serialize, ToSchema)]
pub struct TemplateSummary {
    /// Id.
    pub id: Uuid,
    /// Name.
    #[schema(example = "Default")]
    pub name: String,
    /// The web server it is for (`nginx`).
    pub server: String,
    /// Whether new sites use it.
    pub is_default: bool,
    /// Which save it is (1 for the first).
    pub version: i32,
    /// Last change (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub updated_at: String,
}

/// A template with its text.
#[derive(Debug, Serialize, ToSchema)]
pub struct TemplateView {
    /// Id.
    pub id: Uuid,
    /// Name.
    pub name: String,
    /// The web server it is for (`nginx`).
    pub server: String,
    /// Whether new sites use it.
    pub is_default: bool,
    /// Which save it is.
    pub version: i32,
    /// Last change (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub updated_at: String,
    /// The configuration with `{{variables}}`: `domain`, `server_names`,
    /// `docroot`, `php_upstream`, `policy_http`, `policy` and `custom`.
    pub body: String,
}

fn summary(t: &VhostTemplate) -> TemplateSummary {
    TemplateSummary {
        id: t.id,
        name: t.name.clone(),
        server: t.server.clone(),
        is_default: t.is_default,
        version: t.version,
        updated_at: rfc3339(t.updated_at),
    }
}

fn full(t: VhostTemplate) -> TemplateView {
    TemplateView {
        id: t.id,
        updated_at: rfc3339(t.updated_at),
        name: t.name,
        server: t.server,
        is_default: t.is_default,
        version: t.version,
        body: t.body,
    }
}

/// The templates.
#[derive(Debug, Serialize, ToSchema)]
pub struct TemplateList {
    /// By name.
    pub items: Vec<TemplateSummary>,
}

/// A template to save.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TemplateInput {
    /// Its name: letters, digits, spaces, `.`, `_` and `-`, up to 64.
    #[schema(example = "Default")]
    pub name: String,
    /// The configuration, up to 32 KiB.
    pub body: String,
}

/// One saved version.
#[derive(Debug, Serialize, ToSchema)]
pub struct VersionView {
    /// Which save.
    pub version: i32,
    /// The name then.
    pub name: String,
    /// The text then.
    pub body: String,
    /// Who saved it, if that account still exists.
    pub changed_by: Option<Uuid>,
    /// When (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub created_at: String,
}

/// A template's history.
#[derive(Debug, Serialize, ToSchema)]
pub struct VersionList {
    /// Newest first (at most 50).
    pub items: Vec<VersionView>,
}

/// A template to try.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PreviewInput {
    /// The configuration to fill in.
    pub body: String,
}

/// What a template makes for a made-up site.
#[derive(Debug, Serialize, ToSchema)]
pub struct Preview {
    /// The file.
    pub rendered: String,
}

/// How many accounts will send their files again.
#[derive(Debug, Serialize, ToSchema)]
pub struct Applied {
    /// Accounts with a web server that were put on the list.
    pub tenants: u64,
}

fn checked(input: &TemplateInput) -> Result<(), AppError> {
    let mut errors = Vec::new();
    let plain = input.name.len() <= 64
        && input.name.starts_with(|c: char| c.is_ascii_alphanumeric())
        && input
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || " ._-".contains(c));
    if !plain {
        errors.push(FieldError::new(
            "name",
            "invalid_name",
            "A name is letters, digits, spaces, dots, dashes and underscores, up to 64.",
        ));
    }
    if let Err(e) = vhost::check_template(&input.body) {
        errors.push(FieldError::new("body", e.code, e.message));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(AppError::validation(errors))
    }
}

fn write_error(err: WriteError) -> AppError {
    match err {
        WriteError::NameTaken => {
            AppError::conflict("template_name_taken", "A template with this name exists.")
        }
        WriteError::Database(db) => internal(db),
    }
}

fn audit(
    action: &'static str,
    who: &Principal,
    id: Option<Uuid>,
    meta: serde_json::Value,
) -> AuditEntry {
    let entry = AuditEntry::new(action, Outcome::Success)
        .actor(who.account_id())
        .metadata(meta);
    match id {
        Some(id) => entry.target("vhost_template", id),
        None => entry,
    }
}

/// List the web server templates.
#[utoipa::path(
    get,
    path = "/api/v1/admin/vhost-templates",
    tag = "domains",
    operation_id = "listVhostTemplates",
    security(("session" = [])),
    responses(
        (status = 200, description = "The templates", body = TemplateList),
        (status = 403, description = "`permission_denied`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn list(
    State(state): State<DomainsState>,
    _who: Require<DomainsRead>,
) -> Result<Json<TemplateList>, AppError> {
    Ok(Json(TemplateList {
        items: store::list(&state.db, "nginx")
            .await
            .map_err(internal)?
            .iter()
            .map(summary)
            .collect(),
    }))
}

/// One template with its text.
#[utoipa::path(
    get,
    path = "/api/v1/admin/vhost-templates/{id}",
    tag = "domains",
    operation_id = "getVhostTemplate",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Template id")),
    responses(
        (status = 200, description = "The template", body = TemplateView),
        (status = 403, description = "`permission_denied`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such template", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn show(
    State(state): State<DomainsState>,
    _who: Require<DomainsRead>,
    Path(id): Path<Uuid>,
) -> Result<Json<TemplateView>, AppError> {
    store::get(&state.db, id)
        .await
        .map_err(internal)?
        .map(full)
        .map(Json)
        .ok_or_else(|| AppError::not_found("template"))
}

/// Add a template.
#[utoipa::path(
    post,
    path = "/api/v1/admin/vhost-templates",
    tag = "domains",
    operation_id = "createVhostTemplate",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = TemplateInput,
    responses(
        (status = 201, description = "Created", body = TemplateView),
        (status = 403, description = "`permission_denied`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`template_name_taken`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid name or text (the code says what is wrong with the text)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn create(
    State(state): State<DomainsState>,
    RequireRecent(who, _): RequireRecent<DomainsManage>,
    Json(input): Json<TemplateInput>,
) -> Result<(StatusCode, Json<TemplateView>), AppError> {
    checked(&input)?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    let id = store::create(&mut tx, &input.name, &input.body, Some(who.account_id()))
        .await
        .map_err(write_error)?;
    builtpanel_audit::insert(
        &mut *tx,
        &audit(
            "admin.vhost_template_created",
            &who,
            Some(id),
            serde_json::json!({ "name": input.name }),
        ),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    let created = store::get(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| internal(std::io::Error::other("created template vanished")))?;
    Ok((StatusCode::CREATED, Json(full(created))))
}

/// Save a template (its next version). Sites that exist are not changed until
/// the templates are applied.
#[utoipa::path(
    put,
    path = "/api/v1/admin/vhost-templates/{id}",
    tag = "domains",
    operation_id = "updateVhostTemplate",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Template id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = TemplateInput,
    responses(
        (status = 200, description = "Saved", body = TemplateView),
        (status = 403, description = "`permission_denied`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such template", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`template_name_taken`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid name or text", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn update(
    State(state): State<DomainsState>,
    RequireRecent(who, _): RequireRecent<DomainsManage>,
    Path(id): Path<Uuid>,
    Json(input): Json<TemplateInput>,
) -> Result<Json<TemplateView>, AppError> {
    checked(&input)?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    let version = store::update(
        &mut tx,
        id,
        &input.name,
        &input.body,
        Some(who.account_id()),
    )
    .await
    .map_err(write_error)?
    .ok_or_else(|| AppError::not_found("template"))?;
    builtpanel_audit::insert(
        &mut *tx,
        &audit(
            "admin.vhost_template_updated",
            &who,
            Some(id),
            serde_json::json!({ "name": input.name, "version": version }),
        ),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    show_one(&state, id).await
}

async fn show_one(state: &DomainsState, id: Uuid) -> Result<Json<TemplateView>, AppError> {
    store::get(&state.db, id)
        .await
        .map_err(internal)?
        .map(full)
        .map(Json)
        .ok_or_else(|| AppError::not_found("template"))
}

/// Remove a template that is not the default.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/vhost-templates/{id}",
    tag = "domains",
    operation_id = "deleteVhostTemplate",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Template id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 204, description = "Removed"),
        (status = 403, description = "`permission_denied`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such template", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`template_is_default`: make another the default first", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn remove(
    State(state): State<DomainsState>,
    RequireRecent(who, _): RequireRecent<DomainsManage>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let existing = store::get(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("template"))?;
    if existing.is_default {
        return Err(AppError::conflict(
            "template_is_default",
            "This is the template sites use. Make another one the default first.",
        ));
    }
    let mut tx = state.db.begin().await.map_err(internal)?;
    if !store::delete(&mut tx, id).await.map_err(internal)? {
        return Err(AppError::not_found("template"));
    }
    builtpanel_audit::insert(
        &mut *tx,
        &audit(
            "admin.vhost_template_deleted",
            &who,
            Some(id),
            serde_json::json!({ "name": existing.name }),
        ),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// A template's earlier saves.
#[utoipa::path(
    get,
    path = "/api/v1/admin/vhost-templates/{id}/versions",
    tag = "domains",
    operation_id = "listVhostTemplateVersions",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Template id")),
    responses(
        (status = 200, description = "Newest first", body = VersionList),
        (status = 403, description = "`permission_denied`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such template", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn versions(
    State(state): State<DomainsState>,
    _who: Require<DomainsRead>,
    Path(id): Path<Uuid>,
) -> Result<Json<VersionList>, AppError> {
    store::get(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("template"))?;
    Ok(Json(VersionList {
        items: store::versions(&state.db, id)
            .await
            .map_err(internal)?
            .into_iter()
            .map(|v| VersionView {
                version: v.version,
                name: v.name,
                body: v.body,
                changed_by: v.changed_by,
                created_at: rfc3339(v.created_at),
            })
            .collect(),
    }))
}

/// Make a template the one new sites use. Sites that exist change when the
/// templates are applied.
#[utoipa::path(
    post,
    path = "/api/v1/admin/vhost-templates/{id}/default",
    tag = "domains",
    operation_id = "makeVhostTemplateDefault",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Template id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 200, description = "Done", body = TemplateView),
        (status = 403, description = "`permission_denied`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such template", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn set_default(
    State(state): State<DomainsState>,
    RequireRecent(who, _): RequireRecent<DomainsManage>,
    Path(id): Path<Uuid>,
) -> Result<Json<TemplateView>, AppError> {
    let mut tx = state.db.begin().await.map_err(internal)?;
    if !store::make_default(&mut tx, id).await.map_err(internal)? {
        return Err(AppError::not_found("template"));
    }
    builtpanel_audit::insert(
        &mut *tx,
        &audit(
            "admin.vhost_template_default",
            &who,
            Some(id),
            serde_json::json!({}),
        ),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    show_one(&state, id).await
}

/// Fill a template in for a made-up site that uses every part, without saving anything.
#[utoipa::path(
    post,
    path = "/api/v1/admin/vhost-templates/preview",
    tag = "domains",
    operation_id = "previewVhostTemplate",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = PreviewInput,
    responses(
        (status = 200, description = "What it makes", body = Preview),
        (status = 403, description = "`permission_denied` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "The text does not pass the checks", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn preview(
    State(_state): State<DomainsState>,
    _who: Require<DomainsRead>,
    Json(input): Json<PreviewInput>,
) -> Result<Json<Preview>, AppError> {
    let bad = |e: vhost::VhostError| {
        AppError::validation(vec![FieldError::new("body", e.code, e.message)])
    };
    let sample_error = |e: &dyn std::fmt::Display| internal(std::io::Error::other(e.to_string()));
    let domain = Fqdn::parse("example.com").map_err(|e| sample_error(&e))?;
    let aliases = [Fqdn::parse("www.example.com").map_err(|e| sample_error(&e))?];
    let docroot = Docroot::parse("public_html").map_err(|e| sample_error(&e))?;
    let policy = Policy::parse(
        Some(10),
        vec![Header {
            name: "X-Example".into(),
            value: "preview".into(),
        }],
    )
    .map_err(bad)?;
    let rendered = vhost::render(
        &input.body,
        &Site {
            domain: &domain,
            aliases: &aliases,
            docroot: &docroot,
            php_upstream: "bp-example-php:9000",
            policy: &policy,
            custom: "location /old { return 301 /new; }",
            proxy: None,
        },
    )
    .map_err(bad)?;
    Ok(Json(Preview { rendered }))
}

/// Send every account's web server files again (after templates changed).
#[utoipa::path(
    post,
    path = "/api/v1/admin/vhost-templates/apply",
    tag = "domains",
    operation_id = "applyVhostTemplates",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses(
        (status = 202, description = "Accounts with a web server are on the list; the files go out a few at a time", body = Applied),
        (status = 403, description = "`permission_denied`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn apply(
    State(state): State<DomainsState>,
    RequireRecent(who, _): RequireRecent<DomainsManage>,
) -> Result<(StatusCode, Json<Applied>), AppError> {
    let mut tx = state.db.begin().await.map_err(internal)?;
    let tenants = store::resend_all(&mut tx).await.map_err(internal)?;
    builtpanel_audit::insert(
        &mut *tx,
        &audit(
            "admin.vhost_templates_applied",
            &who,
            None,
            serde_json::json!({ "tenants": tenants }),
        ),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok((StatusCode::ACCEPTED, Json(Applied { tenants })))
}

/// The longest template, in bytes (for the editor).
pub const MAX_BYTES: usize = MAX_TEMPLATE_BYTES;
