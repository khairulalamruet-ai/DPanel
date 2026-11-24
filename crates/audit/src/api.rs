//! Read API for the audit log.
//!
//! | Method | Path | Permission |
//! |---|---|---|
//! | GET | `/api/v1/admin/audit-logs` | `admin.audit.read` |
//!
//! Newest first, filtered and paged with an opaque cursor. Entries can only be
//! read here; there is no way to change or remove them through the API.

use axum::{
    Json, Router,
    extract::{FromRef, Query, State},
    routing::get,
};
use builtpanel_database::sqlx::PgPool;
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_rbac::{Require, permissions::AuditRead};
use builtpanel_session::SessionStore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use utoipa::{IntoParams, OpenApi, ToSchema};
use uuid::Uuid;

/// Entries per page when `limit` is not given.
pub const DEFAULT_LIMIT: i64 = 50;
/// Most entries per page.
pub const MAX_LIMIT: i64 = 100;

/// Routes with full paths; merge into the application router.
pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    SessionStore: FromRef<S>,
    PgPool: FromRef<S>,
{
    Router::new().route("/api/v1/admin/audit-logs", get(list))
}

/// Filters and paging for the audit list.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AuditQuery {
    /// Only actions starting with this text, e.g. `auth.` or `auth.login_failed`.
    pub action: Option<String>,
    /// Only entries by this account.
    pub actor_id: Option<String>,
    /// `success`, `failure` or `denied`.
    pub outcome: Option<String>,
    /// Only entries about this kind of object, e.g. `session`.
    pub target_type: Option<String>,
    /// Only entries about this object (use with `target_type`).
    pub target_id: Option<String>,
    /// Earliest time, inclusive (RFC 3339).
    pub from: Option<String>,
    /// Latest time, exclusive (RFC 3339).
    pub to: Option<String>,
    /// Entries per page, 1..=100 (default 50).
    pub limit: Option<String>,
    /// `next_cursor` of the previous page.
    pub cursor: Option<String>,
}

/// One audit entry.
#[derive(Debug, Serialize, ToSchema)]
pub struct AuditItem {
    /// Entry id (time-ordered).
    pub id: Uuid,
    /// When it happened (RFC 3339, UTC, microseconds).
    #[schema(format = DateTime, example = "2026-11-11T09:30:00.123456Z")]
    pub at: String,
    /// Account that acted, if known.
    pub actor_id: Option<Uuid>,
    /// IP address the request came from.
    pub actor_ip: Option<String>,
    /// What happened, e.g. `auth.login_failed`.
    #[schema(example = "auth.login")]
    pub action: String,
    /// Kind of object acted on.
    pub target_type: Option<String>,
    /// Object acted on.
    pub target_id: Option<String>,
    /// Tenant the action belongs to.
    pub tenant_id: Option<Uuid>,
    /// Node the action ran on.
    pub node_id: Option<Uuid>,
    /// `success`, `failure` or `denied`.
    #[schema(example = "success")]
    pub outcome: String,
    /// Request correlation id; matches the `x-request-id` header and the logs.
    pub correlation_id: Option<String>,
    /// Extra details (never secrets).
    #[schema(value_type = Object)]
    pub metadata: Value,
}

/// A page of audit entries.
#[derive(Debug, Serialize, ToSchema)]
pub struct AuditPage {
    /// Newest first.
    pub items: Vec<AuditItem>,
    /// Pass as `cursor` to get the next (older) page; absent on the last page.
    pub next_cursor: Option<String>,
}

fn micros_to_rfc3339(micros: i64) -> String {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(micros) * 1000)
        .ok()
        .and_then(|t| t.format(&Rfc3339).ok())
        .unwrap_or_default()
}

fn parse_time(field: &str, value: &str, errors: &mut Vec<FieldError>) -> Option<i64> {
    match OffsetDateTime::parse(value, &Rfc3339) {
        Ok(t) => i64::try_from(t.unix_timestamp_nanos() / 1000).ok(),
        Err(_) => {
            errors.push(FieldError::new(
                field,
                "invalid_format",
                "Expected an RFC 3339 timestamp such as 2026-11-11T09:30:00Z.",
            ));
            None
        }
    }
}

/// A validated query.
#[derive(Debug, Default, PartialEq)]
struct Filters {
    action: Option<String>,
    actor_id: Option<Uuid>,
    outcome: Option<String>,
    target_type: Option<String>,
    target_id: Option<String>,
    from: Option<i64>,
    to: Option<i64>,
    limit: i64,
    cursor: Option<(i64, Uuid)>,
}

fn validate(q: AuditQuery) -> Result<Filters, AppError> {
    let mut errors = Vec::new();
    let mut f = Filters {
        limit: DEFAULT_LIMIT,
        ..Filters::default()
    };
    let nonempty = |v: Option<String>| v.filter(|s| !s.is_empty());

    f.action = nonempty(q.action);
    if let Some(a) = &f.action
        && (a.len() > 64
            || !a
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'.'))
    {
        errors.push(FieldError::new(
            "action",
            "invalid_format",
            "Use lowercase letters, digits, '_' and '.' (at most 64 characters).",
        ));
    }
    if let Some(id) = nonempty(q.actor_id) {
        match id.parse::<Uuid>() {
            Ok(id) => f.actor_id = Some(id),
            Err(_) => errors.push(FieldError::new(
                "actor_id",
                "invalid_format",
                "Expected a UUID.",
            )),
        }
    }
    f.outcome = nonempty(q.outcome);
    if let Some(o) = &f.outcome
        && !matches!(o.as_str(), "success" | "failure" | "denied")
    {
        errors.push(FieldError::new(
            "outcome",
            "invalid_value",
            "Expected success, failure or denied.",
        ));
    }
    f.target_type = nonempty(q.target_type);
    f.target_id = nonempty(q.target_id);
    for (name, value) in [("target_type", &f.target_type), ("target_id", &f.target_id)] {
        if value.as_ref().is_some_and(|v| v.chars().count() > 128) {
            errors.push(FieldError::new(name, "too_long", "At most 128 characters."));
        }
    }
    if let Some(t) = nonempty(q.from) {
        f.from = parse_time("from", &t, &mut errors);
    }
    if let Some(t) = nonempty(q.to) {
        f.to = parse_time("to", &t, &mut errors);
    }
    if let Some(l) = nonempty(q.limit) {
        match l.parse::<i64>() {
            Ok(n) if (1..=MAX_LIMIT).contains(&n) => f.limit = n,
            _ => errors.push(FieldError::new(
                "limit",
                "out_of_range",
                format!("Expected a number from 1 to {MAX_LIMIT}."),
            )),
        }
    }
    if let Some(c) = nonempty(q.cursor) {
        let parsed = c
            .split_once('.')
            .and_then(|(micros, id)| Some((micros.parse::<i64>().ok()?, id.parse::<Uuid>().ok()?)));
        match parsed {
            Some(cursor) => f.cursor = Some(cursor),
            None => errors.push(FieldError::new(
                "cursor",
                "invalid_value",
                "Use the next_cursor value of a previous response.",
            )),
        }
    }
    if errors.is_empty() {
        Ok(f)
    } else {
        Err(AppError::validation(errors))
    }
}

/// List audit entries, newest first.
#[utoipa::path(
    get,
    path = "/api/v1/admin/audit-logs",
    tag = "audit",
    operation_id = "listAuditLogs",
    security(("session" = [])),
    params(AuditQuery),
    responses(
        (status = 200, description = "A page of entries", body = AuditPage),
        (status = 401, description = "Not logged in", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "`permission_denied`: needs admin.audit.read", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid filter", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn list(
    State(pool): State<PgPool>,
    _who: Require<AuditRead>,
    Query(query): Query<AuditQuery>,
) -> Result<Json<AuditPage>, AppError> {
    let f = validate(query)?;
    let (cursor_micros, cursor_id) = f.cursor.unzip();
    let mut rows = sqlx::query!(
        r#"SELECT id,
                  (extract(epoch FROM at) * 1000000)::bigint AS "at_micros!",
                  actor_id, host(actor_ip) AS actor_ip, action, target_type, target_id,
                  tenant_id, node_id, outcome, correlation_id, metadata
           FROM audit_logs
           WHERE ($1::text IS NULL OR starts_with(action, $1))
             AND ($2::uuid IS NULL OR actor_id = $2)
             AND ($3::text IS NULL OR outcome = $3)
             AND ($4::text IS NULL OR target_type = $4)
             AND ($5::text IS NULL OR target_id = $5)
             AND ($6::bigint IS NULL OR at >= 'epoch'::timestamptz + $6::bigint * interval '1 microsecond')
             AND ($7::bigint IS NULL OR at <  'epoch'::timestamptz + $7::bigint * interval '1 microsecond')
             AND ($8::bigint IS NULL
                  OR (at, id) < ('epoch'::timestamptz + $8::bigint * interval '1 microsecond', $9::uuid))
           ORDER BY at DESC, id DESC
           LIMIT $10"#,
        f.action,
        f.actor_id,
        f.outcome,
        f.target_type,
        f.target_id,
        f.from,
        f.to,
        cursor_micros,
        cursor_id,
        f.limit + 1,
    )
    .fetch_all(&pool)
    .await
    .map_err(AppError::internal)?;

    let has_more = i64::try_from(rows.len()).is_ok_and(|n| n > f.limit);
    if has_more {
        rows.truncate(usize::try_from(f.limit).unwrap_or(usize::MAX));
    }
    let next_cursor = has_more
        .then(|| rows.last().map(|r| format!("{}.{}", r.at_micros, r.id)))
        .flatten();
    let items = rows
        .into_iter()
        .map(|r| AuditItem {
            id: r.id,
            at: micros_to_rfc3339(r.at_micros),
            actor_id: r.actor_id,
            actor_ip: r.actor_ip,
            action: r.action,
            target_type: r.target_type,
            target_id: r.target_id,
            tenant_id: r.tenant_id,
            node_id: r.node_id,
            outcome: r.outcome,
            correlation_id: r.correlation_id,
            metadata: r.metadata,
        })
        .collect();
    Ok(Json(AuditPage { items, next_cursor }))
}

/// OpenAPI fragment (full paths; merge into the root document).
#[derive(OpenApi)]
#[openapi(
    paths(list),
    components(schemas(AuditItem, AuditPage)),
    tags((name = "audit", description = "Append-only audit log (read only)"))
)]
pub struct AuditApi;

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn q() -> AuditQuery {
        AuditQuery::default()
    }

    fn field_codes(err: AppError) -> Vec<String> {
        match err {
            AppError::Validation { errors } => errors.into_iter().map(|e| e.field).collect(),
            other => panic!("expected validation error, got {other:?}"),
        }
    }

    #[test]
    fn empty_query_uses_defaults() {
        let f = validate(q()).unwrap();
        assert_eq!(f.limit, DEFAULT_LIMIT);
        assert_eq!(
            f,
            Filters {
                limit: DEFAULT_LIMIT,
                ..Filters::default()
            }
        );
        // Empty strings (an unfilled form field) mean "no filter".
        let f = validate(AuditQuery {
            action: Some(String::new()),
            limit: Some(String::new()),
            ..q()
        })
        .unwrap();
        assert_eq!(f.action, None);
    }

    #[test]
    fn valid_filters_are_parsed() {
        let id = Uuid::now_v7();
        let f = validate(AuditQuery {
            action: Some("auth.login_failed".into()),
            actor_id: Some(id.to_string()),
            outcome: Some("denied".into()),
            from: Some("2026-11-11T00:00:00Z".into()),
            to: Some("2026-11-12T00:00:00.5+06:00".into()),
            limit: Some("100".into()),
            cursor: Some(format!("1794400000123456.{id}")),
            ..q()
        })
        .unwrap();
        assert_eq!(f.from, Some(1_794_355_200_000_000));
        assert_eq!(f.to, Some(1_794_441_600_500_000 - 6 * 3_600_000_000));
        assert_eq!(f.limit, 100);
        assert_eq!(f.cursor, Some((1_794_400_000_123_456, id)));
    }

    #[test]
    fn every_bad_field_is_reported_at_once() {
        let err = validate(AuditQuery {
            action: Some("Auth Login".into()),
            actor_id: Some("nope".into()),
            outcome: Some("maybe".into()),
            from: Some("yesterday".into()),
            to: Some("2026-11-11".into()),
            limit: Some("0".into()),
            cursor: Some("garbage".into()),
            target_id: Some("x".repeat(129)),
            ..q()
        })
        .unwrap_err();
        assert_eq!(
            field_codes(err),
            [
                "action",
                "actor_id",
                "outcome",
                "target_id",
                "from",
                "to",
                "limit",
                "cursor"
            ]
        );
        for limit in ["101", "-1", "ten"] {
            assert!(
                validate(AuditQuery {
                    limit: Some(limit.into()),
                    ..q()
                })
                .is_err(),
                "{limit}"
            );
        }
    }

    #[test]
    fn microsecond_timestamps_round_trip() {
        assert_eq!(
            micros_to_rfc3339(1_794_400_000_123_456),
            "2026-11-11T12:26:40.123456Z"
        );
        assert_eq!(micros_to_rfc3339(0), "1970-01-01T00:00:00Z");
    }
}
