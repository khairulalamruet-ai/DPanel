//! The panel's own background jobs (`system_jobs`, `system_job_runs`): the repeat time, the switch, the
//! claim of a run and its result. Only one run of a job goes on at a time, also with several panel
//! processes: a run is claimed in one statement.

use sqlx::{PgPool, types::Uuid};

/// How many past runs a job keeps.
const KEEP_RUNS: i64 = 50;

/// One job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    /// Its name (`usage_rollup`, ...).
    pub key: String,
    /// What the list calls it.
    pub title: String,
    /// What it does.
    pub description: String,
    /// The repeat time it came with, in seconds.
    pub default_interval_secs: i32,
    /// The shortest repeat time that may be set.
    pub min_interval_secs: i32,
    /// The repeat time now, in seconds.
    pub interval_secs: i32,
    /// Whether it runs by itself.
    pub enabled: bool,
    /// Whether a run is going on.
    pub running: bool,
    /// Last start (Unix seconds).
    pub last_started_at: Option<i64>,
    /// Last end (Unix seconds).
    pub last_finished_at: Option<i64>,
    /// `ok` or `error`.
    pub last_status: Option<String>,
    /// What the last failure said.
    pub last_error: Option<String>,
    /// Runs so far.
    pub run_count: i64,
    /// When it runs next (Unix seconds); `None` when it is switched off or running.
    pub next_run_at: Option<i64>,
}

/// One past run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    /// Start (Unix seconds).
    pub started_at: i64,
    /// End (Unix seconds).
    pub finished_at: i64,
    /// Whether it went well.
    pub ok: bool,
    /// Whether an administrator started it.
    pub manual: bool,
    /// What it said (the failure, when there was one).
    pub message: String,
}

type JobRow = (
    String,
    String,
    String,
    i32,
    i32,
    i32,
    bool,
    bool,
    Option<i64>,
    Option<i64>,
    Option<String>,
    Option<String>,
    i64,
    Option<i64>,
);

const JOB_COLUMNS: &str = "key, title, description, default_interval_secs, min_interval_secs, interval_secs, \
     enabled, running, EXTRACT(EPOCH FROM last_started_at)::bigint, EXTRACT(EPOCH FROM last_finished_at)::bigint, \
     last_status, last_error, run_count, \
     CASE WHEN enabled AND NOT running THEN EXTRACT(EPOCH FROM COALESCE(last_started_at + make_interval(secs => interval_secs), now()))::bigint END";

fn job(r: JobRow) -> Job {
    Job {
        key: r.0,
        title: r.1,
        description: r.2,
        default_interval_secs: r.3,
        min_interval_secs: r.4,
        interval_secs: r.5,
        enabled: r.6,
        running: r.7,
        last_started_at: r.8,
        last_finished_at: r.9,
        last_status: r.10,
        last_error: r.11,
        run_count: r.12,
        next_run_at: r.13,
    }
}

/// Makes the job known (or updates its name, text and limits, keeping what an administrator set) and
/// forgets a run that was going on when the process last stopped.
///
/// # Errors
/// The database.
pub async fn register(
    db: &PgPool,
    key: &str,
    title: &str,
    description: &str,
    default_secs: i32,
    min_secs: i32,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO system_jobs (key, title, description, default_interval_secs, min_interval_secs, interval_secs)
         VALUES ($1, $2, $3, $4, $5, $4)
         ON CONFLICT (key) DO UPDATE
            SET title = $2, description = $3, default_interval_secs = $4, min_interval_secs = $5,
                interval_secs = GREATEST(system_jobs.interval_secs, $5), running = false",
    )
    .bind(key)
    .bind(title)
    .bind(description)
    .bind(default_secs)
    .bind(min_secs)
    .execute(db)
    .await
    .map(|_| ())
}

/// Claims a run if the job is due (or an administrator asked for one) and nothing of it is going on.
/// `Some(manual)` when the caller may run it now.
///
/// # Errors
/// The database.
pub async fn claim(db: &PgPool, key: &str) -> sqlx::Result<Option<bool>> {
    sqlx::query_scalar(
        "UPDATE system_jobs j SET running = true, last_started_at = now(), run_requested = false, updated_at = now()
           FROM (SELECT key, run_requested FROM system_jobs WHERE key = $1 FOR UPDATE) o
          WHERE j.key = o.key AND NOT j.running
            AND (o.run_requested
                 OR (j.enabled AND (j.last_started_at IS NULL
                                    OR j.last_started_at + make_interval(secs => j.interval_secs) <= now())))
        RETURNING o.run_requested",
    )
    .bind(key)
    .fetch_optional(db)
    .await
}

/// Records how a claimed run ended.
///
/// # Errors
/// The database.
pub async fn finish(
    db: &PgPool,
    key: &str,
    ok: bool,
    manual: bool,
    message: &str,
) -> sqlx::Result<()> {
    let message: String = message.chars().take(2000).collect();
    let mut tx = db.begin().await?;
    sqlx::query(
        "INSERT INTO system_job_runs (job_key, started_at, finished_at, ok, manual, message)
         SELECT key, COALESCE(last_started_at, now()), now(), $2, $3, $4 FROM system_jobs WHERE key = $1",
    )
    .bind(key)
    .bind(ok)
    .bind(manual)
    .bind(&message)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE system_jobs
            SET running = false, last_finished_at = now(), last_status = $2,
                last_error = CASE WHEN $2 = 'error' THEN $3 END, run_count = run_count + 1, updated_at = now()
          WHERE key = $1",
    )
    .bind(key)
    .bind(if ok { "ok" } else { "error" })
    .bind(&message)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "DELETE FROM system_job_runs WHERE job_key = $1
            AND id < COALESCE((SELECT id FROM system_job_runs WHERE job_key = $1 ORDER BY id DESC OFFSET $2 LIMIT 1), 0)",
    )
    .bind(key)
    .bind(KEEP_RUNS)
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

/// Every job, by name.
///
/// # Errors
/// The database.
pub async fn list(db: &PgPool) -> sqlx::Result<Vec<Job>> {
    let sql = format!("SELECT {JOB_COLUMNS} FROM system_jobs ORDER BY title");
    Ok(sqlx::query_as::<_, JobRow>(sqlx::AssertSqlSafe(sql))
        .fetch_all(db)
        .await?
        .into_iter()
        .map(job)
        .collect())
}

/// One job.
///
/// # Errors
/// The database.
pub async fn get(db: &PgPool, key: &str) -> sqlx::Result<Option<Job>> {
    let sql = format!("SELECT {JOB_COLUMNS} FROM system_jobs WHERE key = $1");
    Ok(sqlx::query_as::<_, JobRow>(sqlx::AssertSqlSafe(sql))
        .bind(key)
        .fetch_optional(db)
        .await?
        .map(job))
}

/// Changes the repeat time and/or the switch (what is `None` stays). `reset` puts the repeat time back to
/// what the job came with. Answers whether the job exists.
///
/// # Errors
/// The database.
pub async fn update(
    db: &PgPool,
    key: &str,
    interval_secs: Option<i32>,
    enabled: Option<bool>,
    reset: bool,
) -> sqlx::Result<bool> {
    sqlx::query(
        "UPDATE system_jobs
            SET interval_secs = CASE WHEN $4 THEN default_interval_secs ELSE COALESCE($2, interval_secs) END,
                enabled = COALESCE($3, enabled), updated_at = now()
          WHERE key = $1",
    )
    .bind(key)
    .bind(interval_secs)
    .bind(enabled)
    .bind(reset)
    .execute(db)
    .await
    .map(|r| r.rows_affected() == 1)
}

/// Asks for a run at once (it starts within a couple of seconds). Answers whether a run was asked for
/// (false: no such job, or one is going on already).
///
/// # Errors
/// The database.
pub async fn request_run(db: &PgPool, key: &str) -> sqlx::Result<bool> {
    sqlx::query("UPDATE system_jobs SET run_requested = true, updated_at = now() WHERE key = $1 AND NOT running")
        .bind(key)
        .execute(db)
        .await
        .map(|r| r.rows_affected() == 1)
}

/// The last runs of a job, the newest first.
///
/// # Errors
/// The database.
pub async fn runs(db: &PgPool, key: &str, limit: i64) -> sqlx::Result<Vec<Run>> {
    Ok(sqlx::query_as::<_, (i64, i64, bool, bool, String)>(
        "SELECT EXTRACT(EPOCH FROM started_at)::bigint, EXTRACT(EPOCH FROM finished_at)::bigint, ok, manual, message
           FROM system_job_runs WHERE job_key = $1 ORDER BY id DESC LIMIT $2",
    )
    .bind(key)
    .bind(limit)
    .fetch_all(db)
    .await?
    .into_iter()
    .map(|r| Run {
        started_at: r.0,
        finished_at: r.1,
        ok: r.2,
        manual: r.3,
        message: r.4,
    })
    .collect())
}

/// An operation on its way to a node, for the task list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedOperation {
    /// Its id.
    pub id: Uuid,
    /// What it is (`create_backup`, ...).
    pub kind: String,
    /// `pending` or `dispatched`.
    pub status: String,
    /// The node's name.
    pub node: String,
    /// Since when (Unix seconds).
    pub since: i64,
}

/// Operations that are waiting for a node or being done by one (the oldest first).
///
/// # Errors
/// The database.
pub async fn queued_operations(db: &PgPool, limit: i64) -> sqlx::Result<Vec<QueuedOperation>> {
    Ok(sqlx::query_as::<_, (Uuid, String, String, String, i64)>(
        "SELECT o.id, o.kind, o.status, n.name, EXTRACT(EPOCH FROM o.created_at)::bigint
           FROM operations o JOIN nodes n ON n.id = o.node_id
          WHERE o.status IN ('pending', 'dispatched') ORDER BY o.created_at LIMIT $1",
    )
    .bind(limit)
    .fetch_all(db)
    .await?
    .into_iter()
    .map(|r| QueuedOperation {
        id: r.0,
        kind: r.1,
        status: r.2,
        node: r.3,
        since: r.4,
    })
    .collect())
}
