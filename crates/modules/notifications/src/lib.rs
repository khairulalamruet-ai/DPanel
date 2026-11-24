//! Notifications (T18.03, T18.04).
//!
//! An event that matters (a certificate that ends, a backup that failed, a node that is offline,
//! a node's service that is down) becomes a notification in the inbox of the people it concerns,
//! worded in their language with the panel's own name. Each account can add channels (an email
//! address, a webhook) and say which events go to which; pausing or snoozing stops everything
//! outside the inbox. A channel's messages are queued and sent by [`deliver_due`], which the
//! panel runs every half minute and which tries again, with growing waits, when a try fails.
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET | `/api/v1/{client,admin}/notifications` | the signed-in account |
//! | POST | `/api/v1/{client,admin}/notifications/read` | the signed-in account |
//! | GET, PUT | `/api/v1/{client,admin}/notifications/quiet` | the signed-in account |
//! | GET | `/api/v1/{client,admin}/notifications/events` | the signed-in account |
//! | GET, POST | `/api/v1/{client,admin}/notifications/channels` | the signed-in account |
//! | PATCH, DELETE | `/api/v1/{client,admin}/notifications/channels/{id}` | the signed-in account |
//! | GET, PUT | `/api/v1/admin/notifications/smtp` | `admin.settings.read`, `admin.settings.manage` |
//! | POST | `/api/v1/admin/notifications/smtp/test` | `admin.settings.manage` |

pub mod send;
pub mod templates;

use std::sync::{Arc, OnceLock};

use axum::{
    Json, Router,
    extract::{FromRef, Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use builtpanel_auth_core::secretbox::MasterKey;
use builtpanel_database::{
    accounts::DbAccountKind,
    notifications::{self as store, NewNotification, Quiet},
    sqlx::PgPool,
};
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_kernel::{Event, module::EventError};
use builtpanel_rbac::{
    Principal, Require,
    permissions::{SettingsManage, SettingsRead},
};
use builtpanel_session::SessionStore;
use builtpanel_settings::{Scope, SettingsRegistry};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, OpenApi, ToSchema};
use uuid::Uuid;

/// The module's manifest.
pub const MANIFEST: &str = include_str!("../../../../modules/notifications/module.toml");

/// How often a delivery is tried before it is given up.
pub const MAX_ATTEMPTS: i32 = 5;

/// What the module's handlers share.
#[derive(Clone)]
pub struct NotificationsState {
    /// Database pool.
    pub db: PgPool,
    /// Session store.
    pub sessions: SessionStore,
    /// The checked settings of all modules (for the panel's name), set once they are known.
    pub settings: Arc<OnceLock<Arc<SettingsRegistry>>>,
    /// Seals the webhook secrets and the mail server's password.
    pub master_key: MasterKey,
}

impl FromRef<NotificationsState> for PgPool {
    fn from_ref(s: &NotificationsState) -> Self {
        s.db.clone()
    }
}

impl FromRef<NotificationsState> for SessionStore {
    fn from_ref(s: &NotificationsState) -> Self {
        s.sessions.clone()
    }
}

fn internal(err: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>) -> AppError {
    AppError::internal(err)
}

fn bad(field: &'static str, code: &'static str, message: &str) -> AppError {
    AppError::validation(vec![FieldError::new(field, code, message)])
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// The panel's name (its branding), or `Built Panel` when it cannot be read.
async fn panel_name(state: &NotificationsState) -> String {
    let fallback = || "Built Panel".to_owned();
    let Some(registry) = state.settings.get() else {
        return fallback();
    };
    let Ok(mut conn) = state.db.acquire().await else {
        return fallback();
    };
    builtpanel_settings::get(registry, &mut conn, "system", Scope::Global)
        .await
        .ok()
        .and_then(|values| {
            values
                .get("panel_name")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(fallback)
}

// ------------------------------------------------------------------------ events

/// Turns an event into notifications: one for each person it concerns, queued for their channels
/// unless they paused or snoozed. Safe to repeat: an event is never turned into the same
/// notification twice.
///
/// # Errors
/// [`EventError`] when the database fails (the event is then delivered again).
pub async fn handle_event(state: &NotificationsState, event: &Event) -> Result<(), EventError> {
    let fail = |e: builtpanel_database::sqlx::Error| EventError(e.to_string());
    let panel = panel_name(state).await;
    let node_name = match event {
        Event::NodeServiceDown { node_id, .. } | Event::NodeServiceRecovered { node_id, .. } => {
            store::node_name(&state.db, *node_id).await.map_err(fail)?
        }
        Event::NodeOffline { name, .. } => Some(name.clone()),
        _ => None,
    };
    // Who it concerns does not depend on the language: ask with English.
    let probe = templates::Context {
        locale: "en",
        panel: &panel,
        node: node_name.as_deref(),
        now: now(),
    };
    let Some((audience, _, _)) = templates::render(event, &probe) else {
        return Ok(());
    };
    let recipients = match audience {
        templates::Audience::Admins => store::admins(&state.db).await.map_err(fail)?,
        templates::Audience::TenantOwner(tenant) => store::tenant_owner(&state.db, tenant)
            .await
            .map_err(fail)?
            .into_iter()
            .collect(),
    };
    for recipient in recipients {
        let ctx = templates::Context {
            locale: &recipient.locale,
            ..probe
        };
        let Some((_, key, rendered)) = templates::render(event, &ctx) else {
            continue;
        };
        let event_type = event.type_name();
        let mut tx = state.db.begin().await.map_err(fail)?;
        let added = store::add(
            &mut tx,
            &NewNotification {
                account_id: recipient.account_id,
                event_type,
                severity: rendered.severity,
                subject: &rendered.subject,
                body: &rendered.body,
                data: &rendered.data,
                dedupe_key: &key,
            },
        )
        .await
        .map_err(fail)?;
        if let Some(id) = added
            && !store::is_quiet(&mut tx, recipient.account_id)
                .await
                .map_err(fail)?
        {
            for channel in store::channels_for(&mut tx, recipient.account_id, event_type)
                .await
                .map_err(fail)?
            {
                store::enqueue(&mut tx, id, channel).await.map_err(fail)?;
            }
        }
        tx.commit().await.map_err(fail)?;
    }
    Ok(())
}

// --------------------------------------------------------------------- delivery

fn webhook_context(account: Uuid) -> Vec<u8> {
    format!("notification-channel:{account}").into_bytes()
}

const SMTP_CONTEXT: &[u8] = b"notification-smtp";

async fn mail_server(state: &NotificationsState) -> Result<Option<send::MailServer>, String> {
    let Some(smtp) = store::smtp(&state.db).await.map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    if !smtp.enabled {
        return Ok(None);
    }
    let password = match &smtp.sealed_password {
        Some(sealed) => Some(
            String::from_utf8(
                state
                    .master_key
                    .open(SMTP_CONTEXT, sealed)
                    .map_err(|_| "the mail server's password cannot be opened".to_owned())?,
            )
            .map_err(|_| "the mail server's password is not text".to_owned())?,
        ),
        None => None,
    };
    Ok(Some(send::MailServer {
        host: smtp.host,
        port: u16::try_from(smtp.port)
            .map_err(|_| "the mail server's port is not valid".to_owned())?,
        security: smtp.security,
        username: smtp.username,
        password,
        from_address: smtp.from_address,
        from_name: smtp.from_name,
    }))
}

/// Sends what is due to the channels. Returns how many deliveries were taken (sent or not).
pub async fn deliver_due(state: &NotificationsState) -> usize {
    let due = match store::claim_due(&state.db, 20).await {
        Ok(due) => due,
        Err(error) => {
            tracing::warn!(%error, "could not read the notifications that are due");
            return 0;
        }
    };
    let taken = due.len();
    let panel = panel_name(state).await;
    for item in due {
        let result = deliver_one(state, &item, &panel).await;
        let marked = match result {
            Ok(()) => store::mark_sent(&state.db, item.id).await,
            Err(reason) => {
                // 1, 2, 4, 8 minutes, then it is `failed`.
                let wait = 60 * (1 << (item.attempts.clamp(1, 6) - 1));
                tracing::info!(delivery = %item.id, %reason, "a notification could not be sent");
                store::mark_failed(&state.db, item.id, &reason, wait, MAX_ATTEMPTS).await
            }
        };
        if let Err(error) = marked {
            tracing::warn!(%error, "could not note how a notification went");
        }
    }
    taken
}

async fn deliver_one(
    state: &NotificationsState,
    item: &store::Due,
    panel: &str,
) -> Result<(), String> {
    match item.kind.as_str() {
        "email" => {
            let server = mail_server(state)
                .await?
                .ok_or_else(|| "no mail server is set up".to_owned())?;
            let (text, html) =
                templates::mail(&item.subject, &item.body, panel, item.locale == "bn");
            let to = item.target.clone();
            let subject = item.subject.clone();
            tokio::task::spawn_blocking(move || {
                send::send_mail(&server, &to, &subject, &text, &html)
            })
            .await
            .map_err(|_| "the sending was cancelled".to_owned())?
        }
        "webhook" => {
            let secret = match &item.sealed_secret {
                Some(sealed) => state
                    .master_key
                    .open(&webhook_context(item.account_id), sealed)
                    .map_err(|_| "the webhook's secret cannot be opened".to_owned())?,
                None => return Err("the webhook has no secret".to_owned()),
            };
            let body = serde_json::json!({
                "id": item.notification_id,
                "event": item.event_type,
                "severity": item.severity,
                "subject": item.subject,
                "body": item.body,
                "data": item.data,
                "created_at": item.created_at,
            })
            .to_string();
            let (url, event) = (item.target.clone(), item.event_type.clone());
            tokio::task::spawn_blocking(move || send::post_webhook(&url, &secret, &event, &body))
                .await
                .map_err(|_| "the sending was cancelled".to_owned())?
        }
        other => Err(format!("unknown channel kind {other}")),
    }
}

// ------------------------------------------------------------------------ routes

/// Routes for hosting accounts.
pub fn client_router() -> Router<NotificationsState> {
    inbox_routes("/api/v1/client")
}

/// Routes for administrators.
pub fn admin_router() -> Router<NotificationsState> {
    inbox_routes("/api/v1/admin")
        .route(
            "/api/v1/admin/notifications/smtp",
            get(get_smtp).put(put_smtp),
        )
        .route("/api/v1/admin/notifications/smtp/test", post(test_smtp))
}

fn inbox_routes(prefix: &str) -> Router<NotificationsState> {
    Router::new()
        .route(&format!("{prefix}/notifications"), get(list))
        .route(&format!("{prefix}/notifications/read"), post(mark_read))
        .route(
            &format!("{prefix}/notifications/quiet"),
            get(get_quiet).put(put_quiet),
        )
        .route(&format!("{prefix}/notifications/events"), get(events))
        .route(
            &format!("{prefix}/notifications/channels"),
            get(channels).post(add_channel),
        )
        .route(
            &format!("{prefix}/notifications/channels/{{id}}"),
            axum::routing::patch(update_channel).delete(delete_channel),
        )
}

/// Which notifications.
#[derive(Debug, Deserialize, IntoParams)]
pub struct ListQuery {
    /// Only the ones not read.
    pub unread: Option<bool>,
    /// How many (1 to 100, default 30).
    pub limit: Option<i64>,
}

/// A notification.
#[derive(Debug, Serialize, ToSchema)]
pub struct NotificationView {
    /// Id.
    pub id: Uuid,
    /// The event it comes from.
    pub event_type: String,
    /// `info`, `warning` or `critical`.
    pub severity: String,
    /// One line.
    pub subject: String,
    /// The text.
    pub body: String,
    /// The ids it is about.
    pub data: serde_json::Value,
    /// When (Unix seconds).
    pub created_at: i64,
    /// Whether it was read.
    pub read: bool,
}

/// The inbox.
#[derive(Debug, Serialize, ToSchema)]
pub struct NotificationList {
    /// How many are not read (all of them, not only these).
    pub unread: i64,
    /// The newest first.
    pub items: Vec<NotificationView>,
}

/// The newest notifications of the signed-in account.
#[utoipa::path(
    get,
    path = "/api/v1/client/notifications",
    tag = "notifications",
    operation_id = "listNotifications",
    security(("session" = [])),
    params(ListQuery),
    responses(
        (status = 200, description = "The inbox", body = NotificationList),
        (status = 422, description = "A limit that is not usable", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn list(
    State(state): State<NotificationsState>,
    who: Principal,
    Query(q): Query<ListQuery>,
) -> Result<Json<NotificationList>, AppError> {
    let limit = q.limit.unwrap_or(30);
    if !(1..=100).contains(&limit) {
        return Err(bad("limit", "out_of_range", "Between 1 and 100."));
    }
    let account = who.account_id();
    let items = store::list(&state.db, account, q.unread.unwrap_or(false), limit)
        .await
        .map_err(internal)?
        .into_iter()
        .map(|n| NotificationView {
            id: n.id,
            event_type: n.event_type,
            severity: n.severity,
            subject: n.subject,
            body: n.body,
            data: n.data,
            created_at: n.created_at,
            read: n.read_at.is_some(),
        })
        .collect();
    let unread = store::unread_count(&state.db, account)
        .await
        .map_err(internal)?;
    Ok(Json(NotificationList { unread, items }))
}

/// Which notifications to mark read.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadRequest {
    /// These ones; all of them when it is left out.
    pub ids: Option<Vec<Uuid>>,
}

/// How many changed.
#[derive(Debug, Serialize, ToSchema)]
pub struct Updated {
    /// How many.
    pub updated: u64,
}

/// Marks notifications as read.
#[utoipa::path(
    post,
    path = "/api/v1/client/notifications/read",
    tag = "notifications",
    operation_id = "readNotifications",
    security(("session" = [])),
    request_body = ReadRequest,
    responses((status = 200, description = "Done", body = Updated))
)]
pub async fn mark_read(
    State(state): State<NotificationsState>,
    who: Principal,
    Json(req): Json<ReadRequest>,
) -> Result<Json<Updated>, AppError> {
    if req.ids.as_ref().is_some_and(|ids| ids.len() > 500) {
        return Err(bad("ids", "too_many", "At most 500 at a time."));
    }
    let updated = store::mark_read(&state.db, who.account_id(), req.ids.as_deref())
        .await
        .map_err(internal)?;
    Ok(Json(Updated { updated }))
}

/// Pause and snooze.
#[derive(Debug, Serialize, ToSchema)]
pub struct QuietView {
    /// Nothing leaves the inbox until this is turned off.
    pub paused: bool,
    /// Nothing leaves the inbox until this time (Unix seconds).
    pub snoozed_until: Option<i64>,
}

/// Pause and snooze, to set.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct QuietRequest {
    /// Pause everything outside the inbox.
    pub paused: bool,
    /// Or for this many minutes (1 to 43200: thirty days); `null` or 0 ends a snooze.
    pub snooze_minutes: Option<u32>,
}

fn quiet_view(q: Quiet) -> QuietView {
    QuietView {
        paused: q.paused,
        snoozed_until: q.snoozed_until.filter(|until| *until > now()),
    }
}

/// The signed-in account's pause and snooze.
#[utoipa::path(
    get,
    path = "/api/v1/client/notifications/quiet",
    tag = "notifications",
    operation_id = "getNotificationQuiet",
    security(("session" = [])),
    responses((status = 200, description = "Pause and snooze", body = QuietView))
)]
pub async fn get_quiet(
    State(state): State<NotificationsState>,
    who: Principal,
) -> Result<Json<QuietView>, AppError> {
    let q = store::quiet(&state.db, who.account_id())
        .await
        .map_err(internal)?;
    Ok(Json(quiet_view(q)))
}

/// Pauses or snoozes (or ends it).
#[utoipa::path(
    put,
    path = "/api/v1/client/notifications/quiet",
    tag = "notifications",
    operation_id = "putNotificationQuiet",
    security(("session" = [])),
    request_body = QuietRequest,
    responses(
        (status = 200, description = "Pause and snooze", body = QuietView),
        (status = 422, description = "A snooze that is too long", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn put_quiet(
    State(state): State<NotificationsState>,
    who: Principal,
    Json(req): Json<QuietRequest>,
) -> Result<Json<QuietView>, AppError> {
    let minutes = req.snooze_minutes.unwrap_or(0);
    if minutes > 43_200 {
        return Err(bad(
            "snooze_minutes",
            "out_of_range",
            "At most 43200 (thirty days).",
        ));
    }
    let q = Quiet {
        paused: req.paused,
        snoozed_until: (minutes > 0).then(|| now() + i64::from(minutes) * 60),
    };
    store::set_quiet(&state.db, who.account_id(), q)
        .await
        .map_err(internal)?;
    Ok(Json(quiet_view(q)))
}

/// The events that can be sent to a channel.
#[derive(Debug, Serialize, ToSchema)]
pub struct EventList {
    /// Their names.
    pub events: Vec<&'static str>,
}

fn allowed_events(who: &Principal) -> &'static [&'static str] {
    if who.kind == DbAccountKind::Client {
        templates::CLIENT_EVENTS
    } else {
        templates::ADMIN_EVENTS
    }
}

/// The events the signed-in account may send to a channel.
#[utoipa::path(
    get,
    path = "/api/v1/client/notifications/events",
    tag = "notifications",
    operation_id = "listNotificationEvents",
    security(("session" = [])),
    responses((status = 200, description = "The events", body = EventList))
)]
pub async fn events(who: Principal) -> Json<EventList> {
    Json(EventList {
        events: allowed_events(&who).to_vec(),
    })
}

/// A channel.
#[derive(Debug, Serialize, ToSchema)]
pub struct ChannelView {
    /// Id.
    pub id: Uuid,
    /// `email` or `webhook`.
    pub kind: String,
    /// Its name.
    pub name: String,
    /// The address or the URL.
    pub target: String,
    /// Whether it is used.
    pub enabled: bool,
    /// The events that go to it.
    pub events: Vec<String>,
}

/// The channels of the signed-in account.
#[utoipa::path(
    get,
    path = "/api/v1/client/notifications/channels",
    tag = "notifications",
    operation_id = "listNotificationChannels",
    security(("session" = [])),
    responses((status = 200, description = "The channels", body = Vec<ChannelView>))
)]
pub async fn channels(
    State(state): State<NotificationsState>,
    who: Principal,
) -> Result<Json<Vec<ChannelView>>, AppError> {
    let list = store::channels(&state.db, who.account_id())
        .await
        .map_err(internal)?
        .into_iter()
        .map(|c| ChannelView {
            id: c.id,
            kind: c.kind,
            name: c.name,
            target: c.target,
            enabled: c.enabled,
            events: c.events,
        })
        .collect();
    Ok(Json(list))
}

/// A channel to make.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NewChannel {
    /// `email` or `webhook`.
    pub kind: String,
    /// A name (1 to 64 characters).
    pub name: String,
    /// An email address, or an `https` URL.
    pub target: String,
    /// The events that go to it.
    #[serde(default)]
    pub events: Vec<String>,
}

/// A channel that was made.
#[derive(Debug, Serialize, ToSchema)]
pub struct CreatedChannel {
    /// The channel.
    pub channel: ChannelView,
    /// A webhook's signing secret: shown this once (the calls carry `X-BuiltPanel-Signature`, the
    /// HMAC-SHA256 of the body under it).
    pub secret: Option<String>,
}

fn check_events(who: &Principal, events: &[String]) -> Result<(), AppError> {
    let allowed = allowed_events(who);
    if events.len() > allowed.len() || events.iter().any(|e| !allowed.contains(&e.as_str())) {
        return Err(bad(
            "events",
            "invalid",
            &format!("Only these: {}.", allowed.join(", ")),
        ));
    }
    Ok(())
}

fn check_email(address: &str) -> bool {
    address.len() <= 254
        && address
            .chars()
            .all(|c| !c.is_whitespace() && !c.is_control() && c != '<' && c != '>' && c != ',')
        && address.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
        })
}

/// Makes a channel.
#[utoipa::path(
    post,
    path = "/api/v1/client/notifications/channels",
    tag = "notifications",
    operation_id = "createNotificationChannel",
    security(("session" = [])),
    request_body = NewChannel,
    responses(
        (status = 201, description = "Made", body = CreatedChannel),
        (status = 409, description = "`channel_exists` or `too_many_channels`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "A kind, name, target or event that is not usable", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn add_channel(
    State(state): State<NotificationsState>,
    who: Principal,
    Json(req): Json<NewChannel>,
) -> Result<(StatusCode, Json<CreatedChannel>), AppError> {
    let name = req.name.trim();
    if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
        return Err(bad("name", "invalid", "1 to 64 characters."));
    }
    let target = req.target.trim();
    match req.kind.as_str() {
        "email" if check_email(target) => {}
        "email" => return Err(bad("target", "invalid", "Not an email address.")),
        "webhook" => {
            send::check_webhook_url(target).map_err(|m| bad("target", "invalid", m))?;
        }
        _ => return Err(bad("kind", "invalid", "email or webhook.")),
    }
    check_events(&who, &req.events)?;
    let account = who.account_id();
    let (secret, sealed) = if req.kind == "webhook" {
        let mut raw = [0u8; 32];
        ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut raw)
            .map_err(|_| internal(std::io::Error::other("no randomness")))?;
        let text = format!(
            "whsec_{}",
            raw.iter().map(|b| format!("{b:02x}")).collect::<String>()
        );
        let sealed = state
            .master_key
            .seal(&webhook_context(account), text.as_bytes())
            .map_err(|e| internal(std::io::Error::other(e.to_string())))?;
        (Some(text), Some(sealed))
    } else {
        (None, None)
    };
    let id = store::add_channel(
        &state.db,
        account,
        &req.kind,
        name,
        target,
        sealed.as_deref(),
    )
    .await
    .map_err(|e| match e {
        store::ChannelError::Duplicate => {
            AppError::conflict("channel_exists", "A channel of that name exists already.")
        }
        store::ChannelError::TooMany => AppError::conflict(
            "too_many_channels",
            format!("At most {} channels.", store::MAX_CHANNELS),
        ),
        store::ChannelError::Database(db) => internal(db),
    })?;
    if !req.events.is_empty() {
        store::update_channel(&state.db, account, id, None, Some(&req.events))
            .await
            .map_err(internal)?;
    }
    let mut events = req.events;
    events.sort();
    events.dedup();
    Ok((
        StatusCode::CREATED,
        Json(CreatedChannel {
            channel: ChannelView {
                id,
                kind: req.kind,
                name: name.to_owned(),
                target: target.to_owned(),
                enabled: true,
                events,
            },
            secret,
        }),
    ))
}

/// What to change in a channel.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ChannelChange {
    /// Switch it on or off.
    pub enabled: Option<bool>,
    /// The events that go to it (replaces the list).
    pub events: Option<Vec<String>>,
}

/// Changes a channel.
#[utoipa::path(
    patch,
    path = "/api/v1/client/notifications/channels/{id}",
    tag = "notifications",
    operation_id = "updateNotificationChannel",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "The channel")),
    request_body = ChannelChange,
    responses(
        (status = 204, description = "Changed"),
        (status = 404, description = "No such channel", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "An event that is not usable", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn update_channel(
    State(state): State<NotificationsState>,
    who: Principal,
    Path(id): Path<Uuid>,
    Json(req): Json<ChannelChange>,
) -> Result<StatusCode, AppError> {
    if let Some(events) = &req.events {
        check_events(&who, events)?;
    }
    let found = store::update_channel(
        &state.db,
        who.account_id(),
        id,
        req.enabled,
        req.events.as_deref(),
    )
    .await
    .map_err(internal)?;
    if found {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError::not_found("channel"))
    }
}

/// Removes a channel.
#[utoipa::path(
    delete,
    path = "/api/v1/client/notifications/channels/{id}",
    tag = "notifications",
    operation_id = "deleteNotificationChannel",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "The channel")),
    responses(
        (status = 204, description = "Removed"),
        (status = 404, description = "No such channel", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn delete_channel(
    State(state): State<NotificationsState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    if store::delete_channel(&state.db, who.account_id(), id)
        .await
        .map_err(internal)?
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError::not_found("channel"))
    }
}

// -------------------------------------------------------------------- mail server

/// The mail server (without its password).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SmtpView {
    /// Host name.
    pub host: String,
    /// Port.
    pub port: u16,
    /// `starttls`, `tls` or `none`.
    pub security: String,
    /// The login, if any.
    pub username: Option<String>,
    /// Whether a password is set.
    pub has_password: bool,
    /// The address mail comes from.
    pub from_address: String,
    /// The name mail comes from.
    pub from_name: Option<String>,
    /// Whether mail is sent.
    pub enabled: bool,
}

/// The mail server, to set.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SmtpRequest {
    /// Host name.
    pub host: String,
    /// Port.
    pub port: u16,
    /// `starttls`, `tls` or `none`.
    pub security: String,
    /// The login, if any.
    pub username: Option<String>,
    /// The password; left out, the one there is stays.
    pub password: Option<String>,
    /// The address mail comes from.
    pub from_address: String,
    /// The name mail comes from.
    pub from_name: Option<String>,
    /// Whether mail is sent.
    pub enabled: bool,
}

/// The panel's mail server.
#[utoipa::path(
    get,
    path = "/api/v1/admin/notifications/smtp",
    tag = "notifications",
    operation_id = "getNotificationSmtp",
    security(("session" = [])),
    responses(
        (status = 200, description = "The mail server (`null` when none is set)", body = Option<SmtpView>),
        (status = 403, description = "`permission_denied` (needs admin.settings.read)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn get_smtp(
    State(state): State<NotificationsState>,
    _who: Require<SettingsRead>,
) -> Result<Json<Option<SmtpView>>, AppError> {
    let smtp = store::smtp(&state.db).await.map_err(internal)?;
    Ok(Json(smtp.map(|s| SmtpView {
        host: s.host,
        port: u16::try_from(s.port).unwrap_or(0),
        security: s.security,
        username: s.username,
        has_password: s.sealed_password.is_some(),
        from_address: s.from_address,
        from_name: s.from_name,
        enabled: s.enabled,
    })))
}

/// Sets the panel's mail server.
#[utoipa::path(
    put,
    path = "/api/v1/admin/notifications/smtp",
    tag = "notifications",
    operation_id = "putNotificationSmtp",
    security(("session" = [])),
    request_body = SmtpRequest,
    responses(
        (status = 204, description = "Set"),
        (status = 403, description = "`permission_denied` (needs admin.settings.manage)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "A host, port, security or address that is not usable", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn put_smtp(
    State(state): State<NotificationsState>,
    _who: Require<SettingsManage>,
    Json(req): Json<SmtpRequest>,
) -> Result<StatusCode, AppError> {
    let host = req.host.trim();
    if host.is_empty()
        || host.len() > 253
        || !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'))
    {
        return Err(bad("host", "invalid", "A host name or an address."));
    }
    if req.port == 0 {
        return Err(bad("port", "invalid", "1 to 65535."));
    }
    if !matches!(req.security.as_str(), "starttls" | "tls" | "none") {
        return Err(bad("security", "invalid", "starttls, tls or none."));
    }
    if !check_email(req.from_address.trim()) {
        return Err(bad("from_address", "invalid", "Not an email address."));
    }
    if req
        .username
        .as_ref()
        .is_some_and(|u| u.len() > 254 || u.chars().any(char::is_control))
        || req.password.as_ref().is_some_and(|p| p.len() > 500)
        || req
            .from_name
            .as_ref()
            .is_some_and(|n| n.chars().count() > 100 || n.chars().any(char::is_control))
    {
        return Err(bad(
            "username",
            "invalid",
            "A login, password or name that is too long or has control characters.",
        ));
    }
    let sealed = req
        .password
        .as_deref()
        .filter(|p| !p.is_empty())
        .map(|p| state.master_key.seal(SMTP_CONTEXT, p.as_bytes()))
        .transpose()
        .map_err(|e| internal(std::io::Error::other(e.to_string())))?;
    store::set_smtp(
        &state.db,
        &store::Smtp {
            host: host.to_owned(),
            port: i32::from(req.port),
            security: req.security,
            username: req.username.filter(|u| !u.is_empty()),
            sealed_password: sealed,
            from_address: req.from_address.trim().to_owned(),
            from_name: req.from_name.filter(|n| !n.is_empty()),
            enabled: req.enabled,
        },
    )
    .await
    .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Where to send a test mail.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SmtpTest {
    /// The address.
    pub to: String,
}

/// Sends a test mail through the mail server that is set.
#[utoipa::path(
    post,
    path = "/api/v1/admin/notifications/smtp/test",
    tag = "notifications",
    operation_id = "testNotificationSmtp",
    security(("session" = [])),
    request_body = SmtpTest,
    responses(
        (status = 204, description = "Sent"),
        (status = 409, description = "`smtp_failed` (the reason is in the message) or `smtp_not_set`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "An address that is not usable", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn test_smtp(
    State(state): State<NotificationsState>,
    _who: Require<SettingsManage>,
    Json(req): Json<SmtpTest>,
) -> Result<StatusCode, AppError> {
    let to = req.to.trim().to_owned();
    if !check_email(&to) {
        return Err(bad("to", "invalid", "Not an email address."));
    }
    let server = mail_server(&state)
        .await
        .map_err(|m| AppError::conflict("smtp_failed", m))?
        .ok_or_else(|| {
            AppError::conflict(
                "smtp_not_set",
                "No mail server is set up (or it is switched off).",
            )
        })?;
    let panel = panel_name(&state).await;
    let (text, html) = templates::mail(
        &format!("[{panel}] Test"),
        "This is a test message from the panel. If you can read it, the mail server works.",
        &panel,
        false,
    );
    tokio::task::spawn_blocking(move || {
        send::send_mail(&server, &to, &format!("[{panel}] Test"), &text, &html)
    })
    .await
    .map_err(|_| internal(std::io::Error::other("cancelled")))?
    .map_err(|m| AppError::conflict("smtp_failed", m))?;
    Ok(StatusCode::NO_CONTENT)
}

/// The module's OpenAPI document.
#[derive(OpenApi)]
#[openapi(
    paths(
        list, mark_read, get_quiet, put_quiet, events, channels, add_channel, update_channel,
        delete_channel, get_smtp, put_smtp, test_smtp
    ),
    components(schemas(
        NotificationView, NotificationList, ReadRequest, Updated, QuietView, QuietRequest, EventList,
        ChannelView, NewChannel, CreatedChannel, ChannelChange, SmtpView, SmtpRequest, SmtpTest
    )),
    tags((name = "notifications", description = "Notifications"))
)]
pub struct NotificationsApi;
