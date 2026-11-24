//! SQL exports (T07.08).
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | POST | `/api/v1/client/databases/{id}/exports` | the owner; the `databases` feature |
//! | GET | `/api/v1/client/exports` | a client |
//! | GET, DELETE | `/api/v1/client/exports/{id}` | the owner |
//! | POST | `/api/v1/client/exports/{id}/link` | the owner |
//! | PUT | `/api/v1/public/exports/{id}/chunks/{n}` | the node, with the export's token |
//! | POST | `/api/v1/public/exports/{id}/complete` | the node, with the export's token |
//! | GET | `/api/v1/public/exports/{id}/download?token=` | whoever has a link (five minutes) |
//!
//! The node dumps the database (structure, data, views, triggers, routines,
//! events), compresses it and uploads it to the spool folder; the export is then
//! `ready` for a day. The client asks for a link, which works for five minutes
//! (and can be asked for again), and downloads a `.sql.gz` file with it.

use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
    routing::{get, post, put},
};
use builtpanel_database::exports::{self as store, Export, StartError};
use builtpanel_domain::TenantUsername;
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_node_protocol::Operation;
use builtpanel_rbac::Principal;
use builtpanel_session::rfc3339;
use builtpanel_settings::Scope;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tower::ServiceExt;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    DatabasesState, client_hosting, entry,
    imports::{MAX_CHUNK_BYTES, checksum, token_matches, write_chunk},
    internal, queue, require_feature, running,
};

const DEFAULT_MAX_EXPORT_MB: i64 = 5120;
/// How long a finished export waits to be downloaded.
const KEEP_HOURS: i32 = 24;
/// How long a download link works.
pub const LINK_MINUTES: i32 = 5;

/// Where the dump of an export waits.
#[must_use]
pub fn spool_path(dir: &std::path::Path, id: Uuid) -> std::path::PathBuf {
    dir.join(format!("{}.sql.gz", id.simple()))
}

/// Routes for clients, with full paths.
pub fn client_router() -> Router<DatabasesState> {
    Router::new()
        .route("/api/v1/client/databases/{id}/exports", post(start_export))
        .route("/api/v1/client/exports", get(list_exports))
        .route(
            "/api/v1/client/exports/{id}",
            get(get_export).delete(delete_export),
        )
        .route("/api/v1/client/exports/{id}/link", post(make_link))
}

/// Routes without a session: the node uploading, and a client's link.
pub fn public_router() -> Router<DatabasesState> {
    Router::new()
        .route(
            "/api/v1/public/exports/{id}/chunks/{n}",
            put(upload_chunk).layer(DefaultBodyLimit::max(MAX_CHUNK_BYTES)),
        )
        .route(
            "/api/v1/public/exports/{id}/complete",
            post(upload_complete),
        )
        .route("/api/v1/public/exports/{id}/download", get(download))
}

/// An export.
#[derive(Debug, Serialize, ToSchema)]
pub struct ExportView {
    /// Id.
    pub id: Uuid,
    /// The database dumped.
    pub database_id: Uuid,
    /// The database's name.
    #[schema(example = "alice_shop")]
    pub database: String,
    /// `queued`, `running`, `ready`, `failed` or `expired`.
    pub status: String,
    /// While it is being made: bytes of the compressed dump that have arrived.
    pub received_bytes: i64,
    /// Size of the finished `.sql.gz` file, in bytes.
    pub size_bytes: Option<i64>,
    /// Why it failed.
    pub error: Option<String>,
    /// When it was asked for (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub created_at: String,
    /// When the file goes (RFC 3339, UTC); while `ready`.
    #[schema(format = DateTime)]
    pub expires_at: Option<String>,
}

fn view(e: Export) -> ExportView {
    ExportView {
        id: e.id,
        database_id: e.database_id,
        database: e.database,
        status: e.status,
        received_bytes: e.received_bytes,
        size_bytes: e.size_bytes,
        error: e.error,
        created_at: rfc3339(e.created_at),
        expires_at: e.expires_at.map(rfc3339),
    }
}

/// The exports of an account.
#[derive(Debug, Serialize, ToSchema)]
pub struct ExportList {
    /// Newest first (the last 50).
    pub items: Vec<ExportView>,
}

/// A download link.
#[derive(Debug, Serialize, ToSchema)]
pub struct DownloadLink {
    /// Where to download from (a path on this panel); the file is `<database>.sql.gz`.
    #[schema(example = "/api/v1/public/exports/0190.../download?token=...")]
    pub url: String,
    /// Until when the link works (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub expires_at: String,
}

async fn max_size_mb(state: &DatabasesState) -> Result<i64, AppError> {
    let mut conn = state.db.acquire().await.map_err(internal)?;
    let values = builtpanel_settings::get(&state.settings, &mut conn, "databases", Scope::Global)
        .await
        .map_err(internal)?;
    Ok(values
        .get("max_export_mb")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(DEFAULT_MAX_EXPORT_MB)
        .max(1))
}

async fn owned(state: &DatabasesState, tenant_id: Uuid, id: Uuid) -> Result<Export, AppError> {
    store::get(&state.db, tenant_id, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("export"))
}

fn token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

/// Dump a database to a `.sql.gz` file.
#[utoipa::path(
    post,
    path = "/api/v1/client/databases/{id}/exports",
    tag = "databases",
    operation_id = "clientStartExport",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Database id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 202, description = "Queued; follow `status` until `ready`", body = ExportView),
        (status = 403, description = "`client_only`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such database", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`export_active`, `database_not_ready`, `imports_unavailable` or `tenant_not_running`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn start_export(
    State(state): State<DatabasesState>,
    who: Principal,
    Path(database_id): Path<Uuid>,
) -> Result<(StatusCode, Json<ExportView>), AppError> {
    let h = client_hosting(&state, &who).await?;
    require_feature(&state, &h, "databases", "Databases are").await?;
    running(&h)?;
    let Some(agent_url) = state.agent_url.clone() else {
        return Err(AppError::conflict(
            "imports_unavailable",
            "Exports are not set up on this panel (agent.public_url).",
        ));
    };
    let db = builtpanel_database::databases::get(&state.db, h.tenant.id, database_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("database"))?;
    if db.status != "ready" {
        return Err(AppError::conflict(
            "database_not_ready",
            "The database is not ready yet.",
        ));
    }
    let token = token();
    let hash = Sha256::digest(token.as_bytes());
    let tenant_name = TenantUsername::parse(&h.tenant.username).map_err(internal)?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    let id = store::start(&mut tx, h.tenant.id, db.id, &hash, who.account_id())
        .await
        .map_err(|e| match e {
            StartError::Active => {
                AppError::conflict("export_active", "This database is already being exported.")
            }
            StartError::Database(db) => internal(db),
        })?;
    let sealed = state
        .master_key
        .seal(&store::token_context(id), token.as_bytes())
        .map_err(internal)?;
    store::keep_sealed_token(&mut tx, id, &sealed)
        .await
        .map_err(internal)?;
    queue(
        &mut tx,
        &h,
        &Operation::ExportSql {
            username: tenant_name.as_str().to_owned(),
            database: db.name.clone(),
            export_id: id,
            url: format!("{agent_url}/api/v1/public/exports/{id}"),
            token: None,
            engine: builtpanel_node_protocol::DbEngine::parse(&db.engine).unwrap_or_default(),
            dedicated: super::server_for(
                &state,
                &h,
                builtpanel_node_protocol::DbEngine::parse(&db.engine).unwrap_or_default(),
            )
            .await?,
        },
        format!("export:{id}"),
        &who,
    )
    .await?;
    builtpanel_audit::insert(
        &mut *tx,
        &entry("database.export_started", &who, &h, "database_export", id)
            .metadata(serde_json::json!({ "database": db.name })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(view(owned(&state, h.tenant.id, id).await?)),
    ))
}

/// The exports of the account.
#[utoipa::path(
    get,
    path = "/api/v1/client/exports",
    tag = "databases",
    operation_id = "clientListExports",
    security(("session" = [])),
    responses(
        (status = 200, description = "The exports", body = ExportList),
        (status = 403, description = "`client_only`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn list_exports(
    State(state): State<DatabasesState>,
    who: Principal,
) -> Result<Json<ExportList>, AppError> {
    let h = client_hosting(&state, &who).await?;
    let rows = store::list(&state.db, h.tenant.id, 50)
        .await
        .map_err(internal)?;
    Ok(Json(ExportList {
        items: rows.into_iter().map(view).collect(),
    }))
}

/// One export.
#[utoipa::path(
    get,
    path = "/api/v1/client/exports/{id}",
    tag = "databases",
    operation_id = "clientGetExport",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Export id")),
    responses(
        (status = 200, description = "The export", body = ExportView),
        (status = 404, description = "No such export (or it is not yours)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn get_export(
    State(state): State<DatabasesState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<Json<ExportView>, AppError> {
    let h = client_hosting(&state, &who).await?;
    Ok(Json(view(owned(&state, h.tenant.id, id).await?)))
}

/// A link to download the finished file; it works for five minutes.
#[utoipa::path(
    post,
    path = "/api/v1/client/exports/{id}/link",
    tag = "databases",
    operation_id = "clientExportLink",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Export id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 200, description = "The link (a new one replaces the last)", body = DownloadLink),
        (status = 404, description = "No such export", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`not_ready`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn make_link(
    State(state): State<DatabasesState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<Json<DownloadLink>, AppError> {
    let h = client_hosting(&state, &who).await?;
    let export = owned(&state, h.tenant.id, id).await?;
    if export.status != "ready" {
        return Err(AppError::conflict(
            "not_ready",
            "This export is not ready (or its time is up).",
        ));
    }
    let token = token();
    let at = store::set_link(
        &state.db,
        h.tenant.id,
        id,
        &Sha256::digest(token.as_bytes()),
        LINK_MINUTES,
    )
    .await
    .map_err(internal)?
    .ok_or_else(|| AppError::conflict("not_ready", "This export is not ready."))?;
    builtpanel_audit::insert(
        &state.db,
        &entry("database.export_link", &who, &h, "database_export", id),
    )
    .await
    .map_err(internal)?;
    Ok(Json(DownloadLink {
        url: format!("/api/v1/public/exports/{id}/download?token={token}"),
        expires_at: rfc3339(at),
    }))
}

/// Remove a finished export (or give up one that is being made).
#[utoipa::path(
    delete,
    path = "/api/v1/client/exports/{id}",
    tag = "databases",
    operation_id = "clientDeleteExport",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Export id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 204, description = "Removed; the file goes at once"),
        (status = 404, description = "No such export", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`already_ended`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn delete_export(
    State(state): State<DatabasesState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let h = client_hosting(&state, &who).await?;
    let export = owned(&state, h.tenant.id, id).await?;
    let done = match export.status.as_str() {
        "ready" => store::expire(&state.db, id).await.map_err(internal)?,
        "queued" | "running" => store::fail(&state.db, id, "cancelled by its owner")
            .await
            .map_err(internal)?,
        _ => false,
    };
    if !done {
        return Err(AppError::conflict(
            "already_ended",
            "This export has ended.",
        ));
    }
    let _ = tokio::fs::remove_file(spool_path(&state.spool_dir, id)).await;
    builtpanel_audit::insert(
        &state.db,
        &entry("database.export_removed", &who, &h, "database_export", id),
    )
    .await
    .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

/// The upload the node's token is good for, or the refusal.
async fn authorised(
    state: &DatabasesState,
    id: Uuid,
    headers: &HeaderMap,
) -> Result<store::UploadTarget, AppError> {
    let denied = || AppError::authentication_failed("bad_token", "This upload is not allowed.");
    let given = bearer(headers).ok_or_else(denied)?;
    let target = store::upload_target(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(denied)?;
    if token_matches(given, &target.token_hash) {
        Ok(target)
    } else {
        Err(denied())
    }
}

/// The node sends the next chunk of the dump (the body is the raw bytes).
pub async fn upload_chunk(
    State(state): State<DatabasesState>,
    Path((id, n)): Path<(Uuid, i32)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, AppError> {
    let target = authorised(&state, id, &headers).await?;
    if n != target.next_chunk {
        return Err(AppError::conflict(
            "wrong_chunk",
            format!("Chunk {} is expected next.", target.next_chunk),
        ));
    }
    if body.is_empty() {
        return Err(AppError::validation(vec![FieldError::new(
            "body",
            "empty_chunk",
            "A chunk has at least one byte.",
        )]));
    }
    let new_total = target
        .received_bytes
        .saturating_add(i64::try_from(body.len()).unwrap_or(i64::MAX));
    let limit = max_size_mb(&state).await?.saturating_mul(1024 * 1024);
    if new_total > limit {
        let _ = store::fail(&state.db, id, "the dump is bigger than the panel accepts").await;
        let _ = tokio::fs::remove_file(spool_path(&state.spool_dir, id)).await;
        return Err(AppError::validation(vec![FieldError::new(
            "body",
            "file_too_big",
            "The dump is bigger than the panel accepts.",
        )]));
    }
    write_chunk(
        &spool_path(&state.spool_dir, id),
        target.received_bytes,
        &body,
    )
    .await
    .map_err(internal)?;
    if !store::chunk_written(&state.db, id, n, new_total)
        .await
        .map_err(internal)?
    {
        return Err(AppError::conflict(
            "wrong_chunk",
            "Another request sent this chunk first.",
        ));
    }
    Ok(StatusCode::OK)
}

/// What the node says of the finished dump.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finished {
    /// Size of the compressed dump.
    pub size_bytes: u64,
    /// Its SHA-256 (hex).
    pub sha256: String,
}

/// The node says the dump is whole: it is checked and the export is `ready`.
pub async fn upload_complete(
    State(state): State<DatabasesState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(said): Json<Finished>,
) -> Result<StatusCode, AppError> {
    let target = authorised(&state, id, &headers).await?;
    let size = i64::try_from(said.size_bytes).unwrap_or(i64::MAX);
    let sha = said.sha256.to_ascii_lowercase();
    if target.received_bytes != size || size == 0 {
        let _ = store::fail(
            &state.db,
            id,
            "the dump that arrived is not the size announced",
        )
        .await;
        let _ = tokio::fs::remove_file(spool_path(&state.spool_dir, id)).await;
        return Err(AppError::conflict(
            "incomplete",
            "The dump that arrived is not the size announced.",
        ));
    }
    let path = spool_path(&state.spool_dir, id);
    let found = checksum(path.clone()).await.map_err(internal)?;
    if found != sha {
        let _ = store::fail(
            &state.db,
            id,
            "the dump that arrived does not match its checksum",
        )
        .await;
        let _ = tokio::fs::remove_file(&path).await;
        return Err(AppError::validation(vec![FieldError::new(
            "sha256",
            "checksum_mismatch",
            "The dump that arrived does not match its checksum.",
        )]));
    }
    store::ready(&state.db, id, size, &found, KEEP_HOURS)
        .await
        .map_err(internal)?;
    Ok(StatusCode::OK)
}

/// The link's token.
#[derive(Debug, Deserialize)]
pub struct LinkQuery {
    token: String,
}

/// A client downloads the file with a link that has not run out.
pub async fn download(
    State(state): State<DatabasesState>,
    Path(id): Path<Uuid>,
    Query(q): Query<LinkQuery>,
    request: Request,
) -> Result<Response, AppError> {
    let denied = || AppError::authentication_failed("bad_link", "This link does not work.");
    let (wanted, database) = store::link_target(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(denied)?;
    if !token_matches(&q.token, &wanted) {
        return Err(denied());
    }
    // Serving a file cannot fail as a service: a missing file is a 404 response.
    let Ok(served) = tower_http::services::ServeFile::new(spool_path(&state.spool_dir, id))
        .oneshot(request)
        .await;
    let mut response = served.map(Body::new);
    let h = response.headers_mut();
    h.insert(
        header::CONTENT_ENCODING,
        HeaderValue::from_static("identity"),
    );
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/gzip"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    // The name is a plain identifier (letters, digits, `_`), so it is safe in a header.
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{database}.sql.gz\"")) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    Ok(response)
}
