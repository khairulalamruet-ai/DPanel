//! The `migration_transfers` table (T15.04, T15.06): imports of another panel's backup.
//!
//! The queries are plain (not checked at build time), so this table needs no offline cache.

use serde_json::Value;
use sqlx::{PgPool, Row as _, postgres::PgRow, types::Uuid};

/// Most log lines kept per transfer.
pub const MAX_LOG_LINES: i64 = 2000;

/// One import.
#[derive(Debug, Clone)]
pub struct Transfer {
    /// Id.
    pub id: Uuid,
    /// `cpanel`, `cyberpanel`, `openpanel` or `builtpanel_archive`.
    pub source_type: String,
    /// The server it was fetched from.
    pub source_host: Option<String>,
    /// The backup's file name in the import folder.
    pub backup_file: String,
    /// The account's name here.
    pub tenant_username: String,
    /// The main domain here.
    pub target_domain: String,
    /// Whether it only plans.
    pub dry_run: bool,
    /// `fail`, `rename` or `skip`.
    pub on_conflict: String,
    /// `pending`, `running`, `completed`, `failed` or `cancelled`.
    pub status: String,
    /// 0 to 100.
    pub progress_percent: i32,
    /// The step being done.
    pub current_step: String,
    /// The backup's size.
    pub size_bytes: Option<i64>,
    /// The steps.
    pub steps: Value,
    /// The log.
    pub logs: Value,
    /// The dry run's answer.
    pub plan: Option<Value>,
    /// Why it failed.
    pub error_message: Option<String>,
    /// Who started it.
    pub created_by: Option<Uuid>,
    /// Unix seconds.
    pub created_at: i64,
    /// Unix seconds.
    pub updated_at: i64,
}

const COLUMNS: &str = "id, source_type, source_host, backup_file, tenant_username, target_domain, \
    dry_run, on_conflict, status, progress_percent, current_step, size_bytes, steps, logs, plan, \
    error_message, created_by, \
    extract(epoch FROM created_at)::bigint AS created_at, \
    extract(epoch FROM updated_at)::bigint AS updated_at";

fn row(r: &PgRow) -> Result<Transfer, sqlx::Error> {
    Ok(Transfer {
        id: r.try_get("id")?,
        source_type: r.try_get("source_type")?,
        source_host: r.try_get("source_host")?,
        backup_file: r.try_get("backup_file")?,
        tenant_username: r.try_get("tenant_username")?,
        target_domain: r.try_get("target_domain")?,
        dry_run: r.try_get("dry_run")?,
        on_conflict: r.try_get("on_conflict")?,
        status: r.try_get("status")?,
        progress_percent: r.try_get("progress_percent")?,
        current_step: r.try_get("current_step")?,
        size_bytes: r.try_get("size_bytes")?,
        steps: r.try_get("steps")?,
        logs: r.try_get("logs")?,
        plan: r.try_get("plan")?,
        error_message: r.try_get("error_message")?,
        created_by: r.try_get("created_by")?,
        created_at: r.try_get("created_at")?,
        updated_at: r.try_get("updated_at")?,
    })
}

/// A transfer to record.
#[derive(Debug)]
pub struct NewTransfer<'a> {
    /// Source panel.
    pub source_type: &'a str,
    /// Server it came from.
    pub source_host: Option<&'a str>,
    /// File name in the import folder.
    pub backup_file: &'a str,
    /// Account name.
    pub tenant_username: &'a str,
    /// Main domain.
    pub target_domain: &'a str,
    /// Only plan.
    pub dry_run: bool,
    /// Conflict policy.
    pub on_conflict: &'a str,
    /// Who.
    pub created_by: Uuid,
}

/// Records a transfer (`pending`).
///
/// # Errors
/// Database errors.
pub async fn insert(pool: &PgPool, new: &NewTransfer<'_>) -> Result<Transfer, sqlx::Error> {
    let sql = format!(
        "INSERT INTO migration_transfers (source_type, source_host, backup_file, tenant_username, \
         target_domain, dry_run, on_conflict, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING {COLUMNS}"
    );
    let r = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(new.source_type)
        .bind(new.source_host)
        .bind(new.backup_file)
        .bind(new.tenant_username)
        .bind(new.target_domain)
        .bind(new.dry_run)
        .bind(new.on_conflict)
        .bind(new.created_by)
        .fetch_one(pool)
        .await?;
    row(&r)
}

/// The newest transfers first.
///
/// # Errors
/// Database errors.
pub async fn list(pool: &PgPool, limit: i64) -> Result<Vec<Transfer>, sqlx::Error> {
    let sql = format!(
        "SELECT {COLUMNS} FROM migration_transfers ORDER BY created_at DESC, id DESC LIMIT $1"
    );
    sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(limit)
        .fetch_all(pool)
        .await?
        .iter()
        .map(row)
        .collect()
}

/// One transfer.
///
/// # Errors
/// Database errors.
pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<Transfer>, sqlx::Error> {
    let sql = format!("SELECT {COLUMNS} FROM migration_transfers WHERE id = $1");
    sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(pool)
        .await?
        .as_ref()
        .map(row)
        .transpose()
}

/// What changes while a transfer runs.
#[derive(Debug)]
pub struct Progress<'a> {
    /// New status.
    pub status: &'a str,
    /// 0 to 100.
    pub progress_percent: i32,
    /// The step being done.
    pub current_step: &'a str,
    /// The size of the backup.
    pub size_bytes: Option<i64>,
    /// All steps.
    pub steps: &'a Value,
    /// The dry run's answer.
    pub plan: Option<&'a Value>,
    /// Why it failed.
    pub error_message: Option<&'a str>,
}

/// Records progress.
///
/// # Errors
/// Database errors.
pub async fn set_progress(pool: &PgPool, id: Uuid, p: &Progress<'_>) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE migration_transfers SET status = $2, progress_percent = $3, current_step = $4, \
         size_bytes = COALESCE($5, size_bytes), steps = $6, plan = COALESCE($7, plan), \
         error_message = $8, updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(p.status)
    .bind(p.progress_percent)
    .bind(p.current_step)
    .bind(p.size_bytes)
    .bind(p.steps)
    .bind(p.plan)
    .bind(p.error_message)
    .execute(pool)
    .await?;
    Ok(())
}

/// Adds lines to the log; only the last [`MAX_LOG_LINES`] stay.
///
/// # Errors
/// Database errors.
pub async fn append_log(pool: &PgPool, id: Uuid, lines: &[String]) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE migration_transfers SET updated_at = now(), logs = ( \
           SELECT COALESCE(jsonb_agg(l ORDER BY n), '[]'::jsonb) FROM ( \
             SELECT l, n FROM jsonb_array_elements(logs || to_jsonb($2::text[])) \
                     WITH ORDINALITY AS t(l, n) ORDER BY n DESC LIMIT $3 \
           ) kept ) \
         WHERE id = $1",
    )
    .bind(id)
    .bind(lines)
    .bind(MAX_LOG_LINES)
    .execute(pool)
    .await?;
    Ok(())
}

/// Cancels a transfer that has not ended. Whether it was.
///
/// # Errors
/// Database errors.
pub async fn cancel(pool: &PgPool, id: Uuid) -> Result<bool, sqlx::Error> {
    let done = sqlx::query(
        "UPDATE migration_transfers SET status = 'cancelled', current_step = 'Cancelled', \
         updated_at = now() WHERE id = $1 AND status IN ('pending', 'running')",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// Puts a transfer that failed or was cancelled back to `pending`. Whether it was.
///
/// # Errors
/// Database errors.
pub async fn requeue(pool: &PgPool, id: Uuid) -> Result<bool, sqlx::Error> {
    let done = sqlx::query(
        "UPDATE migration_transfers SET status = 'pending', progress_percent = 0, \
         current_step = '', steps = '[]', plan = NULL, error_message = NULL, updated_at = now() \
         WHERE id = $1 AND status IN ('failed', 'cancelled')",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// Sets a real import that failed or was cancelled running again from the step that stopped it:
/// the steps that were done stay done. Whether it was.
///
/// # Errors
/// Database errors.
pub async fn resume(pool: &PgPool, id: Uuid) -> Result<bool, sqlx::Error> {
    let done = sqlx::query(
        "UPDATE migration_transfers SET status = 'running', error_message = NULL, updated_at = now(), \
           steps = ( SELECT COALESCE(jsonb_agg( \
                       CASE WHEN s->>'status' IN ('failed', 'running') \
                            THEN jsonb_set(s - 'details', '{status}', '\"pending\"') ELSE s END \
                       ORDER BY n), '[]'::jsonb) \
                     FROM jsonb_array_elements(steps) WITH ORDINALITY AS t(s, n) ) \
         WHERE id = $1 AND status IN ('failed', 'cancelled') AND NOT dry_run \
           AND plan IS NOT NULL AND jsonb_array_length(steps) > 0",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// Removes a transfer that has ended. Whether it was.
///
/// # Errors
/// Database errors.
pub async fn delete(pool: &PgPool, id: Uuid) -> Result<bool, sqlx::Error> {
    let done = sqlx::query(
        "DELETE FROM migration_transfers WHERE id = $1 AND status NOT IN ('pending', 'running')",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// The names the panel already uses, for the dry run.
#[derive(Debug, Default)]
pub struct Taken {
    /// Login names and hosting-account names.
    pub usernames: Vec<String>,
    /// Domains.
    pub domains: Vec<String>,
    /// Database and database-user names.
    pub db_names: Vec<String>,
}

/// Loads the names that are in use.
///
/// # Errors
/// Database errors.
pub async fn taken(pool: &PgPool) -> Result<Taken, sqlx::Error> {
    let usernames = sqlx::query_scalar::<_, String>(
        "SELECT username::text FROM accounts UNION SELECT username FROM tenants",
    )
    .fetch_all(pool)
    .await?;
    let domains = sqlx::query_scalar::<_, String>("SELECT fqdn::text FROM domains")
        .fetch_all(pool)
        .await?;
    let db_names = sqlx::query_scalar::<_, String>(
        "SELECT name FROM databases UNION SELECT name FROM db_users",
    )
    .fetch_all(pool)
    .await?;
    Ok(Taken {
        usernames,
        domains,
        db_names,
    })
}
