//! The file manager's API (T08.08).
//!
//! Every request becomes one node operation (`file_op`) that the node does as the
//! account's own user on the account's own files; the API waits a few seconds for
//! the answer and, for work that takes longer (packing and unpacking, copying a
//! big folder), answers `202` with an operation id to ask about.
//!
//! | Method | Path |
//! |---|---|
//! | GET | `/api/v1/client/files?path=&offset=&limit=` a page of a folder |
//! | GET | `/api/v1/client/files/stat?path=` one path |
//! | GET, PUT | `/api/v1/client/files/content` read, write a text file |
//! | POST | `/api/v1/client/files/folders` make a folder |
//! | POST | `/api/v1/client/files/move`, `/copy` |
//! | DELETE | `/api/v1/client/files?path=` move to the trash |
//! | GET, DELETE | `/api/v1/client/files/trash` list, empty |
//! | POST | `/api/v1/client/files/trash/{id}/restore` |
//! | POST | `/api/v1/client/files/permissions`, `/permissions/fix` |
//! | POST | `/api/v1/client/files/archive`, `/extract` |
//! | GET | `/api/v1/client/files/operations/{id}` the answer to a `202` |
//!
//! Everything needs a hosting account whose plan includes `files`. Writing,
//! deleting, moving, copying, restoring, permissions, packing and unpacking are
//! put in the audit log.

pub mod ftp;
pub mod transfers;

use std::{path::PathBuf, sync::Arc, time::Duration};

use axum::{
    Json, Router,
    extract::{FromRef, Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use builtpanel_audit::{AuditEntry, Outcome};
use builtpanel_database::{
    accounts::DbAccountKind,
    operations::{self, EnqueueError, NewOperation, Status},
    plans,
    sqlx::PgPool,
    tenants, users,
};
use builtpanel_http_kit::{AppError, ProblemDetails};
use builtpanel_node_protocol::{
    Operation,
    files::{ArchiveFormat, FileErrorCode, FileRequest, FileResult, MAX_EDIT_BYTES},
};
use builtpanel_rbac::Principal;
use builtpanel_session::SessionStore;
use builtpanel_settings::{Scope, SettingsRegistry};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, OpenApi, ToSchema};
use uuid::Uuid;

/// The module's manifest.
pub const MANIFEST: &str = include_str!("../../../../modules/files/module.toml");
/// The module's settings schema.
pub const SETTINGS_SCHEMA: &str = include_str!("../../../../modules/files/settings.schema.json");

/// How long a request waits for the node by default.
pub const DEFAULT_WAIT: Duration = Duration::from_secs(20);
const OPERATION_TTL_SECS: i32 = 3600;

/// What the module's handlers share.
#[derive(Clone)]
pub struct FilesState {
    /// Database pool.
    pub db: PgPool,
    /// Session store.
    pub sessions: SessionStore,
    /// The checked settings schemas of all modules.
    pub settings: Arc<SettingsRegistry>,
    /// How long a request waits for the node before answering `202`.
    pub wait: Duration,
    /// Seals the tokens of transfers until the node has them.
    pub master_key: builtpanel_auth_core::secretbox::MasterKey,
    /// Where big files wait for the node, or for the client (`imports.spool_dir`).
    pub spool_dir: PathBuf,
    /// Where nodes reach this panel (`agent.public_url`); without it no transfers.
    pub agent_url: Option<String>,
}

impl FromRef<FilesState> for PgPool {
    fn from_ref(s: &FilesState) -> Self {
        s.db.clone()
    }
}

impl FromRef<FilesState> for SessionStore {
    fn from_ref(s: &FilesState) -> Self {
        s.sessions.clone()
    }
}

/// Routes for clients, with full paths.
pub fn client_router() -> Router<FilesState> {
    Router::new()
        .route("/api/v1/client/files", get(list).delete(remove))
        .route("/api/v1/client/files/stat", get(stat))
        .route("/api/v1/client/files/content", get(read).put(write))
        .route("/api/v1/client/files/folders", post(mkdir))
        .route("/api/v1/client/files/move", post(rename))
        .route("/api/v1/client/files/copy", post(copy))
        .route(
            "/api/v1/client/files/trash",
            get(trash_list).delete(trash_empty),
        )
        .route("/api/v1/client/files/trash/{id}/restore", post(restore))
        .route("/api/v1/client/files/permissions", post(chmod))
        .route(
            "/api/v1/client/files/permissions/fix",
            post(fix_permissions),
        )
        .route("/api/v1/client/files/archive", post(archive))
        .route("/api/v1/client/files/extract", post(extract))
        .route("/api/v1/client/files/operations/{id}", get(operation))
        .route("/api/v1/client/files/usage", get(usage).post(measure_now))
        .merge(transfers::client_router())
        .merge(ftp::client_router())
}

/// Routes without a session: the node, with a transfer's token, and a download link.
pub fn public_router() -> Router<FilesState> {
    transfers::public_router()
}

/// Routes for administrators (none yet).
pub fn admin_router() -> Router<FilesState> {
    Router::new()
}

fn internal(err: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>) -> AppError {
    AppError::internal(err)
}

pub(crate) struct Hosting {
    pub(crate) tenant: tenants::Tenant,
    pub(crate) plan: plans::Plan,
}

pub(crate) async fn client_hosting(
    state: &FilesState,
    who: &Principal,
) -> Result<Hosting, AppError> {
    client_hosting_for(state, who, "files").await
}

/// The signed-in client's hosting account, when its plan includes `feature`.
pub(crate) async fn client_hosting_for(
    state: &FilesState,
    who: &Principal,
    feature: &'static str,
) -> Result<Hosting, AppError> {
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
    if !plans::includes_feature(&state.db, plan.id, feature)
        .await
        .map_err(internal)?
    {
        return Err(AppError::forbidden(
            "feature_disabled",
            "This is not included in the plan.",
        ));
    }
    if tenant.status.as_str() != "active" {
        return Err(AppError::conflict(
            "tenant_not_running",
            "Only a running account can use the file manager.",
        ));
    }
    Ok(Hosting { tenant, plan })
}

/// The settings the file manager reads.
struct Limits {
    page_size: u32,
    edit_bytes: u32,
    max_archive: u64,
    max_extract: u64,
}

async fn limits(state: &FilesState) -> Result<Limits, AppError> {
    let mut conn = state.db.acquire().await.map_err(internal)?;
    let values = builtpanel_settings::get(&state.settings, &mut conn, "files", Scope::Global)
        .await
        .map_err(internal)?;
    let int = |key: &str, default: u64| {
        values
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(default)
    };
    Ok(Limits {
        page_size: u32::try_from(int("page_size", 200).clamp(10, 1000)).unwrap_or(200),
        edit_bytes: u32::try_from(int("edit_size_kb", 256).clamp(1, 256) * 1024)
            .unwrap_or(MAX_EDIT_BYTES)
            .min(MAX_EDIT_BYTES),
        max_archive: int("max_archive_mb", 2048).saturating_mul(1024 * 1024),
        max_extract: int("max_extract_mb", 4096).saturating_mul(1024 * 1024),
    })
}

// -------------------------------------------------------------------- views

/// What a path is.
#[derive(Debug, Serialize, ToSchema)]
pub struct EntryView {
    /// Its name.
    #[schema(example = "index.php")]
    pub name: String,
    /// `file`, `dir`, `link` (never followed) or `other`.
    #[schema(example = "file")]
    pub kind: String,
    /// Size in bytes.
    pub size: u64,
    /// Last change of content (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub modified_at: String,
    /// Permission bits, as the octal number written out (`0o644` is 420).
    pub mode: u32,
}

/// A page of a folder.
#[derive(Debug, Serialize, ToSchema)]
pub struct DirectoryPage {
    /// Folders first, then by name.
    pub items: Vec<EntryView>,
    /// How many entries the folder has.
    pub total: u32,
    /// Entries skipped.
    pub offset: u32,
    /// Entries asked for.
    pub limit: u32,
}

/// A text file.
#[derive(Debug, Serialize, ToSchema)]
pub struct FileContent {
    /// The text; none when the file is not UTF-8 text.
    pub content: Option<String>,
    /// Whether the file is not text.
    pub binary: bool,
    /// Size in bytes.
    pub size: u64,
    /// Modification time (Unix seconds): give it back as `expected_mtime` when saving,
    /// to be told if someone changed the file meanwhile.
    pub mtime: i64,
}

/// Something in the trash.
#[derive(Debug, Serialize, ToSchema)]
pub struct TrashEntry {
    /// Its id.
    pub id: String,
    /// Where it was.
    pub path: String,
    /// `file`, `dir`, `link` or `other`.
    pub kind: String,
    /// Size in bytes.
    pub size: u64,
    /// When it was deleted (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub deleted_at: String,
}

/// The trash.
#[derive(Debug, Serialize, ToSchema)]
pub struct TrashList {
    /// Newest first.
    pub items: Vec<TrashEntry>,
}

/// What a change did.
#[derive(Debug, Serialize, ToSchema)]
pub struct Changed {
    /// Files and folders touched.
    pub count: u32,
}

/// Work that is still going on.
#[derive(Debug, Serialize, ToSchema)]
pub struct Pending {
    /// Ask `GET /api/v1/client/files/operations/{id}` for the answer.
    pub operation_id: Uuid,
}

fn kind_name(kind: builtpanel_node_protocol::files::EntryKind) -> &'static str {
    use builtpanel_node_protocol::files::EntryKind as K;
    match kind {
        K::File => "file",
        K::Dir => "dir",
        K::Link => "link",
        K::Other => "other",
    }
}

fn entry_view(e: builtpanel_node_protocol::files::FileEntry) -> EntryView {
    EntryView {
        kind: kind_name(e.kind).to_owned(),
        modified_at: builtpanel_session::rfc3339(e.mtime),
        name: e.name,
        size: e.size,
        mode: e.mode,
    }
}

/// A refused request, as an error.
fn file_error(code: FileErrorCode, message: String) -> AppError {
    let field = |c: &'static str| {
        AppError::validation(vec![builtpanel_http_kit::FieldError::new(
            "path",
            c,
            message.clone(),
        )])
    };
    match code {
        FileErrorCode::InvalidPath => field("invalid_path"),
        FileErrorCode::Link => field("link_in_path"),
        FileErrorCode::InvalidMode => field("invalid_mode"),
        FileErrorCode::InvalidArguments => field("invalid_arguments"),
        FileErrorCode::TooMany => field("too_many"),
        FileErrorCode::BadArchive => field("bad_archive"),
        FileErrorCode::NotFound => AppError::not_found("file"),
        FileErrorCode::Exists => AppError::conflict("exists", message),
        FileErrorCode::NotDir => AppError::conflict("not_a_directory", message),
        FileErrorCode::IsDir => AppError::conflict("is_a_directory", message),
        FileErrorCode::Conflict => AppError::conflict("changed_meanwhile", message),
        FileErrorCode::Denied => AppError::forbidden("denied", message),
        FileErrorCode::TooLarge => AppError::payload_too_large("too_large", message),
        FileErrorCode::Failed => internal(std::io::Error::other(message)),
    }
}

/// A node's answer, as an HTTP answer.
fn respond(result: FileResult) -> Result<Response, AppError> {
    Ok(match result {
        FileResult::Entries { entries, total } => Json(serde_json::json!({
            "items": entries.into_iter().map(entry_view).collect::<Vec<_>>(),
            "total": total,
        }))
        .into_response(),
        FileResult::Entry { entry } => Json(entry_view(entry)).into_response(),
        FileResult::Content {
            content,
            binary,
            size,
            mtime,
        } => Json(FileContent {
            content,
            binary,
            size,
            mtime,
        })
        .into_response(),
        FileResult::Done { count } => Json(Changed { count }).into_response(),
        FileResult::Usage {
            total_bytes,
            inodes,
            folders,
            partial,
        } => Json(serde_json::json!({
            "total_bytes": total_bytes,
            "inodes": inodes,
            "partial": partial,
            "folders": folders,
        }))
        .into_response(),
        FileResult::Trash { items } => Json(TrashList {
            items: items
                .into_iter()
                .map(|i| TrashEntry {
                    id: i.id,
                    path: i.path,
                    kind: kind_name(i.kind).to_owned(),
                    size: i.size,
                    deleted_at: builtpanel_session::rfc3339(i.deleted_at),
                })
                .collect(),
        })
        .into_response(),
        FileResult::Error { code, message } => return Err(file_error(code, message)),
    })
}

// ------------------------------------------------------------ running things

pub(crate) fn audit(
    action: &'static str,
    who: &Principal,
    hosting: &Hosting,
    metadata: serde_json::Value,
) -> AuditEntry {
    AuditEntry::new(action, Outcome::Success)
        .actor(who.account_id())
        .tenant(hosting.tenant.id)
        .metadata(metadata)
}

/// Queues the request and waits for the node. `mutating` requests are not sent
/// twice by themselves (a repeated move would be a different move).
async fn run(
    state: &FilesState,
    hosting: &Hosting,
    who: &Principal,
    request: FileRequest,
    mutating: bool,
    audited: Option<(&'static str, serde_json::Value)>,
) -> Result<Response, AppError> {
    let op = Operation::FileOp {
        tenant_id: hosting.tenant.id,
        uid: u32::try_from(hosting.tenant.uid).map_err(internal)?,
        request,
    };
    let payload = serde_json::to_value(&op).map_err(internal)?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    let queued = operations::enqueue_in(
        &mut tx,
        &NewOperation {
            node_id: hosting.tenant.node_id,
            kind: op.type_name(),
            tenant_id: Some(hosting.tenant.id),
            payload: &payload,
            idempotency_key: &format!("file:{}", Uuid::new_v4().simple()),
            created_by: Some(who.account_id()),
            ttl_secs: OPERATION_TTL_SECS,
            max_attempts: if mutating { 1 } else { 3 },
        },
    )
    .await
    .map_err(|e| match e {
        EnqueueError::UnknownNode => AppError::conflict(
            "node_unavailable",
            "The node of this account no longer exists.",
        ),
        other => internal(other),
    })?;
    let id = queued.operation().id;
    if let Some((action, metadata)) = audited {
        builtpanel_audit::insert(&mut *tx, &audit(action, who, hosting, metadata))
            .await
            .map_err(internal)?;
    }
    tx.commit().await.map_err(internal)?;
    wait(state, hosting.tenant.id, id, state.wait).await
}

/// Waits for an operation; `202` with its id when it is not done in time.
async fn wait(
    state: &FilesState,
    tenant_id: Uuid,
    id: Uuid,
    limit: Duration,
) -> Result<Response, AppError> {
    let started = std::time::Instant::now();
    loop {
        let row = operations::get(&state.db, id)
            .await
            .map_err(internal)?
            .filter(|r| r.tenant_id == Some(tenant_id) && r.kind == "file_op")
            .ok_or_else(|| AppError::not_found("operation"))?;
        match row.status {
            Status::Succeeded => {
                let result = row
                    .output
                    .and_then(|o| o.get("result").cloned())
                    .and_then(|r| serde_json::from_value::<FileResult>(r).ok())
                    .ok_or_else(|| internal(std::io::Error::other("an unreadable answer")))?;
                return respond(result);
            }
            Status::Failed => {
                return Err(AppError::unavailable(
                    "node_unavailable",
                    "The node could not do this; try again in a moment.",
                ));
            }
            Status::Pending | Status::Dispatched | Status::Retryable => {}
        }
        if started.elapsed() >= limit {
            return Ok((StatusCode::ACCEPTED, Json(Pending { operation_id: id })).into_response());
        }
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
}

// ----------------------------------------------------------------- handlers

/// Which path.
#[derive(Debug, Deserialize, IntoParams)]
pub struct PathQuery {
    /// Relative to the account's files (`""`: the top).
    #[serde(default)]
    pub path: String,
}

/// Which folder and which page.
#[derive(Debug, Deserialize, IntoParams)]
pub struct ListQuery {
    /// Relative to the account's files (`""`: the top).
    #[serde(default)]
    pub path: String,
    /// Entries to skip.
    pub offset: Option<u32>,
    /// Entries to give (at most the panel's page size setting).
    pub limit: Option<u32>,
}

/// A page of a folder.
#[utoipa::path(
    get,
    path = "/api/v1/client/files",
    tag = "files",
    operation_id = "clientListFiles",
    security(("session" = [])),
    params(ListQuery),
    responses(
        (status = 200, description = "Folders first, then by name", body = DirectoryPage),
        (status = 202, description = "Still going", body = Pending),
        (status = 404, description = "`not_found`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "`invalid_path` or `link_in_path`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn list(
    State(state): State<FilesState>,
    who: Principal,
    Query(q): Query<ListQuery>,
) -> Result<Response, AppError> {
    let h = client_hosting(&state, &who).await?;
    let l = limits(&state).await?;
    let limit = q.limit.unwrap_or(l.page_size).clamp(1, l.page_size);
    let offset = q.offset.unwrap_or(0);
    let request = FileRequest::List {
        path: q.path,
        offset,
        limit,
    };
    let response = run(&state, &h, &who, request, false, None).await?;
    Ok(add_paging(response, offset, limit).await)
}

/// Puts the paging into a listing answer.
async fn add_paging(response: Response, offset: u32, limit: u32) -> Response {
    if response.status() != StatusCode::OK {
        return response;
    }
    let (parts, body) = response.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, 8 * 1024 * 1024).await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    value["offset"] = offset.into();
    value["limit"] = limit.into();
    (parts.status, Json(value)).into_response()
}

/// One path.
#[utoipa::path(
    get,
    path = "/api/v1/client/files/stat",
    tag = "files",
    operation_id = "clientStatFile",
    security(("session" = [])),
    params(PathQuery),
    responses(
        (status = 200, description = "What it is", body = EntryView),
        (status = 404, description = "`not_found`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn stat(
    State(state): State<FilesState>,
    who: Principal,
    Query(q): Query<PathQuery>,
) -> Result<Response, AppError> {
    let h = client_hosting(&state, &who).await?;
    run(
        &state,
        &h,
        &who,
        FileRequest::Stat { path: q.path },
        false,
        None,
    )
    .await
}

/// A text file, for the editor.
#[utoipa::path(
    get,
    path = "/api/v1/client/files/content",
    tag = "files",
    operation_id = "clientReadFile",
    security(("session" = [])),
    params(PathQuery),
    responses(
        (status = 200, description = "The text", body = FileContent),
        (status = 413, description = "`too_large`: bigger than the editor's limit", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "`not_found`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn read(
    State(state): State<FilesState>,
    who: Principal,
    Query(q): Query<PathQuery>,
) -> Result<Response, AppError> {
    let h = client_hosting(&state, &who).await?;
    let l = limits(&state).await?;
    let request = FileRequest::Read {
        path: q.path,
        max_bytes: l.edit_bytes,
    };
    run(&state, &h, &who, request, false, None).await
}

/// A file to save.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveFile {
    /// The file.
    #[schema(example = "www/index.php")]
    pub path: String,
    /// What to write (the whole file).
    pub content: String,
    /// The `mtime` the editor opened it with; refused (409) when it changed since.
    pub expected_mtime: Option<i64>,
    /// Refuse (409) when the file already exists.
    #[serde(default)]
    pub create_only: bool,
}

/// Save a text file (whole, atomically).
#[utoipa::path(
    put,
    path = "/api/v1/client/files/content",
    tag = "files",
    operation_id = "clientWriteFile",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = SaveFile,
    responses(
        (status = 200, description = "Saved", body = Changed),
        (status = 409, description = "`exists` or `changed_meanwhile`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 413, description = "`too_large`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn write(
    State(state): State<FilesState>,
    who: Principal,
    Json(req): Json<SaveFile>,
) -> Result<Response, AppError> {
    let h = client_hosting(&state, &who).await?;
    let l = limits(&state).await?;
    let meta = serde_json::json!({ "path": req.path, "bytes": req.content.len() });
    let request = FileRequest::Write {
        path: req.path,
        content: req.content,
        max_bytes: l.edit_bytes,
        expected_mtime: req.expected_mtime,
        create_only: req.create_only,
    };
    run(
        &state,
        &h,
        &who,
        request,
        true,
        Some(("files.written", meta)),
    )
    .await
}

/// One path in a body.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct OnePath {
    /// The path.
    pub path: String,
}

/// Make a folder.
#[utoipa::path(
    post,
    path = "/api/v1/client/files/folders",
    tag = "files",
    operation_id = "clientMakeFolder",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = OnePath,
    responses(
        (status = 200, description = "Made", body = Changed),
        (status = 409, description = "`exists`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn mkdir(
    State(state): State<FilesState>,
    who: Principal,
    Json(req): Json<OnePath>,
) -> Result<Response, AppError> {
    let h = client_hosting(&state, &who).await?;
    let meta = serde_json::json!({ "path": req.path });
    run(
        &state,
        &h,
        &who,
        FileRequest::Mkdir { path: req.path },
        true,
        Some(("files.folder_made", meta)),
    )
    .await
}

/// From and to.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct FromTo {
    /// What to move or copy.
    pub from: String,
    /// The new path (not the folder it goes into).
    pub to: String,
}

/// Move or rename (never over something).
#[utoipa::path(
    post,
    path = "/api/v1/client/files/move",
    tag = "files",
    operation_id = "clientMoveFile",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = FromTo,
    responses(
        (status = 200, description = "Moved", body = Changed),
        (status = 409, description = "`exists`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn rename(
    State(state): State<FilesState>,
    who: Principal,
    Json(req): Json<FromTo>,
) -> Result<Response, AppError> {
    let h = client_hosting(&state, &who).await?;
    let meta = serde_json::json!({ "from": req.from, "to": req.to });
    let request = FileRequest::Move {
        from: req.from,
        to: req.to,
    };
    run(&state, &h, &who, request, true, Some(("files.moved", meta))).await
}

/// Copy a file or a whole folder (never over something; links are left out).
#[utoipa::path(
    post,
    path = "/api/v1/client/files/copy",
    tag = "files",
    operation_id = "clientCopyFile",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = FromTo,
    responses(
        (status = 200, description = "Copied", body = Changed),
        (status = 202, description = "Still going", body = Pending),
        (status = 409, description = "`exists`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn copy(
    State(state): State<FilesState>,
    who: Principal,
    Json(req): Json<FromTo>,
) -> Result<Response, AppError> {
    let h = client_hosting(&state, &who).await?;
    let meta = serde_json::json!({ "from": req.from, "to": req.to });
    let request = FileRequest::Copy {
        from: req.from,
        to: req.to,
    };
    run(
        &state,
        &h,
        &who,
        request,
        true,
        Some(("files.copied", meta)),
    )
    .await
}

/// Delete: the file goes to the trash.
#[utoipa::path(
    delete,
    path = "/api/v1/client/files",
    tag = "files",
    operation_id = "clientDeleteFile",
    security(("session" = [])),
    params(PathQuery, ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses(
        (status = 200, description = "In the trash", body = Changed),
        (status = 404, description = "`not_found`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn remove(
    State(state): State<FilesState>,
    who: Principal,
    Query(q): Query<PathQuery>,
) -> Result<Response, AppError> {
    let h = client_hosting(&state, &who).await?;
    let meta = serde_json::json!({ "path": q.path });
    run(
        &state,
        &h,
        &who,
        FileRequest::Delete { path: q.path },
        true,
        Some(("files.deleted", meta)),
    )
    .await
}

/// What is in the trash.
#[utoipa::path(
    get,
    path = "/api/v1/client/files/trash",
    tag = "files",
    operation_id = "clientListTrash",
    security(("session" = [])),
    responses((status = 200, description = "Newest first", body = TrashList))
)]
pub async fn trash_list(
    State(state): State<FilesState>,
    who: Principal,
) -> Result<Response, AppError> {
    let h = client_hosting(&state, &who).await?;
    run(&state, &h, &who, FileRequest::TrashList, false, None).await
}

/// Which trash items.
#[derive(Debug, Deserialize, IntoParams)]
pub struct EmptyQuery {
    /// Only this item.
    pub id: Option<String>,
    /// Only items deleted more than this many days ago.
    pub older_than_days: Option<u32>,
}

/// Empty the trash (all of it, one item, or what is old).
#[utoipa::path(
    delete,
    path = "/api/v1/client/files/trash",
    tag = "files",
    operation_id = "clientEmptyTrash",
    security(("session" = [])),
    params(EmptyQuery, ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses((status = 200, description = "How many items went", body = Changed))
)]
pub async fn trash_empty(
    State(state): State<FilesState>,
    who: Principal,
    Query(q): Query<EmptyQuery>,
) -> Result<Response, AppError> {
    let h = client_hosting(&state, &who).await?;
    let meta = serde_json::json!({ "id": q.id, "older_than_days": q.older_than_days });
    let request = FileRequest::EmptyTrash {
        id: q.id,
        older_than_days: q.older_than_days,
    };
    run(
        &state,
        &h,
        &who,
        request,
        true,
        Some(("files.trash_emptied", meta)),
    )
    .await
}

/// Put something back where it was.
#[utoipa::path(
    post,
    path = "/api/v1/client/files/trash/{id}/restore",
    tag = "files",
    operation_id = "clientRestoreFile",
    security(("session" = [])),
    params(
        ("id" = String, Path, description = "The trash id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 200, description = "Back", body = Changed),
        (status = 409, description = "`exists`: something is there now", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn restore(
    State(state): State<FilesState>,
    who: Principal,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let h = client_hosting(&state, &who).await?;
    let meta = serde_json::json!({ "id": id });
    run(
        &state,
        &h,
        &who,
        FileRequest::Restore { id },
        true,
        Some(("files.restored", meta)),
    )
    .await
}

/// New permission bits.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetMode {
    /// The path.
    pub path: String,
    /// The bits, as the number (`0o755` is 493); at most 0o777: no setuid, setgid
    /// or sticky bit, and the owner keeps read and write.
    pub mode: u32,
    /// For a folder: everything below it too.
    #[serde(default)]
    pub recursive: bool,
}

/// Change permissions.
#[utoipa::path(
    post,
    path = "/api/v1/client/files/permissions",
    tag = "files",
    operation_id = "clientChmod",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = SetMode,
    responses(
        (status = 200, description = "Changed", body = Changed),
        (status = 422, description = "`invalid_mode`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn chmod(
    State(state): State<FilesState>,
    who: Principal,
    Json(req): Json<SetMode>,
) -> Result<Response, AppError> {
    let h = client_hosting(&state, &who).await?;
    let meta =
        serde_json::json!({ "path": req.path, "mode": req.mode, "recursive": req.recursive });
    let request = FileRequest::Chmod {
        path: req.path,
        mode: req.mode,
        recursive: req.recursive,
    };
    run(
        &state,
        &h,
        &who,
        request,
        true,
        Some(("files.permissions_changed", meta)),
    )
    .await
}

/// Set sane permissions on a whole tree (folders 755, files 644).
#[utoipa::path(
    post,
    path = "/api/v1/client/files/permissions/fix",
    tag = "files",
    operation_id = "clientFixPermissions",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = OnePath,
    responses(
        (status = 200, description = "Fixed", body = Changed),
        (status = 202, description = "Still going", body = Pending)
    )
)]
pub async fn fix_permissions(
    State(state): State<FilesState>,
    who: Principal,
    Json(req): Json<OnePath>,
) -> Result<Response, AppError> {
    let h = client_hosting(&state, &who).await?;
    let meta = serde_json::json!({ "path": req.path });
    run(
        &state,
        &h,
        &who,
        FileRequest::FixPermissions { path: req.path },
        true,
        Some(("files.permissions_fixed", meta)),
    )
    .await
}

/// What to pack.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PackRequest {
    /// Files and folders (1 to 1000).
    pub paths: Vec<String>,
    /// The archive to make (never replaced).
    #[schema(example = "backup.zip")]
    pub dest: String,
    /// `zip` or `tar_gz`.
    #[schema(example = "zip")]
    pub format: String,
}

/// Pack files and folders into a zip or tar.gz.
#[utoipa::path(
    post,
    path = "/api/v1/client/files/archive",
    tag = "files",
    operation_id = "clientArchive",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = PackRequest,
    responses(
        (status = 200, description = "Packed", body = Changed),
        (status = 202, description = "Still going", body = Pending),
        (status = 413, description = "`too_large`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn archive(
    State(state): State<FilesState>,
    who: Principal,
    Json(req): Json<PackRequest>,
) -> Result<Response, AppError> {
    let h = client_hosting(&state, &who).await?;
    let l = limits(&state).await?;
    let format = match req.format.as_str() {
        "zip" => ArchiveFormat::Zip,
        "tar_gz" => ArchiveFormat::TarGz,
        _ => {
            return Err(AppError::validation(vec![
                builtpanel_http_kit::FieldError::new(
                    "format",
                    "invalid_format",
                    "The format is `zip` or `tar_gz`.",
                ),
            ]));
        }
    };
    let meta = serde_json::json!({ "paths": req.paths, "dest": req.dest });
    let request = FileRequest::Archive {
        paths: req.paths,
        dest: req.dest,
        format,
        max_bytes: l.max_archive,
    };
    run(
        &state,
        &h,
        &who,
        request,
        true,
        Some(("files.archived", meta)),
    )
    .await
}

/// What to unpack.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UnpackRequest {
    /// The archive (`.zip`, `.tar.gz`, `.tgz`).
    pub path: String,
    /// The folder to unpack into (made when missing).
    pub dest: String,
}

/// Unpack an archive; the whole archive is checked first.
#[utoipa::path(
    post,
    path = "/api/v1/client/files/extract",
    tag = "files",
    operation_id = "clientExtract",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = UnpackRequest,
    responses(
        (status = 200, description = "Unpacked", body = Changed),
        (status = 202, description = "Still going", body = Pending),
        (status = 422, description = "`bad_archive`, `too_many`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 413, description = "`too_large`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn extract(
    State(state): State<FilesState>,
    who: Principal,
    Json(req): Json<UnpackRequest>,
) -> Result<Response, AppError> {
    let h = client_hosting(&state, &who).await?;
    let l = limits(&state).await?;
    let meta = serde_json::json!({ "path": req.path, "dest": req.dest });
    let request = FileRequest::Extract {
        path: req.path,
        dest: req.dest,
        max_bytes: l.max_extract,
        max_entries: 100_000,
    };
    run(
        &state,
        &h,
        &who,
        request,
        true,
        Some(("files.extracted", meta)),
    )
    .await
}

/// The answer to a request that was answered `202`.
#[utoipa::path(
    get,
    path = "/api/v1/client/files/operations/{id}",
    tag = "files",
    operation_id = "clientFileOperation",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "The operation id")),
    responses(
        (status = 200, description = "Done: the answer the request would have given", body = Changed),
        (status = 202, description = "Still going", body = Pending),
        (status = 404, description = "No such operation (or not yours)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn operation(
    State(state): State<FilesState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<Response, AppError> {
    let h = client_hosting(&state, &who).await?;
    wait(&state, h.tenant.id, id, Duration::from_millis(1)).await
}

// ------------------------------------------------------------- disk usage

/// What one folder uses.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct FolderUse {
    /// The folder.
    #[schema(example = "www/uploads")]
    pub path: String,
    /// Disk used below it, in bytes.
    pub bytes: u64,
    /// Files and folders below it.
    pub inodes: u64,
}

/// What the account's files use.
#[derive(Debug, Serialize, ToSchema)]
pub struct DiskUsageView {
    /// When it was measured (RFC 3339, UTC); the node measures every few hours.
    #[schema(format = DateTime)]
    pub measured_at: Option<String>,
    /// Disk used, in bytes.
    pub total_bytes: i64,
    /// Files, folders and links.
    pub inodes: i64,
    /// Whether the measurement stopped at its limit (the figures are then a lower bound).
    pub partial: bool,
    /// The biggest folders, biggest first.
    pub folders: Vec<FolderUse>,
}

/// The last measurement of what the account's files use.
#[utoipa::path(
    get,
    path = "/api/v1/client/files/usage",
    tag = "files",
    operation_id = "clientDiskUsage",
    security(("session" = [])),
    responses((status = 200, description = "Zeros and no `measured_at` until the first measurement", body = DiskUsageView))
)]
pub async fn usage(
    State(state): State<FilesState>,
    who: Principal,
) -> Result<Json<DiskUsageView>, AppError> {
    let h = client_hosting(&state, &who).await?;
    let found = builtpanel_database::file_transfers::usage_of(&state.db, h.tenant.id)
        .await
        .map_err(internal)?;
    Ok(Json(match found {
        Some(u) => DiskUsageView {
            measured_at: Some(builtpanel_session::rfc3339(u.measured_at)),
            total_bytes: u.total_bytes,
            inodes: u.inodes,
            partial: u.partial,
            folders: serde_json::from_value(u.folders).unwrap_or_default(),
        },
        None => DiskUsageView {
            measured_at: None,
            total_bytes: 0,
            inodes: 0,
            partial: false,
            folders: Vec::new(),
        },
    }))
}

/// Measure now (at most once in five minutes); the answer arrives with the next
/// `GET` of the usage.
#[utoipa::path(
    post,
    path = "/api/v1/client/files/usage",
    tag = "files",
    operation_id = "clientMeasureDiskUsage",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses((status = 202, description = "Queued", body = Pending))
)]
pub async fn measure_now(
    State(state): State<FilesState>,
    who: Principal,
) -> Result<Response, AppError> {
    let h = client_hosting(&state, &who).await?;
    let slot = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 300);
    let id = queue_usage(
        &state.db,
        h.tenant.id,
        h.tenant.node_id,
        h.tenant.uid,
        &format!("du-now:{}:{slot}", h.tenant.id.simple()),
        Some(who.account_id()),
    )
    .await?;
    Ok((StatusCode::ACCEPTED, Json(Pending { operation_id: id })).into_response())
}

async fn queue_usage(
    db: &PgPool,
    tenant_id: Uuid,
    node_id: Uuid,
    uid: i32,
    key: &str,
    by: Option<Uuid>,
) -> Result<Uuid, AppError> {
    let op = Operation::FileOp {
        tenant_id,
        uid: u32::try_from(uid).map_err(internal)?,
        request: FileRequest::DiskUsage { top: 20 },
    };
    let payload = serde_json::to_value(&op).map_err(internal)?;
    let mut conn = db.acquire().await.map_err(internal)?;
    let queued = operations::enqueue_in(
        &mut conn,
        &NewOperation {
            node_id,
            kind: op.type_name(),
            tenant_id: Some(tenant_id),
            payload: &payload,
            idempotency_key: key,
            created_by: by,
            ttl_secs: 6 * 3600,
            max_attempts: 2,
        },
    )
    .await
    .map_err(internal)?;
    Ok(queued.operation().id)
}

/// Asks the node of every active account to measure its files (once per six hours
/// each). Run now and then; returns how many were queued.
///
/// # Errors
/// Database errors.
pub async fn measure_usage(db: &PgPool) -> Result<usize, AppError> {
    let rows: Vec<(Uuid, Uuid, i32)> = builtpanel_database::sqlx::query_as(
        "SELECT id, node_id, uid FROM tenants WHERE status = 'active'",
    )
    .fetch_all(db)
    .await
    .map_err(internal)?;
    let slot = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / (6 * 3600));
    let mut queued = 0;
    for (tenant_id, node_id, uid) in rows {
        let key = format!("du:{}:{slot}", tenant_id.simple());
        let before: Option<Uuid> = builtpanel_database::sqlx::query_scalar(
            "SELECT id FROM operations WHERE node_id = $1 AND idempotency_key = $2",
        )
        .bind(node_id)
        .bind(&key)
        .fetch_optional(db)
        .await
        .map_err(internal)?;
        if before.is_none()
            && queue_usage(db, tenant_id, node_id, uid, &key, None)
                .await
                .is_ok()
        {
            queued += 1;
        }
    }
    Ok(queued)
}

// ------------------------------------------------------------ trash purging

/// Asks every active account's node to empty what has been in the trash longer
/// than the setting `trash_purge_days` (a day's worth of work is queued once per
/// account per day). Run now and then; returns how many were queued.
///
/// # Errors
/// Database errors.
pub async fn purge_trash(db: &PgPool, settings: &SettingsRegistry) -> Result<usize, AppError> {
    let mut conn = db.acquire().await.map_err(internal)?;
    let values = builtpanel_settings::get(settings, &mut conn, "files", Scope::Global)
        .await
        .map_err(internal)?;
    drop(conn);
    let days = values
        .get("trash_purge_days")
        .and_then(serde_json::Value::as_u64)
        .and_then(|d| u32::try_from(d).ok())
        .unwrap_or(30);
    if days == 0 {
        return Ok(0);
    }
    let rows: Vec<(Uuid, Uuid, i32)> = builtpanel_database::sqlx::query_as(
        "SELECT id, node_id, uid FROM tenants WHERE status = 'active'",
    )
    .fetch_all(db)
    .await
    .map_err(internal)?;
    let day = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 86_400);
    let mut queued = 0;
    for (tenant_id, node_id, uid) in rows {
        let Ok(uid) = u32::try_from(uid) else {
            continue;
        };
        let op = Operation::FileOp {
            tenant_id,
            uid,
            request: FileRequest::EmptyTrash {
                id: None,
                older_than_days: Some(days),
            },
        };
        let payload = serde_json::to_value(&op).map_err(internal)?;
        let mut conn = db.acquire().await.map_err(internal)?;
        match operations::enqueue_in(
            &mut conn,
            &NewOperation {
                node_id,
                kind: op.type_name(),
                tenant_id: Some(tenant_id),
                payload: &payload,
                idempotency_key: &format!("trashpurge:{}:{day}", tenant_id.simple()),
                created_by: None,
                ttl_secs: 20 * 3600,
                max_attempts: 3,
            },
        )
        .await
        {
            Ok(operations::Enqueued::Created(_)) => queued += 1,
            Ok(operations::Enqueued::Existing(_)) | Err(_) => {}
        }
    }
    Ok(queued)
}

/// The module's OpenAPI document.
#[derive(OpenApi)]
#[openapi(
    paths(
        list, stat, read, write, mkdir, rename, copy, remove, trash_list, trash_empty, restore,
        chmod, fix_permissions, archive, extract, operation, usage, measure_now, ftp::list_accounts,
        ftp::create_account, ftp::change_account, ftp::set_password, ftp::delete_account, transfers::start_upload,
        transfers::put_chunk, transfers::complete_upload, transfers::start_download,
        transfers::get_transfer, transfers::cancel_transfer, transfers::make_link
    ),
    components(schemas(
        EntryView, DirectoryPage, FileContent, TrashEntry, TrashList, Changed, Pending, SaveFile,
        OnePath, FromTo, SetMode, PackRequest, UnpackRequest, FolderUse, DiskUsageView, ftp::FtpAccountView, ftp::FtpAccountList, ftp::AddFtpAccount,
        ftp::ChangeFtpAccount, ftp::NewFtpPassword, ftp::FtpAccountCreated, transfers::StartUpload,
        transfers::TransferView, transfers::FileDownloadLink
    )),
    tags((name = "files", description = "The file manager"))
)]
pub struct FilesApi;
