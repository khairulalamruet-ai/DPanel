//! Big files between the browser and an account's files (T08.07).
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | POST | `/api/v1/client/files/uploads` | a client with the `files` feature |
//! | PUT | `/api/v1/client/files/uploads/{id}/chunks/{n}` | the owner |
//! | POST | `/api/v1/client/files/uploads/{id}/complete` | the owner |
//! | POST | `/api/v1/client/files/downloads` | the owner |
//! | GET, DELETE | `/api/v1/client/files/transfers/{id}` | the owner |
//! | POST | `/api/v1/client/files/transfers/{id}/link` | the owner |
//! | GET | `/api/v1/public/files/transfers/{id}/data` | the node, with the transfer's token |
//! | PUT, POST | `/api/v1/public/files/transfers/{id}/chunks/{n}`, `.../complete` | the node |
//! | GET | `/api/v1/public/files/transfers/{id}/download?token=` | a link (five minutes) |
//!
//! **Upload:** the browser sends the file in chunks of at most 8 MiB, in order (a lost
//! answer or a closed tab is repaired by asking where the transfer stands and going on
//! from the chunk it expects), then says it is whole; the node fetches it (ranges, so a
//! cut fetch goes on) and puts it in the account's files as the account's own user.
//! **Download:** the node reads the file as the account's user and sends it here in
//! chunks; when it is whole the client asks for a link that works for five minutes.

use std::path::{Path as FsPath, PathBuf};

use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
    routing::{get, post, put},
};
use builtpanel_database::file_transfers::{self as store, Transfer};
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

use crate::{FilesState, client_hosting, internal};

/// The largest chunk.
pub const MAX_CHUNK_BYTES: usize = 8 * 1024 * 1024;
/// How long a finished download waits to be fetched.
const KEEP_HOURS: i32 = 24;
/// How long a download link works.
pub const LINK_MINUTES: i32 = 5;
const OPERATION_TTL_SECS: i32 = 6 * 3600;

fn spool_path(dir: &FsPath, id: Uuid, kind: &str) -> PathBuf {
    dir.join(format!("{}.{kind}", id.simple()))
}

/// Routes for clients, with full paths.
pub fn client_router() -> Router<FilesState> {
    Router::new()
        .route("/api/v1/client/files/uploads", post(start_upload))
        .route(
            "/api/v1/client/files/uploads/{id}/chunks/{n}",
            put(put_chunk).layer(DefaultBodyLimit::max(MAX_CHUNK_BYTES)),
        )
        .route(
            "/api/v1/client/files/uploads/{id}/complete",
            post(complete_upload),
        )
        .route("/api/v1/client/files/downloads", post(start_download))
        .route(
            "/api/v1/client/files/transfers/{id}",
            get(get_transfer).delete(cancel_transfer),
        )
        .route("/api/v1/client/files/transfers/{id}/link", post(make_link))
}

/// Routes without a session.
pub fn public_router() -> Router<FilesState> {
    Router::new()
        .route(
            "/api/v1/public/files/transfers/{id}/data",
            get(node_fetches),
        )
        .route(
            "/api/v1/public/files/transfers/{id}/chunks/{n}",
            put(node_sends_chunk).layer(DefaultBodyLimit::max(MAX_CHUNK_BYTES)),
        )
        .route(
            "/api/v1/public/files/transfers/{id}/complete",
            post(node_sends_done),
        )
        .route(
            "/api/v1/public/files/transfers/{id}/download",
            get(download),
        )
}

// -------------------------------------------------------------------- views

/// A transfer.
#[derive(Debug, Serialize, ToSchema)]
pub struct TransferView {
    /// Id.
    pub id: Uuid,
    /// `upload` or `download`.
    pub kind: String,
    /// The file in the account's files.
    #[schema(example = "www/backup.zip")]
    pub path: String,
    /// `uploading`, `queued`, `running`, `ready` (a download), `done` (an upload),
    /// `failed`, `cancelled` or `expired`.
    pub status: String,
    /// Size in bytes (a download: known when it is `ready`).
    pub size_bytes: Option<i64>,
    /// Bytes the panel has received.
    pub received_bytes: i64,
    /// The chunk to send next (an upload that is `uploading`).
    pub next_chunk: i32,
    /// The largest chunk accepted, in bytes.
    pub max_chunk_bytes: usize,
    /// Why it failed.
    pub error: Option<String>,
    /// When it was started (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub created_at: String,
    /// When a finished download goes (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub expires_at: Option<String>,
}

fn view(t: Transfer) -> TransferView {
    TransferView {
        id: t.id,
        kind: t.kind,
        path: t.path,
        status: t.status,
        size_bytes: t.size_bytes,
        received_bytes: t.received_bytes,
        next_chunk: t.next_chunk,
        max_chunk_bytes: MAX_CHUNK_BYTES,
        error: t.error,
        created_at: rfc3339(t.created_at),
        expires_at: t.expires_at.map(rfc3339),
    }
}

async fn owned(state: &FilesState, tenant: Uuid, id: Uuid) -> Result<Transfer, AppError> {
    store::get(&state.db, tenant, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("transfer"))
}

async fn max_mb(state: &FilesState, key: &str, default: i64) -> Result<i64, AppError> {
    let mut conn = state.db.acquire().await.map_err(internal)?;
    let values = builtpanel_settings::get(&state.settings, &mut conn, "files", Scope::Global)
        .await
        .map_err(internal)?;
    Ok(values
        .get(key)
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(default)
        .max(1))
}

/// A path as text: relative, no `..`, no NUL, not the trash.
fn check_path(path: &str) -> Result<(), AppError> {
    let bad = |code: &'static str, msg: &'static str| {
        AppError::validation(vec![FieldError::new("path", code, msg)])
    };
    if path.is_empty() || path.len() > 4096 || path.contains('\0') {
        return Err(bad("invalid_path", "The path is not acceptable."));
    }
    if path.starts_with('/') {
        return Err(bad("invalid_path", "The path must be relative."));
    }
    let parts: Vec<&str> = path
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    if parts.is_empty() || parts.contains(&"..") || parts.len() > 64 || parts[0] == ".trash" {
        return Err(bad("invalid_path", "The path is not acceptable."));
    }
    if parts.iter().any(|p| p.len() > 255) {
        return Err(bad("invalid_path", "A name in the path is too long."));
    }
    Ok(())
}

fn agent_url(state: &FilesState) -> Result<&str, AppError> {
    state.agent_url.as_deref().ok_or_else(|| {
        AppError::conflict(
            "transfers_unavailable",
            "Big file transfers are not set up on this panel (agent.public_url).",
        )
    })
}

fn token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

fn hash_of(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

fn token_matches(given: &str, wanted: &[u8]) -> bool {
    let given = Sha256::digest(given.as_bytes());
    given.len() == wanted.len()
        && given
            .iter()
            .zip(wanted)
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

/// Queues a node operation for a transfer.
async fn queue(
    state: &FilesState,
    tenant: &builtpanel_database::tenants::Tenant,
    op: &Operation,
    id: Uuid,
    by: Uuid,
) -> Result<(), AppError> {
    let payload = serde_json::to_value(op).map_err(internal)?;
    let mut conn = state.db.acquire().await.map_err(internal)?;
    builtpanel_database::operations::enqueue_in(
        &mut conn,
        &builtpanel_database::operations::NewOperation {
            node_id: tenant.node_id,
            kind: op.type_name(),
            tenant_id: Some(tenant.id),
            payload: &payload,
            idempotency_key: &format!("transfer:{id}"),
            created_by: Some(by),
            ttl_secs: OPERATION_TTL_SECS,
            max_attempts: 3,
        },
    )
    .await
    .map_err(internal)?;
    Ok(())
}

// ------------------------------------------------------------------ uploads

/// A file to upload.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct StartUpload {
    /// Where it goes in the account's files (the folders are made if missing).
    #[schema(example = "www/backup.zip")]
    pub path: String,
    /// Its size in bytes.
    pub size_bytes: u64,
    /// The file's SHA-256 (64 hex digits), if the browser has it: checked at the end.
    pub sha256: Option<String>,
    /// Replace a file that is already there.
    #[serde(default)]
    pub overwrite: bool,
}

/// Start an upload: say where the file goes and how big it is, then send its chunks.
#[utoipa::path(
    post,
    path = "/api/v1/client/files/uploads",
    tag = "files",
    operation_id = "clientStartUpload",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = StartUpload,
    responses(
        (status = 201, description = "Send the chunks next", body = TransferView),
        (status = 409, description = "`transfers_unavailable`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 413, description = "`too_large`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "`invalid_path`, `empty_file` or `invalid_checksum`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn start_upload(
    State(state): State<FilesState>,
    who: Principal,
    Json(req): Json<StartUpload>,
) -> Result<(StatusCode, Json<TransferView>), AppError> {
    let h = client_hosting(&state, &who).await?;
    agent_url(&state)?;
    check_path(&req.path)?;
    if req.size_bytes == 0 {
        return Err(AppError::validation(vec![FieldError::new(
            "size_bytes",
            "empty_file",
            "The file is empty.",
        )]));
    }
    let max = max_mb(&state, "max_upload_mb", 4096).await?;
    if req.size_bytes > u64::try_from(max).unwrap_or(1).saturating_mul(1024 * 1024) {
        return Err(AppError::payload_too_large(
            "too_large",
            format!("Files of up to {max} MB are accepted."),
        ));
    }
    let sha = match &req.sha256 {
        Some(s) => {
            let s = s.to_ascii_lowercase();
            if s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(AppError::validation(vec![FieldError::new(
                    "sha256",
                    "invalid_checksum",
                    "The checksum is 64 hexadecimal digits.",
                )]));
            }
            Some(s)
        }
        None => None,
    };
    let id = store::start(
        &state.db,
        h.tenant.id,
        "upload",
        &req.path,
        req.overwrite,
        Some(i64::try_from(req.size_bytes).unwrap_or(i64::MAX)),
        None,
        who.account_id(),
    )
    .await
    .map_err(internal)?;
    if let Some(sha) = sha {
        builtpanel_database::sqlx::query("UPDATE file_transfers SET sha256 = $2 WHERE id = $1")
            .bind(id)
            .bind(sha)
            .execute(&state.db)
            .await
            .map_err(internal)?;
    }
    let t = owned(&state, h.tenant.id, id).await?;
    Ok((StatusCode::CREATED, Json(view(t))))
}

/// Send the next chunk (the body is the raw bytes).
#[utoipa::path(
    put,
    path = "/api/v1/client/files/uploads/{id}/chunks/{n}",
    tag = "files",
    operation_id = "clientPutUploadChunk",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Transfer id"),
        ("n" = i32, Path, description = "Chunk number, from 0; the one the transfer expects"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body(content = Vec<u8>, content_type = "application/octet-stream"),
    responses(
        (status = 200, description = "Kept; `next_chunk` is the one to send now", body = TransferView),
        (status = 409, description = "`not_uploading` or `wrong_chunk` (ask for the transfer to see which is expected)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 413, description = "Bigger than `max_chunk_bytes`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "`empty_chunk` or `too_much_data`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn put_chunk(
    State(state): State<FilesState>,
    who: Principal,
    Path((id, n)): Path<(Uuid, i32)>,
    body: Bytes,
) -> Result<Json<TransferView>, AppError> {
    let h = client_hosting(&state, &who).await?;
    let t = owned(&state, h.tenant.id, id).await?;
    if t.kind != "upload" || t.status != "uploading" {
        return Err(AppError::conflict(
            "not_uploading",
            "This transfer takes no more data.",
        ));
    }
    if n != t.next_chunk {
        return Err(AppError::conflict(
            "wrong_chunk",
            format!("Chunk {} is expected next.", t.next_chunk),
        ));
    }
    if body.is_empty() {
        return Err(AppError::validation(vec![FieldError::new(
            "body",
            "empty_chunk",
            "A chunk has at least one byte.",
        )]));
    }
    let new_total = t
        .received_bytes
        .saturating_add(i64::try_from(body.len()).unwrap_or(i64::MAX));
    if new_total > t.size_bytes.unwrap_or(0) {
        return Err(AppError::validation(vec![FieldError::new(
            "body",
            "too_much_data",
            "More data than the file was said to have.",
        )]));
    }
    write_chunk(
        &spool_path(&state.spool_dir, id, "upload"),
        t.received_bytes,
        &body,
    )
    .await
    .map_err(internal)?;
    if !store::upload_chunk_written(&state.db, h.tenant.id, id, n, new_total)
        .await
        .map_err(internal)?
    {
        return Err(AppError::conflict(
            "wrong_chunk",
            "Another request sent this chunk first.",
        ));
    }
    Ok(Json(view(owned(&state, h.tenant.id, id).await?)))
}

async fn write_chunk(path: &FsPath, offset: i64, data: &[u8]) -> Result<(), std::io::Error> {
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

async fn checksum(path: PathBuf) -> Result<String, std::io::Error> {
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

/// The file is whole: the node is asked to take it.
#[utoipa::path(
    post,
    path = "/api/v1/client/files/uploads/{id}/complete",
    tag = "files",
    operation_id = "clientCompleteUpload",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Transfer id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 202, description = "Queued; follow `status` (`queued`, `running`, `done`)", body = TransferView),
        (status = 409, description = "`not_uploading` or `incomplete`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "`checksum_mismatch`: the transfer has ended; start again", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn complete_upload(
    State(state): State<FilesState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<TransferView>), AppError> {
    let h = client_hosting(&state, &who).await?;
    let base = agent_url(&state)?.to_owned();
    let t = owned(&state, h.tenant.id, id).await?;
    if t.kind != "upload" || t.status != "uploading" {
        return Err(AppError::conflict(
            "not_uploading",
            "This transfer takes no more data.",
        ));
    }
    let size = t.size_bytes.unwrap_or(0);
    if t.received_bytes != size {
        return Err(AppError::conflict(
            "incomplete",
            format!("{} of {size} bytes have arrived.", t.received_bytes),
        ));
    }
    let path = spool_path(&state.spool_dir, id, "upload");
    let wanted: Option<String> =
        builtpanel_database::sqlx::query_scalar("SELECT sha256 FROM file_transfers WHERE id = $1")
            .bind(id)
            .fetch_one(&state.db)
            .await
            .map_err(internal)?;
    if let Some(wanted) = wanted
        && checksum(path.clone()).await.map_err(internal)? != wanted
    {
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
            "The file that arrived does not match its checksum; start again.",
        )]));
    }
    let token = token();
    let sealed = state
        .master_key
        .seal(&store::token_context(id), token.as_bytes())
        .map_err(internal)?;
    if !store::queue_upload(&state.db, id, &hash_of(&token), &sealed)
        .await
        .map_err(internal)?
    {
        return Err(AppError::conflict(
            "not_uploading",
            "This transfer takes no more data.",
        ));
    }
    let op = Operation::FetchFile {
        tenant_id: h.tenant.id,
        uid: u32::try_from(h.tenant.uid).map_err(internal)?,
        transfer_id: id,
        dest: t.path.clone(),
        url: format!("{base}/api/v1/public/files/transfers/{id}/data"),
        size_bytes: u64::try_from(size).unwrap_or(0),
        overwrite: t.overwrite,
        token: None,
    };
    queue(&state, &h.tenant, &op, id, who.account_id()).await?;
    builtpanel_audit::insert(
        &state.db,
        &crate::audit(
            "files.uploaded",
            &who,
            &h,
            serde_json::json!({ "path": t.path, "bytes": size }),
        ),
    )
    .await
    .map_err(internal)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(view(owned(&state, h.tenant.id, id).await?)),
    ))
}

// ---------------------------------------------------------------- downloads

/// A file to download.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct StartDownload {
    /// The file in the account's files.
    #[schema(example = "www/backup.zip")]
    pub path: String,
}

/// Start a download: the node sends the file here; when the transfer is `ready` ask for a link.
#[utoipa::path(
    post,
    path = "/api/v1/client/files/downloads",
    tag = "files",
    operation_id = "clientStartDownload",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = StartDownload,
    responses(
        (status = 202, description = "Queued; follow `status` until `ready`", body = TransferView),
        (status = 409, description = "`transfers_unavailable`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "`invalid_path`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn start_download(
    State(state): State<FilesState>,
    who: Principal,
    Json(req): Json<StartDownload>,
) -> Result<(StatusCode, Json<TransferView>), AppError> {
    let h = client_hosting(&state, &who).await?;
    let base = agent_url(&state)?.to_owned();
    check_path(&req.path)?;
    let max = max_mb(&state, "max_download_mb", 4096).await?;
    let token = token();
    let id = store::start(
        &state.db,
        h.tenant.id,
        "download",
        &req.path,
        false,
        None,
        Some(&hash_of(&token)),
        who.account_id(),
    )
    .await
    .map_err(internal)?;
    let sealed = state
        .master_key
        .seal(&store::token_context(id), token.as_bytes())
        .map_err(internal)?;
    store::keep_sealed_token(&state.db, id, &sealed)
        .await
        .map_err(internal)?;
    let op = Operation::SendFile {
        tenant_id: h.tenant.id,
        uid: u32::try_from(h.tenant.uid).map_err(internal)?,
        transfer_id: id,
        path: req.path.clone(),
        url: format!("{base}/api/v1/public/files/transfers/{id}"),
        max_bytes: u64::try_from(max).unwrap_or(1).saturating_mul(1024 * 1024),
        token: None,
    };
    queue(&state, &h.tenant, &op, id, who.account_id()).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(view(owned(&state, h.tenant.id, id).await?)),
    ))
}

/// A download link.
#[derive(Debug, Serialize, ToSchema)]
pub struct FileDownloadLink {
    /// Where to download from (a path on this panel).
    pub url: String,
    /// Until when the link works (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub expires_at: String,
}

/// A link that downloads the finished file; it works for five minutes.
#[utoipa::path(
    post,
    path = "/api/v1/client/files/transfers/{id}/link",
    tag = "files",
    operation_id = "clientTransferLink",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Transfer id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 200, description = "The link (a new one replaces the last)", body = FileDownloadLink),
        (status = 409, description = "`not_ready`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn make_link(
    State(state): State<FilesState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<Json<FileDownloadLink>, AppError> {
    let h = client_hosting(&state, &who).await?;
    let t = owned(&state, h.tenant.id, id).await?;
    if t.kind != "download" || t.status != "ready" {
        return Err(AppError::conflict(
            "not_ready",
            "This download is not ready (or its time is up).",
        ));
    }
    let token = token();
    let at = store::set_link(&state.db, h.tenant.id, id, &hash_of(&token), LINK_MINUTES)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::conflict("not_ready", "This download is not ready."))?;
    builtpanel_audit::insert(
        &state.db,
        &crate::audit(
            "files.download_link",
            &who,
            &h,
            serde_json::json!({ "path": t.path }),
        ),
    )
    .await
    .map_err(internal)?;
    Ok(Json(FileDownloadLink {
        url: format!("/api/v1/public/files/transfers/{id}/download?token={token}"),
        expires_at: rfc3339(at),
    }))
}

// ---------------------------------------------------- state, cancel (client)

/// Where a transfer stands.
#[utoipa::path(
    get,
    path = "/api/v1/client/files/transfers/{id}",
    tag = "files",
    operation_id = "clientGetTransfer",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Transfer id")),
    responses(
        (status = 200, description = "The transfer (for an upload: where to go on from)", body = TransferView),
        (status = 404, description = "No such transfer (or not yours)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn get_transfer(
    State(state): State<FilesState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<Json<TransferView>, AppError> {
    let h = client_hosting(&state, &who).await?;
    Ok(Json(view(owned(&state, h.tenant.id, id).await?)))
}

/// Give up (an upload not yet taken by the node, a download not yet fetched) or remove a finished download.
#[utoipa::path(
    delete,
    path = "/api/v1/client/files/transfers/{id}",
    tag = "files",
    operation_id = "clientCancelTransfer",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Transfer id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 204, description = "Gone; the spooled data is removed"),
        (status = 409, description = "`transfer_running` or `already_ended`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn cancel_transfer(
    State(state): State<FilesState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let h = client_hosting(&state, &who).await?;
    let t = owned(&state, h.tenant.id, id).await?;
    let done = match (t.kind.as_str(), t.status.as_str()) {
        (_, "uploading" | "queued") => store::end(&state.db, id, "cancelled", None)
            .await
            .map_err(internal)?,
        ("download", "ready") => store::expire(&state.db, id).await.map_err(internal)?,
        (_, "running") => {
            return Err(AppError::conflict(
                "transfer_running",
                "The node is working on this transfer.",
            ));
        }
        _ => false,
    };
    if !done {
        return Err(AppError::conflict(
            "already_ended",
            "This transfer has ended.",
        ));
    }
    let _ = tokio::fs::remove_file(spool_path(&state.spool_dir, id, &t.kind)).await;
    Ok(StatusCode::NO_CONTENT)
}

// ------------------------------------------------------------ the node's side

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

async fn node_auth(
    state: &FilesState,
    id: Uuid,
    kind: &str,
    headers: &HeaderMap,
) -> Result<(i64, i32), AppError> {
    let denied = || AppError::authentication_failed("bad_token", "This transfer is not allowed.");
    let given = bearer(headers).ok_or_else(denied)?;
    let (hash, received, next) = store::node_token_hash(&state.db, id, kind)
        .await
        .map_err(internal)?
        .ok_or_else(denied)?;
    if token_matches(given, &hash) {
        Ok((received, next))
    } else {
        Err(denied())
    }
}

/// The node fetches an upload (ranges work).
pub async fn node_fetches(
    State(state): State<FilesState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    request: Request,
) -> Result<Response, AppError> {
    node_auth(&state, id, "upload", &headers).await?;
    let Ok(served) =
        tower_http::services::ServeFile::new(spool_path(&state.spool_dir, id, "upload"))
            .oneshot(request)
            .await;
    let mut response = served.map(Body::new);
    let h = response.headers_mut();
    h.insert(
        header::CONTENT_ENCODING,
        HeaderValue::from_static("identity"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

/// The node sends the next chunk of a download.
pub async fn node_sends_chunk(
    State(state): State<FilesState>,
    Path((id, n)): Path<(Uuid, i32)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, AppError> {
    let (received, next) = node_auth(&state, id, "download", &headers).await?;
    if n != next {
        return Err(AppError::conflict(
            "wrong_chunk",
            format!("Chunk {next} is expected next."),
        ));
    }
    if body.is_empty() {
        return Err(AppError::validation(vec![FieldError::new(
            "body",
            "empty_chunk",
            "A chunk has at least one byte.",
        )]));
    }
    let new_total = received.saturating_add(i64::try_from(body.len()).unwrap_or(i64::MAX));
    let limit = max_mb(&state, "max_download_mb", 4096)
        .await?
        .saturating_mul(1024 * 1024);
    if new_total > limit {
        let _ = store::end(
            &state.db,
            id,
            "failed",
            Some("the file is bigger than the panel accepts"),
        )
        .await;
        let _ = tokio::fs::remove_file(spool_path(&state.spool_dir, id, "download")).await;
        return Err(AppError::payload_too_large(
            "too_large",
            "The file is bigger than the panel accepts.",
        ));
    }
    write_chunk(
        &spool_path(&state.spool_dir, id, "download"),
        received,
        &body,
    )
    .await
    .map_err(internal)?;
    if !store::download_chunk_written(&state.db, id, n, new_total)
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

/// What the node says of the finished file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finished {
    /// Size of the file.
    pub size_bytes: u64,
    /// Its SHA-256 (hex).
    pub sha256: String,
}

/// The node says the download is whole: it is checked and made `ready`.
pub async fn node_sends_done(
    State(state): State<FilesState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(said): Json<Finished>,
) -> Result<StatusCode, AppError> {
    let (received, _) = node_auth(&state, id, "download", &headers).await?;
    let size = i64::try_from(said.size_bytes).unwrap_or(i64::MAX);
    let path = spool_path(&state.spool_dir, id, "download");
    if received != size || size == 0 {
        let _ = store::end(
            &state.db,
            id,
            "failed",
            Some("the file that arrived is not the size announced"),
        )
        .await;
        let _ = tokio::fs::remove_file(&path).await;
        return Err(AppError::conflict(
            "incomplete",
            "The file that arrived is not the size announced.",
        ));
    }
    let found = checksum(path.clone()).await.map_err(internal)?;
    if found != said.sha256.to_ascii_lowercase() {
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
            "The file that arrived does not match its checksum.",
        )]));
    }
    store::download_ready(&state.db, id, size, &found, KEEP_HOURS)
        .await
        .map_err(internal)?;
    Ok(StatusCode::OK)
}

/// The link's token.
#[derive(Debug, Deserialize)]
pub struct LinkQuery {
    token: String,
}

/// A client downloads a finished file with a link that has not run out.
pub async fn download(
    State(state): State<FilesState>,
    Path(id): Path<Uuid>,
    Query(q): Query<LinkQuery>,
    request: Request,
) -> Result<Response, AppError> {
    let denied = || AppError::authentication_failed("bad_link", "This link does not work.");
    let (wanted, path) = store::link_target(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(denied)?;
    if !token_matches(&q.token, &wanted) {
        return Err(denied());
    }
    let Ok(served) =
        tower_http::services::ServeFile::new(spool_path(&state.spool_dir, id, "download"))
            .oneshot(request)
            .await;
    let mut response = served.map(Body::new);
    let name: String = path
        .rsplit('/')
        .next()
        .unwrap_or("download")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let h = response.headers_mut();
    h.insert(
        header::CONTENT_ENCODING,
        HeaderValue::from_static("identity"),
    );
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{name}\"")) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    Ok(response)
}

// -------------------------------------------------------------------- sweep

/// Expires downloads whose time is up, cancels forgotten uploads and removes spool files
/// nobody needs any more. Returns how many files were removed.
///
/// # Errors
/// Database or file errors.
pub async fn sweep(
    db: &builtpanel_database::sqlx::PgPool,
    spool_dir: &FsPath,
) -> Result<usize, AppError> {
    store::tidy(db, 24).await.map_err(internal)?;
    let live: std::collections::HashSet<Uuid> = store::live_ids(db)
        .await
        .map_err(internal)?
        .into_iter()
        .collect();
    let mut entries = match tokio::fs::read_dir(spool_dir).await {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(internal(e)),
    };
    let mut removed = 0;
    while let Some(item) = entries.next_entry().await.map_err(internal)? {
        let name = item.file_name().to_string_lossy().into_owned();
        let stem = name
            .strip_suffix(".upload")
            .or_else(|| name.strip_suffix(".download"));
        let id = stem
            .filter(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            .and_then(|s| Uuid::parse_str(s).ok());
        if let Some(id) = id
            && !live.contains(&id)
            && tokio::fs::remove_file(item.path()).await.is_ok()
        {
            removed += 1;
        }
    }
    Ok(removed)
}
