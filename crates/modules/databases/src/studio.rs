//! The database studio for clients: browse tables, edit rows and run SQL in the browser.
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | POST | `/api/v1/client/databases/{id}/studio` | the owner; the `databases` feature |
//!
//! One typed request in, one typed answer out. The node does the work with a temporary
//! account that has rights on that one database only; the control plane never sees a
//! database password and never keeps what a request returned beyond the operation ledger's
//! own row. The audit log records what kind of request it was, never its SQL or values.

use std::time::Duration;

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::post,
};
use builtpanel_database::operations::{self, EnqueueError, NewOperation, Status};
use builtpanel_http_kit::{AppError, ProblemDetails};
use builtpanel_node_protocol::{
    DbEngine, Operation,
    studio::{StudioRequest, StudioResult},
};
use builtpanel_rbac::Principal;
use uuid::Uuid;

use crate::{DatabasesState, client_hosting, entry, internal, require_feature, running};

/// How long the request waits for the node.
const WAIT: Duration = Duration::from_secs(40);
/// Most requests an account may make in a minute.
const MAX_PER_MINUTE: i64 = 240;

/// Routes for clients, with full paths.
pub fn client_router() -> Router<DatabasesState> {
    Router::new().route("/api/v1/client/databases/{id}/studio", post(studio))
}

fn kind(request: &StudioRequest) -> (&'static str, bool) {
    match request {
        StudioRequest::Tables => ("tables", false),
        StudioRequest::Describe { .. } => ("describe", false),
        StudioRequest::Rows { .. } => ("rows", false),
        StudioRequest::Query { .. } => ("query", true),
        StudioRequest::InsertRow { .. } => ("insert", true),
        StudioRequest::UpdateRow { .. } => ("update", true),
        StudioRequest::DeleteRow { .. } => ("delete", true),
    }
}

/// Do one thing in a database of the account: look at it, change a row or run SQL.
#[utoipa::path(
    post,
    path = "/api/v1/client/databases/{id}/studio",
    tag = "databases",
    operation_id = "clientDatabaseStudio",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Database id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = StudioRequest,
    responses(
        (status = 200, description = "The answer (a request the server refused is a `type: error` answer)", body = StudioResult),
        (status = 403, description = "`client_only`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such database", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`web_client_unavailable`, `database_not_ready` or `tenant_not_running`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 429, description = "Too many requests in a short time", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 503, description = "`node_unavailable` or `node_busy`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn studio(
    State(state): State<DatabasesState>,
    who: Principal,
    Path(database_id): Path<Uuid>,
    Json(request): Json<StudioRequest>,
) -> Result<Json<StudioResult>, AppError> {
    let h = client_hosting(&state, &who).await?;
    require_feature(&state, &h, "databases", "Databases are").await?;
    running(&h)?;
    let db = builtpanel_database::databases::get(&state.db, h.tenant.id, database_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("database"))?;
    let engine = if db.engine == "postgres" {
        DbEngine::Postgres
    } else {
        DbEngine::Mysql
    };
    if super::server_for(&state, &h, engine).await?.is_some() {
        return Err(AppError::conflict(
            "web_client_unavailable",
            "The web client is not available for a database on a server of your own; connect with your own client.",
        ));
    }
    if db.status != "ready" {
        return Err(AppError::conflict(
            "database_not_ready",
            "The database is not ready yet.",
        ));
    }
    let recent: i64 = builtpanel_database::sqlx::query_scalar(
        "SELECT count(*) FROM operations
         WHERE tenant_id = $1 AND kind = 'studio_op' AND created_at > now() - interval '1 minute'",
    )
    .bind(h.tenant.id)
    .fetch_one(&state.db)
    .await
    .map_err(internal)?;
    if recent >= MAX_PER_MINUTE {
        return Err(AppError::too_many_requests(
            "too_many_requests",
            "Too many database requests; wait a moment.",
            Duration::from_secs(30),
        ));
    }
    let (what, changes) = kind(&request);
    let op = Operation::StudioOp {
        database: db.name.clone(),
        engine,
        request,
    };
    let payload = serde_json::to_value(&op).map_err(internal)?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    // A statement may have changed something, so it is never sent twice by itself.
    let queued = operations::enqueue_in(
        &mut tx,
        &NewOperation {
            node_id: h.tenant.node_id,
            kind: op.type_name(),
            tenant_id: Some(h.tenant.id),
            payload: &payload,
            idempotency_key: &format!("studio:{}", Uuid::new_v4().simple()),
            created_by: Some(who.account_id()),
            ttl_secs: 120,
            max_attempts: if changes { 1 } else { 3 },
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
    if changes {
        builtpanel_audit::insert(
            &mut *tx,
            &entry("database.studio", &who, &h, "database", db.id)
                .metadata(serde_json::json!({ "database": db.name, "request": what })),
        )
        .await
        .map_err(internal)?;
    }
    tx.commit().await.map_err(internal)?;

    let started = std::time::Instant::now();
    loop {
        let row = operations::get(&state.db, id)
            .await
            .map_err(internal)?
            .ok_or_else(|| internal(std::io::Error::other("an operation vanished")))?;
        match row.status {
            Status::Succeeded => {
                let result = row
                    .output
                    .and_then(|o| o.get("result").cloned())
                    .and_then(|r| serde_json::from_value::<StudioResult>(r).ok())
                    .ok_or_else(|| internal(std::io::Error::other("an unreadable answer")))?;
                return Ok(Json(result));
            }
            Status::Failed => {
                return Err(AppError::unavailable(
                    "node_unavailable",
                    "The node could not do this; try again in a moment.",
                ));
            }
            Status::Pending | Status::Dispatched | Status::Retryable => {}
        }
        if started.elapsed() > WAIT {
            return Err(AppError::unavailable(
                "node_busy",
                "The node did not answer in time; try again in a moment.",
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
