//! The panel's own background jobs (Server: Scheduled actions and Tasks).
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET | `/api/v1/admin/system/jobs` | `admin.server.read` |
//! | PUT | `/api/v1/admin/system/jobs/{key}` | `admin.server.manage`, sudo mode |
//! | POST | `/api/v1/admin/system/jobs/{key}/run` | `admin.server.manage`, sudo mode |
//! | GET | `/api/v1/admin/system/jobs/{key}/runs` | `admin.server.read` |
//! | GET | `/api/v1/admin/system/tasks` | `admin.server.read` |
//!
//! A job is a piece of work the control plane does again and again (sending notifications, rolling up
//! usage, backups that are due, ...). [`spawn`] runs one: how often, and whether at all, is read from the
//! database at every turn, so an administrator's change takes effect within a couple of seconds, and a
//! run can be asked for at once. The result of each run is kept (the last fifty).

use std::{future::Future, time::Duration};

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use builtpanel_audit::{AuditEntry, Outcome};
use builtpanel_database::{sqlx::PgPool, system_jobs as store};
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_rbac::{
    Require, RequireRecent,
    permissions::{ServerManage, ServerRead},
};
use builtpanel_session::rfc3339;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::SystemState;

/// How often a job looks whether it is due.
const POLL: Duration = Duration::from_secs(2);
/// The longest repeat time that may be set.
const MAX_INTERVAL_SECS: i32 = 30 * 24 * 3600;
/// Most past runs shown.
const RUNS_SHOWN: i64 = 50;
/// Most waiting or running operations shown among the tasks.
const TASKS_SHOWN: i64 = 100;

/// What a job is.
#[derive(Debug, Clone, Copy)]
pub struct Spec {
    /// Its name: lower-case letters, digits and `_`.
    pub key: &'static str,
    /// What the list calls it.
    pub title: &'static str,
    /// What it does, in a sentence.
    pub description: &'static str,
    /// How often it runs unless an administrator says otherwise.
    pub default_interval: Duration,
    /// The shortest time between two runs that may be set.
    pub min_interval: Duration,
}

fn secs(d: Duration) -> i32 {
    i32::try_from(d.as_secs()).unwrap_or(i32::MAX).max(1)
}

/// Runs `task` for as long as the process lives, whenever the job is due. The task answers `Err(text)` for a
/// failed run (the text is kept and shown). A panic in the task is a failed run too.
pub fn spawn<F, Fut>(db: PgPool, spec: Spec, task: F)
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<(), String>> + Send + 'static,
{
    tokio::spawn(async move {
        while let Err(error) = store::register(
            &db,
            spec.key,
            spec.title,
            spec.description,
            secs(spec.default_interval),
            secs(spec.min_interval),
        )
        .await
        {
            tracing::warn!(job = spec.key, %error, "cannot register the job; trying again");
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
        loop {
            tokio::time::sleep(POLL).await;
            let manual = match store::claim(&db, spec.key).await {
                Ok(Some(manual)) => manual,
                Ok(None) => continue,
                Err(error) => {
                    tracing::warn!(job = spec.key, %error, "cannot look whether the job is due");
                    continue;
                }
            };
            let outcome = match tokio::spawn(task()).await {
                Ok(result) => result,
                Err(_) => Err("the job stopped unexpectedly".to_owned()),
            };
            if let Err(message) = &outcome {
                tracing::warn!(job = spec.key, %message, "a run of the job failed");
            }
            let (ok, message) = match outcome {
                Ok(()) => (true, String::new()),
                Err(message) => (false, message),
            };
            if let Err(error) = store::finish(&db, spec.key, ok, manual, &message).await {
                tracing::warn!(job = spec.key, %error, "cannot record the run");
            }
        }
    });
}

// ----------------------------------------------------------------------------------- views

/// One job.
#[derive(Debug, Serialize, ToSchema)]
pub struct JobView {
    /// Its name.
    pub key: String,
    /// What the list calls it.
    pub title: String,
    /// What it does.
    pub description: String,
    /// The repeat time now, in seconds.
    pub interval_seconds: i32,
    /// The repeat time it came with.
    pub default_interval_seconds: i32,
    /// The shortest repeat time that may be set.
    pub min_interval_seconds: i32,
    /// Whether it runs by itself.
    pub enabled: bool,
    /// Whether a run is going on.
    pub running: bool,
    /// When it last started (RFC 3339).
    #[schema(format = DateTime)]
    pub last_started_at: Option<String>,
    /// When it last ended (RFC 3339).
    #[schema(format = DateTime)]
    pub last_finished_at: Option<String>,
    /// `ok` or `error` (absent before the first run).
    pub last_status: Option<String>,
    /// What the last failure said.
    pub last_error: Option<String>,
    /// Runs so far.
    pub run_count: i64,
    /// When it runs next (RFC 3339); absent while it is switched off or running.
    #[schema(format = DateTime)]
    pub next_run_at: Option<String>,
}

fn job_view(j: store::Job) -> JobView {
    JobView {
        key: j.key,
        title: j.title,
        description: j.description,
        interval_seconds: j.interval_secs,
        default_interval_seconds: j.default_interval_secs,
        min_interval_seconds: j.min_interval_secs,
        enabled: j.enabled,
        running: j.running,
        last_started_at: j.last_started_at.map(rfc3339),
        last_finished_at: j.last_finished_at.map(rfc3339),
        last_status: j.last_status,
        last_error: j.last_error,
        run_count: j.run_count,
        next_run_at: j.next_run_at.map(rfc3339),
    }
}

/// One past run.
#[derive(Debug, Serialize, ToSchema)]
pub struct RunView {
    /// Start (RFC 3339).
    #[schema(format = DateTime)]
    pub started_at: String,
    /// End (RFC 3339).
    #[schema(format = DateTime)]
    pub finished_at: String,
    /// Whether it went well.
    pub ok: bool,
    /// Whether an administrator started it.
    pub manual: bool,
    /// What it said (the failure when there was one).
    pub message: String,
}

/// A change to a job. What is absent stays.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct JobUpdate {
    /// Seconds between two runs.
    pub interval_seconds: Option<i32>,
    /// Switch it on or off.
    pub enabled: Option<bool>,
    /// Put the repeat time back to what the job came with (instead of `interval_seconds`).
    #[serde(default)]
    pub reset_interval: bool,
}

/// Something that is going on, or about to.
#[derive(Debug, Serialize, ToSchema)]
pub struct TaskView {
    /// `job` (a background job of the panel) or `operation` (work on its way to a node).
    pub kind: String,
    /// What it is.
    pub name: String,
    /// `running`, `scheduled`, `waiting` (an operation for a node that has not taken it yet) or `working`.
    pub status: String,
    /// Where (a node's name), for an operation.
    pub node: Option<String>,
    /// Since when it runs, or when it runs next (RFC 3339).
    #[schema(format = DateTime)]
    pub at: Option<String>,
}

fn internal(e: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>) -> AppError {
    AppError::internal(e)
}

/// A job name as the table takes it; anything else is no job (and never reaches the database: a NUL byte
/// is not text there).
fn known_key(key: &str) -> Result<(), AppError> {
    let mut chars = key.chars();
    let plain = chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && (2..=63).contains(&key.len())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if plain {
        Ok(())
    } else {
        Err(AppError::not_found("job"))
    }
}

// ----------------------------------------------------------------------------------- handlers

/// The panel's background jobs.
#[utoipa::path(
    get,
    path = "/api/v1/admin/system/jobs",
    tag = "system",
    operation_id = "listSystemJobs",
    security(("session" = [])),
    responses(
        (status = 200, description = "By title", body = Vec<JobView>),
        (status = 403, description = "`permission_denied` (needs admin.server.read)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn list(
    State(state): State<SystemState>,
    _who: Require<ServerRead>,
) -> Result<Json<Vec<JobView>>, AppError> {
    Ok(Json(
        store::list(&state.db)
            .await
            .map_err(internal)?
            .into_iter()
            .map(job_view)
            .collect(),
    ))
}

/// Change how often a job runs, or switch it on or off.
#[utoipa::path(
    put,
    path = "/api/v1/admin/system/jobs/{key}",
    tag = "system",
    operation_id = "updateSystemJob",
    security(("session" = [])),
    params(("key" = String, Path, description = "Job name"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = JobUpdate,
    responses(
        (status = 200, description = "The job as it is now", body = JobView),
        (status = 403, description = "`reauth_required` (sudo mode), `permission_denied` (needs admin.server.manage) or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such job", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "A repeat time below the job's shortest or above 30 days", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn update(
    State(state): State<SystemState>,
    RequireRecent(who, _): RequireRecent<ServerManage>,
    Path(key): Path<String>,
    Json(req): Json<JobUpdate>,
) -> Result<Json<JobView>, AppError> {
    known_key(&key)?;
    let current = store::get(&state.db, &key)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("job"))?;
    if let Some(s) = req.interval_seconds
        && !req.reset_interval
        && !(current.min_interval_secs..=MAX_INTERVAL_SECS).contains(&s)
    {
        return Err(AppError::validation(vec![FieldError::new(
            "interval_seconds",
            "out_of_range",
            "Out of the range this job allows (see min_interval_seconds; at most 30 days).",
        )]));
    }
    store::update(
        &state.db,
        &key,
        req.interval_seconds,
        req.enabled,
        req.reset_interval,
    )
    .await
    .map_err(internal)?;
    let now = store::get(&state.db, &key)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("job"))?;
    builtpanel_audit::insert(
        &state.db,
        &AuditEntry::new("system_job.updated", Outcome::Success)
            .actor(who.account_id())
            .target("system_job", Uuid::nil())
            .metadata(serde_json::json!({
                "job": key,
                "interval_seconds": now.interval_secs,
                "enabled": now.enabled,
            })),
    )
    .await
    .map_err(internal)?;
    Ok(Json(job_view(now)))
}

/// Run a job at once (it starts within a couple of seconds, whether or not it is switched on).
#[utoipa::path(
    post,
    path = "/api/v1/admin/system/jobs/{key}/run",
    tag = "system",
    operation_id = "runSystemJob",
    security(("session" = [])),
    params(("key" = String, Path, description = "Job name"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses(
        (status = 202, description = "Asked for", body = JobView),
        (status = 403, description = "`reauth_required` (sudo mode), `permission_denied` (needs admin.server.manage) or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such job", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`job_running`: a run is going on", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn run_now(
    State(state): State<SystemState>,
    RequireRecent(who, _): RequireRecent<ServerManage>,
    Path(key): Path<String>,
) -> Result<(StatusCode, Json<JobView>), AppError> {
    known_key(&key)?;
    let current = store::get(&state.db, &key)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("job"))?;
    if current.running
        || !store::request_run(&state.db, &key)
            .await
            .map_err(internal)?
    {
        return Err(AppError::conflict(
            "job_running",
            "A run of this job is going on.",
        ));
    }
    builtpanel_audit::insert(
        &state.db,
        &AuditEntry::new("system_job.run_requested", Outcome::Success)
            .actor(who.account_id())
            .target("system_job", Uuid::nil())
            .metadata(serde_json::json!({ "job": key })),
    )
    .await
    .map_err(internal)?;
    Ok((StatusCode::ACCEPTED, Json(job_view(current))))
}

/// The last runs of a job, the newest first.
#[utoipa::path(
    get,
    path = "/api/v1/admin/system/jobs/{key}/runs",
    tag = "system",
    operation_id = "listSystemJobRuns",
    security(("session" = [])),
    params(("key" = String, Path, description = "Job name")),
    responses(
        (status = 200, description = "At most fifty", body = Vec<RunView>),
        (status = 404, description = "No such job", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn runs(
    State(state): State<SystemState>,
    _who: Require<ServerRead>,
    Path(key): Path<String>,
) -> Result<Json<Vec<RunView>>, AppError> {
    known_key(&key)?;
    if store::get(&state.db, &key)
        .await
        .map_err(internal)?
        .is_none()
    {
        return Err(AppError::not_found("job"));
    }
    Ok(Json(
        store::runs(&state.db, &key, RUNS_SHOWN)
            .await
            .map_err(internal)?
            .into_iter()
            .map(|r| RunView {
                started_at: rfc3339(r.started_at),
                finished_at: rfc3339(r.finished_at),
                ok: r.ok,
                manual: r.manual,
                message: r.message,
            })
            .collect(),
    ))
}

/// What is going on now and what is about to: running jobs, jobs that are due, and the operations waiting
/// for a node or being done by one.
#[utoipa::path(
    get,
    path = "/api/v1/admin/system/tasks",
    tag = "system",
    operation_id = "listSystemTasks",
    security(("session" = [])),
    responses(
        (status = 200, description = "Running first, then by time", body = Vec<TaskView>),
        (status = 403, description = "`permission_denied` (needs admin.server.read)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn tasks(
    State(state): State<SystemState>,
    _who: Require<ServerRead>,
) -> Result<Json<Vec<TaskView>>, AppError> {
    let mut running = Vec::new();
    let mut scheduled = Vec::new();
    for j in store::list(&state.db).await.map_err(internal)? {
        if j.running {
            running.push(TaskView {
                kind: "job".into(),
                name: j.title,
                status: "running".into(),
                node: None,
                at: j.last_started_at.map(rfc3339),
            });
        } else if let (true, Some(at)) = (j.enabled, j.next_run_at) {
            scheduled.push((
                at,
                TaskView {
                    kind: "job".into(),
                    name: j.title,
                    status: "scheduled".into(),
                    node: None,
                    at: Some(rfc3339(at)),
                },
            ));
        }
    }
    scheduled.sort_by_key(|(at, _)| *at);
    let mut out = running;
    for o in store::queued_operations(&state.db, TASKS_SHOWN)
        .await
        .map_err(internal)?
    {
        out.push(TaskView {
            kind: "operation".into(),
            name: o.kind,
            status: if o.status == "pending" {
                "waiting"
            } else {
                "working"
            }
            .into(),
            node: Some(o.node),
            at: Some(rfc3339(o.since)),
        });
    }
    out.extend(scheduled.into_iter().map(|(_, t)| t));
    Ok(Json(out))
}
