//! The backups of a hosting account, for its owner (T09.09's API).
//!
//! | Method | Path | What |
//! |---|---|---|
//! | GET, POST | `/api/v1/client/backups` | the backups; make one now |
//! | DELETE | `/api/v1/client/backups/{id}` | remove one |
//! | POST | `/api/v1/client/backups/{id}/restore` | put back all of it, a folder, or one database |
//! | GET | `/api/v1/client/backup-restores` | the restores and how they went |
//! | GET, PUT, DELETE | `/api/v1/client/backup-schedule` | when backups are made and how many are kept |
//!
//! Handlers only write rows (a backup, a restore, a schedule); the runner turns them into
//! node operations (see [`crate::runner`]), so a request never waits for a node.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use builtpanel_audit::{AuditEntry, Outcome};
use builtpanel_backup::{crypto::new_key, schedule::Cron};
use builtpanel_database::{
    accounts::DbAccountKind,
    backups::{self as store, Backup, NewRestore, Restore, RestoreError},
    plans, tenants, users,
};
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_rbac::Principal;
use builtpanel_session::rfc3339;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{BackupsState, internal};

struct Hosting {
    tenant: tenants::Tenant,
}

async fn hosting(state: &BackupsState, who: &Principal) -> Result<Hosting, AppError> {
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
    if !plans::includes_feature(&state.db, plan.id, "backups")
        .await
        .map_err(internal)?
    {
        return Err(AppError::forbidden(
            "feature_disabled",
            "Backups are not included in this plan.",
        ));
    }
    Ok(Hosting { tenant })
}

fn running(h: &Hosting) -> Result<(), AppError> {
    if h.tenant.status.as_str() == "active" {
        Ok(())
    } else {
        Err(AppError::conflict(
            "tenant_not_running",
            "Only a running account can make or restore backups.",
        ))
    }
}

async fn audit(
    state: &BackupsState,
    action: &'static str,
    who: &Principal,
    h: &Hosting,
    kind: &'static str,
    id: Uuid,
) -> Result<(), AppError> {
    builtpanel_audit::insert(
        &state.db,
        &AuditEntry::new(action, Outcome::Success)
            .actor(who.account_id())
            .tenant(h.tenant.id)
            .target(kind, id),
    )
    .await
    .map_err(internal)
}

/// One stored part of a backup.
#[derive(Debug, Serialize, ToSchema)]
pub struct PartView {
    /// The part's name at the destination.
    pub name: String,
    /// `files`, `database`, `dns` or `meta`.
    pub kind: String,
    /// Its size as stored (encrypted), in bytes.
    pub stored_bytes: u64,
    /// The database, for a `database` part.
    pub database: Option<String>,
}

/// A backup.
#[derive(Debug, Serialize, ToSchema)]
pub struct BackupView {
    /// Id.
    pub id: Uuid,
    /// `manual` or `schedule`.
    pub trigger: String,
    /// `queued`, `running`, `ready`, `failed` or `deleting`.
    pub status: String,
    /// Everything stored, in bytes (once ready).
    pub size_bytes: Option<i64>,
    /// What it holds (once ready).
    pub parts: Vec<PartView>,
    /// What went wrong, for a failed one.
    pub error: Option<String>,
    /// When it was asked for.
    pub created_at: String,
    /// When it ended.
    pub finished_at: Option<String>,
}

pub(crate) fn backup_view(b: Backup) -> BackupView {
    let parts = b
        .parts
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    Some(PartView {
                        name: p.get("name")?.as_str()?.to_owned(),
                        kind: p.get("kind")?.as_str()?.to_owned(),
                        stored_bytes: p.get("stored_bytes")?.as_u64()?,
                        database: p
                            .get("database")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    BackupView {
        id: b.id,
        trigger: b.trigger,
        status: b.status,
        size_bytes: b.size_bytes,
        parts,
        error: b.error,
        created_at: rfc3339(b.created_at),
        finished_at: b.finished_at.map(rfc3339),
    }
}

/// The backups.
#[derive(Debug, Serialize, ToSchema)]
pub struct BackupList {
    /// Newest first.
    pub items: Vec<BackupView>,
}

/// The backups of the account, newest first.
#[utoipa::path(
    get,
    path = "/api/v1/client/backups",
    tag = "backups",
    operation_id = "clientListBackups",
    security(("session" = [])),
    responses(
        (status = 200, description = "The backups", body = BackupList),
        (status = 403, description = "`client_only` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn list_backups(
    State(state): State<BackupsState>,
    who: Principal,
) -> Result<Json<BackupList>, AppError> {
    let h = hosting(&state, &who).await?;
    let items = store::backups_of(&state.db, h.tenant.id)
        .await
        .map_err(internal)?
        .into_iter()
        .map(backup_view)
        .collect();
    Ok(Json(BackupList { items }))
}

/// Makes a backup now (it is queued; follow `status` until `ready`).
#[utoipa::path(
    post,
    path = "/api/v1/client/backups",
    tag = "backups",
    operation_id = "clientStartBackup",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses(
        (status = 202, description = "Queued", body = BackupView),
        (status = 403, description = "`client_only`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`backup_active`, `no_destination` or `tenant_not_running`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn start_backup(
    State(state): State<BackupsState>,
    who: Principal,
) -> Result<(StatusCode, Json<BackupView>), AppError> {
    let h = hosting(&state, &who).await?;
    running(&h)?;
    if store::has_active_backup(&state.db, h.tenant.id)
        .await
        .map_err(internal)?
    {
        return Err(AppError::conflict(
            "backup_active",
            "A backup of this account is already being made.",
        ));
    }
    let destination = store::default_destination(&state.db)
        .await
        .map_err(internal)?
        .ok_or_else(|| {
            AppError::conflict(
                "no_destination",
                "No place for backups is set up on this panel.",
            )
        })?;
    let id = store::new_backup(
        &state.db,
        h.tenant.id,
        &h.tenant.username,
        "manual",
        destination,
        Some(who.account_id()),
    )
    .await
    .map_err(internal)?;
    let key = new_key().map_err(internal)?;
    let sealed = state
        .master_key
        .seal(&store::key_context(id), &key)
        .map_err(internal)?;
    store::keep_key(&state.db, id, &sealed)
        .await
        .map_err(internal)?;
    audit(&state, "backup.started", &who, &h, "backup", id).await?;
    let made = store::backup_of(&state.db, h.tenant.id, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("backup"))?;
    Ok((StatusCode::ACCEPTED, Json(backup_view(made))))
}

/// Removes a backup (its files at the destination follow shortly).
#[utoipa::path(
    delete,
    path = "/api/v1/client/backups/{id}",
    tag = "backups",
    operation_id = "clientDeleteBackup",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Backup id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 204, description = "Being removed"),
        (status = 404, description = "No such backup", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`backup_busy`: it is not ready or failed", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn delete_backup(
    State(state): State<BackupsState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let h = hosting(&state, &who).await?;
    store::backup_of(&state.db, h.tenant.id, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("backup"))?;
    if !store::discard(&state.db, h.tenant.id, id)
        .await
        .map_err(internal)?
    {
        return Err(AppError::conflict(
            "backup_busy",
            "Only a finished backup can be removed.",
        ));
    }
    audit(&state, "backup.removed", &who, &h, "backup", id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// What to put back.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct StartRestore {
    /// Put the files back.
    #[serde(default)]
    pub files: bool,
    /// Only this folder of the files (relative, such as `www/blog`).
    pub folder: Option<String>,
    /// The databases to put back (their whole names).
    #[serde(default)]
    pub databases: Vec<String>,
    /// Put the DNS records of the account's domains back: in each zone that is still the
    /// account's, the present records are replaced by the ones in the backup (they are kept in
    /// the restore's `dns_summary`). Default: false.
    #[serde(default)]
    pub dns: bool,
    /// Keep the present state first, so that the restore can be undone. Default: true.
    #[serde(default = "yes")]
    pub snapshot: bool,
    /// Only check that the backup is whole; change nothing.
    #[serde(default)]
    pub verify_only: bool,
}

fn yes() -> bool {
    true
}

/// A restore.
#[derive(Debug, Serialize, ToSchema)]
pub struct RestoreView {
    /// Id.
    pub id: Uuid,
    /// The backup it reads.
    pub backup_id: Uuid,
    /// `queued`, `running`, `done` or `failed`.
    pub status: String,
    /// Files put back.
    pub files: bool,
    /// Only this folder.
    pub folder: Option<String>,
    /// Databases put back.
    pub databases: Vec<String>,
    /// Whether the DNS records were asked for.
    pub dns: bool,
    /// What putting the DNS records back did, once it was done: `zones_restored`,
    /// `zones_missing`, `records_added`, `records_removed`, `records_skipped` and `replaced`
    /// (every zone with the records it had before).
    pub dns_summary: Option<serde_json::Value>,
    /// Whether it was only a check.
    pub verify_only: bool,
    /// What went wrong.
    pub error: Option<String>,
    /// How many parts were read and found whole (once done).
    pub verified_parts: Option<u64>,
    /// When it was asked for.
    pub created_at: String,
    /// When it ended.
    pub finished_at: Option<String>,
}

fn restore_view(r: Restore) -> RestoreView {
    RestoreView {
        id: r.id,
        backup_id: r.backup_id,
        status: r.status,
        files: r.files,
        folder: r.folder,
        databases: r.databases,
        dns: r.dns,
        dns_summary: r.dns_summary,
        verify_only: r.verify_only,
        error: r.error,
        verified_parts: r
            .result
            .as_ref()
            .and_then(|v| v.get("verified_parts"))
            .and_then(serde_json::Value::as_u64),
        created_at: rfc3339(r.created_at),
        finished_at: r.finished_at.map(rfc3339),
    }
}

pub(crate) fn bad(name: &'static str, code: &'static str, message: &str) -> AppError {
    AppError::validation(vec![FieldError::new(name, code, message)])
}

/// Puts back a backup, or a part of it.
#[utoipa::path(
    post,
    path = "/api/v1/client/backups/{id}/restore",
    tag = "backups",
    operation_id = "clientStartRestore",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Backup id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = StartRestore,
    responses(
        (status = 202, description = "Queued; follow the restore until `done`", body = RestoreView),
        (status = 404, description = "No such backup", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`backup_not_ready`, `restore_active` or `tenant_not_running`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Nothing to restore, a folder that is not usable, or something the backup does not have", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn start_restore(
    State(state): State<BackupsState>,
    who: Principal,
    Path(id): Path<Uuid>,
    Json(req): Json<StartRestore>,
) -> Result<(StatusCode, Json<RestoreView>), AppError> {
    let h = hosting(&state, &who).await?;
    running(&h)?;
    let backup = store::backup_of(&state.db, h.tenant.id, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("backup"))?;
    if backup.status != "ready" {
        return Err(AppError::conflict(
            "backup_not_ready",
            "Only a finished backup can be restored.",
        ));
    }
    let view = backup_view(backup);
    if !req.verify_only && !req.files && req.databases.is_empty() && !req.dns {
        return Err(bad(
            "files",
            "nothing_to_restore",
            "Choose the files, a database or the DNS records.",
        ));
    }
    if req.dns && !view.parts.iter().any(|p| p.kind == "dns") {
        return Err(bad(
            "dns",
            "not_in_backup",
            "This backup has no DNS records.",
        ));
    }
    if req.files && !view.parts.iter().any(|p| p.kind == "files") {
        return Err(bad("files", "not_in_backup", "This backup has no files."));
    }
    for d in &req.databases {
        if !view.parts.iter().any(|p| p.database.as_deref() == Some(d)) {
            return Err(bad(
                "databases",
                "not_in_backup",
                "This backup has no such database.",
            ));
        }
    }
    if req.databases.len() > 100 {
        return Err(bad("databases", "too_many", "Too many databases."));
    }
    let folder = match &req.folder {
        None => None,
        Some(f) => {
            let f = f.trim_matches('/');
            if !req.files
                || f.is_empty()
                || f.len() > 1024
                || f.contains(['\0', '\\'])
                || f.split('/').any(|p| p.is_empty() || p == "." || p == "..")
            {
                return Err(bad("folder", "invalid", "The folder is not usable."));
            }
            Some(f.to_owned())
        }
    };
    let restore = store::new_restore(
        &state.db,
        &NewRestore {
            backup_id: id,
            tenant_id: h.tenant.id,
            files: req.files,
            folder: folder.as_deref(),
            databases: &req.databases,
            dns: req.dns,
            snapshot: req.snapshot,
            verify_only: req.verify_only,
            created_by: Some(who.account_id()),
        },
    )
    .await
    .map_err(|e| match e {
        RestoreError::Active => AppError::conflict(
            "restore_active",
            "Another restore of this account is not finished.",
        ),
        RestoreError::Database(db) => internal(db),
    })?;
    audit(
        &state,
        "backup.restore_started",
        &who,
        &h,
        "backup_restore",
        restore,
    )
    .await?;
    let made = store::restores_of(&state.db, h.tenant.id)
        .await
        .map_err(internal)?
        .into_iter()
        .find(|r| r.id == restore)
        .ok_or_else(|| AppError::not_found("restore"))?;
    Ok((StatusCode::ACCEPTED, Json(restore_view(made))))
}

/// The restores.
#[derive(Debug, Serialize, ToSchema)]
pub struct RestoreList {
    /// Newest first.
    pub items: Vec<RestoreView>,
}

/// The restores of the account and how they went.
#[utoipa::path(
    get,
    path = "/api/v1/client/backup-restores",
    tag = "backups",
    operation_id = "clientListRestores",
    security(("session" = [])),
    responses((status = 200, description = "The restores", body = RestoreList))
)]
pub async fn list_restores(
    State(state): State<BackupsState>,
    who: Principal,
) -> Result<Json<RestoreList>, AppError> {
    let h = hosting(&state, &who).await?;
    let items = store::restores_of(&state.db, h.tenant.id)
        .await
        .map_err(internal)?
        .into_iter()
        .map(restore_view)
        .collect();
    Ok(Json(RestoreList { items }))
}

/// When backups are made and how many are kept.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ScheduleView {
    /// Five fields (minute hour day-of-month month day-of-week), UTC. At most once an hour.
    #[schema(example = "30 2 * * *")]
    pub cron: String,
    /// How many recent days keep their newest backup (0-14).
    pub keep_daily: i32,
    /// How many recent weeks keep their newest backup (0-8).
    pub keep_weekly: i32,
    /// Whether it runs.
    pub enabled: bool,
    /// The next run (read-only).
    #[serde(default, skip_deserializing)]
    pub next_run_at: Option<String>,
    /// The last run (read-only).
    #[serde(default, skip_deserializing)]
    pub last_run_at: Option<String>,
}

pub(crate) fn schedule_view(s: store::Schedule) -> ScheduleView {
    ScheduleView {
        cron: s.cron,
        keep_daily: s.keep_daily,
        keep_weekly: s.keep_weekly,
        enabled: s.enabled,
        next_run_at: s.next_run_at.map(rfc3339),
        last_run_at: s.last_run_at.map(rfc3339),
    }
}

/// The account's schedule.
#[utoipa::path(
    get,
    path = "/api/v1/client/backup-schedule",
    tag = "backups",
    operation_id = "clientGetBackupSchedule",
    security(("session" = [])),
    responses(
        (status = 200, description = "The schedule", body = ScheduleView),
        (status = 404, description = "There is none", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn get_schedule(
    State(state): State<BackupsState>,
    who: Principal,
) -> Result<Json<ScheduleView>, AppError> {
    let h = hosting(&state, &who).await?;
    store::schedule_of(&state.db, h.tenant.id)
        .await
        .map_err(internal)?
        .map(|s| Json(schedule_view(s)))
        .ok_or_else(|| AppError::not_found("schedule"))
}

pub(crate) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// Makes or changes the account's schedule.
#[utoipa::path(
    put,
    path = "/api/v1/client/backup-schedule",
    tag = "backups",
    operation_id = "clientPutBackupSchedule",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = ScheduleView,
    responses(
        (status = 200, description = "The schedule", body = ScheduleView),
        (status = 422, description = "A cron expression that is not usable or runs more than hourly, or counts out of range", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn put_schedule(
    State(state): State<BackupsState>,
    who: Principal,
    Json(req): Json<ScheduleView>,
) -> Result<Json<ScheduleView>, AppError> {
    let h = hosting(&state, &who).await?;
    let cron = Cron::parse(req.cron.trim()).map_err(|e| bad("cron", "invalid", &e.to_string()))?;
    let t = now();
    let first = cron
        .next_after(t)
        .ok_or_else(|| bad("cron", "never_runs", "This expression never runs."))?;
    if cron
        .next_after(first)
        .is_some_and(|second| second - first < 3600)
    {
        return Err(bad(
            "cron",
            "too_often",
            "Backups can be made once an hour at most.",
        ));
    }
    if !(0..=14).contains(&req.keep_daily) {
        return Err(bad("keep_daily", "out_of_range", "Between 0 and 14 days."));
    }
    if !(0..=8).contains(&req.keep_weekly) {
        return Err(bad("keep_weekly", "out_of_range", "Between 0 and 8 weeks."));
    }
    store::upsert_schedule(
        &state.db,
        h.tenant.id,
        req.cron.trim(),
        req.keep_daily,
        req.keep_weekly,
        None,
        req.enabled,
        req.enabled.then_some(first),
    )
    .await
    .map_err(internal)?;
    audit(
        &state,
        "backup.schedule_set",
        &who,
        &h,
        "tenant",
        h.tenant.id,
    )
    .await?;
    let saved = store::schedule_of(&state.db, h.tenant.id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("schedule"))?;
    Ok(Json(schedule_view(saved)))
}

/// Removes the account's schedule (backups already made stay).
#[utoipa::path(
    delete,
    path = "/api/v1/client/backup-schedule",
    tag = "backups",
    operation_id = "clientDeleteBackupSchedule",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses((status = 204, description = "Removed"), (status = 404, description = "There is none", body = ProblemDetails, content_type = "application/problem+json"))
)]
pub async fn delete_schedule(
    State(state): State<BackupsState>,
    who: Principal,
) -> Result<StatusCode, AppError> {
    let h = hosting(&state, &who).await?;
    if !store::drop_schedule(&state.db, h.tenant.id)
        .await
        .map_err(internal)?
    {
        return Err(AppError::not_found("schedule"));
    }
    audit(
        &state,
        "backup.schedule_removed",
        &who,
        &h,
        "tenant",
        h.tenant.id,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// How to wrap the key of an export.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct StartExport {
    /// A passphrase of at least 12 characters. The file's key is wrapped with it; it is not
    /// stored, and without it the file cannot be opened.
    pub passphrase: String,
}

/// An export.
#[derive(Debug, Serialize, ToSchema)]
pub struct BackupExportView {
    /// Id.
    pub id: Uuid,
    /// The backup it packs.
    pub backup_id: Uuid,
    /// `queued`, `running`, `ready` or `failed`.
    pub status: String,
    /// The file's name at the destination (once ready).
    pub object: Option<String>,
    /// Its size in bytes (once ready).
    pub size_bytes: Option<i64>,
    /// What went wrong.
    pub error: Option<String>,
    /// When it was asked for.
    pub created_at: String,
    /// When it ended.
    pub finished_at: Option<String>,
}

fn export_view(e: store::Export) -> BackupExportView {
    BackupExportView {
        id: e.id,
        backup_id: e.backup_id,
        status: e.status,
        object: e.object,
        size_bytes: e.size_bytes,
        error: e.error,
        created_at: rfc3339(e.created_at),
        finished_at: e.finished_at.map(rfc3339),
    }
}

/// The exports.
#[derive(Debug, Serialize, ToSchema)]
pub struct BackupExportList {
    /// Newest first.
    pub items: Vec<BackupExportView>,
}

/// Packs a backup into one file (an account export) that another panel can import.
#[utoipa::path(
    post,
    path = "/api/v1/client/backups/{id}/export",
    tag = "backups",
    operation_id = "clientStartExport",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Backup id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = StartExport,
    responses(
        (status = 202, description = "Queued; follow the export until `ready`", body = BackupExportView),
        (status = 404, description = "No such backup", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`backup_not_ready` or `export_active`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "The passphrase is too short", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn start_export(
    State(state): State<BackupsState>,
    who: Principal,
    Path(id): Path<Uuid>,
    Json(req): Json<StartExport>,
) -> Result<(StatusCode, Json<BackupExportView>), AppError> {
    let h = hosting(&state, &who).await?;
    running(&h)?;
    if req.passphrase.chars().count() < builtpanel_backup::export::MIN_PASSPHRASE {
        return Err(bad(
            "passphrase",
            "too_short",
            "Use at least 12 characters.",
        ));
    }
    let backup = store::backup_of(&state.db, h.tenant.id, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("backup"))?;
    if backup.status != "ready" {
        return Err(AppError::conflict(
            "backup_not_ready",
            "Only a finished backup can be exported.",
        ));
    }
    let sealed = store::sealed_key_of(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| internal(std::io::Error::other("a backup has no key")))?;
    let key: [u8; builtpanel_backup::KEY_LEN] = state
        .master_key
        .open(&store::key_context(id), &sealed)
        .map_err(internal)?
        .try_into()
        .map_err(|_| internal(std::io::Error::other("a backup key is not the right size")))?;
    let (passphrase, account, created_at) = (
        req.passphrase.clone(),
        h.tenant.username.clone(),
        backup.created_at,
    );
    let header = tokio::task::spawn_blocking(move || {
        builtpanel_backup::export::Header::new(
            &id.simple().to_string(),
            &account,
            created_at,
            &key,
            &passphrase,
        )
        .and_then(|h| h.to_json())
    })
    .await
    .map_err(internal)?
    .map_err(internal)?;
    let header = String::from_utf8(header).map_err(internal)?;
    let export = store::new_export(&state.db, id, h.tenant.id, &header, Some(who.account_id()))
        .await
        .map_err(|e| match e {
            RestoreError::Active => AppError::conflict(
                "export_active",
                "Another export of this account is not finished.",
            ),
            RestoreError::Database(db) => internal(db),
        })?;
    audit(
        &state,
        "backup.export_started",
        &who,
        &h,
        "backup_export",
        export,
    )
    .await?;
    let made = store::exports_of(&state.db, h.tenant.id)
        .await
        .map_err(internal)?
        .into_iter()
        .find(|e| e.id == export)
        .ok_or_else(|| AppError::not_found("export"))?;
    Ok((StatusCode::ACCEPTED, Json(export_view(made))))
}

/// The exports of the account.
#[utoipa::path(
    get,
    path = "/api/v1/client/backup-exports",
    tag = "backups",
    operation_id = "clientListExports",
    security(("session" = [])),
    responses((status = 200, description = "The exports", body = BackupExportList))
)]
pub async fn list_exports(
    State(state): State<BackupsState>,
    who: Principal,
) -> Result<Json<BackupExportList>, AppError> {
    let h = hosting(&state, &who).await?;
    let items = store::exports_of(&state.db, h.tenant.id)
        .await
        .map_err(internal)?
        .into_iter()
        .map(export_view)
        .collect();
    Ok(Json(BackupExportList { items }))
}
