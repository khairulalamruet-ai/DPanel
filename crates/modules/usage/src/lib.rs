//! Usage charts (T10.10's API).
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET | `/api/v1/client/usage?range=` | the owner |
//! | GET | `/api/v1/admin/usage/top?by=&range=&limit=` | `admin.users.read` |
//! | GET | `/api/v1/admin/usage/nodes/{id}?range=` | `admin.nodes.read` |
//!
//! Ranges: `1h` (1-minute points), `24h` (5 minutes), `7d` (1 hour), `30d` (6 hours). CPU is
//! in thousandths of a CPU (1000 = one whole CPU), bytes are per point. The numbers come from
//! the node readings of `builtpanel_database::usage` (one a minute).

use axum::{
    Json, Router,
    extract::{FromRef, Path, Query, State},
    routing::get,
};
use builtpanel_database::{
    accounts::DbAccountKind,
    plans,
    sqlx::PgPool,
    tenants,
    usage::{self, Rank},
    users,
};
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_rbac::{
    Principal, Require,
    permissions::{NodesRead, UsersRead},
};
use builtpanel_session::SessionStore;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, OpenApi, ToSchema};
use uuid::Uuid;

/// The module's manifest.
pub const MANIFEST: &str = include_str!("../../../../modules/usage/module.toml");

/// What the module's handlers share.
#[derive(Clone)]
pub struct UsageState {
    /// Database pool.
    pub db: PgPool,
    /// Session store.
    pub sessions: SessionStore,
}

impl FromRef<UsageState> for PgPool {
    fn from_ref(s: &UsageState) -> Self {
        s.db.clone()
    }
}

impl FromRef<UsageState> for SessionStore {
    fn from_ref(s: &UsageState) -> Self {
        s.sessions.clone()
    }
}

/// Routes for hosting accounts.
pub fn client_router() -> Router<UsageState> {
    Router::new().route("/api/v1/client/usage", get(client_usage))
}

/// Routes for administrators.
pub fn admin_router() -> Router<UsageState> {
    Router::new()
        .route("/api/v1/admin/usage/top", get(admin_top))
        .route("/api/v1/admin/usage/nodes/{id}", get(admin_node))
}

fn internal(err: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>) -> AppError {
    AppError::internal(err)
}

/// `(seconds back, seconds per point)` of a range name.
fn range(name: Option<&str>) -> Result<(i64, i64), AppError> {
    match name.unwrap_or("24h") {
        "1h" => Ok((3600, 60)),
        "24h" => Ok((86_400, 300)),
        "7d" => Ok((7 * 86_400, 3600)),
        "30d" => Ok((30 * 86_400, 6 * 3600)),
        _ => Err(AppError::validation(vec![FieldError::new(
            "range",
            "invalid",
            "1h, 24h, 7d or 30d.",
        )])),
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// Which range.
#[derive(Debug, Deserialize, IntoParams)]
pub struct RangeQuery {
    /// `1h`, `24h` (default), `7d` or `30d`.
    pub range: Option<String>,
}

/// One point of an account's usage.
#[derive(Debug, Serialize, ToSchema)]
pub struct UsagePoint {
    /// Start of the point (Unix seconds).
    pub at: i64,
    /// Average thousandths of a CPU.
    pub cpu_millis: i32,
    /// Average memory in use, in bytes.
    pub memory_bytes: i64,
    /// Most processes.
    pub pids: i32,
    /// Bytes received in the point.
    pub net_rx_bytes: i64,
    /// Bytes sent in the point.
    pub net_tx_bytes: i64,
    /// Bytes read from disk in the point.
    pub io_read_bytes: i64,
    /// Bytes written to disk in the point.
    pub io_write_bytes: i64,
}

/// What the plan allows (to draw the limit line).
#[derive(Debug, Serialize, ToSchema)]
pub struct UsageLimits {
    /// Thousandths of a CPU (`null`: no limit).
    pub cpu_millis: Option<i32>,
    /// Memory in bytes (`null`: no limit).
    pub memory_bytes: Option<i64>,
    /// Processes (`null`: no limit).
    pub pids: Option<i32>,
}

/// An account's usage.
#[derive(Debug, Serialize, ToSchema)]
pub struct UsageView {
    /// The range asked for.
    pub range: String,
    /// Seconds per point.
    pub step_seconds: i64,
    /// Oldest first (points with no reading are left out).
    pub points: Vec<UsagePoint>,
    /// The plan's limits.
    pub limits: UsageLimits,
}

/// The account's CPU, memory, network and disk use over time.
#[utoipa::path(
    get,
    path = "/api/v1/client/usage",
    tag = "usage",
    operation_id = "clientUsage",
    security(("session" = [])),
    params(RangeQuery),
    responses(
        (status = 200, description = "The usage", body = UsageView),
        (status = 403, description = "`client_only`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "A range that is not one of the four", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn client_usage(
    State(state): State<UsageState>,
    who: Principal,
    Query(q): Query<RangeQuery>,
) -> Result<Json<UsageView>, AppError> {
    if who.kind != DbAccountKind::Client {
        return Err(AppError::forbidden(
            "client_only",
            "This is for hosting accounts.",
        ));
    }
    let (back, step) = range(q.range.as_deref())?;
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
    let to = now() + 60;
    let points = usage::series(&state.db, tenant.id, to - back, to, step)
        .await
        .map_err(internal)?
        .into_iter()
        .map(|p| UsagePoint {
            at: p.at,
            cpu_millis: p.cpu_millis,
            memory_bytes: p.memory_bytes,
            pids: p.pids,
            net_rx_bytes: p.net_rx_bytes,
            net_tx_bytes: p.net_tx_bytes,
            io_read_bytes: p.io_read_bytes,
            io_write_bytes: p.io_write_bytes,
        })
        .collect();
    Ok(Json(UsageView {
        range: q.range.unwrap_or_else(|| "24h".to_owned()),
        step_seconds: step,
        points,
        limits: UsageLimits {
            cpu_millis: plan.cpu_millis,
            memory_bytes: plan.memory_bytes,
            pids: plan.pids,
        },
    }))
}

/// Which ranking.
#[derive(Debug, Deserialize, IntoParams)]
pub struct TopQuery {
    /// `cpu` (default), `memory`, `network` or `io`.
    pub by: Option<String>,
    /// `1h`, `24h` (default), `7d` or `30d`.
    pub range: Option<String>,
    /// How many (1-100, default 10).
    pub limit: Option<i64>,
}

/// One ranked account.
#[derive(Debug, Serialize, ToSchema)]
pub struct TopItem {
    /// The account.
    pub tenant_id: Uuid,
    /// Its system name.
    pub username: String,
    /// Average thousandths of a CPU over the range.
    pub cpu_millis: i64,
    /// Average memory in bytes over the range.
    pub memory_bytes: i64,
    /// Network bytes (in and out) over the range.
    pub net_bytes: i64,
    /// Disk bytes (read and written) over the range.
    pub io_bytes: i64,
}

/// The ranking.
#[derive(Debug, Serialize, ToSchema)]
pub struct TopList {
    /// Most first.
    pub items: Vec<TopItem>,
}

/// The accounts that used most.
#[utoipa::path(
    get,
    path = "/api/v1/admin/usage/top",
    tag = "usage",
    operation_id = "adminUsageTop",
    security(("session" = [])),
    params(TopQuery),
    responses(
        (status = 200, description = "The ranking", body = TopList),
        (status = 403, description = "`permission_denied` (needs admin.users.read)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "A ranking, range or count that is not usable", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn admin_top(
    State(state): State<UsageState>,
    _who: Require<UsersRead>,
    Query(q): Query<TopQuery>,
) -> Result<Json<TopList>, AppError> {
    let (back, _) = range(q.range.as_deref())?;
    let by = match q.by.as_deref().unwrap_or("cpu") {
        "cpu" => Rank::Cpu,
        "memory" => Rank::Memory,
        "network" => Rank::Network,
        "io" => Rank::Io,
        _ => {
            return Err(AppError::validation(vec![FieldError::new(
                "by",
                "invalid",
                "cpu, memory, network or io.",
            )]));
        }
    };
    let limit = q.limit.unwrap_or(10);
    if !(1..=100).contains(&limit) {
        return Err(AppError::validation(vec![FieldError::new(
            "limit",
            "out_of_range",
            "Between 1 and 100.",
        )]));
    }
    let to = now() + 60;
    let items = usage::top(&state.db, to - back, to, by, limit)
        .await
        .map_err(internal)?
        .into_iter()
        .map(|r| TopItem {
            tenant_id: r.tenant_id,
            username: r.username,
            cpu_millis: r.cpu_millis,
            memory_bytes: r.memory_bytes,
            net_bytes: r.net_bytes,
            io_bytes: r.io_bytes,
        })
        .collect();
    Ok(Json(TopList { items }))
}

/// One point of a node's usage.
#[derive(Debug, Serialize, ToSchema)]
pub struct NodePointView {
    /// Start of the point (Unix seconds).
    pub at: i64,
    /// Average thousandths of the node's CPU that were busy.
    pub cpu_busy_millis: i32,
    /// Average memory in use, in bytes.
    pub memory_used_bytes: i64,
    /// Memory of the node, in bytes.
    pub memory_total_bytes: i64,
    /// Average one-minute load times 100.
    pub load1_x100: i32,
    /// Disk bytes in use.
    pub disk_used_bytes: i64,
    /// Disk bytes in all.
    pub disk_total_bytes: i64,
    /// Running containers.
    pub containers: i32,
}

/// A node's usage.
#[derive(Debug, Serialize, ToSchema)]
pub struct NodeUsageView {
    /// Seconds per point.
    pub step_seconds: i64,
    /// Oldest first.
    pub points: Vec<NodePointView>,
}

/// A node's CPU, memory, load and disk over time (the last 30 days at most).
#[utoipa::path(
    get,
    path = "/api/v1/admin/usage/nodes/{id}",
    tag = "usage",
    operation_id = "adminNodeUsage",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Node id"), RangeQuery),
    responses(
        (status = 200, description = "The usage", body = NodeUsageView),
        (status = 403, description = "`permission_denied` (needs admin.nodes.read)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "A range that is not one of the four", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn admin_node(
    State(state): State<UsageState>,
    _who: Require<NodesRead>,
    Path(id): Path<Uuid>,
    Query(q): Query<RangeQuery>,
) -> Result<Json<NodeUsageView>, AppError> {
    let (back, step) = range(q.range.as_deref())?;
    let to = now() + 60;
    let points = usage::node_series(&state.db, id, to - back, to, step)
        .await
        .map_err(internal)?
        .into_iter()
        .map(|p| NodePointView {
            at: p.at,
            cpu_busy_millis: p.cpu_busy_millis,
            memory_used_bytes: p.memory_used_bytes,
            memory_total_bytes: p.memory_total_bytes,
            load1_x100: p.load1_x100,
            disk_used_bytes: p.disk_used_bytes,
            disk_total_bytes: p.disk_total_bytes,
            containers: p.containers,
        })
        .collect();
    Ok(Json(NodeUsageView {
        step_seconds: step,
        points,
    }))
}

/// The module's OpenAPI document.
#[derive(OpenApi)]
#[openapi(
    paths(client_usage, admin_top, admin_node),
    components(schemas(
        UsageView, UsagePoint, UsageLimits, TopList, TopItem, NodeUsageView, NodePointView
    )),
    tags((name = "usage", description = "Usage"))
)]
pub struct UsageApi;
