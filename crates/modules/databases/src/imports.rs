//! SQL imports (T07.07).
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | POST | `/api/v1/client/databases/{id}/imports` | the owner; the `databases` feature |
//! | GET | `/api/v1/client/imports` | a client |
//! | GET, DELETE | `/api/v1/client/imports/{id}` | the owner |
//! | PUT | `/api/v1/client/imports/{id}/chunks/{n}` | the owner |
//! | POST | `/api/v1/client/imports/{id}/complete` | the owner |
//! | GET | `/api/v1/public/imports/{id}/data` | the node, with the import's one-time token |
//!
//! The browser uploads the dump in chunks of at most [`MAX_CHUNK_BYTES`], in order
//! (chunk 0, 1, 2, ...; a lost answer is repaired by asking where the import
//! stands). They go to a spool file in the control plane's `imports.spool_dir`.
//! When the file is whole its size and SHA-256 are checked and the node is asked
//! to fetch it (`ImportSql`) with a token that only that import knows; the node
//! runs the statements as a temporary account that has rights on that database
//! only, and the spool file is removed as soon as the import has ended.

use std::path::{Path as FsPath, PathBuf};

use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Path, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
    routing::{get, post, put},
};
use builtpanel_database::imports::{self as store, Import, StartError};
use builtpanel_domain::TenantUsername;
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_node_protocol::Operation;
use builtpanel_rbac::Principal;
use builtpanel_session::rfc3339;
use builtpanel_settings::Scope;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tower::ServiceExt;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{DatabasesState, client_hosting, entry, internal, queue, require_feature, running};

/// The largest chunk of an upload.
pub const MAX_CHUNK_BYTES: usize = 8 * 1024 * 1024;
/// The default of the setting `max_import_mb`.
const DEFAULT_MAX_IMPORT_MB: i64 = 1024;
/// How long an upload may be left alone before it is cancelled.
const ABANDONED_AFTER_HOURS: i32 = 24;

/// Where the dump of an import waits.
#[must_use]
pub fn spool_path(dir: &FsPath, id: Uuid) -> PathBuf {
    dir.join(format!("{}.dump", id.simple()))
}

// ------------------------------------------------------------------- routes

/// Routes for clients, with full paths.
pub fn client_router() -> Router<DatabasesState> {
    Router::new()
        .route("/api/v1/client/databases/{id}/imports", post(start_import))
        .route("/api/v1/client/imports", get(list_imports))
        .route(
            "/api/v1/client/imports/{id}",
            get(get_import).delete(cancel_import),
        )
        .route(
            "/api/v1/client/imports/{id}/chunks/{n}",
            put(put_chunk).layer(DefaultBodyLimit::max(MAX_CHUNK_BYTES)),
        )
        .route(
            "/api/v1/client/imports/{id}/complete",
            post(complete_import),
        )
}

/// The route the node fetches a dump from (no session: the token is the credential).
pub fn public_router() -> Router<DatabasesState> {
    Router::new().route("/api/v1/public/imports/{id}/data", get(download))
}

// -------------------------------------------------------------------- views

/// An import.
#[derive(Debug, Serialize, ToSchema)]
pub struct ImportView {
    /// Id.
    pub id: Uuid,
    /// The database it fills.
    pub database_id: Uuid,
    /// The database's name.
    #[schema(example = "alice_shop")]
    pub database: String,
    /// `uploading`, `queued`, `running`, `done`, `failed` or `cancelled`.
    pub status: String,
    /// Whether the file is gzip-compressed.
    pub gzip: bool,
    /// Size of the file in bytes.
    pub size_bytes: i64,
    /// Bytes the panel has received.
    pub received_bytes: i64,
    /// The chunk to send next (while `uploading`).
    pub next_chunk: i32,
    /// The largest chunk that is accepted, in bytes.
    pub max_chunk_bytes: usize,
    /// While `running`: `downloading`, `checking` or `importing`.
    pub stage: Option<String>,
    /// While `running`: bytes of the file the node has taken in so far.
    pub bytes_done: i64,
    /// While `running`: statements run so far.
    pub statements: i64,
    /// Why it failed.
    pub error: Option<String>,
    /// When it was started (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub created_at: String,
    /// When it ended (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub finished_at: Option<String>,
}

fn view(i: Import) -> ImportView {
    ImportView {
        id: i.id,
        database_id: i.database_id,
        database: i.database,
        status: i.status,
        gzip: i.gzip,
        size_bytes: i.size_bytes,
        received_bytes: i.received_bytes,
        next_chunk: i.next_chunk,
        max_chunk_bytes: MAX_CHUNK_BYTES,
        stage: i.stage,
        bytes_done: i.bytes_done,
        statements: i.statements,
        error: i.error,
        created_at: rfc3339(i.created_at),
        finished_at: i.finished_at.map(rfc3339),
    }
}

/// The imports of an account.
#[derive(Debug, Serialize, ToSchema)]
pub struct ImportList {
    /// Newest first (the last 50).
    pub items: Vec<ImportView>,
    /// The biggest file that is accepted, in MB.
    pub max_size_mb: i64,
}

/// A dump to import.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct StartImport {
    /// Size of the file in bytes (as uploaded: compressed, if it is).
    pub size_bytes: u64,
    /// The file's SHA-256, 64 hex digits.
    pub sha256: String,
    /// Whether the file is gzip-compressed (`.sql.gz`).
    #[serde(default)]
    pub gzip: bool,
}

// ----------------------------------------------------------------- handlers

async fn max_size_mb(state: &DatabasesState) -> Result<i64, AppError> {
    let mut conn = state.db.acquire().await.map_err(internal)?;
    let values = builtpanel_settings::get(&state.settings, &mut conn, "databases", Scope::Global)
        .await
        .map_err(internal)?;
    Ok(values
        .get("max_import_mb")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(DEFAULT_MAX_IMPORT_MB)
        .max(1))
}

async fn owned(state: &DatabasesState, tenant_id: Uuid, id: Uuid) -> Result<Import, AppError> {
    store::get(&state.db, tenant_id, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("import"))
}

/// Start an import: say what the file is, then upload it in chunks.
#[utoipa::path(
    post,
    path = "/api/v1/client/databases/{id}/imports",
    tag = "databases",
    operation_id = "clientStartImport",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Database id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = StartImport,
    responses(
        (status = 201, description = "Upload the chunks next", body = ImportView),
        (status = 403, description = "`client_only`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such database", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`import_active`, `database_not_ready`, `imports_unavailable` or `tenant_not_running`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "`invalid_checksum`, `empty_file` or `file_too_big`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn start_import(
    State(state): State<DatabasesState>,
    who: Principal,
    Path(database_id): Path<Uuid>,
    Json(req): Json<StartImport>,
) -> Result<(StatusCode, Json<ImportView>), AppError> {
    let h = client_hosting(&state, &who).await?;
    require_feature(&state, &h, "databases", "Databases are").await?;
    running(&h)?;
    if state.agent_url.is_none() {
        return Err(AppError::conflict(
            "imports_unavailable",
            "Imports are not set up on this panel (agent.public_url).",
        ));
    }
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
    let sha256 = req.sha256.to_ascii_lowercase();
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(AppError::validation(vec![FieldError::new(
            "sha256",
            "invalid_checksum",
            "The checksum is 64 hexadecimal digits (SHA-256 of the file).",
        )]));
    }
    if req.size_bytes == 0 {
        return Err(AppError::validation(vec![FieldError::new(
            "size_bytes",
            "empty_file",
            "The file is empty.",
        )]));
    }
    let max_mb = max_size_mb(&state).await?;
    let limit = u64::try_from(max_mb)
        .unwrap_or(1)
        .saturating_mul(1024 * 1024);
    if req.size_bytes > limit {
        return Err(AppError::validation(vec![FieldError::new(
            "size_bytes",
            "file_too_big",
            format!("Files of up to {max_mb} MB are accepted."),
        )]));
    }
    let size = i64::try_from(req.size_bytes).unwrap_or(i64::MAX);
    let mut tx = state.db.begin().await.map_err(internal)?;
    let id = store::start(
        &mut tx,
        h.tenant.id,
        db.id,
        req.gzip,
        size,
        &sha256,
        who.account_id(),
    )
    .await
    .map_err(|e| match e {
        StartError::Active => AppError::conflict(
            "import_active",
            "This database already has an import that has not ended.",
        ),
        StartError::Database(db) => internal(db),
    })?;
    builtpanel_audit::insert(
        &mut *tx,
        &entry("database.import_started", &who, &h, "database_import", id)
            .metadata(serde_json::json!({ "database": db.name, "size_bytes": size })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    let started = owned(&state, h.tenant.id, id).await?;
    Ok((StatusCode::CREATED, Json(view(started))))
}

/// The imports of the account.
#[utoipa::path(
    get,
    path = "/api/v1/client/imports",
    tag = "databases",
    operation_id = "clientListImports",
    security(("session" = [])),
    responses(
        (status = 200, description = "The imports", body = ImportList),
        (status = 403, description = "`client_only`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn list_imports(
    State(state): State<DatabasesState>,
    who: Principal,
) -> Result<Json<ImportList>, AppError> {
    let h = client_hosting(&state, &who).await?;
    let rows = store::list(&state.db, h.tenant.id, 50)
        .await
        .map_err(internal)?;
    Ok(Json(ImportList {
        items: rows.into_iter().map(view).collect(),
        max_size_mb: max_size_mb(&state).await?,
    }))
}

/// One import: where the upload stands, or how far the node has come.
#[utoipa::path(
    get,
    path = "/api/v1/client/imports/{id}",
    tag = "databases",
    operation_id = "clientGetImport",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Import id")),
    responses(
        (status = 200, description = "The import", body = ImportView),
        (status = 404, description = "No such import (or it is not yours)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn get_import(
    State(state): State<DatabasesState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<Json<ImportView>, AppError> {
    let h = client_hosting(&state, &who).await?;
    Ok(Json(view(owned(&state, h.tenant.id, id).await?)))
}

/// Send the next chunk of the file (the body is the raw bytes).
#[utoipa::path(
    put,
    path = "/api/v1/client/imports/{id}/chunks/{n}",
    tag = "databases",
    operation_id = "clientPutImportChunk",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Import id"),
        ("n" = i32, Path, description = "Chunk number, from 0; must be the one the import expects"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body(content = Vec<u8>, content_type = "application/octet-stream"),
    responses(
        (status = 200, description = "Kept; `next_chunk` is the one to send now", body = ImportView),
        (status = 404, description = "No such import", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`not_uploading` or `wrong_chunk` (ask for the import to see which one is expected)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 413, description = "The chunk is bigger than `max_chunk_bytes`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "`empty_chunk` or `too_much_data`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn put_chunk(
    State(state): State<DatabasesState>,
    who: Principal,
    Path((id, n)): Path<(Uuid, i32)>,
    body: Bytes,
) -> Result<Json<ImportView>, AppError> {
    let h = client_hosting(&state, &who).await?;
    let import = owned(&state, h.tenant.id, id).await?;
    if import.status != "uploading" {
        return Err(AppError::conflict(
            "not_uploading",
            "This import takes no more data.",
        ));
    }
    if n != import.next_chunk {
        return Err(AppError::conflict(
            "wrong_chunk",
            format!("Chunk {} is expected next.", import.next_chunk),
        ));
    }
    if body.is_empty() {
        return Err(AppError::validation(vec![FieldError::new(
            "body",
            "empty_chunk",
            "A chunk has at least one byte.",
        )]));
    }
    let new_total = import
        .received_bytes
        .saturating_add(i64::try_from(body.len()).unwrap_or(i64::MAX));
    if new_total > import.size_bytes {
        return Err(AppError::validation(vec![FieldError::new(
            "body",
            "too_much_data",
            "More data than the file was said to have.",
        )]));
    }
    write_chunk(
        &spool_path(&state.spool_dir, id),
        import.received_bytes,
        &body,
    )
    .await
    .map_err(internal)?;
    let mut conn = state.db.acquire().await.map_err(internal)?;
    if !store::chunk_written(&mut conn, id, n, new_total)
        .await
        .map_err(internal)?
    {
        return Err(AppError::conflict(
            "wrong_chunk",
            "Another request sent this chunk first; ask for the import to see where it stands.",
        ));
    }
    Ok(Json(view(owned(&state, h.tenant.id, id).await?)))
}

/// Appends a chunk at `offset` (anything past it, a half-written earlier try,
/// is cut off first).
pub(crate) async fn write_chunk(
    path: &FsPath,
    offset: i64,
    data: &[u8],
) -> Result<(), std::io::Error> {
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(path)
        .await?;
    let at = u64::try_from(offset).unwrap_or(0);
    file.set_len(at).await?;
    tokio::io::AsyncSeekExt::seek(&mut file, std::io::SeekFrom::Start(at)).await?;
    file.write_all(data).await?;
    file.sync_data().await
}

/// The file is whole: it is checked and the node is asked to run it.
#[utoipa::path(
    post,
    path = "/api/v1/client/imports/{id}/complete",
    tag = "databases",
    operation_id = "clientCompleteImport",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Import id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 202, description = "Queued; follow `status`, `stage` and `bytes_done`", body = ImportView),
        (status = 404, description = "No such import", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`not_uploading`, `incomplete` or `tenant_not_running`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "`checksum_mismatch`: the import has ended; start again", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn complete_import(
    State(state): State<DatabasesState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<ImportView>), AppError> {
    let h = client_hosting(&state, &who).await?;
    running(&h)?;
    let import = owned(&state, h.tenant.id, id).await?;
    if import.status != "uploading" {
        return Err(AppError::conflict(
            "not_uploading",
            "This import takes no more data.",
        ));
    }
    if import.received_bytes != import.size_bytes {
        return Err(AppError::conflict(
            "incomplete",
            format!(
                "{} of {} bytes have arrived.",
                import.received_bytes, import.size_bytes
            ),
        ));
    }
    let Some(agent_url) = state.agent_url.clone() else {
        return Err(AppError::conflict(
            "imports_unavailable",
            "Imports are not set up on this panel (agent.public_url).",
        ));
    };
    let path = spool_path(&state.spool_dir, id);
    let found = checksum(path.clone()).await.map_err(internal)?;
    if found != import.sha256 {
        let _ = store::end(
            &state.db,
            id,
            "failed",
            Some("the file that arrived does not match its checksum"),
        )
        .await;
        let _ = tokio::fs::remove_file(&path).await;
        return Err(AppError::validation(vec![FieldError::new(
            "sha256",
            "checksum_mismatch",
            "The file that arrived does not match its checksum; start the import again.",
        )]));
    }
    // The node's credential: only its hash is looked up, and the token itself is
    // kept sealed until the node has had it.
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let sealed = state
        .master_key
        .seal(&store::token_context(id), token.as_bytes())
        .map_err(internal)?;
    let hash = Sha256::digest(token.as_bytes());
    let tenant_name = TenantUsername::parse(&h.tenant.username).map_err(internal)?;
    let engine =
        builtpanel_database::databases::engine_of(&state.db, h.tenant.id, &import.database)
            .await
            .map_err(internal)?
            .and_then(|e| builtpanel_node_protocol::DbEngine::parse(&e))
            .unwrap_or_default();
    let mut tx = state.db.begin().await.map_err(internal)?;
    if !store::queue(&mut tx, id, &hash, &sealed)
        .await
        .map_err(internal)?
    {
        return Err(AppError::conflict(
            "not_uploading",
            "This import takes no more data.",
        ));
    }
    queue(
        &mut tx,
        &h,
        &Operation::ImportSql {
            username: tenant_name.as_str().to_owned(),
            database: import.database.clone(),
            import_id: id,
            url: format!("{agent_url}/api/v1/public/imports/{id}/data"),
            size_bytes: u64::try_from(import.size_bytes).unwrap_or(0),
            sha256: import.sha256.clone(),
            gzip: import.gzip,
            token: None,
            engine,
            dedicated: super::server_for(&state, &h, engine).await?,
        },
        format!("import:{id}"),
        &who,
    )
    .await?;
    builtpanel_audit::insert(
        &mut *tx,
        &entry("database.import_queued", &who, &h, "database_import", id)
            .metadata(serde_json::json!({ "database": import.database })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(view(owned(&state, h.tenant.id, id).await?)),
    ))
}

/// Stops an import that the node has not started yet (an upload, or a queued one).
#[utoipa::path(
    delete,
    path = "/api/v1/client/imports/{id}",
    tag = "databases",
    operation_id = "clientCancelImport",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Import id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 204, description = "Cancelled; the uploaded data is removed"),
        (status = 404, description = "No such import", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`import_running` (the node is at it) or `already_ended`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn cancel_import(
    State(state): State<DatabasesState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let h = client_hosting(&state, &who).await?;
    let import = owned(&state, h.tenant.id, id).await?;
    match import.status.as_str() {
        "uploading" | "queued" => {}
        "running" => {
            return Err(AppError::conflict(
                "import_running",
                "The node is running this import and it cannot be stopped half way.",
            ));
        }
        _ => {
            return Err(AppError::conflict(
                "already_ended",
                "This import has ended.",
            ));
        }
    }
    if store::end(&state.db, id, "cancelled", None)
        .await
        .map_err(internal)?
    {
        let _ = tokio::fs::remove_file(spool_path(&state.spool_dir, id)).await;
        builtpanel_audit::insert(
            &state.db,
            &entry("database.import_cancelled", &who, &h, "database_import", id),
        )
        .await
        .map_err(internal)?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// SHA-256 of a file, as hex.
pub(crate) async fn checksum(path: PathBuf) -> Result<String, std::io::Error> {
    tokio::task::spawn_blocking(move || {
        use std::io::Read as _;
        let mut file = std::fs::File::open(path)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Ok(hasher
            .finalize()
            .iter()
            .fold(String::with_capacity(64), |mut s, b| {
                use std::fmt::Write as _;
                let _ = write!(s, "{b:02x}");
                s
            }))
    })
    .await
    .map_err(std::io::Error::other)?
}

/// Whether `given` is the token whose SHA-256 is `wanted` (compared without
/// stopping at the first difference).
pub(crate) fn token_matches(given: &str, wanted: &[u8]) -> bool {
    let given = Sha256::digest(given.as_bytes());
    given.len() == wanted.len()
        && given
            .iter()
            .zip(wanted)
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

/// The node fetches the dump (`Authorization: Bearer <token>`; ranges work, so a
/// cut download goes on where it stopped).
pub async fn download(
    State(state): State<DatabasesState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    request: Request,
) -> Result<Response, AppError> {
    let denied = || AppError::authentication_failed("bad_token", "This download is not allowed.");
    let given = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(denied)?;
    let wanted = store::download_token_hash(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(denied)?;
    if !token_matches(given, &wanted) {
        return Err(denied());
    }
    // Serving a file cannot fail as a service: a missing file is a 404 response.
    let Ok(served) = tower_http::services::ServeFile::new(spool_path(&state.spool_dir, id))
        .oneshot(request)
        .await;
    let mut response = served.map(Body::new);
    // Never compressed on the way (a range of compressed bytes is of no use).
    response.headers_mut().insert(
        header::CONTENT_ENCODING,
        HeaderValue::from_static("identity"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

// -------------------------------------------------------------------- sweep

/// Cancels uploads that were left alone, expires exports whose time is up and
/// removes spool files nobody needs any more (an import or export that has ended,
/// or that does not exist). Run now and then.
///
/// # Errors
/// Database or file errors.
pub async fn sweep(
    db: &builtpanel_database::sqlx::PgPool,
    spool_dir: &FsPath,
) -> Result<usize, AppError> {
    store::cancel_abandoned(db, ABANDONED_AFTER_HOURS)
        .await
        .map_err(internal)?;
    builtpanel_database::exports::expire_due(db)
        .await
        .map_err(internal)?;
    let mut live: std::collections::HashSet<String> = std::collections::HashSet::new();
    for id in store::live_ids(db).await.map_err(internal)? {
        live.insert(format!("{}.dump", id.simple()));
    }
    for id in builtpanel_database::exports::live_ids(db)
        .await
        .map_err(internal)?
    {
        live.insert(format!("{}.sql.gz", id.simple()));
    }
    let mut entries = match tokio::fs::read_dir(spool_dir).await {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(internal(e)),
    };
    let mut removed = 0;
    while let Some(item) = entries.next_entry().await.map_err(internal)? {
        let path = item.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
            continue;
        };
        // Only files this module makes: `<32 hex>.dump` and `<32 hex>.sql.gz`.
        let stem = name
            .strip_suffix(".dump")
            .or_else(|| name.strip_suffix(".sql.gz"));
        let ours = stem.is_some_and(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()));
        if ours && !live.contains(&name) && tokio::fs::remove_file(&path).await.is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}
