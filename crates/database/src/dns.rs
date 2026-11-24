//! DNS zones, records and zone templates (T05.08).

use std::net::{Ipv4Addr, Ipv6Addr};

use builtpanel_domain::{DnsRecord, DnsRecordType, DnsTemplateRecord};
use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, PgPool, types::Uuid};

/// A zone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Zone {
    /// Id.
    pub id: Uuid,
    /// The domain it belongs to.
    pub domain_id: Uuid,
    /// The zone's name (the domain's), lower-case ASCII.
    pub name: String,
    /// `pending`, `active` or `error`.
    pub status: String,
    /// Why the last push to the DNS server failed.
    pub last_error: Option<String>,
    /// Changes whenever the zone row does: a push only marks the zone done if
    /// this is still what it started from (otherwise something changed meanwhile).
    pub version: String,
}

/// A stored record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRecord {
    /// Id.
    pub id: Uuid,
    /// The zone.
    pub zone_id: Uuid,
    /// The checked record.
    pub record: DnsRecord,
}

/// The JSON form of a template line in `dns_zone_templates.records`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredLine {
    /// Relative name.
    pub name: String,
    /// Type, as written.
    #[serde(rename = "type")]
    pub rtype: String,
    /// Content with variables.
    pub content: String,
    /// Time to live.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl: Option<u32>,
    /// Priority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<u32>,
}

impl From<StoredLine> for DnsTemplateRecord {
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

impl From<DnsTemplateRecord> for StoredLine {
    fn from(l: DnsTemplateRecord) -> Self {
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
#[derive(Debug, Clone, PartialEq)]
pub struct Template {
    /// Id.
    pub id: Uuid,
    /// Unique name.
    pub name: String,
    /// Description.
    pub description: String,
    /// The lines.
    pub records: Vec<StoredLine>,
    /// New zones start from this one.
    pub is_default: bool,
    /// Made by the installation: it can be changed, not deleted.
    pub built_in: bool,
    /// Last change (Unix seconds).
    pub updated_at: i64,
}

struct TemplateRow {
    id: Uuid,
    name: String,
    description: String,
    records: serde_json::Value,
    is_default: bool,
    built_in: bool,
    updated_at: i64,
}

impl From<TemplateRow> for Template {
    fn from(r: TemplateRow) -> Self {
        Self {
            id: r.id,
            name: r.name,
            description: r.description,
            // Written by `write_template` after validation; a row that does not
            // read back is shown as empty rather than hiding the template.
            records: serde_json::from_value(r.records).unwrap_or_default(),
            is_default: r.is_default,
            built_in: r.built_in,
            updated_at: r.updated_at,
        }
    }
}

/// Why writing a template failed.
#[derive(Debug, thiserror::Error)]
pub enum TemplateError {
    /// Another template has this name.
    #[error("a template with this name exists")]
    NameTaken,
    /// Any other database error.
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// Every template, the default first.
///
/// # Errors
/// Database errors.
pub async fn templates(pool: &PgPool) -> Result<Vec<Template>, sqlx::Error> {
    let rows = sqlx::query_as!(
        TemplateRow,
        r#"SELECT id, name, description, records, is_default, built_in,
                  extract(epoch FROM updated_at)::bigint AS "updated_at!"
           FROM dns_zone_templates ORDER BY is_default DESC, name"#
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(Template::from).collect())
}

/// One template.
///
/// # Errors
/// Database errors.
pub async fn template(pool: &PgPool, id: Uuid) -> Result<Option<Template>, sqlx::Error> {
    let row = sqlx::query_as!(
        TemplateRow,
        r#"SELECT id, name, description, records, is_default, built_in,
                  extract(epoch FROM updated_at)::bigint AS "updated_at!"
           FROM dns_zone_templates WHERE id = $1"#,
        id,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(Template::from))
}

/// The template new zones start from.
///
/// # Errors
/// Database errors.
pub async fn default_template(conn: &mut PgConnection) -> Result<Option<Template>, sqlx::Error> {
    let row = sqlx::query_as!(
        TemplateRow,
        r#"SELECT id, name, description, records, is_default, built_in,
                  extract(epoch FROM updated_at)::bigint AS "updated_at!"
           FROM dns_zone_templates WHERE is_default"#
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(Template::from))
}

fn name_error(err: sqlx::Error) -> TemplateError {
    match err.as_database_error().and_then(|e| e.constraint()) {
        Some("dns_zone_templates_name_key") => TemplateError::NameTaken,
        _ => TemplateError::Database(err),
    }
}

/// Creates a template (the lines already checked). With `is_default` it becomes
/// the default and the previous default stops being it.
///
/// # Errors
/// [`TemplateError`].
pub async fn create_template(
    conn: &mut PgConnection,
    name: &str,
    description: &str,
    records: &[StoredLine],
    is_default: bool,
) -> Result<Uuid, TemplateError> {
    if is_default {
        sqlx::query!("UPDATE dns_zone_templates SET is_default = false WHERE is_default")
            .execute(&mut *conn)
            .await?;
    }
    let json = serde_json::to_value(records).map_err(|e| sqlx::Error::Encode(Box::new(e)))?;
    sqlx::query_scalar!(
        "INSERT INTO dns_zone_templates (name, description, records, is_default)
         VALUES ($1, $2, $3, $4) RETURNING id",
        name,
        description,
        json,
        is_default,
    )
    .fetch_one(conn)
    .await
    .map_err(name_error)
}

/// Replaces a template's contents. Returns whether it exists. A template that
/// is the default stays the default (make another one the default instead).
///
/// # Errors
/// [`TemplateError`].
pub async fn update_template(
    conn: &mut PgConnection,
    id: Uuid,
    name: &str,
    description: &str,
    records: &[StoredLine],
    make_default: bool,
) -> Result<bool, TemplateError> {
    if make_default {
        sqlx::query!(
            "UPDATE dns_zone_templates SET is_default = false WHERE is_default AND id <> $1",
            id
        )
        .execute(&mut *conn)
        .await?;
    }
    let json = serde_json::to_value(records).map_err(|e| sqlx::Error::Encode(Box::new(e)))?;
    let done = sqlx::query!(
        "UPDATE dns_zone_templates
         SET name = $2, description = $3, records = $4, is_default = is_default OR $5
         WHERE id = $1",
        id,
        name,
        description,
        json,
        make_default,
    )
    .execute(conn)
    .await
    .map_err(name_error)?;
    Ok(done.rows_affected() == 1)
}

/// What deleting a template did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteOutcome {
    /// Gone.
    Deleted,
    /// No such template.
    NotFound,
    /// Made by the installation.
    BuiltIn,
    /// New zones start from it.
    IsDefault,
}

/// Deletes a template that is neither built in nor the default.
///
/// # Errors
/// Database errors.
pub async fn delete_template(
    conn: &mut PgConnection,
    id: Uuid,
) -> Result<DeleteOutcome, sqlx::Error> {
    let row = sqlx::query!(
        "SELECT built_in, is_default FROM dns_zone_templates WHERE id = $1 FOR UPDATE",
        id,
    )
    .fetch_optional(&mut *conn)
    .await?;
    let Some(row) = row else {
        return Ok(DeleteOutcome::NotFound);
    };
    if row.built_in {
        return Ok(DeleteOutcome::BuiltIn);
    }
    if row.is_default {
        return Ok(DeleteOutcome::IsDefault);
    }
    sqlx::query!("DELETE FROM dns_zone_templates WHERE id = $1", id)
        .execute(conn)
        .await?;
    Ok(DeleteOutcome::Deleted)
}

/// The address(es) a node answers on, for the template variables.
///
/// # Errors
/// Database errors.
pub async fn node_addresses(
    conn: &mut PgConnection,
    node_id: Uuid,
) -> Result<(Option<Ipv4Addr>, Option<Ipv6Addr>), sqlx::Error> {
    let ip = sqlx::query_scalar!(
        "SELECT host(public_ip) FROM nodes WHERE id = $1 AND public_ip IS NOT NULL",
        node_id,
    )
    .fetch_optional(conn)
    .await?
    .flatten();
    Ok(match ip {
        Some(text) => (text.parse().ok(), text.parse().ok()),
        None => (None, None),
    })
}

/// Creates a zone and its records.
///
/// # Errors
/// Database errors.
pub async fn create_zone(
    conn: &mut PgConnection,
    domain_id: Uuid,
    name: &str,
    records: &[DnsRecord],
) -> Result<Zone, sqlx::Error> {
    let zone = sqlx::query_as!(
        Zone,
        "INSERT INTO dns_zones (domain_id, name) VALUES ($1, $2)
         RETURNING id, domain_id, name, status, last_error, updated_at::text AS \"version!\"",
        domain_id,
        name,
    )
    .fetch_one(&mut *conn)
    .await?;
    // A zone of this name that was removed a moment ago is back: the server still
    // has it, and the next sync makes it match.
    sqlx::query!("DELETE FROM dns_zone_removals WHERE name = $1", name)
        .execute(&mut *conn)
        .await?;
    insert_records(conn, zone.id, records).await?;
    Ok(zone)
}

/// Adds records to a zone.
///
/// # Errors
/// Database errors (a duplicate is a unique violation).
pub async fn insert_records(
    conn: &mut PgConnection,
    zone_id: Uuid,
    records: &[DnsRecord],
) -> Result<(), sqlx::Error> {
    for r in records {
        sqlx::query!(
            "INSERT INTO dns_records (zone_id, name, rtype, content, ttl, priority)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT DO NOTHING",
            zone_id,
            r.name,
            r.rtype.as_str(),
            r.content,
            i32::try_from(r.ttl).unwrap_or(i32::MAX),
            r.priority.map(i32::from),
        )
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// The zone of a domain.
///
/// # Errors
/// Database errors.
pub async fn zone_of_domain(pool: &PgPool, domain_id: Uuid) -> Result<Option<Zone>, sqlx::Error> {
    sqlx::query_as!(
        Zone,
        "SELECT id, domain_id, name, status, last_error, updated_at::text AS \"version!\" FROM dns_zones WHERE domain_id = $1",
        domain_id,
    )
    .fetch_optional(pool)
    .await
}

/// The zone of a tenant that is the closest ancestor of `fqdn`: for
/// `a.b.example.com` the zone `b.example.com` if the tenant has it, else
/// `example.com`.
///
/// # Errors
/// Database errors.
pub async fn nearest_zone(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    fqdn: &str,
) -> Result<Option<Zone>, sqlx::Error> {
    sqlx::query_as!(
        Zone,
        "SELECT z.id, z.domain_id, z.name, z.status, z.last_error, z.updated_at::text AS \"version!\"
         FROM dns_zones z JOIN domains d ON d.id = z.domain_id
         WHERE d.tenant_id = $1 AND $2 LIKE '%.' || z.name
         ORDER BY length(z.name) DESC LIMIT 1",
        tenant_id,
        fqdn,
    )
    .fetch_optional(conn)
    .await
}

/// The records of a zone, by name and type.
///
/// # Errors
/// Database errors.
pub async fn records(pool: &PgPool, zone_id: Uuid) -> Result<Vec<StoredRecord>, sqlx::Error> {
    let rows = sqlx::query!(
        "SELECT id, zone_id, name, rtype, content, ttl, priority
         FROM dns_records WHERE zone_id = $1 ORDER BY name, rtype, content",
        zone_id,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            Some(StoredRecord {
                id: r.id,
                zone_id: r.zone_id,
                record: DnsRecord {
                    name: r.name,
                    rtype: DnsRecordType::parse(&r.rtype).ok()?,
                    content: r.content,
                    ttl: u32::try_from(r.ttl).ok()?,
                    priority: r.priority.and_then(|p| u16::try_from(p).ok()),
                },
            })
        })
        .collect())
}

/// Every DNS record of the zones of a tenant's domains, as the JSON a backup keeps
/// (`dns.json`): `[{"zone", "name", "type", "content", "ttl", "priority"}]`, ordered.
///
/// # Errors
/// Database errors.
pub async fn backup_records(
    pool: &PgPool,
    tenant_id: Uuid,
) -> Result<serde_json::Value, sqlx::Error> {
    let rows = sqlx::query!(
        "SELECT z.name AS zone, r.name, r.rtype, r.content, r.ttl, r.priority
         FROM dns_zones z
         JOIN domains d ON d.id = z.domain_id
         JOIN dns_records r ON r.zone_id = z.id
         WHERE d.tenant_id = $1
         ORDER BY z.name, r.name, r.rtype, r.content",
        tenant_id,
    )
    .fetch_all(pool)
    .await?;
    Ok(serde_json::Value::Array(
        rows.into_iter()
            .map(|r| {
                serde_json::json!({
                    "zone": r.zone,
                    "name": r.name,
                    "type": r.rtype,
                    "content": r.content,
                    "ttl": r.ttl,
                    "priority": r.priority,
                })
            })
            .collect(),
    ))
}

/// How many records a zone may hold (the same limit as when records are added by hand).
const MAX_RECORDS_PER_ZONE: usize = 1000;

/// Puts the records of a backup (the JSON of [`backup_records`]) back into the zones of the
/// tenant's domains: the records a zone has now are replaced by the ones in the backup. Only
/// zones that still belong to the tenant are touched; a zone that is gone is skipped, and so is
/// a record that is not valid today or clashes with one already taken. Each zone's records as
/// they were are returned in the summary, so that nothing is lost for good.
///
/// Call inside a transaction (after [`crate::domains::lock_tenant`]).
///
/// # Errors
/// Database errors.
pub async fn restore_records(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    records: &serde_json::Value,
) -> Result<serde_json::Value, sqlx::Error> {
    let mut by_zone: std::collections::BTreeMap<String, Vec<&serde_json::Value>> =
        std::collections::BTreeMap::new();
    for record in records.as_array().map_or(&[][..], Vec::as_slice) {
        if let Some(zone) = record.get("zone").and_then(serde_json::Value::as_str) {
            by_zone.entry(zone.to_owned()).or_default().push(record);
        }
    }
    let (mut missing, mut skipped, mut added, mut removed) = (Vec::new(), 0_u64, 0_u64, 0_u64);
    let mut zones = Vec::new();
    for (name, wanted) in by_zone {
        let zone_id = sqlx::query_scalar!(
            "SELECT z.id FROM dns_zones z JOIN domains d ON d.id = z.domain_id
             WHERE z.name = $1 AND d.tenant_id = $2",
            name,
            tenant_id
        )
        .fetch_optional(&mut *conn)
        .await?;
        let Some(zone_id) = zone_id else {
            missing.push(name);
            continue;
        };
        let mut accepted: Vec<DnsRecord> = Vec::new();
        for r in wanted {
            let text = |key: &str| r.get(key).and_then(serde_json::Value::as_str);
            let number = |key: &str| {
                r.get(key)
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|n| u32::try_from(n).ok())
            };
            let parsed = DnsRecord::parse(
                text("name").unwrap_or_default(),
                text("type").unwrap_or_default(),
                text("content").unwrap_or_default(),
                number("ttl"),
                number("priority"),
            );
            match parsed {
                Ok(record)
                    if accepted.len() < MAX_RECORDS_PER_ZONE
                        && builtpanel_domain::check_dns_conflicts(accepted.iter(), &record)
                            .is_ok() =>
                {
                    accepted.push(record);
                }
                _ => skipped += 1,
            }
        }
        let before = records_in(&mut *conn, zone_id).await?;
        let previous: Vec<serde_json::Value> = before
            .iter()
            .map(|r| {
                serde_json::json!({
                    "name": r.record.name,
                    "type": r.record.rtype.as_str(),
                    "content": r.record.content,
                    "ttl": r.record.ttl,
                    "priority": r.record.priority,
                })
            })
            .collect();
        sqlx::query!("DELETE FROM dns_records WHERE zone_id = $1", zone_id)
            .execute(&mut *conn)
            .await?;
        removed += before.len() as u64;
        for record in &accepted {
            // The records were checked against each other above, so a duplicate cannot be.
            if add_record(&mut *conn, zone_id, record).await.is_ok() {
                added += 1;
            } else {
                skipped += 1;
            }
        }
        mark_changed(&mut *conn, zone_id).await?;
        zones.push(serde_json::json!({"zone": name, "previous": previous}));
    }
    Ok(serde_json::json!({
        "zones_restored": zones.len(),
        "zones_missing": missing,
        "records_added": added,
        "records_removed": removed,
        "records_skipped": skipped,
        "replaced": zones,
    }))
}

/// Removes the address records (`A`, `AAAA`) of one name in a zone: what was
/// added for a subdomain that no longer exists.
///
/// # Errors
/// Database errors.
pub async fn remove_host_records(
    conn: &mut PgConnection,
    zone_id: Uuid,
    name: &str,
) -> Result<u64, sqlx::Error> {
    let done = sqlx::query!(
        "DELETE FROM dns_records WHERE zone_id = $1 AND name = $2 AND rtype IN ('A', 'AAAA')",
        zone_id,
        name,
    )
    .execute(conn)
    .await?;
    Ok(done.rows_affected())
}

/// Zones whose copy on the DNS server is not up to date: new or changed ones
/// (`pending`), refused ones (`error`) once they have waited `retry_after_secs`,
/// and active ones that were pushed with other name servers than `ns_digest`.
///
/// # Errors
/// Database errors.
pub async fn zones_to_sync(
    pool: &PgPool,
    retry_after_secs: i32,
    ns_digest: &str,
    limit: i64,
) -> Result<Vec<Zone>, sqlx::Error> {
    sqlx::query_as!(
        Zone,
        r#"SELECT id, domain_id, name, status, last_error, updated_at::text AS "version!"
           FROM dns_zones
           WHERE status = 'pending'
              OR (status = 'error' AND updated_at < now() - make_interval(secs => $1::int))
              OR (status = 'active' AND ns_digest IS DISTINCT FROM $3)
           ORDER BY updated_at, id
           LIMIT $2"#,
        retry_after_secs,
        limit,
        ns_digest,
    )
    .fetch_all(pool)
    .await
}

/// One zone.
///
/// # Errors
/// Database errors.
pub async fn zone(pool: &PgPool, id: Uuid) -> Result<Option<Zone>, sqlx::Error> {
    sqlx::query_as!(
        Zone,
        r#"SELECT id, domain_id, name, status, last_error, updated_at::text AS "version!"
           FROM dns_zones WHERE id = $1"#,
        id,
    )
    .fetch_optional(pool)
    .await
}

/// Records how a push went. `version` is the zone's version when the push
/// started: if the zone changed since, nothing is written (it stays `pending`
/// and the next pass pushes it again). Returns whether it was written.
///
/// # Errors
/// Database errors.
pub async fn mark_zone(
    pool: &PgPool,
    id: Uuid,
    version: &str,
    status: &str,
    last_error: Option<&str>,
    ns_digest: Option<&str>,
) -> Result<bool, sqlx::Error> {
    let last_error = last_error.map(|e| e.chars().take(1000).collect::<String>());
    let done = sqlx::query!(
        "UPDATE dns_zones SET status = $3, last_error = $4, ns_digest = COALESCE($5, ns_digest)
         WHERE id = $1 AND updated_at::text = $2",
        id,
        version,
        status,
        last_error,
        ns_digest,
    )
    .execute(pool)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// Names of zones to remove from the DNS server, oldest first.
///
/// # Errors
/// Database errors.
pub async fn queued_removals(pool: &PgPool, limit: i64) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT name FROM dns_zone_removals ORDER BY queued_at, name LIMIT $1",
        limit,
    )
    .fetch_all(pool)
    .await
}

/// Whether a zone of this name exists (again).
///
/// # Errors
/// Database errors.
pub async fn zone_name_exists(pool: &PgPool, name: &str) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM dns_zones WHERE name = $1) AS "exists!""#,
        name,
    )
    .fetch_one(pool)
    .await
}

/// Takes a name off the removal list.
///
/// # Errors
/// Database errors.
pub async fn finish_removal(pool: &PgPool, name: &str) -> Result<(), sqlx::Error> {
    sqlx::query!("DELETE FROM dns_zone_removals WHERE name = $1", name)
        .execute(pool)
        .await?;
    Ok(())
}

/// Why changing a record failed.
#[derive(Debug, thiserror::Error)]
pub enum RecordWriteError {
    /// The very same record exists in the zone.
    #[error("this record exists already")]
    Duplicate,
    /// Any other database error.
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// How many records a zone has.
///
/// # Errors
/// Database errors.
pub async fn record_count(conn: &mut PgConnection, zone_id: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!" FROM dns_records WHERE zone_id = $1"#,
        zone_id,
    )
    .fetch_one(conn)
    .await
}

/// The records of a zone on a plain connection (inside a transaction).
///
/// # Errors
/// Database errors.
pub async fn records_in(
    conn: &mut PgConnection,
    zone_id: Uuid,
) -> Result<Vec<StoredRecord>, sqlx::Error> {
    let rows = sqlx::query!(
        "SELECT id, zone_id, name, rtype, content, ttl, priority
         FROM dns_records WHERE zone_id = $1 ORDER BY name, rtype, content",
        zone_id,
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            Some(StoredRecord {
                id: r.id,
                zone_id: r.zone_id,
                record: DnsRecord {
                    name: r.name,
                    rtype: DnsRecordType::parse(&r.rtype).ok()?,
                    content: r.content,
                    ttl: u32::try_from(r.ttl).ok()?,
                    priority: r.priority.and_then(|p| u16::try_from(p).ok()),
                },
            })
        })
        .collect())
}

/// Adds one record and returns its id.
///
/// # Errors
/// [`RecordWriteError`].
pub async fn add_record(
    conn: &mut PgConnection,
    zone_id: Uuid,
    r: &DnsRecord,
) -> Result<Uuid, RecordWriteError> {
    sqlx::query_scalar!(
        "INSERT INTO dns_records (zone_id, name, rtype, content, ttl, priority)
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
        zone_id,
        r.name,
        r.rtype.as_str(),
        r.content,
        i32::try_from(r.ttl).unwrap_or(i32::MAX),
        r.priority.map(i32::from),
    )
    .fetch_one(conn)
    .await
    .map_err(
        |err| match err.as_database_error().and_then(|e| e.constraint()) {
            Some("dns_records_unique") => RecordWriteError::Duplicate,
            _ => RecordWriteError::Database(err),
        },
    )
}

/// Changes a record's content, TTL and priority. Returns whether it exists.
///
/// # Errors
/// [`RecordWriteError`].
pub async fn change_record(
    conn: &mut PgConnection,
    zone_id: Uuid,
    id: Uuid,
    r: &DnsRecord,
) -> Result<bool, RecordWriteError> {
    let done = sqlx::query!(
        "UPDATE dns_records SET content = $3, ttl = $4, priority = $5
         WHERE zone_id = $1 AND id = $2",
        zone_id,
        id,
        r.content,
        i32::try_from(r.ttl).unwrap_or(i32::MAX),
        r.priority.map(i32::from),
    )
    .execute(conn)
    .await
    .map_err(
        |err| match err.as_database_error().and_then(|e| e.constraint()) {
            Some("dns_records_unique") => RecordWriteError::Duplicate,
            _ => RecordWriteError::Database(err),
        },
    )?;
    Ok(done.rows_affected() == 1)
}

/// Removes a record. Returns whether it existed.
///
/// # Errors
/// Database errors.
pub async fn delete_record(
    conn: &mut PgConnection,
    zone_id: Uuid,
    id: Uuid,
) -> Result<bool, sqlx::Error> {
    let done = sqlx::query!(
        "DELETE FROM dns_records WHERE zone_id = $1 AND id = $2",
        zone_id,
        id,
    )
    .execute(conn)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// Marks a zone as changed, so the next sync pushes it (and a zone the server
/// refused is tried again at once).
///
/// # Errors
/// Database errors.
pub async fn mark_changed(conn: &mut PgConnection, zone_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE dns_zones SET status = 'pending', last_error = NULL WHERE id = $1",
        zone_id,
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// The last delegation check of a zone: its result and when it was made.
///
/// # Errors
/// Database errors.
pub async fn delegation(
    pool: &PgPool,
    zone_id: Uuid,
) -> Result<Option<(serde_json::Value, i64)>, sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT delegation, extract(epoch FROM delegation_checked_at)::bigint AS checked_at
           FROM dns_zones WHERE id = $1"#,
        zone_id,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|r| Some((r.delegation?, r.checked_at?))))
}

/// Keeps the result of a delegation check.
///
/// # Errors
/// Database errors.
pub async fn save_delegation(
    pool: &PgPool,
    zone_id: Uuid,
    result: &serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE dns_zones SET delegation = $2, delegation_checked_at = now() WHERE id = $1",
        zone_id,
        result,
    )
    .execute(pool)
    .await?;
    Ok(())
}
