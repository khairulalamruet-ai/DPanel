//! DNS records of a domain's zone (T05.10).
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET, POST | `/api/v1/admin/domains/{id}/dns/records` | `admin.domains.read` / `.manage`, sudo mode |
//! | PATCH, DELETE | `/api/v1/admin/domains/{id}/dns/records/{record_id}` | `admin.domains.manage`, sudo mode |
//! | GET, POST | `/api/v1/client/domains/{id}/dns/records` | the owner of the domain |
//! | PATCH, DELETE | `/api/v1/client/domains/{id}/dns/records/{record_id}` | the owner of the domain |
//!
//! The records are those of the domain's zone, named relative to it (`@` is the
//! zone itself). A subdomain has no zone of its own: its records are in its
//! parent's zone, so asking for them on the subdomain is a 409 `no_zone`.
//!
//! A bad record is a 422 that names the field and says what is wrong
//! (`builtpanel_domain::dns` has the rules per type: addresses, targets, TTL
//! 60..604800, priority for MX and SRV, no CNAME or apex NS). A record that
//! clashes with the zone as it is, a CNAME beside other records or the same
//! record twice, is a 409 with its own code. Changing records needs the `dns`
//! feature in the plan; looking does not. Every change marks the zone as changed
//! so the sync pushes it to the DNS server (`status` in the answer says where
//! that stands).

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, patch},
};
use builtpanel_database::{
    dns::{self as store, RecordWriteError, StoredRecord, Zone},
    plans,
};
use builtpanel_domain::{DnsRecord, DnsRecordError, DomainKind, check_dns_conflicts};
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

/// Most records one zone may have.
pub const MAX_PER_ZONE: i64 = 1000;

/// Routes for administrators.
pub fn admin_router() -> Router<DomainsState> {
    Router::new()
        .route(
            "/api/v1/admin/domains/{id}/dns/records",
            get(admin_list).post(admin_add),
        )
        .route(
            "/api/v1/admin/domains/{id}/dns/records/{record_id}",
            patch(admin_update).delete(admin_delete),
        )
}

/// Routes for clients.
pub fn client_router() -> Router<DomainsState> {
    Router::new()
        .route(
            "/api/v1/client/domains/{id}/dns/records",
            get(client_list).post(client_add),
        )
        .route(
            "/api/v1/client/domains/{id}/dns/records/{record_id}",
            patch(client_update).delete(client_delete),
        )
}

/// A DNS record.
#[derive(Debug, Serialize, ToSchema)]
pub struct RecordView {
    /// Id.
    pub id: Uuid,
    /// Relative to the zone: `@`, `www`, `_dmarc`, ...
    #[schema(example = "www")]
    pub name: String,
    /// The full name, `www.example.com`.
    #[schema(example = "www.example.com")]
    pub fqdn: String,
    /// `A`, `AAAA`, `CNAME`, `MX`, `TXT`, `SRV`, `CAA` or `NS`.
    #[serde(rename = "type")]
    #[schema(example = "A")]
    pub rtype: String,
    /// The data (for an `MX`, the mail server; the priority is separate).
    #[schema(example = "192.0.2.10")]
    pub content: String,
    /// Seconds a resolver may keep the answer.
    #[schema(example = 3600)]
    pub ttl: u32,
    /// For `MX` and `SRV`.
    pub priority: Option<u16>,
}

fn view(zone: &Zone, r: StoredRecord) -> RecordView {
    let name = r.record.name;
    RecordView {
        id: r.id,
        fqdn: if name == "@" {
            zone.name.clone()
        } else {
            format!("{name}.{}", zone.name)
        },
        name,
        rtype: r.record.rtype.as_str().to_owned(),
        content: r.record.content,
        ttl: r.record.ttl,
        priority: r.record.priority,
    }
}

/// How far the zone has got on the DNS server.
#[derive(Debug, Serialize, ToSchema)]
pub struct ZoneView {
    /// The zone's name.
    #[schema(example = "example.com")]
    pub name: String,
    /// `pending` (changes not on the server yet), `active` or `error`.
    #[schema(example = "active")]
    pub status: String,
    /// Why the last push was refused.
    pub last_error: Option<String>,
}

fn zone_view(z: &Zone) -> ZoneView {
    ZoneView {
        name: z.name.clone(),
        status: z.status.clone(),
        last_error: z.last_error.clone(),
    }
}

/// A zone with its records.
#[derive(Debug, Serialize, ToSchema)]
pub struct RecordList {
    /// The zone.
    pub zone: ZoneView,
    /// The records, by name and type.
    pub items: Vec<RecordView>,
}

/// A record to add.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AddRecord {
    /// Relative to the zone; `@` for the zone itself.
    #[schema(example = "www")]
    pub name: String,
    /// `A`, `AAAA`, `CNAME`, `MX`, `TXT`, `SRV`, `CAA` or `NS`.
    #[serde(rename = "type")]
    #[schema(example = "A")]
    pub rtype: String,
    /// The data: an address for `A` and `AAAA`, a host name for `CNAME`, `MX`
    /// (the mail server) and `NS`, text for `TXT`, `weight port target` for `SRV`,
    /// `flags tag "value"` for `CAA`.
    #[schema(example = "192.0.2.10")]
    pub content: String,
    /// Seconds, 60 to 604800 (default 3600).
    pub ttl: Option<u32>,
    /// Required for `MX` and `SRV`, refused for the others.
    pub priority: Option<u32>,
}

/// Changes to a record (its name and type stay: remove it and add another).
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateRecord {
    /// New data.
    pub content: Option<String>,
    /// New TTL.
    pub ttl: Option<u32>,
    /// New priority (`MX` and `SRV`).
    pub priority: Option<u32>,
}

fn invalid(e: &DnsRecordError) -> AppError {
    AppError::validation(vec![FieldError::new(e.field, e.code, e.message.clone())])
}

fn clash(e: &DnsRecordError) -> AppError {
    AppError::conflict(e.code, e.message.clone())
}

pub(crate) async fn zone_of(
    state: &DomainsState,
    domain_id: Uuid,
    kind: DomainKind,
) -> Result<Zone, AppError> {
    let none = || {
        AppError::conflict(
            "no_zone",
            "This domain has no zone of its own; a subdomain's records are in its parent domain's zone.",
        )
    };
    if kind == DomainKind::Sub {
        return Err(none());
    }
    store::zone_of_domain(&state.db, domain_id)
        .await
        .map_err(internal)?
        .ok_or_else(none)
}

async fn require_dns(state: &DomainsState, hosting: &Hosting) -> Result<(), AppError> {
    if plans::includes_feature(&state.db, hosting.plan.id, "dns")
        .await
        .map_err(internal)?
    {
        Ok(())
    } else {
        Err(AppError::forbidden(
            "feature_disabled",
            "DNS editing is not included in this plan.",
        ))
    }
}

async fn list_records(
    state: &DomainsState,
    hosting: &Hosting,
    domain_id: Uuid,
) -> Result<Json<RecordList>, AppError> {
    let domain = owned_domain(state, hosting.tenant.id, domain_id).await?;
    let zone = zone_of(state, domain_id, domain.domain.kind).await?;
    let items = store::records(&state.db, zone.id)
        .await
        .map_err(internal)?
        .into_iter()
        .map(|r| view(&zone, r))
        .collect();
    Ok(Json(RecordList {
        zone: zone_view(&zone),
        items,
    }))
}

async fn add_record(
    state: &DomainsState,
    who: &Principal,
    hosting: &Hosting,
    domain_id: Uuid,
    req: AddRecord,
) -> Result<RecordView, AppError> {
    let domain = owned_domain(state, hosting.tenant.id, domain_id).await?;
    let zone = zone_of(state, domain_id, domain.domain.kind).await?;
    require_dns(state, hosting).await?;
    let record = DnsRecord::parse(&req.name, &req.rtype, &req.content, req.ttl, req.priority)
        .map_err(|e| invalid(&e))?;

    let mut tx = state.db.begin().await.map_err(internal)?;
    builtpanel_database::domains::lock_tenant(&mut tx, hosting.tenant.id)
        .await
        .map_err(internal)?;
    let existing = store::records_in(&mut tx, zone.id)
        .await
        .map_err(internal)?;
    if i64::try_from(existing.len()).unwrap_or(i64::MAX) >= MAX_PER_ZONE {
        return Err(AppError::conflict(
            "limit_reached",
            format!("A zone may have at most {MAX_PER_ZONE} records."),
        ));
    }
    check_dns_conflicts(existing.iter().map(|r| &r.record), &record).map_err(|e| clash(&e))?;
    let id = store::add_record(&mut tx, zone.id, &record)
        .await
        .map_err(|e| match e {
            RecordWriteError::Duplicate => {
                AppError::conflict("duplicate_record", "This record exists already.")
            }
            RecordWriteError::Database(db) => internal(db),
        })?;
    store::mark_changed(&mut tx, zone.id)
        .await
        .map_err(internal)?;
    builtpanel_audit::insert(
        &mut *tx,
        &entry("dns.record_added", who, hosting, domain_id).metadata(serde_json::json!({
            "zone": zone.name,
            "name": record.name,
            "type": record.rtype.as_str(),
            "content": record.content,
        })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(view(
        &zone,
        StoredRecord {
            id,
            zone_id: zone.id,
            record,
        },
    ))
}

async fn update_record(
    state: &DomainsState,
    who: &Principal,
    hosting: &Hosting,
    domain_id: Uuid,
    record_id: Uuid,
    req: UpdateRecord,
) -> Result<RecordView, AppError> {
    let domain = owned_domain(state, hosting.tenant.id, domain_id).await?;
    let zone = zone_of(state, domain_id, domain.domain.kind).await?;
    require_dns(state, hosting).await?;
    if req.content.is_none() && req.ttl.is_none() && req.priority.is_none() {
        return Err(AppError::validation(vec![FieldError::new(
            "content",
            "nothing_to_change",
            "Give content, a TTL or a priority to change.",
        )]));
    }
    let mut tx = state.db.begin().await.map_err(internal)?;
    builtpanel_database::domains::lock_tenant(&mut tx, hosting.tenant.id)
        .await
        .map_err(internal)?;
    let existing = store::records_in(&mut tx, zone.id)
        .await
        .map_err(internal)?;
    let current = existing
        .iter()
        .find(|r| r.id == record_id)
        .ok_or_else(|| AppError::not_found("record"))?;
    let changed = DnsRecord::parse(
        &current.record.name,
        current.record.rtype.as_str(),
        req.content.as_deref().unwrap_or(&current.record.content),
        Some(req.ttl.unwrap_or(current.record.ttl)),
        req.priority.or(current.record.priority.map(u32::from)),
    )
    .map_err(|e| invalid(&e))?;
    check_dns_conflicts(
        existing
            .iter()
            .filter(|r| r.id != record_id)
            .map(|r| &r.record),
        &changed,
    )
    .map_err(|e| clash(&e))?;
    if !store::change_record(&mut tx, zone.id, record_id, &changed)
        .await
        .map_err(|e| match e {
            RecordWriteError::Duplicate => {
                AppError::conflict("duplicate_record", "This record exists already.")
            }
            RecordWriteError::Database(db) => internal(db),
        })?
    {
        return Err(AppError::not_found("record"));
    }
    store::mark_changed(&mut tx, zone.id)
        .await
        .map_err(internal)?;
    builtpanel_audit::insert(
        &mut *tx,
        &entry("dns.record_changed", who, hosting, domain_id).metadata(serde_json::json!({
            "zone": zone.name,
            "name": changed.name,
            "type": changed.rtype.as_str(),
            "from": current.record.content,
            "to": changed.content,
        })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(view(
        &zone,
        StoredRecord {
            id: record_id,
            zone_id: zone.id,
            record: changed,
        },
    ))
}

async fn delete_record(
    state: &DomainsState,
    who: &Principal,
    hosting: &Hosting,
    domain_id: Uuid,
    record_id: Uuid,
) -> Result<(), AppError> {
    let domain = owned_domain(state, hosting.tenant.id, domain_id).await?;
    let zone = zone_of(state, domain_id, domain.domain.kind).await?;
    require_dns(state, hosting).await?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    let existing = store::records_in(&mut tx, zone.id)
        .await
        .map_err(internal)?;
    let current = existing
        .iter()
        .find(|r| r.id == record_id)
        .ok_or_else(|| AppError::not_found("record"))?;
    if !store::delete_record(&mut tx, zone.id, record_id)
        .await
        .map_err(internal)?
    {
        return Err(AppError::not_found("record"));
    }
    store::mark_changed(&mut tx, zone.id)
        .await
        .map_err(internal)?;
    builtpanel_audit::insert(
        &mut *tx,
        &entry("dns.record_deleted", who, hosting, domain_id).metadata(serde_json::json!({
            "zone": zone.name,
            "name": current.record.name,
            "type": current.record.rtype.as_str(),
            "content": current.record.content,
        })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(())
}

/// List the records of a domain's zone.
#[utoipa::path(
    get,
    path = "/api/v1/admin/domains/{id}/dns/records",
    tag = "domains",
    operation_id = "adminListDnsRecords",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Domain id")),
    responses(
        (status = 200, description = "The zone and its records", body = RecordList),
        (status = 403, description = "`permission_denied`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`no_zone`: a subdomain's records are in its parent's zone", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn admin_list(
    State(state): State<DomainsState>,
    _who: Require<DomainsRead>,
    Path(id): Path<Uuid>,
) -> Result<Json<RecordList>, AppError> {
    let h = tenant_of_domain(&state, id).await?;
    list_records(&state, &h, id).await
}

/// Add a record to a domain's zone.
#[utoipa::path(
    post,
    path = "/api/v1/admin/domains/{id}/dns/records",
    tag = "domains",
    operation_id = "adminAddDnsRecord",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = AddRecord,
    responses(
        (status = 201, description = "Added", body = RecordView),
        (status = 403, description = "`permission_denied`, `reauth_required`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`no_zone`, `cname_conflict`, `duplicate_record` or `limit_reached`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "The record is not valid: one error with the field and what is wrong", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn admin_add(
    State(state): State<DomainsState>,
    RequireRecent(who, _): RequireRecent<DomainsManage>,
    Path(id): Path<Uuid>,
    Json(req): Json<AddRecord>,
) -> Result<(StatusCode, Json<RecordView>), AppError> {
    let h = tenant_of_domain(&state, id).await?;
    Ok((
        StatusCode::CREATED,
        Json(add_record(&state, &who, &h, id, req).await?),
    ))
}

/// Change a record's data, TTL or priority.
#[utoipa::path(
    patch,
    path = "/api/v1/admin/domains/{id}/dns/records/{record_id}",
    tag = "domains",
    operation_id = "adminUpdateDnsRecord",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("record_id" = Uuid, Path, description = "Record id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = UpdateRecord,
    responses(
        (status = 200, description = "Updated", body = RecordView),
        (status = 403, description = "`permission_denied`, `reauth_required`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain or record", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`no_zone`, `cname_conflict` or `duplicate_record`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Nothing to change, or the record would not be valid", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn admin_update(
    State(state): State<DomainsState>,
    RequireRecent(who, _): RequireRecent<DomainsManage>,
    Path((id, record_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<UpdateRecord>,
) -> Result<Json<RecordView>, AppError> {
    let h = tenant_of_domain(&state, id).await?;
    Ok(Json(
        update_record(&state, &who, &h, id, record_id, req).await?,
    ))
}

/// Remove a record.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/domains/{id}/dns/records/{record_id}",
    tag = "domains",
    operation_id = "adminDeleteDnsRecord",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("record_id" = Uuid, Path, description = "Record id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 204, description = "Removed"),
        (status = 403, description = "`permission_denied`, `reauth_required`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain or record", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`no_zone`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn admin_delete(
    State(state): State<DomainsState>,
    RequireRecent(who, _): RequireRecent<DomainsManage>,
    Path((id, record_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    let h = tenant_of_domain(&state, id).await?;
    delete_record(&state, &who, &h, id, record_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// List the records of one of your domains' zones.
#[utoipa::path(
    get,
    path = "/api/v1/client/domains/{id}/dns/records",
    tag = "domains",
    operation_id = "clientListDnsRecords",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Domain id")),
    responses(
        (status = 200, description = "The zone and its records", body = RecordList),
        (status = 403, description = "`client_only`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain (or it is not yours)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`no_zone`: a subdomain's records are in its parent's zone", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn client_list(
    State(state): State<DomainsState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<Json<RecordList>, AppError> {
    let h = client_hosting(&state, &who).await?;
    list_records(&state, &h, id).await
}

/// Add a record to one of your domains' zones.
#[utoipa::path(
    post,
    path = "/api/v1/client/domains/{id}/dns/records",
    tag = "domains",
    operation_id = "clientAddDnsRecord",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = AddRecord,
    responses(
        (status = 201, description = "Added", body = RecordView),
        (status = 403, description = "`client_only`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain (or it is not yours)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`no_zone`, `cname_conflict`, `duplicate_record` or `limit_reached`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "The record is not valid: one error with the field and what is wrong", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn client_add(
    State(state): State<DomainsState>,
    who: Principal,
    Path(id): Path<Uuid>,
    Json(req): Json<AddRecord>,
) -> Result<(StatusCode, Json<RecordView>), AppError> {
    let h = client_hosting(&state, &who).await?;
    Ok((
        StatusCode::CREATED,
        Json(add_record(&state, &who, &h, id, req).await?),
    ))
}

/// Change a record of one of your domains' zones.
#[utoipa::path(
    patch,
    path = "/api/v1/client/domains/{id}/dns/records/{record_id}",
    tag = "domains",
    operation_id = "clientUpdateDnsRecord",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("record_id" = Uuid, Path, description = "Record id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = UpdateRecord,
    responses(
        (status = 200, description = "Updated", body = RecordView),
        (status = 403, description = "`client_only`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain or record (or it is not yours)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`no_zone`, `cname_conflict` or `duplicate_record`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Nothing to change, or the record would not be valid", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn client_update(
    State(state): State<DomainsState>,
    who: Principal,
    Path((id, record_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<UpdateRecord>,
) -> Result<Json<RecordView>, AppError> {
    let h = client_hosting(&state, &who).await?;
    Ok(Json(
        update_record(&state, &who, &h, id, record_id, req).await?,
    ))
}

/// Remove a record of one of your domains' zones.
#[utoipa::path(
    delete,
    path = "/api/v1/client/domains/{id}/dns/records/{record_id}",
    tag = "domains",
    operation_id = "clientDeleteDnsRecord",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("record_id" = Uuid, Path, description = "Record id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 204, description = "Removed"),
        (status = 403, description = "`client_only`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain or record (or it is not yours)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`no_zone`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn client_delete(
    State(state): State<DomainsState>,
    who: Principal,
    Path((id, record_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    let h = client_hosting(&state, &who).await?;
    delete_record(&state, &who, &h, id, record_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
