//! Cron jobs of a hosting account (T10.01).
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET, POST | `/api/v1/client/cron-jobs` | the owner; the `cron` feature |
//! | GET, PUT, DELETE | `/api/v1/client/cron-jobs/{id}` | the owner |
//!
//! A job is a name, a five-field schedule (UTC), a command line, a timeout and an on/off
//! switch. The rules are settings of the module (`min_interval_minutes`,
//! `max_command_length`, `max_timeout_seconds`) and the plan's `max_cron_jobs`. This
//! module keeps the definitions and the last result; the node's scheduler (T10.02) runs them.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{FromRef, Path, State},
    http::StatusCode,
    routing::get,
};
use builtpanel_audit::{AuditEntry, Outcome};
use builtpanel_backup::schedule::Cron;
use builtpanel_database::{
    accounts::DbAccountKind,
    cron_jobs::{self as store, CronJob, Fields, SaveError},
    operations::{self, NewOperation},
    plans::{self, Plan},
    sqlx::PgPool,
    tenants, users,
};
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_node_protocol::{CronJobSpec, Operation};
use builtpanel_rbac::Principal;
use builtpanel_session::{SessionStore, rfc3339};
use builtpanel_settings::{Scope, SettingsRegistry};
use serde::{Deserialize, Serialize};
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;

/// The module's manifest.
pub const MANIFEST: &str = include_str!("../../../../modules/cron/module.toml");
/// The module's settings schema.
pub const SETTINGS_SCHEMA: &str = include_str!("../../../../modules/cron/settings.schema.json");

/// What the module's handlers share.
#[derive(Clone)]
pub struct CronState {
    /// Database pool.
    pub db: PgPool,
    /// Session store.
    pub sessions: SessionStore,
    /// The checked settings schemas of all modules.
    pub settings: Arc<SettingsRegistry>,
}

impl FromRef<CronState> for PgPool {
    fn from_ref(s: &CronState) -> Self {
        s.db.clone()
    }
}

impl FromRef<CronState> for SessionStore {
    fn from_ref(s: &CronState) -> Self {
        s.sessions.clone()
    }
}

/// Routes for hosting accounts.
pub fn client_router() -> Router<CronState> {
    Router::new()
        .route("/api/v1/client/cron-jobs", get(list_jobs).post(create_job))
        .route(
            "/api/v1/client/cron-jobs/{id}",
            get(get_job).put(change_job).delete(delete_job),
        )
        .route(
            "/api/v1/client/cron-jobs/{id}/run",
            axum::routing::post(run_job),
        )
}

fn internal(err: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>) -> AppError {
    AppError::internal(err)
}

struct Hosting {
    tenant: tenants::Tenant,
    plan: Plan,
}

async fn hosting(state: &CronState, who: &Principal) -> Result<Hosting, AppError> {
    if who.kind != DbAccountKind::Client {
        return Err(AppError::forbidden(
            "client_only",
            "This is for hosting accounts.",
        ));
    }
    let user = users::get(&state.db, who.account_id())
        .await
        .map_err(internal)?;
    let owned = user.and_then(|u| u.tenant).ok_or_else(|| {
        AppError::conflict("no_hosting", "This account has no hosting account yet.")
    })?;
    let tenant = tenants::get(&state.db, owned.id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("user"))?;
    let plan = plans::get(&state.db, tenant.plan_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| internal(std::io::Error::other("the plan of a tenant vanished")))?;
    if !plans::includes_feature(&state.db, plan.id, "cron")
        .await
        .map_err(internal)?
    {
        return Err(AppError::forbidden(
            "feature_disabled",
            "Cron jobs are not included in this plan.",
        ));
    }
    Ok(Hosting { tenant, plan })
}

fn plan_limit(plan: &Plan, key: &str, default: i64) -> Option<i64> {
    let value = plan
        .limits
        .get(key)
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(default);
    (value >= 0).then_some(value)
}

/// The rules in force.
struct Rules {
    min_interval_minutes: i64,
    max_command_length: usize,
    max_timeout_seconds: i64,
}

async fn rules(state: &CronState) -> Result<Rules, AppError> {
    let mut conn = state.db.acquire().await.map_err(internal)?;
    let values = builtpanel_settings::get(&state.settings, &mut conn, "cron", Scope::Global)
        .await
        .map_err(internal)?;
    let int = |key: &str, default: i64| {
        values
            .get(key)
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(default)
    };
    Ok(Rules {
        min_interval_minutes: int("min_interval_minutes", 5),
        max_command_length: usize::try_from(int("max_command_length", 1000)).unwrap_or(1000),
        max_timeout_seconds: int("max_timeout_seconds", 3600),
    })
}

fn spec(j: &CronJob) -> CronJobSpec {
    CronJobSpec {
        id: j.id,
        schedule: j.schedule.clone(),
        command: j.command.clone(),
        enabled: j.enabled,
        timeout_seconds: u32::try_from(j.timeout_seconds).unwrap_or(300),
    }
}

/// Puts the work for the node in the ledger.
async fn queue(state: &CronState, h: &Hosting, op: &Operation, key: &str) -> Result<(), AppError> {
    let payload = serde_json::to_value(op).map_err(internal)?;
    operations::enqueue(
        &state.db,
        &NewOperation {
            node_id: h.tenant.node_id,
            kind: op.type_name(),
            tenant_id: Some(h.tenant.id),
            payload: &payload,
            idempotency_key: key,
            created_by: None,
            ttl_secs: 6 * 3600,
            max_attempts: 5,
        },
    )
    .await
    .map_err(internal)?;
    Ok(())
}

/// Tells the node the account's jobs as they are now (the whole list: it replaces the old).
async fn sync(state: &CronState, h: &Hosting) -> Result<(), AppError> {
    let jobs = store::list(&state.db, h.tenant.id)
        .await
        .map_err(internal)?
        .iter()
        .map(spec)
        .collect();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_micros());
    queue(
        state,
        h,
        &Operation::SyncCronJobs {
            tenant_id: h.tenant.id,
            username: h.tenant.username.clone(),
            jobs,
        },
        &format!("cronsync:{}:{stamp}", h.tenant.id),
    )
    .await
}

fn field(name: &'static str, code: &'static str, message: &str) -> AppError {
    AppError::validation(vec![FieldError::new(name, code, message)])
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// Checks a schedule against the rules: five fields, something that runs, no two runs closer
/// than the shortest interval.
fn check_schedule(schedule: &str, rules: &Rules) -> Result<(), AppError> {
    let cron = Cron::parse(schedule).map_err(|e| field("schedule", "invalid", &e.to_string()))?;
    // 200 runs reach past every uneven pattern a person writes (`0,1 * * * *`).
    let gap = cron
        .shortest_gap(now(), 200)
        .or_else(|| cron.next_after(now()).map(|_| i64::MAX))
        .ok_or_else(|| field("schedule", "never_runs", "This schedule never runs."))?;
    if gap < rules.min_interval_minutes * 60 {
        return Err(field(
            "schedule",
            "too_often",
            &format!(
                "Jobs may not run more often than every {} minutes.",
                rules.min_interval_minutes
            ),
        ));
    }
    Ok(())
}

/// A job as written by the client.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct JobRequest {
    /// A name for the job, unique in the account (1-80 characters).
    #[schema(example = "Clear the cache")]
    pub name: String,
    /// Five fields: minute hour day-of-month month day-of-week (UTC).
    #[schema(example = "0 * * * *")]
    pub schedule: String,
    /// The command line, run as the account's user in its web container.
    #[schema(example = "php /var/www/html/artisan schedule:run")]
    pub command: String,
    /// Whether it runs. Default: true.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Stopped after this many seconds. Default 300.
    #[serde(default = "five_minutes")]
    pub timeout_seconds: i32,
}

fn yes() -> bool {
    true
}

fn five_minutes() -> i32 {
    300
}

/// A job.
#[derive(Debug, Serialize, ToSchema)]
pub struct JobView {
    /// Id.
    pub id: Uuid,
    /// Its name.
    pub name: String,
    /// Five-field schedule (UTC).
    pub schedule: String,
    /// The command line.
    pub command: String,
    /// Whether it runs.
    pub enabled: bool,
    /// Stopped after this many seconds.
    pub timeout_seconds: i32,
    /// When it last ran.
    pub last_run_at: Option<String>,
    /// `ok`, `failed`, `timeout` or `skipped`.
    pub last_status: Option<String>,
    /// The exit code of the last run.
    pub last_exit_code: Option<i32>,
    /// The end of the last run's output.
    pub last_output: Option<String>,
}

fn view(j: CronJob) -> JobView {
    JobView {
        id: j.id,
        name: j.name,
        schedule: j.schedule,
        command: j.command,
        enabled: j.enabled,
        timeout_seconds: j.timeout_seconds,
        last_run_at: j.last_run_at.map(rfc3339),
        last_status: j.last_status,
        last_exit_code: j.last_exit_code,
        last_output: j.last_output,
    }
}

/// The jobs of an account.
#[derive(Debug, Serialize, ToSchema)]
pub struct JobList {
    /// By name.
    pub items: Vec<JobView>,
    /// How many the plan allows (`null`: no limit).
    pub limit: Option<i64>,
}

/// A checked job.
struct Checked {
    name: String,
    schedule: String,
    command: String,
}

fn check(req: &JobRequest, rules: &Rules) -> Result<Checked, AppError> {
    let name = req.name.trim().to_owned();
    if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
        return Err(field(
            "name",
            "invalid",
            "1 to 80 characters, no control characters.",
        ));
    }
    let schedule = req
        .schedule
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    check_schedule(&schedule, rules)?;
    let command = req.command.trim().to_owned();
    if command.is_empty() {
        return Err(field("command", "required", "The command is empty."));
    }
    if command.chars().count() > rules.max_command_length {
        return Err(field(
            "command",
            "too_long",
            &format!("At most {} characters.", rules.max_command_length),
        ));
    }
    if command.contains('\0') || command.contains('\r') {
        return Err(field(
            "command",
            "invalid",
            "No NUL or carriage-return characters.",
        ));
    }
    if !(1..=rules.max_timeout_seconds).contains(&i64::from(req.timeout_seconds)) {
        return Err(field(
            "timeout_seconds",
            "out_of_range",
            &format!("Between 1 and {} seconds.", rules.max_timeout_seconds),
        ));
    }
    Ok(Checked {
        name,
        schedule,
        command,
    })
}

fn save_error(e: SaveError) -> AppError {
    match e {
        SaveError::Taken => AppError::conflict("name_taken", "A job with this name exists."),
        SaveError::Database(db) => internal(db),
    }
}

async fn audit(
    state: &CronState,
    action: &'static str,
    who: &Principal,
    h: &Hosting,
    id: Uuid,
) -> Result<(), AppError> {
    builtpanel_audit::insert(
        &state.db,
        &AuditEntry::new(action, Outcome::Success)
            .actor(who.account_id())
            .tenant(h.tenant.id)
            .target("cron_job", id),
    )
    .await
    .map_err(internal)
}

/// The cron jobs of the account.
#[utoipa::path(
    get,
    path = "/api/v1/client/cron-jobs",
    tag = "cron",
    operation_id = "clientListCronJobs",
    security(("session" = [])),
    responses(
        (status = 200, description = "The jobs", body = JobList),
        (status = 403, description = "`client_only` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn list_jobs(
    State(state): State<CronState>,
    who: Principal,
) -> Result<Json<JobList>, AppError> {
    let h = hosting(&state, &who).await?;
    let items = store::list(&state.db, h.tenant.id)
        .await
        .map_err(internal)?
        .into_iter()
        .map(view)
        .collect();
    Ok(Json(JobList {
        items,
        limit: plan_limit(&h.plan, "max_cron_jobs", 10),
    }))
}

/// One job.
#[utoipa::path(
    get,
    path = "/api/v1/client/cron-jobs/{id}",
    tag = "cron",
    operation_id = "clientGetCronJob",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Job id")),
    responses(
        (status = 200, description = "The job", body = JobView),
        (status = 404, description = "No such job", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn get_job(
    State(state): State<CronState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<Json<JobView>, AppError> {
    let h = hosting(&state, &who).await?;
    store::get(&state.db, h.tenant.id, id)
        .await
        .map_err(internal)?
        .map(|j| Json(view(j)))
        .ok_or_else(|| AppError::not_found("cron_job"))
}

/// Make a cron job.
#[utoipa::path(
    post,
    path = "/api/v1/client/cron-jobs",
    tag = "cron",
    operation_id = "clientCreateCronJob",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = JobRequest,
    responses(
        (status = 201, description = "Made", body = JobView),
        (status = 403, description = "`client_only`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`name_taken` or `limit_reached`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "The schedule, command or timeout break the rules (`too_often`, `too_long`, ...)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn create_job(
    State(state): State<CronState>,
    who: Principal,
    Json(req): Json<JobRequest>,
) -> Result<(StatusCode, Json<JobView>), AppError> {
    let h = hosting(&state, &who).await?;
    let checked = check(&req, &rules(&state).await?)?;
    if let Some(limit) = plan_limit(&h.plan, "max_cron_jobs", 10)
        && store::count(&state.db, h.tenant.id)
            .await
            .map_err(internal)?
            >= limit
    {
        return Err(AppError::conflict(
            "limit_reached",
            format!("The plan allows {limit} cron jobs."),
        ));
    }
    let id = store::create(
        &state.db,
        h.tenant.id,
        &Fields {
            name: &checked.name,
            schedule: &checked.schedule,
            command: &checked.command,
            enabled: req.enabled,
            timeout_seconds: req.timeout_seconds,
        },
    )
    .await
    .map_err(save_error)?;
    sync(&state, &h).await?;
    audit(&state, "cron_job.created", &who, &h, id).await?;
    let made = store::get(&state.db, h.tenant.id, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("cron_job"))?;
    Ok((StatusCode::CREATED, Json(view(made))))
}

/// Change a cron job (all its fields).
#[utoipa::path(
    put,
    path = "/api/v1/client/cron-jobs/{id}",
    tag = "cron",
    operation_id = "clientChangeCronJob",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Job id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = JobRequest,
    responses(
        (status = 200, description = "Changed", body = JobView),
        (status = 404, description = "No such job", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`name_taken`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "The schedule, command or timeout break the rules", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn change_job(
    State(state): State<CronState>,
    who: Principal,
    Path(id): Path<Uuid>,
    Json(req): Json<JobRequest>,
) -> Result<Json<JobView>, AppError> {
    let h = hosting(&state, &who).await?;
    let checked = check(&req, &rules(&state).await?)?;
    if !store::update(
        &state.db,
        h.tenant.id,
        id,
        &Fields {
            name: &checked.name,
            schedule: &checked.schedule,
            command: &checked.command,
            enabled: req.enabled,
            timeout_seconds: req.timeout_seconds,
        },
    )
    .await
    .map_err(save_error)?
    {
        return Err(AppError::not_found("cron_job"));
    }
    sync(&state, &h).await?;
    audit(&state, "cron_job.changed", &who, &h, id).await?;
    let now = store::get(&state.db, h.tenant.id, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("cron_job"))?;
    Ok(Json(view(now)))
}

/// Remove a cron job.
#[utoipa::path(
    delete,
    path = "/api/v1/client/cron-jobs/{id}",
    tag = "cron",
    operation_id = "clientDeleteCronJob",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Job id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 204, description = "Removed"),
        (status = 404, description = "No such job", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn delete_job(
    State(state): State<CronState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let h = hosting(&state, &who).await?;
    if !store::delete(&state.db, h.tenant.id, id)
        .await
        .map_err(internal)?
    {
        return Err(AppError::not_found("cron_job"));
    }
    sync(&state, &h).await?;
    audit(&state, "cron_job.removed", &who, &h, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Run a job now, apart from its schedule. The result shows as the job's last run within
/// about a minute.
#[utoipa::path(
    post,
    path = "/api/v1/client/cron-jobs/{id}/run",
    tag = "cron",
    operation_id = "clientRunCronJob",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Job id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 202, description = "Asked for"),
        (status = 404, description = "No such job", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`tenant_not_running`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn run_job(
    State(state): State<CronState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let h = hosting(&state, &who).await?;
    if h.tenant.status.as_str() != "active" {
        return Err(AppError::conflict(
            "tenant_not_running",
            "Only a running account can run cron jobs.",
        ));
    }
    let job = store::get(&state.db, h.tenant.id, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("cron_job"))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_micros());
    queue(
        &state,
        &h,
        &Operation::RunCronJob {
            tenant_id: h.tenant.id,
            username: h.tenant.username.clone(),
            job: spec(&job),
        },
        &format!("cronrun:{id}:{stamp}"),
    )
    .await?;
    audit(&state, "cron_job.run_requested", &who, &h, id).await?;
    Ok(StatusCode::ACCEPTED)
}

/// The module's OpenAPI document.
#[derive(OpenApi)]
#[openapi(
    paths(list_jobs, get_job, create_job, change_job, delete_job, run_job),
    components(schemas(JobRequest, JobView, JobList)),
    tags((name = "cron", description = "Cron jobs"))
)]
pub struct CronApi;
