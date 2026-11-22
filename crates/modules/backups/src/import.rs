//! Making a backup from an account export (T09.11): the other half of moving an account.
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | POST | `/api/v1/admin/backup-imports` | `admin.backups.manage` |
//!
//! The export file lies at a destination (put there by hand, or by the other panel's
//! export). The control plane reads only its header, opens the backup's key with the
//! passphrase and seals it with its own master key; the node then unpacks the file into a
//! backup of the account, which is restored as any other (`POST .../restore`).
//! The account's name must be the one the export was made from.

use axum::{Json, extract::State, http::StatusCode};
use builtpanel_audit::{AuditEntry, Outcome};
use builtpanel_database::{backups as store, tenants};
use builtpanel_http_kit::{AppError, ProblemDetails};
use builtpanel_rbac::{Require, permissions::BackupsManage};
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    BackupsState,
    client::{BackupView, backup_view, bad},
    internal,
};

/// An export file to turn into a backup.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct BackupStartImport {
    /// The hosting account the backup is for.
    pub tenant_id: Uuid,
    /// The destination the file is at (and the backup is made at).
    pub destination_id: Uuid,
    /// The file's name, `exports/<id>.tar`.
    #[schema(example = "exports/0190a1b2c3d4e5f60718293a4b5c6d7e.tar")]
    pub object: String,
    /// The passphrase the export was made with.
    pub passphrase: String,
}

/// Turn an export file into a backup of an account.
#[utoipa::path(
    post,
    path = "/api/v1/admin/backup-imports",
    tag = "backups",
    operation_id = "adminStartBackupImport",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = BackupStartImport,
    responses(
        (status = 202, description = "Queued; the backup is `ready` when the node has unpacked it", body = BackupView),
        (status = 404, description = "No such account, destination or file", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`backup_exists`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "`not_an_export`, `wrong_passphrase` or `account_mismatch`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn start_import(
    State(state): State<BackupsState>,
    who: Require<BackupsManage>,
    Json(req): Json<BackupStartImport>,
) -> Result<(StatusCode, Json<BackupView>), AppError> {
    let plain_object = req
        .object
        .strip_prefix("exports/")
        .and_then(|n| n.strip_suffix(".tar"))
        .is_some_and(|n| {
            !n.is_empty() && n.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        });
    if !plain_object {
        return Err(bad("object", "invalid", "The file name is not usable."));
    }
    let tenant = tenants::get(&state.db, req.tenant_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("tenant"))?;
    let destination = store::destination(&state.db, req.destination_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("backup_destination"))?;
    let secrets = match store::sealed_credentials(&state.db, destination.id)
        .await
        .map_err(internal)?
    {
        Some(sealed) => serde_json::from_slice(
            &state
                .master_key
                .open(&store::credentials_context(destination.id), &sealed)
                .map_err(internal)?,
        )
        .map_err(internal)?,
        None => serde_json::Value::Null,
    };
    // Only the header is read: the first entry of the tar.
    let (kind, settings, object) = (
        destination.kind.clone(),
        destination.settings.clone(),
        req.object.clone(),
    );
    let header = tokio::task::spawn_blocking(move || {
        let dest = builtpanel_backup_dest::open::open(&kind, &settings, &secrets)
            .map_err(|e| AppError::conflict("destination_unavailable", e.to_string()))?;
        let input = dest
            .get(&object)
            .map_err(|_| AppError::not_found("export_file"))?;
        let mut archive = tar::Archive::new(input);
        let mut entries = archive
            .entries()
            .map_err(|_| bad("object", "not_an_export", "This is not an export file."))?;
        let first = entries
            .next()
            .and_then(Result::ok)
            .filter(|e| {
                e.path()
                    .is_ok_and(|p| p.to_string_lossy() == builtpanel_backup::export::HEADER)
            })
            .ok_or_else(|| bad("object", "not_an_export", "This is not an export file."))?;
        builtpanel_backup::export::Header::from_reader(first)
            .map_err(|_| bad("object", "not_an_export", "This is not an export file."))
    })
    .await
    .map_err(internal)??;
    if header.account != tenant.username {
        return Err(bad(
            "tenant_id",
            "account_mismatch",
            "The export was made from an account with another name.",
        ));
    }
    let passphrase = req.passphrase.clone();
    let header2 = header.clone();
    let key = tokio::task::spawn_blocking(move || header2.unwrap_key(&passphrase))
        .await
        .map_err(internal)?
        .map_err(|_| bad("passphrase", "wrong_passphrase", "The passphrase is wrong."))?;
    let id = Uuid::parse_str(&header.backup_id).map_err(internal)?;
    let sealed = state
        .master_key
        .seal(&store::key_context(id), &key)
        .map_err(internal)?;
    store::new_import(
        &state.db,
        &store::Import {
            id,
            tenant_id: tenant.id,
            destination_id: destination.id,
            object: req.object.clone(),
        },
        &tenant.username,
        &sealed,
        Some(who.0.account_id()),
    )
    .await
    .map_err(|e| match e {
        store::ImportError::Exists => {
            AppError::conflict("backup_exists", "A backup with this id exists already.")
        }
        store::ImportError::Database(db) => internal(db),
    })?;
    builtpanel_audit::insert(
        &state.db,
        &AuditEntry::new("backup.import_started", Outcome::Success)
            .actor(who.0.account_id())
            .tenant(tenant.id)
            .target("backup", id),
    )
    .await
    .map_err(internal)?;
    let made = store::backup_of(&state.db, tenant.id, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("backup"))?;
    Ok((StatusCode::ACCEPTED, Json(backup_view(made))))
}
