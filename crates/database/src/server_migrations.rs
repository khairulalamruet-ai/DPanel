//! Whole-server migrations (`server_migrations`): the record of one "install a node on that server over
//! SSH and join it to this panel" job. No credential is ever stored here; the log is capped.

use sqlx::{PgPool, types::Uuid};

/// The log keeps its last this-many bytes.
const LOG_CAP: i32 = 65_536;
/// A running job that has been silent this long is taken as lost (the panel restarted, or the
/// connection hung for good).
const STALE_MINUTES: i32 = 20;

/// One migration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerMigration {
    /// Id.
    pub id: Uuid,
    /// The server's address.
    pub host: String,
    /// Its SSH port.
    pub port: i32,
    /// The login used.
    pub ssh_user: String,
    /// The node's name in this panel.
    pub node_name: String,
    /// `running`, `completed` or `failed`.
    pub status: String,
    /// What it is doing (or did last).
    pub phase: String,
    /// The node, once it has joined.
    pub node_id: Option<Uuid>,
    /// The installer's output, secrets removed.
    pub log: String,
    /// Who started it.
    pub created_by: Option<Uuid>,
    /// Start (Unix seconds).
    pub created_at: i64,
    /// Last change (Unix seconds).
    pub updated_at: i64,
}

type Row = (
    Uuid,
    String,
    i32,
    String,
    String,
    String,
    String,
    Option<Uuid>,
    String,
    Option<Uuid>,
    i64,
    i64,
);

const COLUMNS: &str = "id, host, port, ssh_user, node_name, status, phase, node_id, log, created_by, \
     EXTRACT(EPOCH FROM created_at)::bigint, EXTRACT(EPOCH FROM updated_at)::bigint";

fn from_row(r: Row) -> ServerMigration {
    ServerMigration {
        id: r.0,
        host: r.1,
        port: r.2,
        ssh_user: r.3,
        node_name: r.4,
        status: r.5,
        phase: r.6,
        node_id: r.7,
        log: r.8,
        created_by: r.9,
        created_at: r.10,
        updated_at: r.11,
    }
}

/// Starts a record (status `running`).
///
/// # Errors
/// The database.
pub async fn insert(
    db: &PgPool,
    host: &str,
    port: i32,
    ssh_user: &str,
    node_name: &str,
    created_by: Uuid,
) -> sqlx::Result<ServerMigration> {
    let sql = format!(
        "INSERT INTO server_migrations (id, host, port, ssh_user, node_name, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING {COLUMNS}"
    );
    sqlx::query_as::<_, Row>(sqlx::AssertSqlSafe(sql))
        .bind(Uuid::now_v7())
        .bind(host)
        .bind(port)
        .bind(ssh_user)
        .bind(node_name)
        .bind(created_by)
        .fetch_one(db)
        .await
        .map(from_row)
}

/// One record. A running one that went silent is marked failed first.
///
/// # Errors
/// The database.
pub async fn get(db: &PgPool, id: Uuid) -> sqlx::Result<Option<ServerMigration>> {
    abandon_stale(db).await?;
    let sql = format!("SELECT {COLUMNS} FROM server_migrations WHERE id = $1");
    Ok(sqlx::query_as::<_, Row>(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(db)
        .await?
        .map(from_row))
}

/// The newest records first.
///
/// # Errors
/// The database.
pub async fn list(db: &PgPool, limit: i64) -> sqlx::Result<Vec<ServerMigration>> {
    abandon_stale(db).await?;
    let sql = format!("SELECT {COLUMNS} FROM server_migrations ORDER BY created_at DESC LIMIT $1");
    Ok(sqlx::query_as::<_, Row>(sqlx::AssertSqlSafe(sql))
        .bind(limit)
        .fetch_all(db)
        .await?
        .into_iter()
        .map(from_row)
        .collect())
}

/// Adds text to the log (the last bytes are kept) and says what the job is doing now.
///
/// # Errors
/// The database.
pub async fn append(db: &PgPool, id: Uuid, text: &str, phase: Option<&str>) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE server_migrations
            SET log = right(log || $2, $3), phase = COALESCE($4, phase), updated_at = now()
          WHERE id = $1 AND status = 'running'",
    )
    .bind(id)
    .bind(text)
    .bind(LOG_CAP)
    .bind(phase)
    .execute(db)
    .await
    .map(|_| ())
}

/// Ends the job.
///
/// # Errors
/// The database.
pub async fn finish(
    db: &PgPool,
    id: Uuid,
    ok: bool,
    phase: &str,
    node_id: Option<Uuid>,
) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE server_migrations
            SET status = $2, phase = $3, node_id = COALESCE($4, node_id), updated_at = now()
          WHERE id = $1 AND status = 'running'",
    )
    .bind(id)
    .bind(if ok { "completed" } else { "failed" })
    .bind(phase)
    .bind(node_id)
    .execute(db)
    .await
    .map(|_| ())
}

/// Marks running jobs that have been silent for a long time as failed.
///
/// # Errors
/// The database.
pub async fn abandon_stale(db: &PgPool) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE server_migrations
            SET status = 'failed', phase = 'lost', updated_at = now(),
                log = right(log || E'\\nThe panel lost track of this job (it restarted or the connection hung).\\n', $2)
          WHERE status = 'running' AND updated_at < now() - make_interval(mins => $1)",
    )
    .bind(STALE_MINUTES)
    .bind(LOG_CAP)
    .execute(db)
    .await
    .map(|_| ())
}
