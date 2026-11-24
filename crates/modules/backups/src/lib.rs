//! Backups: where they go (T09.02), and the backups themselves.
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET, POST | `/api/v1/admin/backup-destinations` | `admin.backups.read`, `.manage` |
//! | PUT, DELETE | `/api/v1/admin/backup-destinations/{id}` | `admin.backups.manage` |
//! | POST | `/api/v1/admin/backup-destinations/{id}/default` | `admin.backups.manage` |
//!
//! A destination's secret part (an S3 key, an SFTP password or key) is **sealed with the
//! panel's master key** the moment it arrives, kept sealed, and never sent back by the
//! API: the listing only says whether credentials are kept.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{FromRef, Path, State},
    http::StatusCode,
    routing::{get, post},
};
use builtpanel_audit::{AuditEntry, Outcome};
use builtpanel_auth_core::secretbox::MasterKey;
use builtpanel_database::{
    backups::{self as store, Destination, SaveError},
    sqlx::PgPool,
};
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_rbac::{
    Principal, Require,
    permissions::{BackupsManage, BackupsRead},
};
use builtpanel_session::{SessionStore, rfc3339};
use builtpanel_settings::SettingsRegistry;
use serde::{Deserialize, Serialize};
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;

pub mod client;
pub mod import;
pub mod runner;
pub mod system;
pub mod transfer;

/// The module's manifest.
pub const MANIFEST: &str = include_str!("../../../../modules/backups/module.toml");

/// What the module's handlers share.
#[derive(Clone)]
pub struct BackupsState {
    /// Database pool.
    pub db: PgPool,
    /// Session store.
    pub sessions: SessionStore,
    /// The checked settings schemas of all modules.
    pub settings: Arc<SettingsRegistry>,
    /// Seals credentials and keys.
    pub master_key: MasterKey,
}

impl FromRef<BackupsState> for PgPool {
    fn from_ref(s: &BackupsState) -> Self {
        s.db.clone()
    }
}

impl FromRef<BackupsState> for SessionStore {
    fn from_ref(s: &BackupsState) -> Self {
        s.sessions.clone()
    }
}

/// Routes for administrators, with full paths.
pub fn admin_router() -> Router<BackupsState> {
    Router::new()
        .route(
            "/api/v1/admin/backup-destinations",
            get(list_destinations).post(add_destination),
        )
        .route(
            "/api/v1/admin/backup-destinations/{id}",
            axum::routing::put(change_destination).delete(remove_destination),
        )
        .route(
            "/api/v1/admin/backup-destinations/{id}/default",
            post(make_default),
        )
        .route("/api/v1/admin/backup-imports", post(import::start_import))
        .route(
            "/api/v1/admin/node-transfers",
            get(transfer::list).post(transfer::start),
        )
        .route("/api/v1/admin/node-transfers/{id}", get(transfer::show))
        .route(
            "/api/v1/admin/node-transfers/{id}/cutover",
            post(transfer::cutover),
        )
        .route(
            "/api/v1/admin/node-transfers/{id}/cleanup",
            post(transfer::cleanup),
        )
        .route(
            "/api/v1/admin/node-transfers/{id}/cancel",
            post(transfer::cancel),
        )
        .route(
            "/api/v1/admin/system-backups",
            get(system::list_system_backups).post(system::start_system_backup),
        )
        .route(
            "/api/v1/admin/system-backups/{id}",
            axum::routing::delete(system::delete_system_backup),
        )
        .route(
            "/api/v1/admin/system-backup-schedule",
            get(system::get_system_schedule)
                .put(system::put_system_schedule)
                .delete(system::delete_system_schedule),
        )
}

/// Routes for clients (none yet).
pub fn client_router() -> Router<BackupsState> {
    Router::new()
        .route(
            "/api/v1/client/backups",
            get(client::list_backups).post(client::start_backup),
        )
        .route(
            "/api/v1/client/backups/{id}",
            axum::routing::delete(client::delete_backup),
        )
        .route(
            "/api/v1/client/backups/{id}/restore",
            post(client::start_restore),
        )
        .route("/api/v1/client/backup-restores", get(client::list_restores))
        .route(
            "/api/v1/client/backups/{id}/export",
            post(client::start_export),
        )
        .route("/api/v1/client/backup-exports", get(client::list_exports))
        .route(
            "/api/v1/client/backup-schedule",
            get(client::get_schedule)
                .put(client::put_schedule)
                .delete(client::delete_schedule),
        )
}

fn internal(err: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>) -> AppError {
    AppError::internal(err)
}

/// A destination.
#[derive(Debug, Serialize, ToSchema)]
pub struct DestinationView {
    /// Id.
    pub id: Uuid,
    /// Its name.
    #[schema(example = "Offsite S3")]
    pub name: String,
    /// `local`, `s3` or `sftp`.
    pub kind: String,
    /// What is not secret (host, bucket, path, user ...).
    pub settings: serde_json::Value,
    /// Whether credentials are kept (they are never shown).
    pub has_credentials: bool,
    /// May receive backups.
    pub enabled: bool,
    /// Where new backups go unless said otherwise.
    pub is_default: bool,
    /// When it was last tested (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub last_tested_at: Option<String>,
    /// The result of that test.
    pub last_test_ok: Option<bool>,
    /// What was wrong.
    pub last_test_error: Option<String>,
}

fn view(d: Destination) -> DestinationView {
    DestinationView {
        id: d.id,
        name: d.name,
        kind: d.kind,
        settings: d.settings,
        has_credentials: d.has_credentials,
        enabled: d.enabled,
        is_default: d.is_default,
        last_tested_at: d.last_tested_at.map(rfc3339),
        last_test_ok: d.last_test_ok,
        last_test_error: d.last_test_error,
    }
}

/// The destinations.
#[derive(Debug, Serialize, ToSchema)]
pub struct DestinationList {
    /// By name.
    pub items: Vec<DestinationView>,
}

fn field(name: &'static str, code: &'static str, message: &str) -> AppError {
    AppError::validation(vec![FieldError::new(name, code, message)])
}

fn text<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    v.get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// Checks the settings of a kind of destination and returns them cleaned (only the keys
/// that kind has).
fn check_settings(kind: &str, given: &serde_json::Value) -> Result<serde_json::Value, AppError> {
    let plain_path = |p: &str| {
        p.len() <= 500
            && !p.contains('\0')
            && !p.split('/').any(|part| part == "..")
            && !p.chars().any(char::is_control)
    };
    match kind {
        "local" => {
            let path = text(given, "path").ok_or_else(|| {
                field(
                    "settings.path",
                    "required",
                    "A folder on the node is needed.",
                )
            })?;
            if !path.starts_with('/') || !plain_path(path) {
                return Err(field(
                    "settings.path",
                    "invalid_path",
                    "The folder is an absolute path without `..`.",
                ));
            }
            Ok(serde_json::json!({ "path": path }))
        }
        "s3" => {
            let endpoint = text(given, "endpoint")
                .ok_or_else(|| field("settings.endpoint", "required", "The endpoint is needed."))?;
            let local = endpoint.starts_with("http://127.0.0.1")
                || endpoint.starts_with("http://localhost");
            if !(endpoint.starts_with("https://") || local)
                || endpoint.len() > 300
                || endpoint.ends_with('/')
                || endpoint
                    .chars()
                    .any(|c| c.is_control() || c == ' ' || c == '?' || c == '#')
            {
                return Err(field(
                    "settings.endpoint",
                    "invalid_endpoint",
                    "The endpoint is an https address without a trailing slash.",
                ));
            }
            let bucket = text(given, "bucket")
                .ok_or_else(|| field("settings.bucket", "required", "The bucket is needed."))?;
            if !(3..=63).contains(&bucket.len())
                || !bucket.bytes().all(|b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'.')
                })
            {
                return Err(field(
                    "settings.bucket",
                    "invalid_bucket",
                    "A bucket name is 3 to 63 lower-case letters, digits, `-` and `.`.",
                ));
            }
            let region = text(given, "region").unwrap_or("us-east-1");
            if region.len() > 40
                || !region
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            {
                return Err(field(
                    "settings.region",
                    "invalid_region",
                    "The region is letters, digits and `-`.",
                ));
            }
            let prefix = text(given, "prefix").unwrap_or("").trim_matches('/');
            if !plain_path(prefix) {
                return Err(field(
                    "settings.prefix",
                    "invalid_path",
                    "The prefix has no `..`.",
                ));
            }
            Ok(serde_json::json!({
                "endpoint": endpoint,
                "bucket": bucket,
                "region": region,
                "prefix": prefix,
                "path_style": given.get("path_style").and_then(serde_json::Value::as_bool).unwrap_or(true),
            }))
        }
        "sftp" => {
            let host = text(given, "host")
                .ok_or_else(|| field("settings.host", "required", "The host is needed."))?;
            if host.len() > 253
                || !host
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':'))
            {
                return Err(field(
                    "settings.host",
                    "invalid_host",
                    "The host is a name or an address.",
                ));
            }
            let port = given
                .get("port")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(22);
            if !(1..=65535).contains(&port) {
                return Err(field(
                    "settings.port",
                    "invalid_port",
                    "The port is 1 to 65535.",
                ));
            }
            let user = text(given, "user")
                .ok_or_else(|| field("settings.user", "required", "The user is needed."))?;
            if user.len() > 64 || user.chars().any(|c| c.is_control() || c.is_whitespace()) {
                return Err(field(
                    "settings.user",
                    "invalid_user",
                    "The user name has no spaces.",
                ));
            }
            let path = text(given, "path").unwrap_or("/");
            if !path.starts_with('/') || !plain_path(path) {
                return Err(field(
                    "settings.path",
                    "invalid_path",
                    "The folder is an absolute path without `..`.",
                ));
            }
            // The server's key, as the SHA-256 of its public key in hex: the destination is
            // refused when the server shows another (never "trust on first use").
            let fingerprint = text(given, "host_key_sha256").map(str::to_ascii_lowercase);
            if fingerprint
                .as_deref()
                .is_some_and(|f| f.len() != 64 || !f.bytes().all(|b| b.is_ascii_hexdigit()))
            {
                return Err(field(
                    "settings.host_key_sha256",
                    "invalid_fingerprint",
                    "The key fingerprint is 64 hexadecimal digits.",
                ));
            }
            Ok(
                serde_json::json!({ "host": host, "port": port, "user": user, "path": path, "host_key_sha256": fingerprint }),
            )
        }
        _ => Err(field(
            "kind",
            "invalid_kind",
            "The kind is `local`, `s3` or `sftp`.",
        )),
    }
}

/// Checks the credentials of a kind of destination and returns them as one JSON text to seal.
fn check_credentials(
    kind: &str,
    given: Option<&serde_json::Value>,
) -> Result<Option<String>, AppError> {
    let Some(given) = given else {
        return match kind {
            "local" => Ok(None),
            _ => Err(field(
                "credentials",
                "required",
                "This kind needs credentials.",
            )),
        };
    };
    let secret = |key: &'static str| {
        text(given, key)
            .filter(|s| s.len() <= 4096 && !s.contains('\0'))
            .ok_or_else(|| field("credentials", "required", &format!("`{key}` is needed.")))
    };
    match kind {
        "local" => Ok(None),
        "s3" => Ok(Some(
            serde_json::json!({ "access_key": secret("access_key")?, "secret_key": secret("secret_key")? }).to_string(),
        )),
        "sftp" => {
            let password = text(given, "password");
            let key = text(given, "private_key");
            match (password, key) {
                (Some(p), None) if p.len() <= 1024 => Ok(Some(serde_json::json!({ "password": p }).to_string())),
                (None, Some(k)) if k.len() <= 8192 && k.contains("PRIVATE KEY") => {
                    Ok(Some(serde_json::json!({ "private_key": k }).to_string()))
                }
                _ => Err(field("credentials", "invalid_credentials", "Give a password or a private key (not both).")),
            }
        }
        _ => Err(field("kind", "invalid_kind", "The kind is `local`, `s3` or `sftp`.")),
    }
}

fn audit(action: &'static str, who: &Principal, id: Uuid, name: &str) -> AuditEntry {
    AuditEntry::new(action, Outcome::Success)
        .actor(who.account_id())
        .target("backup_destination", id)
        .metadata(serde_json::json!({ "name": name }))
}

fn save_error(e: SaveError) -> AppError {
    match e {
        SaveError::Taken => AppError::conflict("name_taken", "A destination has this name."),
        SaveError::Database(db) => internal(db),
    }
}

fn check_name(name: &str) -> Result<String, AppError> {
    let name = name.trim();
    if name.is_empty() || name.len() > 80 || name.chars().any(char::is_control) {
        return Err(field(
            "name",
            "invalid_name",
            "The name is 1 to 80 characters.",
        ));
    }
    Ok(name.to_owned())
}

/// The destinations.
#[utoipa::path(
    get,
    path = "/api/v1/admin/backup-destinations",
    tag = "backups",
    operation_id = "adminListBackupDestinations",
    security(("session" = [])),
    responses(
        (status = 200, description = "The destinations", body = DestinationList),
        (status = 403, description = "`permission_denied` (needs admin.backups.read)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn list_destinations(
    State(state): State<BackupsState>,
    _who: Require<BackupsRead>,
) -> Result<Json<DestinationList>, AppError> {
    let items = store::destinations(&state.db).await.map_err(internal)?;
    Ok(Json(DestinationList {
        items: items.into_iter().map(view).collect(),
    }))
}

/// A destination to make.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AddDestination {
    /// Its name.
    #[schema(example = "Offsite S3")]
    pub name: String,
    /// `local`, `s3` or `sftp`.
    #[schema(example = "s3")]
    pub kind: String,
    /// What is not secret: `local`: `path`; `s3`: `endpoint`, `bucket`, `region`, `prefix`,
    /// `path_style`; `sftp`: `host`, `port`, `user`, `path`, `host_key_sha256`.
    pub settings: serde_json::Value,
    /// The secret part: `s3`: `access_key`, `secret_key`; `sftp`: `password` or `private_key`.
    /// Sealed at once and never shown again.
    pub credentials: Option<serde_json::Value>,
}

/// Add a destination.
#[utoipa::path(
    post,
    path = "/api/v1/admin/backup-destinations",
    tag = "backups",
    operation_id = "adminAddBackupDestination",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = AddDestination,
    responses(
        (status = 201, description = "Made (the first one is the default)", body = DestinationView),
        (status = 409, description = "`name_taken`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "A setting or the credentials are not acceptable", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn add_destination(
    State(state): State<BackupsState>,
    who: Require<BackupsManage>,
    Json(req): Json<AddDestination>,
) -> Result<(StatusCode, Json<DestinationView>), AppError> {
    let name = check_name(&req.name)?;
    let settings = check_settings(&req.kind, &req.settings)?;
    let secret = check_credentials(&req.kind, req.credentials.as_ref())?;
    let id = store::add_destination(&state.db, &name, &req.kind, &settings, who.0.account_id())
        .await
        .map_err(save_error)?;
    if let Some(secret) = secret {
        let sealed = state
            .master_key
            .seal(&store::credentials_context(id), secret.as_bytes())
            .map_err(internal)?;
        store::keep_credentials(&state.db, id, Some(&sealed))
            .await
            .map_err(internal)?;
    }
    builtpanel_audit::insert(
        &state.db,
        &audit("backup_destination.added", &who.0, id, &name),
    )
    .await
    .map_err(internal)?;
    let made = store::destination(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("backup_destination"))?;
    Ok((StatusCode::CREATED, Json(view(made))))
}

/// What to change; what is left out stays.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeDestination {
    /// A new name.
    pub name: Option<String>,
    /// New settings (the whole set for its kind).
    pub settings: Option<serde_json::Value>,
    /// New credentials (the whole set for its kind), replacing the sealed ones.
    pub credentials: Option<serde_json::Value>,
    /// Whether it may receive backups.
    pub enabled: Option<bool>,
}

/// Change a destination.
#[utoipa::path(
    put,
    path = "/api/v1/admin/backup-destinations/{id}",
    tag = "backups",
    operation_id = "adminChangeBackupDestination",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Destination id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = ChangeDestination,
    responses(
        (status = 200, description = "Changed", body = DestinationView),
        (status = 404, description = "No such destination", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`name_taken` or `is_default` (the default cannot be switched off)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn change_destination(
    State(state): State<BackupsState>,
    who: Require<BackupsManage>,
    Path(id): Path<Uuid>,
    Json(req): Json<ChangeDestination>,
) -> Result<Json<DestinationView>, AppError> {
    let d = store::destination(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("backup_destination"))?;
    let name = match &req.name {
        Some(n) => check_name(n)?,
        None => d.name.clone(),
    };
    let settings = match &req.settings {
        Some(s) => check_settings(&d.kind, s)?,
        None => d.settings.clone(),
    };
    let enabled = req.enabled.unwrap_or(d.enabled);
    if d.is_default && !enabled {
        return Err(AppError::conflict(
            "is_default",
            "Make another destination the default first.",
        ));
    }
    let secret = match &req.credentials {
        Some(c) => check_credentials(&d.kind, Some(c))?,
        None => None,
    };
    store::change_destination(&state.db, id, &name, &settings, enabled)
        .await
        .map_err(save_error)?;
    if let Some(secret) = secret {
        let sealed = state
            .master_key
            .seal(&store::credentials_context(id), secret.as_bytes())
            .map_err(internal)?;
        store::keep_credentials(&state.db, id, Some(&sealed))
            .await
            .map_err(internal)?;
    }
    builtpanel_audit::insert(
        &state.db,
        &audit("backup_destination.changed", &who.0, id, &name),
    )
    .await
    .map_err(internal)?;
    let now = store::destination(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("backup_destination"))?;
    Ok(Json(view(now)))
}

/// Make a destination the default.
#[utoipa::path(
    post,
    path = "/api/v1/admin/backup-destinations/{id}/default",
    tag = "backups",
    operation_id = "adminDefaultBackupDestination",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Destination id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 200, description = "Now the default", body = DestinationView),
        (status = 404, description = "No such (enabled) destination", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn make_default(
    State(state): State<BackupsState>,
    who: Require<BackupsManage>,
    Path(id): Path<Uuid>,
) -> Result<Json<DestinationView>, AppError> {
    if !store::make_default(&state.db, id).await.map_err(internal)? {
        return Err(AppError::not_found("backup_destination"));
    }
    let now = store::destination(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("backup_destination"))?;
    builtpanel_audit::insert(
        &state.db,
        &audit("backup_destination.default", &who.0, id, &now.name),
    )
    .await
    .map_err(internal)?;
    Ok(Json(view(now)))
}

/// Remove a destination nothing uses.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/backup-destinations/{id}",
    tag = "backups",
    operation_id = "adminRemoveBackupDestination",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Destination id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 204, description = "Removed"),
        (status = 404, description = "No such destination", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`in_use` (backups or schedules use it) or `is_default`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn remove_destination(
    State(state): State<BackupsState>,
    who: Require<BackupsManage>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let d = store::destination(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("backup_destination"))?;
    if d.is_default {
        return Err(AppError::conflict(
            "is_default",
            "Make another destination the default first.",
        ));
    }
    if !store::remove_destination(&state.db, id)
        .await
        .map_err(internal)?
    {
        return Err(AppError::conflict(
            "in_use",
            "Backups or schedules still use this destination.",
        ));
    }
    builtpanel_audit::insert(
        &state.db,
        &audit("backup_destination.removed", &who.0, id, &d.name),
    )
    .await
    .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// The module's OpenAPI document.
#[derive(OpenApi)]
#[openapi(
    paths(
        list_destinations, add_destination, change_destination, make_default, remove_destination,
        client::list_backups, client::start_backup, client::delete_backup, client::start_restore,
        client::list_restores, client::start_export, client::list_exports, import::start_import, client::get_schedule, client::put_schedule, client::delete_schedule,
        system::list_system_backups, system::start_system_backup, system::delete_system_backup,
        transfer::list, transfer::show, transfer::start, transfer::cutover, transfer::cleanup, transfer::cancel,
        system::get_system_schedule, system::put_system_schedule, system::delete_system_schedule
    ),
    components(schemas(
        DestinationView, DestinationList, AddDestination, ChangeDestination,
        client::BackupView, client::BackupList, client::PartView, client::StartRestore,
        client::RestoreView, client::RestoreList, client::ScheduleView, client::StartExport,
        client::BackupExportView, client::BackupExportList, import::BackupStartImport,
        transfer::NodeTransferView, transfer::StartTransfer
    )),
    tags((name = "backups", description = "Backups"))
)]
pub struct BackupsApi;
