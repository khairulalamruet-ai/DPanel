//! Restarting an account's own web server or PHP (T06.12).
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | POST | `/api/v1/client/services/{service}/restart` | a client (`service` is `web` or `php`) |
//!
//! The restart is queued for the node and answered with 202 and the id of the
//! operation. An account may ask three times a minute; more is 429 with the time
//! to wait. Only a running (active) account can restart anything.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::post,
};
use builtpanel_database::operations::{self, EnqueueError, NewOperation};
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_node_protocol::Operation;
use builtpanel_rbac::Principal;
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{DomainsState, client_hosting, entry, internal};

/// How many restarts an account may ask for in a minute.
pub const RESTARTS_PER_MINUTE: u32 = 3;

/// Routes for clients.
pub fn client_router() -> Router<DomainsState> {
    Router::new().route("/api/v1/client/services/{service}/restart", post(restart))
}

/// A restart that was queued.
#[derive(Debug, Serialize, ToSchema)]
pub struct Restarted {
    /// The operation that does it.
    pub operation_id: Uuid,
}

/// Restart your web server or your PHP.
#[utoipa::path(
    post,
    path = "/api/v1/client/services/{service}/restart",
    tag = "domains",
    operation_id = "clientRestartService",
    security(("session" = [])),
    params(
        ("service" = String, Path, description = "`web` or `php`"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 202, description = "Queued", body = Restarted),
        (status = 403, description = "`client_only` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`tenant_not_running`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "`unknown_service`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 429, description = "Three restarts a minute, with Retry-After", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn restart(
    State(state): State<DomainsState>,
    who: Principal,
    Path(service): Path<String>,
) -> Result<(StatusCode, Json<Restarted>), AppError> {
    let hosting = client_hosting(&state, &who).await?;
    if !matches!(service.as_str(), "web" | "php") {
        return Err(AppError::validation(vec![FieldError::new(
            "service",
            "unknown_service",
            "The service is `web` or `php`.",
        )]));
    }
    if hosting.tenant.status.as_str() != "active" {
        return Err(AppError::conflict(
            "tenant_not_running",
            "Only a running account can restart its services.",
        ));
    }
    if !state
        .restart_limiter
        .hit(&hosting.tenant.id.to_string())
        .await
        .map_err(internal)?
    {
        return Err(AppError::too_many_requests(
            "too_many_restarts",
            "Restarts are limited to three a minute. Try again shortly.",
            std::time::Duration::from_secs(60),
        ));
    }
    let op = Operation::RestartService {
        username: hosting.tenant.username.clone(),
        service: service.clone(),
    };
    let payload = serde_json::to_value(&op).map_err(internal)?;
    let key = format!("restart:{}:{}", hosting.tenant.id.simple(), Uuid::now_v7());
    let mut tx = state.db.begin().await.map_err(internal)?;
    let queued = operations::enqueue_in(
        &mut tx,
        &NewOperation {
            node_id: hosting.tenant.node_id,
            kind: op.type_name(),
            tenant_id: Some(hosting.tenant.id),
            payload: &payload,
            idempotency_key: &key,
            created_by: Some(who.account_id()),
            ttl_secs: 600,
            max_attempts: 2,
        },
    )
    .await
    .map_err(|e| match e {
        EnqueueError::UnknownNode => {
            AppError::conflict("node_unavailable", "The node of this account is gone.")
        }
        other => internal(other),
    })?;
    builtpanel_audit::insert(
        &mut *tx,
        &entry("service.restarted", &who, &hosting, hosting.tenant.id)
            .metadata(serde_json::json!({ "service": service })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(Restarted {
            operation_id: queued.operation().id,
        }),
    ))
}
