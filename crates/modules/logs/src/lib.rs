//! An account's logs, for its owner (T10.04).
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET | `/api/v1/client/logs` | the owner; the `logs` feature |
//!
//! The three logs the node can read (`web_access`, `web_error`, `php_error`: what the
//! account's containers write, kept by Docker), the newest lines, from a time on, filtered by
//! a text. The request waits a few seconds for the node; a live view asks again with the time
//! of the newest line it has (`since_ms` is exclusive of that line).

use std::time::Duration;

use axum::{
    Json, Router,
    extract::{FromRef, Query, State},
    http::StatusCode,
    response::{IntoResponse as _, Response},
    routing::get,
};
use builtpanel_database::{
    accounts::DbAccountKind,
    operations::{self, NewOperation, Status},
    plans,
    sqlx::PgPool,
    tenants,
};
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_node_protocol::{
    LogLine, LogSource, MAX_LOG_LINES, MAX_TRAFFIC_PATHS, Operation, Output,
};
use builtpanel_rbac::Principal;
use builtpanel_session::SessionStore;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, OpenApi, ToSchema};
use uuid::Uuid;

/// The module's manifest.
pub const MANIFEST: &str = include_str!("../../../../modules/logs/module.toml");

/// What the module's handlers share.
#[derive(Clone)]
pub struct LogsState {
    /// Database pool.
    pub db: PgPool,
    /// Session store.
    pub sessions: SessionStore,
    /// How long a request waits for the node before answering `202`.
    pub wait: Duration,
}

impl FromRef<LogsState> for PgPool {
    fn from_ref(s: &LogsState) -> Self {
        s.db.clone()
    }
}

impl FromRef<LogsState> for SessionStore {
    fn from_ref(s: &LogsState) -> Self {
        s.sessions.clone()
    }
}

/// How long a request waits for the node by default.
pub const DEFAULT_WAIT: Duration = Duration::from_secs(8);

/// Routes for hosting accounts.
pub fn client_router() -> Router<LogsState> {
    Router::new()
        .route("/api/v1/client/logs", get(read_logs))
        .route("/api/v1/client/activity", get(activity))
        .route("/api/v1/client/traffic", get(traffic))
}

fn internal(err: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>) -> AppError {
    AppError::internal(err)
}

/// Which log, from when, what to look for.
#[derive(Debug, Deserialize, IntoParams)]
pub struct LogQuery {
    /// `web_access`, `web_error` or `php_error`.
    pub source: String,
    /// Only lines after this time (Unix milliseconds). Default: any.
    #[serde(default)]
    pub since_ms: i64,
    /// How many of the newest lines (1-2000). Default 200.
    pub lines: Option<u32>,
    /// Only lines that contain this text (case-insensitive, up to 200 characters).
    pub search: Option<String>,
}

/// The lines.
#[derive(Debug, Serialize, ToSchema)]
pub struct LogView {
    /// Oldest first.
    pub lines: Vec<LogLineView>,
    /// Older lines were left out.
    pub truncated: bool,
}

/// One line.
#[derive(Debug, Serialize, ToSchema)]
pub struct LogLineView {
    /// When it was written (Unix milliseconds).
    pub at_ms: i64,
    /// The line.
    pub text: String,
}

/// The node did not answer in time.
#[derive(Debug, Serialize, ToSchema)]
pub struct Pending {
    /// The operation; asking again gives a fresh answer.
    pub operation_id: Uuid,
}

fn bad(name: &'static str, code: &'static str, message: &str) -> AppError {
    AppError::validation(vec![FieldError::new(name, code, message)])
}

/// Read the end of one of the account's logs.
#[utoipa::path(
    get,
    path = "/api/v1/client/logs",
    tag = "logs",
    operation_id = "clientReadLog",
    security(("session" = [])),
    params(LogQuery),
    responses(
        (status = 200, description = "The lines", body = LogView),
        (status = 202, description = "The node is slow; ask again", body = Pending),
        (status = 403, description = "`client_only` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`tenant_not_running`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "A source, count or search that is not usable", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 503, description = "`node_unavailable`, or the account has no such log yet", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn read_logs(
    State(state): State<LogsState>,
    who: Principal,
    Query(q): Query<LogQuery>,
) -> Result<Response, AppError> {
    if who.kind != DbAccountKind::Client {
        return Err(AppError::forbidden(
            "client_only",
            "This is for hosting accounts.",
        ));
    }
    let tenant = running_hosting(&state, &who).await?;
    let source = match q.source.as_str() {
        "web_access" => LogSource::WebAccess,
        "web_error" => LogSource::WebError,
        "php_error" => LogSource::PhpError,
        _ => {
            return Err(bad(
                "source",
                "invalid",
                "web_access, web_error or php_error.",
            ));
        }
    };
    let lines = q.lines.unwrap_or(200);
    if !(1..=MAX_LOG_LINES).contains(&lines) || q.since_ms < 0 {
        return Err(bad("lines", "out_of_range", "Between 1 and 2000 lines."));
    }
    let search = q.search.filter(|s| !s.is_empty());
    if search
        .as_ref()
        .is_some_and(|s| s.chars().count() > 200 || s.contains('\0'))
    {
        return Err(bad("search", "invalid", "At most 200 characters."));
    }
    let op = Operation::ReadLog {
        tenant_id: tenant.id,
        username: tenant.username.clone(),
        source,
        // Lines at the cursor's own time are the ones the viewer has.
        since_ms: q.since_ms.saturating_add(i64::from(q.since_ms > 0)),
        tail: lines,
        contains: search,
    };
    match ask_node(&state, &tenant, who.account_id(), &op, "log").await? {
        Answer::Slow(id) => {
            Ok((StatusCode::ACCEPTED, Json(Pending { operation_id: id })).into_response())
        }
        Answer::Done(Output::LogLines { lines, truncated }) => Ok(Json(LogView {
            lines: lines
                .into_iter()
                .map(|LogLine { at_ms, text }| LogLineView { at_ms, text })
                .collect(),
            truncated,
        })
        .into_response()),
        Answer::Done(_) => Err(internal(std::io::Error::other("an unexpected answer"))),
    }
}

/// What the node said.
enum Answer {
    Done(Output),
    /// Not yet: the operation to ask about again.
    Slow(Uuid),
}

/// The account's hosting, when its plan has logs and it is running.
async fn running_hosting(
    state: &LogsState,
    who: &Principal,
) -> Result<builtpanel_database::tenants::Tenant, AppError> {
    let user = builtpanel_database::users::get(&state.db, who.account_id())
        .await
        .map_err(internal)?;
    let owned = user.and_then(|u| u.tenant).ok_or_else(|| {
        AppError::conflict("no_hosting", "This account has no hosting account yet.")
    })?;
    let tenant = tenants::get(&state.db, owned.id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("user"))?;
    if !plans::includes_feature(&state.db, tenant.plan_id, "logs")
        .await
        .map_err(internal)?
    {
        return Err(AppError::forbidden(
            "feature_disabled",
            "Logs are not included in this plan.",
        ));
    }
    if tenant.status.as_str() != "active" {
        return Err(AppError::conflict(
            "tenant_not_running",
            "Only a running account has logs to read.",
        ));
    }
    Ok(tenant)
}

/// Sends an operation to the account's node and waits a little for its answer.
async fn ask_node(
    state: &LogsState,
    tenant: &builtpanel_database::tenants::Tenant,
    by: Uuid,
    op: &Operation,
    key: &str,
) -> Result<Answer, AppError> {
    let payload = serde_json::to_value(op).map_err(internal)?;
    let queued = operations::enqueue(
        &state.db,
        &NewOperation {
            node_id: tenant.node_id,
            kind: op.type_name(),
            tenant_id: Some(tenant.id),
            payload: &payload,
            idempotency_key: &format!("{key}:{}", Uuid::new_v4().simple()),
            created_by: Some(by),
            ttl_secs: 60,
            max_attempts: 1,
        },
    )
    .await
    .map_err(|e| match e {
        operations::EnqueueError::UnknownNode => AppError::conflict(
            "node_unavailable",
            "The node of this account no longer exists.",
        ),
        other => internal(other),
    })?;
    let id = queued.operation().id;
    let started = std::time::Instant::now();
    loop {
        let row = operations::get(&state.db, id)
            .await
            .map_err(internal)?
            .ok_or_else(|| AppError::not_found("operation"))?;
        match row.status {
            Status::Succeeded => {
                let output: Output = row
                    .output
                    .and_then(|o| serde_json::from_value(o).ok())
                    .ok_or_else(|| internal(std::io::Error::other("an unreadable answer")))?;
                return Ok(Answer::Done(output));
            }
            Status::Failed => {
                return Err(AppError::unavailable(
                    "node_unavailable",
                    "The node could not read this log (the account may have no such log yet).",
                ));
            }
            Status::Pending | Status::Dispatched | Status::Retryable => {}
        }
        if started.elapsed() >= state.wait {
            return Ok(Answer::Slow(id));
        }
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
}

/// Which page of the activity.
#[derive(Debug, Deserialize, IntoParams)]
pub struct ActivityQuery {
    /// How many entries (1-200, default 50).
    pub limit: Option<i64>,
    /// `next_cursor` of the page before.
    pub cursor: Option<String>,
    /// Only actions that start with this (`login`, `database`, ...).
    pub action: Option<String>,
}

/// One thing that happened in the account.
#[derive(Debug, Serialize, ToSchema)]
pub struct ActivityItem {
    /// Entry id.
    pub id: Uuid,
    /// When (RFC 3339, UTC).
    pub at: String,
    /// What happened, e.g. `database.create`.
    pub action: String,
    /// `success`, `failure` or `denied`.
    pub outcome: String,
    /// What it was done to (`domain`, `database`, ...).
    pub target_type: Option<String>,
    /// Which one.
    pub target_id: Option<String>,
    /// Whether the account's own login did it (otherwise staff did, on its behalf).
    pub by_you: bool,
    /// Where from: only for what the account did itself.
    pub ip: Option<String>,
}

/// A page of the activity.
#[derive(Debug, Serialize, ToSchema)]
pub struct ActivityPage {
    /// Newest first.
    pub items: Vec<ActivityItem>,
    /// Pass this as `cursor` for the next page (`null`: the end).
    pub next_cursor: Option<String>,
}

/// What happened in the account, newest first (logins, changes, failures).
#[utoipa::path(
    get,
    path = "/api/v1/client/activity",
    tag = "logs",
    operation_id = "clientActivity",
    security(("session" = [])),
    params(ActivityQuery),
    responses(
        (status = 200, description = "A page of entries", body = ActivityPage),
        (status = 403, description = "`client_only`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "A count or cursor that is not usable", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn activity(
    State(state): State<LogsState>,
    who: Principal,
    Query(q): Query<ActivityQuery>,
) -> Result<Json<ActivityPage>, AppError> {
    if who.kind != DbAccountKind::Client {
        return Err(AppError::forbidden(
            "client_only",
            "This is for hosting accounts.",
        ));
    }
    let limit = q.limit.unwrap_or(50);
    if !(1..=200).contains(&limit) {
        return Err(bad("limit", "out_of_range", "Between 1 and 200."));
    }
    let cursor = match q.cursor.as_deref() {
        None => None,
        Some(c) => {
            let (micros, id) = c
                .split_once('.')
                .and_then(|(m, i)| Some((m.parse::<i64>().ok()?, Uuid::parse_str(i).ok()?)))
                .ok_or_else(|| bad("cursor", "invalid", "Not a cursor this API gave."))?;
            Some((micros, id))
        }
    };
    let action = match q.action.as_deref() {
        None | Some("") => None,
        Some(a)
            if a.len() <= 64
                && a.bytes().all(|b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'.'
                }) =>
        {
            Some(a.to_owned())
        }
        Some(_) => {
            return Err(bad(
                "action",
                "invalid",
                "Lowercase letters, digits, `_` and `.`.",
            ));
        }
    };
    let user = builtpanel_database::users::get(&state.db, who.account_id())
        .await
        .map_err(internal)?;
    let owned = user.and_then(|u| u.tenant).ok_or_else(|| {
        AppError::conflict("no_hosting", "This account has no hosting account yet.")
    })?;
    let me = who.account_id();
    let (cursor_micros, cursor_id) = cursor.unzip();
    let mut rows = builtpanel_database::sqlx::query_as::<
        _,
        (Uuid, i64, Uuid, Option<String>, String, String, Option<String>, Option<String>),
    >(
        "SELECT id, (extract(epoch FROM at) * 1000000)::bigint, COALESCE(actor_id, '00000000-0000-0000-0000-000000000000'::uuid),
                host(actor_ip), action, outcome, target_type, target_id
         FROM audit_logs
         WHERE tenant_id = $1
           AND ($2::text IS NULL OR starts_with(action, $2))
           AND ($3::bigint IS NULL
                OR (at, id) < ('epoch'::timestamptz + $3::bigint * interval '1 microsecond', $4::uuid))
         ORDER BY at DESC, id DESC
         LIMIT $5",
    )
    .bind(owned.id)
    .bind(action)
    .bind(cursor_micros)
    .bind(cursor_id)
    .bind(limit + 1)
    .fetch_all(&state.db)
    .await
    .map_err(internal)?;
    let has_more = i64::try_from(rows.len()).is_ok_and(|n| n > limit);
    if has_more {
        rows.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
    }
    let next_cursor = has_more
        .then(|| rows.last().map(|r| format!("{}.{}", r.1, r.0)))
        .flatten();
    let items = rows
        .into_iter()
        .map(
            |(id, micros, actor, ip, action, outcome, target_type, target_id)| {
                let by_you = actor == me;
                ActivityItem {
                    id,
                    at: rfc3339(micros),
                    action,
                    outcome,
                    target_type,
                    target_id,
                    by_you,
                    ip: if by_you { ip } else { None },
                }
            },
        )
        .collect();
    Ok(Json(ActivityPage { items, next_cursor }))
}

/// Which time, how many pages.
#[derive(Debug, Deserialize, IntoParams)]
pub struct TrafficQuery {
    /// `24h` (default), `7d` or `30d`.
    pub range: Option<String>,
    /// How many of the most asked pages (1-50, default 10).
    pub top: Option<u32>,
}

/// One hour of traffic.
#[derive(Debug, Serialize, ToSchema)]
pub struct TrafficHourView {
    /// The start of the hour (Unix milliseconds, UTC).
    pub at_ms: i64,
    /// Requests in it.
    pub requests: u64,
    /// Bytes sent in it.
    pub bytes: u64,
}

/// A page and its requests.
#[derive(Debug, Serialize, ToSchema)]
pub struct TrafficPathView {
    /// The path (no query string).
    pub path: String,
    /// Requests for it.
    pub requests: u64,
}

/// An account's web traffic.
#[derive(Debug, Serialize, ToSchema)]
pub struct TrafficView {
    /// The range asked for.
    pub range: String,
    /// Requests counted.
    pub requests: u64,
    /// Bytes sent.
    pub bytes: u64,
    /// Answers 200-299.
    pub status_2xx: u64,
    /// Answers 300-399.
    pub status_3xx: u64,
    /// Answers 400-499.
    pub status_4xx: u64,
    /// Answers 500-599.
    pub status_5xx: u64,
    /// Requests from robots (by their User-Agent).
    pub bots: u64,
    /// Per hour, oldest first (hours without a request are left out).
    pub hours: Vec<TrafficHourView>,
    /// The most asked pages, most first.
    pub top_paths: Vec<TrafficPathView>,
    /// The log held less than the range (the numbers are a lower bound).
    pub partial: bool,
}

/// How many requests the account's website got: totals, by kind of answer, per hour and the
/// most asked pages. Counted from the web server's log, which the node keeps for a while only.
#[utoipa::path(
    get,
    path = "/api/v1/client/traffic",
    tag = "logs",
    operation_id = "clientTraffic",
    security(("session" = [])),
    params(TrafficQuery),
    responses(
        (status = 200, description = "The counts", body = TrafficView),
        (status = 202, description = "The node is slow; ask again", body = Pending),
        (status = 403, description = "`client_only` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`tenant_not_running`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "A range or count that is not usable", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 503, description = "`node_unavailable`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn traffic(
    State(state): State<LogsState>,
    who: Principal,
    Query(q): Query<TrafficQuery>,
) -> Result<Response, AppError> {
    if who.kind != DbAccountKind::Client {
        return Err(AppError::forbidden(
            "client_only",
            "This is for hosting accounts.",
        ));
    }
    let range = q.range.unwrap_or_else(|| "24h".to_owned());
    let hours: i64 = match range.as_str() {
        "24h" => 24,
        "7d" => 7 * 24,
        "30d" => 30 * 24,
        _ => return Err(bad("range", "invalid", "24h, 7d or 30d.")),
    };
    let top = q.top.unwrap_or(10);
    if !(1..=MAX_TRAFFIC_PATHS).contains(&top) {
        return Err(bad("top", "out_of_range", "Between 1 and 50."));
    }
    let tenant = running_hosting(&state, &who).await?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(0));
    let op = Operation::ReadTraffic {
        tenant_id: tenant.id,
        username: tenant.username.clone(),
        since_ms: (now_ms - hours * 3_600_000).max(0),
        top,
    };
    match ask_node(&state, &tenant, who.account_id(), &op, "traffic").await? {
        Answer::Slow(id) => {
            Ok((StatusCode::ACCEPTED, Json(Pending { operation_id: id })).into_response())
        }
        Answer::Done(Output::Traffic {
            requests,
            bytes,
            status_2xx,
            status_3xx,
            status_4xx,
            status_5xx,
            bots,
            hours,
            top_paths,
            partial,
        }) => Ok(Json(TrafficView {
            range,
            requests,
            bytes,
            status_2xx,
            status_3xx,
            status_4xx,
            status_5xx,
            bots,
            hours: hours
                .into_iter()
                .map(|h| TrafficHourView {
                    at_ms: h.at_ms,
                    requests: h.requests,
                    bytes: h.bytes,
                })
                .collect(),
            top_paths: top_paths
                .into_iter()
                .map(|p| TrafficPathView {
                    path: p.path,
                    requests: p.requests,
                })
                .collect(),
            partial,
        })
        .into_response()),
        Answer::Done(_) => Err(internal(std::io::Error::other("an unexpected answer"))),
    }
}

/// Microseconds since the epoch as RFC 3339 (UTC).
fn rfc3339(micros: i64) -> String {
    time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(micros) * 1000)
        .ok()
        .and_then(|t| {
            t.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .unwrap_or_default()
}

/// The module's OpenAPI document.
#[derive(OpenApi)]
#[openapi(
    paths(read_logs, activity, traffic),
    components(schemas(LogView, LogLineView, Pending, ActivityItem, ActivityPage, TrafficView, TrafficHourView, TrafficPathView)),
    tags((name = "logs", description = "Logs"))
)]
pub struct LogsApi;
