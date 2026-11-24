//! Operations over HTTP: look at the ledger, ping a node, retry a failure.
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET  | `/api/v1/admin/operations` | `admin.nodes.read` |
//! | GET  | `/api/v1/admin/operations/{id}` | `admin.nodes.read` |
//! | POST | `/api/v1/admin/operations/{id}/retry` | `admin.nodes.manage`, sudo mode |
//! | POST | `/api/v1/admin/nodes/{id}/ping` | `admin.nodes.manage` |

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_node_protocol::Operation;
use builtpanel_operations::{OperationsError, Request, Row, Status};
use builtpanel_rbac::{
    Require, RequireRecent,
    permissions::{NodesManage, NodesRead},
};
use builtpanel_session::rfc3339;
use serde::{Deserialize, Serialize};
use sha2::Digest as _;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::NodesState;

/// An operation as shown to administrators.
#[derive(Debug, Serialize, ToSchema)]
pub struct OperationView {
    /// Ledger id.
    pub id: Uuid,
    /// What it does (`ping`, `report_capabilities`, ...).
    #[schema(example = "ping")]
    pub kind: String,
    /// The node that runs it.
    pub node_id: Uuid,
    /// Tenant it is about.
    pub tenant_id: Option<Uuid>,
    /// `pending`, `dispatched`, `succeeded`, `failed` or `retryable`.
    #[schema(example = "succeeded")]
    pub status: String,
    /// Deliveries so far.
    pub attempts: i32,
    /// Most deliveries.
    pub max_attempts: i32,
    /// The key that makes asking again harmless.
    pub idempotency_key: String,
    /// The operation as sent.
    pub payload: serde_json::Value,
    /// What the node produced, on success.
    pub output: Option<serde_json::Value>,
    /// Why it failed (`code`, `message`, `retryable`), also the last failure of
    /// an operation that is being retried.
    pub error: Option<serde_json::Value>,
    /// Account that asked.
    pub created_by: Option<Uuid>,
    /// Creation time (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub created_at: String,
    /// Not sent after this (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub expires_at: String,
    /// Earliest next delivery (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub next_attempt_at: String,
    /// Last delivery (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub dispatched_at: Option<String>,
    /// When it became final (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub finished_at: Option<String>,
}

impl From<Row> for OperationView {
    fn from(r: Row) -> Self {
        Self {
            id: r.id,
            kind: r.kind,
            node_id: r.node_id,
            tenant_id: r.tenant_id,
            status: r.status.as_str().to_owned(),
            attempts: r.attempts,
            max_attempts: r.max_attempts,
            idempotency_key: r.idempotency_key,
            payload: r.payload,
            output: r.output,
            error: r.error,
            created_by: r.created_by,
            created_at: rfc3339(r.created_at),
            expires_at: rfc3339(r.expires_at),
            next_attempt_at: rfc3339(r.next_attempt_at),
            dispatched_at: r.dispatched_at.map(rfc3339),
            finished_at: r.finished_at.map(rfc3339),
        }
    }
}

/// Recent operations.
#[derive(Debug, Serialize, ToSchema)]
pub struct OperationList {
    /// Newest first (at most 100).
    pub items: Vec<OperationView>,
}

/// Filters for the list.
#[derive(Debug, Deserialize, IntoParams)]
pub struct OperationFilter {
    /// Only this node.
    pub node_id: Option<Uuid>,
    /// Only this status.
    pub status: Option<String>,
    /// Only this kind (`ping`, ...).
    pub kind: Option<String>,
}

fn internal(err: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>) -> AppError {
    AppError::internal(err)
}

fn map_error(err: OperationsError) -> AppError {
    match err {
        OperationsError::UnknownNode => AppError::not_found("node"),
        OperationsError::NodeRevoked => AppError::conflict(
            "node_revoked",
            "This node is revoked and takes no new operations.",
        ),
        OperationsError::KeyReused => AppError::conflict(
            "idempotency_key_reused",
            "This idempotency key was already used for a different operation.",
        ),
        OperationsError::InvalidKey => AppError::validation(vec![FieldError::new(
            "Idempotency-Key",
            "invalid_format",
            "Use 1-128 characters: letters, digits and . _ : -",
        )]),
        other => internal(other),
    }
}

/// List recent operations.
#[utoipa::path(
    get,
    path = "/api/v1/admin/operations",
    tag = "operations",
    operation_id = "listOperations",
    security(("session" = [])),
    params(OperationFilter),
    responses(
        (status = 200, description = "Newest first", body = OperationList),
        (status = 403, description = "`permission_denied` (needs admin.nodes.read) or `mfa_setup_required`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Unknown status", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn list_operations(
    State(state): State<NodesState>,
    _who: Require<NodesRead>,
    Query(filter): Query<OperationFilter>,
) -> Result<Json<OperationList>, AppError> {
    let status = match filter.status.as_deref() {
        None | Some("") => None,
        Some(name) => Some(Status::from_name(name).ok_or_else(|| {
            AppError::validation(vec![FieldError::new(
                "status",
                "unknown_status",
                "Use pending, dispatched, succeeded, failed or retryable.",
            )])
        })?),
    };
    let kind = filter.kind.as_deref().filter(|k| !k.is_empty());
    let items = state
        .operations
        .recent(filter.node_id, status, kind, 100)
        .await
        .map_err(internal)?
        .into_iter()
        .map(OperationView::from)
        .collect();
    Ok(Json(OperationList { items }))
}

/// One operation.
#[utoipa::path(
    get,
    path = "/api/v1/admin/operations/{id}",
    tag = "operations",
    operation_id = "getOperation",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Operation id")),
    responses(
        (status = 200, description = "The operation", body = OperationView),
        (status = 403, description = "`permission_denied` (needs admin.nodes.read)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such operation", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn get_operation(
    State(state): State<NodesState>,
    _who: Require<NodesRead>,
    Path(id): Path<String>,
) -> Result<Json<OperationView>, AppError> {
    let id: Uuid = id.parse().map_err(|_| AppError::not_found("operation"))?;
    state
        .operations
        .get(id)
        .await
        .map_err(internal)?
        .map(|row| Json(row.into()))
        .ok_or_else(|| AppError::not_found("operation"))
}

/// Ask a node to answer a ping (a liveness check that goes through the whole
/// pipeline). Each call is a new ping unless an `Idempotency-Key` header says
/// it is the same request.
#[utoipa::path(
    post,
    path = "/api/v1/admin/nodes/{id}/ping",
    tag = "operations",
    operation_id = "pingNode",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Node id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token"),
        ("Idempotency-Key" = Option<String>, Header, description = "Same key, same ping")
    ),
    responses(
        (status = 202, description = "Queued; follow it with GET /api/v1/admin/operations/{id}", body = OperationView),
        (status = 403, description = "`permission_denied` (needs admin.nodes.manage) or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such node", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`node_revoked` or `idempotency_key_reused`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn ping_node(
    State(state): State<NodesState>,
    who: Require<NodesManage>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<OperationView>), AppError> {
    let node_id: Uuid = id.parse().map_err(|_| AppError::not_found("node"))?;
    let key = match headers.get("idempotency-key") {
        Some(value) => value
            .to_str()
            .map_err(|_| {
                AppError::validation(vec![FieldError::new(
                    "Idempotency-Key",
                    "invalid_format",
                    "Use plain text.",
                )])
            })?
            .to_owned(),
        None => format!("ping:{}", Uuid::now_v7()),
    };
    // The nonce follows the key, so that asking again with the same key is the
    // same request (the ledger compares the whole payload).
    let digest = sha2::Sha256::digest(key.as_bytes());
    let nonce = u64::from_be_bytes(digest[..8].try_into().unwrap_or([0; 8]));
    let mut request = Request::new(node_id, Operation::Ping { nonce }, key);
    request.created_by = Some(who.0.account_id());
    let queued = state
        .operations
        .enqueue(&request)
        .await
        .map_err(map_error)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(queued.operation().clone().into()),
    ))
}

/// Try a failed operation again as a new operation. Asking twice returns the
/// same new operation.
#[utoipa::path(
    post,
    path = "/api/v1/admin/operations/{id}/retry",
    tag = "operations",
    operation_id = "retryOperation",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "The failed operation"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 202, description = "The new operation", body = OperationView),
        (status = 403, description = "`permission_denied`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such operation or node", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`operation_not_retryable` (only failed operations) or `node_revoked`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn retry_operation(
    State(state): State<NodesState>,
    RequireRecent(who, _): RequireRecent<NodesManage>,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<OperationView>), AppError> {
    let id: Uuid = id.parse().map_err(|_| AppError::not_found("operation"))?;
    let old = state
        .operations
        .get(id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("operation"))?;
    if old.status != Status::Failed {
        return Err(AppError::conflict(
            "operation_not_retryable",
            "Only operations that failed can be retried.",
        ));
    }
    let op: Operation = serde_json::from_value(old.payload.clone()).map_err(internal)?;
    let mut request = Request::new(old.node_id, op, format!("retry:{id}"));
    request.tenant_id = old.tenant_id;
    request.created_by = Some(who.account_id());
    let queued = state
        .operations
        .enqueue(&request)
        .await
        .map_err(map_error)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(queued.operation().clone().into()),
    ))
}
