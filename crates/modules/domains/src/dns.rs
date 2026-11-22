//! DNS zones of domains and the templates they start from (T05.08).
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET, POST | `/api/v1/admin/dns/templates` | `admin.domains.read` / `.manage`, sudo mode |
//! | GET, PUT, DELETE | `/api/v1/admin/dns/templates/{id}` | `admin.domains.read` / `.manage`, sudo mode |
//!
//! Adding a main, addon or alias domain creates its zone from the default
//! template (in the same transaction as the domain), with the server's
//! addresses filled in; adding a subdomain puts its address records into the
//! nearest zone of the same account. Pushing zones to the DNS server is the job
//! of the provider adapter (T05.09): a new zone is `pending` until then.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use builtpanel_audit::{AuditEntry, Outcome};
use builtpanel_database::{
    dns::{self as store, DeleteOutcome, StoredLine, Template, TemplateError},
    domains::Domain,
    sqlx::PgConnection,
};
use builtpanel_domain::{
    DnsRecord, DnsTemplateRecord, DnsVariables, DomainKind, Fqdn, check_dns_template,
    render_dns_template,
};
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_rbac::{
    Require, RequireRecent,
    permissions::{DomainsManage, DomainsRead},
};
use builtpanel_session::rfc3339;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::{DomainsState, Hosting, internal};

/// Routes for administrators.
pub fn admin_router() -> Router<DomainsState> {
    Router::new()
        .route(
            "/api/v1/admin/dns/templates",
            get(list_templates).post(create_template),
        )
        .route(
            "/api/v1/admin/dns/templates/{id}",
            get(get_template)
                .put(update_template)
                .delete(delete_template),
        )
}

/// Makes the DNS side of a new domain; returns a note for the audit entry.
///
/// A main, addon or alias domain gets a zone from the default template. A
/// template that no longer renders (it was valid when saved; the rules changed)
/// does not stop the domain: the zone is made empty and the note says why.
///
/// # Errors
/// Database errors.
pub(crate) async fn provision(
    conn: &mut PgConnection,
    hosting: &Hosting,
    domain: &Domain,
    fqdn: &Fqdn,
) -> Result<serde_json::Value, AppError> {
    let (ipv4, ipv6) = store::node_addresses(conn, hosting.tenant.node_id)
        .await
        .map_err(internal)?;
    match domain.kind {
        DomainKind::Sub => {
            let Some(zone) = store::nearest_zone(conn, hosting.tenant.id, fqdn.as_str())
                .await
                .map_err(internal)?
            else {
                return Ok(serde_json::json!({ "zone": null }));
            };
            let relative = fqdn
                .as_str()
                .strip_suffix(&format!(".{}", zone.name))
                .unwrap_or_default()
                .to_owned();
            let mut records = Vec::new();
            if let Some(ip) = ipv4 {
                records.extend(DnsRecord::parse(&relative, "A", &ip.to_string(), None, None).ok());
            }
            if let Some(ip) = ipv6 {
                records
                    .extend(DnsRecord::parse(&relative, "AAAA", &ip.to_string(), None, None).ok());
            }
            store::insert_records(conn, zone.id, &records)
                .await
                .map_err(internal)?;
            Ok(serde_json::json!({ "zone": zone.name, "records": records.len() }))
        }
        DomainKind::Primary | DomainKind::Addon | DomainKind::Alias => {
            let template = store::default_template(conn).await.map_err(internal)?;
            let (records, error) = match template {
                Some(t) => {
                    let lines: Vec<DnsTemplateRecord> =
                        t.records.into_iter().map(Into::into).collect();
                    match render_dns_template(
                        &lines,
                        &DnsVariables {
                            domain: fqdn.as_str().to_owned(),
                            ipv4,
                            ipv6,
                        },
                    ) {
                        Ok(r) => (r, None),
                        Err(e) => (Vec::new(), Some(e.to_string())),
                    }
                }
                None => (Vec::new(), None),
            };
            let zone = store::create_zone(conn, domain.id, fqdn.as_str(), &records)
                .await
                .map_err(internal)?;
            Ok(serde_json::json!({
                "zone": zone.name,
                "records": records.len(),
                "template_error": error,
            }))
        }
    }
}

/// Removes what [`provision`] put into a parent's zone for a subdomain.
///
/// # Errors
/// Database errors.
pub(crate) async fn forget_subdomain(
    conn: &mut PgConnection,
    hosting: &Hosting,
    fqdn: &str,
) -> Result<(), AppError> {
    let Some(zone) = store::nearest_zone(conn, hosting.tenant.id, fqdn)
        .await
        .map_err(internal)?
    else {
        return Ok(());
    };
    if let Some(relative) = fqdn.strip_suffix(&format!(".{}", zone.name)) {
        store::remove_host_records(conn, zone.id, relative)
            .await
            .map_err(internal)?;
    }
    Ok(())
}

// ------------------------------------------------------------- the templates

/// One line of a template.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TemplateLine {
    /// Relative to the zone: `@` for the zone itself, `www`, `_dmarc`, ...
    #[schema(example = "www")]
    pub name: String,
    /// `A`, `AAAA`, `CNAME`, `MX`, `TXT`, `SRV`, `CAA` or `NS`.
    #[serde(rename = "type")]
    #[schema(example = "CNAME")]
    pub rtype: String,
    /// The data. May use `{{domain}}`, `{{ipv4}}` and `{{ipv6}}`; a line whose
    /// address the server does not have is left out.
    #[schema(example = "{{domain}}.")]
    pub content: String,
    /// Seconds (60 to 604800; default 3600).
    pub ttl: Option<u32>,
    /// For `MX` and `SRV`.
    pub priority: Option<u32>,
}

impl From<StoredLine> for TemplateLine {
    fn from(l: StoredLine) -> Self {
        Self {
            name: l.name,
            rtype: l.rtype,
            content: l.content,
            ttl: l.ttl,
            priority: l.priority,
        }
    }
}

impl From<TemplateLine> for StoredLine {
    fn from(l: TemplateLine) -> Self {
        Self {
            name: l.name,
            rtype: l.rtype,
            content: l.content,
            ttl: l.ttl,
            priority: l.priority,
        }
    }
}

/// A zone template.
#[derive(Debug, Serialize, ToSchema)]
pub struct DnsTemplateView {
    /// Id.
    pub id: Uuid,
    /// Unique name.
    #[schema(example = "default")]
    pub name: String,
    /// Description.
    pub description: String,
    /// The lines.
    pub records: Vec<TemplateLine>,
    /// New zones start from this one.
    pub is_default: bool,
    /// Made by the installation: it can be changed but not deleted.
    pub built_in: bool,
    /// Last change (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub updated_at: String,
}

fn view(t: Template) -> DnsTemplateView {
    DnsTemplateView {
        id: t.id,
        name: t.name,
        description: t.description,
        records: t.records.into_iter().map(Into::into).collect(),
        is_default: t.is_default,
        built_in: t.built_in,
        updated_at: rfc3339(t.updated_at),
    }
}

/// The templates.
#[derive(Debug, Serialize, ToSchema)]
pub struct DnsTemplateList {
    /// The default first, then by name.
    pub items: Vec<DnsTemplateView>,
}

/// A template to write.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DnsTemplateInput {
    /// Unique name: lower-case letters, digits and `_`, 2-32 characters.
    #[schema(example = "web_only")]
    pub name: String,
    /// What it is for.
    #[serde(default)]
    pub description: String,
    /// The lines (at most 100).
    pub records: Vec<TemplateLine>,
    /// Make it the template new zones start from.
    #[serde(default)]
    pub is_default: bool,
}

fn checked(input: &DnsTemplateInput) -> Result<Vec<StoredLine>, AppError> {
    let mut errors = Vec::new();
    let name_ok = (2..=32).contains(&input.name.len())
        && input.name.starts_with(|c: char| c.is_ascii_lowercase())
        && input
            .name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if !name_ok {
        errors.push(FieldError::new(
            "name",
            "invalid_name",
            "2-32 characters: a-z, 0-9 and _, starting with a letter.",
        ));
    }
    if input.description.chars().count() > 255 {
        errors.push(FieldError::new(
            "description",
            "too_long",
            "At most 255 characters.",
        ));
    }
    let lines: Vec<DnsTemplateRecord> = input
        .records
        .iter()
        .cloned()
        .map(StoredLine::from)
        .map(Into::into)
        .collect();
    if let Err(e) = check_dns_template(&lines) {
        errors.push(FieldError::new("records", e.code, e.message));
    }
    if errors.is_empty() {
        Ok(input
            .records
            .iter()
            .cloned()
            .map(StoredLine::from)
            .collect())
    } else {
        Err(AppError::validation(errors))
    }
}

fn write_error(err: TemplateError) -> AppError {
    match err {
        TemplateError::NameTaken => {
            AppError::conflict("template_name_taken", "A template with this name exists.")
        }
        TemplateError::Database(db) => internal(db),
    }
}

fn audit(
    action: &'static str,
    who: &builtpanel_rbac::Principal,
    id: Uuid,
    name: &str,
) -> AuditEntry {
    AuditEntry::new(action, Outcome::Success)
        .actor(who.account_id())
        .target("dns_template", id)
        .metadata(serde_json::json!({ "name": name }))
}

/// List the zone templates.
#[utoipa::path(
    get,
    path = "/api/v1/admin/dns/templates",
    tag = "domains",
    operation_id = "listDnsTemplates",
    security(("session" = [])),
    responses(
        (status = 200, description = "The templates", body = DnsTemplateList),
        (status = 403, description = "`permission_denied` (needs admin.domains.read)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn list_templates(
    State(state): State<DomainsState>,
    _who: Require<DomainsRead>,
) -> Result<Json<DnsTemplateList>, AppError> {
    Ok(Json(DnsTemplateList {
        items: store::templates(&state.db)
            .await
            .map_err(internal)?
            .into_iter()
            .map(view)
            .collect(),
    }))
}

/// One zone template.
#[utoipa::path(
    get,
    path = "/api/v1/admin/dns/templates/{id}",
    tag = "domains",
    operation_id = "getDnsTemplate",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Template id")),
    responses(
        (status = 200, description = "The template", body = DnsTemplateView),
        (status = 403, description = "`permission_denied`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such template", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn get_template(
    State(state): State<DomainsState>,
    _who: Require<DomainsRead>,
    Path(id): Path<Uuid>,
) -> Result<Json<DnsTemplateView>, AppError> {
    store::template(&state.db, id)
        .await
        .map_err(internal)?
        .map(view)
        .map(Json)
        .ok_or_else(|| AppError::not_found("template"))
}

/// Create a zone template.
#[utoipa::path(
    post,
    path = "/api/v1/admin/dns/templates",
    tag = "domains",
    operation_id = "createDnsTemplate",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = DnsTemplateInput,
    responses(
        (status = 201, description = "Created", body = DnsTemplateView),
        (status = 403, description = "`permission_denied`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`template_name_taken`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid name or records (the message names the record)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn create_template(
    State(state): State<DomainsState>,
    RequireRecent(who, _): RequireRecent<DomainsManage>,
    Json(input): Json<DnsTemplateInput>,
) -> Result<(StatusCode, Json<DnsTemplateView>), AppError> {
    let lines = checked(&input)?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    let id = store::create_template(
        &mut tx,
        &input.name,
        &input.description,
        &lines,
        input.is_default,
    )
    .await
    .map_err(write_error)?;
    builtpanel_audit::insert(
        &mut *tx,
        &audit("admin.dns_template_created", &who, id, &input.name),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    let created = store::template(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| internal(std::io::Error::other("created template vanished")))?;
    Ok((StatusCode::CREATED, Json(view(created))))
}

/// Replace a zone template. Zones that already exist are not changed.
#[utoipa::path(
    put,
    path = "/api/v1/admin/dns/templates/{id}",
    tag = "domains",
    operation_id = "updateDnsTemplate",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Template id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = DnsTemplateInput,
    responses(
        (status = 200, description = "Saved", body = DnsTemplateView),
        (status = 403, description = "`permission_denied`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such template", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`template_name_taken`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid name or records", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn update_template(
    State(state): State<DomainsState>,
    RequireRecent(who, _): RequireRecent<DomainsManage>,
    Path(id): Path<Uuid>,
    Json(input): Json<DnsTemplateInput>,
) -> Result<Json<DnsTemplateView>, AppError> {
    let lines = checked(&input)?;
    let existing = store::template(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("template"))?;
    // The installation's own template keeps its name.
    if existing.built_in && existing.name != input.name {
        return Err(AppError::validation(vec![FieldError::new(
            "name",
            "built_in_name",
            "The built-in template cannot be renamed.",
        )]));
    }
    let mut tx = state.db.begin().await.map_err(internal)?;
    if !store::update_template(
        &mut tx,
        id,
        &input.name,
        &input.description,
        &lines,
        input.is_default,
    )
    .await
    .map_err(write_error)?
    {
        return Err(AppError::not_found("template"));
    }
    builtpanel_audit::insert(
        &mut *tx,
        &audit("admin.dns_template_updated", &who, id, &input.name),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    store::template(&state.db, id)
        .await
        .map_err(internal)?
        .map(view)
        .map(Json)
        .ok_or_else(|| AppError::not_found("template"))
}

/// Delete a zone template.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/dns/templates/{id}",
    tag = "domains",
    operation_id = "deleteDnsTemplate",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Template id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 204, description = "Deleted"),
        (status = 403, description = "`permission_denied`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such template", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`template_built_in` or `template_is_default` (make another one the default first)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn delete_template(
    State(state): State<DomainsState>,
    RequireRecent(who, _): RequireRecent<DomainsManage>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let name = store::template(&state.db, id)
        .await
        .map_err(internal)?
        .map(|t| t.name)
        .unwrap_or_default();
    let mut tx = state.db.begin().await.map_err(internal)?;
    match store::delete_template(&mut tx, id)
        .await
        .map_err(internal)?
    {
        DeleteOutcome::Deleted => {}
        DeleteOutcome::NotFound => return Err(AppError::not_found("template")),
        DeleteOutcome::BuiltIn => {
            return Err(AppError::conflict(
                "template_built_in",
                "The built-in template cannot be deleted.",
            ));
        }
        DeleteOutcome::IsDefault => {
            return Err(AppError::conflict(
                "template_is_default",
                "New zones start from this template; make another one the default first.",
            ));
        }
    }
    builtpanel_audit::insert(
        &mut *tx,
        &audit("admin.dns_template_deleted", &who, id, &name),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}
