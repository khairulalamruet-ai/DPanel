//! Subscriptions and payments (T17.18, T17.19, T17.20).
//!
//! * **Webhook** (`POST /api/v1/webhooks/payments`): the payment provider tells the
//!   authority that a payment arrived, failed, or that a subscription was cancelled. The
//!   body is signed with a shared secret (`X-BuiltPanel-Signature: t=<unix>,v1=<hex>`, the HMAC-SHA256
//!   of `<t>.<body>`), the time must be within five minutes, and every event id is applied
//!   once: a repeat is answered `duplicate` and changes nothing.
//! * **State machine** ([`on_event`], [`on_time`]): `pending -> active -> past_due -> grace ->
//!   suspended -> expired`, and `cancelled`. Pure functions; the database only keeps the
//!   result.
//! * **Renewal and notices** ([`sweep`]): run every few minutes. A subscription whose period
//!   ended without a payment moves on with the clocks of [`Timing`]; the customer's license
//!   follows it (it works through `past_due` and `grace`, is suspended after, and a payment
//!   brings it back). Every change and every renewal reminder is a row in `billing_notices`
//!   for whatever delivers messages to read and mark delivered.
//!
//! A license that an administrator suspended or revoked by hand is never touched by a
//! payment: only a suspension this module made (its note starts with `subscription:`) is lifted.

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use builtpanel_http_kit::{AppError, FieldError};
use secrecy::ExposeSecret as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{Admin, LaState, audit};

const DAY: i64 = 86_400;
/// How far the signed time may be from ours.
const TOLERANCE_SECS: i64 = 300;

/// The routes.
pub fn router() -> Router<LaState> {
    Router::new()
        .route("/api/v1/webhooks/payments", post(webhook))
        .route(
            "/api/v1/admin/subscriptions",
            get(list_subscriptions).post(add_subscription),
        )
        .route("/api/v1/admin/subscriptions/{id}", get(get_subscription))
        .route(
            "/api/v1/admin/subscriptions/{id}/cancel",
            post(cancel_subscription),
        )
        .route("/api/v1/admin/billing/notices", get(list_notices))
        .route(
            "/api/v1/admin/billing/notices/{id}/delivered",
            post(notice_delivered),
        )
        .route("/api/v1/admin/billing/sweep", post(run_sweep))
}

// ------------------------------------------------------------ the state machine

/// Where a subscription is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubState {
    /// Made, nothing paid yet.
    Pending,
    /// Paid up.
    Active,
    /// The period ended (or a payment failed) and nothing arrived.
    PastDue,
    /// Still not paid; the license still works.
    Grace,
    /// Not paid; the license is suspended.
    Suspended,
    /// Ended by the customer; the license works until the paid period ends.
    Cancelled,
    /// Over.
    Expired,
}

impl SubState {
    /// The name kept in the database and shown.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::PastDue => "past_due",
            Self::Grace => "grace",
            Self::Suspended => "suspended",
            Self::Cancelled => "cancelled",
            Self::Expired => "expired",
        }
    }

    /// Reads the name kept in the database.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "pending" => Self::Pending,
            "active" => Self::Active,
            "past_due" => Self::PastDue,
            "grace" => Self::Grace,
            "suspended" => Self::Suspended,
            "cancelled" => Self::Cancelled,
            "expired" => Self::Expired,
            _ => return None,
        })
    }

    fn over(self) -> bool {
        matches!(self, Self::Cancelled | Self::Expired)
    }
}

/// How long each stage of a subscription that is not paid lasts, in days.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// From the end of the period (or a failed payment) to `grace`.
    pub past_due_days: i64,
    /// `grace` to `suspended`.
    pub grace_days: i64,
    /// `suspended` to `expired`.
    pub suspended_days: i64,
    /// How long before the end of a period the customer is reminded.
    pub renewal_notice_days: i64,
    /// What a payment extends the subscription by when the provider gives no end.
    pub period_days: i64,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            past_due_days: 3,
            grace_days: 7,
            suspended_days: 30,
            renewal_notice_days: 7,
            period_days: 30,
        }
    }
}

/// A subscription as the state machine sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
    /// Where it is.
    pub state: SubState,
    /// Since when (Unix seconds).
    pub since: i64,
    /// The end of the paid period (Unix seconds).
    pub period_end: Option<i64>,
}

/// What the provider reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// A payment arrived, paying until this time.
    Paid {
        /// End of the period paid for.
        period_end: i64,
    },
    /// A payment failed.
    PaymentFailed,
    /// The customer ended the subscription.
    Cancel,
}

/// What a notice tells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    /// The first payment arrived.
    Activated,
    /// A later payment arrived.
    Renewed,
    /// A payment failed or did not come.
    PaymentFailed,
    /// The grace period began.
    GraceStarted,
    /// The license was suspended.
    Suspended,
    /// The customer cancelled.
    Cancelled,
    /// The subscription is over.
    Expired,
}

impl Notice {
    fn as_str(self) -> &'static str {
        match self {
            Self::Activated => "activated",
            Self::Renewed => "renewed",
            Self::PaymentFailed => "payment_failed",
            Self::GraceStarted => "grace_started",
            Self::Suspended => "suspended",
            Self::Cancelled => "cancelled",
            Self::Expired => "expired",
        }
    }
}

/// What an event does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// It moves the subscription.
    Applied {
        /// The new state.
        state: SubState,
        /// The paid period's end after it.
        period_end: Option<i64>,
        /// What to tell the customer.
        notice: Notice,
    },
    /// It is old or changes nothing (a payment for a period already paid, a second failure).
    Stale,
    /// The subscription is over; nothing revives it.
    Ignored,
}

/// What an event does to a subscription.
#[must_use]
pub fn on_event(snap: Snapshot, event: Event) -> Outcome {
    if snap.state.over() {
        return Outcome::Ignored;
    }
    match event {
        Event::Paid { period_end } => {
            if snap.state == SubState::Active && snap.period_end.is_some_and(|p| period_end <= p) {
                return Outcome::Stale;
            }
            Outcome::Applied {
                state: SubState::Active,
                period_end: Some(period_end),
                notice: if snap.state == SubState::Pending {
                    Notice::Activated
                } else {
                    Notice::Renewed
                },
            }
        }
        Event::PaymentFailed => {
            if snap.state == SubState::Active {
                Outcome::Applied {
                    state: SubState::PastDue,
                    period_end: snap.period_end,
                    notice: Notice::PaymentFailed,
                }
            } else {
                Outcome::Stale
            }
        }
        Event::Cancel => Outcome::Applied {
            state: SubState::Cancelled,
            period_end: snap.period_end,
            notice: Notice::Cancelled,
        },
    }
}

/// The next move the clock makes, if it is due: the new state, when it was entered (the
/// instant the old stage ran out, not `now`, so a late sweep keeps the clocks honest), and
/// what to tell.
#[must_use]
pub fn on_time(snap: Snapshot, t: &Timing, now: i64) -> Option<(SubState, i64, Notice)> {
    let (state, at, notice) = match snap.state {
        SubState::Active => (SubState::PastDue, snap.period_end?, Notice::PaymentFailed),
        SubState::PastDue => (
            SubState::Grace,
            snap.since + t.past_due_days * DAY,
            Notice::GraceStarted,
        ),
        SubState::Grace => (
            SubState::Suspended,
            snap.since + t.grace_days * DAY,
            Notice::Suspended,
        ),
        SubState::Suspended => (
            SubState::Expired,
            snap.since + t.suspended_days * DAY,
            Notice::Expired,
        ),
        SubState::Cancelled => (SubState::Expired, snap.period_end?, Notice::Expired),
        SubState::Pending | SubState::Expired => return None,
    };
    (at <= now).then_some((state, at, notice))
}

// ------------------------------------------------------------ signatures

/// HMAC-SHA256 (RFC 2104), made from the SHA-256 the authority already has.
#[must_use]
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(k.map(|b| b ^ 0x36));
    inner.update(message);
    let mut outer = Sha256::new();
    outer.update(k.map(|b| b ^ 0x5c));
    outer.update(inner.finalize());
    outer.finalize().into()
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// The header value a provider sends for a body at a time.
#[must_use]
pub fn sign(secret: &[u8], time: i64, body: &[u8]) -> String {
    let mut message = format!("{time}.").into_bytes();
    message.extend_from_slice(body);
    format!("t={time},v1={}", hex(&hmac_sha256(secret, &message)))
}

/// Whether a header is the signature of a body, made at a time close to `now`.
#[must_use]
pub fn verify(secret: &[u8], header: &str, body: &[u8], now: i64) -> bool {
    let mut time = None;
    let mut signatures = Vec::new();
    for part in header.split(',') {
        match part.trim().split_once('=') {
            Some(("t", v)) => time = v.parse::<i64>().ok(),
            Some(("v1", v)) => signatures.push(v.to_owned()),
            _ => {}
        }
    }
    let Some(time) = time else { return false };
    if (now - time).abs() > TOLERANCE_SECS {
        return false;
    }
    let expected = sign(secret, time, body);
    let expected = expected.rsplit("v1=").next().unwrap_or_default();
    // Every candidate is compared in full, without stopping at the first difference.
    signatures.iter().fold(false, |ok, got| {
        let same = got.len() == expected.len()
            && got
                .bytes()
                .zip(expected.bytes())
                .fold(0u8, |d, (a, b)| d | (a ^ b))
                == 0;
        ok | same
    })
}

// ------------------------------------------------------------ the database

fn db(e: sqlx::Error) -> AppError {
    AppError::internal(e)
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

#[derive(FromRow)]
struct Row {
    id: Uuid,
    license_id: Uuid,
    state: String,
    since: i64,
    period_end: Option<i64>,
}

impl Row {
    fn snapshot(&self) -> Result<Snapshot, AppError> {
        Ok(Snapshot {
            state: SubState::parse(&self.state)
                .ok_or_else(|| AppError::internal(format!("unknown state {}", self.state)))?,
            since: self.since,
            period_end: self.period_end,
        })
    }
}

const ROW: &str = "SELECT id, license_id, state, extract(epoch FROM state_since)::bigint AS since,
    extract(epoch FROM current_period_end)::bigint AS period_end FROM subscriptions";

/// Writes a new state, brings the license along and leaves a notice.
async fn commit(
    tx: &mut Transaction<'_, Postgres>,
    row: &Row,
    timing: &Timing,
    new: Snapshot,
    notice: Notice,
    cause: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE subscriptions SET state = $2, state_since = to_timestamp($3::bigint),
             current_period_end = CASE WHEN $4::bigint IS NULL THEN NULL ELSE to_timestamp($4::bigint) END,
             updated_at = now() WHERE id = $1",
    )
    .bind(row.id)
    .bind(new.state.as_str())
    .bind(new.since)
    .bind(new.period_end)
    .execute(&mut **tx)
    .await
    .map_err(db)?;
    follow_license(tx, row.license_id, new, timing).await?;
    sqlx::query(
        "INSERT INTO billing_notices (subscription_id, license_id, kind, detail) VALUES ($1, $2, $3, $4)",
    )
    .bind(row.id)
    .bind(row.license_id)
    .bind(notice.as_str())
    .bind(serde_json::json!({
        "state": new.state.as_str(),
        "period_end": new.period_end,
        "cause": cause,
    }))
    .execute(&mut **tx)
    .await
    .map_err(db)?;
    audit(
        &mut **tx,
        "billing",
        "subscription.changed",
        Some(&row.id.to_string()),
        serde_json::json!({ "from": row.state, "to": new.state.as_str(), "cause": cause }),
    )
    .await
}

/// What the license does when the subscription is in this state.
async fn follow_license(
    tx: &mut Transaction<'_, Postgres>,
    license: Uuid,
    new: Snapshot,
    t: &Timing,
) -> Result<(), AppError> {
    match new.state {
        SubState::Active | SubState::PastDue | SubState::Grace => {
            let Some(end) = new.period_end else {
                return Ok(());
            };
            // It works through the unpaid stages; only what this module suspended is lifted.
            sqlx::query(
                "UPDATE licenses SET status = 'active', status_note = NULL,
                     expires_at = to_timestamp($2::bigint), updated_at = now()
                 WHERE id = $1 AND (status = 'active'
                     OR (status = 'suspended' AND status_note LIKE 'subscription:%'))",
            )
            .bind(license)
            .bind(end + (t.past_due_days + t.grace_days) * DAY)
            .execute(&mut **tx)
            .await
            .map_err(db)?;
        }
        SubState::Suspended | SubState::Expired => {
            sqlx::query(
                "UPDATE licenses SET status = 'suspended', status_note = $2, updated_at = now()
                 WHERE id = $1 AND status = 'active'",
            )
            .bind(license)
            .bind(format!("subscription: {}", new.state.as_str()))
            .execute(&mut **tx)
            .await
            .map_err(db)?;
        }
        SubState::Cancelled => {
            if let Some(end) = new.period_end {
                sqlx::query(
                    "UPDATE licenses SET expires_at = to_timestamp($2::bigint), updated_at = now()
                     WHERE id = $1 AND status <> 'revoked'",
                )
                .bind(license)
                .bind(end)
                .execute(&mut **tx)
                .await
                .map_err(db)?;
            }
        }
        SubState::Pending => {}
    }
    Ok(())
}

// ------------------------------------------------------------ the webhook

#[derive(Deserialize)]
struct Incoming {
    /// The provider's id of the event.
    id: String,
    /// `payment.succeeded`, `payment.failed` or `subscription.cancelled`.
    #[serde(rename = "type")]
    kind: String,
    /// The subscription's reference.
    reference: String,
    /// The end of the period a payment paid for (Unix seconds).
    period_end: Option<i64>,
}

#[derive(Serialize)]
struct Answer {
    outcome: &'static str,
    state: Option<&'static str>,
}

async fn webhook(
    State(state): State<LaState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Answer>, AppError> {
    let Some(secret) = &state.settings.payment_webhook_secret else {
        return Err(AppError::unavailable(
            "webhook_not_configured",
            "Payments are not set up on this authority.",
        ));
    };
    let now = unix_now();
    let signed = headers
        .get("x-builtpanel-signature")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|h| verify(secret.expose_secret().as_bytes(), h, &body, now));
    if !signed {
        return Err(AppError::authentication_failed(
            "bad_signature",
            "The signature is missing, wrong or too old.",
        ));
    }
    let event: Incoming = serde_json::from_slice(&body).map_err(|_| {
        AppError::validation(vec![FieldError::new(
            "body",
            "invalid",
            "The event is not understood.",
        )])
    })?;
    if event.id.is_empty() || event.id.len() > 200 || event.reference.len() > 200 {
        return Err(AppError::validation(vec![FieldError::new(
            "id",
            "invalid",
            "The event id or the reference is not valid.",
        )]));
    }
    let timing = state.settings.billing;
    if let Some(end) = event.period_end
        && !(now - DAY..=now + 3660 * DAY).contains(&end)
    {
        return Err(AppError::validation(vec![FieldError::new(
            "period_end",
            "invalid",
            "The period ends at an impossible time.",
        )]));
    }
    let mut tx = state.db.begin().await.map_err(db)?;
    // Once: whoever sends the same id again is told it was already taken.
    let first = sqlx::query(
        "INSERT INTO payment_events (event_id, kind, outcome) VALUES ($1, $2, 'received')
         ON CONFLICT DO NOTHING",
    )
    .bind(&event.id)
    .bind(&event.kind)
    .execute(&mut *tx)
    .await
    .map_err(db)?
    .rows_affected();
    if first == 0 {
        return Ok(Json(Answer {
            outcome: "duplicate",
            state: None,
        }));
    }
    let row: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{ROW} WHERE provider_reference = $1 FOR UPDATE"
    )))
    .bind(&event.reference)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db)?;
    let finish = |outcome: &'static str, subscription: Option<Uuid>| {
        sqlx::query(
            "UPDATE payment_events SET outcome = $2, subscription_id = $3 WHERE event_id = $1",
        )
        .bind(event.id.clone())
        .bind(outcome)
        .bind(subscription)
    };
    let Some(row) = row else {
        finish("unknown_subscription", None)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        tx.commit().await.map_err(db)?;
        return Ok(Json(Answer {
            outcome: "unknown_subscription",
            state: None,
        }));
    };
    let snap = row.snapshot()?;
    let wanted = match event.kind.as_str() {
        "payment.succeeded" => Some(Event::Paid {
            period_end: event.period_end.unwrap_or_else(|| {
                snap.period_end.unwrap_or(0).max(now) + timing.period_days * DAY
            }),
        }),
        "payment.failed" => Some(Event::PaymentFailed),
        "subscription.cancelled" => Some(Event::Cancel),
        _ => None,
    };
    let (outcome, new_state) = match wanted.map(|e| on_event(snap, e)) {
        None => ("ignored_type", None),
        Some(Outcome::Stale) => ("stale", Some(snap.state)),
        Some(Outcome::Ignored) => ("ignored_over", Some(snap.state)),
        Some(Outcome::Applied {
            state: new,
            period_end,
            notice,
        }) => {
            commit(
                &mut tx,
                &row,
                &timing,
                Snapshot {
                    state: new,
                    since: now,
                    period_end,
                },
                notice,
                &event.kind,
            )
            .await?;
            ("applied", Some(new))
        }
    };
    finish(outcome, Some(row.id))
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    tx.commit().await.map_err(db)?;
    Ok(Json(Answer {
        outcome,
        state: new_state.map(SubState::as_str),
    }))
}

// ------------------------------------------------------------ renewal and notices

/// What a [`sweep`] did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SweepReport {
    /// Subscriptions that moved to another state.
    pub moved: u32,
    /// Renewal reminders made.
    pub reminded: u32,
}

/// Moves on every subscription whose clock has run out and reminds those that will renew soon.
/// Safe to run at any time, as often as wanted, and from more than one place at once.
///
/// # Errors
/// A database error.
pub async fn sweep(db_pool: &PgPool, timing: &Timing, now: i64) -> Result<SweepReport, AppError> {
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM subscriptions WHERE state NOT IN ('pending', 'expired') ORDER BY id",
    )
    .fetch_all(db_pool)
    .await
    .map_err(db)?;
    let mut report = SweepReport::default();
    for id in ids {
        let mut tx = db_pool.begin().await.map_err(db)?;
        let Some(mut row) = sqlx::query_as::<_, Row>(sqlx::AssertSqlSafe(format!(
            "{ROW} WHERE id = $1 FOR UPDATE"
        )))
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?
        else {
            continue;
        };
        let mut snap = row.snapshot()?;
        let mut moved = false;
        // A sweep that comes late catches up one stage at a time, each with its own date.
        while let Some((state, at, notice)) = on_time(snap, timing, now) {
            let next = Snapshot {
                state,
                since: at,
                period_end: snap.period_end,
            };
            commit(&mut tx, &row, timing, next, notice, "time").await?;
            row.state = state.as_str().to_owned();
            snap = next;
            moved = true;
        }
        if moved {
            report.moved += 1;
        }
        if snap.state == SubState::Active
            && let Some(end) = snap.period_end
            && end > now
            && end - now <= timing.renewal_notice_days * DAY
        {
            let made = sqlx::query(
                "INSERT INTO billing_notices (subscription_id, license_id, kind, dedupe_key, detail)
                 VALUES ($1, $2, 'renewal_upcoming', $3, $4) ON CONFLICT DO NOTHING",
            )
            .bind(row.id)
            .bind(row.license_id)
            .bind(format!("renewal_upcoming:{end}"))
            .bind(serde_json::json!({ "period_end": end }))
            .execute(&mut *tx)
            .await
            .map_err(db)?
            .rows_affected();
            report.reminded += u32::try_from(made).unwrap_or(0);
        }
        tx.commit().await.map_err(db)?;
    }
    Ok(report)
}

/// Runs [`sweep`] every few minutes until the program ends.
pub async fn run_forever(state: LaState) {
    let mut every = tokio::time::interval(std::time::Duration::from_secs(300));
    loop {
        every.tick().await;
        match sweep(&state.db, &state.settings.billing, unix_now()).await {
            Ok(r) if r.moved + r.reminded > 0 => {
                tracing::info!(moved = r.moved, reminded = r.reminded, "billing sweep");
            }
            Ok(_) => {}
            Err(err) => tracing::warn!(error = %err.code(), "billing sweep failed"),
        }
    }
}

// ------------------------------------------------------------ administrators

/// A subscription.
#[derive(Serialize, FromRow)]
pub struct Subscription {
    /// Id.
    pub id: Uuid,
    /// The license it pays for.
    pub license_id: Uuid,
    /// Where it is.
    pub state: String,
    /// The payment provider's name.
    pub provider: Option<String>,
    /// What the provider calls it (what webhooks name).
    pub provider_reference: Option<String>,
    /// End of the paid period (Unix seconds).
    pub current_period_end: Option<i64>,
    /// In this state since (Unix seconds).
    pub state_since: i64,
}

const SUBSCRIPTION: &str = "SELECT id, license_id, state, provider, provider_reference,
    extract(epoch FROM current_period_end)::bigint AS current_period_end,
    extract(epoch FROM state_since)::bigint AS state_since FROM subscriptions";

/// What an administrator gives to start one.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewSubscription {
    /// The license.
    pub license_id: Uuid,
    /// The provider's name.
    pub provider: String,
    /// The provider's reference for it (at most 200 characters).
    pub reference: String,
}

async fn add_subscription(
    State(state): State<LaState>,
    admin: Admin,
    Json(req): Json<NewSubscription>,
) -> Result<(StatusCode, Json<Subscription>), AppError> {
    admin.may_change()?;
    if req.provider.trim().is_empty()
        || req.provider.len() > 40
        || req.reference.trim().is_empty()
        || req.reference.len() > 200
    {
        return Err(AppError::validation(vec![FieldError::new(
            "reference",
            "invalid",
            "The provider is 1 to 40 characters and the reference 1 to 200.",
        )]));
    }
    let status: Option<String> = sqlx::query_scalar("SELECT status FROM licenses WHERE id = $1")
        .bind(req.license_id)
        .fetch_optional(&state.db)
        .await
        .map_err(db)?;
    match status.as_deref() {
        None => return Err(AppError::not_found("license")),
        Some("revoked") => {
            return Err(AppError::conflict(
                "revoked",
                "A revoked license cannot have a subscription.",
            ));
        }
        Some(_) => {}
    }
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO subscriptions (license_id, provider, provider_reference) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(req.license_id)
    .bind(&req.provider)
    .bind(&req.reference)
    .fetch_one(&state.db)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(d) if d.is_unique_violation() => AppError::conflict(
            "already_subscribed",
            "The license already has a subscription, or the reference is taken.",
        ),
        other => db(other),
    })?;
    audit(
        &state.db,
        &admin.email,
        "subscription.created",
        Some(&id.to_string()),
        serde_json::json!({ "license_id": req.license_id, "provider": req.provider }),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(fetch(&state.db, id).await?)))
}

async fn fetch(pool: &PgPool, id: Uuid) -> Result<Subscription, AppError> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!("{SUBSCRIPTION} WHERE id = $1")))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(db)?
        .ok_or_else(|| AppError::not_found("subscription"))
}

/// A filter.
#[derive(Deserialize)]
pub struct ListQuery {
    /// Only this state.
    pub state: Option<String>,
    /// Only this license.
    pub license_id: Option<Uuid>,
}

async fn list_subscriptions(
    State(state): State<LaState>,
    _admin: Admin,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<Subscription>>, AppError> {
    let rows = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{SUBSCRIPTION} WHERE ($1::text IS NULL OR state = $1) AND ($2::uuid IS NULL OR license_id = $2)
         ORDER BY created_at DESC LIMIT 500"
    )))
    .bind(q.state)
    .bind(q.license_id)
    .fetch_all(&state.db)
    .await
    .map_err(db)?;
    Ok(Json(rows))
}

async fn get_subscription(
    State(state): State<LaState>,
    _admin: Admin,
    Path(id): Path<Uuid>,
) -> Result<Json<Subscription>, AppError> {
    Ok(Json(fetch(&state.db, id).await?))
}

async fn cancel_subscription(
    State(state): State<LaState>,
    admin: Admin,
    Path(id): Path<Uuid>,
) -> Result<Json<Subscription>, AppError> {
    admin.may_change()?;
    let mut tx = state.db.begin().await.map_err(db)?;
    let row: Row = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{ROW} WHERE id = $1 FOR UPDATE"
    )))
    .bind(id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db)?
    .ok_or_else(|| AppError::not_found("subscription"))?;
    match on_event(row.snapshot()?, Event::Cancel) {
        Outcome::Applied {
            state: new,
            period_end,
            notice,
        } => {
            commit(
                &mut tx,
                &row,
                &state.settings.billing,
                Snapshot {
                    state: new,
                    since: unix_now(),
                    period_end,
                },
                notice,
                "admin",
            )
            .await?;
        }
        _ => {
            return Err(AppError::conflict(
                "already_over",
                "The subscription is already over.",
            ));
        }
    }
    tx.commit().await.map_err(db)?;
    Ok(Json(fetch(&state.db, id).await?))
}

/// Something to tell a customer.
#[derive(Serialize, FromRow)]
pub struct NoticeView {
    /// Id.
    pub id: Uuid,
    /// The subscription.
    pub subscription_id: Uuid,
    /// The license.
    pub license_id: Uuid,
    /// What happened.
    pub kind: String,
    /// The details.
    pub detail: serde_json::Value,
    /// When it was made (Unix seconds).
    pub created_at: i64,
    /// When it was delivered (Unix seconds).
    pub delivered_at: Option<i64>,
}

/// Which notices.
#[derive(Deserialize)]
pub struct NoticeQuery {
    /// Only those not delivered yet (the default).
    pub pending: Option<bool>,
}

async fn list_notices(
    State(state): State<LaState>,
    _admin: Admin,
    Query(q): Query<NoticeQuery>,
) -> Result<Json<Vec<NoticeView>>, AppError> {
    let rows = sqlx::query_as(
        "SELECT id, subscription_id, license_id, kind, detail,
                extract(epoch FROM created_at)::bigint AS created_at,
                extract(epoch FROM delivered_at)::bigint AS delivered_at
         FROM billing_notices WHERE ($1 = false OR delivered_at IS NULL)
         ORDER BY created_at LIMIT 500",
    )
    .bind(q.pending.unwrap_or(true))
    .fetch_all(&state.db)
    .await
    .map_err(db)?;
    Ok(Json(rows))
}

async fn notice_delivered(
    State(state): State<LaState>,
    admin: Admin,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    admin.may_change()?;
    let done = sqlx::query(
        "UPDATE billing_notices SET delivered_at = now() WHERE id = $1 AND delivered_at IS NULL",
    )
    .bind(id)
    .execute(&state.db)
    .await
    .map_err(db)?
    .rows_affected();
    if done == 0 {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM billing_notices WHERE id = $1)")
                .bind(id)
                .fetch_one(&state.db)
                .await
                .map_err(db)?;
        if !exists {
            return Err(AppError::not_found("notice"));
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn run_sweep(
    State(state): State<LaState>,
    admin: Admin,
) -> Result<Json<SweepReport>, AppError> {
    admin.may_change()?;
    Ok(Json(
        sweep(&state.db, &state.settings.billing, unix_now()).await?,
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn snap(state: SubState, since: i64, period_end: Option<i64>) -> Snapshot {
        Snapshot {
            state,
            since,
            period_end,
        }
    }

    #[test]
    fn hmac_matches_the_published_vectors() {
        // RFC 4231, test cases 1 and 6 (a key longer than the block).
        assert_eq!(
            hex(&hmac_sha256(&[0x0b; 20], b"Hi There")),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        assert_eq!(
            hex(&hmac_sha256(
                &[0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn a_signature_is_checked_for_body_secret_and_time() {
        let (secret, body) = (
            b"a-shared-secret-1234".as_slice(),
            b"{\"id\":\"e1\"}".as_slice(),
        );
        let header = sign(secret, 1_000, body);
        assert!(verify(secret, &header, body, 1_000));
        assert!(verify(secret, &header, body, 1_000 + 299));
        assert!(!verify(secret, &header, body, 1_000 + 301), "too old");
        assert!(
            !verify(secret, &header, body, 1_000 - 301),
            "from the future"
        );
        assert!(
            !verify(secret, &header, b"{\"id\":\"e2\"}", 1_000),
            "other body"
        );
        assert!(!verify(b"another-secret-12345", &header, body, 1_000));
        assert!(!verify(secret, "", body, 1_000));
        assert!(!verify(secret, "t=1000", body, 1_000));
        assert!(!verify(secret, "t=1000,v1=00", body, 1_000));
        // Two signatures (a secret being changed): either may match.
        let rotated = format!("{header},v1=deadbeef");
        assert!(verify(secret, &rotated, body, 1_000));
    }

    #[test]
    fn payments_move_a_subscription_and_a_second_payment_for_the_same_period_changes_nothing() {
        let pending = snap(SubState::Pending, 0, None);
        let Outcome::Applied {
            state,
            period_end,
            notice,
        } = on_event(pending, Event::Paid { period_end: 100 })
        else {
            panic!("applied")
        };
        assert_eq!(
            (state, period_end, notice),
            (SubState::Active, Some(100), Notice::Activated)
        );
        let active = snap(SubState::Active, 0, Some(100));
        assert_eq!(
            on_event(active, Event::Paid { period_end: 100 }),
            Outcome::Stale
        );
        assert_eq!(
            on_event(active, Event::Paid { period_end: 50 }),
            Outcome::Stale
        );
        assert!(matches!(
            on_event(active, Event::Paid { period_end: 200 }),
            Outcome::Applied {
                notice: Notice::Renewed,
                period_end: Some(200),
                ..
            }
        ));
        // Every unpaid stage comes back to active with a payment.
        for s in [SubState::PastDue, SubState::Grace, SubState::Suspended] {
            assert!(matches!(
                on_event(snap(s, 0, Some(100)), Event::Paid { period_end: 300 }),
                Outcome::Applied {
                    state: SubState::Active,
                    notice: Notice::Renewed,
                    ..
                }
            ));
        }
        // Nothing revives a subscription that is over.
        for s in [SubState::Cancelled, SubState::Expired] {
            for e in [
                Event::Paid { period_end: 300 },
                Event::PaymentFailed,
                Event::Cancel,
            ] {
                assert_eq!(on_event(snap(s, 0, Some(100)), e), Outcome::Ignored);
            }
        }
    }

    #[test]
    fn a_failed_payment_and_a_cancellation() {
        let active = snap(SubState::Active, 0, Some(100));
        assert!(matches!(
            on_event(active, Event::PaymentFailed),
            Outcome::Applied {
                state: SubState::PastDue,
                notice: Notice::PaymentFailed,
                period_end: Some(100)
            }
        ));
        for s in [
            SubState::Pending,
            SubState::PastDue,
            SubState::Grace,
            SubState::Suspended,
        ] {
            assert_eq!(
                on_event(snap(s, 0, Some(1)), Event::PaymentFailed),
                Outcome::Stale
            );
        }
        for s in [
            SubState::Pending,
            SubState::Active,
            SubState::PastDue,
            SubState::Grace,
            SubState::Suspended,
        ] {
            assert!(matches!(
                on_event(snap(s, 0, Some(1)), Event::Cancel),
                Outcome::Applied {
                    state: SubState::Cancelled,
                    notice: Notice::Cancelled,
                    period_end: Some(1)
                }
            ));
        }
    }

    #[test]
    fn the_clock_walks_the_stages_with_their_own_dates() {
        let t = Timing::default();
        let end = 1_000_000;
        let active = snap(SubState::Active, 0, Some(end));
        assert_eq!(
            on_time(active, &t, end - 1),
            None,
            "not before the period ends"
        );
        let (s, at, n) = on_time(active, &t, end).unwrap();
        assert_eq!((s, at, n), (SubState::PastDue, end, Notice::PaymentFailed));
        let past_due = snap(s, at, Some(end));
        assert_eq!(on_time(past_due, &t, at + 3 * DAY - 1), None);
        let (s, at2, n) = on_time(past_due, &t, at + 3 * DAY).unwrap();
        assert_eq!(
            (s, at2, n),
            (SubState::Grace, end + 3 * DAY, Notice::GraceStarted)
        );
        let (s, at3, n) = on_time(snap(s, at2, Some(end)), &t, at2 + 7 * DAY).unwrap();
        assert_eq!(
            (s, at3, n),
            (SubState::Suspended, end + 10 * DAY, Notice::Suspended)
        );
        let (s, at4, n) = on_time(snap(s, at3, Some(end)), &t, at3 + 30 * DAY).unwrap();
        assert_eq!(
            (s, at4, n),
            (SubState::Expired, end + 40 * DAY, Notice::Expired)
        );
        assert_eq!(on_time(snap(s, at4, Some(end)), &t, i64::MAX), None, "over");
        // Cancelled: over when the paid period is.
        let cancelled = snap(SubState::Cancelled, 5, Some(end));
        assert_eq!(on_time(cancelled, &t, end - 1), None);
        assert_eq!(on_time(cancelled, &t, end).unwrap().0, SubState::Expired);
        // Nothing paid yet: no clock.
        assert_eq!(
            on_time(snap(SubState::Pending, 0, None), &t, i64::MAX),
            None
        );
    }

    #[test]
    fn states_round_trip_through_their_names() {
        for s in [
            SubState::Pending,
            SubState::Active,
            SubState::PastDue,
            SubState::Grace,
            SubState::Suspended,
            SubState::Cancelled,
            SubState::Expired,
        ] {
            assert_eq!(SubState::parse(s.as_str()), Some(s));
        }
        assert_eq!(SubState::parse("nonsense"), None);
    }
}
