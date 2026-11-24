//! phpMyAdmin for clients (T07.09).
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | POST | `/api/v1/client/databases/{id}/phpmyadmin` | the owner; the `databases` feature |
//!
//! The answer is a link that signs the client in to the node's phpMyAdmin, as a
//! temporary account with rights on that one database. The link carries a token
//! that works once (the control plane tells the node only the token's SHA-256) and
//! five minutes at most; the session lasts `phpmyadmin_session_minutes`. The name
//! phpMyAdmin is served under is the setting `phpmyadmin_host`; empty, the feature
//! is off.

use std::time::Duration;

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::post,
};
use builtpanel_database::operations::{self, Status};
use builtpanel_domain::{Fqdn, TenantUsername};
use builtpanel_http_kit::{AppError, ProblemDetails};
use builtpanel_node_protocol::Operation;
use builtpanel_rbac::Principal;
use builtpanel_session::rfc3339;
use builtpanel_settings::Scope;
use serde::Serialize;
use sha2::{Digest, Sha256};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{DatabasesState, client_hosting, entry, internal, queue, require_feature, running};

/// How long the request waits for the node.
const WAIT: Duration = Duration::from_secs(25);
/// Most sign-ins an account may ask for in ten minutes.
const MAX_PER_TEN_MINUTES: i64 = 20;

/// Routes for clients, with full paths.
pub fn client_router() -> Router<DatabasesState> {
    Router::new().route(
        "/api/v1/client/databases/{id}/phpmyadmin",
        post(open_phpmyadmin),
    )
}

/// A link into phpMyAdmin.
#[derive(Debug, Serialize, ToSchema)]
pub struct PhpMyAdminLink {
    /// Open this address (it works once).
    #[schema(example = "https://pma.example.com/index.php?dhsignon=...")]
    pub url: String,
    /// Until when the link can be opened (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub expires_at: String,
}

async fn settings(state: &DatabasesState) -> Result<(String, u32), AppError> {
    let mut conn = state.db.acquire().await.map_err(internal)?;
    let values = builtpanel_settings::get(&state.settings, &mut conn, "databases", Scope::Global)
        .await
        .map_err(internal)?;
    let host = values
        .get("phpmyadmin_host")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_owned();
    let minutes = values
        .get("phpmyadmin_session_minutes")
        .and_then(serde_json::Value::as_u64)
        .and_then(|m| u32::try_from(m).ok())
        .unwrap_or(120)
        .clamp(5, 720);
    Ok((host, minutes))
}

/// Open a database in phpMyAdmin.
#[utoipa::path(
    post,
    path = "/api/v1/client/databases/{id}/phpmyadmin",
    tag = "databases",
    operation_id = "clientOpenPhpMyAdmin",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Database id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 200, description = "The link", body = PhpMyAdminLink),
        (status = 403, description = "`client_only`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such database", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`phpmyadmin_unavailable`, `database_not_ready` or `tenant_not_running`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 429, description = "Too many sign-ins in a short time", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 503, description = "`phpmyadmin_failed` (the node could not start it) or `node_busy` (it is not ready yet; try again)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn open_phpmyadmin(
    State(state): State<DatabasesState>,
    who: Principal,
    Path(database_id): Path<Uuid>,
) -> Result<Json<PhpMyAdminLink>, AppError> {
    let h = client_hosting(&state, &who).await?;
    require_feature(&state, &h, "databases", "Databases are").await?;
    running(&h)?;
    let (host, minutes) = settings(&state).await?;
    let host = Fqdn::parse(&host)
        .map_err(|_| {
            AppError::conflict(
                "phpmyadmin_unavailable",
                "phpMyAdmin is not set up on this panel.",
            )
        })?
        .as_str()
        .to_owned();
    let db = builtpanel_database::databases::get(&state.db, h.tenant.id, database_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("database"))?;
    if db.engine != "mysql" {
        return Err(AppError::conflict(
            "web_client_unavailable",
            "A web client for PostgreSQL databases is not available yet; connect with your own client.",
        ));
    }
    if super::server_for(&state, &h, builtpanel_node_protocol::DbEngine::Mysql)
        .await?
        .is_some()
    {
        return Err(AppError::conflict(
            "web_client_unavailable",
            "A web client is not available for a database on a server of your own; connect with your own client.",
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
         WHERE tenant_id = $1 AND kind = 'open_phpmyadmin' AND created_at > now() - interval '10 minutes'",
    )
    .bind(h.tenant.id)
    .fetch_one(&state.db)
    .await
    .map_err(internal)?;
    if recent >= MAX_PER_TEN_MINUTES {
        return Err(AppError::too_many_requests(
            "too_many_signins",
            "Too many phpMyAdmin sign-ins; wait a few minutes.",
            Duration::from_secs(300),
        ));
    }
    let session_id = Uuid::new_v4();
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let token_hash: String = Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let host_key: String = Sha256::digest(host.as_bytes())
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect();
    let tenant_name = TenantUsername::parse(&h.tenant.username).map_err(internal)?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    // The route comes first (one operation per name, however many sign-ins).
    let route = queue(
        &mut tx,
        &h,
        &Operation::SetPhpMyAdminHost {
            host: Some(host.clone()),
        },
        format!("pmahost:{host_key}"),
        &who,
    )
    .await?;
    let open = queue(
        &mut tx,
        &h,
        &Operation::OpenPhpMyAdmin {
            username: tenant_name.as_str().to_owned(),
            database: db.name.clone(),
            session_id,
            token_hash,
            session_minutes: minutes,
        },
        format!("pma:{}", session_id.simple()),
        &who,
    )
    .await?;
    builtpanel_audit::insert(
        &mut *tx,
        &entry("database.phpmyadmin", &who, &h, "database", db.id)
            .metadata(serde_json::json!({ "database": db.name })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;

    // The link is worth giving only once the node has made the sign-in.
    let started = std::time::Instant::now();
    loop {
        let (mut route_done, mut open_done) = (false, false);
        for (id, done) in [(route, &mut route_done), (open, &mut open_done)] {
            let row = operations::get(&state.db, id)
                .await
                .map_err(internal)?
                .ok_or_else(|| internal(std::io::Error::other("an operation vanished")))?;
            match row.status {
                Status::Succeeded => *done = true,
                Status::Failed => {
                    return Err(AppError::unavailable(
                        "phpmyadmin_failed",
                        "The node could not start phpMyAdmin.",
                    ));
                }
                Status::Pending | Status::Dispatched | Status::Retryable => {}
            }
        }
        if route_done && open_done {
            break;
        }
        if started.elapsed() > WAIT {
            return Err(AppError::unavailable(
                "node_busy",
                "phpMyAdmin is not ready yet; try again in a moment.",
            ));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let ends = operations::get(&state.db, open)
        .await
        .map_err(internal)?
        .and_then(|r| r.output)
        .and_then(|o| o.get("link_expires_at").and_then(serde_json::Value::as_i64))
        .unwrap_or(0);
    Ok(Json(PhpMyAdminLink {
        url: format!("https://{host}/index.php?dhsignon={token}"),
        expires_at: rfc3339(ends),
    }))
}
