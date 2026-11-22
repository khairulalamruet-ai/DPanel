//! Addresses an account turns away from one of its domains (T11.xx: "block an IP", at the web
//! server). The text is kept in canonical form by the caller (see
//! `builtpanel_node_protocol::normalize_ip_block`).

use std::collections::HashMap;

use sqlx::{PgPool, types::Uuid};

/// Most entries one domain may have.
pub const MAX_PER_DOMAIN: i64 = 100;

/// One entry.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct IpBlock {
    /// Id.
    pub id: Uuid,
    /// The address or the network, canonical.
    pub cidr: String,
    /// Why, if said.
    pub note: Option<String>,
    /// Unix seconds.
    pub created_at: i64,
}

/// Why an entry was not added.
#[derive(Debug)]
pub enum AddError {
    /// Already listed.
    Taken,
    /// The domain has [`MAX_PER_DOMAIN`] already.
    Full,
    /// Database error.
    Database(sqlx::Error),
}

/// A domain's entries, oldest first.
///
/// # Errors
/// Database errors.
pub async fn list(
    pool: &PgPool,
    tenant_id: Uuid,
    domain_id: Uuid,
) -> Result<Vec<IpBlock>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, cidr, note, extract(epoch FROM created_at)::bigint AS created_at \
         FROM domain_ip_blocks WHERE tenant_id = $1 AND domain_id = $2 ORDER BY created_at, id",
    )
    .bind(tenant_id)
    .bind(domain_id)
    .fetch_all(pool)
    .await
}

/// Adds an entry.
///
/// # Errors
/// [`AddError`].
pub async fn add(
    pool: &PgPool,
    tenant_id: Uuid,
    domain_id: Uuid,
    cidr: &str,
    note: Option<&str>,
    by: Uuid,
) -> Result<IpBlock, AddError> {
    let mut tx = pool.begin().await.map_err(AddError::Database)?;
    // One writer per domain at a time, so the limit holds.
    sqlx::query("SELECT id FROM domains WHERE id = $1 FOR UPDATE")
        .bind(domain_id)
        .execute(&mut *tx)
        .await
        .map_err(AddError::Database)?;
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM domain_ip_blocks WHERE domain_id = $1")
            .bind(domain_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(AddError::Database)?;
    if count >= MAX_PER_DOMAIN {
        return Err(AddError::Full);
    }
    let row = sqlx::query_as(
        "INSERT INTO domain_ip_blocks (tenant_id, domain_id, cidr, note, created_by) \
         VALUES ($1, $2, $3, $4, $5) \
         ON CONFLICT (domain_id, cidr) DO NOTHING \
         RETURNING id, cidr, note, extract(epoch FROM created_at)::bigint AS created_at",
    )
    .bind(tenant_id)
    .bind(domain_id)
    .bind(cidr)
    .bind(note)
    .bind(by)
    .fetch_optional(&mut *tx)
    .await
    .map_err(AddError::Database)?;
    let Some(row) = row else {
        return Err(AddError::Taken);
    };
    tx.commit().await.map_err(AddError::Database)?;
    Ok(row)
}

/// Removes an entry. `false` when there was none (of this account's domain).
///
/// # Errors
/// Database errors.
pub async fn remove(
    pool: &PgPool,
    tenant_id: Uuid,
    domain_id: Uuid,
    id: Uuid,
) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query(
        "DELETE FROM domain_ip_blocks WHERE tenant_id = $1 AND domain_id = $2 AND id = $3",
    )
    .bind(tenant_id)
    .bind(domain_id)
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected()
        == 1)
}

/// The entries of several domains of one account: domain id to its networks, oldest first.
///
/// # Errors
/// Database errors.
pub async fn for_domains(
    pool: &PgPool,
    tenant_id: Uuid,
    domain_ids: &[Uuid],
) -> Result<HashMap<Uuid, Vec<String>>, sqlx::Error> {
    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT domain_id, cidr FROM domain_ip_blocks \
         WHERE tenant_id = $1 AND domain_id = ANY($2) ORDER BY created_at, id",
    )
    .bind(tenant_id)
    .bind(domain_ids)
    .fetch_all(pool)
    .await?;
    let mut by: HashMap<Uuid, Vec<String>> = HashMap::new();
    for (domain, cidr) in rows {
        by.entry(domain).or_default().push(cidr);
    }
    Ok(by)
}
