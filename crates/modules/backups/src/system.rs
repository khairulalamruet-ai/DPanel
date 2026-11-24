//! The panel's own backups, for administrators (T09.08).
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET, POST | `/api/v1/admin/system-backups` | `admin.backups.read`, `.manage` |
//! | DELETE | `/api/v1/admin/system-backups/{id}` | `admin.backups.manage` |
//! | GET, PUT, DELETE | `/api/v1/admin/system-backup-schedule` | `.read`, `.manage` |
//!
//! A system backup holds the control database (with the audit log), the configuration and
//! the node CA, encrypted with a key derived from the master key (which is not in it).
//! The control plane makes it itself (see `apps/control-api/src/system_backup.rs`); these
//! handlers only write rows. `deploy/runbooks/system-restore.md` says how to restore.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use builtpanel_audit::{AuditEntry, Outcome};
use builtpanel_backup::schedule::Cron;
use builtpanel_database::backups as store;
use builtpanel_http_kit::{AppError, ProblemDetails};
use builtpanel_rbac::{
    Require,
    permissions::{BackupsManage, BackupsRead},
};
use uuid::Uuid;

use crate::{
    BackupsState,
    client::{BackupList, BackupView, ScheduleView, backup_view, bad, now, schedule_view},
    internal,
};

async fn record(
    state: &BackupsState,
    action: &'static str,
    who: &builtpanel_rbac::Principal,
    id: Uuid,
) -> Result<(), AppError> {
    builtpanel_audit::insert(
        &state.db,
        &AuditEntry::new(action, Outcome::Success)
            .actor(who.account_id())
            .target("system_backup", id),
    )
    .await
    .map_err(internal)
}

/// The panel's own backups, newest first.
#[utoipa::path(
    get,
    path = "/api/v1/admin/system-backups",
    tag = "backups",
    operation_id = "adminListSystemBackups",
    security(("session" = [])),
    responses(
        (status = 200, description = "The system backups", body = BackupList),
        (status = 403, description = "`permission_denied` (needs admin.backups.read)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn list_system_backups(
    State(state): State<BackupsState>,
    _who: Require<BackupsRead>,
) -> Result<Json<BackupList>, AppError> {
    let items = store::system_backups(&state.db)
        .await
        .map_err(internal)?
        .into_iter()
        .map(backup_view)
        .collect();
    Ok(Json(BackupList { items }))
}

/// Makes a system backup now (queued; the control plane makes it within half a minute).
#[utoipa::path(
    post,
    path = "/api/v1/admin/system-backups",
    tag = "backups",
    operation_id = "adminStartSystemBackup",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses(
        (status = 202, description = "Queued", body = BackupView),
        (status = 409, description = "`backup_active` or `no_destination`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn start_system_backup(
    State(state): State<BackupsState>,
    who: Require<BackupsManage>,
) -> Result<(StatusCode, Json<BackupView>), AppError> {
    if store::has_active_system_backup(&state.db)
        .await
        .map_err(internal)?
    {
        return Err(AppError::conflict(
            "backup_active",
            "A system backup is already being made.",
        ));
    }
    let destination = store::default_destination(&state.db)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::conflict("no_destination", "No place for backups is set up."))?;
    let id = store::new_system_backup(&state.db, destination, "manual", Some(who.0.account_id()))
        .await
        .map_err(internal)?;
    record(&state, "system_backup.started", &who.0, id).await?;
    let made = store::system_backups(&state.db)
        .await
        .map_err(internal)?
        .into_iter()
        .find(|b| b.id == id)
        .ok_or_else(|| AppError::not_found("backup"))?;
    Ok((StatusCode::ACCEPTED, Json(backup_view(made))))
}

/// Removes a system backup.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/system-backups/{id}",
    tag = "backups",
    operation_id = "adminDeleteSystemBackup",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Backup id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 204, description = "Being removed"),
        (status = 404, description = "No such backup", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`backup_busy`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn delete_system_backup(
    State(state): State<BackupsState>,
    who: Require<BackupsManage>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let known = store::system_backups(&state.db)
        .await
        .map_err(internal)?
        .into_iter()
        .any(|b| b.id == id);
    if !known {
        return Err(AppError::not_found("backup"));
    }
    if !store::discard_system(&state.db, id)
        .await
        .map_err(internal)?
    {
        return Err(AppError::conflict(
            "backup_busy",
            "Only a finished backup can be removed.",
        ));
    }
    record(&state, "system_backup.removed", &who.0, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The panel's own schedule.
#[utoipa::path(
    get,
    path = "/api/v1/admin/system-backup-schedule",
    tag = "backups",
    operation_id = "adminGetSystemBackupSchedule",
    security(("session" = [])),
    responses(
        (status = 200, description = "The schedule", body = ScheduleView),
        (status = 404, description = "There is none", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn get_system_schedule(
    State(state): State<BackupsState>,
    _who: Require<BackupsRead>,
) -> Result<Json<ScheduleView>, AppError> {
    store::system_schedule(&state.db)
        .await
        .map_err(internal)?
        .map(|s| Json(schedule_view(s)))
        .ok_or_else(|| AppError::not_found("schedule"))
}

/// Makes or changes the panel's own schedule (at most once an hour; keeps 0-365 days and
/// 0-520 weeks).
#[utoipa::path(
    put,
    path = "/api/v1/admin/system-backup-schedule",
    tag = "backups",
    operation_id = "adminPutSystemBackupSchedule",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = ScheduleView,
    responses(
        (status = 200, description = "The schedule", body = ScheduleView),
        (status = 422, description = "A cron expression that is not usable or too frequent, or counts out of range", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn put_system_schedule(
    State(state): State<BackupsState>,
    who: Require<BackupsManage>,
    Json(req): Json<ScheduleView>,
) -> Result<Json<ScheduleView>, AppError> {
    let cron = Cron::parse(req.cron.trim()).map_err(|e| bad("cron", "invalid", &e.to_string()))?;
    let first = cron
        .next_after(now())
        .ok_or_else(|| bad("cron", "never_runs", "This expression never runs."))?;
    if cron
        .next_after(first)
        .is_some_and(|second| second - first < 3600)
    {
        return Err(bad("cron", "too_often", "Once an hour at most."));
    }
    if !(0..=365).contains(&req.keep_daily) {
        return Err(bad("keep_daily", "out_of_range", "Between 0 and 365 days."));
    }
    if !(0..=520).contains(&req.keep_weekly) {
        return Err(bad(
            "keep_weekly",
            "out_of_range",
            "Between 0 and 520 weeks.",
        ));
    }
    store::upsert_system_schedule(
        &state.db,
        req.cron.trim(),
        req.keep_daily,
        req.keep_weekly,
        None,
        req.enabled,
        req.enabled.then_some(first),
    )
    .await
    .map_err(internal)?;
    record(&state, "system_backup.schedule_set", &who.0, Uuid::nil()).await?;
    let saved = store::system_schedule(&state.db)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("schedule"))?;
    Ok(Json(schedule_view(saved)))
}

/// Removes the panel's own schedule (backups already made stay).
#[utoipa::path(
    delete,
    path = "/api/v1/admin/system-backup-schedule",
    tag = "backups",
    operation_id = "adminDeleteSystemBackupSchedule",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses((status = 204, description = "Removed"), (status = 404, description = "There is none", body = ProblemDetails, content_type = "application/problem+json"))
)]
pub async fn delete_system_schedule(
    State(state): State<BackupsState>,
    who: Require<BackupsManage>,
) -> Result<StatusCode, AppError> {
    if !store::drop_system_schedule(&state.db)
        .await
        .map_err(internal)?
    {
        return Err(AppError::not_found("schedule"));
    }
    record(
        &state,
        "system_backup.schedule_removed",
        &who.0,
        Uuid::nil(),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
