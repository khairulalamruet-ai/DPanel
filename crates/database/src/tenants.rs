//! The `tenants` table.

use builtpanel_domain::{TenantStatus, UidRange};
use sqlx::{PgConnection, PgPool, types::Uuid};

/// A tenant row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tenant {
    /// Id.
    pub id: Uuid,
    /// The client account that owns it.
    pub owner_account_id: Uuid,
    /// Its node.
    pub node_id: Uuid,
    /// Its plan.
    pub plan_id: Uuid,
    /// System user name.
    pub username: String,
    /// Numeric user id on the node.
    pub uid: i32,
    /// Life-cycle status.
    pub status: TenantStatus,
    /// Creation time (Unix seconds).
    pub created_at: i64,
}

/// What to insert.
#[derive(Debug, Clone)]
pub struct NewTenant<'a> {
    /// The owner (must be a client account).
    pub owner_account_id: Uuid,
    /// The node.
    pub node_id: Uuid,
    /// The plan.
    pub plan_id: Uuid,
    /// System user name (validated by `builtpanel_domain::TenantUsername`).
    pub username: &'a str,
}

/// Why creating a tenant failed.
#[derive(Debug, thiserror::Error)]
pub enum CreateError {
    /// Another tenant has this system name.
    #[error("a tenant with this name exists")]
    UsernameTaken,
    /// The node has no free ids left.
    #[error("the node has no free user ids")]
    NoFreeUid,
    /// The owner is not a client account, or does not exist.
    #[error("the owner must be a client account")]
    BadOwner,
    /// The node or plan does not exist.
    #[error("unknown node or plan")]
    UnknownReference,
    /// Any other database error.
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// Advisory lock namespace for per-node id allocation.
const UID_LOCK: i32 = 7_304_001;

/// Creates a tenant on a node, taking the lowest free numeric id there. Call it
/// in a transaction: the node's allocation is serialised until it ends.
///
/// # Errors
/// [`CreateError`].
pub async fn insert(conn: &mut PgConnection, new: &NewTenant<'_>) -> Result<Tenant, CreateError> {
    // Serialise allocations for this node (hash of the node id; two nodes sharing a
    // lock only wait for each other, never deadlock).
    sqlx::query!(
        "SELECT pg_advisory_xact_lock($1, hashtext($2::text))",
        UID_LOCK,
        new.node_id.to_string(),
    )
    .execute(&mut *conn)
    .await?;
    let used: Vec<i32> =
        sqlx::query_scalar!("SELECT uid FROM tenants WHERE node_id = $1", new.node_id)
            .fetch_all(&mut *conn)
            .await?;
    let uid = UidRange::DEFAULT
        .allocate(used.into_iter().filter_map(|u| u32::try_from(u).ok()))
        .map_err(|_| CreateError::NoFreeUid)?;
    let uid = i32::try_from(uid.get()).map_err(|_| CreateError::NoFreeUid)?;
    let row = sqlx::query!(
        r#"INSERT INTO tenants (owner_account_id, node_id, plan_id, username, uid)
           VALUES ($1, $2, $3, $4, $5)
           RETURNING id, status, extract(epoch FROM created_at)::bigint AS "created_at!""#,
        new.owner_account_id,
        new.node_id,
        new.plan_id,
        new.username,
        uid,
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(
        |err| match err.as_database_error().map(|e| (e.constraint(), e.code())) {
            Some((Some("tenants_username_key"), _)) => CreateError::UsernameTaken,
            Some((
                Some(
                    "tenants_owner_account_id_fkey"
                    | "tenants_node_id_fkey"
                    | "tenants_plan_id_fkey",
                ),
                _,
            )) => CreateError::UnknownReference,
            // The owner trigger: integrity_constraint_violation.
            Some((_, Some(code))) if code == "23000" => CreateError::BadOwner,
            _ => CreateError::Database(err),
        },
    )?;
    Ok(Tenant {
        id: row.id,
        owner_account_id: new.owner_account_id,
        node_id: new.node_id,
        plan_id: new.plan_id,
        username: new.username.to_owned(),
        uid,
        status: TenantStatus::parse(&row.status).unwrap_or(TenantStatus::Provisioning),
        created_at: row.created_at,
    })
}

/// One tenant.
///
/// # Errors
/// Database errors.
pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<Tenant>, sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT id, owner_account_id, node_id, plan_id, username, uid, status,
                  extract(epoch FROM created_at)::bigint AS "created_at!"
           FROM tenants WHERE id = $1"#,
        id
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| Tenant {
        id: r.id,
        owner_account_id: r.owner_account_id,
        node_id: r.node_id,
        plan_id: r.plan_id,
        username: r.username,
        uid: r.uid,
        status: TenantStatus::parse(&r.status).unwrap_or(TenantStatus::Provisioning),
        created_at: r.created_at,
    }))
}

/// Moves a tenant along its life cycle. Returns whether it changed: `false`
/// when the tenant is unknown or its current status may not become `to`.
///
/// # Errors
/// Database errors.
pub async fn set_status(
    conn: &mut PgConnection,
    id: Uuid,
    to: TenantStatus,
    purge_after_days: i32,
) -> Result<bool, sqlx::Error> {
    let current = sqlx::query_scalar!("SELECT status FROM tenants WHERE id = $1 FOR UPDATE", id)
        .fetch_optional(&mut *conn)
        .await?;
    let Some(current) = current.and_then(|s| TenantStatus::parse(&s).ok()) else {
        return Ok(false);
    };
    if !current.can_become(to) {
        return Ok(false);
    }
    let deleting = to == TenantStatus::Deleting;
    sqlx::query!(
        r#"UPDATE tenants
           SET status = $2,
               deleted_at = CASE WHEN $3 THEN now() ELSE NULL END,
               purge_after = CASE WHEN $3 THEN now() + make_interval(days => $4::int) ELSE NULL END
           WHERE id = $1"#,
        id,
        to.as_str(),
        deleting,
        purge_after_days,
    )
    .execute(conn)
    .await?;
    Ok(true)
}

/// How many tenants are on each plan (deleting ones do not count).
///
/// # Errors
/// Database errors.
pub async fn counts_by_plan(pool: &PgPool) -> Result<Vec<(Uuid, i64)>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT plan_id, count(*) AS "n!" FROM tenants WHERE status <> 'deleting' GROUP BY plan_id"#
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| (r.plan_id, r.n)).collect())
}

/// A deleted tenant whose data is due to be purged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuePurge {
    /// Tenant id.
    pub id: Uuid,
    /// Its node.
    pub node_id: Uuid,
    /// System user name.
    pub username: String,
    /// Numeric user id on the node.
    pub uid: i32,
}

/// Deleted tenants whose `purge_after` has passed and for which no purge is
/// already queued or running, oldest first.
///
/// # Errors
/// Database errors.
pub async fn due_for_purge(pool: &PgPool, limit: i64) -> Result<Vec<DuePurge>, sqlx::Error> {
    sqlx::query_as!(
        DuePurge,
        r#"SELECT t.id, t.node_id, t.username, t.uid
           FROM tenants t
           WHERE t.status = 'deleting' AND t.purge_after <= now()
             AND NOT EXISTS (
                 SELECT 1 FROM operations o
                 WHERE o.tenant_id = t.id AND o.kind = 'delete_tenant'
                   AND o.payload ->> 'purge_data' = 'true'
                   AND o.status IN ('pending', 'dispatched', 'retryable'))
           ORDER BY t.purge_after, t.id
           LIMIT $1"#,
        limit,
    )
    .fetch_all(pool)
    .await
}
